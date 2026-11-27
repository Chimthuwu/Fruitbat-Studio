// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The modulation graph (M1b), evaluated on the engine's clock.
//!
//! Each [`Modulator`] carries one source from [`os_model::modulation`] and the parameters it drives, already resolved
//! to the engine's [`Target`] by the app ([`crate::Engine::set_modulation`]); the graph itself never reaches the
//! engine, and no legacy project `Target` encoding changes (audit K.1). Values are sent through the same parameter-event path
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

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use os_model::modulation::{AudioFollower, Curve, Detection, Distribution, Envelope, Lfo, LfoShape, Macro, ModParam, ModSource, Mseg, Polarity, Random, Steps, Timebase, VoiceAdsr};
use os_model::target::Target;
use os_dsp::voice_pool::{VoicePool, VoiceContext};

use super::param_auto::{ParamAuto, PARAM_STEP};
use super::Engine;

/// Where a parameter sits when nothing has set it: halfway.
pub const DEFAULT_BASE: f64 = 0.5;

/// A target's published base before the engine has run a step for it (a NaN, which no clamped value can be).
const NO_BASE: u32 = u32::MAX;

/// The live modulation values the UI reads every frame (the handoff's lock-free snapshot). Preallocated by the app
/// when it installs a graph; the audio thread only stores into it, and nothing here allocates or locks.
#[derive(Debug, Default)]
pub struct ModSnapshot {
    /// The modulator ids, in the order `outputs` is indexed by.
    ids: Vec<u32>,
    /// The targets, in the order `values` is indexed by.
    targets: Vec<Target>,
    outputs: Box<[AtomicU32]>,
    values: Box<[AtomicU32]>,
    /// What each target's modulation moves from at the last step: the value its automation hands it there, else its
    /// own value ([`NO_BASE`] until the engine runs a step).
    bases: Box<[AtomicU32]>,
}

impl ModSnapshot {
    /// A snapshot for a graph of `ids` driving `targets`.
    pub fn new(ids: &[u32], targets: &[Target]) -> ModSnapshot {
        ModSnapshot {
            ids: ids.to_vec(),
            targets: targets.to_vec(),
            outputs: (0..ids.len()).map(|_| AtomicU32::new(0)).collect(),
            values: (0..targets.len()).map(|_| AtomicU32::new(0)).collect(),
            bases: (0..targets.len()).map(|_| AtomicU32::new(NO_BASE)).collect(),
        }
    }

    /// A modulator's latest `0..=1` output.
    pub fn output(&self, id: u32) -> Option<f32> {
        let i = self.ids.iter().position(|&x| x == id)?;
        Some(f32::from_bits(self.outputs[i].load(Ordering::Relaxed)))
    }

    /// A target's latest effective (modulated, normalized) value.
    pub fn value(&self, target: u32) -> Option<f32> {
        let want = os_model::target::decode(target);
        let i = self.targets.iter().position(|&x| x == want)?;
        Some(f32::from_bits(self.values[i].load(Ordering::Relaxed)))
    }

    /// What a target's modulation moves from at the last step: the value its automation hands it at that position, or
    /// its own value. A knob the graph drives is drawn at this, so its automation still turns it while the modulation
    /// does not. `None` until the engine has run a step (nothing playing).
    pub fn base(&self, target: u32) -> Option<f32> {
        let want = os_model::target::decode(target);
        let i = self.targets.iter().position(|&x| x == want)?;
        let bits = self.bases[i].load(Ordering::Relaxed);
        (bits != NO_BASE).then(|| f32::from_bits(bits))
    }

    fn write_output(&self, index: usize, v: f32) {
        if let Some(a) = self.outputs.get(index) {
            a.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        }
    }

    fn write_value(&self, target: Target, v: f64) {
        let Some(index) = self.targets.iter().position(|&x| x == target) else { return };
        if let Some(a) = self.values.get(index) {
            a.store((v.clamp(0.0, 1.0) as f32).to_bits(), Ordering::Relaxed);
        }
    }

    /// The write half of [`ModSnapshot::base`], called from the audio thread's step pass. Public so a test can stand in
    /// for the engine; the UI only reads.
    pub fn write_base(&self, target: Target, v: f64) {
        let Some(index) = self.targets.iter().position(|&x| x == target) else { return };
        if let Some(a) = self.bases.get(index) {
            a.store((v.clamp(0.0, 1.0) as f32).to_bits(), Ordering::Relaxed);
        }
    }
}

/// One audio follower's detector state: a cheap band filter, a level and its attack/release envelope. No FFT, no
/// allocation, no lock.
#[derive(Debug, Clone, Copy)]
pub(super) struct Follower {
    id: u32,
    source: AudioFollower,
    /// One-pole states: two cascaded low-passes and two cascaded high-passes on the mono sum.
    lp: [f32; 2],
    hp: [f32; 2],
    /// Each high-pass stage's previous **input**. A one-pole high-pass is `y[n] = a * (y[n-1] + x[n] - x[n-1])`,
    /// so the second stage must remember the first stage's output, not the raw sample the first stage saw.
    hp_prev: [f32; 2],
    level: f32,
}

impl Follower {
    fn new(id: u32, source: AudioFollower) -> Follower {
        Follower { id, source, lp: [0.0; 2], hp: [0.0; 2], hp_prev: [0.0; 2], level: 0.0 }
    }

    /// Detects the band-filtered level of `samples` (interleaved stereo) and moves the smoothed level toward it.
    /// The frequency filtering happens **before** level detection, as the handoff requires.
    fn feed(&mut self, samples: &[f32], rate: f32) {
        // Decimated: a control-rate follower needs no sample-rate detector, and this keeps the audio thread free.
        const STEP: usize = 8;
        let dt = STEP as f32 / rate.max(1.0);
        let (low, high) = match self.source.band.edges() {
            Some((low, high)) => (low.clamp(20.0, 20000.0), high.clamp(20.0, 20000.0)),
            None => (0.0, 0.0),
        };
        let hp_a = if low > 0.0 { let rc = 1.0 / (std::f32::consts::TAU * low); rc / (rc + dt) } else { 1.0 };
        let lp_a = if high > 0.0 { let rc = 1.0 / (std::f32::consts::TAU * high); dt / (rc + dt) } else { 1.0 };
        let mut peak = 0.0f32;
        let mut sum_sq = 0.0f64;
        let mut n = 0u32;
        let mut i = 0usize;
        while i + 1 < samples.len() {
            let x = 0.5 * (samples[i] + samples[i + 1]);
            // High-pass first, then low-pass: the detection band, before any level is measured.
            let mut y = x;
            if low > 0.0 {
                for k in 0..2 {
                    // Each stage keeps its own previous input: feeding stage 2 the same raw-sample history as
                    // stage 1 leaks the unfiltered signal into the detector (the two stages stop being a
                    // cascade) and makes the band's low edge far weaker than its cutoff says.
                    let x_prev = self.hp_prev[k];
                    let y_prev = self.hp[k];
                    self.hp_prev[k] = y;
                    self.hp[k] = hp_a * (y_prev + y - x_prev);
                    y = self.hp[k];
                }
            }
            if high > 0.0 {
                for s in &mut self.lp {
                    *s += (y - *s) * lp_a;
                    y = *s;
                }
            }
            peak = peak.max(y.abs());
            sum_sq += f64::from(y) * f64::from(y);
            n += 1;
            i += STEP * 2;
        }
        let rms = if n > 0 { (sum_sq / f64::from(n)).sqrt() as f32 } else { 0.0 };
        let detected = match self.source.detection {
            Detection::Peak => peak,
            Detection::Rms => rms,
        } * 10f32.powf(self.source.gain_db / 20.0);
        // Attack and release are one-pole over the detector's own block; no per-sample branching.
        let tau = if detected > self.level { self.source.attack } else { self.source.release }
            .clamp(0.0005, 30.0);
        let k = (dt / (tau + dt)).clamp(0.0, 1.0);
        self.level += (detected - self.level) * k;
        self.level = self.level.clamp(0.0, 1.0);
    }

    /// Its `0..=1` output: the level scaled by the source's amount *around the middle*, so an amount of 0 is the
    /// middle (no modulation at all) whatever the signal does, and turning it down shrinks the swing.
    fn value(&self) -> f32 {
        let amount = self.source.amount.clamp(0.0, 1.0);
        (0.5 + (self.level - 0.5) * amount).clamp(0.0, 1.0)
    }
}

/// The last note each channel played, as the note sources read it: one slot per channel, holding the key and the
/// velocity of the newest note. A note off leaves it alone — key tracking and velocity are properties of the note the
/// channel last played, not of whether it is still held — so a pad's filter stays where the last chord put it.
#[derive(Debug, Default, Clone)]
pub(super) struct NoteBank {
    channels: Vec<Option<(u8, u8)>>,
    /// The newest note of the whole engine and its channel, for a target that is not on a channel (a master-adjacent
    /// mixer target) and for the graph's own display.
    latest: Option<(usize, u8, u8)>,
}

impl NoteBank {
    /// What a note source reads for `channel`: its `0..=1` value, the middle for a channel that has played nothing.
    fn value(&self, source: &ModSource, channel: usize) -> f32 {
        match self.channels.get(channel).copied().flatten() {
            Some((key, velocity)) => Self::read(source, key, velocity),
            None => 0.5,
        }
    }

