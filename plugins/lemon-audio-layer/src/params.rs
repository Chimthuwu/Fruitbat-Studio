// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Audio Layer parameters and saved state.
//!
//! 18 parameters: master dry/wet plus 4 parallel slots, each with active/type/level/param.
//! The saved state is a version byte followed by 18 little-endian int32s.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use nih_plug::prelude::*;
use os_plugkit::live::LiveEdits;

/// Slot effect types: 0=Bypass, 1=Filter, 2=Delay, 3=Chorus, 4=Drive.
pub const TYPE_NAMES: [&str; 5] = ["Bypass", "Filter", "Delay", "Chorus", "Drive"];
pub const SLOT_COUNT: usize = 4;
pub const PARAM_COUNT: usize = 18;

/// One parallel slot's settings.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SlotSettings {
    pub active: bool,
    pub ty: u8,
    pub level: i32,
    pub param: i32,
}

/// Settings passed to the DSP.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub dry: i32,
    pub wet: i32,
    pub slots: [SlotSettings; SLOT_COUNT],
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            dry: 6000,
            wet: 6000,
            slots: [SlotSettings { active: false, ty: 0, level: 6000, param: 3000 }; SLOT_COUNT],
        }
    }
}

/// How a setting's value reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Text {
    Percent,
    OnOff,
    Type,
}

/// One setting: id, name, text.
#[derive(Clone, Debug)]
pub struct Def {
    pub id: &'static str,
    pub name: &'static str,
    pub text: Text,
}

static DEFS: std::sync::OnceLock<[Def; PARAM_COUNT]> = std::sync::OnceLock::new();

fn build_defs() -> [Def; PARAM_COUNT] {
    [
        Def { id: "dry", name: "Master Dry", text: Text::Percent },
        Def { id: "wet", name: "Master Wet", text: Text::Percent },
        Def { id: "s1active", name: "Slot 1 Active", text: Text::OnOff },
        Def { id: "s1type", name: "Slot 1 Type", text: Text::Type },
        Def { id: "s1level", name: "Slot 1 Level", text: Text::Percent },
        Def { id: "s1param", name: "Slot 1 Param", text: Text::Percent },
        Def { id: "s2active", name: "Slot 2 Active", text: Text::OnOff },
        Def { id: "s2type", name: "Slot 2 Type", text: Text::Type },
        Def { id: "s2level", name: "Slot 2 Level", text: Text::Percent },
        Def { id: "s2param", name: "Slot 2 Param", text: Text::Percent },
        Def { id: "s3active", name: "Slot 3 Active", text: Text::OnOff },
        Def { id: "s3type", name: "Slot 3 Type", text: Text::Type },
        Def { id: "s3level", name: "Slot 3 Level", text: Text::Percent },
        Def { id: "s3param", name: "Slot 3 Param", text: Text::Percent },
        Def { id: "s4active", name: "Slot 4 Active", text: Text::OnOff },
        Def { id: "s4type", name: "Slot 4 Type", text: Text::Type },
        Def { id: "s4level", name: "Slot 4 Level", text: Text::Percent },
        Def { id: "s4param", name: "Slot 4 Param", text: Text::Percent },
    ]
}

pub fn defs() -> &'static [Def; PARAM_COUNT] {
    DEFS.get_or_init(build_defs)
}

/// Factory defaults (stored ints).
pub fn start_values() -> [i32; PARAM_COUNT] {
    let mut v = [0i32; PARAM_COUNT];
    v[0] = 6000; // dry
    v[1] = 6000; // wet
    for i in 0..SLOT_COUNT {
        let b = 2 + i * 4;
        v[b] = 0; // active
        v[b + 1] = 0; // type = Bypass
        v[b + 2] = 6000; // level
        v[b + 3] = 3000; // param
    }
    v
}

