// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The device rack's view (Lemon Studio's design: spec model/track-model.md §7): a panel docked at the bottom of
//! the main window, resizable by its splitter and collapsible to its header, whose device tiles hold the selected
//! track's devices in signal-flow order. Each tile draws its device's own controls, in the app's canvas: the
//! parameters, names, sections and value texts the plugin's editor shows, docked or popped out.

use os_ui::{Color, Rect, kit};

use crate::app::{Action, State};
use crate::plugins::Plugins;
use crate::rack_ui::{EDITOR_TAB, EDITOR_SUBTAB, EDITOR_BANK, EDITOR_PAD, EDITOR_OPEN};
use crate::device_rack::{self, Device, DeviceKind, RackParam, HEADER_H, RackAct};
use crate::history::ChannelControl;
use crate::icons::{self, Icon};
use crate::views::{Ui, fnt};

/// The rack's panel rect in the window: the full width, its height at the bottom edge (`device_rack::height`).
pub fn panel_rect(s: &State, w: f32, h: f32) -> Rect {
    Rect::new(0.0, h - device_rack::height(s), w, device_rack::height(s))
}

/// The splitter strip along the panel's top edge (6 px, like the browser's).
const SPLITTER: f32 = 6.0;

/// Height of the project-modulator row between the rack header and the device headers: a label, eight slots and a
/// hairline. The row is fixed; only the tile body below it moves.
pub(crate) const MOD_H: f32 = 92.0;

/// The row's height for the current state (none while the rack is collapsed).
pub(crate) fn mod_row_h(s: &State) -> f32 {
    if s.device_rack_collapsed {
        0.0
    } else {
        MOD_H
    }
}

/// The eight slot rects in a row `r` wide, and the add/editor overlays' anchor. The slots shrink to fit a narrow
/// window rather than overflowing the rack.
pub(crate) fn mod_slots(s: &State, r: Rect) -> Vec<Rect> {
    if s.device_rack_collapsed {
        return Vec::new();
    }
    let row = Rect::new(r.x, r.y + HEADER_H, r.w, MOD_H);
    let gap = 5.0;
    let w = ((row.w - 14.0 - gap * 7.0) / 8.0).clamp(46.0, 84.0);
    let y = row.y + 16.0;
    let h = (MOD_H - 20.0).max(1.0);
    (0..8).map(|i| Rect::new(row.x + 7.0 + i as f32 * (w + gap), y, w, h)).collect()
}

/// Gap between tiles, holding the "+" insert buttons (before the first, between every
/// pair, and after the last tile).
const GAP: f32 = 22.0;

/// The device tiles over `r` (the panel rect) as (device, tile) pairs, left to right in signal-flow order; the
/// empty leftmost tile on a MIDI track is the MIDI FX placeholder (§8 is a later milestone).
pub(crate) fn layout(s: &State, r: Rect) -> Vec<(Device, Rect)> {
    let top = r.y + HEADER_H + mod_row_h(s);
    if r.h <= HEADER_H + mod_row_h(s) {
        return Vec::new();
    }
    let body = Rect::new(r.x, top, r.w, (r.bottom() - top).max(0.0));
    // Horizontally scrolled: the chain is wider than the panel at times, so the tiles (and every hit rect
    // derived from this layout) shift left by the clamped scroll offset.
    let scroll = scroll_x(s, r.w);
    device_rack::rack_chain(s)
        .into_iter()
        .scan(body.x - scroll, |x, d| {
            let w = tile_width(s, &d, editor_h(body.h));
            let tile = Rect::new(*x, body.y, w, body.h);
            *x += w + GAP;
            Some((d, tile))
        })
        .collect()
}

/// A tile's width: the width of the plugin editor the tile draws (`device_rack::tile_w`), else [`TILE_W`]. `s` has
/// the hosted plugin's name per device key (`State::device_rack_names`), which is what the editor is looked up by.
fn tile_width(s: &State, d: &Device, body_h: f32) -> f32 {
    device_rack::tile_w(plugin_name(s, d), body_h)
}

/// The name of the plugin behind a device: the hosted instance's own (`State::device_rack_names`), else the device's
/// title (a slot may be titled with the FL plugin's name, "Fruity Filter" for Lemon Filter).
fn plugin_name<'a>(s: &'a State, d: &'a Device) -> &'a str {
    d.key
        .and_then(|k| s.device_rack_names.get(&k))
        .map_or(d.name.as_str(), |n| n.as_str())
}

/// The chain's content width over a panel `w` wide: every tile plus the gap after it.
pub(crate) fn content_w(s: &State) -> f32 {
    let r = panel_rect(s, s.screen.0, s.screen.1);
    let body_h = (r.h - HEADER_H - mod_row_h(s)).max(0.0);
    device_rack::rack_chain(s).iter().map(|d| tile_width(s, d, editor_h(body_h)) + GAP).sum()
}

/// The rack's horizontal scroll clamped to what the panel's width `w` can show.
pub(crate) fn scroll_x(s: &State, w: f32) -> f32 {
    s.device_rack_scroll_x.clamp(0.0, (content_w(s) - w).max(0.0))
}

/// The wheel over the rack's body: the tile under the pointer scrolls its own controls when the device has more
/// than its body shows, else the chain scrolls horizontally (`d` notches, up = left, as the mixer's); `r` is the
/// rack panel rect. Returns whether the wheel was over the rack.
pub(crate) fn wheel(s: &mut State, r: Rect, x: f32, y: f32, d: f32) -> bool {
    let area = Rect::new(r.x, r.y + HEADER_H, r.w, (r.h - HEADER_H).max(0.0));
    if !area.contains(x, y) || d == 0.0 {
        return false;
    }
    if let Some((dev, tile)) = layout(s, r).into_iter().find(|(_, t)| t.contains(x, y)) {
        if let Some(key) = dev.key {
            let b = body(tile);
            let max = s.device_rack_params.get(&key).map_or(0.0, |ps| scroll_max(b, ps));
            if max > 0.0 {
                let at = s.device_rack_tile_scroll.entry(key).or_insert(0.0);
                *at = (*at - d * 34.0).clamp(0.0, max);
                return true;
            }
        }
    }
    s.device_rack_scroll_x = (scroll_x(s, r.w) - d * 44.0).clamp(0.0, (content_w(s) - r.w).max(0.0));
    true
}

/// The tiles at the current screen size (for hit-testing a tile drag's drop position).
pub(crate) fn tiles(s: &State) -> Vec<(Device, Rect)> {
    layout(s, panel_rect(s, s.screen.0, s.screen.1))
}

/// The chain gap a tile drag over `x` would drop at (0..=chain length): how many tile middles it passed.
pub(crate) fn drop_index(s: &State, x: f32) -> usize {
    tiles(s).iter().filter(|(_, t)| t.cx() < x).count()
}

/// Paints the drop indicator for a running tile drag: an accent bar in the gap the device would land at.
fn drop_highlight(ui: &mut Ui, s: &State, tiles: &[(Device, Rect)]) {
    if s.device_rack_drag.is_none() {
        return;
    }
    let Some(gap) = s.device_rack_drop else { return };
    let t = ui.t;
    let x = match tiles.get(gap) {
        Some((_, tile)) => tile.x - GAP / 2.0,
        None => tiles.last().map_or(0.0, |(_, tile)| tile.right() + GAP / 2.0),
    };
    let h = tiles.first().map_or(0.0, |(_, tile)| tile.h);
    if h <= 0.0 {
        return;
    }
    ui.c.fill_rect(Rect::new(x - 2.0, tiles[0].1.y, 4.0, h), t.glyph);
}

/// The keys of every device tile in the rack (any kind, floated or not): what the params cache covers so a hosted
/// (non-builtin) tile's body gets its controls too — the cache reads every instance's parameters.
pub fn device_keys(s: &State) -> Vec<usize> {
    let r = panel_rect(s, s.screen.0, s.screen.1);
    layout(s, r).into_iter().filter_map(|(d, _)| d.key).collect()
}

/// The height a tile's editor is given in a panel body `body_h` tall. The tile's width is measured against this — not
/// the whole tile — or the header would make the editor taller than it really is, and the editor would be drawn
/// height-limited inside a body-sized image: a column of its background down the tile's right edge.
fn editor_h(body_h: f32) -> f32 {
    (body_h - HEADER_H).max(1.0)
}

/// A tile's editor body: what is under its header.
fn body(tile: Rect) -> Rect {
    Rect::new(tile.x, tile.y + HEADER_H, tile.w, (tile.h - HEADER_H).max(0.0))
}

