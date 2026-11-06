// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The song's tick line: what is scheduled on it (notes, automations), swing's delay, and the schedule built from a project.

use std::sync::Arc;
use os_model::target::Target;
use os_model::{ChannelKind, ClipSource, Project};
use crate::sample::Sample;
use super::EngineReport;
use super::channel::{CtlEvent, CtlTo};
use super::transport::{PlayMode};
use super::triggers::{ClipFx, Swung};

/// An automation clip placed on the tick line (spec: ui/playlist.md Â§8, flp/events/automation.md).
#[derive(Debug, Clone)]
pub(crate) struct AutoClip {
    /// Ticks at which the clip starts and ends.
    pub(crate) start: u64,
    pub(crate) end: u64,
    /// Beats into the automation at the clip start (its window).
    pub(crate) from_beat: f64,
    /// Index into `Engine::curves`.
    pub(crate) curve: usize,
    /// Every target the clip drives (one link each, spec: audio/automation.md "The link"), unknown ones left out.
    pub(crate) targets: Vec<os_model::target::Target>,
    /// The walk along the curve and the tick steps it has taken (spec: audio/automation.md "Walking the curve").
    pub(crate) walk: Option<(os_model::curve::Walk, u64)>,
    /// The clip's range (its channel's two level knobs) and its LFO (spec: audio/automation.md "Range", "The LFO").
    pub(crate) out: os_model::clip_lfo::ClipOut,
    /// The walk along the LFO's own envelope.
    pub(crate) env: os_model::clip_lfo::EnvWalk,
}
/// A scheduled note on the song's tick line.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Sched {
    pub(crate) tick: u64,
    pub(crate) channel: usize,
    pub(crate) key: u16,
    /// Note fine pitch byte, 120 = none.
    pub(crate) fine: u8,
    pub(crate) velocity: u8,
    /// Note pan byte, 64 = center.
    pub(crate) pan: u8,
    /// Note mod X / mod Y bytes, 128 = none.
    pub(crate) mod_x: u8,
    pub(crate) mod_y: u8,
    /// Tick at which the voice is cut (audio clip end).
    pub(crate) cut: Option<u64>,
    /// Note length in ticks (0 = a step: it has no end of its own).
    pub(crate) len: u64,
    /// Seconds into the sample where the voice starts (audio clip window start).
    pub(crate) from_s: f64,
    /// Seconds into the sample where an audio clip's window ends (`None`: the sample's end).
    pub(crate) to_s: Option<f64>,
    /// An audio clip's fades and gain.
    pub(crate) clip: Option<ClipFx>,
    /// A slide note.
    pub(crate) slide: bool,
    /// The note's colour group.
    pub(crate) colour: u8,
    /// The note's own portamento switch.
    pub(crate) porta: bool,
    /// A note the main swing can move.
    pub(crate) swing: Option<Swung>,
}

impl Sched {
    fn note(tick: u64, n: &os_model::Note) -> Self {
        Sched {
            tick,
            channel: usize::from(n.channel),
            key: n.key,
            fine: n.fine_pitch,
            velocity: n.velocity,
            pan: n.pan,
            mod_x: n.mod_x,
            mod_y: n.mod_y,
            cut: None,
            len: u64::from(n.length),
            from_s: 0.0,
            to_s: None,
            clip: None,
            slide: n.slide(),
            colour: n.colour(),
            porta: n.porta(),
            swing: None,
        }
    }
}

/// Swing delay of a note in whole ticks (spec: audio/swing.md). With a sixteenth `step = PPQ / 4` and `half = step / 2`,
/// only notes in the first half of an odd-numbered step (counted from 0 at the pattern start) move, by
/// `Trunc(swing * share / (128 * 128) * (half - offset))` ticks: a note on the step moves half a step at full swing,
/// and at full swing every note of that half lands on the step's middle. `share` is the channel's share (0..=128).
pub fn swing_delay(position: u32, ppq: u16, swing: u8, share: u16) -> u32 {
    let step = u32::from(ppq) / 4;
    if swing == 0 || step == 0 {
        return 0;
    }
    let half = f64::from(step) / 2.0;
    let (index, offset) = (position / step, position % step);
    if index % 2 == 0 || f64::from(offset) >= half {
        return 0;
    }
    ((f64::from(swing) * f64::from(share)) / (128.0 * 128.0) * (half - f64::from(offset))).trunc() as u32
}

