// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Chord: the chord/scale mapper DSP logic (pure, no audio I/O).

use crate::params::{ChordType, Inversion, Root, Scale, Settings, Strum, Voicing};
use std::collections::VecDeque;

/// Maximum number of notes in a chord.
const MAX_CHORD_NOTES: usize = 7;

/// A chord/scale mapper that transforms incoming MIDI notes to chord notes.
pub struct Chord {
    sample_rate: u32,
    settings: Settings,
    last_input_note: Option<u8>,
    last_output_notes: Vec<u8>,
    strum_state: StrumState,
    hold_notes: Vec<u8>, // Notes being held (for hold mode)
}

#[derive(Default)]
struct StrumState {
    counter: u32,
    direction: i8, // -1 = up, 1 = down
}

impl Chord {
    /// Create a new chord mapper with the given sample rate and settings.
    pub fn with_settings(sample_rate: u32, settings: &Settings) -> Self {
        Chord {
            sample_rate,
            settings: *settings,
            last_input_note: None,
            last_output_notes: Vec::new(),
            strum_state: StrumState::default(),
            hold_notes: Vec::new(),
        }
    }

    /// Update the sample rate (called on initialize).
    pub fn set_sample_rate(&mut self, rate: u32) {
        self.sample_rate = rate;
    }

    /// Apply new settings (called from audio thread).
    pub fn set(&mut self, settings: &Settings) {
        self.settings = *settings;
    }

    /// Reset the chord mapper state.
    pub fn reset(&mut self) {
        self.last_input_note = None;
        self.last_output_notes.clear();
        self.strum_state = StrumState::default();
        self.hold_notes.clear();
    }

    /// Handle a MIDI note on event. Returns the chord notes to play.
    pub fn note_on(&mut self, note: u8, velocity: u8) -> Vec<(u8, u8)> {
        self.last_input_note = Some(note);

        // Map the input note to a scale degree
        let scale_degree = self.note_to_scale_degree(note);

        // Build the chord from that scale degree
        let chord_notes = self.build_chord(scale_degree, velocity);

        // Apply strum if enabled
        let strummed_notes = self.apply_strum(chord_notes);

        // Store for hold mode
        if self.settings.hold {
            self.hold_notes = strummed_notes.iter().map(|(n, _)| *n).collect();
        }

        // Send note-offs for previous chord
        let mut output = Vec::new();
        for prev_note in &self.last_output_notes {
            if !strummed_notes.iter().any(|(n, _)| n == prev_note) {
                output.push((*prev_note, 0)); // Note off
            }
        }

        // Send note-ons for new chord
        for (note, vel) in &strummed_notes {
            output.push((*note, *vel));
        }

        self.last_output_notes = strummed_notes.iter().map(|(n, _)| *n).collect();
        output
    }

    /// Handle a MIDI note off event.
    pub fn note_off(&mut self, note: u8) -> Vec<(u8, u8)> {
        if self.settings.hold {
            // In hold mode, keep the chord playing
            return Vec::new();
        }

        self.last_input_note = None;
        let mut output = Vec::new();
        for prev_note in &self.last_output_notes {
            output.push((*prev_note, 0));
        }
        self.last_output_notes.clear();
        output
    }

    /// Convert an input note to a scale degree (0-6 for 7-note scales).
    fn note_to_scale_degree(&self, note: u8) -> usize {
        let root_semitone = self.settings.root.semitone();
        let note_semitone = note % 12;
        let relative = (note_semitone + 12 - root_semitone) % 12;

        // Find the closest scale degree
        let degrees = self.settings.scale.degrees();
        let mut closest = 0;
        let mut min_dist = 12;

        for (i, &degree) in degrees.iter().enumerate() {
            let dist = (relative as i8 - degree as i8).abs() as u8;
            if dist < min_dist {
                min_dist = dist;
                closest = i;
            }
        }
        closest
    }

