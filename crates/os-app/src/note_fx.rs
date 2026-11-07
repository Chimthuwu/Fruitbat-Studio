// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! A MIDI track's MIDI FX on the note path: the notes its clips play go through the track's MIDI FX chain, left to
//! right, before they reach the instrument (spec: track-model-plan.md §9, plugins/lemon-arp.md). The device rack shows
//! the chain; this is what it does to the sound.
//!
//! The engine gives a channel's instrument each tick's notes with their frame offsets and then one `process` call
//! (`os_engine::Instrument`), so a MIDI FX that moves notes in time — an arpeggiator — fits the instrument's own seat:
//! [`NoteFx`] takes the channel's place in the engine's instrument list, keeps the notes for itself, and plays the
//! instrument what the chain makes of them.
//!
//! Only Lemon Arp transforms notes today (the first one in a chain); the other MIDI FX pass them through unchanged.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;

use os_engine::{ChannelLevels, ChannelVoicing, Instrument, NoteLevels, SharedInstrument, TransportInfo};

/// The plugin name that selects the arpeggiator (`Plugins::midi_fx`).
pub const LEMON_ARP: &str = "Lemon Arp";

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

/// One note the chain is given, with the frame of the next block it falls on.
#[derive(Debug, Clone, Copy, PartialEq)]
enum In {
    On { offset: u32, key: u8, velocity: f32 },
    Off { offset: u32, key: u8 },
}

impl In {
    fn offset(self) -> u32 {
        match self {
            In::On { offset, .. } | In::Off { offset, .. } => offset,
        }
    }
}

/// A note the chain puts out.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Out {
    On { key: u8, velocity: f32 },
    Off(u8),
}

/// The arpeggiator: the keys held, the step it is on, and when its next step and its sounding note's end fall.
#[derive(Debug)]
pub struct Arp {
    settings: Arc<ArpSettings>,
    /// The keys held, in key order, with the velocity they were pressed at.
    held: Vec<(u8, f32)>,
    /// How far through the seen keys' sequence the pattern is.
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