/// The channel's share of the swing as stored (0..=128) from the Misc tab value.
fn swing_share(project: &Project, channel: u16) -> u16 {
    project.channels.get(usize::from(channel)).map_or(128, |c| (c.misc.swing.clamp(0.0, 1.0) * 128.0).round() as u16)
}

/// Schedules a note whose pattern starts at tick `start` (may be negative: a clip that begins inside its pattern).
/// With `swing` the note's start moves by its position inside the pattern; `cap` is the tick at which a note with a
/// length is released at the latest (a clip's end with "play truncated notes"; steps keep their own release). A
/// repeating note is retriggered at every multiple of its interval inside its length (spec:
/// audio/pattern-controls.md, note repeat); a plain step counts as one step long for that. A note the swing can move
/// keeps what `Engine::set_swing` needs to move it again ([`Swung`]); its repeats keep their number.
pub(crate) fn push_note(out: &mut Vec<Sched>, start: i64, n: &os_model::Note, project: &Project, swing: bool, cap: Option<u64>) {
    let ppq = project.ppq.max(1);
    let share = swing_share(project, n.channel);
    let delay = if swing { swing_delay(n.position, ppq, project.swing.unwrap_or(0), share) } else { 0 };
    let first = start + i64::from(n.position) + i64::from(delay);
    if first < 0 {
        return;
    }
    let mut first = Sched::note(first as u64, n);
    // Truncate swing notes: a swung note with a length keeps its end (spec: audio/swing.md, Lemon Studio's rule).
    let keep_end = n.length > 0 && project.channels.get(usize::from(n.channel)).is_some_and(|c| c.misc.truncate_swing);
    if delay > 0 && keep_end {
        first.len = first.len.saturating_sub(u64::from(delay)).max(1);
    }
    // Swing can move it: in the first half of an odd step, on a channel with a share.
    let movable = swing && swing_delay(n.position, ppq, 128, share) > 0;
    let swung = |off: u64| {
        movable.then(|| Swung {
            base: first.tick - u64::from(delay) + off,
            position: n.position,
            share,
            delay,
            len: u64::from(n.length).saturating_sub(off),
            keep_end,
            cap,
        })
    };
    let span = if n.length > 0 { first.len } else { u64::from(ppq / 4).max(1) };
    let capped = |s: Sched| match cap {
        Some(c) if s.len > 0 => Sched { len: s.len.min(c.saturating_sub(s.tick)), ..s },
        _ => s,
    };
    match n.repeat_ticks(ppq) {
        Some(interval) => {
            let mut off = 0;
            while off < span {
                let len = if n.length > 0 { first.len - off } else { 0 };
                let s = capped(Sched { tick: first.tick + off, len, swing: swung(off), ..first });
                if n.length == 0 || s.len > 0 {
                    out.push(s);
                }
                off += u64::from(interval.max(1));
            }
        }
        None => {
            let s = capped(Sched { swing: swung(0), ..first });
            if n.length == 0 || s.len > 0 {
                out.push(s);
            }
        }
    }
}

/// Where a pattern clip plays (spec: audio/song-structure.md, "Song mode"): its pattern time `[from, from + len)`, the
/// tick its pattern time starts at, its own end on the song, and the tick from which it starts no more notes (the
/// next clip of the same pattern taking the pattern over, or its end).
#[derive(Debug, Clone, Copy)]
struct PatternPlay {
    pub(crate) pattern: u16,
    pub(crate) from: u64,
    pub(crate) len: u64,
    pub(crate) origin: i64,
    pub(crate) end: u64,
    pub(crate) starts_until: u64,
}