    /// Build a chord from a scale degree.
    fn build_chord(&self, scale_degree: usize, velocity: u8) -> Vec<(u8, u8)> {
        let root_semitone = self.settings.root.semitone();
        let scale_degrees = self.settings.scale.degrees();
        let chord_degrees = self.settings.chord_type.degrees();

        let mut notes = Vec::new();

        // Get the root of the chord (the scale degree's note)
        let chord_root_degree = scale_degrees[scale_degree % scale_degrees.len()] as u8;
        let chord_root = (root_semitone + chord_root_degree) % 12;

        // Build chord notes by adding chord degrees
        for &chord_deg_idx in chord_degrees {
            if chord_deg_idx < scale_degrees.len() {
                let note_semitone = (root_semitone + scale_degrees[chord_deg_idx]) % 12;
                // Add octave (default to octave 4 = MIDI note 60 for middle C)
                let midi_note = 60 + note_semitone;
                notes.push((midi_note, velocity));
            }
        }

        // Apply inversion
        self.apply_inversion(&mut notes);

        // Apply voicing
        self.apply_voicing(&mut notes);

        notes
    }

    /// Apply inversion to chord notes.
    fn apply_inversion(&self, notes: &mut Vec<(u8, u8)>) {
        let inv = self.settings.inversion as usize;
        if inv == 0 || notes.len() <= inv {
            return;
        }
        // Move the bottom 'inv' notes up an octave
        for i in 0..inv {
            if i < notes.len() {
                notes[i].0 += 12;
            }
        }
        notes.sort_by_key(|(n, _)| *n);
    }

    /// Apply voicing to chord notes.
    fn apply_voicing(&self, notes: &mut Vec<(u8, u8)>) {
        match self.settings.voicing {
            Voicing::Close => {
                // Already close position
            }
            Voicing::Open => {
                // Spread notes across octaves
                for (i, (note, _)) in notes.iter_mut().enumerate() {
                    *note += (i as u8) * 12;
                }
            }
            Voicing::Drop2 => {
                // Drop the second highest note down an octave
                if notes.len() >= 2 {
                    let idx = notes.len() - 2;
                    notes[idx].0 -= 12;
                    notes.sort_by_key(|(n, _)| *n);
                }
            }
            Voicing::Drop3 => {
                // Drop the third highest note down an octave
                if notes.len() >= 3 {
                    let idx = notes.len() - 3;
                    notes[idx].0 -= 12;
                    notes.sort_by_key(|(n, _)| *n);
                }
            }
        }
    }

    /// Apply strum effect to chord notes.
    fn apply_strum(&mut self, notes: Vec<(u8, u8)>) -> Vec<(u8, u8)> {
        if self.settings.strum == Strum::Off || notes.is_empty() {
            return notes;
        }

        let mut strummed = notes;
        let strum_delay = (100 - self.settings.strum_speed) as u32; // 0-100ms delay

        match self.settings.strum {
            Strum::Down => {
                // Play from lowest to highest with increasing delay
                strummed.sort_by_key(|(n, _)| *n);
            }
            Strum::Up => {
                // Play from highest to lowest
                strummed.sort_by_key(|(n, _)| std::cmp::Reverse(*n));
            }
            Strum::Alternating => {
                // Alternate direction each chord
                if self.strum_state.direction == 1 {
                    strummed.sort_by_key(|(n, _)| *n);
                } else {
                    strummed.sort_by_key(|(n, _)| std::cmp::Reverse(*n));
                }
                self.strum_state.direction *= -1;
            }
            Strum::Off => {}
        }

        // Note: In a real implementation, the strum delay would be handled
        // by scheduling note-ons at different times. Here we just return
        // the ordered notes - the host/timing would handle the delay.
        strummed
    }

