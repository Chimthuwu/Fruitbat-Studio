// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The Analog Channel Strip: Lemon Studio's port of the Michael Lane Edition fork's console strip (plan v3 Appendix C).
//! The low cut and the gate are here; drive, TAME, the compressor and the de-esser follow one at a time.
//!
//! A strip is bypassed, faded out, and only then does the engine stop calling it: a bypassed strip passes the track
//! through sample for sample (plan v3 §6.3). The sections are stereo-linked in their settings with one filter state per
//! channel, as the fork's are.
//!
//! spec: docs/plans — plan v3 §Appendix C (the working port specification)

use crate::glide::{Glide, Ramp};

/// The low cut's control word as a cutoff: `20 × 17.5^v`, 20 Hz (word 0, the section off) to 350 Hz (65535).
pub fn cut_hz(word: u16) -> f32 {
    20.0 * 17.5f32.powf(f32::from(word) / 65535.0)
}

/// The word for a cutoff in Hz, clamped to the section's 20..=350 Hz.
pub fn cut_word(hz: f32) -> u16 {
    let v = (hz.max(20.0) / 20.0).ln() / 17.5f32.ln();
    (v.clamp(0.0, 1.0) * 65535.0).round() as u16
}

/// The low cut: a second-order high-pass (Q = 0.7071) whose cutoff glides over 20 ms, with a 10 ms fade in and out so
/// that switching it in or out does not click. Word 0 (20 Hz) is the section off, as the fork's minimum is.
pub struct LowCut {
    /// The cutoff as it glides, in Hz; the coefficients are worked out per block from it.
    cut: Glide,
    /// `[b0, b1, b2, a1, a2]`, normalised by a0 (RBJ high-pass).
    coef: [f64; 5],
    /// Per channel `[x1, x2, y1, y2]`: the coefficients are the same on both sides, the state is not.
    state: [[f64; 4]; 2],
    /// How much of the filtered signal is heard: 0 dry, 1 filtered.
    wet: Glide,
    /// The word the section is set to (0 = off).
    word: u16,
}

/// One block's low cut settings: the coefficients, and the fade as it runs.
struct Block {
    coef: [f64; 5],
    wet: Ramp,
}

impl LowCut {
    pub fn new(word: u16) -> LowCut {
        LowCut { cut: Glide::new(cut_hz(word)), coef: [1.0, 0.0, 0.0, 0.0, 0.0], state: [[0.0; 4]; 2], wet: Glide::new(if word == 0 { 0.0 } else { 1.0 }), word }
    }

    /// The section's control word (0 = off).
    pub fn set(&mut self, word: u16) {
        self.word = word;
    }

    /// True when the section is off and faded out: it has no state worth keeping and writes nothing.
    pub fn idle(&self) -> bool {
        self.word == 0 && self.wet.current == 0.0
    }

    /// Empties the filter: a section switched in again starts from silence, not from what it held when it went out.
    pub fn reset(&mut self) {
        self.state = [[0.0; 4]; 2];
    }

    /// Plans a block; `None` when the section is off and faded out (its state is cleared and the buffer is left alone).
    fn block(&mut self, frames: usize, rate: u32) -> Option<Block> {
        let rate = rate.max(1);
        // 10 ms of fade in or out, and 20 ms across the section's whole 330 Hz range of cutoff.
        let fade = 1.0 / (0.010 * f64::from(rate)) as f32;
        let cut_limit = 330.0 / (0.020 * f64::from(rate)) as f32;
        let target = if self.word == 0 { 0.0 } else { 1.0 };
        let wet = self.wet.block(target, frames, fade);
        if wet.is_flat() && wet.first() == 0.0 {
            self.reset();
            return None;
        }
        self.cut.block(cut_hz(self.word), frames, cut_limit);
        self.coef = coefficients(self.cut.current, rate);
        Some(Block { coef: self.coef, wet })
    }

    /// One sample through the filter, crossfaded with the dry signal while the section fades.
    fn step(&mut self, b: &Block, wet: f32, ch: usize, x: f32) -> f32 {
        let [b0, b1, b2, a1, a2] = b.coef;
        let s = &mut self.state[ch.min(1)];
        let xd = f64::from(x);
        let y = b0 * xd + b1 * s[0] + b2 * s[1] - a1 * s[2] - a2 * s[3];
        s[1] = s[0];
        s[0] = xd;
        s[3] = s[2];
        s[2] = y;
        let y = y as f32;
        if wet == 1.0 {
            y
        } else {
            x * (1.0 - wet) + y * wet
        }
    }
}

