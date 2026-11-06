// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Note Filter: passes or blocks notes by key range, velocity range and channel (pure, no audio I/O).

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

/// Does a note match every criterion (key range, velocity range, channel)? `ch` is 0-based; a channel setting of 0 is any.
pub fn matches(s: &Settings, note: u8, vel: u8, ch: u8) -> bool {
    let (lo, hi) = (s.v[idx::LOW].min(s.v[idx::HIGH]), s.v[idx::LOW].max(s.v[idx::HIGH]));
    let (vlo, vhi) = (s.v[idx::VEL_LOW].min(s.v[idx::VEL_HIGH]), s.v[idx::VEL_LOW].max(s.v[idx::VEL_HIGH]));
    let chan = s.v[idx::CHANNEL];
    (lo..=hi).contains(&(note as i32)) && (vlo..=vhi).contains(&(vel as i32)) && (chan == 0 || chan == ch as i32 + 1)
}

/// Does a note-on get through? Pass mode lets matches through, block mode stops them.
pub fn passes(s: &Settings, note: u8, vel: u8, ch: u8) -> bool {
    matches(s, note, vel, ch) == (s.v[idx::MODE] == 0)
}

pub struct NoteFilter {
    s: Settings,
    /// Notes whose on went through (their off must too, whatever the settings are by then).
    sounding: Vec<(u8, u8)>,
}

impl NoteFilter {
    pub fn with_settings(_sr: u32, s: &Settings) -> Self {
        NoteFilter { s: *s, sounding: Vec::new() }
    }

    pub fn set_sample_rate(&mut self, _rate: u32) {}

    pub fn set(&mut self, s: &Settings) {
        self.s = *s;
    }

    pub fn set_tempo(&mut self, _bpm: f32) {}

    pub fn reset(&mut self) {
        self.sounding.clear();
    }

    pub fn process(&mut self, input: &[Ev], _block: u32) -> Vec<Ev> {
        let mut out = Vec::with_capacity(input.len());
        for e in input {
            if e.on {
                if passes(&self.s, e.note, e.vel, e.ch) {
                    self.sounding.retain(|k| *k != (e.note, e.ch));
                    self.sounding.push((e.note, e.ch));
                    out.push(*e);
                }
            } else if let Some(i) = self.sounding.iter().position(|k| *k == (e.note, e.ch)) {
                self.sounding.remove(i);
                out.push(*e);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(f: impl Fn(&mut [i32; 6])) -> Settings {
        let mut s = Settings::default();
        f(&mut s.v);
        s
    }

    #[test]
    fn defaults_pass_everything() {
        let d = Settings::default();
        for (n, v, c) in [(0, 1, 0), (60, 64, 5), (127, 127, 15)] {
            assert!(passes(&d, n, v, c));
        }
    }

    #[test]
    fn key_range_inclusive_and_swapped() {
        let r = s(|v| {
            v[idx::LOW] = 48;
            v[idx::HIGH] = 72;
        });
        assert!(passes(&r, 48, 100, 0) && passes(&r, 72, 100, 0));
        assert!(!passes(&r, 47, 100, 0) && !passes(&r, 73, 100, 0));
        let swapped = s(|v| {
            v[idx::LOW] = 72;
            v[idx::HIGH] = 48;
        });
        assert!(passes(&swapped, 60, 100, 0));
    }

    #[test]
    fn velocity_range_and_block_mode() {
        let r = s(|v| {
            v[idx::VEL_LOW] = 40;
            v[idx::VEL_HIGH] = 100;
        });
        assert!(passes(&r, 60, 40, 0) && passes(&r, 60, 100, 0));
        assert!(!passes(&r, 60, 39, 0) && !passes(&r, 60, 101, 0));
        let b = s(|v| {
            v[idx::VEL_LOW] = 40;
            v[idx::VEL_HIGH] = 100;
            v[idx::MODE] = 1;
        });
        assert!(!passes(&b, 60, 70, 0) && passes(&b, 60, 120, 0));
    }

    #[test]
    fn channel_is_one_based_and_zero_means_all() {
        let c = s(|v| v[idx::CHANNEL] = 3);
        assert!(passes(&c, 60, 100, 2));
        assert!(!passes(&c, 60, 100, 0));
        let block = s(|v| {
            v[idx::CHANNEL] = 3;
            v[idx::MODE] = 1;
        });
        assert!(!passes(&block, 60, 100, 2) && passes(&block, 60, 100, 1));
    }

    #[test]
    fn note_off_follows_its_on() {
        let mut f = NoteFilter::with_settings(48000, &s(|v| v[idx::HIGH] = 60));
        let out = f.process(&[Ev::on(0, 60, 90), Ev::on(0, 72, 90), Ev::off(100, 72), Ev::off(100, 60)], 256);
        assert_eq!(out, vec![Ev::on(0, 60, 90), Ev::off(100, 60)]);
        // settings change while a note is held: its off still goes through
        f.process(&[Ev::on(0, 50, 90)], 256);
        f.set(&s(|v| v[idx::HIGH] = 10));
        assert_eq!(f.process(&[Ev::off(0, 50)], 256), vec![Ev::off(0, 50)]);
    }
}
