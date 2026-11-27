// Copyright (C) 2026 Chimthuwu
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Persistent parameter identity for the DAW-wide modulation system (M0).
//!
//! This module defines the project-persistent identity types that the modulation graph (later
//! milestones) will store. It is intentionally separate from [`crate::target`]: the existing
//! `Target` encoding and the original `.flp` `AUTO_LINK` record are frozen (see the project's
//! modulation audit, K.1). Everything here is persisted in Lemon Studio's own `.lemon` extension
//! (K.3), never inside legacy project target bytes.
//!
//! Two identities exist:
//! - a *device/owner* identity ([`PersistentObjectId`]), stable across save/load and device moves;
//! - a *parameter* identity ([`ParameterKey`]), which reuses the legacy project-compatible parameter code
//!   space wherever possible rather than inventing a competing host-control scheme.

use std::collections::BTreeMap;

/// A project-persistent identity for a device/owner object that owns parameters: a channel's
/// instrument, a mixer track's host parameters (volume/pan/EQ/send), a mixer-slot hosted plugin,
/// or a container member.
///
/// Unlike [`crate::Id`] (session-only, reassigned whenever a project is opened), a
/// `PersistentObjectId` is saved in the `.lemon` extension and travels with the device through
/// supported moves. `0` ([`PERSISTENT_NONE`]) means "not yet assigned".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct PersistentObjectId(pub u32);

/// The reserved value for "no persistent identity assigned yet".
pub const PERSISTENT_NONE: PersistentObjectId = PersistentObjectId(0);

impl PersistentObjectId {
    pub fn is_none(self) -> bool {
        self == PERSISTENT_NONE
    }
}

/// Where an owner lives in the project, expressed positionally. This is the *current* location of a
/// device; the [`PersistentObjectId`] it maps to is what survives moves. The discriminant matches
/// how the existing `Target` already addresses the placement, so a `ParameterPath` can be bridged
/// to/from a `Target` without inventing new addressing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OwnerLocator {
    /// A channel's instrument. `index` is the channel's stable `Channel.index` (also used by
    /// `ChannelParam`).
    Channel(u16),
    /// A mixer track's host parameters (volume/pan/EQ/send). `index` is `MixerTrack.index`.
    Track(u16),
    /// A mixer-slot hosted plugin: `(track.index, slot)`.
    Slot(u16, u8),
    /// A container member. `(track, slot, member_id)` for a mixer container, or
    /// `(rack_row, CHANNEL, member_id)` for a channel-container (matches `container::CHANNEL = 0xFF`).
    Member(u16, u8, u32),
}

/// The parameter within an owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParameterKey {
    /// Native plugin parameter id: CLAP `clap_id` or VST3 `ParamID`. Stable across plugin
    /// parameter reordering, so it is the preferred identity for hosted plugins.
    PluginNativeId(u32),
    /// A legacy positional target (legacy project-compatible `0x8000 + index`). Permanent: a legacy target is
    /// never silently upgraded to a native binding on load or save (K.2).
    LegacyPluginIndex(u16),
    /// An existing host-control code (the lower 16 bits of a `Target`): volume, pan, EQ, send, and
    /// so on. Reuses the current channel/mixer parameter conventions; we must not invent a second
    /// set of host volume/pan/EQ/send identifiers.
    HostParameterCode(u16),
    /// A modulator's own parameter, for a route *into* the modulation graph ([`ModParam`], [`ModGraph::route`]). The
    /// path's owner is unused ([`PERSISTENT_NONE`]): a modulator is not a device and never moves, so its id is the
    /// whole identity.
    ModulatorParam(ModulatorId, ModParam),
    /// The depth of one route — one assignment a modulator has ([`ModTarget::route`]) — so a modulator can shape how
    /// far another one drives a parameter. The path's owner is unused, like [`ParameterKey::ModulatorParam`].
    RouteDepth(RouteId),
}

/// A parameter of a modulator that another modulator can drive (the handoff's modulator-to-modulator routing).
///
/// Each one is a fraction of the source's own setting, exactly like a device parameter's normalized value, so a route
/// keeps its depth and curve whichever source ends up on either end. Not every source has every parameter:
/// [`ModParam::applies_to`] says which, and [`ModGraph::route`] refuses a route to one it does not have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModParam {
    /// How fast it cycles: an LFO's hertz, a random source's rate, a step sequence's step length.
    Rate,
    /// The source's own output gain.
    Amount,
    /// An LFO's start position in its cycle.
    Phase,
    /// An LFO's constant shift added to its output.
    Offset,
    /// A follower's rise time, in seconds.
    Attack,
    /// A follower's fall time, in seconds.
    Release,
    /// A follower's detector gain, in decibels.
    Gain,
    /// A random source's slew from the previous cycle's value: `0` is a hard step, `1` glides the whole cycle.
    Smooth,
    /// A macro's position, `0..=1`.
    Value,
    /// A voice ADSR's attack time, in seconds.
    AttackV,
    /// A voice ADSR's decay time, in seconds.
    Decay,
    /// A voice ADSR's sustain level, `0..=1`.
    Sustain,
    /// A voice ADSR's release time, in seconds.
    ReleaseV,
    /// Math operator's operation selector.
    MathOp,
    /// Polynomial coefficients (index 0..5 for c0..c5).
    PolyCoeff(u8),
    /// Quantize step count.
    QuantSteps,
    /// Map breakpoints (index 0..7 for point index).
    MapPoint(u8),
    /// Slew rise time.
    SlewRise,
    /// Slew fall time.
    SlewFall,
}

impl ModParam {
    /// Every parameter, in menu order. Not every source has every one of them: see [`ModParam::applies_to`].
    pub const ALL: [ModParam; 31] = [
        ModParam::Rate,
        ModParam::Amount,
        ModParam::Phase,
        ModParam::Offset,
        ModParam::Attack,
        ModParam::Release,
        ModParam::Gain,
        ModParam::Smooth,
        ModParam::Value,
        ModParam::AttackV,
        ModParam::Decay,
        ModParam::Sustain,
        ModParam::ReleaseV,
        ModParam::MathOp,
        ModParam::PolyCoeff(0),
        ModParam::PolyCoeff(1),
        ModParam::PolyCoeff(2),
        ModParam::PolyCoeff(3),
        ModParam::PolyCoeff(4),
        ModParam::PolyCoeff(5),
        ModParam::QuantSteps,
        ModParam::MapPoint(0),
        ModParam::MapPoint(1),
        ModParam::MapPoint(2),
        ModParam::MapPoint(3),
        ModParam::MapPoint(4),
        ModParam::MapPoint(5),
        ModParam::MapPoint(6),
        ModParam::MapPoint(7),
        ModParam::SlewRise,
        ModParam::SlewFall,
    ];

    /// The byte a `.lemon` file stores for it. The first four are the original handoff's; the rest arrived with the
    /// followers' and the random source's own parameters. Voice ADSR parameters are new in layout 4. Operator node
    /// parameters are new in layout 5.
    pub fn code(self) -> u8 {
        match self {
            ModParam::Rate => 0,
            ModParam::Amount => 1,
            ModParam::Phase => 2,
            ModParam::Offset => 3,
            ModParam::Attack => 4,
            ModParam::Release => 5,
            ModParam::Gain => 6,
            ModParam::Smooth => 7,
            ModParam::Value => 8,
            ModParam::AttackV => 9,
            ModParam::Decay => 10,
            ModParam::Sustain => 11,
            ModParam::ReleaseV => 12,
            ModParam::MathOp => 13,
            ModParam::PolyCoeff(i) => 14 + i,
            ModParam::QuantSteps => 20,
            ModParam::MapPoint(i) => 21 + i,
            ModParam::SlewRise => 29,
            ModParam::SlewFall => 30,
        }
    }

    /// The parameter [`ModParam::code`] writes (`None` for a byte this version does not know).
    pub fn of(code: u8) -> Option<ModParam> {
        Some(match code {
            0 => ModParam::Rate,
            1 => ModParam::Amount,
            2 => ModParam::Phase,
            3 => ModParam::Offset,
            4 => ModParam::Attack,
            5 => ModParam::Release,
            6 => ModParam::Gain,
            7 => ModParam::Smooth,
            8 => ModParam::Value,
            9 => ModParam::AttackV,
            10 => ModParam::Decay,
            11 => ModParam::Sustain,
            12 => ModParam::ReleaseV,
            13 => ModParam::MathOp,
            14..=19 => ModParam::PolyCoeff(code - 14),
            20 => ModParam::QuantSteps,
            21..=28 => ModParam::MapPoint(code - 21),
            29 => ModParam::SlewRise,
            30 => ModParam::SlewFall,
            _ => return None,
        })
    }

