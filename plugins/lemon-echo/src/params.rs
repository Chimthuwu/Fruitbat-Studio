// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Echo parameters and saved state.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use nih_plug::prelude::*;
use os_plugkit::live::LiveEdits;

/// Echo mode.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Enum, Default)]
pub enum EchoMode {
    #[name = "Normal"]
    #[default]
    Normal = 0,
    #[name = "Ping Pong"]
    PingPong = 1,
    #[name = "Filter"]
    Filter = 2,
}

impl EchoMode {
    pub fn from_index(i: u8) -> Self {
        match i {
            0 => EchoMode::Normal,
            1 => EchoMode::PingPong,
            _ => EchoMode::Filter,
        }
    }
}

/// Settings passed to the DSP.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Settings {
    pub time: u32,
    pub feedback: u16,
    pub mix: u16,
    pub mode: EchoMode,
    pub sync: bool,
    pub filter: u8,
    pub cutoff: u16,
    pub pan: i16,
    pub width: u16,
    pub mod_rate: u16,
    pub mod_depth: u16,
}

/// How a setting's value reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Text {
    Time,
    Percent,
    Signed,
    OnOff,
    EchoMode,
    Filter,
}

/// One setting: id, name, text.
#[derive(Clone, Debug)]
pub struct Def {
    pub id: &'static str,
    pub name: &'static str,
    pub text: Text,
}

pub const DEFS: [Def; 11] = [
    Def { id: "time", name: "Time", text: Text::Time },
    Def { id: "feedback", name: "Feedback", text: Text::Percent },
    Def { id: "mix", name: "Mix", text: Text::Percent },
    Def { id: "mode", name: "Mode", text: Text::EchoMode },
    Def { id: "sync", name: "Sync", text: Text::OnOff },
    Def { id: "filter", name: "Filter", text: Text::Filter },
    Def { id: "cutoff", name: "Cutoff", text: Text::Percent },
    Def { id: "pan", name: "Pan", text: Text::Signed },
    Def { id: "width", name: "Width", text: Text::Percent },
    Def { id: "mod_rate", name: "Mod Rate", text: Text::Percent },
    Def { id: "mod_depth", name: "Mod Depth", text: Text::Percent },
];

/// Factory defaults (stored ints: 0-6000 range).
pub const START: [i32; 11] = [
    1800, // time: 0-6000 (ms)
    3000, // feedback: 0-6000 (50%)
    3000, // mix: 0-6000 (50%)
    0,    // mode: 0=Normal
    0,    // sync: 0=Off
    0,    // filter: 0=Off
    3000, // cutoff: 0-6000 (50%)
    3000, // pan: 0-6000 (center)
    3000, // width: 0-6000 (50%)
    1000, // mod rate: 0-6000
    1000, // mod depth: 0-6000
];

/// Stored ranges for each setting.
pub const RANGES: [(i32, i32); 11] = [
    (0, 6000),   // time
    (0, 6000),   // feedback
    (0, 6000),   // mix
    (0, 2),      // mode
    (0, 1),      // sync
    (0, 2),      // filter
    (0, 6000),   // cutoff
    (0, 6000),   // pan
    (0, 6000),   // width
    (0, 6000),   // mod rate
    (0, 6000),   // mod depth
];

/// The saved state: version + 11 ints.
pub const STATE_LEN: usize = 1 + 11;
pub const STATE_FIELD: &str = "state";
pub const VERSION: u8 = 1;

/// Setting index of an id.
pub fn index(id: &str) -> Option<usize> {
    DEFS.iter().position(|d| d.id == id)
}

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

/// Any readable saved state in the version-1 layout; unreadable ones give the start (`START`).
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
    match DEFS[k].text {
        Text::Time => {
            if v <= 1000 {
                format!("{}ms", v)
            } else {
                format!("{:.1}s", v as f32 / 1000.0)
            }
        }
        Text::EchoMode => match EchoMode::from_index(v as u8) {
            EchoMode::Normal => "Normal".into(),
            EchoMode::PingPong => "Ping Pong".into(),
            EchoMode::Filter => "Filter".into(),
        },
        Text::Filter => match v {
            0 => "Off".into(),
            1 => "Low Pass".into(),
            _ => "High Pass".into(),
        },
        Text::Percent => format!("{:.1}%", x * 100.0),
        Text::Signed => format!("{:+.1}%", (x - 0.5) * 200.0),
        Text::OnOff => if v != 0 { "On" } else { "Off" }.into(),
    }
}

