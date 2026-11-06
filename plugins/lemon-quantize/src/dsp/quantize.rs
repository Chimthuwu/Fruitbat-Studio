// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Quantize: snaps note starts to a grid (pure, no audio I/O).

use nih_plug::prelude::NoteEvent;

use crate::params::Settings;

/// A division's length in beats (quarter notes): 1/1 .. 1/32, then the triplets 1/2T .. 1/32T (the order of the
/// division setting).
pub fn division_beats(index: u8) -> f64 {
    const STRAIGHT: [f64; 6] = [4.0, 2.0, 1.0, 0.5, 0.25, 0.125];
    let i = (index as usize).min(10);
    if i < 6 { STRAIGHT[i] } else { STRAIGHT[i - 5] * 2.0 / 3.0 }
}

/// An event waiting for its beat.
struct Pending {
    beat: f64,
    seq: u64,
    ev: NoteEvent<()>,
}

/// Timed output: events are held by absolute beat and released in the block they fall in.
#[derive(Default)]
struct Queue {
    items: Vec<Pending>,
    seq: u64,
}

impl Queue {
    /// Queues `ev` for `beat`; returns its sequence number (for `cancel`).
    fn push(&mut self, beat: f64, ev: NoteEvent<()>) -> u64 {
        self.seq += 1;
        self.items.push(Pending { beat, seq: self.seq, ev });
        self.seq
    }

    fn cancel(&mut self, seq: u64) -> bool {
        let n = self.items.len();
        self.items.retain(|p| p.seq != seq);
        n != self.items.len()
    }

    /// Moves every event due before `end` into `out`, in beat order, with sample timings within the block of
    /// `len` samples starting at beat `pos` (`spb` = samples per beat).
    fn release(&mut self, pos: f64, end: f64, spb: f64, len: u32, out: &mut Vec<NoteEvent<()>>) {
        let mut due: Vec<Pending> = Vec::new();
        let mut i = 0;
        while i < self.items.len() {
            if self.items[i].beat < end {
                due.push(self.items.swap_remove(i));
            } else {
                i += 1;
            }
        }
        due.sort_by(|a, b| a.beat.total_cmp(&b.beat).then(a.seq.cmp(&b.seq)));
        for p in due {
            let at = (((p.beat - pos) * spb).round().max(0.0) as u32).min(len.saturating_sub(1));
            out.push(with_timing(p.ev, at));
        }
    }

    fn clear(&mut self) {
        self.items.clear();
    }
}

/// `ev` with its sample timing set.
fn with_timing(ev: NoteEvent<()>, at: u32) -> NoteEvent<()> {
    match ev {
        NoteEvent::NoteOn { voice_id, channel, note, velocity, .. } => NoteEvent::NoteOn { timing: at, voice_id, channel, note, velocity },
        NoteEvent::NoteOff { voice_id, channel, note, velocity, .. } => NoteEvent::NoteOff { timing: at, voice_id, channel, note, velocity },
        NoteEvent::PolyPressure { voice_id, channel, note, pressure, .. } => NoteEvent::PolyPressure { timing: at, voice_id, channel, note, pressure },
        NoteEvent::MidiCC { channel, cc, value, .. } => NoteEvent::MidiCC { timing: at, channel, cc, value },
        NoteEvent::MidiPitchBend { channel, value, .. } => NoteEvent::MidiPitchBend { timing: at, channel, value },
        NoteEvent::MidiChannelPressure { channel, pressure, .. } => NoteEvent::MidiChannelPressure { timing: at, channel, pressure },
        other => other,
    }
}

/// The grid line nearest to `start` (beats), for a grid of `grid` beats (ties round up).
pub fn nearest_grid(start: f64, grid: f64) -> f64 {
    (start / grid + 0.5).floor() * grid
}

/// The quantized start: `start + (grid_pos - start) * strength` (strength 0..=1).
pub fn quantize_start(start: f64, grid: f64, strength: f64) -> f64 {
    start + (nearest_grid(start, grid) - start) * strength
}

/// The quantizer. A note on that moves later is held until its beat; one that moves earlier is pulled back as far as
/// the start of the current block (a live note cannot play in the past). The note off moves by the same amount, so
/// lengths are kept (never ending before the note on).
pub struct Quantize {
    queue: Queue,
    /// Per channel and note: how far its note on moved (beats) and where it plays.
    held: Box<[[Option<(f64, f64)>; 128]; 16]>,
}

impl Default for Quantize {
    fn default() -> Self {
        Quantize { queue: Queue::default(), held: Box::new([[None; 128]; 16]) }
    }
}

impl Quantize {
    pub fn reset(&mut self) {
        self.queue.clear();
        for ch in self.held.iter_mut() {
            ch.fill(None);
        }
    }

