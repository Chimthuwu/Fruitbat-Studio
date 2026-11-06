// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! A note or clip as the engine holds it once scheduled: its trigger, swing and clip shape, and an instrument channel's tuning.


#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Trigger {
    /// Tick at which the note starts.
    pub(crate) tick: u64,
    /// Tick the note or clip itself starts at (`tick` moves where it starts late, `Engine::clips_under_way`).
    pub(crate) at: u64,
    pub(crate) channel: usize,
    /// Key rate; the voice plays at the channel's current base rate times this.
    pub(crate) rate: f64,
    /// Tick at which the voice is cut (end of an audio clip), if any.
    pub(crate) cut: Option<u64>,
    /// Velocity gain and note pan (âˆ’1..=1); the channel's volume and pan apply while it plays.
    pub(crate) vel: f64,
    pub(crate) pan: f64,
    /// Note key.
    pub(crate) key: u16,
    /// The note's fine pitch in semitones (instrument channels get it as the note's tuning).
    pub(crate) fine: f32,
    /// Tick at which the note is released; `None` = a step (length 0), which has no end of its own and is released
    /// with the song (spec: audio/song-structure.md, "A note").
    pub(crate) end: Option<u64>,
    /// Note velocity byte (instrument channels get it as 0..=1).
    pub(crate) velocity: u8,
    /// The note's own pan and mod X / Y as instruments get them.
    pub(crate) levels: crate::NoteLevels,
    /// Sample position (source frames) the voice starts at.
    pub(crate) from: f64,
    /// The same as a fraction of the sample (instrument channels: the sampler plugin starts there).
    pub(crate) start: f32,
    /// An audio clip's trimmed end as a fraction of the sample (0 = none; instrument channels: the sampler plugin
    /// ends the voice there).
    pub(crate) stop: f32,
    /// Which buffer of the channel it plays: 0 = the channel's sample, `k` = `keyed[k - 1]` (realtime stretch).
    pub(crate) buf: u16,
    /// An audio clip's fades, gain and late-start declick (`None` = plays as it is).
    pub(crate) shape: Option<ClipShape>,
    /// A slide note: no sound of its own, it glides the channel's voices of its colour.
    pub(crate) slide: bool,
    /// The note's colour group (0..=15).
    pub(crate) colour: u8,
    /// The note's own portamento switch (spec: audio/note-rules.md, "A note's own portamento switch").
    pub(crate) porta: bool,
    /// A note a layer handed on: the layer and the child's crossfade gain.
    pub(crate) via: Option<Via>,
    /// The group it cuts and the group its voice belongs to as the note carries them from a layer (`None`: the
    /// channel's own).
    pub(crate) cuts: Option<(u32, u32)>,
    /// A note the main swing can move (`None`: one it never moves).
    pub(crate) swing: Option<Swung>,
}

/// What a scheduled note or audio clip is known by when an edited project's engine takes over during playback
/// (`Engine::continue_from`): its channel (a layer's child: the layer), its key, and the tick it starts at before swing.
pub(crate) type Origin = (usize, u16, u64);

impl Trigger {
    pub(crate) fn origin(&self) -> Origin {
        (self.via.map_or(self.channel, |w| w.layer), self.key, self.swing.map_or(self.at, |s| s.base))
    }
}

/// What a note the main swing can move needs to be placed again when the swing changes while the song plays
/// (`Engine::set_swing`): where it starts without swing, its position in its pattern and its channel's share (spec:
/// audio/swing.md), the delay it has now, and how its end follows (`Swung::end`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Swung {
    pub(crate) base: u64,
    pub(crate) position: u32,
    pub(crate) share: u16,
    pub(crate) delay: u32,
    /// Its length without swing (0 = a step), whether "Truncate swing notes" keeps its end, and the tick its clip
    /// releases it at the latest.
    pub(crate) len: u64,
    pub(crate) keep_end: bool,
    pub(crate) cap: Option<u64>,
}

impl Swung {
    /// The note's end when it starts at `tick`, as `push_note` gives it: with "Truncate swing notes" where it ends
    /// without swing, else its length after `tick`; never past the clip's cap, at least one tick long.
    pub(crate) fn end(&self, tick: u64) -> Option<u64> {
        if self.len == 0 {
            return None;
        }
        let end = if self.keep_end { self.base + self.len } else { tick + self.len };
        Some(self.cap.map_or(end, |c| end.min(c)).max(tick + 1))
    }
}

/// The layer a voice's note came through, and the child's crossfade gain (1 without the crossfade) (spec:
/// audio/note-rules.md "Layer channels").
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Via {
    pub(crate) layer: usize,
    pub(crate) gain: f64,
}

/// What a layer adds to the levels of the voices its notes started (spec: audio/note-rules.md "Layer channels": the
/// final volume is multiplied by the layer's volume, its pan, pitch and mods add): its volume with the multiplier (0
/// when it is off), its pan with the offset, its pitch knob in cents, its mod X / mod Y.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LayerAdds {
    pub(crate) volume: f64,
    pub(crate) pan: f64,
    pub(crate) cents: f64,
    pub(crate) mods: (f32, f32),
}

