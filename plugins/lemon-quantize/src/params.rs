// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Quantize parameters.

use std::sync::Arc;

use nih_plug::prelude::*;
use os_plugkit::live::LiveEdits;

/// How a setting's value reads.
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Semitones,
    Octaves,
    Percent,
    OnOff,
    /// One name per value (the first is the range's minimum).
    Names(&'static [&'static str]),
}

/// One setting: id, name, range, factory value, text.
#[derive(Clone, Debug)]
pub struct Def {
    pub id: &'static str,
    pub name: &'static str,
    pub min: i32,
    pub max: i32,
    pub start: i32,
    pub kind: Kind,
}

pub const DEFS: [Def; 2] = [
    Def { id: "division", name: "Division", min: 0, max: 10, start: 4, kind: Kind::Names(&["1/1", "1/2", "1/4", "1/8", "1/16", "1/32", "1/2T", "1/4T", "1/8T", "1/16T", "1/32T"]) },
    Def { id: "strength", name: "Strength", min: 0, max: 100, start: 100, kind: Kind::Percent },
];

/// Setting index of an id.
pub fn index(id: &str) -> Option<usize> {
    DEFS.iter().position(|d| d.id == id)
}

/// A setting's value text.
pub fn value_text(k: usize, v: i32) -> String {
    match DEFS[k].kind {
        Kind::Semitones => format!("{v:+} st"),
        Kind::Octaves => format!("{v:+} oct"),
        Kind::Percent => format!("{v}%"),
        Kind::OnOff => if v != 0 { "On" } else { "Off" }.into(),
        Kind::Names(n) => n.get((v - DEFS[k].min).max(0) as usize).copied().unwrap_or("").to_string(),
    }
}

/// A typed value: a name picks its setting, a number is taken as given (clamped to the range).
pub fn parse_text(k: usize, s: &str) -> Option<i32> {
    let s = s.trim().to_ascii_lowercase();
    let d = &DEFS[k];
    match d.kind {
        Kind::Names(names) => names.iter().position(|n| n.to_ascii_lowercase() == s).map(|i| i as i32 + d.min),
        Kind::OnOff => ["off", "on"].iter().position(|n| s == *n).map(|i| i as i32),
        _ => {
            let lead: String = s.chars().take_while(|c| c.is_ascii_digit() || matches!(c, '-' | '+')).collect();
            lead.parse::<i32>().ok().map(|v| v.clamp(d.min, d.max))
        }
    }
}

/// The plugin's parameters: one integer parameter per `DEFS` entry.
pub struct QuantizeParams {
    pub p: Vec<IntParam>,
    /// Editor edits the host has not confirmed yet (the sound hears them at once).
    pub live: Arc<LiveEdits>,
}

impl Default for QuantizeParams {
    fn default() -> Self {
        let p = DEFS
            .iter()
            .enumerate()
            .map(|(k, d)| {
                IntParam::new(d.name, d.start, IntRange::Linear { min: d.min, max: d.max })
                    .with_value_to_string(Arc::new(move |v| value_text(k, v)))
                    .with_string_to_value(Arc::new(move |s| parse_text(k, s)))
            })
            .collect();
        QuantizeParams { p, live: Arc::default() }
    }
}

unsafe impl Params for QuantizeParams {
    fn param_map(&self) -> Vec<(String, ParamPtr, String)> {
        self.p.iter().zip(&DEFS).map(|(p, d)| (d.id.to_string(), p.as_ptr(), String::new())).collect()
    }
}

/// What the DSP reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Index into the grid divisions (`dsp::division_beats`).
    pub division: u8,
    /// 0..=100.
    pub strength: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Settings::from_values(&std::array::from_fn(|k| DEFS[k].start))
    }
}

impl Settings {
    /// The settings of stored values in `DEFS` order (each clamped to its range).
    pub fn from_values(v: &[i32; 2]) -> Self {
        let v: [i32; 2] = std::array::from_fn(|k| v[k].clamp(DEFS[k].min, DEFS[k].max));
        Settings { division: v[0] as u8, strength: v[1] as u8 }
    }
}

impl QuantizeParams {
    /// The settings the sound plays (an editor edit the host has not confirmed yet wins).
    pub fn settings(&self) -> Settings {
        Settings::from_values(&std::array::from_fn(|k| self.live.value(&self.p[k])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn factory_settings_are_the_defaults() {
        let p = QuantizeParams::default();
        assert_eq!(p.settings(), Settings::default());
    }

    #[test]
    fn value_texts_and_typing() {
        let k = |id| index(id).unwrap();
        assert_eq!(value_text(k("division"), 4), "1/16");
        assert_eq!(value_text(k("division"), 8), "1/8T");
        assert_eq!(value_text(k("strength"), 75), "75%");
        assert_eq!(parse_text(k("division"), "1/4t"), Some(7));
        assert_eq!(parse_text(k("strength"), "250"), Some(100));
    }

}