    /// What a note source reads for the newest note anywhere.
    fn value_latest(&self, source: &ModSource) -> f32 {
        match self.latest {
            Some((_, key, velocity)) => Self::read(source, key, velocity),
            None => 0.5,
        }
    }

    /// A note source's value for one note: the reading scaled by the source's amount around the middle.
    fn read(source: &ModSource, key: u8, velocity: u8) -> f32 {
        match source {
            ModSource::Velocity(v) => {
                let u = f32::from(velocity.min(127)) / 127.0;
                0.5 + (u - 0.5) * v.amount.clamp(0.0, 1.0)
            }
            ModSource::KeyTrack(k) => {
                let u = k.value(key);
                0.5 + (u - 0.5) * k.amount.clamp(0.0, 1.0)
            }
            _ => 0.5,
        }
    }

    /// Records a note on: the channel's last note becomes this one, and (if it is the newest of the tick) the engine's.
    pub(super) fn note_on(&mut self, channel: usize, key: u8, velocity: u8) {
        if self.channels.len() <= channel {
            self.channels.resize(channel + 1, None);
        }
        let velocity = velocity.min(127);
        self.channels[channel] = Some((key, velocity));
        self.latest = Some((channel, key, velocity));
    }

    fn is_note(&self) -> bool {
        !self.channels.is_empty()
    }
}

/// One parameter a modulator drives, resolved to an engine [`Target`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModTarget {
    pub target: Target,
    /// `-1..=1` of the parameter's range; negative inverts the source.
    pub depth: f32,
    pub curve: Curve,
    pub polarity: Polarity,
    /// Off keeps the routing but drives nothing.
    pub enabled: bool,
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
    /// The audio followers in the graph, by modulator id (rebuilt when the graph is installed).
    followers: Vec<Follower>,
    /// The routes inside the graph (a modulator driving another's rate, amount, phase or offset, or the depth of one
    /// of its assignments), as indices into `modulators`. Rebuilt when the graph is installed, never while it runs.
    edges: Vec<Edge>,
    /// Every assignment's depth as it stands this tick: its own depth moved by whatever drives it. Indexed by
    /// `at[modulator] + target`. Recomputed at the start of every evaluation.
    depths: Vec<f32>,
    /// Where each modulator's targets start in `depths`.
    at: Vec<usize>,
    /// The modulators in dependency order, by index: each comes after everything that drives it, so one pass gives
    /// every modulator its driven value. Rebuilt when the graph is installed.
    order: Vec<u32>,
    /// Each modulator's `0..=1` output this evaluation, indexed like `modulators`. Reused every tick.
    outputs: Vec<f32>,
    /// Scratch so a tick never allocates: the offsets a parameter's drivers add up to, and the mixer path's.
    by: Vec<(Target, f64)>,
    host_by: Vec<(usize, HostParam, Target, f64)>,
    /// The last note of each channel, which the note sources read.
    pub(super) notes: NoteBank,
    /// The channel each mixer track plays through (`None` for a track with no channel), so a note source can find the
    /// note of the channel a mixer target belongs to.
    track_channels: Vec<Option<usize>>,
    /// The channel count `note_outputs` was sized for.
    channels: usize,
    /// Each note-driven modulator's value per channel, indexed `modulator × channels + channel`: `NaN` for every other
    /// modulator, so a destination on a channel can tell which ones read a note.
    note_outputs: Vec<f32>,
    /// Per-voice note context for voice-triggered modulation sources (VoiceAdsr).
    /// Updated from the engine's note events before each evaluation tick.
    pub(super) voice_pool: VoicePool,
    /// The longest release of the graph's VoiceAdsr sources as last evaluated (0 with none).
    voice_release: f32,
    /// The live values the UI reads.
    pub(super) snapshot: Option<Arc<ModSnapshot>>,
}

/// One route inside the graph: modulator `from` drives parameter `param` of modulator `to` — or, with no `param`, the
/// depth of assignment `target` of modulator `to` (indices into the graph's modulators and that modulator's targets),
/// through its own `curve` and `polarity`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Edge {
    from: u32,
    to: u32,
    /// The index of this assignment among `from`'s targets: its effective depth lives in that `depths` slot.
    slot: usize,
    /// What it drives on `to`: a parameter, or (with `None`) the depth of target `target`.
    param: Option<ModParam>,
    target: usize,
    depth: f32,
    curve: Curve,
    polarity: Polarity,
    enabled: bool,
}

impl Modulation {
    /// Replaces the graph; every value is sent again from scratch, as after a seek, and every follower starts at
    /// silence rather than at whatever the last graph detected.
    pub fn set(&mut self, modulators: Vec<Modulator>) {
        self.followers = modulators
            .iter()
            .filter_map(|m| match m.source {
                ModSource::AudioFollower(f) => Some(Follower::new(m.id, f)),
                _ => None,
            })
            .collect();
        // The graph's own edges: an assignment whose target is inside the graph (a modulator's parameter or a route's
        // depth) is not a project parameter at all, so it never reaches the parameter-event path.
        self.edges.clear();
        self.depths.clear();
        self.at.clear();
        let mut at = 0usize;
        for m in &modulators {
            self.at.push(at);
            at += m.targets.len();
        }
        self.depths.resize(at, 0.0);
        for (i, m) in modulators.iter().enumerate() {
            for (k, t) in m.targets.iter().enumerate() {
                self.depths[self.at[i] + k] = t.depth;
                let (to, param, target) = match t.target {
                    Target::ModulatorParam(id, code) => {
                        let Some(param) = ModParam::of(code) else { continue };
                        let Some(to) = modulators.iter().position(|o| o.id == id) else { continue };
                        (to, Some(param), 0)
                    }
                    Target::RouteDepth(owner, index) => {
                        let Some(to) = modulators.iter().position(|o| o.id == owner) else { continue };
                        if usize::from(index) >= modulators[to].targets.len() {
                            continue;
                        }
                        (to, None, usize::from(index))
                    }
                    _ => continue,
                };
                self.edges.push(Edge {
                    from: i as u32,
                    to: to as u32,
                    slot: k,
                    param,
                    target,
                    depth: t.depth,
                    curve: t.curve,
                    polarity: t.polarity,
                    enabled: t.enabled,
                });
            }
        }
        self.order = topological(&modulators, &self.edges);
        self.outputs.clear();
        self.outputs.resize(modulators.len(), 0.0);
        self.modulators = modulators;
        self.last.clear();
        self.by.clear();
        self.host_by.clear();
        // The held notes stay: they are the transport's, not the graph's.
        self.voice_release = 0.0;
    }

    pub fn is_empty(&self) -> bool {
        self.modulators.is_empty()
    }

    /// Sizes the per-channel note values for `channels` channels and `tracks`, the channel each mixer track plays
    /// through. Called when the graph is installed and when the project's channels change; never while it runs.
    pub(super) fn set_channels(&mut self, channels: usize, tracks: Vec<Option<usize>>) {
        self.channels = channels;
        self.track_channels = tracks;
        self.note_outputs.clear();
        self.note_outputs.resize(self.modulators.len().saturating_mul(channels), f32::NAN);
        self.voice_pool.set_channels(channels);
    }

    /// The channel a target belongs to, when it has one the note sources can read.
    fn channel_of(&self, target: Target) -> Option<usize> {
        match target {
            Target::ChannelParam(channel, _) => Some(usize::from(channel)),
            Target::MixerVolume(t)
            | Target::MixerPan(t)
            | Target::MixerSeparation(t)
            | Target::MixerEq(t, ..)
            | Target::MixerSend(t, _)
            | Target::SlotMix(t, _) => self.track_channels.get(usize::from(t)).copied().flatten(),
            _ => None,
        }
    }

    /// The `0..=1` value a modulator reads for `target`: its own, unless it is a note source and the target is on a
    /// channel, in which case it is that channel's note. A target with no channel reads the newest note.
    pub(super) fn output_at(&self, modulator: usize, target: Target) -> f32 {
        let Some(m) = self.modulators.get(modulator) else { return 0.5 };
        let own = self.outputs.get(modulator).copied().unwrap_or(0.5);
        match m.source {
            ModSource::Velocity(_) | ModSource::KeyTrack(_) => {
                let at = self
                    .channel_of(target)
                    .and_then(|c| self.note_outputs.get(modulator * self.channels + c))
                    .copied();
                match at {
                    Some(v) if v.is_finite() => v,
                    _ => self.notes.value_latest(&m.source),
                }
            }
            _ => own,
        }
    }

    fn follower(&self, id: u32) -> Option<&Follower> {
        self.followers.iter().find(|f| f.id == id)
    }

    /// The value of `source` (modulator `id`'s) at a position (`0..=1`; a follower reads its detector).
    fn value_of(&self, source: &ModSource, id: u32, beats: f64, secs: f64) -> f32 {
        match source {
            ModSource::AudioFollower(_) => self.follower(id).map_or(0.5, |f| f.value()),
            other => source_value(other, id, beats, secs),
        }
    }

