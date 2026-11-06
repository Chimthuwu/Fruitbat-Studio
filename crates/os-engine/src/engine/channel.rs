// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! One channel's playback state: its sample, trims and stretch, its levels and glides, and the control points it is stepped through.

use std::sync::Arc;
use os_model::ControlPoint;
use crate::envelope;
use crate::instrument::Slot;
use crate::laws;
use crate::sample::Sample;
use super::{layers, notes};
use super::triggers::{LayerAdds, Via};

#[derive(Debug, Clone)]
pub(crate) struct ChannelState {
    /// The frames the channel's voices play (`None` for a sample that only `length` stands for).
    pub(crate) sample: Option<Arc<Sample>>,
    /// A channel its hosted instrument plays from the start (`Engine::with_instruments`): the length in frames and the
    /// rate of its sample in place of the decoded frames, which the engine never plays (`played`).
    pub(crate) length: Option<(usize, u32)>,
    /// Channel volume gain (0 when the channel is muted).
    pub(crate) volume: f64,
    /// Channel pan, −1..=1.
    pub(crate) pan: f64,
    /// The project's panning law (event 23), for the channel pan.
    pub(crate) pan_law: laws::PanLaw,
    pub(crate) insert: usize,
    /// Sample-rate conversion × channel pitch.
    pub(crate) base_rate: f64,
    /// Channel is on (a muted channel stays silent whatever a volume point says, and starts no notes).
    pub(crate) enabled: bool,
    /// The switch and the mixer track the channel stores, which automation changes while the song plays
    /// (`Engine::switch_channel`, `Engine::route_channel`) and a new position or a loop puts back first.
    pub(crate) stored: (bool, usize),
    /// Sample-rate conversion alone (`base_rate` without the channel pitch).
    pub(crate) src_ratio: f64,
    /// The pitch knob in cents as control points and automation leave it, and what the channel adds to it: fine tune,
    /// root note and (when the channel follows it) the main pitch (spec: audio/voice-levels.md "The channel's levels").
    /// `pitched` is false when the channel's pitch does not move its sample: a stretch time fits it, or a realtime
    /// stretch method, whose pitch is in `keyed` (spec: audio/time-stretch.md).
    pub(crate) knob: f64,
    pub(crate) init_knob: f64,
    /// A controller's pitch bend in cents on top of the knob, while its wheel is off centre (never saved; spec:
    /// ui/midi-settings.md §4).
    pub(crate) bend: f64,
    pub(crate) fine: i32,
    pub(crate) root: u8,
    pub(crate) main: Option<i32>,
    pub(crate) init_main: Option<i32>,
    pub(crate) pitched: bool,
    /// Values at the start of playback, restored when control points are replayed from the top.
    pub(crate) init: (f64, f64, f64),
    /// Volume parameter 0..=1 (before the volume law) at the start of the current span.
    pub(crate) vol_v: f64,
    pub(crate) init_v: f64,
    /// Count of glides started (`Ramp::id`, `Voice::serial`).
    pub(crate) serial: u32,
    /// A volume / pan control point in progress (the parameters glide, then the laws apply).
    pub(crate) ramp: Option<Ramp>,
    /// The channel's volume and pitch envelopes, when switched on (sampler channels).
    pub(crate) vol_env: Option<envelope::Params>,
    pub(crate) pitch_env: Option<envelope::Params>,
    /// Seconds per output frame.
    pub(crate) frame_s: f64,
    /// A generator plugin channel; it sounds only once an instrument is attached (`Engine::set_instrument`).
    pub(crate) plugin: bool,
    /// The channel plugin's Auto-bypass when silent (`os_model::PluginCpu::smart_disable`; `instrument_job`).
    pub(crate) smart: bool,
    /// A sampler channel: the instrument playing it (Lemon Sampler) applies the root note and fine tune itself, so its
    /// notes' tuning leaves them out (spec: plugins/lemon-sampler.md §7), and takes the rest as tuning (its keys are
    /// never moved, `InstTuning::moves`).
    pub(crate) own_tuning: bool,
    pub(crate) instrument: Slot,
    /// Polyphony, mono, portamento and cut group rules for the instrument's notes (spec: audio/channel-voices.md).
    pub(crate) rules: notes::Rules,
    /// Trim positions (source frames) times this are positions in what the channel plays: the stretch factor when
    /// that is a stretched copy.
    pub(crate) from_scale: f64,
    /// Realtime stretch methods: the stretch factor and cents, and the sample stretched for each key the channel's
    /// notes play.
    pub(crate) realtime: Option<(f64, f64)>,
    pub(crate) keyed: Vec<(u16, Arc<Sample>)>,
    /// An audio clip channel's own voice level, on what its hosted instrument plays (1 on other channels; spec:
    /// audio/audio-clips.md "Level").
    pub(crate) clip_level: f32,
    /// The level adjustments and filter knobs as automation leaves them, and as the channel stores them.
    pub(crate) adjust: Adjust,
    pub(crate) init_adjust: Adjust,
    /// A layer channel: its children and how it hands its notes on.
    pub(crate) layer: Option<layers::Layer>,
    /// The envelopes, LFOs and filter its hosted instrument gets (one that runs them on its voices).
    pub(crate) voicing: crate::instrument::ChannelVoicing,
    /// A hosted instrument's channel: the layer whose note it was given last. A hosted instrument hears that layer's
    /// levels on its channel's (a Lemon Studio choice: the host does not reach a plugin's voices one by one, so a
    /// child that also plays notes of its own, or of two layers, hears them all with the levels of the last one).
    pub(crate) via: Option<Via>,
}

