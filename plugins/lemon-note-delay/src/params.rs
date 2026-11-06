// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Note Delay parameter definitions and saved-state encoding.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use nih_plug::prelude::*;
use os_plugkit::live::LiveEdits;

/// Parameter indices matching `Settings` order.
#[allow(dead_code)]
pub mod idx {
    pub const DIVISION: usize = 0;
    pub const UNIT: usize = 1;
    pub const MS: usize = 2;
    pub const FEEDBACK: usize = 3;
    pub const DECAY: usize = 4;
}

/// Factory defaults (stored ints).
pub const START: [i32; 5] = [9, 0, 250, 0, 60];

/// The saved state: version + 5 ints.
pub const STATE_LEN: usize = 1 + 5 * 4;
pub const STATE_FIELD: &str = "state";
pub const VERSION: u8 = 1;

/// A fresh default state (version + `START`).
pub fn default_state() -> Vec<u8> {
    let mut v = Vec::with_capacity(STATE_LEN);
    v.push(VERSION);
    for s in START {
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}

/// Read a field from a stored state.
pub fn get_field(state: &[u8], idx: usize) -> i32 {
    let off = 1 + idx * 4;
    state.get(off..off + 4).map(|s| i32::from_le_bytes(s.try_into().unwrap())).unwrap_or(START[idx])
}

/// Write a field into a stored state.
pub fn set_field(state: &mut [u8], idx: usize, val: i32) {
    let off = 1 + idx * 4;
    state[off..off + 4].copy_from_slice(&val.to_le_bytes());
}

/// The settings handed to the DSP (stored values, indexed by `idx`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub v: [i32; 5],
}

impl Default for Settings {
    fn default() -> Self {
        Settings { v: START }
    }
}

/// How a setting's value reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Text {
    /// A number with a unit suffix.
    Num(&'static str),
    /// One of a list of names (value = index).
    Names(&'static [&'static str]),
    OnOff,
    /// 0 = all channels, otherwise 1..16.
    Channel,
}

/// One setting: id, name, text, range.
#[derive(Clone, Debug)]
pub struct Def {
    pub id: &'static str,
    pub name: &'static str,
    pub text: Text,
    pub min: i32,
    pub max: i32,
}

pub const DEFS: [Def; 5] = [
    Def { id: "division", name: "Division", text: Text::Names(&["1/1", "1/1 D", "1/1 T", "1/2", "1/2 D", "1/2 T", "1/4", "1/4 D", "1/4 T", "1/8", "1/8 D", "1/8 T", "1/16", "1/16 D", "1/16 T", "1/32", "1/32 D", "1/32 T"]), min: 0, max: 17 },
    Def { id: "unit", name: "Unit", text: Text::Names(&["Sync", "ms"]), min: 0, max: 1 },
    Def { id: "ms", name: "Time", text: Text::Num(" ms"), min: 0, max: 2000 },
    Def { id: "feedback", name: "Feedback", text: Text::Num(""), min: 0, max: 16 },
    Def { id: "decay", name: "Decay", text: Text::Num("%"), min: 0, max: 100 },
];

pub fn index(id: &str) -> Option<usize> {
    DEFS.iter().position(|d| d.id == id)
}

/// A setting's value text.
pub fn value_text(k: usize, v: i32) -> String {
    let d = &DEFS[k];
    let v = v.clamp(d.min, d.max);
    match d.text {
        Text::Num(unit) => format!("{v}{unit}"),
        Text::Names(n) => n[v as usize].to_string(),
        Text::OnOff => if v != 0 { "On" } else { "Off" }.into(),
        Text::Channel => if v == 0 { "All".into() } else { v.to_string() },
    }
}

/// A typed value: names pick their entry, a number gives the stored value (clamped).
pub fn parse_text(k: usize, s: &str) -> Option<i32> {
    let s = s.trim().to_ascii_lowercase();
    let d = &DEFS[k];
    match d.text {
        Text::Names(n) => n
            .iter()
            .position(|x| x.to_ascii_lowercase() == s)
            .or_else(|| n.iter().position(|x| x.to_ascii_lowercase().starts_with(&s) && !s.is_empty()))
            .map(|i| i as i32),
        Text::OnOff => ["off", "on"].iter().position(|n| s == *n).map(|i| i as i32),
        Text::Channel if s == "all" => Some(0),
        _ => {
            let lead: String = s.chars().take_while(|c| c.is_ascii_digit() || matches!(c, '-' | '+')).collect();
            lead.parse::<i32>().ok().map(|x| x.clamp(d.min, d.max))
        }
    }
}