    /// Fills `outputs` in dependency order: each modulator's own source, moved by the routes driving it, is evaluated
    /// after every modulator that drives it, so a chain (an LFO driving another's rate, which drives a third's depth)
    /// settles in one pass and a modulator driving its own parameter never happens (`ModGraph::route` refuses those).
    /// Allocates nothing: `order` and `outputs` were sized when the graph was installed.
    pub(super) fn evaluate(&mut self, beats: f64, secs: f64) {
        let snap = self.snapshot.clone();
        self.voice_release = 0.0;
        // Every depth starts the tick at what the user set: nothing builds on last tick's swing.
        for (i, m) in self.modulators.iter().enumerate() {
            let Some(start) = self.at.get(i).copied() else { break };
            for (k, t) in m.targets.iter().enumerate() {
                if let Some(d) = self.depths.get_mut(start + k) {
                    *d = t.depth;
                }
            }
        }
        for k in 0..self.order.len() {
            let j = self.order[k] as usize;
            // Routes into this modulator's own assignments: a modulator driving how far one of them goes. Their
            // drivers come before it, so their values are already this tick's.
            for i in 0..self.edges.len() {
                let e = self.edges[i];
                if e.to as usize != j || e.param.is_some() || !e.enabled {
                    continue;
                }
                let (Some(u), Some(depth)) = (self.outputs.get(e.from as usize).copied(), self.slot(e.from as usize, e.slot)) else {
                    continue;
                };
                let by = offset_values(depth, e.curve, e.polarity, u);
                if let Some(slot) = self.at.get(j).and_then(|start| self.depths.get_mut(start + e.target)) {
                    // A depth is a fraction of the parameter's range either way, so it moves like a normalized value:
                    // -1..=1 read as 0..=1, moved, and read back.
                    *slot = (((f64::from(*slot) + 1.0) / 2.0 + by).clamp(0.0, 1.0) * 2.0 - 1.0) as f32;
                }
            }
            let (id, mut source) = match self.modulators.get(j) {
                Some(m) => (m.id, m.source),
                None => continue,
            };
            for i in 0..self.edges.len() {
                let e = self.edges[i];
                if e.to as usize != j || e.from as usize == j || !e.enabled {
                    continue;
                }
                let Some(param) = e.param else { continue };
                let (Some(u), Some(depth)) = (self.outputs.get(e.from as usize).copied(), self.slot(e.from as usize, e.slot)) else {
                    continue;
                };
                apply_param(&mut source, param, offset_values(depth, e.curve, e.polarity, u));
            }
            // A note source reads one note per channel: every channel's value is read here, so a destination on a
            // channel takes its own channel's note rather than whichever channel played last.
            let note_source = matches!(source, ModSource::Velocity(_) | ModSource::KeyTrack(_));
            let u = if note_source {
                self.notes.value_latest(&source)
            } else if let ModSource::VoiceAdsr(adsr) = &source {
                self.voice_release = self.voice_release.max(adsr.release);
                self.voice_adsr_value(adsr, j)
            } else {
                self.value_of(&source, id, beats, secs)
            };
            if note_source && j * self.channels + self.channels <= self.note_outputs.len() {
                let base = j * self.channels;
                let notes = &self.notes;
                for c in 0..self.channels {
                    self.note_outputs[base + c] = notes.value(&source, c);
                }
            }
            if let Some(slot) = self.outputs.get_mut(j) {
                *slot = u;
            }
            if let Some(snap) = &snap {
                snap.write_output(j, u);
            }
        }
    }

    /// One assignment's effective depth (`None` for a target index the graph does not have).
    fn slot(&self, m: usize, k: usize) -> Option<f32> {
        self.at.get(m).and_then(|start| self.depths.get(start + k)).copied()
    }

    /// An assignment's depth as it stands after this tick's evaluation (its own depth moved by whatever drives it).
    fn depth_of(&self, m: usize, k: usize) -> f32 {
        self.slot(m, k).unwrap_or(0.0)
    }

    /// Feeds every follower the master output of the tick that just finished. Called once a tick, after the master's
    /// effects and main volume: the follower is one tick behind the value the tick's modulation used, which is
    /// inaudible at control rate and keeps the detector off the evaluation path.
    pub(super) fn feed_master(&mut self, samples: &[f32], rate: f32) {
        for f in &mut self.followers {
            f.feed(samples, rate);
        }
    }

    /// A VoiceAdsr modulator's output: the average envelope level of the voices on the channel of its first target
    /// that has one (else of every channel's voices; 0 with none sounding), mapped like the other sources.
    fn voice_adsr_value(&self, adsr: &VoiceAdsr, modulator: usize) -> f32 {
        let channel = self.modulators.get(modulator).and_then(|m| m.targets.iter().find_map(|t| self.channel_of(t.target)));
        let (sum, count) = self
            .voice_pool
            .channels()
            .filter(|pool| channel.is_none_or(|c| pool.channel() == c))
            .flat_map(|pool| pool.iter())
            .fold((0.0f32, 0u32), |(sum, n), (_, ctx)| (sum + adsr_level(adsr, ctx), n + 1));
        let level = if count == 0 { 0.0 } else { sum / count as f32 };
        let shaped = if adsr.bipolar { level } else { 0.5 + level * 0.5 };
        0.5 + (shaped - 0.5) * adsr.amount.clamp(0.0, 1.0)
    }

    /// Ages the note voices by `secs`; a released one stays for the longest release any envelope has now.
    pub(super) fn advance_voices(&mut self, secs: f32) {
        self.voice_pool.advance(secs, self.voice_release);
    }
}

/// One voice's envelope level (`0..=1`) at its age: a linear attack to 1, a linear decay to the sustain level, and
/// from the gate's fall a linear release from the level it had then.
fn adsr_level(adsr: &VoiceAdsr, ctx: &VoiceContext) -> f32 {
    let sustain = adsr.sustain.clamp(0.0, 1.0);
    let held = |t: f32| {
        if t < adsr.attack {
            t / adsr.attack
        } else if t - adsr.attack < adsr.decay {
            1.0 - (1.0 - sustain) * (t - adsr.attack) / adsr.decay
        } else {
            sustain
        }
    };
    if ctx.gate {
        return held(ctx.age);
    }
    let left = if adsr.release > 0.0 { 1.0 - (ctx.age - ctx.released_at) / adsr.release } else { 0.0 };
    held(ctx.released_at) * left.clamp(0.0, 1.0)
}

/// The source's value at a position, `0..=1`: `beats` from the transport's start (quarter notes) and `secs`.
pub fn source_value(source: &ModSource, id: u32, beats: f64, secs: f64) -> f32 {
    match source {
        ModSource::Lfo(l) => lfo_value(l, id, cycles(l.period, beats, secs) + f64::from(l.phase)),
        ModSource::Steps(s) => steps_value(s, cycles(s.period, beats, secs)),
        ModSource::Envelope(e) => envelope_value(e, secs),
        ModSource::Random(r) => random_value(r, cycles(r.period, beats, secs)),
        ModSource::Mseg(m) => mseg_value(m, cycles(m.period, beats, secs)),
        // A macro is where the user put it: no clock is involved at all.
        ModSource::Macro(m) => m.value.clamp(0.0, 1.0),
        // A follower's value comes from the master detector, not from the position: `Modulation::value` reads it.
        // With no detector (a preview of a stopped transport) it is the neutral middle, not the bottom.
        ModSource::AudioFollower(_) => 0.5,
        // Bound to a channel in the engine, not here: the newest note stands in for a target with no channel.
        ModSource::Velocity(_) | ModSource::KeyTrack(_) => 0.5,
        // VoiceAdsr is evaluated per-voice in the engine, not here.
        ModSource::VoiceAdsr(_) => 0.5,
        // Operator nodes read their inputs from other modulators through routes; with no clock-bound input here,
        // their neutral preview value is the middle.
        ModSource::Math(_) | ModSource::Polynomial(_) | ModSource::Quantize(_)
        | ModSource::Map(_) | ModSource::Slew(_) => 0.5,
    }
}

/// An MSEG's value at `cycles` (one cycle is one pass through the shape), `0..=1`.
///
/// The shape's points are normalized in time, so the rate scales the whole thing and a seek lands where the shape says.
/// A shape that does not loop holds its last point once the pass is over, which is what makes it an envelope rather
/// than an LFO.
fn mseg_value(m: &Mseg, cycles: f64) -> f32 {
    let at = if m.looped { cycles.rem_euclid(1.0) } else { cycles.clamp(0.0, 1.0) } as f32;
    let points = m.used_points();
    let first = points.first().copied().unwrap_or_default();
    let mut value = first.value;
    // The point the shape has reached: the last segment whose start is behind the position.
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        if at < a.at.max(0.0) {
            break;
        }
        if b.at <= a.at {
            // Two points on one instant: the later one wins from there on.
            value = b.value;
            continue;
        }
        let span = b.at - a.at;
        value = if at >= b.at {
            b.value
        } else if m.smooth {
            let t = (at - a.at) / span;
            a.value + (b.value - a.value) * t
        } else {
            a.value
        };
    }
    // The same reading of a -1..=1 value the LFO gives its wave: bipolar swings either side of the middle, unipolar
    // is lifted above it. The amount is a gain around the middle, applied after that, so 0 is a flat middle.
    let centered = (f64::from(value.clamp(-1.0, 1.0)) + 1.0) / 2.0;
    let shaped = if m.bipolar { centered } else { 0.5 + centered * 0.5 };
    let gained = 0.5 + (shaped - 0.5) * f64::from(m.amount.clamp(0.0, 1.0));
    gained.clamp(0.0, 1.0) as f32
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
    // The source's own output mode: bipolar swings either side of the middle (which the assignment reads as -1..1);
    // unipolar starts at the middle and only rises, so a bipolar assignment moves 0..depth above the base.
    let centered = (wave + 1.0) / 2.0;
    let shaped = if l.bipolar { centered } else { 0.5 + centered * 0.5 };
    // The amount is a gain around the middle applied after the output mode, so 0 is a flat line at the middle (no
    // modulation at all) whatever the shape and whichever mode, and turning it down shrinks the swing inwards.
    let gained = 0.5 + (shaped - 0.5) * f64::from(l.amount.clamp(0.0, 1.0));
    (gained + f64::from(l.offset)).clamp(0.0, 1.0) as f32
}