/// A channel's level adjustments (event 229) and filter knob positions (spec: audio/voice-levels.md "The channel's
/// levels", "Values from automation").
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Adjust {
    /// Pan offset, -6400..6400.
    pub(crate) pan: i32,
    /// The volume multiplier as a gain: `curve(stored / 12800)`, 1 at the default 12800.
    pub(crate) mult: f32,
    /// Mod X and mod Y offsets, -256..256.
    pub(crate) mods: [i32; 2],
    /// Cutoff and resonance knobs (0..256) now and as stored: moving them changes the voices' mod X / mod Y by the
    /// difference.
    pub(crate) filter: [i32; 2],
    pub(crate) stored_filter: [i32; 2],
}

impl Adjust {
    /// The adjustments a channel stores (the model keeps them as knob positions: pan offset / 6400, (multiplier -
    /// 12800) / 12800, mod offsets / 256).
    pub(crate) fn of(ch: &os_model::Channel) -> Adjust {
        let l = ch.misc.levels;
        let at = |v: f32, scale: f32| (f64::from(v) * f64::from(scale)).round() as i32;
        let filter = [ch.sound.cutoff, ch.sound.resonance];
        Adjust {
            pan: at(l[0], 6400.0).clamp(-6400, 6400),
            mult: Adjust::multiplier((at(l[1], 12800.0) + 12800).clamp(0, 25600)),
            mods: [at(l[2], 256.0).clamp(-256, 256), at(l[3], 256.0).clamp(-256, 256)],
            filter,
            stored_filter: filter,
        }
    }

    /// The gain of a stored multiplier (0..25600): `curve(single(m) * single(1 / 12800))`.
    fn multiplier(m: i32) -> f32 {
        os_dsp::level::curve(os_dsp::level::knob(m, os_dsp::level::R12800))
    }

    /// The channel's mod X and mod Y: the offsets plus the filter knobs' moves, over 256.
    fn mods(&self) -> (f32, f32) {
        let m = |k: usize| (self.mods[k] + self.filter[k] - self.stored_filter[k]) as f32 / 256.0;
        (m(0), m(1))
    }
}

/// Glide of the channel's volume and pan parameters towards a control point's values.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Ramp {
    pub(crate) v0: f64,
    pub(crate) v1: f64,
    pub(crate) p0: f64,
    pub(crate) p1: f64,
    /// Frames each parameter takes (0 = already there).
    pub(crate) len_v: u32,
    pub(crate) len_p: u32,
    /// Frames done at the start of the current span.
    pub(crate) pos: u32,
    /// Voices born before this glide started follow it; later notes start at the final values.
    pub(crate) id: u32,
}

impl Ramp {
    /// How far the volume / pan parameter is after `frames` (0..=1).
    pub(crate) fn volume_at(&self, frames: f64) -> f64 {
        if self.len_v == 0 { 1.0 } else { (frames / f64::from(self.len_v)).min(1.0) }
    }
    pub(crate) fn pan_at(&self, frames: f64) -> f64 {
        if self.len_p == 0 { 1.0 } else { (frames / f64::from(self.len_p)).min(1.0) }
    }
}

