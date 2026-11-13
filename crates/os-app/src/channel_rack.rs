// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The Channel Rack, restored: one step-sequencer instrument on one Playlist track. It is a channel whose instrument is a
//! container (`crate::container`) holding the Lemon Drum Sampler's lanes as one instrument member, so the rack plays the
//! channel's notes through one chain and out of the channel's own mixer track: its internal lanes never become channels,
//! Playlist tracks or mixer strips. The lanes are the sampler's pads (vertical) and the steps the project pattern's notes
//! on the rack channel, keyed by each pad's MIDI key exactly as the Drum Sampler's own STEP grid keys them
//! (`crate::rack::drum_step_on`, `set_drum_step`). Patterns stay the project's: Arrangement clips and Clip Launcher slots
//! play the same `Pattern`, and the engine's clock plays them; the rack adds no transport and no second pattern store.
//!
//! The window is the container window (`views::container`, `Win::Container`), which paints the step editor for a rack
//! channel (`views::channel_rack`). A new rack is ADD ▸ Channel Rack: a channel, its one instrument track and the
//! container's window, all in one undo step. The standalone Lemon Drum Sampler is untouched and independent.
//!
//! spec: ui/channel-rack.md; plugins/patcher/README.md slice 3; plugins/lemon-drum-sampler.md

use os_model::{Channel, ChannelKind};

use crate::app::{App, State};
use crate::container::{self, Member, CHANNEL};
use crate::plugins::Plugins;

/// The channel plugin name a Channel Rack is stored under (its state lists the members, as a container's does).
pub const INSTRUMENT: &str = "Lemon Channel Rack";
/// Pads shown at once: one bank of the sampler's two.
pub const LANES: usize = 16;
/// Steps shown at once (the original's default window).
pub const STEPS: u32 = 16;

/// Whether `ch` is a Channel Rack.
pub fn is_rack_channel(ch: &Channel) -> bool {
    ch.kind == ChannelKind::Plugin && ch.plugin.as_deref() == Some(INSTRUMENT)
}

/// One sound lane of a rack: the sampler pad that plays it.
#[derive(Debug, Clone, PartialEq)]
pub struct Lane {
    /// The pad's index in the sampler (what its own editor's pad number is).
    pub pad: usize,
    /// The MIDI key its notes carry, and the row its steps are in the pattern.
    pub key: u16,
    /// The pad's name, else "Pad N".
    pub name: String,
    /// Whether a sample is loaded (an empty lane is drawn quieter).
    pub sample: bool,
}

/// The rack row of channel `ch`, when it is a Channel Rack.
pub fn row_of(s: &State, ch: u16) -> Option<usize> {
    s.project.channels.iter().position(|c| c.index == ch && is_rack_channel(c))
}

/// The row a container window shows as a rack, when it is one.
pub fn rack_row(s: &State) -> Option<usize> {
    let (t, k) = s.container.at;
    if k != CHANNEL {
        return None;
    }
    let row = usize::from(t);
    s.project.channels.get(row).filter(|c| is_rack_channel(c)).map(|_| row)
}

/// The rack's lanes, from the sampler's live saved state, so a renamed or retuned pad is the same row here and in the
/// sampler's own editor.
pub fn lanes(s: &State, plugins: &Plugins, row: usize) -> Vec<Lane> {
    let Ok(row16) = u16::try_from(row) else {
        return Vec::new();
    };
    let Some(list) = container::members(s, row16, CHANNEL) else {
        return Vec::new();
    };
    let Some(at) = list.iter().position(|m| m.instrument) else {
        return Vec::new();
    };
    let key = container::place_key(row16, CHANNEL);
    let state = plugins
        .containers
        .get(&key)
        .and_then(|live| live.keys.get(at).copied().flatten())
        .map(|k| crate::rack_ui::drum_state(plugins, k));
    let Some(state) = state else {
        // Not hosted yet: the sampler's defaults, 32 pads on the General MIDI drum keys.
        return (0..32)
            .map(|i| Lane { pad: i, key: 36 + i as u16, name: format!("Pad {}", i + 1), sample: false })
            .collect();
    };
    state
        .pads()
        .into_iter()
        .enumerate()
        .map(|(i, p)| Lane {
            pad: i,
            key: p.key.clamp(0, 127) as u16,
            name: if p.name.is_empty() { format!("Pad {}", i + 1) } else { p.name.clone() },
            sample: p.sounding().next().is_some(),
        })
        .collect()
}

