// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The glitch grid: a track's second strip style, after dblue Glitch (and OpenGlitch, its free recreation). Sixteen
//! steps run one after the other, each firing a module of its own — a retrigger, a bit and rate crush, a reverse, a gate
//! or a ring mod — so a bar can be sliced into a pattern instead of shaped by one knob.
//!
//! The grid is sample-accurate and needs no transport: the engine calls `process` with the track's block, and the steps
//! run on their own clock (an eighth of a second at any rate). A step that fires nothing passes the audio through, and
//! the section's amount at zero is the section off (the track's audio is the buffer that arrived).
//!
//! spec: docs/plans — the analog strip is plan v3 Appendix C; this is Lemon Studio's own

use crate::glide::{Glide, Ramp};

/// The modules a step can fire, in the order the grid's control word numbers them.
pub const MODULES: [&str; 6] = ["OFF", "STUT", "CRSH", "RVRS", "GATE", "RING"];

/// How many steps a bar of the grid has.
pub const STEPS: usize = 16;

/// The ring the slicer reads its repeats, reverses and retriggers from: 8192 frames, a sixth of a second at 48 kHz.
const RING: usize = 8192;

/// The glitch section's control word as an amount: word 0 is the section off, 65535 all the way in.
pub fn glitch_amount(word: u16) -> f32 {
    f32::from(word) / 65535.0
}

/// The word for an amount 0..=1.
pub fn glitch_word(amount: f32) -> u16 {
    (amount.clamp(0.0, 1.0) * 65535.0).round() as u16
}

/// A grid of sixteen steps, each a module's number (`MODULES` order, 0 = the step fires nothing), and the amount the
/// whole of it is heard at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pattern {
    pub steps: [u8; STEPS],
    pub amount: u16,
}

impl Pattern {
    /// An empty grid: every step off.
    pub const OFF: Pattern = Pattern { steps: [0; STEPS], amount: 0 };

    /// True when no step fires anything, whatever the amount says.
    pub fn empty(&self) -> bool {
        self.steps.iter().all(|&m| m == 0)
    }
}

impl Default for Pattern {
    fn default() -> Self {
        Pattern::OFF
    }
}

/// The glitch grid: sixteen steps on their own clock, each playing the audio the track would have had in it in the way
/// its module says. The amount folds the whole grid in from the dry signal, so at zero the track passes through, and it
/// glides over 20 ms so switching it in or out does not click.
pub struct Glitch {
    /// The last 8192 frames of both channels: the repeats, the reverses and the retriggers read from here.
    ring: [[f32; RING]; 2],
    /// Where the next frame goes in the ring.
    write: usize,
    /// Frames since the grid last started a bar.
    at: usize,
    /// The grid as it is set.
    pattern: Pattern,
    /// The amount as it glides (0..1), 20 ms either way.
    amount: Glide,
    /// The crush's held frame and the frames it still holds for.
    hold: [f32; 2],
    held: usize,
    /// The ring mod's phase and the gate's smoothed gain.
    phase: f32,
    gate: f32,
}

/// One block's grid settings: the step's length is fixed within it and the amount glides through it.
struct Block {
    amount: Ramp,
    /// Frames per step: an eighth of a second, so no tempo is needed and the grid runs on its own.
    step: usize,
    /// The rate, for the ring mod's frequency, and 1 ms for the gate's own smoothing.
    rate: f32,
    smooth: f32,
    /// This frame's amount, moved once and used for both channels.
    a: f32,
}

impl Glitch {
    pub fn new(pattern: Pattern) -> Glitch {
        Glitch {
            ring: [[0.0; RING]; 2],
            write: 0,
            at: 0,
            pattern,
            amount: Glide::new(glitch_amount(pattern.amount)),
            hold: [0.0; 2],
            held: 0,
            phase: 0.0,
            gate: 1.0,
        }
    }

    /// The project's grid (a track rebuilt from a changed project).
    pub fn set(&mut self, pattern: Pattern) {
        self.pattern = pattern;
    }

    /// True when the section is off and has glided out: it holds nothing and writes nothing.
    pub fn idle(&self) -> bool {
        self.pattern.amount == 0 && self.amount.current == 0.0
    }

    /// Empties the ring: a section switched in again starts from silence, not from what it held when it went out.
    pub fn reset(&mut self) {
        self.ring = [[0.0; RING]; 2];
        self.write = 0;
        self.at = 0;
        self.hold = [0.0; 2];
        self.held = 0;
        self.phase = 0.0;
        self.gate = 1.0;
    }

    /// The step the grid is on, for a console that draws a playhead.
    pub fn step(&self, rate: u32) -> usize {
        (self.at / step_frames(rate)).min(STEPS - 1)
    }

