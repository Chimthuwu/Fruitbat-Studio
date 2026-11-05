// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Note Selector parameter definitions and saved-state encoding.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use nih_plug::prelude::*;
use os_plugkit::live::LiveEdits;

/// Parameter indices matching `Settings` order.
#[allow(dead_code)]
pub mod idx {
    pub const MODE: usize = 0;
    pub const MANUAL_SLOT: usize = 1;
    pub const SLOT1_TYPE: usize = 2;
    pub const SLOT1_ACTIVE: usize = 3;
    pub const SLOT2_TYPE: usize = 4;
    pub const SLOT2_ACTIVE: usize = 5;
    pub const SLOT3_TYPE: usize = 6;
    pub const SLOT3_ACTIVE: usize = 7;
    pub const SLOT4_TYPE: usize = 8;
    pub const SLOT4_ACTIVE: usize = 9;
}

/// Factory defaults (knob positions / stored ints).
pub const START: [i32; 10] = [
    0, // mode: 0=Manual, 1=Channel, 2=NoteRange, 3=CC1
    0, // manual_slot: 0-3
    0, // slot1_type: 0=Bypass, 1=Arp, 2=Chord, 3=Echo
    1, // slot1_active
    0, // slot2_type
    1, // slot2_active
    0, // slot3_type
    1, // slot3_active
    0, // slot4_type
    1, // slot4_active
];

/// The saved state: version + 10 ints + 0 tail bytes.
pub const STATE_LEN: usize = 1 + 10;
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

/// Selection mode.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Enum, Default)]
pub enum Mode {
    #[name = "Manual"]
    #[default]
    Manual = 0,
    #[name = "Channel"]
    Channel = 1,
    #[name = "NoteRange"]
    NoteRange = 2,
    #[name = "CC1"]
    Cc1 = 3,
}

impl Mode {
    pub fn from_index(i: u8) -> Self {
        match i {
            0 => Mode::Manual,
            1 => Mode::Channel,
            2 => Mode::NoteRange,
            _ => Mode::Cc1,
        }
    }
}

/// Slot type.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Enum, Default)]
pub enum SlotType {
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

impl SlotType {
    pub fn from_index(i: u8) -> Self {
        match i {
            0 => SlotType::Bypass,
            1 => SlotType::Arp,
            2 => SlotType::Chord,
            _ => SlotType::Echo,
        }
    }
}

/// One slot's settings.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Slot {
    pub slot_type: SlotType,
    pub active: bool,
}

/// Settings passed to the DSP.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Settings {
    pub mode: Mode,
    pub manual_slot: u8,
    pub slots: [Slot; 4],
}

/// How a setting's value reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Text {
    Mode,
    Slot,
    SlotType,
    OnOff,
}

/// One setting: id, name, text.
#[derive(Clone, Debug)]
pub struct Def {
    pub id: &'static str,
    pub name: &'static str,
    pub text: Text,
}

pub const DEFS: [Def; 10] = [
    Def { id: "mode", name: "Mode", text: Text::Mode },
    Def { id: "manual_slot", name: "Manual Slot", text: Text::Slot },
    Def { id: "slot1_type", name: "Slot 1 Type", text: Text::SlotType },
    Def { id: "slot1_active", name: "Slot 1 Active", text: Text::OnOff },
    Def { id: "slot2_type", name: "Slot 2 Type", text: Text::SlotType },
    Def { id: "slot2_active", name: "Slot 2 Active", text: Text::OnOff },
    Def { id: "slot3_type", name: "Slot 3 Type", text: Text::SlotType },
    Def { id: "slot3_active", name: "Slot 3 Active", text: Text::OnOff },
    Def { id: "slot4_type", name: "Slot 4 Type", text: Text::SlotType },
    Def { id: "slot4_active", name: "Slot 4 Active", text: Text::OnOff },
];

pub fn index(id: &str) -> Option<usize> {
    DEFS.iter().position(|d| d.id == id)
}

/// A setting's value text.
pub fn value_text(k: usize, v: i32) -> String {
    match DEFS[k].text {
        Text::Mode => match Mode::from_index(v as u8) {
            Mode::Manual => "Manual".into(),
            Mode::Channel => "Channel".into(),
            Mode::NoteRange => "NoteRange".into(),
            Mode::Cc1 => "CC1".into(),
        },
        Text::Slot => format!("{}", v + 1),
        Text::SlotType => match SlotType::from_index(v as u8) {
            SlotType::Bypass => "Bypass".into(),
            SlotType::Arp => "Arp".into(),
            SlotType::Chord => "Chord".into(),
            SlotType::Echo => "Echo".into(),
        },
        Text::OnOff => if v != 0 { "On" } else { "Off" }.into(),
    }
}

impl Text {
    fn range_min(&self) -> i32 {
        match self {
            Text::Mode => 0,
            Text::Slot => 0,
            Text::SlotType => 0,
            Text::OnOff => 0,
        }
    }
    fn range_max(&self) -> i32 {
        match self {
            Text::Mode => 3,
            Text::Slot => 3,
            Text::SlotType => 3,
            Text::OnOff => 1,
        }
    }
}