/// A random source's value at `cycles`, `0..=1`. Deterministic from the seed and the cycle number; `smooth` is a
/// slew from the previous cycle's value over the first fraction of the cycle, which is seek-safe.
fn random_value(r: &Random, cycles: f64) -> f32 {
    let cycle = cycles.floor() as i64;
    let frac = cycles.rem_euclid(1.0) as f32;
    let cur = random_at(r.seed, cycle, r.distribution);
    let prev = random_at(r.seed, cycle - 1, r.distribution);
    let u = if r.interpolate {
        prev + (cur - prev) * frac
    } else if r.smooth > 0.0005 {
        let k = (frac / r.smooth.clamp(0.0, 1.0)).min(1.0);
        prev + (cur - prev) * k
    } else {
        cur
    };
    let centered = if r.bipolar { u } else { 0.5 + u * 0.5 };
    // The amount is a gain around the middle, so 0 is the middle (no modulation) rather than the bottom of the range.
    (0.5 + (centered - 0.5) * r.amount.clamp(0.0, 1.0)).clamp(0.0, 1.0)
}

/// A random draw in `0..1` for `cycle` from `seed`, in the source's distribution.
fn random_at(seed: u64, cycle: i64, distribution: Distribution) -> f32 {
    let base = seed ^ (cycle as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    match distribution {
        Distribution::Uniform => hash_unit(base),
        // The mean of four draws is a cheap, bounded bell: most values near the middle, no tails past either end.
        Distribution::Gaussian => {
            let sum: f32 = (0..4u64).map(|i| hash_unit(base ^ (i + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15))).sum();
            (sum / 4.0).clamp(0.0, 1.0)
        }
    }
}

/// A hash's value in `0..1`.
fn hash_unit(mut x: u64) -> f32 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    (x >> 11) as f32 / (1u64 << 53) as f32
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
    offset_values(t.depth, t.curve, t.polarity, u)
}

/// The same for a route's own numbers (the graph's edges carry them without a whole `ModTarget`).
fn offset_values(depth: f32, curve: Curve, polarity: Polarity, u: f32) -> f64 {
    let shaped = f64::from(curved(curve, u));
    let signed = match polarity {
        Polarity::Bipolar => shaped * 2.0 - 1.0,
        Polarity::Unipolar => shaped,
    };
    f64::from(depth) * signed
}

/// Moves one parameter of `source` by `by` (`-1..=1` of the parameter's own range; the route's depth already applied).
///
/// A rate is scaled rather than added: `by` multiplies the period by `2^(4 by)`, so a full-depth route on a slow
/// modulator moves it by two octaves up or down in hertz and the motion is musically even. A sync period is a count of
/// sixteenths, so it divides instead and never reaches zero. A follower's attack and release are times, so they scale
/// the same way (a full-depth route is two octaves of time either side) and are kept inside the range the editor's knob
/// covers, rather than running into a negative time constant. A gain is a level, so it moves in
/// decibels over its own 48 dB range, and a smoothing amount is a fraction of the cycle, so it is added. A parameter
/// the source does not have is left alone — the graph refuses to route to one
/// ([`os_model::modulation::ModGraph::route`]), so this only guards hand-made graphs.
fn apply_param(source: &mut ModSource, param: ModParam, by: f64) {
    let scale = 2f64.powf(4.0 * by);
    let add = |own: f32| (f64::from(own) + by).clamp(-1.0, 1.0) as f32;
    match source {
        ModSource::Lfo(l) => match param {
            ModParam::Rate => l.period = scale_period(l.period, scale),
            ModParam::Amount => l.amount = add(l.amount).clamp(0.0, 1.0),
            ModParam::Phase => l.phase = (f64::from(l.phase) + by).rem_euclid(1.0) as f32,
            ModParam::Offset => l.offset = add(l.offset),
            _ => {}
        },
        ModSource::Random(r) => match param {
            ModParam::Rate => r.period = scale_period(r.period, scale),
            ModParam::Amount => r.amount = add(r.amount).clamp(0.0, 1.0),
            ModParam::Smooth => r.smooth = add(r.smooth).clamp(0.0, 1.0),
            _ => {}
        },
        ModSource::Steps(s) => match param {
            ModParam::Rate => s.period = scale_period(s.period, scale),
            _ => {}
        },
        ModSource::AudioFollower(f) => match param {
            ModParam::Amount => f.amount = add(f.amount).clamp(0.0, 1.0),
            // The editor's own ranges: attack to 2 s, release to 10 s, gain ±24 dB.
            ModParam::Attack => f.attack = scale_time(f.attack, scale, 0.0005, 2.0),
            ModParam::Release => f.release = scale_time(f.release, scale, 0.001, 10.0),
            ModParam::Gain => f.gain_db = (f64::from(f.gain_db) + by * 48.0).clamp(-24.0, 24.0) as f32,
            _ => {}
        },
        ModSource::Mseg(m) => match param {
            ModParam::Rate => m.period = scale_period(m.period, scale),
            ModParam::Amount => m.amount = add(m.amount).clamp(0.0, 1.0),
            _ => {}
        },
        // A macro's position is the one thing another modulator can move: it is the source's whole output.
        ModSource::Macro(m) => {
            if param == ModParam::Value {
                m.value = add(m.value).clamp(0.0, 1.0);
            }
        }
        // A note source's amount is its only setting (what it reads is the channel's note, not a parameter).
        ModSource::Velocity(v) => {
            if param == ModParam::Amount {
                v.amount = add(v.amount).clamp(0.0, 1.0);
            }
        }
        ModSource::KeyTrack(k) => {
            if param == ModParam::Amount {
                k.amount = add(k.amount).clamp(0.0, 1.0);
            }
        }
        ModSource::VoiceAdsr(v) => match param {
            ModParam::AttackV => v.attack = scale_time(v.attack, scale, 0.0005, 30.0),
            ModParam::Decay => v.decay = scale_time(v.decay, scale, 0.0005, 30.0),
            ModParam::Sustain => v.sustain = add(v.sustain).clamp(0.0, 1.0),
            ModParam::ReleaseV => v.release = scale_time(v.release, scale, 0.001, 60.0),
            ModParam::Amount => v.amount = add(v.amount).clamp(0.0, 1.0),
            _ => {}
        },
        ModSource::Envelope(_) => {}
        ModSource::Math(m) => {
            if param == ModParam::Amount {
                m.amount = add(m.amount).clamp(0.0, 1.0);
            }
            // MathOp is a discrete selector; it is edited in the UI, not swept by a modulation route.
        }
        ModSource::Polynomial(m) => match param {
            ModParam::PolyCoeff(i) => {
                if let Some(c) = m.coeffs.get_mut(usize::from(i)) {
                    *c = add(*c);
                }
            }
            ModParam::Amount => m.amount = add(m.amount).clamp(0.0, 1.0),
            _ => {}
        },
        ModSource::Quantize(m) => match param {
            ModParam::QuantSteps => {
                m.steps = (f64::from(m.steps) + by * 12.0).clamp(2.0, 128.0).round() as u8;
            }
            ModParam::Amount => m.amount = add(m.amount).clamp(0.0, 1.0),
            _ => {}
        },
        ModSource::Map(m) => match param {
            ModParam::MapPoint(i) => {
                if let Some(p) = m.points.get_mut(usize::from(i)) {
                    p.y = add(p.y).clamp(0.0, 1.0);
                }
            }
            ModParam::Amount => m.amount = add(m.amount).clamp(0.0, 1.0),
            _ => {}
        },
        ModSource::Slew(m) => match param {
            ModParam::SlewRise => m.rise = scale_time(m.rise, scale, 0.0, 10.0),
            ModParam::SlewFall => m.fall = scale_time(m.fall, scale, 0.0, 10.0),
            ModParam::Amount => m.amount = add(m.amount).clamp(0.0, 1.0),
            _ => {}
        },
    }
}

/// A time constant scaled by `scale` (a route's `2^(4 by)`) and kept inside the range the editor's knob covers, so a
/// routed attack cannot reach a negative or silent time.
fn scale_time(seconds: f32, scale: f64, min: f32, max: f32) -> f32 {
    (f64::from(seconds) * scale).clamp(f64::from(min), f64::from(max)) as f32
}

