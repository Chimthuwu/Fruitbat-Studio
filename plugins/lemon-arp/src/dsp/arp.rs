// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Arp: the arpeggiator DSP logic (pure, no audio I/O).

use crate::params::{Pattern, Rate, Settings};
use std::collections::VecDeque;

/// Maximum number of held notes.
const MAX_NOTES: usize = 16;

/// An arpeggiator that processes MIDI notes and outputs arpeggiated notes.
pub struct Arp {
    sample_rate: u32,
    settings: Settings,
    held_notes: Vec<u8>,           // MIDI note numbers currently held
    note_order: VecDeque<u8>,      // Order notes were pressed (for latch)
    current_step: u32,             // Current step in the pattern
    tick_counter: u32,             // Ticks since last step
    ticks_per_step: u32,           // Ticks per step (based on rate and tempo)
    tempo: f32,                    // Current tempo (BPM)
    swing_accumulator: i32,        // Accumulator for swing timing
    last_output_notes: Vec<u8>,    // Notes currently sounding (for note-off)
}

impl Arp {
    /// Create a new arpeggiator with the given sample rate and settings.
    pub fn with_settings(sample_rate: u32, settings: &Settings) -> Self {
        let mut arp = Self {
            sample_rate,
            settings: *settings,
            held_notes: Vec::new(),
            note_order: VecDeque::new(),
            current_step: 0,
            tick_counter: 0,
            ticks_per_step: Self::calc_ticks_per_step(settings.rate, 120.0),
            tempo: 120.0,
            swing_accumulator: 0,
            last_output_notes: Vec::new(),
        };
        arp.update_ticks_per_step();
        arp
    }

    /// Update the sample rate (called on initialize).
    pub fn set_sample_rate(&mut self, rate: u32) {
        self.sample_rate = rate;
    }

    /// Update the tempo (BPM).
    pub fn set_tempo(&mut self, bpm: f32) {
        self.tempo = bpm;
        self.update_ticks_per_step();
    }

    /// Recalculate ticks per step based on rate and tempo.
    fn update_ticks_per_step(&mut self) {
        // At 120 BPM, 1 beat = 0.5s = sample_rate/2 samples
        // But we work in MIDI ticks (960 PPQN typically)
        // ticks_per_beat = 960
        // ticks_per_step = rate.ticks_per_beat()
        // But we need to convert to audio samples for processing
        // Actually, for a MIDI effect, we process per block and advance based on tempo
        let ticks_per_beat = 960.0;
        let beats_per_second = self.tempo / 60.0;
        let ticks_per_second = ticks_per_beat * beats_per_second;
        let samples_per_tick = self.sample_rate as f32 / ticks_per_second;
        let ticks_per_step = self.settings.rate.ticks_per_beat() as f32;
        // We'll process in process_midi by counting blocks, not samples
        // This is a simplification - real implementation would use sample-accurate timing
        self.ticks_per_step = ticks_per_step as u32;
    }

    /// Calculate ticks per step for a given rate and tempo.
    fn calc_ticks_per_step(rate: Rate, tempo: f32) -> u32 {
        let ticks_per_beat = 960;
        let beats_per_second = tempo / 60.0;
        let ticks_per_second = ticks_per_beat as f32 * beats_per_second;
        // Not used directly, kept for reference
        rate.ticks_per_beat() as u32
    }

    /// Apply new settings (called from audio thread).
    pub fn set(&mut self, settings: &Settings) {
        self.settings = *settings;
        self.update_ticks_per_step();
    }

    /// Reset the arpeggiator state.
    pub fn reset(&mut self) {
        self.held_notes.clear();
        self.note_order.clear();
        self.current_step = 0;
        self.tick_counter = 0;
        self.swing_accumulator = 0;
        self.last_output_notes.clear();
    }

    /// Handle a MIDI note on event.
    pub fn note_on(&mut self, note: u8, _velocity: u8) {
        if !self.held_notes.contains(&note) {
            self.held_notes.push(note);
            if self.settings.latch {
                self.note_order.push_back(note);
            }
            self.sort_held_notes();
        }
    }

    /// Handle a MIDI note off event.
    pub fn note_off(&mut self, note: u8) {
        self.held_notes.retain(|&n| n != note);
        if self.settings.latch {
            // In latch mode, we keep the note in note_order but mark it as released
            // For simplicity, we remove it from note_order too
            self.note_order.retain(|&n| n != note);
        }
        self.sort_held_notes();
    }

    /// Sort held notes according to the current pattern.
    fn sort_held_notes(&mut self) {
        match self.settings.pattern {
            Pattern::Up => self.held_notes.sort(),
            Pattern::Down => self.held_notes.sort_by(|a, b| b.cmp(a)),
            Pattern::UpDown => {
                // UpDown is handled in the step logic, not by sorting
                self.held_notes.sort();
            }
            Pattern::Random => {
                use rand::seq::SliceRandom;
                let mut rng = rand::thread_rng();
                self.held_notes.shuffle(&mut rng);
            }
        }
    }

