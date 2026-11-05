// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Note Layer parameter definitions and saved-state encoding.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use nih_plug::prelude::*;
use os_plugkit::live::LiveEdits;

/// Parameter indices matching `Settings` order.
#[allow(dead_code)]
pub mod idx {
    pub const LAYER0_ACTIVE: usize = 0;
    pub const LAYER0_TYPE: usize = 1;
    pub const LAYER0_CHANNEL: usize = 2;
    pub const LAYER0_LEVEL: usize = 3;
    pub const LAYER1_ACTIVE: usize = 4;
    pub const LAYER1_TYPE: usize = 5;
    pub const LAYER1_CHANNEL: usize = 6;
    pub const LAYER1_LEVEL: usize = 7;
    pub const LAYER2_ACTIVE: usize = 8;
    pub const LAYER2_TYPE: usize = 9;
    pub const LAYER2_CHANNEL: usize = 10;
    pub const LAYER2_LEVEL: usize = 11;
    pub const LAYER3_ACTIVE: usize = 12;
    pub const LAYER3_TYPE: usize = 13;
    pub const LAYER3_CHANNEL: usize = 14;
    pub const LAYER3_LEVEL: usize = 15;
    pub const DRY_LEVEL: usize = 16;
}

/// Layer slot type.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Enum, Default)]
pub enum LayerType {
    #[name = "Bypass"]
    #[default]
    Bypass = 0,
    #[name = "Arp"]
    Arp = 1,
    #[name = "Chord"]
    Chord = 2,
    #[name = "Echo"]
    Echo = 3,
}

impl LayerType {
    pub fn from_index(i: u8) -> Self {
        match i {
            0 => LayerType::Bypass,
            1 => LayerType::Arp,
            2 => LayerType::Chord,
            _ => LayerType::Echo,
        }
    }
}

/// One layer slot's settings.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct LayerSlot {
    pub active: bool,
    pub typ: LayerType,
    /// 0-16 where 16 = All channels.
    pub channel: u8,
    /// 0-100.
    pub level: u8,
}

/// Factory defaults (knob positions / stored ints).
/// Order: per layer (active, type, channel, level) x4, then dry level.
pub const START: [i32; 17] = [
    1, 0, 16, 100, // layer 0: on, bypass, all, full
    0, 0, 16, 100, // layer 1: off
    0, 0, 16, 100, // layer 2: off
    0, 0, 16, 100, // layer 3: off
    100, // dry level
];

/// The saved state: version + 17 ints + 0 tail bytes.
pub const STATE_LEN: usize = 1 + 17;
pub const STATE_FIELD: &str = "state";
pub const VERSION: u8 = 1;

/// Build a fresh default state (version 1 + START).
pub fn default_state() -> Vec<u8> {
    let mut v = Vec::with_capacity(STATE_LEN);
    v.push(VERSION);
    for s in START {
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}

/// Read a field from a stored state (version + ints).
pub fn get_field(state: &[u8], idx: usize) -> i32 {
    let off = 1 + idx * 4;
    i32::from_le_bytes(state[off..off + 4].try_into().unwrap())
}

/// Write a field into a stored state.
pub fn set_field(state: &mut [u8], idx: usize, val: i32) {
    let off = 1 + idx * 4;
    state[off..off + 4].copy_from_slice(&val.to_le_bytes());
}

/// Settings passed to the DSP.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Settings {
    pub layers: [LayerSlot; 4],
    pub dry_level: u8,
}

/// How a setting's value reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Text {
    Percent,
    OnOff,
    LayerType,
    Channel,
}

impl Text {
    fn range_min(&self) -> i32 {
        match self {
            Text::OnOff | Text::LayerType | Text::Channel | Text::Percent => 0,
        }
    }
    fn range_max(&self) -> i32 {
        match self {
            Text::OnOff => 1,
            Text::LayerType => 3,
            Text::Channel => 16,
            Text::Percent => 100,
        }
    }
}

/// One setting: id, name, text.
#[derive(Clone, Debug)]
pub struct Def {
    pub id: &'static str,
    pub name: &'static str,
    pub text: Text,
}

