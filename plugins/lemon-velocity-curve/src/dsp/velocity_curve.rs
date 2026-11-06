// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Velocity Curve: remaps note velocity through a curve (pure, no audio I/O).

use crate::params::{Curve, Settings};

/// MIDI event types for internal processing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiEvent {
    NoteOn { note: u8, velocity: u8, channel: u8 },
    NoteOff { note: u8, velocity: u8, channel: u8 },
    ControlChange { controller: u8, value: u8, channel: u8 },
}

pub struct VelocityCurve {
    settings: Settings,
}

/// The curve's value for `x` in 0..=1 (Exp bends down: soft notes get softer; Log bends up).
pub fn curve(c: Curve, x: f32) -> f32 {
    match c {
        Curve::Linear => x,
        Curve::Exp => x * x,
        Curve::Log => x.sqrt(),
    }
}

/// `velocity` remapped: the curve blended in by `amount` percent (0 = untouched, 100 = fully curved). A note-on
/// stays a note-on (1-127); velocity 0 stays 0.
pub fn remap(velocity: u8, c: Curve, amount: u32) -> u8 {
    if velocity == 0 {
        return 0;
    }
    let x = velocity.min(127) as f32 / 127.0;
    let a = amount.min(100) as f32 / 100.0;
    let y = x + (curve(c, x) - x) * a;
    (y * 127.0).round().clamp(1.0, 127.0) as u8
}

impl VelocityCurve {
    pub fn with_settings(_sample_rate: u32, settings: &Settings) -> Self {
        VelocityCurve { settings: *settings }
    }

    pub fn set_sample_rate(&mut self, _rate: u32) {}

    pub fn set(&mut self, settings: &Settings) {
        self.settings = *settings;
    }

    pub fn reset(&mut self) {}

    /// New velocity for a note-on.
    pub fn map(&self, velocity: u8) -> u8 {
        remap(velocity, self.settings.curve, self.settings.amount)
    }

    /// The events with note-on velocities remapped (everything else untouched).
    pub fn process_midi(&mut self, input: &[MidiEvent]) -> Vec<MidiEvent> {
        input
            .iter()
            .map(|e| match *e {
                MidiEvent::NoteOn { note, velocity, channel } => MidiEvent::NoteOn { note, velocity: self.map(velocity), channel },
                e => e,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_is_identity_at_any_amount() {
        for a in [0, 50, 100] {
            for v in 1..=127 {
                assert_eq!(remap(v, Curve::Linear, a), v);
            }
        }
    }

    #[test]
    fn zero_amount_is_identity() {
        for c in [Curve::Exp, Curve::Log] {
            for v in 1..=127 {
                assert_eq!(remap(v, c, 0), v);
            }
        }
    }

    #[test]
    fn exp_pulls_down_and_log_pushes_up() {
        assert_eq!(remap(64, Curve::Exp, 100), 32); // 0.504^2 * 127
        assert!(remap(64, Curve::Log, 100) > 64);
        assert_eq!(remap(64, Curve::Log, 100), 90); // sqrt(0.504) * 127
        for v in 2..120 {
            assert!(remap(v, Curve::Exp, 100) < v && remap(v, Curve::Log, 100) > v, "{v}");
        }
    }

    #[test]
    fn amount_blends_halfway() {
        // x = 100/127; exp -> 0.620; halfway between 0.787 and 0.620.
        assert_eq!(remap(100, Curve::Exp, 50), 89);
    }

    #[test]
    fn endpoints_hold() {
        for c in [Curve::Linear, Curve::Exp, Curve::Log] {
            assert_eq!(remap(127, c, 100), 127);
            assert_eq!(remap(0, c, 100), 0);
        }
        assert_eq!(remap(1, Curve::Exp, 100), 1);
    }

    #[test]
    fn monotonic() {
        for c in [Curve::Exp, Curve::Log] {
            for a in [25, 100] {
                assert!((1..127).all(|v| remap(v, c, a) <= remap(v + 1, c, a)));
            }
        }
    }

    #[test]
    fn midi_stream_only_touches_note_on_velocity() {
        let mut vc = VelocityCurve::with_settings(44100, &Settings { curve: Curve::Exp, amount: 100 });
        let off = MidiEvent::NoteOff { note: 60, velocity: 90, channel: 2 };
        let cc = MidiEvent::ControlChange { controller: 7, value: 99, channel: 0 };
        let out = vc.process_midi(&[MidiEvent::NoteOn { note: 60, velocity: 64, channel: 2 }, off, cc]);
        assert_eq!(out, vec![MidiEvent::NoteOn { note: 60, velocity: 32, channel: 2 }, off, cc]);
    }
}
