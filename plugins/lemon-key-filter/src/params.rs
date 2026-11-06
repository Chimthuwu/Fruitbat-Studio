// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Key Filter parameters and saved state.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use nih_plug::prelude::*;
use os_plugkit::live::LiveEdits;

/// Scale choices.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Scale {
    #[default]
    Chromatic = 0,
    Major = 1,
    Minor = 2,
    Dorian = 3,
    Phrygian = 4,
    Lydian = 5,
    Mixolydian = 6,
    Locrian = 7,
    HarmonicMinor = 8,
    MelodicMinor = 9,
    PentatonicMajor = 10,
    PentatonicMinor = 11,
    Blues = 12,
    WholeTone = 13,
}

impl Scale {
    pub fn intervals(self) -> &'static [u8] {
        match self {
            Scale::Chromatic => &[0,1,2,3,4,5,6,7,8,9,10,11],
            Scale::Major => &[0,2,4,5,7,9,11],
            Scale::Minor => &[0,2,3,5,7,8,10],
            Scale::Dorian => &[0,2,3,5,7,9,10],
            Scale::Phrygian => &[0,1,3,5,7,8,10],
            Scale::Lydian => &[0,2,4,6,7,9,11],
            Scale::Mixolydian => &[0,2,4,5,7,9,10],
            Scale::Locrian => &[0,1,3,5,6,8,10],
            Scale::HarmonicMinor => &[0,2,3,5,7,8,11],
            Scale::MelodicMinor => &[0,2,3,5,7,9,11],
            Scale::PentatonicMajor => &[0,2,4,7,9],
            Scale::PentatonicMinor => &[0,3,5,7,10],
            Scale::Blues => &[0,3,5,6,7,10],
            Scale::WholeTone => &[0,2,4,6,8,10],
        }
    }

    pub fn from_index(i: i32) -> Self {
        match i {
            0 => Scale::Chromatic,
            1 => Scale::Major,
            2 => Scale::Minor,
            3 => Scale::Dorian,
            4 => Scale::Phrygian,
            5 => Scale::Lydian,
            6 => Scale::Mixolydian,
            7 => Scale::Locrian,
            8 => Scale::HarmonicMinor,
            9 => Scale::MelodicMinor,
            10 => Scale::PentatonicMajor,
            11 => Scale::PentatonicMinor,
            12 => Scale::Blues,
            _ => Scale::WholeTone,
        }
    }
}

/// What happens to notes outside the scale.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    #[default]
    Drop = 0,
    Pass = 1,
}

impl Mode {
    pub fn from_index(i: i32) -> Self {
        match i {
            0 => Mode::Drop,
            _ => Mode::Pass,
        }
    }
}


/// Settings passed to the DSP.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub root: u8,
    pub scale: Scale,
    pub mode: Mode,
}

impl Settings {
    /// Settings of stored values (clamped to their ranges).
    pub fn from_vals(v: &[i32; 3]) -> Settings {
        let v: [i32; 3] = std::array::from_fn(|k| decode(k, v[k]));
        Settings { root: v[0] as u8, scale: Scale::from_index(v[1]), mode: Mode::from_index(v[2]) }
    }

    /// The stored values.
    pub fn vals(&self) -> [i32; 3] {
        [self.root as i32, self.scale as i32, self.mode as i32]
    }

    /// Read settings from saved state (version + 3 ints).
    pub fn from_state(b: &[u8]) -> Option<Settings> {
        if b.len() < STATE_LEN {
            return None;
        }
        Some(Settings::from_vals(&std::array::from_fn(|k| field(b, k))))
    }

    /// Write settings to saved state.
    pub fn to_state(&self) -> Vec<u8> {
        let mut v = vec![VERSION];
        for x in self.vals() {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings::from_vals(&START)
    }
}

/// One setting: id, name, and how its value reads (a name per value, or a number with a suffix).
#[derive(Clone, Debug)]
pub struct Def {
    pub id: &'static str,
    pub name: &'static str,
    pub names: &'static [&'static str],
    pub suffix: &'static str,
}

pub const DEFS: [Def; 3] = [
    Def { id: "root", name: "Root", names: &["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"], suffix: "" },
    Def { id: "scale", name: "Scale", names: &["Chromatic", "Major", "Minor", "Dorian", "Phrygian", "Lydian", "Mixolydian", "Locrian", "Harmonic Minor", "Melodic Minor", "Pentatonic Major", "Pentatonic Minor", "Blues", "Whole Tone"], suffix: "" },
    Def { id: "mode", name: "Mode", names: &["Drop", "Pass"], suffix: "" },
];

/// Factory defaults.
pub const START: [i32; 3] = [0, 1, 0];

/// Stored ranges for each setting.
pub const RANGES: [(i32, i32); 3] = [(0, 11), (0, 13), (0, 1)];

/// The saved state: version + 3 ints.
pub const STATE_LEN: usize = 1 + 3 * 4;
pub const STATE_FIELD: &str = "state";
pub const VERSION: u8 = 1;

/// Setting index of an id.
pub fn index(id: &str) -> Option<usize> {
    DEFS.iter().position(|d| d.id == id)
}