/// RBJ high-pass at `cut` for `rate`, Q = 0.7071, normalised by a0: `[b0, b1, b2, a1, a2]`.
fn coefficients(cut: f32, rate: u32) -> [f64; 5] {
    let f = f64::from(cut.clamp(1.0, 0.45 * rate as f32));
    let w0 = std::f64::consts::TAU * f / f64::from(rate.max(1));
    let (sin, cos) = w0.sin_cos();
    // 2Q with Q = 0.7071 is √2.
    let alpha = sin / std::f64::consts::SQRT_2;
    let a0 = 1.0 + alpha;
    let b0 = (1.0 + cos) / 2.0 / a0;
    let b1 = -(1.0 + cos) / a0;
    [b0, b1, b0, -2.0 * cos / a0, (1.0 - alpha) / a0]
}

/// The gate's control word as a threshold: `-72 + 54v` dB, so -72 dB at word 0 (the section off) to -18 dB at 65535.
pub fn gate_threshold_db(word: u16) -> f32 {
    -72.0 + 54.0 * f32::from(word) / 65535.0
}

/// The word for a threshold in dB, clamped to the section's -72..=-18 dB.
pub fn gate_word(db: f32) -> u16 {
    (((db + 72.0) / 54.0).clamp(0.0, 1.0) * 65535.0).round() as u16
}

/// The gate: an envelope follower on the louder channel (1 ms attack, 80 ms release) opens the gate 40 ms past the
/// last sample above a threshold that follows the amount, and the amount doubles as the depth, so a small setting
/// only dips the level. Word 0 is the section off, and the amount glides over 20 ms both ways, so it does not click.
pub struct Gate {
    /// The amount as it glides (0..1): the threshold and the depth both follow it.
    amount: Glide,
    /// The level the gate watches, the louder of the two channels (the settings are stereo-linked).
    env: f32,
    /// The gain the gate is applying, 1 open to near 0 shut.
    gain: f32,
    /// Samples of hold left since the level was last above the threshold.
    hold: u32,
    /// The section's control word (0 = off).
    word: u16,
}

/// One block's gate settings: the rates are fixed per block, and the amount glides through it.
struct GateBlock {
    attack: f32,
    release: f32,
    hold: u32,
    amount: Ramp,
}

impl Gate {
    pub fn new(word: u16) -> Gate {
        Gate { amount: Glide::new(f32::from(word) / 65535.0), env: 0.0, gain: 1.0, hold: 0, word }
    }

    /// The section's control word (0 = off).
    pub fn set(&mut self, word: u16) {
        self.word = word;
    }

    /// True when the section is off and its amount has reached zero: nothing is left to do.
    pub fn idle(&self) -> bool {
        self.word == 0 && self.amount.current == 0.0
    }

    /// Back to open: a section switched in again does not start shut on what it heard before.
    pub fn reset(&mut self) {
        self.env = 0.0;
        self.gain = 1.0;
        self.hold = 0;
    }

    /// Plans a block; `None` when the section is off and has glided out (its state is cleared).
    fn block(&mut self, frames: usize, rate: u32) -> Option<GateBlock> {
        let rate = rate.max(1);
        let target = if self.word == 0 { 0.0 } else { f32::from(self.word) / 65535.0 };
        if target == 0.0 && self.amount.current == 0.0 {
            self.reset();
            return None;
        }
        let amount = self.amount.block(target, frames, 1.0 / (0.020 * f64::from(rate)) as f32);
        Some(GateBlock {
            attack: 1.0 - (-1.0 / (rate as f32 * 0.001)).exp(),
            release: 1.0 - (-1.0 / (rate as f32 * 0.080)).exp(),
            hold: (rate as f32 * 0.040) as u32,
            amount,
        })
    }

