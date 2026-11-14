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
}

/// A persistent, project-wide parameter identity. This is what the modulation graph stores for each
/// assignment; it is resolved to a concrete [`crate::target::Target`] at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParameterPath {
    pub owner: PersistentObjectId,
    pub parameter: ParameterKey,
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

/// What drives a modulator.
///
/// `Steps` and `Envelope` predate the project-level slots (M1) and stay in the format so old projects load. The eight
/// project slots offer the handoff's three sources: [`Lfo`], [`Random`] and [`AudioFollower`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModSource {
    Lfo(Lfo),
    Steps(Steps),
    Envelope(Envelope),
    Random(Random),
    AudioFollower(AudioFollower),
}

impl Default for ModSource {
    fn default() -> Self {
        ModSource::Lfo(Lfo::default())
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

/// One parameter a modulator drives, and how far.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModTarget {
    /// The parameter, by its persistent identity ([`ParameterPath`]).
    pub path: ParameterPath,
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

    /// A target at [`ModTarget::DEPTH_DEFAULT`], linear, bipolar, on.
    pub fn new(path: ParameterPath) -> Self {
        ModTarget { path, depth: Self::DEPTH_DEFAULT, curve: Curve::Linear, polarity: Polarity::Bipolar, enabled: true }
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
    /// Rows of project slots the strip shows: one until the user asks for another row. A project written before rows
    /// existed reads as 0, which is one row.
    pub rows: u8,
}

impl ModGraph {
    /// No modulators (a project without any writes no graph).
    pub fn is_empty(&self) -> bool {
        self.modulators.is_empty()
    }

    /// Adds a modulator with a fresh id, the lowest free project slot (or [`NO_SLOT`] when all slots are taken) and
    /// no targets yet.
    pub fn add(&mut self, source: ModSource) -> ModulatorId {
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
    pub fn remove(&mut self, id: ModulatorId) -> Option<Modulator> {
        let at = self.modulators.iter().position(|m| m.id == id)?;
        Some(self.modulators.remove(at))
    }

    /// Assigns `target` to modulator `id`: `false` for an unknown modulator, or when it already drives that
    /// parameter (one assignment per parameter per modulator — a second would be two depths for one value).
    /// The depth is clamped ([`ModTarget::with_depth`]).
    pub fn assign(&mut self, id: ModulatorId, target: ModTarget) -> bool {
        let Some(m) = self.get_mut(id) else { return false };
        if m.targets.iter().any(|t| t.path == target.path) {
            return false;
        }
        m.targets.push(target.with_depth(target.depth));
        true
    }

    /// Stops modulator `id` driving `path`; `false` when it did not.
    pub fn unassign(&mut self, id: ModulatorId, path: ParameterPath) -> bool {
        let Some(m) = self.get_mut(id) else { return false };
        let before = m.targets.len();
        m.targets.retain(|t| t.path != path);
        m.targets.len() != before
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

    /// The project slots are fixed places: a modulator takes one, removing it leaves the slot empty, and a
    /// modulator beyond the slots has none. Switching tracks cannot move them because the graph is project state.
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
        let overflow = g.add(ModSource::default());
        assert_eq!(g.get(overflow).map(|m| m.slot), Some(NO_SLOT), "a modulator beyond the slots has no slot");
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
}
