// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! FreqBat sound: oscillators, envelopes, LFOs, filter, and voices using os-dsp voice pool.

use std::f64::consts::PI;
use std::sync::Arc;

use os_dsp::articulator::{Record, Timing, Voice as ArtVoice, Shared as ArtShared, Parts};
use os_dsp::voice_pool::{VoicePool, VoiceContext, MAX_VOICES_PER_CHANNEL};
use os_engine::{envelope, laws};

/// Key 60 sounds C6 (spec §2.1).
pub const KEY60_HZ: f64 = 523.251_130_601_197_2;
/// The closing fade's length: 441 samples at every sample rate.
pub const FADE_SAMPLES: u64 = 441;
/// Modulation (envelopes, LFOs, pitch, filter) is updated every this many samples.
pub const CTL: usize = 32;
/// A backstop only: the host keeps the channel's voice limit and ends the oldest held note.
pub const MAX_VOICES: usize = 512;

pub const T_PAN: usize = 0;
pub const T_VOL: usize = 1;
pub const T_CUT: usize = 2;
pub const T_RES: usize = 3;
pub const T_PITCH: usize = 4;

/// Shapes (spec §2.2).
pub const SINE: u8 = 0;
pub const TRIANGLE: u8 = 1;
pub const SQUARE: u8 = 2;
pub const SAW: u8 = 3;
pub const ROUNDED_SAW: u8 = 4;
pub const NOISE: u8 = 5;

/// Rounded saw: time constant (periods) and where the wave drops to −1.
const RS_TAU: f64 = 1.0 / 48.0;
const RS_DROP: f64 = 5.0 / 6.0;

/// One oscillator's settings.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Osc {
    pub shape: u8,
    pub invert: bool,
    /// −1 (left) .. 1 (right).
    pub pan: f64,
    /// Coarse + fine pitch in semitones.
    pub semis: f64,
    /// Right side's phase lead in cycles (stereo phase offset: ±½).
    pub phase: f64,
    /// Right side's detune in cents.
    pub detune: f64,
}

/// One LFO (spec §2.6).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Lfo {
    /// 0 sine, 1 triangle, 2 pulse.
    pub shape: u8,
    pub delay: f64,
    pub attack: f64,
    /// Stored amount (pitch: cents; others ±128).
    pub amount: f64,
    pub hz: f64,
}

impl Lfo {
    fn wave(&self, ph: f64) -> f64 {
        let ph = ph.fract();
        match self.shape {
            0 => (2.0 * PI * ph).sin(),
            1 => 1.0 - 4.0 * ((ph + 0.25).fract() - 0.5).abs(),
            _ => {
                if ph < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
        }
    }

    /// The LFO's wave (−1..1, faded in) at voice time `t`, times the amount.
    pub fn value(&self, t: f64) -> f64 {
        if self.amount == 0.0 || t < self.delay {
            return 0.0;
        }
        let tl = t - self.delay;
        let fade = if self.attack > 0.0 { (tl / self.attack).min(1.0) } else { 1.0 };
        self.amount * fade * self.wave(self.hz * tl)
    }
}

/// LFO speed law: the knob holds the period, `(1001^(speed / 65536) - 1) * 64` ms and at least 3 ticks of
/// `tick` seconds, so a higher value is slower; 32950 is 2 s.
pub fn lfo_hz(speed: i32, tick: f64) -> f64 {
    let ms = (1001f64.powf(f64::from(speed) / 65536.0) - 1.0) * 64.0;
    1.0 / (ms / 1000.0).max(3.0 * tick).max(1e-6)
}

/// An envelope / LFO record's settings.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Target {
    /// The envelope when switched on (its `amount` = the stored amount).
    pub env: Option<envelope::Params>,
    pub lfo: Lfo,
}

impl Target {
    /// Envelope level × amount plus the LFO at time `t` (stored units), `None` once a volume envelope has ended.
    fn mod_at(&self, t: f64, off: Option<f64>) -> f64 {
        let e = self.env.map_or(0.0, |e| e.amount * e.level(t, off).unwrap_or(0.0));
        e + self.lfo.value(t)
    }

    fn modulates(&self) -> bool {
        self.env.is_some_and(|e| e.amount != 0.0) || self.lfo.amount != 0.0
    }
}

