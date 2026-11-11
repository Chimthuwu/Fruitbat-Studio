// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The Analog Channel Strip: Lemon Studio's port of the Michael Lane Edition fork's console strip (plan v3 Appendix C).
//! The low cut, the gate, the drive, TAME, the compressor and the de-esser are here, in the fork's order.
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

/// The drive's four characters, as the fork names them.
pub const DRIVE_TYPES: [&str; 4] = ["Tube", "Tape", "Soft", "Clip"];

/// The character control word 6 stands for: 0 Tube, 1 Tape, 2 Soft, 3 Clip.
pub fn drive_kind(word: u16) -> usize {
    ((f32::from(word) / 65535.0) * 3.0).round().clamp(0.0, 3.0) as usize
}

/// The word for a character index (`DRIVE_TYPES` order).
pub fn drive_kind_word(kind: usize) -> u16 {
    ((kind.min(3) as f32 / 3.0) * 65535.0).round() as u16
}

/// The drive's control word as an amount: word 0 is the section off, 65535 is all the way in.
pub fn drive_amount(word: u16) -> f32 {
    f32::from(word) / 65535.0
}

/// The word for an amount 0..=1.
pub fn drive_word(amount: f32) -> u16 {
    (amount.clamp(0.0, 1.0) * 65535.0).round() as u16
}

/// The shaped value for one sample of one character.
fn curve(kind: usize, x: f32) -> f32 {
    match kind {
        1 => x / (1.0 + x.abs()),
        3 => x.clamp(-1.0, 1.0),
        _ => x.tanh(),
    }
}

/// The integral of `curve` from 0 to `x`: the shape is the curve's mean over each sample's step, which keeps what a
/// plain waveshaper folds back down without oversampling.
fn integral(kind: usize, x: f32) -> f64 {
    let a = f64::from(x).abs();
    match kind {
        1 => a - a.ln_1p(),
        3 => {
            if a <= 1.0 {
                0.5 * a * a
            } else {
                a - 0.5
            }
        }
        _ => a + (-2.0 * a).exp().ln_1p() - std::f64::consts::LN_2,
    }
}

/// The drive: a waveshaping character in front of the rest of the strip, then an 8 Hz DC blocker (the tube character is
/// asymmetric, so without it a track would pick up a floor), all faded in by the amount. The amount glides over 20 ms and
/// doubles as the drive, and the four characters crossfade over 20 ms when the character changes. Word 2 is the amount
/// (0 = off), word 6 the character.
pub struct Drive {
    /// The amount as it glides (0..1), 20 ms either way.
    amount: Glide,
    /// How much of each character is heard: they crossfade, and their weights always add up to one.
    mix: [Glide; 4],
    /// Per channel, the driven sample of the last frame: the shape is averaged from it to the current one.
    last: [f32; 2],
    /// Per channel `[x1, y1]` of the DC blocker.
    dc: [[f32; 2]; 2],
    /// The amount's control word (0 = off) and the character's.
    word: u16,
    kind: u16,
}

/// One block's drive settings: the character is fixed within it, and the amount glides through it.
struct DriveBlock {
    amount: Ramp,
    mix: [Ramp; 4],
    /// The DC blocker's pole for 8 Hz at this rate.
    pole: f32,
    /// This frame's amount and weights, moved once and used for both channels.
    a: f32,
    w: [f32; 4],
}

impl Drive {
    pub fn new(word: u16, kind: u16) -> Drive {
        let k = drive_kind(kind);
        Drive {
            amount: Glide::new(drive_amount(word)),
            mix: std::array::from_fn(|i| Glide::new(if i == k { 1.0 } else { 0.0 })),
            last: [0.0; 2],
            dc: [[0.0; 2]; 2],
            word,
            kind,
        }
    }

    /// The section's control words: the amount (0 = off) and the character.
    pub fn set(&mut self, word: u16, kind: u16) {
        self.word = word;
        self.kind = kind;
    }

    /// True when the amount is at zero: nothing is left to shape.
    pub fn idle(&self) -> bool {
        self.word == 0 && self.amount.current == 0.0
    }

    /// Empties the shaper's memory: a section switched in again starts from silence, not from where it was.
    pub fn reset(&mut self) {
        self.last = [0.0; 2];
        self.dc = [[0.0; 2]; 2];
    }

    /// Plans a block; `None` when the section is off and faded out (its state is cleared).
    fn block(&mut self, frames: usize, rate: u32) -> Option<DriveBlock> {
        let rate = rate.max(1);
        let glide = 1.0 / (0.020 * f64::from(rate)) as f32;
        let target = drive_amount(self.word);
        let k = drive_kind(self.kind);
        if target == 0.0 && self.amount.current == 0.0 {
            // Off: the character jumps to where it is set, and nothing is held.
            self.reset();
            for (i, m) in self.mix.iter_mut().enumerate() {
                m.current = if i == k { 1.0 } else { 0.0 };
            }
            return None;
        }
        let amount = self.amount.block(target, frames, glide);
        let mix = std::array::from_fn(|i| self.mix[i].block(if i == k { 1.0 } else { 0.0 }, frames, glide));
        Some(DriveBlock { amount, mix, pole: (-std::f32::consts::TAU * 8.0 / rate as f32).exp(), a: 0.0, w: [0.0; 4] })
    }

    /// One frame's settings: the amount and the character weights move once, and both channels use them.
    fn frame(b: &mut DriveBlock) {
        b.a = b.amount.next();
        for (w, m) in b.w.iter_mut().zip(b.mix.iter_mut()) {
            *w = m.next();
        }
    }

