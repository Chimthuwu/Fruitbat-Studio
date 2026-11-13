// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Settings > Theme previews: a miniature of the main window (menu bar, toolbar, browser, a playlist and a floating
//! channel rack) drawn from a palette's tokens in its skin's shapes, for the Skins and Themes tiles.
// spec: ui/theme-presets.md "Settings"

use os_theme::Palette;
use os_ui::{Canvas, Color, Rect};

/// Size the miniature is laid out at; `paint` scales it to its rect's width.
pub const W: f32 = 182.0;
pub const H: f32 = 96.0;

/// Draw the miniature of `p` scaled to `r`'s width (`r` is `W` x `H` in shape).
pub fn paint(c: &mut Canvas, p: &Palette, r: Rect) {
    let k = r.w / W;
    let old = c.rescale(c.scale * k);
    layout(c, p, Rect::new(r.x / k, r.y / k, W, r.h / k));
    c.unscale(old);
}

fn layout(c: &mut Canvas, p: &Palette, r: Rect) {
    if p.style.meridian() {
        meridian_layout(c, p, r);
        return;
    }
    let lemon = p.style.lemon();
    // The skin's corners at preview size.
    let rad = p.style.radius * 0.5;
    let rad_small = p.style.radius_small * 0.4;
    let rad_panel = p.style.radius_panel * 0.4;
    let bar = |c: &mut Canvas, x: f32, y: f32, w: f32, col: Color| c.fill_rect(Rect::new(x, y, w, 1.5), col);
    c.push_clip(r);
    c.fill_rect(r, p.workspace);
    // Watermark: a soft disc in the workspace.
    c.fill_circle(r.right() - 30.0, r.bottom() - 18.0, 11.0, p.watermark);

    // Menu bar with its items.
    let menu = Rect::new(r.x, r.y, r.w, 7.0);
    c.fill_rect(menu, p.menu_panel);
    let mut x = r.x + 4.0;
    for w in [8.0, 7.0, 10.0, 9.0, 8.0, 11.0] {
        bar(c, x, menu.y + 3.0, w, p.menu_text);
        x += w + 4.0;
    }

    // Toolbar: the PAT/SONG pill, transport buttons (play lit), tempo, time display, record and an LED.
    let tb = Rect::new(r.x, menu.bottom(), r.w, 13.0);
    c.fill_rect(tb, p.toolbar);
    c.fill_rect(Rect::new(tb.x, tb.bottom() - 1.0, tb.w, 1.0), p.toolbar_line);
    let by = tb.y + 3.0;
    c.fill_rrect(Rect::new(tb.x + 4.0, by, 13.0, 7.0), rad, p.pill);
    for i in 0..4 {
        let b = Rect::new(tb.x + 21.0 + i as f32 * 9.0, by, 7.0, 7.0);
        if i == 0 {
            c.fill_rect_gradient(b, p.on_top, p.on_bottom, rad);
            c.fill_rect(Rect::new(b.x + 2.5, b.y + 2.0, 2.0, 3.0), p.on_glyph);
        } else {
            c.fill_rrect(b, rad, p.button);
            c.fill_rect(Rect::new(b.x + 2.0, b.y + 3.0, 3.0, 1.0), if i == 2 { p.record } else { p.glyph });
        }
    }
    let tempo = Rect::new(tb.x + 60.0, by, 18.0, 7.0);
    c.fill_rrect(tempo, rad, p.tempo_box);
    bar(c, tempo.x + 3.0, tempo.y + 3.0, 12.0, p.text);
    let time = Rect::new(tb.x + 82.0, by, 26.0, 7.0);
    c.fill_rrect(time, rad, p.time_panel);
    bar(c, time.x + 3.0, time.y + 3.0, 18.0, p.accent);
    c.fill_circle(tb.x + 114.0, tb.cy(), 2.0, p.accent);
    c.fill_circle(tb.x + 120.0, tb.cy(), 2.0, p.accent_dim);
    let cpu = Rect::new(tb.right() - 30.0, by, 26.0, 7.0);
    c.fill_rrect(cpu, rad, p.sunken_dark);
    for i in 0..5 {
        c.fill_rect(Rect::new(cpu.x + 3.0 + i as f32 * 4.0, cpu.y + 2.0 + (i % 3) as f32, 2.0, 5.0 - (i % 3) as f32), p.meter_low);
    }

    // Browser: tab strip, category rows with their colours, folders, one highlighted row.
    let br = Rect::new(r.x, tb.bottom(), 44.0, r.bottom() - tb.bottom());
    c.fill_rect(br, p.browser_list);
    c.fill_rect(Rect::new(br.x, br.y, br.w, 5.0), p.browser_tabs);
    c.fill_rect(Rect::new(br.x + 3.0, br.y + 1.0, 12.0, 4.0), p.browser_title);
    c.fill_rect(Rect::new(br.right() - 1.0, br.y, 1.0, br.h), p.splitter_dark);
    let cats = [p.cat_project, p.cat_recent, p.cat_plugin_db, p.cat_plugin_presets, p.cat_channel_presets, p.cat_mixer_presets];
    for i in 0..9 {
        let y = br.y + 8.0 + i as f32 * 7.0;
        if y + 5.0 > br.bottom() {
            break;
        }
        if i == 7 {
            c.fill_rect(Rect::new(br.x, y - 1.5, br.w - 1.0, 6.0), p.browser_row_hi);
        }
        let (dot, w) = match cats.get(i) {
            Some(col) => (*col, [22.0, 18.0, 25.0, 20.0, 24.0, 19.0][i]),
            None => (p.browser_folder, [16.0, 21.0, 14.0][i - cats.len()]),
        };
        c.fill_rrect(Rect::new(br.x + 3.0, y, 3.0, 3.0), if lemon { 1.0 } else { 0.0 }, dot);
        bar(c, br.x + 9.0, y + 1.0, w, if i < cats.len() { dot } else { p.browser_folder });
    }

    // Playlist window: title, track headers, grid with bar groups, clips, the playhead.
    let pl = Rect::new(br.right() + 4.0, tb.bottom() + 4.0, r.right() - br.right() - 8.0, 50.0);
    window(c, p, pl, rad_panel);
    let grid = Rect::new(pl.x + 23.0, pl.y + 6.0, pl.w - 24.0, pl.h - 7.0);
    let rows = 5;
    let row_h = grid.h / rows as f32;
    let bar_w = grid.w / 8.0;
    for i in 0..rows {
        let y = grid.y + i as f32 * row_h;
        c.fill_rect(Rect::new(pl.x + 1.0, y, 21.0, row_h - 1.0), p.track_header);
        bar(c, pl.x + 4.0, y + row_h / 2.0 - 1.0, 11.0, p.text_faint);
        for b in 0..8 {
            let col = if (b / 4) % 2 == 1 { p.grid_alt } else { p.grid };
            // Under the row line too: a theme's line strength may make it see-through.
            c.fill_rect(Rect::new(grid.x + b as f32 * bar_w, y, bar_w, row_h), col);
        }
        c.fill_rect(Rect::new(grid.x, y + row_h - 1.0, grid.w, 1.0), p.pl_line(p.grid_gap));
    }
    for b in 1..8 {
        c.fill_rect(Rect::new(grid.x + b as f32 * bar_w, grid.y, 1.0, grid.h), p.pl_line(if b % 4 == 0 { p.grid_bar } else { p.grid_gap }));
    }
    for (row, from, len) in [(0, 0.0, 4.0), (1, 2.0, 2.0), (1, 4.0, 3.0), (2, 1.0, 6.0), (3, 4.0, 4.0)] {
        let clip = Rect::new(grid.x + from * bar_w + 0.5, grid.y + row as f32 * row_h + 0.5, len * bar_w - 1.0, row_h - 2.0);
        c.fill_rrect(clip, rad_small, p.clip.shade(-0.25));
        c.fill_rrect(Rect::new(clip.x, clip.y, clip.w, 2.5), rad_small, p.clip.shade(0.3));
        c.fill_rect(Rect::new(clip.x + 1.0, clip.y + 3.5, clip.w - 2.0, clip.h - 4.5), p.clip);
    }
    c.fill_rect(Rect::new(grid.x + bar_w * 3.4, grid.y, 1.0, grid.h), p.playhead);

    // Channel rack: rows of name buttons and steps (groups of four, a few lit), over the playlist's corner.
    let rk = Rect::new(br.right() + 12.0, pl.bottom() - 6.0, 106.0, r.bottom() - pl.bottom() + 4.0);
    window(c, p, rk, rad_panel);
    let lit = [0b1000_1000_1000_1000u16, 0b0010_0000_0010_0000, 0b0000_1000_0000_1001];
    for (i, row) in lit.iter().enumerate() {
        let y = rk.y + 8.0 + i as f32 * 8.0;
        if y + 6.0 > rk.bottom() {
            break;
        }
        c.fill_rrect(Rect::new(rk.x + 4.0, y, 9.0, 6.0), rad_small, p.rack_row_box);
        c.fill_rrect(Rect::new(rk.x + 15.0, y, 20.0, 6.0), rad_small, if i == 0 { p.name_button_sel } else { p.name_button });
        bar(c, rk.x + 17.0, y + 2.5, 12.0, p.name_text);
        for st in 0..16 {
            let on = row & (1 << (15 - st)) != 0;
            let face = if on { p.step_on } else if (st / 4) % 2 == 1 { p.step_b } else { p.step_a };
            c.fill_rrect(Rect::new(rk.x + 38.0 + st as f32 * 4.0, y, 3.2, 6.0), rad_small * 0.6, face);
        }
    }
    c.pop_clip();
}

