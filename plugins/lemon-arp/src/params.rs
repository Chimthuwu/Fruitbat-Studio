// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Arp parameter definitions and saved-state encoding.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use nih_plug::prelude::*;
use os_plugkit::live::LiveEdits;

// Types defined locally below

/// Parameter indices matching `Settings` order.
#[allow(dead_code)]
pub mod idx {
    pub const PATTERN: usize = 0;
    pub const OCTAVE: usize = 1;
    pub const RATE: usize = 2;
    pub const GATE: usize = 3;
    pub const SWING: usize = 4;
    pub const HOLD: usize = 5;
    pub const LATCH: usize = 6;
}

/// Factory defaults (knob positions / stored ints).
pub const START: [i32; 7] = [
    0,  // pattern: 0 = up, 1 = down, 2 = up/down, 3 = random
    2,  // octave: 0-4, default 2
    3,  // rate: 0=1/4, 1=1/8, 2=1/16, 3=1/32, 4=1/64, default 3 (1/32)
    50, // gate: 0-100%, default 50%
    0,  // swing: -50 to +50, default 0
    0,  // hold: 0=off, 1=on
    0,  // latch: 0=off, 1=on
];

/// The saved state: version + 7 ints + 0 tail bytes.
pub const STATE_LEN: usize = 1 + 7;
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

/// Arpeggiator pattern types.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Enum, Default)]
pub enum Pattern {
    #[name = "Up"]
    #[default]
    Up = 0,
    #[name = "Down"]
    Down = 1,
    #[name = "Up/Down"]
    UpDown = 2,
    #[name = "Random"]
    Random = 3,
}

impl Pattern {
    pub fn from_index(i: u8) -> Self {
        match i {
            0 => Pattern::Up,
            1 => Pattern::Down,
            2 => Pattern::UpDown,
            _ => Pattern::Random,
        }
    }
}

/// Arpeggiator rate (synced to tempo).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Enum, Default)]
pub enum Rate {
    #[name = "1/4"]
    #[default]
    Div4 = 0,
    #[name = "1/8"]
    Div8 = 1,
    #[name = "1/16"]
    Div16 = 2,
    #[name = "1/32"]
    Div32 = 3,
    #[name = "1/64"]
    Div64 = 4,
}

impl Rate {
    pub fn from_index(i: u8) -> Self {
        match i {
            0 => Rate::Div4,
            1 => Rate::Div8,
            2 => Rate::Div16,
            3 => Rate::Div32,
            _ => Rate::Div64,
        }
    }

    pub fn ticks_per_beat(&self) -> u32 {
        match self {
            Rate::Div4 => 960,
            Rate::Div8 => 480,
            Rate::Div16 => 240,
            Rate::Div32 => 120,
            Rate::Div64 => 60,
        }
    }
}

/// Settings passed to the DSP.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Settings {
    pub pattern: Pattern,
    pub octave: u8,
    pub rate: Rate,
    pub gate: u8,
    pub swing: i8,
    pub hold: bool,
    pub latch: bool,
}

/// How a setting's value reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Text {
    Percent,
    Signed,
    OnOff,
    Pattern,
    Rate,
    Octave,
}

/// One setting: id, name, text.
#[derive(Clone, Debug)]
pub struct Def {
    pub id: &'static str,
    pub name: &'static str,
    pub text: Text,
}

pub const DEFS: [Def; 7] = [
    Def { id: "pattern", name: "Pattern", text: Text::Pattern },
    Def { id: "octave", name: "Octave", text: Text::Octave },
    Def { id: "rate", name: "Rate", text: Text::Rate },
    Def { id: "gate", name: "Gate", text: Text::Percent },
    Def { id: "swing", name: "Swing", text: Text::Signed },
    Def { id: "hold", name: "Hold", text: Text::OnOff },
    Def { id: "latch", name: "Latch", text: Text::OnOff },
];

pub fn index(id: &str) -> Option<usize> {
    DEFS.iter().position(|d| d.id == id)
}

/// A setting's value text.
pub fn value_text(k: usize, v: i32) -> String {
    let x = v as f32 / (DEFS[k].text.range_max() - DEFS[k].text.range_min()) as f32;
    match DEFS[k].text {
        Text::Pattern => match Pattern::from_index(v as u8) {
            Pattern::Up => "Up".into(),
            Pattern::Down => "Down".into(),
            Pattern::UpDown => "Up/Down".into(),
            Pattern::Random => "Random".into(),
        },
        Text::Rate => match Rate::from_index(v as u8) {
            Rate::Div4 => "1/4".into(),
            Rate::Div8 => "1/8".into(),
            Rate::Div16 => "1/16".into(),
            Rate::Div32 => "1/32".into(),
            Rate::Div64 => "1/64".into(),
        },
        Text::Octave => format!("{}", v),
        Text::Percent => format!("{:.1}%", x * 100.0),
        Text::Signed => format!("{:+.1}%", (v as f32 / 50.0) * 100.0),
        Text::OnOff => if v != 0 { "On" } else { "Off" }.into(),
    }
}

impl Text {
    fn range_min(&self) -> i32 {
        match self {
            Text::Pattern => 0,
            Text::Rate => 0,
            Text::Octave => 0,
            Text::Percent => 0,
            Text::Signed => -50,
            Text::OnOff => 0,
        }
    }
    fn range_max(&self) -> i32 {
        match self {
            Text::Pattern => 3,
            Text::Rate => 4,
            Text::Octave => 4,
            Text::Percent => 100,
            Text::Signed => 50,
            Text::OnOff => 1,
        }
    }
}

