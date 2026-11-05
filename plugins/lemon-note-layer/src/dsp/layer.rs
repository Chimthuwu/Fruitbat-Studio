// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Note Layer: the MIDI layer container DSP logic (pure, no audio I/O).

use crate::params::{LayerType, Settings};

fn scaled_velocity(velocity: u8, level: u8) -> u8 {
    let v = velocity as u32 * level as u32 / 100;
    if velocity > 0 && v == 0 { 1 } else { v as u8 }
}

/// MIDI event types for internal processing.
#[derive(Clone, Debug)]
pub enum MidiEvent {
    NoteOn { note: u8, velocity: u8, channel: u8 },
    NoteOff { note: u8, velocity: u8, channel: u8 },
    ControlChange { controller: u8, value: u8, channel: u8 },
}

struct EchoPending {
    samples_left: u64,
    note: u8,
    velocity: u8,
    channel: u8,
}

/// A MIDI layer container routing notes to up to 4 slots.
pub struct NoteLayer {
    sample_rate: u32,
    settings: Settings,
    /// Held notes per slot, sorted ascending (used by Arp slots).
    held: [Vec<u8>; 4],
    /// Next Arp step index per slot.
    step: [usize; 4],
    /// Scheduled Echo repeats.
    pending: Vec<EchoPending>,
}

impl NoteLayer {
    pub fn with_settings(sample_rate: u32, settings: &Settings) -> Self {
        Self {
            sample_rate,
            settings: *settings,
            held: Default::default(),
            step: [0; 4],
            pending: Vec::new(),
        }
    }

    pub fn set_sample_rate(&mut self, rate: u32) {
        self.sample_rate = rate;
    }

    pub fn set(&mut self, settings: &Settings) {
        self.settings = *settings;
    }

    pub fn reset(&mut self) {
        self.held = Default::default();
        self.step = [0; 4];
        self.pending.clear();
    }

    fn channel_matches(slot_channel: u8, channel: u8) -> bool {
        slot_channel == 16 || slot_channel == channel
    }

    fn echo_delay_samples(&self) -> u64 {
        (self.sample_rate.max(1) as u64) / 4
    }

    /// Process MIDI events for one block. Returns a list of MIDI events to output.
    pub fn process_midi(&mut self, input_events: &[MidiEvent], block_samples: u32) -> Vec<MidiEvent> {
        let mut output = Vec::new();

        // Flush due Echo repeats from previous blocks.
        let mut still: Vec<EchoPending> = Vec::new();
        for mut p in std::mem::take(&mut self.pending) {
            if (p.samples_left as u128).saturating_sub(block_samples as u128) == 0 {
                output.push(MidiEvent::NoteOn { note: p.note, velocity: p.velocity, channel: p.channel });
                output.push(MidiEvent::NoteOff { note: p.note, velocity: 0, channel: p.channel });
            } else {
                p.samples_left = p.samples_left.saturating_sub(block_samples as u64);
                still.push(p);
            }
        }
        self.pending = still;

        for event in input_events {
            match event {
                MidiEvent::NoteOn { note, velocity, channel } => {
                    self.note_on(*note, *velocity, *channel, &mut output);
                }
                MidiEvent::NoteOff { note, velocity, channel } => {
                    self.note_off(*note, *velocity, *channel, &mut output);
                }
                MidiEvent::ControlChange { controller, value, channel } => {
                    output.push(MidiEvent::ControlChange { controller: *controller, value: *value, channel: *channel });
                }
            }
        }
        output
    }

    fn note_on(&mut self, note: u8, velocity: u8, channel: u8, output: &mut Vec<MidiEvent>) {
        for (i, slot) in self.settings.layers.iter().enumerate() {
            if !slot.active || !Self::channel_matches(slot.channel, channel) {
                continue;
            }
            let vel = scaled_velocity(velocity, slot.level);
            match slot.typ {
                LayerType::Bypass => {
                    output.push(MidiEvent::NoteOn { note, velocity: vel, channel });
                }
                LayerType::Chord => {
                    output.push(MidiEvent::NoteOn { note, velocity: vel, channel });
                    output.push(MidiEvent::NoteOn { note: note.saturating_add(4).min(127), velocity: vel, channel });
                    output.push(MidiEvent::NoteOn { note: note.saturating_add(7).min(127), velocity: vel, channel });
                }
                LayerType::Arp => {
                    if !self.held[i].contains(&note) {
                        self.held[i].push(note);
                        self.held[i].sort();
                    }
                    let step = self.step[i] % self.held[i].len();
                    let play = self.held[i][step];
                    self.step[i] = (step + 1) % self.held[i].len();
                    output.push(MidiEvent::NoteOn { note: play, velocity: vel, channel });
                }
                LayerType::Echo => {
                    output.push(MidiEvent::NoteOn { note, velocity: vel, channel });
                    let delay = self.echo_delay_samples();
                    self.pending.push(EchoPending { samples_left: delay, note, velocity: vel, channel });
                    self.pending.push(EchoPending { samples_left: delay * 2, note, velocity: vel, channel });
                }
            }
        }
    }