/// Settings read once per block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Block {
    pub osc: [Osc; 3],
    /// Osc 2 / osc 3 mix level, 0..1.
    pub m2: f64,
    pub m3: f64,
    /// Osc 3 = amplitude modulation.
    pub am: bool,
    pub hq: bool,
    /// Stereo phase randomness 0..1.
    pub rand: f64,
    pub targets: [Target; 5],
    pub filter: u8,
    /// Stored cutoff / resonance, 0..256.
    pub cutoff: f64,
    pub resonance: f64,
    /// Pitch bend in cents.
    pub bend: f64,
}

impl Default for Block {
    fn default() -> Self {
        let osc = |semis: f64| Osc { semis, ..Osc::default() };
        Block {
            osc: [osc(0.0), osc(-12.0), osc(-24.0)],
            m2: 0.5,
            m3: 0.25,
            am: false,
            hq: false,
            rand: 0.0,
            targets: [Target::default(); 5],
            filter: 0,
            cutoff: 256.0,
            resonance: 0.0,
            bend: 0.0,
        }
    }
}

impl Block {
    /// No filter: cutoff 256, resonance 0 and nothing modulating them (spec §2.7).
    pub fn filter_off(&self) -> bool {
        self.cutoff >= 256.0 && self.resonance <= 0.0 && !self.targets[T_CUT].modulates() && !self.targets[T_RES].modulates()
    }
}

/// A patch is just the parameters baked into a Block plus version.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Patch {
    pub version: u64,
    pub block: Block,
}

// ---------------------------------------------------------------------------------------------------------------
// Waves (spec §2.2)

