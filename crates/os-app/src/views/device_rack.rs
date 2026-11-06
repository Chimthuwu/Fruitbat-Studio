// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The device rack's view (Lemon Studio's design: spec model/track-model.md §7): a panel docked at the bottom of
//! the main window, resizable by its splitter and collapsible to its header, whose device tiles hold the selected
//! track's devices in signal-flow order. Built-in Lemon plugins' editors render inside their tile; anything else
//! (and a tile whose editor failed) shows a placeholder body.

use os_ui::{Color, Rect, kit};

use crate::app::{Action, State};
use crate::device_rack::{self, Device, HEADER_H, RackAct, TILE_W};
use crate::plugin_menu::{self, pick, Target};
use crate::playlist::{Popup, PItem};
use crate::views::{Ui, fnt};

/// The rack's panel rect in the window: the full width, its height at the bottom edge (`device_rack::height`).
pub fn panel_rect(s: &State, w: f32, h: f32) -> Rect {
    Rect::new(0.0, h - device_rack::height(s), w, device_rack::height(s))
}

/// The splitter strip along the panel's top edge (6 px, like the browser's).
const SPLITTER: f32 = 6.0;

/// Gap between tiles, holding the "+" insert buttons (before the first, between every
/// pair, and after the last tile).
const GAP: f32 = 22.0;

/// The device tiles over `r` (the panel rect) as (device, tile) pairs, left to right in signal-flow order; the
/// empty leftmost tile on a MIDI track is the MIDI FX placeholder (§8 is a later milestone).
pub(crate) fn layout(s: &State, r: Rect) -> Vec<(Device, Rect)> {
    if r.h <= HEADER_H {
        return Vec::new();
    }
    let body = Rect::new(r.x, r.y + HEADER_H, r.w, r.h - HEADER_H);
    device_rack::rack_chain(s)
        .into_iter()
        .scan(body.x, |x, d| {
            let tile = Rect::new(*x, body.y, TILE_W, body.h);
            *x += TILE_W + GAP;
            Some((d, tile))
        })
        .collect()
}

/// The tiles at the current screen size (for hit-testing a tile drag's drop position).
pub(crate) fn tiles(s: &State) -> Vec<(Device, Rect)> {
    layout(s, panel_rect(s, s.screen.0, s.screen.1))
}

/// The tile bodies the rack embeds a built-in editor into: the built-in devices the user has not popped out. The
/// app pins each editor's native window here on the next tick (`device_rack::sync_embed`).
pub fn device_rects(s: &State) -> Vec<(usize, Rect)> {
    let (_, w, h) = ((), s.screen.0, s.screen.1);
    let r = panel_rect(s, w, h);
    layout(s, r)
        .into_iter()
        .filter_map(|(d, tile)| {
            let key = d.key?;
            (d.builtin && !s.device_rack_float.contains(&key)).then(|| (key, body(tile)))
        })
        .collect()
}

/// A tile's editor body: below its header.
fn body(tile: Rect) -> Rect {
    Rect::new(tile.x, tile.y + HEADER_H, tile.w, tile.h - HEADER_H)
}

