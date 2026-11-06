// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Transpose: the note transposer's math and per-note bookkeeping (pure, no audio I/O).

use nih_plug::prelude::NoteEvent;

use crate::params::Settings;

/// The scales' pitch classes above the root (the order of the Scale setting).
pub const SCALES: [&[u8]; 9] = [
    &[0, 2, 4, 5, 7, 9, 11], // Major
    &[0, 2, 3, 5, 7, 8, 10], // Minor
    &[0, 2, 3, 5, 7, 9, 10], // Dorian
    &[0, 1, 3, 5, 7, 8, 10], // Phrygian
    &[0, 2, 4, 6, 7, 9, 11], // Lydian
    &[0, 2, 4, 5, 7, 9, 10], // Mixolydian
    &[0, 1, 3, 5, 6, 8, 10], // Locrian
    &[0, 2, 3, 5, 7, 8, 11], // Harmonic Minor
    &[0, 2, 3, 5, 7, 9, 11], // Melodic Minor
];

/// Is `note` in the scale (`scale` index, root pitch class `root`)?
pub fn in_scale(note: i32, root: u8, scale: u8) -> bool {
    let degrees = SCALES[(scale as usize).min(SCALES.len() - 1)];
    let pc = (note - root as i32).rem_euclid(12) as u8;
    degrees.contains(&pc)
}

/// The nearest note in the scale to `note` (itself when in it; a tie goes down). `None` outside 0..=127.
pub fn snap_to_scale(note: i32, root: u8, scale: u8) -> Option<u8> {
    for d in 0..=6 {
        for cand in [note - d, note + d] {
            if (0..=127).contains(&cand) && in_scale(cand, root, scale) {
                return Some(cand as u8);
            }
        }
    }
    None
}

/// `note` shifted by `shift` semitones; with `scale_aware` the result is snapped into the scale. `None` when the
/// result leaves 0..=127 (the note is dropped).
pub fn transpose_note(note: u8, shift: i32, scale_aware: bool, root: u8, scale: u8) -> Option<u8> {
    let target = note as i32 + shift;
    if !(0..=127).contains(&target) {
        return None;
    }
    if scale_aware {
        snap_to_scale(target, root, scale)
    } else {
        Some(target as u8)
    }
}

/// The transposer: remembers which note each sounding input note became, so its note off (and poly pressure) hits
/// the same note even if the settings moved meanwhile.
pub struct Transpose {
    /// Per channel and input note: the output note (+1; 0 = none).
    active: Box<[[u8; 128]; 16]>,
}

impl Default for Transpose {
    fn default() -> Self {
        Transpose { active: Box::new([[0; 128]; 16]) }
    }
}

impl Transpose {
    pub fn reset(&mut self) {
        for ch in self.active.iter_mut() {
            ch.fill(0);
        }
    }

