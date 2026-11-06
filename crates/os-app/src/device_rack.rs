// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The device rack (Lemon Studio's design: spec model/track-model.md §7): a panel docked at the bottom of the main
//! window that holds the devices of the selected track in signal-flow order — a MIDI track's instrument (its
//! channel's plugin) and the mixer strip's effect slots. Built-in Lemon plugins' editors render inside their device
//! tile; anything else shows a placeholder body with a Pop out button, and every device's header menu can pop the
//! editor out to the floating window machinery (Options ▸ Detached's) or dock it back. Only Linear mode shows the
//! rack: Classic mode keeps today's behaviour untouched.
//!
//! ## Signal-flow container seam
//!
//! [`Device`] models a node in a signal-flow chain (track-model-plan.md §9). [`DeviceKind`] variants are:
//! - `Instrument` — a MIDI track's channel plugin
//! - `Effect` — a mixer strip insert slot
//! - (future) `Container { children }` — a group device holding sub-nodes
//!
//! [`chain`] returns the flat ordered list; the view layer can choose to render a device with children as a
//! nested container rather than a flat tile.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use crate::app::{App, Drag, State};

/// Least body height of the rack (its header and splitter aside), in logical px.
pub const MIN_H: f32 = 96.0;
/// The height the rack opens at, and what a double click on its splitter gives back.
pub const DEFAULT_H: f32 = 224.0;
/// Header row (title, pin, collapse), and the collapsed rack's whole height.
pub const HEADER_H: f32 = 23.0;
/// Width of a device tile.
pub const TILE_W: f32 = 340.0;

/// Whether the rack shows at all: Linear mode only (Classic mode = zero visual change).
pub fn visible(s: &State) -> bool {
    s.project.linear_mode && s.show_device_rack
}

/// The rack's height (logical px): 0 when hidden (Classic mode always), the header when collapsed.
pub fn height(s: &State) -> f32 {
    if !visible(s) {
        0.0
    } else if s.device_rack_collapsed {
        HEADER_H
    } else {
        s.device_rack_h.clamp(MIN_H, (s.screen.1 / 2.0).max(MIN_H))
    }
}

/// A device of the track the rack shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DeviceKind {
    /// A MIDI track's MIDI FX slot `usize` (`crate::midi_fx`): a loaded MIDI FX plugin.
    MidiFx(usize),
    /// The instrument: the channel (its rack row) a MIDI track plays.
    Instrument(usize),
    /// A mixer strip's effect slot: the strip's insert and the slot's index.
    Fx(u16, u8),
}

/// One device of a track's chain: what it is, its plugin-window key (a channel row or an `fx_key`), its name,
/// whether it is on, and whether its editor can embed (a built-in Lemon plugin).
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub kind: DeviceKind,
    /// The hosted plugin-window key (`Plugins::open_window`).
    pub key: Option<usize>,
    pub name: String,
    pub on: bool,
    pub builtin: bool,
}

/// The track whose devices the rack shows: the pinned lane while the chain is pinned, else the selected lane
/// (§7 "Track selection drives the rack").
pub fn rack_track(s: &State) -> Option<&os_model::Track> {
    let row = s.device_rack_pin.unwrap_or(s.playlist.track);
    crate::track_links::lane_track(s, row)
}

/// The devices of `tr` in signal-flow order (§7): MIDI FX → instrument → the mixer strip's effect slots. Audio
/// tracks and buses hold only the strip's slots.
pub fn chain(s: &State, tr: &os_model::Track) -> Vec<Device> {
    let mut v = Vec::new();
    if tr.kind == os_model::TrackKind::Midi {
        // The MIDI FX slots in signal-flow order (§7: they come first). Adding one is the browser's job: every
        // ＋ gap button and the header's ＋ Add list the MIDI FX built-ins (no dedicated add tile).
        for (i, sl) in crate::midi_fx::filled(s, tr.id.0) {
            v.push(Device {
                kind: DeviceKind::MidiFx(i),
                key: Some(crate::plugins::midi_fx_key(tr.id.0, i)),
                name: sl.name,
                on: sl.on,
                builtin: true,
            });
        }
    }
    if let Some(row) = tr
        .channel
        .and_then(|ch| s.project.channels.iter().position(|c| c.index == ch))
    {
        let ch = &s.project.channels[row];
        // A sampler channel stores no plugin name (`plugin: None`, kind Sampler) — it is Lemon Sampler by kind.
        let builtin = ch.kind == os_model::ChannelKind::Sampler
            || ch.plugin.as_deref().is_some_and(stored_builtin)
            || crate::project_plugins::wrapped_channel(ch)
                .map_or(false, |r| stored_builtin(&r.name));
        v.push(Device {
            kind: DeviceKind::Instrument(row),
            key: Some(row),
            name: crate::rack::channel_name(s, row),
            on: ch.enabled,
            builtin,
        });
    }
    if let Some(m) = s.project.mixer.iter().find(|m| m.index == tr.mixer_strip) {
        for (k, slot) in m.slots.iter().enumerate() {
            let Some(sp) = slot else { continue };
            v.push(Device {
                kind: DeviceKind::Fx(m.index, k as u8),
                key: Some(crate::plugins::fx_key(m.index, k as u8)),
                name: crate::project_plugins::slot_title(sp),
                on: m.slot_on[k],
                // Slots store file ids ("Fruity Parametric EQ 2", wrapped plugins) — compare the Lemon name.
                builtin: stored_builtin(&sp.name)
                    || crate::project_plugins::wrapped_slot(sp)
                        .map_or(false, |r| stored_builtin(&r.name)),
            });
        }
    }
    v
}

/// The chain the rack shows (of [`rack_track`]).
pub fn rack_chain(s: &State) -> Vec<Device> {
    rack_track(s).map_or_else(Vec::new, |tr| chain(s, tr))
}