    /// A note reaches the chain.
    pub fn note_on(&mut self, key: u8, velocity: f32) {
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
    pub fn note_off(&mut self, key: u8) {
        let v = self.settings.values();
        if v.hold || v.latch {
            return;
        }
        self.held.retain(|(k, _)| *k != key);
        if self.held.is_empty() {
            self.cut = true;
        }
    }

    /// Everything stops (Stop, a seek, the chain's slot switched off).
    pub fn reset(&mut self) {
        self.held.clear();
        self.step = 0;
        self.clock = 0.0;
        self.steps = 0;
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

    /// Plays the steps that start before block frame `limit` into `out` (frames of the block's `n`).
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

/// A MIDI track's instrument as its MIDI FX chain makes of it: the notes its clips play reach the instrument through
/// the chain.
pub struct NoteFx {
    inner: SharedInstrument,
    /// The chain's arpeggiator, if it has one (the first Lemon Arp in slot order).
    arp: Option<Arp>,
    /// This tick's notes, with their frames, waiting for the next `process`.
    notes: Vec<In>,
    /// The transport's tempo (the arpeggiator's clock).
    tempo: f64,
}

impl NoteFx {
    pub fn new(inner: SharedInstrument, arp: Option<Arc<ArpSettings>>) -> NoteFx {
        NoteFx { inner, arp: arp.map(Arp::new), notes: Vec::new(), tempo: 120.0 }
    }

    /// Renders the chain's notes into the instrument. True when the instrument took the block.
    fn play(&mut self, rate: u32, n: usize, left: &mut [f32], right: &mut [f32]) -> bool {
        let Ok(mut inner) = self.inner.try_lock() else { return false };
        let Some(arp) = &mut self.arp else {
            for note in std::mem::take(&mut self.notes) {
                match note {
                    In::On { offset, key, velocity } => inner.note_on(offset.min(n as u32), key, velocity),
                    In::Off { offset, key } => inner.note_off(offset.min(n as u32), key),
                }
            }
            inner.process(rate, left, right);
            return true;
        };
        let mut events: Vec<(u32, Out)> = Vec::with_capacity(self.notes.len() + 8);
        let mut at = 0.0f64;
        for note in std::mem::take(&mut self.notes) {
            let offset = f64::from(note.offset().min(n as u32));
            if offset > at {
                arp.advance(offset, rate, self.tempo, &mut events);
                at = offset;
            }
            match note {
                In::On { key, velocity, .. } => arp.note_on(key, velocity),
                In::Off { key, .. } => arp.note_off(key),
            }
        }
        let end = f64::from(n as u32);
        arp.advance(end, rate, self.tempo, &mut events);
        arp.end_block(end);
        events.sort_by_key(|(o, _)| *o);
        for (offset, ev) in events {
            match ev {
                Out::On { key, velocity } => inner.note_on(offset.min(n as u32), key, velocity),
                Out::Off(key) => inner.note_off(offset.min(n as u32), key),
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
        self.arp.as_ref().is_some_and(Arp::ringing) || self.inner.try_lock().is_ok_and(|i| i.active())
    }

    fn voices(&self) -> usize {
        self.inner.try_lock().map_or(0, |i| i.voices())
    }

    fn reset(&mut self) {
        self.notes.clear();
        if let Some(arp) = &mut self.arp {
            arp.reset();
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
        let fx = NoteFx::new(rec.clone() as SharedInstrument, Some(settings.clone()));
        (fx, rec, settings)
    }

    fn info(tempo: f64) -> TransportInfo {
        TransportInfo { playing: true, tempo, frame: 0, beats: 0.0, bar_start: 0.0, bar: 0, numerator: 4, denominator: 4, looping: None }
    }

    fn run(fx: &mut NoteFx, rate: u32, blocks: usize, n: usize) -> Vec<f32> {
        let mut out = vec![0.0; 2 * n];
        for _ in 0..blocks {
            let (l, r) = out.split_at_mut(n);
            fx.process(rate, l, r);
        }
        out
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
        let mut heard = Vec::new();
        for _ in 0..6 {
            run(&mut fx, 48_000, 1, 6000);
            heard.extend(rec.lock().unwrap().got.drain(..));
        }
        let ons: Vec<&String> = heard.iter().filter(|s| s.starts_with("on ")).collect();
        assert_eq!(
            ons,
            vec!["on 60 @0 v100", "on 64 @0 v100", "on 67 @0 v100", "on 60 @0 v100", "on 64 @0 v100", "on 67 @0 v100"],
            "one key at a time, in order, the pattern starting over: {heard:?}"
        );
        // Each note is gated: half a step long.
        assert!(heard.contains(&"off 60 @3000".to_string()), "{heard:?}");
    }

    /// Nothing held, nothing plays: the chord's notes do not reach the instrument as a chord.
    #[test]
    fn the_chord_itself_never_reaches_the_instrument() {
        let (mut fx, rec, _) = setup(ArpValues { pattern: 0, octaves: 1, rate: 2, gate: 50, swing: 0, hold: false, latch: false });
        fx.transport(&info(120.0));
        fx.note_on(0, 60, 0.5);
        fx.note_on(0, 64, 0.5);
        run(&mut fx, 48_000, 1, 100);
        let heard: Vec<String> = rec.lock().unwrap().got.drain(..).collect();
        // The step at once, then silence: two keys, one at a time.
        assert!(heard.iter().filter(|s| s.starts_with("on ")).count() <= 1, "{heard:?}");
    }

    /// Releasing the chord ends the arpeggio.
    #[test]
    fn releasing_stops_it() {
        let (mut fx, rec, _) = setup(ArpValues { pattern: 0, octaves: 1, rate: 0, gate: 100, swing: 0, hold: false, latch: false });
        fx.transport(&info(120.0));
        fx.note_on(0, 60, 0.5);
        fx.note_on(0, 64, 0.5);
        run(&mut fx, 48_000, 1, 10);
        rec.lock().unwrap().got.clear();
        fx.note_off(0, 60);
        fx.note_off(0, 64);
        run(&mut fx, 48_000, 1, 10);
        let heard: Vec<String> = rec.lock().unwrap().got.drain(..).collect();
        assert_eq!(heard, vec!["off 60 @0".to_string()], "the sounding note ends at once: {heard:?}");
    }

    /// Octaves put the pattern up an octave per pass.
    #[test]
    fn octaves_stack_the_pattern() {
        let (mut fx, rec, _) = setup(ArpValues { pattern: 0, octaves: 2, rate: 2, gate: 50, swing: 0, hold: false, latch: false });
        fx.transport(&info(120.0));
        fx.note_on(0, 60, 0.5);
        let mut ons = Vec::new();
        for _ in 0..4 {
            run(&mut fx, 48_000, 1, 6000);
            for s in rec.lock().unwrap().got.drain(..) {
                if s.starts_with("on ") {
                    ons.push(s);
                }
            }
        }
        assert_eq!(ons, vec!["on 60 @0 v64", "on 72 @0 v64", "on 60 @0 v64", "on 72 @0 v64"]);
    }

    /// With no MIDI FX the notes reach the instrument as they come, offsets and all.
    #[test]
    fn a_chain_without_an_arp_passes_notes_through() {
        let rec = Arc::new(Mutex::new(Recorder::default()));
        let mut fx = NoteFx::new(rec.clone() as SharedInstrument, None);
        fx.note_on(3, 60, 0.5);
        fx.note_off(4, 60);
        run(&mut fx, 48_000, 1, 100);
        assert_eq!(rec.lock().unwrap().got, vec!["on 60 @3 v64".to_string(), "off 60 @4".to_string()]);
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
        let t = a.state.project.tracks[at].clone();
        // The track over the Trio's channel.
        a.state.project.tracks[at].channel = Some(row as u16);
        let index = a.plugins.midi_fx.iter().position(|x| x.desc.name == LEMON_ARP).expect("Lemon Arp is built in");
        assert!(crate::midi_fx::add(&mut a, t.id.0, None, index).is_none());
        a.sync_note_fx();
        let n = a.plugins.note_fx.get(&row).expect("the row plays through its chain");
        assert!(n.arp.is_some(), "the chain has the arpeggiator");
        let inner = a.plugins.hosted[&row].plugin.clone() as SharedInstrument;
        // The engine takes the list's last instrument for a row, so the chain is what plays.
        let played = a.plugins.instruments().into_iter().filter(|(r, _)| *r == row).next_back().expect("the row plays").1;
        assert!(!std::sync::Arc::ptr_eq(&played, &inner), "the chain, not the bare instrument");
        // The settings come from the plugin's own parameters (its factory: up, 2 octaves, 1/32, 50 %).
        let arp = n.arp.clone().unwrap();
        assert_eq!(arp.values(), ArpSettings::default().values());
        // Off: the row plays its instrument directly again.
        crate::midi_fx::toggle(&mut a.state, t.id.0, 0);
        a.sync_note_fx();
        assert!(a.plugins.note_fx.is_empty(), "the chain is gone");
        let played = a.plugins.instruments().into_iter().filter(|(r, _)| *r == row).next_back().expect("the row plays").1;
        assert!(std::sync::Arc::ptr_eq(&played, &inner), "the instrument itself");
    }

    /// The settings atomics round-trip what the plugin's parameters say.
    #[test]
    fn settings_as_the_plugin_says() {
        let s = ArpSettings::default();
        assert_eq!(
            s.values(),
            ArpValues { pattern: 0, octaves: 2, rate: 3, gate: 50, swing: 0, hold: false, latch: false }
        );
        s.write(ArpValues { pattern: 2, octaves: 4, rate: 1, gate: 90, swing: -20, hold: true, latch: false });
        assert_eq!(
            s.values(),
            ArpValues { pattern: 2, octaves: 4, rate: 1, gate: 90, swing: -20, hold: true, latch: false }
        );
    }
}