    /// Its name in the interface.
    pub fn label(self) -> &'static str {
        match self {
            ModParam::Rate => "Rate",
            ModParam::Amount => "Amount",
            ModParam::Phase => "Phase",
            ModParam::Offset => "Offset",
            ModParam::Attack => "Attack",
            ModParam::Release => "Release",
            ModParam::Gain => "Gain",
            ModParam::Smooth => "Smooth",
            ModParam::Value => "Value",
            ModParam::AttackV => "Attack",
            ModParam::Decay => "Decay",
            ModParam::Sustain => "Sustain",
            ModParam::ReleaseV => "Release",
            ModParam::MathOp => "Operation",
            ModParam::PolyCoeff(i) => match i { 0 => "Coeff 0", 1 => "Coeff 1", 2 => "Coeff 2", 3 => "Coeff 3", 4 => "Coeff 4", 5 => "Coeff 5", _ => "Coeff" },
            ModParam::QuantSteps => "Steps",
            ModParam::MapPoint(i) => match i { 0 => "Point 0", 1 => "Point 1", 2 => "Point 2", 3 => "Point 3", 4 => "Point 4", 5 => "Point 5", 6 => "Point 6", 7 => "Point 7", _ => "Point" },
            ModParam::SlewRise => "Rise",
            ModParam::SlewFall => "Fall",
        }
    }

    /// Whether `source` has this parameter at all.
    ///
    /// An LFO has all of its four (rate, amount, phase, offset); a random source has a rate, an amount and its
    /// smoothing; a step sequence has a rate alone; an MSEG has a rate and an amount (its shape is its own); a macro has
    /// nothing but its position; a follower has an amount, an attack, a release and a gain (it follows the master
    /// rather than a period, so it has no rate); a one-shot envelope has none of them, since it follows the transport.
    /// A voice ADSR has attack, decay, sustain, release and amount.
    /// Operator nodes (Math, Polynomial, Quantize, Map, Slew) have their own parameters plus amount.
    pub fn applies_to(self, source: &ModSource) -> bool {
        match source {
            ModSource::Lfo(_) => matches!(self, ModParam::Rate | ModParam::Amount | ModParam::Phase | ModParam::Offset),
            ModSource::Random(_) => matches!(self, ModParam::Rate | ModParam::Amount | ModParam::Smooth),
            ModSource::Steps(_) => matches!(self, ModParam::Rate),
            ModSource::Mseg(_) => matches!(self, ModParam::Rate | ModParam::Amount),
            ModSource::Macro(_) => matches!(self, ModParam::Value),
            // A note source's amount is its only setting: what it reads (a channel's last note) is not a parameter of
            // the modulator, so nothing else routes into it.
            ModSource::Velocity(_) | ModSource::KeyTrack(_) => matches!(self, ModParam::Amount),
            ModSource::AudioFollower(_) => matches!(self, ModParam::Amount | ModParam::Attack | ModParam::Release | ModParam::Gain),
            ModSource::Envelope(_) => false,
            ModSource::VoiceAdsr(_) => matches!(self, ModParam::AttackV | ModParam::Decay | ModParam::Sustain | ModParam::ReleaseV | ModParam::Amount),
            ModSource::Math(_) => matches!(self, ModParam::MathOp | ModParam::Amount),
            ModSource::Polynomial(_) => matches!(self, ModParam::PolyCoeff(_) | ModParam::Amount),
            ModSource::Quantize(_) => matches!(self, ModParam::QuantSteps | ModParam::Amount),
            ModSource::Map(_) => matches!(self, ModParam::MapPoint(_) | ModParam::Amount),
            ModSource::Slew(_) => matches!(self, ModParam::SlewRise | ModParam::SlewFall | ModParam::Amount),
        }
    }
}

/// A persistent, project-wide parameter identity. This is what the modulation graph stores for each
/// assignment; it is resolved to a concrete [`crate::target::Target`] at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParameterPath {
    pub owner: PersistentObjectId,
    pub parameter: ParameterKey,
}

impl ParameterPath {
    /// The path to a parameter of modulator `id` (an edge inside the graph rather than a device parameter).
    ///
    /// The modulator is its own owner, so nothing here depends on the identity registry: a route into the graph cannot
    /// be lost by a device move, and it cannot outlive the modulator it names ([`ModGraph::remove`] strips it).
    pub fn on_modulator(id: ModulatorId, param: ModParam) -> ParameterPath {
        ParameterPath { owner: PERSISTENT_NONE, parameter: ParameterKey::ModulatorParam(id, param) }
    }

    /// The path to the depth of route `route` ([`ModTarget::route`]): a route's own depth as a destination.
    pub fn on_route(route: RouteId) -> ParameterPath {
        ParameterPath { owner: PERSISTENT_NONE, parameter: ParameterKey::RouteDepth(route) }
    }

    /// The modulator and its parameter this path names, if it is a route inside the graph.
    pub fn modulator(&self) -> Option<(ModulatorId, ModParam)> {
        match self.parameter {
            ParameterKey::ModulatorParam(id, param) => Some((id, param)),
            _ => None,
        }
    }

    /// The route whose depth this path names, if it is one.
    pub fn route(&self) -> Option<RouteId> {
        match self.parameter {
            ParameterKey::RouteDepth(route) => Some(route),
            _ => None,
        }
    }

    /// Whether this path names something inside the modulation graph (a modulator's parameter or a route's depth)
    /// rather than a device parameter.
    pub fn in_graph(&self) -> bool {
        matches!(self.parameter, ParameterKey::ModulatorParam(..) | ParameterKey::RouteDepth(_))
    }
}

/// The result of mapping a parameter identity against the *current* plugin parameter metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamResolution {
    /// Resolved to a positional parameter index; `index` is valid in the plugin's current list.
    Unique(u16),
    /// The native id no longer exists in the plugin's parameter list.
    Missing,
    /// The native id appears more than once. `index` is the deterministic first match; the
    /// ambiguity must be surfaced to the user rather than silently resolved.
    Duplicate(u16),
}

impl ParamResolution {
    /// Resolves a native parameter id against a plugin's current parameter metadata, given the native
    /// ids in index order (index `i` is the positional parameter `i`). Returns `Missing` when the id is
    /// absent, `Unique(index)` when it appears once, and `Duplicate(first_index)` when it appears more
    /// than once (the deterministic first match is kept, and ambiguity is surfaced, never silently fixed).
    ///
    /// This is the K.2 native-id resolver's core: it never falls back to an old positional index, and a
    /// duplicate is reported rather than resolved to an unrelated parameter.
    pub fn from_native(ids: &[u32], native: u32) -> ParamResolution {
        let mut first: Option<u16> = None;
        let mut count = 0u32;
        for (idx, &id) in ids.iter().enumerate() {
            if id == native {
                if first.is_none() {
                    first = Some(idx as u16);
                }
                count += 1;
            }
        }
        match count {
            0 => ParamResolution::Missing,
            1 => ParamResolution::Unique(first.unwrap()),
            _ => ParamResolution::Duplicate(first.unwrap()),
        }
    }
}

/// The reserved slot value for a channel-container member's place (matches `container::CHANNEL`).
pub const CHANNEL_SENTINEL: u8 = 0xFF;

// ---------------------------------------------------------------------------------------------------------------
// M1: the modulation graph
//
// A *modulator* is one source (an LFO, a step sequence or an envelope) plus the parameters it drives, each with its
// own depth, curve and polarity. The graph is the list of them. It refers to parameters by [`ParameterPath`], so an
// assignment survives a device move the way the device's identity does, and it is saved in the `.lemon` extension
// (never in a legacy project target). Nothing here evaluates or renders anything yet: this is the data the engine path and the
// UI are built on.
// ---------------------------------------------------------------------------------------------------------------

/// A modulator's identity in the project's graph. Minted from [`ModGraph::next`], never reused, saved with the graph;
/// `0` means "not assigned" ([`MODULATOR_NONE`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct ModulatorId(pub u32);

/// The reserved value for "no modulator".
pub const MODULATOR_NONE: ModulatorId = ModulatorId(0);

impl ModulatorId {
    pub fn is_none(self) -> bool {
        self == MODULATOR_NONE
    }
}

/// A macro's identity in the project's graph. Minted from [`ModGraph::next_macro`], never reused, saved with the graph;
/// `0` means "not assigned" ([`MACRO_NONE`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct MacroId(pub u32);

/// The reserved value for "no macro".
pub const MACRO_NONE: MacroId = MacroId(0);

impl MacroId {
    pub fn is_none(self) -> bool {
        self == MACRO_NONE
    }
}

/// A seed for a new random modulator: its id mixed with the wall clock, so two modulators made together still draw
/// different sequences. Once drawn it is saved, and every render of the project repeats it exactly.
pub fn fresh_seed(id: ModulatorId) -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let mut x = nanos ^ u64::from(id.0).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    x | 1
}

/// An LFO's waveform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LfoShape {
    #[default]
    Sine,
    Triangle,
    SawUp,
    SawDown,
    Square,
    /// Sample and hold: one new random value each cycle, held until the next.
    Random,
}

/// How a source's period is measured.
///
/// A period is the length of an LFO's cycle or of one step of a [`Steps`]. Both spellings are fractions of the
/// project's own time, so nothing here depends on a sample rate or a tempo that is not the project's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Timebase {
    /// Free-running, in hertz: cycles (or steps) a second.
    Free(f32),
    /// Synced to the project's tempo: the period in sixteenth notes — `1` a sixteenth, `4` a beat, `16` a bar.
    Sync(u16),
}

impl Default for Timebase {
    /// One cycle (or step) a second.
    fn default() -> Self {
        Timebase::Free(1.0)
    }
}

/// A low-frequency oscillator.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lfo {
    pub shape: LfoShape,
    /// One cycle's length.
    pub period: Timebase,
    /// Where in the cycle it starts, `0..=1` (`1` wraps to `0`).
    pub phase: f32,
    /// A constant shift added to the output, `-1..=1` of the depth ("Offset" in the handoff).
    pub offset: f32,
    /// The output gain, `0..=1` ("Amount" in the handoff; the assignment's own depth is separate).
    pub amount: f32,
    /// `true` swings around zero, `false` runs from zero up. Mirrors [`Polarity`] so a source can carry its own
    /// shape while each assignment still chooses how it drives a parameter.
    pub bipolar: bool,
}

impl Default for Lfo {
    fn default() -> Self {
        Lfo { shape: LfoShape::Sine, period: Timebase::Free(1.0), phase: 0.0, offset: 0.0, amount: 1.0, bipolar: true }
    }
}

/// A deterministic random source: one value each period, from a seed stored with the project.
///
/// The value is a pure function of `(seed, cycle)`, so an offline render and live playback agree bit for bit and a
/// seek lands on the value that cycle would have had. Smoothing is applied from the previous cycle's value over the
/// first `smooth` fraction of the cycle, which is also stateless and therefore seek-safe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Random {
    pub period: Timebase,
    /// The output gain, `0..=1`.
    pub amount: f32,
    pub distribution: Distribution,
    /// `0` = hard steps, `1` = the whole cycle glides from the previous value.
    pub smooth: f32,
    /// `true`: linear interpolation across the whole cycle. `false`: sample and hold (with `smooth` as a slew).
    pub interpolate: bool,
    /// Deterministic seed, generated on creation and saved with the project ("reroll seed" changes it).
    pub seed: u64,
    pub bipolar: bool,
}

