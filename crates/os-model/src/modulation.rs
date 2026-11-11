//! Persistent parameter identity for the DAW-wide modulation system (M0).
//!
//! This module defines the project-persistent identity types that the modulation graph (later
//! milestones) will store. It is intentionally separate from [`crate::target`]: the existing
//! `Target` encoding and the FL Studio `.flp` `AUTO_LINK` record are frozen (see the project's
//! modulation audit, K.1). Everything here is persisted in Lemon Studio's own `.lemon` extension
//! (K.3), never inside FLP target bytes.
//!
//! Two identities exist:
//! - a *device/owner* identity ([`PersistentObjectId`]), stable across save/load and device moves;
//! - a *parameter* identity ([`ParameterKey`]), which reuses the FLP-compatible parameter code
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
    /// A legacy positional target (FLP-compatible `0x8000 + index`). Permanent: a legacy target is
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
}