    /// One block: `input` (timings in samples) from beat `pos`, at `tempo` bpm and `sr` Hz, `len` samples long.
    /// The released events go to `out`.
    pub fn block(&mut self, s: &Settings, pos: f64, tempo: f64, sr: f64, len: u32, input: &[NoteEvent<()>], out: &mut Vec<NoteEvent<()>>) {
        let spb = sr * 60.0 / tempo.max(1.0);
        let grid = division_beats(s.division);
        let strength = s.strength.min(100) as f64 / 100.0;
        for ev in input {
            let t = pos + ev.timing() as f64 / spb;
            match *ev {
                NoteEvent::NoteOn { channel, note, .. } => {
                    let to = quantize_start(t, grid, strength).max(pos);
                    self.held[(channel & 15) as usize][note as usize & 127] = Some((to - t, to));
                    self.queue.push(to, *ev);
                }
                NoteEvent::NoteOff { channel, note, .. } => {
                    let to = match self.held[(channel & 15) as usize][note as usize & 127].take() {
                        Some((shift, on_at)) => (t + shift).max(on_at),
                        None => t,
                    };
                    self.queue.push(to, *ev);
                }
                other => {
                    self.queue.push(t, other);
                }
            }
        }
        self.queue.release(pos, pos + len as f64 / spb, spb, len, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(timing: u32, note: u8) -> NoteEvent<()> {
        NoteEvent::NoteOn { timing, voice_id: None, channel: 0, note, velocity: 1.0 }
    }
    fn off(timing: u32, note: u8) -> NoteEvent<()> {
        NoteEvent::NoteOff { timing, voice_id: None, channel: 0, note, velocity: 0.0 }
    }

    #[test]
    fn division_lengths() {
        assert_eq!(division_beats(0), 4.0);
        assert_eq!(division_beats(2), 1.0);
        assert_eq!(division_beats(4), 0.25);
        assert_eq!(division_beats(5), 0.125);
        assert!((division_beats(6) - 4.0 / 3.0).abs() < 1e-12); // 1/2T
        assert!((division_beats(7) - 2.0 / 3.0).abs() < 1e-12); // 1/4T
        assert!((division_beats(8) - 1.0 / 3.0).abs() < 1e-12); // 1/8T
        assert!((division_beats(10) - 1.0 / 12.0).abs() < 1e-12); // 1/32T
    }

    #[test]
    fn grid_math() {
        assert_eq!(nearest_grid(0.2, 0.25), 0.25);
        assert_eq!(nearest_grid(0.1, 0.25), 0.0);
        assert_eq!(nearest_grid(0.125, 0.25), 0.25); // tie up
        // new = start + (grid - start) * strength
        assert!((quantize_start(0.2, 0.25, 1.0) - 0.25).abs() < 1e-12);
        assert!((quantize_start(0.2, 0.25, 0.0) - 0.2).abs() < 1e-12);
        assert!((quantize_start(0.2, 0.25, 0.5) - 0.225).abs() < 1e-12);
        assert!((quantize_start(0.1, 0.25, 0.5) - 0.05).abs() < 1e-12);
        // Triplet grid
        let g = division_beats(8);
        assert!((quantize_start(0.30, g, 1.0) - g).abs() < 1e-12);
    }

    #[test]
    fn late_notes_are_held_and_early_ones_pulled_back() {
        // 120 bpm, 48000 Hz: 24000 samples per beat. 1/16 grid = 6000 samples; full strength.
        let s = Settings { division: 4, strength: 100 };
        let mut q = Quantize::default();
        let mut out = Vec::new();
        // Block of 1000 samples at beat 0, note on at sample 800: grid 0 (nearest), pulled back to block start.
        q.block(&s, 0.0, 120.0, 48000.0, 1000, &[on(800, 60)], &mut out);
        assert_eq!(out, vec![on(0, 60)]);
        // Note on at sample 5000 (beat .208): nearest grid .25 = sample 6000, beyond a 1000 block: held.
        out.clear();
        let mut q = Quantize::default();
        q.block(&s, 0.0, 120.0, 48000.0, 5500, &[on(5000, 62)], &mut out);
        assert!(out.is_empty());
        // The next block starts at beat 5500/24000: it releases at sample 500.
        q.block(&s, 5500.0 / 24000.0, 120.0, 48000.0, 1000, &[], &mut out);
        assert_eq!(out, vec![on(500, 62)]);
    }

    #[test]
    fn note_off_keeps_the_length() {
        let s = Settings { division: 4, strength: 100 };
        let mut q = Quantize::default();
        let mut out = Vec::new();
        // On at sample 5000 (-> 6000, +1000), off at 11000 (-> 12000): one block of 20000 holds both.
        q.block(&s, 0.0, 120.0, 48000.0, 20000, &[on(5000, 60), off(11000, 60)], &mut out);
        assert_eq!(out, vec![on(6000, 60), off(12000, 60)]);
    }

    #[test]
    fn zero_strength_is_transparent_and_other_events_pass() {
        let s = Settings { division: 2, strength: 0 };
        let mut q = Quantize::default();
        let mut out = Vec::new();
        let cc = NoteEvent::<()>::MidiCC { timing: 700, channel: 0, cc: 1, value: 0.5 };
        q.block(&s, 3.0, 120.0, 48000.0, 1000, &[on(100, 60), cc, off(900, 60)], &mut out);
        assert_eq!(out, vec![on(100, 60), cc, off(900, 60)]);
    }

    #[test]
    fn off_never_precedes_its_on() {
        let s = Settings { division: 0, strength: 100 };
        let mut q = Quantize::default();
        let mut out = Vec::new();
        // Both at nearly the same time: on rounds to beat 0 (pulled to pos 0), off keeps order.
        q.block(&s, 0.0, 120.0, 48000.0, 2000, &[on(100, 60), off(200, 60)], &mut out);
        assert!(matches!(out[0], NoteEvent::NoteOn { .. }) && matches!(out[1], NoteEvent::NoteOff { .. }));
        assert!(out[0].timing() <= out[1].timing());
    }
}