    /// One channel through the shaper and the DC blocker, crossfaded with the dry sample by the amount.
    fn step(&mut self, b: &DriveBlock, ch: usize, x: f32) -> f32 {
        let ch = ch.min(1);
        let amount = b.a;
        let drive = 1.0 + 7.0 * amount;
        let v = (x * drive).clamp(-40.0, 40.0);
        let last = std::mem::replace(&mut self.last[ch], v);
        let mut shaped = 0.0;
        for kind in 0..4 {
            let w = b.w[kind];
            if w <= 0.0 {
                continue;
            }
            let bias = if kind == 0 { 0.12 * amount } else { 0.0 };
            let (a, z) = (v + bias, last + bias);
            let sat = if (a - z).abs() > 1e-4 {
                ((integral(kind, a) - integral(kind, z)) / f64::from(a - z)) as f32
            } else {
                curve(kind, (a + z) * 0.5)
            };
            // The small-signal slope is compensated, as the fork's PRE did: a quiet track keeps its level and a loud one
            // rounds off. This is a fixed compensation, not a loudness-following gain rider.
            let slope = if kind == 0 { 1.0 - bias.tanh().powi(2) } else { 1.0 };
            shaped += w * (sat - curve(kind, bias)) / (drive * slope);
        }
        let [x1, y1] = self.dc[ch];
        let y = shaped - x1 + b.pole * y1;
        self.dc[ch] = [shaped, y];
        x + (y - x) * amount
    }
}

/// TAME's bands: the fork's 72, spread logarithmically from 100 Hz to 18 kHz (or 40 % of the rate, below 45 kHz).
const BANDS: usize = 72;

/// How often TAME compares its bands: every 32 frames.
const HOP: u32 = 32;

/// TAME's control word as an amount: word 0 is the section off, 65535 is all the way in.
pub fn tame_amount(word: u16) -> f32 {
    f32::from(word) / 65535.0
}

/// The word for an amount 0..=1.
pub fn tame_word(amount: f32) -> u16 {
    (amount.clamp(0.0, 1.0) * 65535.0).round() as u16
}

/// One of TAME's bands: a constant-Q band-pass (Q = 10) over the signal and over its own detector, with a bounded
/// notch that follows how far the band stands above its neighbours.
#[derive(Clone, Copy)]
struct Band {
    b0: f32,
    a1: f32,
    a2: f32,
    /// How much the band-pass lifts the band: the detector is normalised by it, so a broad slope is not a resonance.
    normalization: f32,
    /// The detector's filter state, per channel `[z0, z1]`.
    detector: [[f32; 2]; 2],
    /// The audio filter's state, kept apart so the detector keeps hearing the unprocessed signal.
    audio: [[f32; 2]; 2],
    /// The band's energy as the detector smooths it, and the notch's depth as it glides to its goal (0..1).
    power: f32,
    depth: f32,
    goal: f32,
}

impl Band {
    fn silent() -> Band {
        Band { b0: 0.0, a1: 0.0, a2: 0.0, normalization: 1.0, detector: [[0.0; 2]; 2], audio: [[0.0; 2]; 2], power: 0.0, depth: 0.0, goal: 0.0 }
    }

    /// Places band `i` of `BANDS` between 100 Hz and 18 kHz for this rate.
    fn tune(&mut self, i: usize, rate: f32) {
        let high = 18_000.0f32.min(rate * 0.40);
        let low = 100.0f32.min(high / 10.0);
        let hz = low * (high / low).powf(i as f32 / (BANDS - 1) as f32);
        let w = std::f32::consts::TAU * hz / rate;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / 20.0;
        self.b0 = alpha / (1.0 + alpha);
        self.a1 = -2.0 * cos / (1.0 + alpha);
        self.a2 = (1.0 - alpha) / (1.0 + alpha);
        self.normalization = (1.0 + alpha) / alpha.max(1e-10);
    }

    fn reset(&mut self) {
        self.detector = [[0.0; 2]; 2];
        self.audio = [[0.0; 2]; 2];
        self.power = 0.0;
        self.depth = 0.0;
        self.goal = 0.0;
    }

    /// The band-pass at this band's frequency: the detector's state, or the audio's.
    fn filter(&mut self, input: [f32; 2], audio: bool) -> [f32; 2] {
        let states = if audio { &mut self.audio } else { &mut self.detector };
        std::array::from_fn(|c| {
            let z = &mut states[c];
            let y = self.b0 * input[c] + z[0];
            z[0] = -self.a1 * y + z[1];
            z[1] = -self.b0 * input[c] - self.a2 * y;
            // A silent tail must not leave denormals running through 72 bands.
            for v in z.iter_mut() {
                if v.abs() < 1e-20 {
                    *v = 0.0;
                }
            }
            y
        })
    }
}

/// TAME: the fork's resonance suppressor, a bank of 72 dynamic notches in series. No FFT, no look-ahead, and nothing on
/// the audio thread allocates. The detectors hear the unprocessed stereo signal, both sides are linked, and every notch is
/// bounded, so a cluster of harmonics cannot be hollowed out. It looks at its bands every 32 frames and glides the notches
/// over 8 ms in and 120 ms out. Word 4 is the amount (0 = off).
pub struct Tame {
    bands: [Band; BANDS],
    /// The rate the bands are tuned for; the first block, or a change of rate, tunes them again.
    rate: u32,
    /// The amount as it glides (0..1), 20 ms either way.
    amount: Glide,
    /// The broadband energy, fast and slow: their ratio is how much the signal is a transient rather than a tone.
    fast: f32,
    slow: f32,
    /// Frames since the bands were last compared.
    tick: u32,
    /// The amount's control word (0 = off).
    word: u16,
}

