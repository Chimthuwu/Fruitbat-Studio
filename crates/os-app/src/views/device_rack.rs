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
use crate::device_rack::{self, Device, DeviceKind, RackParam, HEADER_H, RackAct, TILE_W};
use crate::icons::{self, Icon};
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
            let w = tile_width(s, &d, body.h);
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
    device_rack::rack_chain(s).iter().map(|d| tile_width(s, d, body_h) + GAP).sum()
}

/// The rack's horizontal scroll clamped to what the panel's width `w` can show.
pub(crate) fn scroll_x(s: &State, w: f32) -> f32 {
    s.device_rack_scroll_x.clamp(0.0, (content_w(s) - w).max(0.0))
}

/// The wheel over the rack's body: the tile under the pointer scrolls its own controls when the device has more
/// than its body shows, else the chain scrolls horizontally (`d` notches, up = left, as the mixer's); `r` is the
/// rack panel rect. Returns whether the wheel was over the rack.
pub(crate) fn wheel(s: &mut State, r: Rect, x: f32, y: f32, d: f32) -> bool {
    let area = Rect::new(r.x, r.y + HEADER_H, r.w, r.h - HEADER_H);
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

/// A tile's editor body: below its header.
fn body(tile: Rect) -> Rect {
    Rect::new(tile.x, tile.y + HEADER_H, tile.w, tile.h - HEADER_H)
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
    let mut bx = r.right() - 228.0 - 56.0 * f32::from(overflow);
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
    bx += 66.0;
    // The out-arrow: every device in the chain shows its editor in its own floating window (lit green
    // while they are out), or the windows close and the tiles draw the editors again. An out-arrow,
    // not a wrench — the wrench stays the device wrapper's own symbol (the plugin window's strip).
    let windows = !s.device_rack_float.is_empty();
    header_icon_button(
        ui,
        Rect::new(bx, head.y + 2.0, 24.0, 17.0),
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
    // The name, then the header's pop-out and menu.
    let menu = Rect::new(tile.right() - 22.0, head.y + 4.0, 17.0, 15.0);
    let popout = Rect::new(menu.x - 22.0, head.y + 2.0, 19.0, 19.0);
    // The drum machine's tile flips between its pads and its cards: one more button, and the name
    // gives up the room it takes.
    let flip = Rect::new(popout.x - 22.0, head.y + 2.0, 19.0, 19.0);
    let drum = d.key.is_some_and(|_| matches!(crate::rack_ui::ui(plugin_name(s, &d)), Some(crate::rack_ui::Ui::Drum)));
    ui.c.text_clip(
        fnt(14.0),
        tile.x + 24.0,
        head.y + 3.0,
        tile.w - 24.0 - if drum { 86.0 } else { 64.0 },
        t.text,
        &d.name,
    );
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
        cell_paint(ui, key, &params[*i], *cell, body);
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
    for (hr, id, v) in &img.hits {
        let cell = Rect::new(r.x + hr.x * kx, r.y + hr.y * ky, (hr.w * kx).max(2.0), (hr.h * ky).max(2.0));
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
}

/// One control cell: its name, a dial (or a switch for a two-state parameter) and the plugin's own value text.
fn cell_paint(ui: &mut Ui, key: usize, p: &RackParam, cell: Rect, body: Rect) {
    let t = ui.t;
    let cx = cell.cx();
    ui.c.text_clip(fnt(11.0), cell.x + 2.0, cell.y + 1.0, CELL_W - 4.0, t.text_dim, &p.name);
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

