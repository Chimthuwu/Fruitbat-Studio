// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! A MIDI track's MIDI FX on the note path: the notes its clips play go through the track's MIDI FX chain, left to
//! right, before they reach the instrument (spec: track-model-plan.md §9, plugins/lemon-arp.md, plugins/lemon-chord.md,
//! plugins/lemon-transpose.md). The device rack shows the chain; this is what it does to the sound.
//!
//! The engine gives a channel's instrument each tick's notes with their frame offsets and then one `process` call
//! (`os_engine::Instrument`), so MIDI FX fit the instrument's own seat: [`NoteFx`] takes the channel's place in the
//! engine's instrument list, keeps the notes for itself, runs the chain and plays the instrument what the chain makes.
//!
//! Each stage turns the notes it is given into the notes it puts out ([`Stage::run`]), with the frame they fall on, so
//! the chain composes: Lemon Chord → Lemon Arp plays each chord note in turn, Lemon Arp → Lemon Transpose shifts what
//! the arpeggiator picked.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU16, AtomicU8, Ordering};
use std::sync::Arc;

use os_engine::{ChannelLevels, ChannelVoicing, Instrument, NoteLevels, SharedInstrument, TransportInfo};

/// The plugin names that play on the note path (`Plugins::midi_fx`).
pub const LEMON_ARP: &str = "Lemon Arp";
pub const LEMON_CHORD: &str = "Lemon Chord";
pub const LEMON_TRANSPOSE: &str = "Lemon Transpose";

/// One note the chain is given, with the frame of the next block it falls on.
#[derive(Debug, Clone, Copy, PartialEq)]
enum In {
    On { offset: u32, key: u8, velocity: f32 },
    Off { offset: u32, key: u8 },
}

impl From<In> for (u32, Out) {
    fn from(i: In) -> (u32, Out) {
        match i {
            In::On { offset, key, velocity } => (offset, Out::On { key, velocity }),
            In::Off { offset, key } => (offset, Out::Off(key)),
        }
    }
}

/// A note the chain puts out.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Out {
    On { key: u8, velocity: f32 },
    Off(u8),
}

/// A note's key as MIDI 0..=127.
fn clamp_key(key: i32) -> Option<u8> {
    (0..=127).contains(&key).then_some(key as u8)
}

// ---------------------------------------------------------------------------------------------------------------
// Lemon Arp
// ---------------------------------------------------------------------------------------------------------------

/// Lemon Arp's settings as its live instance has them: the UI thread writes them from the plugin's parameters every
/// tick, the audio thread reads them each block.
#[derive(Debug)]
pub struct ArpSettings {
    /// 0 up, 1 down, 2 up/down, 3 random.
    pattern: AtomicU8,
    /// 1..=5 octaves the pattern spans.
    octaves: AtomicU8,
    /// 0 = 1/4, 1 = 1/8, 2 = 1/16, 3 = 1/32, 4 = 1/64.
    rate: AtomicU8,
    /// Note length, per cent of a step.
    gate: AtomicU8,
    /// Swing, -50..=50 (stored + 50).
    swing: AtomicU8,
    hold: AtomicBool,
    latch: AtomicBool,
}

/// What the arpeggiator reads of its settings in a block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ArpValues {
    pub pattern: u8,
    pub octaves: u8,
    pub rate: u8,
    pub gate: u8,
    pub swing: i8,
    pub hold: bool,
    pub latch: bool,
}

impl ArpValues {
    /// The arpeggiator's steps per beat: 1/4 is a whole beat, 1/32 an eighth of one.
    fn beats_per_step(&self) -> f64 {
        match self.rate {
            0 => 1.0,
            1 => 0.5,
            2 => 0.25,
            3 => 0.125,
            _ => 0.0625,
        }
    }
}

impl Default for ArpSettings {
    /// The plugin's own factory settings (`plugins/lemon-arp/src/params.rs`, `START`): up, 2 octaves, 1/32, 50 % gate.
    fn default() -> ArpSettings {
        ArpSettings {
            pattern: AtomicU8::new(0),
            octaves: AtomicU8::new(2),
            rate: AtomicU8::new(3),
            gate: AtomicU8::new(50),
            swing: AtomicU8::new(50),
            hold: AtomicBool::new(false),
            latch: AtomicBool::new(false),
        }
    }
}

impl ArpSettings {
    pub fn values(&self) -> ArpValues {
        ArpValues {
            pattern: self.pattern.load(Ordering::Relaxed) % 4,
            octaves: self.octaves.load(Ordering::Relaxed).clamp(1, 5),
            rate: self.rate.load(Ordering::Relaxed) % 5,
            gate: self.gate.load(Ordering::Relaxed).min(100),
            swing: self.swing.load(Ordering::Relaxed) as i8 - 50,
            hold: self.hold.load(Ordering::Relaxed),
            latch: self.latch.load(Ordering::Relaxed),
        }
    }

    /// Stores what the plugin's parameters say now (the UI tick reads them; the audio thread keeps reading here).
    pub fn write(&self, v: ArpValues) {
        self.pattern.store(v.pattern % 4, Ordering::Relaxed);
        self.octaves.store(v.octaves.clamp(1, 5), Ordering::Relaxed);
        self.rate.store(v.rate % 5, Ordering::Relaxed);
        self.gate.store(v.gate.min(100), Ordering::Relaxed);
        self.swing.store((v.swing.clamp(-50, 50) + 50) as u8, Ordering::Relaxed);
        self.hold.store(v.hold, Ordering::Relaxed);
        self.latch.store(v.latch, Ordering::Relaxed);
    }
}

/// The arpeggiator: the keys held, the step it is on, and when its next step and its sounding note's end fall.
#[derive(Debug)]
pub struct Arp {
    settings: Arc<ArpSettings>,
    /// The keys held, in key order, with the velocity they were pressed at.
    held: Vec<(u8, f32)>,
    /// How far through the keys' sequence the pattern is.
    step: usize,
    /// Frames from the block's start until the next step.
    clock: f64,
    /// Steps taken so far: swing moves every other one.
    steps: u64,
    /// The key sounding and the block frame its note off falls on.
    sounding: Option<(u8, f64)>,
    /// The chain has nothing held: the sounding note ends at the block's start.
    cut: bool,
}

