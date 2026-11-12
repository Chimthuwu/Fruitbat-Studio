// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The mixer graph: tracks, their slots, their routes and the device's input/output io, in processing order.

use os_dsp::glide::Glide;
use crate::effects::Effect;
use crate::laws;
use super::{auto_bypass, pdc, rec};

pub(crate) struct MixTrack {
    /// Fader (0..16000, 12800 = unity), pan (-6400..6400) and stereo separation (-64..64, positive merges) as stored;
    /// automation rewrites them, then the matrix (spec: audio/mixer.md).
    pub(crate) vol: i32,
    pub(crate) pan: i32,
    pub(crate) sep: i32,
    pub(crate) on: bool,
    /// The project's panning law (event 23), for the pan in the matrix.
    pub(crate) law: laws::PanLaw,
    /// Swap left / right and reverse polarity, on the fader matrix (spec: audio/mixer.md "Fader matrix").
    pub(crate) swap: bool,
    pub(crate) polarity: bool,
    /// The track's "effect slots enabled" switch: when off no slot runs (spec: audio/mixer.md).
    pub(crate) slots_enabled: bool,
    /// Allow threaded processing: off, the track renders on the rendering thread even on several cores (spec:
    /// audio/multicore.md §5).
    pub(crate) threaded: bool,
    /// The three-band equaliser, after the slots and before the fader.
    pub(crate) eq: os_dsp::mixer::TrackEq,
    /// The Analog Channel Strip, before the slots (plan v3 §6.1).
    pub(crate) strip: os_dsp::strip::Strip,
    /// The Glitch strip, which runs in the Analog strip's place when the track chose it (`glitch_style`).
    pub(crate) glitch: os_dsp::glitch::Glitch,
    pub(crate) glitch_style: bool,
    /// The modules the track runs in its strip's place (`MixerTrack::chain`): 1..=6 the analog sections, 7 the grid,
    /// in signal order. The two whole strips keep their old paths in `render`.
    pub(crate) chain: [u8; 8],
    /// A mixed chain's gain, ramped across the strip's bypass switch in 10 ms (`chain_ramp`) so the switch does not
    /// click. 1 when the chain runs; the two whole strips keep their own fade and leave it at 1.
    pub(crate) chain_gain: f32,
    /// Insert effects in slot order (applied before the fader).
    pub(crate) effects: Vec<FxSlot>,
    /// Slot switches and stored mix levels (0..12800) by slot, for hosted effects attached later.
    pub(crate) slot_on: [bool; 10],
    pub(crate) slot_mix: [i32; 10],
    /// Auto-bypass when silent, by slot (`auto_bypass`), for hosted effects attached later.
    pub(crate) slot_smart: [bool; 10],
    /// Largest |sample| per side after the fader since the meters last read them.
    pub(crate) peak: [f32; 2],
    /// The fader matrix's target gains `[[L←L, L←R], [R←L, R←R]]`, and the gains as they glide towards them.
    pub(crate) matrix: [[f32; 2]; 2],
    pub(crate) gains: [[Glide; 2]; 2],
    /// The track's sends. Routes at level 0 (sidechain only) pass no audio but stay: automation may raise them.
    pub(crate) routes: Vec<Route>,
    /// The settings as stored, put back when playback starts at another position.
    pub(crate) stored: Stored,
    /// The output and input latency settings in milliseconds (event 236; spec: audio/delay-compensation.md "Settings").
    pub(crate) latency: (f32, f32),
    /// What the engine itself plays into the track (sampler voices, the metronome) waits here for the latest path
    /// into it (delay compensation).
    pub(crate) pad: pdc::Pad,
    /// The device input the track hears (-1: none, or not monitored) and the output pair it plays on directly (-1:
    /// none), numbered as stored (spec: audio/track-io.md §2.1, §2.2).
    pub(crate) input: i32,
    pub(crate) output: i32,
    /// Armed for disk recording: where the take is tapped and the input it records (spec: audio/track-io.md §2.5).
    pub(crate) rec: Option<rec::Arm>,
}

/// The audio device's inputs as they arrive, for the mixer tracks that monitor one (spec: audio/track-io.md §2.2).
pub trait DeviceInput: Send {
    /// Inputs the device has open.
    fn channels(&self) -> usize;
    /// The next `out.len() / channels` frames of every input, interleaved; what has not arrived is silence.
    fn read(&mut self, out: &mut [f32]);
}

/// A stream's device: its inputs, and its output pairs, which the master and the tracks' direct outputs play on. With
/// it the engine hands out `2 × pairs` interleaved channels instead of the master (`Engine::set_device`).
pub(crate) struct DeviceIo {
    pub(crate) input: Option<Box<dyn DeviceInput>>,
    pub(crate) pairs: usize,
    /// This tick's inputs (interleaved), and what the tracks send to the pairs directly.
    pub(crate) inputs: Vec<f32>,
    pub(crate) direct: Vec<f32>,
}