pub struct NoteDelayParams {
    /// The settings in order.
    pub p: Vec<IntParam>,
    /// The raw saved state the parameters came from.
    pub raw: RwLock<Vec<u8>>,
    /// Bumped on every `raw` change.
    pub raw_version: AtomicU64,
    /// Editor edits the host has not confirmed yet (the sound hears them at once).
    pub live: Arc<LiveEdits>,
}

impl Default for NoteDelayParams {
    fn default() -> Self {
        let p = DEFS
            .iter()
            .enumerate()
            .map(|(k, d)| {
                IntParam::new(d.name, START[k], IntRange::Linear { min: d.min, max: d.max })
                    .with_value_to_string(Arc::new(move |v| value_text(k, v)))
                    .with_string_to_value(Arc::new(move |s| parse_text(k, s)))
            })
            .collect();
        NoteDelayParams { p, raw: RwLock::new(default_state()), raw_version: AtomicU64::new(0), live: Arc::default() }
    }
}

unsafe impl Params for NoteDelayParams {
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
    /// Read settings from a saved state.
    pub fn from_state(b: &[u8]) -> Option<Settings> {
        if b.len() < STATE_LEN {
            return None;
        }
        Some(Settings { v: std::array::from_fn(|k| decode(k, get_field(b, k))) })
    }

    /// Write settings to a saved state.
    pub fn to_state(&self) -> Vec<u8> {
        let mut b = default_state();
        for (k, v) in self.v.iter().enumerate() {
            set_field(&mut b, k, *v);
        }
        b
    }
}

impl NoteDelayParams {
    /// Setting `k`'s value for the sound (an editor edit the host has not confirmed yet wins).
    pub fn v(&self, k: usize) -> i32 {
        self.live.value(&self.p[k])
    }

    /// The settings the sound plays: settings whose stored value still means the same keep it.
    pub fn settings(&self, raw: &[u8]) -> Settings {
        Settings {
            v: std::array::from_fn(|k| {
                let v = self.v(k);
                let s = get_field(raw, k);
                if decode(k, s) == v { decode(k, s) } else { v }
            }),
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

    /// The saved state of the current settings.
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

/// Any readable saved state in the current layout; unreadable ones give the start (`START`).
pub fn normalized(b: Vec<u8>) -> Vec<u8> {
    match Settings::from_state(&b) {
        Some(_) if b.len() >= STATE_LEN && b[0] == VERSION => b,
        Some(s) => s.to_state(),
        None => default_state(),
    }
}

/// Setting `k`'s parameter value for a stored field value.
fn decode(k: usize, s: i32) -> i32 {
    s.clamp(DEFS[k].min, DEFS[k].max)
}

/// The saved state with values `vals`: fields whose value still means the same keep their stored bytes.
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

/// The saved state for a plugin state blob; `None` when it is not one.
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
    fn defaults_are_in_range_and_read_back() {
        for (k, d) in DEFS.iter().enumerate() {
            assert!((d.min..=d.max).contains(&START[k]), "{}", d.id);
            assert_eq!(parse_text(k, &value_text(k, START[k])), Some(START[k]), "{}", d.id);
        }
    }

    #[test]
    fn value_texts_match() {
        assert_eq!(value_text(index("division").unwrap(), 9), "1/8");
        assert_eq!(value_text(index("division").unwrap(), 10), "1/8 D");
        assert_eq!(value_text(index("division").unwrap(), 17), "1/32 T");
        assert_eq!(value_text(index("unit").unwrap(), 1), "ms");
        assert_eq!(value_text(index("ms").unwrap(), 250), "250 ms");
        assert_eq!(value_text(index("decay").unwrap(), 60), "60%");
    }

    #[test]
    fn state_round_trips() {
        let mut fl = default_state();
        set_field(&mut fl, 0, DEFS[0].max);
        let blob = plugin_state_from_fl(&fl);
        assert_eq!(fl_from_plugin_state(&blob).expect("parses"), fl);
        let p = NoteDelayParams::default();
        let json: serde_json::Value = serde_json::from_slice(&blob[8..]).unwrap();
        let fields: BTreeMap<String, String> = json["fields"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect();
        p.deserialize_fields(&fields);
        assert_eq!(*p.raw.read().unwrap(), fl);
    }
}