/// A period scaled by `scale`: free hertz multiply (clamped to an audible-to-slow buzzer range), synced sixteenths
/// divide and round, never below one.
fn scale_period(period: Timebase, scale: f64) -> Timebase {
    match period {
        Timebase::Free(hz) => Timebase::Free((f64::from(hz) * scale).clamp(0.001, 100.0) as f32),
        Timebase::Sync(sixteenths) => {
            Timebase::Sync(((f64::from(sixteenths) / scale).round() as i64).clamp(1, i64::from(u16::MAX)) as u16)
        }
    }
}

/// The modulators' indices in dependency order: each after every modulator that drives it. Kahn's algorithm over the
/// graph's own edges, in graph order so the result is stable. A graph with a loop (only possible through hand-written
/// data) leaves its members out of the ordered part; they are appended in graph order, so nothing stops evaluating.
fn topological(modulators: &[Modulator], edges: &[Edge]) -> Vec<u32> {
    let n = modulators.len();
    let mut order = Vec::with_capacity(n);
    let mut done = vec![false; n];
    loop {
        let mut progressed = false;
        for j in 0..n {
            if done[j] {
                continue;
            }
            let driven = edges.iter().any(|e| e.to as usize == j && e.from as usize != j && !done[e.from as usize]);
            if !driven {
                done[j] = true;
                order.push(j as u32);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    for (j, d) in done.iter().enumerate() {
        if !d {
            order.push(j as u32);
        }
    }
    order
}

/// Adds one step's modulated values, after the clips' values of the same step: every parameter's base (what a clip
/// handed it at this step, else its own value — [`DEFAULT_BASE`] until one is set) carried by each enabled modulator's
/// offset. Called from the automation pass so that a value and its modulation are the two halves of one step, and from
/// [`Engine::run_modulation`] when no clip drives parameters at all.
pub(super) fn step_values(params: &mut ParamAuto, mods: &mut Modulation, pos: f64, at: u32, secs_per_tick: f64, ppq: f64) {
    let (beats, secs) = (pos / ppq, pos * secs_per_tick);
    // Every modulator's value, in dependency order: a route into another modulator is applied here, so the value of a
    // driven modulator already includes what drives it.
    mods.evaluate(beats, secs);
    let snap = mods.snapshot.clone();
    // Every modulator's offset for a parameter adds up, then the parameter's own base carries them. The scratch is the
    // graph's own: a tick allocates nothing.
    let mut offsets = std::mem::take(&mut mods.by);
    offsets.clear();
    for (i, m) in mods.modulators.iter().enumerate().filter(|(_, m)| m.enabled) {
        for (k, t) in m.targets.iter().enumerate().filter(|(_, t)| t.enabled) {
            // A route inside the graph is not a project parameter: the engine already applied it while evaluating.
            if matches!(t.target, Target::ModulatorParam(..) | Target::RouteDepth(..)) {
                continue;
            }
            let u = mods.output_at(i, t.target);
            let by = offset_values(mods.depth_of(i, k), t.curve, t.polarity, u);
            match offsets.iter_mut().find(|(x, _)| *x == t.target) {
                Some((_, v)) => *v += by,
                None => offsets.push((t.target, by)),
            }
        }
    }
    for &(target, by) in offsets.iter() {
        let base = params.base_of(target).unwrap_or(DEFAULT_BASE);
        let value = (base + by).clamp(0.0, 1.0);
        if let Some(snap) = &snap {
            snap.write_value(target, value);
            snap.write_base(target, base);
        }
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
    mods.by = offsets;
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
    /// The live snapshot the UI reads; the app builds it with the graph so both describe the same modulators and targets.
    pub fn set_mod_snapshot(&mut self, snapshot: Arc<ModSnapshot>) {
        self.mods.snapshot = Some(snapshot);
    }

    /// The value each driven parameter has when its modulation is taken away: the app sends these with the graph
    /// (`Transport::set_mod_bases`), because a parameter nothing automates has no base of its own and a modulation would
    /// otherwise move it from halfway. A clip or control point that drives the parameter later still wins
    /// (`ParamAuto::set_base`), and so does a knob the user turns, which the app hands over the same way.
    pub fn set_mod_bases(&mut self, bases: &[(u32, f64)]) {
        for &(target, value) in bases {
            let target = os_model::target::decode(target);
            if super::param_auto::is_param(target) {
                self.params.set_base(target, value.clamp(0.0, 1.0));
            }
        }
    }

    /// Installs the modulation graph (an empty one: no modulation). Values are sent again from scratch, and every
    /// track gives its host parameters back.
    pub fn set_modulation(&mut self, modulators: Vec<Modulator>) {
        self.mods.set(modulators);
        self.refresh_note_subjects();
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

    /// Tells the graph how many channels there are and which one each mixer track plays through, so a note source
    /// finds its note for a target on a channel or a track. Called when the graph is installed and after the project's
    /// tracks or channels change.
    pub fn refresh_note_subjects(&mut self) {
        let channels = self.channels.len();
        let tracks = self
            .tracks
            .iter()
            .map(|t| usize::try_from(t.output).ok().filter(|&c| c < channels))
            .collect();
        self.mods.set_channels(channels, tracks);
    }

    /// The tick's modulated mixer values: a track's fader, pan and separation, sampled once a tick (a fader value is
    /// read by the mixer, not handed to a plugin) and held in `MixTrack::mod_*`, so the authored (or automated) value
    /// stays the base. A track the graph no longer drives gets its own value back.
    pub(super) fn run_host_modulation(&mut self) {
        if self.mods.is_empty() {
            return;
        }
        let now = self.mod_pos();
        let ppq = f64::from(self.ppq.max(1));
        let secs_per_tick = self.tick_len() as f64 / f64::from(self.rate.max(1));
        let (beats, secs) = (now / ppq, now * secs_per_tick);
        self.mods.evaluate(beats, secs);
        let snap = self.mods.snapshot.clone();
        let mut offsets = std::mem::take(&mut self.mods.host_by);
        offsets.clear();
        for (i, m) in self.mods.modulators.iter().enumerate().filter(|(_, m)| m.enabled) {
            for (k, t) in m.targets.iter().enumerate().filter(|(_, t)| t.enabled) {
                let Some((track, param)) = host_target(t.target) else { continue };
                let u = self.mods.output_at(i, t.target);
                let by = offset_values(self.mods.depth_of(i, k), t.curve, t.polarity, u);
                match offsets.iter_mut().find(|(x, p, ..)| *x == track && *p == param) {
                    Some((_, _, _, v)) => *v += by,
                    None => offsets.push((track, param, t.target, by)),
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
            let driven = |param: HostParam| offsets.iter().find(|(t, p, ..)| *t == track && *p == param).map(|(_, _, _, by)| *by);
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
        // Publish every driven host target's effective normalized value (the units differ per control; the UI's cursor
        // wants them all on 0..1, so each is put back into the domain its base is read in).
        if let Some(snap) = &snap {
            for &(track, param, target, by) in &offsets {
                let Some(m) = self.tracks.get(track) else { continue };
                let base = match param {
                    HostParam::Volume => f64::from(m.vol) / 16000.0,
                    HostParam::Pan => (f64::from(m.pan) + 6400.0) / 12800.0,
                    HostParam::Separation => (f64::from(m.sep) + 64.0) / 128.0,
                    HostParam::Eq(band, field) => {
                        let own = f64::from(m.eq_own[usize::from(band)][usize::from(field)]);
                        if field == 0 {
                            (own / 1800.0 + 1.0) / 2.0
                        } else {
                            own / 65536.0
                        }
                    }
                    HostParam::Send(d) => m.routes.iter().find(|r| r.dest as u16 == d).map_or(0.0, |r| f64::from(r.level)),
                    HostParam::SlotMix(k) => f64::from(m.slot_mix[usize::from(k)]) / 12800.0,
                };
                snap.write_value(target, (base + by).clamp(0.0, 1.0));
            }
        }
        // The scratch goes back to the graph with its capacity: the next tick reuses it instead of allocating.
        self.mods.host_by = offsets;
    }

    /// Samples every modulator over the `frames` frames of the tick starting now: the path when no clip drives a
    /// parameter (the automation pass calls [`step_values`] itself, so a value and its modulation stay one step).
    pub(super) fn run_modulation(&mut self, frames: usize) {
        if self.mods.is_empty() {
            return;
        }
        let len = frames.max(1) as f64;
        let now = self.mod_pos();
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

    /// The follower's band must actually reject signal below its low edge: a 100 Hz tone inside the 20–200 Hz low
    /// band reads far higher than a 5 Hz tone the high-passes remove. Regression: both high-pass stages once shared
    /// one previous-input value, so the second stage subtracted the raw sample history again and leaked unfiltered
    /// signal past the band.
    #[test]
    fn a_followers_low_band_rejects_what_is_below_it() {
        fn level(hz: f32) -> f32 {
            let mut f = super::Follower::new(
                1,
                os_model::modulation::AudioFollower {
                    band: os_model::modulation::Band::Low,
                    attack: 0.0005,
                    release: 0.0005,
                    ..Default::default()
                },
            );
            let rate = 48_000.0f32;
            let mut phase = 0.0f32;
            for _ in 0..200 {
                let mut buf = Vec::with_capacity(512 * 2);
                for _ in 0..512 {
                    let s = 0.5 * (phase * std::f32::consts::TAU).sin();
                    phase = (phase + hz / rate).fract();
                    buf.push(s);
                    buf.push(s);
                }
                f.feed(&buf, rate);
            }
            f.level
        }
        let low = level(5.0);
        let mid = level(100.0);
        assert!(
            mid > low * 3.0,
            "the low band must reject 5 Hz far more than 100 Hz (5 Hz {low:.4}, 100 Hz {mid:.4})"
        );
    }

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
            source: ModSource::Lfo(Lfo { shape, period: Timebase::Sync(sixteenths), ..Lfo::default() }),
            enabled: true,
            targets: vec![ModTarget { target: Target::ChannelParam(0, 7), depth, curve: Curve::Linear, polarity, enabled: true }],
        }
    }

    /// Each stage of a voice's envelope, in seconds, so it is the same at any sample rate and tick length.
    #[test]
    fn a_voice_envelope_has_four_stages_at_any_rate() {
        let adsr = VoiceAdsr { attack: 0.1, decay: 0.2, sustain: 0.5, release: 0.4, amount: 1.0, bipolar: true };
        for (rate, frames) in [(44100.0f32, 441usize), (96000.0, 960)] {
            let mut pool = VoicePool::new(1);
            pool.note_on(0, 60, 100);
            // Steps of 10 ms, up to step `to`.
            let mut done = 0;
            let mut at = |pool: &mut VoicePool, to: usize| {
                while done < to {
                    pool.advance(frames as f32 / rate, adsr.release);
                    done += 1;
                }
                pool.channel(0).unwrap().get(1).map(|ctx| adsr_level(&adsr, ctx))
            };
            let near = |got: Option<f32>, want: f32| assert!(got.is_some_and(|g| (g - want).abs() < 1e-3), "{rate}: {got:?} for {want}");
            near(at(&mut pool, 5), 0.5);
            near(at(&mut pool, 20), 0.75);
            near(at(&mut pool, 50), 0.5);
            pool.note_off(0, 60);
            near(at(&mut pool, 70), 0.25);
            assert_eq!(at(&mut pool, 95), None, "{rate}: gone after the release");
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

    /// A modulator drives another modulator's own parameter: a square LFO at the top lets the second LFO move the
    /// parameter, and at the bottom takes its amount away, so the parameter swings for one half of the cycle and sits
    /// still for the other. The driven modulator is listed *first*, so the pass has to follow the route rather than the
    /// list, and without the route the second LFO's amount of zero keeps the parameter still all cycle.
    #[test]
    fn a_modulator_drives_another_modulators_parameter() {
        fn values(route: bool) -> Vec<(u64, f64)> {
            let p = project(0);
            let mut e = Engine::new(&p, &mut SampleLibrary::new(None), 48000, PlayMode::Pattern(1));
            let probe = Arc::new(Mutex::new(Probe::default()));
            e.set_instrument(0, probe.clone());
            let driven = Modulator {
                id: 2,
                source: ModSource::Lfo(Lfo { shape: LfoShape::Sine, period: Timebase::Sync(4), amount: 0.0, ..Lfo::default() }),
                enabled: true,
                targets: vec![ModTarget {
                    target: Target::ChannelParam(0, 7),
                    depth: 1.0,
                    curve: Curve::Linear,
                    polarity: Polarity::Bipolar,
                    enabled: true,
                }],
            };
            let driver = Modulator {
                id: 1,
                source: ModSource::Lfo(Lfo { shape: LfoShape::Square, period: Timebase::Sync(4), ..Lfo::default() }),
                enabled: true,
                targets: if route {
                    vec![ModTarget {
                        target: Target::ModulatorParam(2, ModParam::Amount.code()),
                        depth: 1.0,
                        curve: Curve::Linear,
                        polarity: Polarity::Unipolar,
                        enabled: true,
                    }]
                } else {
                    Vec::new()
                },
            };
            e.set_modulation(vec![driven, driver]);
            run(&mut e, 24000, 1000);
            let seen = probe.lock().unwrap().seen.clone();
            seen.into_iter().map(|(f, _, v)| (f, v)).collect()
        }
        let routed = values(true);
        let early: Vec<f64> = routed.iter().filter(|(f, _)| *f < 12000).map(|&(_, v)| v).collect();
        assert!(early.iter().cloned().fold(0.0f64, f64::max) > 0.9, "the driven LFO swings up while its driver is: {:?}", early.iter().cloned().fold(0.0f64, f64::max));
        assert!(
            routed.iter().filter(|&&(f, _)| f >= 12000).all(|&(_, v)| (v - 0.5).abs() < 1e-6),
            "and the driver's low half leaves it in the middle: {:?}",
            routed.iter().filter(|&&(f, _)| f >= 12000).collect::<Vec<_>>()
        );
        let unrouted = values(false);
        assert!(
            unrouted.iter().all(|&(frame, v)| frame >= 12000 || (v - 0.5).abs() < 1e-6),
            "without the route nothing moves at all: {unrouted:?}"
        );
    }

    /// A route into a modulator's own rate shows in the output the UI plots: the driven LFO's published snapshot output
    /// moves at the rate its driver gives it, not at its own. Regression guard for the tile strip's live plot.
    #[test]
    fn a_driven_modulators_output_follows_its_route() {
        fn outputs(route: bool) -> Vec<f32> {
            let p = project(0);
            let mut e = Engine::new(&p, &mut SampleLibrary::new(None), 48000, PlayMode::Pattern(1));
            let lfo = Modulator {
                id: 2,
                source: ModSource::Lfo(Lfo { shape: LfoShape::Sine, period: Timebase::Sync(4), ..Lfo::default() }),
                enabled: true,
                targets: vec![ModTarget { target: Target::ChannelParam(0, 7), depth: 0.5, curve: Curve::Linear, polarity: Polarity::Bipolar, enabled: true }],
            };
            // A step sequence held at its top: a deterministic driver, so the routed run's rate is a known 16x the
            // LFO's own (a Random drives it the same way, one value a cycle).
            let driver = Modulator {
                id: 1,
                source: ModSource::Steps(Steps { values: [100; 16], period: Timebase::Sync(4), smooth: false }),
                enabled: true,
                targets: if route {
                    vec![ModTarget { target: Target::ModulatorParam(2, ModParam::Rate.code()), depth: 1.0, curve: Curve::Linear, polarity: Polarity::Bipolar, enabled: true }]
                } else {
                    Vec::new()
                },
            };
            let snap = Arc::new(ModSnapshot::new(&[2, 1], &[Target::ChannelParam(0, 7)]));
            e.set_mod_snapshot(snap.clone());
            e.set_modulation(vec![lfo, driver]);
            let mut out = vec![0.0f32; 400 * 2];
            (0..48)
                .filter_map(|_| {
                    e.process(&mut out);
                    snap.output(2)
                })
                .collect()
        }
        let own = outputs(false);
        let driven = outputs(true);
        assert!(own.len() > 10 && own.len() == driven.len(), "both runs published: {} / {}", own.len(), driven.len());
        assert!(
            own.iter().zip(driven.iter()).any(|(a, b)| (a - b).abs() > 0.01),
            "the driven LFO's output changes with the route: {own:?} / {driven:?}"
        );
    }

    /// A modulator moves the value the clips hand the parameter, not a value of its own: a step sequence at its top,
    /// bipolar, a quarter of the range, sits a quarter above the ramp (and stops at the top), and moves from the
    /// parameter's own value again once the clip has passed it.
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
            targets: vec![ModTarget { target: Target::ChannelParam(0, 7), depth: 0.25, curve: Curve::Linear, polarity: Polarity::Bipolar, enabled: true }],
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
        for &(f, v) in last_at.iter().filter(|&&(f, _)| f < 96000) {
            let base = ((f + 250) as f64 / 96000.0).min(1.0);
            let want = (base + 0.25).min(1.0);
            assert!((v - want).abs() < 1e-6, "frame {f}: {v} vs {want} (the ramp's {base})");
        }
        // The ramp reaches 0.75 after three of its four beats: from there the parameter sits at its top while the clip
        // drives it.
        assert!(last_at.iter().filter(|&&(f, _)| (72000..96000).contains(&f)).all(|&(_, v)| (v - 1.0).abs() < 1e-9));
        assert!(last_at.iter().any(|&(f, v)| (72000..96000).contains(&f) && (v - 1.0).abs() < 1e-9));
        assert!(last_at.iter().filter(|&&(f, _)| f < 70000).all(|&(_, v)| v < 1.0), "the ramp's own headroom stays");
        // Past the clip's end nothing drives the parameter, so the swing moves from its own value again: halfway, since
        // no knob was handed over.
        assert!(
            last_at.iter().filter(|&&(f, _)| f >= 96000).all(|&(_, v)| (v - 0.75).abs() < 1e-9),
            "the clip lets go: {:?}",
            last_at.iter().filter(|&&(f, _)| f >= 96000).collect::<Vec<_>>()
        );
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
            targets: vec![ModTarget { target: Target::MixerVolume(1), depth: 1.0, curve: Curve::Linear, polarity: Polarity::Unipolar, enabled: true }],
        };
        e.set_modulation(vec![off, host]);
        run(&mut e, 24000, 1000);
        assert!(probe.lock().unwrap().seen.is_empty());
    }

    /// Pausing holds the song and its automation, not the modulation: a mapped fader keeps sweeping on the held clock,
    /// so a modulator can be set up and watched with the transport stopped (and its values still reach the effects).
    #[test]
    fn modulation_runs_while_the_transport_is_paused() {
        let p = project(0);
        let mut e = Engine::new(&p, &mut SampleLibrary::new(None), 48000, PlayMode::Pattern(1));
        // A step sequence that walks up and down, so the fader has somewhere to go.
        let steps = Steps { values: [0, 20, 40, 60, 80, 100, 80, 60, 40, 20, 0, 20, 40, 60, 80, 100], period: Timebase::Sync(1), smooth: false };
        e.set_modulation(vec![Modulator {
            id: 1,
            source: ModSource::Steps(steps),
            enabled: true,
            targets: vec![ModTarget { target: Target::MixerVolume(1), depth: 0.5, curve: Curve::Linear, polarity: Polarity::Bipolar, enabled: true }],
        }]);
        e.pause();
        let mut seen = Vec::new();
        // The pause fades the channels out over a few ticks first; then they are held.
        for _ in 0..40 {
            e.render_tick();
            if let Some(v) = e.tracks[1].mod_vol {
                seen.push(v);
            }
        }
        assert!(!seen.is_empty(), "the fader is driven while the transport is held");
        assert!(seen.windows(2).any(|w| w[0] != w[1]), "and keeps moving: {seen:?}");
        assert_eq!(e.tracks[1].vol, 12800, "the authored fader is still the base");
        // Playback goes on: modulation carries on from the song's own position.
        e.resume();
        e.render_tick();
        assert!(e.tracks[1].mod_vol.is_some(), "still driven on the way back in");
    }

    /// The shapes read at their own points: a sine starts halfway and peaks a quarter in, a triangle runs its full
    /// swing, the saws go one way, the square is a switch.
    #[test]
    fn the_lfo_shapes() {
        let l = |shape| Lfo { shape, period: Timebase::Free(1.0), ..Lfo::default() };
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

    /// A modulated parameter moves from the value it has without the modulation (`Engine::set_mod_bases`): the engine
    /// knows a base only for what a clip or a control point gave it, so without one a parameter nothing automates would
    /// swing from halfway instead of from where its knob is.
    #[test]
    fn a_modulated_parameter_moves_from_its_own_value() {
        let value = |bases: bool| {
            let p = project(0);
            let mut e = Engine::new(&p, &mut SampleLibrary::new(None), 48000, PlayMode::Pattern(1));
            let probe = Arc::new(Mutex::new(Probe::default()));
            e.set_instrument(0, probe.clone());
            let steps = Steps { values: [100; 16], period: Timebase::Sync(1), smooth: false };
            e.set_modulation(vec![Modulator {
                id: 1,
                source: ModSource::Steps(steps),
                enabled: true,
                targets: vec![ModTarget {
                    target: Target::ChannelParam(0, 7),
                    depth: 0.1,
                    curve: Curve::Linear,
                    polarity: Polarity::Bipolar,
                    enabled: true,
                }],
            }]);
            if bases {
                e.set_mod_bases(&[(os_model::target::channel_param(0, 7), 0.8)]);
            }
            run(&mut e, 4000, 1000);
            probe.lock().unwrap().seen.last().map(|&(_, _, v)| v)
        };
        assert!((value(true).expect("a value reached the instrument") - 0.9).abs() < 1e-6, "0.8 plus a tenth of the range");
        assert!((value(false).expect("a value reached the instrument") - 0.6).abs() < 1e-6, "and halfway without a base");
    }

    /// A source's amount is a gain around the middle, applied after its own output mode: at 0 the source is switched
    /// off (a flat middle at every point of its cycle, whatever its shape and whichever mode), and turning it down
    /// shrinks the swing inwards instead of flattening it against an edge.
    #[test]
    fn the_amount_shrinks_the_swing_to_nothing() {
        let sine = |amount, bipolar| Lfo { shape: LfoShape::Sine, period: Timebase::Free(1.0), amount, bipolar, ..Lfo::default() };
        // Bipolar: the whole range at full amount, halfway either side of the middle at half.
        let (top, bottom) = (lfo_value(&sine(1.0, true), 1, 0.25), lfo_value(&sine(1.0, true), 1, 0.75));
        assert!((top - 1.0).abs() < 1e-6 && bottom.abs() < 1e-6, "full amount: {top} / {bottom}");
        let (top, bottom) = (lfo_value(&sine(0.5, true), 1, 0.25), lfo_value(&sine(0.5, true), 1, 0.75));
        assert!((top - 0.75).abs() < 1e-6 && (bottom - 0.25).abs() < 1e-6, "half amount: {top} / {bottom}");
        // Zero: the middle at every point of the cycle, in either mode and whatever the shape.
        for bipolar in [true, false] {
            for shape in [LfoShape::Sine, LfoShape::Triangle, LfoShape::SawUp, LfoShape::Square] {
                for cycles in [0.0, 0.1, 0.25, 0.5, 0.9] {
                    let l = Lfo { shape, amount: 0.0, bipolar, ..sine(0.0, bipolar) };
                    let v = lfo_value(&l, 1, cycles);
                    assert!((v - 0.5).abs() < 1e-6, "amount 0 is the middle ({shape:?}, bipolar {bipolar}, {cycles}): {v}");
                }
            }
        }
        // Unipolar stays above the middle and shrinks toward it, not toward the top of the range.
        assert!((lfo_value(&sine(1.0, false), 1, 0.75) - 0.5).abs() < 1e-6);
        assert!((lfo_value(&sine(0.5, false), 1, 0.75) - 0.5).abs() < 1e-6);
        assert!((lfo_value(&sine(0.5, false), 1, 0.25) - 0.75).abs() < 1e-6);
        // The offset still moves the result afterwards, so an offset source at no amount sits where its offset says.
        let l = Lfo { offset: 0.25, ..sine(0.0, true) };
        assert!((lfo_value(&l, 1, 0.25) - 0.75).abs() < 1e-6);
        // A random source, in both modes, and a follower: the same middle at zero.
        for bipolar in [true, false] {
            let r = Random { seed: 7, period: Timebase::Sync(4), amount: 0.0, bipolar, ..Random::default() };
            for c in [0.0, 1.0, 2.5, 9.0] {
                assert!((random_value(&r, c) - 0.5).abs() < 1e-6, "a random source at amount 0 is the middle (bipolar {bipolar})");
            }
        }
        let follower = |amount, level| {
            let mut f = super::Follower::new(1, AudioFollower { amount, ..AudioFollower::default() });
            f.level = level;
            f.value()
        };
        assert_eq!(follower(0.0, 1.0), 0.5, "an amount of 0 drives nothing whatever the detected level");
        assert_eq!(follower(0.0, 0.0), 0.5);
        assert!((follower(1.0, 1.0) - 1.0).abs() < 1e-6 && follower(1.0, 0.0).abs() < 1e-6);
        assert!((follower(0.5, 1.0) - 0.75).abs() < 1e-6, "half the swing: {}", follower(0.5, 1.0));
    }

    /// The random shape holds one value a cycle, changes from cycle to cycle, and gives the same value again on a
    /// second read (a render equals playback; a seek lands where it would have).
    #[test]
    fn the_random_shape_holds_and_repeats() {
        let l = Lfo { shape: LfoShape::Random, period: Timebase::Free(2.0), ..Lfo::default() };
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
        let t = |curve, polarity| ModTarget { target: Target::ChannelParam(0, 0), depth: 0.5, curve, polarity, enabled: true };
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
            targets: vec![ModTarget { target: Target::MixerVolume(1), depth: 0.5, curve: Curve::Linear, polarity: Polarity::Bipolar, enabled: true }],
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
        let target = |target, depth, polarity| ModTarget { target, depth, curve: Curve::Linear, polarity, enabled: true };
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
        let target = |target| ModTarget { target, depth: 0.5, curve: Curve::Linear, polarity: Polarity::Bipolar, enabled: true };
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

    /// A follower's own parameters take routes like a device parameter does: an attack and a release scale in time (two
    /// octaves at a quarter depth, kept inside the knob's range), a gain moves in decibels over its 48 dB span, and a
    /// random source's smoothing is a fraction of its cycle. A parameter the source does not have (
    /// an LFO's attack) is left alone.
    #[test]
    fn a_route_moves_a_followers_own_parameters_and_a_random_sources_smoothing() {
        let mut follower = ModSource::AudioFollower(AudioFollower { attack: 0.1, release: 1.0, gain_db: 0.0, ..AudioFollower::default() });
        apply_param(&mut follower, ModParam::Attack, 0.25);
        apply_param(&mut follower, ModParam::Release, -0.25);
        apply_param(&mut follower, ModParam::Gain, 0.25);
        let ModSource::AudioFollower(f) = follower else { panic!("a follower") };
        assert!((f.attack - 0.2).abs() < 1e-6, "a quarter depth doubles the attack: {}", f.attack);
        assert!((f.release - 0.5).abs() < 1e-6, "and halves the release: {}", f.release);
        assert!((f.gain_db - 12.0).abs() < 1e-6, "a quarter of the 48 dB span: {}", f.gain_db);
        assert!((f.amount - 1.0).abs() < 1e-6, "its amount is not touched");
        // A full depth either way is two octaves, and the knob's limits still hold: a time never goes negative, a gain
        // never passes the knob's +24 dB.
        apply_param(&mut follower, ModParam::Attack, -1.0);
        apply_param(&mut follower, ModParam::Gain, 1.0);
        let ModSource::AudioFollower(f) = follower else { panic!("a follower") };
        assert!((f.attack - 0.0125).abs() < 1e-7, "a sixteenth of 0.2 s: {}", f.attack);
        assert_eq!(f.gain_db, 24.0);
        let mut short = ModSource::AudioFollower(AudioFollower { attack: 0.0005, ..AudioFollower::default() });
        apply_param(&mut short, ModParam::Attack, -1.0);
        assert!(matches!(short, ModSource::AudioFollower(f) if f.attack == 0.0005), "kept at the knob's shortest attack");
        // A random source's smoothing: added, inside 0..1.
        let mut random = ModSource::Random(Random { smooth: 0.0, ..Random::default() });
        apply_param(&mut random, ModParam::Smooth, 0.5);
        assert!(matches!(random, ModSource::Random(r) if (r.smooth - 0.5).abs() < 1e-6));
        apply_param(&mut random, ModParam::Smooth, 1.0);
        assert!(matches!(random, ModSource::Random(r) if r.smooth == 1.0), "clamped to a whole cycle");
        // A parameter the source does not have changes nothing at all.
        let lfo = ModSource::Lfo(Lfo::default());
        let mut moved = lfo;
        apply_param(&mut moved, ModParam::Attack, 1.0);
        apply_param(&mut moved, ModParam::Smooth, 1.0);
        assert_eq!(moved, lfo, "an LFO has neither an attack nor a smoothing");
    }

    /// An MSEG is a pure function of the position: straight lines between its points (or a hold when it is not smooth),
    /// looping or holding its last point, with the amount shrinking the swing around the middle.
    #[test]
    fn an_mseg_reads_its_points_at_a_position() {
        let mut m = Mseg::default();
        assert!((mseg_value(&m, 0.0) - 0.0).abs() < 1e-6, "the first point is the bottom");
        assert!((mseg_value(&m, 0.25) - 0.5).abs() < 1e-6, "halfway up the first segment");
        assert!((mseg_value(&m, 0.5) - 1.0).abs() < 1e-6, "the middle point is the top");
        assert!((mseg_value(&m, 0.75) - 0.5).abs() < 1e-6);
        assert!((mseg_value(&m, 1.0) - 0.0).abs() < 1e-6, "one pass is one cycle");
        m.looped = false;
        assert!((mseg_value(&m, 1.5) - 0.0).abs() < 1e-6, "a shape that does not loop holds its last point");
        m.smooth = false;
        assert!((mseg_value(&m, 0.25) - 0.0).abs() < 1e-6, "a held shape keeps the point it is on");
        m.smooth = true;
        m.amount = 0.0;
        assert!((mseg_value(&m, 0.5) - 0.5).abs() < 1e-6, "an amount of zero is the middle");
    }

    /// A route's depth is a destination: a macro shaping how far a step sequence drives a fader changes the swing, and
    /// with nothing shaping it the assignment keeps exactly the depth the user set.
    #[test]
    fn a_modulator_shapes_how_far_another_route_drives() {
        let mut e = Engine::new(&project(0), &mut SampleLibrary::new(None), 48000, PlayMode::Pattern(1));
        let steps = || Modulator {
            id: 2,
            source: ModSource::Steps(Steps { values: [50; 16], period: Timebase::Sync(1), smooth: false }),
            enabled: true,
            targets: vec![ModTarget { target: Target::MixerVolume(1), depth: -0.25, curve: Curve::Linear, polarity: Polarity::Unipolar, enabled: true }],
        };
        let shape = |value| Modulator {
            id: 1,
            source: ModSource::Macro(Macro { value }),
            enabled: true,
            targets: vec![ModTarget { target: Target::RouteDepth(2, 0), depth: 1.0, curve: Curve::Linear, polarity: Polarity::Bipolar, enabled: true }],
        };
        // The authored depth alone: a quarter down from the fader's own value.
        e.set_modulation(vec![steps()]);
        run(&mut e, 1000, 1000);
        assert_eq!(e.tracks[1].mod_vol, Some(9800), "0.8 minus a quarter of 0.75");
        assert_eq!(e.tracks[1].vol, 12800, "the authored fader is still the base");
        // A macro at the bottom of its travel drives the route's depth to its negative end...
        e.set_modulation(vec![shape(0.0), steps()]);
        run(&mut e, 1000, 1000);
        assert_eq!(e.tracks[1].mod_vol, Some(800), "the whole depth down is the bottom");
        // ...and at the top, to its positive one.
        e.set_modulation(vec![shape(1.0), steps()]);
        run(&mut e, 1000, 1000);
        assert_eq!(e.tracks[1].mod_vol, Some(16000), "the whole depth up is the top");
    }

    /// A macro is a source like any other and it can be driven: an LFO turning a macro's value moves everything the
    /// macro drives, at each assignment's own depth.
    #[test]
    fn a_route_turns_a_macro_which_drives_the_rest() {
        let mut e = Engine::new(&project(0), &mut SampleLibrary::new(None), 48000, PlayMode::Pattern(1));
        e.set_modulation(vec![
            Modulator { id: 1, source: ModSource::Lfo(Lfo::default()), enabled: true, targets: vec![
                ModTarget { target: Target::ModulatorParam(2, ModParam::Value.code()), depth: 0.5, curve: Curve::Linear, polarity: Polarity::Unipolar, enabled: true },
            ] },
            Modulator { id: 2, source: ModSource::Macro(Macro { value: 0.0 }), enabled: true, targets: vec![
                ModTarget { target: Target::MixerVolume(1), depth: 0.5, curve: Curve::Linear, polarity: Polarity::Unipolar, enabled: true },
            ] },
        ]);
        run(&mut e, 1000, 1000);
        let turned = e.tracks[1].mod_vol.expect("the macro drives the fader");
        assert!(turned > 12800 && turned <= 16000, "the macro sits somewhere up its range: {turned}");
        assert_eq!(e.tracks[1].vol, 12800, "the authored fader is still the base");
        // A macro that nothing drives sits exactly where the user put it.
        e.set_modulation(vec![Modulator { id: 1, source: ModSource::Macro(Macro { value: 0.5 }), enabled: true, targets: vec![
            ModTarget { target: Target::MixerVolume(1), depth: 0.5, curve: Curve::Linear, polarity: Polarity::Unipolar, enabled: true },
        ] }]);
        run(&mut e, 1000, 1000);
        assert_eq!(e.tracks[1].mod_vol, Some(16000), "half up plus half the range is the top");
    }

    /// A note source reads the note of the channel its target is on, not whichever channel played last: two keyboards
    /// playing two instruments keep their own velocity and their own key range.
    #[test]
    fn a_note_source_reads_each_channels_own_note() {
        let mut mods = Modulation::default();
        let target = |channel: u16| ModTarget {
            target: Target::ChannelParam(channel, 7),
            depth: 1.0,
            curve: Curve::Linear,
            polarity: Polarity::Unipolar,
            enabled: true,
        };
        mods.set(vec![
            Modulator { id: 1, source: ModSource::Velocity(os_model::modulation::Velocity { amount: 1.0 }), enabled: true, targets: vec![target(0), target(1)] },
            Modulator { id: 2, source: ModSource::KeyTrack(os_model::modulation::KeyTrack { amount: 1.0, low: 40, high: 80 }), enabled: true, targets: vec![target(0), target(1)] },
        ]);
        // Track 0 plays through channel 0, track 1 through channel 1.
        mods.set_channels(2, vec![Some(0), Some(1)]);
        mods.notes.note_on(0, 40, 127);
        mods.notes.note_on(1, 80, 64);
        mods.evaluate(0.0, 0.0);
        assert_eq!(mods.output_at(0, Target::ChannelParam(0, 7)), 1.0, "channel 0's loud note");
        assert!((mods.output_at(0, Target::ChannelParam(1, 7)) - 64.0 / 127.0).abs() < 1e-5, "channel 1's softer note");
        assert_eq!(mods.output_at(1, Target::ChannelParam(0, 7)), 0.0, "channel 0's low key sits at the bottom");
        assert_eq!(mods.output_at(1, Target::ChannelParam(1, 7)), 1.0, "channel 1's high key at the top");
        // A mixer track follows the channel it plays through, and a channel that played nothing reads the middle.
        assert!((mods.output_at(0, Target::MixerVolume(1)) - 64.0 / 127.0).abs() < 1e-5);
        mods.notes.note_on(0, 60, 127);
        mods.evaluate(0.0, 0.0);
        assert!((mods.output_at(1, Target::ChannelParam(0, 7)) - 0.5).abs() < 1e-6, "C4 is halfway up 40..80");
    }
}