/// One block's TAME settings: the amount glides through it, and the smoothing is fixed at this rate.
struct TameBlock {
    amount: Ramp,
    /// 12 ms of energy smoothing per band, and the 8 ms / 120 ms the notches move in and out by.
    detector: f32,
    attack: f32,
    release: f32,
    /// 1 ms and 30 ms of broadband energy: how much of a peak is a hit rather than a tone.
    fast: f32,
    slow: f32,
    /// This frame's amount, moved once and used for both channels.
    a: f32,
}

impl Tame {
    pub fn new(word: u16) -> Tame {
        Tame { bands: std::array::from_fn(|_| Band::silent()), rate: 0, amount: Glide::new(tame_amount(word)), fast: 0.0, slow: 0.0, tick: 0, word }
    }

    /// The section's control word (0 = off).
    pub fn set(&mut self, word: u16) {
        self.word = word;
    }

    /// True when the section is off and faded out: it has no state worth keeping.
    pub fn idle(&self) -> bool {
        self.word == 0 && self.amount.current == 0.0
    }

    /// Empties the bands and the detectors: a section switched in again starts from silence, not from where it was.
    pub fn reset(&mut self) {
        for b in &mut self.bands {
            b.reset();
        }
        self.fast = 0.0;
        self.slow = 0.0;
        self.tick = 0;
    }

    /// Plans a block; `None` when the section is off and faded out (its state is cleared).
    fn block(&mut self, frames: usize, rate: u32) -> Option<TameBlock> {
        let rate = rate.max(1);
        if self.rate != rate {
            self.rate = rate;
            let ratef = rate as f32;
            for (i, b) in self.bands.iter_mut().enumerate() {
                b.tune(i, ratef);
                b.reset();
            }
        }
        let target = tame_amount(self.word);
        if target == 0.0 && self.amount.current == 0.0 {
            self.reset();
            return None;
        }
        let ratef = rate as f32;
        let coefficient = |seconds: f32| 1.0 - (-1.0 / (ratef * seconds)).exp();
        let amount = self.amount.block(target, frames, 1.0 / (0.020 * ratef));
        Some(TameBlock {
            amount,
            detector: coefficient(0.012),
            attack: coefficient(0.008),
            release: coefficient(0.120),
            fast: coefficient(0.001),
            slow: coefficient(0.030),
            a: 0.0,
        })
    }

    /// One stereo frame: both sides through the linked bank, so the image holds.
    fn frame(&mut self, b: &mut TameBlock, input: [f32; 2]) -> [f32; 2] {
        b.a = b.amount.next();
        let amount = b.a;
        let power = (input[0] * input[0] + input[1] * input[1]) * 0.5;
        self.fast += (power - self.fast) * b.fast;
        self.slow += (power - self.slow) * b.slow;
        for band in &mut self.bands {
            let y = band.filter(input, false);
            let energy = (y[0] * y[0] + y[1] * y[1]) * 0.5;
            band.power += (energy - band.power) * b.detector;
        }
        // Every 32 frames the bands are compared: a band that stands above its neighbours is a resonance.
        if self.tick == 0 {
            self.detect(amount);
        }
        self.tick = (self.tick + 1) % HOP;
        let mut out = input;
        for band in &mut self.bands {
            band.depth += (band.goal - band.depth) * if band.goal > band.depth { b.attack } else { b.release };
            // The notch is subtracted from what the bands before it left, so the sum of them stays bounded.
            let n = band.filter(out, true);
            for c in 0..2 {
                out[c] -= n[c] * band.depth * amount;
            }
        }
        out
    }

    /// Gives each band its notch depth from how far it stands above its neighbours, in dB.
    fn detect(&mut self, amount: f32) {
        // Each detector is normalised by its band-pass's noise gain, so a broad spectral slope does not read as a peak.
        let levels = self.bands.map(|b| 10.0 * (b.power * b.normalization + 1e-20).log10());
        let mut cuts = [0.0f32; BANDS];
        // A hit is sharper than the broadband rise behind it: hold back while the signal is a transient.
        let protection = (4.0 * self.slow / (self.fast + 1e-20)).min(1.0);
        for i in 4..BANDS - 4 {
            let left = (levels[i - 4] + levels[i - 3]) * 0.5;
            let right = (levels[i + 3] + levels[i + 4]) * 0.5;
            let contrast = levels[i] - (left + right) * 0.5;
            // A stronger amount detects subtler peaks: the threshold falls from 5 dB to 2 dB.
            let over = (contrast - (5.0 - 3.0 * amount)).max(0.0);
            cuts[i] = (over * over / (over + 2.0) * 1.8).min(18.0) * protection;
        }
        for i in 0..BANDS {
            // Neighbouring notches share a budget, so a cluster of harmonics is not cut away.
            let total: f32 = cuts[i.saturating_sub(2)..(i + 3).min(BANDS)].iter().sum();
            let budget = 12.0 + 12.0 * amount;
            let db = cuts[i] * (budget / total.max(budget));
            self.bands[i].goal = 1.0 - 10f32.powf(-db / 20.0);
        }
    }
}

/// A compressor's settings, as the fork tables them per material.
pub struct Profile {
    pub name: &'static str,
    /// How fast the level follower and the gain move toward more reduction, and how fast they come back.
    pub attack: f32,
    pub release: f32,
    pub ratio: f32,
    /// How far the threshold at -6 dB falls as the amount goes from 0 to 1.
    pub range: f32,
}

