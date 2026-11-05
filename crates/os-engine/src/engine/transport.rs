// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! What to play and where it plays from: the play modes, the metronome's bar regions and the spans a render follows.

use os_model::{ChannelKind, ClipSource, ControlPoint, Project, Stretching};
use super::{marks};
use super::{Engine};

/// What to play.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayMode {
    /// The first arrangement's playlist. Falls back to the current pattern if it has no clips.
    Song,
    /// One pattern (1-based), from its start.
    Pattern(u16),
}

/// How much a nudge of the Tempo tapper speeds playback up or slows it down (spec: ui/tempo-tapper.md §6: about 2 %).
pub(crate) const NUDGE: f64 = 0.02;

/// The metronome's regions for `mode` (`Engine::click_bars`): the song's arrangement when it has clips (an empty
/// playlist plays a pattern, which counts in the project's bar), the pattern's own markers in pattern mode. A region's
/// beat is its bar over its numerator: `PPQ * 4 / denominator` ticks, one step-per-beat for the project's bar.
pub(crate) fn click_bars(project: &Project, mode: PlayMode) -> Vec<(u64, u32, u16)> {
    let markers = match mode {
        PlayMode::Song => project.arrangement().filter(|a| !a.clips.is_empty()).map_or(&[][..], |a| &a.markers),
        PlayMode::Pattern(n) => project.pattern(n).map_or(&[][..], |p| &p.markers),
    };
    let beats = |sig: (u16, u16)| sig.0.max(1);
    project.marker_bars(markers).into_iter().map(|(start, bar, sig)| (u64::from(start), (bar / u32::from(beats(sig))).max(1), beats(sig))).collect()
}

/// The spans of ticks a whole-song render of a song ending at `end` plays, in order (`Engine::follow_markers`; spec:
/// audio/song-structure.md "Time markers"): from the Start marker (or 0), broken at skip markers.
pub fn rendered_spans(markers: &[os_model::TimeMarker], end: u64) -> Vec<(u64, u64)> {
    let m = marks::Marks::new(markers);
    let start = m.start().filter(|&s| s < end).unwrap_or(0);
    m.rendered(start, end)
}
