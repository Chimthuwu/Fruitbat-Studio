// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The clip launcher's view (Bitwig-style; plan: docs/clip-launcher.md): the scenes run along the top, one launch
//! button each, and one row per track in the mixer's order, a cell per (track, scene). It takes the Playlist window's
//! client area, so the rows line up with the mixer. Right-click a clip, a scene button or a track row for its menu:
//! rename it in place, or give it a colour.

use os_ui::{Color, Rect};

use crate::app::{Action, State, Win};
use crate::launcher::{self, Act};
use crate::views::{fnt, rgb, Ui};

/// The track column's width: the name, its colour and the row's stop button.
const LABEL_W: f32 = 170.0;
/// The scene button row's height, across the top.
const HEAD_H: f32 = 28.0;
/// Gap between a row's cells.
const PAD: f32 = 3.0;

/// Paints the launcher over the Playlist window's rect `r`.
pub fn paint(ui: &mut Ui, s: &State, r: Rect) {
    let t = ui.t;
    let (_, client) = super::framed(ui, s, Win::Playlist, r, "Clip Launcher", 13.0, true);
    ui.c.fill_rect(client, t.s(0x16191b));
    let cols = launcher::columns(s);
    let label_w = LABEL_W.min(client.w * 0.4);
    let scene_w = ((client.w - label_w) / launcher::SCENES as f32).max(1.0);
    let rows = cols.len().max(1);
    let row_h = ((client.h - HEAD_H) / rows as f32).max(16.0);
    ui.c.push_clip(client);

    // The corner: stop every track (the scene buttons run to its right, along the top).
    let corner = Rect::new(client.x, client.y, label_w, HEAD_H);
    ui.c.fill_rect(corner, t.s(0x1e2224));
    let all = Rect::new(corner.x + 4.0, corner.y + 4.0, label_w - 8.0, HEAD_H - 8.0);
    let stopped = s.launcher.playing.is_empty();
    let hov = ui.hit(all, Action::Launcher(Act::StopAll), "Stop every track");
    ui.c.fill_rrect(all, 2.0, if hov { t.s(0x2b3134) } else { t.s(0x222629) });
    ui.c.text(fnt(13.0), all.x + 6.0, all.y + 1.0, if stopped { t.text_dim } else { t.text }, "\u{25a0} Stop all");

    // The scene buttons: one per scene, left to right. A scene that any track plays is marked, a queued one outlined.
    for scene in 0..launcher::SCENES {
        let b = Rect::new(client.x + label_w + scene as f32 * scene_w, client.y, scene_w, HEAD_H);
        ui.c.fill_rect(Rect::new(b.x, b.y, 1.0, HEAD_H), t.splitter_dark);
        let live = s.launcher.playing.values().any(|p| p.scene == scene);
        let queued = s.launcher.pending.values().any(|p| p.scene == scene);
        let base = launcher::scene_color(s, scene).map(rgb).unwrap_or(t.s(0x1e2224));
        let hov = ui.hit(b, Action::Launcher(Act::LaunchScene(scene)), "Launch every clip of this scene (right-click: rename and colour it)");
        ui.c.fill_rrect(b.inset(1.0), 2.0, if hov { base.shade(0.1) } else { base });
        if live {
            ui.c.fill_rect(Rect::new(b.x + 3.0, b.bottom() - 3.0, b.w - 6.0, 2.0), t.accent);
        }
        if queued {
            ui.c.stroke_rrect(b.inset(1.0), 2.0, 1.5, t.accent);
        }
        // The number, then the play mark and the scene's name.
        let num = format!("{}", scene + 1);
        ui.c.text(fnt(12.0), b.x + 5.0, b.y + 6.0, t.text_dim, &num);
        let tri = Rect::new(b.x + 18.0, b.y + 7.0, 11.0, HEAD_H - 14.0);
        ui.c.fill_polygon(
            &[(tri.x, tri.y + 2.0), (tri.x, tri.y + tri.h - 2.0), (tri.x + 8.0, tri.cy())],
            if hov { t.glyph } else { t.text },
        );
        ui.c.text_clip(fnt(13.0), tri.right() + 4.0, b.y + 6.0, b.right() - tri.right() - 8.0, t.text, &launcher::scene_name(s, scene));
    }

    if cols.is_empty() {
        ui.c.text(fnt(13.0), client.x + 8.0, client.y + HEAD_H + 8.0, t.text_dim, "No tracks to launch");
        ui.c.pop_clip();
        menu_paint(ui, s);
        edit_paint(ui, s);
        return;
    }

    // The track rows: the name and its stop button in the left column, then a cell per scene.
    for (i, c) in cols.iter().enumerate() {
        let y = client.y + HEAD_H + i as f32 * row_h;
        let row = Rect::new(client.x, y, label_w, row_h);
        if i % 2 == 1 {
            ui.c.fill_rect(Rect::new(client.x, y, client.w, row_h), t.s(0x191c1e));
        }
        let hov = ui.hit(row, Action::Launcher(Act::SelectTrack(c.id)), "Click: select this track (right-click: rename and colour it)");
        // A MIDI track with no instrument plays nothing: a mark after its name says why (the rack has the same hint).
        let mark_w = if c.no_instrument { ui.c.text_width(fnt(12.0), "!") + 4.0 } else { 0.0 };
        let name = Rect::new(row.x + 8.0, row.y + (row_h - 15.0) / 2.0, (row.w - 38.0 - mark_w).max(1.0), 15.0);
        ui.c.text_clip(fnt(13.0), name.x, name.y, name.w, if hov { t.text } else { t.tx(0xdce8f0) }, &c.name);
        if c.no_instrument {
            ui.c.text(fnt(12.0), name.right() + 3.0, name.y + 1.0, t.accent, "!");
        }
        if let Some(col) = c.color {
            ui.c.fill_rect(Rect::new(row.x + 8.0, row.bottom() - 3.0, (label_w - 40.0).max(1.0), 2.0), rgb(col));
        }
        // The row's stop button.
        let stop = Rect::new(row.right() - 20.0, row.y + (row_h - 14.0) / 2.0, 14.0, 14.0);
        let sh = ui.hit(stop, Action::Launcher(Act::Stop(c.id)), "Stop this track's clip: it plays its arrangement clips again");
        if sh || launcher::playing(s, c.id).is_some() {
            ui.c.fill_rrect(stop, 2.0, if sh { t.s(0x333a3e) } else { t.s(0x26292c) });
            ui.c.text(fnt(11.0), stop.x + 3.0, stop.y - 1.0, t.text, "\u{25a0}");
        }
        for scene in 0..launcher::SCENES {
            let x = client.x + label_w + scene as f32 * scene_w;
            let cell = Rect::new(x + PAD, row.y + PAD, scene_w - 2.0 * PAD, row_h - 2.0 * PAD);
            slot_paint(ui, s, c, scene, cell);
        }
    }
    ui.c.pop_clip();

    // How the cells are filled, along the bottom of the window.
    ui.c.text_clip(
        fnt(11.0),
        client.x + 6.0,
        client.bottom() - 14.0,
        client.w - 12.0,
        t.text_dim,
        "Right-click a clip for its launch settings and next action, a scene or a track to rename and colour it",
    );
    menu_paint(ui, s);
    edit_paint(ui, s);
}