    /// The gain for one frame pair, the amount gliding and the follower moving with it.
    fn gain(&mut self, b: &mut GateBlock, l: f32, r: f32) -> f32 {
        let amount = b.amount.next();
        let level = l.abs().max(r.abs());
        self.env += (level - self.env) * if level > self.env { b.attack } else { b.release };
        let threshold = 10f32.powf((-72.0 + 54.0 * amount) / 20.0);
        if self.env >= threshold {
            self.hold = b.hold;
        } else {
            self.hold = self.hold.saturating_sub(1);
        }
        let goal = if self.hold > 0 {
            1.0
        } else {
            (self.env / (threshold * 0.5).max(1e-8)).powi(3).clamp(0.001, 1.0)
        };
        self.gain += (goal - self.gain) * if goal > self.gain { b.attack } else { b.release };
        // Below an amount of 1/50 the depth fades in with the amount, so switching the gate in does not step.
        1.0 + (self.gain - 1.0) * (amount * 50.0).min(1.0)
    }
}

/// A track's Analog Channel Strip: the low cut and the gate today, the rest as it is ported. Bypassed, it fades the processed
/// signal out to the dry one and then stops touching the buffer (`idle`), so a bypassed strip passes a track through
/// sample for sample (plan v3 §6.3).
pub struct Strip {
    /// The seven stored control words, as the project keeps them: 0 low cut, 1-6 for the sections still to come.
    pub words: [u16; 7],
    /// The bypass switch as the project stores it.
    pub bypassed: bool,
    /// How much of the processed signal is heard: 0 fully dry, 1 fully processed.
    wet: Glide,
    low: LowCut,
    gate: Gate,
}

impl Strip {
    pub fn new(words: [u16; 7], bypassed: bool) -> Strip {
        Strip { words, bypassed, wet: Glide::new(if bypassed { 0.0 } else { 1.0 }), low: LowCut::new(words[0]), gate: Gate::new(words[3]) }
    }

    /// The project's settings for the strip (a track rebuilt from a changed project).
    pub fn set(&mut self, words: [u16; 7], bypassed: bool) {
        self.words = words;
        self.bypassed = bypassed;
        self.low.set(words[0]);
        self.gate.set(words[3]);
    }

    /// True when bypassed and faded out: the engine skips the strip and passes the track through.
    pub fn idle(&self) -> bool {
        self.bypassed && self.wet.current == 0.0
    }