/// Paints the rack panel over `r` (its splitter, header and device tiles).
pub fn paint(ui: &mut Ui, s: &State, r: Rect) {
    let t = ui.t;
    ui.c.fill_rect(r, t.s(0x1a1d1f));
    // The splitter: a drag on it resizes the rack, a double click gives back the default height.
    let split = Rect::new(r.x, r.y, r.w, SPLITTER);
    let hovered = ui.hit(
        split,
        Action::DeviceRack(RackAct::Splitter),
        "Drag to resize the device rack; double click for the default height",
    );
    let _ = hovered;
    ui.c.fill_rect(split, if hovered { t.splitter_dark } else { t.splitter });

    // Header: which track's chain this is, its pin, its collapse.
    let head = Rect::new(r.x, r.y + SPLITTER, r.w, HEADER_H - SPLITTER);
    ui.c.fill_rect(head, t.window_title);
    let name = device_rack::rack_track(s).map_or("No track selected".to_string(), |tr| {
        crate::playlist::track_name(s, tr.playlist_lane as u16 - 1)
    });
    ui.c.text(
        fnt(15.0),
        r.x + 8.0,
        head.y + 2.0,
        t.text,
        &format!("Device rack - {name}"),
    );
    // The header buttons, right-aligned: ＋ Add (the plugin browser), Pin, Show / Hide.
    let mut bx = r.right() - 198.0;
    header_button(
        ui,
        Rect::new(bx, head.y + 2.0, 60.0, 17.0),
        "＋ Add",
        false,
        Action::DeviceRack(RackAct::AddDevice),
        "Device rack plugin browser: native (built-in) plugins first; the display-mode radios below are the filter",
    );
    bx += 66.0;
    // The pin: the chain stays while the selection moves (§7 "Track selection drives the rack").
    let pinned = s.device_rack_pin.is_some();
    header_button(
        ui,
        Rect::new(bx, head.y + 2.0, 60.0, 17.0),
        if pinned { "Pinned" } else { "Pin" },
        pinned,
        Action::DeviceRack(RackAct::TogglePin),
        pinned
            .then_some("Unpin this track's device chain")
            .unwrap_or("Pin this track's device chain while you edit elsewhere"),
    );
    bx += 66.0;
    header_button(
        ui,
        Rect::new(bx, head.y + 2.0, 60.0, 17.0),
        if s.device_rack_collapsed {
            "Show"
        } else {
            "Hide"
        },
        false,
        Action::DeviceRack(RackAct::ToggleCollapsed),
        if s.device_rack_collapsed {
            "Show the device rack"
        } else {
            "Collapse the device rack to its header"
        },
    );

    if s.device_rack_collapsed {
        return;
    }
    // The tiles, with a "+" gap button before the first, between every pair and after the last.
    let tiles = layout(s, r);
    let slot_after = |i: usize| -> Option<u8> {
        tiles[i..]
            .iter()
            .find_map(|(d, _)| match d.kind {
                DeviceKind::Fx(_, k) => Some(k),
                _ => None,
            })
    };
    let mut gx = r.x;
    for (i, (d, tile)) in tiles.iter().enumerate() {
        gap_button(ui, gx, tile.y, slot_after(i), tile.h);
        tile_paint(ui, s, d.clone(), *tile);
        gx = tile.right();
    }
    gap_button(ui, gx, r.y + HEADER_H, None, tiles.first().map_or(r.h - HEADER_H, |(_, t)| t.h));
}

/// A "+" gap button at x: opens the FX palette for the slot that follows it (`None` = first empty slot).
fn gap_button(ui: &mut Ui, x: f32, y: f32, slot: Option<u8>, h: f32) {
    let t = ui.t;
    let b = Rect::new(x + 2.0, y + h / 2.0 - 9.0, GAP - 4.0, 18.0);
    let hov = ui.hit(b, Action::DeviceRack(RackAct::AddAt(slot)), "Add an effect here");
    ui.c.fill_rrect(b, 2.0, if hov { t.s(0x2b3134) } else { t.s(0x222629) });
    ui.c.text(fnt(14.0), b.x + 4.0, b.y + 1.0, t.text, "+");
}