impl Default for Random {
    fn default() -> Self {
        Random {
            period: Timebase::Sync(4),
            amount: 1.0,
            distribution: Distribution::Uniform,
            smooth: 0.0,
            interpolate: false,
            seed: 0,
            bipolar: true,
        }
    }
}

/// How a [`Random`] draws its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Distribution {
    /// Equal chance across the range.
    #[default]
    Uniform,
    /// Centred on zero, most values near the middle.
    Gaussian,
}

/// Where an [`AudioFollower`] reads its signal. v1 ships the master only; the enum is the scope field that lets
/// track sources arrive later without changing an assignment's format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FollowerSource {
    /// The master output after effects and the main volume (the handoff's recommendation).
    #[default]
    Master,
}

/// How an [`AudioFollower`] turns signal into a level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Detection {
    #[default]
    Rms,
    Peak,
}

/// The band of the signal an [`AudioFollower`] detects, applied **before** level detection (the handoff's critical
/// rule). `Custom` carries log-scaled low/high edges in hertz.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Band {
    Full,
    Low,
    Mid,
    High,
    Custom { low: f32, high: f32 },
}

impl Default for Band {
    fn default() -> Self {
        Band::Full
    }
}

impl Band {
    /// The band's (low, high) edges in hertz; `None` for the full spectrum.
    pub fn edges(self) -> Option<(f32, f32)> {
        match self {
            Band::Full => None,
            Band::Low => Some((20.0, 200.0)),
            Band::Mid => Some((200.0, 4000.0)),
            Band::High => Some((4000.0, 20000.0)),
            Band::Custom { low, high } => Some((low.clamp(20.0, 20000.0), high.clamp(20.0, 20000.0))),
        }
    }
}

/// A follower that turns the master's level into a modulation source. The detection is a cheap RMS/peak detector with
/// attack/release and a band filter before it; no FFT is involved on the audio thread.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioFollower {
    pub source: FollowerSource,
    /// The output gain, `0..=1`.
    pub amount: f32,
    /// Seconds to rise to a louder signal.
    pub attack: f32,
    /// Seconds to fall to a quieter one.
    pub release: f32,
    /// Detector gain in decibels (`-24..=+24`).
    pub gain_db: f32,
    pub detection: Detection,
    pub band: Band,
}

impl Default for AudioFollower {
    fn default() -> Self {
        AudioFollower {
            source: FollowerSource::Master,
            amount: 1.0,
            attack: 0.01,
            release: 0.2,
            gain_db: 0.0,
            detection: Detection::Rms,
            band: Band::Full,
        }
    }
}

/// A step sequence: sixteen held values, one every `period`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Steps {
    /// Each step's value in percent of the depth, `-100..=100` (`i8`, so a file keeps every value exactly).
    pub values: [i8; 16],
    /// How long one step lasts.
    pub period: Timebase,
    /// Glide from one step to the next instead of jumping.
    pub smooth: bool,
}

impl Default for Steps {
    /// Sixteen steps of nothing, one a sixteenth at 120 BPM (an eighth of a second at 120).
    fn default() -> Self {
        Steps { values: [0; 16], period: Timebase::Sync(1), smooth: false }
    }
}

/// How many breakpoints an [`Mseg`] can hold.
pub const MSEG_POINTS: usize = 16;

/// One breakpoint of an [`Mseg`]: where it sits in the shape (`0..=1`) and what it is worth (`-1..=1`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MsegPoint {
    pub at: f32,
    pub value: f32,
}

impl Default for MsegPoint {
    fn default() -> Self {
        MsegPoint { at: 0.0, value: 0.0 }
    }
}

/// A multi-segment envelope: up to [`MSEG_POINTS`] breakpoints across one period, played once (holding the last point)
/// or looped.
///
/// Like the LFO and the random source it is a pure function of the transport's position, so an offline render matches
/// live playback and a seek lands on the point the shape has there. A point's value is `-1..=1`; [`Mseg::bipolar`]
/// reads that as a swing either side of the middle, otherwise it is lifted above the middle the way an LFO's unipolar
/// output is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mseg {
    pub points: [MsegPoint; MSEG_POINTS],
    /// How many of `points` are in use, `2..=MSEG_POINTS` (anything less reads as the default shape).
    pub used: u8,
    /// The length of one pass through the shape.
    pub period: Timebase,
    /// `true`: run the shape again every period. `false`: play it once and hold the last point.
    pub looped: bool,
    /// `true`: a straight line between neighbouring points. `false`: each point holds until the next.
    pub smooth: bool,
    /// The output gain, `0..=1`.
    pub amount: f32,
    pub bipolar: bool,
}

impl Mseg {
    /// The points in use (at least two, never more than the array).
    pub fn used_points(&self) -> &[MsegPoint] {
        &self.points[..usize::from(self.used).clamp(2, MSEG_POINTS)]
    }
}

impl Default for Mseg {
    /// A full swing over one bar: down, up, down again.
    fn default() -> Self {
        let mut points = [MsegPoint::default(); MSEG_POINTS];
        points[0] = MsegPoint { at: 0.0, value: -1.0 };
        points[1] = MsegPoint { at: 0.5, value: 1.0 };
        points[2] = MsegPoint { at: 1.0, value: -1.0 };
        Mseg { points, used: 3, period: Timebase::Sync(16), looped: true, smooth: true, amount: 1.0, bipolar: true }
    }
}

/// A macro: a control the user turns, saved with the project, that drives whatever it is assigned to.
///
/// It is a source like any other, so one modulator can drive another modulator's macro (an LFO turning a macro that
/// drives ten parameters at ten different depths), and its position is a routing destination of its own
/// ([`ModParam::Value`]). Its output is the value itself, so an assignment's polarity and depth decide how it reads.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Macro {
    /// `0..=1`, the knob's position.
    pub value: f32,
}

/// The velocity of the last note a channel played: the handoff's velocity source. A channel that has played nothing
/// reads the middle, so an assignment at `0.5` base is unmoved until the first note.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Velocity {
    /// `0..=1` gain around the middle, like every source's amount.
    pub amount: f32,
}

/// Where a channel's last note sits in a key range: the handoff's key-tracking source. A low key reads the bottom of
/// the range and a high one the top, so an assignment can be as wide or as narrow as the player's hands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeyTrack {
    /// `0..=1` gain around the middle, like every source's amount.
    pub amount: f32,
    /// The MIDI keys the range spans, both ends included.
    pub low: u8,
    pub high: u8,
}

impl Default for KeyTrack {
    fn default() -> Self {
        KeyTrack { amount: 1.0, low: 36, high: 96 }
    }
}

impl KeyTrack {
    /// Its `0..=1` value for `key`: `0` at `low`, `1` at `high`, the middle for a reversed or empty range.
    pub fn value(self, key: u8) -> f32 {
        let (low, high) = (f32::from(self.low), f32::from(self.high));
        if high <= low {
            return 0.5;
        }
        ((f32::from(key) - low) / (high - low)).clamp(0.0, 1.0)
    }
}

/// A one-shot shape: rises over `attack`, falls to `sustain` over `decay`, holds, falls to nothing over `release`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Envelope {
    /// Seconds.
    pub attack: f32,
    pub decay: f32,
    /// `0..=1` of the depth.
    pub sustain: f32,
    pub release: f32,
}

/// A per-voice ADSR envelope: triggered by note-on/note-off per voice, not by the transport.
/// Each voice gets its own envelope state. The output is the envelope's level (0..1).
/// This is the "gate half" of the note sources — Velocity and KeyTrack read the note,
/// VoiceAdsr generates the envelope from the note's gate.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct VoiceAdsr {
    /// Seconds.
    pub attack: f32,
    pub decay: f32,
    /// `0..=1` of the depth.
    pub sustain: f32,
    pub release: f32,
    /// The amount (gain around the middle), like every source.
    pub amount: f32,
    /// `true`: bipolar swing around the middle. `false`: unipolar from the middle up.
    pub bipolar: bool,
}

impl VoiceAdsr {
    /// The parameter codes a VoiceAdsr offers for routing.
    pub const PARAMS: [ModParam; 5] = [
        ModParam::AttackV,
        ModParam::Decay,
        ModParam::Sustain,
        ModParam::ReleaseV,
        ModParam::Amount,
    ];
}

/// Math operator: combines two inputs with an operation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Math {
    /// The operation to apply.
    pub op: MathOp,
    /// The amount (gain around the middle), like every source.
    pub amount: f32,
    /// `true`: bipolar swing around the middle. `false`: unipolar from the middle up.
    pub bipolar: bool,
}

/// Math operation for the Math operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MathOp {
    #[default]
    Add,
    Subtract,
    Multiply,
    Divide,
    /// min(a, b)
    Min,
    /// max(a, b)
    Max,
    /// a raised to the power of b
    Pow,
    /// a modulo b
    Modulo,
}

impl Default for Math {
    fn default() -> Self {
        Math { op: MathOp::Add, amount: 1.0, bipolar: true }
    }
}

/// Polynomial operator: evaluates a polynomial on one input.
/// y = c0 + c1*x + c2*x^2 + c3*x^3 + c4*x^4 + c5*x^5
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Polynomial {
    /// Polynomial coefficients (up to degree 5).
    pub coeffs: [f32; 6],
    /// The amount (gain around the middle), like every source.
    pub amount: f32,
    /// `true`: bipolar swing around the middle. `false`: unipolar from the middle up.
    pub bipolar: bool,
}

impl Default for Polynomial {
    fn default() -> Self {
        Polynomial { coeffs: [0.0, 1.0, 0.0, 0.0, 0.0, 0.0], amount: 1.0, bipolar: true }
    }
}

/// Quantize operator: quantizes one input to discrete steps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quantize {
    /// Number of steps (2..=128).
    pub steps: u8,
    /// The amount (gain around the middle), like every source.
    pub amount: f32,
    /// `true`: bipolar swing around the middle. `false`: unipolar from the middle up.
    pub bipolar: bool,
}