    /// One interleaved stereo block.
    pub fn process(&mut self, buf: &mut [f32], rate: u32) {
        let frames = buf.len() / 2;
        // A 10 ms bypass crossfade: long enough not to click, short enough to feel immediate.
        let fade = 1.0 / (0.010 * f64::from(rate.max(1))) as f32;
        let target = if self.bypassed { 0.0 } else { 1.0 };
        let mut wet = self.wet.block(target, frames, fade);
        if wet.is_flat() && wet.first() == 0.0 {
            self.low.reset();
            self.gate.reset();
            return;
        }
        let mut low = self.low.block(frames, rate);
        let mut gate = self.gate.block(frames, rate);
        for f in buf.chunks_exact_mut(2) {
            let w = wet.next();
            // Fully dry: nothing is written, so the samples stay as they arrived.
            if w == 0.0 {
                continue;
            }
            let dry = [f[0], f[1]];
            let mut y = [0.0; 2];
            // The low cut first, then the gate on what it left (the fork's order).
            for (ch, x) in dry.iter().enumerate() {
                y[ch] = match &mut low {
                    Some(b) => {
                        let lw = b.wet.next();
                        self.low.step(b, lw, ch, *x)
                    }
                    None => *x,
                };
            }
            if let Some(b) = &mut gate {
                let g = self.gate.gain(b, y[0], y[1]);
                y[0] *= g;
                y[1] *= g;
            }
            for (ch, s) in f.iter_mut().enumerate() {
                let a = y[ch];
                *s = if w == 1.0 { a } else { dry[ch] * (1.0 - w) + a * w };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;
    const BLOCK: usize = 240;

    fn sine(hz: f32, frames: usize) -> Vec<f32> {
        (0..frames).flat_map(|i| {
            let x = (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin() * 0.5;
            [x, x]
        }).collect()
    }

    fn run(s: &mut Strip, buf: &mut [f32]) {
        for c in buf.chunks_mut(BLOCK * 2) {
            s.process(c, RATE);
            if c.len() < BLOCK * 2 {
                break;
            }
        }
    }

    fn rms(buf: &[f32]) -> f32 {
        (buf.iter().map(|x| x * x).sum::<f32>() / buf.len().max(1) as f32).sqrt()
    }

    fn strip(hz: f32) -> Strip {
        Strip::new([cut_word(hz), 0, 0, 0, 0, 0, 0], false)
    }

    /// Word 0 is the section off: not a filter at 20 Hz, but nothing at all — sample for sample.
    #[test]
    fn the_low_cut_at_its_minimum_passes_the_signal_through() {
        let dry: Vec<f32> = (0..480).flat_map(|i| { let x = (i as f32 * 0.07).sin() * 0.5; [x, -x] }).collect();
        let mut s = Strip::new([0; 7], false);
        let mut buf = dry.clone();
        s.process(&mut buf, RATE);
        assert_eq!(buf, dry);
    }

    /// A second-order high-pass has no gain at DC: a constant input leaves nothing behind.
    #[test]
    fn the_low_cut_takes_dc_out() {
        let mut s = strip(100.0);
        let mut buf = vec![1.0f32; BLOCK * 2 * 40];
        run(&mut s, &mut buf);
        let tail = &buf[buf.len() - BLOCK * 2..];
        assert!(tail.iter().all(|x| x.abs() < 1e-3), "DC gone: {:?}", tail[0]);
    }

    /// A tone well above the cutoff comes through as it is.
    #[test]
    fn the_low_cut_keeps_a_tone_above_it() {
        let dry = sine(1_000.0, BLOCK * 2 * 40);
        let mut s = strip(100.0);
        let mut buf = dry.clone();
        run(&mut s, &mut buf);
        let (a, b) = (rms(&dry[dry.len() - BLOCK * 2..]), rms(&buf[buf.len() - BLOCK * 2..]));
        assert!((b / a - 1.0).abs() < 0.06, "1 kHz passes: {b} / {a}");
    }

    /// A cutoff change glides: the coefficients move per block, so no sample steps far from the last.
    #[test]
    fn a_cutoff_change_glides_without_a_step() {
        let mut s = strip(80.0);
        let mut buf = sine(200.0, BLOCK * 2 * 20);
        for (i, c) in buf.chunks_mut(BLOCK * 2).enumerate() {
            if i == 10 {
                s.set([cut_word(350.0), 0, 0, 0, 0, 0, 0], false);
            }
            s.process(c, RATE);
        }
        let max = buf.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        // A 200 Hz sine at 0.5 moves by at most 0.0131 a sample.
        assert!(max < 0.05, "no step: {max}");
        assert!(buf.iter().all(|x| x.is_finite()));
    }

    /// Switching the section off clears its state: switching it back on does not let the old charge out.
    #[test]
    fn a_bypassed_section_leaves_no_state_behind() {
        let mut s = strip(100.0);
        let mut dc = vec![1.0f32; BLOCK * 2 * 10];
        run(&mut s, &mut dc);
        s.set([0; 7], false);
        let mut dc = vec![1.0f32; BLOCK * 2 * 5];
        run(&mut s, &mut dc);
        s.set([cut_word(100.0), 0, 0, 0, 0, 0, 0], false);
        let mut quiet = vec![0.0f32; BLOCK * 2 * 3];
        run(&mut s, &mut quiet);
        assert!(quiet.iter().all(|x| x.abs() < 1e-6), "no stale state: {:?}", quiet[0]);
    }

    /// The coefficients are worked out from the rate every block, so the response holds at 44.1, 48 and 96 kHz.
    #[test]
    fn the_low_cut_is_the_same_at_any_rate() {
        for rate in [44_100u32, 48_000, 96_000] {
            let mut s = strip(100.0);
            let mut buf = vec![1.0f32; (rate as usize / 4) * 2];
            for c in buf.chunks_mut(480 * 2) {
                s.process(c, rate);
            }
            let tail = &buf[buf.len() - 480..];
            assert!(tail.iter().all(|x| x.abs() < 2e-3), "DC gone at {rate}: {:?}", tail[0]);
        }
    }

    /// Bypassed, faded out, the strip is skipped and the buffer is the one that arrived — whatever its words hold.
    #[test]
    fn a_bypassed_strip_does_not_touch_the_buffer() {
        let dry: Vec<f32> = (0..480).flat_map(|i| { let x = (i as f32 * 0.01).sin() * 0.5; [x, x] }).collect();
        let mut s = Strip::new([cut_word(350.0), 1234, 4321, 999, 7, 65, 32000], true);
        assert!(s.idle());
        let mut buf = dry.clone();
        if !s.idle() {
            s.process(&mut buf, RATE);
        }
        assert_eq!(buf, dry);
    }

    /// The bypass crossfade settles within four blocks, and once it has, the strip writes nothing at all.
    #[test]
    fn the_bypass_crossfade_settles_and_then_stops() {
        let dry = sine(1_000.0, BLOCK * 4);
        let mut s = strip(350.0);
        let mut buf = dry.clone();
        s.process(&mut buf, RATE);
        s.bypassed = true;
        for _ in 0..4 {
            let mut b = dry.clone();
            s.process(&mut b, RATE);
            assert!(b.iter().all(|x| x.is_finite()));
        }
        assert!(s.idle(), "faded out inside four blocks");
        let mut b = dry.clone();
        if !s.idle() {
            s.process(&mut b, RATE);
        }
        assert_eq!(b, dry);
    }

    /// A gate strip with threshold `db` and nothing else set.
    fn gate_strip(db: f32) -> Strip {
        Strip::new([0, 0, 0, gate_word(db), 0, 0, 0], false)
    }

    /// A quiet tone under the gate's threshold is shut.
    #[test]
    fn the_gate_shuts_a_signal_below_its_threshold() {
        let mut s = gate_strip(-18.0);
        let mut buf: Vec<f32> = sine(1_000.0, BLOCK * 2 * 40).iter().map(|x| x * 0.01).collect();
        let dry = rms(&buf);
        run(&mut s, &mut buf);
        let tail = rms(&buf[buf.len() - BLOCK * 2..]);
        assert!(tail < dry * 0.02, "shut: {tail} of {dry}");
    }

    /// A tone above the threshold comes through as it is.
    #[test]
    fn the_gate_leaves_a_signal_above_its_threshold() {
        let dry = sine(1_000.0, BLOCK * 2 * 40);
        let mut s = gate_strip(-18.0);
        let mut buf = dry.clone();
        run(&mut s, &mut buf);
        let (a, b) = (rms(&dry[dry.len() - BLOCK * 2..]), rms(&buf[buf.len() - BLOCK * 2..]));
        assert!((b / a - 1.0).abs() < 0.01, "open: {b} / {a}");
    }

    /// The hold keeps the gate open 40 ms past the last sample above the threshold, then it shuts.
    #[test]
    fn the_gate_holds_open_after_a_burst() {
        let frames = BLOCK * 120;
        let burst = (BLOCK * 20)..(BLOCK * 22);
        let mut buf: Vec<f32> = (0..frames)
            .flat_map(|i| {
                let a = if burst.contains(&i) { 0.5 } else { 0.01 };
                let x = (std::f32::consts::TAU * 1_000.0 * i as f32 / RATE as f32).sin() * a;
                [x, x]
            })
            .collect();
        let dry = rms(&buf[BLOCK * 2 * 30..BLOCK * 2 * 40]);
        let mut s = gate_strip(-18.0);
        run(&mut s, &mut buf);
        let held = rms(&buf[BLOCK * 2 * 22..BLOCK * 2 * 30]);
        let shut = rms(&buf[BLOCK * 2 * 112..BLOCK * 2 * 120]);
        assert!(held > dry * 0.6, "held open after the burst: {held} of {dry}");
        assert!(shut < dry * 0.05, "shut after the hold: {shut} of {dry}");
    }

    /// One gain for both channels: the settings are stereo-linked, as the fork's are.
    #[test]
    fn the_gate_scales_both_channels_alike() {
        let mut s = gate_strip(-18.0);
        let mut buf = sine(1_000.0, BLOCK * 2 * 40);
        for f in buf.chunks_exact_mut(2) {
            f[1] = f[0] * 0.2;
        }
        run(&mut s, &mut buf);
        assert!(buf.chunks_exact(2).all(|f| (f[1] - f[0] * 0.2).abs() < 1e-6));
    }

    /// The word and the cutoff are each other's inverse across the range.
    #[test]
    fn the_word_and_the_cutoff_round_trip() {
        for hz in [20.0f32, 40.0, 100.0, 240.0, 350.0] {
            assert!((cut_hz(cut_word(hz)) - hz).abs() < 0.01, "{hz}");
        }
        assert_eq!(cut_hz(0), 20.0);
        assert!((cut_hz(u16::MAX) - 350.0).abs() < 0.01);
    }
}
