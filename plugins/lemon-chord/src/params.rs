// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Chord parameters and saved state.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use nih_plug::prelude::*;
use os_plugkit::live::LiveEdits;

/// Root note.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Enum, Default)]
pub enum Root {
    #[name = "C"]
    #[default]
    C = 0,
    #[name = "C#"]
    Cs = 1,
    #[name = "D"]
    D = 2,
    #[name = "D#"]
    Ds = 3,
    #[name = "E"]
    E = 4,
    #[name = "F"]
    F = 5,
    #[name = "F#"]
    Fs = 6,
    #[name = "G"]
    G = 7,
    #[name = "G#"]
    Gs = 8,
    #[name = "A"]
    A = 9,
    #[name = "A#"]
    As = 10,
    #[name = "B"]
    B = 11,
}

impl Root {
    pub fn from_index(i: u8) -> Self {
        match i {
            0 => Root::C,
            1 => Root::Cs,
            2 => Root::D,
            3 => Root::Ds,
            4 => Root::E,
            5 => Root::F,
            6 => Root::Fs,
            7 => Root::G,
            8 => Root::Gs,
            9 => Root::A,
            10 => Root::As,
            _ => Root::B,
        }
    }

    pub fn semitone(&self) -> u8 {
        *self as u8
    }
}

/// Scale type.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Enum, Default)]
pub enum Scale {
    #[name = "Major"]
    #[default]
    Major = 0,
    #[name = "Minor"]
    Minor = 1,
    #[name = "Dorian"]
    Dorian = 2,
    #[name = "Phrygian"]
    Phrygian = 3,
    #[name = "Lydian"]
    Lydian = 4,
    #[name = "Mixolydian"]
    Mixolydian = 5,
    #[name = "Locrian"]
    Locrian = 6,
    #[name = "Harmonic Minor"]
    HarmonicMinor = 7,
    #[name = "Melodic Minor"]
    MelodicMinor = 8,
}

impl Scale {
    pub fn from_index(i: u8) -> Self {
        match i {
            0 => Scale::Major,
            1 => Scale::Minor,
            2 => Scale::Dorian,
            3 => Scale::Phrygian,
            4 => Scale::Lydian,
            5 => Scale::Mixolydian,
            6 => Scale::Locrian,
            7 => Scale::HarmonicMinor,
            _ => Scale::MelodicMinor,
        }
    }

    pub fn degrees(&self) -> &'static [u8] {
        match self {
            Scale::Major => &[0, 2, 4, 5, 7, 9, 11],
            Scale::Minor => &[0, 2, 3, 5, 7, 8, 10],
            Scale::Dorian => &[0, 2, 3, 5, 7, 9, 10],
            Scale::Phrygian => &[0, 1, 3, 5, 7, 8, 10],
            Scale::Lydian => &[0, 2, 4, 6, 7, 9, 11],
            Scale::Mixolydian => &[0, 2, 4, 5, 7, 9, 10],
            Scale::Locrian => &[0, 1, 3, 5, 6, 8, 10],
            Scale::HarmonicMinor => &[0, 2, 3, 5, 7, 8, 11],
            Scale::MelodicMinor => &[0, 2, 3, 5, 7, 9, 11],
        }
    }
}

/// Chord type.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Enum, Default)]
pub enum ChordType {
    #[name = "Triad"]
    #[default]
    Triad = 0,
    #[name = "7th"]
    Seventh = 1,
    #[name = "9th"]
    Ninth = 2,
    #[name = "11th"]
    Eleventh = 3,
    #[name = "13th"]
    Thirteenth = 4,
    #[name = "Sus2"]
    Sus2 = 5,
    #[name = "Sus4"]
    Sus4 = 6,
}

impl ChordType {
    pub fn from_index(i: u8) -> Self {
        match i {
            0 => ChordType::Triad,
            1 => ChordType::Seventh,
            2 => ChordType::Ninth,
            3 => ChordType::Eleventh,
            4 => ChordType::Thirteenth,
            5 => ChordType::Sus2,
            _ => ChordType::Sus4,
        }
    }

