// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Saving as a [DAWproject](https://github.com/bitwig/dawproject) file: the interchange format Bitwig Studio, Studio
//! One, Cubase and others read (ZIP with `project.xml`; spec: the DAWproject 1.0 schema).
//!
//! The project's *standard* data is written the way the format describes it: the transport's tempo, one track and
//! channel per mixer track (name, colour, fader, pan, mute, sends, the channel's instrument and the track's effects),
//! and the arrangement's clips with their notes. What DAWproject 1.0 has no place for — plugin state bytes, the
//! channel strip, the equaliser, the modulation graph, patterns' own settings, the launcher — travels beside it as
//! `fruitbat/project.flp`, the file the rest of the program already knows how to write and read. A DAW ignores files no
//! element references, so the container stays valid for it, and Fruitbat Studio reopens its own project exactly as it
//! was saved.
//!
//! So: a `.dawproject` from here opens elsewhere with its structure, notes and mixer, and opens here with everything.

use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};

use os_model::{Clip, ClipSource, Project};

/// The extension the format fixes.
pub const EXT: &str = "dawproject";
/// Where the project's own (non-standard) state rides inside the archive.
const NATIVE: &str = "fruitbat/project.flp";
/// The application a Fruitbat Studio file names in its `project.xml`.
const APP: &str = "Fruitbat Studio";

/// Whether `p` is a DAWproject file by its extension.
pub fn is_project(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXT))
}

/// `p` with the `.dawproject` extension.
pub fn as_project(p: &Path) -> PathBuf {
    p.with_extension(EXT)
}

/// The project as a `.dawproject` archive: `project.xml` for other DAWs, the project's own file beside it.
pub fn pack(project: &Project, tempo: f64, native: &[u8]) -> Result<Vec<u8>, String> {
    let xml = project_xml(project, tempo);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file("project.xml", options).map_err(|e| e.to_string())?;
    zip.write_all(xml.as_bytes()).map_err(|e| e.to_string())?;
    zip.start_file(NATIVE, options).map_err(|e| e.to_string())?;
    zip.write_all(native).map_err(|e| e.to_string())?;
    let cursor = zip.finish().map_err(|e| e.to_string())?;
    Ok(cursor.into_inner())
}