/// One cell: empty, holding a clip, or the one the track plays (queued cells are outlined while they wait).
fn slot_paint(ui: &mut Ui, s: &State, c: &launcher::Column, scene: usize, cell: Rect) {
    let t = ui.t;
    let hov = ui.hit(
        cell,
        Action::Launcher(Act::Launch(c.id, scene)),
        "Launch this clip (an empty slot stops the track; right-click: its menu)",
    );
    let playing = launcher::playing(s, c.id) == Some(scene);
    // Queued for the next bar line (a launch made while the transport runs): the cell is picked out, so the wait is
    // visible rather than the click seeming to have done nothing.
    let queued = launcher::pending(s, c.id) == Some(scene);
    match launcher::clip(s, c.id, scene) {
        Some(slot) => {
            let base = launcher::clip_color(s, c.id, scene).map(rgb).unwrap_or(t.s(0x2f3a42));
            ui.c.fill_rrect(cell, 2.0, base.with_alpha(if playing { 235 } else { 165 }));
            ui.c.fill_rect_gradient(cell, Color::rgb(0).with_alpha(60), Color::rgb(0).with_alpha(0), 0.0);
            let name = launcher::clip_name(s, c.id, scene);
            if playing {
                ui.c.stroke_rrect(cell, 2.0, 2.0, t.glyph);
                ui.c.fill_polygon(
                    &[
                        (cell.x + 7.0, cell.cy() - 6.0),
                        (cell.x + 7.0, cell.cy() + 6.0),
                        (cell.x + 15.0, cell.cy()),
                    ],
                    t.glyph,
                );
            }
            let (x, w) = if playing { (cell.x + 19.0, cell.w - 25.0) } else { (cell.x + 7.0, cell.w - 14.0) };
            // A clip with a next action carries a mark, so a follow chain is visible without opening its menu.
            let follow = if slot.next != launcher::NextAction::None { 12.0 } else { 0.0 };
            let lh = ui.c.line_height(fnt(13.0));
            ui.c.text_clip(fnt(13.0), x, (cell.cy() - lh / 2.0).round(), (w - follow).max(1.0), t.text, &name);
            if follow > 0.0 {
                ui.c.text(fnt(12.0), cell.right() - follow - 2.0, (cell.cy() - lh / 2.0).round(), t.glyph, "\u{2192}");
            }
            if queued {
                ui.c.stroke_rrect(cell, 2.0, 2.0, t.accent);
            }
        }
        None => {
            ui.c.fill_rrect(cell, 2.0, if hov { t.s(0x242a2d) } else { t.s(0x1b1f21) });
            ui.c.stroke_rrect(cell, 2.0, 1.0, t.splitter_dark);
            if hov {
                ui.c.text(fnt(13.0), cell.cx() - 4.0, cell.cy() - 9.0, t.text_dim, "+");
            }
        }
    }
}

