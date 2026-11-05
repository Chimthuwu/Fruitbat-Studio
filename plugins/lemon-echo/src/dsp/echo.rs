// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Echo: the note echo DSP logic (pure, no audio I/O).

use crate::params::Settings;
use std::collections::VecDeque;

/// Maximum number of echo taps.
const MAX_TAPS: usize = 16;

/// A note echo that repeats MIDI notes with decreasing velocity.
pub struct Echo {
    sample_rate: u32,
    settings: Settings,
    tempo: f32,
    tap_queue: VecDeque<EchoTap>,
    next_tap_time: f32,
    tick_counter: u32,
    last_input_note: Option<(u8, u8)>, // (note, velocity)
}

#[derive(Clone, Copy)]
struct EchoTap {
    note: u8,
    velocity: u8,
    channel: u8,
    trigger_time: f32, // Time in beats when this tap should trigger
    is_ping_pong: bool,
    pan: f32, // -1.0 to 1.0 for ping-pong
}

impl Echo {
    /// Create a new echo with the given sample rate and settings.
    pub fn with_settings(sample_rate: u32, settings: &Settings) -> Self {
        Echo {
            sample_rate,
            settings: *settings,
            tempo: 120.0,
            tap_queue: VecDeque::new(),
            next_tap_time: 0.0,
            tick_counter: 0,
            last_input_note: None,
        }
    }

    /// Update the sample rate (called on initialize).
    pub fn set_sample_rate(&mut self, rate: u32) {
        self.sample_rate = rate;
    }

    /// Update the tempo (BPM).
    pub fn set_tempo(&mut self, bpm: f32) {
        self.tempo = bpm;
    }

    /// Apply new settings (called from audio thread).
    pub fn set(&mut self, settings: &Settings) {
        self.settings = *settings;
    }

    /// Reset the echo state.
    pub fn reset(&mut self) {
        self.tap_queue.clear();
        self.next_tap_time = 0.0;
        self.tick_counter = 0;
        self.last_input_note = None;
    }

    /// Handle a MIDI note on event. Schedules echo taps.
    pub fn note_on(&mut self, note: u8, velocity: u8, channel: u8) -> Vec<EchoEvent> {
        self.last_input_note = Some((note, velocity));
        self.schedule_echoes(note, velocity, channel);
        Vec::new() // No immediate output, original note passes through
    }

    /// Handle a MIDI note off event.
    pub fn note_off(&mut self, note: u8, channel: u8) -> Vec<EchoEvent> {
        // Note off doesn't affect scheduled echoes
        Vec::new()
    }

    /// Schedule echo taps for a note.
    fn schedule_echoes(&mut self, note: u8, velocity: u8, channel: u8) {
        let delay_beats = self.settings.time as f32 / 1000.0; // Convert ms to beats (approx)
        let feedback = self.settings.feedback as f32 / 100.0;
        let velocity_decay = 0.0;
        let mut current_velocity = velocity as f32;
        let mut current_time = delay_beats;

        // Clear old taps if queue is full
        while self.tap_queue.len() >= MAX_TAPS {
            self.tap_queue.pop_front();
        }

        // Schedule initial echo
        if self.settings.sync {
            // Quantize to grid
            current_time = delay_beats;
        }

        let mut ping_pong_state = false;
        let tap_count = if self.settings.feedback > 0 { 8 } else { 1 };

        for i in 0..tap_count {
            if current_velocity < 1.0 {
                break;
            }

            let tap = EchoTap {
                note,
                velocity: current_velocity.clamp(1.0, 127.0) as u8,
                channel,
                trigger_time: current_time,
                is_ping_pong: self.settings.mode == crate::params::EchoMode::PingPong && ping_pong_state,
                pan: if self.settings.mode == crate::params::EchoMode::PingPong && ping_pong_state { 1.0 } else { -1.0 },
            };

            self.tap_queue.push_back(tap);

            // Next tap
            current_velocity *= feedback * (1.0 - velocity_decay * 0.5);
            current_time += delay_beats;
            ping_pong_state = !ping_pong_state;
        }

        // Sort by trigger time
        self.tap_queue.make_contiguous().sort_by(|a, b| a.trigger_time.partial_cmp(&b.trigger_time).unwrap());
    }

