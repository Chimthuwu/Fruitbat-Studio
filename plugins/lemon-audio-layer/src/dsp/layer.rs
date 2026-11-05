// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Audio Layer DSP: 4 parallel stereo slots summed over a master dry/wet.
//!
//! Slot types: 0=Bypass, 1=Filter (one-pole lowpass), 2=Delay (feedback delay),
//! 3=Chorus (modulated delay), 4=Drive (tanh). Delay and Chorus share one stereo
//! delay line sized by the sample rate (1 second per channel).

use crate::params::{Settings, SLOT_COUNT};

const TY_BYPASS: u8 = 0;
const TY_FILTER: u8 = 1;
const TY_DELAY: u8 = 2;
const TY_CHORUS: u8 = 3;
const TY_DRIVE: u8 = 4;

fn norm(v: i32) -> f32 {
    (v.clamp(0, 6000) as f32) / 6000.0
}

fn delay_feedback() -> f32 {
    0.35
}

/// Delay time in samples: param maps to 1..200 ms.
fn delay_time_samples(param: i32, sample_rate: u32) -> usize {
    let ms = 1.0 + norm(param) * 199.0;
    ((ms * 0.001 * sample_rate as f32).round() as usize).clamp(1, sample_rate.max(1) as usize)
}

/// One-pole lowpass coefficient for a cutoff mapped logarithmically from param (20 Hz .. 20 kHz).
fn filter_coeff(param: i32, sample_rate: u32) -> f32 {
    let fc = 20.0_f32 * 1000.0_f32.powf(norm(param));
    let a = (-std::f32::consts::TAU * fc / sample_rate.max(1) as f32).exp();
    1.0 - a
}

/// Chorus LFO rate in Hz: param maps to 0.1..5 Hz.
fn chorus_rate(param: i32) -> f32 {
    0.1 + norm(param) * 4.9
}

/// Stereo parallel layer.
pub struct Layer {
    sample_rate: u32,
    settings: Settings,
    /// Shared delay line, interleaved L/R, 1 second per channel.
    delay_buf: Vec<f32>,
    write_pos: usize,
    /// One-pole filter state per slot, per channel.
    filter_state: [[f32; 2]; SLOT_COUNT],
    /// Chorus LFO phase per slot.
    chorus_phase: [f32; SLOT_COUNT],
}

impl Layer {
    pub fn with_settings(sample_rate: u32, settings: &Settings) -> Self {
        let mut l = Layer {
            sample_rate: sample_rate.max(1),
            settings: *settings,
            delay_buf: Vec::new(),
            write_pos: 0,
            filter_state: [[0.0; 2]; SLOT_COUNT],
            chorus_phase: [0.0; SLOT_COUNT],
        };
        l.resize();
        l
    }

    fn resize(&mut self) {
        let len = (self.sample_rate as usize).max(1);
        self.delay_buf = vec![0.0; len * 2];
        self.write_pos = 0;
        self.filter_state = [[0.0; 2]; SLOT_COUNT];
        self.chorus_phase = [0.0; SLOT_COUNT];
    }

    pub fn set_sample_rate(&mut self, rate: u32) {
        let rate = rate.max(1);
        if rate != self.sample_rate {
            self.sample_rate = rate;
            self.resize();
        }
    }

    pub fn set(&mut self, settings: &Settings) {
        self.settings = *settings;
    }

    pub fn reset(&mut self) {
        for v in &mut self.delay_buf {
            *v = 0.0;
        }
        self.write_pos = 0;
        self.filter_state = [[0.0; 2]; SLOT_COUNT];
        self.chorus_phase = [0.0; SLOT_COUNT];
    }

    fn delay_len(&self) -> usize {
        (self.delay_buf.len() / 2).max(1)
    }

