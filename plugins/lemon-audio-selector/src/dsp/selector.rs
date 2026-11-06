// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Stereo audio selector DSP: only the selected slot processes audio.
//!
//! Selections: 0=Bypass 1=Filter 2=Delay 3=Chorus 4=Drive. Modes: 0=Manual (manual knob) and
//! 1=Note (last note/channel selects). Slot type is fixed per slot index:
//! slot1=Filter, slot2=Delay, slot3=Chorus, slot4=Drive.

use crate::params::{SelMode, Settings, SlotKind};

/// Number of selectable kinds (including Bypass).
pub const SELECTIONS: usize = 5;
/// Number of processable slots (geometrical audio slots).
pub const NUM_SLOTS: usize = 4;

/// Route a channel value to a selection index.
pub fn channel_to_slot(channel: u8) -> usize {
    (channel % SELECTIONS as u8) as usize
}

/// Route a note number to a selection index (hashed by octave-ish bucket).
pub fn note_to_slot(note: u8) -> usize {
    ((note / 12) % SELECTIONS as u8) as usize
}

/// A stereo selector: holds last-selected state and per-slot DSP state.
pub struct Selector {
    sample_rate: u32,
    settings: Settings,
    /// Last selected index (used in Note mode; also updated in Manual).
    last_selected: usize,
    // Filter state (one-pole LP) per channel.
    filter_state: [f32; 2],
    // Delay line (mono summed then split), sized for 0.5s max.
    delay_buf: Vec<f32>,
    delay_pos: usize,
    // Chorus: second delay line with LFO phase.
    chorus_buf: Vec<f32>,
    chorus_pos: usize,
    chorus_phase: f32,
}

impl Selector {
    pub fn new(sample_rate: u32) -> Self {
        Selector::with_settings(sample_rate, &Settings::default())
    }

    pub fn with_settings(sample_rate: u32, settings: &Settings) -> Self {
        let max_delay = (sample_rate as usize / 2).max(1);
        Selector {
            sample_rate,
            settings: *settings,
            last_selected: settings.manual as usize,
            filter_state: [0.0; 2],
            delay_buf: vec![0.0; max_delay],
            delay_pos: 0,
            chorus_buf: vec![0.0; max_delay],
            chorus_pos: 0,
            chorus_phase: 0.0,
        }
    }

    pub fn set_sample_rate(&mut self, rate: u32) {
        if rate == self.sample_rate {
            return;
        }
        self.sample_rate = rate;
        let max_delay = (rate as usize / 2).max(1);
        self.delay_buf.resize(max_delay, 0.0);
        self.chorus_buf.resize(max_delay, 0.0);
    }

    pub fn set(&mut self, settings: &Settings) {
        self.settings = *settings;
    }

    pub fn reset(&mut self) {
        self.filter_state = [0.0; 2];
        for x in &mut self.delay_buf {
            *x = 0.0;
        }
        for x in &mut self.chorus_buf {
            *x = 0.0;
        }
        self.delay_pos = 0;
        self.chorus_pos = 0;
        self.chorus_phase = 0.0;
        self.last_selected = self.settings.manual as usize;
    }

    /// Currently selected index 0..4.
    pub fn selected(&self) -> usize {
        match self.settings.mode {
            SelMode::Manual => self.settings.manual as usize,
            SelMode::Note => self.last_selected,
        }
    }

    /// Directly set the last-selected index (used in Note mode).
    pub fn select(&mut self, index: usize) {
        self.last_selected = index.min(SELECTIONS - 1);
        if self.settings.mode == SelMode::Manual {
            // In Manual mode, the parameter wins on next `selected()`, but keep last in sync where possible.
        }
    }

    /// MIDI note-on: channel selects per MIDI Channel mode; note number otherwise by octave bucket.
    pub fn note_on(&mut self, note: u8, _velocity: f32, channel: u8) {
        let idx = if channel != 0 || note == 0 {
            channel_to_slot(channel)
        } else {
            note_to_slot(note)
        };
        self.last_selected = idx;
    }

    /// Process stereo slices in place. Only selected slot processes.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len());
        let left = &mut left[..len];
        let right = &mut right[..len];
        let sel = self.selected();
        let kind = SlotKind::from_index(sel as i32);