    /// One event through the transposer: `None` when it is dropped.
    pub fn process(&mut self, s: &Settings, ev: NoteEvent<()>) -> Option<NoteEvent<()>> {
        match ev {
            NoteEvent::NoteOn { timing, voice_id, channel, note, velocity } => {
                let out = transpose_note(note, s.shift(), s.scale_aware, s.root, s.scale)?;
                self.active[(channel & 15) as usize][note as usize & 127] = out + 1;
                Some(NoteEvent::NoteOn { timing, voice_id, channel, note: out, velocity })
            }
            NoteEvent::NoteOff { timing, voice_id, channel, note, velocity } => {
                let slot = &mut self.active[(channel & 15) as usize][note as usize & 127];
                let out = slot.checked_sub(1)?;
                *slot = 0;
                Some(NoteEvent::NoteOff { timing, voice_id, channel, note: out, velocity })
            }
            NoteEvent::PolyPressure { timing, voice_id, channel, note, pressure } => {
                let out = self.active[(channel & 15) as usize][note as usize & 127].checked_sub(1)?;
                Some(NoteEvent::PolyPressure { timing, voice_id, channel, note: out, pressure })
            }
            other => Some(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(semitones: i32, octaves: i32, scale_aware: bool, root: u8, scale: u8) -> Settings {
        Settings { semitones, octaves, scale_aware, root, scale }
    }

    #[test]
    fn plain_shift() {
        assert_eq!(transpose_note(60, 7, false, 0, 0), Some(67));
        assert_eq!(transpose_note(60, -24, false, 0, 0), Some(36));
        assert_eq!(transpose_note(120, 12, false, 0, 0), None);
        assert_eq!(transpose_note(5, -12, false, 0, 0), None);
    }

    #[test]
    fn scale_membership() {
        // C major: no black keys.
        for n in [60, 62, 64, 65, 67, 69, 71] {
            assert!(in_scale(n, 0, 0), "{n}");
        }
        for n in [61, 63, 66, 68, 70] {
            assert!(!in_scale(n, 0, 0), "{n}");
        }
        // A minor has the same notes as C major.
        assert!(in_scale(69, 9, 1) && in_scale(64, 9, 1) && !in_scale(66, 9, 1));
        // The root moves the scale: D major has F#.
        assert!(in_scale(66, 2, 0) && !in_scale(65, 2, 0));
    }

    #[test]
    fn snapping_goes_to_the_nearest_and_ties_down() {
        // C# between C and D (tie): down.
        assert_eq!(snap_to_scale(61, 0, 0), Some(60));
        // F# between F and G (tie): down.
        assert_eq!(snap_to_scale(66, 0, 0), Some(65));
        // In-scale notes stay.
        assert_eq!(snap_to_scale(62, 0, 0), Some(62));
        // Harmonic minor in C has Eb and B: Bb (70) is 1 from B(71)? 70-0 is out, 69 is out (A natural is not in C hm), 71 in.
        assert_eq!(snap_to_scale(70, 0, 7), Some(71));
    }

    #[test]
    fn scale_aware_results_always_in_scale() {
        for scale in 0..9u8 {
            for root in 0..12u8 {
                for shift in -24..=24 {
                    for note in 24..100u8 {
                        let out = transpose_note(note, shift, true, root, scale).unwrap();
                        assert!(in_scale(out as i32, root, scale), "note {note} shift {shift} root {root} scale {scale}");
                        assert!((out as i32 - (note as i32 + shift)).abs() <= 2, "(scales have gaps of up to 3 semitones) {note} {shift} {root} {scale} -> {out}");
                    }
                }
            }
        }
    }

    fn on(note: u8) -> NoteEvent<()> {
        NoteEvent::NoteOn { timing: 3, voice_id: None, channel: 0, note, velocity: 0.8 }
    }
    fn off(note: u8) -> NoteEvent<()> {
        NoteEvent::NoteOff { timing: 9, voice_id: None, channel: 0, note, velocity: 0.0 }
    }

    #[test]
    fn note_off_follows_its_note_on_after_a_setting_change() {
        let mut t = Transpose::default();
        let Some(NoteEvent::NoteOn { note, timing, .. }) = t.process(&s(5, 0, false, 0, 0), on(60)) else { panic!() };
        assert_eq!((note, timing), (65, 3));
        // The shift changes while the note is held: the off still ends 65.
        let Some(NoteEvent::NoteOff { note, .. }) = t.process(&s(-12, 0, false, 0, 0), off(60)) else { panic!() };
        assert_eq!(note, 65);
        // A stray off with no note on is dropped.
        assert!(t.process(&s(0, 0, false, 0, 0), off(60)).is_none());
    }

    #[test]
    fn dropped_notes_drop_their_off_and_other_events_pass() {
        let mut t = Transpose::default();
        assert!(t.process(&s(0, 4, false, 0, 0), on(100)).is_none());
        assert!(t.process(&s(0, 4, false, 0, 0), off(100)).is_none());
        let cc = NoteEvent::<()>::MidiCC { timing: 0, channel: 0, cc: 1, value: 0.5 };
        assert_eq!(t.process(&s(0, 0, false, 0, 0), cc), Some(cc));
    }
}