/// Returns the selected tile index in the rack, if any.
#[allow(dead_code)] // The view layer will read it when tile selection is wired up.
pub fn rack_sel(s: &State) -> Option<usize> {
    s.device_rack_sel
}

/// Returns a mutable reference to the selected tile index.
pub fn rack_sel_mut(s: &mut State) -> &mut Option<usize> {
    &mut s.device_rack_sel
}

/// Reorders the device at index `from` to index `to` in the rack's chain. Returns `Ok` on success, or `Err`
/// if the control plane command is not yet implemented (the caller handles the error gracefully).
pub fn reorder(_s: &mut State, _from: usize, _to: usize) -> Result<(), String> {
    // TODO §7: wire up the control plane ReorderDevices command
    Err("ReorderDevices command not yet implemented".into())
}

/// The built-in Lemon plugins' names (what `Plugins` ships with, before any scan).
fn builtin_name(name: &str) -> bool {
    static NAMES: OnceLock<BTreeSet<String>> = OnceLock::new();
    NAMES
        .get_or_init(|| {
            let mut v: BTreeSet<String> = crate::plugins::builtin()
                .iter()
                .chain(crate::plugins::builtin_effects().iter())
                .map(|a| a.desc.name.clone())
                .collect();
            // The compiled-in names themselves (a CLAP descriptor may spell one differently).
            v.extend(crate::plugins::builtin_midi_fx().iter().map(|a| a.desc.name.clone()));
            v.extend(crate::plugins::LINKED_EFFECTS.iter().map(|s| s.to_string()));
            v.extend(crate::plugins::LINKED_MIDI_FX.iter().map(|s| s.to_string()));
            v.extend(crate::plugins::FILE_MAPPED.iter().map(|s| s.to_string()));
            v.extend(crate::plugins::OLD_NAMES.iter().map(|(_, n)| n.to_string()));
            v
        })
        .contains(name)
}

/// Whether a stored plugin name is a built-in Lemon plugin, after the stored-name
/// mappings a project file carries: a slot's FL id / recreation label and a channel's
/// file id (`slot_label`, `osc3::label`), plus the built-ins' old names (`OLD_NAMES`).
fn stored_builtin(name: &str) -> bool {
    if builtin_name(name) {
        return true;
    }
    let n = crate::osc3::label(crate::plugins::slot_label(name));
    let n = crate::plugins::OLD_NAMES
        .iter()
        .find(|(o, _)| *o == n)
        .map(|(_, v)| *v)
        .unwrap_or(n);
    builtin_name(n)
}

/// The rack's actions (the device tiles, its header and splitter).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RackAct {
    /// The splitter between the workspace and the rack: drag to resize, double click for the default height.
    Splitter,
    /// The header's pin toggle: keep this track's chain while editing elsewhere (§7).
    TogglePin,
    /// The header's collapse toggle.
    ToggleCollapsed,
    /// Select a device tile by its index in the chain.
    #[allow(dead_code)] // Constructed by the view layer once tile selection is wired up.
    SelectTile(usize),
    /// A device header's menu: open it for `key`'s device at the mouse.
    Menu(usize),
    /// Dismiss an open device menu.
    MenuClose,
    /// A device's Pop out: its editor leaves the tile for the floating window machinery.
    PopOut(usize),
    /// A device's Dock: its editor comes back into its tile.
    Dock(usize),
    /// A device's enable light: switch the device on / off.
    ToggleOn(DeviceKind),
    /// The header's "+ Add": the plugin palette (instruments and effects).
    #[allow(dead_code)] // The merged rack header uses `AddDevice`'s browser; the palette stays available.
    Add,
    /// A "+" gap button at a chain position: the device browser for mixer slot `slot`
    /// (the next device's slot, or the first empty slot when `None`) — MIDI FX included.
    AddAt(Option<u8>),
    /// A tile header pressed: drag it left/right to reorder (FX tiles swap slots).
    TilePress(DeviceKind),
    /// The device rack's "＋ Add" header button: open the device-rack plugin browser (native plugins first).
    #[allow(dead_code)] // Superseded by `Add`'s plugin palette; kept for the merged browser wiring.
    AddDevice,
    /// Reorder a device: move it from index `from` to index `to`.
    #[allow(dead_code)] // Constructed by the view layer once drag-to-reorder is wired up.
    Reorder { from: usize, to: usize },
    /// An overflow stepper (the header's ‹ ›): scroll the chain by one tile (negative = left).
    ScrollBy(f32),
    /// A tile body's parameter row: set the parameter `id` of the instance under `key` to normalized `v`
    /// (a click, or a drag's move, on a value bar).
    ParamSet { key: usize, id: u32, v: f64 },
}

/// The splitter pressed: a double click (within the browser's double-click window) gives back the default height,
/// else the drag starts.
pub fn splitter_press(app: &mut App) {
    let now = std::time::Instant::now();
    let s = &mut app.state;
    let double = s
        .device_rack_split_click
        .is_some_and(|t| now.duration_since(t).as_millis() < 400);
    s.device_rack_split_click = if double { None } else { Some(now) };
    if double {
        s.device_rack_h = DEFAULT_H;
        s.device_rack_collapsed = false;
        app.invalidate();
        return;
    }
    app.drag = Some(Drag::DeviceRackSplit {
        dy: app.mouse.1 - height(s),
    });
}

/// The splitter dragged so that the rack (its splitter included) ends at `top`... its bottom stays at the window's
/// bottom edge: the rack's height follows the pointer. Whole px, from [`MIN_H`] to half the window.
pub fn split_drag(app: &mut App, y: f32, dy: f32) {
    let s = &mut app.state;
    let h = (y - dy).min(s.screen.1 / 2.0).max(MIN_H).round();
    if h != s.device_rack_h {
        s.device_rack_h = h;
        s.device_rack_collapsed = false;
        app.invalidate();
    }
}

