// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The Channel Rack's editor (spec: ui/channel-rack.md): vertical sound lanes, horizontal programmable steps, the lane
//! names and the playback position, over the project's current pattern. It is the container window's content for a rack
//! channel (`crate::channel_rack`), so the instrument keeps one window with the rest of the device, and every step is a
//! real note of the pattern the Arrangement and the Clip Launcher play.

use os_ui::Rect;

use super::{fnt, FrameInfo};
use crate::app::{Action, State, Ui, Win};
use crate::channel_rack::Lane;
use crate::container::CAct;
use crate::icons::{self, Icon};
use crate::plugins::Plugins;

/// The editor's size: sixteen lanes and steps, the pattern's controls above them.
pub const W: f32 = 664.0;
pub const H: f32 = 322.0;
const TITLE_H: f32 = 26.0;
const ROW_H: f32 = 18.0;
const HEAD_H: f32 = 13.0;
const LANE_W: f32 = 172.0;

/// Paints the rack in `win` (the container window's rect).
pub fn paint(ui: &mut Ui, s: &State, plugins: &Plugins, row: usize, win: Rect, f: &FrameInfo) {
    let t = ui.t;
    let meridian = t.style.meridian();
    let (x0, y0) = (win.x, win.y);
    ui.c.fill_rect(win, if meridian { t.s(0x10171a) } else { t.s(0x16191a) });
    ui.c.fill_rect(win.inset(1.0), if meridian { t.s(0x1b2329) } else { t.s(0x2a2f31) });
    ui.hit(win, Action::Swallow, "");
    let name = crate::rack::channel_name(s, row);
    ui.hit(Rect::new(x0 + 1.0, y0 + 1.0, win.w - 30.0, TITLE_H - 4.0), Action::WinDrag(Win::Container), "Drag to move");
    ui.c.text(fnt(15.0), x0 + 9.0, y0 + 6.0, t.text, &format!("Channel Rack - {name}"));
    let close = Rect::new(x0 + win.w - 20.0, y0 + 9.0, 10.0, 10.0);
    let hov = super::close_button(ui, close.outset(4.0), close.outset(3.0), Action::CloseWin(Win::Container), "Close");
    icons::draw(ui.c, Icon::Close, close.outset(1.0), if hov { t.tx(0xffffff) } else { t.glyph });

    let ch = s.project.channels.get(row).map(|c| c.index);
    let lanes = crate::channel_rack::lanes(s, plugins, row);
    let per = crate::rack::per_step(s);
    let total = crate::rack::pattern_steps(s).max(1);
    let beat = crate::rack::beat_steps(s).max(1);

    // The pattern's row: navigation and identity, the length in bars, the step page, the bank, open and clear.
    let bar = Rect::new(x0 + 6.0, y0 + TITLE_H + 2.0, (win.w - 12.0).max(1.0), 20.0);
    let mut x = bar.x;
    for (label, d) in [("\u{2039}", -1), ("\u{203a}", 1)] {
        let b = Rect::new(x, bar.y, 18.0, 18.0);
        let hv = ui.hit(b, Action::Container(CAct::RackPattern(d)), "Previous / next pattern (every clip plays the same pattern)");
        ui.c.fill_rrect(b, 2.0, if hv { if meridian { t.s(0x33403f) } else { t.s(0x3a4347) } } else { if meridian { t.s(0x222c31) } else { t.s(0x202628) } });
        ui.c.text_center(fnt(12.0), b, t.text, label);
        x += 20.0;
    }
    let pname = format!("P{}  {}", s.pattern, crate::patterns::name_of(s, s.pattern));
    ui.c.text_clip(fnt(11.0), x + 3.0, bar.y + 3.0, 160.0, t.text, &pname);
    // Length in bars (Auto when the pattern sets none).
    let bars = crate::rack::explicit_len(s).map_or(0, |n| n.div_ceil(crate::rack::bar_steps(s).max(1)));
    let mut lx = bar.right() - 300.0;
    for (label, d) in [("\u{2212}", -1), ("+", 1)] {
        let b = Rect::new(lx, bar.y, 18.0, 18.0);
        let hv = ui.hit(b, Action::Container(CAct::RackLength(bars as i32 + d)), "Pattern length in bars (0 = Auto)");
        ui.c.fill_rrect(b, 2.0, if hv { if meridian { t.s(0x33403f) } else { t.s(0x3a4347) } } else { if meridian { t.s(0x222c31) } else { t.s(0x202628) } });
        ui.c.text_center(fnt(12.0), b, t.text, label);
        lx += 20.0;
    }
    let len = if bars == 0 { "Auto".to_string() } else { format!("{bars} bar{}", if bars == 1 { "" } else { "s" }) };
    ui.c.text_clip(fnt(10.0), lx, bar.y + 4.0, 52.0, t.text_dim, &len);
    // The step page (sixteen steps of the pattern at a time).
    let pages = total.div_ceil(crate::channel_rack::STEPS).max(1);
    let page = s.container.page.min(pages.saturating_sub(1));
    let mut px = bar.right() - 190.0;
    ui.c.text_clip(fnt(9.0), px, bar.y + 4.0, 34.0, t.text_dim, &format!("{}/{}", page + 1, pages));
    px += 34.0;
    for (label, np, ok) in [("\u{2039}", page.saturating_sub(1), page > 0), ("\u{203a}", (page + 1).min(pages - 1), page + 1 < pages)] {
        let b = Rect::new(px, bar.y, 16.0, 18.0);
        let hv = ok && ui.hit(b, Action::Container(CAct::RackPage(np as i32)), "Previous / next sixteen steps");
        ui.c.fill_rrect(b, 2.0, if hv { if meridian { t.s(0x33403f) } else { t.s(0x3a4347) } } else { if meridian { t.s(0x222c31) } else { t.s(0x202628) } });
        ui.c.text_center(fnt(10.0), b, if ok { t.text } else { t.text_faint }, label);
        px += 17.0;
    }
    // The bank of lanes (the sampler's pads come in two banks of sixteen).
    let bank = s.container.bank.min(1);
    let bk = Rect::new(bar.right() - 128.0, bar.y, 22.0, 18.0);
    let bh = ui.hit(bk, Action::Container(CAct::RackBank), "Switch lane bank A / B");
    ui.c.fill_rrect(bk, 2.0, if bh { if meridian { t.s(0x33403f) } else { t.s(0x3a4347) } } else { if meridian { t.s(0x222c31) } else { t.s(0x202628) } });
    ui.c.text_center(fnt(10.0), bk, t.text, if bank == 0 { "A" } else { "B" });
    // Open the lane's sampler window (sample assignment, pad settings) and clear this channel's steps.
    let open = Rect::new(bar.right() - 100.0, bar.y, 44.0, 18.0);
    let oh = ui.hit(open, Action::Container(CAct::RackOpen), "Open the lanes' sampler: load each lane's sample");
    ui.c.fill_rrect(open, 2.0, if oh { if meridian { t.s(0x33403f) } else { t.s(0x3a4347) } } else { if meridian { t.s(0x222c31) } else { t.s(0x202628) } });
    ui.c.text_center(fnt(9.0), open, if oh { t.text } else { t.text_dim }, "LANES");
    let clear = Rect::new(bar.right() - 50.0, bar.y, 50.0, 18.0);
    let chh = ui.hit(clear, Action::Container(CAct::RackClear), "Clear this rack's steps in the current pattern");
    ui.c.fill_rrect(clear, 2.0, if chh { t.s(0x4a3030) } else { t.s(0x2a2020) });
    ui.c.text_center(fnt(9.0), clear, if chh { t.text } else { t.text_dim }, "CLEAR");

    // The grid: the step numbers and beat divisions, then one row per lane.
    let grid = Rect::new(x0 + 6.0, bar.bottom() + 3.0, (win.w - 12.0).max(1.0), (win.bottom() - bar.bottom() - 9.0).max(1.0));
    let cols_x = grid.x + LANE_W;
    let step_w = ((grid.right() - cols_x) / crate::channel_rack::STEPS as f32).max(4.0);
    let first = page * crate::channel_rack::STEPS;
    let cur = if f.playing {
        let tick = if f.song { crate::piano::pattern_tick_in_song(s, f.tick) } else { Some(f.tick) };
        tick.map(|tk| ((tk.max(0.0) as u32) / per) % total)
    } else {
        None
    };
    for i in 0..crate::channel_rack::STEPS {
        let step = first + i;
        let sx = cols_x + i as f32 * step_w;
        let bg = if step >= total {
            if meridian { t.s(0x13191c) } else { t.s(0x171b1d) }
        } else if (step / beat) % 2 == 0 {
            if meridian { t.s(0x1c2429) } else { t.s(0x1d2325) }
        } else {
            if meridian { t.s(0x161d21) } else { t.s(0x181d1f) }
        };
        ui.c.fill_rect(Rect::new(sx, grid.y, step_w, grid.h), bg);
        if step < total {
            ui.c.text_center(fnt(8.0), Rect::new(sx, grid.y, step_w, HEAD_H), t.text_faint, &(step + 1).to_string());
        }
        if i % beat == 0 {
            ui.c.fill_rect(Rect::new(sx, grid.y, 1.0, grid.h), if meridian { t.s(0x4a3c22) } else { t.s(0x3d474b) });
        }
    }
    let row_h = ((grid.h - HEAD_H) / crate::channel_rack::LANES as f32).max(5.0);
    for i in 0..crate::channel_rack::LANES {
        let at = bank * crate::channel_rack::LANES + i;
        let y = grid.y + HEAD_H + i as f32 * row_h;
        let lane: Option<&Lane> = lanes.get(at);
        let selected = at == s.container.lane;
        if selected {
            ui.c.fill_rect(Rect::new(grid.x, y, grid.w, row_h), t.accent.with_alpha(18));
        }
        let label = Rect::new(grid.x, y, LANE_W - 2.0, row_h);
        let lh = ui.hit(label, Action::Container(CAct::RackLane(at)), "Select this lane; its steps are this instrument's notes");
        if lh || selected {
            ui.c.fill_rect(label, if meridian { t.s(0x22352e) } else { t.s(0x25302c) });
        }
        let (nm, ky, loaded) = lane.map_or((format!("Lane {}", at + 1), 0u16, false), |l| (l.name.clone(), l.key, l.sample));
        let ink = if !loaded { t.text_faint } else if selected { t.text } else { t.text_dim };
        ui.c.text_clip(fnt(9.5), label.x + 5.0, y + (row_h - 11.0) * 0.5, label.w - 34.0, ink, &nm);
        if loaded {
            ui.c.text(fnt(8.0), label.right() - 27.0, y + (row_h - 9.0) * 0.5, t.text_faint, &ky.to_string());
        }
        if i + 1 < crate::channel_rack::LANES {
            ui.c.fill_rect(Rect::new(grid.x, y + row_h, grid.w, 1.0), t.s(0x101416));
        }
        let (Some(ch), Some(lane)) = (ch, lane) else { continue };
        for i_step in 0..crate::channel_rack::STEPS {
            let step = first + i_step;
            if step >= total {
                continue;
            }
            let sx = cols_x + i_step as f32 * step_w;
            let on = crate::channel_rack::step_on(s, ch, lane, step);
            let cell = Rect::new(sx + 1.0, y + 1.0, (step_w - 2.0).max(1.0), (row_h - 2.0).max(1.0));
            let hv = ui.hit(
                cell,
                Action::Container(CAct::RackStep { pad: at, step, on: !on }),
                if on { "Turn this step off" } else { "Turn this step on" },
            );
            let fill = if on {
                if selected { t.accent } else { t.accent.with_alpha(205) }
            } else if hv {
                if meridian { t.s(0x33403f) } else { t.s(0x3b464a) }
            } else if loaded {
                if meridian { t.s(0x232c31) } else { t.s(0x262e31) }
            } else {
                if meridian { t.s(0x1a2125) } else { t.s(0x1e2426) }
            };
            ui.c.fill_rrect(cell, if meridian { 0.0 } else { 1.5 }, fill);
            if on {
                ui.c.fill_rect(Rect::new(cell.x + 1.0, cell.y + 1.0, (cell.w - 2.0).max(1.0), 1.0), if meridian { t.s(0xe8dcc0) } else { t.s(0xd8ff9a) });
            }
        }
    }
    // The playhead: the engine's own position, drawn over the columns.
    if let Some(k) = cur.filter(|k| (*k >= first) && (*k < first + crate::channel_rack::STEPS)) {
        let sx = cols_x + (k - first) as f32 * step_w;
        ui.c.fill_rect(Rect::new(sx, grid.y + HEAD_H, step_w, grid.h - HEAD_H), if meridian { t.accent.with_alpha(26) } else { t.s(0x00d5c0).with_alpha(34) });
        ui.c.fill_rect(Rect::new(sx, grid.y + HEAD_H, step_w, 2.0), if meridian { t.accent } else { t.s(0x00d5c0) });
    }
    // Meridian: a precision-instrument matrix — brass structural rules frame the editor.
    if meridian {
        ui.c.fill_rect(Rect::new(win.x + 1.0, y0 + TITLE_H, 2.0, win.h - TITLE_H - 2.0), t.accent.with_alpha(110));
        ui.c.fill_rect(Rect::new(x0, y0 + TITLE_H, win.w, 1.0), t.accent.with_alpha(80));
    }
}