        // Gain from selected slot's level (Bypass uses 1.0 unless selected slot inactive).
        let (level, param, active) = match kind {
            SlotKind::Bypass => (6000, 0, true),
            SlotKind::Filter => self.slot(0),
            SlotKind::Delay => self.slot(1),
            SlotKind::Chorus => self.slot(2),
            SlotKind::Drive => self.slot(3),
        };

        if !active {
            // Inactive selected slot = passthrough (still scaled by its level for Bypass/passthrough consistency).
            apply_gain(left, right, level_to_gain(level));
            return;
        }

        match kind {
            SlotKind::Bypass => apply_gain(left, right, level_to_gain(level)),
            SlotKind::Filter => self.process_filter(left, right, param, level),
            SlotKind::Delay => self.process_delay(left, right, param, level),
            SlotKind::Chorus => self.process_chorus(left, right, param, level),
            SlotKind::Drive => self.process_drive(left, right, param, level),
        }
    }

    fn slot(&self, idx: usize) -> (i32, i32, bool) {
        if idx < NUM_SLOTS {
            let s = self.settings.slots[idx];
            (s.level, s.param, s.active)
        } else {
            (6000, 0, false)
        }
    }

    fn process_filter(&mut self, left: &mut [f32], right: &mut [f32], param: i32, level: i32) {
        let x = (param as f32 / 6000.0).clamp(0.0, 1.0);
        let coeff = (x * 0.95 + 0.02).clamp(0.0, 1.0);
        let gain = level_to_gain(level);
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            self.filter_state[0] += coeff * (*l - self.filter_state[0]);
            self.filter_state[1] += coeff * (*r - self.filter_state[1]);
            *l = self.filter_state[0] * gain;
            *r = self.filter_state[1] * gain;
        }
    }

    fn process_delay(&mut self, left: &mut [f32], right: &mut [f32], param: i32, level: i32) {
        let gain = level_to_gain(level);
        let x = (param as f32 / 6000.0).clamp(0.0, 1.0);
        let delay_len = ((0.02 + x * 0.4) * self.sample_rate as f32) as usize;
        let delay_len = delay_len.min(self.delay_buf.len().max(1)).max(1);
        let feedback = 0.35 * x;
        let mix = 0.5;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let mid = (*l + *r) * 0.5;
            let read = self.delay_buf[self.delay_pos];
            self.delay_buf[self.delay_pos] = mid + read * feedback;
            self.delay_pos = (self.delay_pos + 1) % self.delay_buf.len();
            let wet = self.delay_buf[(self.delay_pos + self.delay_buf.len() - delay_len) % self.delay_buf.len()];
            *l = ((*l * (1.0 - mix)) + wet * mix) * gain;
            *r = ((*r * (1.0 - mix)) + wet * mix) * gain;
        }
    }

    fn process_chorus(&mut self, left: &mut [f32], right: &mut [f32], param: i32, level: i32) {
        let gain = level_to_gain(level);
        let x = (param as f32 / 6000.0).clamp(0.0, 1.0);
        let base = (0.005 + x * 0.015) * self.sample_rate as f32;
        let depth = 0.002 * self.sample_rate as f32 * (0.5 + x);
        let rate = 0.5 + x * 4.0;
        let mix = 0.5;
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            self.chorus_phase += rate / self.sample_rate as f32 * std::f32::consts::TAU;
            let mod_offset = depth * self.chorus_phase.sin();
            let len_l = (base + mod_offset).round() as usize;
            let len_r = (base - mod_offset).round() as usize;
            let read_l = if !self.chorus_buf.is_empty() {
                let idx = (self.chorus_pos + self.chorus_buf.len() - len_l.min(self.chorus_buf.len() - 1).max(1)) % self.chorus_buf.len();
                self.chorus_buf[idx]
            } else { 0.0 };
            let read_r = if !self.chorus_buf.is_empty() {
                let idx = (self.chorus_pos + self.chorus_buf.len() - len_r.min(self.chorus_buf.len() - 1).max(1)) % self.chorus_buf.len();
                self.chorus_buf[idx]
            } else { 0.0 };
            self.chorus_buf[self.chorus_pos] = *l;
            self.chorus_pos = (self.chorus_pos + 1) % self.chorus_buf.len().max(1);
            *l = ((*l * (1.0 - mix)) + read_l * mix) * gain;
            *r = ((*r * (1.0 - mix)) + read_r * mix) * gain;
        }
    }

    fn process_drive(&mut self, left: &mut [f32], right: &mut [f32], param: i32, level: i32) {
        let x = (param as f32 / 6000.0).clamp(0.0, 1.0);
        let amt = 1.0 + x * 9.0;
        let gain = level_to_gain(level);
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            *l = ((*l * amt).tanh()) * gain;
            *r = ((*r * amt).tanh()) * gain;
        }
    }
}

