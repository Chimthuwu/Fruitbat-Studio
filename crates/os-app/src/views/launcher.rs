// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The clip launcher's view (Bitwig-style; plan: docs/clip-launcher.md): the scenes run along the top, one launch
//! button each, and one row per track in the mixer's order, a cell per (track, scene). It takes the Playlist window's
//! client area, so the rows line up with the mixer. Right-click a clip, a scene button or a track row for its menu:
//! rename it in place, or give it a colour.
//!
//! The grid outgrows the window in both directions: the scene columns keep [`MIN_COL`] px (so a clip's name stays
//! readable) and the track rows [`MIN_ROW`], and what does not fit scrolls ([`wheel`], `Launcher::scroll_x` /
//! `scroll_y`). The scene buttons and the track column stay put while the cells move under them, so a track's name is
//! never scrolled away from its clips (Bitwig's headers do the same).

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
/// The least width of a scene column: narrower and its clip's name would be cut, so the columns scroll sideways
/// instead of squashing (a window too narrow for all [`launcher::SCENES`] of them scrolls).
pub const MIN_COL: f32 = 96.0;
/// The least height of a track row: below this a clip's name could not be read, so the rows scroll up and down.
pub const MIN_ROW: f32 = 22.0;
/// How much one wheel notch scrolls the scene columns sideways, in px (the playlist's own step).
const COL_NOTCH: f32 = 60.0;
/// How many track rows one wheel notch scrolls (the piano roll's keyboard scrolls three).
const ROW_NOTCH: f32 = 3.0;

/// The grid's layout over a client rect: the scene columns and the track rows at their least sizes, and how far each
/// axis is scrolled (clamped, so neither end leaves a gap).
pub struct Geom {
    pub client: Rect,
    pub label_w: f32,
    pub scene_w: f32,
    pub row_h: f32,
    /// The track rows the grid has.
    pub rows: usize,
    pub sx: f32,
    pub sy: f32,
}

impl Geom {
    /// The scene columns' width, all of them (`launcher::SCENES`).
    fn scenes_w(&self) -> f32 {
        self.scene_w * launcher::SCENES as f32
    }

    /// How far the scene columns can scroll sideways: what the window cannot show of them.
    pub fn max_sx(&self) -> f32 {
        (self.scenes_w() - (self.client.w - self.label_w).max(1.0)).max(0.0)
    }

    /// How far the track rows can scroll up.
    pub fn max_sy(&self) -> f32 {
        (self.row_h * self.rows.max(1) as f32 - (self.client.h - HEAD_H).max(1.0)).max(0.0)
    }

    /// Where the scene buttons show: across the top, right of the track column (they do not scroll up).
    pub fn head(&self) -> Rect {
        Rect::new(self.client.x + self.label_w, self.client.y, (self.client.w - self.label_w).max(0.0), HEAD_H)
    }

    /// Where the track rows show: under the scene buttons, across the whole window.
    pub fn rows(&self) -> Rect {
        Rect::new(self.client.x, self.client.y + HEAD_H, self.client.w, (self.client.h - HEAD_H).max(0.0))
    }

    /// The track column: down the left of the rows (it does not scroll sideways).
    pub fn label(&self) -> Rect {
        let rows = self.rows();
        Rect::new(self.client.x, rows.y, self.label_w, rows.h)
    }

    /// Where the cells show: right of the track column and under the scene buttons (both stay put).
    pub fn cells(&self) -> Rect {
        let rows = self.rows();
        Rect::new(self.client.x + self.label_w, rows.y, (self.client.w - self.label_w).max(0.0), rows.h)
    }

    /// A scene column's x at the current scroll.
    pub fn col_x(&self, scene: usize) -> f32 {
        self.client.x + self.label_w + scene as f32 * self.scene_w - self.sx
    }

    /// A row's y at the current scroll.
    pub fn row_y(&self, i: usize) -> f32 {
        self.client.y + HEAD_H + i as f32 * self.row_h - self.sy
    }

    /// Whether either axis has something to scroll.
    pub fn scrolls(&self) -> bool {
        self.max_sx() > 0.0 || self.max_sy() > 0.0
    }
}