/// The six profiles, in the order word 5 counts them.
pub const PROFILES: [Profile; 6] = [
    Profile { name: "General", attack: 0.010, release: 0.150, ratio: 2.0, range: 30.0 },
    Profile { name: "Vocal", attack: 0.008, release: 0.100, ratio: 2.0, range: 28.0 },
    Profile { name: "Guitar", attack: 0.015, release: 0.120, ratio: 2.0, range: 26.0 },
    Profile { name: "Bass", attack: 0.025, release: 0.200, ratio: 2.5, range: 30.0 },
    Profile { name: "Synthesizer", attack: 0.012, release: 0.180, ratio: 1.5, range: 26.0 },
    Profile { name: "Drums", attack: 0.025, release: 0.080, ratio: 2.5, range: 24.0 },
];

/// The profile control word 5 stands for: 0 General to 5 Drums.
pub fn comp_profile(word: u16) -> usize {
    ((f32::from(word) / 65535.0) * 5.0).round().clamp(0.0, 5.0) as usize
}

/// The word for a profile index (`PROFILES` order).
pub fn comp_profile_word(profile: usize) -> u16 {
    ((profile.min(5) as f32 / 5.0) * 65535.0).round() as u16
}

/// The compressor's control word as an amount: word 0 is the section off, 65535 is all the way in.
pub fn comp_amount(word: u16) -> f32 {
    f32::from(word) / 65535.0
}

/// The word for an amount 0..=1.
pub fn comp_word(amount: f32) -> u16 {
    (amount.clamp(0.0, 1.0) * 65535.0).round() as u16
}

/// The compressor: a level follower on the mean power of both channels (the settings are stereo-linked), a -6 dB
/// threshold that falls with the amount, a ratio that widens with it, and 3 dB of soft knee either side. The makeup is
/// worked out at a fixed -18 dB reference rather than from what is playing, so it does not chase silence or pump gain
/// between phrases. Word 1 is the amount (0 = off) and word 5 the profile.
pub struct Compressor {
    /// The amount as it glides (0..1), 20 ms either way.
    amount: Glide,
    /// The level it follows, as mean power over both channels.
    env: f32,
    /// The gain it is applying: 1 passes the signal, below 1 takes it down.
    gain: f32,
    /// The amount's control word (0 = off) and the profile's.
    word: u16,
    profile: u16,
}

/// One block's compressor settings: the profile is fixed within it, and the amount glides through it.
struct CompBlock {
    amount: Ramp,
    /// The follower's and the gain's retention coefficients: closer to 1 is slower.
    attack: f32,
    release: f32,
    profile: usize,
    /// This frame's amount, moved once and used for both channels.
    a: f32,
}

impl Compressor {
    pub fn new(word: u16, profile: u16) -> Compressor {
        Compressor { amount: Glide::new(comp_amount(word)), env: 0.0, gain: 1.0, word, profile }
    }

    /// The section's control words: the amount (0 = off) and the profile.
    pub fn set(&mut self, word: u16, profile: u16) {
        self.word = word;
        self.profile = profile;
    }

    /// True when the amount is at zero: there is nothing left to take down.
    pub fn idle(&self) -> bool {
        self.word == 0 && self.amount.current == 0.0
    }

    /// Back to unity: a section switched in again does not start on the gain it was holding.
    pub fn reset(&mut self) {
        self.env = 0.0;
        self.gain = 1.0;
    }

    /// Plans a block; `None` when the section is off and has glided out (its state is cleared).
    fn block(&mut self, frames: usize, rate: u32) -> Option<CompBlock> {
        let rate = rate.max(1);
        let target = comp_amount(self.word);
        if target == 0.0 && self.amount.current == 0.0 {
            self.reset();
            return None;
        }
        let ratef = rate as f32;
        let profile = comp_profile(self.profile);
        let p = &PROFILES[profile];
        let amount = self.amount.block(target, frames, 1.0 / (0.020 * ratef));
        Some(CompBlock {
            amount,
            attack: (-1.0 / (ratef * p.attack)).exp(),
            release: (-1.0 / (ratef * p.release)).exp(),
            profile,
            a: 0.0,
        })
    }

    /// The gain for one frame pair, the amount gliding and the follower moving with it.
    fn gain(&mut self, b: &mut CompBlock, l: f32, r: f32) -> f32 {
        b.a = b.amount.next();
        let amount = b.a;
        let p = &PROFILES[b.profile];
        let power = (l * l + r * r) * 0.5;
        self.env = b.attack * self.env + (1.0 - b.attack) * power;
        let threshold = -6.0 - p.range * amount;
        let ratio = p.ratio + 2.0 * amount;
        // 0 dB below the knee, the full slope above it, a quadratic join across it.
        let reduction = |db: f32| {
            let over = db - threshold;
            let slope = 1.0 - 1.0 / ratio;
            if over <= -3.0 {
                0.0
            } else if over >= 3.0 {
                over * slope
            } else {
                (over + 3.0).powi(2) * slope / 12.0
            }
        };
        let db = 10.0 * self.env.max(1e-20).log10();
        // Fixed-reference automatic makeup, not one that follows the level.
        let makeup = reduction(-18.0).min(18.0);
        let wanted = 10f32.powf((makeup - reduction(db)) * amount / 20.0);
        let response = if wanted < self.gain { b.attack } else { b.release };
        self.gain = response * self.gain + (1.0 - response) * wanted;
        self.gain
    }
}

/// How many frames of edges the de-esser's detector compares.
const DEESS_HISTORY: usize = 21;

/// The furthest the de-esser takes a band down: the fork's Depth, `1 / 0.5001`.
const DEESS_DEPTH: f32 = 1.0 / 0.5001;

/// The de-esser's control word as an amount: word 0 is the section off, 65535 is all the way in.
pub fn deess_amount(word: u16) -> f32 {
    f32::from(word) / 65535.0
}