/// A click on the pin toggle: the rack keeps showing this lane's chain while the selection moves elsewhere.
pub fn toggle_pin(app: &mut App) {
    let s = &mut app.state;
    s.device_rack_pin = if s.device_rack_pin.is_some() {
        None
    } else {
        Some(s.playlist.track)
    };
    app.invalidate();
}

/// A device's Pop out: `key`'s editor opens in (or moves to) the floating window machinery; the rack stops
/// pinning it to its tile. An editor that is not open yet opens first, detached.
pub fn pop_out(app: &mut App, key: usize) {
    app.state.device_rack_float.insert(key);
    app.state.device_rack_embed.remove(&key);
    app.plugins.set_detached(key, true);
    // An editor that was not open yet opens detached now; one that was moves out of its tile.
    #[cfg(plugin_windows)]
    if app.plugins.window_of(key).is_none() {
        let _ = open_editor(app, key);
    }
    app.state.device_rack_menu = None;
    app.invalidate();
}

/// A device's Dock: `key`'s editor comes back into its tile (the rack re-pins it on the next frame).
pub fn dock(app: &mut App, key: usize) {
    app.state.device_rack_float.remove(&key);
    app.plugins.set_detached(key, false);
    app.state.device_rack_menu = None;
    app.invalidate();
}

/// Runs a rack action.
pub fn run(app: &mut App, act: RackAct) {
    match act {
        RackAct::Splitter => splitter_press(app),
        RackAct::TogglePin => toggle_pin(app),
        RackAct::ToggleCollapsed => {
            app.state.device_rack_collapsed = !app.state.device_rack_collapsed;
            app.invalidate();
        }
        RackAct::SelectTile(idx) => {
            *rack_sel_mut(&mut app.state) = Some(idx);
            app.invalidate();
        }
        RackAct::Menu(key) => {
            app.state.device_rack_menu = Some(key);
            app.invalidate();
        }
        RackAct::MenuClose => {
            app.state.device_rack_menu = None;
            app.invalidate();
        }
        RackAct::PopOut(key) => pop_out(app, key),
        RackAct::Dock(key) => dock(app, key),
        RackAct::ToggleOn(kind) => toggle_on(app, kind),
        RackAct::Add => {
            let at = app.mouse;
            app.open_plugin_palette(crate::plugin_menu::Target::All, at, None);
        }
        RackAct::AddAt(slot) => {
            // Every ＋ opens the one device browser: MIDI FX, instruments and effects (effects land in
            // `slot`, MIDI FX in the track's MIDI FX slots, instruments as a new channel).
            let p = crate::views::device_rack::add_device_menu(
                &app.state,
                &app.plugins.midi_fx,
                app.mouse.0,
                app.mouse.1,
                slot,
            );
            app.state.playlist.popup = Some(p);
        }
        RackAct::TilePress(kind) => {
            app.state.device_rack_drag = Some(kind);
            app.state.device_rack_drop = None;
            app.drag = Some(Drag::DeviceRackTile { kind, _x0: app.mouse.0 });
        }
        RackAct::AddDevice => {
            let p = crate::views::device_rack::add_device_menu(
                &app.state,
                &app.plugins.midi_fx,
                app.mouse.0,
                app.mouse.1,
                None,
            );
            app.state.playlist.popup = Some(p);
        }
        RackAct::Reorder { from, to } => {
            let _ = reorder(&mut app.state, from, to);
            app.invalidate();
        }
        RackAct::ParamSet { key, id, v } => {
            // The click sets the value at once and keeps dragging it: the row's bar follows the pointer
            // (the drag's move maps x onto the same bar via `views::device_rack::param_bar`).
            set_param_value(app, key, id, v);
            app.drag = Some(Drag::DeviceRackParam { key, id });
        }
        RackAct::ScrollBy(d) => {
            let s = &mut app.state;
            let w = s.screen.0;
            s.device_rack_scroll_x =
                (crate::views::device_rack::scroll_x(s, w) + d * TILE_W)
                    .clamp(0.0, (crate::views::device_rack::content_w(s) - w).max(0.0));
            app.invalidate();
        }
    }
}

/// A device's enable light: the instrument channel or the mixer slot switches on / off, as its own window
/// (the mixer's slot menu, the plugin window's strip) does.
fn toggle_on(app: &mut App, kind: DeviceKind) {
    let s = &mut app.state;
    match kind {
        DeviceKind::Instrument(row) => {
            if let Some(ch) = s.project.channels.get_mut(row) {
                ch.enabled = !ch.enabled;
                s.piano.dirty = true;
            }
        }
        DeviceKind::Fx(track, slot) => {
            if let Some(m) = crate::mixer::track_mut(s, track) {
                m.slot_on[usize::from(slot)] = !m.slot_on[usize::from(slot)];
            }
        }
        DeviceKind::MidiFx(slot) => {
            let id = rack_track(s).map(|tr| tr.id.0);
            if let Some(id) = id {
                crate::midi_fx::toggle(s, id, slot);
            }
        }
    }
    app.invalidate();
}

/// Sets the parameter `id` of the hosted instance under `key` to normalized `v`. The ONE set path for the
/// rack's parameter rows: an edit in the plugin's window, the value box and (later) automation all land here.
pub fn set_param_value(app: &mut App, key: usize, id: u32, v: f64) {
    let v = v.clamp(0.0, 1.0);
    if app.plugins.set_param(key, id, v) {
        app.state.params.set_by_host(key, id, v);
        app.state.device_rack_failed.remove(&key);
        app.invalidate();
    }
}

