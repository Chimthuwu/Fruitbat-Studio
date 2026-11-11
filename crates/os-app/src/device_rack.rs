// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The device rack (Lemon Studio's design: spec model/track-model.md §7): a panel docked at the bottom of the main
//! window that holds the devices of the selected track in signal-flow order — a MIDI track's instrument (its
//! channel's plugin) and the mixer strip's effect slots. A device's tile draws the device's own controls in the app's
//! canvas (its parameters, names, sections and value texts — see [`sync_embed`]), and every device's header menu can
//! pop its full editor out to the floating window machinery (Options ▸ Detached's) or dock it back. Only Linear mode
//! shows the rack: Classic mode keeps today's behaviour untouched.
//!
//! ## Signal-flow container seam
//!
//! [`Device`] models a node in a signal-flow chain (track-model-plan.md §9). [`DeviceKind`] variants are:
//! - `Instrument` — a MIDI track's channel plugin
//! - `Effect` — a mixer strip insert slot
//! - `Container` — a container (Lemon Studio's Patcher, `crate::container`): a mixer slot holding a serial
//!   chain of effects, or a channel whose instrument is such a chain. Its place (track and slot, or rack row
//!   and [`crate::container::CHANNEL`]) names it; its members play through `os_engine::chain::Chain`, so the
//!   tile rides the existing engine paths.
//!
//! [`chain`] returns the flat ordered list; the view layer can choose to render a device with children as a
//! nested container rather than a flat tile.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use crate::app::{App, Drag, State};

/// The shortest the rack may be, in logical px: below this its tiles are too short to read their editor in, and a
/// splitter drag down to the workspace edge (which used to be allowed) left a sliver of a panel.
pub const MIN_H: f32 = 180.0;
/// The tallest the rack may be. The splitter used to give it up to half the window, which on a 1080 px screen is
/// 540 px of editor stretched out of shape and less workspace above it; the rack's own frame now stops at this.
pub const MAX_H: f32 = 360.0;
/// The height the rack opens at, and what a double click on its splitter gives back: tall enough for a MIDI FX
/// editor (the tallest is 380 px, the commonest 302) to be drawn at nearly its own size.
pub const DEFAULT_H: f32 = 336.0;
/// Header row (title, pin, collapse), and the collapsed rack's whole height.
pub const HEADER_H: f32 = 23.0;
/// Width of a device tile (rows stack name above bar, so it stays narrow): 4 tiles fit a 1229 px window.
pub const TILE_W: f32 = 280.0;
/// Padding between a tile's edge and the device controls drawn in it.
pub const TILE_PAD: f32 = 6.0;
/// The widest a tile grows to show a wide editor 1:1 (wider ones are scaled down).
pub const TILE_MAX_W: f32 = 560.0;

/// A tile's width: a device whose plugin the rack draws itself (`crate::rack_ui`) gets its editor's width at the
/// size the rack's body shows it (its natural width where it fits, else scaled down), padded; every other device
/// keeps [`TILE_W`]. `name` is the device's plugin name, `body_h` the tile's body height (the rack's less its
/// header), so the tiles reflow when the rack is resized.
pub fn tile_w(name: &str, body_h: f32) -> f32 {
    let Some(ui) = crate::rack_ui::ui(name) else { return TILE_W };
    let (w, h) = ui.size();
    if w <= 0.0 || h <= 0.0 {
        return TILE_W;
    }
    let fit = ((body_h - 2.0 * TILE_PAD) / h).clamp(0.1, 1.0);
    let cap = ((TILE_MAX_W - 2.0 * TILE_PAD) / w).min(1.0);
    (w * fit.min(cap) + 2.0 * TILE_PAD).max(180.0)
}

/// Whether the rack shows at all: Linear mode only (Classic mode = zero visual change).
pub fn visible(s: &State) -> bool {
    s.project.linear_mode && s.show_device_rack
}

/// The rack's tallest height: [`MAX_H`], and never more than half the window on a short screen.
pub fn max_h(s: &State) -> f32 {
    MAX_H.min((s.screen.1 / 2.0).max(MIN_H))
}