/// A device tile: a header (name, enable light, menu) over an editor body — embedded, or a placeholder.
fn tile_paint(ui: &mut Ui, s: &State, d: Device, tile: Rect) {
    let t = ui.t;
    // The tile: a raised surface with a hairline between header and body.
    ui.c.fill_rect(tile, t.s(0x222629));
    ui.c.stroke_rrect(tile, 0.0, 1.0, t.splitter_dark);
    let head = Rect::new(tile.x, tile.y, tile.w, HEADER_H);
    ui.c.fill_rect(
        Rect::new(tile.x, tile.bottom() - 1.0, tile.w, 1.0),
        t.splitter_dark,
    );
    // Drag the header left/right to reorder (FX tiles swap mixer slots); the LED and menu below win their clicks.
    if d.key.is_some() {
        ui.hit(head, Action::DeviceRack(RackAct::TilePress(d.kind)), "Drag to reorder this device");
    }
    // The enable light: a click switches the device (the channel or the mixer slot).
    let (cx, cy) = (tile.x + 12.0, head.cy());
    crate::views::led(ui, cx, cy, d.on);
    let led = Rect::new(cx - 7.0, cy - 7.0, 14.0, 14.0);
    if d.key.is_some() {
        ui.hit(
            led,
            Action::DeviceRack(RackAct::ToggleOn(d.kind)),
            if d.on {
                "Click to turn the device off"
            } else {
                "Click to turn the device on"
            },
        );
    }
    // The name, then the header menu.
    ui.c.text_clip(
        fnt(14.0),
        tile.x + 24.0,
        head.y + 3.0,
        tile.w - 24.0 - 44.0,
        t.text,
        &d.name,
    );
    let menu = Rect::new(tile.right() - 22.0, head.y + 4.0, 17.0, 15.0);
    if d.key.is_some()
        && ui.hit(
            menu,
            Action::DeviceRack(RackAct::Menu(d.key.unwrap())),
            "Device menu: Pop out / Dock",
        )
    {
        ui.c.text(fnt(13.0), menu.x, menu.y, t.text, "v");
    }
    let body = body(tile);
    match d.key {
        // A built-in's editor renders into the tile (its native window is pinned there by the app each tick);
        // until it is, the empty surface shows.
        Some(key) if d.builtin => {
            ui.c.fill_rect(body, t.s(0x131718));
            if s.device_rack_float.contains(&key) {
                ui.c.text(
                    fnt(13.0),
                    body.x + 10.0,
                    body.y + 8.0,
                    t.text_dim,
                    "Editor popped out",
                );
            } else if s.device_rack_failed.contains(&key) {
                ui.c.text(
                    fnt(13.0),
                    body.x + 10.0,
                    body.y + 8.0,
                    t.text_dim,
                    "Editor unavailable",
                );
            }
        }
        // Hosted, not a built-in: embedding comes with CLAP/VST3 embed (§7 milestone b) — a placeholder and a
        // Pop out button, which the floating window machinery takes.
        Some(key) => placeholder(ui, s, &d, body, Some(Action::DeviceRack(RackAct::PopOut(key)))),
        // The MIDI FX placeholder region (no data yet, §8).
        None => placeholder(ui, s, &d, body, None),
    }
}

/// A placeholder body: what the device is, and a `button` when there is one to press.
fn placeholder(ui: &mut Ui, s: &State, d: &Device, body: Rect, button: Option<Action>) {
    let t = ui.t;
    ui.c.fill_rect(body, t.s(0x131718));
    ui.c.stroke_rrect(body.inset(1.0), 2.0, 1.0, t.splitter_dark);
    let label = match d.kind {
        DeviceKind::MidiFx => "MIDI FX slots arrive with a later milestone",
        _ => "No embedded editor for this plugin yet",
    };
    ui.c.text(fnt(13.0), body.x + 10.0, body.y + 8.0, t.text_dim, label);
    // Live parameters (cached by `sync_embed`): the tile's controls when its editor cannot embed.
    if let Some(key) = d.key {
        let mut py = body.y + 56.0;
        for (name, val) in s.device_rack_params.get(&key).map_or(&Vec::new(), |v| v) {
            ui.c.text_clip(fnt(12.0), body.x + 10.0, py, body.w - 20.0, t.text_dim, name);
            let bar = Rect::new(body.x + 10.0, py + 15.0, (body.w - 20.0).min(180.0), 4.0);
            ui.c.fill_rect(bar, t.s(0x2b3134));
            ui.c.fill_rect(Rect::new(bar.x, bar.y, bar.w * (*val as f32).clamp(0.0, 1.0), bar.h), t.glyph);
            py += 26.0;
            if py > body.bottom() - 20.0 {
                break;
            }
        }
    }
    if let Some(action) = button {
        let b = Rect::new(body.x + 10.0, body.y + 30.0, 70.0, 20.0);
        let hov = ui.hit(b, action, "Pop the editor out to its own window");
        let st = kit::State::hover(hov);
        kit::button(ui.c, t, b, st);
        ui.c.text(fnt(13.0), b.x + 8.0, b.y + 2.0, t.text, "Pop out");
    }
}