/// A typed value: names pick their setting, a number gives the stored value.
pub fn parse_text(k: usize, s: &str) -> Option<i32> {
    let s = s.trim().to_ascii_lowercase();
    let (min, max) = (DEFS[k].text.range_min(), DEFS[k].text.range_max());
    match DEFS[k].text {
        Text::Pattern => ["up", "down", "up/down", "random"].iter().position(|n| s.starts_with(n)).map(|i| i as i32),
        Text::Rate => ["1/4", "1/8", "1/16", "1/32", "1/64"].iter().position(|n| s.starts_with(n)).map(|i| i as i32),
        Text::OnOff => ["off", "on"].iter().position(|n| s == *n).map(|i| i as i32),
        _ => {
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

pub struct ArpParams {
    /// The settings in the original's order.
    pub p: Vec<IntParam>,
    /// The raw saved state the parameters came from.
    pub raw: RwLock<Vec<u8>>,
    /// Bumped on every `raw` change.
    pub raw_version: AtomicU64,
    /// Editor edits the host has not confirmed yet (the sound hears them at once).
    pub live: Arc<LiveEdits>,
}

impl Default for ArpParams {
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
        ArpParams { p, raw: RwLock::new(default_state()), raw_version: AtomicU64::new(0), live: Arc::default() }
    }
}

unsafe impl Params for ArpParams {
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
    /// Read settings from saved state (version + 7 ints).
    pub fn from_state(b: &[u8]) -> Option<Settings> {
        if b.len() < 1 + 7 * 4 {
            return None;
        }
        let read = |k: usize| -> i32 {
            let off = 1 + k * 4;
            b.get(off..off + 4).map(|s| i32::from_le_bytes(s.try_into().unwrap())).unwrap_or(0)
        };
        Some(Settings {
            pattern: Pattern::from_index(read(0) as u8),
            octave: read(1) as u8,
            rate: Rate::from_index(read(2) as u8),
            gate: read(3) as u8,
            swing: read(4) as i8,
            hold: read(5) != 0,
            latch: read(6) != 0,
        })
    }

    /// Write settings to saved state (version + 7 ints).
    pub fn to_state(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(1 + 7 * 4);
        v.push(VERSION);
        v.extend_from_slice(&(self.pattern as i32).to_le_bytes());
        v.extend_from_slice(&(self.octave as i32).to_le_bytes());
        v.extend_from_slice(&(self.rate as i32).to_le_bytes());
        v.extend_from_slice(&(self.gate as i32).to_le_bytes());
        v.extend_from_slice(&(self.swing as i32).to_le_bytes());
        v.extend_from_slice(&(self.hold as i32).to_le_bytes());
        v.extend_from_slice(&(self.latch as i32).to_le_bytes());
        v
    }
}

impl ArpParams {
    /// Setting `k`'s value for the sound (an editor edit the host has not confirmed yet wins).
    pub fn v(&self, k: usize) -> i32 {
        self.live.value(&self.p[k])
    }

    /// The settings the sound plays: settings whose stored value still means the same keep it.
    pub fn settings(&self, raw: &[u8]) -> Settings {
        let v: [i32; 7] = std::array::from_fn(|k| {
            let v = self.v(k);
            let s = get_field(raw, k);
            if decode(k, s) == v { s } else { v }
        });
        Settings {
            pattern: Pattern::from_index(v[idx::PATTERN] as u8),
            octave: v[idx::OCTAVE] as u8,
            rate: Rate::from_index(v[idx::RATE] as u8),
            gate: v[idx::GATE] as u8,
            swing: v[idx::SWING] as i8,
            hold: v[idx::HOLD] != 0,
            latch: v[idx::LATCH] != 0,
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
            ("pattern", 0, "Up"),
            ("pattern", 2, "Up/Down"),
            ("rate", 3, "1/32"),
            ("octave", 2, "2"),
            ("gate", 50, "50.0%"),
            ("swing", 0, "+0.0%"),
            ("hold", 1, "On"),
            ("latch", 0, "Off"),
        ] {
            assert_eq!(value_text(index(id).unwrap(), v), s, "{id} {v}");
        }
    }

    #[test]
    fn typed_values_find_nearest() {
        let p = |id: &str, s: &str| parse_text(index(id).unwrap(), s);
        assert_eq!(p("pattern", "up"), Some(0));
        assert_eq!(p("rate", "1/16"), Some(2));
        assert_eq!(p("hold", "on"), Some(1));
        assert_eq!(p("gate", "50"), Some(50));
    }

    #[test]
    fn state_round_trips() {
        let mut fl = default_state();
        set_field(&mut fl, idx::PATTERN, 2);
        set_field(&mut fl, idx::RATE, 1);
        let blob = plugin_state_from_fl(&fl);
        assert_eq!(fl_from_plugin_state(&blob).expect("parses"), fl);
        let p = ArpParams::default();
        let json: serde_json::Value = serde_json::from_slice(&blob[8..]).unwrap();
        let fields: BTreeMap<String, String> = json["fields"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect();
        p.deserialize_fields(&fields);
        assert_eq!(*p.raw.read().unwrap(), fl);
    }
}