/// The word for an amount 0..=1.
pub fn deess_word(amount: f32) -> u16 {
    (amount.clamp(0.0, 1.0) * 65535.0).round() as u16
}

/// The de-esser: the fork's port of Airwindows DeBess, a detector that watches how sharp the signal's edges are and pulls
/// the sharpest ones down toward a two-sample average of the signal. It looks at the edges of both channels over the last
/// twenty frames, alternates two banks of state frame by frame so a reduction does not read as one frame of dip, and takes
/// a band no more than about 6 dB down. There is no look-ahead and no delay line in the audio path. Word 7 is the amount
/// (0 = off); the amount and the fade glide over 20 ms and 10 ms, so switching it in or out does not click.
pub struct Deess {
    /// Per channel, the last twenty frames' samples the detector measures its slopes from.
    samples: [[f32; DEESS_HISTORY]; 2],
    /// Per channel, the slopes between them.
    slopes: [[f32; DEESS_HISTORY]; 2],
    /// Per channel and per alternating bank: the two-sample average, and the gain the sharp edges are pulled down by.
    low: [[f32; 2]; 2],
    ratio: [[f32; 2]; 2],
    /// Which of the two banks this frame uses; the fork alternates them every frame.
    flip: usize,
    /// The amount as it glides (0..1), 20 ms either way.
    amount: Glide,
    /// How much of the reduced signal is heard: 0 dry, 1 reduced (the section fades in and out with it).
    wet: Glide,
    /// The amount's control word (0 = off).
    word: u16,
}

/// One block's de-esser settings: the amount and the fade glide through it, and the rate's intensity scale is fixed.
struct DeessBlock {
    amount: Ramp,
    wet: Ramp,
    /// How much a sharp edge counts at this rate: the fork's `8192 / (rate / 44100)`.
    scale: f32,
}

impl Deess {
    pub fn new(word: u16) -> Deess {
        Deess {
            samples: [[0.0; DEESS_HISTORY]; 2],
            slopes: [[0.0; DEESS_HISTORY]; 2],
            low: [[0.0; 2]; 2],
            ratio: [[1.0; 2]; 2],
            flip: 0,
            amount: Glide::new(deess_amount(word)),
            wet: Glide::new(if word == 0 { 0.0 } else { 1.0 }),
            word,
        }
    }

    /// The section's control word (0 = off).
    pub fn set(&mut self, word: u16) {
        self.word = word;
    }

    /// True when the section is off and faded out: it has no state worth keeping and writes nothing.
    pub fn idle(&self) -> bool {
        self.word == 0 && self.wet.current == 0.0
    }

    /// Empties the detector and the banks: a section switched in again starts from silence, not from where it was.
    pub fn reset(&mut self) {
        self.samples = [[0.0; DEESS_HISTORY]; 2];
        self.slopes = [[0.0; DEESS_HISTORY]; 2];
        self.low = [[0.0; 2]; 2];
        self.ratio = [[1.0; 2]; 2];
        self.flip = 0;
    }

    /// Plans a block; `None` when the section is off and has glided out (its state is cleared).
    fn block(&mut self, frames: usize, rate: u32) -> Option<DeessBlock> {
        let rate = rate.max(1);
        let ratef = rate as f32;
        let target = deess_amount(self.word);
        if target == 0.0 && self.amount.current == 0.0 {
            self.reset();
            return None;
        }
        Some(DeessBlock {
            amount: self.amount.block(target, frames, 1.0 / (0.020 * ratef)),
            wet: self.wet.block(if self.word == 0 { 0.0 } else { 1.0 }, frames, 1.0 / (0.010 * ratef)),
            scale: 8192.0 / (ratef / 44_100.0),
        })
    }

    /// One stereo frame: both channels through the same detector, the amount and the fade moving together.
    fn frame(&mut self, b: &mut DeessBlock, input: [f32; 2]) -> [f32; 2] {
        let amount = b.amount.next();
        let wet = b.wet.next();
        let intensity = amount.powi(5) * b.scale;
        let bank = self.flip;
        let mut out = input;
        for ch in 0..2 {
            let x = input[ch];
            let s = &mut self.samples[ch];
            s[0] = x;
            for i in (1..=20).rev() {
                s[i] = s[i - 1];
            }
            let m = &mut self.slopes[ch];
            m[1] = (s[1] - s[2]) * ((s[1] - s[2]) / 1.3);
            for i in (2..20).rev() {
                m[i] = (s[i] - s[i + 1]) * ((s[i - 1] - s[i]) / 1.3);
            }
            // A sharp edge is a big second difference: the sharper it is, the more the de-esser takes down.
            let mut sense = (m[1] - m[2]).abs() * 400.0;
            for i in (1..20).rev() {
                let mult = (m[i] - m[i + 1]).abs() * 400.0;
                if mult < 1.0 {
                    sense *= mult;
                }
            }
            sense = (1.0 + intensity * intensity * sense).min(intensity);
            self.low[ch][bank] = self.low[ch][bank] * 0.5 + x * 0.5;
            self.ratio[ch][bank] = (self.ratio[ch][bank] * 0.995 + sense * 0.005).min(DEESS_DEPTH);
            // The upstream gate watches bank 1 while the filter alternates: kept as it is, so the two sides agree.
            let output = if self.ratio[ch][1] > 1.0 {
                self.low[ch][bank] + (x - self.low[ch][bank]) / self.ratio[ch][bank]
            } else {
                x
            };
            out[ch] = x + (output - x) * wet;
        }
        self.flip ^= 1;
        out
    }
}

