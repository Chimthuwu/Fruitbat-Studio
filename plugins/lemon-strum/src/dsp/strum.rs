// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Strum: staggers the starts of simultaneous (chord) notes like a guitar strum (pure, no audio I/O).

use crate::params::{idx, Settings};

/// A note event at a sample offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ev {
    pub time: u32,
    pub on: bool,
    pub note: u8,
    pub vel: u8,
    pub ch: u8,
}

impl Ev {
    pub fn on(time: u32, note: u8, vel: u8) -> Ev {
        Ev { time, on: true, note, vel, ch: 0 }
    }
    pub fn off(time: u32, note: u8) -> Ev {
        Ev { time, on: false, note, vel: 0, ch: 0 }
    }
}

/// Samples in one beat.
pub fn samples_per_beat(bpm: f64, sr: u32) -> f64 {
    sr as f64 * 60.0 / bpm.clamp(1.0, 999.0)
}

/// Milliseconds to samples.
pub fn ms_to_samples(ms: i32, sr: u32) -> u64 {
    (ms.max(0) as f64 * sr as f64 / 1000.0).round() as u64
}

/// Events scheduled at absolute sample times, released block by block in time order.
#[derive(Default)]
pub struct Pending {
    q: Vec<(u64, u64, Ev)>,
    seq: u64,
}

impl Pending {
    pub fn push(&mut self, abs: u64, ev: Ev) {
        self.seq += 1;
        self.q.push((abs, self.seq, ev));
    }

    /// Drops scheduled events matching `f(abs, ev)`; how many went.
    pub fn cancel(&mut self, f: impl Fn(u64, &Ev) -> bool) -> usize {
        let n = self.q.len();
        self.q.retain(|(a, _, e)| !f(*a, e));
        n - self.q.len()
    }

    /// The events in `[now, now + len)`, offsets relative to `now`.
    pub fn drain(&mut self, now: u64, len: u32) -> Vec<Ev> {
        let end = now + len as u64;
        let mut due: Vec<(u64, u64, Ev)> = self.q.iter().filter(|(a, _, _)| *a < end).copied().collect();
        self.q.retain(|(a, _, _)| *a >= end);
        due.sort_by_key(|(a, s, _)| (*a, *s));
        due.into_iter().map(|(a, _, mut e)| { e.time = a.saturating_sub(now) as u32; e }).collect()
    }

    pub fn clear(&mut self) {
        self.q.clear();
    }

    pub fn len(&self) -> usize {
        self.q.len()
    }

    pub fn is_empty(&self) -> bool {
        self.q.is_empty()
    }
}

/// Each chord note's start offset in samples, aligned with `pitches`. The chord is ordered by pitch (lowest first for
/// up, highest first for down); the first sounds at once, the last `spread` samples later, the rest evenly between.
pub fn strum_offsets(pitches: &[u8], down: bool, spread: u64) -> Vec<u64> {
    let n = pitches.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| (pitches[i], i));
    if down {
        order.reverse();
    }
    let mut out = vec![0; n];
    for (rank, &i) in order.iter().enumerate() {
        out[i] = if n > 1 { (spread as f64 * rank as f64 / (n - 1) as f64).round() as u64 } else { 0 };
    }
    out
}

pub struct Strum {
    sr: u32,
    s: Settings,
    now: u64,
    /// Offsets of notes strummed (their off slides the same amount).
    offsets: Vec<(u8, u8, u64)>,
    pending: Pending,
}

impl Strum {
    pub fn with_settings(sr: u32, s: &Settings) -> Self {
        Strum { sr, s: *s, now: 0, offsets: Vec::new(), pending: Pending::default() }
    }

    pub fn set_sample_rate(&mut self, rate: u32) {
        self.sr = rate;
    }

    pub fn set(&mut self, s: &Settings) {
        self.s = *s;
    }

    pub fn set_tempo(&mut self, _bpm: f32) {}

    pub fn reset(&mut self) {
        self.pending.clear();
        self.offsets.clear();
        self.now = 0;
    }