impl Arp {
    pub fn new(settings: Arc<ArpSettings>) -> Arp {
        Arp { settings, held: Vec::new(), step: 0, clock: 0.0, steps: 0, sounding: None, cut: false }
    }

    pub fn settings(&self) -> &Arc<ArpSettings> {
        &self.settings
    }

    /// A note reaches the chain.
    fn note_on(&mut self, key: u8, velocity: f32) {
        let v = self.settings.values();
        if v.latch && self.held.is_empty() {
            // Latch: the first note after everything was released starts a new chord.
            self.held.clear();
            self.step = 0;
        }
        if !self.held.iter().any(|(k, _)| *k == key) {
            self.held.push((key, velocity));
            self.held.sort_by_key(|(k, _)| *k);
        }
        self.cut = false;
    }

    /// A note is released (Hold and Latch keep it in the pattern).
    fn note_off(&mut self, key: u8) {
        let v = self.settings.values();
        if v.hold || v.latch {
            return;
        }
        self.held.retain(|(k, _)| *k != key);
        if self.held.is_empty() {
            self.cut = true;
        }
    }

    fn reset(&mut self) {
        self.held.clear();
        self.step = 0;
        self.clock = 0.0;
        self.steps = 0;
        self.sounding = None;
        self.cut = true;
    }

    /// True while it has a note of its own sounding.
    fn ringing(&self) -> bool {
        self.sounding.is_some()
    }

    /// The keys the pattern walks: the held keys, each octave up on top of the last.
    fn sequence(&self, v: ArpValues) -> Vec<(u8, f32)> {
        let mut out = Vec::with_capacity(self.held.len() * usize::from(v.octaves));
        for o in 0..u32::from(v.octaves) {
            for &(k, vel) in &self.held {
                let k = u32::from(k) + 12 * o;
                if k < 128 {
                    out.push((k as u8, vel));
                }
            }
        }
        out
    }

    /// The next key the pattern plays, stepping it on.
    fn next_key(&mut self, v: ArpValues) -> Option<(u8, f32)> {
        let seq = self.sequence(v);
        let n = seq.len();
        if n == 0 {
            return None;
        }
        let at = match v.pattern {
            0 => {
                let i = self.step % n;
                self.step = (self.step + 1) % n;
                i
            }
            1 => {
                let i = n - 1 - self.step % n;
                self.step = (self.step + 1) % n;
                i
            }
            2 => {
                let cycle = if n == 1 { 1 } else { 2 * n - 2 };
                let i = self.step % cycle;
                self.step = (self.step + 1) % cycle;
                if i < n {
                    i
                } else {
                    cycle - i
                }
            }
            _ => {
                // Random: a small deterministic shuffle, so the same notes press the same way twice.
                let i = (self.steps.wrapping_mul(1103515245).wrapping_add(12345) / 65536) as usize % n;
                self.step = self.step.wrapping_add(1);
                i
            }
        };
        seq.get(at).copied()
    }

    /// Plays the steps that start before block frame `limit` into `out` (frames of the block).
    fn advance(&mut self, limit: f64, rate: u32, tempo: f64, out: &mut Vec<(u32, Out)>) {
        let v = self.settings.values();
        let step = (f64::from(rate) * 60.0 / tempo.max(1.0) * v.beats_per_step()).max(1.0);
        if std::mem::take(&mut self.cut) {
            if let Some((key, _)) = self.sounding.take() {
                out.push((0, Out::Off(key)));
            }
        }
        if self.held.is_empty() {
            self.clock = 0.0;
            return;
        }
        loop {
            // Swing: every other step starts late, by the swing share of half a step.
            let swing = if self.steps % 2 == 1 { f64::from(v.swing) / 100.0 * step * 0.5 } else { 0.0 };
            let at = self.clock + swing.max(0.0);
            if at >= limit {
                break;
            }
            if let Some((key, _)) = self.sounding.take() {
                out.push((at as u32, Out::Off(key)));
            }
            let Some((key, velocity)) = self.next_key(v) else { break };
            out.push((at as u32, Out::On { key, velocity }));
            let gate = step * f64::from(v.gate.min(100)) / 100.0;
            self.sounding = Some((key, at + gate));
            self.steps = self.steps.wrapping_add(1);
            self.clock += step;
        }
        if let Some((key, end)) = self.sounding {
            if end < limit {
                out.push((end.max(0.0) as u32, Out::Off(key)));
                self.sounding = None;
            }
        }
    }

