// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The glitch grid's saved patterns (Lemon Studio's design): the strip menu's Glitch Strip ▸ Pattern ▸ saves the
//! sixteen steps and the amount to a file of the user folder's `Glitch patterns`, named by the user in the save box,
//! and lists the ones saved there to put on a grid again.
//!
//! A pattern is JSON text, as a mixer track state is: the modules' numbers as `os_dsp::glitch::MODULES` numbers them,
//! so a file reads without the app.
// spec: docs/plans — the glitcher is Lemon Studio's own; ui/mixer-track-commands.md is the track menu it sits in

use std::path::{Path, PathBuf};

use os_dsp::glitch::{MODULES, Pattern, STEPS};
use serde_json::{json, Value};

use crate::app::{App, State};

/// The files' extension.
pub const EXT: &str = "osglitch";

/// The undo step of a pattern put on a grid.
pub const LOAD: &str = "Load glitch pattern";

/// `Glitch patterns` in the user folder; in tests, this thread's own folder (as `mixer_state::dir`), so a test's
/// patterns are the ones its menu lists.
pub fn dir() -> Option<PathBuf> {
    if cfg!(test) {
        let thread = format!("{:?}", std::thread::current().id()).chars().filter(char::is_ascii_digit).collect::<String>();
        return Some(std::env::temp_dir().join(format!("lemon-glitch-patterns-{}-{thread}", std::process::id())));
    }
    os_paths::user_file("Glitch patterns")
}

/// Whether `p` names a glitch pattern file.
pub fn is_pattern_file(p: &Path) -> bool {
    p.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXT))
}

/// A file's name without its extension, as the menu and the messages call it.
pub fn shown(p: &Path) -> String {
    p.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The saved patterns, by name.
pub fn list() -> Vec<(String, PathBuf)> {
    let Some(d) = dir() else { return Vec::new() };
    let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(d)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_pattern_file(p))
        .map(|p| (shown(&p), p))
        .collect();
    v.sort_by_key(|(n, _)| n.to_lowercase());
    v
}

/// A pattern as a file holds it.
pub fn to_json(p: &Pattern) -> String {
    serde_json::to_string_pretty(&json!({
        "lemon_glitch_pattern": 1,
        "amount": p.amount,
        "steps": p.steps.to_vec(),
    }))
    .unwrap_or_default()
}

/// Reads a pattern written by `to_json`; `None` for anything else. A step past the modules is taken as the last one,
/// and a short list is padded with steps that fire nothing, so a hand-edited file still loads.
pub fn from_json(text: &str) -> Option<Pattern> {
    let v: Value = serde_json::from_str(text).ok()?;
    v.get("lemon_glitch_pattern")?.as_u64()?;
    let last = (MODULES.len() - 1) as u64;
    let mut steps = [0u8; STEPS];
    for (i, s) in v.get("steps").and_then(Value::as_array).into_iter().flatten().take(STEPS).enumerate() {
        steps[i] = s.as_u64().unwrap_or(0).min(last) as u8;
    }
    let amount = v.get("amount").and_then(Value::as_u64).unwrap_or(0).min(u64::from(u16::MAX)) as u16;
    Some(Pattern { steps, amount })
}

