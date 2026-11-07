// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The clip launcher's view (Bitwig-style; plan: docs/clip-launcher.md): it takes the Playlist window's client area,
//! so the tracks line up with the mixer's order and the window keeps its own frame. Scenes run down the left edge,
//! one column per track, a cell per slot.

use os_ui::{Color, Rect};

use crate::app::{Action, State, Win};
use crate::launcher::{self, Act, SCENES};
use crate::views::{fnt, rgb, Ui};

/// The scene column's width: the scene's number, its name and its launch button.
const SCENE_W: f32 = 128.0;
/// The track header row's height (the name and its strip's colour).
const HEAD_H: f32 = 26.0;

/// Paints the launcher over the Playlist window's rect `r`.
pub fn paint(ui: &mut Ui, s: &State, r: Rect) {
    let t = ui.t;
    let (_, client) = super::framed(ui, s, Win::Playlist, r, "Clip Launcher", 13.0, true);
    ui.c.fill_rect(client, t.s(0x16191b));
    let cols = launcher::columns(s);
    let n = cols.len().max(1);
    let scene_w = SCENE_W.min(client.w * 0.3);
    let cell_w = ((client.w - scene_w) / n as f32).max(1.0);
    let head = Rect::new(client.x, client.y, client.w, HEAD_H);
    ui.c.fill_rect(head, t.s(0x1e2224));
    ui.c.fill_rect(Rect::new(head.x, head.bottom() - 1.0, head.w, 1.0), t.splitter_dark);

    // The header's left corner: stop every track.
    let all = Rect::new(client.x + 4.0, head.y + 4.0, scene_w - 44.0, HEAD_H - 8.0);
    let stopped = s.launcher.playing.is_empty();
    let hov = ui.hit(all, Action::Launcher(Act::StopAll), "Stop every track");
    let bg = if hov { t.s(0x2b3134) } else { t.s(0x222629) };
    ui.c.fill_rrect(all, 2.0, bg);
    ui.c.text(fnt(13.0), all.x + 6.0, all.y + 1.0, if stopped { t.text_dim } else { t.text }, "\u{25a0} Stop all");
    if cols.is_empty() {
        ui.c.text(fnt(13.0), client.x + scene_w + 12.0, client.y + 4.0, t.text_dim, "No tracks to launch");
        return;
    }

    // The track headers, in the mixer's order: the name over its strip's colour.
    for (i, c) in cols.iter().enumerate() {
        let x = client.x + scene_w + i as f32 * cell_w;
        let h = Rect::new(x, head.y, cell_w, HEAD_H);
        ui.c.fill_rect(Rect::new(x, head.y, 1.0, HEAD_H), t.splitter_dark);
        // A MIDI track whose instrument slot is empty plays nothing: its header says so, so a silent column explains
        // itself where its clips are launched (the device rack's empty instrument tile is where one is added). The
        // name gives up the room it needs, and a narrow column falls back to "!" rather than clipping the words.
        let hint_f = fnt(11.0);
        let room = (cell_w - 26.0).max(0.0) - 4.0;
        let hint = if c.no_instrument {
            ["no instrument", "no inst", "!"]
                .into_iter()
                .find(|text| ui.c.text_width(hint_f, text) <= room)
                .unwrap_or("")
        } else {
            ""
        };
        let hint_w = ui.c.text_width(hint_f, hint);
        let name = Rect::new(
            x + 6.0,
            h.y + 3.0,
            (cell_w - 26.0 - if hint.is_empty() { 0.0 } else { hint_w + 4.0 }).max(1.0),
            15.0,
        );
        ui.c.text_clip(fnt(13.0), name.x, name.y, name.w, t.text, &c.name);
        if !hint.is_empty() {
            ui.c.text(hint_f, x + cell_w - 23.0 - hint_w, h.y + 6.0, t.accent, hint);
        }
        if let Some(c) = c.color {
            ui.c.fill_rect(Rect::new(x + 5.0, h.bottom() - 5.0, cell_w - 28.0, 2.0), rgb(c));
        }
        // The track's stop button.
        let b = Rect::new(x + cell_w - 20.0, h.y + 5.0, 15.0, 15.0);
        let on = launcher::playing(s, c.id).is_some();
        let hov = ui.hit(b, Action::Launcher(Act::Stop(c.id)), "Stop this track");
        if on || hov {
            ui.c.fill_rrect(b, 2.0, if hov { t.s(0x333a3e) } else { t.s(0x26292c) });
            ui.c.text(fnt(12.0), b.x + 3.0, b.y, t.text, "\u{25a0}");
        }
    }

    // The scenes down the left, each with its own launch button, and the cells to their right.
    let rows = (client.h - HEAD_H).max(1.0) / SCENES as f32;
    for scene in 0..SCENES {
        let y = client.y + HEAD_H + scene as f32 * rows;
        let row = Rect::new(client.x, y, client.w, rows);
        if scene % 2 == 1 {
            ui.c.fill_rect(row, t.s(0x191c1e));
        }
        let label = Rect::new(client.x + 4.0, y + 2.0, scene_w - 8.0, rows - 4.0);
        let hov = ui.hit(
            label,
            Action::Launcher(Act::LaunchScene(scene)),
            "Start every clip in this scene",
        );
        if hov {
            ui.c.fill_rrect(label, 2.0, t.s(0x242828));
        }
        let num = format!("{}", scene + 1);
        ui.c.text(fnt(13.0), label.x + 4.0, label.y + rows * 0.5 - 9.0, t.text_dim, &num);
        let tri = Rect::new(label.x + 22.0, label.y + rows * 0.5 - 9.0, 12.0, 16.0);
        ui.c.fill_polygon(
            &[(tri.x, tri.y + 3.0), (tri.x, tri.y + 13.0), (tri.x + 9.0, tri.y + 8.0)],
            if hov { t.glyph } else { t.text_dim },
        );
        ui.c.text_clip(
            fnt(12.0),
            tri.x + 14.0,
            tri.y + 1.0,
            label.w - 46.0,
            t.text_dim,
            &launcher::scene_name(s, scene),
        );
        for (i, c) in cols.iter().enumerate() {
            let x = client.x + scene_w + i as f32 * cell_w;
            let cell = Rect::new(x + 3.0, y + 3.0, cell_w - 6.0, rows - 6.0);
            slot_paint(ui, s, c, scene, cell);
        }
    }
    // How the cells are filled, along the bottom of the header row.
    let help = "Right-click a slot: the current pattern";
    ui.c.text_clip(fnt(11.0), client.x + scene_w + 6.0, client.bottom() - 14.0, client.w - scene_w - 12.0, t.text_dim, help);
}