    /// One block. Note-ons within `WINDOW` ms of the first of a group count as one chord; chords split by a block
    /// boundary are not joined. A single note is left alone.
    pub fn process(&mut self, input: &[Ev], block: u32) -> Vec<Ev> {
        let spread = ms_to_samples(self.s.v[idx::SPREAD], self.sr);
        let window = ms_to_samples(self.s.v[idx::WINDOW], self.sr);
        let down = self.s.v[idx::DIRECTION] == 1;
        let mut ons: Vec<usize> = (0..input.len()).filter(|&i| input[i].on).collect();
        ons.sort_by_key(|&i| input[i].time);
        let mut shift = vec![0u64; input.len()];
        let mut g = 0;
        while g < ons.len() {
            let start = input[ons[g]].time as u64;
            let mut end = g + 1;
            while end < ons.len() && input[ons[end]].time as u64 - start <= window {
                end += 1;
            }
            let group = &ons[g..end];
            let pitches: Vec<u8> = group.iter().map(|&i| input[i].note).collect();
            for (k, off) in strum_offsets(&pitches, down, spread).into_iter().enumerate() {
                shift[group[k]] = off;
            }
            g = end;
        }
        for (i, e) in input.iter().enumerate() {
            let abs = self.now + e.time as u64;
            if e.on {
                self.offsets.retain(|o| (o.0, o.1) != (e.note, e.ch));
                self.offsets.push((e.note, e.ch, shift[i]));
                self.pending.push(abs + shift[i], *e);
            } else {
                let by = match self.offsets.iter().position(|o| (o.0, o.1) == (e.note, e.ch)) {
                    Some(p) => self.offsets.remove(p).2,
                    None => 0,
                };
                self.pending.push(abs + by, *e);
            }
        }
        let out = self.pending.drain(self.now, block);
        self.now += block as u64;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(f: impl Fn(&mut [i32; 3])) -> Settings {
        let mut s = Settings::default();
        f(&mut s.v);
        s
    }

    #[test]
    fn spread_math_up() {
        // C major, input order scrambled: sorted by pitch the lowest goes first
        assert_eq!(strum_offsets(&[67, 60, 64], false, 1000), vec![1000, 0, 500]);
        assert_eq!(strum_offsets(&[60, 64, 67, 72], false, 900), vec![0, 300, 600, 900]);
    }

    #[test]
    fn spread_math_down() {
        assert_eq!(strum_offsets(&[60, 64, 67], true, 1000), vec![1000, 500, 0]);
        assert_eq!(strum_offsets(&[60, 64, 67, 72], true, 900), vec![900, 600, 300, 0]);
    }

    #[test]
    fn single_note_and_zero_spread() {
        assert_eq!(strum_offsets(&[60], false, 1000), vec![0]);
        assert_eq!(strum_offsets(&[60, 64], false, 0), vec![0, 0]);
        assert!(strum_offsets(&[], true, 10).is_empty());
    }

    #[test]
    fn equal_pitches_keep_input_order() {
        assert_eq!(strum_offsets(&[60, 60], false, 100), vec![0, 100]);
    }

    fn run(st: &mut Strum, first: &[Ev]) -> Vec<(u64, Ev)> {
        let mut out = Vec::new();
        for b in 0..6u64 {
            let input: &[Ev] = if b == 0 { first } else { &[] };
            out.extend(st.process(input, 4800).into_iter().map(|e| (b * 4800 + e.time as u64, e)));
        }
        out
    }

    #[test]
    fn chord_is_strummed_up_and_offs_follow() {
        // 48 kHz: 10 ms = 480 samples
        let mut st = Strum::with_settings(48000, &s(|v| v[idx::SPREAD] = 20));
        let out = run(&mut st, &[
            Ev::on(100, 67, 90), Ev::on(100, 60, 90), Ev::on(100, 64, 90),
            Ev::off(3000, 60), Ev::off(3000, 64), Ev::off(3000, 67),
        ]);
        let ons: Vec<(u64, u8)> = out.iter().filter(|e| e.1.on).map(|e| (e.0, e.1.note)).collect();
        assert_eq!(ons, vec![(100, 60), (580, 64), (1060, 67)]);
        let offs: Vec<(u64, u8)> = out.iter().filter(|e| !e.1.on).map(|e| (e.0, e.1.note)).collect();
        assert_eq!(offs, vec![(3000, 60), (3480, 64), (3960, 67)]);
    }

    #[test]
    fn chord_down() {
        let mut st = Strum::with_settings(48000, &s(|v| {
            v[idx::SPREAD] = 20;
            v[idx::DIRECTION] = 1;
        }));
        let out = run(&mut st, &[Ev::on(0, 60, 90), Ev::on(0, 64, 90), Ev::on(0, 67, 90)]);
        let ons: Vec<(u64, u8)> = out.iter().filter(|e| e.1.on).map(|e| (e.0, e.1.note)).collect();
        assert_eq!(ons, vec![(0, 67), (480, 64), (960, 60)]);
    }

    #[test]
    fn only_simultaneous_notes_are_strummed() {
        // default window 10 ms = 480 samples: notes 1000 samples apart are separate notes
        let mut st = Strum::with_settings(48000, &s(|v| v[idx::SPREAD] = 100));
        let out = run(&mut st, &[Ev::on(0, 60, 90), Ev::on(1000, 64, 90)]);
        let ons: Vec<(u64, u8)> = out.iter().filter(|e| e.1.on).map(|e| (e.0, e.1.note)).collect();
        assert_eq!(ons, vec![(0, 60), (1000, 64)]);
        // a note inside the window joins the chord
        let mut st = Strum::with_settings(48000, &s(|v| v[idx::SPREAD] = 20));
        let out = run(&mut st, &[Ev::on(0, 64, 90), Ev::on(400, 60, 90)]);
        let ons: Vec<(u64, u8)> = out.iter().filter(|e| e.1.on).map(|e| (e.0, e.1.note)).collect();
        assert_eq!(ons, vec![(400, 60), (960, 64)]);
    }
}