/// Glide of a volume / pan control point on voices already sounding: the parameter moves linearly and the
/// law is applied to it. FL's volume glide lasts about 9.3 ms (shorter for tiny changes) and its pan glide
/// about 4.7 ms (measured; spec: audio/pattern-controls.md).
const VOLUME_GLIDE_SECONDS: f64 = 0.0093;
const PAN_GLIDE_SECONDS: f64 = 0.0047;

/// A pattern control point placed on its tick.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CtlEvent {
    pub(crate) tick: u64,
    pub(crate) to: CtlTo,
    /// In the setting's own unit (spec: audio/automation.md "Pattern control points").
    pub(crate) value: i32,
}

/// What a control point sets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum CtlTo {
    /// A channel's own setting: rack position, parameter (`ChannelState::apply`).
    Channel(usize, u16),
    /// A mixer value, a slot, the tempo.
    Target(os_model::target::Target),
    /// A plugin's parameter, with the level the value stands for once the plugin is known (`Engine::set_effect`,
    /// `Engine::set_instrument`; spec: plugins/plugin-parameters.md §1).
    Param(os_model::target::Target, Option<f64>),
}

/// Frames of a sample of `frames` frames stretched to `factor` times its length.
pub(crate) fn stretched_frames(frames: usize, factor: f64) -> usize {
    (frames as f64 * factor).round().max(1.0) as usize
}

impl ChannelState {
    /// The buffer a voice plays (`Trigger::buf`).
    pub(crate) fn buffer(&self, buf: u16) -> Option<&Arc<Sample>> {
        match buf {
            0 => self.sample.as_ref(),
            b => self.keyed.get(usize::from(b) - 1).map(|k| &k.1),
        }
    }

    /// The length in frames and the rate of what the channel plays: its sample, or the length that stands for it.
    pub(crate) fn played(&self) -> Option<(usize, u32)> {
        self.sample.as_ref().map(|s| (s.frames(), s.rate)).or(self.length)
    }

    /// Frames of the buffer `buf` (`buffer`; the sample's length for a channel that only has that).
    pub(crate) fn frames(&self, buf: u16) -> Option<usize> {
        match buf {
            0 => self.played().map(|p| p.0),
            b => self.buffer(b).map(|s| s.frames()),
        }
    }

    pub(crate) fn reset(&mut self) {
        (self.volume, self.pan, self.base_rate) = self.init;
        (self.enabled, self.insert) = self.stored;
        (self.knob, self.main) = (self.init_knob, self.init_main);
        self.vol_v = self.init_v;
        self.ramp = None;
        self.adjust = self.init_adjust;
    }

    /// The pan with the channel's pan offset: `single(pan * 6400 + offset) * single(1 / 6400)`, the stored integers
    /// added before they are scaled (spec: audio/voice-levels.md "The channel's levels"); the pan as it is without an
    /// offset.
    pub(crate) fn pan_of(&self, pan: f64) -> f64 {
        if self.adjust.pan == 0 {
            return pan;
        }
        f64::from((pan * 6400.0 + f64::from(self.adjust.pan)) as f32 * os_dsp::level::R6400)
    }

    /// What this channel adds as a layer to its children's voices. The pitch is the pitch knob alone (a Lemon Studio
    /// choice: the spec compares a layer's pitch knob only, with its fine tune, root note and the main pitch at their
    /// defaults).
    pub(crate) fn layer_adds(&self) -> LayerAdds {
        LayerAdds { volume: self.volume, pan: self.pan_of(self.pan), cents: self.knob, mods: self.adjust.mods() }
    }

    /// The gain of a note that starts now (`start_gain`) of a voice whose note came through a layer.
    pub(crate) fn start_gain_via(&self, vel: f64, note_pan: f64, layer: Option<(LayerAdds, f64)>) -> (f32, f32) {
        match layer {
            None => self.start_gain(vel, note_pan),
            Some((a, gain)) => self.start_gain(vel * a.volume * gain, note_pan + a.pan),
        }
    }

