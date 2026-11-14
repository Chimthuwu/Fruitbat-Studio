//! Persistent parameter identity for the DAW-wide modulation system (M0, audit K.2/K.3).
//!
//! The [`State::identity`] registry maps a stable [`PersistentObjectId`] to each device/owner's
//! *current* [`OwnerLocator`]. The id is persisted in the `.lemon` extension and survives save/load
//! and device moves; the locator is what gets updated when a device moves. The legacy project `Target` encoding
//! is never consulted or modified here (K.1).
//!
//! [`target_for`] is a pure mapping from an [`OwnerLocator`] + [`ParameterKey`] to a concrete legacy project
//! `Target` (and a [`ParamResolution`]); [`resolve_path`] is the live wrapper that supplies the
//! plugin's current native-id list from [`Plugins`].

use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

use crate::app::State;
use crate::playlist::{Cmd, Mark, PItem};
use crate::plugins::Plugins;
use os_model::modulation::{
    AudioFollower, Band, Curve, Detection, Distribution, Envelope, Lfo, LfoShape, ModSource, Modulator, ModulatorId, OwnerLocator, ParameterKey,
    ParameterPath, ParamResolution, PersistentObjectId, Polarity, Random, Steps, Timebase,
};
use os_model::target;

impl State {
    /// Returns the persistent id for `loc`, minting a new one if it has none. Idempotent: calling it
    /// again for the same locator returns the same id.
    pub fn ensure_owner(&mut self, loc: OwnerLocator) -> PersistentObjectId {
        if let Some(id) = self.owner_of(loc) {
            return id;
        }
        let id = PersistentObjectId(self.identity.next.max(1));
        self.identity.next = id.0 + 1;
        self.identity.owners.insert(id, loc);
        id
    }

    /// The persistent id currently bound to `loc`, if any.
    pub fn owner_of(&self, loc: OwnerLocator) -> Option<PersistentObjectId> {
        self.identity.owners.iter().find(|(_, l)| **l == loc).map(|(&id, _)| id)
    }

    /// The current locator of `id`, if it is still registered.
    pub fn locator_of(&self, id: PersistentObjectId) -> Option<OwnerLocator> {
        self.identity.owners.get(&id).copied()
    }

    /// Keeps `id`'s identity while the device moves: the locator changes, the id does not. A device that
    /// is deleted or replaced must not transfer its id to the replacement — callers must not pass a new
    /// locator for an unrelated device (K.5 §6).
    pub fn retarget_owner(&mut self, old: OwnerLocator, new: OwnerLocator) {
        if let Some(id) = self.owner_of(old) {
            self.identity.owners.insert(id, new);
        }
    }
}

/// Every owner the project currently holds, as a locator: each channel's instrument, each mixer track's
/// host parameters, each mixer slot's hosted plugin, and each container member.
pub fn owner_locators(s: &State) -> Vec<OwnerLocator> {
    let p = &s.project;
    let mut v = Vec::new();
    for c in &p.channels {
        v.push(OwnerLocator::Channel(c.index));
        if crate::container::is_container_channel(c) {
            if let Some(members) = crate::container::members(s, c.index, crate::container::CHANNEL) {
                for m in members {
                    v.push(OwnerLocator::Member(c.index, crate::container::CHANNEL, m.id));
                }
            }
        }
    }
    for m in &p.mixer {
        v.push(OwnerLocator::Track(m.index));
        for (k, slot) in m.slots.iter().enumerate() {
            if let Some(slot) = slot {
                let k = k as u8;
                v.push(OwnerLocator::Slot(m.index, k));
                if slot.name == crate::container::CONTAINER {
                    if let Some(members) = crate::container::members(s, m.index, k) {
                        for mem in members {
                            v.push(OwnerLocator::Member(m.index, k, mem.id));
                        }
                    }
                }
            }
        }
    }
    v
}

/// Ensures every current owner has a persistent id (called on load, before save, and at mark-saved).
pub fn assign_all(s: &mut State) {
    for loc in owner_locators(s) {
        s.ensure_owner(loc);
    }
}

/// The native parameter ids of `owner`'s hosted plugin, in positional order (`None` where the plugin is
/// not hosted or its list has not been read yet, or for owners that have no plugin — tracks, container
/// members whose hosted-key lookup is deferred).
fn hosted_native_ids(plugins: &Plugins, owner: OwnerLocator) -> Option<Vec<u32>> {
    let key = match owner {
        OwnerLocator::Channel(i) => i as usize,
        OwnerLocator::Slot(t, k) => crate::plugins::fx_key(t, k),
        OwnerLocator::Track(_) | OwnerLocator::Member(..) => return None,
    };
    plugins.hosted.get(&key).and_then(|h| h.param_ids())
}

/// Pure mapping from an owner locator + parameter key to a concrete legacy project `Target`, plus how the native id
/// resolved. `native_ids` is the hosted plugin's native ids in positional order (from [`hosted_native_ids`]
/// or a test fixture). For a `PluginNativeId` with no plugin metadata, the path is unresolved (`None`).
///
/// Host parameters pass through unchanged (the existing lower-16-bit host code is reused, never reinvented).
/// A `LegacyPluginIndex` is encoded positionally and is **never** silently upgraded to a native id here —
/// only an explicit user migration writes a native binding (K.2).
pub fn target_for(owner: OwnerLocator, key: ParameterKey, native_ids: Option<&[u32]>) -> Option<(u32, ParamResolution)> {
    let mk = |lower: u16| match owner {
        OwnerLocator::Channel(i) => u32::from(i) << 16 | u32::from(lower),
        OwnerLocator::Track(t) => (0x7000 + u32::from(t) * 64) << 16 | u32::from(lower),
        OwnerLocator::Slot(t, k) => (0x7000 + u32::from(t) * 64 + u32::from(k)) << 16 | u32::from(lower),
        OwnerLocator::Member(t, k, _) => {
            if k == crate::container::CHANNEL {
                u32::from(t) << 16 | u32::from(lower)
            } else {
                (0x7000 + u32::from(t) * 64 + u32::from(k)) << 16 | u32::from(lower)
            }
        }
    };
    let plugin_lower = |idx: u16| -> Option<u16> {
        Some(match owner {
            OwnerLocator::Channel(_) | OwnerLocator::Slot(..) => target::PLUGIN_PARAM | (idx & 0x7FFF),
            OwnerLocator::Member(_, _, m) => {
                target::PLUGIN_PARAM | target::member_item(m, idx & 0x7FFF)?
            }
            OwnerLocator::Track(_) => return None,
        })
    };
    match key {
        ParameterKey::HostParameterCode(c) => Some((mk(c), ParamResolution::Unique(0))),
        // A route into the graph is not a device parameter: `resolve_path` handles it before asking the registry.
        ParameterKey::ModulatorParam(..) => None,
        ParameterKey::LegacyPluginIndex(i) => {
            let lower = plugin_lower(i)?;
            Some((mk(lower), ParamResolution::Unique(i)))
        }
        ParameterKey::PluginNativeId(native) => {
            let ids = native_ids?;
            let res = ParamResolution::from_native(ids, native);
            let idx = match res {
                ParamResolution::Missing => return None,
                ParamResolution::Unique(i) | ParamResolution::Duplicate(i) => i,
            };
            let lower = plugin_lower(idx)?;
            Some((mk(lower), res))
        }
    }
}

/// Resolves a persisted [`ParameterPath`] against the current project and plugin metadata.
pub fn resolve_path(s: &State, plugins: &Plugins, path: ParameterPath) -> Option<(u32, ParamResolution)> {
    // A route into the graph names a modulator, not a device: it needs no registry (a modulator never moves) and
    // resolves only while that modulator is still there.
    if let ParameterKey::ModulatorParam(id, param) = path.parameter {
        s.modulation.get(id)?;
        return Some((target::modulator_param(id.0, param.code()), ParamResolution::Unique(0)));
    }
    let owner = s.identity.owners.get(&path.owner).copied()?;
    let native_ids = hosted_native_ids(plugins, owner);
    target_for(owner, path.parameter, native_ids.as_deref())
}

/// The owner and parameter a legacy project `Target` names, read back from its encoding ([`target_for`] is the other way). This is
/// how the interface turns the control under the cursor into a graph assignment. The owner is a locator, not an id:
/// [`path_in`] asks the registry, [`target_path`] mints one when the owner is new.
pub fn path_of(target: u32) -> Option<(OwnerLocator, ParameterKey)> {
    let lower = (target & 0xFFFF) as u16;
    let high = target >> 16;
    // A plugin parameter's index lives behind the low half's PLUGIN_PARAM bit; anything else is a host code.
    let plugin = lower & target::PLUGIN_PARAM != 0;
    let parameter = if plugin { ParameterKey::LegacyPluginIndex(lower & 0x7FFF) } else { ParameterKey::HostParameterCode(lower) };
    let owner = if high >= 0x7000 {
        // A mixer track: `0x7000 + track * 64 + slot`. Only a plugin parameter names a slot.
        let rest = high - 0x7000;
        let track = (rest / 64) as u16;
        let slot = (rest % 64) as u8;
        if plugin {
            OwnerLocator::Slot(track, slot)
        } else {
            OwnerLocator::Track(track)
        }
    } else {
        OwnerLocator::Channel(high as u16)
    };
    Some((owner, parameter))
}

/// A route into the graph as a path: modulator `id`'s parameter `param` is its own owner, so it needs no registry.
fn modulator_path(target: u32) -> Option<ParameterPath> {
    let target::Target::ModulatorParam(id, code) = target::decode(target) else { return None };
    let param = os_model::modulation::ModParam::of(code)?;
    Some(ParameterPath::on_modulator(os_model::modulation::ModulatorId(id), param))
}

/// `target` as a [`ParameterPath`] against the current registry: `None` when the owner has no id yet.
pub fn path_in(s: &State, target: u32) -> Option<ParameterPath> {
    if let Some(path) = modulator_path(target) {
        s.modulation.get(path.modulator()?.0)?;
        return Some(path);
    }
    let (owner, parameter) = path_of(target)?;
    Some(ParameterPath { owner: s.owner_of(owner)?, parameter })
}

/// `target` as a [`ParameterPath`], giving its owner an identity if it has none: what an assignment stores.
pub fn target_path(s: &mut State, target: u32) -> Option<ParameterPath> {
    if let Some(path) = modulator_path(target) {
        return Some(path);
    }
    let (owner, parameter) = path_of(target)?;
    Some(ParameterPath { owner: s.ensure_owner(owner), parameter })
}

/// A persisted parameter path as the browser lists it ("Lemon Filter - Cutoff"). Resolved against the current project
/// only, so a path whose owner or plugin is gone still names what it was ("Missing parameter"). This is the browser's
/// label; the engine's own resolution (`engine_spec`) stays the authority on whether a routing reaches audio.
pub fn path_label(s: &State, path: ParameterPath) -> String {
    use os_model::modulation::ParameterKey;
    // A route into the graph names a modulator, which the graph itself still holds (or does not).
    if let Some((id, param)) = path.modulator() {
        return match s.modulation.get(id) {
            Some(m) => format!("{} - {}", name_of(m), param.label()),
            None => "Missing parameter".into(),
        };
    }
    let Some(loc) = s.identity.owners.get(&path.owner).copied() else { return "Missing parameter".into() };
    let key = match loc {
        OwnerLocator::Channel(ch) => Some(usize::from(ch)),
        OwnerLocator::Slot(track, slot) => Some(crate::plugins::fx_key(track, slot)),
        OwnerLocator::Member(track, slot, id) => crate::container::member_key(s, crate::container::place_key(track, slot), id),
        OwnerLocator::Track(_) => None,
    };
    let pname = key.and_then(|key| {
        let l = s.params.lists.get(&key)?;
        match path.parameter {
            ParameterKey::LegacyPluginIndex(i) => l.params.get(usize::from(i)),
            ParameterKey::PluginNativeId(native) => l.params.iter().find(|e| e.id == native),
            ParameterKey::HostParameterCode(_) | ParameterKey::ModulatorParam(..) => None,
        }
        .map(|e| e.name.clone())
    });
    match (key, pname) {
        (Some(key), Some(name)) => format!("{} - {name}", crate::params::owner_name(s, key)),
        (_, Some(name)) => name,
        (_, None) => match loc {
            OwnerLocator::Track(track) => format!("{} - control", crate::mixer::name(s, track)),
            _ => "Parameter".into(),
        },
    }
}

/// A modulator's name for a menu: what the user typed, else a default made from its source ("LFO 2", "STEPS 1").
pub fn name_of(m: &Modulator) -> String {
    if !m.name.is_empty() {
        return m.name.clone();
    }
    let kind = match m.source {
        os_model::modulation::ModSource::Lfo(_) => "LFO",
        os_model::modulation::ModSource::Steps(_) => "STEPS",
        os_model::modulation::ModSource::Envelope(_) => "ENV",
        os_model::modulation::ModSource::Random(_) => "RANDOM",
        os_model::modulation::ModSource::AudioFollower(_) => "FOLLOWER",
    };
    format!("{kind} {}", m.id.0)
}

// ---------------------------------------------------------------------------------------------------------------
// The modulator editor
//
// A modulator's source, its rate and how it drives one control are all editable from the knob's Modulation menu: the
// action that makes a modulator also has to be able to shape it. Every edit is one undo step and reaches playback at once
// ([`run`]), and the menu is built from the graph as it stands, so it reads as the modulator's own state.
// ---------------------------------------------------------------------------------------------------------------

/// A modulator's kind, as the editor's Source items choose it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Lfo,
    Random,
    Follower,
    Steps,
    Envelope,
}