/// Opens `key`'s plugin window with the info its frame shows (a channel row's strip or an `fx_key`'s slot title).
/// `Err` (a plugin that cannot open, or none hosted) is the caller's placeholder: the tile degrades, the app stays up.
#[cfg(plugin_windows)]
fn open_editor(app: &mut App, key: usize) -> Option<Result<(), String>> {
    let info = crate::plugins::fx_of(key)
        .and_then(|(_, _)| crate::mixer::fx_info(&app.state.project, &app.state.mixer, key))
        .or_else(|| crate::midi_fx::info(&app.state, key))
        .or_else(|| crate::app::channel_info(&app.state.project, key))?;
    Some(app.plugins.open_window(key, info))
}

/// A tile drag released at mouse x: the device moves to the chain position the drop gap says — MIDI FX reorder
/// among themselves (never past the instrument: they only ever sit in the MIDI FX slots), FX among their strip's
/// slots (adjacent `SlotMove`s, so remapping/automation follow), the instrument stays (a track holds one).
pub fn tile_drop(app: &mut App, kind: DeviceKind, x: f32) {
    app.state.device_rack_drag = None;
    app.state.device_rack_drop = None;
    let tiles = crate::views::device_rack::tiles(&app.state);
    let Some(src) = tiles.iter().position(|(d, _)| d.kind == kind) else { return };
    // The drop gap: how many tile middles the pointer passed (0..=len).
    let gap = tiles.iter().filter(|(_, t)| t.cx() < x).count();
    match kind {
        DeviceKind::MidiFx(_) => {
            let Some(tr) = rack_track(&app.state) else { return };
            let id = tr.id.0;
            let src_pos = tiles[..src].iter().filter(|(d, _)| matches!(d.kind, DeviceKind::MidiFx(_))).count();
            let mut to = tiles[..gap.min(tiles.len())]
                .iter()
                .filter(|(d, _)| matches!(d.kind, DeviceKind::MidiFx(_)))
                .count();
            if to > src_pos {
                to -= 1; // the removal shifts the later slots left by one
            }
            let n = crate::midi_fx::filled(&app.state, id).len();
            if to >= n {
                return;
            }
            if to != src_pos {
                crate::midi_fx::reorder(app, id, src_pos, to);
            }
        }
        DeviceKind::Fx(track, slot) => {
            // The strip's FX slots in visual order; the drop lands among them (self excluded).
            let fx: Vec<u8> = tiles
                .iter()
                .filter_map(|(d, _)| match d.kind {
                    DeviceKind::Fx(t, k) if t == track => Some(k),
                    _ => None,
                })
                .collect();
            let Some(src_i) = fx.iter().position(|&k| k == slot) else { return };
            let mut to = tiles[..gap.min(tiles.len())]
                .iter()
                .filter(|(d, _)| matches!(d.kind, DeviceKind::Fx(t, _) if t == track))
                .count();
            if to > src_i {
                to -= 1;
            }
            let others: Vec<u8> = fx.iter().copied().filter(|&k| k != slot).collect();
            if to >= others.len() + 1 || to == src_i {
                return;
            }
            let target = others.get(to).copied().unwrap_or_else(|| others.iter().max().copied().unwrap_or(slot) + 1);
            if target >= 10 || target == slot {
                return;
            }
            let prev = app.state.selected_mixer;
            app.state.selected_mixer = track;
            let mut cur = i32::from(slot);
            let target = i32::from(target);
            while cur != target {
                let step = (target - cur).clamp(-1, 1) as i8;
                crate::mixer::run(app, crate::mixer::MCmd::SlotMove(cur as u8, step));
                cur += i32::from(step);
            }
            app.state.selected_mixer = prev;
            app.invalidate();
        }
        DeviceKind::Instrument(_) => {
            // A track holds one instrument: nothing to reorder it with (its channel is the track's).
        }
    }
    app.invalidate();
}