    pub fn degrees(&self) -> &'static [usize] {
        match self {
            ChordType::Triad => &[0, 2, 4],
            ChordType::Seventh => &[0, 2, 4, 6],
            ChordType::Ninth => &[0, 2, 4, 6, 8],
            ChordType::Eleventh => &[0, 2, 4, 6, 10],
            ChordType::Thirteenth => &[0, 2, 4, 6, 12],
            ChordType::Sus2 => &[0, 1, 4],
            ChordType::Sus4 => &[0, 3, 4],
        }
    }
}

/// Inversion.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Enum, Default)]
pub enum Inversion {
    #[name = "Root"]
    #[default]
    Root = 0,
    #[name = "1st"]
    First = 1,
    #[name = "2nd"]
    Second = 2,
    #[name = "3rd"]
    Third = 3,
}

impl Inversion {
    pub fn from_index(i: u8) -> Self {
        match i {
            0 => Inversion::Root,
            1 => Inversion::First,
            2 => Inversion::Second,
            _ => Inversion::Third,
        }
    }
}

/// Voicing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Enum, Default)]
pub enum Voicing {
    #[name = "Close"]
    #[default]
    Close = 0,
    #[name = "Open"]
    Open = 1,
    #[name = "Drop 2"]
    Drop2 = 2,
    #[name = "Drop 3"]
    Drop3 = 3,
}

impl Voicing {
    pub fn from_index(i: u8) -> Self {
        match i {
            0 => Voicing::Close,
            1 => Voicing::Open,
            2 => Voicing::Drop2,
            _ => Voicing::Drop3,
        }
    }
}

/// Strum direction.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Enum, Default)]
pub enum Strum {
    #[name = "Off"]
    #[default]
    Off = 0,
    #[name = "Down"]
    Down = 1,
    #[name = "Up"]
    Up = 2,
    #[name = "Alt"]
    Alternating = 3,
}

impl Strum {
    pub fn from_index(i: u8) -> Self {
        match i {
            0 => Strum::Off,
            1 => Strum::Down,
            2 => Strum::Up,
            _ => Strum::Alternating,
        }
    }
}

/// Settings passed to the DSP.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Settings {
    pub root: Root,
    pub scale: Scale,
    pub chord_type: ChordType,
    pub inversion: Inversion,
    pub voicing: Voicing,
    pub strum: Strum,
    pub strum_speed: u16,
    pub hold: bool,
}

/// How a setting's value reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Text {
    Root,
    Scale,
    ChordType,
    Inversion,
    Voicing,
    Strum,
    Percent,
    OnOff,
}

/// One setting: id, name, text.
#[derive(Clone, Debug)]
pub struct Def {
    pub id: &'static str,
    pub name: &'static str,
    pub text: Text,
}

pub const DEFS: [Def; 8] = [
    Def { id: "root", name: "Root", text: Text::Root },
    Def { id: "scale", name: "Scale", text: Text::Scale },
    Def { id: "chord_type", name: "Chord Type", text: Text::ChordType },
    Def { id: "inversion", name: "Inversion", text: Text::Inversion },
    Def { id: "voicing", name: "Voicing", text: Text::Voicing },
    Def { id: "strum", name: "Strum", text: Text::Strum },
    Def { id: "strum_speed", name: "Strum Speed", text: Text::Percent },
    Def { id: "hold", name: "Hold", text: Text::OnOff },
];

/// Factory defaults (stored ints).
pub const START: [i32; 8] = [
    0,  // root: 0=C
    0,  // scale: 0=Major
    0,  // chord type: 0=Triad
    0,  // inversion: 0=Root
    0,  // voicing: 0=Close
    0,  // strum: 0=Off
    5000, // strum speed: 0-10000 (50%)
    0,  // hold: 0=off
];

/// Stored ranges for each setting.
pub const RANGES: [(i32, i32); 8] = [
    (0, 11),    // root
    (0, 8),     // scale
    (0, 6),     // chord type
    (0, 3),     // inversion
    (0, 3),     // voicing
    (0, 3),     // strum
    (0, 10000), // strum speed
    (0, 1),     // hold
];