/// Meridian's miniature: an editorial masthead, separate transport plates, a typographic browser rail, squared clips
/// with datum rails and a full-width device rack. It reads as a different instrument from the other skins even at this
/// scale, while its colours still come entirely from `p` (so themes recolour it).
fn meridian_layout(c: &mut Canvas, p: &Palette, r: Rect) {
    let bar = |c: &mut Canvas, x: f32, y: f32, w: f32, h: f32, col: Color| c.fill_rect(Rect::new(x, y, w, h), col);
    c.push_clip(r);
    c.fill_rect(r, p.workspace);
    // Masthead: menu rail and a thin brass datum.
    bar(c, r.x, r.y, r.w, 6.0, p.menu_panel);
    bar(c, r.x, r.y + 5.0, r.w, 1.0, p.accent_dim);
    for i in 0..6 {
        bar(c, r.x + 4.0 + i as f32 * 10.0, r.y + 2.0, 6.0, 1.5, p.menu_text);
    }
    // Header: three separate plates, not one capsule.
    let hd = Rect::new(r.x, r.y + 7.0, r.w, 16.0);
    c.fill_rect(hd, p.toolbar);
    bar(c, hd.x, hd.bottom() - 2.0, hd.w, 1.0, p.accent_dim);
    let plate = |c: &mut Canvas, x: f32, w: f32| {
        c.fill_rect(Rect::new(x, hd.y + 3.0, w, 10.0), p.sunken);
        bar(c, x, hd.y + 3.0, w, 1.0, p.toolbar_edge);
    };
    plate(c, r.x + 7.0, 16.0);
    c.fill_rect(Rect::new(r.x + 9.0, hd.y + 6.0, 6.0, 4.0), p.pill);
    plate(c, r.x + 25.0, 29.0);
    c.fill_rect(Rect::new(r.x + 29.0, hd.y + 5.0, 5.0, 6.0), p.accent);
    c.fill_rect(Rect::new(r.x + 38.0, hd.y + 5.0, 5.0, 6.0), p.button_hi);
    // Tempo: brass left edge and a rule under the digits.
    let tempo = Rect::new(r.x + 59.0, hd.y + 3.0, 17.0, 10.0);
    c.fill_rect(tempo, p.time_panel);
    c.fill_rect(Rect::new(tempo.x, tempo.y, 2.0, tempo.h), p.accent);
    bar(c, tempo.x + 4.0, tempo.y + 3.0, 10.0, 2.0, p.accent);
    // Song position: a surveyor's rule.
    let pos = Rect::new(r.x + 81.0, hd.y + 6.0, 26.0, 4.0);
    c.fill_rect(pos, p.sunken_dark);
    c.fill_rect(Rect::new(pos.x, pos.y, 15.0, 2.0), p.accent);
    c.fill_rect(Rect::new(pos.x + 14.0, pos.y - 2.0, 3.0, 7.0), p.accent);
    // Short utility ticks.
    for i in 0..4 {
        c.fill_rect(Rect::new(r.x + 114.0 + i as f32 * 5.0, hd.y + 5.0, 3.0, 5.0), p.button);
    }
    // Browser rail: typographic tabs with one brass datum, flat rows and category ticks.
    let br = Rect::new(r.x, hd.bottom(), 39.0, r.bottom() - hd.bottom() - 22.0);
    c.fill_rect(br, p.browser_list);
    c.fill_rect(Rect::new(br.x, br.y, br.w, 7.0), p.browser_tabs);
    c.fill_rect(Rect::new(br.x + 4.0, br.y + 6.0, 7.0, 2.0), p.accent);
    for i in 0..4 {
        c.fill_rect(Rect::new(br.x + 4.0 + i as f32 * 8.0, br.y + 1.0, 5.0, 2.0), p.text_faint);
    }
    c.fill_rect(Rect::new(br.right() - 1.0, br.y, 1.0, br.h), p.splitter_dark);
    let cats = [p.cat_project, p.cat_recent, p.cat_plugin_db, p.cat_plugin_presets, p.cat_channel_presets, p.cat_mixer_presets];
    for i in 0..8 {
        let y = br.y + 11.0 + i as f32 * 7.0;
        if y + 4.0 > br.bottom() {
            break;
        }
        let col = cats.get(i).copied().unwrap_or(p.browser_folder);
        c.fill_rect(Rect::new(br.x + 3.0, y, 2.0, 2.0), col);
        bar(c, br.x + 8.0, y, [19.0, 14.0, 22.0, 17.0, 20.0, 15.0, 18.0, 12.0][i], 1.5, if i < cats.len() { col } else { p.browser_folder });
        if i == 5 {
            c.fill_rect(Rect::new(br.x, y - 1.0, br.w - 1.0, 4.0), p.browser_row_hi);
        }
    }
    // Playlist: squared window, ruler, clips with datum rails, head band and playhead.
    let pl = Rect::new(br.right() + 3.0, hd.bottom() + 3.0, r.right() - br.right() - 6.0, 45.0);
    c.fill_rect(pl, p.window_title);
    c.fill_rect(Rect::new(pl.x, pl.y, pl.w, 2.0), p.accent);
    c.fill_rect(pl.inset(1.0), p.window_body);
    let ruler = Rect::new(pl.x + 17.0, pl.y + 4.0, pl.w - 18.0, 4.0);
    c.fill_rect(ruler, p.ruler);
    for i in 0..8 {
        c.fill_rect(Rect::new(ruler.x + i as f32 * ruler.w / 8.0, ruler.y + 2.0, 1.0, 4.0), p.ruler_text);
    }
    let grid = Rect::new(ruler.x, ruler.bottom(), ruler.w, pl.bottom() - ruler.bottom() - 1.0);
    let row_h = grid.h / 4.0;
    let bar_w = grid.w / 8.0;
    for i in 0..4 {
        let y = grid.y + i as f32 * row_h;
        c.fill_rect(Rect::new(pl.x + 1.0, y, 16.0, row_h - 1.0), p.track_header);
        c.fill_rect(Rect::new(pl.x + 2.0, y, 2.0, row_h - 1.0), p.signal.with_alpha(150));
        bar(c, pl.x + 6.0, y + row_h / 2.0, 8.0, 1.5, p.text_faint);
        for b in 0..8 {
            c.fill_rect(Rect::new(grid.x + b as f32 * bar_w, y, bar_w, row_h), if (b / 4) % 2 == 1 { p.grid_alt } else { p.grid });
        }
    }
    for (row, from, len) in [(0, 0.0, 3.0), (1, 2.0, 2.0), (2, 1.0, 4.0), (3, 4.0, 2.0)] {
        let clip = Rect::new(grid.x + from * bar_w + 0.5, grid.y + row as f32 * row_h + 0.5, len * bar_w - 1.0, row_h - 2.0);
        c.fill_rect(clip, p.clip.shade(-0.25));
        c.fill_rect(Rect::new(clip.x, clip.y, clip.w, 3.0), p.clip);
        c.fill_rect(Rect::new(clip.x, clip.y, 2.0, clip.h), p.clip.shade(0.25));
    }
    c.fill_rect(Rect::new(grid.x + bar_w * 2.6, grid.y, 1.0, grid.h), p.playhead);
    // Device rack: one full-width instrument strip under the playlist, with family spines and squared wells.
    let rk = Rect::new(br.right() + 3.0, pl.bottom() + 2.0, pl.w, r.bottom() - pl.bottom() - 4.0);
    c.fill_rect(rk, p.rack_inset);
    c.fill_rect(Rect::new(rk.x, rk.y, 2.0, rk.h), p.accent_dim);
    for i in 0..3 {
        let x = rk.x + 5.0 + i as f32 * 30.0;
        c.fill_rect(Rect::new(x, rk.y + 2.0, 26.0, rk.h - 4.0), p.mixer_strip);
        c.fill_rect(Rect::new(x, rk.y + 2.0, 3.0, rk.h - 4.0), [p.signal, p.modulation, p.warning][i]);
        for k in 0..3 {
            c.fill_rect(Rect::new(x + 5.0 + k as f32 * 7.0, rk.y + 5.0, 5.0, 3.0), p.slot);
        }
    }
    c.pop_clip();
}

