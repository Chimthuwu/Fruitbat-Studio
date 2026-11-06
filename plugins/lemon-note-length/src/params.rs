// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Note Length parameters.

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

pub const DEFS: [Def; 3] = [
    Def { id: "mode", name: "Mode", min: 0, max: 1, start: 0, kind: Kind::Names(&["Percent", "Fixed"]) },
    Def { id: "amount", name: "Amount", min: 1, max: 400, start: 100, kind: Kind::Percent },
    Def { id: "length", name: "Length", min: 0, max: 10, start: 4, kind: Kind::Names(&["1/1", "1/2", "1/4", "1/8", "1/16", "1/32", "1/2T", "1/4T", "1/8T", "1/16T", "1/32T"]) },
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
pub struct NoteLengthParams {
    pub p: Vec<IntParam>,
    /// Editor edits the host has not confirmed yet (the sound hears them at once).
    pub live: Arc<LiveEdits>,
}

impl Default for NoteLengthParams {
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
        NoteLengthParams { p, live: Arc::default() }
    }
}

unsafe impl Params for NoteLengthParams {
    fn param_map(&self) -> Vec<(String, ParamPtr, String)> {
        self.p.iter().zip(&DEFS).map(|(p, d)| (d.id.to_string(), p.as_ptr(), String::new())).collect()
    }
}

/// How the length is set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The incoming length times `amount` percent.
    Percent,
    /// Every note lasts one `length` division.
    Fixed,
}

/// What the DSP reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub mode: Mode,
    /// Percent mode: 1..=400.
    pub amount: u16,
    /// Fixed mode: index into the grid divisions (`dsp::division_beats`).
    pub length: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Settings::from_values(&std::array::from_fn(|k| DEFS[k].start))
    }
}

impl Settings {
    /// The settings of stored values in `DEFS` order (each clamped to its range).
    pub fn from_values(v: &[i32; 3]) -> Self {
        let v: [i32; 3] = std::array::from_fn(|k| v[k].clamp(DEFS[k].min, DEFS[k].max));
        Settings { mode: if v[0] == 0 { Mode::Percent } else { Mode::Fixed }, amount: v[1] as u16, length: v[2] as u8 }
    }
}

impl NoteLengthParams {
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
        let p = NoteLengthParams::default();
        assert_eq!(p.settings(), Settings::default());
    }

    #[test]
    fn value_texts_and_typing() {
        let k = |id| index(id).unwrap();
        assert_eq!(value_text(k("mode"), 1), "Fixed");
        assert_eq!(value_text(k("amount"), 150), "150%");
        assert_eq!(value_text(k("length"), 3), "1/8");
        assert_eq!(parse_text(k("mode"), "fixed"), Some(1));
        assert_eq!(parse_text(k("amount"), "999"), Some(400));
        assert_eq!(parse_text(k("length"), "1/16T"), Some(9));
    }

}