/// One slot: empty, holding a pattern, or the one the track is playing.
fn slot_paint(ui: &mut Ui, s: &State, c: &launcher::Column, scene: usize, cell: Rect) {
    let t = ui.t;
    let hov = ui.hit(
        cell,
        Action::Launcher(Act::Launch(c.id, scene)),
        "Launch this clip (an empty slot stops the track)",
    );
    let playing = launcher::playing(s, c.id) == Some(scene);
    let color = c.color.map(rgb);
    match launcher::clip(s, c.id, scene) {
        Some(slot) => {
            let base = color.unwrap_or(t.s(0x2f3a42));
            ui.c.fill_rrect(cell, 2.0, base.with_alpha(if playing { 235 } else { 165 }));
            ui.c.fill_rect_gradient(cell, Color::rgb(0).with_alpha(60), Color::rgb(0).with_alpha(0), 0.0);
            let name = crate::playlist::source_name(s, os_model::ClipSource::Pattern(slot.pattern));
            if playing {
                ui.c.stroke_rrect(cell, 2.0, 2.0, t.glyph);
                ui.c.fill_polygon(
                    &[(cell.x + 7.0, cell.y + 6.0), (cell.x + 7.0, cell.y + 16.0), (cell.x + 15.0, cell.y + 11.0)],
                    t.glyph,
                );
            }
            let (x, w) = if playing { (cell.x + 19.0, cell.w - 25.0) } else { (cell.x + 7.0, cell.w - 14.0) };
            ui.c.text_clip(fnt(13.0), x, cell.y + cell.h * 0.5 - 8.0, w, t.text, &name);
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