/// Paints the rack panel over `r` (its splitter, header and device tiles).
pub fn paint(ui: &mut Ui, s: &State, plugins: &Plugins, r: Rect) {
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
    ui.c.fill_rect(Rect::new(r.x, head.y + 4.0, 3.0, head.h - 8.0), t.accent);
    let name = device_rack::rack_track(s).map_or("No track selected".to_string(), |tr| {
        crate::playlist::track_name(s, tr.playlist_lane as u16 - 1)
    });
    // A technical module tag, then the current chain's track name. The tag is fixed-width so the name starts at
    // the same x for every track.
    ui.c.text(fnt(12.0), r.x + 12.0, head.y + 8.0, t.accent, "DEVICE RACK");
    ui.c.fill_rect(Rect::new(r.x + 91.0, head.y + 5.0, 1.0, head.h - 10.0), t.splitter_dark);
    ui.c.text_clip(fnt(15.0), r.x + 98.0, head.y + 6.5, (r.w - 330.0).max(20.0), t.text, &name);
    // The header buttons, right-aligned: ＋ Add (the plugin browser), Pin, Show / Hide — and, when the chain
    // overflows the panel, the ‹ › steppers (an alternative to the wheel).
    let overflow = crate::views::device_rack::content_w(s) > r.w
        && !s.device_rack_collapsed
        && r.h > HEADER_H;
    let mut bx = r.right() - 228.0 - 56.0 * f32::from(overflow);
    if overflow {
        let (at_start,) = (s.device_rack_scroll_x <= 0.5,);
        let steppers = [
            ("‹", -1.0, Rect::new(bx, head.y + 6.5, 24.0, 17.0)),
            ("›", 1.0, Rect::new(bx + 28.0, head.y + 6.5, 24.0, 17.0)),
        ];
        for (label, d, b) in steppers {
            let dim = d < 0.0 && at_start;
            if dim {
                header_button(ui, b, label, false, Action::DeviceRack(RackAct::ScrollBy(0.0)), "");
            } else {
                ui.hit(
                    b,
                    Action::DeviceRack(RackAct::ScrollBy(d)),
                    if d < 0.0 { "Scroll the chain left" } else { "Scroll the chain right" },
                );
                header_button(ui, b, label, false, Action::DeviceRack(RackAct::ScrollBy(0.0)), "");
            }
        }
        bx += 56.0;
    }
    header_button(
        ui,
        Rect::new(bx, head.y + 6.5, 60.0, 17.0),
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
        Rect::new(bx, head.y + 6.5, 60.0, 17.0),
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
        Rect::new(bx, head.y + 6.5, 60.0, 17.0),
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
    bx += 66.0;
    // The out-arrow: every device in the chain shows its editor in its own floating window (lit green
    // while they are out), or the windows close and the tiles draw the editors again. An out-arrow,
    // not a wrench — the wrench stays the device wrapper's own symbol (the plugin window's strip).
    let windows = !s.device_rack_float.is_empty();
    header_icon_button(
        ui,
        Rect::new(bx, head.y + 6.5, 24.0, 17.0),
        Icon::PopOut,
        windows,
        Action::DeviceRack(RackAct::ToggleEditors),
        if windows {
            "Close the devices' floating editor windows and draw them in their tiles"
        } else {
            "Show every docked device's editor in its own floating window"
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
        tile_paint(ui, s, plugins, i, d.clone(), *tile);
        gx = tile.right();
    }
    gap_button(ui, gx, r.y + HEADER_H + mod_row_h(s), None, tiles.first().map_or(r.h - HEADER_H - mod_row_h(s), |(_, t)| t.h));
    drop_highlight(ui, s, &tiles);
    // The modulator row over the device headers, and its editor / menus over everything (the handoff §2).
    mod_row_paint(ui, s, r);
    mod_overlays_paint(ui, s, r);
}

/// A "+" gap button at x: opens the device browser for the slot that follows it (`None` = first empty slot).
fn gap_button(ui: &mut Ui, x: f32, y: f32, slot: Option<u8>, h: f32) {
    let t = ui.t;
    let b = Rect::new(x + 2.0, y + h / 2.0 - 9.0, GAP - 4.0, 18.0);
    let hov = ui.hit(b, Action::DeviceRack(RackAct::AddAt(slot)), "Add a device here");
    ui.c.fill_rrect(b, 2.0, if hov { t.s(0x2b3134) } else { t.s(0x222629) });
    ui.c.text(fnt(14.0), b.x + 4.0, b.y + 1.0, t.text, "+");
}

/// A device tile: a header (name, enable light, wrench, menu) over a body of the device's own controls. A
/// selected tile's header is brighter: the header bar is clickable and selects (§7).
fn tile_paint(ui: &mut Ui, s: &State, plugins: &Plugins, i: usize, d: Device, tile: Rect) {
    let t = ui.t;
    let selected = device_rack::tile_selected(s, i);
    // The tile: a raised surface with a hairline between header and body.
    ui.c.fill_rect(tile, t.s(0x222629));
    ui.c.stroke_rrect(tile, 0.0, 1.0, t.splitter_dark);
    let head = Rect::new(tile.x, tile.y, tile.w, HEADER_H);
    // The kind's gradient behind the header's content, veiled so the LED and name stay readable (the pairs are
    // constants in `device_rack` so user theming can rewire them later). The selected tile's veil is lighter,
    // so its bar reads slightly brighter.
    let (top, bottom) = device_rack::header_gradient(d.kind);
    ui.c.fill_rect_gradient(head, t.s(top), t.s(bottom), 0.0);
    ui.c.fill_rect(head, os_ui::Color::rgb(0).with_alpha(if selected { 60 } else { 110 }));
    // A hard kind strip at the left edge: the device's family is readable before the name is.
    ui.c.fill_rect(Rect::new(tile.x, head.y, 3.0, head.h), t.s(top).shade(-0.12));
    if selected {
        ui.c.fill_rect(Rect::new(tile.x, head.bottom() - 2.0, tile.w, 2.0), t.accent);
    }
    ui.c.fill_rect(
        Rect::new(tile.x, tile.bottom() - 1.0, tile.w, 1.0),
        t.splitter_dark,
    );
    // The header selects the tile (Shift+click extends a range); the press also starts the drag, so the bar
    // still drags left/right to reorder (FX tiles swap mixer slots). The LED and buttons below win their clicks.
    if d.key.is_some() || matches!(d.kind, DeviceKind::Container { .. }) {
        ui.hit(head, Action::DeviceRack(RackAct::SelectTile(i)), if selected { "Selected: click another tile to select it, Shift+click to extend the selection" } else { "Click to select this device, Shift+click to extend the selection; drag to reorder" });
    }
    // The enable light: a click switches the device (the channel or the mixer slot).
    let (cx, cy) = (tile.x + 12.0, head.cy());
    crate::views::led(ui, cx, cy, d.on);
    let led = Rect::new(cx - 7.0, cy - 7.0, 14.0, 14.0);
    if d.key.is_some() || matches!(d.kind, DeviceKind::Container { .. }) {
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
    // The name, then the header's pop-out and menu, the channel's knobs standing in front of them.
    let menu = Rect::new(tile.right() - 22.0, cy - 7.5, 17.0, 15.0);
    let popout = Rect::new(menu.x - 22.0, cy - 9.5, 19.0, 19.0);
    // The drum machine's tile flips between its pads and its cards: one more button, and the name
    // gives up the room it takes.
    let flip = Rect::new(popout.x - 22.0, cy - 9.5, 19.0, 19.0);
    let drum = d.key.is_some_and(|_| matches!(crate::rack_ui::ui(plugin_name(s, &d)), Some(crate::rack_ui::Ui::Drum)));
    // The Trio's pages are the tile's to show, its editor being hosted here: three buttons beside the pop-out, where
    // the editor's own strip row keeps them in its window.
    let trio = d.key.is_some_and(|_| matches!(crate::rack_ui::ui(plugin_name(s, &d)), Some(crate::rack_ui::Ui::Trio)));
    // The button cluster's left edge — the trio's pages, the drum's flip, or the pop-out alone — and the channel's
    // knobs in front of it, which the name gives room to (only an instrument has a channel behind its tile).
    let cluster = tile.right()
        - if drum {
            66.0
        } else if trio {
            110.0
        } else {
            44.0
        };
    let knobs = matches!(d.kind, DeviceKind::Instrument(_)).then(|| cluster - 8.0 - KNOB_STEP * 3.0);
    let room = knobs.unwrap_or(cluster) - 6.0 - (tile.x + 24.0);
    ui.c.text_clip(fnt(14.0), tile.x + 24.0, cy - 8.0, room.max(12.0), t.text, &d.name);
    // The pop-out arrow: pop the device's editor out to its own floating window (or dock it back, lit
    // while it is out). An out-arrow, not a wrench — the wrench is the device wrapper's own symbol
    // (the plugin window's Misc tab), so the tile's button says what it does instead.
    match d.kind {
        DeviceKind::Container { t: ct, k: ck } => {
            header_icon_button(
                ui,
                popout,
                Icon::Wrench,
                false,
                Action::DeviceRack(RackAct::OpenContainer { t: ct, k: ck }),
                "Open the container",
            );
        }
        _ if d.key.is_some() => {
            let key = d.key.unwrap();
            let floated = s.device_rack_float.contains(&key);
            header_icon_button(
                ui,
                popout,
                Icon::PopOut,
                floated,
                Action::DeviceRack(if floated { RackAct::Dock(key) } else { RackAct::PopOut(key) }),
                if floated { "Dock this device's editor in its tile" } else { "Pop this device's editor out to its own window" },
            );
        }
        _ => {}
    }
    // The drum machine's flip: its pads, or the other half of its UI (the cards), in place — no pop-out.
    if trio {
        let key = d.key.unwrap();
        let tab = match s.rack_ui_cache.borrow().view(&key) {
            Some(crate::rack_ui::TileView::Trio(v)) => v.tab,
            _ => 0,
        };
        // The same three pages, in the editor's order: oscillators, envelopes, the custom shape's sample.
        for (n, icon, hint) in [
            (0u8, Icon::Plug, "Oscillators"),
            (1, Icon::Timeline, "Envelopes, LFOs and filter"),
            (2, Icon::Wave, "Custom shape sample"),
        ] {
            let r = Rect::new(popout.x - 22.0 * f32::from(3 - n), cy - 9.5, 19.0, 19.0);
            header_icon_button(
                ui,
                r,
                icon,
                tab == n,
                Action::DeviceRack(RackAct::EditorTab { key, n }),
                hint,
            );
        }
    }
    if drum {
        let key = d.key.unwrap();
        let flipped = matches!(
            s.rack_ui_cache.borrow().view(&key),
            Some(crate::rack_ui::TileView::Drum { flip: true, .. })
        );
        header_icon_button(
            ui,
            flip,
            Icon::Slip,
            flipped,
            Action::DeviceRack(RackAct::EditorFlip { key }),
            if flipped { "Show this machine's pads" } else { "Show this machine's pad and layer cards" },
        );
    }
    if d.key.is_some()
        && ui.hit(
            menu,
            Action::DeviceRack(RackAct::Menu(d.key.unwrap())),
            "Device menu: Pop out / Dock",
        )
    {
        ui.c.text(fnt(13.0), menu.x, menu.y, t.text, "v");
    }
    // The channel's PAN / VOL / PITCH, in the header beside the name.
    if let (DeviceKind::Instrument(row), Some(left)) = (d.kind, knobs) {
        channel_knobs(ui, s, row, left, cy);
    }
    let body = body(tile);
    // The empty instrument slot: the whole tile opens the device browser (the instrument section of it).
    if d.kind == DeviceKind::NoInstrument {
        ui.c.fill_rect(body, t.s(0x131718));
        ui.c.stroke_rrect(body.inset(1.0), 2.0, 1.0, t.splitter_dark);
        ui.c.text(fnt(14.0), tile.x + 24.0, body.y + 10.0, t.text_dim, "This track has no instrument yet.");
        ui.c.text(fnt(13.0), tile.x + 24.0, body.y + 30.0, t.text_dim, "Click to choose one.");
        let _ = ui.hit(body, Action::DeviceRack(RackAct::AddAt(None)), "Choose this track's instrument");
        return;
    }
    // A container tile: its body lists its members; a click opens the container window (its wrench above does).
    if let DeviceKind::Container { t: ct, k: ck } = d.kind {
        container_body(ui, s, body, ct, ck);
        return;
    }
    // The device's own controls, drawn here rather than by an embedded native window (see
    // `device_rack::sync_embed`): the same names, sections and value texts its editor shows.
    controls(ui, s, plugins, &d, body);
}

/// A container tile's body: the members in chain order ("1. Lemon Limiter"), and the empty container's hint.
/// A click (like the wrench) opens the container window, whose list edits the members.
fn container_body(ui: &mut Ui, s: &State, body: Rect, t: u16, k: u8) {
    let th = ui.t;
    ui.c.fill_rect(body, th.s(0x131718));
    ui.c.stroke_rrect(body.inset(1.0), 2.0, 1.0, th.splitter_dark);
    ui.hit(body, Action::DeviceRack(RackAct::OpenContainer { t, k }), "Open the container");
    let members = crate::container::members(s, t, k).unwrap_or_default();
    if members.is_empty() {
        ui.c.text(fnt(13.0), body.x + 10.0, body.y + 9.0, th.text_dim, "Empty container.");
        ui.c.text(fnt(13.0), body.x + 10.0, body.y + 29.0, th.text_dim, "Open it to add effects.");
        return;
    }
    ui.c.push_clip(body);
    for (i, m) in members.iter().take(8).enumerate() {
        let y = body.y + 9.0 + i as f32 * 19.0;
        if y > body.bottom() - 16.0 {
            break;
        }
        ui.c.text_clip(fnt(13.0), body.x + 10.0, y, body.w - 20.0, th.text, &format!("{}. {}", i + 1, m.shown()));
    }
    if members.len() > 8 {
        ui.c.text_clip(fnt(12.0), body.x + 10.0, body.bottom() - 20.0, body.w - 20.0, th.text_dim, &format!("+{} more", members.len() - 8));
    }
    ui.c.pop_clip();
}

/// The channel's PAN / VOL / PITCH, in a tile's header where the plugin window's strip has them: dragging one up /
/// down sets it through the channel's undo-aware path. Each knob carries its own word (VOL, PAN) or its value (the
/// pitch's semitones) in its own colour, so the row needs no caption. Those words and colours are a stopgap — user-set
/// labels and colours are on `todo.md`.
pub(crate) const KNOB_STEP: f32 = 34.0;

fn channel_knobs(ui: &mut Ui, s: &State, row: usize, left: f32, cy: f32) {
    let Some(ch) = s.project.channels.get(row) else { return };
    let range = i32::from(ch.misc.pitch_range.clamp(1, 48));
    let pitch = 0.5 + f64::from(ch.pitch_cents) / (200.0 * f64::from(range));
    let semi = (ch.pitch_cents as f32 / 100.0).round() as i32;
    let pitch_text = if semi > 0 { format!("+{semi}") } else { semi.to_string() };
    let knobs = [
        ("VOL", ChannelControl::Volume, ch.volume, false, 0x9ed44a, 8.5),
        ("PAN", ChannelControl::Pan, (ch.pan + 1.0) * 0.5, true, 0x54a6e8, 8.5),
        ("PITCH", ChannelControl::Pitch, pitch, true, 0xe0a040, 9.5),
    ];
    for (i, (label, which, v, bipolar, accent, font)) in knobs.into_iter().enumerate() {
        // The pitch knob is a numbered dial: its face shows the semitones rather than a word that would not fit.
        let text = if which == ChannelControl::Pitch { pitch_text.clone() } else { label.to_string() };
        let cell = Rect::new(left + i as f32 * KNOB_STEP, cy - KNOB_STEP * 0.5, KNOB_STEP, KNOB_STEP);
        let hov = ui.hit(
            cell,
            Action::DeviceRack(RackAct::ChannelKnob { row, which, v }),
            format!("{label}: drag up / down to set it, double click to reset it"),
        );
        header_knob(ui, cell.cx(), cy, 13.0, v.clamp(0.0, 1.0) as f32, bipolar, Color::rgb(accent), &text, font, hov);
    }
}

/// A knob in a tile's header: a 270° ring in its own colour round a dark disc that carries its word or value. Too
/// small for a caption or a pointer line — the arc is the reading, the face is the label.
fn header_knob(
    ui: &mut Ui,
    cx: f32,
    cy: f32,
    r: f32,
    v: f32,
    bipolar: bool,
    accent: Color,
    text: &str,
    font: f32,
    hov: bool,
) {
    let t = ui.t;
    let (start, sweep) = (0.75 * std::f32::consts::PI, 1.5 * std::f32::consts::PI);
    ui.c.arc(cx, cy, r, start, sweep, 2.4, t.s(0x2a2f31));
    let a = start + sweep * v.clamp(0.0, 1.0);
    let a0 = if bipolar { start + sweep / 2.0 } else { start };
    if (a - a0).abs() > 0.01 {
        ui.c.arc(cx, cy, r, a0.min(a), (a - a0).abs(), 2.4, accent);
    }
    ui.c.fill_circle(cx, cy, r - 3.2, if hov { t.s(0x2b3134) } else { t.s(0x1b1f21) });
    ui.c.text_center(fnt(font), Rect::new(cx - r, cy - 6.0, 2.0 * r, 12.0), accent, text);
}

/// One control cell: the name above, the dial (or the switch), the value below.
const CELL_W: f32 = 62.0;
const CELL_H: f32 = 64.0;
/// A section row (a parameter group's name) above its cells.
const GROUP_H: f32 = 15.0;
/// The body's padding.
const PAD: f32 = 7.0;

/// A tile body's laid-out controls: the section headers, the cells and the content height (which the wheel's
/// scroll clamps against).
struct Grid {
    sections: Vec<(Rect, String)>,
    cells: Vec<(usize, Rect)>,
    height: f32,
}

/// Lays out a body's control cells `scroll` px up: `cols` per row at `CELL_W`, with a header row above each named
/// parameter group. The one geometry `controls` paints, `param_dial` hit-tests and `scroll_max` measures.
fn grid(body: Rect, params: &[RackParam], scroll: f32) -> Grid {
    let cols = (((body.w - 2.0 * PAD) / CELL_W).floor().max(1.0)) as usize;
    let x0 = body.x + (body.w - cols as f32 * CELL_W) * 0.5;
    let mut g = Grid { sections: Vec::new(), cells: Vec::new(), height: 0.0 };
    let top = body.y + PAD - scroll;
    let mut y = top;
    let mut group = "\u{0}";
    let mut i = 0;
    while i < params.len() {
        let name = params[i].group.as_str();
        if name != group {
            group = name;
            if !name.is_empty() {
                g.sections.push((Rect::new(body.x + PAD, y, body.w - 2.0 * PAD, GROUP_H), name.to_string()));
                y += GROUP_H;
            }
        }
        let mut col = 0;
        while i < params.len() && params[i].group == group {
            g.cells.push((i, Rect::new(x0 + col as f32 * CELL_W, y, CELL_W, CELL_H)));
            i += 1;
            col += 1;
            if col == cols {
                col = 0;
                y += CELL_H;
            }
        }
        if col != 0 {
            y += CELL_H;
        }
    }
    g.height = y - top + 2.0 * PAD;
    g
}

/// How far a tile's controls scroll: their content past the body (0 when they all fit).
fn scroll_max(body: Rect, params: &[RackParam]) -> f32 {
    (grid(body, params, 0.0).height - body.h).max(0.0)
}

/// A control's hit rect in its tile body, by hosted key and param id: the knob and its value together, so the
/// whole cell is draggable (the same geometry `controls` paints). The paint path does not need it — it has each
/// cell at hand — so it exists for the rack's own hit-testing tests.
#[cfg(test)]
pub(crate) fn param_dial(s: &State, key: usize, id: u32) -> Option<Rect> {
    let (_, w, h) = ((), s.screen.0, s.screen.1);
    let (_, tile) = layout(s, panel_rect(s, w, h)).into_iter().find(|(d, _)| d.key == Some(key))?;
    let params = s.device_rack_params.get(&key)?;
    let i = params.iter().position(|p| p.id == id)?;
    let body = body(tile);
    let scroll = s.device_rack_tile_scroll.get(&key).copied().unwrap_or(0.0);
    let (_, cell) = *grid(body, params, scroll).cells.iter().find(|(j, _)| *j == i)?;
    Some(Rect::new(cell.x + 4.0, cell.y + 18.0, CELL_W - 8.0, CELL_H - 20.0))
}

/// A tile's body: the device's own controls, as its editor shows them — a section header per parameter group, then
/// a cell per parameter (its name, a dial or a switch, and the plugin's own value text). More controls than the
/// body fits scroll with the wheel (the wheel's `scroll_max` clamp); a bar on the right edge says so.
fn controls(ui: &mut Ui, s: &State, plugins: &Plugins, d: &Device, body: Rect) {
    let t = ui.t;
    ui.c.fill_rect(body, t.s(0x131718));
    ui.c.stroke_rrect(body.inset(1.0), 2.0, 1.0, t.splitter_dark);
    let Some(key) = d.key else {
        ui.c.text_clip(fnt(13.0), body.x + 10.0, body.y + 9.0, body.w - 20.0, t.text_dim, "No device");
        return;
    };
    // The plugin's own editor, drawn by the app: the same panels, dials, switches and wordmark its window shows
    // (`crate::rack_ui`). A plugin without a shared editor (an instrument, a third-party plugin) keeps the grid.
    if let Some(editor) = crate::rack_ui::ui(plugin_name(s, d)) {
        editor_body(ui, s, plugins, key, editor, body);
        return;
    }
    let params: &[RackParam] = s.device_rack_params.get(&key).map_or(&[], |v| v.as_slice());
    if params.is_empty() {
        ui.c.text_clip(fnt(13.0), body.x + 10.0, body.y + 9.0, body.w - 20.0, t.text_dim, "No parameters to show");
        return;
    }
    let max = scroll_max(body, params);
    let scroll = s.device_rack_tile_scroll.get(&key).copied().unwrap_or(0.0).clamp(0.0, max);
    let g = grid(body, params, scroll);
    // Scrolled controls are clipped to the body: the row the body's top edge cuts through must not paint over the
    // tile's header (its switches and value texts used to sit on the header bar).
    ui.c.push_clip(body);
    for (r, name) in &g.sections {
        if r.bottom() <= body.y + 2.0 || r.y >= body.bottom() - 2.0 {
            continue;
        }
        ui.c.text_clip(fnt(11.0), r.x + 2.0, r.y + 1.0, r.w - 4.0, t.text, name);
        ui.c.fill_rect(Rect::new(r.x + 2.0, r.bottom() - 3.0, r.w - 4.0, 1.0), t.splitter_dark);
    }
    for (i, cell) in &g.cells {
        // A cell scrolled past the body is not drawn (and its hit goes with it).
        if cell.bottom() <= body.y || cell.y >= body.bottom() {
            continue;
        }
        cell_paint(ui, s, key, *i, &params[*i], *cell, body);
    }
    // The scroll bar: how much of the control list the body is showing, and where.
    if max > 0.0 {
        let track = Rect::new(body.right() - 4.0, body.y + 2.0, 3.0, body.h - 4.0);
        ui.c.fill_rect(track, t.s(0x1b1f21));
        let len = (track.h * body.h / (body.h + max)).max(14.0);
        let top = track.y + (track.h - len) * (scroll / max);
        ui.c.fill_rect(Rect::new(track.x, top, 3.0, len), t.s(0x4a5358));
    }
    ui.c.pop_clip();
}

/// A tile body showing the device's own editor: the plugin's UI (`crate::rack_ui`) filling the body — stretched to
/// the space the tile has, so no bars sit beside it — with every dial, switch and selector of it live on the hosted
/// parameter it names.
fn editor_body(ui: &mut Ui, s: &State, plugins: &Plugins, key: usize, editor: crate::rack_ui::Ui, body: Rect) {
    let t = ui.t;
    let params: &[crate::device_rack::RackParam] = s.device_rack_params.get(&key).map_or(&[], |v| v.as_slice());
    let (ew, eh) = editor.size();
    if ew <= 0.0 || eh <= 0.0 {
        return;
    }
    // The editor fills the body: the container keeps its size, only the plugin's UI inside it grows to fill it (a
    // 2 px inset keeps the tile's frame visible).
    let r = Rect::new(body.x + 2.0, body.y + 2.0, (body.w - 4.0).max(1.0), (body.h - 4.0).max(1.0));
    // The cache key: the stretch the body asks for (it changes when the body does, so the canvas re-renders).
    let k = (r.w / ew).max(r.h / eh).clamp(0.12, 3.0);
    // Rendered at the app's scale, covering the body's own size: the editor draws whole at a uniform
    // scale and its background covers the rest of the canvas, so there are no bars and no squash.
    let scale = ui.c.scale();
    let (cw, ch) = ((r.w * scale).round().max(1.0) as u32, (r.h * scale).round().max(1.0) as u32);
    let Some(img) = crate::rack_ui::image(s, plugins, key, editor.name(), params, k * scale, cw, ch) else {
        ui.c.text_clip(fnt(13.0), body.x + 10.0, body.y + 9.0, body.w - 20.0, t.text_dim, "This device's editor could not be drawn");
        return;
    };
    ui.c.image(r, img.w, img.h, &img.px);
    // The editor's controls, in the tile: a dial or fader drags its parameter, a switch or a selector row sets it.
    // The hits are in the image's device px; the blit maps them onto the body.
    let kx = r.w / img.w as f32;
    let ky = r.h / img.h as f32;
    // The controls the modulation graph drives: after their own hits are in, each gets its arcs (a knob) or a range
    // beside its track (a fader), with handles that drag the depth, drawn over the editor's pixels.
    let mut driven: Vec<(Rect, u32, f32)> = Vec::new();
    for (hr, id, v) in &img.hits {
        let cell = Rect::new(r.x + hr.x * kx, r.y + hr.y * ky, (hr.w * kx).max(2.0), (hr.h * ky).max(2.0));
        if let Some(index) = params.iter().position(|p| p.id == *id) {
            if let Some(target) = crate::params::target_of(s, key, index) {
                if !crate::modulation::routings_on(s, target).is_empty() {
                    driven.push((cell, target, params[index].value.clamp(0.0, 1.0) as f32));
                }
            }
        }
        let n = (*v).unwrap_or(0.0) as u8;
        let act = if *id == EDITOR_TAB {
            RackAct::EditorTab { key, n }
        } else if *id == EDITOR_SUBTAB {
            RackAct::EditorSubTab { key, n }
        } else if *id == EDITOR_BANK {
            RackAct::EditorBank { key, n }
        } else if *id == EDITOR_PAD {
            RackAct::EditorPad { key, n }
        } else if *id == EDITOR_OPEN {
            RackAct::EditorOpen { key }
        } else {
            let p = params.iter().find(|p| p.id == *id);
            match v {
                Some(v) => RackAct::ParamTap { key, id: *id, v: Some(*v) },
                None => RackAct::ParamSet { key, id: *id, v: p.map_or(0.0, |p| p.value) },
            }
        };
        let hint = if matches!(*id, EDITOR_TAB | EDITOR_SUBTAB | EDITOR_BANK | EDITOR_PAD | EDITOR_OPEN) {
            String::new()
        } else {
            let p = params.iter().find(|p| p.id == *id);
            p.map_or(String::new(), |p| p.text.clone())
        };
        ui.hit(cell, Action::DeviceRack(act), hint);
    }
    ui.c.push_clip(body);
    for (cell, target, base) in driven {
        if cell.h > cell.w * 1.6 {
            // A fader: the range runs up its track, its base at the bottom.
            mod_fader(ui, s, target, cell.cx(), base, |v| cell.bottom() - v.clamp(0.0, 1.0) * cell.h);
        } else {
            let radius = (cell.w.min(cell.h) * 0.5 + 3.0).max(7.0);
            mod_arcs(ui, s, target, cell.cx(), cell.cy(), radius, base);
        }
    }
    ui.c.pop_clip();
}

/// One control cell: its name, a dial (or a switch for a two-state parameter) and the plugin's own value text.
fn cell_paint(ui: &mut Ui, s: &State, key: usize, index: usize, p: &RackParam, cell: Rect, body: Rect) {
    let t = ui.t;
    let cx = cell.cx();
    // Routing mode tints every control project modulation can reach (the handoff §4.1). This is the same predicate
    // that decides whether the drag assigns, so a tint never lies.
    let target = crate::params::target_of(s, key, index);
    let eligible = s.mod_routing.is_some() && target.is_some_and(|t| crate::modulation::is_modulatable(s, t));
    if eligible {
        ui.c.fill_rrect(cell, 2.0, ui.t.modulation.with_alpha(30));
        ui.c.stroke_rrect(cell, 2.0, 1.0, ui.t.modulation.with_alpha(110));
    }
    ui.c.text_clip(fnt(11.0), cell.x + 2.0, cell.y + 1.0, CELL_W - 4.0, if eligible { Color::rgb(0x9ff5ea) } else { t.text_dim }, &p.name);
    // Clamped to the body like the paint: a row the body's top edge cuts through does not take the header's clicks.
    let hit = Rect::new(cell.x + 4.0, cell.y + 18.0, CELL_W - 8.0, CELL_H - 20.0).intersect(&body);
    // A switch flips on the press; a dial starts at its value and follows a vertical drag (the one set path,
    // `device_rack::set_param_value`).
    let v = if p.toggle { f64::from(p.value < 0.5) } else { p.value };
    let hov = ui.hit(
        hit,
        Action::DeviceRack(RackAct::ParamSet { key, id: p.id, v }),
        "Drag up / down to set this parameter",
    );
    if p.toggle {
        let b = Rect::new(cx - 8.0, cell.y + 27.0, 16.0, 16.0);
        let on = p.value >= 0.5;
        ui.c.fill_rrect(b, 3.0, if on { t.glyph } else { t.s(0x24282b) });
        ui.c.stroke_rrect(b, 3.0, 1.0, if hov || on { t.text } else { t.splitter_dark });
    } else {
        kit::knob(
            ui.c,
            t,
            cx,
            cell.y + 35.0,
            13.0,
            p.value.clamp(0.0, 1.0) as f32,
            false,
            kit::KnobLook::Ring,
            kit::State::hover(hov),
        );
    }
    ui.c.text_center(fnt(11.0), Rect::new(cell.x, cell.y + 47.0, CELL_W, 14.0), t.text_dim, &p.text);
    // The assigned control's modulation range (the handoff §4.3): one arc per source, from the base to base + depth,
    // an endpoint handle that drags that depth, and a moving cursor at the live value. The knob keeps showing the
    // base, independently draggable — modulation applies on top and never rewrites it.
    if let Some(target) = target {
        mod_arcs(ui, s, target, cx, cell.y + 35.0, 17.5, p.value.clamp(0.0, 1.0) as f32);
    }
}

/// The knob angles a control's arc uses: 135° round to 405°, clockwise (the kit's ring).
const ARC_START: f32 = 2.356_194_5;
const ARC_SWEEP: f32 = 4.712_389;

/// One assignment's ink, by its place in the list: the first is warm, the next cool, so two sources on one control
/// stay apart.
fn mod_ink(base: Color, n: usize, enabled: bool) -> Color {
    const INK: [u8; 3] = [255, 220, 200];
    base.with_alpha(if enabled { INK[n % INK.len()] } else { 70 })
}

/// The arcs a control's assignments draw round a knob (`r0` is the first arc's radius): one per source, from the
/// base value to base + depth, an endpoint handle that drags that depth, and a cursor at the live value. Shared by
/// the rack's parameter cells, the mixer's knobs and the docked plugin editors' knobs.
pub(crate) fn mod_arcs(ui: &mut Ui, s: &State, target: u32, cx: f32, cy: f32, r0: f32, base: f32) {
    let routings = crate::modulation::routings_on(s, target);
    if routings.is_empty() {
        return;
    }
    for (n, (id, depth, enabled)) in routings.iter().enumerate() {
        let r = r0 + n as f32 * 2.5;
        let a0 = ARC_START + base * ARC_SWEEP;
        let a1 = ARC_START + (base + depth).clamp(-1.5, 2.5) * ARC_SWEEP;
        let c = mod_ink(ui.t.modulation, n, *enabled);
        ui.c.arc(cx, cy, r, a0.min(a1), (a1 - a0).abs(), 2.0, c);
        // The endpoint handle: drag up / down for more or less depth on this one source.
        let (hx, hy) = (cx + r * a1.cos(), cy + r * a1.sin());
        let hr = Rect::new(hx - 4.0, hy - 4.0, 8.0, 8.0);
        let hov = ui.hit(hr, Action::Mod(crate::modulation::ModAct::DepthDrag(*id, target)), "Drag to set this modulation's depth");
        ui.c.fill_circle(hx, hy, if hov { 4.0 } else { 3.0 }, c);
    }
    // The moving cursor: where the modulated value actually is, from the engine's lock-free snapshot.
    if let Some(v) = crate::modulation::live_value(s, target) {
        let a = ARC_START + v.clamp(-1.5, 2.5) * ARC_SWEEP;
        let (x0, y0) = (cx + (r0 - 3.5) * a.cos(), cy + (r0 - 3.5) * a.sin());
        let (x1, y1) = (cx + (r0 + 2.5) * a.cos(), cy + (r0 + 2.5) * a.sin());
        ui.c.line(x0, y0, x1, y1, 2.0, Color::rgb(0xd8fffa).with_alpha(230));
    }
}

/// A fader's assignments: a vertical range beside the track from the base to base + depth, a handle at that end that
/// drags the depth, and a tick at the live value. `y_at` maps a normalized 0..1 value to the track's y, so the range
/// follows whatever law the fader uses.
///
/// A control's own hits are registered before this runs, so its handles are the last word under the pointer.
pub(crate) fn mod_fader(ui: &mut Ui, s: &State, target: u32, x: f32, base: f32, y_at: impl Fn(f32) -> f32) {
    let routings = crate::modulation::routings_on(s, target);
    if routings.is_empty() {
        return;
    }
    let y0 = y_at(base);
    for (n, (id, depth, enabled)) in routings.iter().enumerate() {
        let y1 = y_at(base + depth);
        let c = mod_ink(ui.t.modulation, n, *enabled);
        let xx = x + 4.0 + n as f32 * 2.5;
        ui.c.line(xx, y0, xx, y1, 2.0, c);
        let hr = Rect::new(xx - 4.0, y1 - 4.0, 8.0, 8.0);
        let hov = ui.hit(hr, Action::Mod(crate::modulation::ModAct::DepthDrag(*id, target)), "Drag to set this modulation's depth");
        ui.c.fill_circle(xx, y1, if hov { 4.0 } else { 3.0 }, c);
    }
    if let Some(v) = crate::modulation::live_value(s, target) {
        let y = y_at(v);
        ui.c.line(x - 3.5, y, x + 3.5, y, 1.5, Color::rgb(0xd8fffa).with_alpha(230));
    }
}

/// A small flat header button with an icon only (the rack's wrench): a lit one is green (the devices' editor
/// windows are out).
fn header_icon_button(ui: &mut Ui, r: Rect, icon: Icon, on: bool, action: Action, hint: &'static str) {
    let t = ui.t;
    let hov = ui.hit(r, action, hint);
    let base: Color = if on {
        t.led(0x3d7803)
    } else if hov {
        t.s(0x2b3134)
    } else {
        t.s(0x26292c)
    };
    ui.c.fill_rrect(r, 2.0, base);
    let ink = if on {
        t.led(0xd7f7a6)
    } else if hov {
        t.glyph
    } else {
        t.text_dim
    };
    icons::draw(ui.c, icon, Rect::new(r.cx() - 7.0, r.cy() - 7.0, 14.0, 14.0), ink);
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

/// One slot's short line: what it is and how fast ("sine 1/4", "random 1/2", "low band").
pub(crate) fn slot_text(m: &os_model::modulation::Modulator) -> String {
    use os_model::modulation::{Band, ModSource};
    match m.source {
        ModSource::Lfo(l) => format!("{:?} {}", l.shape, crate::modulation::rate_text(l.period)).to_lowercase(),
        ModSource::Random(r) => format!("random {}", crate::modulation::rate_text(r.period)),
        ModSource::AudioFollower(f) => match f.band {
            Band::Full => "full band".into(),
            Band::Low => "low band".into(),
            Band::Mid => "mid band".into(),
            Band::High => "high band".into(),
            Band::Custom { .. } => "custom band".into(),
        },
        ModSource::Steps(st) => format!("steps {}", crate::modulation::rate_text(st.period)),
        ModSource::Envelope(_) => "envelope".into(),
    }
}

/// How bright the patch-point button is while routing mode is on: a flash that keeps pulsing, so the mode is never
/// mistaken for normal.
fn route_flash() -> u8 {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let phase = ((ms % 900) as f32 / 900.0) * std::f32::consts::TAU;
    (140.0 + 115.0 * (0.5 + 0.5 * phase.sin())) as u8
}

/// The eight project-modulator slots over the device headers. They are project state, so every track shows the same
/// eight; only the devices below them change with the selection.
pub(crate) fn mod_row_paint(ui: &mut Ui, s: &State, r: Rect) {
    if s.device_rack_collapsed {
        return;
    }
    let t = ui.t;
    let row = Rect::new(r.x, r.y + HEADER_H, r.w, MOD_H);
    if row.h <= 0.0 {
        return;
    }
    ui.c.fill_rect(row, t.s(0x15181a));
    ui.c.fill_rect(Rect::new(row.x, row.bottom() - 1.0, row.w, 1.0), t.splitter_dark);
    ui.c.text(fnt(10.5), row.x + 8.0, row.y + 2.0, t.text_dim, "PROJECT MODULATORS");
    if s.modulation.modulators.is_empty() {
        ui.c.text(fnt(10.0), row.x + 138.0, row.y + 2.0, t.text_faint, "click a + to add an LFO, Random or Audio Follower");
    }
    // The plots follow the snapshot; sampling once a frame at ~30 Hz is what the handoff asks for.
    s.mod_hist.borrow_mut().sample(s);
    ui.c.push_clip(row);
    for (i, slot) in mod_slots(s, r).into_iter().enumerate() {
        if slot.x + slot.w > row.right() - 2.0 {
            break;
        }
        match s.modulation.modulator_at_slot(i as u8) {
            Some(m) => mod_slot_paint(ui, s, slot, i as u8, m),
            None => mod_empty_slot(ui, slot, i as u8),
        }
    }
    ui.c.pop_clip();
}

/// An occupied slot: its name, a live scrolling plot of its actual output, its rate, and the patch-point button.
fn mod_slot_paint(ui: &mut Ui, s: &State, r: Rect, slot: u8, m: &os_model::modulation::Modulator) {
    let t = ui.t;
    let id = m.id;
    let routing = s.mod_routing == Some(id);
    let edge = if routing {
        ui.t.modulation.with_alpha(route_flash())
    } else if s.mod_editor == Some(id) {
        ui.t.modulation.with_alpha(170)
    } else {
        t.splitter_dark
    };
    ui.c.fill_rrect(r, 3.0, if m.enabled { t.s(0x202528) } else { t.s(0x1a1d1f) });
    ui.c.stroke_rrect(r, 3.0, if routing || s.mod_editor == Some(id) { 1.6 } else { 1.0 }, edge);
    // Click the tile to open its graphical editor; the buttons added below win their own clicks.
    let _ = ui.hit(r, Action::Mod(crate::modulation::ModAct::Open(id)), format!("{}: click to open its editor", crate::modulation::name_of(m)));
    // The name and the enable light.
    let ink = if m.enabled { t.text } else { t.text_faint };
    ui.c.text_clip(fnt(10.5), r.x + 5.0, r.y + 2.0, (r.w - 26.0).max(8.0), ink, &crate::modulation::name_of(m));
    let led = Rect::new(r.right() - 16.0, r.y + 2.0, 12.0, 12.0);
    let lhov = ui.hit(led, Action::Mod(crate::modulation::ModAct::Enable(id)), if m.enabled { "On: click to turn it off" } else { "Off: click to turn it on" });
    ui.c.fill_circle(led.cx(), led.cy(), 4.0, if m.enabled { ui.t.modulation.with_alpha(230) } else { t.s(0x30363a) });
    if lhov {
        ui.c.stroke_circle(led.cx(), led.cy(), 5.5, 1.0, t.glyph);
    }
    // The live plot: the last ~2 s of actual output, not a drawing of the waveform.
    let plot = Rect::new(r.x + 4.0, r.y + 16.0, (r.w - 8.0).max(6.0), (r.h - 36.0).max(8.0));
    ui.c.fill_rect(plot, t.s(0x101315));
    ui.c.line(plot.x, plot.cy(), plot.right(), plot.cy(), 1.0, t.s(0x242a2d));
    let hist = s.mod_hist.borrow();
    if let Some(series) = hist.get(id) {
        let n = series.len().max(2);
        let pts: Vec<(f32, f32)> = series
            .iter()
            .enumerate()
            .map(|(i, &v)| (plot.x + plot.w * i as f32 / (n - 1) as f32, plot.bottom() - v.clamp(0.0, 1.0) * plot.h))
            .collect();
        if pts.len() > 1 {
            ui.c.polyline(&pts, 1.4, ui.t.modulation.with_alpha(if m.enabled { 235 } else { 90 }));
        }
    }
    // The rate / band line, and the patch-point button (the handoff's assignment mechanic).
    ui.c.text_clip(fnt(9.5), r.x + 5.0, r.bottom() - 14.0, (r.w - 28.0).max(8.0), t.text_dim, &slot_text(m));
    let patch = Rect::new(r.right() - 20.0, r.bottom() - 17.0, 15.0, 15.0);
    let phov = ui.hit(
        patch,
        Action::Mod(crate::modulation::ModAct::Route(id)),
        if routing { "Routing mode: click a tinted control and drag; Esc cancels" } else { "Route this modulator: click, then drag any blue-tinted control" },
    );
    let dot = ui.t.modulation.with_alpha(if routing { route_flash() } else if phov { 255 } else { 150 });
    ui.c.fill_circle(patch.cx(), patch.cy(), 3.3, dot);
    ui.c.line(patch.cx(), patch.cy(), patch.right() - 2.0, patch.cy() - 4.0, 1.4, dot);
    ui.c.line(patch.right() - 2.0, patch.cy() - 4.0, patch.right() - 2.0, patch.cy() - 8.0, 1.2, dot);
    let _ = slot;
}

/// An empty slot's "+" (the add menu opens from it).
fn mod_empty_slot(ui: &mut Ui, r: Rect, slot: u8) {
    let t = ui.t;
    let hov = ui.hit(r, Action::Mod(crate::modulation::ModAct::AddMenu(slot)), "Add a project modulator (LFO, Random or Audio Follower)");
    ui.c.fill_rrect(r, 3.0, if hov { t.s(0x232a2e) } else { t.s(0x1b1f21) });
    ui.c.stroke_rrect(r, 3.0, 1.0, t.splitter_dark);
    ui.c.text_center(fnt(17.0), Rect::new(r.x, r.cy() - 9.0, r.w, 18.0), t.text_dim, "+");
}

/// The add menu, the slot editor, and the slot's right-click menu, over the rack.
pub(crate) fn mod_overlays_paint(ui: &mut Ui, s: &State, r: Rect) {
    let slots = mod_slots(s, r);
    if let Some(slot) = s.mod_add_slot {
        if let Some(&anchor) = slots.get(usize::from(slot)) {
            let w = 148.0;
            let m = Rect::new(anchor.x.min(r.right() - w - 2.0), anchor.bottom() + 2.0, w, 3.0 * 21.0 + 55.0);
            ui.c.fill_rrect(m, 3.0, ui.t.s(0x26292c));
            ui.c.stroke_rrect(m, 3.0, 1.0, ui.t.splitter_dark);
            ui.c.text(fnt(10.0), m.x + 8.0, m.y + 4.0, ui.t.text_dim, "NEW PROJECT MODULATOR");
            let mut y = m.y + 20.0;
            for kind in crate::modulation::Kind::PROJECT {
                let row = Rect::new(m.x + 2.0, y, m.w - 4.0, 20.0);
                let hov = ui.hit(row, Action::Mod(crate::modulation::ModAct::AddKind(slot, kind)), kind.label());
                if hov {
                    ui.c.fill_rect(row, ui.t.s(0x333a3e));
                }
                ui.c.text(fnt(12.5), row.x + 8.0, row.y + 2.0, ui.t.text, kind.label());
                if kind == crate::modulation::Kind::Lfo {
                    ui.c.text(fnt(9.5), row.x + 8.0, row.y + 13.0, ui.t.text_faint, "waves, tempo-synced or free");
                }
                if kind == crate::modulation::Kind::Random {
                    ui.c.text(fnt(9.5), row.x + 8.0, row.y + 13.0, ui.t.text_faint, "seeded, deterministic");
                }
                if kind == crate::modulation::Kind::Follower {
                    ui.c.text(fnt(9.5), row.x + 8.0, row.y + 13.0, ui.t.text_faint, "follows the master's level");
                }
                y += 30.0;
            }
        }
    }
    if let Some(id) = s.mod_editor {
        if let Some(m) = s.modulation.get(id) {
            if let Some(&anchor) = slots.get(usize::from(m.slot)) {
                mod_editor_paint(ui, s, m, anchor, r);
            }
        }
    }
    if let Some(id) = s.mod_menu {
        if let Some(m) = s.modulation.get(id) {
            if let Some(&anchor) = slots.get(usize::from(m.slot)) {
                mod_menu_paint(ui, s, m, anchor, r);
            }
        }
    }
}

/// One modulator's graphical editor: its type, its shape / rate, and the knobs its type has. Not a dropdown list —
/// the controls the handoff asks for, native and live.
fn mod_editor_paint(ui: &mut Ui, s: &State, m: &os_model::modulation::Modulator, anchor: Rect, r: Rect) {
    use os_model::modulation::ModSource;
    let t = ui.t;
    let w = 252.0;
    let h = 168.0;
    let x = anchor.x.min(r.right() - w - 2.0).max(r.x + 2.0);
    let y = (anchor.bottom() + 2.0).min(r.bottom() - h - 2.0).max(r.y + 2.0);
    let panel = Rect::new(x, y, w, h);
    ui.c.fill_rrect(panel, 4.0, t.s(0x22272a));
    ui.c.stroke_rrect(panel, 4.0, 1.0, ui.t.modulation.with_alpha(150));
    // The title is the name field: clicking it edits the name in place (the same inline box track and slot names use).
    let title = Rect::new(panel.x + 5.0, panel.y + 2.0, panel.w - 30.0, 18.0);
    let th = ui.hit(title, Action::Mod(crate::modulation::ModAct::Rename(m.id, (title.x, title.y))), "Click to rename this modulator");
    ui.c.text_clip(fnt(12.5), panel.x + 8.0, panel.y + 4.0, panel.w - 34.0, if th { t.glyph } else { t.text }, &crate::modulation::name_of(m));
    if th {
        ui.c.fill_rect(Rect::new(title.x + 3.0, title.bottom() - 3.0, title.w - 8.0, 1.0), t.splitter_dark);
    }
    let close = Rect::new(panel.right() - 20.0, panel.y + 3.0, 16.0, 16.0);
    if ui.hit(close, Action::Mod(crate::modulation::ModAct::Close), "Close the modulator editor") {
        ui.c.text(fnt(13.0), close.x + 4.0, close.y, t.glyph, "x");
    }
    // The three types the project slots offer.
    let mut bx = panel.x + 8.0;
    for kind in crate::modulation::Kind::PROJECT {
        let b = Rect::new(bx, panel.y + 22.0, 74.0, 17.0);
        let on = crate::modulation::Kind::of(m.source) == kind;
        let hov = ui.hit(b, Action::Mod(crate::modulation::ModAct::Kind(m.id, kind)), kind.label());
        ui.c.fill_rrect(b, 2.0, if on { ui.t.modulation.with_alpha(190) } else if hov { t.s(0x333a3e) } else { t.s(0x2a2f32) });
        ui.c.text_center(fnt(10.5), b, if on { t.glyph } else { t.text_dim }, kind.label());
        bx += 78.0;
    }
    let cy = panel.y + 64.0;
    match m.source {
        ModSource::Lfo(l) => {
            // Shape and rate step; both are buttons, not dropdowns.
            let b = Rect::new(panel.x + 8.0, cy - 8.0, 92.0, 18.0);
            let hov = ui.hit(b, Action::Mod(crate::modulation::ModAct::NextShape(m.id)), "Next waveform");
            ui.c.fill_rrect(b, 2.0, if hov { t.s(0x333a3e) } else { t.s(0x2a2f32) });
            ui.c.text(fnt(11.0), b.x + 6.0, b.y + 2.0, t.text, &format!("{:?}", l.shape).to_lowercase());
            rate_paint(ui, m.id, panel.x + 106.0, cy - 8.0, l.period);
            mod_knob(ui, s, m.id, crate::modulation::KnobField::Amount, "amount", panel.x + 30.0, cy + 42.0);
            mod_knob(ui, s, m.id, crate::modulation::KnobField::Phase, "phase", panel.x + 78.0, cy + 42.0);
            mod_knob(ui, s, m.id, crate::modulation::KnobField::Offset, "offset", panel.x + 126.0, cy + 42.0);
            let b = Rect::new(panel.x + 168.0, cy + 30.0, 74.0, 18.0);
            let hov = ui.hit(b, Action::Mod(crate::modulation::ModAct::ToggleBipolar(m.id)), "Bipolar / unipolar output");
            ui.c.fill_rrect(b, 2.0, if l.bipolar { ui.t.modulation.with_alpha(150) } else if hov { t.s(0x333a3e) } else { t.s(0x2a2f32) });
            ui.c.text_center(fnt(9.5), b, t.text, if l.bipolar { "bipolar" } else { "unipolar" });
        }
        ModSource::Random(rd) => {
            rate_paint(ui, m.id, panel.x + 8.0, cy - 8.0, rd.period);
            let b = Rect::new(panel.x + 8.0, cy + 22.0, 74.0, 18.0);
            let hov = ui.hit(b, Action::Mod(crate::modulation::ModAct::ToggleDistribution(m.id)), "Uniform / Gaussian");
            ui.c.fill_rrect(b, 2.0, if hov { t.s(0x333a3e) } else { t.s(0x2a2f32) });
            ui.c.text_center(fnt(9.5), b, t.text, if rd.distribution == os_model::modulation::Distribution::Uniform { "uniform" } else { "gaussian" });
            let b = Rect::new(panel.x + 86.0, cy + 22.0, 86.0, 18.0);
            let hov = ui.hit(b, Action::Mod(crate::modulation::ModAct::ToggleInterpolate(m.id)), "Hold or interpolate");
            ui.c.fill_rrect(b, 2.0, if rd.interpolate { ui.t.modulation.with_alpha(150) } else if hov { t.s(0x333a3e) } else { t.s(0x2a2f32) });
            ui.c.text_center(fnt(9.5), b, t.text, if rd.interpolate { "interpolated" } else { "hold" });
            mod_knob(ui, s, m.id, crate::modulation::KnobField::Amount, "amount", panel.x + 30.0, cy + 58.0);
            mod_knob(ui, s, m.id, crate::modulation::KnobField::Smooth, "smoothing", panel.x + 78.0, cy + 58.0);
            let b = Rect::new(panel.x + 168.0, cy + 50.0, 74.0, 18.0);
            let hov = ui.hit(b, Action::Mod(crate::modulation::ModAct::RerollSeed(m.id)), "Draw a new seed");
            ui.c.fill_rrect(b, 2.0, if hov { t.s(0x333a3e) } else { t.s(0x2a2f32) });
            ui.c.text_center(fnt(9.5), b, t.text, "reroll seed");
        }
        ModSource::AudioFollower(f) => {
            let b = Rect::new(panel.x + 8.0, cy - 8.0, 74.0, 18.0);
            let hov = ui.hit(b, Action::Mod(crate::modulation::ModAct::CycleDetection(m.id)), "RMS / peak detection");
            ui.c.fill_rrect(b, 2.0, if hov { t.s(0x333a3e) } else { t.s(0x2a2f32) });
            ui.c.text_center(fnt(9.5), b, t.text, if f.detection == os_model::modulation::Detection::Rms { "RMS" } else { "peak" });
            let b = Rect::new(panel.x + 86.0, cy - 8.0, 86.0, 18.0);
            let hov = ui.hit(b, Action::Mod(crate::modulation::ModAct::CycleBand(m.id)), "Full / low / mid / high band");
            ui.c.fill_rrect(b, 2.0, if hov { t.s(0x333a3e) } else { t.s(0x2a2f32) });
            ui.c.text_center(fnt(9.5), b, t.text, &format!("{:?} band", f.band).to_lowercase());
            mod_knob(ui, s, m.id, crate::modulation::KnobField::Amount, "amount", panel.x + 24.0, cy + 42.0);
            mod_knob(ui, s, m.id, crate::modulation::KnobField::Attack, "attack", panel.x + 66.0, cy + 42.0);
            mod_knob(ui, s, m.id, crate::modulation::KnobField::Release, "release", panel.x + 108.0, cy + 42.0);
            mod_knob(ui, s, m.id, crate::modulation::KnobField::Gain, "gain", panel.x + 150.0, cy + 42.0);
        }
        ModSource::Steps(st) => {
            rate_paint(ui, m.id, panel.x + 8.0, cy - 8.0, st.period);
            ui.c.text(fnt(10.0), panel.x + 8.0, cy + 20.0, t.text_dim, "A step pattern from an old project: 16 steps.");
        }
        ModSource::Envelope(_) => {
            ui.c.text(fnt(10.0), panel.x + 8.0, cy + 8.0, t.text_dim, "An envelope from an old project.");
        }
    }
    // The footer: the routing button and the on / off switch, so the editor can start the same gesture as the tile.
    let route = Rect::new(panel.x + 8.0, panel.bottom() - 26.0, 92.0, 19.0);
    let on = s.mod_routing == Some(m.id);
    let hov = ui.hit(route, Action::Mod(crate::modulation::ModAct::Route(m.id)), "Route this modulator: click, then drag a blue-tinted control");
    ui.c.fill_rrect(route, 2.0, if on { ui.t.modulation.with_alpha(route_flash()) } else if hov { t.s(0x333a3e) } else { t.s(0x2a2f32) });
    ui.c.text_center(fnt(10.0), route, t.text, if on { "routing..." } else { "route to..." });
    let sw = Rect::new(panel.right() - 68.0, panel.bottom() - 26.0, 60.0, 19.0);
    let hov = ui.hit(sw, Action::Mod(crate::modulation::ModAct::Enable(m.id)), "On / off");
    ui.c.fill_rrect(sw, 2.0, if m.enabled { ui.t.modulation.with_alpha(150) } else if hov { t.s(0x333a3e) } else { t.s(0x2a2f32) });
    ui.c.text_center(fnt(10.0), sw, t.text, if m.enabled { "on" } else { "off" });
}

/// The rate stepper: the current musical rate and -/+ buttons.
fn rate_paint(ui: &mut Ui, id: os_model::modulation::ModulatorId, x: f32, y: f32, period: os_model::modulation::Timebase) {
    let t = ui.t;
    let label = crate::modulation::rate_text(period);
    ui.c.text(fnt(10.5), x, y + 2.0, t.text, &label);
    let minus = Rect::new(x + 54.0, y, 16.0, 17.0);
    let plus = Rect::new(x + 72.0, y, 16.0, 17.0);
    for (r, d, s) in [(minus, -1, "-"), (plus, 1, "+")] {
        let hov = ui.hit(r, Action::Mod(crate::modulation::ModAct::RateStep(id, d)), "Slower / faster");
        ui.c.fill_rrect(r, 2.0, if hov { t.s(0x333a3e) } else { t.s(0x2a2f32) });
        ui.c.text_center(fnt(11.0), r, t.text, s);
    }
    let mode = Rect::new(x + 92.0, y + 1.0, 34.0, 15.0);
    let hov = ui.hit(mode, Action::Mod(crate::modulation::ModAct::RateStep(id, 0)), "Sync / free is picked by the rate");
    ui.c.fill_rrect(mode, 2.0, if hov { t.s(0x333a3e) } else { t.s(0x23282b) });
    ui.c.text_center(
        fnt(8.5),
        mode,
        t.text_dim,
        match period {
            os_model::modulation::Timebase::Sync(_) => "sync",
            os_model::modulation::Timebase::Free(_) => "free",
        },
    );
}

/// One editor knob: the ring, its label, and a drag hit (`ModAct::Knob`).
fn mod_knob(ui: &mut Ui, s: &State, id: os_model::modulation::ModulatorId, field: crate::modulation::KnobField, label: &str, cx: f32, cy: f32) {
    let t = ui.t;
    let v = s.modulation.get(id).map_or(0.5, |m| crate::modulation::knob_value(m.source, field));
    let hov = ui.hit(Rect::new(cx - 14.0, cy - 14.0, 28.0, 28.0), Action::Mod(crate::modulation::ModAct::Knob(id, field)), format!("{label}: drag up / down"));
    kit::knob(ui.c, t, cx, cy, 11.0, v, false, kit::KnobLook::Ring, kit::State::hover(hov));
    ui.c.text_center(fnt(9.0), Rect::new(cx - 23.0, cy + 11.0, 46.0, 12.0), t.text_dim, label);
}

/// A slot's right-click menu: on / off, clear routings, reroll the seed, remove.
fn mod_menu_paint(ui: &mut Ui, _s: &State, m: &os_model::modulation::Modulator, anchor: Rect, r: Rect) {
    let t = ui.t;
    let w = 136.0;
    let h = if matches!(m.source, os_model::modulation::ModSource::Random(_)) { 5.0 * 21.0 + 4.0 } else { 4.0 * 21.0 + 4.0 };
    let panel = Rect::new(anchor.x.min(r.right() - w - 2.0), anchor.bottom() + 2.0, w, h);
    ui.c.fill_rrect(panel, 3.0, t.s(0x26292c));
    ui.c.stroke_rrect(panel, 3.0, 1.0, t.splitter_dark);
    let item = |ui: &mut Ui, y: f32, label: &str, act: Action| {
        let row = Rect::new(panel.x + 2.0, y, panel.w - 4.0, 20.0);
        let hov = ui.hit(row, act, label.to_string());
        if hov {
            ui.c.fill_rect(row, t.s(0x333a3e));
        }
        ui.c.text(fnt(11.5), row.x + 8.0, row.y + 2.0, t.text, label);
    };
    let mut y = panel.y + 2.0;
    item(ui, y, if m.enabled { "Disable" } else { "Enable" }, Action::Mod(crate::modulation::ModAct::Enable(m.id)));
    y += 21.0;
    if matches!(m.source, os_model::modulation::ModSource::Random(_)) {
        item(ui, y, "Reroll seed", Action::Mod(crate::modulation::ModAct::RerollSeed(m.id)));
        y += 21.0;
    }
    item(ui, y, "Clear routings", Action::Mod(crate::modulation::ModAct::Clear(m.id)));
    y += 21.0;
    item(ui, y, "Remove modulator", Action::Mod(crate::modulation::ModAct::Remove(m.id)));
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
    let kind = d.kind;
    let n = 2 + usize::from(matches!(kind, DeviceKind::Instrument(_)));
    let m = Rect::new(tile.right() - 80.0, tile.y + HEADER_H, 80.0, n as f32 * 22.0 + 4.0);
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
    let mut y = m.y + 2.0;
    if popped {
        item(ui, y, "Dock", Action::DeviceRack(RackAct::Dock(key)));
    } else {
        item(ui, y, "Pop out", Action::DeviceRack(RackAct::PopOut(key)));
    }
    y += 22.0;
    if matches!(kind, DeviceKind::Instrument(_)) {
        item(ui, y, "Replace", Action::DeviceRack(RackAct::Replace));
        y += 22.0;
    }
    item(ui, y, "Remove", Action::DeviceRack(RackAct::Remove(kind)));
}