pub const DEFS: [Def; 17] = [
    Def { id: "layer1_active", name: "Layer 1 Active", text: Text::OnOff },
    Def { id: "layer1_type", name: "Layer 1 Type", text: Text::LayerType },
    Def { id: "layer1_channel", name: "Layer 1 Channel", text: Text::Channel },
    Def { id: "layer1_level", name: "Layer 1 Level", text: Text::Percent },
    Def { id: "layer2_active", name: "Layer 2 Active", text: Text::OnOff },
    Def { id: "layer2_type", name: "Layer 2 Type", text: Text::LayerType },
    Def { id: "layer2_channel", name: "Layer 2 Channel", text: Text::Channel },
    Def { id: "layer2_level", name: "Layer 2 Level", text: Text::Percent },
    Def { id: "layer3_active", name: "Layer 3 Active", text: Text::OnOff },
    Def { id: "layer3_type", name: "Layer 3 Type", text: Text::LayerType },
    Def { id: "layer3_channel", name: "Layer 3 Channel", text: Text::Channel },
    Def { id: "layer3_level", name: "Layer 3 Level", text: Text::Percent },
    Def { id: "layer4_active", name: "Layer 4 Active", text: Text::OnOff },
    Def { id: "layer4_type", name: "Layer 4 Type", text: Text::LayerType },
    Def { id: "layer4_channel", name: "Layer 4 Channel", text: Text::Channel },
    Def { id: "layer4_level", name: "Layer 4 Level", text: Text::Percent },
    Def { id: "dry_level", name: "Dry Level", text: Text::Percent },
];

pub fn index(id: &str) -> Option<usize> {
    DEFS.iter().position(|d| d.id == id)
}

/// A setting's value text.
pub fn value_text(k: usize, v: i32) -> String {
    match DEFS[k].text {
        Text::LayerType => match LayerType::from_index(v as u8) {
            LayerType::Bypass => "Bypass".into(),
            LayerType::Arp => "Arp".into(),
            LayerType::Chord => "Chord".into(),
            LayerType::Echo => "Echo".into(),
        },
        Text::Channel => if v == 16 { "All".into() } else { format!("{v}") },
        Text::Percent => format!("{v}.0%"),
        Text::OnOff => if v != 0 { "On" } else { "Off" }.into(),
    }
}