/// A track's Analog Channel Strip: the low cut, the gate, the drive, TAME, the compressor and the de-esser. Bypassed, it
/// fades the processed signal out to the dry one and then stops touching the buffer (`idle`), so a bypassed strip passes
/// a track through sample for sample (plan v3 §6.3).
pub struct Strip {
    /// The eight stored control words, as the project keeps them: 0 low cut, 1 compressor, 2 drive, 3 gate, 4 TAME,
    /// 5 compressor profile, 6 drive character, 7 de-esser.
    pub words: [u16; 8],
    /// The bypass switch as the project stores it.
    pub bypassed: bool,
    /// How much of the processed signal is heard: 0 fully dry, 1 fully processed.
    wet: Glide,
    low: LowCut,
    gate: Gate,
    drive: Drive,
    tame: Tame,
    comp: Compressor,
    deess: Deess,
}

impl Strip {
    pub fn new(words: [u16; 8], bypassed: bool) -> Strip {
        Strip { words, bypassed, wet: Glide::new(if bypassed { 0.0 } else { 1.0 }), low: LowCut::new(words[0]), gate: Gate::new(words[3]), drive: Drive::new(words[2], words[6]), tame: Tame::new(words[4]), comp: Compressor::new(words[1], words[5]), deess: Deess::new(words[7]) }
    }

    /// The project's settings for the strip (a track rebuilt from a changed project).
    pub fn set(&mut self, words: [u16; 8], bypassed: bool) {
        self.words = words;
        self.bypassed = bypassed;
        self.low.set(words[0]);
        self.gate.set(words[3]);
        self.drive.set(words[2], words[6]);
        self.tame.set(words[4]);
        self.comp.set(words[1], words[5]);
        self.deess.set(words[7]);
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
            self.drive.reset();
            self.tame.reset();
            self.comp.reset();
            self.deess.reset();
            return;
        }
        let mut low = self.low.block(frames, rate);
        let mut gate = self.gate.block(frames, rate);
        let mut drive = self.drive.block(frames, rate);
        let mut tame = self.tame.block(frames, rate);
        let mut comp = self.comp.block(frames, rate);
        let mut deess = self.deess.block(frames, rate);
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
            // Then the drive, on what the gate let through.
            if let Some(b) = &mut drive {
                Drive::frame(b);
                y[0] = self.drive.step(b, 0, y[0]);
                y[1] = self.drive.step(b, 1, y[1]);
            }
            // Then TAME, on the tone the drive left.
            if let Some(b) = &mut tame {
                y = self.tame.frame(b, y);
            }
            // Then the compressor, and the de-esser last (the fork's order).
            if let Some(b) = &mut comp {
                let g = self.comp.gain(b, y[0], y[1]);
                y[0] *= g;
                y[1] *= g;
            }
            if let Some(b) = &mut deess {
                y = self.deess.frame(b, y);
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
        Strip::new([cut_word(hz), 0, 0, 0, 0, 0, 0, 0], false)
    }