/// The pattern clips that play, with their windows. A clip whose start offset is negative (uncut) plays the whole
/// pattern, whatever length its record stores. A pattern has one play position: a later clip of the same pattern
/// takes it over from its start, and of two clips that start on the same tick only the later one in the file plays.
/// Muted clips and clips on a switched-off track play nothing and take nothing over.
fn pattern_plays(project: &Project, clips: &[os_model::Clip]) -> Vec<PatternPlay> {
    let mut list: Vec<(usize, &os_model::Clip, u16)> = clips
        .iter()
        .enumerate()
        .filter(|(_, c)| clip_audible(project, c))
        .filter_map(|(i, c)| match c.source {
            ClipSource::Pattern(n) => Some((i, c, n)),
            _ => None,
        })
        .collect();
    list.sort_by_key(|&(i, c, n)| (n, c.position, i));
    let mut out = Vec::new();
    for (k, &(_, c, n)) in list.iter().enumerate() {
        let later = &list[k + 1..];
        if later.first().is_some_and(|&(_, d, m)| m == n && d.position == c.position) {
            continue;
        }
        let (from, len) = match c.window {
            Some(w) => (w.0.max(0.0).round() as u64, u64::from(c.length)),
            None => (0, pattern_length(project, n)),
        };
        let start = u64::from(c.position);
        let end = start + len;
        let takeover = later.iter().take_while(|&&(_, _, m)| m == n).map(|&(_, d, _)| u64::from(d.position)).find(|&p| p > start);
        out.push(PatternPlay { pattern: n, from, len, origin: i64::from(c.position) - from as i64, end, starts_until: end.min(takeover.unwrap_or(u64::MAX)) });
    }
    out
}

/// Control points as (tick, target word, value): a pattern's own, or those of every pattern clip of the playlist
/// (spec: audio/pattern-controls.md, audio/song-structure.md "Control points").
pub(crate) fn schedule_controls(project: &Project, mode: PlayMode) -> Vec<(u64, u32, i32)> {
    let points = |n: u16| project.patterns.iter().find(|p| p.number == n).map(|p| p.controls.as_slice()).unwrap_or(&[]);
    let place = |out: &mut Vec<_>, n: u16| {
        for c in points(n) {
            out.push((u64::from(c.position), c.kind, c.value));
        }
    };
    let mut out = Vec::new();
    match mode {
        PlayMode::Pattern(n) => place(&mut out, n),
        PlayMode::Song => {
            let clips = project.arrangement().map(|a| a.clips.as_slice()).unwrap_or(&[]);
            if clips.is_empty() {
                place(&mut out, project.current_pattern.unwrap_or(1));
            }
            // Points inside a clip's window act, placed with the clip like notes.
            for play in pattern_plays(project, clips) {
                for p in points(play.pattern).iter().filter(|p| u64::from(p.position) >= play.from && u64::from(p.position) < play.from + play.len) {
                    let tick = play.origin + i64::from(p.position);
                    if tick >= 0 && (tick as u64) < play.starts_until {
                        out.push((tick as u64, p.kind, p.value));
                    }
                }
            }
        }
    }
    out
}

/// What a control point's target word sets: a channel's own setting below the project's place, else the target an
/// automation link with that word names (spec: audio/automation.md "Pattern control points"). A channel's switch and
/// mixer track are set as a clip sets them (`point_level`).
pub(crate) fn ctl_to(kind: u32) -> CtlTo {
    use os_model::target::{self, CHANNEL_MIXER_TRACK, CHANNEL_ON, PLUGIN_PARAM};
    let (place, id) = ((kind >> 16) as u16, kind as u16);
    if place < 0x4000 && id < PLUGIN_PARAM && id != CHANNEL_ON && id != CHANNEL_MIXER_TRACK {
        return CtlTo::Channel(usize::from(place), id);
    }
    match target::decode(kind) {
        t @ (Target::ChannelParam(..) | Target::SlotParam(..)) => CtlTo::Param(t, None),
        t => CtlTo::Target(t),
    }
}

