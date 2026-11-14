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
use crate::rack_ui::{DrumMode, EDITOR_TAB, EDITOR_SUBTAB, EDITOR_BANK, EDITOR_PAD, EDITOR_OPEN, EDITOR_EQ_TOKEN, EDITOR_EQ_GRAPH};
use crate::device_rack::{self, Device, DeviceKind, RackParam, HEADER_H, TILE_HEAD, RackAct};
use crate::history::ChannelControl;
use crate::icons::{self, Icon};
use crate::views::{Ui, fnt, FrameInfo};

/// The rack's panel rect in the window: the full width, its height at the bottom edge (`device_rack::height`).
pub fn panel_rect(s: &State, w: f32, h: f32) -> Rect {
    Rect::new(0.0, h - device_rack::height(s), w, device_rack::height(s))
}

/// The splitter strip along the panel's top edge (6 px, like the browser's).
const SPLITTER: f32 = 6.0;

/// Height of the project-modulator strip above the rack panel: a label, the slot row and a hairline. It sits in the
/// workspace above the rack, not inside it, so the device tiles keep the rack's whole body.
pub(crate) const MOD_H: f32 = 92.0;

/// Height of a slot row, the gap between rows, and the trailing "＋" that asks for another row.
const MOD_SLOT_H: f32 = 72.0;
const MOD_ROW_GAP: f32 = 6.0;

/// The strip's height for the current state: one line, plus another line's worth for each one after it (none while
/// the rack is collapsed).
pub(crate) fn mod_row_h(s: &State) -> f32 {
    if s.device_rack_collapsed {
        0.0
    } else {
        MOD_H + (strip_lines(s) as f32 - 1.0) * (MOD_SLOT_H + MOD_ROW_GAP)
    }
}

/// The lines the strip takes: a tile for every modulator, then the one add tile, [`PROJECT_SLOTS`] to a line.
fn strip_lines(s: &State) -> usize {
    (s.modulation.modulators.len() + 1).div_ceil(os_model::modulation::PROJECT_SLOTS).max(1)
}

/// The strip's tiles in a strip `r` wide, in reading order (line by line): every modulator's project slot and rect,
/// then exactly one add tile in the first free slot. There is no grid of empty slots to scroll past. The tiles shrink
/// to fit a narrow window rather than overflowing the rack.
pub(crate) fn mod_slots(s: &State, r: Rect) -> Vec<(u8, Rect)> {
    if s.device_rack_collapsed {
        return Vec::new();
    }
    let row = mod_row_rect(s, r);
    let gap = 5.0;
    let n = os_model::modulation::PROJECT_SLOTS;
    let mut slots: Vec<u8> = s.modulation.modulators.iter().map(|m| m.slot).filter(|&s| s != os_model::modulation::NO_SLOT).collect();
    slots.sort_unstable();
    // Exactly one empty slot: the one the next modulator would take (NO_SLOT once the strip is full, where adding
    // grows another line).
    slots.push(s.modulation.free_slot().unwrap_or(os_model::modulation::NO_SLOT));
    let cols = slots.len().clamp(1, n);
    let w = ((row.w - 14.0 - gap * (cols as f32 - 1.0)) / cols as f32).clamp(24.0, 84.0);
    slots
        .into_iter()
        .enumerate()
        .map(|(i, slot)| {
            let (col, line) = (i % n, i / n);
            let y = row.y + 16.0 + line as f32 * (MOD_SLOT_H + MOD_ROW_GAP);
            (slot, Rect::new(row.x + 7.0 + col as f32 * (w + gap), y, w, MOD_SLOT_H))
        })
        .collect()
}

/// The modulator strip's rect, immediately above the rack panel `r` (none while the rack is collapsed).
pub(crate) fn mod_row_rect(s: &State, r: Rect) -> Rect {
    Rect::new(r.x, r.y - mod_row_h(s), r.w, mod_row_h(s))
}

/// Gap between tiles, holding the "+" insert buttons (before the first, between every
/// pair, and after the last tile).
const GAP: f32 = 22.0;