    /// Word 0 is the section off: not a filter at 20 Hz, but nothing at all — sample for sample.
    #[test]
    fn the_low_cut_at_its_minimum_passes_the_signal_through() {
        let dry: Vec<f32> = (0..480).flat_map(|i| { let x = (i as f32 * 0.07).sin() * 0.5; [x, -x] }).collect();
        let mut s = Strip::new([0; 8], false);
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
                s.set([cut_word(350.0), 0, 0, 0, 0, 0, 0, 0], false);
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
        s.set([0; 8], false);
        let mut dc = vec![1.0f32; BLOCK * 2 * 5];
        run(&mut s, &mut dc);
        s.set([cut_word(100.0), 0, 0, 0, 0, 0, 0, 0], false);
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
        let mut s = Strip::new([cut_word(350.0), 1234, 4321, 999, 7, 65, 32000, 40000], true);
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
        Strip::new([0, 0, 0, gate_word(db), 0, 0, 0, 0], false)
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

    /// The drive's words and their values round trip.
    #[test]
    fn the_drive_words_round_trip() {
        assert_eq!(drive_amount(0), 0.0);
        assert!((drive_amount(drive_word(0.5)) - 0.5).abs() < 1e-4);
        for kind in 0..4 {
            assert_eq!(drive_kind(drive_kind_word(kind)), kind);
        }
    }

    /// Word 2 is the section off, whatever character it is set to: sample for sample.
    #[test]
    fn the_drive_off_passes_the_signal_through() {
        let dry = sine(1_000.0, BLOCK * 2 * 4);
        let mut s = Strip::new([0, 0, 0, 0, 0, 0, drive_kind_word(3), 0], false);
        let mut buf = dry.clone();
        run(&mut s, &mut buf);
        assert_eq!(buf, dry);
    }

    /// Driven all the way in, the tube shape rounds a loud tone down instead of letting it through.
    #[test]
    fn the_drive_rounds_a_loud_signal_down() {
        let dry = sine(1_000.0, BLOCK * 2 * 40);
        let mut s = Strip::new([0, 0, u16::MAX, 0, 0, 0, drive_kind_word(0), 0], false);
        let mut buf = dry.clone();
        run(&mut s, &mut buf);
        let peak = |b: &[f32]| b.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let (a, b) = (peak(&dry), peak(&buf));
        assert!(b < a * 0.4, "rounded off: {b} of {a}");
        assert!(buf.iter().all(|x| x.is_finite()));
    }

    /// A quiet tone keeps its level: the drive's compensation is fixed, not a loudness follower.
    #[test]
    fn the_drive_keeps_a_quiet_signal_at_its_level() {
        let dry: Vec<f32> = sine(1_000.0, BLOCK * 2 * 40).iter().map(|x| x * 0.01).collect();
        let mut s = Strip::new([0, 0, u16::MAX, 0, 0, 0, drive_kind_word(0), 0], false);
        let mut buf = dry.clone();
        run(&mut s, &mut buf);
        let (a, b) = (rms(&dry[dry.len() - BLOCK * 2..]), rms(&buf[buf.len() - BLOCK * 2..]));
        assert!((b / a - 1.0).abs() < 0.05, "level kept: {b} of {a}");
    }

    /// The tube character's asymmetry does not leave a floor under the track: the DC blocker takes it out.
    #[test]
    fn the_drive_blocks_the_offset_the_tube_character_makes() {
        let mut s = Strip::new([0, 0, u16::MAX, 0, 0, 0, drive_kind_word(0), 0], false);
        let mut buf = vec![0.3f32; BLOCK * 2 * 40];
        run(&mut s, &mut buf);
        let tail = &buf[buf.len() - BLOCK * 2..];
        assert!(tail.iter().all(|x| x.abs() < 1e-3), "no floor: {:?}", tail[0]);
    }

    /// Every character stays inside its bounds and finite, at any amount.
    #[test]
    fn every_drive_character_is_bounded() {
        for kind in 0..4 {
            let mut s = Strip::new([0, 0, u16::MAX, 0, 0, 0, drive_kind_word(kind), 0], false);
            let mut buf = sine(1_000.0, BLOCK * 2 * 20);
            run(&mut s, &mut buf);
            assert!(buf.iter().all(|x| x.is_finite() && x.abs() < 1.0), "{}", DRIVE_TYPES[kind]);
        }
    }

    /// A character change crossfades: switching from tube to clip mid-tone leaves no click.
    #[test]
    fn a_character_change_does_not_click() {
        let mut s = Strip::new([0, 0, u16::MAX, 0, 0, 0, drive_kind_word(0), 0], false);
        let mut buf = sine(1_000.0, BLOCK * 2 * 40);
        for (i, c) in buf.chunks_mut(BLOCK * 2).enumerate() {
            if i == 20 {
                s.set([0, 0, u16::MAX, 0, 0, 0, drive_kind_word(3), 0], false);
            }
            s.process(c, RATE);
        }
        let max = buf.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        // Both characters are held well inside the shaper's bounds, so a step would stand out.
        assert!(max < 0.1, "no click: {max}");
    }

    /// Word 1 is the section off: nothing at all, sample for sample.
    #[test]
    fn the_compressor_off_passes_the_signal_through() {
        let dry = sine(1_000.0, 480);
        let mut s = Strip::new([0; 8], false);
        let mut buf = dry.clone();
        s.process(&mut buf, RATE);
        assert_eq!(buf, dry);
    }

    /// Full on, a tone above the threshold is taken down, and the gain holds still once it has settled.
    #[test]
    fn the_compressor_takes_a_loud_signal_down() {
        let dry = sine(1_000.0, BLOCK * 2 * 40);
        let mut s = Strip::new([0, u16::MAX, 0, 0, 0, 0, 0, 0], false);
        let mut buf = dry.clone();
        run(&mut s, &mut buf);
        let rms_tail = |b: &[f32]| rms(&b[b.len() - BLOCK * 2..]);
        let ratio = rms_tail(&buf) / rms_tail(&dry);
        assert!(ratio < 0.85, "taken down: {ratio}");
        assert!(ratio > 0.3, "not squashed: {ratio}");
        assert!(buf.iter().all(|x| x.is_finite()));
    }

    /// The makeup is fixed at the -18 dB reference: quiet material comes up rather than the loud material pulling it
    /// around, so the two ends of the dynamic range move toward each other.
    #[test]
    fn the_compressor_narrows_the_dynamic_range() {
        let quiet: Vec<f32> = sine(1_000.0, BLOCK * 2 * 40).iter().map(|x| x * 0.02).collect();
        let mut s = Strip::new([0, u16::MAX, 0, 0, 0, 0, 0, 0], false);
        let mut buf = quiet.clone();
        run(&mut s, &mut buf);
        let rms_tail = |b: &[f32]| rms(&b[b.len() - BLOCK * 2..]);
        let ratio = rms_tail(&buf) / rms_tail(&quiet);
        assert!(ratio > 1.5, "quiet material lifted: {ratio}");
        assert!(ratio < 6.0, "but not without bound: {ratio}");
    }

    /// Every profile stays bounded and finite, whatever its attack and release.
    #[test]
    fn every_compressor_profile_is_bounded() {
        for profile in 0..PROFILES.len() {
            let mut s = Strip::new([0, u16::MAX, 0, 0, 0, comp_profile_word(profile), 0, 0], false);
            let mut buf = sine(1_000.0, BLOCK * 2 * 20);
            run(&mut s, &mut buf);
            assert!(buf.iter().all(|x| x.is_finite() && x.abs() < 1.0), "{}", PROFILES[profile].name);
        }
    }

    /// The profile word counts the six profiles and comes back to itself.
    #[test]
    fn the_compressor_words_round_trip() {
        for profile in 0..PROFILES.len() {
            assert_eq!(comp_profile(comp_profile_word(profile)), profile, "{}", PROFILES[profile].name);
        }
        assert_eq!(comp_profile(0), 0);
        assert_eq!(comp_profile(u16::MAX), 5);
        for amount in [0.0f32, 0.1, 0.45, 1.0] {
            assert!((comp_amount(comp_word(amount)) - amount).abs() < 0.001);
        }
    }

    /// Noise, and a tone standing on it: a resonance to find.
    fn resonance_signal(hz: f32, frames: usize) -> Vec<f32> {
        let mut seed = 1_234_567u32;
        (0..frames).flat_map(|n| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let noise = (seed as f64 / f64::from(u32::MAX) * 2.0 - 1.0) as f32 * 0.08;
            let x = noise + 0.22 * (std::f32::consts::TAU * hz * n as f32 / RATE as f32).sin();
            [x, x]
        }).collect()
    }