/// A fresh default state.
pub fn default_state() -> Vec<u8> {
    Settings::default().to_state()
}

/// Field `k` of a stored state (0 when short).
fn field(b: &[u8], k: usize) -> i32 {
    let off = 1 + k * 4;
    b.get(off..off + 4).map_or(0, |s| i32::from_le_bytes(s.try_into().unwrap()))
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    (s.len() % 2 == 0).then_some(())?;
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok()).collect()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Any readable saved state in the version-1 layout; unreadable ones give the start.
pub fn normalized(b: Vec<u8>) -> Vec<u8> {
    match Settings::from_state(&b) {
        Some(_) if b.len() == STATE_LEN && b[0] == VERSION => b,
        Some(s) => s.to_state(),
        None => default_state(),
    }
}

/// Setting `k`'s value clamped to its range.
fn decode(k: usize, s: i32) -> i32 {
    s.clamp(RANGES[k].0, RANGES[k].1)
}

/// A setting's value text.
pub fn value_text(k: usize, v: i32) -> String {
    let d = &DEFS[k];
    if d.names.is_empty() {
        format!("{v}{}", d.suffix)
    } else {
        d.names.get((v - RANGES[k].0).max(0) as usize).copied().unwrap_or("?").to_string()
    }
}

/// A typed value: a name picks its value, a number is clamped to the range.
pub fn parse_text(k: usize, s: &str) -> Option<i32> {
    let s = s.trim().to_ascii_lowercase();
    let d = &DEFS[k];
    if !d.names.is_empty() {
        let by_name = d
            .names
            .iter()
            .position(|n| n.to_ascii_lowercase() == s)
            .or_else(|| d.names.iter().position(|n| n.to_ascii_lowercase().starts_with(&s)));
        return by_name.map(|i| RANGES[k].0 + i as i32);
    }
    let lead: String = s.chars().take_while(|c| c.is_ascii_digit() || matches!(c, '-' | '+')).collect();
    lead.parse::<i32>().ok().map(|x| decode(k, x))
}

/// Parameter values of a saved state.
pub fn values_of(fl: &[u8]) -> Vec<i32> {
    (0..DEFS.len()).map(|k| decode(k, field(fl, k))).collect()
}

/// The saved state with values `vals` (missing ones keep `raw`'s).
pub fn to_fl(vals: &[i32], raw: &[u8]) -> Vec<u8> {
    let b = normalized(raw.to_vec());
    let mut v: [i32; 3] = std::array::from_fn(|k| field(&b, k));
    for (k, &x) in vals.iter().enumerate().take(3) {
        v[k] = x;
    }
    Settings::from_vals(&v).to_state()
}

pub struct KeyFilterParams {
    /// The settings in order.
    pub p: Vec<IntParam>,
    /// The saved state the parameters came from.
    pub raw: RwLock<Vec<u8>>,
    /// Bumped on every `raw` change.
    pub raw_version: AtomicU64,
    /// Editor edits the host has not confirmed yet (the sound hears them at once).
    pub live: Arc<LiveEdits>,
}

impl Default for KeyFilterParams {
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
        KeyFilterParams { p, raw: RwLock::new(default_state()), raw_version: AtomicU64::new(0), live: Arc::default() }
    }
}

unsafe impl Params for KeyFilterParams {
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

impl KeyFilterParams {
    /// Setting `k`'s value for the sound (an editor edit the host has not confirmed yet wins).
    pub fn v(&self, k: usize) -> i32 {
        self.live.value(&self.p[k])
    }

    /// The settings the sound plays.
    pub fn settings(&self, raw: &[u8]) -> Settings {
        let v: [i32; 3] = std::array::from_fn(|k| {
            let v = self.v(k);
            let s = field(raw, k);
            if decode(k, s) == v { s } else { v }
        });
        Settings::from_vals(&v)
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
    fn typed_values_parse() {
        for (k, d) in DEFS.iter().enumerate() {
            for v in RANGES[k].0..=RANGES[k].1 {
                assert_eq!(parse_text(k, &value_text(k, v)), Some(v), "{} {v}", d.id);
            }
        }
    }

    #[test]
    fn defaults_are_in_range() {
        for k in 0..DEFS.len() {
            assert!((RANGES[k].0..=RANGES[k].1).contains(&START[k]), "{}", DEFS[k].id);
        }
        assert_eq!(Settings::from_state(&default_state()), Some(Settings::default()));
    }

    #[test]
    fn state_round_trips() {
        let mut v = values_of(&default_state());
        v[0] = RANGES[0].1;
        let fl = to_fl(&v, &default_state());
        let blob = plugin_state_from_fl(&fl);
        assert_eq!(fl_from_plugin_state(&blob).expect("parses"), fl);
        let p = KeyFilterParams::default();
        let json: serde_json::Value = serde_json::from_slice(&blob[8..]).unwrap();
        let fields: BTreeMap<String, String> =
            json["fields"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect();
        p.deserialize_fields(&fields);
        assert_eq!(*p.raw.read().unwrap(), fl);
        assert_eq!(normalized(vec![9, 9]), default_state());
    }
}