/// Keeps the rack's embedded editors on their tiles: opens the built-in devices' editors that are missing, closes
/// the ones whose device left the chain, and moves each to its tile's rect. Called from `clip_docked` (every tick,
/// like the docked windows' stacking and clipping); guarded so a tile whose editor fails becomes a placeholder
/// instead of a crash (§7 risks) — `open_window`'s `Err` and an instance that stopped (`Request::Stopped`) both
/// leave the tile with its placeholder body.
pub fn sync_embed(app: &mut App) {
    let prev: Vec<usize> = app.state.device_rack_embed.iter().copied().collect();
    if !visible(&app.state) || app.state.device_rack_collapsed {
        for key in prev {
            app.plugins.close_if_open(key);
        }
        app.state.device_rack_embed.clear();
        return;
    }
    // The tiles the rack embeds: every built-in device the user has not popped out.
    let tiles = crate::views::device_rack::device_rects(&app.state);
    let keys: BTreeSet<usize> = tiles.iter().map(|&(k, _)| k).collect();
    for key in prev {
        if !keys.contains(&key) {
            app.plugins.close_if_open(key);
        }
    }
    app.state.device_rack_embed = keys;
    // The tiles' live parameters (every device's key, so a tile's placeholder body always has its controls): each
    // row is (param id, name, normalized value) for the params a tile can render and set.
    app.state.device_rack_params = tiles
        .iter()
        .filter_map(|&(k, _)| {
            let ps = app
                .plugins
                .param_infos(k)
                .iter()
                .filter(|i| !i.read_only && !i.bypass)
                .take(8)
                .map(|i| (i.id, i.name.clone(), app.plugins.param_value(k, i.id).unwrap_or(i.default)))
                .collect::<Vec<_>>();
            (!ps.is_empty()).then_some((k, ps))
        })
        .collect();
    // Open what is missing and pin each editor to its tile's rect. Without plugin windows there is nothing to
    // embed: every tile keeps its placeholder body.
    #[cfg(plugin_windows)]
    {
        // A stopped or failed editor is not retried: its tile keeps the placeholder.
        for &(key, _) in &tiles {
            if app.plugins.window_of(key).is_some()
                || app.state.device_rack_failed.contains(&key)
                || app.plugins.is_stopped(key)
            {
                continue;
            }
            if !matches!(open_editor(app, key), Some(Ok(()))) {
                app.state.device_rack_failed.insert(key);
            }
        }
        let k = app.plugins.scale;
        let (ox, oy) = app.canvas_off;
        for (key, r) in &tiles {
            let rect = [
                (r.x * k).floor() as i32 + ox,
                (r.y * k).floor() as i32 + oy,
                (r.right() * k).ceil() as i32 + ox,
                (r.bottom() * k).ceil() as i32 + oy,
            ];
            app.plugins.pin_window(*key, rect);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::playlist::Cmd;

    fn app() -> App {
        let mut app = App::new(None);
        app.state.project = crate::app::test_project();
        app.state.project.linear_mode = true;
        app
    }

    /// A MIDI track's chain: the instrument (its channel) first, then the strip's effect slots in slot order.
    /// An audio track holds only its strip's slots.
    #[test]
    fn chain_is_the_selected_tracks_devices() {
        let mut app = app();
        crate::playlist::run(
            &mut app,
            Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi),
        );
        let tr = app.state.project.tracks.last().unwrap().clone();
        let strip = tr.mixer_strip;
        let m = crate::mixer::track_mut(&mut app.state, strip).unwrap();
        m.slots[1] = Some(os_model::SlotPlugin {
            id: os_model::Id::NONE,
            name: "Lemon EQ2".into(),
            state: Vec::new(),
        });
        m.slot_on[1] = false;
        assert_eq!(
            app.state.playlist.track,
            tr.playlist_lane as u16 - 1,
            "the insert selects the lane"
        );
        let got = rack_chain(&app.state);
        // A MIDI track's chain leads with its (empty) MIDI FX region — no ＋ Add tile: adding goes through
        // the browser (every gap ＋ and the header's ＋ Add list the MIDI FX built-ins).
        assert_eq!(got.len(), 2);
        assert_eq!(
            got[0].kind,
            DeviceKind::Instrument(app.state.project.channels.len() - 1)
        );
        assert!(got[0].on, "the channel is on");
        assert_eq!(
            got[1],
            Device {
                kind: DeviceKind::Fx(strip, 1),
                key: Some(crate::plugins::fx_key(strip, 1)),
                name: "Lemon EQ2".into(),
                on: false,
                builtin: false
            }
        );
        // Selecting another (audio) track's lane swaps the chain to its strip's slots only.
        crate::playlist::run(
            &mut app,
            Cmd::TrackMode(2, crate::track_links::Mode::LinearAudio),
        );
        let audio = app.state.project.tracks.last().unwrap().clone();
        app.state.playlist.track = audio.playlist_lane as u16 - 1;
        let got = rack_chain(&app.state);
        assert!(
            !got.iter()
                .any(|d| matches!(d.kind, DeviceKind::Instrument(_))),
            "no instrument on an audio track"
        );
    }

    /// The pin toggle: the pinned chain stays while the selection moves; unpinning follows the selection again.
    #[test]
    fn pin_keeps_the_chain() {
        let mut app = app();
        crate::playlist::run(
            &mut app,
            Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi),
        );
        let midi = app.state.project.tracks.last().unwrap().playlist_lane as u16 - 1;
        crate::playlist::run(
            &mut app,
            Cmd::TrackMode(2, crate::track_links::Mode::LinearAudio),
        );
        let audio = app.state.project.tracks.last().unwrap().playlist_lane as u16 - 1;
        app.state.playlist.track = midi;
        run(&mut app, RackAct::TogglePin);
        assert_eq!(app.state.device_rack_pin, Some(midi));
        app.state.playlist.track = audio;
        assert_eq!(
            rack_track(&app.state).unwrap().playlist_lane as u16,
            midi + 1,
            "pinned"
        );
        run(&mut app, RackAct::TogglePin);
        assert_eq!(app.state.device_rack_pin, None);
        assert_eq!(
            rack_track(&app.state).unwrap().playlist_lane as u16,
            audio + 1,
            "follows the selection again"
        );
    }

    /// Pop out / Dock: the header menu's actions move the device's editor window between the rack and the floating
    /// window machinery (`Plugins::set_detached`), and the rack's pinning keeps out of the way while floated.
    #[test]
    fn pop_out_and_dock_change_the_window_state() {
        let mut app = app();
        app.plugins = crate::plugins::Plugins::builtins();
        crate::playlist::run(
            &mut app,
            Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi),
        );
        let tr = app.state.project.tracks.last().unwrap().clone();
        let row = app
            .state
            .project
            .channels
            .iter()
            .position(|c| Some(c.index) == tr.channel)
            .expect("the track's channel");
        // Give the instrument a built-in plugin so its editor can open (like the rack's plugin menu does).
        app.state.project.channels[row].plugin =
            Some(crate::plugins::builtin()[0].desc.name.clone());
        #[cfg(not(plugin_windows))]
        {
            // Without plugin windows the state change is the flag: Pop out floats the tile, Dock returns it.
            app.state.device_rack_float.insert(row);
            assert!(app.state.device_rack_float.contains(&row));
            run(&mut app, RackAct::Dock(row));
            assert!(!app.state.device_rack_float.contains(&row));
        }
        #[cfg(plugin_windows)]
        {
            // The channel's hosted instance (like the app's start-up plugin tick does), then its editor window.
            let index = app
                .plugins
                .index_of(crate::plugins::LEMON_SAMPLER)
                .expect("built-in Lemon Sampler");
            app.plugins.create_with(row, index, None).unwrap();
            let info = crate::app::channel_info(&app.state.project, row).unwrap();
            app.plugins.open_window(row, info).unwrap();
            assert!(
                app.plugins.window_of(row).is_some(),
                "the editor window opened"
            );
            // Without a main window the frame is forced detached (owner == 0), so the docked/detached bit of the
            // window itself is not testable here: the Hosted state the rack drives (`set_detached`) is.
            assert!(!app.plugins.hosted[&row].detached, "docked by default");
            run(&mut app, RackAct::PopOut(row));
            assert!(app.plugins.hosted[&row].detached, "popped out");
            assert!(app.state.device_rack_float.contains(&row));
            assert!(
                !crate::views::device_rack::device_rects(&app.state)
                    .iter()
                    .any(|&(k, _)| k == row),
                "not pinned while floated"
            );
            run(&mut app, RackAct::Dock(row));
            assert!(!app.plugins.hosted[&row].detached, "docked back");
            assert!(!app.state.device_rack_float.contains(&row));
        }
    }

    /// Built-in detection for stored names: a sampler channel (plugin None, kind Sampler) is a built-in, and
    /// a mixer slot holding a file id ("Fruity Parametric EQ 2") is its Lemon built-in.
    #[test]
    fn builtins_are_detected_for_stored_names() {
        let mut app = app();
        crate::playlist::run(
            &mut app,
            Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi),
        );
        let tr = app.state.project.tracks.last().unwrap().clone();
        let strip = tr.mixer_strip;
        let m = crate::mixer::track_mut(&mut app.state, strip).unwrap();
        m.slots[0] = Some(os_model::SlotPlugin {
            id: os_model::Id::NONE,
            name: crate::plugins::EQ_FILE_NAME.into(),
            state: Vec::new(),
        });
        let got = rack_chain(&app.state);
        let fx = got.iter().find(|d| matches!(d.kind, DeviceKind::Fx(_, 0))).unwrap();
        assert!(fx.builtin, "Fruity Parametric EQ 2 slot is Lemon Parametric EQ");
        let inst = got.iter().find(|d| matches!(d.kind, DeviceKind::Instrument(_))).unwrap();
        assert!(inst.builtin, "the sampler channel is a built-in");
    }

    /// Only Linear mode shows the rack: Classic mode's height is 0 (no layout change anywhere) and its chain is
    /// never read.
    #[test]
    fn classic_mode_shows_no_rack() {
        let mut app = app();
        crate::playlist::run(
            &mut app,
            Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi),
        );
        app.state.project.linear_mode = false;
        assert!(!visible(&app.state));
        assert_eq!(height(&app.state), 0.0);
        assert!(rack_chain(&app.state).is_empty());
        // Linear mode: the panel is there, resizable within its bounds.
        app.state.project.linear_mode = true;
        assert!(visible(&app.state));
        assert!(height(&app.state) >= HEADER_H);
        app.state.device_rack_h = 40.0;
        assert_eq!(height(&app.state), MIN_H);
        app.state.device_rack_collapsed = true;
        assert_eq!(height(&app.state), HEADER_H);
        // Hiding it (as a collapse does) gives the workspace its room back: Classic untouched either way.
        app.state.show_device_rack = false;
        assert_eq!(height(&app.state), 0.0);
    }

    /// The rack is visible and sized at the default height after a Linear template is loaded.
    #[test]
    fn linear_template_shows_rack() {
        let mut app = app();
        // Linear mode is on in app() via test_project, but ensure it's true and the rack is open.
        assert!(app.state.project.linear_mode);
        assert!(visible(&app.state));
        assert!(height(&app.state) >= DEFAULT_H.min((app.state.screen.1 / 2.0).max(MIN_H)));
    }

    /// Delete-key semantics: `RackAct::SelectTile` sets the selection, and the control plane handles
    /// `RemoveEffect` when the user presses Delete — the rack action does not delete directly.
    #[test]
    fn select_tile_and_delete_key_handling() {
        let mut app = app();
        crate::playlist::run(
            &mut app,
            Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi),
        );
        let chain = rack_chain(&app.state);
        assert!(!chain.is_empty(), "a MIDI track has at least the instrument");

        // Selecting a tile stores its index.
        let idx = 0;
        run(&mut app, RackAct::SelectTile(idx));
        assert_eq!(rack_sel(&app.state), Some(idx));

        // Selecting another tile replaces the selection.
        let idx = 1.min(chain.len() - 1);
        run(&mut app, RackAct::SelectTile(idx));
        assert_eq!(rack_sel(&app.state), Some(idx));

        // Delete key: the control plane receives RemoveEffect; here we verify the selection is set
        // so the caller can dispatch the command. The command itself is tested elsewhere.
        assert!(rack_sel(&app.state).is_some());
    }

    /// Horizontal scroll: a chain wider than the panel scrolls (the wheel), clamped to the content width;
    /// the layout shifts by the scroll offset (the tiles and every hit rect with them).
    #[test]
    fn horizontal_scroll_clamps_and_shifts_tiles() {
        let mut app = app();
        crate::playlist::run(
            &mut app,
            crate::playlist::Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi),
        );
        app.plugins = crate::plugins::Plugins::builtins();
        // Three tiles are wider than a narrow panel.
        app.state.screen.0 = 600.0;
        let strip = {
            let tr = app.state.project.tracks.last().unwrap().clone();
            tr.mixer_strip
        };
        let m = crate::mixer::track_mut(&mut app.state, strip).unwrap();
        m.slots[0] = Some(os_model::SlotPlugin { id: os_model::Id::NONE, name: "Lemon EQ2".into(), state: Vec::new() });
        m.slots[1] = Some(os_model::SlotPlugin { id: os_model::Id::NONE, name: "Lemon Reverb".into(), state: Vec::new() });
        assert!(crate::views::device_rack::content_w(&app.state) > 600.0, "the chain overflows");
        // Wheel down (right) twice, then far beyond: clamped to the content width.
        let r = crate::views::device_rack::panel_rect(&app.state, 600.0, app.state.screen.1);
        let (x, y) = (r.x + 50.0, r.y + crate::device_rack::HEADER_H + 20.0);
        assert!(crate::views::device_rack::wheel(&mut app.state, r, x, y, -1.0), "the wheel is over the body");
        let one = app.state.device_rack_scroll_x;
        assert!(one > 0.0, "a wheel notch down scrolls right");
        crate::views::device_rack::wheel(&mut app.state, r, x, y, -100.0);
        let max = crate::views::device_rack::scroll_x(&app.state, 600.0);
        assert!(app.state.device_rack_scroll_x <= max + 0.001, "clamped to the content");
        assert!((max - (crate::views::device_rack::content_w(&app.state) - 600.0)).abs() < 0.001);
        // The wheel off the rack's body does not scroll it.
        app.state.device_rack_scroll_x = 0.0;
        assert!(!crate::views::device_rack::wheel(&mut app.state, r, x, r.y - 5.0, 1.0));
        assert_eq!(app.state.device_rack_scroll_x, 0.0);
        // The layout shifts by the scroll: tiles land one offset left of their unscrolled x.
        app.state.device_rack_scroll_x = one;
        let shifted = crate::views::device_rack::layout(&app.state, r);
        app.state.device_rack_scroll_x = 0.0;
        let plain = crate::views::device_rack::layout(&app.state, r);
        assert_eq!(shifted.len(), plain.len());
        for ((_, a), (_, b)) in shifted.iter().zip(&plain) {
            assert!((a.x - (b.x - one)).abs() < 0.001, "tiles shift by the scroll");
            assert!((a.y - b.y).abs() < 0.001);
        }
        // The steppers move the scroll a tile at a time, and stop at 0.
        run(&mut app, RackAct::ScrollBy(1.0));
        let stepped = app.state.device_rack_scroll_x;
        assert!(stepped >= one - 0.001, "the › stepper scrolled right");
        run(&mut app, RackAct::ScrollBy(-10.0));
        assert_eq!(app.state.device_rack_scroll_x, 0.0, "the ‹ stepper stops at the start");
    }

    /// The tile's parameter rows set through the one path (`set_param_value` → `Plugins::set_param`): with an
    /// instance hosted, a ParamSet (click or drag move) moves the plugin's parameter; with none, it is a no-op.
    #[test]
    fn tile_params_set_through_one_path() {
        let mut app = app();
        app.plugins = crate::plugins::Plugins::builtins();
        // Nothing hosted under key 0: the set is a no-op, the app stays up.
        run(&mut app, RackAct::ParamSet { key: 0, id: 0, v: 0.5 });
        #[cfg(plugin_windows)]
        {
            crate::playlist::run(
                &mut app,
                crate::playlist::Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi),
            );
            let tr = app.state.project.tracks.last().unwrap().clone();
            let row = app
                .state
                .project
                .channels
                .iter()
                .position(|c| Some(c.index) == tr.channel)
                .expect("the track's channel");
            app.state.project.channels[row].plugin = Some(crate::plugins::builtin()[0].desc.name.clone());
            let index = app
                .plugins
                .index_of(&crate::plugins::builtin()[0].desc.name)
                .expect("the built-in is available");
            app.plugins.create_with(row, index, None).expect("hosted");
            let Some(info) = app.plugins.param_infos(row).first().cloned() else {
                panic!("the hosted instance has parameters");
            };
            run(&mut app, RackAct::ParamSet { key: row, id: info.id, v: 1.0 });
            assert_eq!(
                app.plugins.param_value(row, info.id),
                Some(1.0),
                "the parameter moved to the set value"
            );
            run(&mut app, RackAct::ParamSet { key: row, id: info.id, v: 2.0 });
            assert_eq!(
                app.plugins.param_value(row, info.id),
                Some(1.0),
                "out-of-range values clamp to 1.0"
            );
        }
    }

    /// Tile drag and drop: the drop gap follows the pointer (`drop_index`), and a release reorders — an FX
    /// tile lands at any of its strip's slots, a MIDI FX among the MIDI FX slots (never past the instrument),
    /// and the drag state clears.
    #[test]
    fn drag_reorders_with_drop_highlight_state() {
        let mut app = app();
        crate::playlist::run(
            &mut app,
            crate::playlist::Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi),
        );
        let tr = app.state.project.tracks.last().unwrap().clone();
        let strip = tr.mixer_strip;
        let m = crate::mixer::track_mut(&mut app.state, strip).unwrap();
        m.slots[0] = Some(os_model::SlotPlugin { id: os_model::Id::NONE, name: "Lemon EQ2".into(), state: Vec::new() });
        m.slots[1] = Some(os_model::SlotPlugin { id: os_model::Id::NONE, name: "Lemon Reverb".into(), state: Vec::new() });
        m.slots[2] = Some(os_model::SlotPlugin { id: os_model::Id::NONE, name: "Lemon Chorus".into(), state: Vec::new() });
        // Two MIDI FX slots in front (their slots keep fake data; the test exe cannot host, the order is the point).
        app.state.midi_fx.entry(tr.id.0).or_default().resize(crate::midi_fx::SLOTS, None);
        app.state.midi_fx.get_mut(&tr.id.0).unwrap()[0] = Some(crate::midi_fx::Slot { name: "A".into(), index: 0, on: true });
        app.state.midi_fx.get_mut(&tr.id.0).unwrap()[1] = Some(crate::midi_fx::Slot { name: "B".into(), index: 0, on: true });
        let tiles = crate::views::device_rack::tiles(&app.state);
        assert_eq!(
            tiles.iter().filter(|(d, _)| matches!(d.kind, DeviceKind::MidiFx(_))).count(),
            2,
            "two MIDI FX tiles first"
        );
        assert_eq!(
            tiles.iter().filter(|(d, _)| matches!(d.kind, DeviceKind::Fx(_, _))).count(),
            3,
            "three FX tiles"
        );
        assert!(matches!(tiles[0].0.kind, DeviceKind::MidiFx(_)), "the MIDI FX lead the chain");
        // The drop gap follows the pointer: over the third tile's middle, the gap is 3.
        let x3 = tiles[3].1.cx();
        assert_eq!(crate::views::device_rack::drop_index(&app.state, x3), 3);
        // A drag starts (TilePress), the highlight state rides the drag, the release clears it.
        run(&mut app, RackAct::TilePress(tiles[5].0.kind));
        assert!(app.state.device_rack_drag.is_some());
        app.state.device_rack_drop = Some(1);
        // Release the last FX (slot 2) left of the first FX's tile (gap 2, before slot 0): they swap.
        let x0 = tiles[2].1.cx();
        app.mouse.0 = x0;
        app.drag = Some(Drag::DeviceRackTile { kind: DeviceKind::Fx(strip, 2), _x0: 0.0 });
        crate::device_rack::tile_drop(&mut app, DeviceKind::Fx(strip, 2), x0);
        assert!(app.state.device_rack_drag.is_none() && app.state.device_rack_drop.is_none(), "the drag cleared");
        let m = crate::mixer::track(&app.state, strip).unwrap();
        assert_eq!(m.slots[0].as_ref().unwrap().name, "Lemon Chorus", "the dragged FX took slot 0");
        // Adjacent SlotMoves: EQ2 shifts to slot 1, Reverb to slot 2.
        assert_eq!(m.slots[1].as_ref().unwrap().name, "Lemon EQ2");
        assert_eq!(m.slots[2].as_ref().unwrap().name, "Lemon Reverb");
        // A MIDI FX drag reorders among the MIDI FX slots only.
        crate::midi_fx::reorder(&mut app, tr.id.0, 1, 0);
        let sl = crate::midi_fx::slots(&app.state, tr.id.0);
        assert_eq!(sl[0].as_ref().unwrap().name, "B", "the second MIDI FX moved to the front");
        assert_eq!(sl[1].as_ref().unwrap().name, "A");
    }

    /// Reorder action: the stub returns Err until the control plane command is wired up.
    #[test]
    fn reorder_command_is_stubbed() {
        let mut app = app();
        // Reorder returns Err (not yet implemented).
        let result = reorder(&mut app.state, 0, 1);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "ReorderDevices command not yet implemented");

        // Running the action through run() does not panic (the error is swallowed).
        run(&mut app, RackAct::Reorder { from: 0, to: 1 });
        // The selection is untouched by a failed reorder.
    }

    /// The device rack's "＋ Add" browser lists the native (built-in) plugins first, then the installed ones, and
    /// exposes the display-mode radios as the filter option for the full plugin hierarchy.
    #[test]
    fn add_device_browser_lists_native_plugins_first() {
        let mut app = app();
        crate::playlist::run(
            &mut app,
            crate::playlist::Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi),
        );
        app.plugins = crate::plugins::Plugins::builtins();
        app.plugins.available.push(crate::plugin_db::fake("Test Synth", os_host::Kind::Instrument));
        app.plugins.effects.push(crate::plugin_db::fake("Test FX", os_host::Kind::Effect));
        app.state.plugdb.rebuild(&app.plugins);
        let s = &app.state;
        let popup = crate::views::device_rack::add_device_menu(s, &app.plugins.midi_fx, 0.0, 0.0, None);
        let labels: Vec<&str> = popup.items.iter().map(|i| i.label.as_ref()).collect();
        // Native plugins must appear before installed plugins (native-first), within each section.
        let at = |name: &str| labels.iter().position(|l| *l == name).unwrap_or_else(|| panic!("{name} missing\nlabels={labels:?}"));
        // The MIDI FX section leads the browser (its picks go to the MIDI FX slots, before the instrument).
        assert_eq!(labels.first().copied(), Some("MIDI FX"), "the MIDI FX section is first\nlabels={labels:?}");
        assert!(at("Lemon Arp") < at("Instruments"), "MIDI FX built-ins listed above the instruments");
        assert!(at("Lemon Sampler") < at("Test Synth"), "native instruments first\nlabels={labels:?}");
        assert!(at("Lemon Reverb") < at("Test FX"), "native effects first\nlabels={labels:?}");
        assert!(at("Instruments") < at("Test Synth") && at("Test Synth") < at("Effects") && at("Effects") < at("Test FX"));
        // Each section groups its built-ins under a "Lemon" header at the very top, before third-party.
        let lemon_after = |head: &str| {
            let h = labels.iter().position(|l| *l == head).unwrap();
            labels[h..].iter().position(|l| *l == "Lemon").unwrap() + h
        };
        assert_eq!(labels[lemon_after("Instruments")], "Lemon", "the instruments' Lemon group");
        assert!(lemon_after("Instruments") < at("Test Synth"), "the Lemon group precedes installed instruments");
        assert_eq!(labels[lemon_after("Effects")], "Lemon", "the effects' Lemon group");
        assert!(lemon_after("Effects") < at("Test FX"), "the Lemon group precedes installed effects");
        // Every built-in must be in the list, before the installed one of its section.
        for a in crate::plugins::builtin().iter() {
            assert!(at(&a.desc.name) < at("Test Synth"), "{} must precede installed instruments", a.desc.name);
        }
        for a in crate::plugins::builtin_effects().iter() {
            assert!(at(&a.desc.name) < at("Test FX"), "{} must precede installed effects", a.desc.name);
        }
        // The footer of the browser holds the display-mode filter radios (Categories / Simple / By vendor).
        let footer = popup.items[popup.items.len() - 3..].to_vec();
        assert!(
            footer.iter().any(|i| i.label == "Categories"),
            "footer must offer the display-mode filter\nfooter={footer:?}"
        );
    }
}