    /// The block is over: the clock and the sounding note's end move to the next block's frames.
    fn end_block(&mut self, n: f64) {
        self.clock = (self.clock - n).max(0.0);
        if let Some((key, end)) = self.sounding {
            if end < n {
                // Its note off went out with the block.
                self.sounding = None;
            } else {
                self.sounding = Some((key, end - n));
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Lemon Transpose
// ---------------------------------------------------------------------------------------------------------------

/// Lemon Transpose's settings as its live instance has them.
#[derive(Debug)]
pub struct TransposeSettings {
    /// Whole semitones, -24..=24.
    semitones: AtomicI32,
    /// Whole octaves, -4..=4.
    octaves: AtomicI32,
    /// Snap the result into the scale (`root`, `scale`).
    scale_aware: AtomicBool,
    /// Pitch class of the scale's root, 0 = C.
    root: AtomicU8,
    /// Index into the plugin's scales.
    scale: AtomicU8,
}

/// What the transposer reads of its settings in a block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransposeValues {
    pub semitones: i32,
    pub octaves: i32,
    pub scale_aware: bool,
    pub root: u8,
    pub scale: u8,
}

impl TransposeValues {
    /// The whole shift in semitones.
    pub fn shift(&self) -> i32 {
        self.semitones + 12 * self.octaves
    }
}

impl Default for TransposeSettings {
    /// The plugin's factory state: no shift, no scale.
    fn default() -> TransposeSettings {
        TransposeSettings {
            semitones: AtomicI32::new(0),
            octaves: AtomicI32::new(0),
            scale_aware: AtomicBool::new(false),
            root: AtomicU8::new(0),
            scale: AtomicU8::new(0),
        }
    }
}

impl TransposeSettings {
    pub fn values(&self) -> TransposeValues {
        TransposeValues {
            semitones: self.semitones.load(Ordering::Relaxed).clamp(-24, 24),
            octaves: self.octaves.load(Ordering::Relaxed).clamp(-4, 4),
            scale_aware: self.scale_aware.load(Ordering::Relaxed),
            root: self.root.load(Ordering::Relaxed) % 12,
            scale: self.scale.load(Ordering::Relaxed) % 9,
        }
    }

    pub fn write(&self, v: TransposeValues) {
        self.semitones.store(v.semitones.clamp(-24, 24), Ordering::Relaxed);
        self.octaves.store(v.octaves.clamp(-4, 4), Ordering::Relaxed);
        self.scale_aware.store(v.scale_aware, Ordering::Relaxed);
        self.root.store(v.root % 12, Ordering::Relaxed);
        self.scale.store(v.scale % 9, Ordering::Relaxed);
    }
}

/// Is `note` in the scale (`scale` index, root pitch class `root`)?
fn in_scale(note: i32, root: u8, scale: u8) -> bool {
    let degrees = lemon_chord::params::Scale::from_index(scale).degrees();
    degrees.contains(&((note - i32::from(root)).rem_euclid(12) as u8))
}

/// The nearest note in the scale to `note` (itself when in it; a tie goes down). `None` outside 0..=127.
fn snap_to_scale(note: i32, root: u8, scale: u8) -> Option<u8> {
    for d in 0..=6 {
        for cand in [note - d, note + d] {
            if (0..=127).contains(&cand) && in_scale(cand, root, scale) {
                return Some(cand as u8);
            }
        }
    }
    None
}

/// `note` shifted by `shift` semitones, snapped into the scale with `scale_aware`; `None` when it leaves 0..=127.
fn transpose_note(note: u8, shift: i32, v: &TransposeValues) -> Option<u8> {
    let target = i32::from(note) + shift;
    if !(0..=127).contains(&target) {
        return None;
    }
    if v.scale_aware {
        snap_to_scale(target, v.root, v.scale)
    } else {
        Some(target as u8)
    }
}

/// The transposer: remembers which note each sounding input note became, so its note off hits the same note even if
/// the settings moved meanwhile.
#[derive(Debug)]
pub struct Transpose {
    settings: Arc<TransposeSettings>,
    /// Per input key: the output key + 1 (0 = none).
    active: [u8; 128],
}

impl Transpose {
    pub fn new(settings: Arc<TransposeSettings>) -> Transpose {
        Transpose { settings, active: [0; 128] }
    }

    pub fn settings(&self) -> &Arc<TransposeSettings> {
        &self.settings
    }

    fn run(&mut self, input: Vec<(u32, Out)>, _n: usize, _rate: u32, _tempo: f64) -> Vec<(u32, Out)> {
        let v = self.settings.values();
        let shift = v.shift();
        input
            .into_iter()
            .filter_map(|(at, ev)| match ev {
                Out::On { key, velocity } => {
                    let out = transpose_note(key, shift, &v)?;
                    self.active[usize::from(key) & 127] = out + 1;
                    Some((at, Out::On { key: out, velocity }))
                }
                Out::Off(key) => {
                    let slot = &mut self.active[usize::from(key) & 127];
                    let out = slot.checked_sub(1)?;
                    *slot = 0;
                    Some((at, Out::Off(out)))
                }
            })
            .collect()
    }

    fn reset(&mut self) {
        self.active.fill(0);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Lemon Chord
// ---------------------------------------------------------------------------------------------------------------

/// Lemon Chord's settings as its live instance has them.
#[derive(Debug)]
pub struct ChordSettings {
    root: AtomicU8,
    scale: AtomicU8,
    /// Chord type: 0 triad, 1 7th, 2 9th, 3 11th, 4 13th, 5 sus2, 6 sus4.
    chord: AtomicU8,
    /// 0 root, 1 first, 2 second, 3 third.
    inversion: AtomicU8,
    /// 0 close, 1 open, 2 drop 2, 3 drop 3.
    voicing: AtomicU8,
    /// 0 off, 1 down, 2 up, 3 alternating.
    strum: AtomicU8,
    /// 0..=10000, 100 = slowest (the plugin's).
    strum_speed: AtomicU16,
    hold: AtomicBool,
}

/// What the chord maker reads of its settings in a block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChordValues {
    pub root: u8,
    pub scale: u8,
    pub chord: u8,
    pub inversion: u8,
    pub voicing: u8,
    pub strum: u8,
    pub strum_speed: u16,
    pub hold: bool,
}

impl Default for ChordSettings {
    /// The plugin's factory state: C major, triads, root position, close, no strum.
    fn default() -> ChordSettings {
        ChordSettings {
            root: AtomicU8::new(0),
            scale: AtomicU8::new(0),
            chord: AtomicU8::new(0),
            inversion: AtomicU8::new(0),
            voicing: AtomicU8::new(0),
            strum: AtomicU8::new(0),
            strum_speed: AtomicU16::new(5000),
            hold: AtomicBool::new(false),
        }
    }
}

impl ChordSettings {
    pub fn values(&self) -> ChordValues {
        ChordValues {
            root: self.root.load(Ordering::Relaxed) % 12,
            scale: self.scale.load(Ordering::Relaxed) % 9,
            chord: self.chord.load(Ordering::Relaxed) % 7,
            inversion: self.inversion.load(Ordering::Relaxed).min(3),
            voicing: self.voicing.load(Ordering::Relaxed).min(3),
            strum: self.strum.load(Ordering::Relaxed).min(3),
            strum_speed: self.strum_speed.load(Ordering::Relaxed).min(10_000),
            hold: self.hold.load(Ordering::Relaxed),
        }
    }

    pub fn write(&self, v: ChordValues) {
        self.root.store(v.root % 12, Ordering::Relaxed);
        self.scale.store(v.scale % 9, Ordering::Relaxed);
        self.chord.store(v.chord % 7, Ordering::Relaxed);
        self.inversion.store(v.inversion.min(3), Ordering::Relaxed);
        self.voicing.store(v.voicing.min(3), Ordering::Relaxed);
        self.strum.store(v.strum.min(3), Ordering::Relaxed);
        self.strum_speed.store(v.strum_speed.min(10_000), Ordering::Relaxed);
        self.hold.store(v.hold, Ordering::Relaxed);
    }
}

/// The chord a note stands for, keeping its octave: the note is read as a scale degree of `root` / `scale` and the
/// chord type stacked on it inside that scale (`plugins/lemon-chord/src/dsp/chord.rs`, but anchored on the played
/// note rather than on middle C).
pub fn chord_of(key: u8, v: &ChordValues) -> Vec<u8> {
    let scale = lemon_chord::params::Scale::from_index(v.scale).degrees();
    let chord = lemon_chord::params::ChordType::from_index(v.chord).degrees();
    let root = i32::from(v.root);
    // The scale degree the played note is closest to.
    let rel = (i32::from(key % 12) - root).rem_euclid(12);
    let degree = (0..scale.len())
        .min_by_key(|&i| (rel - i32::from(scale[i])).abs())
        .unwrap_or(0);
    let octave = i32::from(key / 12) * 12;
    let chord_root = octave + (root + i32::from(scale[degree])) % 12;
    let mut notes: Vec<u8> = chord
        .iter()
        .filter_map(|&cd| {
            let idx = degree + cd;
            let octaves = (idx / scale.len()) as i32;
            let at = idx % scale.len();
            let interval = (i32::from(scale[at]) - i32::from(scale[degree])).rem_euclid(12) + 12 * octaves;
            clamp_key(chord_root + interval)
        })
        .collect();
    notes.sort_unstable();
    notes.dedup();
    // Inversions: the lowest notes go up an octave.
    for _ in 0..usize::from(v.inversion).min(notes.len().saturating_sub(1)) {
        if let Some(k) = notes.first_mut() {
            *k = k.saturating_add(12);
        }
        notes.sort_unstable();
    }
    // Voicings.
    match v.voicing {
        1 => {
            // Open: the 2nd, 3rd … notes of the close position go up an octave each.
            for (i, k) in notes.iter_mut().enumerate() {
                if i % 2 == 1 {
                    *k = k.saturating_add(12);
                }
            }
            notes.sort_unstable();
        }
        2 if notes.len() >= 2 => {
            let i = notes.len() - 2;
            notes[i] = notes[i].saturating_sub(12);
            notes.sort_unstable();
        }
        3 if notes.len() >= 3 => {
            let i = notes.len() - 3;
            notes[i] = notes[i].saturating_sub(12);
            notes.sort_unstable();
        }
        _ => {}
    }
    notes.retain(|&k| k < 128);
    notes
}

/// The chord maker: the chord each input key made (so its note offends them all), the strum's pending notes.
#[derive(Debug)]
pub struct Chord {
    settings: Arc<ChordSettings>,
    /// The notes each held key stands for.
    active: Vec<(u8, Vec<u8>)>,
    /// Frames processed so far (the strum's clock).
    now: u64,
    /// Notes the strum put later than the block they came in.
    pending: Vec<(u64, Out)>,
    /// Alternating strum's direction, 1 or -1.
    dir: i8,
}

impl Chord {
    pub fn new(settings: Arc<ChordSettings>) -> Chord {
        Chord { settings, active: Vec::new(), now: 0, pending: Vec::new(), dir: 1 }
    }

    pub fn settings(&self) -> &Arc<ChordSettings> {
        &self.settings
    }

    /// How many frames note `i` of a strummed chord waits: `strum_speed` is the plugin's 0..10000, 10000 = together.
    fn strum_frames(i: usize, v: &ChordValues, rate: u32) -> u64 {
        if v.strum == 0 || i == 0 {
            return 0;
        }
        let gap_ms = (100.0 - f64::from(v.strum_speed) / 100.0).max(0.0);
        (f64::from(rate) * gap_ms / 1000.0 * i as f64) as u64
    }

    fn run(&mut self, input: Vec<(u32, Out)>, n: usize, rate: u32, _tempo: f64) -> Vec<(u32, Out)> {
        let v = self.settings.values();
        let start = self.now;
        let end = start + n as u64;
        let mut timed: Vec<(u64, Out)> = Vec::with_capacity(self.pending.len() + input.len());
        for (at, ev) in input {
            let at = start + u64::from(at.min(n as u32));
            match ev {
                Out::On { key, velocity } => {
                    // A new note on the same key replaces the chord it made.
                    if let Some((_, old)) = self.active.iter().position(|(k, _)| *k == key).map(|i| self.active.remove(i)) {
                        timed.extend(old.into_iter().map(|k| (at, Out::Off(k))));
                    }
                    let mut notes = chord_of(key, &v);
                    if notes.is_empty() {
                        notes.push(key);
                    }
                    match v.strum {
                        2 => notes.reverse(),
                        3 => {
                            if self.dir < 0 {
                                notes.reverse();
                            }
                            self.dir = -self.dir;
                        }
                        _ => {}
                    }
                    let mut made = vec![key];
                    for (i, k) in notes.iter().enumerate() {
                        timed.push((at + Self::strum_frames(i, &v, rate), Out::On { key: *k, velocity }));
                    }
                    made.extend(notes);
                    made.sort_unstable();
                    made.dedup();
                    self.active.push((key, made));
                }
                Out::Off(key) => {
                    if v.hold {
                        continue;
                    }
                    if let Some(i) = self.active.iter().position(|(k, _)| *k == key) {
                        let (_, notes) = self.active.remove(i);
                        timed.extend(notes.into_iter().map(|k| (at, Out::Off(k))));
                    }
                }
            }
        }
        timed.extend(self.pending.drain(..));
        timed.sort_by_key(|(t, _)| *t);
        let mut out = Vec::with_capacity(timed.len());
        for (t, ev) in timed {
            if t < end {
                out.push(((t - start) as u32, ev));
            } else {
                self.pending.push((t, ev));
            }
        }
        self.now = end;
        out
    }

    fn reset(&mut self) {
        self.active.clear();
        self.pending.clear();
        self.now = 0;
        self.dir = 1;
    }

    /// True while a Hold keeps a chord of its own.
    fn ringing(&self) -> bool {
        !self.active.is_empty()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The chain
// ---------------------------------------------------------------------------------------------------------------

/// One MIDI FX of a chain, with its live settings.
pub enum Stage {
    Arp(Arp),
    Transpose(Transpose),
    Chord(Chord),
}

impl Stage {
    /// The stage's settings, the way the UI tick refreshes them.
    pub fn settings(&self) -> ChainSettings {
        match self {
            Stage::Arp(a) => ChainSettings::Arp(a.settings.clone()),
            Stage::Transpose(t) => ChainSettings::Transpose(t.settings.clone()),
            Stage::Chord(c) => ChainSettings::Chord(c.settings.clone()),
        }
    }

    fn reset(&mut self) {
        match self {
            Stage::Arp(a) => a.reset(),
            Stage::Transpose(t) => t.reset(),
            Stage::Chord(c) => c.reset(),
        }
    }

    fn ringing(&self) -> bool {
        match self {
            Stage::Arp(a) => a.ringing(),
            Stage::Transpose(_) => false,
            Stage::Chord(c) => c.ringing(),
        }
    }

    /// This block's notes through the stage.
    fn run(&mut self, input: Vec<(u32, Out)>, n: usize, rate: u32, tempo: f64) -> Vec<(u32, Out)> {
        match self {
            Stage::Arp(a) => {
                let mut events = Vec::with_capacity(input.len() + 4);
                let mut at = 0.0;
                for (offset, ev) in input {
                    let o = f64::from(offset.min(n as u32));
                    if o > at {
                        a.advance(o, rate, tempo, &mut events);
                        at = o;
                    }
                    match ev {
                        Out::On { key, velocity } => a.note_on(key, velocity),
                        Out::Off(key) => a.note_off(key),
                    }
                }
                a.advance(f64::from(n as u32), rate, tempo, &mut events);
                a.end_block(f64::from(n as u32));
                events.sort_by_key(|(o, _)| *o);
                events
            }
            Stage::Transpose(t) => t.run(input, n, rate, tempo),
            Stage::Chord(c) => c.run(input, n, rate, tempo),
        }
    }
}

/// A chain stage's settings, whichever plugin it is (the UI thread writes them every tick).
#[derive(Clone)]
pub enum ChainSettings {
    Arp(Arc<ArpSettings>),
    Transpose(Arc<TransposeSettings>),
    Chord(Arc<ChordSettings>),
}

impl ChainSettings {
    /// A stage reading these settings.
    pub fn stage(&self) -> Stage {
        match self {
            ChainSettings::Arp(s) => Stage::Arp(Arp::new(s.clone())),
            ChainSettings::Transpose(s) => Stage::Transpose(Transpose::new(s.clone())),
            ChainSettings::Chord(s) => Stage::Chord(Chord::new(s.clone())),
        }
    }
}

/// A MIDI track's instrument as its MIDI FX chain makes of it: the notes its clips play reach the instrument through
/// the chain.
pub struct NoteFx {
    inner: SharedInstrument,
    stages: Vec<Stage>,
    /// This tick's notes, with their frames, waiting for the next `process`.
    notes: Vec<In>,
    /// The transport's tempo (the arpeggiator's clock).
    tempo: f64,
}

impl NoteFx {
    pub fn new(inner: SharedInstrument, stages: Vec<Stage>) -> NoteFx {
        NoteFx { inner, stages, notes: Vec::new(), tempo: 120.0 }
    }

    /// The chain's stages, for the app to read their settings back.
    pub fn stages(&self) -> &[Stage] {
        &self.stages
    }

    /// Renders the chain's notes into the instrument. False when the instrument is busy.
    fn play(&mut self, rate: u32, n: usize, left: &mut [f32], right: &mut [f32]) -> bool {
        let Ok(mut inner) = self.inner.try_lock() else { return false };
        let mut events: Vec<(u32, Out)> = std::mem::take(&mut self.notes).into_iter().map(Into::into).collect();
        events.sort_by_key(|(o, _)| *o);
        if !self.stages.is_empty() {
            for stage in &mut self.stages {
                events = stage.run(events, n, rate, self.tempo);
            }
        }
        for (offset, ev) in events {
            let at = offset.min(n.saturating_sub(1) as u32);
            match ev {
                Out::On { key, velocity } => inner.note_on(at, key, velocity),
                Out::Off(key) => inner.note_off(at, key),
            }
        }
        inner.process(rate, left, right);
        true
    }
}

impl Instrument for NoteFx {
    fn note_on(&mut self, offset: u32, key: u8, velocity: f32) {
        self.notes.push(In::On { offset, key, velocity });
    }

    fn note_off(&mut self, offset: u32, key: u8) {
        self.notes.push(In::Off { offset, key });
    }

    /// Every note of a song starts this way; a MIDI FX chain takes the note and its velocity (per-note pan and mods go
    /// with the channel).
    fn note_start(&mut self, offset: u32, key: u8, velocity: f32, start: f32, levels: NoteLevels) {
        let _ = start;
        let _ = levels;
        self.notes.push(In::On { offset, key, velocity });
    }

    /// A cut ends the note: the chain hears it released.
    fn choke(&mut self, offset: u32, key: u8) {
        self.notes.push(In::Off { offset, key });
    }

    fn tune(&mut self, offset: u32, key: u8, semitones: f32) {
        if let Ok(mut i) = self.inner.try_lock() {
            i.tune(offset, key, semitones);
        }
    }

    fn set_bend_range(&mut self, semitones: u32) {
        if let Ok(mut i) = self.inner.try_lock() {
            i.set_bend_range(semitones);
        }
    }

    fn takes_tuning(&self) -> bool {
        self.inner.try_lock().is_ok_and(|i| i.takes_tuning())
    }

    fn voice_ends(&self) -> bool {
        self.inner.try_lock().is_ok_and(|i| i.voice_ends())
    }

    fn voice_gains(&self) -> bool {
        self.inner.try_lock().is_ok_and(|i| i.voice_gains())
    }

    /// An audio clip's voice is not a note a MIDI FX moves: it goes straight to the instrument.
    fn voice_gain(&mut self, offset: u32, voice: u32, gain: f32) {
        if let Ok(mut i) = self.inner.try_lock() {
            i.voice_gain(offset, voice, gain);
        }
    }

    fn voice_choke(&mut self, offset: u32, key: u8, voice: u32) {
        if let Ok(mut i) = self.inner.try_lock() {
            i.voice_choke(offset, key, voice);
        }
    }

    fn voice_start(&mut self, offset: u32, key: u8, velocity: f32, start: f32, end: f32, levels: NoteLevels, voice: u32) {
        if let Ok(mut i) = self.inner.try_lock() {
            i.voice_start(offset, key, velocity, start, end, levels, voice);
        }
    }

    fn param(&mut self, offset: u32, index: u32, value: f64) {
        if let Ok(mut i) = self.inner.try_lock() {
            i.param(offset, index, value);
        }
    }

    fn tick_params(&self) -> bool {
        self.inner.try_lock().is_ok_and(|i| i.tick_params())
    }

    fn tick_params_at(&self, index: u32) -> bool {
        self.inner.try_lock().is_ok_and(|i| i.tick_params_at(index))
    }

    fn param_range(&self, index: usize) -> Option<(f64, f64)> {
        self.inner.try_lock().ok().and_then(|i| i.param_range(index))
    }

    fn channel_levels(&mut self, levels: ChannelLevels) -> bool {
        self.inner.try_lock().is_ok_and(|mut i| i.channel_levels(levels))
    }

    fn channel_voicing(&mut self, voicing: &ChannelVoicing) {
        if let Ok(mut i) = self.inner.try_lock() {
            i.channel_voicing(voicing);
        }
    }

    fn transport(&mut self, info: &TransportInfo) {
        self.tempo = info.tempo.max(1.0);
        if let Ok(mut i) = self.inner.try_lock() {
            i.transport(info);
        }
    }

    fn midi(&mut self, offset: u32, bytes: [u8; 3]) {
        if let Ok(mut i) = self.inner.try_lock() {
            i.midi(offset, bytes);
        }
    }

    fn reported_delay(&self) -> u32 {
        self.inner.try_lock().map_or(0, |i| i.reported_delay())
    }

    fn process(&mut self, rate: u32, left: &mut [f32], right: &mut [f32]) {
        let n = left.len();
        if !self.play(rate, n, left, right) {
            // The instrument is busy elsewhere: this tick's notes wait for the next one.
            left.fill(0.0);
            right.fill(0.0);
        }
    }

    fn active(&self) -> bool {
        self.stages.iter().any(Stage::ringing) || self.inner.try_lock().is_ok_and(|i| i.active())
    }

    fn voices(&self) -> usize {
        self.inner.try_lock().map_or(0, |i| i.voices())
    }

    fn reset(&mut self) {
        self.notes.clear();
        for stage in &mut self.stages {
            stage.reset();
        }
        if let Ok(mut i) = self.inner.try_lock() {
            i.reset();
        }
    }
}

impl crate::app::App {
    /// Every track with MIDI FX slots filled and on plays its row through them (`Plugins::note_fx`); a track that lost
    /// them plays its instrument directly again. Runs every tick, before the engine is told what to play.
    pub(crate) fn sync_note_fx(&mut self) {
        let rows: Vec<(usize, Vec<(usize, String)>)> = self
            .state
            .project
            .tracks
            .iter()
            .filter_map(|t| {
                let row = usize::from(t.channel?);
                let slots: Vec<(usize, String)> = crate::midi_fx::filled(&self.state, t.id.0)
                    .into_iter()
                    .filter(|(_, sl)| sl.on)
                    .map(|(slot, sl)| (crate::plugins::midi_fx_key(t.id.0, slot), sl.name))
                    .collect();
                (!slots.is_empty()).then_some((row, slots))
            })
            .collect();
        if self.plugins.set_note_fx(rows) {
            self.transport.set_instruments(self.plugins.instruments());
        }
        self.plugins.refresh_note_fx();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// What an instrument was told, in order.
    #[derive(Default)]
    struct Recorder {
        got: Vec<String>,
    }

    impl Instrument for Recorder {
        fn note_on(&mut self, offset: u32, key: u8, velocity: f32) {
            self.got.push(format!("on {key} @{offset} v{}", (velocity * 128.0).round() as i32));
        }
        fn note_off(&mut self, offset: u32, key: u8) {
            self.got.push(format!("off {key} @{offset}"));
        }
        fn process(&mut self, _rate: u32, left: &mut [f32], right: &mut [f32]) {
            left.fill(0.0);
            right.fill(0.0);
        }
    }

    fn setup(values: ArpValues) -> (NoteFx, Arc<Mutex<Recorder>>, Arc<ArpSettings>) {
        let rec = Arc::new(Mutex::new(Recorder::default()));
        let settings = Arc::new(ArpSettings::default());
        settings.write(values);
        let fx = NoteFx::new(rec.clone() as SharedInstrument, vec![Stage::Arp(Arp::new(settings.clone()))]);
        (fx, rec, settings)
    }

    fn transpose(values: TransposeValues) -> Stage {
        let s = Arc::new(TransposeSettings::default());
        s.write(values);
        Stage::Transpose(Transpose::new(s))
    }

    fn chord(values: ChordValues) -> Stage {
        let s = Arc::new(ChordSettings::default());
        s.write(values);
        Stage::Chord(Chord::new(s))
    }

    fn info(tempo: f64) -> TransportInfo {
        TransportInfo { playing: true, tempo, frame: 0, beats: 0.0, bar_start: 0.0, bar: 0, numerator: 4, denominator: 4, looping: None }
    }

    fn run_all(fx: &mut NoteFx, blocks: usize, n: usize) {
        let mut out = vec![0.0; 2 * n];
        for _ in 0..blocks {
            let (l, r) = out.split_at_mut(n);
            fx.process(48_000, l, r);
        }
    }

    fn heard(rec: &Arc<Mutex<Recorder>>) -> Vec<String> {
        rec.lock().unwrap().got.drain(..).collect()
    }

    /// A held chord is played one note at a time, in pattern order, instead of all at once.
    #[test]
    fn a_chord_is_arpeggiated() {
        let (mut fx, rec, _) = setup(ArpValues { pattern: 0, octaves: 1, rate: 2, gate: 50, swing: 0, hold: false, latch: false });
        fx.transport(&info(120.0));
        for key in [60, 64, 67] {
            fx.note_start(0, key, 100.0 / 128.0, 0.0, NoteLevels::default());
        }
        // At 48 kHz and 120 BPM a 1/16 step is 6000 frames: one block of 6000 frames plays one step.
        run_all(&mut fx, 6, 6000);
        let got = heard(&rec);
        let ons: Vec<&String> = got.iter().filter(|s| s.starts_with("on ")).collect();
        assert_eq!(
            ons,
            vec!["on 60 @0 v100", "on 64 @0 v100", "on 67 @0 v100", "on 60 @0 v100", "on 64 @0 v100", "on 67 @0 v100"],
            "one key at a time, in order, the pattern starting over: {got:?}"
        );
        assert!(got.contains(&"off 60 @3000".to_string()), "the gate ends each note: {got:?}");
    }

    /// Nothing held, nothing plays: the chord's notes do not reach the instrument as a chord.
    #[test]
    fn the_chord_itself_never_reaches_the_instrument() {
        let (mut fx, rec, _) = setup(ArpValues { pattern: 0, octaves: 1, rate: 2, gate: 50, swing: 0, hold: false, latch: false });
        fx.transport(&info(120.0));
        fx.note_on(0, 60, 0.5);
        fx.note_on(0, 64, 0.5);
        run_all(&mut fx, 1, 100);
        let got = heard(&rec);
        assert!(got.iter().filter(|s| s.starts_with("on ")).count() <= 1, "{got:?}");
    }

    /// Releasing the chord ends the arpeggio.
    #[test]
    fn releasing_stops_it() {
        let (mut fx, rec, _) = setup(ArpValues { pattern: 0, octaves: 1, rate: 0, gate: 100, swing: 0, hold: false, latch: false });
        fx.transport(&info(120.0));
        fx.note_on(0, 60, 0.5);
        fx.note_on(0, 64, 0.5);
        run_all(&mut fx, 1, 10);
        heard(&rec);
        fx.note_off(0, 60);
        fx.note_off(0, 64);
        run_all(&mut fx, 1, 10);
        assert_eq!(heard(&rec), vec!["off 60 @0".to_string()], "the sounding note ends at once");
    }

    /// Octaves put the pattern up an octave per pass.
    #[test]
    fn octaves_stack_the_pattern() {
        let (mut fx, rec, _) = setup(ArpValues { pattern: 0, octaves: 2, rate: 2, gate: 50, swing: 0, hold: false, latch: false });
        fx.transport(&info(120.0));
        fx.note_on(0, 60, 0.5);
        let mut ons = Vec::new();
        for _ in 0..4 {
            run_all(&mut fx, 1, 6000);
            for s in heard(&rec) {
                if s.starts_with("on ") {
                    ons.push(s);
                }
            }
        }
        assert_eq!(ons, vec!["on 60 @0 v64", "on 72 @0 v64", "on 60 @0 v64", "on 72 @0 v64"]);
    }

    /// With no MIDI FX the notes reach the instrument as they come, offsets and all.
    #[test]
    fn a_chain_without_a_midi_fx_passes_notes_through() {
        let rec = Arc::new(Mutex::new(Recorder::default()));
        let mut fx = NoteFx::new(rec.clone() as SharedInstrument, Vec::new());
        fx.note_on(3, 60, 0.5);
        fx.note_off(4, 60);
        run_all(&mut fx, 1, 100);
        assert_eq!(heard(&rec), vec!["on 60 @3 v64".to_string(), "off 60 @4".to_string()]);
    }

    /// Transpose moves the notes, and their offs follow them.
    #[test]
    fn transpose_shifts_the_notes() {
        let rec = Arc::new(Mutex::new(Recorder::default()));
        let stage = transpose(TransposeValues { semitones: 7, octaves: 1, scale_aware: false, root: 0, scale: 0 });
        let mut fx = NoteFx::new(rec.clone() as SharedInstrument, vec![stage]);
        fx.note_on(2, 60, 0.5);
        fx.note_off(9, 60);
        run_all(&mut fx, 1, 64);
        assert_eq!(heard(&rec), vec!["on 79 @2 v64".to_string(), "off 79 @9".to_string()]);
    }

    /// Scale aware: a shift lands on the scale's notes.
    #[test]
    fn transpose_snaps_into_the_scale() {
        let rec = Arc::new(Mutex::new(Recorder::default()));
        let stage = transpose(TransposeValues { semitones: 1, octaves: 0, scale_aware: true, root: 0, scale: 0 });
        let mut fx = NoteFx::new(rec.clone() as SharedInstrument, vec![stage]);
        // C up one is C#, which C major does not have: it snaps back to C (the tie goes down).
        fx.note_on(0, 60, 0.5);
        run_all(&mut fx, 1, 32);
        assert_eq!(heard(&rec), vec!["on 60 @0 v64".to_string()]);
    }

    /// A note becomes the chord of its scale degree, in its own octave.
    #[test]
    fn a_note_becomes_its_chord() {
        let rec = Arc::new(Mutex::new(Recorder::default()));
        let stage = chord(ChordValues { root: 0, scale: 0, chord: 0, inversion: 0, voicing: 0, strum: 0, strum_speed: 5000, hold: false });
        let mut fx = NoteFx::new(rec.clone() as SharedInstrument, vec![stage]);
        fx.note_on(0, 60, 0.5);
        run_all(&mut fx, 1, 32);
        assert_eq!(
            heard(&rec),
            vec!["on 60 @0 v64".to_string(), "on 64 @0 v64".to_string(), "on 67 @0 v64".to_string()],
            "C major on C: C E G"
        );
        // Its note off ends all of them.
        fx.note_off(0, 60);
        run_all(&mut fx, 1, 32);
        assert_eq!(heard(&rec), vec!["off 60 @0".to_string(), "off 64 @0".to_string(), "off 67 @0".to_string()]);
        // A different degree builds the chord on that degree: E4 is E G B.
        fx.note_on(0, 64, 0.5);
        run_all(&mut fx, 1, 32);
        assert_eq!(
            heard(&rec),
            vec!["on 64 @0 v64".to_string(), "on 67 @0 v64".to_string(), "on 71 @0 v64".to_string()],
            "the minor triad on E"
        );
        assert_eq!(chord_of(60, &ChordValues { root: 0, scale: 0, chord: 1, inversion: 0, voicing: 0, strum: 0, strum_speed: 5000, hold: false }), vec![60, 64, 67, 71], "C major 7");
        assert_eq!(chord_of(60, &ChordValues { root: 0, scale: 0, chord: 0, inversion: 1, voicing: 0, strum: 0, strum_speed: 5000, hold: false }), vec![64, 67, 72], "first inversion");
    }

    /// The chain composes: a chord through the arpeggiator plays the chord's notes one at a time.
    #[test]
    fn the_chain_runs_left_to_right() {
        let rec = Arc::new(Mutex::new(Recorder::default()));
        let triad = ChordValues { root: 0, scale: 0, chord: 0, inversion: 0, voicing: 0, strum: 0, strum_speed: 5000, hold: false };
        let arp = Arc::new(ArpSettings::default());
        arp.write(ArpValues { pattern: 0, octaves: 1, rate: 2, gate: 50, swing: 0, hold: false, latch: false });
        let mut fx = NoteFx::new(rec.clone() as SharedInstrument, vec![chord(triad), Stage::Arp(Arp::new(arp))]);
        fx.transport(&info(120.0));
        fx.note_on(0, 60, 0.5);
        run_all(&mut fx, 1, 6000);
        let got = heard(&rec);
        assert_eq!(got.iter().filter(|s| s.starts_with("on ")).count(), 1, "one note of the chord at a time: {got:?}");
        // The chord alone plays all three.
        let rec2 = Arc::new(Mutex::new(Recorder::default()));
        let mut plain = NoteFx::new(rec2.clone() as SharedInstrument, vec![chord(triad)]);
        plain.note_on(0, 60, 0.5);
        run_all(&mut plain, 1, 6000);
        assert_eq!(heard(&rec2).iter().filter(|s| s.starts_with("on ")).count(), 3);
    }

    /// The settings atomics round-trip what the plugin's parameters say.
    #[test]
    fn settings_as_the_plugin_says() {
        let s = ArpSettings::default();
        assert_eq!(s.values(), ArpValues { pattern: 0, octaves: 2, rate: 3, gate: 50, swing: 0, hold: false, latch: false });
        s.write(ArpValues { pattern: 2, octaves: 4, rate: 1, gate: 90, swing: -20, hold: true, latch: false });
        assert_eq!(s.values(), ArpValues { pattern: 2, octaves: 4, rate: 1, gate: 90, swing: -20, hold: true, latch: false });
        let t = TransposeSettings::default();
        assert_eq!(t.values(), TransposeValues { semitones: 0, octaves: 0, scale_aware: false, root: 0, scale: 0 });
        t.write(TransposeValues { semitones: -5, octaves: 2, scale_aware: true, root: 3, scale: 7 });
        assert_eq!(t.values(), TransposeValues { semitones: -5, octaves: 2, scale_aware: true, root: 3, scale: 7 });
        let c = ChordSettings::default();
        assert_eq!(c.values().chord, 0);
        c.write(ChordValues { root: 9, scale: 1, chord: 4, inversion: 2, voicing: 3, strum: 2, strum_speed: 100, hold: true });
        assert_eq!(c.values(), ChordValues { root: 9, scale: 1, chord: 4, inversion: 2, voicing: 3, strum: 2, strum_speed: 100, hold: true });
    }

    /// A track with a MIDI FX slot filled plays its row through the chain; switching the slot off plays the instrument
    /// directly again.
    #[test]
    fn a_track_with_midi_fx_plays_through_its_chain() {
        use crate::playlist::Cmd;
        let mut a = crate::app::App::new(None);
        a.state.project = crate::app::test_project();
        a.plugins = crate::plugins::Plugins::builtins();
        a.state.plugdb.rebuild(&a.plugins);
        let row = crate::app::add_plugin_channel(&mut a.state.project, crate::osc3::LEMON_TRIO);
        for _ in 0..6 {
            a.plugin_tick();
        }
        assert!(a.state.hosted_rows.contains(&row), "the Trio is hosted: {:?}", a.state.message);
        crate::playlist::run(&mut a, Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi));
        let at = a.state.project.tracks.len() - 1;
        a.state.project.tracks[at].channel = Some(row as u16);
        let id = a.state.project.tracks[at].id.0;
        let index = a.plugins.midi_fx.iter().position(|x| x.desc.name == LEMON_ARP).expect("Lemon Arp is built in");
        assert!(crate::midi_fx::add(&mut a, id, None, index).is_none());
        a.sync_note_fx();
        let n = a.plugins.note_fx.get(&row).expect("the row plays through its chain");
        assert!(n.stages.len() == 1, "one stage: {}", n.stages.len());
        let inner = a.plugins.hosted[&row].plugin.clone() as SharedInstrument;
        // The engine takes the list's last instrument for a row, so the chain is what plays.
        let played = a.plugins.instruments().into_iter().filter(|(r, _)| *r == row).next_back().expect("the row plays").1;
        assert!(!std::sync::Arc::ptr_eq(&played, &inner), "the chain, not the bare instrument");
        let ChainSettings::Arp(arp) = &n.stages[0].1 else { panic!("the arpeggiator's settings") };
        assert_eq!(arp.values(), ArpSettings::default().values());
        // Off: the row plays its instrument directly again.
        crate::midi_fx::toggle(&mut a.state, id, 0);
        a.sync_note_fx();
        assert!(a.plugins.note_fx.is_empty(), "the chain is gone");
        let played = a.plugins.instruments().into_iter().filter(|(r, _)| *r == row).next_back().expect("the row plays").1;
        assert!(std::sync::Arc::ptr_eq(&played, &inner), "the instrument itself");
    }
}