/// The automation level (0..1) a point's value in its setting's own unit stands for: the inverse of what a clip's
/// level becomes (`Engine::auto_value`; spec: audio/automation.md "Pattern control points"). `None` for the targets a
/// point does not move (a send's unit is not measured).
pub(crate) fn point_level(t: Target, value: i32) -> Option<f64> {
    let v = f64::from(value);
    Some(match t {
        Target::MixerVolume(_) => v / 16000.0,
        Target::MixerPan(_) => (v / 6400.0 + 1.0) / 2.0,
        Target::MixerSeparation(_) => (v / 64.0 + 1.0) / 2.0,
        Target::MixerEq(_, _, 0) => (v / 1800.0 + 1.0) / 2.0,
        Target::MixerEq(..) => v / 65536.0,
        Target::SlotOn(..) => v,
        Target::SlotMix(..) => v / 12800.0,
        // The channel rack's controls in their own units: the switch 0 / 1, the track number, the main swing 0..128.
        Target::ChannelOn(_) => v.clamp(0.0, 1.0),
        Target::ChannelMixerTrack(_) => os_model::target::mixer_track_level(value.clamp(0, 500) as u16),
        Target::MainSwing => v.clamp(0.0, 128.0) / 128.0,
        // The main pitch in cents (spec: audio/voice-levels.md "The main pitch").
        Target::MainPitch => (v.clamp(-1200.0, 1200.0) + 1200.0) / 2400.0,
        _ => return None,
    })
}

/// Gives the plugin-parameter points `mine` picks their level, from the parameter's range as the plugin states it
/// (`os_model::target::plugin_point_level`).
pub(crate) fn param_points(controls: &mut [CtlEvent], mine: impl Fn(Target) -> bool, range: impl Fn(usize) -> Option<(f64, f64)>) {
    for e in controls {
        if let CtlTo::Param(t, level) = &mut e.to {
            let (Target::ChannelParam(_, k) | Target::SlotParam(_, _, k)) = *t else { continue };
            if mine(*t) {
                *level = range(usize::from(k)).map(|(min, max)| os_model::target::plugin_point_level(min, max, e.value));
            }
        }
    }
}

/// A clip plays unless it is muted or its playlist track is (spec: ui/playlist.md §2, §6).
pub(crate) fn clip_audible(project: &Project, c: &os_model::Clip) -> bool {
    let track_on = project.arrangement().and_then(|a| a.tracks.get(usize::from(c.track))).is_none_or(|t| t.enabled);
    !c.muted && track_on
}

/// A plucked tone at key 60 (261.6 Hz), about 1.5 s: decaying harmonics of a saw.
pub(crate) fn fallback_tone(rate: u32) -> Arc<Sample> {
    use std::f64::consts::TAU;
    let n = (f64::from(rate) * 1.5) as usize;
    let f0 = 261.625_565;
    let mut left = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f64 / f64::from(rate);
        let attack = (t / 0.004).min(1.0);
        let mut v = 0.0;
        for h in 1..=10 {
            let hf = f64::from(h);
            if f0 * hf > f64::from(rate) / 2.0 {
                break;
            }
            v += (TAU * f0 * hf * t).sin() / hf * (-t * (2.5 + 2.0 * hf)).exp();
        }
        left.push((0.45 * attack * v) as f32);
    }
    Arc::new(Sample { rate, right: left.clone(), left })
}

/// A pattern's length in ticks, by the rule the app shares ([`Project::pattern_length`]): an explicit length wins;
/// otherwise its last note's end rounded up to a bar line of the pattern's own time signature, at least one bar (spec:
/// audio/song-structure.md, "Time signatures and a pattern's length"; audio/timing.md, "Where the song ends").
fn pattern_length(project: &Project, number: u16) -> u64 {
    u64::from(project.pattern_length(number))
}