/// Input number `n` of a device with `channels` inputs as (left, right) channels: the stereo pairs first, then each
/// input alone, heard on both sides (spec: audio/track-io.md §2.1).
pub fn input_channels(n: i32, channels: usize) -> Option<(usize, usize)> {
    let n = usize::try_from(n).ok()?;
    let pairs = channels / 2;
    if n < pairs {
        Some((2 * n, 2 * n + 1))
    } else {
        (n - pairs < channels).then_some((n - pairs, n - pairs))
    }
}

/// A mixer track's settings as the project stores them.
#[derive(Debug, Clone)]
pub(crate) struct Stored {
    pub(crate) vol: i32,
    pub(crate) pan: i32,
    pub(crate) sep: i32,
    pub(crate) eq: [[i32; 3]; 3],
    pub(crate) slot_on: [bool; 10],
    pub(crate) slot_mix: [i32; 10],
    /// Send gains, one per route.
    pub(crate) sends: Vec<f32>,
}

/// A send to another track: its gain and the gain as it glides there (spec: audio/mixer.md "Sends", "Gains ramp").
pub(crate) struct Route {
    pub(crate) dest: usize,
    pub(crate) gain: f32,
    pub(crate) glide: Glide,
    /// Delay compensation: the send, after its level, waits here for the latest path into `dest`.
    pub(crate) pad: pdc::Pad,
}

/// One filled mixer slot: Lemon Studio's own effect or a hosted plugin, with the slot's switch and stored mix level
/// (0..12800) and its dry level as it glides (spec: audio/mixer.md "An effect slot").
pub(crate) struct FxSlot {
    pub(crate) slot: usize,
    pub(crate) on: bool,
    pub(crate) mix: i32,
    /// Auto-bypass when silent, and how long the slot has heard and put out silence (`auto_bypass`).
    pub(crate) smart: bool,
    pub(crate) quiet: auto_bypass::Quiet,
    /// The last tick passed the effect by.
    pub(crate) skipped: bool,
    pub(crate) dry: Glide,
    /// Switched on during the song: the effect is emptied before it runs again.
    pub(crate) fresh: bool,
    pub(crate) fx: Fx,
    /// The delay the effect last reported (read after it processes), and the pad its dry part waits in for it (spec:
    /// audio/delay-compensation.md "A slot's dry part").
    pub(crate) delay: u32,
    pub(crate) dry_pad: pdc::Pad,
    /// The sidechain input: the track it listens to (worked out with the pads), and the pad that delays that track's
    /// output to the lateness at the slot (spec: audio/sidechain.md "Sidechain input").
    pub(crate) key: Option<usize>,
    pub(crate) key_pad: pdc::Pad,
    /// A hosted plugin's input ports, main and first auxiliary bus (0 = none), and the track its main bus takes instead
    /// of the track's own audio, with the pad that delays it like a sidechain input (spec: audio/sidechain.md "Hosted
    /// plugins' input buses").
    pub(crate) ports: [usize; 2],
    pub(crate) main: Option<usize>,
    pub(crate) main_pad: pdc::Pad,
}

impl FxSlot {
    pub(crate) fn new(slot: usize, on: bool, mix: i32, smart: bool, fx: Fx) -> FxSlot {
        let delay = match &fx {
            Fx::Own(e) => e.latency() as u32,
            Fx::Hosted(_) => 0,
        };
        FxSlot {
            slot,
            on,
            mix,
            smart,
            quiet: auto_bypass::Quiet::default(),
            skipped: false,
            dry: Glide::new(os_dsp::mixer::dry_level(mix)),
            fresh: false,
            fx,
            delay,
            dry_pad: pdc::Pad::default(),
            key: None,
            key_pad: pdc::Pad::default(),
            ports: [0; 2],
            main: None,
            main_pad: pdc::Pad::default(),
        }
    }

    /// The number of the track's input the slot listens to, counted from 1 (0 = none).
    pub(crate) fn key_input(&self) -> usize {
        match &self.fx {
            Fx::Own(e) => e.key_input(),
            Fx::Hosted(_) => self.ports[1],
        }
    }
}

/// A stored mix level (0..12800) from the model's 0..1.
pub(crate) fn mix_level(v: f64) -> i32 {
    (v.clamp(0.0, 1.0) * 12800.0).round() as i32
}

pub(crate) enum Fx {
    Own(Box<dyn Effect>),
    Hosted(crate::instrument::SharedEffect),
}

impl MixTrack {
    pub(crate) fn unity() -> MixTrack {
        let unity = [[1.0, 0.0], [0.0, 1.0]];
        MixTrack {
            vol: 12800,
            pan: 0,
            sep: 0,
            on: true,
            law: laws::PanLaw::Circular,
            swap: false,
            polarity: false,
            slots_enabled: true,
            threaded: true,
            eq: os_dsp::mixer::TrackEq::default(),
            strip: os_dsp::strip::Strip::new([0; 8], true),
            glitch: os_dsp::glitch::Glitch::new(os_dsp::glitch::Pattern::OFF),
            glitch_style: false,
            chain: os_model::CHAIN_ANALOG,
            chain_gain: 1.0,
            effects: Vec::new(),
            slot_on: [true; 10],
            slot_mix: [12800; 10],
            slot_smart: [false; 10],
            peak: [0.0; 2],
            matrix: unity,
            gains: unity.map(|r| r.map(Glide::new)),
            routes: vec![],
            stored: Stored {
                vol: 12800,
                pan: 0,
                sep: 0,
                eq: os_dsp::mixer::EQ_DEFAULT,
                slot_on: [true; 10],
                slot_mix: [12800; 10],
                sends: vec![],
            },
            latency: (0.0, 0.0),
            pad: pdc::Pad::default(),
            input: -1,
            output: -1,
            rec: None,
        }
    }

