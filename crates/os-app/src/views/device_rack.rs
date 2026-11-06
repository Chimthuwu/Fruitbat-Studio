// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The device rack's view (Lemon Studio's design: spec model/track-model.md §7): a panel docked at the bottom of
//! the main window, resizable by its splitter and collapsible to its header, whose device tiles hold the selected
//! track's devices in signal-flow order. Built-in Lemon plugins' editors render inside their tile; anything else
//! (and a tile whose editor failed) shows a placeholder body.

use os_ui::{Color, Rect, kit};

use crate::app::{Action, State};
use crate::device_rack::{self, Device, DeviceKind, HEADER_H, RackAct, TILE_W};
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
    // Horizontally scrolled: the chain is wider than the panel at times, so the tiles (and every hit rect
    // derived from this layout) shift left by the clamped scroll offset.
    let scroll = scroll_x(s, r.w);
    device_rack::rack_chain(s)
        .into_iter()
        .scan(body.x - scroll, |x, d| {
            let tile = Rect::new(*x, body.y, TILE_W, body.h);
            *x += TILE_W + GAP;
            Some((d, tile))
        })
        .collect()
}

/// The chain's content width over a panel `w` wide: every tile plus the gap after it.
pub(crate) fn content_w(s: &State) -> f32 {
    let n = device_rack::rack_chain(s).len();
    n as f32 * (TILE_W + GAP)
}

/// The rack's horizontal scroll clamped to what the panel's width `w` can show.
pub(crate) fn scroll_x(s: &State, w: f32) -> f32 {
    s.device_rack_scroll_x.clamp(0.0, (content_w(s) - w).max(0.0))
}