impl Kind {
    /// The kind of `src`.
    pub fn of(src: ModSource) -> Kind {
        match src {
            ModSource::Lfo(_) => Kind::Lfo,
            ModSource::Steps(_) => Kind::Steps,
            ModSource::Envelope(_) => Kind::Envelope,
            ModSource::Random(_) => Kind::Random,
            ModSource::AudioFollower(_) => Kind::Follower,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Lfo => "LFO",
            Kind::Random => "Random",
            Kind::Follower => "Audio Follower",
            Kind::Steps => "Steps",
            Kind::Envelope => "Envelope",
        }
    }

    /// The three types the project slots offer (the handoff's v1 set). Steps and Envelope stay readable in
    /// old projects and keep their knob-menu entries, but a new slot starts as one of these.
    pub const PROJECT: [Kind; 3] = [Kind::Lfo, Kind::Random, Kind::Follower];
}

/// `src` as `kind`, keeping the rate it had where the new kind has one: an LFO and a step pattern are both timed, so a
/// switch between them keeps the tempo the user chose (an envelope has no rate, and keeps nothing).
pub fn set_kind(src: ModSource, kind: Kind) -> ModSource {
    let period = match src {
        ModSource::Lfo(l) => Some(l.period),
        ModSource::Steps(s) => Some(s.period),
        ModSource::Random(r) => Some(r.period),
        ModSource::Envelope(_) | ModSource::AudioFollower(_) => None,
    };
    match (kind, src) {
        (Kind::Lfo, ModSource::Lfo(_))
        | (Kind::Steps, ModSource::Steps(_))
        | (Kind::Envelope, ModSource::Envelope(_))
        | (Kind::Random, ModSource::Random(_))
        | (Kind::Follower, ModSource::AudioFollower(_)) => src,
        (Kind::Lfo, _) => ModSource::Lfo(Lfo { period: period.unwrap_or(Timebase::Sync(4)), ..Lfo::default() }),
        // A pattern, not sixteen zeros: a step source that came in silent would read as a broken modulator.
        (Kind::Steps, _) => ModSource::Steps(Steps { values: step_preset(1), period: period.unwrap_or(Timebase::Sync(1)), smooth: false }),
        (Kind::Envelope, _) => ModSource::Envelope(envelope_preset(0)),
        (Kind::Random, _) => ModSource::Random(Random { period: period.unwrap_or(Timebase::Sync(4)), seed: os_model::modulation::fresh_seed(ModulatorId(0)), ..Random::default() }),
        (Kind::Follower, _) => ModSource::AudioFollower(AudioFollower::default()),
    }
}

/// The sync rates the Rate items offer, in sixteenths, with the name a musician reads them by.
const SYNC_RATES: [(u16, &str); 6] = [(1, "1/16"), (2, "1/8"), (4, "1/4"), (8, "1/2"), (16, "1 bar"), (32, "2 bars")];

/// The free rates, in hertz.
const FREE_RATES: [(f32, &str); 6] = [(0.25, "0.25 Hz"), (0.5, "0.5 Hz"), (1.0, "1 Hz"), (2.0, "2 Hz"), (4.0, "4 Hz"), (8.0, "8 Hz")];

/// A period as the interface names it ("1/4", "1 bar", "2 Hz").
pub fn rate_text(period: Timebase) -> String {
    match period {
        Timebase::Sync(n) => SYNC_RATES.iter().find(|(k, _)| *k == n).map_or_else(|| format!("{n}/16"), |&(_, l)| l.to_string()),
        Timebase::Free(h) => format!("{h} Hz"),
    }
}

/// `period` as the rate knob's 0..=1 position. Synced rates step through the six musical values; free rates run
/// logarithmically from 0.0625 Hz to 32 Hz, so 1 Hz sits below the middle and dragging up raises it.
pub fn rate_pos(period: Timebase) -> f32 {
    match period {
        Timebase::Sync(n) => {
            let i = SYNC_RATES.iter().position(|(k, _)| *k == n).unwrap_or(0);
            i as f32 / (SYNC_RATES.len() - 1) as f32
        }
        Timebase::Free(hz) => {
            let lo = 0.0625f32.ln();
            let hi = 32.0f32.ln();
            ((hz.max(0.0625).ln() - lo) / (hi - lo)).clamp(0.0, 1.0)
        }
    }
}

/// The rate `period` takes at rate-knob position `pos`, keeping its sync / free mode.
fn period_at(period: Timebase, pos: f32) -> Timebase {
    let pos = pos.clamp(0.0, 1.0);
    match period {
        Timebase::Sync(_) => {
            let i = (pos * (SYNC_RATES.len() - 1) as f32).round() as usize;
            Timebase::Sync(SYNC_RATES[i.min(SYNC_RATES.len() - 1)].0)
        }
        Timebase::Free(_) => {
            let hz = (0.0625f32.ln() + pos * (32.0f32.ln() - 0.0625f32.ln())).exp();
            Timebase::Free((hz * 100.0).round() / 100.0)
        }
    }
}

/// The mode button: free hertz becomes a beat, a synced rate becomes 1 Hz.
fn toggle_period(period: Timebase) -> Timebase {
    match period {
        Timebase::Sync(_) => Timebase::Free(1.0),
        Timebase::Free(_) => Timebase::Sync(4),
    }
}

fn shape_text(shape: LfoShape) -> &'static str {
    match shape {
        LfoShape::Sine => "sine",
        LfoShape::Triangle => "triangle",
        LfoShape::SawUp => "saw up",
        LfoShape::SawDown => "saw down",
        LfoShape::Square => "square",
        LfoShape::Random => "random",
    }
}

fn curve_text(curve: Curve) -> &'static str {
    match curve {
        Curve::Linear => "Linear",
        Curve::Exp => "Exponential",
        Curve::Log => "Logarithmic",
        Curve::S => "S-curve",
    }
}

/// What a modulator drives with, as a hint reading ("LFO sine 1/4", "steps 1 bar", "envelope").
pub fn source_text(src: ModSource) -> String {
    match src {
        ModSource::Lfo(l) => format!("LFO {} {}", shape_text(l.shape), rate_text(l.period)),
        ModSource::Steps(s) => format!("steps {}", rate_text(s.period)),
        ModSource::Envelope(_) => "envelope".into(),
        ModSource::Random(r) => format!("random {}", rate_text(r.period)),
        ModSource::AudioFollower(_) => "audio follower".into(),
    }
}

/// A sixteen-step pattern the editor can lay: clear, stairs up, stairs down, alternating, and a fixed scatter (fixed so
/// undo then the same entry lays the same pattern).
fn step_preset(k: u8) -> [i8; 16] {
    let mut v = [0i8; 16];
    for (i, s) in v.iter_mut().enumerate() {
        let up = (i as i32 * 100 / 15) as i8;
        *s = match k {
            0 => 0,
            1 => up,
            2 => -up,
            3 => {
                if i % 2 == 0 {
                    100
                } else {
                    -100
                }
            }
            _ => (((i as u32).wrapping_mul(2654435761) >> 24) as i32 % 201 - 100) as i8,
        };
    }
    v
}

/// An envelope the editor can lay: a pluck, an organ, a pad and a tick.
fn envelope_preset(k: u8) -> Envelope {
    match k {
        0 => Envelope { attack: 0.005, decay: 0.25, sustain: 0.0, release: 0.2 },
        1 => Envelope { attack: 0.02, decay: 0.3, sustain: 0.7, release: 0.4 },
        2 => Envelope { attack: 1.2, decay: 1.5, sustain: 0.8, release: 2.0 },
        _ => Envelope { attack: 0.001, decay: 0.08, sustain: 0.0, release: 0.05 },
    }
}

fn same_env(a: &Envelope, b: &Envelope) -> bool {
    (a.attack - b.attack).abs() < 1e-4 && (a.decay - b.decay).abs() < 1e-4 && (a.sustain - b.sustain).abs() < 1e-4 && (a.release - b.release).abs() < 1e-4
}

/// The Rate items: the six synced rates, then the six free ones, ticked where the source's period is now.
fn rate_items(items: &mut Vec<PItem>, id: ModulatorId, cur: Timebase, make: impl Fn(Timebase) -> ModSource) {
    items.push(PItem::head("Rate"));
    for (n, label) in SYNC_RATES {
        items.push(PItem::new(label, Cmd::Mod(ModCmd::SetSource(id, make(Timebase::Sync(n))))).mark(Mark::Radio(cur == Timebase::Sync(n))));
    }
    for (h, label) in FREE_RATES {
        items.push(PItem::new(label, Cmd::Mod(ModCmd::SetSource(id, make(Timebase::Free(h))))).mark(Mark::Radio(cur == Timebase::Free(h))));
    }
}

/// One modulator's editor, as the submenu the knob's Modulation part opens for it: on / off, its kind, its shape or
/// pattern, its rate, and — when `target` is a control it drives — that assignment's depth, curve and polarity. `None`
/// for an unknown modulator.
pub fn editor_menu(s: &State, id: ModulatorId, target: u32) -> Option<PItem> {
    let m = s.modulation.get(id)?;
    let (name, source, enabled) = (name_of(m), m.source, m.enabled);
    let here = path_in(s, target).and_then(|path| s.modulation.get(id).and_then(|m| m.targets.iter().find(|t| t.path == path).copied()));
    let mut items = vec![
        PItem::new(if enabled { "On" } else { "Off" }, Cmd::Mod(ModCmd::SetEnabled(id, !enabled))).mark(Mark::Check(enabled)),
        PItem::sep(),
        PItem::head("Source"),
    ];
    for kind in [Kind::Lfo, Kind::Steps, Kind::Envelope] {
        items.push(PItem::new(kind.label(), Cmd::Mod(ModCmd::SetKind(id, kind))).mark(Mark::Radio(Kind::of(source) == kind)));
    }
    match source {
        ModSource::Lfo(l) => {
            items.push(PItem::head("Shape"));
            for (shape, label) in [
                (LfoShape::Sine, "Sine"),
                (LfoShape::Triangle, "Triangle"),
                (LfoShape::SawUp, "Saw up"),
                (LfoShape::SawDown, "Saw down"),
                (LfoShape::Square, "Square"),
                (LfoShape::Random, "Random"),
            ] {
                let cmd = Cmd::Mod(ModCmd::SetSource(id, ModSource::Lfo(Lfo { shape, ..l })));
                items.push(PItem::new(label, cmd).mark(Mark::Radio(l.shape == shape)));
            }
            rate_items(&mut items, id, l.period, |period| ModSource::Lfo(Lfo { period, ..l }));
        }
        ModSource::Steps(st) => {
            items.push(PItem::head("Pattern"));
            for (k, label) in [(0, "Clear"), (1, "Stairs up"), (2, "Stairs down"), (3, "Alternating"), (4, "Scatter")] {
                let cmd = Cmd::Mod(ModCmd::SetSource(id, ModSource::Steps(Steps { values: step_preset(k), ..st })));
                items.push(PItem::new(label, cmd).mark(Mark::Check(st.values == step_preset(k))));
            }
            let glide = Cmd::Mod(ModCmd::SetSource(id, ModSource::Steps(Steps { smooth: !st.smooth, ..st })));
            items.push(PItem::new("Glide", glide).mark(Mark::Check(st.smooth)));
            rate_items(&mut items, id, st.period, |period| ModSource::Steps(Steps { period, ..st }));
        }
        ModSource::Envelope(e) => {
            items.push(PItem::head("Shape"));
            for (k, label) in [(0, "Pluck"), (1, "Organ"), (2, "Pad"), (3, "Tick")] {
                let preset = envelope_preset(k);
                let cmd = Cmd::Mod(ModCmd::SetSource(id, ModSource::Envelope(preset)));
                items.push(PItem::new(label, cmd).mark(Mark::Radio(same_env(&e, &preset))));
            }
        }
        ModSource::Random(r) => {
            items.push(PItem::head("Distribution"));
            for (d, label) in [(Distribution::Uniform, "Uniform"), (Distribution::Gaussian, "Gaussian")] {
                let cmd = Cmd::Mod(ModCmd::SetSource(id, ModSource::Random(Random { distribution: d, ..r })));
                items.push(PItem::new(label, cmd).mark(Mark::Radio(r.distribution == d)));
            }
            items.push(PItem::head("Smoothing"));
            for (s, label) in [(0.0, "Hard steps"), (0.5, "Half smoothed"), (1.0, "Fully smoothed")] {
                let cmd = Cmd::Mod(ModCmd::SetSource(id, ModSource::Random(Random { smooth: s, interpolate: false, ..r })));
                items.push(PItem::new(label, cmd).mark(Mark::Radio((r.smooth - s).abs() < 0.01 && !r.interpolate)));
            }
            let interp = Cmd::Mod(ModCmd::SetSource(id, ModSource::Random(Random { interpolate: !r.interpolate, ..r })));
            items.push(PItem::new("Interpolate instead of hold", interp).mark(Mark::Check(r.interpolate)));
            let seed = Cmd::Mod(ModCmd::RerollSeed(id));
            items.push(PItem::new("Reroll seed", seed));
            rate_items(&mut items, id, r.period, |period| ModSource::Random(Random { period, ..r }));
        }
        ModSource::AudioFollower(f) => {
            items.push(PItem::head("Detection"));
            for (d, label) in [(Detection::Rms, "RMS"), (Detection::Peak, "Peak")] {
                let cmd = Cmd::Mod(ModCmd::SetSource(id, ModSource::AudioFollower(AudioFollower { detection: d, ..f })));
                items.push(PItem::new(label, cmd).mark(Mark::Radio(f.detection == d)));
            }
            items.push(PItem::head("Band"));
            for (band, label) in [(Band::Full, "Full spectrum"), (Band::Low, "Low"), (Band::Mid, "Mid"), (Band::High, "High")] {
                let cmd = Cmd::Mod(ModCmd::SetSource(id, ModSource::AudioFollower(AudioFollower { band, ..f })));
                items.push(PItem::new(label, cmd).mark(Mark::Radio(f.band == band)));
            }
        }
    }
    if let Some(t) = here {
        items.extend([PItem::sep(), PItem::head("This control")]);
        for pct in [5, 10, 25, 50, 75, 100] {
            for (sign, prefix) in [(1.0f32, "+"), (-1.0f32, "\u{2212}")] {
                let d = sign * pct as f32 / 100.0;
                let cmd = Cmd::Mod(ModCmd::SetDepth(id, target, d));
                items.push(PItem::new(format!("{prefix}{pct}%"), cmd).mark(Mark::Check((t.depth - d).abs() < 0.005)));
            }
        }
        items.push(PItem::head("Curve"));
        for curve in [Curve::Linear, Curve::Exp, Curve::Log, Curve::S] {
            let cmd = Cmd::Mod(ModCmd::SetCurve(id, target, curve));
            items.push(PItem::new(curve_text(curve), cmd).mark(Mark::Radio(t.curve == curve)));
        }
        items.push(PItem::head("Polarity"));
        for (pol, label) in [(Polarity::Bipolar, "Bipolar"), (Polarity::Unipolar, "Unipolar")] {
            let cmd = Cmd::Mod(ModCmd::SetPolarity(id, target, pol));
            items.push(PItem::new(label, cmd).mark(Mark::Radio(t.polarity == pol)));
        }
    }
    items.push(PItem::sep());
    items.push(PItem::new("Remove modulator", Cmd::Mod(ModCmd::Remove(id))));
    Some(PItem::sub(format!("Edit {name}"), items))
}

