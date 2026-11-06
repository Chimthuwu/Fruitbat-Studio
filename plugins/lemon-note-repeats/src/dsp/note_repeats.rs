// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Note Repeats: retriggers held notes at a rate division, optionally tapering velocity (pure, no audio I/O).

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

/// A repeat's step in samples (at least 1).
pub fn step_samples(rate: i32, bpm: f64, sr: u32) -> u64 {
    ((division_beats(rate) * samples_per_beat(bpm, sr)).round() as u64).max(1)
}

/// How long each repeat sounds, `gate` percent of the step (at least 1 sample).
pub fn gate_samples(step: u64, gate: i32) -> u64 {
    (step * gate.clamp(1, 100) as u64 / 100).max(1)
}

/// Velocity of repeat `n` (0 = the played note) when every repeat loses `taper` percent.
pub fn taper_velocity(vel: u8, n: u32, taper: i32) -> u8 {
    let f = (1.0 - taper.clamp(0, 100) as f64 / 100.0).powi(n as i32);
    ((vel as f64 * f).round() as i32).clamp(1, 127) as u8
}

struct Held {
    note: u8,
    vel: u8,
    ch: u8,
    next: u64,
    count: u32,
}

pub struct NoteRepeats {
    sr: u32,
    s: Settings,
    bpm: f64,
    now: u64,
    held: Vec<Held>,
    pending: Pending,
}

impl NoteRepeats {
    pub fn with_settings(sr: u32, s: &Settings) -> Self {
        NoteRepeats { sr, s: *s, bpm: 120.0, now: 0, held: Vec::new(), pending: Pending::default() }
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
        self.held.clear();
        self.pending.clear();
        self.now = 0;
    }

    fn step(&self) -> u64 {
        step_samples(self.s.v[idx::RATE], self.bpm, self.sr)
    }

    fn sound(&mut self, at: u64, note: u8, vel: u8, ch: u8) {
        let gate = gate_samples(self.step(), self.s.v[idx::GATE]);
        self.pending.push(at, Ev { time: 0, on: true, note, vel, ch });
        self.pending.push(at + gate, Ev { time: 0, on: false, note, vel: 0, ch });
    }

    /// Fires every repeat due before `t`, in time order.
    fn fire_until(&mut self, t: u64) {
        loop {
            let Some(i) = (0..self.held.len()).filter(|&i| self.held[i].next < t).min_by_key(|&i| self.held[i].next) else { break };
            let step = self.step();
            let h = &mut self.held[i];
            h.count += 1;
            let (at, note, ch) = (h.next, h.note, h.ch);
            let vel = taper_velocity(h.vel, h.count, self.s.v[idx::TAPER]);
            h.next += step;
            self.sound(at, note, vel, ch);
        }
    }

    /// One block: held notes in, retriggers out.
    pub fn process(&mut self, input: &[Ev], block: u32) -> Vec<Ev> {
        let mut evs = input.to_vec();
        evs.sort_by_key(|e| e.time);
        for e in evs {
            let t = self.now + e.time as u64;
            self.fire_until(t);
            if e.on {
                self.held.retain(|h| !(h.note == e.note && h.ch == e.ch));
                self.held.push(Held { note: e.note, vel: e.vel, ch: e.ch, next: t + self.step(), count: 0 });
                self.sound(t, e.note, e.vel, e.ch);
            } else if let Some(i) = self.held.iter().position(|h| h.note == e.note && h.ch == e.ch) {
                self.held.remove(i);
                // Cut the repeat still sounding.
                let gone = self.pending.cancel(|a, p| !p.on && p.note == e.note && p.ch == e.ch && a >= t);
                if gone > 0 {
                    self.pending.push(t, Ev { time: 0, on: false, note: e.note, vel: 0, ch: e.ch });
                }
            }
        }
        self.fire_until(self.now + block as u64);
        let out = self.pending.drain(self.now, block);
        self.now += block as u64;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eighth(taper: i32, gate: i32) -> Settings {
        let mut s = Settings::default();
        s.v[idx::RATE] = 9;
        s.v[idx::TAPER] = taper;
        s.v[idx::GATE] = gate;
        s
    }

    /// Runs `blocks` blocks of 4800 samples; (absolute time, on, note, vel).
    fn run(d: &mut NoteRepeats, first: &[Ev], blocks: u64) -> Vec<(u64, bool, u8, u8)> {
        let mut out = Vec::new();
        for b in 0..blocks {
            let input: &[Ev] = if b == 0 { first } else { &[] };
            for e in d.process(input, 4800) {
                out.push((b * 4800 + e.time as u64, e.on, e.note, e.vel));
            }
        }
        out
    }

    #[test]
    fn step_and_gate_math() {
        assert_eq!(step_samples(9, 120.0, 48000), 12000);
        assert_eq!(step_samples(10, 120.0, 48000), 18000);
        assert_eq!(step_samples(11, 120.0, 48000), 8000);
        assert_eq!(step_samples(0, 120.0, 48000), 96000);
        assert_eq!(gate_samples(12000, 50), 6000);
        assert_eq!(gate_samples(12000, 100), 12000);
        assert_eq!(gate_samples(1, 1), 1);
    }

    #[test]
    fn taper_math() {
        let v: Vec<u8> = (0..4).map(|n| taper_velocity(100, n, 20)).collect();
        assert_eq!(v, vec![100, 80, 64, 51]);
        assert_eq!(taper_velocity(100, 5, 0), 100);
        assert_eq!(taper_velocity(100, 30, 50), 1);
    }

    #[test]
    fn held_note_retriggers_until_released() {
        let mut d = NoteRepeats::with_settings(48000, &eighth(0, 50));
        let out = run(&mut d, &[Ev::on(0, 60, 90)], 6);
        let ons: Vec<u64> = out.iter().filter(|e| e.1).map(|e| e.0).collect();
        assert_eq!(ons, vec![0, 12000, 24000]);
        let offs: Vec<u64> = out.iter().filter(|e| !e.1).map(|e| e.0).collect();
        assert_eq!(offs, vec![6000, 18000]);
        // 28800 samples have run; a release at 30000 cuts the repeat sounding and stops the 36000 one
        let out = d.process(&[Ev::off(1200, 60)], 4800);
        assert_eq!(out.iter().map(|e| (e.time, e.on)).collect::<Vec<_>>(), vec![(1200, false)]);
        let out = d.process(&[], 48000);
        assert!(out.iter().all(|e| !e.on), "no repeats after release");
    }

    #[test]
    fn velocity_tapers_per_repeat() {
        let mut d = NoteRepeats::with_settings(48000, &eighth(20, 50));
        let out = run(&mut d, &[Ev::on(0, 60, 100)], 6);
        let vels: Vec<u8> = out.iter().filter(|e| e.1).map(|e| e.3).collect();
        assert_eq!(vels, vec![100, 80, 64]);
    }

    #[test]
    fn two_notes_repeat_independently() {
        let mut d = NoteRepeats::with_settings(48000, &eighth(0, 50));
        let out = run(&mut d, &[Ev::on(0, 60, 90), Ev::on(3000, 64, 90)], 4);
        let ons: Vec<(u64, u8)> = out.iter().filter(|e| e.1).map(|e| (e.0, e.2)).collect();
        assert_eq!(ons, vec![(0, 60), (3000, 64), (12000, 60), (15000, 64)]);
    }
}