impl Default for Quantize {
    fn default() -> Self {
        Quantize { steps: 12, amount: 1.0, bipolar: true }
    }
}

/// Map operator: remaps one input through a curve defined by breakpoints.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Map {
    /// Breakpoints for the mapping curve (x, y pairs, 0..1).
    pub points: [MapPoint; 8],
    /// Number of points in use (2..=8).
    pub used: u8,
    /// The amount (gain around the middle), like every source.
    pub amount: f32,
    /// `true`: bipolar swing around the middle. `false`: unipolar from the middle up.
    pub bipolar: bool,
}

/// One breakpoint of a Map curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapPoint {
    pub x: f32,
    pub y: f32,
}

impl Default for MapPoint {
    fn default() -> Self {
        MapPoint { x: 0.0, y: 0.0 }
    }
}

impl Default for Map {
    fn default() -> Self {
        let mut points = [MapPoint::default(); 8];
        points[0] = MapPoint { x: 0.0, y: 0.0 };
        points[1] = MapPoint { x: 1.0, y: 1.0 };
        Map { points, used: 2, amount: 1.0, bipolar: true }
    }
}

/// Slew operator: limits the rate of change of one input (slew limiter).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slew {
    /// Rising slew time in seconds (0 = instant).
    pub rise: f32,
    /// Falling slew time in seconds (0 = instant).
    pub fall: f32,
    /// The amount (gain around the middle), like every source.
    pub amount: f32,
    /// `true`: bipolar swing around the middle. `false`: unipolar from the middle up.
    pub bipolar: bool,
}

impl Default for Slew {
    fn default() -> Self {
        Slew { rise: 0.01, fall: 0.01, amount: 1.0, bipolar: true }
    }
}

/// What drives a modulator.
///
/// `Steps` and `Envelope` predate the project-level slots (M1) and stay in the format so old projects load. The project
/// slots offer the handoff's sources — [`Lfo`], [`Random`], [`AudioFollower`], [`Mseg`] and [`Macro`].
/// Operator nodes (math, polynomial, quantize, map, slew) are sources with one or two inputs (routes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModSource {
    Lfo(Lfo),
    Steps(Steps),
    Envelope(Envelope),
    Random(Random),
    AudioFollower(AudioFollower),
    Mseg(Mseg),
    Macro(Macro),
    /// The velocity of each channel's last note.
    Velocity(Velocity),
    /// Where each channel's last note sits in a key range.
    KeyTrack(KeyTrack),
    /// A per-voice ADSR envelope: triggered by note-on/note-off per voice.
    VoiceAdsr(VoiceAdsr),
    /// Math operator: combines two inputs with an operation.
    Math(Math),
    /// Polynomial operator: evaluates a polynomial on one input.
    Polynomial(Polynomial),
    /// Quantize operator: quantizes one input to discrete steps.
    Quantize(Quantize),
    /// Map operator: remaps one input through a curve.
    Map(Map),
    /// Slew operator: limits the rate of change of one input.
    Slew(Slew),
}

impl Default for ModSource {
    fn default() -> Self {
        ModSource::Lfo(Lfo::default())
    }
}

impl ModSource {
    /// Its own output gain (`0..=1`). At `0` the source drives nothing at all: its output is the neutral middle, so
    /// the modulator is effectively off whatever shape it has. A step sequence's steps are its values and a one-shot
    /// envelope follows the transport, so neither has a gain.
    pub fn amount(self) -> f32 {
        match self {
            ModSource::Lfo(l) => l.amount,
            ModSource::Random(r) => r.amount,
            ModSource::AudioFollower(f) => f.amount,
            ModSource::Mseg(m) => m.amount,
            ModSource::Velocity(v) => v.amount,
            ModSource::KeyTrack(k) => k.amount,
            ModSource::VoiceAdsr(v) => v.amount,
            ModSource::Math(m) => m.amount,
            ModSource::Polynomial(p) => p.amount,
            ModSource::Quantize(q) => q.amount,
            ModSource::Map(mp) => mp.amount,
            ModSource::Slew(s) => s.amount,
            // A macro at 0 is still on: the whole point of it is to be turned up.
            ModSource::Steps(_) | ModSource::Envelope(_) | ModSource::Macro(_) => 1.0,
        }
    }

    /// Whether the source can move anything at all: an amount of zero leaves it a flat line at the middle.
    pub fn is_on(self) -> bool {
        self.amount() > 0.0
    }
}

/// The shape of a target's response to the source's `0..=1` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Curve {
    #[default]
    Linear,
    /// Slow at the bottom, fast at the top.
    Exp,
    /// Fast at the bottom, slow at the top.
    Log,
    /// Eased at both ends.
    S,
}

/// Where the modulation moves the parameter from its own value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Polarity {
    /// Above and below the value: the source swings both ways.
    #[default]
    Bipolar,
    /// Only above it: the parameter's own value is the floor.
    Unipolar,
}

/// A route's identity in the project's graph: minted from [`ModGraph::next_route`], never reused, saved with the
/// assignment it names. `0` means "not assigned" ([`ROUTE_NONE`]).
///
/// A route is an assignment (a modulator, a parameter, a depth), so its identity has to live on the assignment itself;
/// that is what lets another modulator drive the route's depth ([`ParameterKey::RouteDepth`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct RouteId(pub u32);

/// The reserved value for "no route".
pub const ROUTE_NONE: RouteId = RouteId(0);

impl RouteId {
    pub fn is_none(self) -> bool {
        self == ROUTE_NONE
    }
}

/// One parameter a modulator drives, and how far.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModTarget {
    /// The parameter, by its persistent identity ([`ParameterPath`]).
    pub path: ParameterPath,
    /// The route's own identity, minted by [`ModGraph::assign`]; the handle another modulator drives when it shapes
    /// how far this route goes ([`ParameterKey::RouteDepth`]).
    pub route: RouteId,
    /// `-DEPTH_MAX..=DEPTH_MAX` of the parameter's range; negative inverts the source.
    pub depth: f32,
    pub curve: Curve,
    pub polarity: Polarity,
    /// Off keeps the assignment (and its depth) but contributes nothing. Set per routing from the browser.
    pub enabled: bool,
}

impl ModTarget {
    /// The largest depth: the whole range either way.
    pub const DEPTH_MAX: f32 = 1.0;
    /// What an assignment gets before the user moves it: a quarter of the range, so a new target is audible.
    pub const DEPTH_DEFAULT: f32 = 0.25;

    /// A target at [`ModTarget::DEPTH_DEFAULT`], linear, bipolar, on, with no route identity yet (the graph mints one
    /// when it is assigned).
    pub fn new(path: ParameterPath) -> Self {
        ModTarget { path, route: ROUTE_NONE, depth: Self::DEPTH_DEFAULT, curve: Curve::Linear, polarity: Polarity::Bipolar, enabled: true }
    }

    /// With its depth set and clamped to `-DEPTH_MAX..=DEPTH_MAX` (a value that is not a number is 0).
    pub fn with_depth(mut self, depth: f32) -> Self {
        self.depth = if depth.is_finite() { depth.clamp(-Self::DEPTH_MAX, Self::DEPTH_MAX) } else { 0.0 };
        self
    }
}

/// One modulator: its source, its project slot, and the parameters it drives.
#[derive(Debug, Clone, PartialEq)]
pub struct Modulator {
    pub id: ModulatorId,
    /// The user's name; empty until they give one (the UI makes a default from the source).
    pub name: String,
    pub source: ModSource,
    /// In assignment order; a parameter appears at most once ([`ModGraph::assign`]).
    pub targets: Vec<ModTarget>,
    /// Off keeps the assignments and drives nothing.
    pub enabled: bool,
    /// The project slot (`0..PROJECT_SLOTS`), or [`NO_SLOT`] for a modulator made before slots existed. Slots are
    /// fixed: removing a modulator leaves its slot empty rather than shifting the others.
    pub slot: u8,
}

/// The project's modulator slots in one row (the row above the device rack): the original eighteen, with three more
/// at the end. The row's "＋" adds another row of these (`ModGraph::rows`).
pub const PROJECT_SLOTS: usize = 21;
/// How many rows of slots a project can ask for: the twelve the 8-bit slot numbers can hold.
pub const MAX_ROWS: u8 = 12;
/// A modulator with no project slot (legacy M1 data, or a graph fuller than the slots).
pub const NO_SLOT: u8 = 255;

/// The project's modulation graph: every modulator, in creation order, and the counter that mints ids.
///
/// Assignments name their parameter by ([`ParameterPath`]), never by position, so a device move follows the same
/// identity rules the registry does (K.5 §6, K.6): an owner that is gone leaves the assignment unresolved rather
/// than pointing at whatever took its place.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModGraph {
    pub modulators: Vec<Modulator>,
    /// Next [`ModulatorId`] to mint (monotonic, never reused).
    pub next: u32,
    /// Next [`RouteId`] to mint (monotonic, never reused).
    pub next_route: u32,
    /// Next [`MacroId`] to mint (monotonic, never reused).
    pub next_macro: u32,
    /// Rows of project slots the strip shows: one until the user asks for another row. A project written before rows
    /// existed reads as 0, which is one row.
    pub rows: u8,
    /// XY pad pairs: each pair is two macros (X and Y axes) that are edited together as a pad.
    /// The first macro is the X axis, the second is the Y axis.
    pub xy_pads: Vec<(MacroId, MacroId)>,
}

impl ModGraph {
    /// No modulators (a project without any writes no graph).
    pub fn is_empty(&self) -> bool {
        self.modulators.is_empty()
    }

    /// Adds a modulator with a fresh id, the lowest free project slot (or [`NO_SLOT`] when all slots are taken) and
    /// no targets yet.
    pub fn add(&mut self, source: ModSource) -> ModulatorId {
        // The strip shows one add tile, so a full row grows another rather than leaving nowhere to put one.
        if self.free_slot().is_none() {
            self.add_row();
        }
        let slot = self.free_slot().unwrap_or(NO_SLOT);
        self.add_with_slot(source, slot)
    }