/// How far the enabled modulators drive `target` together, clamped to ±1: what the depth ring around a knob shows.
/// `None` when no enabled modulator drives it (or it drives it by nothing).
pub fn depth_on(s: &State, target: u32) -> Option<f32> {
    let path = path_in(s, target)?;
    let total: f32 = s.modulation.modulators.iter().filter(|m| m.enabled).flat_map(|m| m.targets.iter().filter(|t| t.path == path)).map(|t| t.depth).sum();
    (total.abs() > 0.001).then(|| total.clamp(-1.0, 1.0))
}

/// The modulator tiles' scrolling output history: the last two seconds of each modulator's actual output, sampled by
/// the UI from the lock-free snapshot at about 30 Hz. It is view state, never saved.
#[derive(Debug, Default)]
pub struct History {
    at: Option<Instant>,
    /// When the tiles started sampling: the preview clock used while the transport is stopped.
    start: Option<Instant>,
    series: BTreeMap<u32, VecDeque<f32>>,
}

impl History {
    /// Samples at most every 33 ms. Playing, the values are the engine's actual outputs; stopped, the sources are
    /// evaluated on a free-running preview clock so the plots still move (a follower has no signal then and stays
    /// at zero). `tempo` gives a sync source its project-tempo speed.
    pub fn sample(&mut self, s: &State, playing: bool, tempo: f64) {
        let now = Instant::now();
        if self.at.is_some_and(|t| now.duration_since(t).as_millis() < 33) {
            return;
        }
        self.at = Some(now);
        let start = *self.start.get_or_insert(now);
        let secs = now.duration_since(start).as_secs_f64();
        let beats = secs * tempo.max(1.0) / 60.0;
        for m in &s.modulation.modulators {
            let v = if playing {
                s.mod_snapshot.output(m.id.0).unwrap_or(0.0)
            } else {
                os_engine::source_value(&m.source, m.id.0, beats, secs)
            };
            let q = self.series.entry(m.id.0).or_default();
            q.push_back(v);
            while q.len() > 60 {
                q.pop_front();
            }
        }
        // A removed modulator's history goes with it.
        self.series.retain(|id, _| s.modulation.get(ModulatorId(*id)).is_some());
    }

    pub fn get(&self, id: ModulatorId) -> Option<&VecDeque<f32>> {
        self.series.get(&id.0)
    }
}

/// A stepped parameter with more steps than this is a continuous control stored as an integer (Lemon Trio's knobs),
/// not a selector or switch. Its steps are what the LFO can sweep through.
const CONTINUOUS_STEPS: u32 = 32;

/// Whether a control may receive project modulation (the handoff's `is_modulatable`). Continuous plugin parameters
/// and the mixer's own controls qualify; discrete/boolean parameters, hidden ones and everything else do not. This is
/// the one predicate behind the blue routing tint, the drag and the browser.
pub fn is_modulatable(s: &State, target: u32) -> bool {
    use os_model::target::Target;
    match target::decode(target) {
        Target::MixerVolume(_)
        | Target::MixerPan(_)
        | Target::MixerSeparation(_)
        | Target::MixerEq(..)
        | Target::MixerSend(..)
        | Target::SlotMix(..) => true,
        Target::ChannelParam(..) | Target::SlotParam(..) => {
            let Some((key, index)) = crate::params::key_of(s, target::decode(target)) else { return false };
            let Some(e) = s.params.lists.get(&key).and_then(|l| l.params.get(index)) else { return false };
            // A knob with a wide integer range (Trio's cutoff, pitch, envelope times) is continuous for modulation;
            // a handful of steps is a shape selector or a switch.
            !e.hidden && (e.steps == 0 || e.steps > CONTINUOUS_STEPS)
        }
        _ => false,
    }
}

/// A target's user-facing name ("Lemon Filter - Cutoff", "Master - Volume"): what the browser and the parameter-side
/// menu list.
pub fn target_name(s: &State, target: u32) -> String {
    if let Some((key, index)) = crate::params::key_of(s, target::decode(target)) {
        let name = s
            .params
            .lists
            .get(&key)
            .and_then(|l| l.params.get(index))
            .map_or_else(|| format!("Parameter {}", index + 1), |e| e.name.clone());
        return format!("{} - {name}", crate::params::owner_name(s, key));
    }
    format!("Control {target:#x}")
}

/// Every assignment driving `target`, as (modulator, depth, enabled), including assignments whose modulator is off.
/// The parameter-side menu lists these so one source can be removed without touching the others.
pub fn routings_on(s: &State, target: u32) -> Vec<(ModulatorId, f32, bool)> {
    let Some(path) = path_in(s, target) else { return Vec::new() };
    s.modulation
        .modulators
        .iter()
        .flat_map(|m| m.targets.iter().filter(|t| t.path == path).map(move |t| (m.id, t.depth, t.enabled && m.enabled)))
        .collect()
}

/// The current live value of `target` from the engine snapshot (the UI's moving cursor), `None` when the engine has
/// not published it (no graph, or a target the graph does not drive).
pub fn live_value(s: &State, target: u32) -> Option<f32> {
    s.mod_snapshot.value(target)
}

/// What a knob's Modulation menu asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum ModCmd {
    /// A new LFO driving the parameter `Target` names.
    New(u32),
    /// A new modulator of `source` in project slot `.0` (the empty slot's add menu).
    AddInSlot(u8, ModSource),
    /// Copied modulators, each pasted into its slot with a fresh id and its settings and routings intact.
    PasteMany(Vec<(u8, Modulator)>),
    /// Modulator `.0` starts driving that parameter.
    Assign(ModulatorId, u32),
    /// Assign and set the depth in one step (routing mode's drag on release), by the assignment's persistent path.
    AssignDepthPath(ModulatorId, ParameterPath, f32),
    /// Modulator `.0` stops driving it.
    Unassign(ModulatorId, u32),
    /// The same, by the assignment's persistent path (the browser and the parameter-side menu).
    UnassignPath(ModulatorId, ParameterPath),
    /// A modulator goes, with every assignment it had.
    Remove(ModulatorId),
    /// Several modulators go together, one undo step.
    RemoveMany(Vec<ModulatorId>),
    /// A modulator on or off: its assignments stay, it drives nothing while off.
    SetEnabled(ModulatorId, bool),
    /// One routing on or off (the browser's checkbox).
    SetTargetEnabled(ModulatorId, ParameterPath, bool),
    /// A source as it is (a shape, a pattern, a glide toggle, a rate).
    SetSource(ModulatorId, ModSource),
    /// Its kind, keeping the rate where the new kind has one.
    SetKind(ModulatorId, Kind),
    /// How far modulator `.0` drives the parameter `Target` names.
    SetDepth(ModulatorId, u32, f32),
    /// The same, by path.
    SetDepthPath(ModulatorId, ParameterPath, f32),
    /// That one assignment's response curve.
    SetCurve(ModulatorId, u32, Curve),
    /// And whether it swings both ways or only up from the parameter's value.
    SetPolarity(ModulatorId, u32, Polarity),
    /// A new seed for a random modulator's sequence.
    RerollSeed(ModulatorId),
    /// The user's name for it.
    Rename(ModulatorId, String),
    /// Every assignment it has, removed.
    Clear(ModulatorId),
    /// Modulator `.0` drives parameter `.2` of modulator `.1`, at `.3` (a route inside the graph).
    Route(ModulatorId, ModulatorId, os_model::modulation::ModParam, f32),
    /// That route, removed.
    Unroute(ModulatorId, ModulatorId, os_model::modulation::ModParam),
}

/// Runs a [`ModCmd`] as one undo step (the graph and the identities it names are part of the project's undo), and hands
/// the graph to playback at once so the change is heard without touching anything else.
pub fn run(app: &mut crate::app::App, cmd: ModCmd) -> Option<String> {
    let msg = crate::control::edit_app(app, "Modulation", crate::control::command::Region::Project, |app| {
        let s = &mut app.state;
        let path = match cmd {
            ModCmd::New(t)
            | ModCmd::Assign(_, t)
            | ModCmd::Unassign(_, t)
            | ModCmd::SetDepth(_, t, _)
            | ModCmd::SetCurve(_, t, _)
            | ModCmd::SetPolarity(_, t, _) => target_path(s, t),
            ModCmd::AddInSlot(..)
            | ModCmd::PasteMany(..)
            | ModCmd::AssignDepthPath(..)
            | ModCmd::UnassignPath(..)
            | ModCmd::SetTargetEnabled(..)
            | ModCmd::SetDepthPath(..)
            | ModCmd::Remove(_)
            | ModCmd::RemoveMany(_)
            | ModCmd::SetEnabled(..)
            | ModCmd::SetSource(..)
            | ModCmd::SetKind(..)
            | ModCmd::RerollSeed(_)
            | ModCmd::Rename(..)
            | ModCmd::Clear(_)
            | ModCmd::Route(..)
            | ModCmd::Unroute(..) => None,
        };
        let msg = match cmd {
            ModCmd::New(_) => path.map(|path| {
                let id = s.modulation.add(os_model::modulation::ModSource::default());
                s.modulation.assign(id, os_model::modulation::ModTarget::new(path));
                let m = s.modulation.get(id).expect("just added");
                format!("{} now drives this control", name_of(m))
            }),
            ModCmd::AddInSlot(slot, source) => s.modulation.add_in_slot(slot, source).and_then(|id| {
                let m = s.modulation.get(id)?;
                Some(format!("{} added to slot {}", name_of(m), slot + 1))
            }),
            ModCmd::PasteMany(clips) => {
                let n = clips.len();
                let mut done = 0usize;
                for (slot, clip) in clips {
                    let Modulator { name, source, targets, enabled, .. } = clip;
                    let Some(id) = s.modulation.add_in_slot(slot, source) else { continue };
                    let Some(m) = s.modulation.get_mut(id) else { continue };
                    m.name = name;
                    m.targets = targets;
                    m.enabled = enabled;
                    done += 1;
                }
                (done > 0).then(|| if n == 1 { "1 modulator pasted".to_string() } else { format!("{done} modulators pasted") })
            }
            ModCmd::Assign(id, _) => path.map(|path| {
                s.modulation.assign(id, os_model::modulation::ModTarget::new(path));
                let m = s.modulation.get(id).expect("an existing modulator");
                format!("{} now drives this control", name_of(m))
            }),
            ModCmd::AssignDepthPath(id, path, depth) => {
                let target = os_model::modulation::ModTarget::new(path).with_depth(depth);
                s.modulation.assign(id, target);
                // A second routing on the same parameter updates its depth instead of being refused.
                if let Some(m) = s.modulation.get_mut(id) {
                    if let Some(t) = m.targets.iter_mut().find(|t| t.path == path) {
                        *t = t.with_depth(depth);
                    }
                }
                Some(format!("Depth {:+.0}%", depth * 100.0))
            }
            ModCmd::Unassign(id, _) => path.and_then(|path| {
                s.modulation.unassign(id, path).then(|| "Stopped driving this control".to_string())
            }),
            ModCmd::UnassignPath(id, path) => {
                s.modulation.unassign(id, path).then(|| "Routing removed".to_string())
            }
            ModCmd::Remove(id) => s.modulation.remove(id).map(|m| format!("{} removed", name_of(&m))),
            ModCmd::RemoveMany(ids) => {
                let n = ids.iter().filter_map(|id| s.modulation.remove(*id)).count();
                (n > 0).then(|| if n == 1 { "1 modulator removed".to_string() } else { format!("{n} modulators removed") })
            }
            ModCmd::SetEnabled(id, on) => s.modulation.get_mut(id).map(|m| {
                m.enabled = on;
                format!("{}: {}", name_of(m), if on { "on" } else { "off" })
            }),
            ModCmd::SetTargetEnabled(id, path, on) => s.modulation.get_mut(id).and_then(|m| {
                let t = m.targets.iter_mut().find(|t| t.path == path)?;
                t.enabled = on;
                Some(if on { "Routing on".to_string() } else { "Routing off".to_string() })
            }),
            ModCmd::SetSource(id, src) => s.modulation.get_mut(id).map(|m| {
                m.source = src;
                format!("{}: {}", name_of(m), source_text(m.source))
            }),
            ModCmd::SetKind(id, kind) => s.modulation.get_mut(id).map(|m| {
                m.source = set_kind(m.source, kind);
                format!("{}: {}", name_of(m), source_text(m.source))
            }),
            ModCmd::SetDepth(id, _, depth) => path.and_then(|path| {
                s.modulation.get_mut(id).and_then(|m| {
                    let t = m.targets.iter_mut().find(|t| t.path == path)?;
                    *t = t.with_depth(depth);
                    Some(format!("Depth {:+.0}%", t.depth * 100.0))
                })
            }),
            ModCmd::SetDepthPath(id, path, depth) => s.modulation.get_mut(id).and_then(|m| {
                let t = m.targets.iter_mut().find(|t| t.path == path)?;
                *t = t.with_depth(depth);
                Some(format!("Depth {:+.0}%", t.depth * 100.0))
            }),
            ModCmd::SetCurve(id, _, curve) => path.and_then(|path| {
                s.modulation.get_mut(id).and_then(|m| {
                    let t = m.targets.iter_mut().find(|t| t.path == path)?;
                    t.curve = curve;
                    Some(format!("Curve: {}", curve_text(curve)))
                })
            }),
            ModCmd::SetPolarity(id, _, pol) => path.and_then(|path| {
                s.modulation.get_mut(id).and_then(|m| {
                    let t = m.targets.iter_mut().find(|t| t.path == path)?;
                    t.polarity = pol;
                    Some(if pol == Polarity::Unipolar { "Unipolar".to_string() } else { "Bipolar".to_string() })
                })
            }),
            ModCmd::RerollSeed(id) => s.modulation.get_mut(id).map(|m| {
                m.source = match m.source {
                    ModSource::Random(r) => ModSource::Random(Random { seed: os_model::modulation::fresh_seed(id), ..r }),
                    other => other,
                };
                format!("{}: new seed", name_of(m))
            }),
            ModCmd::Rename(id, name) => s.modulation.get_mut(id).map(|m| {
                m.name = name;
                format!("Renamed to {}", name_of(m))
            }),
            ModCmd::Clear(id) => s.modulation.get_mut(id).and_then(|m| {
                let n = m.targets.len();
                m.targets.clear();
                (n > 0).then(|| format!("{n} routings removed"))
            }),
            ModCmd::Route(from, to, param, depth) => {
                if !s.modulation.route(from, to, param, depth) {
                    return None;
                }
                let (driver, driven) = (s.modulation.get(from).map(name_of), s.modulation.get(to).map(name_of));
                Some(format!(
                    "{} drives {} {}",
                    driver.unwrap_or_default(),
                    param.label().to_lowercase(),
                    driven.unwrap_or_default()
                ))
            }
            ModCmd::Unroute(from, to, param) => s
                .modulation
                .unroute(from, to, param)
                .then(|| format!("{} route removed", param.label())),
        };
        if msg.is_some() {
            s.piano.dirty = true;
        }
        msg
    });
    if msg.is_some() {
        sync(app);
        app.invalidate();
    }
    msg
}

