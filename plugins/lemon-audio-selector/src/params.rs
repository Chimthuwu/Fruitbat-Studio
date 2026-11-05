// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Audio Selector parameters and saved state.
//!
//! 14 params: `mode` (0=Manual, 1=Note), `manual` (0=Bypass,1=Filter,2=Delay,3=Chorus,4=Drive),
//! plus 4 slots (Filter/Delay/Chorus/Drive) x (active, level, param). Slot type is fixed per slot
//! index (slot 1=Filter, 2=Delay, 3=Chorus, 4=Drive), so `type` is represented by the slot index
//! itself and not counted as a separate param.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use nih_plug::prelude::*;
use os_plugkit::live::LiveEdits;

/// Selection mode.
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Enum)]
pub enum SelMode {
    #[name = "Manual"]
    #[default]
    Manual = 0,
    #[name = "Note"]
    Note = 1,
}

impl SelMode {
    pub fn from_index(i: i32) -> Self {
        match i {
            1 => SelMode::Note,
            _ => SelMode::Manual,
        }
    }
}

/// Slot kind: 0=Bypass 1=Filter 2=Delay 3=Chorus 4=Drive.
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SlotKind {
    Bypass,
    Filter,
    Delay,
    Chorus,
    Drive,
}

impl Default for SlotKind {
    fn default() -> Self { SlotKind::Bypass }
}

impl SlotKind {
    pub fn from_index(i: i32) -> Self {
        match i.clamp(0, 4) {
            0 => SlotKind::Bypass,
            1 => SlotKind::Filter,
            2 => SlotKind::Delay,
            3 => SlotKind::Chorus,
            _ => SlotKind::Drive,
        }
    }
    pub fn name(&self) -> &'static str {
        match self {
            SlotKind::Bypass => "Bypass",
            SlotKind::Delay => "Delay",
            SlotKind::Chorus => "Chorus",
            SlotKind::Drive => "Drive",
            SlotKind::Filter => "Filter",
        }
    }
}

/// One slot's stored values.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct SlotSettings {
    pub active: bool,
    pub level: i32, // 0..6000
    pub param: i32, // 0..6000
}

/// Settings passed to the DSP.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Settings {
    pub mode: SelMode,
    pub manual: SlotKind,
    pub slots: [SlotSettings; 4],
}