    fn read_delay(&self, offset: usize, channel: usize) -> f32 {
        let len = self.delay_len();
        let off = offset.clamp(1, len - 1);
        let pos = (self.write_pos + len - off) % len;
        self.delay_buf[pos * 2 + channel]
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Process one stereo frame in place: out = dry*in + wet*sum(active slots).
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len());
        let rate = self.sample_rate;
        for i in 0..n {
            let in_l = left[i];
            let in_r = right[i];

            // Slot contributions read from the shared delay line (history only).
            let mut wet_l = 0.0f32;
            let mut wet_r = 0.0f32;
            let mut fb_l = 0.0f32;
            let mut fb_r = 0.0f32;
            let mut has_delay = false;

            for (si, slot) in self.settings.slots.iter().enumerate() {
                if !slot.active {
                    continue;
                }
                let level = norm(slot.level);
                let (o_l, o_r) = match slot.ty {
                    TY_BYPASS => (in_l, in_r),
                    TY_FILTER => {
                        let a = filter_coeff(slot.param, rate);
                        let st = &mut self.filter_state[si];
                        st[0] += a * (in_l - st[0]);
                        st[1] += a * (in_r - st[1]);
                        (st[0], st[1])
                    }
                    TY_DELAY => {
                        let off = delay_time_samples(slot.param, rate);
                        let r_l = self.read_delay(off, 0);
                        let r_r = self.read_delay(off, 1);
                        if !has_delay {
                            fb_l = r_l * delay_feedback();
                            fb_r = r_r * delay_feedback();
                            has_delay = true;
                        }
                        (r_l, r_r)
                    }
                    TY_CHORUS => {
                        let rate_hz = chorus_rate(slot.param);
                        self.chorus_phase[si] += std::f32::consts::TAU * rate_hz / rate as f32;
                        let lfo = self.chorus_phase[si].sin();
                        let base_ms = 12.0 + lfo * 4.0; // 8..16 ms modulated delay
                        let off = ((base_ms * 0.001 * rate as f32).round() as usize).clamp(1, self.delay_len() - 1);
                        (self.read_delay(off, 0), self.read_delay(off, 1))
                    }
                    TY_DRIVE => {
                        let gain = 1.0 + norm(slot.param) * 9.0;
                        ((in_l * gain).tanh(), (in_r * gain).tanh())
                    }
                    _ => (in_l, in_r),
                };
                wet_l += o_l * level;
                wet_r += o_r * level;
            }

            // Write this frame (plus delay feedback) into the shared delay line.
            let wp = self.write_pos;
            self.delay_buf[wp * 2] = in_l + fb_l;
            self.delay_buf[wp * 2 + 1] = in_r + fb_r;
            self.write_pos = (wp + 1) % self.delay_len();

            let dry = norm(self.settings.dry);
            let wet = norm(self.settings.wet);
            left[i] = in_l * dry + wet_l * wet;
            right[i] = in_r * dry + wet_r * wet;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::SlotSettings;

    fn layer_with(slots: [SlotSettings; SLOT_COUNT], dry: i32, wet: i32) -> Layer {
        Layer::with_settings(44100, &Settings { dry, wet, slots })
    }

    fn inactive() -> [SlotSettings; SLOT_COUNT] {
        [SlotSettings::default(); SLOT_COUNT]
    }

    #[test]
    fn bypass_passthrough() {
        // No active slots: output is the dry input.
        let orig: Vec<f32> = (0..4410).map(|i| 0.5 * (i as f32 * 0.05).sin()).collect();
        let mut l2 = layer_with(inactive(), 6000, 6000);
        let mut lft = orig.clone();
        let mut rgt = orig.clone();
        l2.process(&mut lft, &mut rgt);
        for (a, b) in lft.iter().zip(&orig) {
            assert!((a - b).abs() < 1e-6);
        }
        for (a, b) in rgt.iter().zip(&orig) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn bypass_slot_passthrough() {
        let mut slots = inactive();
        slots[0] = SlotSettings { active: true, ty: TY_BYPASS, level: 6000, param: 0 };
        let mut l = layer_with(slots, 0, 6000);
        let orig: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.01).sin() * 0.7).collect();
        let mut lft = orig.clone();
        let mut rgt = orig.clone();
        l.process(&mut lft, &mut rgt);
        for (a, b) in lft.iter().zip(&orig) {
            assert!((a - b).abs() < 1e-5);
        }
    }

    #[test]
    fn filter_attenuates() {
        let mut slots = inactive();
        slots[0] = SlotSettings { active: true, ty: TY_FILTER, level: 6000, param: 0 }; // 20 Hz cutoff
        let mut l = layer_with(slots, 0, 6000);
        let sr = 44100usize;
        let orig: Vec<f32> = (0..sr).map(|i| (std::f32::consts::TAU * 1000.0 * i as f32 / sr as f32).sin()).collect();
        let mut lft = orig.clone();
        let mut rgt = orig.clone();
        l.process(&mut lft, &mut rgt);
        let in_rms = (orig.iter().map(|x| x * x).sum::<f32>() / orig.len() as f32).sqrt();
        // Measure the settled second half.
        let tail = &lft[sr / 2..];
        let out_rms = (tail.iter().map(|x| x * x).sum::<f32>() / tail.len() as f32).sqrt();
        assert!(out_rms < in_rms * 0.3, "out {out_rms} in {in_rms}");
    }

    #[test]
    fn delay_produces_repeat() {
        let mut slots = inactive();
        // param 3000 -> ~100 ms delay at 44100 Hz (~4410 samples)
        slots[0] = SlotSettings { active: true, ty: TY_DELAY, level: 6000, param: 3000 };
        let mut l = layer_with(slots, 0, 6000);
        let n = 12000usize;
        let mut lft = vec![0.0f32; n];
        let mut rgt = vec![0.0f32; n];
        lft[0] = 1.0;
        rgt[0] = 1.0;
        l.process(&mut lft, &mut rgt);
        // Nothing before the delay time (allow 1 sample of slack), a repeat near it.
        assert!(lft[1..4000].iter().all(|x| *x == 0.0));
        let peak = lft[4000..6000].iter().fold(0.0f32, |a, x| a.max(x.abs()));
        assert!(peak > 0.5, "peak {peak}");
        // Feedback gives a second, quieter repeat near twice the delay time.
        let peak2 = lft[8400..10500].iter().fold(0.0f32, |a, x| a.max(x.abs()));
        assert!(peak2 > 0.05 && peak2 < peak, "peak2 {peak2} peak {peak}");
    }

    #[test]
    fn chorus_and_drive_produce_output() {
        for ty in [TY_CHORUS, TY_DRIVE] {
            let mut slots = inactive();
            slots[1] = SlotSettings { active: true, ty, level: 6000, param: 3000 };
            let mut l = layer_with(slots, 0, 6000);
            let orig: Vec<f32> = (0..44100).map(|i| (std::f32::consts::TAU * 440.0 * i as f32 / 44100.0).sin() * 0.5).collect();
            let mut lft = orig.clone();
            let mut rgt = orig.clone();
            l.process(&mut lft, &mut rgt);
            let tail = &lft[22050..];
            let rms = (tail.iter().map(|x| x * x).sum::<f32>() / tail.len() as f32).sqrt();
            assert!(rms > 0.01 && rms <= 1.0, "ty {ty} rms {rms}");
            assert!(lft.iter().all(|x| x.is_finite()));
        }
    }

    #[test]
    fn delay_line_sized_by_sample_rate() {
        let l = Layer::with_settings(48000, &Settings::default());
        assert_eq!(l.delay_len(), 48000);
        assert_eq!(l.sample_rate(), 48000);
    }
}