/// Whether step `step` of `lane` is on in the current pattern.
pub fn step_on(s: &State, ch: u16, lane: &Lane, step: u32) -> bool {
    crate::rack::drum_step_on(s, ch, lane.key, step)
}

/// Turns step `step` of `lane` on or off in the current pattern (one undo step is the caller's, `rack::snapshot`).
pub fn set_step(s: &mut State, ch: u16, lane: &Lane, step: u32, on: bool) {
    crate::rack::set_drum_step(s, ch, lane.key, step, on);
}

/// Clears this rack channel's steps in the current pattern, leaving other channels' notes alone.
pub fn clear_steps(s: &mut State, ch: u16) {
    crate::rack::clear_channel_steps(s, ch);
}

/// The Channel Rack's undo step name, and the name of moving a lane's steps.
pub const ADD: &str = "Add channel";

/// A new Channel Rack: a channel whose instrument is a container holding the drum sampler as its one instrument member,
/// on a new instrument track, with its editor open. One undo step. `None` when there is no room for the track.
pub fn create(app: &mut App) -> Option<String> {
    crate::control::edit_app(app, ADD, crate::control::command::Region::Project, |app| {
        let Some(uid) = sampler_uid(app) else {
            return Some(format!("{INSTRUMENT}: the Lemon Drum Sampler is not available"));
        };
        let row = crate::app::add_plugin_channel(&mut app.state.project, INSTRUMENT);
        let member = Member {
            id: 1,
            instrument: true,
            uid,
            name: crate::drums::LEMON_DRUMS.to_string(),
            wet: container::INSTRUMENT_LEVELS.0,
            dry: container::INSTRUMENT_LEVELS.1,
            ..Member::default()
        };
        let ch = &mut app.state.project.channels[row];
        ch.plugin = Some(INSTRUMENT.into());
        ch.plugin_state = Some(container::encode(&[member]));
        app.state.selected_channel = row;
        crate::rack::autosize(&mut app.state);
        // One parent track: the channel's instrument track, which also names the channel's insert after it.
        let msg = crate::track_links::assign_instrument(&mut app.state, &[row]);
        if let Ok(t) = u16::try_from(row) {
            container::show(&mut app.state, t, CHANNEL);
        }
        app.state.piano.dirty = true;
        msg.or_else(|| Some(format!("Added {INSTRUMENT}")))
    })
}

