// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Humanize: randomises a note's timing, velocity and length (pure, no audio I/O).
//!
//! Every random number comes from a hash of (seed, note, channel, how many times that voice has played), so the same
//! input always gives the same output and each voice has its own stream.

use crate::params::Settings;

/// A note to humanise: velocity 1-127, times in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Note {
    pub note: u8,
    pub channel: u8,
    pub velocity: u8,
    pub start_ms: f32,
    pub length_ms: f32,
}

pub struct Humanize {
    settings: Settings,
    /// Notes played so far per (channel, note): each voice's position in its random stream.
    counts: Vec<u32>,
}

/// splitmix64 finaliser.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A value in [-1, 1] for `(seed, voice, n, dim)`.
pub fn unit(seed: u32, voice: u16, n: u32, dim: u8) -> f32 {
    let h = mix(((seed as u64) << 32 | n as u64) ^ mix((voice as u64) << 8 | dim as u64));
    ((h >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
}

impl Humanize {
    pub fn with_settings(_sample_rate: u32, settings: &Settings) -> Self {
        Humanize { settings: *settings, counts: vec![0; 16 * 128] }
    }

    pub fn set_sample_rate(&mut self, _rate: u32) {}

    pub fn set(&mut self, settings: &Settings) {
        self.settings = *settings;
    }

    /// Back to the start of every voice's stream.
    pub fn reset(&mut self) {
        self.counts.iter_mut().for_each(|c| *c = 0);
    }

    /// The humanised `n`.
    pub fn process_note(&mut self, n: Note) -> Note {
        let s = &self.settings;
        let voice = ((n.channel & 15) as u16) << 7 | (n.note & 127) as u16;
        let c = &mut self.counts[voice as usize];
        let i = *c;
        *c = c.wrapping_add(1);
        let r = |dim| unit(s.seed, voice, i, dim);
        let start_ms = (n.start_ms + r(0) * s.timing as f32).max(0.0);
        let velocity = (n.velocity as f32 + r(1) * s.velocity as f32).round().clamp(1.0, 127.0) as u8;
        let length_ms = (n.length_ms * (1.0 + r(2) * s.length as f32 / 100.0)).max(1.0);
        Note { start_ms, velocity, length_ms, ..n }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(timing: i32, velocity: i32, length: i32) -> Settings {
        Settings::from_vals(&[timing, velocity, length, 7])
    }

    fn note(n: u8) -> Note {
        Note { note: n, channel: 0, velocity: 64, start_ms: 500.0, length_ms: 200.0 }
    }

    #[test]
    fn zero_amounts_change_nothing() {
        let mut h = Humanize::with_settings(44100, &settings(0, 0, 0));
        for _ in 0..20 {
            assert_eq!(h.process_note(note(60)), note(60));
        }
    }

    #[test]
    fn deterministic_for_a_seed() {
        let run = || {
            let mut h = Humanize::with_settings(44100, &settings(30, 20, 50));
            (0..10).map(|i| h.process_note(note(60 + i))).collect::<Vec<_>>()
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn reset_replays_the_stream() {
        let mut h = Humanize::with_settings(44100, &settings(30, 20, 50));
        let a: Vec<_> = (0..5).map(|_| h.process_note(note(60))).collect();
        h.reset();
        let b: Vec<_> = (0..5).map(|_| h.process_note(note(60))).collect();
        assert_eq!(a, b);
        assert!(a.windows(2).any(|w| w[0] != w[1]), "repeats of one voice differ");
    }

    #[test]
    fn voices_and_seeds_differ() {
        let mut h = Humanize::with_settings(44100, &settings(30, 20, 50));
        assert_ne!(h.process_note(note(60)), h.process_note(note(61)).clone_with(60));
        let mut g = Humanize::with_settings(44100, &Settings::from_vals(&[30, 20, 50, 8]));
        let mut h = Humanize::with_settings(44100, &settings(30, 20, 50));
        assert_ne!(g.process_note(note(60)), h.process_note(note(60)));
    }

    impl Note {
        fn clone_with(mut self, note: u8) -> Note {
            self.note = note;
            self
        }
    }

    #[test]
    fn results_stay_inside_the_amounts() {
        let mut h = Humanize::with_settings(44100, &settings(25, 10, 40));
        for i in 0..2000 {
            let o = h.process_note(Note { note: (i % 128) as u8, channel: (i % 16) as u8, ..note(0) });
            assert!((o.start_ms - 500.0).abs() <= 25.0 + 1e-3, "{o:?}");
            assert!((o.velocity as i32 - 64).abs() <= 10, "{o:?}");
            assert!((120.0 - 1e-3..=280.0 + 1e-3).contains(&o.length_ms), "{o:?}");
        }
    }

    #[test]
    fn spread_covers_both_directions() {
        let mut h = Humanize::with_settings(44100, &settings(25, 10, 40));
        let v: Vec<_> = (0..500).map(|_| h.process_note(note(60))).collect();
        assert!(v.iter().any(|o| o.start_ms < 490.0) && v.iter().any(|o| o.start_ms > 510.0));
        assert!(v.iter().any(|o| o.velocity < 60) && v.iter().any(|o| o.velocity > 68));
    }

    #[test]
    fn start_never_goes_negative_and_velocity_stays_midi() {
        let mut h = Humanize::with_settings(44100, &settings(100, 64, 100));
        for _ in 0..500 {
            let o = h.process_note(Note { velocity: 1, start_ms: 0.0, ..note(60) });
            assert!(o.start_ms >= 0.0 && (1..=127).contains(&o.velocity) && o.length_ms >= 1.0);
        }
    }
}