/// The selected slots' modulators, in slot order. With no slot selection, the open editor/menu/routing is the
/// fallback (a browser row can open a modulator without touching the slot row).
pub fn selected_ids(s: &State) -> Vec<ModulatorId> {
    let mut v: Vec<ModulatorId> = s
        .mod_sel
        .iter()
        .filter_map(|slot| s.modulation.modulator_at_slot(*slot))
        .map(|m| m.id)
        .collect();
    if v.is_empty() {
        if let Some(id) = s.mod_editor.or(s.mod_menu).or(s.mod_routing)
            && s.modulation.get(id).is_some()
        {
            v.push(id);
        }
    }
    v
}

fn selection_has(s: &State, id: ModulatorId) -> bool {
    s.mod_sel
        .iter()
        .any(|slot| s.modulation.modulator_at_slot(*slot).is_some_and(|m| m.id == id))
}

/// A plain click: this one modulator, and its slot is the arrow-key cursor.
pub fn select_one(s: &mut State, id: ModulatorId) {
    let Some(slot) = s.modulation.get(id).map(|m| m.slot) else { return };
    s.mod_sel = vec![slot];
    s.mod_cursor = Some(slot);
    s.mod_anchor = Some(slot);
    s.keyboard_focus = crate::app::KeyboardFocus::Modulators;
}

/// Shift-click: add the modulator to the selection, or take it out, without opening or closing its editor.
pub fn extend_selection(app: &mut crate::app::App, id: ModulatorId) {
    let Some(slot) = app.state.modulation.get(id).map(|m| m.slot) else { return };
    let s = &mut app.state;
    if let Some(i) = s.mod_sel.iter().position(|s| *s == slot) {
        s.mod_sel.remove(i);
    } else {
        s.mod_sel.push(slot);
        s.mod_sel.sort_unstable();
        s.mod_anchor.get_or_insert(slot);
    }
    s.mod_cursor = Some(slot);
    s.keyboard_focus = crate::app::KeyboardFocus::Modulators;
    s.mod_add_slot = None;
    s.mod_menu = None;
    if s.mod_editor.is_some_and(|id| !selection_has(s, id)) {
        s.mod_editor = None;
    }
}

/// Move the modulator cursor by `dir`, selecting the slot's modulator (or clearing the selection on an empty slot).
/// With Shift, select the occupied slots from the anchor to the cursor inclusive.
pub fn move_selection(app: &mut crate::app::App, dir: i32, shift: bool) -> bool {
    let s = &mut app.state;
    let last = s.modulation.slots() as u8 - 1;
    let cur = s.mod_cursor.or_else(|| s.mod_sel.first().copied()).unwrap_or(0).min(last);
    let next = if dir < 0 { cur.saturating_sub(1) } else { (cur + 1).min(last) };
    if shift {
        let anchor = s.mod_anchor.unwrap_or(cur).min(last);
        let (a, b) = if anchor <= next { (anchor, next) } else { (next, anchor) };
        s.mod_sel = (a..=b)
            .filter(|slot| s.modulation.modulator_at_slot(*slot).is_some())
            .collect();
    } else {
        s.mod_sel = s.modulation.modulator_at_slot(next).map(|_| vec![next]).unwrap_or_default();
        s.mod_anchor = Some(next);
    }
    s.mod_cursor = Some(next);
    s.mod_add_slot = None;
    s.mod_menu = None;
    if s.mod_editor.is_some_and(|id| !selection_has(s, id)) {
        s.mod_editor = None;
    }
    if s.mod_routing.is_some_and(|id| !selection_has(s, id)) {
        s.mod_routing = None;
    }
    true
}

/// Ctrl+A: every modulator in the strip is selected (the devices' Ctrl+A, for the modulator row).
pub fn select_all(app: &mut crate::app::App) -> bool {
    let s = &mut app.state;
    let slots: Vec<u8> = (0..s.modulation.slots() as u8)
        .filter(|slot| s.modulation.modulator_at_slot(*slot).is_some())
        .collect();
    let Some((&first, &last)) = slots.first().zip(slots.last()) else { return false };
    s.mod_sel = slots;
    s.mod_anchor = Some(first);
    s.mod_cursor = Some(last);
    s.mod_add_slot = None;
    s.mod_menu = None;
    true
}

/// Delete / Backspace: removes every selected modulator, closing their editor/menu, as one undo step.
pub fn delete_selected(app: &mut crate::app::App) -> bool {
    let ids = selected_ids(&app.state);
    if ids.is_empty() {
        return false;
    }
    if run(app, ModCmd::RemoveMany(ids.clone())).is_some() {
        let s = &mut app.state;
        s.mod_sel.clear();
        if s.mod_editor.is_some_and(|id| ids.contains(&id)) {
            s.mod_editor = None;
        }
        if s.mod_menu.is_some_and(|id| ids.contains(&id)) {
            s.mod_menu = None;
        }
        if s.mod_routing.is_some_and(|id| ids.contains(&id)) {
            s.mod_routing = None;
        }
        app.state.message = Some(if ids.len() == 1 { "1 modulator removed".into() } else { format!("{} modulators removed", ids.len()) });
        app.invalidate();
    }
    true
}

/// Ctrl+C: every selected modulator, with its source, name, enabled state and routings.
pub fn copy_selected(app: &mut crate::app::App) -> bool {
    let ids = selected_ids(&app.state);
    let clips: Vec<Modulator> = ids.iter().filter_map(|id| app.state.modulation.get(*id).cloned()).collect();
    if clips.is_empty() {
        return false;
    }
    let n = clips.len();
    app.state.mod_clipboard = clips;
    app.state.message = Some(if n == 1 { "1 modulator copied".into() } else { format!("{n} modulators copied") });
    app.invalidate();
    true
}

/// Ctrl+A selects every modulator in the strip, the modulator row's version of the device rack's Ctrl+A.
#[test]
fn ctrl_a_selects_every_modulator() {
        use winit::keyboard::Key;
        let mut app = crate::app::App::new(None);
        act(&mut app, ModAct::AddKind(0, Kind::Lfo));
        act(&mut app, ModAct::AddKind(1, Kind::Random));
        let first = app.state.modulation.modulator_at_slot(0).expect("the first modulator").id;
        select_one(&mut app.state, first);
        assert!(key(&mut app, &Key::Character("a".into()), true, false, false));
        assert_eq!(app.state.mod_sel, vec![0, 1], "both modulators selected");
        assert_eq!(app.state.mod_cursor, Some(1));
    }

    /// Enter opens the selected modulator's options menu, the arrows walk its rows and Enter runs the highlighted
    /// one; Ctrl+X copies the modulator and removes it.
    #[test]
    fn the_options_menu_is_walked_by_the_keyboard_and_ctrl_x_cuts() {
        use winit::keyboard::{Key, NamedKey};
        let mut app = crate::app::App::new(None);
        act(&mut app, ModAct::AddKind(0, Kind::Lfo));
        let id = app.state.modulation.modulator_at_slot(0).expect("a modulator").id;
        let press = |app: &mut crate::app::App, k: Key| key(app, &k, false, false, false);
        assert!(press(&mut app, Key::Named(NamedKey::Enter)));
        assert_eq!(app.state.mod_menu, Some(id), "Enter opens the menu");
        assert_eq!(app.state.mod_menu_sel, 0, "on its first row");
        assert!(press(&mut app, Key::Named(NamedKey::ArrowDown)));
        assert_eq!(app.state.mod_menu_sel, 1, "the arrow walks the rows");
        assert!(press(&mut app, Key::Named(NamedKey::Enter)), "the row runs");
        assert_eq!(app.state.mod_menu, None, "and the menu closes");
        // Ctrl+X: the modulator is copied and cut out of its slot.
        assert!(key(&mut app, &Key::Character("x".into()), true, false, false));
        assert_eq!(app.state.modulation.modulator_at_slot(0), None, "cut out of the slot");
        assert_eq!(app.state.mod_clipboard.len(), 1, "the clipboard kept it");
    }

    /// Ctrl+X: copies the selected modulators and removes them (the removal is one undo step).
pub fn cut_selected(app: &mut crate::app::App) -> bool {
    if !copy_selected(app) {
        return false;
    }
    let n = app.state.mod_clipboard.len();
    delete_selected(app);
    app.state.message = Some(if n == 1 { "1 modulator cut".into() } else { format!("{n} modulators cut") });
    app.invalidate();
    true
}

/// Ctrl+V: pastes the copied modulators into empty slots, starting at the clicked/cursed empty slot and wrapping
/// through the project's slots. All of them must fit (no partial paste); the pasted ones become the selection
/// and the first one's editor opens.
pub fn paste_clipboard(app: &mut crate::app::App) -> bool {
    let clips = app.state.mod_clipboard.clone();
    if clips.is_empty() {
        return false;
    }
    let s = &mut app.state;
    let last = s.modulation.slots() as u8 - 1;
    let start = s.mod_add_slot.or(s.mod_cursor).unwrap_or(0).min(last);
    let free: Vec<u8> = (0..s.modulation.slots() as u8)
        .filter(|slot| s.modulation.modulator_at_slot(*slot).is_none())
        .collect();
    if free.len() < clips.len() {
        s.message = Some(format!("Need {} empty modulator slots; only {} available", clips.len(), free.len()));
        app.invalidate();
        return true;
    }
    let slots: Vec<u8> = free
        .iter()
        .copied()
        .filter(|slot| *slot >= start)
        .chain(free.iter().copied().filter(|slot| *slot < start))
        .take(clips.len())
        .collect();
    let jobs: Vec<(u8, Modulator)> = slots.iter().copied().zip(clips).collect();
    if let Some(msg) = run(app, ModCmd::PasteMany(jobs)) {
        let made: Vec<u8> = slots
            .iter()
            .copied()
            .filter(|slot| app.state.modulation.modulator_at_slot(*slot).is_some())
            .collect();
        let first = made.first().and_then(|slot| app.state.modulation.modulator_at_slot(*slot)).map(|m| m.id);
        let s = &mut app.state;
        s.mod_add_slot = None;
        s.mod_menu = None;
        s.mod_sel = made.clone();
        s.mod_cursor = made.last().copied();
        s.mod_anchor = made.first().copied();
        s.mod_editor = first;
        s.message = Some(msg);
        app.invalidate();
    }
    true
}

/// The rows of a modulator's options menu: what the panel draws and the keyboard walks.
pub fn menu_items(s: &State, m: &Modulator) -> Vec<(String, ModAct)> {
    use os_model::modulation::ModSource;
    let mut v = vec![(
        if m.enabled { "Disable".to_string() } else { "Enable".to_string() },
        ModAct::Enable(m.id),
    )];
    if matches!(m.source, ModSource::Random(_)) {
        v.push(("Reroll seed".into(), ModAct::RerollSeed(m.id)));
    }
    // With routing mode armed, this modulator's own parameters are the routing's destinations: the armed modulator can
    // drive another modulator's rate, amount, phase or offset, which is how a route into the graph is made.
    if let Some(from) = s.mod_routing.filter(|a| *a != m.id)
        && let Some(driver) = s.modulation.get(from)
    {
        let name = name_of(driver);
        for param in os_model::modulation::ModParam::ALL {
            if !can_route(s, from, m.id, param) {
                continue;
            }
            // The route lives on the driver, naming this modulator's parameter.
            let driving = driver.targets.iter().any(|t| t.path.modulator() == Some((m.id, param)));
            let label = if driving {
                format!("Stop {name}: {}", param.label().to_lowercase())
            } else {
                format!("{name}: {}", param.label().to_lowercase())
            };
            v.push((label, ModAct::DriveParam(m.id, from, param)));
        }
    }
    v.push(("Clear routings".into(), ModAct::Clear(m.id)));
    v.push(("Remove modulator".into(), ModAct::Remove(m.id)));
    v
}