    /// The settings as they are now, kept to be put back by [`MixTrack::restore`].
    pub(crate) fn store(&mut self) {
        self.stored = Stored {
            vol: self.vol,
            pan: self.pan,
            sep: self.sep,
            eq: [0, 1, 2].map(|b| self.eq.settings(b)),
            slot_on: self.slot_on,
            slot_mix: self.slot_mix,
            sends: self.routes.iter().map(|r| r.gain).collect(),
        };
    }

    /// Puts the stored settings back (playback starting at another position: nothing of an automation clip holds
    /// before the clip; spec: audio/automation.md "When a clip acts"), with every gain at its target when `settle`,
    /// else gliding there.
    pub(crate) fn restore(&mut self, settle: bool) {
        let s = self.stored.clone();
        (self.vol, self.pan, self.sep) = (s.vol, s.pan, s.sep);
        for (b, band) in s.eq.iter().enumerate() {
            for (f, &v) in band.iter().enumerate() {
                if self.eq.settings(b)[f] != v {
                    self.eq.set(b, f, v);
                }
            }
        }
        for k in 0..10 {
            self.set_slot_on(k, s.slot_on[k]);
            self.set_slot_mix(k, s.slot_mix[k]);
        }
        for (r, &g) in self.routes.iter_mut().zip(&s.sends) {
            r.gain = g;
        }
        self.matrix = self.compute_matrix();
        if settle {
            self.settle();
        }
    }

    /// Every gain at its target at once: the matrix, the slots' dry levels, the sends.
    pub(crate) fn settle(&mut self) {
        self.gains = self.matrix.map(|r| r.map(Glide::new));
        for fx in &mut self.effects {
            fx.dry = Glide::new(os_dsp::mixer::dry_level(fx.mix));
        }
        for r in &mut self.routes {
            r.glide = Glide::new(r.gain);
        }
    }

    /// Slot `slot`'s switch. A slot switched on empties its effect first; neither way fades (spec: audio/mixer.md
    /// "Changes while the song plays").
    pub(crate) fn set_slot_on(&mut self, slot: usize, on: bool) {
        let Some(s) = self.slot_on.get_mut(slot) else { return };
        *s = on;
        for fx in self.effects.iter_mut().filter(|f| f.slot == slot) {
            fx.fresh |= on && !fx.on;
            fx.on = on;
        }
    }

    /// Slot `slot`'s stored mix level (0..12800); its dry level glides there.
    pub(crate) fn set_slot_mix(&mut self, slot: usize, mix: i32) {
        let Some(m) = self.slot_mix.get_mut(slot) else { return };
        *m = mix;
        for fx in self.effects.iter_mut().filter(|f| f.slot == slot) {
            fx.mix = mix;
        }
    }

    /// The fader matrix (volume, pan under the project's panning law, stereo separation, switch; spec: audio/mixer.md
    /// "Fader matrix", audio/gain-laws.md "Panning law: Triangular") with swap and polarity on top, as
    /// `[[L←L, L←R], [R←L, R←R]]`.
    pub(crate) fn compute_matrix(&self) -> [[f32; 2]; 2] {
        let [ll, rl, rr, lr] = os_dsp::mixer::fader_matrix_law(self.vol, self.pan, self.sep, self.on, self.law);
        let mut m = [[ll, rl], [lr, rr]];
        // Swap exchanges the two outputs after pan and separation, not the inputs: (LL, RR, RL, LR) become
        // (LR, RL, RR, LL).
        if self.swap {
            m = [m[1], m[0]];
        }
        if self.polarity {
            m = m.map(|r| r.map(|g| -g));
        }
        m
    }
}
/// Orders tracks so that every track is processed before any track it routes to.
pub(crate) fn processing_order(tracks: &[MixTrack]) -> Vec<usize> {
    fn visit(t: usize, tracks: &[MixTrack], state: &mut [u8], order: &mut Vec<usize>) {
        if state[t] != 0 {
            return; // done, or a cycle (ignored)
        }
        state[t] = 1;
        for r in &tracks[t].routes {
            visit(r.dest, tracks, state, order);
        }
        state[t] = 2;
        order.push(t);
    }
    let mut state = vec![0u8; tracks.len()];
    let mut order = Vec::with_capacity(tracks.len());
    for t in 0..tracks.len() {
        visit(t, tracks, &mut state, &mut order);
    }
    order.reverse(); // sources first, destinations (Master) last
    order
}
