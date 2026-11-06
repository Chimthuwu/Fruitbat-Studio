// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Note Length: scales note lengths or sets them to a fixed division (pure, no audio I/O).
//!
//! A live note stream only knows a note's length when its note off arrives, so the plugin can lengthen a note
//! (the note off is held back) but cannot end it before the incoming note off: below 100 % the incoming off
//! passes unchanged. Fixed mode ends every note at `start + length` whatever the incoming off does, which also
//! shortens (a note off arriving first is swallowed). The track model's note path applies a shortening exactly,
//! since there the note lengths are known up front.

use nih_plug::prelude::NoteEvent;

use crate::params::{Mode, Settings};

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

/// The output length (beats) of a note whose incoming length is `len` beats. Percent mode scales it
/// (`amount` percent); Fixed mode gives `fixed`.
pub fn scaled_length(s: &Settings, len: f64) -> f64 {
    match s.mode {
        Mode::Percent => len * s.amount as f64 / 100.0,
        Mode::Fixed => division_beats(s.length),
    }
}

/// One sounding note.
#[derive(Clone, Copy)]
struct Active {
    start: f64,
    /// The queued note off of fixed mode (so a retrigger can cancel it).
    off_seq: Option<u64>,
    /// Fixed mode: the incoming off must not be passed (the plugin ends the note itself).
    fixed: bool,
}

/// The note-length processor.
pub struct NoteLength {
    queue: Queue,
    active: Box<[[Option<Active>; 128]; 16]>,
}

impl Default for NoteLength {
    fn default() -> Self {
        NoteLength { queue: Queue::default(), active: Box::new([[None; 128]; 16]) }
    }
}

impl NoteLength {
    pub fn reset(&mut self) {
        self.queue.clear();
        for ch in self.active.iter_mut() {
            ch.fill(None);
        }
    }

    /// One block: `input` (timings in samples) from beat `pos`, at `tempo` bpm and `sr` Hz, `len` samples long.
    pub fn block(&mut self, s: &Settings, pos: f64, tempo: f64, sr: f64, len: u32, input: &[NoteEvent<()>], out: &mut Vec<NoteEvent<()>>) {
        let spb = sr * 60.0 / tempo.max(1.0);
        for ev in input {
            let t = pos + ev.timing() as f64 / spb;
            match *ev {
                NoteEvent::NoteOn { channel, note, voice_id, .. } => {
                    let slot = (channel & 15) as usize;
                    let n = note as usize & 127;
                    // A retrigger ends the previous instance's pending off first.
                    if let Some(Active { off_seq: Some(q), .. }) = self.active[slot][n] {
                        self.queue.cancel(q);
                        self.queue.push(t, NoteEvent::NoteOff { timing: 0, voice_id, channel, note, velocity: 0.0 });
                    }
                    let fixed = s.mode == Mode::Fixed;
                    let off_seq = fixed.then(|| {
                        let off = NoteEvent::NoteOff { timing: 0, voice_id, channel, note, velocity: 0.0 };
                        self.queue.push(t + scaled_length(s, 0.0), off)
                    });
                    self.active[slot][n] = Some(Active { start: t, off_seq, fixed });
                    self.queue.push(t, *ev);
                }
                NoteEvent::NoteOff { channel, note, voice_id, velocity, .. } => {
                    let slot = (channel & 15) as usize;
                    let n = note as usize & 127;
                    match self.active[slot][n] {
                        Some(a) if a.fixed => {
                            // The plugin's own off ends this note: the incoming one is swallowed, but its release
                            // velocity is lost with it.
                            if !self.queue_has(a.off_seq) {
                                self.active[slot][n] = None;
                            }
                        }
                        Some(a) => {
                            self.active[slot][n] = None;
                            let end = t.max(a.start + scaled_length(s, t - a.start));
                            self.queue.push(end, NoteEvent::NoteOff { timing: 0, voice_id, channel, note, velocity });
                        }
                        None => {
                            self.queue.push(t, *ev);
                        }
                    }
                }
                other => {
                    self.queue.push(t, other);
                }
            }
        }
        self.queue.release(pos, pos + len as f64 / spb, spb, len, out);
        // A fixed note whose off has played is finished.
        let queue = &self.queue;
        for ch in self.active.iter_mut() {
            for a in ch.iter_mut() {
                if let Some(Active { off_seq: Some(q), .. }) = a {
                    if !queue.has(*q) {
                        *a = None;
                    }
                }
            }
        }
    }