/// The context menu a right-click opened (`launcher::Menu`): the target's commands, then a swatch per colour (the
/// dark one takes its colour away).
fn menu_paint(ui: &mut Ui, s: &State) {
    let Some(m) = &s.launcher.menu else { return };
    let t = ui.t;
    let items = launcher::menu_items(s, m.target);
    let row_h = 19.0;
    let (sw, gap) = (15.0, 3.0);
    let w = 10.0 + (launcher::PALETTE.len() + 1) as f32 * (sw + gap);
    let h = 5.0 + items.len() as f32 * row_h + 6.0 + sw + 7.0;
    let x = m.at.0.clamp(0.0, (s.screen.0 - w - 2.0).max(0.0));
    let y = m.at.1.clamp(0.0, (s.screen.1 - h - 2.0).max(0.0));
    let area = Rect::new(x, y, w, h);
    ui.c.fill_rect(area, t.s(0x24282b));
    ui.c.stroke_rrect(area, 0.0, 1.0, t.glyph);
    let mut yy = y + 5.0;
    for (label, value, act) in items {
        let row = Rect::new(x + 3.0, yy, w - 6.0, row_h);
        let hov = ui.hit(row, Action::Launcher(act), label);
        if hov {
            ui.c.fill_rect(row, t.s(0x2f363a));
        }
        ui.c.text(fnt(13.0), row.x + 7.0, row.y + 2.0, t.text, label);
        // The current value on the right (a launch setting a click walks to its next value).
        if !value.is_empty() {
            let vw = ui.c.text_width(fnt(13.0), &value);
            ui.c.text(fnt(13.0), (row.right() - 7.0 - vw).max(row.x + 7.0), row.y + 2.0, t.accent, &value);
        }
        yy += row_h;
    }
    yy += 6.0;
    let mut sx = x + 7.0;
    for color in std::iter::once(None).chain((0..launcher::PALETTE.len()).map(Some)) {
        let r = Rect::new(sx, yy, sw, sw);
        let act = launcher::color_act(m.target, color.map(launcher::palette));
        let hov = ui.hit(r, Action::Launcher(act), "Colour");
        match color {
            Some(c) => ui.c.fill_rrect(r, 2.0, rgb(launcher::palette(c))),
            None => {
                ui.c.fill_rrect(r, 2.0, t.s(0x14171a));
                ui.c.line(r.x + 4.0, r.bottom() - 4.0, r.right() - 4.0, r.y + 4.0, 1.0, t.text_dim);
            }
        }
        if hov {
            ui.c.stroke_rrect(r, 2.0, 1.5, t.glyph);
        }
        sx += sw + gap;
    }
}

/// The rename box opened from the launcher (`EditTarget::LauncherClip`, `LauncherScene`, and a track's `TrackName`):
/// the playlist's inline box, drawn over the grid because the launcher has replaced it.
fn edit_paint(ui: &mut Ui, s: &State) {
    let Some(e) = &s.playlist.edit else { return };
    use crate::playlist::EditTarget;
    if !matches!(e.target, EditTarget::LauncherClip(..) | EditTarget::LauncherScene(_) | EditTarget::TrackName(_)) {
        return;
    }
    let t = ui.t;
    let r = Rect::new(e.at.0, e.at.1, 150.0, 20.0);
    ui.hit(r, Action::Swallow, "Type, Enter accepts, Esc cancels");
    ui.c.fill_rect(r, t.s(0x0e1214));
    ui.c.stroke_rrect(r, 0.0, 1.0, t.glyph);
    ui.c.text_clip(fnt(14.0), r.x + 4.0, r.y + 2.0, r.w - 8.0, t.text, &e.text);
    let cw = ui.c.text_width(fnt(14.0), &e.text).min(r.w - 10.0);
    ui.c.fill_rect(Rect::new(r.x + 5.0 + cw, r.y + 3.0, 1.0, 14.0), t.text);
}