pub const RANGES: [(i32, i32); PARAM_COUNT] = [
    (0, 6000), // dry
    (0, 6000), // wet
    (0, 1), (0, 4), (0, 6000), (0, 6000), // slot 1
    (0, 1), (0, 4), (0, 6000), (0, 6000), // slot 2
    (0, 1), (0, 4), (0, 6000), (0, 6000), // slot 3
    (0, 1), (0, 4), (0, 6000), (0, 6000), // slot 4
];

/// The saved state: version + 18 ints.
pub const STATE_LEN: usize = 1 + 18 * 4;
pub const STATE_FIELD: &str = "state";
pub const VERSION: u8 = 1;

/// Setting index of an id.
pub fn index(id: &str) -> Option<usize> {
    defs().iter().position(|d| d.id == id)
}

/// Build a fresh default state (version 1 + START).
pub fn default_state() -> Vec<u8> {
    Settings::default().to_state()
}

/// Read a field from a stored state (version + ints).
fn field(b: &[u8], k: usize) -> i32 {
    let off = 1 + k * 4;
    b.get(off..off + 4).map_or(0, |s| i32::from_le_bytes(s.try_into().unwrap()))
}

/// Write a field into a stored state.
fn set_field(b: &mut [u8], k: usize, val: i32) {
    let off = 1 + k * 4;
    if let Some(s) = b.get_mut(off..off + 4) {
        s.copy_from_slice(&val.to_le_bytes());
    }
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    (s.len() % 2 == 0).then_some(())?;
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Any readable saved state in the version-1 layout; unreadable ones give the default.
pub fn normalized(b: Vec<u8>) -> Vec<u8> {
    match Settings::from_state(&b) {
        Some(_) if b.len() >= STATE_LEN && b[0] == VERSION => b,
        Some(s) => s.to_state(),
        None => default_state(),
    }
}

/// Setting `k`'s parameter value for a stored field value.
fn decode(k: usize, s: i32) -> i32 {
    s.clamp(RANGES[k].0, RANGES[k].1)
}

/// A setting's value text.
pub fn value_text(k: usize, v: i32) -> String {
    let (min, max) = RANGES[k];
    let x = if max == min { 0.0 } else { (v - min) as f32 / (max - min) as f32 };
    let d = defs();
    match d[k].text {
        Text::Type => TYPE_NAMES[v.clamp(0, 4) as usize].to_string(),
        Text::Percent => format!("{:.1}%", x * 100.0),
        Text::OnOff => if v != 0 { "On" } else { "Off" }.into(),
    }
}

/// A typed value: names pick their setting, a number gives the stored value whose text shows the nearest number.
pub fn parse_text(k: usize, s: &str) -> Option<i32> {
    let s = s.trim().to_ascii_lowercase();
    let (min, max) = RANGES[k];
    let d = defs();
    match d[k].text {
        Text::Type => TYPE_NAMES.iter().position(|n| s.starts_with(&n.to_ascii_lowercase())).map(|i| i as i32),
        Text::OnOff => ["off", "on"].iter().position(|n| s == *n).map(|i| i as i32),
        Text::Percent => {
            let lead: String = s.chars().take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+')).collect();
            let x: f64 = lead.parse().ok()?;
            let v = ((x / 100.0) * (max - min) as f64).round() as i32 + min;
            Some(v.clamp(min, max))
        }
    }
}

/// Parameter values of a saved state.
pub fn values_of(fl: &[u8]) -> Vec<i32> {
    (0..PARAM_COUNT).map(|k| decode(k, field(fl, k + 1))).collect()
}

/// The saved state with values `vals`: fields whose value still means the same keep their stored bytes.
pub fn to_fl(vals: &[i32], raw: &[u8]) -> Vec<u8> {
    let mut b = normalized(raw.to_vec());
    for (k, &v) in vals.iter().enumerate().take(PARAM_COUNT) {
        if decode(k, field(&b, k + 1)) != v {
            set_field(&mut b, k + 1, v);
        }
    }
    b
}

pub struct AudioLayerParams {
    /// The settings in order.
    pub p: Vec<IntParam>,
    /// The saved state the parameters came from.
    pub raw: RwLock<Vec<u8>>,
    /// Bumped on every `raw` change.
    pub raw_version: AtomicU64,
    /// Editor edits the host has not confirmed yet (the sound hears them at once).
    pub live: Arc<LiveEdits>,
}

impl Default for AudioLayerParams {
    fn default() -> Self {
        let d = defs();
        let start = start_values();
        let p = d
            .iter()
            .enumerate()
            .map(|(k, def)| {
                let (min, max) = RANGES[k];
                IntParam::new(def.name, start[k], IntRange::Linear { min, max })
                    .with_value_to_string(Arc::new(move |v| value_text(k, v)))
                    .with_string_to_value(Arc::new(move |s| parse_text(k, s)))
            })
            .collect();
        AudioLayerParams { p, raw: RwLock::new(default_state()), raw_version: AtomicU64::new(0), live: Arc::default() }
    }
}

unsafe impl Params for AudioLayerParams {
    fn param_map(&self) -> Vec<(String, ParamPtr, String)> {
        let d = defs();
        self.p.iter().zip(d.iter()).map(|(p, def)| (def.id.to_string(), p.as_ptr(), String::new())).collect()
    }

    fn serialize_fields(&self) -> BTreeMap<String, String> {
        let hex = self.raw.read().map(|b| hex(&b)).unwrap_or_default();
        BTreeMap::from([(STATE_FIELD.to_string(), format!("\"{hex}\""))])
    }

    fn deserialize_fields(&self, serialized: &BTreeMap<String, String>) {
        if let Some(bytes) = serialized.get(STATE_FIELD).and_then(|s| from_hex(s.trim_matches('"'))) {
            self.set_raw(normalized(bytes));
        }
    }
}

impl Settings {
    /// Read settings from saved state (version + 18 ints).
    pub fn from_state(b: &[u8]) -> Option<Settings> {
        if b.len() < STATE_LEN {
            return None;
        }
        let read = |k: usize| -> i32 {
            let off = 1 + k * 4;
            b.get(off..off + 4).map(|s| i32::from_le_bytes(s.try_into().unwrap())).unwrap_or(0)
        };
        let mut slots = [SlotSettings::default(); SLOT_COUNT];
        for i in 0..SLOT_COUNT {
            let base = 2 + i * 4;
            slots[i] = SlotSettings { active: read(base) != 0, ty: read(base + 1) as u8, level: read(base + 2), param: read(base + 3) };
        }
        Some(Settings { dry: read(0), wet: read(1), slots })
    }

    /// Write settings to saved state (version + 18 ints).
    pub fn to_state(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(STATE_LEN);
        v.push(VERSION);
        v.extend_from_slice(&self.dry.to_le_bytes());
        v.extend_from_slice(&self.wet.to_le_bytes());
        for s in &self.slots {
            v.extend_from_slice(&(s.active as i32).to_le_bytes());
            v.extend_from_slice(&(s.ty as i32).to_le_bytes());
            v.extend_from_slice(&s.level.to_le_bytes());
            v.extend_from_slice(&s.param.to_le_bytes());
        }
        v
    }
}

impl AudioLayerParams {
    /// Setting `k`'s value for the sound (an editor edit the host has not confirmed yet wins).
    pub fn v(&self, k: usize) -> i32 {
        self.live.value(&self.p[k])
    }

    /// The settings the sound plays.
    pub fn settings(&self, raw: &[u8]) -> Settings {
        let v: [i32; PARAM_COUNT] = std::array::from_fn(|k| {
            let v = self.v(k);
            let s = field(raw, k + 1);
            if decode(k, s) == v { s } else { v }
        });
        let mut slots = [SlotSettings::default(); SLOT_COUNT];
        for i in 0..SLOT_COUNT {
            let base = 2 + i * 4;
            slots[i] = SlotSettings { active: v[base] != 0, ty: v[base + 1] as u8, level: v[base + 2], param: v[base + 3] };
        }
        Settings { dry: v[0], wet: v[1], slots }
    }

    pub fn set_raw(&self, b: Vec<u8>) {
        if let Ok(mut r) = self.raw.write() {
            *r = b;
        }
        self.raw_version.fetch_add(1, Ordering::AcqRel);
    }

    pub fn raw_copy(&self) -> Vec<u8> {
        self.raw.read().map(|b| b.clone()).unwrap_or_else(|_| default_state())
    }

    /// The saved state of the current settings.
    pub fn fl_state(&self) -> Vec<u8> {
        let vals: Vec<i32> = self.p.iter().map(|p| p.value()).collect();
        to_fl(&vals, &self.raw_copy())
    }
}

/// The plugin's own saved state for a saved state.
pub fn plugin_state_from_fl(fl: &[u8]) -> Vec<u8> {
    let b = normalized(fl.to_vec());
    let d = defs();
    let mut params_obj = serde_json::Map::new();
    for (v, def) in values_of(&b).iter().zip(d.iter()) {
        params_obj.insert(def.id.to_string(), serde_json::json!({"i32": v}));
    }
    let state = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "params": params_obj,
        "fields": { STATE_FIELD: format!("\"{}\"", hex(&b)) },
    })
    .to_string();
    let mut blob = (state.len() as u64).to_le_bytes().to_vec();
    blob.extend_from_slice(state.as_bytes());
    blob
}

