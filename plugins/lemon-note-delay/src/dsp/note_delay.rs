// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Note Delay: delays notes by a synced division or milliseconds, optionally repeating them (pure, no audio I/O).

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

/// Length in beats of division `index`: base (1/1, 1/2, ...) x 3 variants (straight, dotted, triplet).
pub fn division_beats(index: i32) -> f64 {
    const BASE: [f64; 6] = [4.0, 2.0, 1.0, 0.5, 0.25, 0.125];
    let i = index.max(0) as usize;
    let base = BASE[(i / 3).min(BASE.len() - 1)];
    match i % 3 {
        0 => base,
        1 => base * 1.5,
        _ => base * 2.0 / 3.0,
    }
}

/// The delay in samples for `s` at `bpm`.
pub fn delay_for(s: &Settings, bpm: f64, sr: u32) -> u64 {
    if s.v[idx::UNIT] == 1 {
        ms_to_samples(s.v[idx::MS], sr)
    } else {
        (division_beats(s.v[idx::DIVISION]) * samples_per_beat(bpm, sr)).round() as u64
    }
}

/// Velocity of repeat `k` (1-based) when each repeat keeps `decay` percent of the one before.
pub fn repeat_velocity(vel: u8, k: u32, decay: i32) -> u8 {
    let f = (decay.clamp(0, 100) as f64 / 100.0).powi(k as i32);
    ((vel as f64 * f).round() as i32).clamp(1, 127) as u8
}

pub struct NoteDelay {
    sr: u32,
    s: Settings,
    bpm: f64,
    now: u64,
    pending: Pending,
}

impl NoteDelay {
    pub fn with_settings(sr: u32, s: &Settings) -> Self {
        NoteDelay { sr, s: *s, bpm: 120.0, now: 0, pending: Pending::default() }
    }

    pub fn set_sample_rate(&mut self, rate: u32) {
        self.sr = rate;
    }

    pub fn set(&mut self, s: &Settings) {
        self.s = *s;
    }

    pub fn set_tempo(&mut self, bpm: f32) {
        self.bpm = bpm as f64;
    }

    pub fn reset(&mut self) {
        self.pending.clear();
        self.now = 0;
    }

    /// The current delay in samples.
    pub fn delay_samples(&self) -> u64 {
        delay_for(&self.s, self.bpm, self.sr)
    }

    /// One block: notes in, delayed (and repeated) notes out. Without feedback only the delayed note sounds; with
    /// feedback the note plays at once and then repeats `feedback` times, each `decay` percent as loud.
    pub fn process(&mut self, input: &[Ev], block: u32) -> Vec<Ev> {
        let d = self.delay_samples();
        let fb = self.s.v[idx::FEEDBACK].max(0) as u64;
        let decay = self.s.v[idx::DECAY];
        for e in input {
            let abs = self.now + e.time as u64;
            if fb == 0 {
                self.pending.push(abs + d, *e);
            } else {
                self.pending.push(abs, *e);
                for k in 1..=fb {
                    let mut r = *e;
                    if r.on {
                        r.vel = repeat_velocity(e.vel, k as u32, decay);
                    }
                    self.pending.push(abs + k * d, r);
                }
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

    fn settings(f: impl Fn(&mut [i32; 5])) -> Settings {
        let mut s = Settings::default();
        f(&mut s.v);
        s
    }

    #[test]
    fn division_math() {
        assert_eq!(division_beats(0), 4.0);
        assert_eq!(division_beats(9), 0.5);
        assert_eq!(division_beats(10), 0.75);
        assert!((division_beats(11) - 1.0 / 3.0).abs() < 1e-12);
        assert_eq!(division_beats(15), 0.125);
        assert_eq!(division_beats(17), 0.125 * 2.0 / 3.0);
    }

    #[test]
    fn delay_in_samples() {
        // 120 BPM at 48 kHz: a beat is 24000 samples
        let eighth = settings(|v| v[idx::DIVISION] = 9);
        assert_eq!(delay_for(&eighth, 120.0, 48000), 12000);
        let dotted = settings(|v| v[idx::DIVISION] = 10);
        assert_eq!(delay_for(&dotted, 120.0, 48000), 18000);
        let trip = settings(|v| v[idx::DIVISION] = 11);
        assert_eq!(delay_for(&trip, 120.0, 48000), 8000);
        let ms = settings(|v| {
            v[idx::UNIT] = 1;
            v[idx::MS] = 250;
        });
        assert_eq!(delay_for(&ms, 120.0, 48000), 12000);
        assert_eq!(delay_for(&ms, 77.0, 48000), 12000, "ms ignores tempo");
    }

    #[test]
    fn delays_notes_and_keeps_length() {
        let s = settings(|v| v[idx::DIVISION] = 9);
        let mut d = NoteDelay::with_settings(48000, &s);
        let out = d.process(&[Ev::on(100, 60, 90), Ev::off(5000, 60)], 4800);
        assert!(out.is_empty());
        let mut got = Vec::new();
        for b in 0..5u64 {
            for e in d.process(&[], 4800) {
                got.push((e.time as u64 + (b + 1) * 4800, e));
            }
        }
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].0, got[0].1.on), (12100, true));
        assert_eq!((got[1].0, got[1].1.on), (17000, false));
    }

    #[test]
    fn feedback_repeats_decay() {
        let s = settings(|v| {
            v[idx::DIVISION] = 9;
            v[idx::FEEDBACK] = 3;
            v[idx::DECAY] = 50;
        });
        let mut d = NoteDelay::with_settings(48000, &s);
        let mut on = Vec::new();
        let mut input = vec![Ev::on(0, 64, 100)];
        for b in 0..10u64 {
            for e in d.process(&input, 4800) {
                if e.on {
                    on.push((e.time as u64 + b * 4800, e.vel));
                }
            }
            input.clear();
        }
        assert_eq!(on, vec![(0, 100), (12000, 50), (24000, 25), (36000, 13)]);
    }

    #[test]
    fn repeat_velocity_floor_and_ceiling() {
        assert_eq!(repeat_velocity(100, 1, 100), 100);
        assert_eq!(repeat_velocity(100, 20, 10), 1);
        assert_eq!(repeat_velocity(127, 0, 70), 127);
    }
}