/// The saved state: version + 8 ints.
pub const STATE_LEN: usize = 1 + 8;
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
        Text::Root => match Root::from_index(v as u8) {
            Root::C => "C".into(),
            Root::Cs => "C#".into(),
            Root::D => "D".into(),
            Root::Ds => "D#".into(),
            Root::E => "E".into(),
            Root::F => "F".into(),
            Root::Fs => "F#".into(),
            Root::G => "G".into(),
            Root::Gs => "G#".into(),
            Root::A => "A".into(),
            Root::As => "A#".into(),
            Root::B => "B".into(),
        },
        Text::Scale => match Scale::from_index(v as u8) {
            Scale::Major => "Major".into(),
            Scale::Minor => "Minor".into(),
            Scale::Dorian => "Dorian".into(),
            Scale::Phrygian => "Phrygian".into(),
            Scale::Lydian => "Lydian".into(),
            Scale::Mixolydian => "Mixolydian".into(),
            Scale::Locrian => "Locrian".into(),
            Scale::HarmonicMinor => "Harmonic Minor".into(),
            Scale::MelodicMinor => "Melodic Minor".into(),
        },
        Text::ChordType => match ChordType::from_index(v as u8) {
            ChordType::Triad => "Triad".into(),
            ChordType::Seventh => "7th".into(),
            ChordType::Ninth => "9th".into(),
            ChordType::Eleventh => "11th".into(),
            ChordType::Thirteenth => "13th".into(),
            ChordType::Sus2 => "Sus2".into(),
            ChordType::Sus4 => "Sus4".into(),
        },
        Text::Inversion => match Inversion::from_index(v as u8) {
            Inversion::Root => "Root".into(),
            Inversion::First => "1st".into(),
            Inversion::Second => "2nd".into(),
            Inversion::Third => "3rd".into(),
        },
        Text::Voicing => match Voicing::from_index(v as u8) {
            Voicing::Close => "Close".into(),
            Voicing::Open => "Open".into(),
            Voicing::Drop2 => "Drop 2".into(),
            Voicing::Drop3 => "Drop 3".into(),
        },
        Text::Strum => match Strum::from_index(v as u8) {
            Strum::Off => "Off".into(),
            Strum::Down => "Down".into(),
            Strum::Up => "Up".into(),
            Strum::Alternating => "Alt".into(),
        },
        Text::Percent => format!("{:.1}%", x * 100.0),
        Text::OnOff => if v != 0 { "On" } else { "Off" }.into(),
    }
}