/// The device tiles over `r` (the panel rect) as (device, tile) pairs, left to right in signal-flow order; the
/// empty leftmost tile on a MIDI track is the MIDI FX placeholder (§8 is a later milestone).
pub(crate) fn layout(s: &State, r: Rect) -> Vec<(Device, Rect)> {
    let top = r.y + HEADER_H;
    if r.h <= HEADER_H {
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
    let body_h = (r.h - HEADER_H).max(0.0);
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
    (body_h - TILE_HEAD).max(1.0)
}

/// A tile's editor body: what is under its header.
fn body(tile: Rect) -> Rect {
    Rect::new(tile.x, tile.y + TILE_HEAD, tile.w, (tile.h - TILE_HEAD).max(0.0))
}

/// Paints the rack panel over `r` (its splitter, header and device tiles).
pub fn paint(ui: &mut Ui, s: &State, plugins: &Plugins, r: Rect, f: &FrameInfo) {
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
        tile_paint(ui, s, plugins, i, d.clone(), *tile, f);
        gx = tile.right();
    }
    gap_button(ui, gx, r.y + HEADER_H, None, tiles.first().map_or(r.h - HEADER_H, |(_, t)| t.h));
    drop_highlight(ui, s, &tiles);
    // The modulator row over the device headers, and its editor / menus over everything (the handoff §2).
    mod_row_paint(ui, s, r, f);
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
fn tile_paint(ui: &mut Ui, s: &State, plugins: &Plugins, i: usize, d: Device, tile: Rect, f: &FrameInfo) {
    let t = ui.t;
    let meridian = t.style.meridian();
    let selected = device_rack::tile_selected(s, i);
    // The tile: a raised surface with a hairline between header and body.
    ui.c.fill_rect(tile, if meridian { t.s(0x14181d) } else { t.s(0x222629) });
    ui.c.stroke_rrect(tile, 0.0, 1.0, t.splitter_dark);
    let head = Rect::new(tile.x, tile.y, tile.w, TILE_HEAD);
    // The kind's gradient behind the header's content, veiled so the LED and name stay readable (the pairs are
    // constants in `device_rack` so user theming can rewire them later). The selected tile's veil is lighter,
    // so its bar reads slightly brighter.
    let (top, bottom) = device_rack::header_gradient(d.kind);
    if meridian {
        // A flat instrument header: the family is a saturated spine at the left, the rest is ink. Selection is a
        // brass cap under the header, not a lighter veil.
        ui.c.fill_rect(head, t.s(0x191e24));
        ui.c.fill_rect(Rect::new(tile.x, head.y, 4.0, head.h), t.s(top));
        ui.c.fill_rect(Rect::new(tile.x + 4.0, head.y, 1.0, head.h), t.s(top).shade(-0.25));
        ui.c.fill_rect(Rect::new(tile.x, head.bottom() - 1.0, tile.w, 1.0), t.s(0x080a0d));
    } else {
        ui.c.fill_rect_gradient(head, t.s(top), t.s(bottom), 0.0);
        ui.c.fill_rect(head, os_ui::Color::rgb(0).with_alpha(if selected { 60 } else { 110 }));
        // A hard kind strip at the left edge: the device's family is readable before the name is.
        ui.c.fill_rect(Rect::new(tile.x, head.y, 3.0, head.h), t.s(top).shade(-0.12));
    }
    if selected {
        // The header lights up as a whole, not just its lower edge: the accent veil (and outline) is the
        // selected device's visual indication, the same in every skin.
        ui.c.fill_rect(head, t.accent.with_alpha(if meridian { 30 } else { 42 }));
        ui.c.stroke_rrect(head, 0.0, 1.0, t.accent.with_alpha(190));
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
        // Anywhere in the tile that is not a control selects it too (the controls' own hits are registered
        // later and win over this one).
        let body = Rect::new(tile.x, head.bottom(), tile.w, (tile.bottom() - head.bottom()).max(0.0));
        ui.hit(body, Action::DeviceRack(RackAct::FocusTile(i)), if selected { "Selected device" } else { "Click to select this device" });
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
    // The EQ's dock hides its fader bank by default; its header button reveals it in place.
    let eq = d.key.is_some_and(|_| matches!(crate::rack_ui::ui(plugin_name(s, &d)), Some(crate::rack_ui::Ui::Eq)));
    // The button cluster's left edge — the drum's flip or the pop-out alone — and the channel's knobs in front of it,
    // which the name gives room to (only an instrument has a channel behind its tile). A docked editor keeps its own
    // page strip in the body (the Trio and Sampler both do), so the tile adds no page buttons here.
    let cluster = tile.right() - if drum { 66.0 } else { 44.0 } - if eq { 56.0 } else { 0.0 };
    let knobs = matches!(d.kind, DeviceKind::Instrument(_)).then(|| cluster - 8.0 - KNOB_STEP * 3.0);
    let room = knobs.unwrap_or(cluster) - 6.0 - (tile.x + 24.0);
    if meridian {
        ui.c.text_clip(os_ui::Font::condensed(12, 4).bold(), tile.x + 28.0, cy - 8.0, room.max(12.0), t.text, &d.name.to_uppercase());
    } else {
        ui.c.text_clip(fnt(14.0), tile.x + 24.0, cy - 8.0, room.max(12.0), t.text, &d.name);
    }
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
    // The EQ's fader-bank drawer: hidden in the dock unless this is on. The pop-out always shows the full editor.
    if eq {
        let key = d.key.unwrap();
        let on = matches!(s.rack_ui_cache.borrow().view(&key), Some(crate::rack_ui::TileView::Eq { faders: true }));
        let b = Rect::new(tile.right() - 100.0, cy - 8.0, 52.0, 16.0);
        let hov = ui.hit(
            b,
            Action::DeviceRack(RackAct::EditorFaders { key }),
            if on { "Hide the EQ's fader bank" } else { "Show the EQ's fader bank" },
        );
        ui.c.fill_rrect(b, 3.0, if on { t.accent.with_alpha(150) } else if hov { t.s(0x333a3e) } else { t.s(0x252a2d) });
        ui.c.text_center(fnt(9.0), b, if on { t.glyph } else { t.text_dim }, "FADERS");
    }
    // The channel's PAN / VOL / PITCH, in the header beside the name.
    if let (DeviceKind::Instrument(row), Some(left)) = (d.kind, knobs) {
        channel_knobs(ui, s, row, left, cy);
    }
    let body = body(tile);
    if meridian {
        ui.c.fill_rect(body, t.s(0x0d1013));
        ui.c.stroke_rrect(body.inset(1.0), 0.0, 1.0, t.toolbar_line);
        ui.c.fill_rect(Rect::new(body.x + 1.0, body.y + 1.0, body.w - 2.0, 1.0), t.toolbar_edge.with_alpha(90));
    }
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
    controls(ui, s, plugins, &d, body, f);
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
pub(crate) const KNOB_STEP: f32 = 28.0;

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
        header_knob(ui, cell.cx(), cy, 12.0, v.clamp(0.0, 1.0) as f32, bipolar, Color::rgb(accent), &text, font, hov);
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
fn controls(ui: &mut Ui, s: &State, plugins: &Plugins, d: &Device, body: Rect, f: &FrameInfo) {
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
        editor_body(ui, s, plugins, key, editor, body, f);
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
        cell_paint(ui, s, key, &params[*i], *cell, body);
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
fn editor_body(ui: &mut Ui, s: &State, plugins: &Plugins, key: usize, editor: crate::rack_ui::Ui, body: Rect, f: &FrameInfo) {
    if matches!(editor, crate::rack_ui::Ui::Drum) {
        drum_editor_body(ui, s, plugins, key, body, f);
        return;
    }
    editor_image_body(ui, s, plugins, key, editor, body);
}

/// The drum instrument's interface: a STEP / PADS switch over either the real pattern grid or the hosted pad grid.
/// Both edit and play the same hosted sampler and the same project patterns, so switching never touches playback.
fn drum_editor_body(ui: &mut Ui, s: &State, plugins: &Plugins, key: usize, body: Rect, f: &FrameInfo) {
    s.rack_ui_cache.borrow_mut().ensure(&key, crate::rack_ui::Ui::Drum);
    let t = ui.t;
    let (mode, bank, lane, page) = match s.rack_ui_cache.borrow().view(&key) {
        Some(crate::rack_ui::TileView::Drum { mode, view, lane, page, .. }) => (*mode, view.bank, *lane, *page),
        _ => (DrumMode::Step, 0, 0, 0),
    };
    // The interface switch is always the first row: the two views are one instrument, never two devices.
    let bar = Rect::new(body.x + 4.0, body.y + 3.0, (body.w - 8.0).max(1.0), 18.0);
    let mut x = bar.x;
    for (label, m) in [("STEP", DrumMode::Step), ("PADS", DrumMode::Pads)] {
        let b = Rect::new(x, bar.y, 52.0, 18.0);
        let on = mode == m;
        let hov = ui.hit(
            b,
            Action::DeviceRack(RackAct::EditorMode { key, mode: m }),
            if m == DrumMode::Step { "Program the pattern on the step grid" } else { "Play the same lanes on performance pads" },
        );
        ui.c.fill_rrect(b, 2.0, if on { t.accent } else if hov { t.s(0x2b3134) } else { t.s(0x202628) });
        ui.c.text_center(fnt(10.0), b, if on { t.s(0x0b0f10) } else { t.text_dim }, label);
        x += 56.0;
    }
    // The bank is the same in both interfaces: sixteen lanes at a time of the drum instrument's 32.
    let bk = Rect::new(x + 4.0, bar.y, 22.0, 18.0);
    let bh = ui.hit(bk, Action::DeviceRack(RackAct::EditorBank { key, n: (1 - bank.min(1)) as u8 }), "Switch pad bank A / B");
    ui.c.fill_rrect(bk, 2.0, if bh { t.s(0x2b3134) } else { t.s(0x202628) });
    ui.c.text_center(fnt(10.0), bk, t.text, if bank == 0 { "A" } else { "B" });
    let rest = Rect::new(body.x, bar.bottom() + 2.0, body.w, (body.bottom() - bar.bottom() - 2.0).max(1.0));
    if mode == DrumMode::Step {
        drum_step_body(ui, s, plugins, key, rest, f, bank, lane, page);
    } else {
        editor_image_body(ui, s, plugins, key, crate::rack_ui::Ui::Drum, rest);
    }
}

/// Mode A: the lane panel and the pattern grid. Every cell is a real note of the current project pattern, keyed to the
/// pad's MIDI key, so one clip launches the whole pattern and no lane becomes a track or mixer channel.
fn drum_step_body(ui: &mut Ui, s: &State, plugins: &Plugins, key: usize, body: Rect, f: &FrameInfo, bank: usize, lane: usize, page: u32) {
    let t = ui.t;
    ui.c.fill_rect(body, t.s(0x101416));
    let lane_w = (body.w * 0.31).clamp(84.0, 148.0);
    let step_w = ((body.w - lane_w - 8.0) / 16.0).max(5.0);
    // Pattern row: navigation, identity, length in bars, page and clear. The pattern is the project's, as every clip
    // references it; the instrument's rows are the notes on its own channel.
    let row = Rect::new(body.x + 4.0, body.y + 2.0, (body.w - 8.0).max(1.0), 20.0);
    let mut x = row.x;
    for (label, d) in [("‹", -1), ("›", 1)] {
        let b = Rect::new(x, row.y, 18.0, 18.0);
        let hov = ui.hit(b, Action::DeviceRack(RackAct::EditorPattern { d }), "Previous / next pattern");
        ui.c.fill_rrect(b, 2.0, if hov { t.s(0x343b3f) } else { t.s(0x202628) });
        ui.c.text_center(fnt(12.0), b, t.text, label);
        x += 20.0;
    }
    let name = format!("P{}  {}", s.pattern, crate::patterns::name_of(s, s.pattern));
    let name_w = (row.right() - x - 216.0).max(20.0);
    ui.c.text_clip(fnt(11.0), x + 4.0, row.y + 3.0, name_w, t.text, &name);
    // Length in bars (`Auto` when no explicit length); the step grid itself supports up to 32 bars.
    let bars = crate::rack::explicit_len(s).map_or(0, |n| n.div_ceil(crate::rack::bar_steps(s).max(1)));
    let len_txt = if bars == 0 { "Auto".to_string() } else { format!("{bars} bar{}", if bars == 1 { "" } else { "s" }) };
    let mut rx = row.right() - 212.0;
    for (label, d) in [("−", -1), ("+", 1)] {
        let b = Rect::new(rx, row.y, 18.0, 18.0);
        let hv = ui.hit(b, Action::DeviceRack(RackAct::EditorLength { bars: bars as i32 + d }), "Pattern length in bars (0 = Auto)");
        ui.c.fill_rrect(b, 2.0, if hv { t.s(0x343b3f) } else { t.s(0x202628) });
        ui.c.text_center(fnt(12.0), b, t.text, label);
        rx += 20.0;
    }
    ui.c.text_clip(fnt(10.0), rx, row.y + 4.0, 48.0, t.text_dim, &len_txt);
    let pages = crate::rack::pattern_steps(s).div_ceil(16).max(1);
    let p = page.min(pages.saturating_sub(1));
    rx = row.right() - 126.0;
    ui.c.text(fnt(9.0), rx, row.y + 4.0, t.text_dim, &format!("{}/{}", p + 1, pages));
    rx = row.right() - 102.0;
    for (label, np, ok) in [("‹", p.saturating_sub(1), p > 0), ("›", (p + 1).min(pages - 1), p + 1 < pages)] {
        let b = Rect::new(rx, row.y, 15.0, 18.0);
        let hv = ok && ui.hit(b, Action::DeviceRack(RackAct::EditorPage { key, page: np }), "Previous / next 16 steps");
        ui.c.fill_rrect(b, 2.0, if hv { t.s(0x343b3f) } else { t.s(0x202628) });
        ui.c.text_center(fnt(10.0), b, if ok { t.text } else { t.text_faint }, label);
        rx += 16.0;
    }
    // Duplicate the project pattern, so a beat and its fill stay independent; Clear touches this instrument's channel
    // only, leaving other channels' notes in the pattern.
    let dup = Rect::new(row.right() - 70.0, row.y, 30.0, 18.0);
    let dh = ui.hit(dup, Action::DeviceRack(RackAct::EditorDuplicate), "Duplicate the current pattern");
    ui.c.fill_rrect(dup, 2.0, if dh { t.s(0x343b3f) } else { t.s(0x202628) });
    ui.c.text_center(fnt(8.5), dup, t.text_dim, "DUP");
    let clear = Rect::new(row.right() - 38.0, row.y, 38.0, 18.0);
    let ch = ui.hit(clear, Action::DeviceRack(RackAct::EditorStepClear { key }), "Clear this instrument's steps in the current pattern");
    ui.c.fill_rrect(clear, 2.0, if ch { t.s(0x4a3030) } else { t.s(0x2a2020) });
    ui.c.text_center(fnt(9.0), clear, if ch { t.text } else { t.text_dim }, "CLEAR");
    // Step numbers and beat / bar divisions.
    let grid = Rect::new(body.x + 4.0, row.bottom() + 3.0, (body.w - 8.0).max(1.0), (body.bottom() - row.bottom() - 5.0).max(1.0));
    let head_h = 12.0;
    let cols_x = grid.x + lane_w;
    let first = p * 16;
    let total = crate::rack::pattern_steps(s).max(1);
    let per = crate::rack::per_step(s);
    let beat = crate::rack::beat_steps(s).max(1);
    let cur = if f.playing {
        let tick = if f.song { crate::piano::pattern_tick_in_song(s, f.tick) } else { Some(f.tick) };
        tick.map(|t| ((t.max(0.0) as u32) / per) % total)
    } else {
        None
    };
    for i in 0..16u32 {
        let step = first + i;
        let x0 = cols_x + i as f32 * step_w;
        let bg = if step >= total { t.s(0x171b1d) } else if (step / beat) % 2 == 0 { t.s(0x1b2123) } else { t.s(0x161b1d) };
        ui.c.fill_rect(Rect::new(x0, grid.y, step_w, grid.h), bg);
        if step < total {
            ui.c.text_center(fnt(8.0), Rect::new(x0, grid.y, step_w, head_h), t.text_faint, &(step + 1).to_string());
        }
        if i % beat == 0 {
            ui.c.fill_rect(Rect::new(x0, grid.y, 1.0, grid.h), t.s(0x3a4448));
        }
    }
    // The hosted sampler's pads are the lane definitions. They are read from its live saved state, the same one the
    // pads interface draws, so a reassigned key is the same row in both views.
    let lanes = crate::rack_ui::drum_state(plugins, key).pads();
    let rows = 16usize;
    let row_h = ((grid.h - head_h) / rows as f32).max(6.0);
    let channel = crate::device_rack::instrument_row(s, key).and_then(|r| s.project.channels.get(r)).map(|c| c.index);
    for row_i in 0..rows {
        let pad_i = bank * 16 + row_i;
        let pad = lanes.get(pad_i);
        let y = grid.y + head_h + row_i as f32 * row_h;
        let selected = pad_i == lane;
        if selected {
            ui.c.fill_rect(Rect::new(grid.x, y, grid.w, row_h), t.accent.with_alpha(18));
        }
        let label = Rect::new(grid.x, y, lane_w - 2.0, row_h);
        let lh = ui.hit(label, Action::DeviceRack(RackAct::EditorLane { key, lane: pad_i }), "Select this lane; its steps are the same in both interfaces");
        if lh || selected {
            ui.c.fill_rect(label, t.s(0x25302c));
        }
        let (nm, ky) = pad.map_or((format!("Pad {}", pad_i + 1), 60), |p| {
            (if p.name.is_empty() { format!("Pad {}", pad_i + 1) } else { p.name.clone() }, p.key)
        });
        ui.c.text_clip(fnt(9.5), label.x + 4.0, y + (row_h - 11.0) * 0.5, label.w - 30.0, if selected { t.text } else { t.text_dim }, &nm);
        ui.c.text(fnt(8.0), label.right() - 24.0, y + (row_h - 9.0) * 0.5, t.text_faint, &ky.to_string());
        if row_i + 1 < rows {
            ui.c.fill_rect(Rect::new(grid.x, y + row_h, grid.w, 1.0), t.s(0x101416));
        }
        let Some(ch) = channel else { continue };
        let pad_key = ky.clamp(0, 127) as u16;
        for i in 0..16u32 {
            let step = first + i;
            if step >= total {
                continue;
            }
            let x0 = cols_x + i as f32 * step_w;
            let on = crate::rack::drum_step_on(s, ch, pad_key, step);
            let cell = Rect::new(x0 + 1.0, y + 1.0, (step_w - 2.0).max(1.0), (row_h - 2.0).max(1.0));
            let hv = ui.hit(
                cell,
                Action::DeviceRack(RackAct::EditorStep { key, lane: pad_i, step, on: !on }),
                if on { "Turn this step off" } else { "Turn this step on" },
            );
            let fill = if on {
                if selected { t.accent } else { t.accent.with_alpha(205) }
            } else if hv {
                t.s(0x394347)
            } else {
                t.s(0x252c2f)
            };
            ui.c.fill_rrect(cell, 1.5, fill);
            if on {
                ui.c.fill_rect(Rect::new(cell.x + 1.0, cell.y + 1.0, (cell.w - 2.0).max(1.0), 1.0), t.s(0xd8ff9a));
            }
        }
    }
    // The playhead: one clear column, drawn over the cells, not a second clock.
    if let Some(k) = cur.filter(|k| (*k >= first) && (*k < first + 16)) {
        let x0 = cols_x + (k - first) as f32 * step_w;
        ui.c.fill_rect(Rect::new(x0, grid.y + head_h, step_w, grid.h - head_h), t.s(0x00d5c0).with_alpha(34));
        ui.c.fill_rect(Rect::new(x0, grid.y + head_h, step_w, 2.0), t.s(0x00d5c0));
    }
}

fn editor_image_body(ui: &mut Ui, s: &State, plugins: &Plugins, key: usize, editor: crate::rack_ui::Ui, body: Rect) {
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
            if let Some(target) = crate::params::target_of_id(s, key, *id) {
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
        } else if *id == EDITOR_EQ_TOKEN {
            RackAct::EditorEqToken { key, band: n as usize }
        } else if *id == EDITOR_EQ_GRAPH {
            RackAct::EditorEqGraph { key }
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
        let hint = if matches!(*id, EDITOR_TAB | EDITOR_SUBTAB | EDITOR_BANK | EDITOR_PAD | EDITOR_OPEN | EDITOR_EQ_GRAPH) {
            String::new()
        } else if *id == EDITOR_EQ_TOKEN {
            format!("Drag band {} to set its frequency and gain", n + 1)
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
fn cell_paint(ui: &mut Ui, s: &State, key: usize, p: &RackParam, cell: Rect, body: Rect) {
    let t = ui.t;
    let cx = cell.cx();
    // Routing mode tints every control project modulation can reach (the handoff §4.1). This is the same predicate
    // that decides whether the drag assigns, so a tint never lies.
    let target = crate::params::target_of_id(s, key, p.id);
    let eligible = s.mod_routing.is_some() && target.is_some_and(|t| crate::modulation::is_modulatable(s, t));
    // The tint is the colour of the modulator being routed (its tile's own colour), not the theme's, so the control
    // you are about to drive matches the slot you picked: an LFO, a random or a follower each read as themselves.
    let ink = s.mod_routing.and_then(|id| s.modulation.get(id)).map_or(t.modulation, |m| mod_accent(t.modulation, &m.source));
    if eligible {
        ui.c.fill_rrect(cell, 2.0, ink.with_alpha(30));
        ui.c.stroke_rrect(cell, 2.0, 1.0, ink.with_alpha(110));
    }
    ui.c.text_clip(fnt(11.0), cell.x + 2.0, cell.y + 1.0, CELL_W - 4.0, if eligible { ink } else { t.text_dim }, &p.name);
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

/// One assignment's ink: the modulator's own colour (`mod_accent`, the one its tile, its plot and its patch point
/// carry), with the second and third sources on one control a little paler so two arcs stay apart.
fn mod_ink(colour: Color, n: usize, enabled: bool) -> Color {
    const ALPHA: [u8; 3] = [255, 220, 200];
    colour.with_alpha(if enabled { ALPHA[n % ALPHA.len()] } else { 70 })
}

/// A modulator's ink by id, from its source: what routing mode's tint and every arc is drawn in.
fn mod_colour(s: &State, base: Color, id: os_model::modulation::ModulatorId) -> Color {
    s.modulation.get(id).map_or(base, |m| mod_accent(base, &m.source))
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
        let c = mod_ink(mod_colour(s, ui.t.modulation, *id), n, *enabled);
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
        let c = mod_ink(mod_colour(s, ui.t.modulation, *id), n, *enabled);
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

/// The project-modulator slots in the strip above the rack panel. They are project state, so every track shows the
/// same slots; only the devices below them change with the selection.
pub(crate) fn mod_row_paint(ui: &mut Ui, s: &State, r: Rect, f: &FrameInfo) {
    if s.device_rack_collapsed {
        return;
    }
    let t = ui.t;
    let row = mod_row_rect(s, r);
    if row.h <= 0.0 {
        return;
    }
    ui.c.fill_rect(row, t.s(0x15181a));
    ui.c.fill_rect(Rect::new(row.x, row.bottom() - 1.0, row.w, 1.0), t.splitter_dark);
    // Anywhere on the strip takes the editing keys for it: Ctrl+A selects its modulators, not notes elsewhere.
    ui.hit(row, Action::Mod(crate::modulation::ModAct::FocusStrip), "Modulator strip: its keys act on its slots");
    ui.c.text(fnt(10.5), row.x + 8.0, row.y + 2.0, t.text_dim, "PROJECT MODULATORS");
    // Toggle modulator rack visibility button (click to collapse/expand the modulator row).
    let vis_rect = Rect::new(row.right() - 28.0, row.y + 4.0, 20.0, 16.0);
    let vis_hov = ui.hit(vis_rect, Action::ModVisibility, "Toggle modulator rack visibility");
    let vis_face = if s.device_rack_collapsed { t.s(0x3a4046) } else { t.s(0x2a3137) };
    ui.c.fill_rrect(vis_rect, 2.0, vis_face);
    ui.c.stroke_rrect(vis_rect, 2.0, 1.0, if vis_hov { t.glyph } else { t.splitter_dark });
    ui.c.text_center(fnt(10.0), vis_rect, if s.device_rack_collapsed { t.glyph } else { t.s(0x1b1f21) }, "◉");
    if s.device_rack_collapsed {
        ui.c.text(fnt(10.0), row.x + 138.0, row.y + 2.0, t.text_faint, "click a + to add an LFO, Random or Audio Follower");
    }
    // The plots follow the snapshot; sampling once a frame at ~30 Hz is what the handoff asks for.
    s.mod_hist.borrow_mut().sample(s, f.playing, f.tempo);
    ui.c.push_clip(row);
    for (slot, at) in mod_slots(s, r) {
        if at.x + at.w > row.right() - 2.0 {
            break;
        }
        match s.modulation.modulator_at_slot(slot) {
            Some(m) => mod_slot_paint(ui, s, at, slot, m),
            None => mod_empty_slot(ui, s, at, slot),
        }
    }
    ui.c.pop_clip();
}

/// The colour a modulator's kind uses: the themed modulation accent for an LFO, a green-yellow random and a violet
/// follower so the three read apart at a glance.
pub(crate) fn mod_accent(base: Color, source: &os_model::modulation::ModSource) -> Color {
    match source {
        os_model::modulation::ModSource::Random(_) => Color::rgb(0xc6ff53),
        os_model::modulation::ModSource::AudioFollower(_) => Color::rgb(0xae65ff),
        _ => base,
    }
}

/// Whether a slot's plot draws anything: a source whose amount is zero is switched off, so where the graph used to
/// show a flat line at the middle there is nothing at all.
pub(crate) fn mod_plot_visible(m: &os_model::modulation::Modulator) -> bool {
    m.source.is_on()
}

/// An occupied slot: a live scrolling plot of its output, its rate, and the patch-point button, in its kind's colour.
fn mod_slot_paint(ui: &mut Ui, s: &State, r: Rect, slot: u8, m: &os_model::modulation::Modulator) {
    let t = ui.t;
    let id = m.id;
    let accent = mod_accent(t.modulation, &m.source);
    let routing = s.mod_routing == Some(id);
    let selected = s.mod_sel.contains(&slot) || s.mod_editor == Some(id);
    let edge = if routing {
        accent.with_alpha(route_flash())
    } else if selected {
        accent.with_alpha(170)
    } else {
        t.splitter_dark
    };
    ui.c.fill_rrect(r, 3.0, if m.enabled { t.s(0x202528) } else { t.s(0x1a1d1f) });
    ui.c.stroke_rrect(r, 3.0, if routing || selected { 1.6 } else { 1.0 }, edge);
    // Click the tile to open its graphical editor; the buttons added below win their own clicks.
    let _ = ui.hit(r, Action::Mod(crate::modulation::ModAct::Open(id)), format!("{}: click to open its editor", crate::modulation::name_of(m)));
    // The live plot: the last ~2 s of actual output (a source preview while stopped), not a drawing of the
    // waveform. The tile carries no name, so the plot gets the top of the slot.
    let plot = Rect::new(r.x + 4.0, r.y + 4.0, (r.w - 8.0).max(6.0), (r.h - 24.0).max(8.0));
    ui.c.fill_rect(plot, t.s(0x101315));
    ui.c.line(plot.x, plot.cy(), plot.right(), plot.cy(), 1.0, t.s(0x242a2d));
    let hist = s.mod_hist.borrow();
    if let Some(series) = hist.get(id).filter(|_| mod_plot_visible(m)) {
        let n = series.len().max(2);
        let pts: Vec<(f32, f32)> = series
            .iter()
            .enumerate()
            .map(|(i, &v)| (plot.x + plot.w * i as f32 / (n - 1) as f32, plot.bottom() - v.clamp(0.0, 1.0) * plot.h))
            .collect();
        if pts.len() > 1 {
            ui.c.polyline(&pts, 1.4, accent.with_alpha(if m.enabled { 235 } else { 90 }));
        }
    }
    // The enable light, over the plot's top-right corner; the remove button sits just left of it, so a padded
    // modulator is removable without opening a menu (Delete and the tile menu also remove it).
    let close = Rect::new(r.right() - 31.0, r.y + 2.0, 13.0, 12.0);
    let xhov = ui.hit(close, Action::Mod(crate::modulation::ModAct::Remove(id)), format!("Remove {}", crate::modulation::name_of(m)));
    ui.c.text_center(fnt(11.0), close, if xhov { t.record } else { t.text_faint }, "×");
    let led = Rect::new(r.right() - 16.0, r.y + 2.0, 12.0, 12.0);
    let lhov = ui.hit(led, Action::Mod(crate::modulation::ModAct::Enable(id)), if m.enabled { "On: click to turn it off" } else { "Off: click to turn it on" });
    ui.c.fill_circle(led.cx(), led.cy(), 4.0, if m.enabled { accent.with_alpha(230) } else { t.s(0x30363a) });
    if lhov {
        ui.c.stroke_circle(led.cx(), led.cy(), 5.5, 1.0, t.glyph);
    }
    // The rate / band line, and the patch-point button (the handoff's assignment mechanic).
    ui.c.text_clip(fnt(9.5), r.x + 5.0, r.bottom() - 14.0, (r.w - 28.0).max(8.0), t.text_dim, &slot_text(m));
    let patch = Rect::new(r.right() - 20.0, r.bottom() - 17.0, 15.0, 15.0);
    let phov = ui.hit(
        patch,
        Action::Mod(crate::modulation::ModAct::Route(id)),
        if routing { "Routing mode: click a tinted control and drag; Esc cancels" } else { "Route this modulator: click, then drag a control tinted in its colour" },
    );
    let dot = accent.with_alpha(if routing { route_flash() } else if phov { 255 } else { 150 });
    ui.c.fill_circle(patch.cx(), patch.cy(), 3.3, dot);
    ui.c.line(patch.cx(), patch.cy(), patch.right() - 2.0, patch.cy() - 4.0, 1.4, dot);
    ui.c.line(patch.right() - 2.0, patch.cy() - 4.0, patch.right() - 2.0, patch.cy() - 8.0, 1.2, dot);
    let _ = slot;
}

/// An empty slot's "+" (the add menu opens from it).
fn mod_empty_slot(ui: &mut Ui, s: &State, r: Rect, slot: u8) {
    let t = ui.t;
    let hov = ui.hit(r, Action::Mod(crate::modulation::ModAct::AddMenu(slot)), "Add a project modulator (LFO, Random or Audio Follower)");
    let cursor = s.mod_cursor == Some(slot);
    ui.c.fill_rrect(r, 3.0, if hov { t.s(0x232a2e) } else { t.s(0x1b1f21) });
    ui.c.stroke_rrect(r, 3.0, if cursor { 1.6 } else { 1.0 }, if cursor { ui.t.modulation.with_alpha(170) } else { t.splitter_dark });
    ui.c.text_center(fnt(17.0), Rect::new(r.x, r.cy() - 9.0, r.w, 18.0), t.text_dim, "+");
}

/// The empty slot's add menu. It is painted by the main window after its windows, not inside the rack's clipped
/// panel, so an open picker can stand above the playlist and mixer instead of being cut off by the rack edge.
pub(crate) fn mod_add_menu_paint(ui: &mut Ui, s: &State, r: Rect) {
    let slots = mod_slots(s, r);
    if let Some(slot) = s.mod_add_slot {
        if let Some((_, anchor)) = slots.iter().find(|(at, _)| *at == slot).copied() {
            let w = 148.0;
            let h = 3.0 * 21.0 + 55.0;
            let m = Rect::new(anchor.x.min(r.right() - w - 2.0), (anchor.y - h - 2.0).max(2.0), w, h);
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
}

/// Size of the modulator editor's own native window (always on top of the app's windows).
pub(crate) const MOD_EDITOR_W: f32 = 252.0;
pub(crate) const MOD_EDITOR_H: f32 = 168.0;

/// The editor as the whole of its always-on-top native window.
pub(crate) fn mod_editor_window(ui: &mut Ui, s: &State, m: &os_model::modulation::Modulator) {
    // The anchoring/clamping rect is two px wider on each side, so the usual 2 px edge clamp lands the panel at
    // (0, 0) and it fills the native window without a clipped right edge.
    let r = Rect::new(-2.0, 0.0, MOD_EDITOR_W + 4.0, MOD_EDITOR_H);
    // Anchored just under the editor's own bottom: the panel lands at (0, 0) inside the window.
    let anchor = Rect::new(0.0, MOD_EDITOR_H + 4.0, 0.0, 0.0);
    mod_editor_paint(ui, s, m, anchor, r);
    // The slot's options menu lives in this same always-on-top window, so the piano roll, playlist and detached
    // windows cannot cover its buttons.
    if let Some(menu) = s.mod_menu.and_then(|id| s.modulation.get(id)) {
        mod_menu_window(ui, s, menu);
    }
}

/// The slot's options menu inside the always-on-top editor window, at its top-left.
fn mod_menu_window(ui: &mut Ui, s: &State, m: &os_model::modulation::Modulator) {
    let r = Rect::new(0.0, 0.0, MOD_EDITOR_W, MOD_EDITOR_H);
    let h = mod_menu_h(s, m);
    // `mod_menu_paint` places the panel just above its anchor: this one puts it at (4, 4).
    let anchor = Rect::new(4.0, 4.0 + h + 2.0, 0.0, 0.0);
    mod_menu_paint(ui, s, m, anchor, r);
}

/// One modulator's graphical editor: its type, its shape / rate, and the knobs its type has. Not a dropdown list —
/// the controls the handoff asks for, native and live.
fn mod_editor_paint(ui: &mut Ui, s: &State, m: &os_model::modulation::Modulator, anchor: Rect, r: Rect) {
    use os_model::modulation::ModSource;
    let t = ui.t;
    let accent = mod_accent(t.modulation, &m.source);
    let w = MOD_EDITOR_W;
    let h = MOD_EDITOR_H;
    let x = anchor.x.min(r.right() - w - 2.0).max(r.x + 2.0);
    // Above the slot strip, over the workspace: the editor never covers the device tiles below the rack header.
    let y = (anchor.y - h - 4.0).max(2.0);
    let panel = Rect::new(x, y, w, h);
    ui.c.fill_rrect(panel, 4.0, t.s(0x22272a));
    ui.c.stroke_rrect(panel, 4.0, 1.0, accent.with_alpha(150));
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
        ui.c.fill_rrect(b, 2.0, if on { accent.with_alpha(190) } else if hov { t.s(0x333a3e) } else { t.s(0x2a2f32) });
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
            rate_paint(ui, s, m.id, panel.x + 106.0, cy - 8.0, l.period);
            mod_knob(ui, s, m.id, crate::modulation::KnobField::Amount, "amount", panel.x + 30.0, cy + 42.0);
            mod_knob(ui, s, m.id, crate::modulation::KnobField::Phase, "phase", panel.x + 78.0, cy + 42.0);
            mod_knob(ui, s, m.id, crate::modulation::KnobField::Offset, "offset", panel.x + 126.0, cy + 42.0);
            let b = Rect::new(panel.x + 168.0, cy + 30.0, 74.0, 18.0);
            let hov = ui.hit(b, Action::Mod(crate::modulation::ModAct::ToggleBipolar(m.id)), "Bipolar / unipolar output");
            ui.c.fill_rrect(b, 2.0, if l.bipolar { accent.with_alpha(150) } else if hov { t.s(0x333a3e) } else { t.s(0x2a2f32) });
            ui.c.text_center(fnt(9.5), b, t.text, if l.bipolar { "bipolar" } else { "unipolar" });
        }
        ModSource::Random(rd) => {
            rate_paint(ui, s, m.id, panel.x + 8.0, cy - 8.0, rd.period);
            let b = Rect::new(panel.x + 8.0, cy + 22.0, 74.0, 18.0);
            let hov = ui.hit(b, Action::Mod(crate::modulation::ModAct::ToggleDistribution(m.id)), "Uniform / Gaussian");
            ui.c.fill_rrect(b, 2.0, if hov { t.s(0x333a3e) } else { t.s(0x2a2f32) });
            ui.c.text_center(fnt(9.5), b, t.text, if rd.distribution == os_model::modulation::Distribution::Uniform { "uniform" } else { "gaussian" });
            let b = Rect::new(panel.x + 86.0, cy + 22.0, 86.0, 18.0);
            let hov = ui.hit(b, Action::Mod(crate::modulation::ModAct::ToggleInterpolate(m.id)), "Hold or interpolate");
            ui.c.fill_rrect(b, 2.0, if rd.interpolate { accent.with_alpha(150) } else if hov { t.s(0x333a3e) } else { t.s(0x2a2f32) });
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
            rate_paint(ui, s, m.id, panel.x + 8.0, cy - 8.0, st.period);
            ui.c.text(fnt(10.0), panel.x + 8.0, cy + 20.0, t.text_dim, "A step pattern from an old project: 16 steps.");
        }
        ModSource::Envelope(_) => {
            ui.c.text(fnt(10.0), panel.x + 8.0, cy + 8.0, t.text_dim, "An envelope from an old project.");
        }
    }
    // The footer: the routing button and the on / off switch, so the editor can start the same gesture as the tile.
    let route = Rect::new(panel.x + 8.0, panel.bottom() - 26.0, 92.0, 19.0);
    let on = s.mod_routing == Some(m.id);
    let hov = ui.hit(route, Action::Mod(crate::modulation::ModAct::Route(m.id)), "Route this modulator: click, then drag a control tinted in its colour");
    ui.c.fill_rrect(route, 2.0, if on { accent.with_alpha(route_flash()) } else if hov { t.s(0x333a3e) } else { t.s(0x2a2f32) });
    ui.c.text_center(fnt(10.0), route, t.text, if on { "routing..." } else { "route to..." });
    let sw = Rect::new(panel.right() - 68.0, panel.bottom() - 26.0, 60.0, 19.0);
    let hov = ui.hit(sw, Action::Mod(crate::modulation::ModAct::Enable(m.id)), "On / off");
    ui.c.fill_rrect(sw, 2.0, if m.enabled { accent.with_alpha(150) } else if hov { t.s(0x333a3e) } else { t.s(0x2a2f32) });
    ui.c.text_center(fnt(10.0), sw, t.text, if m.enabled { "on" } else { "off" });
}

/// The rate knob: the current musical rate, a drag knob and its sync / free mode button. The rate is a routing
/// destination like any other knob ([`os_model::modulation::ModParam::Rate`]), so it is ringed while routing mode is
/// armed and its routes' arcs are drawn round its smaller radius.
fn rate_paint(ui: &mut Ui, s: &State, id: os_model::modulation::ModulatorId, x: f32, y: f32, period: os_model::modulation::Timebase) {
    let t = ui.t;
    let label = crate::modulation::rate_text(period);
    let knob = Rect::new(x, y - 2.0, 24.0, 21.0);
    let route = route_destination(s, id, crate::modulation::KnobField::Rate);
    let hov = ui.hit(
        knob,
        Action::Mod(crate::modulation::ModAct::Knob(id, crate::modulation::KnobField::Rate)),
        mod_hint(s, route, &format!("Rate: {label}")),
    );
    let v = crate::modulation::rate_pos(period);
    kit::knob(
        ui.c,
        t,
        knob.cx(),
        knob.cy(),
        8.0,
        v,
        false,
        kit::KnobLook::Ring,
        kit::State::hover(hov),
    );
    if let Some(from) = route {
        route_ring(ui, s, from, knob.cx(), knob.cy(), 10.2);
    }
    mod_arcs(ui, s, os_model::target::modulator_param(id.0, os_model::modulation::ModParam::Rate.code()), knob.cx(), knob.cy(), 12.5, v);
    ui.c.text(fnt(10.5), x + 28.0, y + 2.0, t.text, &label);
    let mode = Rect::new(x + 88.0, y + 1.0, 38.0, 15.0);
    let hov = ui.hit(
        mode,
        Action::Mod(crate::modulation::ModAct::RateMode(id)),
        "Tempo sync / free Hz",
    );
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

/// With routing mode armed on another modulator, the modulator whose route a press on this knob would start: the
/// knob is the armed modulator's destination. `modulation::act` then draws the route's depth on the press instead of
/// moving the knob, and the knob is ringed and carries the route's arc.
fn route_destination(s: &State, id: os_model::modulation::ModulatorId, field: crate::modulation::KnobField) -> Option<os_model::modulation::ModulatorId> {
    let from = s.mod_routing.filter(|from| *from != id)?;
    let param = crate::modulation::knob_param(field)?;
    crate::modulation::can_route(s, from, id, param).then_some(from)
}

/// A knob's hint: what dragging it does, or which modulator's route a press on it would start.
fn mod_hint(s: &State, route: Option<os_model::modulation::ModulatorId>, label: &str) -> String {
    match route {
        Some(from) => {
            let name = s.modulation.get(from).map_or_else(|| "The modulator".to_string(), crate::modulation::name_of);
            format!("{name}: {label} - drag to set how far it drives this")
        }
        None => format!("{label}: drag up / down"),
    }
}

/// The ring that marks a knob as the armed modulator's route destination, in the driver's colour.
fn route_ring(ui: &mut Ui, s: &State, from: os_model::modulation::ModulatorId, cx: f32, cy: f32, r: f32) {
    let accent = s.modulation.get(from).map_or(ui.t.modulation, |m| mod_accent(ui.t.modulation, &m.source));
    ui.c.stroke_circle(cx, cy, r, 1.3, accent.with_alpha(route_flash()));
}

/// One editor knob: the ring, its label, and a drag hit (`ModAct::Knob`).
fn mod_knob(ui: &mut Ui, s: &State, id: os_model::modulation::ModulatorId, field: crate::modulation::KnobField, label: &str, cx: f32, cy: f32) {
    let t = ui.t;
    let v = s.modulation.get(id).map_or(0.5, |m| crate::modulation::knob_value(m.source, field));
    let route = route_destination(s, id, field);
    let hov = ui.hit(Rect::new(cx - 14.0, cy - 14.0, 28.0, 28.0), Action::Mod(crate::modulation::ModAct::Knob(id, field)), mod_hint(s, route, label));
    kit::knob(ui.c, t, cx, cy, 11.0, v, false, kit::KnobLook::Ring, kit::State::hover(hov));
    if let Some(from) = route {
        route_ring(ui, s, from, cx, cy, 13.4);
    }
    // The routes driving this parameter, as the same arc a device parameter shows: one per source, from the knob's own
    // value to value + depth, with an endpoint handle that drags that route's depth (`ModAct::DepthDrag`). The route's
    // target is the graph's own modulator parameter, so nothing here depends on a device's identity. No live cursor:
    // the engine publishes a value for project parameters only (`engine_spec`).
    if let Some(param) = crate::modulation::knob_param(field) {
        mod_arcs(ui, s, os_model::target::modulator_param(id.0, param.code()), cx, cy, 15.5, v);
    }
    ui.c.text_center(fnt(9.0), Rect::new(cx - 23.0, cy + 11.0, 46.0, 12.0), t.text_dim, label);
}

/// A slot's right-click menu: on / off, clear routings, reroll the seed, remove.
fn mod_menu_paint(ui: &mut Ui, s: &State, m: &os_model::modulation::Modulator, anchor: Rect, r: Rect) {
    let t = ui.t;
    // Wide enough for a routing row's name ("Stop LFO 2: amount"), which the menu grows when routing mode is armed.
    let w = 168.0;
    let h = mod_menu_h(s, m);
    let panel = Rect::new(anchor.x.min(r.right() - w - 2.0), (anchor.y - h - 2.0).max(2.0), w, h);
    ui.c.fill_rrect(panel, 3.0, t.s(0x26292c));
    ui.c.stroke_rrect(panel, 3.0, 1.0, t.splitter_dark);
    let mut y = panel.y + 2.0;
    // The rows come from `modulation::menu_items`, so the keyboard (Enter, then arrows) walks exactly what is drawn.
    for (i, (label, action)) in crate::modulation::menu_items(s, m).into_iter().enumerate() {
        let row = Rect::new(panel.x + 2.0, y, panel.w - 4.0, 20.0);
        let hov = ui.hit(row, Action::Mod(action), label.clone());
        if hov || (s.mod_menu == Some(m.id) && s.mod_menu_sel == i) {
            ui.c.fill_rect(row, t.s(0x333a3e));
        }
        ui.c.text(fnt(11.5), row.x + 8.0, row.y + 2.0, t.text, &label);
        y += 21.0;
    }
}

/// Height of the options menu: a 21 px row each, from the same list the rows are drawn from.
fn mod_menu_h(s: &State, m: &os_model::modulation::Modulator) -> f32 {
    crate::modulation::menu_items(s, m).len() as f32 * 21.0 + 4.0
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
    let m = Rect::new(tile.right() - 80.0, tile.y + TILE_HEAD, 80.0, n as f32 * 22.0 + 4.0);
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