/// A small flat header button (the rack's Pin / Hide).
fn header_button(ui: &mut Ui, r: Rect, label: &str, on: bool, action: Action, hint: &'static str) {
    let t = ui.t;
    let hov = ui.hit(r, action, hint);
    let base: Color = if on {
        t.s(0x324049)
    } else if hov {
        t.s(0x2b3134)
    } else {
        t.s(0x26292c)
    };
    ui.c.fill_rrect(r, 2.0, base);
    ui.c.text(
        fnt(13.0),
        r.x + 6.0,
        r.y + 1.0,
        if on { t.glyph } else { t.text_dim },
        label,
    );
}

/// An open device header menu, over the windows: Pop out (or Dock, per where the editor is now).
pub fn paint_menu(ui: &mut Ui, s: &State) {
    let Some(key) = s.device_rack_menu else {
        return;
    };
    let (_, w, h) = ((), s.screen.0, s.screen.1);
    let Some((d, tile)) = layout(s, panel_rect(s, w, h))
        .into_iter()
        .find(|(d, _)| d.key == Some(key))
    else {
        return;
    };
    let t = ui.t;
    let m = Rect::new(
        tile.right() - 70.0,
        tile.y + HEADER_H,
        70.0,
        2.0 * 22.0 + 4.0,
    );
    ui.c.fill_rrect(m, 3.0, t.s(0x26292c));
    ui.c.stroke_rrect(m, 3.0, 1.0, t.splitter_dark);
    // A click anywhere else dismisses it.
    ui.hit(
        Rect::new(0.0, 0.0, s.screen.0, s.screen.1),
        Action::DeviceRack(RackAct::MenuClose),
        "",
    );
    let popped = s.device_rack_float.contains(&key);
    let item = |ui: &mut Ui, y: f32, label: &'static str, act: Action| {
        let r = Rect::new(m.x + 2.0, y, m.w - 4.0, 22.0);
        let hov = ui.hit(r, act, label);
        if hov {
            ui.c.fill_rect(r, t.s(0x333a3e));
        }
        ui.c.text(fnt(14.0), r.x + 8.0, r.y + 3.0, t.text, label);
    };
    let y = m.y + 2.0;
    if popped {
        item(ui, y, "Dock", Action::DeviceRack(RackAct::Dock(key)));
    } else {
        item(ui, y, "Pop out", Action::DeviceRack(RackAct::PopOut(key)));
    }
    let _ = d;
}

/// The device rack's "＋ Add" browser popup (spec: ui/device-rack.md §3): one section per kind of plugin, built-in
/// (native) plugins first, then installed plugins; the display-mode radios below the list are the filter option
/// for the full plugin hierarchy. Clicking an entry adds the device to the rack (generators become a new channel
/// whose plugin hosts the browser's pick; effects go into the selected mixer track's first empty slot).
pub(crate) fn add_device_menu(s: &State, x: f32, y: f32) -> Popup {
    let mut items = Vec::new();
    items.push(PItem::head("Instruments"));
    items.extend(add_device_section(s, false));
    items.push(PItem::head("Effects"));
    items.extend(add_device_section(s, true));
    let mut p = Popup { x, y, items, sub: None, scroll: [0; 2] };
    p.items.extend(plugin_menu::footer(s));
    p
}

/// One device-rack browser section: built-in (native) plugins first, then the installed ones, in display order.
fn add_device_section(s: &State, effect: bool) -> Vec<PItem> {
    let list = if effect { &s.plugdb.lists.fx } else { &s.plugdb.lists.gens };
    let mut v = Vec::new();
    for e in list.iter().filter(|e| e.builtin) {
        v.push(PItem::new(e.label.clone(), pick(Target::Gen(None), effect, e.index)));
    }
    for e in list.iter().filter(|e| !e.builtin) {
        v.push(PItem::new(e.label.clone(), pick(Target::Gen(None), effect, e.index)));
    }
    v
}
