// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Transpose parameters.

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

pub const DEFS: [Def; 5] = [
    Def { id: "semitones", name: "Semitones", min: -24, max: 24, start: 0, kind: Kind::Semitones },
    Def { id: "octaves", name: "Octaves", min: -4, max: 4, start: 0, kind: Kind::Octaves },
    Def { id: "scale_aware", name: "Scale Aware", min: 0, max: 1, start: 0, kind: Kind::OnOff },
    Def { id: "root", name: "Root", min: 0, max: 11, start: 0, kind: Kind::Names(&["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]) },
    Def { id: "scale", name: "Scale", min: 0, max: 8, start: 0, kind: Kind::Names(&["Major", "Minor", "Dorian", "Phrygian", "Lydian", "Mixolydian", "Locrian", "Harmonic Minor", "Melodic Minor"]) },
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
pub struct TransposeParams {
    pub p: Vec<IntParam>,
    /// Editor edits the host has not confirmed yet (the sound hears them at once).
    pub live: Arc<LiveEdits>,
}

impl Default for TransposeParams {
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
        TransposeParams { p, live: Arc::default() }
    }
}

unsafe impl Params for TransposeParams {
    fn param_map(&self) -> Vec<(String, ParamPtr, String)> {
        self.p.iter().zip(&DEFS).map(|(p, d)| (d.id.to_string(), p.as_ptr(), String::new())).collect()
    }
}

/// What the DSP reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub semitones: i32,
    pub octaves: i32,
    pub scale_aware: bool,
    /// Pitch class of the scale's root (0 = C).
    pub root: u8,
    /// Index into `dsp::SCALES`.
    pub scale: u8,
}

impl Default for Settings {
    fn default() -> Self {
        Settings::from_values(&std::array::from_fn(|k| DEFS[k].start))
    }
}

impl Settings {
    /// The settings of stored values in `DEFS` order (each clamped to its range).
    pub fn from_values(v: &[i32; 5]) -> Self {
        let v: [i32; 5] = std::array::from_fn(|k| v[k].clamp(DEFS[k].min, DEFS[k].max));
        Settings { semitones: v[0], octaves: v[1], scale_aware: v[2] != 0, root: v[3] as u8, scale: v[4] as u8 }
    }

    /// The whole shift in semitones.
    pub fn shift(&self) -> i32 {
        self.semitones + 12 * self.octaves
    }
}

impl TransposeParams {
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
        let p = TransposeParams::default();
        assert_eq!(p.settings(), Settings::default());
    }

    #[test]
    fn value_texts_and_typing() {
        let k = |id| index(id).unwrap();
        assert_eq!(value_text(k("semitones"), -3), "-3 st");
        assert_eq!(value_text(k("octaves"), 2), "+2 oct");
        assert_eq!(value_text(k("scale_aware"), 1), "On");
        assert_eq!(value_text(k("root"), 9), "A");
        assert_eq!(value_text(k("scale"), 1), "Minor");
        assert_eq!(parse_text(k("semitones"), "+7 st"), Some(7));
        assert_eq!(parse_text(k("semitones"), "99"), Some(24));
        assert_eq!(parse_text(k("scale"), "dorian"), Some(2));
        assert_eq!(parse_text(k("root"), "f#"), Some(6));
    }

    #[test]
    fn settings_combine_octaves() {
        let s = Settings::from_values(&[5, -1, 1, 2, 0]);
        assert_eq!(s.shift(), -7);
        assert!(s.scale_aware);
    }
}