/// The drum sampler's identity, hosting it first when the program has not scanned it yet.
fn sampler_uid(app: &mut App) -> Option<String> {
    if let Some(i) = app.plugins.index_of(crate::drums::LEMON_DRUMS) {
        return Some(app.plugins.available.get(i)?.desc.uid());
    }
    let a = crate::plugins::builtin().into_iter().find(|a| a.desc.name == crate::drums::LEMON_DRUMS)?;
    let uid = a.desc.uid();
    app.plugins.available.push(a);
    Some(uid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use os_engine::Instrument;

    /// A 0.2 s stereo 16-bit WAV at 44.1 kHz: a tone the rack's lane can sound.
    fn wav(name: &str) -> std::path::PathBuf {
        let frames = 8820usize;
        let mut data = Vec::new();
        for i in 0..frames {
            let v = ((i as f32 * 0.05).sin() * 16000.0) as i16;
            data.extend(v.to_le_bytes());
            data.extend(v.to_le_bytes());
        }
        let mut fmt = Vec::new();
        for v in [1u16, 2] {
            fmt.extend(v.to_le_bytes());
        }
        fmt.extend(44100u32.to_le_bytes());
        fmt.extend((44100u32 * 4).to_le_bytes());
        fmt.extend(4u16.to_le_bytes());
        fmt.extend(16u16.to_le_bytes());
        let mut body = b"WAVE".to_vec();
        for (id, b) in [(b"fmt ", &fmt), (b"data", &data)] {
            body.extend(id);
            body.extend((b.len() as u32).to_le_bytes());
            body.extend(b);
        }
        let mut f = b"RIFF".to_vec();
        f.extend((body.len() as u32).to_le_bytes());
        f.extend(body);
        let p = std::env::temp_dir().join(format!("lemon-rack-{}-{name}.wav", std::process::id()));
        std::fs::write(&p, f).unwrap();
        p
    }

    /// The rack channel's chain, as the engine plays it.
    fn chain_of(a: &App, row: usize) -> os_engine::SharedInstrument {
        a.plugins.instruments().into_iter().find(|(r, _)| *r == row).map(|(_, i)| i).expect("the rack plays its row")
    }

    fn peak(inst: &os_engine::SharedInstrument) -> f32 {
        let mut i = inst.lock().unwrap();
        let (mut l, mut r) = (vec![0.0f32; 2400], vec![0.0f32; 2400]);
        i.process(48000, &mut l, &mut r);
        l.iter().chain(&r).fold(0.0f32, |m, v| m.max(v.abs()))
    }

    /// A rack made by ADD ▸ Channel Rack: one channel, one instrument track, one mixer strip; the member hosted and its
    /// lanes the sampler's pads; the lane plays only its own key, and its steps are the pattern's notes on the rack
    /// channel — the same pattern Arrangement clips and launcher slots play.
    #[test]
    fn a_rack_is_one_instrument_on_one_track() {
        let mut a = App::new(None);
        a.state.project = crate::app::test_project();
        a.plugins.available = crate::plugins::builtin();
        let rows = a.state.project.channels.len();
        let inserts = a.state.project.mixer.len();
        let msg = create(&mut a).expect("the rack is added");
        assert!(msg.contains("Channel Rack"), "{msg}");
        assert_eq!(a.state.project.channels.len(), rows + 1, "one channel");
        let row = rows;
        let ch = a.state.project.channels[row].clone();
        assert!(is_rack_channel(&ch));
        assert_eq!(a.state.project.channels.iter().filter(|c| is_rack_channel(c)).count(), 1);
        assert_eq!(a.state.project.mixer.len(), inserts, "no extra mixer strip");
        let track = crate::track_links::instrument_track_of(&a.state, ch.index).expect("its instrument track");
        assert!(crate::track_links::is_instrument(&a.state, track));
        assert_eq!(crate::track_links::audio_tracks_of(&a.state, ch.mixer_track).len(), 0, "no audio tracks");
        assert!(a.state.shown(crate::app::Win::Container), "its editor is open");

        // A lane's sample is stored in its member, so the chain is built with it (as the sampler's editor writes it).
        let path = wav("lane");
        put_sample(&mut a, row, 0, &path);
        a.sync_containers();
        let lanes = lanes(&a.state, &a.plugins, row);
        assert!(lanes.len() >= 32, "the sampler's pads");
        assert!(lanes[0].sample, "the lane's sample is loaded");
        let key0 = lanes[0].key;
        assert!(lanes.iter().all(|l| (36..=127).contains(&l.key)), "every lane has a MIDI key");
        assert_ne!(lanes[0].key, lanes[1].key, "each lane has its own key");
        let id = a.state.project.channels[row].index;
        // Step 0 of the lane on key 36, in the project's pattern.
        crate::rack::snapshot(&mut a.state);
        set_step(&mut a.state, id, &lanes[0], 0, true);
        assert!(step_on(&a.state, id, &lanes[0], 0));
        assert_eq!(a.state.project.patterns[0].notes.iter().filter(|n| n.channel == id && n.key == key0).count(), 1);
        assert!(a.state.project.patterns[0].notes.iter().all(|n| n.channel == id), "only the rack's row");
        // The same pattern a clip launcher slot and an Arrangement clip reference.
        assert_eq!(os_model::ClipSource::Pattern(1), os_model::ClipSource::Pattern(a.state.pattern));
        // Right: the row plays the pattern through the existing clock. Here: the chain sounds that lane's key only.
        let inst = chain_of(&a, row);
        let mut i = inst.lock().unwrap();
        i.note_on(0, lanes[1].key as u8, 100.0 / 128.0);
        drop(i);
        assert!(peak(&inst) < 0.001, "an empty lane is silent");
        let mut i = inst.lock().unwrap();
        i.note_on(0, key0 as u8, 100.0 / 128.0);
        drop(i);
        assert!(peak(&inst) > 0.2, "the lane's key plays its sample");
        let _ = std::fs::remove_file(path);
    }

    /// Writes a sample into the rack's lane `pad`, as the sampler's own editor would, and keeps it in the member state.
    fn put_sample(a: &mut App, row: usize, pad: usize, path: &std::path::Path) {
        let mut list = container::members(&a.state, u16::try_from(row).unwrap(), CHANNEL).expect("the members");
        let fl = {
            let mut st = lemon_drum_sampler::state::State::default();
            st.set_sample(pad, &path.to_string_lossy());
            st.to_bytes()
        };
        list[0].state = crate::drums::plugin_state(&fl);
        a.state.project.channels[row].plugin_state = Some(container::encode(&list));
    }

    /// Two racks are independent: each plays its own channel's steps and its own mixer strip, and lanes never become
    /// tracks, channels or strips of their own.
    #[test]
    fn two_racks_are_independent() {
        let mut a = App::new(None);
        a.state.project = crate::app::test_project();
        a.plugins.available = crate::plugins::builtin();
        create(&mut a).expect("the first rack");
        create(&mut a).expect("the second rack");
        let racks: Vec<usize> = (0..a.state.project.channels.len()).filter(|&r| is_rack_channel(&a.state.project.channels[r])).collect();
        assert_eq!(racks.len(), 2);
        let first = a.state.project.channels[racks[0]].index;
        let second = a.state.project.channels[racks[1]].index;
        assert_ne!(first, second);
        assert_ne!(
            a.state.project.channels[racks[0]].mixer_track,
            a.state.project.channels[racks[1]].mixer_track,
            "each rack has its own mixer strip"
        );
        let t0 = crate::track_links::instrument_track_of(&a.state, first).expect("first track");
        let t1 = crate::track_links::instrument_track_of(&a.state, second).expect("second track");
        assert_ne!(t0, t1);
        a.sync_containers();
        let lanes = lanes(&a.state, &a.plugins, racks[1]);
        set_step(&mut a.state, second, &lanes[0], 1, true);
        assert!(!step_on(&a.state, first, &lanes[0], 1), "the first rack's pattern row is untouched");
        assert_eq!(crate::track_links::link(&a.state, t0).instrument_channel(), Some(first));
        assert_eq!(crate::track_links::link(&a.state, t1).instrument_channel(), Some(second));
    }

    /// A saved project keeps the rack's identity and its members, and reopening it hosts its lanes again.
    #[test]
    fn a_rack_saves_and_reopens() {
        let file = std::env::temp_dir().join(format!("lemon-rack-{}.flp", std::process::id()));
        let path = wav("saved");
        let mut a = App::new(None);
        a.state.project = crate::app::test_project();
        a.plugins.available = crate::plugins::builtin();
        a.state.file = None;
        a.state.path = None;
        create(&mut a).expect("the rack");
        let row = a.state.project.channels.len() - 1;
        put_sample(&mut a, row, 0, &path);
        let ch = a.state.project.channels[row].clone();
        a.save_to(file.clone());
        let mut b = App::new(None);
        b.plugins.available = crate::plugins::builtin();
        b.open(file.clone());
        let back = b.state.project.channels.iter().find(|c| c.index == ch.index).expect("the rack channel");
        assert!(is_rack_channel(back));
        assert_eq!(back.plugin_state, ch.plugin_state, "its members are the file's state");
        b.sync_containers();
        let lanes = lanes(&b.state, &b.plugins, b.state.project.channels.iter().position(|c| c.index == ch.index).unwrap());
        assert!((36..=127).contains(&lanes[0].key), "its keys are kept");
        assert!(lanes[0].sample, "the lane's sample is back");
        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_file(path);
    }
}