/// A typed value: names pick their setting, a number gives the stored value whose text shows the nearest number.
pub fn parse_text(k: usize, s: &str) -> Option<i32> {
    let s = s.trim().to_ascii_lowercase();
    let (min, max) = RANGES[k];
    match DEFS[k].text {
        Text::Root => ["c", "c#", "d", "d#", "e", "f", "f#", "g", "g#", "a", "a#", "b"].iter().position(|n| s.starts_with(n)).map(|i| i as i32),
        Text::Scale => ["major", "minor", "dorian", "phrygian", "lydian", "mixolydian", "locrian", "harmonic minor", "melodic minor"].iter().position(|n| s.starts_with(n)).map(|i| i as i32),
        Text::ChordType => ["triad", "7th", "9th", "11th", "13th", "sus2", "sus4"].iter().position(|n| s.starts_with(n)).map(|i| i as i32),
        Text::Inversion => ["root", "1st", "2nd", "3rd"].iter().position(|n| s.starts_with(n)).map(|i| i as i32),
        Text::Voicing => ["close", "open", "drop 2", "drop 3"].iter().position(|n| s.starts_with(n)).map(|i| i as i32),
        Text::Strum => ["off", "down", "up", "alt"].iter().position(|n| s.starts_with(n)).map(|i| i as i32),
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

pub struct ChordParams {
    /// The settings in the original's order.
    pub p: Vec<IntParam>,
    /// The original's saved state the parameters came from.
    pub raw: RwLock<Vec<u8>>,
    /// Bumped on every `raw` change.
    pub raw_version: AtomicU64,
    /// Editor edits the host has not confirmed yet (the sound hears them at once).
    pub live: Arc<LiveEdits>,
}

impl Default for ChordParams {
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
        ChordParams { p, raw: RwLock::new(default_state()), raw_version: AtomicU64::new(0), live: Arc::default() }
    }
}

unsafe impl Params for ChordParams {
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
    /// Read settings from saved state (version + 8 ints).
    pub fn from_state(b: &[u8]) -> Option<Settings> {
        if b.len() < 1 + 8 * 4 {
            return None;
        }
        let read = |k: usize| -> i32 {
            let off = 1 + k * 4;
            b.get(off..off + 4).map(|s| i32::from_le_bytes(s.try_into().unwrap())).unwrap_or(0)
        };
        Some(Settings {
            root: Root::from_index(read(0) as u8),
            scale: Scale::from_index(read(1) as u8),
            chord_type: ChordType::from_index(read(2) as u8),
            inversion: Inversion::from_index(read(3) as u8),
            voicing: Voicing::from_index(read(4) as u8),
            strum: Strum::from_index(read(5) as u8),
            strum_speed: read(6) as u16,
            hold: read(7) != 0,
        })
    }

    /// Write settings to saved state (version + 8 ints).
    pub fn to_state(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(1 + 8 * 4);
        v.push(VERSION);
        v.extend_from_slice(&(self.root as i32).to_le_bytes());
        v.extend_from_slice(&(self.scale as i32).to_le_bytes());
        v.extend_from_slice(&(self.chord_type as i32).to_le_bytes());
        v.extend_from_slice(&(self.inversion as i32).to_le_bytes());
        v.extend_from_slice(&(self.voicing as i32).to_le_bytes());
        v.extend_from_slice(&(self.strum as i32).to_le_bytes());
        v.extend_from_slice(&(self.strum_speed as i32).to_le_bytes());
        v.extend_from_slice(&(self.hold as i32).to_le_bytes());
        v
    }
}

impl ChordParams {
    /// Setting `k`'s value for the sound (an editor edit the host has not confirmed yet wins).
    pub fn v(&self, k: usize) -> i32 {
        self.live.value(&self.p[k])
    }

    /// The settings the sound plays.
    pub fn settings(&self, raw: &[u8]) -> Settings {
        let v: [i32; 8] = std::array::from_fn(|k| {
            let v = self.v(k);
            let s = field(raw, k + 1);
            if decode(k, s) == v { s } else { v }
        });
        Settings {
            root: Root::from_index(v[0] as u8),
            scale: Scale::from_index(v[1] as u8),
            chord_type: ChordType::from_index(v[2] as u8),
            inversion: Inversion::from_index(v[3] as u8),
            voicing: Voicing::from_index(v[4] as u8),
            strum: Strum::from_index(v[5] as u8),
            strum_speed: v[6] as u16,
            hold: v[7] != 0,
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
            ("root", 0, "C"),
            ("root", 7, "G"),
            ("scale", 0, "Major"),
            ("scale", 1, "Minor"),
            ("chord_type", 0, "Triad"),
            ("chord_type", 1, "7th"),
            ("inversion", 0, "Root"),
            ("voicing", 0, "Close"),
            ("strum", 0, "Off"),
            ("strum", 1, "Down"),
            ("strum_speed", 5000, "50.0%"),
            ("hold", 1, "On"),
        ] {
            assert_eq!(value_text(index(id).unwrap(), v), s, "{id} {v}");
        }
    }

    #[test]
    fn typed_values_find_nearest() {
        let p = |id: &str, s: &str| parse_text(index(id).unwrap(), s);
        assert_eq!(p("root", "c"), Some(0));
        assert_eq!(p("scale", "major"), Some(0));
        assert_eq!(p("chord_type", "triad"), Some(0));
        assert!(p("strum_speed", "50").is_some());
    }

    #[test]
    fn state_round_trips() {
        let mut fl = default_state();
        set_field(&mut fl, 1, 7);  // root = G
        set_field(&mut fl, 2, 1);  // scale = Minor
        let blob = plugin_state_from_fl(&fl);
        assert_eq!(fl_from_plugin_state(&blob).expect("parses"), fl);
        let p = ChordParams::default();
        let json: serde_json::Value = serde_json::from_slice(&blob[8..]).unwrap();
        let fields: BTreeMap<String, String> = json["fields"].as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect();
        p.deserialize_fields(&fields);
        assert_eq!(*p.raw.read().unwrap(), fl);
    }
}