fn level_to_gain(level: i32) -> f32 {
    (level as f32 / 6000.0).clamp(0.0, 1.0) * 1.5
}

fn apply_gain(left: &mut [f32], right: &mut [f32], gain: f32) {
    for (l, r) in left.iter_mut().zip(right.iter_mut()) {
        *l *= gain;
        *r *= gain;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{SlotKind, SlotSettings};

    fn settings(manual: SlotKind, mode: SelMode) -> Settings {
        Settings {
            mode,
            manual,
            slots: [
                SlotSettings { active: true, level: 6000, param: 6000 },
                SlotSettings { active: true, level: 6000, param: 0 },
                SlotSettings { active: true, level: 6000, param: 0 },
                SlotSettings { active: true, level: 6000, param: 0 },
            ],
        }
    }

    #[test]
    fn manual_selection_routing() {
        // Bypass: passthrough at unity gain (6000 -> 1.5? level 6000 => 1.5, so use 4000 => 1.0)
        let mut s = Selector::with_settings(44100, &settings(SlotKind::Bypass, SelMode::Manual));
        // Bypass level is fixed 6000 in dsp -> gain 1.5
        let mut l = vec![0.5; 8];
        let mut r = vec![0.25; 8];
        s.process(&mut l, &mut r);
        assert!((l[0] - 0.75).abs() < 1e-6);
        // Filter: one-pole at full param passes almost unchanged (coeff~0.97)
        let mut s = Selector::with_settings(44100, &settings(SlotKind::Filter, SelMode::Manual));
        let mut l = vec![1.0; 32];
        let mut r = vec![1.0; 32];
        s.process(&mut l, &mut r);
        assert!(l[31] > 0.9);
        // Drive: tanh at low param still produces non-zero, bounded output.
        let mut s = Selector::with_settings(44100, &settings(SlotKind::Drive, SelMode::Manual));
        let mut l = vec![0.8; 8];
        let mut r = vec![-0.8; 8];
        s.process(&mut l, &mut r);
        assert!(l[0].abs() <= 1.5 && l[0] != 0.8);
        // Delay & Chorus produce finite output and respect levels.
        for kind in [SlotKind::Delay, SlotKind::Chorus] {
            let mut s = Selector::with_settings(44100, &settings(kind, SelMode::Manual));
            let mut l = vec![0.5; 16];
            let mut r = vec![0.5; 16];
            s.process(&mut l, &mut r);
            assert!(l.iter().all(|x| x.is_finite()));
        }
    }

    #[test]
    fn note_mode_selects_by_channel() {
        let mut s = Selector::with_settings(44100, &settings(SlotKind::Bypass, SelMode::Note));
        s.note_on(60, 100.0, 3);
        assert_eq!(s.selected(), 3);
        s.note_on(60, 100.0, 4);
        assert_eq!(s.selected(), 4);
        s.note_on(60, 100.0, 0);
        assert_eq!(s.selected(), 0);
    }

    #[test]
    fn inactive_slot_is_passthrough() {
        let mut st = settings(SlotKind::Filter, SelMode::Manual);
        st.slots[0].active = false;
        st.slots[0].level = 4000; // unity
        let mut s = Selector::with_settings(44100, &st);
        let mut l = vec![0.5; 4];
        let mut r = vec![0.5; 4];
        s.process(&mut l, &mut r);
        assert!((l[0] - 0.5).abs() < 1e-6);
    }
}