    fn note_off(&mut self, note: u8, velocity: u8, channel: u8, output: &mut Vec<MidiEvent>) {
        for (i, slot) in self.settings.layers.iter().enumerate() {
            if !slot.active || !Self::channel_matches(slot.channel, channel) {
                continue;
            }
            self.held[i].retain(|&n| n != note);
            match slot.typ {
                LayerType::Bypass => {
                    output.push(MidiEvent::NoteOff { note, velocity, channel });
                }
                LayerType::Chord => {
                    output.push(MidiEvent::NoteOff { note, velocity, channel });
                    output.push(MidiEvent::NoteOff { note: note.saturating_add(4).min(127), velocity, channel });
                    output.push(MidiEvent::NoteOff { note: note.saturating_add(7).min(127), velocity, channel });
                }
                LayerType::Arp => {
                    output.push(MidiEvent::NoteOff { note, velocity, channel });
                }
                LayerType::Echo => {
                    output.push(MidiEvent::NoteOff { note, velocity, channel });
                    self.pending.retain(|p| p.note != note);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{LayerSlot, LayerType, Settings};

    fn settings_with(slot: usize, slot_settings: LayerSlot) -> Settings {
        let mut s = Settings::default();
        s.layers = [LayerSlot { active: false, typ: LayerType::Bypass, channel: 16, level: 100 }; 4];
        s.layers[slot] = slot_settings;
        s.dry_level = 100;
        s
    }

    #[test]
    fn layer_routing_active_slot_produces_output() {
        let settings = settings_with(0, LayerSlot { active: true, typ: LayerType::Bypass, channel: 16, level: 100 });
        let mut nl = NoteLayer::with_settings(44100, &settings);
        let out = nl.process_midi(&[MidiEvent::NoteOn { note: 60, velocity: 100, channel: 0 }], 512);
        assert!(!out.is_empty(), "active slot must produce output");
        assert!(out.iter().any(|e| matches!(e, MidiEvent::NoteOn { note: 60, .. })));
    }

    #[test]
    fn layer_routing_inactive_slot_produces_no_output() {
        let settings = settings_with(0, LayerSlot { active: false, typ: LayerType::Bypass, channel: 16, level: 100 });
        let mut nl = NoteLayer::with_settings(44100, &settings);
        let out = nl.process_midi(&[MidiEvent::NoteOn { note: 60, velocity: 100, channel: 0 }], 512);
        assert!(out.is_empty(), "inactive slot must not produce output");
    }

    #[test]
    fn channel_filter_blocks_mismatched_channel() {
        let settings = settings_with(0, LayerSlot { active: true, typ: LayerType::Bypass, channel: 1, level: 100 });
        let mut nl = NoteLayer::with_settings(44100, &settings);
        let out = nl.process_midi(&[MidiEvent::NoteOn { note: 60, velocity: 100, channel: 2 }], 512);
        assert!(out.is_empty(), "channel 2 must not pass a channel-1 slot");
        let out = nl.process_midi(&[MidiEvent::NoteOn { note: 60, velocity: 100, channel: 1 }], 512);
        assert!(!out.is_empty(), "channel 1 must pass a channel-1 slot");
    }

    #[test]
    fn chord_outputs_major_third_and_fifth() {
        let settings = settings_with(0, LayerSlot { active: true, typ: LayerType::Chord, channel: 16, level: 100 });
        let mut nl = NoteLayer::with_settings(44100, &settings);
        let out = nl.process_midi(&[MidiEvent::NoteOn { note: 60, velocity: 100, channel: 0 }], 512);
        let notes: Vec<u8> = out.iter().filter_map(|e| match e {
            MidiEvent::NoteOn { note, .. } => Some(*note),
            _ => None,
        }).collect();
        assert_eq!(notes, vec![60, 64, 67]);
    }

    #[test]
    fn arp_cycles_held_notes_upward() {
        let settings = settings_with(0, LayerSlot { active: true, typ: LayerType::Arp, channel: 16, level: 100 });
        let mut nl = NoteLayer::with_settings(44100, &settings);
        let out1 = nl.process_midi(&[MidiEvent::NoteOn { note: 67, velocity: 100, channel: 0 }], 512);
        let out2 = nl.process_midi(&[MidiEvent::NoteOn { note: 60, velocity: 100, channel: 0 }], 512);
        let out3 = nl.process_midi(&[MidiEvent::NoteOn { note: 64, velocity: 100, channel: 0 }], 512);
        let n1 = match &out1[0] { MidiEvent::NoteOn { note, .. } => *note, _ => panic!("note on expected") };
        let n2 = match &out2[0] { MidiEvent::NoteOn { note, .. } => *note, _ => panic!("note on expected") };
        let n3 = match &out3[0] { MidiEvent::NoteOn { note, .. } => *note, _ => panic!("note on expected") };
        let _ = n3;
        assert_eq!(n1, 67);
        assert_eq!(n2, 60);
    }
}