    fn queue_has(&self, seq: Option<u64>) -> bool {
        seq.is_some_and(|q| self.queue.has(q))
    }
}

impl Queue {
    fn has(&self, seq: u64) -> bool {
        self.items.iter().any(|p| p.seq == seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::Mode;

    fn on(timing: u32, note: u8) -> NoteEvent<()> {
        NoteEvent::NoteOn { timing, voice_id: None, channel: 0, note, velocity: 1.0 }
    }
    fn off(timing: u32, note: u8) -> NoteEvent<()> {
        NoteEvent::NoteOff { timing, voice_id: None, channel: 0, note, velocity: 0.0 }
    }
    // 120 bpm, 48 kHz: 24000 samples per beat.
    const SR: f64 = 48000.0;

    #[test]
    fn scaled_lengths() {
        let p = |amount| Settings { mode: Mode::Percent, amount, length: 4 };
        assert_eq!(scaled_length(&p(50), 2.0), 1.0);
        assert_eq!(scaled_length(&p(100), 2.0), 2.0);
        assert_eq!(scaled_length(&p(400), 0.25), 1.0);
        let f = Settings { mode: Mode::Fixed, amount: 100, length: 3 };
        assert_eq!(scaled_length(&f, 9.0), 0.5);
    }

    #[test]
    fn percent_lengthens_by_holding_the_off() {
        // 200 %: a 1-beat note (on 0, off 24000) ends at beat 2 (sample 48000).
        let s = Settings { mode: Mode::Percent, amount: 200, length: 4 };
        let mut n = NoteLength::default();
        let mut out = Vec::new();
        n.block(&s, 0.0, 120.0, SR, 24000, &[on(0, 60)], &mut out);
        assert_eq!(out, vec![on(0, 60)]);
        out.clear();
        n.block(&s, 0.0 + 1.0, 120.0, SR, 24000, &[off(0, 60)], &mut out);
        assert!(out.is_empty(), "off held");
        n.block(&s, 2.0 - 0.0, 120.0, SR, 24000, &[], &mut out);
        assert_eq!(out, vec![off(0, 60)]);
    }

    #[test]
    fn percent_cannot_end_before_the_incoming_off() {
        let s = Settings { mode: Mode::Percent, amount: 50, length: 4 };
        let mut n = NoteLength::default();
        let mut out = Vec::new();
        n.block(&s, 0.0, 120.0, SR, 24000, &[on(0, 60), off(20000, 60)], &mut out);
        assert_eq!(out, vec![on(0, 60), off(20000, 60)]);
    }

    #[test]
    fn fixed_ends_at_the_division_and_swallows_the_incoming_off() {
        // 1/8 = half a beat = 12000 samples; the incoming note is a beat long.
        let s = Settings { mode: Mode::Fixed, amount: 100, length: 3 };
        let mut n = NoteLength::default();
        let mut out = Vec::new();
        n.block(&s, 0.0, 120.0, SR, 48000, &[on(1000, 60), off(25000, 60)], &mut out);
        assert_eq!(out, vec![on(1000, 60), off(13000, 60)]);
    }

    #[test]
    fn fixed_off_lands_in_a_later_block() {
        let s = Settings { mode: Mode::Fixed, amount: 100, length: 2 }; // 1/4: 24000 samples
        let mut n = NoteLength::default();
        let mut out = Vec::new();
        n.block(&s, 0.0, 120.0, SR, 10000, &[on(2000, 64)], &mut out);
        assert_eq!(out, vec![on(2000, 64)]);
        out.clear();
        n.block(&s, 10000.0 / 24000.0, 120.0, SR, 20000, &[], &mut out);
        assert_eq!(out, vec![off(16000, 64)]);
    }

    #[test]
    fn retrigger_ends_the_old_note() {
        let s = Settings { mode: Mode::Fixed, amount: 100, length: 0 }; // a whole bar
        let mut n = NoteLength::default();
        let mut out = Vec::new();
        n.block(&s, 0.0, 120.0, SR, 48000, &[on(0, 60), on(10000, 60)], &mut out);
        assert_eq!(out, vec![on(0, 60), off(10000, 60), on(10000, 60)]);
    }
}