/// A typed value: names pick their setting, a number gives the stored value.
pub fn parse_text(k: usize, s: &str) -> Option<i32> {
    let s = s.trim().to_ascii_lowercase();
    let (min, max) = (DEFS[k].text.range_min(), DEFS[k].text.range_max());
    match DEFS[k].text {
        Text::LayerType => ["bypass", "arp", "chord", "echo"].iter().position(|n| s.starts_with(n)).map(|i| i as i32),
        Text::Channel => {
            if s == "all" {
                Some(16)
            } else {
                let lead: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
                let x: i32 = lead.parse().ok()?;
                (min..=max).contains(&x).then_some(x)
            }
        }
        Text::OnOff => ["off", "on"].iter().position(|n| s == *n).map(|i| i as i32),
        Text::Percent => {
            let lead: String = s.chars().take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+')).collect();
            let x: f64 = lead.parse().ok()?;
            (min..=max)
                .filter_map(|v| {
                    let text = value_text(k, v);
                    let n: f64 = text.trim_end_matches('%').parse().ok()?;
                    Some((v, (n - x).abs()))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(v, _)| v)
        }
    }
}

pub struct NoteLayerParams {
    /// The settings in the original's order.
    pub p: Vec<IntParam>,
    /// The raw saved state the parameters came from.
    pub raw: RwLock<Vec<u8>>,
    /// Bumped on every `raw` change.
    pub raw_version: AtomicU64,
    /// Editor edits the host has not confirmed yet (the sound hears them at once).
    pub live: Arc<LiveEdits>,
}

impl Default for NoteLayerParams {
    fn default() -> Self {
        let p = DEFS
            .iter()
            .enumerate()
            .map(|(k, d)| {
                let (min, max) = (d.text.range_min(), d.text.range_max());
                IntParam::new(d.name, START[k], IntRange::Linear { min, max })
                    .with_value_to_string(Arc::new(move |v| value_text(k, v)))
                    .with_string_to_value(Arc::new(move |s| parse_text(k, s)))
            })
            .collect();
        NoteLayerParams { p, raw: RwLock::new(default_state()), raw_version: AtomicU64::new(0), live: Arc::default() }
    }
}

unsafe impl Params for NoteLayerParams {
    fn param_map(&self) -> Vec<(String, ParamPtr, String)> {
        self.p.iter().zip(&DEFS).map(|(p, d)| (d.id.to_string(), p.as_ptr(), String::new())).collect()
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

fn slot_from(v: &[i32], i: usize) -> LayerSlot {
    LayerSlot {
        active: v[i] != 0,
        typ: LayerType::from_index(v[i + 1] as u8),
        channel: v[i + 2] as u8,
        level: v[i + 3] as u8,
    }
}

fn slot_to(s: &LayerSlot, out: &mut Vec<u8>) {
    out.extend_from_slice(&(s.active as i32).to_le_bytes());
    out.extend_from_slice(&(s.typ as i32).to_le_bytes());
    out.extend_from_slice(&(s.channel as i32).to_le_bytes());
    out.extend_from_slice(&(s.level as i32).to_le_bytes());
}

impl Settings {
    /// Read settings from saved state (version + 17 ints).
    pub fn from_state(b: &[u8]) -> Option<Settings> {
        if b.len() < 1 + 17 * 4 {
            return None;
        }
        let read = |k: usize| -> i32 {
            let off = 1 + k * 4;
            b.get(off..off + 4).map(|s| i32::from_le_bytes(s.try_into().unwrap())).unwrap_or(0)
        };
        let v: [i32; 17] = std::array::from_fn(read);
        Some(Settings {
            layers: [slot_from(&v, 0), slot_from(&v, 4), slot_from(&v, 8), slot_from(&v, 12)],
            dry_level: v[16] as u8,
        })
    }

    /// Write settings to saved state (version + 17 ints).
    pub fn to_state(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(1 + 17 * 4);
        v.push(VERSION);
        for s in &self.layers {
            slot_to(s, &mut v);
        }
        v.extend_from_slice(&(self.dry_level as i32).to_le_bytes());
        v
    }
}

impl NoteLayerParams {
    /// Setting `k`'s value for the sound (an editor edit the host has not confirmed yet wins).
    pub fn v(&self, k: usize) -> i32 {
        self.live.value(&self.p[k])
    }

    /// The settings the sound plays: settings whose stored value still means the same keep it.
    pub fn settings(&self, raw: &[u8]) -> Settings {
        let v: [i32; 17] = std::array::from_fn(|k| {
            let v = self.v(k);
            let s = get_field(raw, k);
            if decode(k, s) == v { s } else { v }
        });
        Settings {
            layers: [slot_from(&v, 0), slot_from(&v, 4), slot_from(&v, 8), slot_from(&v, 12)],
            dry_level: v[idx::DRY_LEVEL] as u8,
        }
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

    /// The original's saved state of the current settings.
    pub fn fl_state(&self) -> Vec<u8> {
        let vals: Vec<i32> = self.p.iter().map(|p| p.value()).collect();
        to_fl(&vals, &self.raw_copy())
    }
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    (s.len() % 2 == 0).then_some(())?;
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Any readable saved state (version 0 or 1) in the version-1 layout; unreadable ones give the start (`START`).
pub fn normalized(b: Vec<u8>) -> Vec<u8> {
    match Settings::from_state(&b) {
        Some(_) if b.len() >= STATE_LEN && b[0] == VERSION => b,
        Some(s) => s.to_state(),
        None => default_state(),
    }
}

/// Setting `k`'s parameter value for a stored field value.
fn decode(k: usize, s: i32) -> i32 {
    s.clamp(DEFS[k].text.range_min(), DEFS[k].text.range_max())
}

/// The original's saved state with values `vals`: fields whose value still means the same keep their stored bytes.
pub fn to_fl(vals: &[i32], raw: &[u8]) -> Vec<u8> {
    let mut b = normalized(raw.to_vec());
    for (k, &v) in vals.iter().enumerate().take(DEFS.len()) {
        if decode(k, get_field(&b, k)) != v {
            set_field(&mut b, k, v);
        }
    }
    b
}

/// Parameter values of a saved state.
pub fn values_of(fl: &[u8]) -> Vec<i32> {
    (0..DEFS.len()).map(|k| decode(k, get_field(fl, k))).collect()
}

/// The plugin's own saved state for a slot's saved state.
pub fn plugin_state_from_fl(fl: &[u8]) -> Vec<u8> {
    let b = normalized(fl.to_vec());
    let params: Vec<String> = values_of(&b).iter().zip(&DEFS).map(|(v, d)| format!("\"{}\":{{\"i32\":{v}}}", d.id)).collect();
    let state = format!(
        "{{\"version\":\"{}\",\"params\":{{{}}},\"fields\":{{\"{STATE_FIELD}\":\"\\\"{}\\\"\"}}}}",
        env!("CARGO_PKG_VERSION"),
        params.join(","),
        hex(&b)
    );
    let mut blob = (state.len() as u64).to_le_bytes().to_vec();
    blob.extend_from_slice(state.as_bytes());
    blob
}

/// The original's saved state for a plugin state blob; `None` when it is not one.
pub fn fl_from_plugin_state(blob: &[u8]) -> Option<Vec<u8>> {
    let (_, json) = os_hostlink::state::json(blob);
    let v: serde_json::Value = serde_json::from_str(std::str::from_utf8(json).ok()?).ok()?;
    let raw = normalized(from_hex(v.get("fields")?.get(STATE_FIELD)?.as_str()?.trim_matches('"'))?);
    let params = v.get("params")?;
    let mut vals = values_of(&raw);
    for (k, d) in DEFS.iter().enumerate() {
        if let Some(x) = params.get(d.id).and_then(|p| p.get("i32")).and_then(|x| x.as_i64()) {
            vals[k] = x as i32;
        }
    }
    Some(to_fl(&vals, &raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_texts_match() {
        for (id, v, s) in [
            ("layer1_active", 1, "On"),
            ("layer1_active", 0, "Off"),
            ("layer1_type", 0, "Bypass"),
            ("layer1_type", 1, "Arp"),
            ("layer1_type", 2, "Chord"),
            ("layer1_type", 3, "Echo"),
            ("layer1_channel", 5, "5"),
            ("layer1_channel", 16, "All"),
            ("layer1_level", 80, "80.0%"),
            ("dry_level", 100, "100.0%"),
        ] {
            assert_eq!(value_text(index(id).unwrap(), v), s, "{id} {v}");
        }
    }

    #[test]
    fn typed_values_find_nearest() {
        let p = |id: &str, s: &str| parse_text(index(id).unwrap(), s);
        assert_eq!(p("layer1_type", "chord"), Some(2));
        assert_eq!(p("layer1_type", "echo"), Some(3));
        assert_eq!(p("layer1_channel", "3"), Some(3));
        assert_eq!(p("layer1_channel", "all"), Some(16));
        assert_eq!(p("layer1_active", "on"), Some(1));
        assert_eq!(p("layer1_level", "80"), Some(80));
        assert_eq!(p("dry_level", "50"), Some(50));
    }

    #[test]
    fn state_round_trips() {
        let mut fl = default_state();
        set_field(&mut fl, idx::LAYER0_TYPE, 2);
        set_field(&mut fl, idx::DRY_LEVEL, 42);
        let blob = plugin_state_from_fl(&fl);
        assert_eq!(fl_from_plugin_state(&blob).expect("parses"), fl);
        let p = NoteLayerParams::default();
        let json: serde_json::Value = serde_json::from_slice(&blob[8..]).unwrap();
        let fields: BTreeMap<String, String> = json["fields"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect();
        p.deserialize_fields(&fields);
        assert_eq!(*p.raw.read().unwrap(), fl);
    }
}
