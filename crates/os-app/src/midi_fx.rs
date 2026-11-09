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

/// Moves the MIDI FX at filled position `from` to filled position `to` (both among the track's filled slots,
/// so a MIDI FX never crosses the instrument — the slots sit before it in signal flow): the filled order is
/// rebuilt compacted from slot 0, and the moved instance re-hosts under its new slot's key (a host failure
/// leaves the order standing, the tile degrading to its placeholder).
pub fn reorder(app: &mut App, track: u32, from: usize, to: usize) {
    let filled = filled(&app.state, track);
    if from >= filled.len() {
        return;
    }
    let mut order = filled;
    let moving = order.remove(from);
    let to = to.min(order.len());
    order.insert(to, moving);
    {
        let v = app.state.midi_fx.entry(track).or_default();
        v.clear();
        v.resize(SLOTS, None);
        for (i, (_, sl)) in order.iter().enumerate() {
            v[i] = Some(sl.clone());
        }
    }
    // Every instance follows its slot's new key. Compacting the slots shifts all of them, so re-hosting only the
    // moved plugin left the others with no instance — their tiles read "No parameters to show" and their editors
    // went blank. `rekey` keeps the plugin objects, so parameters and open editors survive the move.
    let moves: Vec<(usize, usize)> = order
        .iter()
        .enumerate()
        .map(|(i, (old, _))| (crate::plugins::midi_fx_key(track, *old), crate::plugins::midi_fx_key(track, i)))
        .collect();
    app.plugins.rekey(&moves);
    app.invalidate();
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A rack with a MIDI track that hosts two MIDI FX, the first holding a parameter value of its own.
    fn app_with_fx() -> (App, u32) {
        let mut app = App::new(None);
        app.state.project = crate::app::test_project();
        app.state.project.linear_mode = true;
        // The built-ins, as the app has them (a bare `App::new` has not scanned any plugin yet).
        app.plugins = crate::plugins::Plugins::builtins();
        app.state.plugdb.rebuild(&app.plugins);
        crate::playlist::run(&mut app, crate::playlist::Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi));
        let track = app.state.project.tracks.last().expect("a MIDI track").id.0;
        let at = |name: &str| app.plugins.midi_fx.iter().position(|a| a.desc.name == name).expect(name);
        let (arp, trans) = (at(crate::note_fx::LEMON_ARP), at(crate::note_fx::LEMON_TRANSPOSE));
        assert!(add(&mut app, track, None, arp).is_none());
        assert!(add(&mut app, track, None, trans).is_none());
        (app, track)
    }

    /// Reordering keeps every instance with its own slot: the compaction shifts all the keys, so the plugins that
    /// did not move need their instances moved too (only the dragged one used to be re-hosted, leaving the rest
    /// with "No parameters to show"). Values survive, because the plugin objects are moved, not recreated.
    #[test]
    fn reordering_keeps_each_plugins_parameters() {
        let (mut app, track) = app_with_fx();
        let key0 = crate::plugins::midi_fx_key(track, 0);
        let key1 = crate::plugins::midi_fx_key(track, 1);
        let id = app
            .plugins
            .param_infos(key0)
            .iter()
            .find(|p| !p.read_only && !p.hidden && p.steps != 1)
            .map(|p| p.id)
            .expect("the arpeggiator has a dial");
        assert!(app.plugins.set_param(key0, id, 0.25));
        reorder(&mut app, track, 0, 1);
        assert!(app.plugins.param_infos(key1).iter().any(|p| p.id == id), "the arpeggiator answers at its new key");
        assert_eq!(info(&app.state, key1).map(|i| i.name), Some(crate::note_fx::LEMON_ARP.to_string()), "the arp moved");
        assert_eq!(info(&app.state, key0).map(|i| i.name), Some(crate::note_fx::LEMON_TRANSPOSE.to_string()), "the other took slot 0");
        assert_eq!(app.plugins.param_value(key1, id), Some(0.25), "the moved plugin kept its value");
        assert!(!app.plugins.rack_controls(key0, &[]).is_empty(), "the other plugin still shows its controls");
        assert!(!app.plugins.rack_controls(key1, &[]).is_empty(), "and so does the moved one");
    }
}