/// The project's own file inside a `.dawproject`: what a Fruitbat Studio file saved, for the existing open path to read.
/// A file written by another DAW has none, and says so.
pub fn native(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut zip = zip::ZipArchive::new(Cursor::new(data)).map_err(|e| format!("not a DAWproject archive: {e}"))?;
    let mut file = zip.by_name(NATIVE).map_err(|_| "A project saved by another program is not read yet: this build reads the projects it writes.".to_string())?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// `project.xml`: the standard part of the project (spec: the DAWproject 1.0 schema, `Project.xsd`).
pub fn project_xml(project: &Project, tempo: f64) -> String {
    let mut ids = 0u32;
    let mut next = || {
        let id = format!("id{ids}");
        ids += 1;
        id
    };
    let ppq = f64::from(project.ppq.max(1));
    let master_track = next();
    let master_channel = next();
    let mut out = String::with_capacity(4096);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
    out.push_str("<Project version=\"1.0\">\n");
    out.push_str(&format!("<Application name=\"{APP}\" version=\"{}\"/>\n", escape(env!("CARGO_PKG_VERSION"))));
    out.push_str("<Transport>\n");
    out.push_str(&format!("<Tempo max=\"666.000000\" min=\"20.000000\" unit=\"bpm\" value=\"{tempo:.6}\" id=\"{}\" name=\"Tempo\"/>\n", next()));
    if let Some((numerator, steps)) = project.bar {
        // A bar of `numerator` beats over `steps` sixteenths is the signature `numerator / (16 / steps)`.
        let denominator = 16 / u32::from(steps.max(1));
        out.push_str(&format!(
            "<TimeSignature denominator=\"{denominator}\" numerator=\"{numerator}\" id=\"{}\"/>\n",
            next()
        ));
    }
    out.push_str("</Transport>\n<Structure>\n");
    // Everything plays into the master: the format wants a destination on every channel that is not the master.
    let tracks: Vec<(String, String)> = project
        .mixer
        .iter()
        .filter(|m| m.index != 0)
        .map(|m| (next(), next()))
        .collect();
    for (i, track) in project.mixer.iter().filter(|m| m.index != 0).enumerate() {
        let (track_id, channel_id) = &tracks[i];
        out.push_str(&format!(
            "<Track contentType=\"audio notes\" loaded=\"true\" id=\"{track_id}\" name=\"{}\"{}>\n",
            escape(track.name.as_deref().unwrap_or(&format!("Track {}", track.index))),
            track.color.map_or_else(String::new, |c| format!(" color=\"#{:02x}{:02x}{:02x}\"", c.r, c.g, c.b)),
        ));
        out.push_str(&format!(
            "<Channel audioChannels=\"2\" destination=\"{master_channel}\" role=\"regular\" solo=\"false\" id=\"{channel_id}\">\n"
        ));
        // The channel's instrument, then the track's effect slots, both as built-in devices by name: a plugin is named
        // rather than given an identity the format would need a real plugin ID for.
        let instruments: Vec<&os_model::Channel> = project.channels.iter().filter(|c| c.mixer_track == track.index && c.kind == os_model::ChannelKind::Plugin).collect();
        let slots: Vec<&os_model::SlotPlugin> = track.slots.iter().flatten().collect();
        if !instruments.is_empty() || !slots.is_empty() {
            out.push_str("<Devices>\n");
            for c in &instruments {
                let name = c.plugin.as_deref().or(c.name.as_deref()).unwrap_or("Instrument");
                device(&mut out, &mut next, name, "instrument");
            }
            for slot in slots {
                device(&mut out, &mut next, &slot.name, "audioFX");
            }
            out.push_str("</Devices>\n");
        }
        out.push_str(&format!("<Mute value=\"{}\" id=\"{}\" name=\"Mute\"/>\n", track.enabled, next()));
        let pan = (track.pan.unwrap_or(0.0) + 1.0) / 2.0;
        out.push_str(&format!(
            "<Pan max=\"1.000000\" min=\"0.000000\" unit=\"normalized\" value=\"{pan:.6}\" id=\"{}\" name=\"Pan\"/>\n",
            next()
        ));
        if !track.routes.is_empty() {
            out.push_str("<Sends>\n");
            for &destination in &track.routes {
                let Some((dest_id, _)) = tracks.get(usize::from(destination).saturating_sub(1)) else { continue };
                let level = track.send_levels.iter().find(|(d, _)| *d == destination).map_or(0.8, |&(_, l)| l);
                out.push_str(&format!(
                    "<Send destination=\"{dest_id}\" type=\"post\" id=\"{}\"><Volume max=\"2.000000\" min=\"0.000000\" unit=\"linear\" value=\"{:.6}\" id=\"{}\" name=\"Volume\"/></Send>\n",
                    next(),
                    level / 0.8,
                    next()
                ));
            }
            out.push_str("</Sends>\n");
        }
        // A fader of 0.8 is unity on the strip; the format's linear volume is 1 at unity.
        let volume = track.volume.unwrap_or(0.8) / 0.8;
        out.push_str(&format!(
            "<Volume max=\"2.000000\" min=\"0.000000\" unit=\"linear\" value=\"{volume:.6}\" id=\"{}\" name=\"Volume\"/>\n",
            next()
        ));
        out.push_str("</Channel>\n</Track>\n");
    }
    out.push_str(&format!(
        "<Track contentType=\"audio notes\" loaded=\"true\" id=\"{master_track}\" name=\"Master\">\n<Channel audioChannels=\"2\" role=\"master\" solo=\"false\" id=\"{master_channel}\">\n"
    ));
    out.push_str(&format!("<Mute value=\"false\" id=\"{}\" name=\"Mute\"/>\n", next()));
    out.push_str(&format!("<Pan max=\"1.000000\" min=\"0.000000\" unit=\"normalized\" value=\"0.500000\" id=\"{}\" name=\"Pan\"/>\n", next()));
    out.push_str(&format!("<Volume max=\"2.000000\" min=\"0.000000\" unit=\"linear\" value=\"1.000000\" id=\"{}\" name=\"Volume\"/>\n", next()));
    out.push_str("</Channel>\n</Track>\n</Structure>\n");
    out.push_str(&format!("<Arrangement id=\"{}\">\n<Lanes timeUnit=\"beats\" id=\"{}\">\n", next(), next()));
    let arrangement = project.arrangements.iter().find(|a| Some(a.number) == project.current_arrangement).or_else(|| project.arrangements.first());
    for (i, track) in project.mixer.iter().filter(|m| m.index != 0).enumerate() {
        let (track_id, _) = &tracks[i];
        out.push_str(&format!("<Lanes track=\"{track_id}\" id=\"{}\">\n<Clips id=\"{}\">\n", next(), next()));
        if let Some(a) = arrangement {
            for clip in a.clips.iter().filter(|c| playlist_clip_track(a, c) == Some(track.index)) {
                clip_xml(&mut out, &mut next, project, clip, ppq, a);
            }
        }
        out.push_str("</Clips>\n</Lanes>\n");
    }
    out.push_str(&format!("<Lanes track=\"{master_track}\" id=\"{}\">\n<Clips id=\"{}\"/>\n</Lanes>\n", next(), next()));
    out.push_str("</Lanes>\n</Arrangement>\n<Scenes/>\n</Project>\n");
    out
}

/// One `<Clip>`: its notes from the pattern it plays, or its audio file.
fn clip_xml(out: &mut String, next: &mut impl FnMut() -> String, project: &Project, clip: &Clip, ppq: f64, a: &os_model::Arrangement) {
    let time = f64::from(clip.position) / ppq;
    let duration = f64::from(clip.length) / ppq;
    let name = a.tracks.iter().find(|t| t.number == u32::from(clip.track) + 1).and_then(|t| t.name.clone());
    out.push_str(&format!(
        "<Clip time=\"{time:.6}\" duration=\"{duration:.6}\" playStart=\"0.000000\" enable=\"{}\"{}>\n",
        !clip.muted,
        name.map_or_else(String::new, |n| format!(" name=\"{}\"", escape(&n))),
    ));
    match clip.source {
        ClipSource::Pattern(number) => {
            out.push_str(&format!("<Notes id=\"{}\">\n", next()));
            if let Some(pattern) = project.patterns.iter().find(|p| p.number == number) {
                // The clip's window into the pattern is in pattern ticks; the format's time is beats from the clip's start.
                let start = clip.window.map_or(0.0, |w| w.0);
                let len = f64::from(clip.length);
                for note in &pattern.notes {
                    let at = f64::from(note.position) - start;
                    // A note that starts past the clip is the next clip's; a step has no length of its own.
                    if at < 0.0 || at >= len {
                        continue;
                    }
                    let note_len = f64::from(note.length.max(1)).min(len - at);
                    let lane = project.channels.iter().position(|c| c.index == note.channel).map_or(0, |i| i);
                    out.push_str(&format!(
                        "<Note time=\"{:.6}\" duration=\"{:.6}\" channel=\"{lane}\" key=\"{}\" vel=\"{:.6}\" rel=\"{:.6}\"/>\n",
                        at / ppq,
                        note_len / ppq,
                        note.key.min(127),
                        f64::from(note.velocity) / 127.0,
                        f64::from(note.release) / 127.0,
                    ));
                }
            }
            out.push_str("</Notes>\n");
        }
        ClipSource::Channel(index) => {
            if let Some(path) = project.channels.iter().find(|c| c.index == index).and_then(|c| c.sample_path.clone()) {
                // The file is referenced where it is: a copy would double every sample in the project.
                out.push_str(&format!(
                    "<Audio algorithm=\"raw\" channels=\"2\" duration=\"{duration:.6}\" sampleRate=\"44100\" id=\"{}\"><File path=\"{}\" external=\"true\"/></Audio>\n",
                    next(),
                    escape(&path),
                ));
            }
        }
    }
    out.push_str("</Clip>\n");
}

/// A device element: an instrument or effect named for what it is, with no plugin identity invented.
fn device(out: &mut String, next: &mut impl FnMut() -> String, name: &str, role: &str) {
    out.push_str(&format!(
        "<BuiltinDevice deviceName=\"{}\" deviceRole=\"{role}\" loaded=\"true\" id=\"{}\"><Enabled value=\"true\" id=\"{}\"/></BuiltinDevice>\n",
        escape(name),
        next(),
        next()
    ));
}

/// The mixer track a playlist clip plays on: the playlist track it names, linked to a mixer strip.
fn playlist_clip_track(a: &os_model::Arrangement, clip: &Clip) -> Option<u16> {
    let track = a.tracks.iter().find(|t| t.number == u32::from(clip.track) + 1)?;
    track.link.mixer
}

/// XML text with its five special characters escaped.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use os_model::{Channel, ChannelKind, MixerTrack, Pattern};

    fn project() -> Project {
        let mut p = Project::default();
        p.ppq = 96;
        p.tempo = Some(128.0);
        p.mixer = vec![MixerTrack::new(0), MixerTrack::new(1), MixerTrack::new(2)];
        p.mixer[1].name = Some("Bass & \"Lead\"".into());
        p.mixer[1].volume = Some(0.4);
        let mut channel = crate::app::sampler_channel(0, "Surge <XT>", 1);
        channel.kind = ChannelKind::Plugin;
        channel.plugin = Some("Surge <XT>".into());
        p.channels = vec![channel];
        let mut pat = Pattern::default();
        pat.number = 1;
        p.patterns = vec![pat];
        p
    }

    #[test]
    fn the_archive_holds_the_standard_project_and_the_original() {
        let p = project();
        let bytes = pack(&p, 128.0, b"native-bytes").expect("pack");
        assert_eq!(&bytes[..2], b"PK", "a zip");
        // Two files and no more: the standard project, and the project's own state beside it. No `.lemon` file.
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes.clone())).expect("a zip archive");
        let mut names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).expect("an entry").name().expect("a name").to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["fruitbat/project.flp", "project.xml"]);
        assert_eq!(native(&bytes).expect("unpack"), b"native-bytes");
    }

    #[test]
    fn project_xml_is_the_formats_own_shape() {
        let xml = project_xml(&project(), 128.0);
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Project version=\"1.0\">"));
        assert!(xml.contains("<Application name=\"Fruitbat Studio\""));
        assert!(xml.contains("<Tempo max=\"666.000000\" min=\"20.000000\" unit=\"bpm\" value=\"128.000000\""));
        assert!(xml.contains("name=\"Bass &amp; &quot;Lead&quot;\""), "names are escaped: {xml}");
        assert!(xml.contains("deviceName=\"Surge &lt;XT&gt;\" deviceRole=\"instrument\""));
        // A fader of 0.4 over 0.8 unity is 0.5 linear.
        assert!(xml.contains("<Volume max=\"2.000000\" min=\"0.000000\" unit=\"linear\" value=\"0.500000\""));
        assert!(xml.ends_with("</Project>\n"));
        // Every id is unique: the format keys its references by them.
        let ids: Vec<&str> = xml
            .match_indices(" id=\"")
            .map(|(i, _)| {
                let rest = &xml[i + 5..];
                &rest[..rest.find('"').unwrap_or(0)]
            })
            .collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(ids.len(), unique.len(), "ids repeat: {ids:?}");
    }

    #[test]
    fn a_foreign_file_does_not_pass_for_ours() {
        let xml = project_xml(&project(), 128.0);
        let options = zip::write::SimpleFileOptions::default();
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file("project.xml", options).unwrap();
        zip.write_all(xml.as_bytes()).unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        assert!(native(&bytes).is_err());
        assert!(native(b"not a zip").is_err());
    }
}