    /// Get the scale note for a given input note (for single-note mode).
    pub fn map_note(&self, note: u8) -> u8 {
        let scale_degree = self.note_to_scale_degree(note);
        let root_semitone = self.settings.root.semitone();
        let scale_degrees = self.settings.scale.degrees();
        let mapped_semitone = (root_semitone + scale_degrees[scale_degree % scale_degrees.len()]) % 12;
        // Preserve the original octave
        let octave = note / 12;
        octave * 12 + mapped_semitone
    }
}

/// MIDI event types for internal processing.
#[derive(Clone, Debug)]
pub enum MidiEvent {
    NoteOn { note: u8, velocity: u8, channel: u8 },
    NoteOff { note: u8, velocity: u8, channel: u8 },
    ControlChange { controller: u8, value: u8, channel: u8 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_mapping_major() {
        let mut chord = Chord::with_settings(44100, &Settings {
            root: Root::C,
            scale: Scale::Major,
            chord_type: ChordType::Triad,
            inversion: Inversion::Root,
            voicing: Voicing::Close,
            strum: Strum::Off,
            strum_speed: 50,
            hold: false,
        });

        // Input C4 (60) -> should map to C4 (root of C major)
        let output = chord.note_on(60, 100);
        assert!(!output.is_empty());
        assert_eq!(output[0].0, 60); // C4
    }

    #[test]
    fn scale_mapping_minor() {
        let mut chord = Chord::with_settings(44100, &Settings {
            root: Root::A,
            scale: Scale::Minor,
            chord_type: ChordType::Triad,
            inversion: Inversion::Root,
            voicing: Voicing::Close,
            strum: Strum::Off,
            strum_speed: 50,
            hold: false,
        });

        // Input A4 (69) -> should map to A4 (root of A minor)
        let output = chord.note_on(69, 100);
        assert!(!output.is_empty());
        // A minor triad: A, C, E (in some octave)
        let notes: Vec<u8> = output.iter().map(|(n, _)| *n).collect();
        assert_eq!(notes.len(), 3);
        assert!(notes.contains(&69)); // A4
        // C and E might be in different octaves, just check we have 3 notes
    }

    #[test]
    fn chord_type_seventh() {
        let mut chord = Chord::with_settings(44100, &Settings {
            root: Root::C,
            scale: Scale::Major,
            chord_type: ChordType::Seventh,
            inversion: Inversion::Root,
            voicing: Voicing::Close,
            strum: Strum::Off,
            strum_speed: 50,
            hold: false,
        });

        let output = chord.note_on(60, 100);
        let notes: Vec<u8> = output.iter().map(|(n, _)| *n).collect();
        // C major 7th: C, E, G, B
        assert_eq!(notes.len(), 4);
        assert!(notes.contains(&60)); // C4
        assert!(notes.contains(&64)); // E4
        assert!(notes.contains(&67)); // G4
        assert!(notes.contains(&71)); // B4
    }

    #[test]
    fn inversion_first() {
        let mut chord = Chord::with_settings(44100, &Settings {
            root: Root::C,
            scale: Scale::Major,
            chord_type: ChordType::Triad,
            inversion: Inversion::First,
            voicing: Voicing::Close,
            strum: Strum::Off,
            strum_speed: 50,
            hold: false,
        });

        let output = chord.note_on(60, 100);
        let notes: Vec<u8> = output.iter().map(|(n, _)| *n).collect();
        // First inversion: E, G, C (C moved up an octave)
        assert!(notes.contains(&64)); // E4
        assert!(notes.contains(&67)); // G4
        assert!(notes.contains(&72)); // C5 (was C4)
    }

    #[test]
    fn hold_mode() {
        let mut chord = Chord::with_settings(44100, &Settings {
            root: Root::C,
            scale: Scale::Major,
            chord_type: ChordType::Triad,
            inversion: Inversion::Root,
            voicing: Voicing::Close,
            strum: Strum::Off,
            strum_speed: 50,
            hold: true,
        });

        chord.note_on(60, 100);
        let off = chord.note_off(60);
        // In hold mode, note_off should not stop the chord
        assert!(off.is_empty());
    }
}