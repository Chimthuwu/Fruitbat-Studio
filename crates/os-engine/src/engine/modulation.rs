// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The modulation graph (M1b), evaluated on the engine's clock.
//!
//! Each [`Modulator`] carries one source from [`os_model::modulation`] and the parameters it drives, already resolved
//! to the engine's [`Target`] by the app ([`crate::Engine::set_modulation`]); the graph itself never reaches the
//! engine, and no FLP `Target` encoding changes (audit K.1). Values are sent through the same parameter-event path
//! automation uses, sampled every [`PARAM_STEP`] frames, so a hosted plugin's own smoothing does the rest.
//!
//! **Where a value moves from.** A modulation does not own a parameter's value: it moves the value the parameter
//! already has. That base is the last value a clip (or a pattern control point) handed that parameter, or
//! [`DEFAULT_BASE`] halfway when nothing has. So a modulated knob still follows its automation, and a clip and a
//! modulator on one parameter compose instead of fighting. Several modulators on one parameter add up.
//!
//! **What is driven.** Plugin parameters — a channel's instrument ([`Target::ChannelParam`]) and a mixer slot's hosted
//! effect ([`Target::SlotParam`]) — through the parameter-event path, and a track's own mixer values (fader, pan,
//! separation, equaliser, sends, a slot's mix level) through [`Engine::run_host_modulation`], each in the units the
//! mixer reads it in. None of them loses its authored value: a modulation moves the base, so it never builds on itself
//! and gives the value back when it goes.
//!
//! **Time.** A source follows the transport: `Sync` periods count the project's own sixteenths, `Free` ones seconds.
//! The random LFO is a hash of its cycle number, so a render equals live playback and a seek lands on the same value.
//! An envelope is a one-shot from the transport's start for now (a note-triggered one needs the note path).

use os_model::modulation::{Curve, Envelope, Lfo, LfoShape, ModSource, Polarity, Steps, Timebase};
use os_model::target::Target;

use super::param_auto::{ParamAuto, PARAM_STEP};
use super::Engine;

/// Where a parameter sits when nothing has set it: halfway.
pub const DEFAULT_BASE: f64 = 0.5;

/// One parameter a modulator drives, resolved to an engine [`Target`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModTarget {
    pub target: Target,
    /// `-1..=1` of the parameter's range; negative inverts the source.
    pub depth: f32,
    pub curve: Curve,
    pub polarity: Polarity,
}

/// A modulator as the engine runs it.
#[derive(Debug, Clone, PartialEq)]
pub struct Modulator {
    /// The graph's own id (the number a `.lemon` file holds): the random shape's value depends on it, so two random
    /// modulators at the same rate do not move together.
    pub id: u32,
    pub source: ModSource,
    pub enabled: bool,
    pub targets: Vec<ModTarget>,
}

/// The graph in the engine, with the last value it sent per parameter and the base it moved from then: a value is
/// sent again when either changes (a base that moves under a value held at a limit still has to be re-sent, or the
/// clip's own value would leave the parameter where the modulation had lifted it from).
#[derive(Debug, Default)]
pub(super) struct Modulation {
    modulators: Vec<Modulator>,
    last: Vec<(Target, f64, f64)>,
}

impl Modulation {
    /// Replaces the graph; every value is sent again from scratch, as after a seek.
    pub fn set(&mut self, modulators: Vec<Modulator>) {
        self.modulators = modulators;
        self.last.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.modulators.is_empty()
    }
}

/// The source's value at a position, `0..=1`: `beats` from the transport's start (quarter notes) and `secs`.
pub fn source_value(source: &ModSource, id: u32, beats: f64, secs: f64) -> f32 {
    match source {
        ModSource::Lfo(l) => lfo_value(l, id, cycles(l.period, beats, secs) + f64::from(l.phase)),
        ModSource::Steps(s) => steps_value(s, cycles(s.period, beats, secs)),
        ModSource::Envelope(e) => envelope_value(e, secs),
    }
}

/// How many periods (an LFO's cycles, a step sequence's steps) have gone by at a position.
fn cycles(period: Timebase, beats: f64, secs: f64) -> f64 {
    match period {
        Timebase::Free(hz) => secs * f64::from(hz.max(0.0)),
        // A period in sixteenths: four of them to a beat.
        Timebase::Sync(sixteenths) => beats * 4.0 / f64::from(sixteenths.max(1)),
    }
}

