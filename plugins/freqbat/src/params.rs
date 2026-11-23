// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! FreqBat parameters.

use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use nih_plug::prelude::*;

/// A FreqBat patch (event 213's bytes) as a persisted field: a hex string.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StateBytes(pub Vec<u8>);

impl serde::Serialize for StateBytes {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex(&self.0))
    }
}

impl<'de> serde::Deserialize<'de> for StateBytes {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = <String as serde::Deserialize>::deserialize(d)?;
        from_hex(&text).map(StateBytes).ok_or_else(|| serde::de::Error::custom("not a hex string"))
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn from_hex(t: &str) -> Option<Vec<u8>> {
    let t = t.as_bytes();
    if t.len() % 2 != 0 {
        return None;
    }
    t.chunks(2).map(|c| u8::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok()).collect()
}

/// The persisted field holding the FreqBat state.
pub const STATE_FIELD: &str = "freqbat-state";

/// A plugin state (the host's state blob: its length, then nih-plug's JSON) holding the FreqBat state `fl` as it is.
pub fn plugin_state_from_fl(fl: &[u8]) -> Vec<u8> {
    let field = serde_json::to_string(&StateBytes(fl.to_vec())).unwrap_or_default();
    let json = serde_json::json!({ "version": env!("CARGO_PKG_VERSION"), "params": {}, "fields": { STATE_FIELD: field } }).to_string();
    let mut blob = (json.len() as u64).to_le_bytes().to_vec();
    blob.extend_from_slice(json.as_bytes());
    blob
}

/// The FreqBat state inside a plugin state blob (see [`plugin_state_from_fl`]); `Some(empty)` for an instance that
/// still plays its default patch, `None` when the blob is not a FreqBat state.
pub fn fl_from_plugin_state(blob: &[u8]) -> Option<Vec<u8>> {
    let (_, json) = os_hostlink::state::json(blob);
    let v: serde_json::Value = serde_json::from_str(std::str::from_utf8(json).ok()?).ok()?;
    let field = v.get("fields")?.get(STATE_FIELD)?.as_str()?;
    serde_json::from_str::<StateBytes>(field).ok().map(|s| s.0)
}

#[derive(Params)]
pub struct FreqBatParams {
    /// The FreqBat state in use (event 213's bytes), as loaded; empty for the default patch.
    #[persist = "freqbat-state"]
    pub state: Arc<RwLock<StateBytes>>,
    /// Bumped whenever `state` is replaced (not when the editor edits it).
    pub state_version: AtomicU64,
    /// The editor's latest edit for the audio thread: the patch, and the state version it edits.
    pub edit: Mutex<Option<(u64, dsp::Patch)>>,
    /// Notes the editor's testing keyboard plays itself (key, on).
    pub previews: Mutex<Vec<(u8, bool)>>,
    /// The keys held down, the host's notes and the testing keyboard's (bit `k % 64` of word `k / 64` for key `k`),
    /// as of the audio thread's last block: the editor's keyboard shows them.
    pub playing: [AtomicU64; 2],
    /// Mod X per-note modulation ID.
    #[id = "modx"]
    pub mod_x: FloatParam,
    /// Mod Y per-note modulation ID.
    #[id = "mody"]
    pub mod_y: FloatParam,
}

impl Default for FreqBatParams {
    fn default() -> Self {
        FreqBatParams {
            state: Arc::new(RwLock::new(StateBytes::default())),
            state_version: AtomicU64::new(1),
            edit: Mutex::new(None),
            previews: Mutex::new(Vec::new()),
            playing: [AtomicU64::new(0), AtomicU64::new(0)],
            mod_x: FloatParam::new("Mod X", 0.0, FloatRange::Linear { min: -1.0, max: 1.0 }).with_poly_modulation_id(MOD_X_POLY),
            mod_y: FloatParam::new("Mod Y", 0.0, FloatRange::Linear { min: -1.0, max: 1.0 }).with_poly_modulation_id(MOD_Y_POLY),
        }
    }
}

impl FreqBatParams {
    /// Replaces the state (a preset or a project's channel); the audio thread picks it up at its next block.
    pub fn set_state(&self, bytes: Vec<u8>) {
        // The version moves while the state is locked, so an edit of the state it replaces cannot land after it.
        let s = self.state.write();
        if let Ok(mut s) = s {
            *s = StateBytes(bytes);
        }
        self.state_version.fetch_add(1, Ordering::AcqRel);
    }

    /// An edit's state without its patch yet (during a drag the editor makes the patch once a repaint): `bytes`
    /// become the state the host saves. False, and nothing changes, when the state was replaced since version `version` was read.
    pub fn stored(&self, version: u64, bytes: Vec<u8>) -> bool {
        let Ok(mut s) = self.state.write() else { return false };
        if self.state_version.load(Ordering::Acquire) != version {
            return false;
        }
        *s = StateBytes(bytes);
        true
    }

    /// The patch of the edited state: the audio thread plays it from its next block. False, and nothing changes, when
    /// the state was replaced since version `version` was read.
    pub fn play(&self, version: u64, patch: dsp::Patch) -> bool {
        let Ok(_s) = self.state.read() else { return false };
        if self.state_version.load(Ordering::Acquire) != version {
            return false;
        }
        if let Ok(mut e) = self.edit.lock() {
            *e = Some((version, patch));
        }
        true
    }

    /// An edit from the editor (spec plugins/freqbat.md §6): `bytes` become the state the host saves and the audio
    /// thread plays `patch` from its next block. The state version stays (the audio thread does not load the bytes
    /// again). False, and nothing changes, when the state was replaced since version `version` was read.
    pub fn edited(&self, version: u64, bytes: Vec<u8>, patch: dsp::Patch) -> bool {
        let Ok(mut s) = self.state.write() else { return false };
        if self.state_version.load(Ordering::Acquire) != version {
            return false;
        }
        *s = StateBytes(bytes);
        if let Ok(mut e) = self.edit.lock() {
            *e = Some((version, patch));
        }
        true
    }
}

/// The channel the testing keyboard's notes are held on (no MIDI channel goes past 15).
const PREVIEW: u8 = 255;

/// The poly modulation ids of Mod X and Mod Y: a note's mod X / Y as the host's per-note modulation, in mod units
/// added to the parameter (spec: plugins/freqbat.md §2).
pub const MOD_X_POLY: u32 = 1;
pub const MOD_Y_POLY: u32 = 2;

use crate::dsp;

mod dsp {
    use super::*;
    
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct Patch {
        pub version: u64,
    }
}