/// The wave at phase `u` (0..1).
pub fn wave(shape: u8, u: f64) -> f64 {
    match shape {
        SINE => (2.0 * PI * u).sin(),
        TRIANGLE => {
            if u < 0.25 {
                4.0 * u
            } else if u < 0.75 {
                2.0 - 4.0 * u
            } else {
                4.0 * u - 4.0
            }
        }
        SQUARE => {
            if u < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        SAW => 2.0 * u - 1.0,
        ROUNDED_SAW => {
            if u < RS_DROP {
                1.0 - 2.0 * (-u / RS_TAU).exp()
            } else {
                -1.0
            }
        }
        _ => 0.0,
    }
}

/// The integral of one period's wave from 0 to `u` (0..=1).
fn prim1(shape: u8, u: f64) -> f64 {
    match shape {
        SINE => (1.0 - (2.0 * PI * u).cos()) / (2.0 * PI),
        TRIANGLE => {
            if u < 0.25 {
                2.0 * u * u
            } else if u < 0.75 {
                2.0 * u - 2.0 * u * u - 0.25
            } else {
                2.0 * u * u - 4.0 * u + 2.0
            }
        }
        SQUARE => {
            if u < 0.5 {
                u
            } else {
                1.0 - u
            }
        }
        SAW => u * u - u,
        ROUNDED_SAW => {
            let p = |u: f64| u - 2.0 * RS_TAU * (1.0 - (-u / RS_TAU).exp());
            if u < RS_DROP {
                p(u)
            } else {
                p(RS_DROP) - (u - RS_DROP)
            }
        }
        _ => 0.0,
    }
}

/// The integral of the periodic wave from 0 to `x` (any x ≥ 0).
fn prim(shape: u8, x: f64) -> f64 {
    let n = x.floor();
    n * prim1(shape, 1.0) + prim1(shape, x - n)
}

/// The wave's average over the next sample interval (phase `u` .. `u + dt`): the plain oscillators (spec §2.2 "Edges").
/// That is FL 26's plugin; FL 24.1's exports show a table lookup without averaging (spec: plugins/3x-osc-dsp.md), and
/// which is right is open.
pub fn boxed(shape: u8, u: f64, dt: f64) -> f64 {
    if dt < 1e-9 {
        return wave(shape, u);
    }
    (prim(shape, u + dt) - prim(shape, u)) / dt
}

/// Band-limited step correction (polyBLEP) for a unit jump at phase 0.
fn blep(t: f64, dt: f64) -> f64 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// The high-quality oscillators: the wave with band-limited edges (Lemon Studio choice: polyBLEP).
pub fn hq(shape: u8, u: f64, dt: f64) -> f64 {
    let dt = dt.min(0.5);
    match shape {
        SAW => 2.0 * u - 1.0 - blep(u, dt),
        SQUARE => wave(SQUARE, u) + blep(u, dt) - blep((u + 0.5).fract(), dt),
        ROUNDED_SAW => wave(ROUNDED_SAW, u) - blep((u - RS_DROP + 1.0).fract(), dt),
        _ => wave(shape, u),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Filter (spec §2.7)

/// Log interpolation through `(x, y)` points, extrapolated with the end segments.
fn log_interp(pts: &[(f64, f64)], x: f64) -> f64 {
    let n = pts.len();
    let i = pts.windows(2).position(|w| x <= w[1].0).unwrap_or(n - 2);
    let (x0, y0) = pts[i];
    let (x1, y1) = pts[i + 1];
    (y0.ln() + (y1.ln() - y0.ln()) * (x - x0) / (x1 - x0)).exp()
}

/// Natural frequency of the LP-family types (1..5) for a stored cutoff.
pub fn lp_hz(c: f64) -> f64 {
    log_interp(&[(32.0, 405.0), (64.0, 837.0), (128.0, 2727.0), (192.0, 8203.0)], c)
}

/// Damping of the LP-family types for a stored resonance.
pub fn lp_damping(r: f64) -> f64 {
    log_interp(&[(0.0, 10.0), (128.0, 0.949), (256.0, 0.227)], r)
}

/// Fast LP coefficient at 44.1 kHz for a stored cutoff (one-pole `y += a (x − y)`).
pub fn fast_lp_a(c: f64) -> f64 {
    (0.00655 * 5.975f64.powf((c - 64.0) / 64.0)).min(1.0)
}

/// SVF types (6, 7): natural frequency and Q.
pub fn svf_hz(c: f64) -> f64 {
    280.0 * 3.9f64.powf((c - 64.0) / 64.0)
}

pub fn svf_q(r: f64) -> f64 {
    log_interp(&[(0.0, 0.75), (128.0, 1.6), (192.0, 2.4)], r)
}

/// The SVF types' response floor (−43 dB at Nyquist).
const SVF_FLOOR: f64 = 0.007;

/// Trapezoidal state-variable filter, one channel.
#[derive(Clone, Copy, Debug, Default)]
struct Svf {
    ic1: f64,
    ic2: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct SvfCoefs {
    a1: f64,
    a2: f64,
    a3: f64,
    k: f64,
}

impl SvfCoefs {
    fn new(hz: f64, k: f64, sr: f64) -> Self {
        let g = (PI * hz.clamp(5.0, sr * 0.49) / sr).tan();
        let a1 = 1.0 / (1.0 + g * (g + k));
        SvfCoefs { a1, a2: g * a1, a3: g * g * a1, k }
    }
}

impl Svf {
    /// (low, band, high).
    #[inline]
    fn tick(&mut self, c: &SvfCoefs, x: f64) -> (f64, f64, f64) {
        let v3 = x - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, v1, x - c.k * v1 - v2)
    }
}

/// A filter setting for one control span.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Coefs {
    Off,
    OnePole(f64),
    /// Coefficients, output (1 LP, 2 BP, 3 HP, 4 BS, 5 LPx2, 6 SVF LP, 7 SVF LPx2).
    Svf(SvfCoefs, u8),
}

fn coefs(kind: u8, c: f64, r: f64, sr: f64) -> Coefs {
    let (c, r) = (c.clamp(0.0, 256.0), r.clamp(0.0, 256.0));
    match kind {
        0 => {
            let a = fast_lp_a(c);
            if a >= 1.0 {
                return Coefs::OnePole(1.0);
            }
            // Same corner frequency at any rate.
            let fc = -(1.0 - a).ln() * 44100.0 / (2.0 * PI);
            Coefs::OnePole(1.0 - (-2.0 * PI * fc / sr).exp())
        }
        1..=5 => Coefs::Svf(SvfCoefs::new(lp_hz(c), lp_damping(r), sr), kind),
        _ => Coefs::Svf(SvfCoefs::new(svf_hz(c), 1.0 / svf_q(r), sr), kind.min(7)),
    }
}

/// One side's filter memory.
#[derive(Clone, Copy, Debug, Default)]
struct Filter {
    y: f64,
    s1: Svf,
    s2: Svf,
}

impl Filter {
    #[inline]
    fn tick(&mut self, c: &Coefs, x: f64) -> f64 {
        match c {
            Coefs::Off => x,
            Coefs::OnePole(a) => {
                self.y += a * (x - self.y);
                self.y
            }
            Coefs::Svf(k, kind) => {
                let (lp, bp, hp) = self.s1.tick(k, x);
                match kind {
                    1 => lp,
                    2 => k.k * bp,
                    3 => hp,
                    4 => lp + hp,
                    5 => self.s2.tick(k, lp).0,
                    6 => lp + SVF_FLOOR * x,
                    _ => {
                        let y = lp + SVF_FLOOR * x;
                        self.s2.tick(k, y).0 + SVF_FLOOR * y
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Voices (spec §2.3 - §2.5)

struct Voice {
    key: u8,
    id: Option<i32>,
    /// Started by a host note: its end is reported back (spec: audio/voices.md §4). Editor previews are not.
    host: bool,
    /// Velocity gain.
    vel: f64,
    /// Samples since the note-on.
    n: u64,
    /// Note-off sample.
    off: Option<u64>,
    /// Cut by the host (choke) at this sample: the 10 ms fade.
    cut: Option<u64>,
    /// The host's tuning of the voice in semitones (portamento and slide notes: a note expression).
    tune: f64,
    /// The note's own pan (a note expression).
    pan: f32,
    /// The note's mod X / mod Y (the host's per-note modulation of the cutoff and resonance), 1 = 256 knob steps.
    mods: [f64; 2],
    /// The gains of the final levels per side at the end of the last span; `None` before the first.
    sides: Option<[f64; 2]>,
    /// Phase per oscillator and side (cycles, 0..1).
    ph: [[f64; 2]; 3],
    filt: [Filter; 2],
    done: bool,
    /// The os-dsp articulator voice for per-voice envelopes and LFOs.
    art_voice: ArtVoice,
}

impl Voice {
    fn new(key: u8, id: Option<i32>, host: bool, vel: f64, records: &[Record; 5], timing: &[Timing; 5], art_shared: &ArtShared) -> Self {
        let mut ph = [[0.0; 2]; 3];
        // Phases will be set in Engine::start
        Voice {
            key,
            id,
            host,
            vel,
            n: 0,
            off: None,
            cut: None,
            tune: 0.0,
            pan: 0.0,
            mods: [0.0; 2],
            sides: None,
            ph,
            filt: [Filter::default(); 2],
            done: false,
            art_voice: ArtVoice::start(records, timing),
        }
    }
}

/// A final level split by a final pan: the host's pan law (spec: audio/conventions.md), 0.7071 of the level per side
/// at the centre.
fn split(pan: f32, level: f32) -> [f64; 2] {
    let lv = f64::from(level);
    let a = (f64::from(pan.clamp(-1.0, 1.0)) + 1.0) * (PI / 4.0);
    [f64::from((lv * a.cos()) as f32), f64::from((lv * a.sin()) as f32)]
}

/// The closing fade's factor on its sample `j` (from 0): `(cos(single(j + 1) * single(pi / 442)) + 1) * 0.5`.
fn fade(j: u64) -> f64 {
    if j >= FADE_SAMPLES {
        return 0.0;
    }
    let x = (j + 1) as f32 * (PI / 442.0) as f32;
    (f64::from(x).cos() + 1.0) * 0.5
}

pub struct Engine {
    rate: f64,
    voices: Vec<Voice>,
    rng: u64,
    /// Host voices that ended since the last `take_ended`: (key, host note id).
    ended: Vec<(u8, Option<i32>)>,
    /// The channel's five articulator records.
    records: [Record; 5],
    /// The channel's five articulator timings.
    timing: [Timing; 5],
    /// The shared phases of the records with the global LFO.
    art_shared: ArtShared,
    /// The voice pool for per-voice modulation (os-dsp).
    voice_pool: VoicePool,
    /// The channel's volume and pan ([`Engine::set_levels`]).
    levels: (f32, f32),
}

impl Engine {
    pub fn new(rate: f32) -> Self {
        Engine {
            rate: f64::from(rate),
            voices: Vec::with_capacity(MAX_VOICES),
            rng: 0x9e37_79b9_7f4a_7c15,
            ended: Vec::with_capacity(MAX_VOICES * 2),
            records: Record::new_channel_records(),
            timing: Timing::all(&Record::new_channel_records(), 96, 140.0),
            art_shared: ArtShared::default(),
            voice_pool: VoicePool::new(1), // Single channel for the instrument
            levels: (std::f32::consts::SQRT_2, 0.0),
        }
    }

    /// Sets the patch from the parameters.
    pub fn set_patch(&mut self, patch: Patch) {
        // Convert patch to records and timing
        let block = patch.block;
        self.records = block_to_records(&block);
        self.timing = Timing::all(&self.records, 96, 140.0); // tempo will be updated per block
        // Reset articulator shared state
        self.art_shared.reset();
        // Reinitialize all voices with new records/timing
        for v in &mut self.voices {
            v.art_voice = ArtVoice::start(&self.records, &self.timing);
        }
    }

    /// The channel's volume and pan, part of every voice's final levels (spec: audio/voice-levels.md "Final levels"):
    /// sounding voices glide to them over their next control span. Without a call they are √2 and 0, unity at the
    /// centre, so that in another host, whose own levels act on the output, the voices keep their level.
    pub fn set_levels(&mut self, volume: f32, pan: f32) {
        self.levels = (volume, pan);
    }

    pub fn set_rate(&mut self, rate: f32) {
        self.rate = f64::from(rate);
        self.voices.clear();
        self.ended.clear();
        self.voice_pool.clear();
    }

    /// Drops every voice. Not reported: a host resetting the instrument forgets its voices itself.
    pub fn reset(&mut self) {
        self.voices.clear();
        self.ended.clear();
        self.voice_pool.clear();
        self.art_shared.reset();
    }

    /// Host voices that ended since the last call: (key, host note id).
    pub fn take_ended(&mut self) -> std::vec::Drain<'_, (u8, Option<i32>)> {
        self.ended.drain(..)
    }

    pub fn has_voices(&self) -> bool {
        !self.voices.is_empty()
    }

    /// Keys of the voices still held or ringing.
    pub fn sounding(&self) -> impl Iterator<Item = u8> + '_ {
        self.voices.iter().filter(|v| !v.done).map(|v| v.key)
    }

    /// Uniform −1..1 (xorshift).
    fn noise(&mut self) -> f64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (x >> 11) as f64 / (1u64 << 52) as f64 - 1.0
    }

    /// Starts a voice of the plugin's own (the editor's keyboard): velocity 0..1 (the host's byte / 128).
    pub fn note_on(&mut self, key: u8, velocity: f32, id: Option<i32>, b: &Block) {
        self.start(key, velocity, id, false, b);
    }

    /// Starts the voice of a host note: its end is reported (spec: audio/voices.md §4).
    pub fn host_note_on(&mut self, key: u8, velocity: f32, id: Option<i32>, b: &Block) {
        self.start(key, velocity, id, true, b);
    }

    fn start(&mut self, key: u8, velocity: f32, id: Option<i32>, host: bool, b: &Block) {
        if self.voices.len() >= MAX_VOICES {
            let v = self.voices.remove(0);
            if v.host {
                self.ended.push((v.key, v.id));
            }
            // Also remove from voice pool
            if let Some(voice_id) = v.id {
                self.voice_pool.remove_voice(0, voice_id as u32);
            }
        }
        let mut ph = [[0.0; 2]; 3];
        for (n, p) in ph.iter_mut().enumerate() {
            let mut r = [0.0; 2];
            if b.rand > 0.0 {
                // Random start within ±(r/64)·½ cycle per side (spec §2.3).
                for x in &mut r {
                    *x = self.noise() * b.rand * 0.5;
                }
            }
            p[0] = r[0].rem_euclid(1.0);
            p[1] = (b.osc[n].phase + r[1]).rem_euclid(1.0);
        }
        let byte = (velocity.clamp(0.0, 1.0) * 128.0).round() as u8;
        let voice_id = self.voice_pool.note_on(0, key, byte) as i32;
        self.art_shared.note();
        let mut voice = Voice::new(key, id, host, laws::velocity_gain(byte), &self.records, &self.timing, &self.art_shared);
        voice.ph = ph;
        voice.id = Some(voice_id);
        self.voices.push(voice);
    }

    pub fn note_off(&mut self, key: u8) {
        for v in self.voices.iter_mut().filter(|v| v.key == key && v.off.is_none()) {
            v.off = Some(v.n);
        }
        self.voice_pool.note_off(0, key);
    }

    /// Note-off of one voice by its host id (falls back to the key).
    pub fn note_off_id(&mut self, key: u8, id: Option<i32>) {
        if let Some(i) = id {
            if let Some(v) = self.voices.iter_mut().find(|v| v.id == Some(i) && v.off.is_none()) {
                v.off = Some(v.n);
                self.voice_pool.remove_voice(0, i as u32);
                return;
            }
        }
        self.note_off(key);
    }

    /// Tunes the voices of `key` to `semitones` from the key's own pitch (a host's tuning expression: portamento and
    /// slide notes). With a host id only that voice is tuned; without one (or when no voice has it) every voice of the
    /// key, released ones too.
    pub fn tune(&mut self, key: u8, id: Option<i32>, semitones: f64) {
        if let Some(i) = id {
            if let Some(v) = self.voices.iter_mut().find(|v| v.id == Some(i)) {
                v.tune = semitones;
                return;
            }
        }
        for v in self.voices.iter_mut().filter(|v| v.key == key) {
            v.tune = semitones;
        }
    }

    /// A voice's own pan (a host's pan expression), the voice its id names or else every voice of `key`.
    pub fn pan(&mut self, key: u8, id: Option<i32>, pan: f32) {
        if let Some(i) = id {
            if let Some(v) = self.voices.iter_mut().find(|v| v.id == Some(i)) {
                v.pan = pan;
                return;
            }
        }
        for v in self.voices.iter_mut().filter(|v| v.key == key) {
            v.pan = pan;
        }
    }

    /// A voice's own mod X (`k` 0) or mod Y (`k` 1), the voice its id names: added to the cutoff or resonance knob,
    /// 256 steps per unit (spec: plugins/freqbat.md §6.1).
    pub fn modulate(&mut self, id: i32, k: usize, amount: f32) {
        for v in self.voices.iter_mut().filter(|v| v.id == Some(id)) {
            v.mods[k.min(1)] = f64::from(amount);
        }
    }

    /// Cuts the voices of `key` with the short fade.
    pub fn choke(&mut self, key: u8) {
        for v in self.voices.iter_mut().filter(|v| v.key == key && v.cut.is_none()) {
            v.cut = Some(v.n);
        }
        self.voice_pool.choke(0, key);
    }

    pub fn all_off(&mut self) {
        for v in self.voices.iter_mut().filter(|v| v.cut.is_none()) {
            v.cut = Some(v.n);
        }
        self.voice_pool.clear();
    }

    /// The sample on which a voice's closing fade starts: its note's end without a volume envelope, or a cut.
    fn closing(v: &Voice, b: &Block) -> Option<u64> {
        let off = if b.targets[T_VOL].env.is_none() { v.off } else { None };
        match (off, v.cut) {
            (Some(o), Some(c)) => Some(o.min(c)),
            (o, c) => o.or(c),
        }
    }

    /// Voice gain at sample `n` (velocity, volume envelope and LFO; the closing fade is applied per sample);
    /// `None` once the volume envelope is over.
    fn gain_at(&self, v: &Voice, b: &Block, n: u64) -> Option<f64> {
        let sr = self.rate;
        let t = n as f64 / sr;
        let off = v.off.map(|o| o as f64 / sr);
        let vol = &b.targets[T_VOL];
        let mut g = v.vel;
        if let Some(e) = vol.env {
            g *= e.level(t, off)?;
        }
        // Volume LFO: gain × (1 + a·w), a = amount / 128.
        g *= 1.0 + vol.lfo.value(t) / 128.0;
        Some(g)
    }

    /// Adds the voices into `l` / `r`.
    pub fn render(&mut self, l: &mut [f32], r: &mut [f32], b: &Block) {
        let len = l.len().min(r.len());
        let mut at = 0;
        while at < len {
            let end = (at + CTL).min(len);
            self.span(&mut l[at..end], &mut r[at..end], b);
            at = end;
        }
        let ended = &mut self.ended;
        self.voices.retain(|v| {
            if v.done && v.host {
                ended.push((v.key, v.id));
            }
            !v.done
        });
    }

    fn span(&mut self, l: &mut [f32], r: &mut [f32], b: &Block) {
        let len = l.len();
        let sr = self.rate;
        // Constant-power pan gains per oscillator and side.
        let pan: [[f64; 2]; 3] = std::array::from_fn(|n| {
            let a = (1.0 + b.osc[n].pan.clamp(-1.0, 1.0)) * PI / 4.0;
            [a.cos(), a.sin()]
        });
        // Mix weights: osc 1, osc 2, osc 3 (AM: osc 3 modulates instead).
        let w = [1.0 - b.m2, b.m2];
        let (use1, use2) = (w[0] != 0.0, w[1] != 0.0);
        let use3 = b.m3 != 0.0;
        let filter_off = b.filter_off();
        let (volume, lpan) = self.levels;
        
        // Update articulator timing for current tempo
        let tick = 60.0 / (b.bend.abs().max(1.0) * 96.0); // placeholder, will be overridden by block()
        let tempo = 140.0; // from block
        self.timing = Timing::all(&self.records, 96, tempo);
        
        // Step articulator shared state
        self.art_shared.tick(&self.timing);
        let shared_phases = self.art_shared.phases(&self.records);
        
        let mut voices = std::mem::take(&mut self.voices);
        for v in voices.iter_mut().filter(|v| !v.done) {
            // Step the os-dsp articulator voice
            v.art_voice.step(&self.records, &self.timing, &shared_phases);
            let parts = v.art_voice.parts(&self.records);
            
            let t0 = v.n as f64 / sr;
            let off = v.off.map(|o| o as f64 / sr);
            let close = Self::closing(v, b);
            let Some(g0) = self.gain_at(v, b, v.n).filter(|_| close.is_none_or(|c| v.n < c + FADE_SAMPLES)) else {
                v.done = true;
                continue;
            };
            // A voice that ends inside this span fades to nothing in it.
            let g1 = self.gain_at(v, b, v.n + len as u64).unwrap_or_else(|| {
                v.done = true;
                0.0
            });
            // Pitch: envelope (cents) + LFO (cents) + bend + the host's tuning.
            let cents = b.targets[T_PITCH].mod_at(t0, off) + b.bend + v.tune * 100.0;
            let base = KEY60_HZ * 2f64.powf((f64::from(v.key) - 60.0) / 12.0 + cents / 1200.0);
            let inc: [[f64; 2]; 3] = std::array::from_fn(|n| {
                let f = base * 2f64.powf(b.osc[n].semis / 12.0) / sr;
                [f, f * 2f64.powf(b.osc[n].detune / 1200.0)]
            });
            let co = if filter_off && v.mods == [0.0; 2] {
                Coefs::Off
            } else {
                let c = b.cutoff + v.mods[0] * 256.0 + b.targets[T_CUT].mod_at(t0, off) / 128.0 * 256.0;
                let q = b.resonance + v.mods[1] * 256.0 + b.targets[T_RES].mod_at(t0, off) / 128.0 * 256.0;
                coefs(b.filter, c, q, sr)
            };
            let dg = (g1 - g0) / len as f64;
            // The final levels: the note's pan plus the channel's, split with the channel's volume (the note's volume is
            // the velocity in `g`); a change glides over the span.
            let to = split(v.pan + lpan, volume);
            let from = v.sides.replace(to).unwrap_or(to);
            let ds = [(to[0] - from[0]) / len as f64, (to[1] - from[1]) / len as f64];
            for i in 0..len {
                let a = v.n + i as u64;
                let g = (g0 + dg * i as f64) * close.map_or(1.0, |c| if a < c { 1.0 } else { fade(a - c) });
                let k = (i + 1) as f64;
                let side = [from[0] + ds[0] * k, from[1] + ds[1] * k];
                let mut out = [0.0f64; 2];
                for (s, o) in out.iter_mut().enumerate() {
                    let mut osc = [0.0f64; 3];
                    for n in 0..3 {
                        let used = match n {
                            0 => use1,
                            1 => use2,
                            _ => use3,
                        };
                        let os = &b.osc[n];
                        let u = v.ph[n][s];
                        let dt = inc[n][s];
                        if used {
                            let x = match os.shape {
                                NOISE => self.noise(),
                                sh if b.hq => hq(sh, u, dt),
                                sh => boxed(sh, u, dt),
                            };
                            osc[n] = if os.invert { -x } else { x } * pan[n][s];
                        }
                        let p = u + dt;
                        v.ph[n][s] = p - p.floor();
                    }
                    let mix12 = w[0] * osc[0] + w[1] * osc[1];
                    let y = if b.am { mix12 * (1.0 - b.m3 * osc[2]) } else { (1.0 - b.m3) * mix12 + b.m3 * osc[2] };
                    *o = v.filt[s].tick(&co, y) * g * side[s];
                }
                l[i] += out[0] as f32;
                r[i] += out[1] as f32;
            }
            v.n += len as u64;
            if close.is_some_and(|c| v.n >= c + FADE_SAMPLES) {
                v.done = true;
                if let Some(voice_id) = v.id {
                    self.voice_pool.remove_voice(0, voice_id as u32);
                }
            }
        }
        self.voices = voices;
    }
}

fn block_to_records(block: &Block) -> [Record; 5] {
    let mut records = Record::new_channel_records();
    for (i, target) in block.targets.iter().enumerate() {
        if let Some(env) = target.env {
            records[i].on = env.enabled;
            records[i].delay = (env.delay * 65536.0) as i32;
            records[i].attack = (env.attack * 65536.0) as i32;
            records[i].hold = (env.hold * 65536.0) as i32;
            records[i].decay = (env.decay * 65536.0) as i32;
            records[i].sustain = (env.sustain * 128.0) as i32;
            records[i].release = (env.release * 65536.0) as i32;
            records[i].amount = (env.amount * 128.0) as i32;
        }
        records[i].lfo_delay = (target.lfo.delay * 65536.0) as i32;
        records[i].lfo_attack = (target.lfo.attack * 65536.0) as i32;
        records[i].lfo_amount = (target.lfo.amount * 128.0) as i32;
        records[i].lfo_speed = (target.lfo.hz * 65536.0) as i32; // approximate
        records[i].lfo_shape = target.lfo.shape as i32;
    }
    records
}

/// Builds a Block from the plugin parameters.
pub fn block(p: &crate::params::FreqBatParams, bend: f64, tick: f64) -> Block {
    // For now, return a default block. The actual parameter mapping will be done in params.rs
    // when the parameter definitions are added.
    let mut b = Block::default();
    b.bend = bend;
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44100.0;

    fn render(b: &Block, key: u8, vel: f32, frames: usize, off_at: Option<usize>) -> (Vec<f32>, Vec<f32>) {
        let mut e = Engine::new(SR);
        e.records = block_to_records(b);
        e.timing = Timing::all(&e.records, 96, 140.0);
        e.note_on(key, vel, None, b);
        let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
        match off_at {
            Some(o) => {
                e.render(&mut l[..o], &mut r[..o], b);
                e.note_off(key);
                e.render(&mut l[o..], &mut r[o..], b);
            }
            None => e.render(&mut l, &mut r, b),
        }
        (l, r)
    }

    /// Only osc 1 sounds: mix levels 0.
    fn solo(shape: u8) -> Block {
        let mut b = Block { m2: 0.0, m3: 0.0, ..Block::default() };
        b.osc[0].shape = shape;
        b
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
    }

    /// Frequency by zero crossings (rising) over a stretch.
    fn freq(x: &[f32]) -> f64 {
        let ups: Vec<f64> = (1..x.len())
            .filter(|&i| x[i - 1] < 0.0 && x[i] >= 0.0)
            .map(|i| (i - 1) as f64 + f64::from(-x[i - 1] / (x[i] - x[i - 1])))
            .collect();
        f64::from(SR) * (ups.len() - 1) as f64 / (ups[ups.len() - 1] - ups[0])
    }

    #[test]
    fn one_oscillator_peaks_at_one_before_pan_and_velocity_100_is_unity() {
        let (l, r) = render(&solo(SINE), 60, 100.0 / 128.0, 4410, None);
        assert!((peak(&l) - 0.70711).abs() < 2e-3, "{}", peak(&l));
        assert!((peak(&r) - 0.70711).abs() < 2e-3);
        // Velocity 50: the sampler's law, 0.23165.
        let (l, _) = render(&solo(SINE), 60, 50.0 / 128.0, 4410, None);
        assert!((peak(&l) / 0.70711 - 0.23165).abs() < 2e-3);
    }

    #[test]
    fn pitch_follows_the_measured_formula() {
        let (l, _) = render(&solo(SINE), 60, 0.78, 44100, None);
        assert!((freq(&l) - 523.25).abs() < 0.05, "{}", freq(&l));
        let (l, _) = render(&solo(SINE), 48, 0.78, 44100, None);
        assert!((freq(&l) - 261.63).abs() < 0.05);
        let mut b = solo(SINE);
        b.osc[0].semis = 7.0 + 0.5;
        let (l, _) = render(&b, 60, 0.78, 44100, None);
        assert!((freq(&l) - 806.96).abs() < 0.1, "{}", freq(&l));
        // Stereo detune moves only the right side.
        b = solo(SINE);
        b.osc[0].detune = 50.0;
        let (l, r) = render(&b, 60, 0.78, 44100, None);
        assert!((freq(&l) - 523.25).abs() < 0.05);
        assert!((freq(&r) - 538.58).abs() < 0.1, "{}", freq(&r));
    }
}