/// An LFO's value at `cycles`: the whole part is the cycle, the fraction the position in it. `0..=1`.
fn lfo_value(l: &Lfo, id: u32, cycles: f64) -> f32 {
    let p = cycles.rem_euclid(1.0);
    let wave = match l.shape {
        LfoShape::Sine => (p * std::f64::consts::TAU).sin(),
        LfoShape::Triangle => 1.0 - 4.0 * (p - 0.5).abs(),
        LfoShape::SawUp => 2.0 * p - 1.0,
        LfoShape::SawDown => 1.0 - 2.0 * p,
        LfoShape::Square => {
            if p < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        LfoShape::Random => random(cycles.floor() as i64, id),
    };
    ((wave + 1.0) / 2.0) as f32
}

/// A deterministic value in `-1..1` for `cycle`, from modulator `id`: a random source repeats identically on every
/// render and every seek, so an export equals live playback.
fn random(cycle: i64, id: u32) -> f64 {
    let mut x = (cycle as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ u64::from(id).wrapping_mul(0xD1B5_4A32_D192_ED03);
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    (x >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
}

/// A step sequence's value at `cycles`, `0..=1`: step `i` holds until step `i + 1`, gliding when `smooth`.
fn steps_value(s: &Steps, cycles: f64) -> f32 {
    let n = s.values.len();
    let pos = cycles.rem_euclid(n as f64);
    let at = pos.floor() as usize % n;
    let value = f32::from(s.values[at]) / 100.0;
    if !s.smooth {
        return (value + 1.0) / 2.0;
    }
    let next = f32::from(s.values[(at + 1) % n]) / 100.0;
    let f = (pos - pos.floor()) as f32;
    ((value + (next - value) * f) + 1.0) / 2.0
}

/// An envelope's value `secs` after the transport's start: attack to 1, decay to the sustain, then the sustain.
fn envelope_value(e: &Envelope, secs: f64) -> f32 {
    let (attack, decay) = (f64::from(e.attack.max(0.0)), f64::from(e.decay.max(0.0)));
    let sustain = f64::from(e.sustain.clamp(0.0, 1.0));
    let t = secs.max(0.0);
    if t < attack && attack > 0.0 {
        (t / attack) as f32
    } else if t < attack + decay && decay > 0.0 {
        (1.0 - (1.0 - sustain) * (t - attack) / decay) as f32
    } else {
        sustain as f32
    }
}

/// A target's response to a `0..=1` source value.
fn curved(curve: Curve, u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    match curve {
        Curve::Linear => u,
        Curve::Exp => u * u,
        Curve::Log => u.sqrt(),
        Curve::S => u * u * (3.0 - 2.0 * u),
    }
}

/// The offset a target's depth makes of a `0..=1` source value: bipolar swings `-depth..depth` around the base,
/// unipolar `0..depth` above it.
fn offset(t: &ModTarget, u: f32) -> f64 {
    let shaped = f64::from(curved(t.curve, u));
    let signed = match t.polarity {
        Polarity::Bipolar => shaped * 2.0 - 1.0,
        Polarity::Unipolar => shaped,
    };
    f64::from(t.depth) * signed
}

/// Adds one step's modulated values, after the clips' values of the same step: every parameter's base (what a clip
/// last gave it, [`DEFAULT_BASE`] until then) carried by each enabled modulator's offset. Called from the automation
/// pass so that a value and its modulation are the two halves of one step, and from [`Engine::run_modulation`] when
/// no clip drives parameters at all.
pub(super) fn step_values(params: &mut ParamAuto, mods: &mut Modulation, pos: f64, at: u32, secs_per_tick: f64, ppq: f64) {
    let (beats, secs) = (pos / ppq, pos * secs_per_tick);
    // Every modulator's offset for a parameter adds up, then the parameter's own base carries them.
    let mut offsets: Vec<(Target, f64)> = Vec::new();
    for m in mods.modulators.iter().filter(|m| m.enabled) {
        let u = source_value(&m.source, m.id, beats, secs);
        for t in &m.targets {
            let by = offset(t, u);
            match offsets.iter_mut().find(|(x, _)| *x == t.target) {
                Some((_, v)) => *v += by,
                None => offsets.push((t.target, by)),
            }
        }
    }
    for (target, by) in offsets {
        let base = params.base_of(target).unwrap_or(DEFAULT_BASE);
        let value = (base + by).clamp(0.0, 1.0);
        match mods.last.iter_mut().find(|(x, ..)| *x == target) {
            Some((_, last_value, last_base)) if (*last_value - value).abs() < 1e-7 && (*last_base - base).abs() < 1e-7 => continue,
            Some((_, last_value, last_base)) => {
                *last_value = value;
                *last_base = base;
            }
            None => mods.last.push((target, value, base)),
        }
        params.push(target, at, value);
    }
}

/// A mixer track's host parameters the mixer path drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HostParam {
    /// The fader, stored 0..16000 (unity 12800).
    Volume,
    /// Pan, stored -6400..6400.
    Pan,
    /// Stereo separation, stored -64..64.
    Separation,
    /// Equaliser band and field: gain is stored ±1800, frequency and width 0..65536.
    Eq(u8, u8),
    /// A send's destination track; its level is 0..1 as automation speaks it (0.8 unity).
    Send(u16),
    /// A slot's mix level, stored 0..12800 (full wet).
    SlotMix(u8),
}

/// The track and host parameter a target names, or `None` for a target this path does not drive (a plugin parameter).
fn host_target(target: Target) -> Option<(usize, HostParam)> {
    Some(match target {
        Target::MixerVolume(t) => (usize::from(t), HostParam::Volume),
        Target::MixerPan(t) => (usize::from(t), HostParam::Pan),
        Target::MixerSeparation(t) => (usize::from(t), HostParam::Separation),
        Target::MixerEq(t, band, field) => (usize::from(t), HostParam::Eq(band, field)),
        Target::MixerSend(t, d) => (usize::from(t), HostParam::Send(d)),
        Target::SlotMix(t, k) => (usize::from(t), HostParam::SlotMix(k)),
        _ => return None,
    })
}

impl Engine {
    /// Installs the modulation graph (an empty one: no modulation). Values are sent again from scratch, and every
    /// track gives its host parameters back.
    pub fn set_modulation(&mut self, modulators: Vec<Modulator>) {
        self.mods.set(modulators);
        self.params.forget();
        for m in &mut self.tracks {
            let released = m.mod_vol.take().is_some() || m.mod_pan.take().is_some() || m.mod_sep.take().is_some();
            if released {
                m.matrix = m.compute_matrix();
            }
            // The equaliser, the slots' mix levels and the sends go back to their authored values too.
            m.release_modulation();
        }
    }

    /// The tick's modulated mixer values: a track's fader, pan and separation, sampled once a tick (a fader value is
    /// read by the mixer, not handed to a plugin) and held in `MixTrack::mod_*`, so the authored (or automated) value
    /// stays the base. A track the graph no longer drives gets its own value back.
    pub(super) fn run_host_modulation(&mut self) {
        if self.mods.is_empty() {
            return;
        }
        let now = self.tick as f64;
        let ppq = f64::from(self.ppq.max(1));
        let secs_per_tick = self.tick_len() as f64 / f64::from(self.rate.max(1));
        let (beats, secs) = (now / ppq, now * secs_per_tick);
        let mut offsets: Vec<(usize, HostParam, f64)> = Vec::new();
        for m in self.mods.modulators.iter().filter(|m| m.enabled) {
            let u = source_value(&m.source, m.id, beats, secs);
            for t in &m.targets {
                let Some((track, param)) = host_target(t.target) else { continue };
                let by = offset(t, u);
                match offsets.iter_mut().find(|(x, p, _)| *x == track && *p == param) {
                    Some((_, _, v)) => *v += by,
                    None => offsets.push((track, param, by)),
                }
            }
        }
        for (track, m) in self.tracks.iter_mut().enumerate() {
            // The authored values are the base, so a modulation never builds on its own last value.
            let base = [
                f64::from(m.vol) / 16000.0,
                (f64::from(m.pan) + 6400.0) / 12800.0,
                (f64::from(m.sep) + 64.0) / 128.0,
            ];
            let driven = |param: HostParam| offsets.iter().find(|(t, p, _)| *t == track && *p == param).map(|(_, _, by)| *by);
            let mut changed = false;
            for (i, param) in [HostParam::Volume, HostParam::Pan, HostParam::Separation].into_iter().enumerate() {
                let want = driven(param).map(|by| {
                    let v = (base[i] + by).clamp(0.0, 1.0);
                    match param {
                        HostParam::Volume => (v * 16000.0).round() as i32,
                        HostParam::Pan => (v * 12800.0 - 6400.0).round() as i32,
                        _ => (v * 128.0 - 64.0).round() as i32,
                    }
                });
                let slot = match param {
                    HostParam::Volume => &mut m.mod_vol,
                    HostParam::Pan => &mut m.mod_pan,
                    _ => &mut m.mod_sep,
                };
                if *slot != want {
                    *slot = want;
                    changed = true;
                }
            }
            if changed {
                m.matrix = m.compute_matrix();
            }
            // The equaliser's nine fields, in the units the equaliser is stored in (the same encoding the automation
            // path writes: gain ±1800, frequency and width 0..65536).
            let mut eq_changed = false;
            for band in 0..3usize {
                for field in 0..3usize {
                    let want = driven(HostParam::Eq(band as u8, field as u8)).map(|by| {
                        let own = f64::from(m.eq_own[band][field]);
                        let base = if field == 0 { (own / 1800.0 + 1.0) / 2.0 } else { own / 65536.0 };
                        let v = (base + by).clamp(0.0, 1.0);
                        if field == 0 { ((2.0 * v - 1.0) * 1800.0).round() as i32 } else { (65536.0 * v).round() as i32 }
                    });
                    if m.mod_eq[band][field] != want {
                        m.mod_eq[band][field] = want;
                        eq_changed = true;
                    }
                }
            }
            if eq_changed {
                for band in 0..3 {
                    for field in 0..3 {
                        m.sync_eq(band, field);
                    }
                }
            }
            // A slot's mix level (0..12800): the effect keeps the value it reads, the authored one stays in `slot_mix`.
            for k in 0..10usize {
                let want = driven(HostParam::SlotMix(k as u8)).map(|by| {
                    ((f64::from(m.slot_mix[k]) / 12800.0 + by).clamp(0.0, 1.0) * 12800.0).round() as i32
                });
                if m.mod_slot_mix[k] != want {
                    m.mod_slot_mix[k] = want;
                    let mix = want.unwrap_or(m.slot_mix[k]);
                    for fx in m.effects.iter_mut().filter(|f| f.slot == k) {
                        fx.mix = mix;
                    }
                }
            }
            // A send's level, in the 0..1 domain automation writes (0.8 is unity); its gain follows from it.
            for route in &mut m.routes {
                let by = driven(HostParam::Send(route.dest as u16));
                route.mod_level = by.map(|by| (f64::from(route.level) + by).clamp(0.0, 1.0) as f32);
            }
        }
    }

    /// Samples every modulator over the `frames` frames of the tick starting now: the path when no clip drives a
    /// parameter (the automation pass calls [`step_values`] itself, so a value and its modulation stay one step).
    pub(super) fn run_modulation(&mut self, frames: usize) {
        if self.mods.is_empty() {
            return;
        }
        let len = frames.max(1) as f64;
        let now = self.tick as f64;
        let secs_per_tick = self.tick_len() as f64 / f64::from(self.rate.max(1));
        let ppq = f64::from(self.ppq.max(1));
        for step in (0..frames).step_by(PARAM_STEP) {
            step_values(&mut self.params, &mut self.mods, now + step as f64 / len, step as u32, secs_per_tick, ppq);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use os_model::{AutoPoint, Automation, Channel, ChannelKind, Clip, ClipSource, Project};

    use crate::{Engine, Instrument, PlayMode, SampleLibrary};

    use super::*;

    /// Records parameter events as (absolute frame, index, value).
    #[derive(Default)]
    struct Probe {
        frame: u64,
        seen: Vec<(u64, u32, f64)>,
        pending: Vec<(u32, u32, f64)>,
    }

    impl Probe {
        fn take(&mut self, n: usize) {
            for (o, i, v) in self.pending.drain(..) {
                self.seen.push((self.frame + u64::from(o), i, v));
            }
            self.frame += n as u64;
        }
    }

    impl Instrument for Probe {
        fn note_on(&mut self, _: u32, _: u8, _: f32) {}
        fn note_off(&mut self, _: u32, _: u8) {}
        fn process(&mut self, _rate: u32, l: &mut [f32], r: &mut [f32]) {
            l.fill(0.0);
            r.fill(0.0);
            self.take(l.len());
        }
        fn param(&mut self, offset: u32, index: u32, value: f64) {
            self.pending.push((offset, index, value));
        }
    }

    fn channel(index: u16, kind: ChannelKind) -> Channel {
        Channel {
            id: os_model::Id::NONE,
            index,
            kind,
            name: None,
            plugin: None,
            color: None,
            enabled: true,
            volume: if kind == ChannelKind::Automation { 1.0 } else { 0.78125 },
            pan: if kind == ChannelKind::Automation { -1.0 } else { 0.0 },
            pitch_cents: 0,
            mixer_track: 0,
            sample_path: None,
            layer_children: vec![],
            envelopes: Default::default(),
            rack: Default::default(),
            plugin_state: None,
            misc: Default::default(),
            sound: Default::default(),
        }
    }

    /// One plugin channel (its instrument on parameter 7) and a second one, with no automation; `target`'s ramp over
    /// four beats when it is a plugin parameter.
    fn project(target: u32) -> Project {
        let auto = Automation {
            channel: 1,
            targets: vec![target],
            points: vec![AutoPoint::new(0.0, 0.0), AutoPoint::new(4.0, 1.0)],
            header: vec![0; 21],
            tail: vec![],
            lfo: Default::default(),
        };
        let mut p = Project {
            ppq: 96,
            tempo: Some(120.0),
            channels: vec![channel(0, ChannelKind::Plugin), channel(1, ChannelKind::Automation)],
            mixer: (0..3).map(os_model::MixerTrack::new).collect(),
            ..Project::default()
        };
        if matches!(os_model::target::decode(target), Target::ChannelParam(..)) {
            p.automations = vec![auto];
            p.arrangements = vec![os_model::Arrangement { clips: vec![Clip::new(0, 4 * 96, 0, ClipSource::Channel(1))], ..Default::default() }];
        }
        p
    }

    fn run(e: &mut Engine, frames: usize, block: usize) {
        let mut out = vec![0.0f32; block * 2];
        for _ in 0..frames / block {
            e.process(&mut out);
        }
    }

    fn lfo(shape: LfoShape, sixteenths: u16, depth: f32, polarity: Polarity) -> Modulator {
        Modulator {
            id: 1,
            source: ModSource::Lfo(Lfo { shape, period: Timebase::Sync(sixteenths), phase: 0.0 }),
            enabled: true,
            targets: vec![ModTarget { target: Target::ChannelParam(0, 7), depth, curve: Curve::Linear, polarity }],
        }
    }

    /// A sine LFO on a channel parameter swings the parameter halfway up and down: the middle at the cycle's start,
    /// the top a quarter cycle later, the bottom three quarters in, sampled every 64 frames.
    #[test]
    fn an_lfo_moves_a_parameter_between_its_limits() {
        let p = project(0);
        let mut e = Engine::new(&p, &mut SampleLibrary::new(None), 48000, PlayMode::Pattern(1));
        let probe = Arc::new(Mutex::new(Probe::default()));
        e.set_instrument(0, probe.clone());
        // One cycle a beat: 24000 frames at 120 BPM.
        e.set_modulation(vec![lfo(LfoShape::Sine, 4, 0.5, Polarity::Bipolar)]);
        run(&mut e, 24000, 1000);
        let seen = probe.lock().unwrap().seen.clone();
        assert!(seen.len() > 300, "{}", seen.len());
        assert!(seen.iter().all(|&(_, i, v)| i == 7 && (0.0..=1.0).contains(&v)));
        assert!(seen.windows(2).all(|w| w[1].0 - w[0].0 <= 64), "a value at least every 64 frames");
        let at = |f: u64| seen.iter().find(|&&(frame, _, _)| frame >= f).map(|&(_, _, v)| v).unwrap();
        assert!((seen[0].2 - 0.5).abs() < 1e-6, "the sine starts in the middle: {}", seen[0].2);
        assert!((at(6000) - 1.0).abs() < 1e-3, "the top a quarter cycle in: {}", at(6000));
        assert!(at(18000) < 1e-3, "the bottom three quarters in: {}", at(18000));
    }

    /// A modulator moves the value the clips hand the parameter, not a value of its own: a step sequence at its top,
    /// bipolar, a quarter of the range, sits a quarter above the ramp (and stops at the top).
    #[test]
    fn modulation_moves_from_the_value_the_clips_hand_on() {
        let p = project(os_model::target::channel_param(0, 7));
        let mut e = Engine::new(&p, &mut SampleLibrary::new(None), 48000, PlayMode::Song);
        let probe = Arc::new(Mutex::new(Probe::default()));
        e.set_instrument(0, probe.clone());
        let steps = Steps { values: [100; 16], period: Timebase::Sync(1), smooth: false };
        e.set_modulation(vec![Modulator {
            id: 1,
            source: ModSource::Steps(steps),
            enabled: true,
            targets: vec![ModTarget { target: Target::ChannelParam(0, 7), depth: 0.25, curve: Curve::Linear, polarity: Polarity::Bipolar }],
        }]);
        run(&mut e, 98000, 1000);
        let seen = probe.lock().unwrap().seen.clone();
        assert!(seen.len() > 300, "{}", seen.len());
        // The clip's own value is sent first and the composed one after it at the same frame: the last value a frame
        // gets is the modulated one.
        let mut last_at: Vec<(u64, f64)> = Vec::new();
        for &(f, i, v) in &seen {
            assert_eq!(i, 7);
            match last_at.last_mut() {
                Some((frame, val)) if *frame == f => *val = v,
                _ => last_at.push((f, v)),
            }
        }
        for &(f, v) in &last_at {
            let base = ((f + 250) as f64 / 96000.0).min(1.0);
            let want = (base + 0.25).min(1.0);
            assert!((v - want).abs() < 1e-6, "frame {f}: {v} vs {want} (the ramp's {base})");
        }
        // The ramp reaches 0.75 after three of its four beats: from there the parameter sits at its top (and, a value
        // being sent only when it changes, at the top is where it stays).
        assert!(last_at.iter().filter(|&&(f, _)| f > 72000).all(|&(_, v)| (v - 1.0).abs() < 1e-9));
        assert!(last_at.iter().any(|&(f, v)| f > 72000 && (v - 1.0).abs() < 1e-9));
        assert!(last_at.iter().filter(|&&(f, _)| f < 70000).all(|&(_, v)| v < 1.0), "the ramp's own headroom stays");
    }

    /// A disabled modulator, and one whose target the engine does not drive yet (a host parameter), send nothing.
    #[test]
    fn nothing_is_driven_that_the_engine_cannot_or_must_not() {
        let p = project(0);
        let mut e = Engine::new(&p, &mut SampleLibrary::new(None), 48000, PlayMode::Pattern(1));
        let probe = Arc::new(Mutex::new(Probe::default()));
        e.set_instrument(0, probe.clone());
        let mut off = lfo(LfoShape::Square, 4, 1.0, Polarity::Bipolar);
        off.enabled = false;
        let host = Modulator {
            id: 2,
            source: ModSource::Steps(Steps { values: [100; 16], ..Steps::default() }),
            enabled: true,
            targets: vec![ModTarget { target: Target::MixerVolume(1), depth: 1.0, curve: Curve::Linear, polarity: Polarity::Unipolar }],
        };
        e.set_modulation(vec![off, host]);
        run(&mut e, 24000, 1000);
        assert!(probe.lock().unwrap().seen.is_empty());
    }

    /// The shapes read at their own points: a sine starts halfway and peaks a quarter in, a triangle runs its full
    /// swing, the saws go one way, the square is a switch.
    #[test]
    fn the_lfo_shapes() {
        let l = |shape| Lfo { shape, period: Timebase::Free(1.0), phase: 0.0 };
        let at = |shape, cycles: f64| lfo_value(&l(shape), 1, cycles);
        assert!((at(LfoShape::Sine, 0.0) - 0.5).abs() < 1e-6);
        assert!((at(LfoShape::Sine, 0.25) - 1.0).abs() < 1e-6);
        assert!(at(LfoShape::Sine, 0.75) < 1e-6);
        assert!(at(LfoShape::Triangle, 0.0) < 1e-6);
        assert!((at(LfoShape::Triangle, 0.5) - 1.0).abs() < 1e-6);
        assert!(at(LfoShape::SawUp, 0.0) < 1e-6);
        assert!((at(LfoShape::SawUp, 0.5) - 0.5).abs() < 1e-6);
        assert!((at(LfoShape::SawDown, 0.0) - 1.0).abs() < 1e-6);
        assert!((at(LfoShape::Square, 0.25) - 1.0).abs() < 1e-6);
        assert!(at(LfoShape::Square, 0.75) < 1e-6);
    }

    /// The random shape holds one value a cycle, changes from cycle to cycle, and gives the same value again on a
    /// second read (a render equals playback; a seek lands where it would have).
    #[test]
    fn the_random_shape_holds_and_repeats() {
        let l = Lfo { shape: LfoShape::Random, period: Timebase::Free(2.0), phase: 0.0 };
        let (a, b, c) = (lfo_value(&l, 1, 0.1), lfo_value(&l, 1, 0.9), lfo_value(&l, 1, 1.1));
        assert_eq!(a, b, "one value a cycle");
        assert_ne!(a, c, "a new value the next cycle");
        assert_eq!(a, lfo_value(&l, 1, 0.1), "and the same one again");
        assert_ne!(a, lfo_value(&l, 2, 0.1), "another modulator draws its own");
        assert!((0.0..=1.0).contains(&a));
    }

    /// A step sequence jumps by default and glides when smooth; its values run -100..100 as 0..1.
    #[test]
    fn the_steps_jump_or_glide() {
        let mut values = [0i8; 16];
        values[0] = -100;
        values[1] = 100;
        let jump = steps_value(&Steps { values, period: Timebase::Sync(1), smooth: false }, 0.9);
        assert!(jump < 1e-6, "the first step until the next");
        let glide = steps_value(&Steps { values, period: Timebase::Sync(1), smooth: true }, 0.5);
        assert!((glide - 0.5).abs() < 1e-6, "halfway between the two: {glide}");
        assert_eq!(steps_value(&Steps { values, ..Steps::default() }, 0.0), 0.0);
    }

    /// An envelope rises over its attack, decays to its sustain and holds it.
    #[test]
    fn the_envelope_rises_decays_and_holds() {
        let e = Envelope { attack: 0.5, decay: 1.0, sustain: 0.25, release: 0.0 };
        assert_eq!(envelope_value(&e, 0.0), 0.0);
        assert!((envelope_value(&e, 0.25) - 0.5).abs() < 1e-6);
        assert!((envelope_value(&e, 1.0) - 0.625).abs() < 1e-6, "{}", envelope_value(&e, 1.0));
        assert_eq!(envelope_value(&e, 5.0), 0.25);
        assert_eq!(envelope_value(&Envelope { attack: 0.0, decay: 0.0, sustain: 0.75, release: 0.0 }, 0.1), 0.75);
    }

    /// The curves and the two polarities: a quarter of the range means different offsets above and around the base.
    #[test]
    fn curves_and_polarity() {
        let t = |curve, polarity| ModTarget { target: Target::ChannelParam(0, 0), depth: 0.5, curve, polarity };
        assert!((offset(&t(Curve::Linear, Polarity::Bipolar), 0.5) - 0.0).abs() < 1e-9);
        assert!((offset(&t(Curve::Linear, Polarity::Bipolar), 0.0) + 0.5).abs() < 1e-9);
        assert!((offset(&t(Curve::Linear, Polarity::Unipolar), 0.0) - 0.0).abs() < 1e-9);
        assert!((offset(&t(Curve::Linear, Polarity::Unipolar), 1.0) - 0.5).abs() < 1e-9);
        assert!((offset(&t(Curve::Exp, Polarity::Unipolar), 0.5) - 0.125).abs() < 1e-9);
        assert!((curved(Curve::Log, 0.25) - 0.5).abs() < 1e-6);
        assert!((curved(Curve::S, 0.5) - 0.5).abs() < 1e-6);
        // A depth past the range still lands the parameter inside 0..1.
        let t = ModTarget { depth: 3.0, ..t(Curve::Linear, Polarity::Bipolar) };
        assert_eq!((0.9 + offset(&t, 1.0)).clamp(0.0, 1.0), 1.0);
    }

    /// A track's fader is a host parameter: a step source at its top, bipolar, half the range, takes the fader of a
    /// unity track (0.8 of its range) to the top, leaves the other tracks alone, and its fader matrix follows; with
    /// the graph gone the track has its own value and matrix back.
    #[test]
    fn modulation_drives_a_mixer_tracks_fader_and_gives_it_back() {
        let p = project(0);
        let mut e = Engine::new(&p, &mut SampleLibrary::new(None), 48000, PlayMode::Pattern(1));
        let steps = Steps { values: [100; 16], period: Timebase::Sync(1), smooth: false };
        e.set_modulation(vec![Modulator {
            id: 1,
            source: ModSource::Steps(steps),
            enabled: true,
            targets: vec![ModTarget { target: Target::MixerVolume(1), depth: 0.5, curve: Curve::Linear, polarity: Polarity::Bipolar }],
        }]);
        run(&mut e, 1000, 1000);
        assert_eq!(e.tracks[1].mod_vol, Some(16000), "unity plus half the range is the top");
        assert_eq!(e.tracks[1].vol, 12800, "the authored fader is still the base");
        assert_eq!(e.tracks[0].mod_vol, None, "another track keeps its own value");
        assert_ne!(e.tracks[1].matrix, e.tracks[0].matrix, "the fader matrix follows the modulated value");
        // The graph goes: every track is released to its own values.
        e.set_modulation(Vec::new());
        run(&mut e, 1000, 1000);
        assert_eq!(e.tracks[1].mod_vol, None);
        assert_eq!(e.tracks[1].matrix, e.tracks[0].matrix);
    }

    /// Pan and separation are driven in their own stored units: bipolar around the middle (0) and unipolar above the
    /// middle; a separation of 32 merges a quarter of the way.
    #[test]
    fn a_modulated_pan_and_separation_use_the_mixers_units() {
        let p = project(0);
        let mut e = Engine::new(&p, &mut SampleLibrary::new(None), 48000, PlayMode::Pattern(1));
        let steps = Steps { values: [100; 16], period: Timebase::Sync(1), smooth: false };
        let target = |target, depth, polarity| ModTarget { target, depth, curve: Curve::Linear, polarity };
        e.set_modulation(vec![Modulator {
            id: 1,
            source: ModSource::Steps(steps),
            enabled: true,
            targets: vec![
                target(Target::MixerPan(1), 0.25, Polarity::Unipolar),
                target(Target::MixerSeparation(1), 0.25, Polarity::Unipolar),
            ],
        }]);
        run(&mut e, 1000, 1000);
        assert_eq!(e.tracks[1].mod_pan, Some(3200), "a quarter right of the middle");
        assert_eq!(e.tracks[1].mod_sep, Some(32), "and a twenty-eighth merged");
        assert_eq!((e.tracks[1].pan, e.tracks[1].sep), (0, 0), "the stored values stay");
        assert_eq!(e.tracks[2].mod_pan, None);
    }

    /// The equaliser, a send and a slot's mix level are host parameters too, each moved in its own stored units and
    /// each giving its authored value back when the graph goes.
    #[test]
    fn modulation_drives_the_eq_a_send_and_a_slots_mix() {
        let mut p = project(0);
        p.mixer[1].routes.push(2);
        p.mixer[1].send_levels.push((2, 0.4));
        p.mixer[1].slot_mix[0] = 0.5;
        let mut e = Engine::new(&p, &mut SampleLibrary::new(None), 48000, PlayMode::Pattern(1));
        let steps = Steps { values: [100; 16], period: Timebase::Sync(1), smooth: false };
        let target = |target| ModTarget { target, depth: 0.5, curve: Curve::Linear, polarity: Polarity::Bipolar };
        e.set_modulation(vec![Modulator {
            id: 1,
            source: ModSource::Steps(steps),
            enabled: true,
            targets: vec![target(Target::MixerEq(1, 0, 0)), target(Target::MixerSend(1, 2)), target(Target::SlotMix(1, 0))],
        }]);
        run(&mut e, 1000, 1000);
        assert_eq!(e.tracks[1].eq.settings(0)[0], 1800, "0 dB plus half the gain range is the top");
        assert_eq!(e.tracks[1].eq_own[0][0], 0, "the authored gain is still the base");
        assert_eq!(e.tracks[1].mod_slot_mix[0], Some(12800), "half wet plus half the range is full wet");
        assert_eq!(e.tracks[1].slot_mix[0], 6400, "the stored mix level stays");
        // A track always routes to the master; the send to track 2 is the second route.
        let send = e.tracks[1].routes.iter().find(|r| r.dest == 2).expect("the send to track 2");
        let level = send.mod_level.expect("the send is driven");
        assert!((level - 0.9).abs() < 1e-6, "0.4 plus half the level range: {level}");
        assert_eq!(send.level, 0.4, "the authored send level stays the base");
        assert_eq!(e.tracks[1].routes[0].mod_level, None, "the master route is not driven");
        assert_eq!(e.tracks[2].eq.settings(0)[0], 0, "another track's EQ is untouched");
        // The graph goes: every one of them is back where it was.
        e.set_modulation(Vec::new());
        run(&mut e, 1000, 1000);
        assert_eq!(e.tracks[1].eq.settings(0)[0], 0);
        assert_eq!(e.tracks[1].mod_slot_mix[0], None);
        assert_eq!(e.tracks[1].routes.iter().find(|r| r.dest == 2).and_then(|r| r.mod_level), None);
    }
}
