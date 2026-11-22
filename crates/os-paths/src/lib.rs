// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Where Fruitbat Studio keeps its own files: settings, the remembered theme and design, presets, browser folders.
//!
//! One folder per user, shared by the application and its own plugins (a plugin runs in its own module and reads the
//! theme from here itself):
//!
//! - Windows: `%APPDATA%\Fruitbat Studio`
//! - macOS: `~/Library/Application Support/Fruitbat Studio`
//! - Linux and other Unix: `$XDG_CONFIG_HOME/Fruitbat Studio`, else `~/.config/Fruitbat Studio`
//!
//! `OPENSTUDIO_USER_DIR` overrides the folder on every platform. Targets without a file system (the web) have none.

use std::path::PathBuf;

/// The folder's own name inside the platform's per-user settings folder.
const APP: &str = "Fruitbat Studio";
/// Its names before the current one, newest first: the app was Lemon Studio until the Fruitbat Studio rename and
/// `OpenStudio` before that (`legacy_dirs`).
const LEGACY: [&str; 2] = ["Lemon Studio", "OpenStudio"];

/// Fruitbat Studio's per-user folder, or `None` when the platform has none (or its variables are not set). It may not
/// exist yet: writers create it.
pub fn user_dir() -> Option<PathBuf> {
    if let Some(d) = var("OPENSTUDIO_USER_DIR") {
        return Some(d);
    }
    platform_dir().map(|d| d.join(APP))
}

/// The per-user folders from before the current name (`Lemon Studio`, then `OpenStudio`, in the same place), newest
/// first, which the app moves to [`user_dir`] once. Empty when `OPENSTUDIO_USER_DIR` chooses the folder.
pub fn legacy_dirs() -> Vec<PathBuf> {
    if var("OPENSTUDIO_USER_DIR").is_some() {
        return Vec::new();
    }
    let Some(dir) = platform_dir() else { return Vec::new() };
    LEGACY.iter().map(|name| dir.join(name)).collect()
}

/// A file or folder inside [`user_dir`].
pub fn user_file(name: &str) -> Option<PathBuf> {
    user_dir().map(|d| d.join(name))
}

/// A set, absolute environment variable (empty and relative values are ignored, as XDG asks).
fn var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute())
}

#[cfg(windows)]
fn platform_dir() -> Option<PathBuf> {
    var("APPDATA")
}

#[cfg(target_os = "macos")]
fn platform_dir() -> Option<PathBuf> {
    var("HOME").map(|h| h.join("Library").join("Application Support"))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn platform_dir() -> Option<PathBuf> {
    var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|h| h.join(".config")))
}

#[cfg(not(any(windows, unix)))]
fn platform_dir() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_is_named_after_the_app() {
        // Whatever the platform resolves to, the last part is ours (unless overridden).
        if std::env::var_os("OPENSTUDIO_USER_DIR").is_none() {
            if let Some(d) = user_dir() {
                assert_eq!(d.file_name().and_then(|n| n.to_str()), Some(APP));
                assert!(d.is_absolute());
            }
        }
        assert_eq!(user_file("monitor.txt").map(|f| f.file_name().unwrap().to_owned()), user_dir().map(|_| "monitor.txt".into()));
        // The old folders sit next to it, newest first.
        if let Some(new) = user_dir() {
            let old = legacy_dirs();
            let names: Vec<_> = old.iter().map(|d| d.file_name().and_then(|n| n.to_str())).collect();
            assert_eq!(names, LEGACY.iter().map(|n| Some(*n)).collect::<Vec<_>>());
            assert!(old.iter().all(|d| d.parent() == new.parent()));
        }
    }
}
