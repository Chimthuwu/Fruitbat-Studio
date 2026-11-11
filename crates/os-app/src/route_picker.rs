// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The miniature mixer destination picker: a sheet of small mixer strips from a channel's TRACK box or the playlist's
//! Send to mixer track..., ported from the Michael Lane Edition. Opt-in: Settings ▸ Miscellaneous ▸ Miniature mixer
//! routing picker (`prefs::MIXER_ROUTING_PICKER`).
// spec: ui/mixer.md "Routing picker"

use crate::app::App;
use crate::dialog::{Body, Field};

/// The sheet: 12 columns of 4 rows, up to 48 tiles at a time.
pub const COLS: usize = 12;
pub const ROWS: usize = 4;

/// One tile of the sheet.
pub struct Track {
    pub index: u16,
    pub name: String,
    pub color: Option<os_model::Rgb>,
    /// Fader position 0..=1, for the miniature strip.
    pub level: f64,
}

/// The open sheet.
pub struct Form {
    /// The channel row being routed.
    pub row: usize,
    pub channel: String,
    /// The channel's current target mixer track.
    pub current: u16,
    pub tracks: Vec<Track>,
    /// The tiles the search lets through, as indices into `tracks`.
    pub shown: Vec<usize>,
    pub search: Field,
    /// The first tile shown (a multiple of `COLS`).
    pub first: usize,
}

impl Form {
    /// Keeps the tracks whose number or name holds every word of the search.
    pub fn filter(&mut self) {
        let q = self.search.text.to_lowercase();
        self.shown = self
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                q.split_whitespace()
                    .all(|w| format!("{} {}", t.index, t.name).to_lowercase().contains(w))
            })
            .map(|(i, _)| i)
            .collect();
        self.first = 0;
    }

    /// Scrolls the sheet by `rows` (the footer's Previous / Next, the wheel).
    pub fn scroll(&mut self, rows: isize) {
        let max = (self.shown.len().div_ceil(COLS)).saturating_sub(ROWS);
        self.first = (self.first / COLS).saturating_add_signed(rows).min(max) * COLS;
    }

    /// Brings the current route into view.
    pub fn reveal_current(&mut self) {
        if let Some(i) = self.shown.iter().position(|&i| self.tracks[i].index == self.current) {
            self.first = (i / COLS).saturating_sub(ROWS - 1) * COLS;
        }
    }
}

/// Whether the feature is on (Settings ▸ Miscellaneous ▸ Miniature mixer routing picker).
pub fn enabled() -> bool {
    crate::prefs::check(&crate::prefs::MIXER_ROUTING_PICKER)
}

/// Opens the sheet for channel row `row`.
pub fn open(app: &mut App, row: usize) {
    if !enabled() {
        return;
    }
    let Some(f) = form(&app.state, row) else { return };
    app.state.playlist.popup = None;
    app.show_form(Body::RoutePicker(f));
}

/// The sheet for channel `row`: every mixer track up to the last insert, plus the master.
fn form(s: &crate::app::State, row: usize) -> Option<Form> {
    let ch = s.project.channels.get(row)?;
    let max = crate::mixer::max_insert(&s.project);
    let mut tracks: Vec<Track> = s
        .project
        .mixer
        .iter()
        .filter(|m| m.index <= max)
        .map(|m| Track {
            index: m.index,
            name: crate::mixer::name(s, m.index),
            color: m.color,
            level: m.volume.unwrap_or(0.8),
        })
        .collect();
    tracks.sort_by_key(|t| t.index);
    let mut f = Form {
        row,
        channel: crate::rack::channel_name(s, row),
        current: ch.mixer_track,
        tracks,
        shown: Vec::new(),
        search: Field::default(),
        first: 0,
    };
    f.filter();
    f.reveal_current();
    Some(f)
}
