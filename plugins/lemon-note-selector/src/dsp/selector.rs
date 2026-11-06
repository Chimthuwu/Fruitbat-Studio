// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Note Selector: MIDI selector container DSP (pure, no audio I/O).
//!
//! Four slots, each Bypass/Arp/Chord/Echo. Only the selected slot sounds.
//! Selection modes: 0=Manual (manual_slot param), 1=Channel (channel % 4),
//! 2=NoteRange (note / 32), 3=CC1 (last CC1 value / 32).

use crate::params::{Mode, Settings, SlotType};

pub const NSLOTS: usize = 4;

/// MIDI event types for internal processing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MidiEvent {
    NoteOn { note: u8, velocity: u8, channel: u8 },
    NoteOff { note: u8, velocity: u8, channel: u8 },
    ControlChange { controller: u8, value: u8, channel: u8 },
}

pub struct Selector {
    sample_rate: u32,
    settings: Settings,
    /// Last CC1 value seen (0-127); used in CC1 selection mode.
    pub cc1: u8,
}

impl Selector {
    pub fn with_settings(sample_rate: u32, settings: &Settings) -> Self {
        Selector { sample_rate, settings: *settings, cc1: 0 }
    }

    pub fn set_sample_rate(&mut self, rate: u32) {
        self.sample_rate = rate;
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn set(&mut self, settings: &Settings) {
        self.settings = *settings;
    }

    pub fn reset(&mut self) {
        self.cc1 = 0;
    }

    /// Slot selected for an event on `channel` playing `note`.
    pub fn selected_slot(&self, channel: u8, note: u8) -> usize {
        match self.settings.mode {
            Mode::Manual => (self.settings.manual_slot as usize) % NSLOTS,
            Mode::Channel => (channel as usize) % NSLOTS,
            Mode::NoteRange => ((note as usize) / 32) % NSLOTS,
            Mode::Cc1 => ((self.cc1 as usize) / 32) % NSLOTS,
        }
    }

    /// Slot selected from a CC1 value (CC1 mode).
    pub fn selected_slot_for_cc1(&self, value: u8) -> usize {
        ((value as usize) / 32) % NSLOTS
    }

    fn slot_sounds(&self, slot: usize) -> bool {
        self.settings.slots[slot].active
    }

    /// Process MIDI events for one block. Only the selected slot sounds:
    /// events routing to an inactive slot are dropped; events routing to an
    /// active slot pass through (Chord adds major-third+fifth on note on/off).
    pub fn process_midi(&mut self, input_events: &[MidiEvent], _block_samples: u32) -> Vec<MidiEvent> {
        let mut output = Vec::new();
        for event in input_events {
            // CC1 always updates the CC1-mode selection state.
            if let MidiEvent::ControlChange { controller: 1, value, .. } = event {
                self.cc1 = *value;
            }
            let (channel, note) = match event {
                MidiEvent::NoteOn { note, channel, .. } => (*channel, *note),
                MidiEvent::NoteOff { note, channel, .. } => (*channel, *note),
                MidiEvent::ControlChange { channel, .. } => (*channel, 0),
            };
            // In CC1 mode, a CC1 event selects by its own value.
            let slot = match event {
                MidiEvent::ControlChange { controller: 1, value, .. } if self.settings.mode == Mode::Cc1 => {
                    self.selected_slot_for_cc1(*value)
                }
                _ => self.selected_slot(channel, note),
            };
            if !self.slot_sounds(slot) {
                continue;
            }
            match (self.settings.slots[slot].slot_type, event) {
                (SlotType::Chord, MidiEvent::NoteOn { note, velocity, channel }) => {
                    output.push(MidiEvent::NoteOn { note: *note, velocity: *velocity, channel: *channel });
                    for extra in [4u16, 7] {
                        if let Some(n) = (*note as u16 + extra).try_into().ok().filter(|n| *n < 128) {
                            output.push(MidiEvent::NoteOn { note: n, velocity: *velocity, channel: *channel });
                        }
                    }
                }
                (SlotType::Chord, MidiEvent::NoteOff { note, velocity, channel }) => {
                    output.push(MidiEvent::NoteOff { note: *note, velocity: *velocity, channel: *channel });
                    for extra in [4u16, 7] {
                        if let Some(n) = (*note as u16 + extra).try_into().ok().filter(|n| *n < 128) {
                            output.push(MidiEvent::NoteOff { note: n, velocity: *velocity, channel: *channel });
                        }
                    }
                }
                // Bypass / Arp / Echo: pass through unchanged.
                _ => output.push(event.clone()),
            }
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{Mode, Settings, Slot, SlotType};

    fn slots() -> [Slot; 4] {
        [Slot { slot_type: SlotType::Bypass, active: true }; 4]
    }

    fn note_on(note: u8, channel: u8) -> MidiEvent {
        MidiEvent::NoteOn { note, velocity: 100, channel }
    }

    #[test]
    fn manual_routing() {
        let mut sel = Selector::with_settings(44100, &Settings { mode: Mode::Manual, manual_slot: 2, slots: slots() });
        assert_eq!(sel.selected_slot(0, 60), 2);
        assert_eq!(sel.selected_slot(5, 10), 2);
        let out = sel.process_midi(&[note_on(60, 0)], 512);
        assert_eq!(out.len(), 1, "selected slot sounds");
        // Inactive slot sounds nothing.
        sel.settings.slots[2].active = false;
        assert!(sel.process_midi(&[note_on(60, 0)], 512).is_empty());
    }

    #[test]
    fn channel_routing() {
        let mut sel = Selector::with_settings(44100, &Settings { mode: Mode::Channel, manual_slot: 0, slots: slots() });
        assert_eq!(sel.selected_slot(0, 60), 0);
        assert_eq!(sel.selected_slot(1, 60), 1);
        assert_eq!(sel.selected_slot(5, 60), 1, "5 % 4 == 1");
        assert_eq!(sel.selected_slot(6, 60), 2, "6 % 4 == 2");
        sel.settings.slots[1].active = false;
        assert!(sel.process_midi(&[note_on(60, 1)], 512).is_empty(), "inactive selected slot drops");
        assert_eq!(sel.process_midi(&[note_on(60, 0)], 512).len(), 1, "active slot sounds");
    }

    #[test]
    fn note_range_routing() {
        let mut sel = Selector::with_settings(44100, &Settings { mode: Mode::NoteRange, manual_slot: 0, slots: slots() });
        assert_eq!(sel.selected_slot(0, 0), 0);
        assert_eq!(sel.selected_slot(0, 31), 0);
        assert_eq!(sel.selected_slot(0, 32), 1);
        assert_eq!(sel.selected_slot(0, 64), 2);
        assert_eq!(sel.selected_slot(0, 96), 3);
        assert_eq!(sel.selected_slot(0, 127), 3);
        sel.settings.slots[3].active = false;
        assert!(sel.process_midi(&[note_on(100, 0)], 512).is_empty());
        assert_eq!(sel.process_midi(&[note_on(10, 0)], 512).len(), 1);
    }

    #[test]
    fn cc1_routing() {
        let mut sel = Selector::with_settings(44100, &Settings { mode: Mode::Cc1, manual_slot: 0, slots: slots() });
        assert_eq!(sel.selected_slot_for_cc1(0), 0);
        assert_eq!(sel.selected_slot_for_cc1(31), 0);
        assert_eq!(sel.selected_slot_for_cc1(32), 1);
        assert_eq!(sel.selected_slot_for_cc1(64), 2);
        assert_eq!(sel.selected_slot_for_cc1(96), 3);
        assert_eq!(sel.selected_slot_for_cc1(127), 3);
        // CC1 event updates selection for subsequent notes.
        let _ = sel.process_midi(&[MidiEvent::ControlChange { controller: 1, value: 64, channel: 0 }], 512);
        assert_eq!(sel.selected_slot(0, 60), 2, "CC1=64 -> slot 2");
        sel.settings.slots[2].active = false;
        assert!(sel.process_midi(&[note_on(60, 0)], 512).is_empty(), "slot 2 inactive drops");
        let _ = sel.process_midi(&[MidiEvent::ControlChange { controller: 1, value: 0, channel: 0 }], 512);
        assert_eq!(sel.process_midi(&[note_on(60, 0)], 512).len(), 1, "CC1=0 -> slot 0 active sounds");
    }

    #[test]
    fn only_selected_slot_sounds_and_chord_adds() {
        let mut s = slots();
        s[0].slot_type = SlotType::Chord;
        s[1].slot_type = SlotType::Arp;
        let mut sel = Selector::with_settings(44100, &Settings { mode: Mode::Manual, manual_slot: 0, slots: s });
        let out = sel.process_midi(&[note_on(60, 0)], 512);
        assert_eq!(out.len(), 3, "chord: root + third + fifth");
        sel.set(&Settings { mode: Mode::Manual, manual_slot: 1, slots: s });
        let out = sel.process_midi(&[note_on(60, 0)], 512);
        assert_eq!(out.len(), 1, "arp pass-through, single note");
    }
}