/// Writes `p` as "<name>.<EXT>", with a number after the name when one of that name exists. The app's own save goes
/// through the save box (`save_as`, which lets the user name it); a test saves by name.
#[cfg(test)]
fn save(p: &Pattern, name: &str) -> Result<PathBuf, String> {
    let d = dir().ok_or("no user folder")?;
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let clean: String = name.trim().chars().map(|c| if r#"\ /:*?"<>|"#.contains(c) { '_' } else { c }).collect();
    let clean = if clean.is_empty() { "Pattern".to_string() } else { clean };
    let free = |p: &Path| !p.exists();
    let path = (1..).map(|i| d.join(if i == 1 { format!("{clean}.{EXT}") } else { format!("{clean} {i}.{EXT}") })).find(|p| free(p)).unwrap();
    std::fs::write(&path, to_json(p)).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Reads the pattern in `path`.
pub fn load(path: &Path) -> Option<Pattern> {
    from_json(&std::fs::read_to_string(path).ok()?)
}

/// Save pattern as...: the save box in `Glitch patterns`, named after the track (`save_chosen` writes it).
pub fn save_as(app: &mut App, n: u16) -> Option<String> {
    let Some(d) = dir() else { return Some("There is no folder to save glitch patterns in".into()) };
    let _ = std::fs::create_dir_all(&d);
    let name = crate::presets::folder_name(&crate::mixer::name(&app.state, n));
    let dlg = crate::filedlg::FileDlg::new(
        crate::filedlg::Mode::Save,
        "Save glitch pattern as",
        "glitch pattern",
        crate::dialog::Then::SaveGlitchPattern(n),
    )
    .filter("Lemon Studio glitch pattern", &[EXT])
    .start(Some(&d))
    .name(&format!("{name}.{EXT}"));
    app.file_dialog(dlg);
    None
}

/// The file chosen in Save pattern as...: the grid of track `n` is written there. Returns the status message.
pub fn save_chosen(app: &mut App, n: u16, target: PathBuf) -> Option<String> {
    let target = if is_pattern_file(&target) { target } else { target.with_extension(EXT) };
    // The save box's folder is made here too: a user who moved or deleted `Glitch patterns` still gets their pattern.
    if let Some(d) = target.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let name = shown(&target);
    let Some(m) = crate::mixer::track(&app.state, n) else { return Some("This track has no grid to save".into()) };
    let p = Pattern { steps: m.glitch, amount: m.glitch_amount };
    Some(match std::fs::write(&target, to_json(&p)).map_err(|e| e.to_string()) {
        Ok(()) => format!("Saved glitch pattern {name}"),
        Err(e) => format!("Glitch pattern not saved: {e}"),
    })
}

/// A pattern put on track `n`'s grid: its sixteen steps and its amount, and the glitcher switched on to hear it. The
/// undo step is the caller's (`crate::mixer::run` with `LOAD`).
pub fn apply(s: &mut State, n: u16, path: &Path) -> Option<String> {
    let name = shown(path);
    let Some(p) = load(path) else { return Some(format!("Couldn't read the glitch pattern {name}")) };
    let track = crate::mixer::name(s, n);
    let Some(m) = crate::mixer::track_mut(s, n) else { return Some(format!("The glitch pattern {name} has no track to go on")) };
    m.glitch = p.steps;
    m.glitch_amount = p.amount;
    m.glitch_style = true;
    s.piano.dirty = true;
    Some(format!("{track} takes the glitch pattern \"{name}\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pattern goes through a file unchanged, and a file that is not one is refused; a module past the list is read
    /// as the last one, and a short list is padded with steps that fire nothing.
    #[test]
    fn a_pattern_round_trips() {
        let _ = std::fs::remove_dir_all(dir().unwrap());
        let p = Pattern { steps: std::array::from_fn(|i| (i % MODULES.len()) as u8), amount: 40_000 };
        assert_eq!(from_json(&to_json(&p)), Some(p));
        assert_eq!(from_json("{}"), None);
        assert_eq!(from_json("not json"), None);
        let odd = from_json(r#"{"lemon_glitch_pattern": 1, "amount": 999999, "steps": [9, 2]}"#).unwrap();
        assert_eq!((odd.steps[0], odd.steps[1], odd.steps[2], odd.amount), ((MODULES.len() - 1) as u8, 2, 0, u16::MAX));

        let path = save(&p, "Roll").unwrap();
        assert_eq!((path.extension().and_then(|e| e.to_str()), load(&path)), (Some(EXT), Some(p)));
        assert_eq!(list().iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["Roll"]);
        // A second save of the same name sits beside the first instead of over it.
        let again = save(&p, "Roll").unwrap();
        assert_eq!(shown(&again), "Roll 2");
        let _ = std::fs::remove_dir_all(dir().unwrap());
    }
}