/// A floating window: border, body and title bar with its caption.
fn window(c: &mut Canvas, p: &Palette, r: Rect, rad: f32) {
    c.fill_rrect(r, rad, p.window_border);
    let inner = r.inset(1.0);
    c.fill_rrect(inner, rad, p.window_body);
    c.fill_rrect(Rect::new(inner.x, inner.y, inner.w, 5.0), rad, p.window_title);
    c.fill_rect(Rect::new(inner.x, inner.y + 3.0, inner.w, 2.0), p.window_title);
    c.fill_rect(Rect::new(inner.x + 4.0, inner.y + 2.0, 18.0, 1.5), p.text);
}

#[cfg(test)]
mod tests {
    use super::*;
    use os_theme::Design;

    /// Every skin's preview renders, under its own colours and under a real theme. Meridian's miniature is a distinct
    /// composition, not just the Lemon preview in brass.
    #[test]
    fn every_skin_and_a_theme_render() {
        let th = os_theme::FlTheme::parse(os_theme::presets::PRESETS[0].1);
        for d in Design::ALL {
            let mut own = Canvas::new(W as u32, H as u32, 1.0).expect("a preview canvas");
            paint(&mut own, &os_theme::design::palette(d, None), Rect::new(0.0, 0.0, W, H));
            let mut themed = Canvas::new(W as u32, H as u32, 1.0).expect("a preview canvas");
            paint(&mut themed, &os_theme::design::palette(d, Some(&th)), Rect::new(0.0, 0.0, W, H));
        }
        let meridian = os_theme::design::palette(Design::Meridian, None);
        let lemon = os_theme::design::palette(Design::Lemon, None);
        assert_ne!(meridian.toolbar, lemon.toolbar);
        assert!(meridian.style.meridian() && lemon.style.lemon());
    }
}