/// An audio clip's fades, gain and start declick on its voice's clock (spec: audio/audio-clips.md "Fades and clip
/// gain", "Starting inside a clip").
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ClipShape {
    /// Frames of the clip already past when the voice starts (playback started inside the clip).
    pub(crate) since: u64,
    /// The clip's length in frames.
    pub(crate) total: u64,
    pub(crate) fade_in: u64,
    pub(crate) fade_out: u64,
    pub(crate) tension_in: f64,
    pub(crate) tension_out: f64,
    pub(crate) gain: f32,
    /// Frames of the short fade a late start begins with (0 = none).
    pub(crate) declick: u64,
}

impl ClipShape {
    /// The gain `age` frames after the voice started.
    pub(crate) fn at(&self, age: u64) -> f32 {
        let t = self.since + age;
        let mut g = f64::from(self.gain);
        if self.fade_in > 0 && t < self.fade_in {
            g *= os_model::fade_curve(t as f64 / self.fade_in as f64, self.tension_in);
        }
        if self.fade_out > 0 {
            let left = self.total.saturating_sub(t);
            if left < self.fade_out {
                g *= os_model::fade_curve(left as f64 / self.fade_out as f64, self.tension_out);
            }
        }
        if self.declick > 0 && age < self.declick {
            g *= os_model::fade_curve(age as f64 / self.declick as f64, 0.0);
        }
        g as f32
    }
}

/// Frames of the fade-in a late start inside an audio clip begins with (a Lemon Studio choice, spec:
/// audio/audio-clips.md "Starting inside a clip").
pub(crate) const LATE_START_SECONDS: f64 = 0.005;

/// What an instrument channel's notes were last tuned with: the channel's shift (semitones), and per key the fine pitch
/// of the note last started there, its glide and the key the instrument was given for it; `keys` marks the keys
/// started on it. A change of the shift (the main pitch, the pitch knob, automation) retunes those keys, so sounding
/// notes follow it (spec: audio/voice-levels.md "The main pitch"). `moves`: the instrument ignores a note's tuning
/// (`Instrument::takes_tuning`), so a plugin channel's notes start on keys moved by their pitch's whole semitones and
/// are tuned only by what is left; their note offs go to the moved keys.
#[derive(Debug, Clone, Copy)]
pub(crate) struct InstTuning {
    pub(crate) shift: f32,
    pub(crate) moves: bool,
    pub(crate) fine: [f32; 128],
    pub(crate) glide: [f32; 128],
    pub(crate) sent: [u8; 128],
    pub(crate) keys: u128,
}

impl Default for InstTuning {
    fn default() -> Self {
        InstTuning { shift: 0.0, moves: false, fine: [0.0; 128], glide: [0.0; 128], sent: std::array::from_fn(|k| k as u8), keys: 0 }
    }
}

impl InstTuning {
    /// The key the instrument was given for `key`'s latest note.
    pub(crate) fn sent(&self, key: u8) -> u8 {
        self.sent[usize::from(key & 127)]
    }

    /// The tuning (semitones) of `key`'s latest note: the shift, its fine pitch and glide, less what its key moved.
    pub(crate) fn tuning(&self, key: u8) -> f32 {
        let k = usize::from(key & 127);
        self.shift + self.fine[k] + self.glide[k] - (f32::from(self.sent[k]) - f32::from(key))
    }
}

/// An audio clip with a shape playing on an instrument channel: its voice, frames since the voice started, and the
/// gain the voice was last given (an instrument that takes voice gains; `None` before the first).
#[derive(Debug, Clone, Copy)]
pub(crate) struct InstClip {
    pub(crate) channel: usize,
    pub(crate) voice: u32,
    pub(crate) shape: ClipShape,
    pub(crate) age: u64,
    pub(crate) sent: Option<f32>,
}

/// An audio clip's fades (milliseconds, tension) and gain: the fades it plays (with automatic crossfades on, an edge
/// without its bit plays as the overlaps make it, `os_model::crossfade::playing`) and the gain the record stores.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ClipFx {
    pub(crate) fade_in: (f32, f32),
    pub(crate) fade_out: (f32, f32),
    pub(crate) gain: f32,
}

impl ClipFx {
    pub(crate) fn of(c: &os_model::Clip, (fade_in, fade_out): (Option<(f32, f32)>, Option<(f32, f32)>)) -> Option<ClipFx> {
        let fx = ClipFx { fade_in: fade_in.unwrap_or((0.0, 0.0)), fade_out: fade_out.unwrap_or((0.0, 0.0)), gain: c.gain() };
        (fx.fade_in.0 > 0.0 || fx.fade_out.0 > 0.0 || fx.gain != 1.0).then_some(fx)
    }
}