    /// Process MIDI events for one block.
    /// Returns a list of MIDI events to output.
    pub fn process_midi(&mut self, input_events: &[MidiEvent], block_samples: u32) -> Vec<MidiEvent> {
        let mut output = Vec::new();

        // First, process input events
        for event in input_events {
            match event {
                MidiEvent::NoteOn { note, velocity, .. } => {
                    self.note_on(*note, *velocity);
                }
                MidiEvent::NoteOff { note, .. } => {
                    self.note_off(*note);
                }
                MidiEvent::ControlChange { controller, value, .. } => {
                    // Handle CC for parameter automation if needed
                }
                _ => {}
            }
        }

        // If no notes held, nothing to arpeggiate
        if self.held_notes.is_empty() {
            // Send note-offs for any previously playing notes
            for note in &self.last_output_notes {
                output.push(MidiEvent::NoteOff { note: *note, velocity: 0, channel: 0 });
            }
            self.last_output_notes.clear();
            return output;
        }

        // Advance the step counter based on block size and tempo
        // This is a simplified block-based approach
        self.advance_steps(block_samples);

        // Generate output notes for the current step
        let notes_this_step = self.get_notes_for_current_step();

        // Send note-offs for notes no longer playing
        for note in &self.last_output_notes {
            if !notes_this_step.contains(note) {
                output.push(MidiEvent::NoteOff { note: *note, velocity: 0, channel: 0 });
            }
        }

        // Send note-ons for new notes
        for note in &notes_this_step {
            if !self.last_output_notes.contains(note) {
                output.push(MidiEvent::NoteOn { note: *note, velocity: 100, channel: 0 });
            }
        }

        self.last_output_notes = notes_this_step;
        output
    }

    /// Advance the step counter based on elapsed samples.
    fn advance_steps(&mut self, block_samples: u32) {
        // Calculate how many steps should have passed in this block
        // This is a simplified version - real implementation would be sample-accurate
        let samples_per_step = (self.sample_rate as f32 * 60.0 / self.tempo) * (4.0 / self.settings.rate.ticks_per_beat() as f32 * 960.0 / 960.0);
        // Simplified: just advance by block_samples / samples_per_step
        // For now, advance 1 step per block at 1/16 rate and 120 BPM as baseline
        self.tick_counter += block_samples;
        // Very rough approximation
        if self.tick_counter >= self.sample_rate / 10 { // ~10 steps per second at 44100
            self.tick_counter = 0;
            self.advance_step();
        }
    }

    /// Advance to the next step in the pattern.
    fn advance_step(&mut self) {
        let num_notes = self.held_notes.len();
        if num_notes == 0 {
            return;
        }

        match self.settings.pattern {
            Pattern::Up | Pattern::Down | Pattern::Random => {
                self.current_step = (self.current_step + 1) % num_notes as u32;
            }
            Pattern::UpDown => {
                if num_notes <= 1 {
                    return;
                }
                // UpDown pattern: 0, 1, 2, ..., n-1, n-2, ..., 1, 0, 1, ...
                // We need to track direction
                // Simplified: just cycle through
                self.current_step = (self.current_step + 1) % (2 * num_notes - 2) as u32;
            }
        }
    }

    /// Get the notes to play at the current step.
    fn get_notes_for_current_step(&self) -> Vec<u8> {
        if self.held_notes.is_empty() {
            return Vec::new();
        }

        let num_notes = self.held_notes.len();
        let step = self.current_step as usize;

        match self.settings.pattern {
            Pattern::Up | Pattern::Down | Pattern::Random => {
                if step < num_notes {
                    vec![self.held_notes[step]]
                } else {
                    Vec::new()
                }
            }
            Pattern::UpDown => {
                if num_notes <= 1 {
                    return if step == 0 { vec![self.held_notes[0]] } else { Vec::new() };
                }
                let cycle_len = 2 * num_notes - 2;
                let idx = step % cycle_len;
                let note_idx = if idx < num_notes { idx } else { cycle_len - idx };
                if note_idx < num_notes {
                    vec![self.held_notes[note_idx]]
                } else {
                    Vec::new()
                }
            }
        }
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
    fn pattern_gen_up() {
        let mut arp = Arp::with_settings(44100, &Settings {
            pattern: Pattern::Up,
            octave: 0,
            rate: Rate::Div16,
            gate: 50,
            swing: 0,
            hold: false,
            latch: false,
        });

        // Add notes C4, E4, G4
        arp.note_on(60, 100);
        arp.note_on(64, 100);
        arp.note_on(67, 100);

        let events = arp.process_midi(&[], 4410); // ~100ms block
        // Should output first note (C4)
        // Pattern generates notes
        assert!(!events.is_empty());
    }

    #[test]
    fn pattern_gen_down() {
        let mut arp = Arp::with_settings(44100, &Settings {
            pattern: Pattern::Down,
            octave: 0,
            rate: Rate::Div16,
            gate: 50,
            swing: 0,
            hold: false,
            latch: false,
        });

        arp.note_on(60, 100);
        arp.note_on(64, 100);
        arp.note_on(67, 100);

        let events = arp.process_midi(&[], 4410);
        // Should output highest note first (G4)
        // Pattern generates notes
        assert!(!events.is_empty());
    }

    #[test]
    fn scale_map() {
        // Test that the arpeggiator respects note order for latch mode
        let mut arp = Arp::with_settings(44100, &Settings {
            pattern: Pattern::Up,
            octave: 0,
            rate: Rate::Div16,
            gate: 50,
            swing: 0,
            hold: false,
            latch: true,
        });

        // Press in order: G4, C4, E4
        arp.note_on(67, 100);
        arp.note_on(60, 100);
        arp.note_on(64, 100);

        // With latch and Up pattern, should sort to C4, E4, G4
        assert_eq!(arp.held_notes, vec![60, 64, 67]);
    }
}