/// Open the options menu on the selected modulator, its keyboard row at the top.
pub fn open_menu(app: &mut crate::app::App) -> bool {
    let Some(id) = selected_ids(&app.state).last().copied().or(app.state.mod_menu) else { return false };
    act(app, ModAct::Menu(id));
    true
}

/// Keys while the options menu is open: arrows walk its rows, Enter runs the highlighted one, Escape closes it.
fn menu_key(app: &mut crate::app::App, key: &winit::keyboard::Key) -> bool {
    use winit::keyboard::{Key, NamedKey};
    let Some(id) = app.state.mod_menu else { return false };
    let Some(m) = app.state.modulation.get(id).cloned() else { return false };
    let items = menu_items(&app.state, &m);
    if items.is_empty() {
        return false;
    }
    match key {
        Key::Named(NamedKey::ArrowDown | NamedKey::ArrowRight) => {
            app.state.mod_menu_sel = (app.state.mod_menu_sel + 1) % items.len();
            app.invalidate();
            true
        }
        Key::Named(NamedKey::ArrowUp | NamedKey::ArrowLeft) => {
            let n = items.len();
            app.state.mod_menu_sel = (app.state.mod_menu_sel + n - 1) % n;
            app.invalidate();
            true
        }
        Key::Named(NamedKey::Enter) => {
            let act_item = items[app.state.mod_menu_sel.min(items.len() - 1)].1;
            act(app, act_item);
            true
        }
        Key::Named(NamedKey::Escape) => {
            app.state.mod_menu = None;
            app.invalidate();
            true
        }
        _ => false,
    }
}

/// The modulator window's keys (`App::key` calls this while its slot row/editor/menu is active).
pub fn key(app: &mut crate::app::App, key: &winit::keyboard::Key, ctrl: bool, alt: bool, shift: bool) -> bool {
    use winit::keyboard::{Key, NamedKey};
    if !ctrl && !alt && app.state.mod_menu.is_some() && menu_key(app, key) {
        return true;
    }
    match key {
        // Every one of these belongs to the modulator strip while it has the keyboard: the key is consumed even when
        // there is nothing to act on, so it never falls through to the piano roll's or the playlist's Ctrl+A / Ctrl+C /
        // Ctrl+X / Ctrl+V / Delete. That leak is what made Ctrl+A in the strip select notes outside it.
        Key::Named(NamedKey::Delete | NamedKey::Backspace) if !ctrl && !alt => {
            delete_selected(app);
            true
        }
        Key::Character(c) if ctrl && !alt && c.eq_ignore_ascii_case("a") => {
            select_all(app);
            true
        }
        Key::Character(c) if ctrl && !alt && c.eq_ignore_ascii_case("c") => {
            copy_selected(app);
            true
        }
        Key::Character(c) if ctrl && !alt && c.eq_ignore_ascii_case("x") => {
            cut_selected(app);
            true
        }
        Key::Character(c) if ctrl && !alt && c.eq_ignore_ascii_case("v") => {
            paste_clipboard(app);
            true
        }
        Key::Named(NamedKey::Enter) if !ctrl && !alt => {
            open_menu(app);
            true
        }
        Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowUp) if !ctrl && !alt => {
            move_selection(app, -1, shift);
            true
        }
        Key::Named(NamedKey::ArrowRight | NamedKey::ArrowDown) if !ctrl && !alt => {
            move_selection(app, 1, shift);
            true
        }
        _ => false,
    }
}

/// Puts the route at `depth` on the graph and hands the pointer a depth drag for it. The graph as it was goes into
/// the drag, so a release is one undo step and Esc restores it exactly.
fn depth_drag(app: &mut crate::app::App, id: ModulatorId, path: ParameterPath, depth: f32, before: os_model::modulation::ModGraph) {
    if let Some(m) = app.state.modulation.get_mut(id) {
        if let Some(t) = m.targets.iter_mut().find(|t| t.path == path) {
            *t = t.with_depth(depth);
        } else {
            m.targets.push(os_model::modulation::ModTarget::new(path).with_depth(depth));
        }
    }
    app.drag = Some(crate::app::Drag::ModDepth { id, path, y0: app.mouse.1, depth, before });
    sync(app);
    app.invalidate();
}

/// Routing mode: a press on `target`. The assignment is created at depth 0 so the arc and the sound follow the drag;
/// the base value is never written.
pub fn begin_routing_drag(app: &mut crate::app::App, id: ModulatorId, target: u32) {
    let Some(path) = target_path(&mut app.state, target) else { return };
    let before = app.state.modulation.clone();
    let depth = app
        .state
        .modulation
        .get(id)
        .and_then(|m| m.targets.iter().find(|t| t.path == path))
        .map_or(0.0, |t| t.depth);
    depth_drag(app, id, path, depth, before);
}

/// Routing mode, on another modulator's tile: a press on one of its knobs. The armed modulator starts driving that
/// parameter — an ordinary assignment, so the depth, curve, polarity, undo and `.lemon` come along — at the default
/// depth (an existing route keeps its own), and the pointer drags that depth. False when the graph would refuse.
pub fn begin_route_drag(app: &mut crate::app::App, from: ModulatorId, to: ModulatorId, param: os_model::modulation::ModParam) -> bool {
    if !can_route(&app.state, from, to, param) {
        return false;
    }
    let path = ParameterPath::on_modulator(to, param);
    let before = app.state.modulation.clone();
    let depth = app
        .state
        .modulation
        .get(from)
        .and_then(|m| m.targets.iter().find(|t| t.path == path))
        .map_or(os_model::modulation::ModTarget::DEPTH_DEFAULT, |t| t.depth);
    depth_drag(app, from, path, depth, before);
    true
}

/// Whether the graph would accept modulator `from` driving `param` of modulator `to`: the destination has that
/// parameter, and the route is neither a self-drive nor one that closes a loop (`ModGraph::route` refuses exactly
/// these). An existing route is always allowed, because its row is how it is taken away again.
pub fn can_route(s: &State, from: ModulatorId, to: ModulatorId, param: os_model::modulation::ModParam) -> bool {
    let Some(destination) = s.modulation.get(to) else { return false };
    if from == to || s.modulation.get(from).is_none() || !param.applies_to(&destination.source) {
        return false;
    }
    let driving = s
        .modulation
        .get(from)
        .is_some_and(|m| m.targets.iter().any(|t| t.path.modulator() == Some((to, param))));
    driving || !s.modulation.reaches(to, from)
}

/// The graph parameter a knob of the editor stands for. A knob the graph cannot address (a follower's attack, say)
/// is not a routing destination.
pub fn knob_param(field: KnobField) -> Option<os_model::modulation::ModParam> {
    use os_model::modulation::ModParam;
    match field {
        KnobField::Rate => Some(ModParam::Rate),
        KnobField::Amount => Some(ModParam::Amount),
        KnobField::Phase => Some(ModParam::Phase),
        KnobField::Offset => Some(ModParam::Offset),
        KnobField::Attack | KnobField::Release | KnobField::Gain | KnobField::Smooth => None,
    }
}

/// Hands the current graph and its snapshot to playback, and keeps the same snapshot in the app state for the UI's
/// live values. Every edit path ends here, so nothing has to be restarted.
pub fn sync(app: &mut crate::app::App) {
    let (spec, snapshot) = engine_spec(&app.state, &app.plugins);
    app.state.mod_snapshot = snapshot.clone();
    app.transport.set_modulation(spec, snapshot);
}

/// The modulator UI's actions (the eight slot row, its editor and its menus).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ModAct {
    /// A click on the strip's empty surface: it takes the editing keys (the keyboard focus) without changing the graph.
    FocusStrip,
    /// The empty slot's "+" (or a click elsewhere closes it).
    AddMenu(u8),
    CloseAdd,
    /// A type from the add menu.
    AddKind(u8, Kind),
    /// The patch-point button: enter / leave routing mode.
    Route(ModulatorId),
    /// A route into the graph from the options menu: modulator `.1` drives parameter `.2` of modulator `.0`, or stops
    /// driving it if it already does (the menu row is a switch).
    DriveParam(ModulatorId, ModulatorId, os_model::modulation::ModParam),
    /// Open / close a slot's graphical editor.
    Open(ModulatorId),
    /// Start the inline name field at `(x, y)` (the rack editor's title or the browser's row).
    Rename(ModulatorId, (f32, f32)),
    Close,
    /// Open / close a slot's right-click menu.
    Menu(ModulatorId),
    MenuClose,
    Enable(ModulatorId),
    Remove(ModulatorId),
    Clear(ModulatorId),
    RerollSeed(ModulatorId),
    /// The LFO shape selector steps to the next waveform.
    NextShape(ModulatorId),
    /// The rate rounds up (`+1`) or down (`-1`) to the next musical step.
    RateStep(ModulatorId, i32),
    /// The rate knob's mode button: tempo-synced rates and free hertz.
    RateMode(ModulatorId),
    ToggleBipolar(ModulatorId),
    ToggleDistribution(ModulatorId),
    ToggleInterpolate(ModulatorId),
    CycleBand(ModulatorId),
    CycleDetection(ModulatorId),
    /// A type button in the editor (keeps the rate where the new type has one).
    Kind(ModulatorId, Kind),
    /// A knob in the editor pressed: drag up / down to set it.
    Knob(ModulatorId, KnobField),
    /// An assigned control's arc endpoint: drag it to change that one routing's depth (the handoff §4.3).
    DepthDrag(ModulatorId, u32),
    /// The browser's depth cell: the same drag, by the assignment's persistent path.
    DepthDragPath(ModulatorId, ParameterPath),
    /// The browser's routing row: focus its target (select the track/device) and open the modulator's editor.
    Focus(ModulatorId, ParameterPath),
    /// The browser's routing switch / remove / depth.
    TargetEnabled(ModulatorId, ParameterPath, bool),
    RemovePath(ModulatorId, ParameterPath),
    DepthPath(ModulatorId, ParameterPath, f32),
}

impl ModAct {
    /// Whether this is a press on the modulator strip itself (its tiles and their buttons) rather than elsewhere in the
    /// program. The editor and its options menu are their own window, so a press there never reaches the main window;
    /// on the strip the editor stays open, and everywhere else it closes (`App::mouse_down`).
    pub fn on_the_strip(self) -> bool {
        matches!(
            self,
            ModAct::FocusStrip
                | ModAct::AddMenu(_)
                | ModAct::CloseAdd
                | ModAct::AddKind(..)
                | ModAct::Route(_)
                | ModAct::Open(_)
                | ModAct::Rename(..)
                | ModAct::Menu(_)
                | ModAct::Enable(_)
                | ModAct::Remove(_)
        )
    }
}

/// A knob in a modulator's editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnobField {
    Rate,
    Amount,
    Phase,
    Offset,
    Attack,
    Release,
    Gain,
    Smooth,
}

/// The value a knob shows, `0..=1`.
pub fn knob_value(source: ModSource, field: KnobField) -> f32 {
    match (source, field) {
        (ModSource::Lfo(l), KnobField::Rate) => rate_pos(l.period),
        (ModSource::Lfo(l), KnobField::Amount) => l.amount,
        (ModSource::Lfo(l), KnobField::Phase) => l.phase,
        (ModSource::Lfo(l), KnobField::Offset) => (l.offset + 1.0) * 0.5,
        (ModSource::Random(r), KnobField::Rate) => rate_pos(r.period),
        (ModSource::Random(r), KnobField::Amount) => r.amount,
        (ModSource::Random(r), KnobField::Smooth) => r.smooth,
        (ModSource::AudioFollower(f), KnobField::Amount) => f.amount,
        (ModSource::AudioFollower(f), KnobField::Attack) => (f.attack / 2.0).clamp(0.0, 1.0),
        (ModSource::AudioFollower(f), KnobField::Release) => (f.release / 10.0).clamp(0.0, 1.0),
        (ModSource::AudioFollower(f), KnobField::Gain) => (f.gain_db + 24.0) / 48.0,
        (ModSource::Steps(s), KnobField::Rate) => rate_pos(s.period),
        _ => 0.5,
    }
}

/// The source with `field` set from a `0..=1` knob value.
pub fn with_knob(source: ModSource, field: KnobField, v: f32) -> ModSource {
    let v = v.clamp(0.0, 1.0);
    match (source, field) {
        (ModSource::Lfo(l), KnobField::Rate) => ModSource::Lfo(Lfo { period: period_at(l.period, v), ..l }),
        (ModSource::Lfo(l), KnobField::Amount) => ModSource::Lfo(Lfo { amount: v, ..l }),
        (ModSource::Lfo(l), KnobField::Phase) => ModSource::Lfo(Lfo { phase: v, ..l }),
        (ModSource::Lfo(l), KnobField::Offset) => ModSource::Lfo(Lfo { offset: v * 2.0 - 1.0, ..l }),
        (ModSource::Random(r), KnobField::Rate) => ModSource::Random(Random { period: period_at(r.period, v), ..r }),
        (ModSource::Random(r), KnobField::Amount) => ModSource::Random(Random { amount: v, ..r }),
        (ModSource::Random(r), KnobField::Smooth) => ModSource::Random(Random { smooth: v, interpolate: false, ..r }),
        (ModSource::AudioFollower(f), KnobField::Amount) => ModSource::AudioFollower(AudioFollower { amount: v, ..f }),
        (ModSource::AudioFollower(f), KnobField::Attack) => ModSource::AudioFollower(AudioFollower { attack: (0.0005 + v * 2.0).max(0.0005), ..f }),
        (ModSource::AudioFollower(f), KnobField::Release) => ModSource::AudioFollower(AudioFollower { release: (0.001 + v * 10.0).max(0.001), ..f }),
        (ModSource::AudioFollower(f), KnobField::Gain) => ModSource::AudioFollower(AudioFollower { gain_db: v * 48.0 - 24.0, ..f }),
        (ModSource::Steps(st), KnobField::Rate) => ModSource::Steps(Steps { period: period_at(st.period, v), ..st }),
        (s, _) => s,
    }
}