    /// The channel's levels for a hosted instrument.
    ///
    /// A control point or automation value is handed on whole from its tick: the instrument takes it into its voices'
    /// gain ramp, one limited ramp per tick (spec: audio/sampler.md "Gains"); the engine's glide is its own voices'
    /// and its output gain's.
    pub(crate) fn levels(&self) -> crate::instrument::ChannelLevels {
        let (mod_x, mod_y) = self.adjust.mods();
        let (v, p) = self.ramp.map_or((self.vol_v, self.pan), |r| (r.v1, r.p1));
        crate::instrument::ChannelLevels { volume: self.vol_gain(v) as f32, pan: self.pan_of(p) as f32, mod_x, mod_y }
    }

    /// A value for one of the channel's filter knobs or level adjustments from an automation clip (`v` 0..1, spec:
    /// audio/voice-levels.md "Values from automation"); sounding voices follow.
    pub(crate) fn automate_level(&mut self, id: u16, v: f64) {
        use os_model::target;
        let round = |x: f64| x.round_ties_even() as i32;
        let a = &mut self.adjust;
        match id {
            target::CHANNEL_CUTOFF => a.filter[0] = round(256.0 * v),
            target::CHANNEL_RESONANCE => a.filter[1] = round(256.0 * v),
            target::CHANNEL_PAN_OFFSET => a.pan = round(12800.0 * v) - 6400,
            target::CHANNEL_VOLUME_MULT => {
                a.mult = Adjust::multiplier(round(25600.0 * v));
                self.volume = self.vol_gain(self.vol_v);
            }
            target::CHANNEL_MOD_X => a.mods[0] = round(512.0 * v) - 256,
            target::CHANNEL_MOD_Y => a.mods[1] = round(512.0 * v) - 256,
            _ => {}
        }
    }

    /// Gain of a note that starts now: a glide in progress is already at its final values for it.
    pub(crate) fn start_gain(&self, vel: f64, note_pan: f64) -> (f32, f32) {
        match self.ramp {
            None => self.gain(vel, note_pan),
            Some(r) => {
                let g = self.vol_gain(r.v1) * vel;
                let (pl, pr) = laws::channel_pan_law(self.pan_of(r.p1) + note_pan, self.pan_law);
                ((g * pl) as f32, (g * pr) as f32)
            }
        }
    }

    /// The channel's volume gain for volume parameter `v`: its level curve times the volume multiplier (spec:
    /// audio/voice-levels.md "Final levels": the channel's two first), 0 when the channel is off.
    pub(crate) fn vol_gain(&self, v: f64) -> f64 {
        if self.enabled { laws::channel_gain(v) * f64::from(self.adjust.mult) } else { 0.0 }
    }

    /// Moves the span-start values on by `frames` of a running glide.
    pub(crate) fn advance(&mut self, frames: u32) {
        let Some(r) = self.ramp.as_mut() else { return };
        r.pos = r.pos.saturating_add(frames);
        let pos = f64::from(r.pos);
        self.vol_v = r.v0 + (r.v1 - r.v0) * r.volume_at(pos);
        self.pan = r.p0 + (r.p1 - r.p0) * r.pan_at(pos);
        if r.pos >= r.len_v && r.pos >= r.len_p {
            (self.vol_v, self.pan) = (r.v1, r.p1);
            self.ramp = None;
        }
        self.volume = self.vol_gain(self.vol_v);
    }

    /// Starts gliding the volume and / or pan parameter to a new value over `len` frames (0 = at once).
    pub(crate) fn glide(&mut self, v1: Option<f64>, p1: Option<f64>, len: u32) {
        // A parameter that is not being set carries on towards its old target in the time it had left.
        let (tv, tp, rest_v, rest_p) = self.ramp.map_or((self.vol_v, self.pan, 0, 0), |r| {
            (r.v1, r.p1, r.len_v.saturating_sub(r.pos), r.len_p.saturating_sub(r.pos))
        });
        let (nv, np) = (v1.unwrap_or(tv), p1.unwrap_or(tp));
        let len_v = if v1.is_some() { len } else { rest_v };
        let len_p = if p1.is_some() { len } else { rest_p };
        if (nv, np) == (self.vol_v, self.pan) && self.ramp.is_none() {
            return;
        }
        if len_v == 0 && len_p == 0 {
            self.ramp = None;
            (self.vol_v, self.pan) = (nv, np);
            self.volume = self.vol_gain(nv);
        } else {
            self.serial += 1;
            self.ramp = Some(Ramp { v0: self.vol_v, v1: nv, p0: self.pan, p1: np, len_v, len_p, pos: 0, id: self.serial });
        }
    }

