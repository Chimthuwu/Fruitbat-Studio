// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Random: random velocity / timing variation with density control and probabilistic note drop (pure, no audio I/O).

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

/// A small deterministic xorshift generator (no allocation, no OS calls: safe on the audio thread).
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// In [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// True with `percent` probability (0 never, 100 always).
    pub fn chance(&mut self, percent: i32) -> bool {
        percent >= 100 || (percent > 0 && self.unit() * 100.0 < percent as f64)
    }
}

/// A velocity moved by `r` in [-1, 1] of `amount` percent of the full range (127), kept in 1..=127.
pub fn vary_velocity(vel: u8, amount: i32, r: f64) -> u8 {
    let span = amount.clamp(0, 100) as f64 / 100.0 * 127.0;
    ((vel as f64 + r.clamp(-1.0, 1.0) * span).round() as i32).clamp(1, 127) as u8
}

/// A timing delay of `r` in [0, 1) of `max_ms`, in samples.
pub fn jitter_samples(max_ms: i32, r: f64, sr: u32) -> u64 {
    (max_ms.max(0) as f64 * r.clamp(0.0, 1.0) * sr as f64 / 1000.0).round() as u64
}

struct Fate {
    note: u8,
    ch: u8,
    delay: u64,
    dropped: bool,
}

pub struct Random {
    sr: u32,
    s: Settings,
    now: u64,
    rng: Rng,
    fates: Vec<Fate>,
    pending: Pending,
}

impl Random {
    pub fn with_settings(sr: u32, s: &Settings) -> Self {
        Random { sr, s: *s, now: 0, rng: Rng::new(0x9E37_79B9_7F4A_7C15), fates: Vec::new(), pending: Pending::default() }
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
        self.fates.clear();
        self.now = 0;
    }

    /// One block. Each note-on may be dropped (`DROP` percent); a kept note is varied with `DENSITY` percent chance:
    /// velocity moves up or down by up to `VELOCITY` percent, the start slides later by up to `TIMING` ms. Its note-off
    /// follows the same slide so the length stays.
    pub fn process(&mut self, input: &[Ev], block: u32) -> Vec<Ev> {
        for e in input {
            let abs = self.now + e.time as u64;
            if e.on {
                let dropped = self.rng.chance(self.s.v[idx::DROP]);
                let (mut ev, mut delay) = (*e, 0);
                if !dropped && self.rng.chance(self.s.v[idx::DENSITY]) {
                    ev.vel = vary_velocity(e.vel, self.s.v[idx::VELOCITY], self.rng.unit() * 2.0 - 1.0);
                    delay = jitter_samples(self.s.v[idx::TIMING], self.rng.unit(), self.sr);
                }
                self.fates.retain(|f| !(f.note == e.note && f.ch == e.ch));
                self.fates.push(Fate { note: e.note, ch: e.ch, delay, dropped });
                if !dropped {
                    self.pending.push(abs + delay, ev);
                }
            } else if let Some(i) = self.fates.iter().position(|f| f.note == e.note && f.ch == e.ch) {
                let f = self.fates.remove(i);
                if !f.dropped {
                    self.pending.push(abs + f.delay, *e);
                }
            } else {
                self.pending.push(abs, *e);
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

    fn s(f: impl Fn(&mut [i32; 4])) -> Settings {
        let mut s = Settings::default();
        f(&mut s.v);
        s
    }

    #[test]
    fn rng_is_deterministic_and_in_range() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        for _ in 0..1000 {
            let x = a.unit();
            assert!((0.0..1.0).contains(&x));
            assert_eq!(x, b.unit());
        }
    }

    #[test]
    fn chance_edges_and_rate() {
        let mut r = Rng::new(1);
        assert!((0..100).all(|_| !r.chance(0)));
        assert!((0..100).all(|_| r.chance(100)));
        let hits = (0..20000).filter(|_| r.chance(25)).count();
        assert!((4500..5500).contains(&hits), "{hits}");
    }

    #[test]
    fn velocity_variation_math() {
        assert_eq!(vary_velocity(64, 0, 1.0), 64);
        assert_eq!(vary_velocity(64, 50, 1.0), 127); // 64 + 63.5 clamps to 127
        assert_eq!(vary_velocity(64, 10, -1.0), 51);
        assert_eq!(vary_velocity(10, 100, -1.0), 1);
        assert_eq!(vary_velocity(120, 100, 1.0), 127);
    }

    #[test]
    fn jitter_math() {
        assert_eq!(jitter_samples(100, 1.0, 48000), 4800);
        assert_eq!(jitter_samples(100, 0.5, 48000), 2400);
        assert_eq!(jitter_samples(0, 0.9, 48000), 0);
    }

    fn collect(r: &mut Random, evs: &[Ev]) -> Vec<(u64, Ev)> {
        let mut out: Vec<(u64, Ev)> = r.process(evs, 4800).into_iter().map(|e| (e.time as u64, e)).collect();
        for b in 1..4u64 {
            out.extend(r.process(&[], 4800).into_iter().map(|e| (b * 4800 + e.time as u64, e)));
        }
        out
    }

    #[test]
    fn zeroed_amounts_leave_timing_and_drop_alone() {
        let mut r = Random::with_settings(48000, &s(|v| {
            v[idx::VELOCITY] = 0;
            v[idx::TIMING] = 0;
        }));
        let out = collect(&mut r, &[Ev::on(100, 60, 80), Ev::off(900, 60)]);
        assert_eq!(out.iter().map(|(t, e)| (*t, e.on)).collect::<Vec<_>>(), vec![(100, true), (900, false)]);
    }

    #[test]
    fn drop_all_drops_on_and_off() {
        let mut r = Random::with_settings(48000, &s(|v| v[idx::DROP] = 100));
        assert!(collect(&mut r, &[Ev::on(0, 60, 80), Ev::off(500, 60)]).is_empty());
    }

    #[test]
    fn timing_slides_on_and_off_together() {
        let mut r = Random::with_settings(48000, &s(|v| {
            v[idx::TIMING] = 100;
            v[idx::VELOCITY] = 0;
        }));
        let mut moved = 0;
        for _ in 0..50 {
            let out = collect(&mut r, &[Ev::on(0, 60, 80), Ev::off(1000, 60)]);
            assert_eq!(out.len(), 2);
            assert_eq!(out[1].0 - out[0].0, 1000, "length kept");
            assert!(out[0].0 <= 4800);
            if out[0].0 > 0 {
                moved += 1;
            }
            assert_eq!(out[0].1.vel, 80);
        }
        assert!(moved > 40);
    }

    #[test]
    fn zero_density_means_no_variation() {
        let mut r = Random::with_settings(48000, &s(|v| {
            v[idx::DENSITY] = 0;
            v[idx::TIMING] = 100;
            v[idx::VELOCITY] = 100;
        }));
        for _ in 0..20 {
            let out = collect(&mut r, &[Ev::on(10, 60, 80), Ev::off(20, 60)]);
            assert_eq!(out[0].0, 10);
            assert_eq!(out[0].1.vel, 80);
        }
    }
}