    /// Plans a block; `None` when the section is off and has glided out (its state is cleared).
    fn block(&mut self, frames: usize, rate: u32) -> Option<Block> {
        let rate = rate.max(1);
        let target = glitch_amount(self.pattern.amount);
        if target == 0.0 && self.amount.current == 0.0 {
            self.reset();
            return None;
        }
        let ratef = rate as f32;
        let step = step_frames(rate);
        // A pattern dropped in whole starts a bar from its beginning, so a console's click lands on the step it shows.
        if self.at >= step * STEPS {
            self.at = 0;
        }
        Some(Block {
            amount: self.amount.block(target, frames, 1.0 / (0.020 * ratef)),
            step,
            rate: ratef,
            smooth: 1.0 - (-1.0 / (ratef * 0.001)).exp(),
            a: 0.0,
        })
    }

    /// One stereo frame: the step's module over what came in, folded in by the amount.
    fn frame(&mut self, b: &mut Block, input: [f32; 2]) -> [f32; 2] {
        b.a = b.amount.next();
        let amount = b.a;
        let pos = self.at % b.step;
        let step = (self.at / b.step).min(STEPS - 1);
        if pos == 0 {
            // A step starts fresh: the crush holds nothing and the ring mod starts its cycle again.
            self.held = 0;
            self.phase = 0.0;
        }
        for ch in 0..2 {
            self.ring[ch][self.write] = input[ch];
        }
        let mut out = input;
        match self.pattern.steps[step] {
            // STUT: the step's first quarter, played four times over.
            1 => {
                let span = (b.step / 4).max(1);
                // Back to the start of the quarter this frame belongs in, then on through it again.
                let back = pos - pos % span;
                let at = (self.write + RING - back) % RING;
                for ch in 0..2 {
                    let y = self.ring[ch][at];
                    out[ch] = input[ch] + (y - input[ch]) * amount;
                }
            }
            // CRSH: fewer bits and a sample and hold that gets longer as the amount goes up.
            2 => {
                let bits = 2.0 + 14.0 * (1.0 - amount);
                if self.held == 0 {
                    let levels = 2f32.powf(bits).max(2.0);
                    self.hold = input.map(|x| (x.clamp(-1.0, 1.0) * levels).round() / levels);
                    self.held = (1.0 + 15.0 * amount) as usize;
                }
                self.held = self.held.saturating_sub(1);
                for ch in 0..2 {
                    out[ch] = input[ch] + (self.hold[ch] - input[ch]) * amount;
                }
            }
            // RVRS: the step before this one, played backwards into it (a reverse needs the audio, so one step behind).
            3 => {
                let back = (2 * pos + 1).min(RING - 1);
                let at = (self.write + RING - back) % RING;
                for ch in 0..2 {
                    let y = self.ring[ch][at];
                    out[ch] = input[ch] + (y - input[ch]) * amount;
                }
            }
            // GATE: the step opens for less and less of itself as the amount goes up.
            4 => {
                let open = (pos as f32) < (1.0 - 0.95 * amount) * b.step as f32;
                self.gate += (if open { 1.0 } else { 0.0 } - self.gate) * b.smooth;
                for ch in 0..2 {
                    out[ch] = input[ch] * self.gate;
                }
            }
            // RING: a sine across the signal, 40 Hz up to 900 Hz.
            5 => {
                self.phase += std::f32::consts::TAU * (40.0 + 860.0 * amount) / b.rate;
                if self.phase > std::f32::consts::TAU {
                    self.phase -= std::f32::consts::TAU;
                }
                let g = 1.0 - amount + amount * self.phase.sin();
                for ch in 0..2 {
                    out[ch] = input[ch] * g;
                }
            }
            // OFF: the step passes the audio through, and the amount takes it back to the dry signal anyway.
            _ => {
                for ch in 0..2 {
                    out[ch] = input[ch];
                }
            }
        }
        self.write = (self.write + 1) % RING;
        self.at += 1;
        if self.at >= STEPS * b.step {
            self.at = 0;
        }
        out
    }

    /// One interleaved stereo block.
    pub fn process(&mut self, buf: &mut [f32], rate: u32) {
        let frames = buf.len() / 2;
        let Some(mut b) = self.block(frames, rate) else { return };
        for f in buf.chunks_exact_mut(2) {
            let y = self.frame(&mut b, [f[0], f[1]]);
            f[0] = y[0];
            f[1] = y[1];
        }
    }
}

