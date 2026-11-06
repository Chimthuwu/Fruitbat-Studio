// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Key Filter: drops (or passes) the notes that are not in a scale (pure, no audio I/O).

use crate::params::{Mode, Scale, Settings};

/// MIDI event types for internal processing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiEvent {
    NoteOn { note: u8, velocity: u8, channel: u8 },
    NoteOff { note: u8, velocity: u8, channel: u8 },
    ControlChange { controller: u8, value: u8, channel: u8 },
}

/// A note filter: root + scale, and what happens to the other notes.
pub struct KeyFilter {
    settings: Settings,
    /// Per channel, the notes whose note-on was dropped (their note-off is dropped too).
    dropped: [u128; 16],
}

/// Whether `note` is in `scale` rooted at pitch class `root` (0 = C).
pub fn in_scale(note: u8, root: u8, scale: Scale) -> bool {
    let pc = (note % 12 + 12 - root % 12) % 12;
    scale.intervals().contains(&pc)
}

impl KeyFilter {
    pub fn with_settings(_sample_rate: u32, settings: &Settings) -> Self {
        KeyFilter { settings: *settings, dropped: [0; 16] }
    }

    pub fn set_sample_rate(&mut self, _rate: u32) {}

    pub fn set(&mut self, settings: &Settings) {
        self.settings = *settings;
    }

    pub fn reset(&mut self) {
        self.dropped = [0; 16];
    }

    /// Whether a note-on of `note` plays under the current settings.
    pub fn allows(&self, note: u8) -> bool {
        self.settings.mode == Mode::Pass || in_scale(note, self.settings.root, self.settings.scale)
    }

    /// The events that come through (order kept). A dropped note-on takes its note-off with it.
    pub fn process_midi(&mut self, input: &[MidiEvent]) -> Vec<MidiEvent> {
        let mut out = Vec::with_capacity(input.len());
        for e in input {
            match *e {
                MidiEvent::NoteOn { note, channel, .. } => {
                    let bit = 1u128 << (note & 127);
                    let ch = (channel & 15) as usize;
                    if self.allows(note) {
                        self.dropped[ch] &= !bit;
                        out.push(*e);
                    } else {
                        self.dropped[ch] |= bit;
                    }
                }
                MidiEvent::NoteOff { note, channel, .. } => {
                    let bit = 1u128 << (note & 127);
                    let ch = (channel & 15) as usize;
                    if self.dropped[ch] & bit != 0 {
                        self.dropped[ch] &= !bit;
                    } else {
                        out.push(*e);
                    }
                }
                _ => out.push(*e),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kf(root: u8, scale: Scale, mode: Mode) -> KeyFilter {
        KeyFilter::with_settings(44100, &Settings { root, scale, mode })
    }

    fn on(note: u8) -> MidiEvent {
        MidiEvent::NoteOn { note, velocity: 100, channel: 0 }
    }
    fn off(note: u8) -> MidiEvent {
        MidiEvent::NoteOff { note, velocity: 0, channel: 0 }
    }

    #[test]
    fn c_major_keeps_the_white_keys() {
        let k = kf(0, Scale::Major, Mode::Drop);
        let white: Vec<u8> = (60..72).filter(|n| k.allows(*n)).collect();
        assert_eq!(white, vec![60, 62, 64, 65, 67, 69, 71]);
    }

    #[test]
    fn root_transposes_the_scale() {
        // A minor = the same notes as C major.
        let k = kf(9, Scale::Minor, Mode::Drop);
        assert_eq!((60..72).filter(|n| k.allows(*n)).collect::<Vec<_>>(), vec![60, 62, 64, 65, 67, 69, 71]);
        // D major pentatonic: D E F# A B.
        let k = kf(2, Scale::PentatonicMajor, Mode::Drop);
        assert_eq!((60..72).filter(|n| k.allows(*n)).collect::<Vec<_>>(), vec![62, 64, 66, 69, 71]);
    }

    #[test]
    fn chromatic_keeps_everything() {
        let k = kf(5, Scale::Chromatic, Mode::Drop);
        assert!((0..128).all(|n| k.allows(n)));
    }

    #[test]
    fn drop_mode_drops_the_note_and_its_off() {
        let mut k = kf(0, Scale::Major, Mode::Drop);
        let out = k.process_midi(&[on(61), on(60), off(61), off(60)]);
        assert_eq!(out, vec![on(60), off(60)]);
        // A later note-off for a kept note is not swallowed by the earlier drop.
        assert_eq!(k.process_midi(&[off(61)]), vec![off(61)]);
    }

    #[test]
    fn pass_mode_passes_everything() {
        let mut k = kf(0, Scale::Major, Mode::Pass);
        let ev = [on(61), off(61), on(60)];
        assert_eq!(k.process_midi(&ev), ev.to_vec());
    }

    #[test]
    fn other_messages_pass() {
        let mut k = kf(0, Scale::Major, Mode::Drop);
        let cc = MidiEvent::ControlChange { controller: 1, value: 5, channel: 0 };
        assert_eq!(k.process_midi(&[cc]), vec![cc]);
    }

    #[test]
    fn every_scale_has_its_root() {
        for i in 0..14 {
            let s = Scale::from_index(i);
            assert!(s.intervals().contains(&0));
            assert!(s.intervals().windows(2).all(|w| w[0] < w[1]));
        }
    }
}