    /// Sets the pitch knob (cents) and with it the channel's rate: the knob, fine tune, root note and main pitch
    /// (spec: audio/voice-levels.md), counted here from key 60 as the voices' key rates are.
    pub(crate) fn retune(&mut self, knob: f64) {
        self.knob = knob;
        self.base_rate = self.src_ratio * if self.pitched { laws::cents_rate(self.pitch_from_60()) } else { 1.0 };
    }

    /// The channel's pitch in cents from key 60.
    pub(crate) fn pitch_from_60(&self) -> f64 {
        f64::from(laws::channel_pitch(self.knob, self.fine, self.root, self.main)) + self.bend + 6000.0
    }

    /// The tuning (semitones) a hosted instrument hears with each note: the channel's pitch, without the root note and
    /// fine tune when the instrument applies those itself.
    pub(crate) fn hosted_shift(&self) -> f32 {
        let shift = if self.own_tuning {
            if self.pitched { (f64::from(laws::channel_pitch(self.knob, 0, 60, self.main)) + 6000.0) / 100.0 } else { 0.0 }
        } else {
            12.0 * (self.base_rate / self.src_ratio).log2()
        };
        let shift = shift as f32;
        if shift.is_finite() && shift.abs() > 1e-3 { shift } else { 0.0 }
    }

    /// A control point replaces the channel's own setting (spec: audio/pattern-controls.md).
    pub(crate) fn apply(&mut self, param: u16, value: i32, rate: u32) {
        match param {
            ControlPoint::VOLUME => self.glide(Some((f64::from(value) / 12800.0).clamp(0.0, 1.0)), None, (f64::from(rate) * VOLUME_GLIDE_SECONDS).round() as u32),
            ControlPoint::PAN => self.glide(None, Some((f64::from(value) / 6400.0 - 1.0).clamp(-1.0, 1.0)), (f64::from(rate) * PAN_GLIDE_SECONDS).round() as u32),
            ControlPoint::PITCH => self.retune(f64::from(value)),
            // The filter knobs' positions 0..256 (spec: audio/voice-levels.md "Values from automation").
            os_model::target::CHANNEL_CUTOFF => self.adjust.filter[0] = value.clamp(0, 256),
            os_model::target::CHANNEL_RESONANCE => self.adjust.filter[1] = value.clamp(0, 256),
            _ => {}
        }
    }

    /// `gain` for a voice whose note came through a layer.
    pub(crate) fn gain_via(&self, vel: f64, note_pan: f64, layer: Option<(LayerAdds, f64)>) -> (f32, f32) {
        match layer {
            None => self.gain(vel, note_pan),
            Some((a, gain)) => self.gain(vel * a.volume * gain, note_pan + a.pan),
        }
    }

    /// `gain_via` of no note pan where the channel will be `frames` from now, a running glide that far along.
    pub(crate) fn gain_after(&self, frames: u32, vel: f64, layer: Option<(LayerAdds, f64)>) -> (f32, f32) {
        let Some(r) = self.ramp else { return self.gain_via(vel, 0.0, layer) };
        let (lv, lp) = layer.map_or((1.0, 0.0), |(a, g)| (a.volume * g, a.pan));
        let at = f64::from(r.pos.saturating_add(frames));
        let g = self.vol_gain(r.v0 + (r.v1 - r.v0) * r.volume_at(at)) * vel * lv;
        let (pl, pr) = laws::channel_pan_law(self.pan_of(r.p0 + (r.p1 - r.p0) * r.pan_at(at)) + lp, self.pan_law);
        ((g * pl) as f32, (g * pr) as f32)
    }

    /// Gain of a note on this channel: channel volume × velocity × pan.
    // spec: audio/sampler-playback.md (velocity multiplies volume; pans add, clamped)
    pub(crate) fn gain(&self, vel: f64, note_pan: f64) -> (f32, f32) {
        let g = self.volume * vel;
        let (pl, pr) = laws::channel_pan_law(self.pan_of(self.pan) + note_pan, self.pan_law);
        ((g * pl) as f32, (g * pr) as f32)
    }
}