impl Settings {
    fn slot_kind(&self, slot: usize) -> SlotKind {
        // Slot index 0..3 maps to Filter/Delay/Chorus/Drive, Active flag gates it.
        SlotKind::from_index((slot + 1) as i32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Text {
    Mode,
    Slot,
    OnOff,
    Percent,
    Param,
}

/// One setting: id, name, text.
#[derive(Clone, Debug)]
pub struct Def {
    pub id: &'static str,
    pub name: &'static str,
    pub text: Text,
}

pub const DEFS: [Def; 14] = [
    Def { id: "mode", name: "Selection mode", text: Text::Mode },
    Def { id: "manual", name: "Manual slot", text: Text::Slot },
    Def { id: "s1_active", name: "Slot 1 Filter active", text: Text::OnOff },
    Def { id: "s1_level", name: "Slot 1 Filter level", text: Text::Percent },
    Def { id: "s1_param", name: "Slot 1 Filter param", text: Text::Param },
    Def { id: "s2_active", name: "Slot 2 Delay active", text: Text::OnOff },
    Def { id: "s2_level", name: "Slot 2 Delay level", text: Text::Percent },
    Def { id: "s2_param", name: "Slot 2 Delay param", text: Text::Param },
    Def { id: "s3_active", name: "Slot 3 Chorus active", text: Text::OnOff },
    Def { id: "s3_level", name: "Slot 3 Chorus level", text: Text::Percent },
    Def { id: "s3_param", name: "Slot 3 Chorus param", text: Text::Param },
    Def { id: "s4_active", name: "Slot 4 Drive active", text: Text::OnOff },
    Def { id: "s4_level", name: "Slot 4 Drive level", text: Text::Percent },
    Def { id: "s4_param", name: "Slot 4 Drive param", text: Text::Param },
];

pub const START: [i32; 14] = [
    0,    // mode Manual
    0,    // manual Bypass
    1, 3000, 3000, // s1 Filter on
    1, 3000, 3000, // s2 Delay on
    1, 3000, 3000, // s3 Chorus on
    1, 3000, 3000, // s4 Drive on
];

pub const RANGES: [(i32, i32); 14] = [
    (0, 1),     // mode
    (0, 4),     // manual
    (0, 1), (0, 6000), (0, 6000),
    (0, 1), (0, 6000), (0, 6000),
    (0, 1), (0, 6000), (0, 6000),
    (0, 1), (0, 6000), (0, 6000),
];

pub const STATE_LEN: usize = 1 + 14;
pub const STATE_FIELD: &str = "state";
pub const VERSION: u8 = 1;

pub fn index(id: &str) -> Option<usize> {
    DEFS.iter().position(|d| d.id == id)
}

pub fn default_state() -> Vec<u8> {
    let mut v = Vec::with_capacity(STATE_LEN * 4);
    v.push(VERSION);
    for s in START {
        v.extend_from_slice(&s.to_le_bytes());
    }
    v
}

fn field(b: &[u8], k: usize) -> i32 {
    let off = 1 + k * 4;
    b.get(off..off + 4)
        .map_or(0, |s| i32::from_le_bytes(s.try_into().unwrap()))
}

fn set_field(b: &mut [u8], k: usize, val: i32) {
    let off = 1 + k * 4;
    if let Some(s) = b.get_mut(off..off + 4) {
        s.copy_from_slice(&val.to_le_bytes());
    }
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    (s.len() % 2 == 0).then_some(())?;
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn normalized(b: Vec<u8>) -> Vec<u8> {
    match Settings::from_state(&b) {
        Some(_) if b.len() >= STATE_LEN * 4 && !b.is_empty() && b[0] == VERSION => b,
        Some(s) => s.to_state(),
        None => default_state(),
    }
}

fn decode(k: usize, s: i32) -> i32 {
    s.clamp(RANGES[k].0, RANGES[k].1)
}

pub fn value_text(k: usize, v: i32) -> String {
    match DEFS[k].text {
        Text::Mode => match SelMode::from_index(v) {
            SelMode::Manual => "Manual".into(),
            SelMode::Note => "Note".into(),
        },
        Text::Slot => SlotKind::from_index(v).name().into(),
        Text::OnOff => if v != 0 { "On" } else { "Off" }.into(),
        Text::Percent => format!("{:.1}%", v as f32 / 60.0),
        Text::Param => format!("{:.1}%", v as f32 / 60.0),
    }
}

pub fn parse_text(k: usize, s: &str) -> Option<i32> {
    let s = s.trim().to_ascii_lowercase();
    let (min, max) = RANGES[k];
    match DEFS[k].text {
        Text::Mode => ["manual", "note"].iter().position(|n| s.starts_with(n)).map(|i| i as i32),
        Text::Slot => ["bypass", "filter", "delay", "chorus", "drive"].iter().position(|n| s.starts_with(n)).map(|i| i as i32),
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

pub fn values_of(fl: &[u8]) -> Vec<i32> {
    (0..DEFS.len()).map(|k| decode(k, field(fl, k + 1))).collect()
}

pub fn to_fl(vals: &[i32], raw: &[u8]) -> Vec<u8> {
    let mut b = normalized(raw.to_vec());
    for (k, &v) in vals.iter().enumerate().take(DEFS.len()) {
        if decode(k, field(&b, k + 1)) != v {
            set_field(&mut b, k + 1, v);
        }
    }
    b
}

pub struct SelectorParams {
    pub p: Vec<IntParam>,
    pub raw: RwLock<Vec<u8>>,
    pub raw_version: AtomicU64,
    pub live: Arc<LiveEdits>,
}

impl Default for SelectorParams {
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
    pub fn from_state(b: &[u8]) -> Option<Settings> {
        if b.len() < 1 + 14 * 4 {
            return None;
        }
        let read = |k: usize| -> i32 {
            let off = 1 + k * 4;
            b.get(off..off + 4).map(|s| i32::from_le_bytes(s.try_into().unwrap())).unwrap_or(0)
        };
        let slots = std::array::from_fn(|i| {
            let base = 2 + i * 3;
            SlotSettings { active: read(base) != 0, level: read(base + 1), param: read(base + 2) }
        });
        Some(Settings {
            mode: SelMode::from_index(read(0)),
            manual: SlotKind::from_index(read(1)),
            slots,
        })
    }

    pub fn to_state(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(1 + 14 * 4);
        v.push(VERSION);
        v.extend_from_slice(&(self.mode as i32).to_le_bytes());
        v.extend_from_slice(&(self.manual as i32).to_le_bytes());
        for s in &self.slots {
            v.extend_from_slice(&(s.active as i32).to_le_bytes());
            v.extend_from_slice(&s.level.to_le_bytes());
            v.extend_from_slice(&s.param.to_le_bytes());
        }
        v
    }

    /// The kind of each slot index (fixed type: Filter, Delay, Chorus, Drive).
    pub fn kind_for_slot(&self, slot: usize) -> SlotKind {
        self.slot_kind(slot)
    }
}

impl SelectorParams {
    pub fn v(&self, k: usize) -> i32 {
        self.live.value(&self.p[k])
    }

    pub fn settings(&self, raw: &[u8]) -> Settings {
        let v: [i32; 14] = std::array::from_fn(|k| {
            let val = self.v(k);
            let s = field(raw, k + 1);
            if decode(k, s) == val { s } else { val }
        });
        let slots = std::array::from_fn(|i| {
            let base = 2 + i * 3;
            SlotSettings { active: v[base] != 0, level: v[base + 1], param: v[base + 2] }
        });
        Settings { mode: SelMode::from_index(v[0]), manual: SlotKind::from_index(v[1]), slots }
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

    pub fn fl_state(&self) -> Vec<u8> {
        let vals: Vec<i32> = self.p.iter().map(|p| p.value()).collect();
        to_fl(&vals, &self.raw_copy())
    }
}

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
    fn state_round_trips() {
        let mut fl = default_state();
        set_field(&mut fl, 1, 3); // manual = Chorus
        set_field(&mut fl, 3, 4500);
        let s = Settings::from_state(&fl).expect("parse");
        assert_eq!(s.manual, SlotKind::Chorus);
        assert_eq!(s.slots[0].level, 4500);
        assert_eq!(s.to_state(), fl);
        let blob = plugin_state_from_fl(&fl);
        assert_eq!(fl_from_plugin_state(&blob).expect("parses"), fl);
    }

    #[test]
    fn value_texts_match() {
        for (id, v, s) in [
            ("mode", 0, "Manual"),
            ("mode", 1, "Note"),
            ("manual", 2, "Delay"),
            ("s1_active", 1, "On"),
            ("s1_level", 3000, "50.0%"),
        ] {
            assert_eq!(value_text(index(id).unwrap(), v), s, "{id}");
        }
    }
}