    /// Process scheduled echoes for the current block.
    /// Returns a list of MIDI events to output.
    pub fn process_block(&mut self, block_beats: f32) -> Vec<EchoEvent> {
        let mut output = Vec::new();
        let current_time = self.tick_counter as f32 * block_beats;

        // Check for taps that should trigger now
        while let Some(tap) = self.tap_queue.front() {
            if tap.trigger_time <= current_time {
                let tap = self.tap_queue.pop_front().unwrap();
                output.push(EchoEvent::NoteOn {
                    note: tap.note,
                    velocity: tap.velocity,
                    channel: tap.channel,
                    pan: tap.pan,
                });
            } else {
                break;
            }
        }

        self.tick_counter += 1;
        output
    }
}

/// Echo event types for internal processing.
#[derive(Clone, Debug)]
pub enum EchoEvent {
    NoteOn { note: u8, velocity: u8, channel: u8, pan: f32 },
    NoteOff { note: u8, velocity: u8, channel: u8 },
    ControlChange { controller: u8, value: u8, channel: u8 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn echo_scheduling_basic() {
        let mut echo = Echo::with_settings(44100, &Settings {
            time: 250,
            feedback: 50,
            mix: 50,
            mode: crate::params::EchoMode::Normal,
            sync: true,
            filter: 0,
            cutoff: 100,
            pan: 0,
            width: 50,
            mod_rate: 0,
            mod_depth: 0,
        });

        echo.set_tempo(120.0);

        // Trigger a note
        echo.note_on(60, 100, 0);

        // Process one block (1/8 note at 120 BPM = 250ms = ~11025 samples at 44.1kHz)
        // Block size in beats: 128 samples / 44100 * 120/60 = ~0.00058 beats
        let events = echo.process_block(0.01); // Small time step

        // Should have scheduled echoes
        assert!(!echo.tap_queue.is_empty());
    }

    #[test]
    fn feedback_decay() {
        let mut echo = Echo::with_settings(44100, &Settings {
            time: 250,
            feedback: 50,  // 50% feedback
            mix: 50,
            mode: crate::params::EchoMode::Normal,
            sync: true,
            filter: 0,
            cutoff: 100,
            pan: 0,
            width: 50,
            mod_rate: 0,
            mod_depth: 0,
        });

        echo.set_tempo(120.0);
        echo.note_on(60, 100, 0);

        // Check that scheduled taps have decreasing velocity
        let mut prev_vel = 101;
        for tap in &echo.tap_queue {
            assert!(tap.velocity < prev_vel, "Velocity should decay: {} >= {}", tap.velocity, prev_vel);
            prev_vel = tap.velocity;
        }
    }

    #[test]
    fn ping_pong_alternates() {
        let mut echo = Echo::with_settings(44100, &Settings {
            time: 250,
            feedback: 50,
            mix: 50,
            mode: crate::params::EchoMode::PingPong,
            sync: true,
            filter: 0,
            cutoff: 100,
            pan: 0,
            width: 50,
            mod_rate: 0,
            mod_depth: 0,
        });

        echo.set_tempo(120.0);
        echo.note_on(60, 100, 0);

        let mut ping_pong_count = 0;
        for (i, tap) in echo.tap_queue.iter().enumerate() {
            if i > 0 {
                // Should alternate pan
                let expected_pan = if i % 2 == 0 { -1.0 } else { 1.0 };
                assert_eq!(tap.pan, expected_pan, "Tap {} should have pan {}", i, expected_pan);
            }
            ping_pong_count += 1;
        }
        assert!(ping_pong_count > 1);
    }
}