    fn amplitude(x: &[f32], hz: f32, rate: u32) -> f32 {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, f) in x.chunks_exact(2).enumerate() {
            let phase = std::f64::consts::TAU * f64::from(hz) * i as f64 / f64::from(rate);
            re += f64::from(f[0]) * phase.cos();
            im += f64::from(f[0]) * phase.sin();
        }
        ((re * re + im * im).sqrt() / (x.len() / 2) as f64 * 2.0) as f32
    }

    /// TAME takes a resonance down, and not all the way out: the notch is bounded.
    #[test]
    fn the_tame_takes_a_resonance_down() {
        let dry = resonance_signal(2_300.0, RATE as usize / 3);
        let mut s = Strip::new([0, 0, 0, 0, u16::MAX, 0, 0, 0], false);
        let mut buf = dry.clone();
        run(&mut s, &mut buf);
        let start = buf.len() / 3;
        let ratio = amplitude(&buf[start..], 2_300.0, RATE) / amplitude(&dry[start..], 2_300.0, RATE);
        assert!(ratio < 0.55, "the resonance is taken down: {ratio}");
        assert!(ratio > 0.1, "but not torn out: {ratio}");
        assert!(buf.iter().all(|x| x.is_finite()));
    }

    /// Broad noise is not a resonance: TAME leaves most of it, and a rush of it (a hit) is left alone alike.
    #[test]
    fn the_tame_leaves_broad_noise_alone() {
        let dry = resonance_signal(0.0, RATE as usize / 3);
        let mut s = Strip::new([0, 0, 0, 0, u16::MAX, 0, 0, 0], false);
        let mut buf = dry.clone();
        run(&mut s, &mut buf);
        let (a, b) = (rms(&dry[dry.len() / 2..]), rms(&buf[buf.len() / 2..]));
        let db = 20.0 * (b / a).log10();
        assert!(db > -0.7, "broadband signal dulled: {db} dB");
    }

    /// Word 4 is the section off: nothing at all, sample for sample.
    #[test]
    fn the_tame_off_passes_the_signal_through() {
        let dry = resonance_signal(2_300.0, 480);
        let mut s = Strip::new([0; 8], false);
        let mut buf = dry.clone();
        s.process(&mut buf, RATE);
        assert_eq!(buf, dry);
    }

    /// Both sides are linked through one bank of notches, so the image they came in with is the one they leave with.
    #[test]
    fn the_tame_holds_the_stereo_image() {
        let dry = resonance_signal(3_000.0, RATE as usize / 6);
        let mut buf: Vec<f32> = dry.chunks_exact(2).flat_map(|f| [f[0], -f[0] * 0.3]).collect();
        let mut s = Strip::new([0, 0, 0, 0, u16::MAX, 0, 0, 0], false);
        run(&mut s, &mut buf);
        assert!(buf.chunks_exact(2).all(|f| (f[1] + f[0] * 0.3).abs() < 2e-5), "the image holds");
    }

    /// A strip with only the de-esser set.
    fn deess_strip(pct: u16) -> Strip {
        Strip::new([0, 0, 0, 0, 0, 0, 0, deess_word(f32::from(pct) / 100.0)], false)
    }

    /// A 220 Hz tone: well below what the de-esser listens for.
    #[test]
    fn the_de_esser_leaves_a_tone_alone() {
        let dry = sine(220.0, BLOCK * 2 * 40);
        let mut s = deess_strip(100);
        let mut buf = dry.clone();
        run(&mut s, &mut buf);
        let (a, b) = (rms(&dry[dry.len() - BLOCK * 2..]), rms(&buf[buf.len() - BLOCK * 2..]));
        assert!((b / a - 1.0).abs() < 0.02, "the tone passes: {b} of {a}");
    }

    /// Full on, the de-esser takes the edge off broadband material: the signal loses energy, and not all of it.
    #[test]
    fn the_de_esser_takes_the_edge_off_sibilance() {
        let dry = resonance_signal(0.0, RATE as usize / 3);
        let mut s = deess_strip(100);
        let mut buf = dry.clone();
        run(&mut s, &mut buf);
        let (a, b) = (rms(&dry[dry.len() / 2..]), rms(&buf[buf.len() / 2..]));
        assert!(b < a * 0.95, "the edge is taken off: {b} of {a}");
        assert!(b > a * 0.5, "but the signal is not gone: {b} of {a}");
        assert!(buf.iter().all(|x| x.is_finite() && x.abs() < 1.0));
    }

    /// Every section at once, de-esser last: the chain stays linked, finite and bounded.
    #[test]
    fn the_whole_chain_stays_linked_and_bounded() {
        let dry = resonance_signal(2_300.0, RATE as usize / 3);
        let mut s = Strip::new([
            cut_word(60.0),
            comp_word(0.5),
            drive_word(0.6),
            gate_word(-54.0),
            u16::MAX,
            comp_profile_word(1),
            drive_kind_word(1),
            u16::MAX,
        ], false);
        let mut buf = dry.clone();
        run(&mut s, &mut buf);
        assert!(buf.iter().all(|x| x.is_finite() && x.abs() < 2.0));
        assert!(buf.chunks_exact(2).all(|f| (f[1] - f[0]).abs() < 1e-6), "linked");
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