/// Applies a modulator UI action. Edits that change the project run as one undo step (`run`); routing mode itself is
/// not an edit.
pub fn act(app: &mut crate::app::App, a: ModAct) {
    // A click on a modulator slot or its editor hands that strip the selection keys; a click on a device
    // hands them back (`App::act`).
    app.state.keyboard_focus = crate::app::KeyboardFocus::Modulators;
    // Routing mode, a press on another modulator's knob: this is the route's destination, so the press starts the
    // route and drags its depth instead of moving the knob (the device controls' gesture).
    if let ModAct::Knob(id, field) = a
        && let Some(param) = knob_param(field)
        && let Some(from) = app.state.mod_routing.filter(|from| *from != id)
        && begin_route_drag(app, from, id, param)
    {
        return;
    }
    let ctrl = app.ctrl;
    let s = &mut app.state;
    match a {
        ModAct::FocusStrip => {}
        ModAct::AddMenu(slot) => {
            s.mod_add_slot = if s.mod_add_slot == Some(slot) { None } else { Some(slot) };
            s.mod_sel.clear();
            s.mod_cursor = Some(slot);
            s.mod_anchor = Some(slot);
            s.mod_menu = None;
            s.mod_editor = None;
        }
        ModAct::CloseAdd => s.mod_add_slot = None,
        ModAct::AddKind(slot, kind) => {
            s.mod_add_slot = None;
            let source = set_kind(ModSource::default(), kind);
            // The strip's add tile sits in the first free slot; with every row taken it grows another line first.
            let slot = if slot == os_model::modulation::NO_SLOT {
                s.modulation.add_row();
                s.modulation.free_slot().unwrap_or(os_model::modulation::NO_SLOT)
            } else {
                slot
            };
            run(app, ModCmd::AddInSlot(slot, source));
            if let Some(id) = app.state.modulation.modulator_at_slot(slot).map(|m| m.id) {
                select_one(&mut app.state, id);
                app.state.mod_editor = Some(id);
            }
            return;
        }
        ModAct::DriveParam(to, from, param) => {
            s.mod_menu = None;
            let driving = s
                .modulation
                .get(from)
                .is_some_and(|m| m.targets.iter().any(|t| t.path.modulator() == Some((to, param))));
            if driving {
                run(app, ModCmd::Unroute(from, to, param));
            } else {
                run(app, ModCmd::Route(from, to, param, os_model::modulation::ModTarget::DEPTH_DEFAULT));
            }
            return;
        }
        ModAct::Route(id) => {
            select_one(s, id);
            s.mod_routing = if s.mod_routing == Some(id) { None } else { Some(id) };
            s.mod_flash = Some(Instant::now());
            s.mod_add_slot = None;
            s.mod_menu = None;
        }
        ModAct::Open(id) => {
            // Ctrl+click on another modulator's tile opens its panel without leaving routing mode, so the armed
            // modulator's route can be dropped straight onto its knobs (Bitwig's gesture). A plain click is an ordinary
            // open, which leaves routing mode, as every other click on a tile does.
            if ctrl && s.mod_routing.is_some_and(|from| from != id) && s.modulation.get(id).is_some() {
                select_one(s, id);
                s.mod_editor = Some(id);
                s.mod_add_slot = None;
            } else {
                select_one(s, id);
                s.mod_editor = if s.mod_editor == Some(id) { None } else { Some(id) };
                s.mod_routing = None;
                s.mod_menu = None;
                s.mod_add_slot = None;
            }
        }
        ModAct::Rename(id, at) => {
            crate::playlist::start_edit(s, crate::playlist::EditTarget::ModulatorName(id), at);
            return;
        }
        ModAct::Close => {
            s.mod_editor = None;
            s.mod_menu = None;
            s.mod_menu_sel = 0;
            s.mod_add_slot = None;
        }
        ModAct::Menu(id) => {
            select_one(s, id);
            s.mod_menu_sel = 0;
            s.mod_menu = if s.mod_menu == Some(id) { None } else { Some(id) };
            s.mod_editor = None;
            s.mod_add_slot = None;
        }
        ModAct::MenuClose => s.mod_menu = None,
        ModAct::Enable(id) => {
            let on = s.modulation.get(id).is_some_and(|m| !m.enabled);
            s.mod_menu = None;
            run(app, ModCmd::SetEnabled(id, on));
            return;
        }
        ModAct::Remove(id) => {
            if let Some(slot) = s.modulation.get(id).map(|m| m.slot) {
                s.mod_sel.retain(|s| *s != slot);
            }
            s.mod_menu = None;
            s.mod_editor = None;
            if s.mod_routing == Some(id) {
                s.mod_routing = None;
            }
            run(app, ModCmd::Remove(id));
            return;
        }
        ModAct::Clear(id) => {
            s.mod_menu = None;
            run(app, ModCmd::Clear(id));
            return;
        }
        ModAct::RerollSeed(id) => {
            s.mod_menu = None;
            run(app, ModCmd::RerollSeed(id));
            return;
        }
        ModAct::NextShape(id) => {
            let next = s.modulation.get(id).map(|m| match m.source {
                ModSource::Lfo(l) => {
                    let all = [LfoShape::Sine, LfoShape::Triangle, LfoShape::SawUp, LfoShape::SawDown, LfoShape::Square, LfoShape::Random];
                    let i = all.iter().position(|&x| x == l.shape).unwrap_or(0);
                    ModSource::Lfo(Lfo { shape: all[(i + 1) % all.len()], ..l })
                }
                other => other,
            });
            if let Some(src) = next {
                run(app, ModCmd::SetSource(id, src));
                return;
            }
        }
        ModAct::RateMode(id) => {
            let next = s.modulation.get(id).map(|m| match m.source {
                ModSource::Lfo(l) => ModSource::Lfo(Lfo { period: toggle_period(l.period), ..l }),
                ModSource::Random(r) => ModSource::Random(Random { period: toggle_period(r.period), ..r }),
                ModSource::Steps(st) => ModSource::Steps(Steps { period: toggle_period(st.period), ..st }),
                other => other,
            });
            if let Some(src) = next {
                run(app, ModCmd::SetSource(id, src));
                return;
            }
        }
        ModAct::RateStep(id, dir) => {
            let next = s.modulation.get(id).map(|m| {
                let step = |t: Timebase| match t {
                    Timebase::Sync(n) if dir > 0 => Timebase::Sync((n.saturating_mul(2)).min(32)),
                    Timebase::Sync(n) => Timebase::Sync((n / 2).max(1)),
                    Timebase::Free(hz) if dir > 0 => Timebase::Free((hz * 2.0).min(32.0)),
                    Timebase::Free(hz) => Timebase::Free((hz / 2.0).max(0.0625)),
                };
                match m.source {
                    ModSource::Lfo(l) => ModSource::Lfo(Lfo { period: step(l.period), ..l }),
                    ModSource::Random(r) => ModSource::Random(Random { period: step(r.period), ..r }),
                    ModSource::Steps(st) => ModSource::Steps(Steps { period: step(st.period), ..st }),
                    other => other,
                }
            });
            if let Some(src) = next {
                run(app, ModCmd::SetSource(id, src));
                return;
            }
        }
        ModAct::ToggleBipolar(id) => {
            let next = s.modulation.get(id).map(|m| match m.source {
                ModSource::Lfo(l) => ModSource::Lfo(Lfo { bipolar: !l.bipolar, ..l }),
                ModSource::Random(r) => ModSource::Random(Random { bipolar: !r.bipolar, ..r }),
                other => other,
            });
            if let Some(src) = next {
                run(app, ModCmd::SetSource(id, src));
                return;
            }
        }
        ModAct::ToggleDistribution(id) => {
            let next = s.modulation.get(id).map(|m| match m.source {
                ModSource::Random(r) => ModSource::Random(Random {
                    distribution: if r.distribution == Distribution::Uniform { Distribution::Gaussian } else { Distribution::Uniform },
                    ..r
                }),
                other => other,
            });
            if let Some(src) = next {
                run(app, ModCmd::SetSource(id, src));
                return;
            }
        }
        ModAct::ToggleInterpolate(id) => {
            let next = s.modulation.get(id).map(|m| match m.source {
                ModSource::Random(r) => ModSource::Random(Random { interpolate: !r.interpolate, ..r }),
                other => other,
            });
            if let Some(src) = next {
                run(app, ModCmd::SetSource(id, src));
                return;
            }
        }
        ModAct::CycleBand(id) => {
            let next = s.modulation.get(id).map(|m| match m.source {
                ModSource::AudioFollower(f) => ModSource::AudioFollower(AudioFollower {
                    band: match f.band {
                        Band::Full => Band::Low,
                        Band::Low => Band::Mid,
                        Band::Mid => Band::High,
                        Band::High | Band::Custom { .. } => Band::Full,
                    },
                    ..f
                }),
                other => other,
            });
            if let Some(src) = next {
                run(app, ModCmd::SetSource(id, src));
                return;
            }
        }
        ModAct::CycleDetection(id) => {
            let next = s.modulation.get(id).map(|m| match m.source {
                ModSource::AudioFollower(f) => ModSource::AudioFollower(AudioFollower {
                    detection: if f.detection == Detection::Rms { Detection::Peak } else { Detection::Rms },
                    ..f
                }),
                other => other,
            });
            if let Some(src) = next {
                run(app, ModCmd::SetSource(id, src));
                return;
            }
        }
        ModAct::Kind(id, kind) => {
            run(app, ModCmd::SetKind(id, kind));
            return;
        }
        ModAct::Knob(id, field) => {
            let Some(src) = s.modulation.get(id).map(|m| m.source) else { return };
            let v = knob_value(src, field);
            app.drag = Some(crate::app::Drag::ModKnob { id, field, y0: app.mouse.1, v0: v, src0: src, src_now: src });
            return;
        }
        ModAct::DepthDrag(id, target) => {
            begin_routing_drag(app, id, target);
            return;
        }
        ModAct::DepthDragPath(id, path) => {
            if let Some((target, _)) = resolve_path(s, &app.plugins, path) {
                begin_routing_drag(app, id, target);
            }
            return;
        }
        ModAct::Focus(id, path) => {
            // Bring the target's owner into view: the rack follows the track, and the modulator's editor opens.
            if let Some(loc) = s.identity.owners.get(&path.owner).copied() {
                if let OwnerLocator::Track(t) | OwnerLocator::Slot(t, _) | OwnerLocator::Member(t, _, _) = loc {
                    s.playlist.track = t;
                }
            }
            select_one(s, id);
            s.mod_editor = Some(id);
        }
        ModAct::TargetEnabled(id, path, on) => {
            run(app, ModCmd::SetTargetEnabled(id, path, on));
            return;
        }
        ModAct::RemovePath(id, path) => {
            run(app, ModCmd::UnassignPath(id, path));
            return;
        }
        ModAct::DepthPath(id, path, depth) => {
            run(app, ModCmd::SetDepthPath(id, path, depth));
            return;
        }
    }
    app.invalidate();
}

/// The modulation graph as the engine runs it (`os_engine::Engine::set_modulation`): each modulator's source and its
/// targets resolved to the concrete legacy project [`os_model::target::Target`] the engine addresses. An assignment that does
/// not resolve is left out here (it stays in the graph, and [`unresolved`] reports it), so a plugin that comes back
/// brings its modulation back with it.
///
/// The engine drives plugin parameters from this today: a channel's instrument and a mixer slot's hosted effect. A
/// host parameter (a track's volume, pan, EQ, a send) decodes to another `Target` and waits for the mixer path.
pub fn engine_spec(s: &State, plugins: &Plugins) -> (Vec<os_engine::Modulator>, std::sync::Arc<os_engine::ModSnapshot>) {
    let mut ids = Vec::new();
    let mut targets: Vec<os_model::target::Target> = Vec::new();
    let modulators: Vec<os_engine::Modulator> = s
        .modulation
        .modulators
        .iter()
        .map(|m| {
            ids.push(m.id.0);
            let resolved: Vec<os_engine::ModTarget> = m
                .targets
                .iter()
                .filter_map(|t| {
                    let (target, _) = resolve_path(s, plugins, t.path)?;
                    let target = target::decode(target);
                    // A route into the graph is an edge, not a project parameter: the engine applies it while it evaluates
                    // and never publishes a value for it, so the snapshot must not offer one (a cursor pinned at zero).
                    if !matches!(target, target::Target::ModulatorParam(..)) && !targets.contains(&target) {
                        targets.push(target);
                    }
                    Some(os_engine::ModTarget { target, depth: t.depth, curve: t.curve, polarity: t.polarity, enabled: t.enabled })
                })
                .collect();
            os_engine::Modulator { id: m.id.0, source: m.source, enabled: m.enabled, targets: resolved }
        })
        .collect();
    (modulators, std::sync::Arc::new(os_engine::ModSnapshot::new(&ids, &targets)))
}