/// Frames per step: an eighth of a second, and never more than half the ring.
fn step_frames(rate: u32) -> usize {
    ((rate.max(1) as usize) / 8).clamp(1, RING / 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn sine(hz: f32, frames: usize) -> Vec<f32> {
        (0..frames).flat_map(|i| {
            let x = (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin() * 0.5;
            [x, x]
        }).collect()
    }

    fn ramp(frames: usize) -> Vec<f32> {
        (0..frames).flat_map(|i| {
            let x = i as f32 / frames as f32;
            [x, x]
        }).collect()
    }

    /// A grid that fires one module on every step.
    fn grid(module: u8, amount: f32) -> Glitch {
        Glitch::new(Pattern { steps: [module; STEPS], amount: glitch_word(amount) })
    }

    fn run(g: &mut Glitch, buf: &mut [f32]) {
        g.process(buf, RATE);
    }

    /// Every step off is the section off: the buffer is the one that arrived.
    #[test]
    fn an_empty_grid_passes_the_signal_through() {
        let dry = sine(1_000.0, 480 * 4);
        let mut g = Glitch::new(Pattern { steps: [0; STEPS], amount: u16::MAX });
        let mut buf = dry.clone();
        run(&mut g, &mut buf);
        assert_eq!(buf, dry);
    }

    /// The amount at zero is the section off too, whatever the grid holds.
    #[test]
    fn a_zeroed_amount_passes_the_signal_through() {
        let dry = sine(1_000.0, 480 * 4);
        let mut g = grid(1, 0.0);
        let mut buf = dry.clone();
        run(&mut g, &mut buf);
        assert_eq!(buf, dry);
        assert!(g.idle());
    }

    /// STUT plays the step's first quarter four times over, so the step's audio repeats.
    #[test]
    fn a_stutter_step_repeats_itself() {
        let mut buf = ramp(step_frames(RATE) * 2);
        let mut g = grid(1, 1.0);
        run(&mut g, &mut buf);
        let step = step_frames(RATE);
        let quarter = step / 4;
        // Inside one step, the second quarter is the first one played again.
        let a = &buf[..quarter * 2];
        let b = &buf[quarter * 2..quarter * 4];
        assert_eq!(a, b, "the quarter repeats");
    }

    /// CRSH holds frames and rounds them, so the output has far fewer values than the input.
    #[test]
    fn a_crush_step_quantises_the_signal() {
        let mut buf = sine(220.0, 480 * 40);
        let mut g = grid(2, 1.0);
        run(&mut g, &mut buf);
        let mut levels: Vec<f32> = buf[buf.len() / 2..].to_vec();
        levels.sort_by(|a, b| a.partial_cmp(b).unwrap());
        levels.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        assert!(levels.len() < 20, "few values: {}", levels.len());
    }

    /// RVRS walks back through the audio before the step, so a rising signal comes out falling.
    #[test]
    fn a_reverse_step_plays_backwards() {
        let step = step_frames(RATE);
        let dry = ramp(step * 2);
        let mut buf = dry.clone();
        let mut g = grid(3, 1.0);
        run(&mut g, &mut buf);
        // The second step reads back from where it starts, so it opens high and falls while the dry signal rises.
        let (open, close) = (buf[2 * step], buf[2 * (step * 2 - 1)]);
        assert!(open > close + 0.1, "backwards: {open} then {close}");
    }

    /// GATE closes part of the step and leaves the rest: the step goes quiet and comes back.
    #[test]
    fn a_gate_step_closes_part_of_itself() {
        let mut buf = sine(1_000.0, step_frames(RATE) * 2);
        let mut g = grid(4, 1.0);
        run(&mut g, &mut buf);
        let step = step_frames(RATE);
        let loud = buf[..step / 20].iter().any(|x| x.abs() > 0.1);
        let quiet = buf[step / 4..step].iter().all(|x| x.abs() < 1e-3);
        assert!(loud, "the start of the step is heard");
        assert!(quiet, "the rest of it is closed");
    }

    /// The words round trip, and the amount folds the grid in: less amount, less change from the dry signal.
    #[test]
    fn the_amount_folds_the_grid_in() {
        assert_eq!(glitch_amount(0), 0.0);
        assert!((glitch_amount(glitch_word(0.5)) - 0.5).abs() < 1e-4);
        let dry = sine(1_000.0, step_frames(RATE) * 2);
        let change = |v: f32| {
            let mut buf = dry.clone();
            let mut g = grid(2, v);
            run(&mut g, &mut buf);
            dry.iter().zip(&buf).map(|(a, b)| (a - b).abs()).sum::<f32>() / dry.len() as f32
        };
        let (a, b) = (change(0.3), change(1.0));
        assert!(b > a, "more amount, more crush: {b} of {a}");
    }

    /// A grid of every module at once stays finite and bounded.
    #[test]
    fn the_grid_stays_bounded() {
        let mut buf = sine(440.0, step_frames(RATE) * 4);
        let mut g = Glitch::new(Pattern { steps: std::array::from_fn(|i| (i % 6) as u8), amount: u16::MAX });
        run(&mut g, &mut buf);
        assert!(buf.iter().all(|x| x.is_finite() && x.abs() < 2.0));
    }

    /// The playhead walks the grid one step an eighth of a second, and wraps at the bar.
    #[test]
    fn the_playhead_walks_the_grid() {
        let mut g = grid(0, 1.0);
        let mut buf = vec![0.0f32; 2];
        let step = step_frames(RATE);
        assert_eq!(g.step(RATE), 0);
        for _ in 0..step {
            run(&mut g, &mut buf);
        }
        assert_eq!(g.step(RATE), 1);
        for _ in 0..step * (STEPS - 1) {
            run(&mut g, &mut buf);
        }
        assert_eq!(g.step(RATE), 0, "the bar wraps");
    }
}
