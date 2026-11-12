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
//! **What is driven.** Plugin parameters today — a channel's instrument ([`Target::ChannelParam`]) and a mixer slot's
//! hosted effect ([`Target::SlotParam`]). A host parameter (a track's volume, pan, EQ or a send) decodes to a different
//! `Target` and is left alone until the mixer path takes modulated values (the next slice).
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

impl Engine {
    /// Installs the modulation graph (an empty one: no modulation). Values are sent again from scratch.
    pub fn set_modulation(&mut self, modulators: Vec<Modulator>) {
        self.mods.set(modulators);
        self.params.forget();
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
}