/// A typed value: names pick their setting, a number gives the stored value.
pub fn parse_text(k: usize, s: &str) -> Option<i32> {
    let s = s.trim().to_ascii_lowercase();
    let (min, max) = (DEFS[k].text.range_min(), DEFS[k].text.range_max());
    match DEFS[k].text {
        Text::Mode => ["manual", "channel", "noterange", "note range", "cc1"].iter().position(|n| s.starts_with(n)).and_then(|i| if i == 4 { Some(3) } else if i == 3 { Some(2) } else { Some(i) }).map(|i| i as i32),
        Text::SlotType => ["bypass", "arp", "chord", "echo"].iter().position(|n| s.starts_with(n)).map(|i| i as i32),
        Text::OnOff => ["off", "on"].iter().position(|n| s == *n).map(|i| i as i32),
        Text::Slot => {
            let lead: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
            let x: i32 = lead.parse().ok()?;
            // Slot text is 1-based (1-4), stored 0-3.
            let v = (x - 1).clamp(min, max);
            Some(v)
        }
    }
}

pub struct SelectorParams {
    /// The settings in the original's order.
    pub p: Vec<IntParam>,
    /// The raw saved state the parameters came from.
    pub raw: RwLock<Vec<u8>>,
    /// Bumped on every `raw` change.
    pub raw_version: AtomicU64,
    /// Editor edits the host has not confirmed yet (the sound hears them at once).
    pub live: Arc<LiveEdits>,
}

impl Default for SelectorParams {
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
        SelectorParams { p, raw: RwLock::new(default_state()), raw_version: AtomicU64::new(0), live: Arc::default() }
    }
}

unsafe impl Params for SelectorParams {
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

impl Settings {
    /// Read settings from saved state (version + 10 ints).
    pub fn from_state(b: &[u8]) -> Option<Settings> {
        if b.len() < 1 + 10 * 4 {
            return None;
        }
        let read = |k: usize| -> i32 {
            let off = 1 + k * 4;
            b.get(off..off + 4).map(|s| i32::from_le_bytes(s.try_into().unwrap())).unwrap_or(0)
        };
        let slot = |t: usize, a: usize| Slot {
            slot_type: SlotType::from_index(read(t) as u8),
            active: read(a) != 0,
        };
        Some(Settings {
            mode: Mode::from_index(read(0) as u8),
            manual_slot: read(1) as u8,
            slots: [slot(2, 3), slot(4, 5), slot(6, 7), slot(8, 9)],
        })
    }

    /// Write settings to saved state (version + 10 ints).
    pub fn to_state(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(1 + 10 * 4);
        v.push(VERSION);
        v.extend_from_slice(&(self.mode as i32).to_le_bytes());
        v.extend_from_slice(&(self.manual_slot as i32).to_le_bytes());
        for s in &self.slots {
            v.extend_from_slice(&(s.slot_type as i32).to_le_bytes());
            v.extend_from_slice(&(s.active as i32).to_le_bytes());
        }
        v
    }
}

impl SelectorParams {
    /// Setting `k`'s value for the sound (an editor edit the host has not confirmed yet wins).
    pub fn v(&self, k: usize) -> i32 {
        self.live.value(&self.p[k])
    }

    /// The settings the sound plays: settings whose stored value still means the same keep it.
    pub fn settings(&self, raw: &[u8]) -> Settings {
        let v: [i32; 10] = std::array::from_fn(|k| {
            let v = self.v(k);
            let s = get_field(raw, k);
            if decode(k, s) == v { s } else { v }
        });
        let slot = |t: usize, a: usize| Slot {
            slot_type: SlotType::from_index(v[t] as u8),
            active: v[a] != 0,
        };
        Settings {
            mode: Mode::from_index(v[idx::MODE] as u8),
            manual_slot: v[idx::MANUAL_SLOT] as u8,
            slots: [slot(2, 3), slot(4, 5), slot(6, 7), slot(8, 9)],
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
    let params: Vec<String> = values_of(&b).iter().zip(&DEFS).map(|(v, d)| format!("\"{}\": {{\"i32\":{v}}}", d.id)).collect();
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
            ("mode", 0, "Manual"),
            ("mode", 1, "Channel"),
            ("mode", 2, "NoteRange"),
            ("mode", 3, "CC1"),
            ("manual_slot", 0, "1"),
            ("slot1_type", 2, "Chord"),
            ("slot1_active", 1, "On"),
            ("slot2_active", 0, "Off"),
        ] {
            assert_eq!(value_text(index(id).unwrap(), v), s, "{id} {v}");
        }
    }

    #[test]
    fn typed_values_find_nearest() {
        let p = |id: &str, s: &str| parse_text(index(id).unwrap(), s);
        assert_eq!(p("mode", "channel"), Some(1));
        assert_eq!(p("slot1_type", "arp"), Some(1));
        assert_eq!(p("slot1_active", "on"), Some(1));
        assert_eq!(p("manual_slot", "3"), Some(2));
    }

    #[test]
    fn state_round_trips() {
        let mut fl = default_state();
        set_field(&mut fl, idx::MODE, 2);
        set_field(&mut fl, idx::MANUAL_SLOT, 3);
        set_field(&mut fl, idx::SLOT2_TYPE, 2);
        set_field(&mut fl, idx::SLOT3_ACTIVE, 0);
        let blob = plugin_state_from_fl(&fl);
        assert_eq!(fl_from_plugin_state(&blob).expect("parses"), fl);
        let p = SelectorParams::default();
        let json: serde_json::Value = serde_json::from_slice(&blob[8..]).unwrap();
        let fields: BTreeMap<String, String> = json["fields"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect();
        p.deserialize_fields(&fields);
        assert_eq!(*p.raw.read().unwrap(), fl);
        assert_eq!(Settings::from_state(&fl).unwrap().to_state(), fl);
    }
}