/// A typed value: names pick their setting, a number gives the stored value whose text shows the nearest number.
pub fn parse_text(k: usize, s: &str) -> Option<i32> {
    let s = s.trim().to_ascii_lowercase();
    let (min, max) = RANGES[k];
    match DEFS[k].text {
        Text::EchoMode => ["normal", "ping", "filter"].iter().position(|n| s.starts_with(n)).map(|i| i as i32),
        Text::Filter => ["off", "low", "high"].iter().position(|n| s.starts_with(n)).map(|i| i as i32),
        Text::OnOff => ["off", "on"].iter().position(|n| s == *n).map(|i| i as i32),
        _ => {
            let lead: String = s.chars().take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+')).collect();
            let x: f64 = lead.parse().ok()?;
            (min..=max)
                .filter_map(|v| {
                    let text = value_text(k, v);
                    let n: f64 = text.trim_end_matches(['%', 'm', 's']).parse().ok()?;
                    Some((v, (n - x).abs()))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(v, _)| v)
        }
    }
}

/// Parameter values of a saved state.
pub fn values_of(fl: &[u8]) -> Vec<i32> {
    (0..DEFS.len()).map(|k| decode(k, field(fl, k + 1))).collect()
}

/// The original's saved state with values `vals`: fields whose value still means the same keep their stored bytes.
pub fn to_fl(vals: &[i32], raw: &[u8]) -> Vec<u8> {
    let mut b = normalized(raw.to_vec());
    for (k, &v) in vals.iter().enumerate().take(DEFS.len()) {
        if decode(k, field(&b, k + 1)) != v {
            set_field(&mut b, k + 1, v);
        }
    }
    b
}

pub struct EchoParams {
    /// The settings in the original's order.
    pub p: Vec<IntParam>,
    /// The original's saved state the parameters came from.
    pub raw: RwLock<Vec<u8>>,
    /// Bumped on every `raw` change.
    pub raw_version: AtomicU64,
    /// Editor edits the host has not confirmed yet (the sound hears them at once).
    pub live: Arc<LiveEdits>,
}

impl Default for EchoParams {
    fn default() -> Self {
        let p = DEFS
            .iter()
            .enumerate()
            .map(|(k, d)| {
                let (min, max) = RANGES[k];
                IntParam::new(d.name, START[k], IntRange::Linear { min, max })
                    .with_value_to_string(Arc::new(move |v| value_text(k, v)))
                    .with_string_to_value(Arc::new(move |s| parse_text(k, s)))
            })
            .collect();
        EchoParams { p, raw: RwLock::new(default_state()), raw_version: AtomicU64::new(0), live: Arc::default() }
    }
}

unsafe impl Params for EchoParams {
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
    /// Read settings from saved state (version + 11 ints).
    pub fn from_state(b: &[u8]) -> Option<Settings> {
        if b.len() < 1 + 11 * 4 {
            return None;
        }
        let read = |k: usize| -> i32 {
            let off = 1 + k * 4;
            b.get(off..off + 4).map(|s| i32::from_le_bytes(s.try_into().unwrap())).unwrap_or(0)
        };
        Some(Settings {
            time: read(0) as u32,
            feedback: read(1) as u16,
            mix: read(2) as u16,
            mode: EchoMode::from_index(read(3) as u8),
            sync: read(4) != 0,
            filter: read(5) as u8,
            cutoff: read(6) as u16,
            pan: read(7) as i16,
            width: read(8) as u16,
            mod_rate: read(9) as u16,
            mod_depth: read(10) as u16,
        })
    }

    /// Write settings to saved state (version + 11 ints).
    pub fn to_state(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(1 + 11 * 4);
        v.push(VERSION);
        v.extend_from_slice(&(self.time as i32).to_le_bytes());
        v.extend_from_slice(&(self.feedback as i32).to_le_bytes());
        v.extend_from_slice(&(self.mix as i32).to_le_bytes());
        v.extend_from_slice(&(self.mode as i32).to_le_bytes());
        v.extend_from_slice(&(self.sync as i32).to_le_bytes());
        v.extend_from_slice(&(self.filter as i32).to_le_bytes());
        v.extend_from_slice(&(self.cutoff as i32).to_le_bytes());
        v.extend_from_slice(&(self.pan as i32).to_le_bytes());
        v.extend_from_slice(&(self.width as i32).to_le_bytes());
        v.extend_from_slice(&(self.mod_rate as i32).to_le_bytes());
        v.extend_from_slice(&(self.mod_depth as i32).to_le_bytes());
        v
    }
}

impl EchoParams {
    /// Setting `k`'s value for the sound (an editor edit the host has not confirmed yet wins).
    pub fn v(&self, k: usize) -> i32 {
        self.live.value(&self.p[k])
    }

    /// The settings the sound plays.
    pub fn settings(&self, raw: &[u8]) -> Settings {
        let v: [i32; 11] = std::array::from_fn(|k| {
            let v = self.v(k);
            let s = field(raw, k + 1);
            if decode(k, s) == v { s } else { v }
        });
        Settings {
            time: v[0] as u32,
            feedback: v[1] as u16,
            mix: v[2] as u16,
            mode: EchoMode::from_index(v[3] as u8),
            sync: v[4] != 0,
            filter: v[5] as u8,
            cutoff: v[6] as u16,
            pan: v[7] as i16,
            width: v[8] as u16,
            mod_rate: v[9] as u16,
            mod_depth: v[10] as u16,
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
            ("time", 300, "300ms"),
            ("feedback", 3000, "50.0%"),
            ("mix", 3000, "50.0%"),
            ("mode", 0, "Normal"),
            ("mode", 1, "Ping Pong"),
            ("sync", 1, "On"),
            ("filter", 1, "Low Pass"),
            ("cutoff", 3000, "50.0%"),
            ("pan", 3000, "+0.0%"),
            ("width", 3000, "50.0%"),
        ] {
            assert_eq!(value_text(index(id).unwrap(), v), s, "{id} {v}");
        }
    }

    #[test]
    fn typed_values_find_nearest() {
        let p = |id: &str, s: &str| parse_text(index(id).unwrap(), s);
        assert_eq!(p("mode", "normal"), Some(0));
        assert!(p("feedback", "50").is_some());
    }

    #[test]
    fn state_round_trips() {
        let mut fl = default_state();
        set_field(&mut fl, 4, 1);  // mode = Ping Pong
        set_field(&mut fl, 5, 1);  // sync = On
        let blob = plugin_state_from_fl(&fl);
        assert_eq!(fl_from_plugin_state(&blob).expect("parses"), fl);
        let p = EchoParams::default();
        let json: serde_json::Value = serde_json::from_slice(&blob[8..]).unwrap();
        let fields: BTreeMap<String, String> = json["fields"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect();
        p.deserialize_fields(&fields);
        assert_eq!(*p.raw.read().unwrap(), fl);
    }
}