    /// The rows of project slots the strip shows (at least one).
    pub fn rows(&self) -> usize {
        usize::from(self.rows.max(1))
    }

    /// The project's slots: [`PROJECT_SLOTS`] in each of [`ModGraph::rows`] rows.
    pub fn slots(&self) -> usize {
        PROJECT_SLOTS * self.rows()
    }

    /// Another row of slots (the strip's "＋"). False at [`MAX_ROWS`].
    pub fn add_row(&mut self) -> bool {
        if self.rows() >= usize::from(MAX_ROWS) {
            return false;
        }
        self.rows = self.rows().saturating_add(1) as u8;
        true
    }

    /// Adds a modulator into project slot `slot` (`0..slots()`). `None` for a slot out of range or already
    /// taken: a slot is a fixed place, not a list position.
    pub fn add_in_slot(&mut self, slot: u8, source: ModSource) -> Option<ModulatorId> {
        if usize::from(slot) >= self.slots() || self.modulator_at_slot(slot).is_some() {
            return None;
        }
        Some(self.add_with_slot(source, slot))
    }

    fn add_with_slot(&mut self, source: ModSource, slot: u8) -> ModulatorId {
        let id = ModulatorId(self.next.max(1));
        self.next = id.0 + 1;
        let seed = if let ModSource::Random(r) = source { r.seed } else { 0 };
        let source = match source {
            // A seed of zero means "not drawn yet": give a new random modulator its own, saved with the project.
            ModSource::Random(r) if seed == 0 => ModSource::Random(Random { seed: fresh_seed(id), ..r }),
            other => other,
        };
        self.modulators.push(Modulator { id, name: String::new(), source, targets: Vec::new(), enabled: true, slot });
        id
    }

    /// The first free project slot, if any.
    pub fn free_slot(&self) -> Option<u8> {
        (0..self.slots() as u8).find(|s| self.modulator_at_slot(*s).is_none())
    }

    /// The modulator in project slot `slot`.
    pub fn modulator_at_slot(&self, slot: u8) -> Option<&Modulator> {
        self.modulators.iter().find(|m| m.slot == slot)
    }

    pub fn modulator_at_slot_mut(&mut self, slot: u8) -> Option<&mut Modulator> {
        self.modulators.iter_mut().find(|m| m.slot == slot)
    }

    pub fn get(&self, id: ModulatorId) -> Option<&Modulator> {
        self.modulators.iter().find(|m| m.id == id)
    }

    pub fn get_mut(&mut self, id: ModulatorId) -> Option<&mut Modulator> {
        self.modulators.iter_mut().find(|m| m.id == id)
    }

    /// Takes a modulator out (and its assignments with it). Ids are never reused, so `next` stays where it is.
    ///
    /// Every route *into* it goes too: a target naming a modulator that is gone would be an edge to nothing, and it
    /// would keep the graph from being a DAG (the id is never minted again, so nothing could ever reach it). The same
    /// goes for a target naming the depth of one of its routes, which is going with it.
    pub fn remove(&mut self, id: ModulatorId) -> Option<Modulator> {
        let at = self.modulators.iter().position(|m| m.id == id)?;
        let gone = self.modulators.remove(at);
        self.drop_routes_to(id, &gone.targets.iter().map(|t| t.route).collect::<Vec<_>>());
        Some(gone)
    }

    /// Every assignment anywhere in the graph that drives modulator `gone`, or the depth of one of the routes in
    /// `routes` (`gone`'s own assignments, which go with it).
    fn drop_routes_to(&mut self, gone: ModulatorId, routes: &[RouteId]) {
        for m in &mut self.modulators {
            m.targets.retain(|t| match t.path.parameter {
                ParameterKey::ModulatorParam(to, _) => to != gone,
                ParameterKey::RouteDepth(route) => !routes.contains(&route),
                _ => true,
            });
        }
    }

    /// The modulator a path lives on: a modulator's own parameter names it, and a route's depth names the modulator
    /// that makes the route. `None` for a device parameter, and for a route the graph no longer holds.
    pub fn lives_on(&self, path: ParameterPath) -> Option<ModulatorId> {
        match path.parameter {
            ParameterKey::ModulatorParam(id, _) => self.get(id).map(|m| m.id),
            ParameterKey::RouteDepth(route) => self.route_owner(route),
            _ => None,
        }
    }

    /// The modulator whose assignment carries route `route`.
    pub fn route_owner(&self, route: RouteId) -> Option<ModulatorId> {
        self.modulators
            .iter()
            .find(|m| m.targets.iter().any(|t| t.route == route))
            .map(|m| m.id)
    }

    /// The parameter route `route` drives.
    pub fn route_path(&self, route: RouteId) -> Option<ParameterPath> {
        self.modulators
            .iter()
            .flat_map(|m| m.targets.iter())
            .find(|t| t.route == route)
            .map(|t| t.path)
    }

    /// The route one modulator has into `path`.
    pub fn route_between(&self, id: ModulatorId, path: ParameterPath) -> Option<RouteId> {
        self.get(id)?.targets.iter().find(|t| t.path == path).map(|t| t.route)
    }

    /// Routes modulator `from` into parameter `param` of modulator `to` at `depth`: the handoff's
    /// modulator-to-modulator assignment.
    ///
    /// `false` and no change for: an unknown modulator at either end, a parameter `to`'s source does not have
    /// ([`ModParam::applies_to`]), `from` driving itself, and any route that would close a loop (see
    /// [`ModGraph::reaches`]). A modulator already driving that parameter is not changed either — one assignment per
    /// parameter per modulator, as for a device parameter.
    pub fn route(&mut self, from: ModulatorId, to: ModulatorId, param: ModParam, depth: f32) -> bool {
        let Some(destination) = self.get(to) else { return false };
        if self.get(from).is_none() || !param.applies_to(&destination.source) {
            return false;
        }
        if from == to || self.reaches(to, from) {
            return false;
        }
        self.assign(from, ModTarget::new(ParameterPath::on_modulator(to, param)).with_depth(depth))
    }

    /// Stops modulator `from` driving parameter `param` of modulator `to`.
    pub fn unroute(&mut self, from: ModulatorId, to: ModulatorId, param: ModParam) -> bool {
        self.unassign(from, ParameterPath::on_modulator(to, param))
    }

    /// Whether modulator `to` sits downstream of modulator `from`: some chain of routes runs from `from` to `to`. A new
    /// route from `to` back into `from` (or up its own chain) would close a loop, which is what [`ModGraph::route`]
    /// refuses. A modulator does not count as downstream of itself.
    pub fn reaches(&self, from: ModulatorId, to: ModulatorId) -> bool {
        if from == to {
            return false;
        }
        let mut stack = vec![from];
        let mut seen: Vec<ModulatorId> = Vec::new();
        while let Some(x) = stack.pop() {
            if seen.contains(&x) {
                continue;
            }
            seen.push(x);
            let Some(m) = self.get(x) else { continue };
            for t in &m.targets {
                // A route inside the graph names either a modulator's parameter or a route's depth, which belongs to
                // the modulator making that route: both are edges to that modulator.
                let Some(down) = self.lives_on(t.path) else { continue };
                if down == to {
                    return true;
                }
                stack.push(down);
            }
        }
        false
    }

    /// Every modulator in an order where each one comes after everything that drives it, so one evaluation pass gives
    /// every modulator its driven value (the engine's [`ModGraph::evaluation_order`] is how it runs the graph).
    ///
    /// Modulators with nothing driving them keep graph order. [`ModGraph::route`] refuses loops, so a well-formed
    /// graph is fully ordered; a modulator left over (a graph edited by hand, or data from a later version) is put at
    /// the end rather than dropped, so nothing ever stops being evaluated.
    pub fn evaluation_order(&self) -> Vec<ModulatorId> {
        let mut order: Vec<ModulatorId> = Vec::with_capacity(self.modulators.len());
        loop {
            let mut progressed = false;
            for m in &self.modulators {
                if order.contains(&m.id) {
                    continue;
                }
                let driven = self.modulators.iter().any(|o| {
                    o.id != m.id
                        && !order.contains(&o.id)
                        && o.targets.iter().any(|t| self.lives_on(t.path) == Some(m.id))
                });
                if !driven {
                    order.push(m.id);
                    progressed = true;
                }
            }
            if !progressed {
                break;
            }
        }
        for m in &self.modulators {
            if !order.contains(&m.id) {
                order.push(m.id);
            }
        }
        order
    }

    /// Assigns `target` to modulator `id`: `false` for an unknown modulator, or when it already drives that
    /// parameter (one assignment per parameter per modulator — a second would be two depths for one value).
    /// The depth is clamped ([`ModTarget::with_depth`]) and a route that has no identity yet is given one.
    ///
    /// A route's depth is a destination too ([`ParameterPath::on_route`]), with three rules that keep the graph a
    /// DAG: the route has to exist, its own destination may not be another route's depth (no depth of a depth), and a
    /// modulator cannot drive a depth on its own side of the chain (itself, or a modulator upstream of it) — the same
    /// loop check [`ModGraph::route`] makes.
    pub fn assign(&mut self, id: ModulatorId, target: ModTarget) -> bool {
        if let Some(route) = target.path.route() {
            let Some(owner) = self.route_owner(route) else { return false };
            if self.route_path(route).is_some_and(|p| p.route().is_some()) {
                return false;
            }
            if id == owner || self.reaches(owner, id) {
                return false;
            }
        }
        let route = if target.route.is_none() {
            // 16 bits: that is what the engine's target encoding carries for a route.
            let route = RouteId(self.next_route.max(1) & 0xFFFF);
            self.next_route = route.0 + 1;
            route
        } else {
            target.route
        };
        let Some(m) = self.get_mut(id) else { return false };
        if m.targets.iter().any(|t| t.path == target.path) {
            return false;
        }
        m.targets.push(ModTarget { route, ..target.with_depth(target.depth) });
        true
    }