/// The rack's height (logical px): 0 when hidden (Classic mode always), the header when collapsed.
pub fn height(s: &State) -> f32 {
    if !visible(s) {
        0.0
    } else if s.device_rack_collapsed {
        HEADER_H
    } else {
        s.device_rack_h.clamp(MIN_H, max_h(s))
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
    /// A MIDI track's empty instrument slot: no channel plays the track yet — the "Add instrument"
    /// placeholder tile (the track is silent until one is picked).
    NoInstrument,
    /// A container (Lemon Studio's Patcher, `crate::container`): mixer slot `k` of strip `t` holding a
    /// serial chain of effects, or for `k` = [`crate::container::CHANNEL`] the channel `t`'s instrument
    /// chain (its rack row's channel plays it).
    Container { t: u16, k: u8 },
}

/// The tile header's gradient, top and bottom (0xRRGGBB), per device kind. Hard-coded until user theming grows
/// per-kind colors — change these pairs to rewire every tile header's look.
pub const HEADER_GRAD_MIDI_FX: (u32, u32) = (0xb9a4ec, 0x8fc7f2); // lavender -> sky blue
pub const HEADER_GRAD_INSTRUMENT: (u32, u32) = (0xb9e98b, 0xf6ec85); // lime -> yellow
pub const HEADER_GRAD_FX: (u32, u32) = (0xf3f4f5, 0x9dc9ee); // white -> sky blue
pub const HEADER_GRAD_CONTAINER: (u32, u32) = (0x9fe8c0, 0x8fc7f2); // mint -> sky blue

/// The header's gradient for `kind` (top, bottom).
pub fn header_gradient(kind: DeviceKind) -> (u32, u32) {
    match kind {
        DeviceKind::MidiFx(_) => HEADER_GRAD_MIDI_FX,
        DeviceKind::Instrument(_) | DeviceKind::NoInstrument => HEADER_GRAD_INSTRUMENT,
        DeviceKind::Fx(..) => HEADER_GRAD_FX,
        DeviceKind::Container { .. } => HEADER_GRAD_CONTAINER,
    }
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

/// One control a device tile draws in its body: a parameter of the device's hosted instance, with everything the
/// tile needs to show it the way the plugin's own editor does.
#[derive(Debug, Clone, PartialEq)]
pub struct RackParam {
    /// The plugin's parameter id (`Plugins::set_param`).
    pub id: u32,
    /// The parameter's name ("Semitones").
    pub name: String,
    /// The plugin's section for it ("Envelope"), empty when the plugin names none.
    pub group: String,
    /// The normalized value, 0..=1.
    pub value: f64,
    /// The same value in the plugin's own unit (`min + (max - min) * value`): what the parts of an editor that read
    /// plain values use — a selector's row, a switch, the reverb's room visual.
    pub plain: f64,
    /// The plugin's plain range for the parameter.
    pub min: f64,
    pub max: f64,
    /// The plugin's own text for the value ("Mixolydian", "+0 st", "On").
    pub text: String,
    /// A two-state parameter: drawn as a switch, not a dial.
    pub toggle: bool,
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
        // A channel whose instrument is a container chain (`Wrap in container` on the channel) is a container
        // device, not an instrument: its members are the chain (crate::container, C1).
        if crate::container::is_container_channel(ch) {
            v.push(Device {
                kind: DeviceKind::Container { t: row as u16, k: crate::container::CHANNEL },
                key: None,
                name: crate::container::title(s, row as u16, crate::container::CHANNEL)
                    .unwrap_or_else(|| crate::container::CONTAINER.to_string()),
                on: ch.enabled,
                builtin: false,
            });
        } else {
            v.push(Device {
                kind: DeviceKind::Instrument(row),
                key: Some(row),
                name: crate::rack::channel_name(s, row),
                on: ch.enabled,
                builtin,
            });
        }
    } else if tr.kind == os_model::TrackKind::Midi {
        // The empty instrument slot: a placeholder tile whose click opens the device browser (§7 — a MIDI
        // track may be silent until an instrument is picked, e.g. the default sampler was removed).
        v.push(Device {
            kind: DeviceKind::NoInstrument,
            key: None,
            name: "Add instrument".into(),
            on: false,
            builtin: false,
        });
    }
    if let Some(m) = s.project.mixer.iter().find(|m| m.index == tr.mixer_strip) {
        for (k, slot) in m.slots.iter().enumerate() {
            let Some(sp) = slot else { continue };
            // A container slot (the browser's container entry, or Wrap all in container): its own tile kind
            // (C1) — its members play through the existing engine chain.
            if sp.name == crate::container::CONTAINER {
                v.push(Device {
                    kind: DeviceKind::Container { t: m.index, k: k as u8 },
                    key: None,
                    name: crate::container::title(s, m.index, k as u8)
                        .unwrap_or_else(|| crate::container::CONTAINER.to_string()),
                    on: m.slot_on[k],
                    builtin: false,
                });
                continue;
            }
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

/// The last selected tile index in the rack, if any (the selection is a set, `State::device_rack_sels`; the
/// keyboard and Delete act on the last one).
pub fn rack_sel(s: &State) -> Option<usize> {
    s.device_rack_sels.last().copied()
}

/// Whether chain tile `i` is selected (a header click paints it brighter).
pub fn tile_selected(s: &State, i: usize) -> bool {
    s.device_rack_sels.contains(&i)
}

/// Selects a tile by a header click: plain selects it alone, `extend` (a click with Shift held, the anchor
/// being the last plain click) selects every tile from the anchor to `i` — the playlist's track headers'
/// pattern (`playlist::select_track`).
pub fn select_tile(s: &mut State, i: usize, extend: bool) {
    let anchor = s.device_rack_anchor.or_else(|| s.device_rack_sels.last().copied());
    if extend && let Some(a) = anchor {
        for j in a.min(i)..=a.max(i) {
            s.device_rack_sels.insert(j);
        }
        return;
    }
    s.device_rack_sels = BTreeSet::from([i]);
    s.device_rack_anchor = None;
}

/// Sets the selection to just tile `i` (the arrow keys: a multi-selection collapses to the tile moved to).
pub fn set_sel(s: &mut State, i: usize) {
    s.device_rack_sels = BTreeSet::from([i]);
    s.device_rack_anchor = None;
}

/// Clears the selection (a device removal does).
pub fn clear_sel(s: &mut State) {
    s.device_rack_sels.clear();
    s.device_rack_anchor = None;
}

/// A cut device (Ctrl+X), pasted with Ctrl+V: an effect (its name, the plugin it came from, its saved state
/// and slot switches) or a MIDI FX (its slot and index).
#[derive(Debug, Clone, PartialEq)]
pub enum Clip {
    Fx { name: String, index: usize, state: Vec<u8>, on: bool },
    MidiFx { index: usize, on: bool },
}

/// Reorders the device at chain index `from` to chain index `to` in the rack's chain. The move stays within the
/// device's segment: a MIDI FX reorders among the MIDI FX slots (never past the instrument), an effect reorders
/// among its strip's slots with adjacent `SlotMove`s (the same path the drag uses, so the plugin instances,
/// automation and session UI all follow), and the instrument stays — a track holds one. Returns `Ok` on a completed
/// move or a no-op (a target the device cannot land on), and `Err` when there is no device at `from`.
pub fn reorder(app: &mut App, from: usize, to: usize) -> Result<(), String> {
    let chain = rack_chain(&app.state);
    let Some(src) = chain.get(from) else {
        return Err(format!("no device at chain index {from}"));
    };
    match src.kind {
        DeviceKind::MidiFx(_) => {
            let Some(tr) = rack_track(&app.state).map(|t| t.id.0) else {
                return Err("no rack track".into());
            };
            let n = crate::midi_fx::filled(&app.state, tr).len();
            // The MIDI FX lead the chain, so `from` is their slot index. A target at or past the last slot
            // (the instrument's position or beyond) is not a slot a MIDI FX can land in.
            if from >= n || to >= n || from == to {
                return Ok(());
            }
            crate::midi_fx::reorder(app, tr, from, to);
            Ok(())
        }
        DeviceKind::Fx(track, slot) => {
            let fx: Vec<u8> = chain
                .iter()
                .filter_map(|d| match d.kind {
                    DeviceKind::Fx(t, k) if t == track => Some(k),
                    _ => None,
                })
                .collect();
            let Some(src_i) = fx.iter().position(|&k| k == slot) else {
                return Err("effect slot not in the rack chain".into());
            };
            // The drop position among this strip's FX: how many of its slots sit before chain index `to`.
            let mut to_i = chain[..to.min(chain.len())]
                .iter()
                .filter(|d| matches!(d.kind, DeviceKind::Fx(t, _) if t == track))
                .count();
            if to_i > src_i {
                to_i -= 1; // removing the source shifts the later slots left by one
            }
            if to_i == src_i {
                return Ok(());
            }
            let target = fx.get(to_i).copied().unwrap_or_else(|| *fx.last().unwrap_or(&slot));
            if target == slot {
                return Ok(());
            }
            // Adjacent SlotMoves (the same path the drag uses) so the plugin instances, automation and session
            // UI all follow the reorder.
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
            Ok(())
        }
        DeviceKind::Instrument(_) | DeviceKind::NoInstrument => Ok(()),
        DeviceKind::Container { .. } => Ok(()),
    }
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
    /// Select a device tile by its index in the chain: a plain header click selects it alone, Shift+click
    /// extends the selection to it (the anchor pattern of the playlist's track headers).
    SelectTile(usize),
    /// A container tile's wrench (or body): open its container window (`crate::container`, the docked
    /// `Win::Container`).
    OpenContainer { t: u16, k: u8 },
    /// A device header's menu: open it for `key`'s device at the mouse.
    Menu(usize),
    /// Dismiss an open device menu.
    MenuClose,
    /// A device's Pop out: its editor leaves the tile for the floating window machinery.
    PopOut(usize),
    /// The header's wrench: every device in the chain shows its editor in its own floating window (lit green while
    /// they are out), or the windows close and the tiles draw the editors again.
    ToggleEditors,
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
    /// A tile body's control pressed: set the parameter `id` of the instance under `key` to normalized `v` and drag
    /// it from there (a dial, a fader, a value bar).
    ParamSet { key: usize, id: u32, v: f64 },
    /// A tile body's switch, selector row or radio pressed: set the parameter to normalized `v` once, with no drag
    /// (`None` picks nothing).
    ParamTap { key: usize, id: u32, v: Option<f64> },
    /// The device menu's Remove: delete the device from its chain.
    Remove(DeviceKind),
    /// The device menu's Replace (the instrument): the device browser, whose instrument pick replaces it.
    Replace,
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
/// bottom edge: the rack's height follows the pointer. Whole px, from [`MIN_H`] to [`max_h`].
pub fn split_drag(app: &mut App, y: f32, dy: f32) {
    let s = &mut app.state;
    let h = (y - dy).clamp(MIN_H, max_h(s)).round();
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
    app.plugins.set_detached(key, true);
    // An editor that was not open yet opens detached now; one that was moves out of its tile.
    #[cfg(plugin_windows)]
    if app.plugins.window_of(key).is_none() {
        let _ = open_editor(app, key);
    }
    app.state.device_rack_menu = None;
    app.invalidate();
}

/// A device's Dock: `key`'s floating editor window closes and its tile draws the device's controls again (the tile
/// always draws them; the window is what comes and goes).
pub fn dock(app: &mut App, key: usize) {
    app.state.device_rack_float.remove(&key);
    app.plugins.set_detached(key, false);
    app.plugins.close_if_open(key);
    app.state.device_rack_menu = None;
    app.invalidate();
}

/// The header's wrench: every device in the chain shows its editor in its own floating window (the editors open
/// where they were not yet open), or — when any editor is already out — they all dock back into their tiles.
pub fn toggle_editors(app: &mut App) {
    if app.state.device_rack_float.is_empty() {
        let keys: Vec<usize> = rack_chain(&app.state).iter().filter_map(|d| d.key).collect();
        for key in keys {
            pop_out(app, key);
        }
    } else {
        let floated: Vec<usize> = app.state.device_rack_float.iter().copied().collect();
        for key in floated {
            dock(app, key);
        }
    }
    app.invalidate();
}

/// Runs a rack action.
pub fn run(app: &mut App, act: RackAct) {    match act {
        RackAct::Splitter => splitter_press(app),
        RackAct::TogglePin => toggle_pin(app),
        RackAct::ToggleCollapsed => {
            app.state.device_rack_collapsed = !app.state.device_rack_collapsed;
            app.invalidate();
        }
        RackAct::SelectTile(idx) => {
            // A header click selects the tile (Shift extends a range, the playlist's track-header pattern);
            // the press also starts the drag, as the old header press did, so a click still reorders.
            let extend = app.shift;
            if let Some(d) = rack_chain(&app.state).get(idx).cloned() {
                select_tile(&mut app.state, idx, extend);
                if !extend {
                    app.state.device_rack_drag = Some(d.kind);
                    app.state.device_rack_drop = None;
                    app.drag = Some(Drag::DeviceRackTile { kind: d.kind, _x0: app.mouse.0 });
                }
            }
            app.invalidate();
        }
        RackAct::OpenContainer { t, k } => {
            crate::container::show(&mut app.state, t, k);
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
        RackAct::ToggleEditors => toggle_editors(app),
        RackAct::Dock(key) => dock(app, key),
        RackAct::ToggleOn(kind) => toggle_on(app, kind),
        RackAct::Add => {
            // The header ＋ opens the same track-kind-gated device browser the gaps do (Target::All's generator
            // picks would add an orphan channel to the channel rack, not this track's instrument).
            app.open_plugin_palette(crate::plugin_menu::Target::Rack(None), app.mouse, None);
        }
        RackAct::AddAt(slot) => {
            // Every ＋ opens the palette-style device browser (the rack's Target::Rack): MIDI FX first, then
            // instruments and effects — an effect pick lands in `slot`, MIDI FX in the MIDI FX slots, an
            // instrument on the track's channel.
            app.open_plugin_palette(crate::plugin_menu::Target::Rack(slot), app.mouse, None);
        }
        RackAct::TilePress(kind) => {
            app.state.device_rack_drag = Some(kind);
            app.state.device_rack_drop = None;
            app.drag = Some(Drag::DeviceRackTile { kind, _x0: app.mouse.0 });
        }
        RackAct::AddDevice => {
            app.open_plugin_palette(crate::plugin_menu::Target::Rack(None), app.mouse, None);
        }
        RackAct::Reorder { from, to } => {
            let _ = reorder(app, from, to);
            app.invalidate();
        }
        RackAct::ParamSet { key, id, v } => {
            // Sets the value at once (a bar's click, or the dial's start value) and keeps dragging it: the
            // drag's move maps y onto a vertical dial drag.
            set_param_value(app, key, id, v);
            app.drag = Some(Drag::DeviceRackParam { key, id, y0: app.mouse.1, v0: v });
        }
        RackAct::ParamTap { key, id, v } => {
            if let Some(v) = v {
                set_param_value(app, key, id, v);
            }
        }
        RackAct::Remove(kind) => {
            app.state.device_rack_menu = None;
            remove_device(app, kind);
        }
        RackAct::Replace => {
            app.state.device_rack_menu = None;
            app.open_plugin_palette(crate::plugin_menu::Target::Rack(None), app.mouse, None);
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
        // The empty slot has nothing to switch.
        DeviceKind::NoInstrument => {}
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
        // A container's enable light is its slot's (or the channel's) on/off: the container window's LED and
        // the mixer's slot LED are the same switch.
        DeviceKind::Container { t, k } => {
            if k == crate::container::CHANNEL {
                if let Some(ch) = s.project.channels.get_mut(usize::from(t)) {
                    ch.enabled = !ch.enabled;
                    s.piano.dirty = true;
                }
            } else if let Some(m) = crate::mixer::track_mut(s, t) {
                m.slot_on[usize::from(k)] = !m.slot_on[usize::from(k)];
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
        // The tile's own control follows the drag at once: its value, and the plugin's text for it read for that one
        // parameter (re-reading a big instrument's whole list per moved pixel costs the audio thread its lock: Vital).
        let text = app.plugins.param_text(key, id, v);
        if let Some(p) = app.state.device_rack_params.get_mut(&key).and_then(|ps| ps.iter_mut().find(|p| p.id == id)) {
            p.value = v;
            p.plain = p.min + (p.max - p.min) * v;
            if let Some(text) = text {
                p.text = text;
            }
        }
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
        DeviceKind::Instrument(_) | DeviceKind::NoInstrument => {
            // A track holds one instrument: nothing to reorder it with (its channel is the track's); the
            // empty slot's placeholder is not draggable.
        }
        DeviceKind::Container { .. } => {
            // A container stays where its slot (or channel) is; its members reorder in its own window.
        }
    }
    app.invalidate();
}

/// Deletes the selected devices (Delete key): each selected tile goes through [`remove_device`].
pub fn delete_selected(app: &mut App) {
    let sels: Vec<usize> = app.state.device_rack_sels.iter().copied().collect();
    if sels.is_empty() {
        return;
    }
    let chain = rack_chain(&app.state);
    // Right to left: a removal changes nothing to the left of it.
    for i in sels.iter().rev() {
        let Some(d) = chain.get(*i).cloned() else { continue };
        remove_device(app, d.kind);
    }
    clear_sel(&mut app.state);
}

/// Removes a device from its chain by the paths the rest of the app uses: an effect goes through the control
/// plane's `RemoveEffect` (undoable), a MIDI FX through `midi_fx::remove`, the instrument deletes its channel (with
/// the delete confirmation; its notes go with it) leaving the empty "Add instrument" slot. A container (a slot's or
/// a channel's) goes through its own kind's path: the slot's container is an effect to the control plane, a
/// channel's container deletes its channel like an instrument does.
pub fn remove_device(app: &mut App, kind: DeviceKind) {
    match kind {
        DeviceKind::Fx(strip, slot) => remove_fx(app, strip, slot),
        DeviceKind::Container { t: strip, k: slot } => {
            if slot == crate::container::CHANNEL {
                // A channel's container is the channel's instrument: removing it deletes the channel, as an
                // instrument tile does (its notes go with it, the confirmation applies).
                crate::rack::ask_delete(&mut app.state, vec![usize::from(strip)]);
            } else {
                remove_fx(app, strip, slot);
            }
        }
        DeviceKind::MidiFx(slot) => {
            if let Some(id) = rack_track(&app.state).map(|tr| tr.id.0) {
                app.state.device_rack_params.remove(&crate::plugins::midi_fx_key(id, slot));
                crate::midi_fx::remove(app, id, slot);
            }
        }
        DeviceKind::Instrument(row) => crate::rack::ask_delete(&mut app.state, vec![row]),
        DeviceKind::NoInstrument => {}
    }
    clear_sel(&mut app.state);
    app.invalidate();
}

/// Removes the effect (or the container) in slot `slot` of strip `strip` through the control plane's
/// `RemoveEffect`: the same undoable path the mixer's Delete uses.
fn remove_fx(app: &mut App, strip: u16, slot: u8) {
    let Some(id) = crate::mixer::track(&app.state, strip).map(|m| m.id.0) else { return };
    let key = crate::plugins::fx_key(strip, slot);
    app.plugins.drop_key(key);
    app.state.device_rack_params.remove(&key);
    let cmd = crate::control::command::Command::RemoveEffect { track: id, slot };
    if let Err(e) = crate::control::apply(&mut app.state, &crate::control::By::User, None, vec![cmd]) {
        app.state.message = Some(format!("Remove: {e}"));
    }
}

/// The rack's Ctrl keys while a tile is selected (app.rs's rack block): D duplicates the selected device, X
/// cuts it, V pastes the cut device, G groups the selected effects into a container (§7's group device).
pub fn key(app: &mut App, c: &str) -> bool {
    match c.to_ascii_lowercase().as_str() {
        "d" => duplicate_selected(app),
        "x" => cut_selected(app),
        "v" => paste_clip(app),
        "g" => group_selected(app),
        _ => false,
    }
}

/// What a cut needs from an effect: its shown name, its plugin's index (for hosting), its saved state and
/// whether its slot is on.
fn clip_fx(app: &App, t: u16, k: u8) -> Option<(String, usize, Vec<u8>, bool)> {
    let m = crate::mixer::track(&app.state, t)?;
    let sp = m.slots[usize::from(k)].as_ref()?;
    let name = crate::project_plugins::slot_title(sp);
    let index = app.plugins.effects.iter().position(|a| a.desc.name == name || sp.name == a.desc.name)?;
    let state = app
        .plugins
        .hosted
        .get(&crate::plugins::fx_key(t, k))
        .and_then(|h| h.plugin.lock().ok().and_then(|p| p.save_state()))
        .unwrap_or_else(|| sp.state.clone());
    Some((name, index, state, m.slot_on[usize::from(k)]))
}

/// Adds an effect `index` into slot `k` of strip `t`, hosting it from `state` (the cut device's saved state,
/// not its defaults): [`MCmd::AddEffect`]'s own path, its request's state patched in.
fn add_fx_with_state(app: &mut App, t: u16, k: u8, index: usize, state: Vec<u8>) {
    let prev = app.state.selected_mixer;
    app.state.selected_mixer = t;
    crate::mixer::run(app, crate::mixer::MCmd::AddEffect(k, index));
    // The slot's request now waits in `pending`; it starts from the cut state.
    if let Some(r) = app.state.mixer.pending.last_mut() {
        if let crate::mixer::FxReq::Create { state: st, .. } = r {
            *st = Some(state);
        }
    }
    app.state.selected_mixer = prev;
}

/// Ctrl+D: the selected device is duplicated — an effect into the first free slot after its own (with its
/// state), a MIDI FX into the next free MIDI FX slot; the instrument and the containers are no-ops (a track
/// holds one instrument, a container's members are its own).
pub fn duplicate_selected(app: &mut App) -> bool {
    let Some(sel) = rack_sel(&app.state) else { return false };
    let Some(d) = rack_chain(&app.state).get(sel).cloned() else { return false };
    match d.kind {
        DeviceKind::Fx(t, k) => {
            let Some((name, index, state, _on)) = clip_fx(app, t, k) else { return false };
            let free = (k + 1..10).find(|&i| crate::mixer::track(&app.state, t).is_some_and(|m| m.slots[usize::from(i)].is_none()));
            let Some(k2) = free else {
                app.state.message = Some("No free slot after this device to duplicate into".into());
                return true;
            };
            add_fx_with_state(app, t, k2, index, state);
            app.state.message = Some(format!("{name} duplicated in slot {}", k2 + 1));
            true
        }
        DeviceKind::MidiFx(k) => {
            let Some(id) = rack_track(&app.state).map(|tr| tr.id.0) else { return false };
            let Some(sl) = crate::midi_fx::slots(&app.state, id).get(k).cloned().flatten() else { return false };
            let free = (0..crate::midi_fx::SLOTS).find(|&i| crate::midi_fx::slots(&app.state, id)[i].is_none());
            let Some(i) = free else {
                app.state.message = Some("No free MIDI FX slot to duplicate into".into());
                return true;
            };
            crate::midi_fx::add(app, id, Some(i), sl.index);
            app.state.message = Some(format!("{} duplicated", sl.name));
            true
        }
        _ => {
            app.state.message = Some("Only effects and MIDI FX can be duplicated".into());
            true
        }
    }
}

/// Ctrl+X: the selected device is cut — copied (its state with it) and removed ([`remove_device`]).
pub fn cut_selected(app: &mut App) -> bool {
    let Some(sel) = rack_sel(&app.state) else { return false };
    let Some(d) = rack_chain(&app.state).get(sel).cloned() else { return false };
    let clip = match d.kind {
        DeviceKind::Fx(t, k) => match clip_fx(app, t, k) {
            Some((name, index, state, on)) => Clip::Fx { name, index, state, on },
            None => return false,
        },
        DeviceKind::MidiFx(k) => {
            let Some(id) = rack_track(&app.state).map(|tr| tr.id.0) else { return false };
            let Some(sl) = crate::midi_fx::slots(&app.state, id).get(k).cloned().flatten() else { return false };
            Clip::MidiFx { index: sl.index, on: sl.on }
        }
        _ => {
            app.state.message = Some("Only effects and MIDI FX can be cut".into());
            return true;
        }
    };
    app.state.device_rack_clip = Some(clip);
    app.state.message = Some(format!("{} cut", d.name));
    remove_device(app, d.kind);
    true
}

/// Ctrl+V: the cut device is pasted — an effect after the selected device's slot (else the strip's first free
/// slot), from the state it was cut with; a MIDI FX into the next free MIDI FX slot.
pub fn paste_clip(app: &mut App) -> bool {
    let Some(clip) = app.state.device_rack_clip.clone() else {
        app.state.message = Some("Nothing cut to paste".into());
        return true;
    };
    match clip {
        Clip::Fx { name, index, state, on } => {
            let Some(t) = rack_track(&app.state).map(|tr| tr.mixer_strip) else { return false };
            // After the selected device's slot when it is one of this strip's effects, else from the start.
            let from = match rack_sel(&app.state).and_then(|i| rack_chain(&app.state).get(i).map(|d| d.kind)) {
                Some(DeviceKind::Fx(t2, k)) if t2 == t => k + 1,
                _ => 0,
            };
            let free = (from..10).find(|&i| crate::mixer::track(&app.state, t).is_some_and(|m| m.slots[usize::from(i)].is_none()));
            let Some(k) = free else {
                app.state.message = Some("No free slot of this track to paste into".into());
                return true;
            };
            add_fx_with_state(app, t, k, index, state);
            if !on
                && let Some(m) = crate::mixer::track_mut(&mut app.state, t)
            {
                m.slot_on[usize::from(k)] = false;
            }
            app.state.message = Some(format!("{name} pasted in slot {}", k + 1));
            true
        }
        Clip::MidiFx { index, on } => {
            let Some(id) = rack_track(&app.state).map(|tr| tr.id.0) else { return false };
            let Some(i) = (0..crate::midi_fx::SLOTS).find(|&i| crate::midi_fx::slots(&app.state, id)[i].is_none()) else {
                app.state.message = Some("No free MIDI FX slot to paste into".into());
                return true;
            };
            crate::midi_fx::add(app, id, Some(i), index);
            if !on {
                crate::midi_fx::toggle(&mut app.state, id, i);
            }
            true
        }
    }
}

/// Ctrl+G: the selected devices are grouped into a container — the effects of one strip become the members of
/// a container in the first one's slot ([`crate::container::group_slots`]), the instrument alone becomes the
/// channel's container ([`crate::container::wrap_channel`], the instrument rack). A container is not grouped
/// into another container, and the MIDI FX rack is the note-path milestone's.
pub fn group_selected(app: &mut App) -> bool {
    let sels: Vec<usize> = app.state.device_rack_sels.iter().copied().collect();
    if sels.is_empty() {
        app.state.message = Some("Select devices to group".into());
        return true;
    }
    let kinds: Vec<DeviceKind> = sels.iter().filter_map(|&i| rack_chain(&app.state).get(i).map(|d| d.kind)).collect();
    if kinds.len() != sels.len() {
        app.state.message = Some("Select devices to group".into());
        return true;
    }
    if kinds.iter().any(|k| matches!(k, DeviceKind::NoInstrument | DeviceKind::Container { .. })) {
        app.state.message = Some("A container cannot be grouped into another container".into());
        return true;
    }
    if kinds.iter().all(|k| matches!(k, DeviceKind::MidiFx(_))) {
        app.state.message = Some("The MIDI FX rack comes with the note-path milestone: MIDI FX cannot be grouped yet".into());
        return true;
    }
    if kinds.len() == 1
        && let DeviceKind::Instrument(row) = kinds[0]
    {
        let msg = app.wrap_channel(row);
        app.state.message = msg;
        clear_sel(&mut app.state);
        app.invalidate();
        return true;
    }
    if kinds.iter().all(|k| matches!(k, DeviceKind::Fx(..))) {
        let strips: BTreeSet<u16> = kinds.iter().map(|k| match k {
            DeviceKind::Fx(t, _) => *t,
            _ => unreachable!("checked above"),
        }).collect();
        if strips.len() != 1 {
            app.state.message = Some("Group the effects of one track".into());
            return true;
        }
        let slots: Vec<u8> = kinds.iter().map(|k| match k {
            DeviceKind::Fx(_, s) => *s,
            _ => unreachable!("checked above"),
        }).collect();
        let msg = app.group_slots(*strips.iter().next().unwrap(), &slots);
        app.state.message = msg;
        clear_sel(&mut app.state);
        app.invalidate();
        return true;
    }
    app.state.message = Some("Group the effects of one track, or the instrument alone".into());
    true
}

/// Keeps the rack's tiles showing their devices' own controls. Called from `clip_docked` (every tick).
///
/// The tile draws the controls itself, in the app's canvas: the device's own editor in miniature, with the same
/// parameter names, sections and value texts. It does not embed a native plugin window — pinning the editor's frame
/// into the tile never painted reliably (a tile stayed flat until its editor was popped out, and only then showed
/// controls, because the tile had fallen back to drawing them), so the rack now draws them from the start.
pub fn sync_embed(app: &mut App) {
    if !visible(&app.state) || app.state.device_rack_collapsed {
        app.state.device_rack_params.clear();
        app.state.device_rack_names.clear();
        app.state.device_rack_tile_scroll.clear();
        app.state.rack_ui_cache.borrow_mut().retain(&[]);
        return;
    }
    // Every device tile's instance, hosted or built-in: its parameter list is read straight off the instance, so a
    // tile never needs its editor opened to show (and set) its controls.
    let keys = crate::views::device_rack::device_keys(&app.state);
    app.state.device_rack_tile_scroll.retain(|k, _| keys.contains(k));
    app.state.rack_ui_cache.borrow_mut().retain(&keys);
    let mut params = std::collections::BTreeMap::new();
    let mut names = std::collections::BTreeMap::new();
    for &key in &keys {
        // The hosted plugin's own name, so the tile finds the editor it draws (a slot's stored title may be the
        // FL plugin's, "Fruity Filter").
        if let Some(d) = app.plugins.descriptor(key) {
            names.insert(key, d.name);
        }
        let prev = app.state.device_rack_params.get(&key).cloned().unwrap_or_default();
        // A tile re-reads its instance when the instance has something to say (a value moved in its own window) or
        // every 40 ms, not on every tick: reading a big instrument's ~900 values 60 times a second held the lock its
        // audio thread wanted (Vital distorted while the rack showed its track). A tile with nothing yet is read now.
        if !prev.is_empty() && !app.plugins.rack_due(key) {
            params.insert(key, prev);
            continue;
        }
        let rows = app.plugins.rack_controls(key, &prev);
        if !rows.is_empty() {
            params.insert(key, rows);
        }
    }
    app.state.device_rack_params = params;
    app.state.device_rack_names = names;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{Action, App};
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
        app.state.plugdb.rebuild(&app.plugins);
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
            assert!(app.plugins.window_of(row).is_some(), "popped out: its window is open");
            run(&mut app, RackAct::Dock(row));
            assert!(!app.plugins.hosted[&row].detached, "docked back");
            assert!(!app.state.device_rack_float.contains(&row));
            assert!(
                app.plugins.window_of(row).is_none(),
                "docked: the floating window closes, the tile draws the device's controls"
            );
        }
    }

    /// Each tile header's gradient is keyed by its kind (MIDI FX lavender→sky, instrument lime→yellow, audio FX
    /// white→sky), as named constants so user theming can rewire them later.
    #[test]
    fn tile_header_gradients_are_keyed_by_kind() {
        let (mt, mb) = header_gradient(DeviceKind::MidiFx(0));
        let (it, ib) = header_gradient(DeviceKind::Instrument(0));
        let (ft, fb) = header_gradient(DeviceKind::Fx(1, 0));
        assert_eq!((mt, mb), (HEADER_GRAD_MIDI_FX.0, HEADER_GRAD_MIDI_FX.1));
        assert_eq!((it, ib), (HEADER_GRAD_INSTRUMENT.0, HEADER_GRAD_INSTRUMENT.1));
        assert_eq!((ft, fb), (HEADER_GRAD_FX.0, HEADER_GRAD_FX.1));
        assert_ne!(mt, mb, "each pair is a gradient, not a flat fill");
        assert_ne!(it, ib);
        assert_ne!(ft, fb);
        // The three kinds are visually distinct from each other.
        assert_ne!((mt, mb), (it, ib));
        assert_ne!((mt, mb), (ft, fb));
        assert_ne!((it, ib), (ft, fb));
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
        assert!(height(&app.state) >= DEFAULT_H.min(max_h(&app.state)));
    }

    /// The rack's height stays between [`MIN_H`] and [`MAX_H`] however far the splitter is dragged (it used to
    /// reach half the window and shrink to a sliver), and a short window pulls the tallest height down with it.
    #[test]
    fn the_splitter_stays_within_the_racks_bounds() {
        let mut app = app();
        crate::playlist::run(
            &mut app,
            Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi),
        );
        assert_eq!(height(&app.state), DEFAULT_H);
        split_drag(&mut app, 0.0, 0.0);
        assert_eq!(height(&app.state), MIN_H, "dragged to the top of the workspace");
        let bottom = app.state.screen.1;
        split_drag(&mut app, bottom, 0.0);
        assert_eq!(height(&app.state), MAX_H, "dragged to the bottom edge");
        // Half the window still wins on a short screen.
        app.state.screen.1 = 500.0;
        assert_eq!(max_h(&app.state), 250.0);
        app.state.device_rack_h = 400.0;
        assert_eq!(height(&app.state), 250.0);
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
        app.state.plugdb.rebuild(&app.plugins);
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

    /// A hosted (non-builtin) device tile gets parameter rows without its editor ever being opened: the params
    /// cache (`device_rack_params`) covers every keyed tile's instance, built-in or not. A fake effect with one
    /// parameter stands in for the hosted instance.
    #[test]
    fn hosted_tile_gets_param_rows() {
        struct P;
        impl os_engine::Instrument for P {
            fn note_on(&mut self, _: u32, _: u8, _: f32) {}
            fn note_off(&mut self, _: u32, _: u8) {}
            fn process(&mut self, _: u32, _: &mut [f32], _: &mut [f32]) {}
        }
        impl os_engine::HostedEffect for P {
            fn process_fx(&mut self, _: u32, _: &mut [f32], _: &mut [f32]) {}
        }
        impl os_host::Plugin for P {
            fn desc(&self) -> &os_host::Descriptor {
                static D: std::sync::OnceLock<os_host::Descriptor> = std::sync::OnceLock::new();
                D.get_or_init(|| os_host::Descriptor {
                    format: os_host::Format::Clap,
                    id: "test.fx".into(),
                    name: "Test FX".into(),
                    vendor: "Test".into(),
                    version: "1.0".into(),
                    features: Vec::new(),
                    kind: os_host::Kind::Effect,
                    path: std::path::PathBuf::from("C:/Plugins/Test FX.clap"),
                })
            }
            fn activate(&mut self, _: u32) -> bool {
                true
            }
            fn idle(&mut self) {}
            fn has_editor(&self) -> bool {
                false
            }
            fn editor_open(&self) -> bool {
                false
            }
            fn open_editor(&mut self, _: *mut std::ffi::c_void, _: f32) -> Result<(u32, u32), String> {
                Err(String::new())
            }
            fn editor_size(&self) -> Option<(u32, u32)> {
                None
            }
            fn close_editor(&mut self) {}
            fn save_state(&self) -> Option<Vec<u8>> {
                None
            }
            fn load_state(&mut self, _: &[u8]) -> bool {
                false
            }
            fn param_count(&self) -> usize {
                1
            }
            fn param_info(&self, index: usize) -> Option<os_host::ParamInfo> {
                (index == 0).then(|| os_host::ParamInfo {
                    id: 7,
                    name: "Drive".into(),
                    unit: String::new(),
                    group: String::new(),
                    min: 0.0,
                    max: 1.0,
                    default: 0.5,
                    steps: 0,
                    automatable: true,
                    read_only: false,
                    hidden: false,
                    bypass: false,
                    enumerated: false,
                })
            }
            fn param_value(&self, _: u32) -> Option<f64> {
                Some(0.5)
            }
        }
        let mut app = app();
        crate::playlist::run(
            &mut app,
            crate::playlist::Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi),
        );
        let tr = app.state.project.tracks.last().unwrap().clone();
        let strip = tr.mixer_strip;
        // The slot holds a (non-builtin) device and an instance is hosted under its key: the rack tile's state.
        crate::mixer::track_mut(&mut app.state, strip).unwrap().slots[0] =
            Some(os_model::SlotPlugin { id: os_model::Id::NONE, name: "Test FX".into(), state: Vec::new() });
        let plugin: std::sync::Arc<std::sync::Mutex<P>> = std::sync::Arc::new(std::sync::Mutex::new(P));
        app.plugins.hosted.insert(crate::plugins::fx_key(strip, 0), crate::plugins::Hosted::new(plugin));
        sync_embed(&mut app);
        let key = crate::plugins::fx_key(strip, 0);
        assert!(
            app.state.device_rack_params.get(&key).map(|ps| !ps.is_empty()).unwrap_or(false),
            "the hosted slot tile has >= 1 param row"
        );
    }

    /// The tile's parameter rows set through the one path (`set_param_value` → `Plugins::set_param`): with an
    /// instance hosted, a ParamSet (click or drag move) moves the plugin's parameter; with none, it is a no-op.
    #[test]
    fn tile_params_set_through_one_path() {
        let mut app = app();
        app.plugins = crate::plugins::Plugins::builtins();
        app.state.plugdb.rebuild(&app.plugins);
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

    /// Reorder action: an effect moves via the control plane's `MoveEffect` (one undo step), a MIDI FX among its
    /// slots, and the instrument stays put.
    #[test]
    fn reorder_moves_effects_and_midi_fx() {
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
        app.state.midi_fx.entry(tr.id.0).or_default().resize(crate::midi_fx::SLOTS, None);
        app.state.midi_fx.get_mut(&tr.id.0).unwrap()[0] = Some(crate::midi_fx::Slot { name: "A".into(), index: 0, on: true });
        app.state.midi_fx.get_mut(&tr.id.0).unwrap()[1] = Some(crate::midi_fx::Slot { name: "B".into(), index: 0, on: true });
        // Chain: [A, B, instrument, EQ2, Reverb, Chorus]. The last FX (slot 2, Chorus) is at index 5.
        let tiles = crate::views::device_rack::tiles(&app.state);
        let chorus = tiles.iter().position(|(d, _)| matches!(d.kind, DeviceKind::Fx(strip, 2))).unwrap();
        assert_eq!(chorus, 5, "the last FX is at chain index 5");
        // Move Chorus (index 5) to the first FX slot's chain position (index 3): it takes slot 0.
        reorder(&mut app, 5, 3).unwrap();
        let m = crate::mixer::track(&app.state, strip).unwrap();
        assert_eq!(m.slots[0].as_ref().unwrap().name, "Lemon Chorus", "the moved FX took slot 0");
        assert_eq!(m.slots[1].as_ref().unwrap().name, "Lemon EQ2");
        assert_eq!(m.slots[2].as_ref().unwrap().name, "Lemon Reverb");
        // A MIDI FX reorder among its slots: move B (index 1) to the front (index 0).
        reorder(&mut app, 1, 0).unwrap();
        let sl = crate::midi_fx::slots(&app.state, tr.id.0);
        assert_eq!(sl[0].as_ref().unwrap().name, "B", "the second MIDI FX moved to the front");
        assert_eq!(sl[1].as_ref().unwrap().name, "A");
        // The instrument (index 2) does not move: a no-op that still returns Ok.
        reorder(&mut app, 2, 0).unwrap();
        // A bogus index errors instead of panicking.
        assert!(reorder(&mut app, 99, 0).is_err());
    }

    /// The device rack's ＋ browser is the plugin palette in its rack form: a MIDI FX section first, then the
    /// instruments and effects each led by a "Lemon" group of the built-ins before the installed ones.
    #[test]
    fn add_device_browser_lists_native_plugins_first() {
        use crate::plugin_palette::{self as pal, Row};
        let mut app = app();
        crate::playlist::run(
            &mut app,
            crate::playlist::Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi),
        );
        app.plugins = crate::plugins::Plugins::builtins();
        app.state.plugdb.rebuild(&app.plugins);
        app.plugins.available.push(crate::plugin_db::fake("Test Synth", os_host::Kind::Instrument));
        app.plugins.effects.push(crate::plugin_db::fake("Test FX", os_host::Kind::Effect));
        app.state.plugdb.rebuild(&app.plugins);
        app.open_plugin_palette(crate::plugin_menu::Target::Rack(Some(1)), (0.0, 0.0), None);
        assert!(pal::is_open(&app.state), "the ＋ opens the palette-style browser");
        let rows = pal::rows(&app.state);
        let labels: Vec<String> = rows
            .iter()
            .map(|r| match r {
                Row::Head { label, .. } => label.clone(),
                Row::MidiFx { at } => app.state.palette.midi_fx[*at].0.clone(),
                Row::Plugin { effect, at } => pal::entry(&app.state, *effect, *at).map(|e| e.label.clone()).unwrap_or_default(),
                Row::Note(t) => (*t).to_string(),
            })
            .collect();
        let at = |name: &str| labels.iter().position(|l| l == name).unwrap_or_else(|| panic!("{name} missing\nlabels={labels:?}"));
        // The MIDI FX section leads the browser (its picks go to the MIDI FX slots, before the instrument).
        assert_eq!(labels.first().map(String::as_str), Some("MIDI FX"), "the MIDI FX section is first\nlabels={labels:?}");
        assert!(at("Lemon Arp") < at("Instruments"), "MIDI FX built-ins listed above the instruments");
        assert!(at("Lemon Sampler") < at("Test Synth"), "native instruments first\nlabels={labels:?}");
        assert!(at("Lemon Reverb") < at("Test FX"), "native effects first\nlabels={labels:?}");
        // Everything native precedes the installed tail: no third-party instrument between Lemon's instruments and effects.
        assert!(at("Instruments") < at("Lemon Reverb") && at("Lemon Reverb") < at("Test Synth") && at("Test Synth") < at("Test FX"));
        // Each section groups its built-ins under a "Lemon" header at the very top, before third-party.
        let lemon_after = |head: &str| {
            let h = labels.iter().position(|l| l == head).unwrap();
            labels[h..].iter().position(|l| l == "Lemon").unwrap() + h
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
    }

    /// The rack browser follows the track's kind (§7's chain): a MIDI track's ＋ offers MIDI FX, instruments and
    /// audio FX; an audio track's ＋ offers only audio FX (no instruments, no MIDI FX).
    #[test]
    fn rack_browser_filters_by_track_kind() {
        use crate::plugin_palette::{self as pal, Row};
        let mut app = app();
        crate::playlist::run(
            &mut app,
            crate::playlist::Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi),
        );
        app.plugins = crate::plugins::Plugins::builtins();
        app.state.plugdb.rebuild(&app.plugins);
        app.plugins.available.push(crate::plugin_db::fake("Test Synth", os_host::Kind::Instrument));
        app.plugins.effects.push(crate::plugin_db::fake("Test FX", os_host::Kind::Effect));
        app.state.plugdb.rebuild(&app.plugins);
        // A MIDI track: all three sections.
        app.open_plugin_palette(crate::plugin_menu::Target::Rack(None), (0.0, 0.0), None);
        let rows = pal::rows(&app.state);
        let heads: Vec<&str> = rows
            .iter()
            .filter_map(|r| match r {
                Row::Head { label, major: true } => Some(label.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            heads,
            ["MIDI FX", "Instruments", "Effects", "Installed instruments", "Installed effects"],
            "a MIDI track's sections\n{heads:?}"
        );
        // An audio track: only the effects (no MIDI FX rows, no instrument rows).
        crate::playlist::run(
            &mut app,
            crate::playlist::Cmd::TrackMode(2, crate::track_links::Mode::LinearAudio),
        );
        app.open_plugin_palette(crate::plugin_menu::Target::Rack(None), (0.0, 0.0), None);
        let rows = pal::rows(&app.state);
        let heads: Vec<&str> = rows
            .iter()
            .filter_map(|r| match r {
                Row::Head { label, major: true } => Some(label.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(heads, ["Effects", "Installed effects"], "an audio track's sections\n{heads:?}");
        assert!(!rows.iter().any(|r| matches!(r, Row::MidiFx { .. })), "no MIDI FX on an audio track");
        assert!(
            !rows.iter().any(|r| matches!(r, Row::Plugin { effect: false, .. })),
            "no instruments on an audio track"
        );
    }

    /// The rack browser's picks keep the chain's routing (§7): a MIDI FX pick goes to the MIDI FX slots, an
    /// effect pick to the gap's mixer slot of the rack track's strip, an instrument pick onto the track's own
    /// instrument channel (a replace, not a new channel).
    #[test]
    fn rack_browser_picks_route_by_the_chain() {
        use crate::plugin_palette::{self as pal, Row};
        use crate::playlist::Cmd;
        let mut app = app();
        crate::playlist::run(
            &mut app,
            crate::playlist::Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi),
        );
        app.plugins = crate::plugins::Plugins::builtins();
        app.state.plugdb.rebuild(&app.plugins);
        app.plugins.available.push(crate::plugin_db::fake("Test Synth", os_host::Kind::Instrument));
        app.plugins.effects.push(crate::plugin_db::fake("Test FX", os_host::Kind::Effect));
        app.state.plugdb.rebuild(&app.plugins);
        app.open_plugin_palette(crate::plugin_menu::Target::Rack(Some(3)), (0.0, 0.0), None);
        let rows = pal::rows(&app.state);
        let tr = app.state.project.tracks.last().unwrap().clone();
        let strip = tr.mixer_strip;
        let chan = tr.channel.unwrap();
        let chan_row = app.state.project.channels.iter().position(|c| c.index == chan).unwrap();
        for (i, r) in rows.iter().enumerate() {
            match *r {
                Row::MidiFx { .. } => {
                    assert!(
                        matches!(pal::pick_cmd(&app.state, r), Some(Cmd::Plugin(crate::plugin_menu::PlugCmd::MidiFxAdd { .. }))),
                        "a MIDI FX pick adds a MIDI FX"
                    );
                }
                Row::Plugin { effect: true, at } => {
                    let idx = pal::entry(&app.state, true, at).unwrap().index;
                    assert_eq!(
                        pal::pick_cmd(&app.state, r),
                        Some(Cmd::SlotWin(strip, Box::new(Cmd::Mixer(crate::mixer::MCmd::AddEffect(3, idx))))),
                        "an effect pick lands in the gap's slot of the rack track's strip"
                    );
                }
                Row::Plugin { effect: false, at } => {
                    let idx = pal::entry(&app.state, false, at).unwrap().index;
                    assert_eq!(
                        pal::pick_cmd(&app.state, r),
                        Some(Cmd::Plugin(crate::plugin_menu::PlugCmd::Replace { index: idx, row: chan_row })),
                        "an instrument pick replaces the track's instrument channel"
                    );
                }
                _ => {}
            }
            let _ = i;
        }
    }

    /// A rack track with no instrument channel (an emptied slot, the default sampler removed) picking an
    /// instrument: the pick makes a channel ON the track — a TrackMode NewInstrument on the track's lane, whose
    /// `new_instrument` links the lane and routes the channel to the track's strip — never an orphan channel in
    /// the channel rack.
    #[test]
    fn rack_browser_instrument_pick_without_a_channel_links_the_track() {
        use crate::plugin_palette::{self as pal, Row};
        use crate::playlist::Cmd;
        let mut app = app();
        crate::playlist::run(&mut app, Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi));
        app.plugins = crate::plugins::Plugins::builtins();
        app.state.plugdb.rebuild(&app.plugins);
        app.plugins.available.push(crate::plugin_db::fake("Test Synth", os_host::Kind::Instrument));
        app.state.plugdb.rebuild(&app.plugins);
        // Empty the instrument slot: the track keeps its lane and strip, no channel.
        let strip = app.state.project.tracks.last().unwrap().clone().mixer_strip;
        let lane = app.state.project.tracks.last().unwrap().clone().playlist_lane;
        app.state.project.tracks.last_mut().unwrap().channel = None;
        app.open_plugin_palette(crate::plugin_menu::Target::Rack(None), (0.0, 0.0), None);
        let rows = pal::rows(&app.state);
        let pick = rows
            .iter()
            .find_map(|r| match r {
                Row::Plugin { effect: false, .. } => pal::pick_cmd(&app.state, r),
                _ => None,
            })
            .expect("an instrument pick routes");
        assert!(
            matches!(pick, Cmd::TrackMode(_, crate::track_links::Mode::NewInstrument(_))),
            "the pick makes the track's instrument, not an orphan channel\n{pick:?}"
        );
        // Running it: the new channel plays the track's strip and the lane is its instrument track.
        crate::playlist::run(&mut app, pick);
        let tr = app.state.project.tracks.last().unwrap().clone();
        assert!(tr.channel.is_some(), "the pick linked a channel to the track");
        let row = app.state.project.channels.iter().position(|c| Some(c.index) == tr.channel).unwrap();
        assert_eq!(app.state.project.channels[row].mixer_track, strip, "the channel routes to the track's strip");
        assert_eq!(lane, app.state.project.tracks.last().unwrap().playlist_lane, "the lane is unchanged");
    }

    /// The default sampler is removable: Delete on the instrument tile asks (the same confirmation the channel
    /// rack's Delete uses), and once gone the track keeps its lane and strip with an empty instrument slot —
    /// the chain shows the "Add instrument" placeholder, the project keeps its other channels.
    #[test]
    fn the_instrument_tile_is_removable_to_an_empty_slot() {
        use crate::device_rack::{DeviceKind, delete_selected};
        let mut app = app();
        crate::playlist::run(&mut app, Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi));
        let strip = app.state.project.tracks.last().unwrap().clone().mixer_strip;
        let row = app
            .state
            .project
            .channels
            .iter()
            .position(|c| Some(c.index) == app.state.project.tracks.last().unwrap().channel)
            .unwrap();
        // The instrument tile is in the chain; select it and Delete.
        let chain = crate::device_rack::rack_chain(&app.state);
        let i = chain.iter().position(|d| matches!(d.kind, DeviceKind::Instrument(_))).expect("the instrument tile");
        app.state.device_rack_sels.insert(i);
        crate::messages::set(&crate::messages::DELETE_CHANNEL, false);
        delete_selected(&mut app);
        assert_eq!(app.state.rack.delete_now, Some(vec![row]), "the tile's Delete asks the channel delete");
        crate::rack::delete_step(&mut app, &[row]);
        let tr = app.state.project.tracks.last().unwrap().clone();
        assert!(tr.channel.is_none(), "the track keeps its lane and strip, channel gone");
        assert!(crate::mixer::track(&app.state, strip).is_some(), "the strip stays");
        let chain = crate::device_rack::rack_chain(&app.state);
        assert!(
            matches!(chain[0].kind, DeviceKind::NoInstrument) && chain[0].name == "Add instrument",
            "the empty slot shows the Add instrument placeholder\n{chain:?}"
        );
        // And the slot is fillable again: an instrument pick makes the track's channel (Bug A's route).
        app.plugins = crate::plugins::Plugins::builtins();
        app.state.plugdb.rebuild(&app.plugins);
        app.open_plugin_palette(crate::plugin_menu::Target::Rack(None), (0.0, 0.0), None);
        let rows = crate::plugin_palette::rows(&app.state);
        let pick = rows
            .iter()
            .find_map(|r| match r {
                crate::plugin_palette::Row::Plugin { effect: false, .. } => crate::plugin_palette::pick_cmd(&app.state, r),
                _ => None,
            })
            .expect("an instrument pick routes");
        crate::playlist::run(&mut app, pick);
        assert!(
            app.state.project.tracks.last().unwrap().channel.is_some(),
            "the pick refilled the slot"
        );
        assert!(crate::device_rack::rack_chain(&app.state).iter().any(|d| matches!(d.kind, DeviceKind::Instrument(_))));
    }

    /// MIDI FX → Instrument → Audio FX end to end on a track whose instrument slot is empty: all three ＋
    /// paths land (the MIDI FX slots, the track's channel, the strip's first free slot).
    #[test]
    fn midi_fx_instrument_and_fx_add_end_to_end_on_an_empty_track() {
        use crate::plugin_palette::{self as pal, Row};
        use crate::playlist::Cmd;
        let mut app = app();
        crate::playlist::run(&mut app, Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi));
        app.plugins = crate::plugins::Plugins::builtins();
        app.state.plugdb.rebuild(&app.plugins);
        let strip = app.state.project.tracks.last().unwrap().clone().mixer_strip;
        let id = app.state.project.tracks.last().unwrap().clone().id.0;
        // Empty the instrument slot (the default sampler deleted).
        let row = app.state.project.channels.iter().position(|c| Some(c.index) == app.state.project.tracks.last().unwrap().channel).unwrap();
        let i = crate::device_rack::rack_chain(&app.state).iter().position(|d| matches!(d.kind, DeviceKind::Instrument(_))).unwrap();
        app.state.device_rack_sels.insert(i);
        crate::messages::set(&crate::messages::DELETE_CHANNEL, false);
        crate::device_rack::delete_selected(&mut app);
        crate::rack::delete_step(&mut app, &[row]);
        app.open_plugin_palette(crate::plugin_menu::Target::Rack(None), (0.0, 0.0), None);
        let run_pick = |app: &mut App, want: &dyn Fn(&pal::Row) -> bool| {
            let rows = pal::rows(&app.state);
            let r = rows.iter().find(|r| want(r)).expect("the row");
            let cmd = pal::pick_cmd(&app.state, r).expect("the pick");
            crate::playlist::run(app, cmd);
        };
        run_pick(&mut app, &|r| matches!(r, Row::MidiFx { .. }));
        assert_eq!(crate::midi_fx::filled(&app.state, id).len(), 1, "the MIDI FX landed");
        run_pick(&mut app, &|r| matches!(r, Row::Plugin { effect: false, .. }));
        assert!(app.state.project.tracks.last().unwrap().channel.is_some(), "the instrument landed");
        run_pick(&mut app, &|r| matches!(r, Row::Plugin { effect: true, .. }));
        assert!(
            crate::mixer::track(&app.state, strip).is_some_and(|m| m.slots.iter().any(|s| s.is_some())),
            "the audio FX landed on the track's strip"
        );
    }

    /// The wheel over the rack's body scrolls the chain through the real dispatch (`App::wheel` → `wheel_act`),
    /// not just the helper: notches move the tiles, clamped to the content.
    #[test]
    fn wheel_scrolls_the_rack_through_the_dispatch() {
        let mut app = app();
        crate::playlist::run(
            &mut app,
            Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi),
        );
        let strip = app.state.project.tracks.last().unwrap().clone().mixer_strip;
        let m = crate::mixer::track_mut(&mut app.state, strip).unwrap();
        for k in 0..7 {
            m.slots[k] = Some(os_model::SlotPlugin { id: os_model::Id::NONE, name: format!("Dev {k}"), state: Vec::new() });
        }
        let (w, h) = app.state.screen;
        assert!(crate::views::device_rack::content_w(&app.state) > w, "the chain overflows the panel");
        let r = crate::views::device_rack::panel_rect(&app.state, w, h);
        app.mouse_move(r.x + 50.0, r.y + crate::device_rack::HEADER_H + 20.0);
        // Wheel down (right) twice, then past the end: the dispatch moves and clamps the scroll.
        app.wheel(0.0, -1.0, false, false);
        let one = app.state.device_rack_scroll_x;
        assert!(one > 0.0, "a wheel notch through wheel_act scrolls right");
        for _ in 0..50 {
            app.wheel(0.0, -1.0, false, false);
        }
        let max = crate::views::device_rack::scroll_x(&app.state, w);
        assert!(app.state.device_rack_scroll_x <= max + 0.001, "clamped to the content");
        // Wheel up past the start: clamped at 0.
        for _ in 0..50 {
            app.wheel(0.0, 1.0, false, false);
        }
        assert_eq!(app.state.device_rack_scroll_x, 0.0, "clamped at the start");
        // A sideways wheel (dx) scrolls too: up = left.
        app.wheel(-3.0, 0.0, false, false);
        assert!(app.state.device_rack_scroll_x > 0.0, "a sideways wheel scrolls");
    }

    /// Builds a MIDI track with one MIDI FX per slot (all 8 filled), the default instrument and one effect, hosted.
    fn full_chain_app() -> App {
        use crate::plugin_palette::{self as pal, Row};
        let mut app = app();
        crate::playlist::run(&mut app, Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi));
        app.plugins = crate::plugins::Plugins::builtins();
        app.state.plugdb.rebuild(&app.plugins);
        let id = app.state.project.tracks.last().unwrap().id.0;
        for i in 0..app.plugins.midi_fx.len().min(crate::midi_fx::SLOTS) {
            crate::midi_fx::add(&mut app, id, None, i);
        }
        app.open_plugin_palette(crate::plugin_menu::Target::Rack(None), (0.0, 0.0), None);
        let rows = pal::rows(&app.state);
        let r = rows.iter().find(|r| matches!(r, Row::Plugin { effect: true, .. })).expect("an effect");
        let cmd = pal::pick_cmd(&app.state, r).unwrap();
        crate::playlist::run(&mut app, cmd);
        app.mixer_tick();
        app.sync_samplers(None);
        sync_embed(&mut app);
        app
    }

    /// The render path: every tile that is not showing an embedded editor has >= 1 parameter row that is really
    /// on screen (inside the body, with height), and paints a ParamSet hit for it.
    #[test]
    fn every_tile_renders_its_own_controls() {
        let Ok(mut c) = os_ui::Canvas::new(1920, 1080, 1.0) else { return };
        let mut app = full_chain_app();
        app.paint(&mut c, None);
        let r = crate::views::device_rack::panel_rect(&app.state, app.state.screen.0, app.state.screen.1);
        let tiles = crate::views::device_rack::layout(&app.state, r);
        assert!(tiles.len() >= 4, "{tiles:?}");
        let mut midi_fx = 0;
        for (d, tile) in &tiles {
            let Some(key) = d.key else { continue };
            midi_fx += usize::from(matches!(d.kind, DeviceKind::MidiFx(_)));
            let ps = app.state.device_rack_params.get(&key).unwrap_or_else(|| panic!("{} has no params cached", d.name));
            assert!(!ps.is_empty(), "{}", d.name);
            assert!(ps.iter().all(|p| !p.text.is_empty()), "{}: every control has the plugin's value text", d.name);
            let dial = crate::views::device_rack::param_dial(&app.state, key, ps[0].id)
                .unwrap_or_else(|| panic!("{}: first control is not on screen", d.name));
            assert!(dial.w > 20.0 && dial.h > 0.0 && tile.contains(dial.x + 1.0, dial.y + 1.0), "{}: {dial:?}", d.name);
            let hit = app.hits.iter().any(|h| matches!(h.action, Action::DeviceRack(RackAct::ParamSet { key: k, .. }) if k == key));
            assert!(hit, "{}: no painted control hit", d.name);
        }
        assert!(midi_fx >= crate::midi_fx::SLOTS, "every MIDI FX slot is a tile ({midi_fx})");
    }

    /// A MIDI FX tile draws its plugin's own editor (`crate::rack_ui`): its panels' controls, each painted on the
    /// tile (the live dials, switches and selectors), and the tile is as wide as that editor instead of the generic
    /// width.
    #[test]
    fn a_midi_fx_tile_draws_its_plugins_own_editor() {
        let Ok(mut c) = os_ui::Canvas::new(1920, 1080, 1.0) else { return };
        let mut app = full_chain_app();
        app.paint(&mut c, None);
        let (d, tile) = crate::views::device_rack::tiles(&app.state)
            .into_iter()
            .find(|(d, _)| matches!(d.kind, DeviceKind::MidiFx(_)))
            .expect("a MIDI FX tile");
        let key = d.key.unwrap();
        let name = app.state.device_rack_names.get(&key).expect("the hosted plugin's name");
        let editor = crate::rack_ui::ui(name).unwrap_or_else(|| panic!("{name} has an editor"));
        let params = &app.state.device_rack_params[&key];
        let hits: Vec<_> = app
            .hits
            .iter()
            .filter(|h| {
                matches!(h.action, Action::DeviceRack(RackAct::ParamSet { key: k, .. } | RackAct::ParamTap { key: k, .. }) if k == key)
            })
            .collect();
        assert!(
            hits.len() >= editor.settings().len(),
            "{name}: {} of its {} controls are painted",
            hits.len(),
            editor.settings().len()
        );
        for h in &hits {
            assert!(tile.contains(h.rect.cx(), h.rect.cy()), "{name}: {:?} is outside its tile {tile:?}", h.rect);
        }
        let (w, _) = editor.size();
        assert!(tile.w > TILE_W && tile.w <= w + 2.0 * TILE_PAD + 1.0, "{name}: tile {} for a {w} px editor", tile.w);
        assert!(params.iter().any(|p| p.plain != 0.0 || p.min != 0.0 || p.max != 0.0), "plain ranges reach the tile");
    }

    /// A MIDI FX tile draws every one of its controls — the rack used to cap a tile at eight, so a device's later
    /// controls (a sampler's ADSR, a filter's knobs) were simply absent. All of them get a hit and a place in the
    /// grid; what does not fit scrolls.
    #[test]
    fn a_midi_fx_tile_shows_all_of_its_controls() {
        let Ok(mut c) = os_ui::Canvas::new(1920, 1080, 1.0) else { return };
        let mut app = full_chain_app();
        app.paint(&mut c, None);
        let tiles = crate::views::device_rack::tiles(&app.state);
        let (d, tile) = tiles.iter().find(|(d, _)| matches!(d.kind, DeviceKind::MidiFx(_))).unwrap().clone();
        let key = d.key.unwrap();
        let count = app.state.device_rack_params[&key].len();
        assert!(count > 0, "{}", d.name);
        for p in &app.state.device_rack_params[&key] {
            let cell = crate::views::device_rack::param_dial(&app.state, key, p.id)
                .unwrap_or_else(|| panic!("{}: {} has no cell", d.name, p.name));
            assert!(tile.contains(cell.x + 1.0, cell.y + 1.0), "{}: {} is outside its tile: {cell:?}", d.name, p.name);
        }
        assert!(
            app.hits.iter().filter(|h| matches!(h.action, Action::DeviceRack(RackAct::ParamSet { key: k, .. }) if k == key)).count() >= count.min(4),
            "{}: the tile paints a hit per visible control",
            d.name
        );
    }

    /// Right-click on a docked device's knob / parameter control opens its parameter menu
    /// (Reset, automation, link — the same menu the detached editor shows), so automation is
    /// reachable without popping the device out (user, 2026-11-11).
    #[test]
    fn right_click_a_docked_device_knob_opens_its_param_menu() {
        let Ok(mut c) = os_ui::Canvas::new(1920, 1080, 1.0) else { return };
        let mut app = full_chain_app();
        app.paint(&mut c, None);
        // An automatable control hit in the docked device rack.
        let hit = app
            .hits
            .iter()
            .find(|h| {
                if let Action::DeviceRack(crate::device_rack::RackAct::ParamSet { key, id, .. }) = &h.action {
                    crate::params::param_menu(&app.state, *key, *id as usize, 0.0, 0.0).is_some()
                } else {
                    false
                }
            })
            .expect("an automatable docked device control");
        app.mouse = (hit.rect.cx(), hit.rect.cy());
        app.mouse_down_right();
        let popup = app.state.playlist.popup.as_ref().expect("right-click opens the device param menu");
        assert!(!popup.items.is_empty(), "the param menu has entries");
    }

    /// The chain fits ~4 tiles with their ＋ gaps in the default panel width without scrolling.
    #[test]
    fn four_devices_fit_the_default_panel_width() {
        let mut app = app();
        crate::playlist::run(&mut app, Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi));
        let w = app.state.screen.0;
        assert!(4.0 * (TILE_W + 22.0) <= w * 0.7, "tile {TILE_W} px");
        assert!(4.0 * (TILE_W + 22.0) <= 1229.0, "four tiles fit the narrowest usual window");
        assert!(TILE_W >= 240.0 && TILE_W < 340.0);
    }

    /// Remove from the tile menu: a MIDI FX leaves its slot, an FX leaves its mixer slot (through RemoveEffect, one
    /// undo step), the instrument asks the channel delete and leaves the Add instrument placeholder.
    #[test]
    fn the_tile_menu_removes_each_device_kind() {
        let mut app = full_chain_app();
        let tr = app.state.project.tracks.last().unwrap().clone();
        let n_fx = crate::midi_fx::filled(&app.state, tr.id.0).len();
        assert!(n_fx >= 2);
        let first = rack_chain(&app.state).into_iter().find(|d| matches!(d.kind, DeviceKind::MidiFx(_))).unwrap();
        run(&mut app, RackAct::Remove(first.kind));
        assert_eq!(crate::midi_fx::filled(&app.state, tr.id.0).len(), n_fx - 1, "the MIDI FX is gone");
        assert!(!rack_chain(&app.state).iter().any(|d| d.kind == first.kind));

        let fx = rack_chain(&app.state).into_iter().find(|d| matches!(d.kind, DeviceKind::Fx(..))).expect("an FX tile");
        let DeviceKind::Fx(strip, slot) = fx.kind else { unreachable!() };
        let undo_before = app.state.history.done();
        run(&mut app, RackAct::Remove(fx.kind));
        assert!(crate::mixer::track(&app.state, strip).unwrap().slots[usize::from(slot)].is_none(), "the slot is empty");
        assert!(!rack_chain(&app.state).iter().any(|d| d.kind == fx.kind));
        assert!(app.state.history.done() > undo_before, "RemoveEffect recorded an undo step");

        crate::messages::set(&crate::messages::DELETE_CHANNEL, false);
        let inst = rack_chain(&app.state).into_iter().find(|d| matches!(d.kind, DeviceKind::Instrument(_))).unwrap();
        let DeviceKind::Instrument(row) = inst.kind else { unreachable!() };
        run(&mut app, RackAct::Remove(inst.kind));
        crate::rack::delete_step(&mut app, &[row]);
        let chain = rack_chain(&app.state);
        assert!(chain.iter().any(|d| d.kind == DeviceKind::NoInstrument && d.name == "Add instrument"), "{chain:?}");
    }

    /// A right-click on a tile opens its device menu, which lists Remove.
    #[test]
    fn right_click_on_a_tile_opens_the_menu_with_remove() {
        let Ok(mut c) = os_ui::Canvas::new(1920, 1080, 1.0) else { return };
        let mut app = full_chain_app();
        app.paint(&mut c, None);
        let tiles = crate::views::device_rack::tiles(&app.state);
        let (d, t) = tiles.iter().find(|(d, _)| matches!(d.kind, DeviceKind::MidiFx(_))).unwrap().clone();
        app.mouse_move(t.cx(), t.y + HEADER_H + 4.0);
        app.mouse_down_right();
        assert_eq!(app.state.device_rack_menu, d.key);
        app.paint(&mut c, None);
        let remove = app.hits.iter().find(|h| h.action == Action::DeviceRack(RackAct::Remove(d.kind)));
        assert!(remove.is_some(), "the menu offers Remove");
    }

    /// A header click selects the tile (painting its bar brighter, the selected veil lighter); Shift+click
    /// extends the selection through the anchor (the playlist's track-header pattern), and Delete removes every
    /// selected device.
    #[test]
    fn the_header_selects_and_shift_extends() {
        let mut app = full_chain_app();
        // A second effect, so the Shift-extended range spans two deletions.
        let strip = app.state.project.tracks.last().unwrap().clone().mixer_strip;
        let free = crate::mixer::track(&app.state, strip).unwrap().slots.iter().position(|s| s.is_none()).unwrap() as u8;
        let index = app.plugins.effects.iter().position(|a| a.desc.name == crate::plugins::builtin_effects()[0].desc.name).unwrap();
        app.state.selected_mixer = strip;
        crate::mixer::run(&mut app, crate::mixer::MCmd::AddEffect(free, index));
        app.state.control.settle(&mut app.state.project);
        app.mixer_tick();
        let tiles = crate::views::device_rack::tiles(&app.state);
        let fx_idx = tiles.iter().position(|(d, _)| matches!(d.kind, DeviceKind::Fx(..))).unwrap();
        // A plain click selects one tile; the paint of the selected tile's header is a lighter veil (asserted
        // through `tile_selected`, what the view reads).
        run(&mut app, RackAct::SelectTile(fx_idx));
        assert_eq!(rack_sel(&app.state), Some(fx_idx));
        assert!(tile_selected(&app.state, fx_idx));
        assert!(!tile_selected(&app.state, 0));
        // Shift+click extends through the anchor: every tile between.
        app.shift = true;
        let other = tiles.iter().rposition(|(d, _)| matches!(d.kind, DeviceKind::Fx(..))).unwrap();
        run(&mut app, RackAct::SelectTile(other));
        app.shift = false;
        assert_eq!(
            app.state.device_rack_sels,
            (fx_idx..=other).collect::<BTreeSet<usize>>(),
            "the range from the anchor is selected"
        );
        // Delete removes the selected devices at once.
        let n_fx = crate::mixer::track(&app.state, strip).unwrap().slots.iter().filter(|s| s.is_some()).count();
        delete_selected(&mut app);
        let m = crate::mixer::track(&app.state, strip).unwrap();
        assert_eq!(m.slots.iter().filter(|s| s.is_some()).count(), n_fx - 2, "both selected effects went");
        assert!(app.state.device_rack_sels.is_empty(), "the selection cleared");
    }

    /// Each device tile's header carries a wrench (right of the v menu): it pops that device's editor out to its
    /// own floating window — lit while it is out, and docking it again — without the header menu's round trip.
    #[test]
    fn the_wrench_pops_out_one_device() {
        let Ok(mut c) = os_ui::Canvas::new(1920, 1080, 1.0) else { return };
        let mut app = full_chain_app();
        app.paint(&mut c, None);
        let tiles = crate::views::device_rack::tiles(&app.state);
        let (d, tile) = tiles.iter().find(|(d, _)| matches!(d.kind, DeviceKind::Fx(..))).unwrap().clone();
        let key = d.key.unwrap();
        // The wrench sits right of the v menu, inside the header.
        let hit = app.hits.iter().find(|h| h.action == Action::DeviceRack(RackAct::PopOut(key)));
        assert!(hit.is_some(), "the tile paints a pop-out wrench");
        assert!(tile.contains(hit.unwrap().rect.cx(), hit.unwrap().rect.y + 2.0), "in the header");
        assert!(!app.state.device_rack_float.contains(&key), "docked first");
        run(&mut app, RackAct::PopOut(key));
        assert!(app.state.device_rack_float.contains(&key), "floated");
        // Painted again, the wrench is the Dock action and lit.
        app.paint(&mut c, None);
        assert!(app.hits.iter().any(|h| h.action == Action::DeviceRack(RackAct::Dock(key))), "lit: Dock now");
        run(&mut app, RackAct::Dock(key));
        assert!(!app.state.device_rack_float.contains(&key), "docked back");
    }

    /// A container is a device kind of its own: an empty container (MCmd::AddContainer, the browser's entry
    /// dropped on the rack) shows as a Container tile — its own gradient, its own place name — and the tile's
    /// wrench (or body) opens the docked container window.
    #[test]
    fn a_container_is_a_device_of_the_chain() {
        let mut app = full_chain_app();
        let strip = app.state.project.tracks.last().unwrap().clone().mixer_strip;
        let free = app.state.project.mixer.iter().find(|m| m.index == strip).unwrap().slots.iter().position(|s| s.is_none()).unwrap() as u8;
        app.state.selected_mixer = strip;
        crate::mixer::run(&mut app, crate::mixer::MCmd::AddContainer(free));
        let chain = rack_chain(&app.state);
        let Some(d) = chain.iter().find(|d| matches!(d.kind, DeviceKind::Container { k: k2, .. } if k2 == free)) else {
            panic!("the container slot is a Container device\n{chain:?}");
        };
        assert_eq!(d.name, crate::container::title(&app.state, strip, free).unwrap(), "titled by its place");
        // Its gradient is its own, distinct from the other kinds'.
        let (ct, cb) = header_gradient(d.kind);
        assert_eq!((ct, cb), HEADER_GRAD_CONTAINER);
        assert_ne!((ct, cb), header_gradient(DeviceKind::Fx(strip, free)));
        // The wrench (and the body) opens the container window on its place.
        run(&mut app, RackAct::OpenContainer { t: strip, k: free });
        assert!(app.state.shown(crate::app::Win::Container), "the container window opened");
        assert_eq!(app.state.container.at, (strip, free), "on the container's place");
        // Its LED switches the slot.
        run(&mut app, RackAct::ToggleOn(d.kind));
        assert!(!crate::mixer::track(&app.state, strip).unwrap().slot_on[usize::from(free)], "the slot switched off");
        // Removing it empties the slot (the same RemoveEffect path an effect goes through).
        run(&mut app, RackAct::Remove(d.kind));
        assert!(crate::mixer::track(&app.state, strip).unwrap().slots[usize::from(free)].is_none(), "removed");
    }

    /// The browser's container entry dropped over the device rack makes an empty container in the rack track's
    /// strip — its own container device tile (the mixer selection is lent to the drop and given back).
    #[test]
    fn a_browser_container_drop_lands_on_the_rack() {
        let mut app = full_chain_app();
        let strip = app.state.project.tracks.last().unwrap().clone().mixer_strip;
        let n = crate::mixer::track(&app.state, strip).unwrap().slots.iter().filter(|s| s.is_some()).count();
        let _msg = app.drop_container_on_rack_strip(None);
        let m = crate::mixer::track(&app.state, strip).unwrap();
        assert_eq!(m.slots.iter().filter(|s| s.is_some()).count(), n + 1, "the container landed on the rack strip");
        assert!(m.slots.iter().any(|s| s.as_ref().is_some_and(|s| s.name == crate::container::CONTAINER)));
        assert!(rack_chain(&app.state).iter().any(|d| matches!(d.kind, DeviceKind::Container { .. })));
    }

    /// Ctrl+D duplicates the selected effect into the first free slot after it (its plugin, its state with it),
    /// Ctrl+X cuts it (the slot clears, the clip keeps it), Ctrl+V pastes it back — and on the instrument both
    /// are a no-op message.
    #[test]
    fn ctrl_d_x_v_duplicate_cut_and_paste() {
        let mut app = full_chain_app();
        let tiles = crate::views::device_rack::tiles(&app.state);
        let (d, _) = tiles.iter().find(|(d, _)| matches!(d.kind, DeviceKind::Fx(..))).unwrap().clone();
        let DeviceKind::Fx(strip, k) = d.kind else { unreachable!() };
        let key = d.key.unwrap();
        let state = app.plugins.hosted.get(&key).and_then(|h| h.plugin.lock().ok().and_then(|p| p.save_state()));
        app.state.device_rack_sels.insert(tiles.iter().position(|(t, _)| t.kind == d.kind).unwrap());
        // Ctrl+D: the duplicate lands in the next free slot, same name.
        assert!(duplicate_selected(&mut app));
        let dup_name = crate::mixer::track(&app.state, strip).unwrap().slots[usize::from(k + 1)]
            .as_ref()
            .map(crate::project_plugins::slot_title)
            .expect("the duplicate took the next slot");
        assert_eq!(dup_name, d.name);
        // Ctrl+X the original: the slot empties, the clip holds the device.
        assert!(cut_selected(&mut app), "the cut happened");
        assert!(
            crate::mixer::track(&app.state, strip).unwrap().slots[usize::from(k)].is_none(),
            "the cut slot is empty"
        );
        let clip = app.state.device_rack_clip.clone();
        let Some(Clip::Fx { name, state: clip_state, .. }) = clip else { panic!("an effect in the clip") };
        assert_eq!(name, d.name);
        if let Some(st) = state {
            assert_eq!(clip_state, st, "the cut keeps the device's state");
        }
        // Ctrl+V pastes it into the freed slot, from the state it was cut with.
        assert!(paste_clip(&mut app));
        let m = crate::mixer::track(&app.state, strip).unwrap();
        assert_eq!(
            crate::project_plugins::slot_title(m.slots[usize::from(k)].as_ref().expect("pasted back")),
            d.name
        );
        // The instrument is a no-op for the pair.
        let inst = tiles.iter().find(|(t, _)| matches!(t.kind, DeviceKind::Instrument(_))).unwrap().0.clone();
        app.state.device_rack_sels = BTreeSet::from([tiles.iter().position(|(t, _)| t.kind == inst.kind).unwrap()]);
        assert!(duplicate_selected(&mut app));
        assert!(app.state.message.as_deref().unwrap().contains("Only effects"));
    }

    /// Ctrl+G groups the selected effects of one strip into a container in the first one's slot: every selected
    /// effect becomes a member (one undo step), the wrapped slot reads as a Container device.
    #[test]
    fn ctrl_g_groups_selected_effects_into_a_container() {
        use crate::container::members;
        let mut app = full_chain_app();
        let strip = app.state.project.tracks.last().unwrap().clone().mixer_strip;
        // A second effect to group with the first.
        let free = crate::mixer::track(&app.state, strip).unwrap().slots.iter().position(|s| s.is_none()).unwrap() as u8;
        let index = app.plugins.effects.iter().position(|a| a.desc.name == crate::plugins::builtin_effects()[0].desc.name).unwrap();
        app.state.selected_mixer = strip;
        crate::mixer::run(&mut app, crate::mixer::MCmd::AddEffect(free, index));
        app.state.control.settle(&mut app.state.project);
        app.mixer_tick();
        // Select the two effects through their tiles (a Shift-extended header selection), then Ctrl+G.
        let tiles = crate::views::device_rack::tiles(&app.state);
        let sels: Vec<usize> = tiles
            .iter()
            .enumerate()
            .filter(|(_, (d, _))| matches!(d.kind, DeviceKind::Fx(..)))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(sels.len(), 2, "the two effects to group");
        app.state.device_rack_sels = sels.iter().copied().collect();
        assert!(group_selected(&mut app));
        let first_slot = match tiles[sels[0]].0.kind {
            DeviceKind::Fx(_, k) => k,
            _ => unreachable!(),
        };
        let chain = rack_chain(&app.state);
        let container = chain
            .iter()
            .find(|d| matches!(d.kind, DeviceKind::Container { k, .. } if k == first_slot))
            .expect("the group container\n{chain:?}");
        let DeviceKind::Container { t: ct, k: ck } = container.kind else { unreachable!() };
        assert_eq!(members(&app.state, ct, ck).unwrap_or_default().len(), 2, "every selected effect became a member");
        assert!(app.state.device_rack_sels.is_empty(), "the selection cleared");
    }

    /// Ctrl+G with the instrument tile selected wraps the channel (the instrument rack): a plugin channel's
    /// instrument becomes a Container device at (row, CHANNEL) — the container plays the channel's notes.
    #[test]
    fn ctrl_g_on_the_instrument_wraps_the_channel() {
        let mut app = app();
        crate::playlist::run(&mut app, Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi));
        app.plugins = crate::plugins::Plugins::builtins();
        app.state.plugdb.rebuild(&app.plugins);
        app.plugin_tick();
        let trio = app.plugins.index_of(crate::osc3::LEMON_TRIO).unwrap();
        // The rack track's channel: a rack row's row is the channel's index in the project.
        let tr = app.state.project.tracks.first().unwrap().clone();
        let row = app.state.project.channels.iter().position(|c| Some(c.index) == tr.channel).unwrap();
        crate::plugin_menu::replace_generator(&mut app, trio, row);
        (0..3).for_each(|_| app.plugin_tick());
        let i = crate::views::device_rack::tiles(&app.state)
            .iter()
            .position(|(d, _)| d.kind == DeviceKind::Instrument(row))
            .unwrap_or_else(|| panic!("the instrument tile\n{:?}", crate::views::device_rack::tiles(&app.state)));
        app.state.device_rack_sels = BTreeSet::from([i]);
        assert!(group_selected(&mut app));
        let chain = rack_chain(&app.state);
        assert!(
            chain.iter().any(|d| matches!(d.kind, DeviceKind::Container { t: ct, k: kk } if ct == row as u16 && kk == crate::container::CHANNEL)),
            "the instrument is a container now\n{chain:?}"
        );
        assert_eq!(crate::container::members(&app.state, row as u16, crate::container::CHANNEL).map(|m| m.len()), Some(1));
    }
}