/// Note triggers plus the song's end tick: in pattern mode the pattern's length, in song mode the largest end of a
/// clip that is not muted (clips on a switched-off playlist track count; spec: audio/timing.md, audio/song-structure.md).
pub(crate) fn schedule(project: &Project, mode: PlayMode, report: &mut EngineReport) -> (Vec<Sched>, u64) {
    let pattern_notes = |n: u16| project.patterns.iter().find(|p| p.number == n).map(|p| p.notes.as_slice()).unwrap_or(&[]);
    let one_pattern = |n: u16| {
        // Muted notes are silent (spec: ui/piano-roll.md Â§9); notes past an explicit length are not played.
        let end = pattern_length(project, n);
        let mut t = Vec::new();
        for x in pattern_notes(n).iter().filter(|x| !x.muted() && u64::from(x.position) < end) {
            push_note(&mut t, 0, x, project, true, None);
        }
        (t, end)
    };
    match mode {
        PlayMode::Pattern(n) => one_pattern(n),
        PlayMode::Song => {
            let clips = project.arrangement().map(|a| a.clips.as_slice()).unwrap_or(&[]);
            if clips.is_empty() {
                return one_pattern(project.current_pattern.unwrap_or(1));
            }
            let mut out = Vec::new();
            let mut end: u64 = 0;
            let truncated = project.play_truncated;
            for play in pattern_plays(project, clips) {
                for note in pattern_notes(play.pattern).iter().filter(|x| !x.muted()) {
                    let p = u64::from(note.position);
                    let cap = truncated.then_some(play.end);
                    if p >= play.from && p < play.from + play.len {
                        // Starts inside the window: from its place in the clip, unless the pattern was taken over.
                        if play.origin + (p as i64) < play.starts_until as i64 {
                            push_note(&mut out, play.origin, note, project, true, cap);
                        }
                    } else if truncated && p < play.from && note.length > 0 && p + u64::from(note.length) > play.from {
                        // Began before the window and reaches into it: starts at the clip's position, for the part
                        // inside the window.
                        let inside = (p + u64::from(note.length)).min(play.from + play.len) - play.from;
                        let cut = os_model::Note { position: play.from as u32, length: inside as u32, ..*note };
                        if play.origin + (play.from as i64) < play.starts_until as i64 {
                            push_note(&mut out, play.origin, &cut, project, false, cap);
                        }
                    }
                }
            }
            // spec: audio/audio-clips.md "Automatic crossfades"
            let fades = os_model::crossfade::playing(project, clips, project.tempo.unwrap_or(140.0));
            for (c, &fx) in clips.iter().zip(&fades) {
                let start = u64::from(c.position);
                let len = u64::from(c.length);
                if !c.muted {
                    end = end.max(start + len);
                }
                // Muted clips and clips on muted tracks are silent (spec: ui/playlist.md Â§2, Â§6).
                if !clip_audible(project, c) {
                    continue;
                }
                match c.source {
                    // Pattern clips were placed above.
                    ClipSource::Pattern(_) => {}
                    ClipSource::Channel(ch) => match project.channels.get(usize::from(ch)).map(|c| c.kind) {
                        // An audio clip plays its channel's sample from the window start (milliseconds) to the
                        // window end, either one negative meaning the whole sample (spec: audio/audio-clips.md
                        // "Settings"), cut at the clip end where the instrument does not end it there.
                        Some(ChannelKind::AudioClip | ChannelKind::Sampler) => {
                            let from_s = c.window.map_or(0.0, |w| w.0.max(0.0) / 1000.0);
                            let to_s = c.window.filter(|w| w.0 >= 0.0 && w.1 >= 0.0).map(|w| w.1 / 1000.0);
                            out.push(Sched { tick: start, channel: usize::from(ch), key: 60, fine: 120, velocity: 100, pan: 64, mod_x: 128, mod_y: 128, cut: Some(start + len), len, from_s, to_s, clip: ClipFx::of(c, fx), slide: false, colour: 0, porta: false, swing: None });
                        }
                        Some(ChannelKind::Automation) => {}
                        _ => report.automation_clips_ignored += 1,
                    },
                }
            }
            (out, end)
        }
    }
}