/// Every assignment in the modulation graph whose [`ParameterPath`] does not resolve right now, as (modulator,
/// parameter), in graph order: a native id the plugin no longer exposes, or an owner the project no longer has.
/// The UI lists these as missing rather than dropping them — a plugin that comes back brings its assignments back.
pub fn unresolved(s: &State, plugins: &Plugins) -> Vec<(ModulatorId, ParameterPath)> {
    let mut v = Vec::new();
    for m in &s.modulation.modulators {
        for t in &m.targets {
            if resolve_path(s, plugins, t.path).is_none() {
                v.push((m.id, t.path));
            }
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use os_model::modulation::{ParameterKey, ParamResolution};
    use os_model::target;

    #[test]
    fn legacy_plugin_index_is_positional_and_permanent() {
        // A legacy target stays a positional plugin parameter; the resolver never upgrades it.
        let (t, res) = target_for(OwnerLocator::Channel(3), ParameterKey::LegacyPluginIndex(2), None).unwrap();
        assert_eq!(t, target::channel_param(3, 2));
        assert_eq!(res, ParamResolution::Unique(2));
    }

    #[test]
    fn host_parameter_code_is_reused_unchanged() {
        // Host parameters reuse the existing lower-16-bit code; no second host-control scheme is invented.
        let (t, res) = target_for(OwnerLocator::Track(5), ParameterKey::HostParameterCode(0x1F00 | 3), None).unwrap();
        assert_eq!(t, target::mixer(5, 3));
        assert_eq!(res, ParamResolution::Unique(0));
    }

    #[test]
    fn native_id_resolves_to_positional_index_when_unique() {
        let (t, res) = target_for(OwnerLocator::Slot(2, 0), ParameterKey::PluginNativeId(100), Some(&[10, 100, 30])).unwrap();
        assert_eq!(t, target::slot_param(2, 0, 1));
        assert_eq!(res, ParamResolution::Unique(1));
    }

    #[test]
    fn native_id_missing_is_none_not_a_legacy_fallback() {
        // A native id with no plugin metadata is unresolved; it must NOT silently become a legacy index.
        assert!(target_for(OwnerLocator::Slot(2, 0), ParameterKey::PluginNativeId(99), None).is_none());
        assert!(target_for(OwnerLocator::Slot(2, 0), ParameterKey::PluginNativeId(99), Some(&[10, 20])).is_none());
    }

    #[test]
    fn native_id_duplicate_keeps_first_index_and_reports_ambiguity() {
        let (t, res) = target_for(OwnerLocator::Slot(2, 0), ParameterKey::PluginNativeId(7), Some(&[7, 7])).unwrap();
        assert_eq!(t, target::slot_param(2, 0, 0));
        assert_eq!(res, ParamResolution::Duplicate(0));
    }

    #[test]
    fn native_id_resolves_inside_a_container_member() {
        let (t, res) = target_for(OwnerLocator::Member(2, 0, 1), ParameterKey::PluginNativeId(50), Some(&[50, 60])).unwrap();
        assert_eq!(t, target::slot_param(2, 0, target::member_item(1, 0).unwrap()));
        assert_eq!(res, ParamResolution::Unique(0));
    }

    /// The options menu is how a route into the graph is made by hand: with routing mode armed on one modulator, another
    /// modulator's menu lists the parameters the armed one can drive into it, and a row creates or removes that route.
    #[test]
    fn the_options_menu_routes_one_modulator_into_another() {
        use os_model::modulation::ModParam;
        let mut app = crate::app::App::new(None);
        let a = app.state.modulation.add(ModSource::default());
        let b = app.state.modulation.add(ModSource::default());
        // Nothing is offered until routing mode is armed, and a modulator never offers itself.
        let plain = menu_items(&app.state, app.state.modulation.get(b).unwrap());
        assert!(!plain.iter().any(|(_, act)| matches!(act, ModAct::DriveParam(..))));
        crate::modulation::act(&mut app, ModAct::Route(a));
        assert!(!menu_items(&app.state, app.state.modulation.get(a).unwrap())
            .iter()
            .any(|(_, act)| matches!(act, ModAct::DriveParam(..))), "a modulator is not its own route");
        // An LFO has all four of them, and the rows name the driver.
        let rows: Vec<(String, ModAct)> = menu_items(&app.state, app.state.modulation.get(b).unwrap());
        let offered: Vec<ModParam> = rows
            .iter()
            .filter_map(|(_, act)| match act {
                ModAct::DriveParam(_, _, p) => Some(*p),
                _ => None,
            })
            .collect();
        assert_eq!(offered, ModParam::ALL.to_vec(), "every parameter an LFO has");
        assert!(rows.iter().any(|(label, _)| label.starts_with("LFO 1")), "the rows name the driver: {rows:?}");
        // A row routes the armed modulator into that parameter of this one.
        crate::modulation::act(&mut app, ModAct::DriveParam(b, a, ModParam::Rate));
        let target = app.state.modulation.get(a).and_then(|m| m.targets.first()).copied().expect("the route");
        assert_eq!(target.path.modulator(), Some((b, ModParam::Rate)));
        assert_eq!(target.depth, os_model::modulation::ModTarget::DEPTH_DEFAULT, "a new route is audible");
        // The menu now says so, and the same row takes the route away again.
        let rows = menu_items(&app.state, app.state.modulation.get(b).unwrap());
        assert!(
            rows.iter().any(|(label, act)| *act == ModAct::DriveParam(b, a, ModParam::Rate) && label.starts_with("Stop ")),
            "the row switches the route off: {rows:?}"
        );
        crate::modulation::act(&mut app, ModAct::DriveParam(b, a, ModParam::Rate));
        assert!(app.state.modulation.get(a).unwrap().targets.is_empty(), "routed again: gone");
    }

    /// Only the parameters the destination actually has are offered, and a route that would close a loop is not. The
    /// route into the graph is drawn the way a device routing is: Ctrl+click the destination's tile to open its panel
    /// without leaving routing mode, then press the knob the route should land on.
    #[test]
    fn routing_mode_offers_only_what_can_be_routed_and_keeps_itself_armed() {
        use os_model::modulation::{ModParam, ModTarget};
        let mut app = crate::app::App::new(None);
        let a = app.state.modulation.add(ModSource::default());
        let steps = app.state.modulation.add(ModSource::Steps(os_model::modulation::Steps::default()));
        crate::modulation::act(&mut app, ModAct::Route(a));
        let offered = |app: &crate::app::App, id| -> Vec<ModParam> {
            menu_items(&app.state, app.state.modulation.get(id).unwrap())
                .into_iter()
                .filter_map(|(_, act)| match act {
                    ModAct::DriveParam(_, _, p) => Some(p),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(offered(&app, steps), vec![ModParam::Rate], "a step sequence has only a rate");
        // Ctrl+click on the other modulator's tile opens its panel and keeps routing mode; a plain click is an ordinary
        // open and leaves it.
        app.ctrl = true;
        crate::modulation::act(&mut app, ModAct::Open(steps));
        assert_eq!(app.state.mod_editor, Some(steps), "its panel opened");
        assert_eq!(app.state.mod_menu, None, "the panel, not the menu");
        assert_eq!(app.state.mod_routing, Some(a), "routing mode is still armed");
        // A press on one of its knobs is the route's destination: the armed modulator starts driving that parameter at the
        // default depth, and the pointer drags that depth (the knob's own value is not touched).
        crate::modulation::act(&mut app, ModAct::Knob(steps, KnobField::Rate));
        let route = app
            .state
            .modulation
            .get(a)
            .and_then(|m| m.targets.iter().find(|t| t.path.modulator() == Some((steps, ModParam::Rate))).copied())
            .expect("the press routed a's output into the steps' rate");
        assert_eq!(route.depth, ModTarget::DEPTH_DEFAULT, "a new route is audible");
        assert!(matches!(app.drag, Some(crate::app::Drag::ModDepth { id, .. }) if id == a), "and the drag is its depth");
        app.drag = None;
        app.ctrl = false;
        crate::modulation::act(&mut app, ModAct::Open(a));
        assert_eq!(app.state.mod_routing, None, "a plain click leaves routing mode");
        // The route now runs a → steps, so the steps driving a's rate would close a loop: the graph refuses it, its menu
        // does not offer it, and the press on a's knob edits the knob instead of drawing a route.
        crate::modulation::act(&mut app, ModAct::Route(steps));
        assert!(offered(&app, a).is_empty(), "the route back is not offered");
        app.ctrl = true;
        crate::modulation::act(&mut app, ModAct::Open(a));
        crate::modulation::act(&mut app, ModAct::Knob(a, KnobField::Rate));
        assert!(app.state.modulation.get(steps).unwrap().targets.is_empty(), "no loop was drawn");
        assert!(matches!(app.drag, Some(crate::app::Drag::ModKnob { .. })), "the knob edited itself instead");
    }

    /// A route into the graph resolves from the graph itself (no device identity) and reaches the engine as the
    /// graph's own edge, so the engine can apply it while it evaluates.
    #[test]
    fn a_route_reaches_the_engine_as_the_graphs_own_edge() {
        let mut app = crate::app::App::new(None);
        let a = app.state.modulation.add(ModSource::default());
        let b = app.state.modulation.add(ModSource::default());
        assert!(app.state.modulation.route(a, b, os_model::modulation::ModParam::Amount, 0.5));
        let path = app.state.modulation.get(a).and_then(|m| m.targets.first()).map(|t| t.path).expect("the route");
        let (t, _) = resolve_path(&app.state, &app.plugins, path).expect("a route needs no device");
        assert_eq!(
            target::decode(t),
            target::Target::ModulatorParam(b.0, os_model::modulation::ModParam::Amount.code())
        );
        assert!(!unresolved(&app.state, &app.plugins).iter().any(|(_, p)| *p == path), "and it is not missing");
        let label = path_label(&app.state, path);
        assert!(label.ends_with(" - Amount"), "the row names the route: {label}");
        let (spec, _) = engine_spec(&app.state, &app.plugins);
        let driver = spec.iter().find(|m| m.id == a.0).expect("the driver is in the spec");
        assert!(
            driver.targets.iter().any(|t| matches!(t.target, target::Target::ModulatorParam(..))),
            "the engine gets the edge: {:?}",
            driver.targets
        );
    }

    /// A stopped project with a modulator repaints the modulator strip from `tick` alone: its plot is a live sample,
    /// so without this it froze until a knob drag happened to repaint the strip.
    #[test]
    fn a_stopped_modulator_strip_repaints_itself() {
        let mut app = crate::app::App::new(None);
        app.state.project.linear_mode = true;
        app.state.show_device_rack = true;
        app.state.device_rack_collapsed = false;
        app.state.modulation.add(ModSource::default());
        let _ = app.take_damage();
        app.tick();
        let damage = app.take_damage().expect("the strip asked for a frame");
        let s = &app.state;
        let h = crate::device_rack::height(s);
        let rack = os_ui::Rect::new(0.0, s.screen.1 - h, s.screen.0, h);
        let row = crate::views::device_rack::mod_row_rect(s, rack);
        if let Some(rects) = damage {
            assert!(rects.iter().any(|r| !r.intersect(&row).is_empty()), "the modulator strip is in the damage: {rects:?}");
        }
    }

    /// The graph reaches the engine with its sources and the assignments that resolve, and without the ones that do
    /// not (a native id and no plugin metadata); depths and curves travel unchanged.
    #[test]
    fn the_engine_spec_carries_the_graphs_targets() {
        let mut app = crate::app::App::new(None);
        app.state.project = crate::app::test_project();
        app.state.control.settle(&mut app.state.project);
        let owner = app.state.ensure_owner(OwnerLocator::Channel(0));
        let id = app.state.modulation.add(os_model::modulation::ModSource::default());
        let voice = ParameterPath { owner, parameter: ParameterKey::HostParameterCode(target::CHANNEL_VOLUME) };
        let native = ParameterPath { owner, parameter: ParameterKey::PluginNativeId(5) };
        assert!(app.state.modulation.assign(id, os_model::modulation::ModTarget::new(voice).with_depth(-0.75)));
        assert!(app.state.modulation.assign(id, os_model::modulation::ModTarget::new(native)));
        let (spec, _) = engine_spec(&app.state, &app.plugins);
        assert_eq!(spec.len(), 1);
        assert_eq!(spec[0].id, id.0);
        assert_eq!(spec[0].targets.len(), 1, "the native id has no metadata to resolve against");
        assert_eq!(spec[0].targets[0].target, target::decode(target::channel(0, target::CHANNEL_VOLUME)));
        assert_eq!(spec[0].targets[0].depth, -0.75);
        assert_eq!(spec[0].targets[0].polarity, os_model::modulation::Polarity::Bipolar);
    }

    /// A knob's target reads back to the owner and parameter that wrote it, and `target_for` writes the same target
    /// again: the inverse the Modulation menu needs, for a channel, a track's host parameter and a slot's plugin.
    #[test]
    fn a_target_reads_back_to_its_parameter() {
        let cases = [
            (target::channel_param(3, 2), (OwnerLocator::Channel(3), ParameterKey::LegacyPluginIndex(2))),
            (target::mixer(5, 3), (OwnerLocator::Track(5), ParameterKey::HostParameterCode(0x1F00 | 3))),
            (target::slot_param(2, 1, 4), (OwnerLocator::Slot(2, 1), ParameterKey::LegacyPluginIndex(4))),
        ];
        for (t, want) in cases {
            assert_eq!(path_of(t), Some(want), "target {t:#x}");
            let (owner, key) = want;
            assert_eq!(target_for(owner, key, None).map(|(t, _)| t), Some(t), "and back: {want:?}");
        }
    }

    /// A knob's Modulation menu makes a modulator that drives it, the engine gets the assignment, and each step is one
    /// undo away: assigning, unassigning and removing.
    #[test]
    fn a_new_modulator_from_a_knob_is_an_undo_step() {
        let mut app = crate::app::App::new(None);
        app.state.project = crate::app::test_project();
        app.state.control.settle(&mut app.state.project);
        let t = target::channel_param(0, 1);
        let msg = run(&mut app, ModCmd::New(t)).expect("made a modulator");
        assert!(msg.contains("drives"), "{msg}");
        assert_eq!(app.state.modulation.modulators.len(), 1);
        let (id, path) = {
            let m = &app.state.modulation.modulators[0];
            (m.id, m.targets.first().expect("one assignment").path)
        };
        assert_eq!(app.state.identity.owners.get(&path.owner), Some(&OwnerLocator::Channel(0)));
        let heard = engine_spec(&app.state, &app.plugins).0.first().and_then(|m| m.targets.first()).map(|t| t.target);
        assert_eq!(heard, Some(target::decode(t)), "the assignment is the knob's own parameter");
        // Taking the assignment away leaves the modulator, driving nothing.
        run(&mut app, ModCmd::Unassign(id, t)).expect("unassigned");
        assert_eq!(app.state.modulation.get(id).map(|m| m.targets.len()), Some(0));
        assert!(engine_spec(&app.state, &app.plugins).0.first().is_some_and(|m| m.targets.is_empty()));
        // Removing it and undoing that puts it back.
        run(&mut app, ModCmd::Remove(id)).expect("removed");
        assert!(app.state.modulation.is_empty());
        crate::playlist::run(&mut app, crate::playlist::Cmd::Undo);
        assert_eq!(app.state.modulation.get(id).map(|m| m.targets.len()), Some(0), "the undo brings the modulator back");
    }

    /// Switching a modulator's kind keeps the rate it had (an LFO and a step pattern are both timed), and a pattern
    /// that comes in is one that is heard rather than sixteen zeros.
    #[test]
    fn changing_kind_keeps_the_rate() {
        let lfo = ModSource::Lfo(Lfo { shape: LfoShape::Triangle, period: Timebase::Sync(8), phase: 0.5, ..Lfo::default() });
        let steps = set_kind(lfo, Kind::Steps);
        assert!(matches!(steps, ModSource::Steps(Steps { period: Timebase::Sync(8), smooth: false, .. })), "{steps:?}");
        assert!(matches!(steps, ModSource::Steps(Steps { values, .. }) if values.iter().any(|&v| v != 0)), "a laid pattern is audible");
        assert_eq!(set_kind(steps, Kind::Lfo), ModSource::Lfo(Lfo { shape: LfoShape::Sine, period: Timebase::Sync(8), phase: 0.0, ..Lfo::default() }));
        assert_eq!(Kind::of(set_kind(steps, Kind::Envelope)), Kind::Envelope);
        assert_eq!(set_kind(steps, Kind::Steps), steps, "asking for the kind it already is changes nothing");
    }

    /// The editor's items edit the modulator and reach the engine at once: its source, this control's depth and
    /// polarity, and its on / off switch. Every item is live, and each edit is one undo step.
    #[test]
    fn a_modulators_editor_edits_it_and_undoes() {
        let mut app = crate::app::App::new(None);
        app.state.project = crate::app::test_project();
        app.state.control.settle(&mut app.state.project);
        let t = target::channel_param(0, 1);
        run(&mut app, ModCmd::New(t)).expect("a modulator");
        let id = app.state.modulation.modulators[0].id;
        let menu = editor_menu(&app.state, id, t).expect("an editor");
        assert!(menu.children.iter().all(|i| i.head || i.sep || i.enabled), "every entry works");
        let labels: Vec<&str> = menu.children.iter().map(|i| i.label.as_ref()).collect();
        assert!(labels.contains(&"LFO") && labels.contains(&"Steps") && labels.contains(&"1/4"), "{labels:?}");
        assert!(labels.contains(&"+50%") && labels.contains(&"Unipolar") && labels.contains(&"Sine"), "{labels:?}");
        assert!(run(&mut app, ModCmd::SetDepth(id, t, 0.5)).is_some());
        assert!(run(&mut app, ModCmd::SetPolarity(id, t, Polarity::Unipolar)).is_some());
        assert!(run(&mut app, ModCmd::SetKind(id, Kind::Steps)).is_some());
        let (spec, _) = engine_spec(&app.state, &app.plugins);
        assert_eq!(spec[0].targets[0].depth, 0.5);
        assert_eq!(spec[0].targets[0].polarity, Polarity::Unipolar);
        assert!(matches!(spec[0].source, ModSource::Steps(_)));
        // The depth ring reads the enabled modulators alone.
        assert_eq!(depth_on(&app.state, t), Some(0.5));
        run(&mut app, ModCmd::SetEnabled(id, false)).expect("off");
        assert_eq!(depth_on(&app.state, t), None);
        assert!(!engine_spec(&app.state, &app.plugins).0[0].enabled);
        // Back one step: the modulator is on again, still at half depth.
        crate::playlist::run(&mut app, crate::playlist::Cmd::Undo);
        assert!(app.state.modulation.get(id).is_some_and(|m| m.enabled));
        assert_eq!(depth_on(&app.state, t), Some(0.5));
    }

    /// While the transport is stopped the tile's plot still moves: the history falls back to evaluating the source
    /// on its own preview clock instead of reading the engine's idle snapshot.
    #[test]
    fn the_tile_history_previews_a_source_while_stopped() {
        let mut app = crate::app::App::new(None);
        app.state.project = crate::app::test_project();
        app.state.control.settle(&mut app.state.project);
        let id = app.state.modulation.add(ModSource::default());
        let mut h = History::default();
        h.sample(&app.state, false, 120.0);
        let first = *h.get(id).and_then(|q| q.back()).unwrap();
        // Step the 33 ms sampler past its gate without sleeping.
        h.at = Some(Instant::now() - std::time::Duration::from_millis(200));
        h.sample(&app.state, false, 120.0);
        let second = *h.get(id).and_then(|q| q.back()).unwrap();
        assert_ne!(first, second, "the preview plot moves while stopped");
    }

    /// The clicked modulator copies with its settings and routings, pastes into the clicked empty slot with a fresh
    /// id, and Delete removes the selected copy — each through the modulator window's keys.
    #[test]
    fn a_modulator_copies_pastes_into_an_empty_slot_and_deletes() {
        use winit::keyboard::{Key, NamedKey};
        let mut app = crate::app::App::new(None);
        app.state.project = crate::app::test_project();
        app.state.control.settle(&mut app.state.project);
        let path = ParameterPath { owner: PersistentObjectId(7), parameter: ParameterKey::HostParameterCode(0x1F00) };
        let id = app.state.modulation.add(ModSource::Lfo(Lfo { shape: LfoShape::Triangle, ..Lfo::default() }));
        app.state.modulation.assign(id, os_model::modulation::ModTarget::new(path));
        if let Some(m) = app.state.modulation.get_mut(id) {
            m.name = "Wobble".into();
            m.enabled = false;
        }
        app.state.mod_editor = Some(id);
        app.state.keyboard_focus = crate::app::KeyboardFocus::Modulators;
        app.key(&Key::Character("c".into()), true, false);
        assert_eq!(app.state.mod_clipboard.first().map(|m| m.name.as_str()), Some("Wobble"), "Ctrl+C copies the selected modulator");
        // Clicking empty slot 4 selects it for the paste.
        app.state.mod_add_slot = Some(3);
        app.key(&Key::Character("v".into()), true, false);
        let pasted = app.state.modulation.modulator_at_slot(3).expect("slot 4 holds the copy");
        let pasted_id = pasted.id;
        assert_ne!(pasted_id, id, "the copy has its own id");
        assert_eq!(pasted.name, "Wobble");
        assert!(!pasted.enabled, "its enabled state came too");
        assert_eq!(pasted.targets, app.state.modulation.get(id).unwrap().targets, "and its routings");
        assert_eq!(app.state.mod_editor, Some(pasted_id), "the pasted modulator's editor opens");
        assert_eq!(app.state.mod_add_slot, None);
        app.key(&Key::Named(NamedKey::Delete), false, false);
        assert_eq!(app.state.modulation.get(pasted_id), None, "Delete removes the selected copy");
        assert!(app.state.modulation.get(pasted_id).is_none());
        assert_eq!(app.state.mod_editor, None);
        assert!(app.state.modulation.get(id).is_some(), "the original stays");
    }

    /// Clicking a modulator selects it independently of its editor window: clicking the open slot again closes the
    /// window but leaves it selected, so Delete still removes it.
    #[test]
    fn a_selected_modulator_deletes_after_its_editor_toggles_closed() {
        use winit::keyboard::{Key, NamedKey};
        let mut app = crate::app::App::new(None);
        app.state.project = crate::app::test_project();
        app.state.control.settle(&mut app.state.project);
        let id = app.state.modulation.add(ModSource::default());
        let slot = app.state.modulation.get(id).unwrap().slot;
        app.act(crate::app::Action::Mod(ModAct::Open(id)));
        assert_eq!((app.state.mod_sel.clone(), app.state.mod_editor), (vec![slot], Some(id)));
        app.act(crate::app::Action::Mod(ModAct::Open(id)));
        assert_eq!(app.state.mod_editor, None, "the second click closes the window");
        assert_eq!(app.state.mod_sel, vec![slot], "but the slot stays selected");
        app.key(&Key::Named(NamedKey::Delete), false, false);
        assert!(app.state.modulation.get(id).is_none(), "Delete removes the selected modulator");
        assert!(app.state.mod_sel.is_empty() && app.state.mod_editor.is_none());
    }

    /// Shift+arrows grow the selection across slots, Ctrl+C copies every selected modulator, Ctrl+V pastes them into
    /// the empty slots, and Delete removes the whole selection as one undo step.
    #[test]
    fn shift_arrows_select_multiple_modulators_and_copy_paste_delete_them() {
        use winit::keyboard::{Key, NamedKey};
        let mut app = crate::app::App::new(None);
        app.state.project = crate::app::test_project();
        app.state.control.settle(&mut app.state.project);
        let ids: Vec<_> = (0..3).map(|_| app.state.modulation.add(ModSource::default())).collect();
        select_one(&mut app.state, ids[0]);
        app.shift = true;
        app.key(&Key::Named(NamedKey::ArrowRight), false, false);
        assert_eq!(app.state.mod_sel, vec![0, 1], "Shift+Right adds the next modulator");
        app.key(&Key::Named(NamedKey::ArrowRight), false, false);
        assert_eq!(app.state.mod_sel, vec![0, 1, 2]);
        app.shift = false;
        // Copy the three, click empty slot 4, and paste all three into slots 4, 5 and 6.
        app.state.mod_add_slot = Some(3);
        app.key(&Key::Character("c".into()), true, false);
        assert_eq!(app.state.mod_clipboard.len(), 3);
        app.key(&Key::Character("v".into()), true, false);
        assert_eq!(app.state.mod_sel, vec![3, 4, 5]);
        assert!((3..=5).all(|slot| app.state.modulation.modulator_at_slot(slot).is_some()));
        app.key(&Key::Named(NamedKey::Delete), false, false);
        assert!((3..=5).all(|slot| app.state.modulation.modulator_at_slot(slot).is_none()), "Delete removes all pasted copies");
        crate::playlist::run(&mut app, crate::playlist::Cmd::Undo);
        assert!(
            (3..=5).all(|slot| app.state.modulation.modulator_at_slot(slot).is_some()),
            "one undo restores every modulator in the multi-delete"
        );
        assert!(ids.iter().all(|id| app.state.modulation.get(*id).is_some()), "the originals stay");
    }

    /// Shift-clicking a second slot adds it to the selection without closing the first modulator's editor.
    #[test]
    fn shift_clicking_modulators_builds_the_selection() {
        let Ok(mut c) = os_ui::Canvas::new(1920, 1080, 1.0) else { return };
        let mut app = crate::app::App::new(None);
        app.state.project = crate::app::test_project();
        app.state.project.linear_mode = true;
        app.state.control.settle(&mut app.state.project);
        let ids: Vec<_> = (0..2).map(|_| app.state.modulation.add(ModSource::default())).collect();
        let at = |app: &crate::app::App, id| {
            app.hits
                .iter()
                .rev()
                .find(|h| h.action == crate::app::Action::Mod(ModAct::Open(id)))
                .map(|h| h.rect)
                .expect("a modulator slot")
        };
        app.paint(&mut c, None);
        let first = at(&app, ids[0]);
        app.mouse_move(first.cx(), first.cy());
        app.mouse_down();
        assert_eq!(app.state.mod_sel, vec![0]);
        let second = at(&app, ids[1]);
        app.shift = true;
        app.mouse_move(second.cx(), second.cy());
        app.mouse_down();
        app.shift = false;
        assert_eq!(app.state.mod_sel, vec![0, 1], "Shift-click adds the second modulator");
        assert_eq!(app.state.mod_editor, Some(ids[0]), "the editor stays on the first");
    }

    /// The editor's rate knob lifts an LFO off its 1 Hz default and the mode button moves between free and synced
    /// rates without losing the chosen rate.
    #[test]
    fn the_rate_knob_moves_up_from_one_hertz() {
        let lfo = ModSource::Lfo(Lfo::default());
        let pos = knob_value(lfo, KnobField::Rate);
        assert!((pos - 0.444).abs() < 0.01, "1 Hz on the free scale: {pos}");
        let faster = with_knob(lfo, KnobField::Rate, (pos + 0.1).min(1.0));
        let ModSource::Lfo(faster_lfo) = faster else { unreachable!() };
        assert!(matches!(faster_lfo.period, Timebase::Free(hz) if hz > 1.0), "{:?}", faster_lfo.period);
        let synced = toggle_period(faster_lfo.period);
        assert!(matches!(synced, Timebase::Sync(4)), "free switches to a beat");
        assert!(matches!(toggle_period(synced), Timebase::Free(hz) if (hz - 1.0).abs() < 0.01));
    }
}