/// The grid's layout over `client`, from the window's size and the launcher's own scroll.
pub fn geom(s: &State, client: Rect) -> Geom {
    let rows = launcher::columns(s).len().max(1);
    let label_w = LABEL_W.min(client.w * 0.4);
    let scene_w = ((client.w - label_w).max(1.0) / launcher::SCENES as f32).max(MIN_COL);
    let row_h = ((client.h - HEAD_H).max(1.0) / rows as f32).max(MIN_ROW);
    let mut g = Geom { client, label_w, scene_w, row_h, rows, sx: 0.0, sy: 0.0 };
    g.sx = s.launcher.scroll_x.clamp(0.0, g.max_sx());
    g.sy = s.launcher.scroll_y.clamp(0.0, g.max_sy());
    g
}

/// The Playlist window's client area: what the launcher's grid fills, the same rect `paint` lays it out in.
pub fn client_rect(s: &State) -> Rect {
    super::frame_rects(s, Win::Playlist, s.win_rect(Win::Playlist)).1
}

/// The wheel over the launcher's grid: the track rows scroll up and down (three a notch), and sideways — Shift, or a
/// sideways wheel — the scene columns. Returns whether the pointer was over the grid (which then takes the wheel).
pub fn wheel(s: &mut State, x: f32, y: f32, dx: f32, dy: f32, shift: bool) -> bool {
    let client = client_rect(s);
    if !client.contains(x, y) {
        return false;
    }
    let g = geom(s, client);
    let (dx, dy) = if shift { (dy, dx) } else { (dx, dy) };
    let (sx, sy) = crate::scroll_speed::scaled(dx, dy);
    if sy != 0.0 {
        s.launcher.scroll_y = (g.sy - sy * g.row_h * ROW_NOTCH).clamp(0.0, g.max_sy());
    }
    if sx != 0.0 {
        s.launcher.scroll_x = (g.sx - sx * COL_NOTCH).clamp(0.0, g.max_sx());
    }
    true
}

