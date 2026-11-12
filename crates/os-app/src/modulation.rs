//! Persistent parameter identity for the DAW-wide modulation system (M0, audit K.2/K.3).
//!
//! The [`State::identity`] registry maps a stable [`PersistentObjectId`] to each device/owner's
//! *current* [`OwnerLocator`]. The id is persisted in the `.lemon` extension and survives save/load
//! and device moves; the locator is what gets updated when a device moves. The FLP `Target` encoding
//! is never consulted or modified here (K.1).
//!
//! [`target_for`] is a pure mapping from an [`OwnerLocator`] + [`ParameterKey`] to a concrete FLP
//! `Target` (and a [`ParamResolution`]); [`resolve_path`] is the live wrapper that supplies the
//! plugin's current native-id list from [`Plugins`].

use crate::app::State;
use crate::plugins::Plugins;
use os_model::modulation::{Modulator, ModulatorId, OwnerLocator, ParameterKey, ParameterPath, ParamResolution, PersistentObjectId};
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

/// Pure mapping from an owner locator + parameter key to a concrete FLP `Target`, plus how the native id
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
    let owner = s.identity.owners.get(&path.owner).copied()?;
    let native_ids = hosted_native_ids(plugins, owner);
    target_for(owner, path.parameter, native_ids.as_deref())
}

/// The owner and parameter a FLP `Target` names, read back from its encoding ([`target_for`] is the other way). This is
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

/// `target` as a [`ParameterPath`] against the current registry: `None` when the owner has no id yet.
pub fn path_in(s: &State, target: u32) -> Option<ParameterPath> {
    let (owner, parameter) = path_of(target)?;
    Some(ParameterPath { owner: s.owner_of(owner)?, parameter })
}

/// `target` as a [`ParameterPath`], giving its owner an identity if it has none: what an assignment stores.
pub fn target_path(s: &mut State, target: u32) -> Option<ParameterPath> {
    let (owner, parameter) = path_of(target)?;
    Some(ParameterPath { owner: s.ensure_owner(owner), parameter })
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
    };
    format!("{kind} {}", m.id.0)
}

/// What a knob's Modulation menu asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModCmd {
    /// A new LFO driving the parameter `Target` names.
    New(u32),
    /// Modulator `.0` starts driving that parameter.
    Assign(ModulatorId, u32),
    /// Modulator `.0` stops driving it.
    Unassign(ModulatorId, u32),
    /// A modulator goes, with every assignment it had.
    Remove(ModulatorId),
}

/// Runs a [`ModCmd`] as one undo step (the graph and the identities it names are part of the project's undo), and hands
/// the graph to playback at once so the change is heard without touching anything else.
pub fn run(app: &mut crate::app::App, cmd: ModCmd) -> Option<String> {
    let msg = crate::control::edit_app(app, "Modulation", crate::control::command::Region::Project, |app| {
        let s = &mut app.state;
        let path = match cmd {
            ModCmd::New(t) | ModCmd::Assign(_, t) | ModCmd::Unassign(_, t) => target_path(s, t),
            ModCmd::Remove(_) => None,
        };
        let msg = match cmd {
            ModCmd::New(_) => path.map(|path| {
                let id = s.modulation.add(os_model::modulation::ModSource::default());
                s.modulation.assign(id, os_model::modulation::ModTarget::new(path));
                let m = s.modulation.get(id).expect("just added");
                format!("{} now drives this control", name_of(m))
            }),
            ModCmd::Assign(id, _) => path.map(|path| {
                s.modulation.assign(id, os_model::modulation::ModTarget::new(path));
                let m = s.modulation.get(id).expect("an existing modulator");
                format!("{} now drives this control", name_of(m))
            }),
            ModCmd::Unassign(id, _) => path.and_then(|path| {
                s.modulation.unassign(id, path).then(|| "Stopped driving this control".to_string())
            }),
            ModCmd::Remove(id) => s.modulation.remove(id).map(|m| format!("{} removed", name_of(&m))),
        };
        if msg.is_some() {
            s.piano.dirty = true;
        }
        msg
    });
    if msg.is_some() {
        let spec = engine_spec(&app.state, &app.plugins);
        app.transport.set_modulation(spec);
        app.invalidate();
    }
    msg
}

/// The modulation graph as the engine runs it (`os_engine::Engine::set_modulation`): each modulator's source and its
/// targets resolved to the concrete FLP [`os_model::target::Target`] the engine addresses. An assignment that does
/// not resolve is left out here (it stays in the graph, and [`unresolved`] reports it), so a plugin that comes back
/// brings its modulation back with it.
///
/// The engine drives plugin parameters from this today: a channel's instrument and a mixer slot's hosted effect. A
/// host parameter (a track's volume, pan, EQ, a send) decodes to another `Target` and waits for the mixer path.
pub fn engine_spec(s: &State, plugins: &Plugins) -> Vec<os_engine::Modulator> {
    s.modulation
        .modulators
        .iter()
        .map(|m| os_engine::Modulator {
            id: m.id.0,
            source: m.source,
            enabled: m.enabled,
            targets: m
                .targets
                .iter()
                .filter_map(|t| {
                    let (target, _) = resolve_path(s, plugins, t.path)?;
                    Some(os_engine::ModTarget { target: target::decode(target), depth: t.depth, curve: t.curve, polarity: t.polarity })
                })
                .collect(),
        })
        .collect()
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
        let spec = engine_spec(&app.state, &app.plugins);
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
        let heard = engine_spec(&app.state, &app.plugins).first().and_then(|m| m.targets.first()).map(|t| t.target);
        assert_eq!(heard, Some(target::decode(t)), "the assignment is the knob's own parameter");
        // Taking the assignment away leaves the modulator, driving nothing.
        run(&mut app, ModCmd::Unassign(id, t)).expect("unassigned");
        assert_eq!(app.state.modulation.get(id).map(|m| m.targets.len()), Some(0));
        assert!(engine_spec(&app.state, &app.plugins).first().is_some_and(|m| m.targets.is_empty()));
        // Removing it and undoing that puts it back.
        run(&mut app, ModCmd::Remove(id)).expect("removed");
        assert!(app.state.modulation.is_empty());
        crate::playlist::run(&mut app, crate::playlist::Cmd::Undo);
        assert_eq!(app.state.modulation.get(id).map(|m| m.targets.len()), Some(0), "the undo brings the modulator back");
    }
}