    /// Stops modulator `id` driving `path`; `false` when it did not. A route's depth that named it goes too: the route
    /// it shaped no longer exists.
    pub fn unassign(&mut self, id: ModulatorId, path: ParameterPath) -> bool {
        let Some(m) = self.get_mut(id) else { return false };
        let Some(at) = m.targets.iter().position(|t| t.path == path) else { return false };
        let gone = m.targets.remove(at).route;
        if !gone.is_none() {
            for m in &mut self.modulators {
                m.targets.retain(|t| t.path.route() != Some(gone));
            }
        }
        true
    }

    /// Every assignment driving `path`, as (`modulator`, its assignment), in graph order. A parameter may be driven
    /// by several modulators; one modulator drives it at most once.
    pub fn targets_on(&self, path: ParameterPath) -> Vec<(ModulatorId, ModTarget)> {
        let mut v = Vec::new();
        for m in &self.modulators {
            for t in m.targets.iter().filter(|t| t.path == path) {
                v.push((m.id, *t));
            }
        }
        v
    }

    // --- XY pad support ---

    /// Creates a new macro with a fresh id. Macros are not modulators; they are simple 0..1 controls
    /// that can be paired into XY pads.
    pub fn add_macro(&mut self) -> MacroId {
        let id = MacroId(self.next_macro.max(1));
        self.next_macro = id.0 + 1;
        id
    }

    /// Creates an XY pad: a pair of macros (X and Y axes) that are edited together.
    /// Returns the (x_macro, y_macro) pair.
    pub fn add_xy_pad(&mut self) -> (MacroId, MacroId) {
        let x_macro = self.add_macro();
        let y_macro = self.add_macro();
        self.xy_pads.push((x_macro, y_macro));
        (x_macro, y_macro)
    }

    /// Finds the XY pad that contains `macro_id`, if any. Returns (x_macro, y_macro).
    pub fn xy_pad_for(&self, macro_id: MacroId) -> Option<(MacroId, MacroId)> {
        self.xy_pads.iter().find(|(x, y)| *x == macro_id || *y == macro_id).copied()
    }

    /// Removes an XY pad pair. Both macros become standalone.
    pub fn remove_xy_pad(&mut self, macro_id: MacroId) -> bool {
        if let Some(pos) = self.xy_pads.iter().position(|(x, y)| *x == macro_id || *y == macro_id) {
            self.xy_pads.remove(pos);
            true
        } else {
            false
        }
    }
}

/// The authoritative mapping from persistent owner identity to the owner's current location,
/// plus the counter that mints new identities. Persisted in the `.lemon` extension.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IdentityRegistry {
    /// `owner id -> current locator`. The reverse (locator -> id) is derived by scanning.
    pub owners: BTreeMap<PersistentObjectId, OwnerLocator>,
    /// Next `PersistentObjectId` to mint (monotonic, never reused).
    pub next: u32,
    /// Any native parameter bindings the user has explicitly established, keyed by path. Persisted
    /// so a native id survives even when its `LegacyPluginIndex` would otherwise be the only record.
    pub native_bindings: BTreeMap<PersistentObjectId, BTreeMap<u16, u32>>,
}

impl IdentityRegistry {
    /// Updates every owner's locator through a mixer place move (`track` and `slot` map an old place to
    /// its new one, `None` for one that vanished). The persistent id is preserved; an owner whose place
    /// is gone is dropped (its assignments become explicitly missing rather than silently kept). This is
    /// the identity counterpart of [`crate::Project::follow_mixer_places`], driven by the same closures so
    /// device identity follows the device exactly as its `Target`s do (K.5 §6, K.6).
    pub fn retarget_mixer_places(&mut self, track: &dyn Fn(u16) -> Option<u16>, slot: &dyn Fn(u16, u8) -> Option<(u16, u8)>) {
        let mut moved: Vec<(PersistentObjectId, OwnerLocator)> = Vec::new();
        let mut gone: Vec<PersistentObjectId> = Vec::new();
        for (&id, &loc) in &self.owners {
            let next = match loc {
                OwnerLocator::Channel(i) => track(i).map(OwnerLocator::Channel),
                OwnerLocator::Track(i) => track(i).map(OwnerLocator::Track),
                OwnerLocator::Slot(t, k) => slot(t, k).map(|(t, k)| OwnerLocator::Slot(t, k)),
                OwnerLocator::Member(t, k, m) => {
                    if k == CHANNEL_SENTINEL {
                        track(t).map(|nt| OwnerLocator::Member(nt, CHANNEL_SENTINEL, m))
                    } else {
                        slot(t, k).map(|(nt, nk)| OwnerLocator::Member(nt, nk, m))
                    }
                }
            };
            match next {
                Some(nl) if nl != loc => moved.push((id, nl)),
                Some(_) => {}
                None => gone.push(id),
            }
        }
        for (id, nl) in moved {
            self.owners.insert(id, nl);
        }
        for id in gone {
            self.owners.remove(&id);
        }
    }