/// `r` cut to the part of it inside `area`.
fn clip_to(r: Rect, area: Rect) -> Rect {
    let x0 = r.x.max(area.x);
    let y0 = r.y.max(area.y);
    let x1 = r.right().min(area.right());
    let y1 = r.bottom().min(area.bottom());
    Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

/// Registers `act` over the part of `r` that `area` shows: a cell or a row scrolled out of the grid answers nowhere,
/// and one half out of it only where it shows (so a scrolled-away cell never takes a click meant for what is under it).
fn hit_in(ui: &mut Ui, r: Rect, area: Rect, act: Action, hint: &'static str) -> bool {
    let visible = clip_to(r, area);
    visible.w > 0.0 && visible.h > 0.0 && ui.hit(visible, act, hint)
}

/// Paints the launcher over the Playlist window's rect `r`.
pub fn paint(ui: &mut Ui, s: &State, r: Rect) {
    let t = ui.t;
    // The launcher takes the Playlist window whole (as `playlist::paint`): a click in its frame or client area is
    // the launcher's, not a click through to what is under the window.
    super::window_body(ui, r);
    let (_, client) = super::framed(ui, s, Win::Playlist, r, "Clip Launcher", 13.0, true);
    ui.c.fill_rect(client, t.s(0x16191b));
    let cols = launcher::columns(s);
    let g = geom(s, client);
    ui.c.push_clip(client);

    // The corner: stop every track (the scene buttons run to its right, along the top).
    let corner = Rect::new(client.x, client.y, g.label_w, HEAD_H);
    ui.c.fill_rect(corner, t.s(0x1e2224));
    let all = Rect::new(corner.x + 4.0, corner.y + 4.0, g.label_w - 8.0, HEAD_H - 8.0);
    let stopped = s.launcher.playing.is_empty();
    let hov = ui.hit(all, Action::Launcher(Act::StopAll), "Stop every track");
    ui.c.fill_rrect(all, 2.0, if hov { t.s(0x2b3134) } else { t.s(0x222629) });
    ui.c.text(fnt(13.0), all.x + 6.0, all.y + 1.0, if stopped { t.text_dim } else { t.text }, "\u{25a0} Stop all");

    // The scene buttons: one per scene, left to right, scrolling sideways with the columns they head. A scene that any
    // track plays is marked, a queued one outlined.
    let head = g.head();
    ui.c.push_clip(head);
    for scene in 0..launcher::SCENES {
        let b = Rect::new(g.col_x(scene), client.y, g.scene_w, HEAD_H);
        ui.c.fill_rect(Rect::new(b.x, b.y, 1.0, HEAD_H), t.splitter_dark);
        let live = s.launcher.playing.values().any(|p| p.scene == scene);
        let queued = s.launcher.pending.values().any(|p| p.scene == scene);
        let base = launcher::scene_color(s, scene).map(rgb).unwrap_or(t.s(0x1e2224));
        let hov = hit_in(ui, b, head, Action::Launcher(Act::LaunchScene(scene)), "Launch every clip of this scene (right-click: rename and colour it)");
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
    ui.c.pop_clip();

    if cols.is_empty() {
        ui.c.text(fnt(13.0), client.x + 8.0, client.y + HEAD_H + 8.0, t.text_dim, "No tracks to launch");
        ui.c.pop_clip();
        menu_paint(ui, s);
        edit_paint(ui, s);
        return;
    }

    // The track rows: the name and its stop button in the left column (which stays put), then a cell per scene.
    let (label, cells) = (g.label(), g.cells());
    ui.c.push_clip(g.rows());
    for (i, c) in cols.iter().enumerate() {
        let y = g.row_y(i);
        let row = Rect::new(client.x, y, g.label_w, g.row_h);
        if i % 2 == 1 {
            ui.c.fill_rect(Rect::new(client.x, y, client.w, g.row_h), t.s(0x191c1e));
        }
        // The rows the arrangement has selected (a header click, or Shift+click for a range) read as one block.
        if crate::playlist::track_selected(s, c.lane) {
            ui.c.fill_rect(Rect::new(client.x, y, client.w, g.row_h), t.s(0x252c30));
            ui.c.fill_rect(Rect::new(client.x, y, 3.0, g.row_h), t.accent);
        }
        // The track header fills with the track's colour (Bitwig-style), so the launcher's rows read like the
        // playlist's — brighter while the row is selected.
        if let Some(col) = c.color {
            let sel = crate::playlist::track_selected(s, c.lane);
            ui.c.fill_rrect(row.inset(1.0), 2.0, rgb(col).with_alpha(if sel { 95 } else { 55 }));
        }
        let hov = hit_in(
            ui,
            row,
            label,
            Action::Launcher(Act::SelectTrack(c.id)),
            "Click: select this track, Shift+click: select a range of tracks (right-click: rename and colour it)",
        );
        // A MIDI track with no instrument plays nothing: a mark after its name says why (the rack has the same hint).
        let mark_w = if c.no_instrument { ui.c.text_width(fnt(12.0), "!") + 4.0 } else { 0.0 };
        let name = Rect::new(row.x + 8.0, row.y + (g.row_h - 15.0) / 2.0, (row.w - 38.0 - mark_w).max(1.0), 15.0);
        ui.c.text_clip(fnt(13.0), name.x, name.y, name.w, if hov { t.text } else { t.tx(0xdce8f0) }, &c.name);
        if c.no_instrument {
            ui.c.text(fnt(12.0), name.right() + 3.0, name.y + 1.0, t.accent, "!");
        }
        if let Some(col) = c.color {
            ui.c.fill_rect(Rect::new(row.x + 8.0, row.bottom() - 3.0, (g.label_w - 40.0).max(1.0), 2.0), rgb(col));
        }
        // The row's stop button.
        let stop = Rect::new(row.right() - 20.0, row.y + (g.row_h - 14.0) / 2.0, 14.0, 14.0);
        let sh = hit_in(ui, stop, label, Action::Launcher(Act::Stop(c.id)), "Stop this track's clip: it plays its arrangement clips again");
        if sh || launcher::playing(s, c.id).is_some() {
            ui.c.fill_rrect(stop, 2.0, if sh { t.s(0x333a3e) } else { t.s(0x26292c) });
            ui.c.text(fnt(11.0), stop.x + 3.0, stop.y - 1.0, t.text, "\u{25a0}");
        }
        ui.c.push_clip(cells);
        for scene in 0..launcher::SCENES {
            let cell = Rect::new(g.col_x(scene) + PAD, y + PAD, g.scene_w - 2.0 * PAD, g.row_h - 2.0 * PAD);
            slot_paint(ui, s, c, scene, cell, cells);
        }
        ui.c.pop_clip();
    }
    ui.c.pop_clip();

    // How the cells are filled, along the bottom of the window — and how to reach the ones that do not fit.
    let mut hint = String::from(
        "Double-click an empty cell to make a clip (drop a sample on one to play it) - Shift+click a track to select a range - right-click a clip, scene or track to rename and colour it",
    );
    if g.scrolls() {
        hint.push_str(" - wheel scrolls, Shift for the scenes");
    }
    ui.c.text_clip(fnt(11.0), client.x + 6.0, client.bottom() - 14.0, client.w - 12.0, t.text_dim, &hint);
    // The launcher leaves the band to the toolbar and the overlays to `paint`: anything after it (the toolbar paints
    // after the windows) must not stay clipped to the launcher's client (the toolbar once vanished over the launcher).
    ui.c.pop_clip();
    menu_paint(ui, s);
    edit_paint(ui, s);
}

/// One cell: empty, holding a clip, or the one the track plays (queued cells are outlined while they wait). `area` is
/// what the grid shows of the cells, so one half scrolled out only answers where it shows.
fn slot_paint(ui: &mut Ui, s: &State, c: &launcher::Column, scene: usize, cell: Rect, area: Rect) {
    let t = ui.t;
    let hov = hit_in(
        ui,
        cell,
        area,
        Action::Launcher(Act::Launch(c.id, scene)),
        "Launch this clip (an empty slot stops the track; double-click: a new clip, right-click: its menu)",
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
            if slot.audio.is_some() {
                audio_paint(ui, s, &slot, cell, t.glyph.with_alpha(110));
            }
            let name = launcher::clip_name(s, c.id, scene);
            // Bitwig-style play/stop button in the top-right corner: a play triangle while the clip is stopped, a
            // stop square while it plays. While playing it is a `Stop` hit (distinct from the cell's `Launch` hit, so
            // clicking it stops the clip); while stopped the whole cell already launches, so the button is drawn but
            // not separately hit.
            let btn_w = 15.0;
            let btn = Rect::new(cell.right() - btn_w - 1.0, cell.y + 2.0, btn_w, (cell.h - 5.0).max(10.0));
            if playing {
                let bhov = hit_in(ui, btn, area, Action::Launcher(Act::Stop(c.id)), "Stop this clip");
                ui.c.fill_rrect(btn, 2.0, if bhov { t.s(0x39424a) } else { t.s(0x23282b) });
                ui.c.fill_rect(btn.inset(3.5), t.glyph);
            } else {
                ui.c.fill_rrect(btn, 2.0, t.s(0x23282b));
                let tri = Rect::new(btn.x + 4.5, btn.y + 3.5, 5.0, 7.0);
                ui.c.fill_polygon(&[(tri.x, tri.y), (tri.x, tri.y + tri.h), (tri.x + tri.w, tri.cy())], t.text);
            }
            if queued && !playing {
                ui.c.stroke_rrect(btn, 2.0, 1.5, t.accent);
            }
            if playing {
                ui.c.stroke_rrect(cell, 2.0, 2.0, t.glyph);
            }
            // A clip with a next action carries a mark, so a follow chain is visible without opening its menu.
            let follow = if slot.next != launcher::NextAction::None { 12.0 } else { 0.0 };
            let gutter = btn_w + 4.0 + follow;
            let (x, w) = if playing { (cell.x + 19.0, cell.w - 25.0 - gutter) } else { (cell.x + 7.0, cell.w - 14.0 - gutter) };
            let lh = ui.c.line_height(fnt(13.0));
            ui.c.text_clip(fnt(13.0), x, (cell.cy() - lh / 2.0).round(), w.max(1.0), t.text, &name);
            if follow > 0.0 {
                ui.c.text(fnt(12.0), cell.right() - btn_w - 5.0 - follow, (cell.cy() - lh / 2.0).round(), t.glyph, "\u{2192}");
            }
            if queued {
                ui.c.stroke_rrect(cell, 2.0, 2.0, t.accent);
            }
            // The cell a click selected (the panel's clip editor shows its sample): a thin mark, under the
            // playing and queued rings (`launcher::Panel`).
            if s.launcher.sel == Some((c.id, scene)) && !playing && !queued {
                ui.c.stroke_rrect(cell, 2.0, 1.0, t.text_dim);
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
    // The cell a take records into: ringed in the record colour with a dot, so the armed cell is not lost among the
    // clips (an empty one stays empty until the first note, then shows the clip it is making).
    if launcher::recording(s, c.id, scene) {
        ui.c.stroke_rrect(cell, 2.0, 1.5, t.record);
        ui.c.text(fnt(10.0), cell.right() - 10.0, cell.y + 1.0, t.record, "\u{25cf}");
    }
}

/// The bottom panel while the launcher is out and an audio clip's editor stands ([`launcher::Panel`]): where the
/// device rack goes, the selected clip shows — its name, its sample's peaks — and the Tesla hosted on the clip's
/// audio channel holds the editing ([`launcher::open_audio_clip`]; its window opens with the panel, as any
/// plugin's does). ◀ Devices gives the rack its panel back.
pub fn paint_panel(ui: &mut Ui, s: &State, r: Rect) {
    let t = ui.t;
    ui.c.fill_rect(r, t.s(0x16191b));
    ui.c.fill_rect(Rect::new(r.x, r.y, r.w, 1.0), t.splitter_dark);
    // The rack comes back from here too (a track's click does the same).
    let back = Rect::new(r.x + 6.0, r.y + 5.0, 78.0, 19.0);
    let hov = ui.hit(back, Action::Launcher(Act::PanelDevices), "Back to the device rack");
    ui.c.fill_rrect(back, 2.0, if hov { t.s(0x2b3134) } else { t.s(0x222629) });
    ui.c.text(fnt(12.0), back.x + 6.0, back.y + 2.0, t.text, "\u{25c0} Devices");
    ui.c.text(fnt(13.0), back.right() + 10.0, r.y + 5.0, t.text, "Clip editor");
    let sel = s.launcher.sel.and_then(|(tr, sc)| launcher::clip(s, tr, sc).map(|sl| (tr, sc, sl)));
    match sel.filter(|(_, _, sl)| sl.audio.is_some()) {
        Some((tr, sc, slot)) => {
            let name = launcher::clip_name(s, tr, sc);
            ui.c.text_clip(fnt(13.0), back.right() + 96.0, r.y + 5.0, 240.0, t.accent, &name);
            let sample = slot
                .audio
                .and_then(|ch| s.project.channels.iter().find(|c| c.index == ch))
                .and_then(|c| c.sample_path.clone())
                .map(|p| {
                    std::path::Path::new(&p)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or(p)
                })
                .unwrap_or_default();
            let sw = ui.c.text_width(fnt(12.0), &sample);
            ui.c.text_clip(fnt(12.0), r.right() - 8.0 - sw.min(r.w * 0.4), r.y + 6.0, r.w * 0.4, t.text_dim, &sample);
            // The sample across the panel's body, the peaks a cell draws, larger.
            let body = Rect::new(r.x + 8.0, r.y + 34.0, r.w - 16.0, r.h - 42.0);
            ui.c.fill_rrect(body, 3.0, t.s(0x101315));
            audio_paint(ui, s, &slot, body, t.s(0x9fc5dc));
        }
        None => {
            ui.c.text(fnt(13.0), back.right() + 96.0, r.y + 5.0, t.text_dim, "Double-click an audio clip to open it here");
        }
    }
}

/// A cell holding a sample draws the sample behind its name — its peaks, one column per 2 px — so an audio clip is not
/// just a title (`launcher::add_audio`).
fn audio_paint(ui: &mut Ui, s: &State, slot: &launcher::Slot, cell: Rect, col: Color) {
    let Some(w) = slot
        .audio
        .and_then(|ch| s.project.channels.iter().find(|c| c.index == ch))
        .and_then(|c| c.sample_path.clone())
        .and_then(|path| s.playlist.wave(&path))
    else {
        return;
    };
    let n = w.peaks.len();
    if n == 0 || cell.w <= 8.0 {
        return;
    }
    let amp = (cell.h * 0.32).max(1.5);
    let mid = cell.cy();
    let mut x = cell.x + 3.0;
    while x < cell.right() - 3.0 {
        let at = |x: f32| (((x - cell.x) / cell.w).clamp(0.0, 0.999) * n as f32) as usize;
        let (a, b) = (at(x), at(x + 2.0).max(at(x) + 1).min(n));
        let (lo, hi) = w.peaks[a..b].iter().fold((f32::MAX, f32::MIN), |(lo, hi), &(p, q)| (lo.min(p), hi.max(q)));
        if lo <= hi {
            let (lo, hi) = (lo.clamp(-1.0, 1.0), hi.clamp(-1.0, 1.0));
            ui.c.line(x, mid - hi * amp, x, mid - lo * amp, 2.0, col);
        }
        x += 2.0;
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