/// The saved state for a plugin state blob; `None` when it is not one.
pub fn fl_from_plugin_state(blob: &[u8]) -> Option<Vec<u8>> {
    let (_, json) = os_hostlink::state::json(blob);
    let v: serde_json::Value = serde_json::from_str(std::str::from_utf8(json).ok()?).ok()?;
    let raw = normalized(from_hex(v.get("fields")?.get(STATE_FIELD)?.as_str()?.trim_matches('"'))?);
    let params = v.get("params")?;
    let d = defs();
    let mut vals = values_of(&raw);
    for (k, def) in d.iter().enumerate() {
        if let Some(x) = params.get(def.id).and_then(|p| p.get("i32")).and_then(|x| x.as_i64()) {
            vals[k] = x as i32;
        }
    }
    Some(to_fl(&vals, &raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trips() {
        let mut s = Settings::default();
        s.dry = 1234;
        s.wet = 4321;
        s.slots[0] = SlotSettings { active: true, ty: 2, level: 5000, param: 2500 };
        s.slots[1] = SlotSettings { active: true, ty: 3, level: 4000, param: 1500 };
        s.slots[2] = SlotSettings { active: false, ty: 4, level: 6000, param: 6000 };
        s.slots[3] = SlotSettings { active: true, ty: 1, level: 3000, param: 100 };
        let blob = s.to_state();
        assert_eq!(blob.len(), STATE_LEN);
        assert_eq!(blob[0], VERSION);
        assert_eq!(Settings::from_state(&blob), Some(s));
        // Through the plugin blob layer as well.
        assert_eq!(fl_from_plugin_state(&plugin_state_from_fl(&blob)).expect("parses"), blob);
    }

    #[test]
    fn eighteen_params() {
        assert_eq!(defs().len(), PARAM_COUNT);
        let p = AudioLayerParams::default();
        assert_eq!(p.p.len(), PARAM_COUNT);
        assert_eq!(p.param_map().len(), PARAM_COUNT);
    }

    #[test]
    fn value_texts_match() {
        assert_eq!(value_text(0, 3000), "50.0%");
        assert_eq!(value_text(2, 1), "On");
        assert_eq!(value_text(3, 2), "Delay");
        assert_eq!(value_text(3, 0), "Bypass");
        assert_eq!(parse_text(3, "chorus"), Some(3));
        assert_eq!(parse_text(2, "on"), Some(1));
    }
}
