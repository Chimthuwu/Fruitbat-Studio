// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! A MIDI track's MIDI FX slots (the device rack's first region, spec: track-model-plan.md §9,
//! plugins/lemon-arp.md): plugins that transform the notes a MIDI track plays — Lemon Arp, Lemon Chord, Lemon Echo
//! today. In memory for now (the `.flp` records and the engine's note-path wiring are the next milestone): each
//! slot holds the plugin's name and its `Plugins::hosted` key (`midi_fx_key`), and the hosted instance — its editor
//! embeds in its tile like any built-in's, and its parameters show in the placeholder body.

use std::collections::BTreeMap;

use crate::app::App;

/// How many MIDI FX slots a track has (FL Studio's).
pub const SLOTS: usize = 8;

/// One MIDI FX slot of a MIDI track: what is in it, and whether it is on.
#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    /// The plugin's name ("Lemon Arp").
    pub name: String,
    /// The plugin's index in `Plugins::midi_fx`.
    pub index: usize,
    pub on: bool,
}

/// The MIDI FX slots, by the track's id (`os_model::Track::id`). In memory only: a session's slots go with it.
pub type Slots = BTreeMap<u32, Vec<Option<Slot>>>;

/// The slots of the track whose id is `track`, in slot order (empty when none).
pub fn slots(s: &crate::app::State, track: u32) -> Vec<Option<Slot>> {
    s.midi_fx.get(&track).cloned().unwrap_or_default()
}

/// The filled slots of track `track`, as (slot, slot data) pairs.
pub fn filled(s: &crate::app::State, track: u32) -> Vec<(usize, Slot)> {
    slots(s, track).into_iter().enumerate().filter_map(|(i, o)| o.map(|sl| (i, sl))).collect()
}

/// Adds plugin `index` (of `Plugins::midi_fx`) to track `track`'s slot `slot` (the first empty one when `None`):
/// the slot takes the plugin, and its instance is hosted under `midi_fx_key` (its editor opens on its tile).
pub fn add(app: &mut App, track: u32, slot: Option<usize>, index: usize) -> Option<String> {
    let name = app.plugins.midi_fx.get(index)?.desc.name.clone();
    let at = slot.unwrap_or_else(|| {
        let v = app.state.midi_fx.get(&track);
        v.and_then(|v| v.iter().position(Option::is_none)).unwrap_or(v.map_or(0, |v| v.len().min(SLOTS - 1)))
    });
    if at >= SLOTS {
        return Some("Every MIDI FX slot of this track is in use".into());
    }
    let v = app.state.midi_fx.entry(track).or_default();
    v.resize(SLOTS, None);
    v[at] = Some(Slot { name: name.clone(), index, on: true });
    match app.plugins.create_midi_fx(track, at, index, None) {
        Ok(_) => {
            app.invalidate();
            None
        }
        Err(e) => {
            // The slot goes back out: nothing stays half-hosted.
            if let Some(v) = app.state.midi_fx.get_mut(&track) {
                v[at] = None;
            }
            Some(e)
        }
    }
}

/// Removes track `track`'s MIDI FX slot `slot` (its instance goes with it).
pub fn remove(app: &mut App, track: u32, slot: usize) {
    if let Some(v) = app.state.midi_fx.get_mut(&track) {
        if slot < v.len() {
            v[slot] = None;
        }
    }
    app.plugins.drop_key(crate::plugins::midi_fx_key(track, slot));
    app.invalidate();
}

/// The window-frame info of the MIDI FX instance hosted at `key` (its tile's name and the track it is on).
pub fn info(s: &crate::app::State, key: usize) -> Option<crate::plugin_window::ChannelInfo> {
    let (track, slot) = crate::plugins::midi_fx_of(key)?;
    let sl = slots(s, track).get(slot)?.clone()?;
    Some(crate::plugin_window::ChannelInfo {
        misc: None,
        icon: None,
        name: sl.name,
        target: "MIDI FX".into(),
        enabled: sl.on,
        volume: 0.8,
        pan: 0.0,
        pitch_cents: 0,
        range: 2,
        track: 0,
        max_track: 0,
    })
}

/// Track `track`'s MIDI FX slot `slot` switches on / off.
pub fn toggle(app: &mut crate::app::State, track: u32, slot: usize) {
    if let Some(Some(sl)) = app.midi_fx.get_mut(&track).and_then(|v| v.get_mut(slot)) {
        sl.on = !sl.on;
    }
}