    /// Applies explicit old→new locator moves (used by container wraps). Each owner at an `old` locator is
    /// re-bound to `new`, keeping its persistent id; owners not listed are left alone. This is the identity
    /// counterpart of [`crate::container::Moved::retarget`]: plugins wrapped into a container become members,
    /// and a wrapped container's members keep their identities at their new container place (K.5 §6, K.6).
    pub fn retarget_owners(&mut self, moves: &[(OwnerLocator, OwnerLocator)]) {
        if moves.is_empty() {
            return;
        }
        let mut done: Vec<(PersistentObjectId, OwnerLocator)> = Vec::new();
        for (&id, &loc) in &self.owners {
            if let Some((_, new)) = moves.iter().find(|(old, _)| *old == loc) {
                done.push((id, *new));
            }
        }
        for (id, new) in done {
            self.owners.insert(id, new);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target;

    #[test]
    fn native_id_unique() {
        // Native id 11 is at positional index 1; the resolver maps to that index.
        assert_eq!(ParamResolution::from_native(&[10, 11, 12], 11), ParamResolution::Unique(1));
    }

    #[test]
    fn native_id_missing() {
        // A native id the plugin no longer exposes is explicitly unresolved, never a silent fallback.
        assert_eq!(ParamResolution::from_native(&[10, 20], 99), ParamResolution::Missing);
    }

    #[test]
    fn native_id_duplicate() {
        // The first match (index 0) is kept deterministically; ambiguity is reported, never hidden.
        assert_eq!(ParamResolution::from_native(&[7, 7, 7], 7), ParamResolution::Duplicate(0));
        // A duplicate later in the list still resolves to the first occurrence.
        assert_eq!(ParamResolution::from_native(&[3, 9, 9, 3], 3), ParamResolution::Duplicate(0));
    }

    #[test]
    fn key_tracking_spans_its_range() {
        let k = KeyTrack { amount: 1.0, low: 40, high: 80 };
        assert_eq!(k.value(40), 0.0, "the low key is the bottom");
        assert_eq!(k.value(80), 1.0, "the high key is the top");
        assert_eq!(k.value(20), 0.0, "below the range stays at the bottom");
        assert!((k.value(60) - 0.5).abs() < 1e-6);
        assert_eq!(k.value(127), 1.0);
        assert_eq!(KeyTrack { amount: 1.0, low: 80, high: 40 }.value(60), 0.5, "a reversed range reads the middle");
    }

    #[test]
    fn note_sources_offer_only_their_amount() {
        let velocity = ModSource::Velocity(Velocity { amount: 1.0 });
        assert!(ModParam::Amount.applies_to(&velocity));
        assert!(!ModParam::Value.applies_to(&velocity));
        assert!(ModParam::Amount.applies_to(&ModSource::KeyTrack(KeyTrack::default())));
        assert!(velocity.is_on());
        assert!(!ModSource::Velocity(Velocity { amount: 0.0 }).is_on());
    }

    #[test]
    fn identity_registry_default_empty() {
        let r = IdentityRegistry::default();
        assert!(r.owners.is_empty());
        assert_eq!(r.next, 0);
    }

    // K.8 §7: a high mixer-slot plugin target with bit 31 set still encodes and decodes as before.
    #[test]
    fn high_mixer_target_bit31_intact() {
        let t = target::slot_param(64, 0, 0);
        assert_eq!(t, 0x80008000, "slot_param(64,0,0) must keep bit 31 set (K.8 §7)");
        assert_eq!(t & 0xFFFF, u32::from(target::PLUGIN_PARAM), "lower 16 bits are the plugin parameter code, unchanged");
        assert!(
            matches!(target::decode(t), target::Target::SlotParam { .. }),
            "the existing decoder still recognises the target"
        );
    }

    #[test]
    fn retarget_mixer_places_preserves_identity() {
        // M0 move preservation (K.5 §6, K.6): device identity follows the device through a mixer move.
        let mut r = IdentityRegistry::default();
        r.owners.insert(PersistentObjectId(1), OwnerLocator::Channel(2));
        r.owners.insert(PersistentObjectId(2), OwnerLocator::Track(3));
        r.owners.insert(PersistentObjectId(3), OwnerLocator::Slot(1, 0));
        r.owners.insert(PersistentObjectId(4), OwnerLocator::Member(1, 0, 2));
        r.owners.insert(PersistentObjectId(5), OwnerLocator::Member(5, CHANNEL_SENTINEL, 1));
        // Reorder: track 2->0, 3->1; slot (1,0)->(0,2); track 5 vanished (delete).
        let track = |t: u16| match t { 2 => Some(0), 3 => Some(1), 5 => None, _ => Some(t) };
        let slot = |t: u16, k: u8| if (t, k) == (1, 0) { Some((0, 2)) } else { Some((t, k)) };
        r.retarget_mixer_places(&track, &slot);
        assert_eq!(r.owners.get(&PersistentObjectId(1)), Some(&OwnerLocator::Channel(0)), "channel owner keeps its id at its new track");
        assert_eq!(r.owners.get(&PersistentObjectId(2)), Some(&OwnerLocator::Track(1)));
        assert_eq!(r.owners.get(&PersistentObjectId(3)), Some(&OwnerLocator::Slot(0, 2)), "slot owner follows the slot move");
        assert_eq!(r.owners.get(&PersistentObjectId(4)), Some(&OwnerLocator::Member(0, 2, 2)), "member follows its container's slot move");
        assert!(r.owners.get(&PersistentObjectId(5)).is_none(), "a vanished track's owner is dropped, not silently kept");
    }

    #[test]
    fn retarget_owners_wrap_to_member() {
        // M0 container wrap (K.5 §6, K.6): a wrapped plugin and a wrapped container's members keep their
        // persistent ids at their new member place.
        let mut r = IdentityRegistry::default();
        r.owners.insert(PersistentObjectId(1), OwnerLocator::Slot(1, 0));
        r.owners.insert(PersistentObjectId(2), OwnerLocator::Channel(3));
        r.owners.insert(PersistentObjectId(3), OwnerLocator::Member(1, 0, 5));
        let moves = vec![
            (OwnerLocator::Slot(1, 0), OwnerLocator::Member(4, 2, 100)),
            (OwnerLocator::Member(1, 0, 5), OwnerLocator::Member(4, 2, 200)),
            (OwnerLocator::Channel(3), OwnerLocator::Member(4, 2, 101)),
        ];
        r.retarget_owners(&moves);
        assert_eq!(r.owners.get(&PersistentObjectId(1)), Some(&OwnerLocator::Member(4, 2, 100)), "wrapped slot plugin keeps id as member");
        assert_eq!(r.owners.get(&PersistentObjectId(2)), Some(&OwnerLocator::Member(4, 2, 101)), "wrapped channel instrument keeps id as member");
        assert_eq!(r.owners.get(&PersistentObjectId(3)), Some(&OwnerLocator::Member(4, 2, 200)), "nested member keeps id at new container place");
    }

    // M1: the modulation graph (model only — no engine reads it yet).

    fn track_path(t: u16) -> ParameterPath {
        ParameterPath { owner: PersistentObjectId(1), parameter: ParameterKey::HostParameterCode(t) }
    }

    #[test]
    fn a_modulators_id_is_fresh_and_never_reused() {
        let mut g = ModGraph::default();
        let a = g.add(ModSource::Lfo(Lfo::default()));
        let b = g.add(ModSource::Envelope(Envelope::default()));
        assert_eq!((a.0, b.0), (1, 2), "ids start at 1; 0 is \"no modulator\"");
        assert!(!a.is_none() && !g.get(a).is_some_and(|m| m.id != a));
        assert!(g.get(a).is_some_and(|m| m.enabled), "a new modulator is on");
        assert_eq!(g.add(ModSource::Steps(Steps::default())).0, 3);
        assert_eq!(g.remove(b).map(|m| m.id), Some(b));
        assert!(g.get(b).is_none());
        // An id stays spent: nothing ever takes the removed modulator's place.
        assert_eq!(g.add(ModSource::default()).0, 4);
        assert_eq!(g.modulators.len(), 3);
    }

    /// The strip's "＋" adds a row of slots: the project has twice as many places, and a modulator takes one in the
    /// new row; asking beyond the last row changes nothing.
    #[test]
    fn another_row_adds_project_slots() {
        let mut g = ModGraph::default();
        assert_eq!((g.rows(), g.slots()), (1, PROJECT_SLOTS));
        assert!(g.add_row());
        assert_eq!((g.rows(), g.slots()), (2, PROJECT_SLOTS * 2));
        let first_in_row_two = PROJECT_SLOTS as u8;
        let id = g.add_in_slot(first_in_row_two, ModSource::default()).expect("the new row's first slot is free");
        assert_eq!(g.modulator_at_slot(first_in_row_two).map(|m| m.id), Some(id));
        // `add` reaches the new row too, filling the lowest free slot of them all.
        let mut empty = ModGraph::default();
        empty.add_row();
        for _ in 0..PROJECT_SLOTS - 1 {
            empty.add(ModSource::default());
        }
        assert_eq!(Some(empty.add(ModSource::default())), empty.modulator_at_slot(PROJECT_SLOTS as u8 - 1).map(|m| m.id));
        assert_eq!(Some(empty.add(ModSource::default())), empty.modulator_at_slot(PROJECT_SLOTS as u8).map(|m| m.id), "the new row's first slot");
        // The strip stops at MAX_ROWS rows.
        while g.rows() < usize::from(MAX_ROWS) {
            assert!(g.add_row());
        }
        assert!(!g.add_row());
        assert_eq!(g.rows(), usize::from(MAX_ROWS));
    }

    #[test]
    fn a_parameter_takes_one_assignment_per_modulator() {
        let mut g = ModGraph::default();
        let a = g.add(ModSource::default());
        let b = g.add(ModSource::default());
        let path = track_path(0x0100);
        assert!(g.assign(a, ModTarget::new(path)));
        assert!(!g.assign(a, ModTarget::new(path)), "the same parameter twice on one modulator is one assignment");
        assert!(g.assign(b, ModTarget::new(path)), "two modulators may drive one parameter");
        assert_eq!(g.targets_on(path).len(), 2);
        assert!(g.targets_on(track_path(0x0101)).is_empty());
        assert!(g.unassign(a, path));
        assert!(!g.unassign(a, path), "removing what is not assigned does nothing");
        assert_eq!(g.targets_on(path).len(), 1);
    }

    /// The project slots are fixed places: a modulator takes one, removing it leaves the slot empty, and the strip
    /// grows another line rather than running out of places. Switching tracks cannot move them because the graph is
    /// project state.
    #[test]
    fn project_slots_are_fixed_places() {
        assert_eq!(PROJECT_SLOTS, 21);
        let mut g = ModGraph::default();
        let a = g.add_in_slot(3, ModSource::Lfo(Lfo::default())).expect("slot 3 is free");
        assert_eq!(g.get(a).map(|m| m.slot), Some(3));
        assert!(g.add_in_slot(3, ModSource::default()).is_none(), "a taken slot is not reused");
        assert_eq!(g.modulator_at_slot(3).map(|m| m.id), Some(a));
        // `add` fills the lowest free slot, never one that is taken.
        let b = g.add(ModSource::default());
        assert_eq!(g.get(b).map(|m| m.slot), Some(0));
        for _ in 0..PROJECT_SLOTS - 2 {
            g.add(ModSource::default());
        }
        assert_eq!(g.free_slot(), None);
        // The strip has no "add row" button any more: a full line grows another one for the next modulator.
        let overflow = g.add(ModSource::default());
        assert_eq!(g.rows(), 2, "the graph grew a line");
        assert_eq!(g.get(overflow).map(|m| m.slot), Some(PROJECT_SLOTS as u8), "the first slot of the new line");
        // Removing frees exactly that slot, and no other modulator moves.
        g.remove(a);
        assert_eq!(g.free_slot(), Some(3));
        assert_eq!(g.get(b).map(|m| m.slot), Some(0));
    }

    /// A random modulator always carries a nonzero seed, saved with the graph; the same seed is kept on a reload.
    #[test]
    fn a_random_modulator_gets_a_persistent_seed() {
        let mut g = ModGraph::default();
        let id = g.add(ModSource::Random(Random::default()));
        let seed = match g.get(id).map(|m| m.source) {
            Some(ModSource::Random(r)) => r.seed,
            _ => panic!("a random source"),
        };
        assert_ne!(seed, 0, "zero means no seed drawn yet");
        let other = g.add(ModSource::Random(Random::default()));
        let other_seed = match g.get(other).map(|m| m.source) {
            Some(ModSource::Random(r)) => r.seed,
            _ => panic!(),
        };
        assert_ne!(seed, other_seed, "two random modulators do not share a sequence");
    }

    /// A routing keeps its own enable switch: off is still assigned, and turning it back on restores the depth.
    #[test]
    fn a_routing_can_be_disabled_without_losing_its_depth() {
        let path = track_path(0x0100);
        let mut t = ModTarget::new(path).with_depth(0.5);
        assert!(t.enabled);
        t.enabled = false;
        assert_eq!((t.depth, t.enabled), (0.5, false));
        t.enabled = true;
        assert_eq!(t.with_depth(t.depth).depth, 0.5);
    }

    /// The follower's band edges are the handoff's presets, and a custom band is clamped into the audible range.
    #[test]
    fn follower_bands_have_edges() {
        assert_eq!(Band::Full.edges(), None);
        assert_eq!(Band::Low.edges(), Some((20.0, 200.0)));
        assert_eq!(Band::Mid.edges(), Some((200.0, 4000.0)));
        assert_eq!(Band::High.edges(), Some((4000.0, 20000.0)));
        assert_eq!(Band::Custom { low: 1.0, high: 99999.0 }.edges(), Some((20.0, 20000.0)));
    }

    #[test]
    fn a_targets_depth_is_clamped_and_defaults_to_a_quarter() {
        let mut g = ModGraph::default();
        let id = g.add(ModSource::default());
        let path = track_path(0x0100);
        assert_eq!(ModTarget::new(path).depth, ModTarget::DEPTH_DEFAULT);
        // A depth past the range is the range; a value that is not a number is nothing.
        assert!(g.assign(id, ModTarget::new(path).with_depth(3.0)));
        assert_eq!(g.get(id).unwrap().targets[0].depth, 1.0);
        assert!(g.unassign(id, path));
        assert!(g.assign(id, ModTarget::new(path).with_depth(-3.0)));
        assert_eq!(g.get(id).unwrap().targets[0].depth, -1.0);
        assert_eq!(ModTarget::new(path).with_depth(f32::NAN).depth, 0.0);
        assert!(!g.assign(ModulatorId(99), ModTarget::new(path)), "an unknown modulator takes no assignment");
        assert!(!g.unassign(ModulatorId(99), path));
    }

    /// Modulator-to-modulator routing: a route is an ordinary assignment to another modulator's own parameter, and the
    /// graph refuses the two shapes that cannot be evaluated — a modulator driving itself, and any route that would
    /// close a loop.
    #[test]
    fn routes_into_the_graph_refuse_self_drives_and_loops() {
        let mut g = ModGraph::default();
        let a = g.add(ModSource::Lfo(Lfo::default()));
        let b = g.add(ModSource::Lfo(Lfo::default()));
        let c = g.add(ModSource::Lfo(Lfo::default()));
        assert!(g.route(a, b, ModParam::Rate, 0.5), "a drives b's rate");
        assert!(g.route(b, c, ModParam::Amount, -0.5), "b drives c's amount");
        assert_eq!(g.get(a).unwrap().targets[0].path, ParameterPath::on_modulator(b, ModParam::Rate));
        assert!(!g.route(c, a, ModParam::Rate, 0.5), "c back into a would close a loop");
        assert!(!g.route(b, a, ModParam::Phase, 0.5), "and a loop is refused whichever parameter names it");
        assert!(!g.route(a, a, ModParam::Rate, 0.5), "a modulator cannot drive itself");
        assert!(!g.route(a, b, ModParam::Rate, 0.9), "one assignment per parameter per modulator");
        assert!(g.route(a, b, ModParam::Offset, 0.5), "but another of its parameters is its own route");
        assert_eq!(g.get(a).unwrap().targets.len(), 2, "a's rate and offset routes to b");
        assert!(g.reaches(a, c) && !g.reaches(c, a), "downstream runs one way");
        assert!(g.reaches(b, c) && !g.reaches(c, b));
        // A source that has no such parameter takes no route to it.
        let steps = g.add(ModSource::Steps(Steps::default()));
        assert!(!g.route(a, steps, ModParam::Phase, 0.5), "a step sequence has no phase");
        assert!(g.route(a, steps, ModParam::Rate, 0.5), "but it has a rate");
        let envelope = g.add(ModSource::Envelope(Envelope::default()));
        assert!(!g.route(a, envelope, ModParam::Rate, 0.5), "a one-shot envelope follows the transport, not a rate");
    }

    /// A follower's attack, release and gain and a random source's smoothing are destinations like any other, and a
    /// source without one refuses the route (a follower has no rate: it follows the master, not a period). Every
    /// parameter keeps a stable byte, so a route saved by this version reads back as the same parameter.
    #[test]
    fn a_followers_and_a_random_sources_own_parameters_take_routes() {
        let follower = AudioFollower::default();
        let random = Random::default();
        for param in [ModParam::Amount, ModParam::Attack, ModParam::Release, ModParam::Gain] {
            assert!(param.applies_to(&ModSource::AudioFollower(follower)), "a follower has a {param:?}");
        }
        assert!(!ModParam::Rate.applies_to(&ModSource::AudioFollower(follower)), "a follower has no rate");
        for param in [ModParam::Rate, ModParam::Amount, ModParam::Smooth] {
            assert!(param.applies_to(&ModSource::Random(random)), "a random source has a {param:?}");
        }
        assert!(!ModParam::Attack.applies_to(&ModSource::Random(random)));
        assert!(!ModParam::Smooth.applies_to(&ModSource::Lfo(Lfo::default())), "an LFO does not slew");
        for (i, param) in ModParam::ALL.iter().enumerate() {
            assert_eq!(param.code() as usize, i, "{param:?} keeps its byte");
            assert_eq!(ModParam::of(param.code()), Some(*param));
        }
        assert_eq!(ModParam::of(31), None, "a byte this version does not know is not a parameter");
        let mut g = ModGraph::default();
        let driver = g.add(ModSource::Lfo(Lfo::default()));
        let f = g.add(ModSource::AudioFollower(AudioFollower::default()));
        let r = g.add(ModSource::Random(Random::default()));
        assert!(g.route(driver, f, ModParam::Attack, 0.5), "a follower's attack is a destination");
        assert!(g.route(driver, f, ModParam::Gain, 0.5));
        assert!(!g.route(driver, f, ModParam::Rate, 0.5), "a follower has no rate to drive");
        assert!(g.route(driver, r, ModParam::Smooth, 0.5), "a random source's smoothing is a destination");
        // The routes live on the driver, naming the destination's parameter.
        assert_eq!(g.get(driver).unwrap().targets[1].path, ParameterPath::on_modulator(f, ModParam::Gain));
    }

    /// A macro's position is a destination of its own (so an LFO can turn it), and an MSEG takes a rate and an amount
    /// but nothing else: its shape is its own. Both are sources like any other, so they join the same graph.
    #[test]
    fn a_macro_and_an_mseg_have_their_own_addressable_parameters() {
        let macro_source = ModSource::Macro(Macro { value: 0.25 });
        assert!(ModParam::Value.applies_to(&macro_source));
        assert_eq!(macro_source.amount(), 1.0, "a macro at any position drives");
        assert!(!ModParam::Rate.applies_to(&macro_source));
        let mseg = ModSource::Mseg(Mseg::default());
        for param in [ModParam::Rate, ModParam::Amount] {
            assert!(param.applies_to(&mseg), "an MSEG has a {param:?}");
        }
        assert!(!ModParam::Value.applies_to(&mseg));
        assert!(!ModParam::Attack.applies_to(&mseg));
        assert_eq!(ModParam::Value.code(), 8, "after the follower's and the random source's");
        assert_eq!(ModParam::of(8), Some(ModParam::Value));
        let mut g = ModGraph::default();
        let lfo = g.add(ModSource::Lfo(Lfo::default()));
        let mac = g.add(ModSource::Macro(Macro { value: 0.0 }));
        assert!(g.route(lfo, mac, ModParam::Value, 0.5), "an LFO turns the macro");
        assert!(!g.route(mac, lfo, ModParam::Amount, 0.5), "and a route back would close the loop");
        assert_eq!(g.evaluation_order(), vec![lfo, mac], "the LFO is evaluated first");
        let shape = g.add(ModSource::Mseg(Mseg::default()));
        assert!(!g.route(mac, shape, ModParam::Value, 0.5), "an MSEG has no value to turn");
        assert!(g.route(mac, shape, ModParam::Rate, 0.5));
    }

    /// Every assignment carries its own route identity, and a route's depth is a destination: another modulator can shape
    /// how far it drives. Three rules keep the graph a DAG — the depth's own driver cannot shape it, a route that is
    /// itself a depth takes no depth, and a loop is refused — and removing the route (or its modulator) takes the depth
    /// routes with it.
    #[test]
    fn a_route_has_an_identity_and_its_depth_takes_routes() {
        let mut g = ModGraph::default();
        let a = g.add(ModSource::Lfo(Lfo::default()));
        let b = g.add(ModSource::Lfo(Lfo::default()));
        let path = track_path(0x0100);
        assert!(g.assign(b, ModTarget::new(path)));
        let route = g.route_between(b, path).expect("the assignment has a route");
        assert!(!route.is_none());
        assert_eq!((g.route_owner(route), g.route_path(route)), (Some(b), Some(path)));
        assert_eq!(g.next_route, route.0 + 1, "ids are minted and never reused");
        // A shapes how far B drives that parameter: an edge into B, so it is evaluated first.
        let deeper = ParameterPath::on_route(route);
        assert!(g.assign(a, ModTarget::new(deeper).with_depth(0.5)));
        assert_eq!(g.lives_on(deeper), Some(b));
        assert!(g.reaches(a, b) && !g.reaches(b, a), "the depth is an edge into the modulator making the route");
        assert_eq!(g.evaluation_order(), vec![a, b]);
        // The three refusals: B shaping its own route, a depth of a depth, and a loop formed by shaping a depth upstream.
        assert!(!g.assign(b, ModTarget::new(deeper)), "a route's own driver cannot shape its depth");
        let inner = g.route_between(a, deeper).expect("A's own route");
        assert!(!g.assign(b, ModTarget::new(ParameterPath::on_route(inner))), "a depth is not itself a destination");
        // Removing B takes its route, and A's route into that route's depth with it.
        g.remove(b);
        assert!(g.get(a).is_some_and(|m| m.targets.is_empty()), "the depth route went with the route it shaped");
        // Taking an assignment away does the same.
        let b2 = g.add(ModSource::Lfo(Lfo::default()));
        assert!(g.assign(b2, ModTarget::new(path)));
        let route2 = g.route_between(b2, path).unwrap();
        assert!(g.assign(a, ModTarget::new(ParameterPath::on_route(route2))));
        assert!(g.unassign(b2, path));
        assert!(g.get(a).is_some_and(|m| m.targets.is_empty()), "the depth route went with the assignment");
    }

    /// The evaluation order puts everything after what drives it, even when the driven modulator was created first;
    /// removing a modulator takes the routes on both its sides with it.
    #[test]
    fn the_evaluation_order_follows_the_routes_and_removal_prunes_them() {
        let mut g = ModGraph::default();
        let driven = g.add(ModSource::Lfo(Lfo::default()));
        let driver = g.add(ModSource::Lfo(Lfo::default()));
        assert!(driver.0 > driven.0, "the driven modulator is the older one");
        assert!(g.route(driver, driven, ModParam::Rate, 1.0));
        assert_eq!(g.evaluation_order(), vec![driver, driven], "the driver comes first, not the older one");
        assert!(g.remove(driver).is_some());
        assert!(g.get(driven).unwrap().targets.is_empty(), "the route into it went with the driver");
    }

    /// A route is addressed in the engine's own space, in the range no legacy project target uses, and its path names
    /// the modulator rather than a device.
    #[test]
    fn a_route_is_addressed_and_named_by_the_modulator() {
        let t = target::modulator_param(7, ModParam::Phase.code());
        assert_eq!(target::decode(t), target::Target::ModulatorParam(7, ModParam::Phase.code()));
        assert_eq!(t >> 16, 0x6002, "the modulator parameter's own range, above the project's constants");
        let path = ParameterPath::on_modulator(ModulatorId(7), ModParam::Phase);
        assert_eq!(path.modulator(), Some((ModulatorId(7), ModParam::Phase)));
        assert_eq!(path.owner, PERSISTENT_NONE, "a modulator is its own owner");
        assert_eq!(track_path(0x0100).modulator(), None, "a device parameter is not a route");
    }
}