/// The wheel over the rack's body scrolls the chain horizontally (`d` notches, up = left, as the mixer's);
/// `r` is the rack panel rect. Returns whether the wheel was over the rack.
pub(crate) fn wheel(s: &mut State, r: Rect, x: f32, y: f32, d: f32) -> bool {
    let body = Rect::new(r.x, r.y + HEADER_H, r.w, r.h - HEADER_H);
    if !body.contains(x, y) || d == 0.0 {
        return false;
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
/// (non-builtin) tile's placeholder body gets its parameter rows too.
pub fn device_keys(s: &State) -> Vec<usize> {
    let r = panel_rect(s, s.screen.0, s.screen.1);
    layout(s, r).into_iter().filter_map(|(d, _)| d.key).collect()
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
    // The header buttons, right-aligned: ＋ Add (the plugin browser), Pin, Show / Hide — and, when the chain
    // overflows the panel, the ‹ › steppers (an alternative to the wheel).
    let overflow = crate::views::device_rack::content_w(s) > r.w
        && !s.device_rack_collapsed
        && r.h > HEADER_H;
    let mut bx = r.right() - 198.0 - 56.0 * f32::from(overflow);
    if overflow {
        let (at_start,) = (s.device_rack_scroll_x <= 0.5,);
        let steppers = [
            ("‹", -1.0, Rect::new(bx, head.y + 2.0, 24.0, 17.0)),
            ("›", 1.0, Rect::new(bx + 28.0, head.y + 2.0, 24.0, 17.0)),
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
    drop_highlight(ui, s, &tiles);
}

/// A "+" gap button at x: opens the device browser for the slot that follows it (`None` = first empty slot).
fn gap_button(ui: &mut Ui, x: f32, y: f32, slot: Option<u8>, h: f32) {
    let t = ui.t;
    let b = Rect::new(x + 2.0, y + h / 2.0 - 9.0, GAP - 4.0, 18.0);
    let hov = ui.hit(b, Action::DeviceRack(RackAct::AddAt(slot)), "Add a device here");
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
    // The kind's gradient behind the header's content, veiled so the LED and name stay readable (the pairs are
    // constants in `device_rack` so user theming can rewire them later).
    let (top, bottom) = device_rack::header_gradient(d.kind);
    ui.c.fill_rect_gradient(head, t.s(top), t.s(bottom), 0.0);
    ui.c.fill_rect(head, os_ui::Color::rgb(0).with_alpha(110));
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
    // The empty instrument slot: the whole tile opens the device browser (the instrument section of it).
    if d.kind == DeviceKind::NoInstrument {
        ui.c.fill_rect(body, t.s(0x131718));
        ui.c.stroke_rrect(body.inset(1.0), 2.0, 1.0, t.splitter_dark);
        ui.c.text(fnt(14.0), tile.x + 24.0, body.y + 10.0, t.text_dim, "This track has no instrument yet.");
        ui.c.text(fnt(13.0), tile.x + 24.0, body.y + 30.0, t.text_dim, "Click to choose one.");
        let _ = ui.hit(body, Action::DeviceRack(RackAct::AddAt(None)), "Choose this track's instrument");
        return;
    }
    match (d.key, body_mode(s, &d)) {
        // A built-in's editor renders into the tile (its native window is pinned there by the app each tick).
        (Some(key), BodyMode::Embedded) => {
            ui.c.fill_rect(body, t.s(0x131718));
            if s.device_rack_failed.contains(&key) {
                ui.c.text(fnt(13.0), body.x + 10.0, body.y + 8.0, t.text_dim, "Editor unavailable");
            }
        }
        // Everything else (hosted third-party, built-ins whose editor is popped out, failed, not open yet or
        // absent -- e.g. the linked MIDI FX): the parameter rows, and a Pop out button when there is an editor.
        (Some(key), BodyMode::Rows) => {
            let button = (!s.device_rack_float.contains(&key)).then_some(Action::DeviceRack(RackAct::PopOut(key)));
            placeholder(ui, s, &d, body, button)
        }
        // No hosted key (nothing hosted yet): the plain placeholder.
        (None, _) => placeholder(ui, s, &d, body, None),
    }
}

/// How a device tile's body is filled: by the editor of a built-in docked into it, or by parameter rows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum BodyMode {
    Embedded,
    Rows,
}

/// A tile's body mode: `Embedded` only while a built-in's editor is really open in the tile (popped-out, failed,
/// not-yet-open and editor-less devices get parameter rows instead of a blank body).
pub(crate) fn body_mode(s: &State, d: &Device) -> BodyMode {
    match d.key {
        Some(key) if d.builtin && !s.device_rack_float.contains(&key) && s.device_rack_editor.contains(&key) => BodyMode::Embedded,
        _ => BodyMode::Rows,
    }
}

/// Height of one parameter row (label line + bar).
const ROW_H: f32 = 26.0;

/// The top of the first parameter row: below the Pop out button strip when the tile has one.
fn rows_top(body: Rect, button: bool) -> f32 {
    body.y + if button { 32.0 } else { 8.0 }
}

/// A tile body's parameter row bar, by hosted key and param id: what a drag maps the pointer's x onto
/// (the same geometry the placeholder paints: cached params, `ROW_H` px a row, from the tile's body).
pub(crate) fn param_bar(s: &State, key: usize, id: u32) -> Option<Rect> {
    let (_, w, h) = ((), s.screen.0, s.screen.1);
    let (d, tile) = layout(s, panel_rect(s, w, h)).into_iter().find(|(d, _)| d.key == Some(key))?;
    let row = s.device_rack_params.get(&key)?.iter().position(|(i, ..)| *i == id)?;
    let body = body(tile);
    let button = !s.device_rack_float.contains(&key) && d.key.is_some();
    let bar = row_bar(body, rows_top(body, button), row);
    (bar.bottom() <= body.bottom() - 2.0).then_some(bar)
}

/// Row `i`'s bar: the body width less its margins, under the label line.
fn row_bar(body: Rect, top: f32, i: usize) -> Rect {
    Rect::new(body.x + 10.0, top + 15.0 + ROW_H * i as f32, (body.w - 20.0).max(20.0), 4.0)
}

/// How many parameter rows fit in `body` below `top`.
pub(crate) fn rows_fit(body: Rect, top: f32) -> usize {
    ((body.bottom() - top - 2.0) / ROW_H).floor().max(0.0) as usize
}

/// A placeholder body: parameter rows (when the device has any), and a `button` when there is one to press.
fn placeholder(ui: &mut Ui, s: &State, d: &Device, body: Rect, button: Option<Action>) {
    let t = ui.t;
    ui.c.fill_rect(body, t.s(0x131718));
    ui.c.stroke_rrect(body.inset(1.0), 2.0, 1.0, t.splitter_dark);
    let params: &[(u32, String, f64)] = d.key.and_then(|k| s.device_rack_params.get(&k)).map_or(&[], |v| v);
    let top = rows_top(body, button.is_some());
    if params.is_empty() {
        let label = if d.key.is_some() { "No parameters to show" } else { "No device" };
        ui.c.text_clip(fnt(13.0), body.x + 10.0, top, body.w - 20.0, t.text_dim, label);
    }
    // Live parameters (cached by `sync_embed`): the tile's controls when its editor is not embedded. A row's bar
    // sets the parameter where it is clicked or dragged (through `device_rack::set_param_value`, the one set
    // path). The name sits above the bar, truncated, with the value at its right end.
    if let Some(key) = d.key {
        let fit = rows_fit(body, top);
        for (i, (id, name, val)) in params.iter().take(fit).enumerate() {
            let bar = row_bar(body, top, i);
            let py = bar.y - 15.0;
            let v = (*val as f32).clamp(0.0, 1.0);
            let value = format!("{:.0}%", v * 100.0);
            let vw = 36.0;
            ui.c.text_clip(fnt(12.0), body.x + 10.0, py, body.w - 20.0 - vw, t.text_dim, name);
            ui.c.text(fnt(12.0), body.right() - 10.0 - vw + 6.0, py, t.text_dim, &value);
            let hit = Rect::new(bar.x - 4.0, py - 2.0, bar.w + 8.0, ROW_H - 3.0);
            ui.hit(
                hit,
                Action::DeviceRack(RackAct::ParamSet { key, id: *id, v: (((ui.mouse.0 - bar.x) / bar.w).clamp(0.0, 1.0)) as f64 }),
                "Click or drag to set this parameter",
            );
            ui.c.fill_rect(bar, t.s(0x2b3134));
            ui.c.fill_rect(Rect::new(bar.x, bar.y, bar.w * v, bar.h), t.glyph);
        }
    }
    if let Some(action) = button {
        let b = Rect::new(body.x + 10.0, body.y + 5.0, 70.0, 20.0);
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

