// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Mixer window (wide layout: strips plus the selected track's slot panel).
// spec: ui/main-window.md §7 (layout), ui/mixer.md (controls, hints, routing look); fader law: audio/gain-laws.md

use std::f32::consts::PI;

use os_ui::kit::lemon as lk;
use os_ui::{Color, Font, Glyph, Rect};

use super::device_rack::{mod_arcs, mod_fader};
use super::{led, resize_zones, window_body, window_buttons, FrameInfo};
use crate::knobs::KnobId;
use crate::app::{Action, Live, State, Ui, Win, TITLE_H};
use crate::meters;
use crate::mixer::{self as mx, MHit};
use crate::icons::{self, Icon};

pub const STRIP_W: f32 = 44.0;
/// The gap a track's Separator puts before its strip (measured: 6 px).
pub const SEP_W: f32 = 6.0;
/// The gap after the master strip (and after the left-docked strips).
const MASTER_GAP: f32 = 9.0;
/// From the strips' left edge: the master strip's left edge (the "C" level-scale column comes before it).
const MASTER_X: f32 = 44.0;

/// Where the inserts' strips are (spec: ui/mixer.md §6.1 "Group, Layout and CPU entries"). Per part (docked left,
/// middle, right) the tracks in track order, each with its x from the part's start and whether a separator's gap is
/// before it; and each part's width. A separator on a part's first strip adds no gap (Lemon Studio's choice).
pub struct Docks {
    pub parts: [Vec<(u16, f32, bool)>; 3],
    pub widths: [f32; 3],
    /// The strips' width, the chosen layout's (`mx::strip_w`; Settings ▸ Mixer ▸ layout, in the mixer's title bar).
    pub width: f32,
}

/// The inserts' display order (spec: model/track-model.md §5 "Mixer reorders to track order"): Classic mode in index
/// order, as it always was. Linear mode, the project's [`os_model::Track`]s in their order — each naming its strip —
/// then the inserts no track names, in index order. View ordering only: the mixer's logic, each strip's dock part and
/// its separator are untouched (`crate::mixer` stays as it is).
pub fn strip_order(s: &State) -> Vec<u16> {
    let n = mx::inserts(s);
    if !s.project.linear_mode {
        return (1..=n).collect();
    }
    let mut v: Vec<u16> = Vec::new();
    for m in s.project.tracks.iter().map(|t| t.mixer_strip) {
        if m >= 1 && m <= n && !v.contains(&m) {
            v.push(m);
        }
    }
    v.extend((1..=n).filter(|m| !v.contains(m)).collect::<Vec<_>>());
    v
}

pub fn docks(s: &State) -> Docks {
    let mut d = Docks { parts: Default::default(), widths: [0.0; 3], width: mx::strip_w(s) };
    for n in strip_order(s) {
        let (dock, sep) = mx::track(s, n).map_or((os_model::DOCK_MIDDLE, false), |m| (m.dock.min(os_model::DOCK_RIGHT), m.separator));
        let p = usize::from(dock);
        let gap = sep && !d.parts[p].is_empty();
        if gap {
            d.widths[p] += SEP_W;
        }
        d.parts[p].push((n, d.widths[p], gap));
        d.widths[p] += d.width;
    }
    d
}

impl Docks {
    /// The scrolling middle part: after the master, the left-docked strips and the gap, up to the right-docked strips.
    pub fn area(&self, strips: Rect) -> Rect {
        let x = strips.x + MASTER_X + self.width + MASTER_GAP + self.widths[0];
        Rect::new(x, strips.y, (strips.right() - self.widths[2] - x).max(0.0), strips.h)
    }

    /// Track `n`'s part (0 left, 1 middle, 2 right) and its x from the part's start.
    pub fn place(&self, n: u16) -> Option<(usize, f32)> {
        (0..3).find_map(|p| self.parts[p].iter().find(|e| e.0 == n).map(|e| (p, e.1)))
    }

    /// Track `n`'s strip's left edge (the master's too), the middle part scrolled by `scroll`.
    pub fn x(&self, n: u16, strips: Rect, scroll: f32) -> Option<f32> {
        if n == 0 {
            return Some(strips.x + 44.0);
        }
        let (p, off) = self.place(n)?;
        Some(match p {
            0 => strips.x + MASTER_X + self.width + off,
            1 => self.area(strips).x + off - scroll,
            _ => self.area(strips).right() + off,
        })
    }

    /// Whether track `n`'s strip scrolls (the middle part).
    pub fn scrolls(&self, n: u16) -> bool {
        self.place(n).is_some_and(|(p, _)| p == 1)
    }
}

/// A separator's gap before a strip at `left`: a lighter band with darker edges (measured: #282b2d, 1 px #1e2123).
fn separator(ui: &mut Ui, left: f32, top: f32, bottom: f32) {
    let r = Rect::new(left - SEP_W, top, SEP_W, bottom - top);
    ui.c.fill_rect(r, ui.t.s(0x1e2123));
    ui.c.fill_rect(r.inset(1.0), ui.t.s(0x282b2d));
}

/// Fader travel: value units per pixel (handle centre 618 at 0 dB = value 0.8, 970 at value 0).
pub const FADER_PER_PX: f64 = 1.0 / 440.0;
/// Fader change per wheel notch: 4 px of drag. Fitted to the one measurement (spec: ui/mixer.md §4): three notches up
/// from unity read `103%  0.8dB  1.09`, which puts a notch between 0.0088 and 0.0092.
pub const FADER_NOTCH: f64 = 4.0 * FADER_PER_PX;

// Vertical layout at the default 1080p window (spec §7): absolute y for a window top of 83;
// bottom-anchored values are offsets from the window bottom.
const HEAD_Y0: f32 = 107.0; // header numbers row
const HEAD_Y1: f32 = 127.0;
const METER_Y0: f32 = 130.0;
const METER_Y1: f32 = 453.0;
const PANEL_Y0: f32 = 128.0; // dark head panel behind meter, name, LED, pan
const PANEL_Y1: f32 = 508.0;
const LED_Y: f32 = 467.0;
const PAN_Y: f32 = 496.5;
const SEP_Y: f32 = 527.0;
const SWAP_Y: f32 = 545.0;
const RING_Y: f32 = 563.0;
const TRACK_Y0: f32 = 525.0;
const HANDLE_0DB: f32 = 618.0; // handle centre at 0 dB (value 0.8)

/// dB marks of the level scale and their measured y positions (number centres).
const SCALE: [(&str, f32); 17] = [
    ("3", 181.0), ("0", 241.0), ("3", 301.0), ("6", 360.0), ("9", 419.0), ("12", 479.0), ("15", 535.0), ("18", 588.0),
    ("21", 639.0), ("24", 686.0), ("27", 728.0), ("30", 766.0), ("33", 799.0), ("36", 826.0), ("39", 848.0), ("42", 866.0),
    ("45", 880.0),
];

/// The "C" column's marks as (label, reference y), top down: the default scale's `SCALE`, its first mark the "+3"; with
/// `alt` ("Alternate meter scale") 0, 3 … 30 at the heights the default law puts the levels `meters::ALTERNATE` maps
/// them to, with no +3 (spec: ui/settings-choices.md "Miscellaneous: seven more checks, Default template and Startup
/// project").
pub fn scale_marks(alt: bool, floor: f32) -> Vec<(&'static str, f32)> {
    if !alt {
        return SCALE.to_vec();
    }
    SCALE[1..].iter().zip(meters::ALTERNATE).map(|(&(m, _), (_, d))| (m, meters::law_y(d, floor, false))).collect()
}

/// Height of the mixer window the absolute y values above were measured at (1080p workspace).
const REF_H: f32 = 997.0;
/// Reference y of the window top.
const REF_TOP: f32 = 83.0;
/// Height of the parts that do not scale: header, head-panel controls and the bottom controls.
const FIXED_H: f32 = 218.0;

/// How much the mixer's stretchy parts (meter, level scale, fader) are scaled at window `r`:
/// 1 at the measured height; the rest of the strip keeps its size.
pub fn scale(r: Rect) -> f32 {
    ((r.h - FIXED_H) / (REF_H - FIXED_H)).max(0.2)
}

/// Maps the measured (absolute, window top 83) y values to the window `r`: the meter (130-453) and
/// the fader (525-981) scale, the controls between them and below them keep their size.
struct Ly {
    top: f32,
    k: f32,
}

impl Ly {
    fn new(r: Rect) -> Ly {
        Ly { top: r.y, k: scale(r) }
    }

    fn y(&self, abs: f32) -> f32 {
        let (m0, m1, f0, f1) = (130.0 - REF_TOP, 453.0 - REF_TOP, 525.0 - REF_TOP, 981.0 - REF_TOP);
        let ry = abs - REF_TOP;
        let k = self.k;
        let y = if ry < m0 {
            ry
        } else if ry < m1 {
            m0 + (ry - m0) * k
        } else if ry < f0 {
            m0 + (m1 - m0) * k + (ry - m1)
        } else if ry < f1 {
            m0 + (m1 - m0) * k + (f0 - m1) + (ry - f0) * k
        } else {
            m0 + (m1 - m0) * k + (f0 - m1) + (f1 - f0) * k + (ry - f1)
        };
        self.top + y
    }
}

pub fn fader_db_text(v: f64) -> String {
    let gain = os_engine::laws::fader_gain(v);
    if gain <= 0.0 { "-inf dB".into() } else { format!("{:+.1} dB", 20.0 * gain.log10()) }
}


pub fn paint(ui: &mut Ui, s: &State, r: Rect, f: &FrameInfo) {
    let panel_w = 221.0;
    window_body(ui, r);
    let outer = r;
    let inner = super::float_frame(ui, s, Win::Mixer, r);
    let r = inner.unwrap_or(r);
    let client = super::window_frame_ex(ui, Win::Mixer, Rect::new(r.x, r.y, r.w - panel_w, r.h), "", 0.0, false, inner.is_some());
    title_tools(ui, s, r);
    if r.h < 2.0 * TITLE_H {
        // Collapsed to the title bar: only the slot panel's title with the window buttons.
        s.cables.borrow_mut().clear();
        slot_panel(ui, s, Rect::new(r.right() - panel_w, r.y, panel_w, r.h));
        return;
    }
    let ly = Ly::new(r);
    let w = mx::strip_w(s);

    let strips = Rect::new(r.x, client.y, r.w - panel_w - 7.0, client.h);
    ui.c.fill_rect(Rect::new(r.x, client.y, r.w - panel_w, client.h), ui.t.s(0x232527));
    ui.hit(strips, Action::MixerArea, "Mixer");
    let bottom = r.bottom();

    // Header row: the level-scale and master columns on a raised band, closed by a lemon hairline.
    ui.c.fill_rect(Rect::new(r.x + 1.0, ly.y(HEAD_Y0), MASTER_X + w, HEAD_Y1 - HEAD_Y0 + 1.0), ui.t.s(0x20282b));
    ui.c.fill_rect(Rect::new(r.x + 1.0, ly.y(HEAD_Y1) + 1.0, MASTER_X + w, 1.0), ui.t.accent_dim);
    ui.c.text_center(Font::condensed(8, 2).bold(), Rect::new(r.x + 2.0, ly.y(HEAD_Y0) + 3.0, 41.0, 17.0), ui.t.tx(0x9aa7a5), "LEVEL");
    ui.c.text_center(Font::condensed(8, 2).bold(), Rect::new(r.x + MASTER_X, ly.y(HEAD_Y0) + 3.0, w, 17.0), ui.t.tx(0xb8c4c1), "MASTER");

    // "C" column: the level scale with the current track's meter (spec: ui/mixer.md §9).
    let col = Rect::new(r.x + 9.0, ly.y(METER_Y0), 28.0, bottom - 156.0 - ly.y(METER_Y0));
    let cfg = &s.mixer.meter_cfg;
    let floor = cfg.v[meters::k::FLOOR_DB];
    // The column is the law's whole range (130-924 at the measured size) squeezed evenly into its height, so marks and
    // bars keep their proportions at any mixer height (the strips' fixed-size controls do not apply here).
    let law_h = meters::LAW_BOTTOM - meters::LAW_TOP;
    let cy = |abs: f32| col.y + (abs - meters::LAW_TOP) / law_h * col.h;
    let alt = crate::prefs::check(&crate::prefs::ALTERNATE_METER_SCALE);
    let hint = if col.contains(ui.mouse.0, ui.mouse.1) {
        // The level at the mouse (the scale is not linear in y: invert it through the law).
        let db = meters::db_at((col.bottom() - ui.mouse.1) / col.h.max(1.0), floor, alt);
        format!("Current  {db:.2}dB  {:.2}", 10f32.powf(db / 20.0))
    } else {
        "Level scale".to_string()
    };
    ui.hit(col, Action::Mix(MHit::Scale), hint);
    let lemon = super::lemon(ui);
    if lemon {
        lk::outlined(ui.c, col, lk::r_btn(ui.t), ui.t.s(0x191d1e), ui.t.s(0x222527));
    } else {
        ui.c.fill_rrect(col, 3.0, ui.t.s(0x191d1e));
        ui.c.fill_rrect(col.inset(1.0), 2.5, ui.t.s(0x222527));
    }
    // Marks closer than a label height are skipped when the window is squished: "0" always shows, "+3" only with room
    // above it, the rest downward from "0". The alternate scale has no "+3".
    let scale = scale_marks(alt, floor);
    let (plus, below) = if alt { (None, &scale[..]) } else { (Some(scale[0]), &scale[1..]) };
    let zero = cy(below[0].1);
    let mut marks: Vec<(&str, f32)> = Vec::new();
    if let Some((m, y)) = plus.map(|(m, y)| (m, cy(y))).filter(|&(_, y)| zero - y >= 14.0) {
        marks.push((m, y));
    }
    marks.push((below[0].0, zero));
    let mut last = zero;
    for &(m, y) in &below[1..] {
        let y = cy(y);
        if y - last >= 14.0 {
            marks.push((m, y));
            last = y;
        }
    }
    for (m, y) in marks {
        ui.c.text_center(Font::new(12), Rect::new(col.x, y - 8.0, col.w, 15.0), ui.t.s(0x7f8a90), m);
        ui.c.fill_rect(Rect::new(col.x + 4.0, y + 7.0, col.w - 8.0, 1.0), ui.t.s(0x111315));
    }
    // Four unlabelled ticks below 45 (the alternate scale has none below its 30).
    for k in (0..4).filter(|_| !alt) {
        let y = cy(893.0 + 7.5 * k as f32);
        if y - last >= 10.0 {
            ui.c.fill_rect(Rect::new(col.x + 4.0, y, col.w - 8.0, 1.0), ui.t.s(0x111315));
        }
    }
    // The bars cover the labels (measured: two 13 px bars filling the column inside its 1 px border).
    ui.live(Live::Meter, col);
    if let Some(m) = s.mixer.meters.get(usize::from(s.selected_mixer)) {
        let ys = |db: f32| cy(meters::law_y(db, floor, alt)).min(col.bottom() - 1.0);
        for i in 0..2 {
            meter_bar(ui, col.x + 1.0 + 13.0 * i as f32, 13.0, col.bottom() - 1.0, m.db[i], m.hold[i], cfg, &ys);
        }
    }

    // The middle inserts scroll; the master strip, the scale and the left- and right-docked strips stay put (spec:
    // ui/mixer.md §6.1 "Group, Layout and CPU entries").
    let docks = docks(s);
    let area = docks.area(strips);
    let gap = area.x - MASTER_GAP;
    ui.c.fill_rect(Rect::new(gap, client.y, MASTER_GAP, client.h), ui.t.s(0x1a1e20));
    for k in 0..3 {
        ui.c.fill_rect(Rect::new(gap + 4.0, ly.y(497.0 + 3.0 * k as f32), 1.0, 1.0), ui.t.s(0x3a3e41));
    }
    let (sep_top, sep_bottom) = (ly.y(HEAD_Y0), bottom - 18.0);
    ui.c.push_clip(area);
    for &(n, off, gap) in &docks.parts[1] {
        let left = area.x + off - s.mixer_scroll;
        if left + STRIP_W < area.x || left - SEP_W > area.right() {
            continue;
        }
        if gap {
            separator(ui, left, sep_top, sep_bottom);
        }
        strip(ui, s, n, left, bottom, f, &ly);
    }
    // "+" after the last strip, at the fader handle's height (spec: ui/mixer.md §1).
    let px = area.x + docks.widths[1] - s.mixer_scroll + 14.0;
    let pb = Rect::new(px - 9.0, ly.y(HANDLE_0DB) - 9.0, 18.0, 18.0);
    if px < area.right() + 10.0 {
        let hov = ui.hit(pb, Action::Mix(MHit::Add), "Add more tracks");
        if hov && lemon {
            ui.c.fill_rrect(pb, lk::r_btn(ui.t), ui.t.button_hi);
        } else if hov {
            ui.c.stroke_rrect(pb, 3.0, 1.0, ui.t.h(0xf09a28));
        }
        icons::draw(ui.c, Icon::Plus, pb.inset(3.0), if hov { ui.t.tx(0xffffff) } else { ui.t.tx(0x9aa1a5) });
    }
    ui.c.pop_clip();
    strip(ui, s, 0, r.x + MASTER_X, bottom, f, &ly);
    for (p, x0) in [(0, strips.x + MASTER_X + w), (2, area.right())] {
        for &(n, off, gap) in &docks.parts[p] {
            if gap {
                separator(ui, x0 + off, sep_top, sep_bottom);
            }
            strip(ui, s, n, x0 + off, bottom, f, &ly);
        }
    }

    // Horizontal scroll bar under the strips.
    let sb = Rect::new(area.x + 13.0, bottom - 14.0, area.w - 20.0, 9.0);
    super::scrollbar(ui, sb, false, s.mixer_scroll, area.w, docks.widths[1] + docks.width, crate::app::Scroll::MixerX);
    chevron(ui, area.x + 5.0, bottom - 9.5, false, ui.t.s(0x5d6366));
    cables(ui, s, strips, &docks, bottom);

    slot_panel(ui, s, Rect::new(r.right() - panel_w, r.y, panel_w, r.h));
    resize_zones(ui, s, Win::Mixer, outer);
}

/// A strip's control column (`left` is its left edge): its knobs, its ⬇ and its send knobs are centred there, so the
/// cables plug in exactly under them. Extra large keeps the column on the strip's left, beside its plugin list, so a
/// strip's width no longer places it (`left + 22` used to be the centre of every layout: the cables then drew a second
/// arrow and knob a few px off the real ones).
fn strip_cx(s: &State, left: f32) -> f32 {
    if s.mixer.layout >= 7 { left + 28.0 } else { left + crate::mixer::strip_w(s) / 2.0 }
}

/// The routing cables (spec: ui/mixer.md §5): one from the current track's ⬇ into each of its send knobs, sagging
/// and swinging (crate::cable). Sockets on strips scrolled out of view still hold their cable's end.
fn cables(ui: &mut Ui, s: &State, strips: Rect, docks: &Docks, bottom: f32) {
    let area = docks.area(strips);
    let socket_x = |n: u16| strip_cx(s, docks.x(n, strips, s.mixer_scroll).unwrap_or(area.x));
    // Only the middle strips scroll: what is on the others is not clipped to the scrolling part.
    let fixed = |n: u16| !docks.scrolls(n);
    // Each cable is plugged in under the middle of its ⬇ and its knob, which are drawn again over the cables' ends.
    let (ay, ky) = (bottom - 38.0, bottom - 40.0);
    let cur = s.selected_mixer;
    let from = (socket_x(cur), ay);
    let sends: Vec<(u16, f64)> = (0..=mx::inserts(s)).filter(|&n| n != cur).filter_map(|n| Some((n, mx::send(s, cur, n)?))).collect();
    let mut cables = s.cables.borrow_mut();
    cables.begin(cur);
    for &(n, level) in &sends {
        // A send at zero is dimmed, as its knob.
        let col = if level <= 0.0 { ui.t.accent.mix(ui.t.s(0x26292b), 0.65) } else { ui.t.accent };
        cables.connect(n, from, (socket_x(n), ky), fixed(cur) || fixed(n), col);
    }
    cables.end();
    // Cables between middle strips are clipped where they scroll; those into the master or a docked strip are not.
    for (master, pts, col) in cables.each() {
        ui.c.push_clip(if master { strips } else { area });
        ui.c.polyline(pts, 2.0, col);
        ui.c.pop_clip();
    }
    drop(cables);
    if sends.is_empty() {
        return;
    }
    let clip = |n: u16| if fixed(n) { strips } else { area };
    ui.c.push_clip(clip(cur));
    down_arrow(ui, from.0, ay);
    ui.c.pop_clip();
    for (n, level) in sends {
        ui.c.push_clip(clip(n));
        send_knob(ui, s, n, socket_x(n), ky, level);
        ui.c.pop_clip();
    }
}

fn title_tools(ui: &mut Ui, s: &State, r: Rect) {
    let y = r.y + 11.5;
    let ob = Rect::new(r.x + 3.0, y - 9.0, 16.0, 18.0);
    let hov = ui.hit(ob, Action::Mix(MHit::Options), "Mixer options");
    chevron(ui, r.x + 11.0, y, true, if hov { ui.t.tx(0xffffff) } else { ui.t.tx(0xc4ccd0) });
    // View buttons (multi-touch, waveforms, extra properties, track inspector). Multi-touch and the inspector are not
    // available yet, greyed out (drawn in their colours faded into the title bar, like `Ui::veil`; a veil would show
    // over the bar's gradient). View waveforms and Extra volume / stereo properties are Options ▸ View ▸'s switches
    // (§6.5), white while on. Lemon centres them and the "+" 24 px apart: its inspector sits 10 px further right, the
    // "+" and the combo 16 px.
    let bg = ui.t.window_title;
    let dim = |c: Color| c.mix(bg, f32::from(crate::app::OFF_VEIL) / 255.0);
    let b = Rect::new(r.x + 37.0 - 9.0, y - 9.0, 18.0, 18.0);
    ui.off(b, "Multi-touch control");
    icons::draw(ui.c, Icon::Touch, b.inset(1.0), dim(ui.t.tx(0xc4ccd0)));
    let b = Rect::new(r.x + 61.0 - 9.0, y - 9.0, 18.0, 18.0);
    let hov = ui.hit(b, Action::Mix(MHit::Waveforms), "View waveforms (Alt+W)");
    let on = s.mixer.waveforms;
    icons::draw(ui.c, Icon::Plug, b.inset(1.0), if on || hov { ui.t.tx(0xffffff) } else { dim(ui.t.tx(0xc4ccd0)).mix(ui.t.tx(0xc4ccd0), 0.5) });
    let (ix, px) = if super::lemon(ui) { (10.0, 16.0) } else { (0.0, 0.0) };
    let hov = ui.hit(Rect::new(r.x + 76.0, y - 9.0, 16.0, 18.0), Action::Mix(MHit::Extra), "View extra volume / stereo properties");
    ui.off(Rect::new(r.x + 92.0 + ix, y - 9.0, 15.0, 18.0), "Track inspector");
    // Bow-tie (stereo) in blue, layout square in orange, then "+" and the view combo.
    let mut g = Glyph::new();
    let cx = r.x + 85.0;
    g.polygon(&[(cx - 8.0, y - 6.0), (cx - 1.0, y), (cx - 8.0, y + 6.0)]).polygon(&[(cx + 8.0, y - 6.0), (cx + 1.0, y), (cx + 8.0, y + 6.0)]);
    g.rect(cx - 8.5, y - 6.0, 1.5, 12.0);
    let blue = Color::rgb(0x5fb6f0);
    ui.c.glyph(if hov { ui.t.tx(0xffffff) } else if s.mixer.extra { blue } else { dim(blue) }, &g);
    let mut g = Glyph::new();
    g.frame(Rect::new(r.x + 93.0 + ix, y - 6.0, 12.0, 12.0), 1.6);
    ui.c.glyph(dim(ui.t.h(0xf3b234)), &g);
    ui.c.fill_rect(Rect::new(r.x + 101.5 + ix, y - 4.5, 2.0, 9.0), dim(Color::rgb(0xd03a2a)));
    let pb = Rect::new(r.x + 109.0 + px, y - 8.0, 16.0, 16.0);
    let hov = ui.hit(pb, Action::Mix(MHit::AddAfter), "Add a new track after the selected track");
    icons::draw(ui.c, Icon::Plus, Rect::new(r.x + 110.0 + px, y - 7.0, 14.0, 14.0), if hov { ui.t.tx(0xffffff) } else { ui.t.tx(0xc4ccd0) });
    let lb = Rect::new(r.x + 130.0 + px, r.y + 2.0, 70.0, 20.0);
    let hov = ui.hit(lb, Action::Mix(MHit::Layout), "Mixer layout");
    let name = crate::mixer::LAYOUTS[usize::from(s.mixer.layout.min(crate::mixer::LAYOUTS.len() as u8 - 1))];
    ui.c.text_clip(Font::new(17), r.x + 134.0 + px, r.y + 2.0, 66.0, if hov { ui.t.tx(0xffffff) } else { ui.t.tx(0xb8c4cb) }, name);

    // The project's modulators, one chip each: a chip dragged onto any knob or fader assigns that modulator to it
    // (`App::drop_modulator`), which is the graph's only visible handle outside a knob's own menu. The row says how to
    // make the first one, so an empty graph is not a dead end.
    let f = Font::new(13);
    let mut x = r.x + 216.0 + px;
    let limit = r.right() - 229.0;
    if s.modulation.modulators.is_empty() {
        if limit > x {
            ui.c.text_clip(f, x, r.y + 3.0, limit - x, ui.t.s(0x6d777d), "No modulators — right-click a knob ▸ New LFO");
        }
        return;
    }
    for (i, m) in s.modulation.modulators.iter().enumerate() {
        let label = chip_label(m);
        let w = (ui.c.text_width(f, &label) + 14.0).min(104.0);
        if x + w > limit {
            let more = format!("+{}", s.modulation.modulators.len() - i);
            ui.c.text_clip(Font::new(12), x, r.y + 4.0, (limit - x).max(0.0), ui.t.s(0x777f84), &more);
            break;
        }
        let b = Rect::new(x, r.y + 3.0, w, 17.0);
        let hov = ui.hit(b, Action::Modulator(m.id), format!("Drag {} onto a knob to assign it", crate::modulation::name_of(m)));
        ui.c.fill_rrect(b, 3.0, if hov { ui.t.s(0x3a3f42) } else { ui.t.s(0x2c3033) });
        ui.c.stroke_rrect(b, 3.0, 0.8, ui.t.h(0xf09a28).with_alpha(if hov { 180 } else { 60 }));
        ui.c.text_clip(f, b.x + 6.0, b.y + 2.0, b.w - 10.0, if m.enabled { ui.t.tx(0xd6dde1) } else { ui.t.s(0x828a8f) }, &label);
        x += w + 4.0;
    }
}

/// A modulator chip's label: its name and, after a dot, its rate (an envelope has none, so it reads "env").
fn chip_label(m: &os_model::modulation::Modulator) -> String {
    use os_model::modulation::ModSource;
    let short = match m.source {
        ModSource::Lfo(l) => crate::modulation::rate_text(l.period),
        ModSource::Steps(st) => crate::modulation::rate_text(st.period),
        ModSource::Envelope(_) => "env".to_string(),
        ModSource::Random(r) => crate::modulation::rate_text(r.period),
        ModSource::AudioFollower(_) => "follow".to_string(),
    };
    format!("{} · {short}", crate::modulation::name_of(m))
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let ch = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round().clamp(0.0, 255.0) as u32;
    Color::rgb(ch(a.r, b.r) << 16 | ch(a.g, b.g) << 8 | ch(a.b, b.b))
}

fn shade(c: Color, k: f32) -> Color {
    mix(Color::rgb(0), c, k)
}

/// One meter bar (spec: ui/mixer.md §9, measured from FL): the colour depends on the height, not on the level: green up
/// to "Yellow from", yellow-orange up to "Red from", red above; each zone is a slight vertical gradient. The outer
/// columns are darker, the next ones a touch lighter; a 1 px dark cap sits on the top and the peak-hold line is a
/// 1 px orange line. `ys` maps a level to y; `bottom` is the bar's foot.
#[allow(clippy::too_many_arguments)]
/// View ▸ Waveforms (Lemon Studio's design): the track's recent peaks, one pixel row each, newest at the bottom,
/// scrolling up; the left side's level grows leftwards from the middle, the right side's rightwards, in the meter's
/// green (yellow above -6 dBFS, the clip colour above 0 dBFS).
/// A strip's level meter in `meter`: its recessed slot, the measured tick marks when `ly` is given (the meter column's
/// own absolute positions), and the two 6 px peak bars on the "C" column's scale (the waveform view when View ▸
/// Waveforms is on).
fn meter_body(ui: &mut Ui, s: &State, n: u16, meter: Rect, lemon: bool, ly: Option<&Ly>) {
    // Clicking the meter is clicking the strip; its right-click menu holds the scale and waveform options (§9).
    ui.hit(meter, Action::Mix(crate::mixer::MHit::Meter(n)), "Level meter (right click for its scale and waveforms)");
    // The bars fill the slot, whatever width the layout gives it: the two channels split the inner width between
    // them, so a wider lane is wider bars (and a narrower strip still shows both sides).
    let inner = Rect::new(meter.x + 2.0, meter.y + 2.0, (meter.w - 4.0).max(4.0), meter.h - 4.0);
    if lemon {
        ui.c.fill_rrect(meter, lk::r_small(ui.t), ui.t.s(0x191d1e));
        ui.c.fill_rrect(inner, 2.5, ui.t.s(0x232729));
    } else {
        ui.c.fill_rrect(meter, 2.0, ui.t.s(0x191d1e));
        ui.c.fill_rect(inner, ui.t.s(0x232729));
    }
    if let Some(ly) = ly {
        for y in [181.0, 230.0, 278.0, 327.0, 375.0, 423.0] {
            ui.c.fill_rect(Rect::new(meter.x + 3.0, ly.y(y), (meter.w - 6.0).max(3.0), 1.0), ui.t.s(0x111315));
        }
    }
    // Per-track peak meter (spec: ui/mixer.md §9): two bars on the same scale as the "C" column.
    ui.live(Live::Meter, meter);
    if s.mixer.waveforms {
        wave(ui, s, n, inner);
    } else if let Some(m) = s.mixer.meters.get(usize::from(n)) {
        let cfg = &s.mixer.meter_cfg;
        let (top, bot) = (meter.y + 2.0, meter.bottom() - 2.0);
        let floor = cfg.v[meters::k::FLOOR_DB];
        let alt = crate::prefs::check(&crate::prefs::ALTERNATE_METER_SCALE);
        let ys = |db: f32| bot - meters::frac(db, floor, alt) * (bot - top);
        let bw = ((inner.w - 1.0) / 2.0).max(2.0);
        for i in 0..2 {
            meter_bar(ui, inner.x + (bw + 1.0) * i as f32, bw, bot, m.db[i], m.hold[i], cfg, &ys);
        }
    }
}

fn wave(ui: &mut Ui, s: &State, n: u16, r: Rect) {
    let Some(w) = s.mixer.waves.get(usize::from(n)) else { return };
    let mid = r.x + r.w / 2.0;
    let half = r.w / 2.0;
    for (i, p) in w.iter().rev().enumerate() {
        let y = r.bottom() - 1.0 - i as f32;
        if y < r.y {
            break;
        }
        let peak = p[0].max(p[1]);
        let c = if peak >= 1.0 { ui.t.clipping } else if peak >= 0.5 { ui.t.warning } else { ui.t.signal };
        let (l, rr) = (p[0].min(1.0) * half, p[1].min(1.0) * half);
        if l >= 0.25 {
            ui.c.fill_rect(Rect::new(mid - l, y, l, 1.0), c);
        }
        if rr >= 0.25 {
            ui.c.fill_rect(Rect::new(mid, y, rr, 1.0), c);
        }
    }
}

fn meter_bar(ui: &mut Ui, x: f32, w: f32, bottom: f32, db: f32, hold: f32, cfg: &meters::Settings, ys: &dyn Fn(f32) -> f32) {
    use meters::k;
    let t = ui.t;
    let full_top = ys(12.0);
    let grad = cfg.v[k::GRADIENT];
    let edge = cfg.v[k::EDGE];
    // Signal zones: turquoise for level, orange approaching the ceiling, coral at clipping.
    let green = (t.signal.shade(-0.10), t.signal);
    let yellow = (t.warning.shade(-0.08), t.warning);
    let red = (t.clipping, t.clipping);
    let y_yel = ys(cfg.v[k::YELLOW_DB]);
    let y_red = ys(cfg.v[k::RED_DB].max(cfg.v[k::YELLOW_DB]));
    if db > meters::SILENT {
        let top = ys(db).min(bottom);
        for (z0, z1, (ct, cb)) in [(y_yel, bottom, green), (y_red, y_yel, yellow), (full_top, y_red, red)] {
            let (a, b) = (top.max(z0), z1.min(bottom));
            if b - a < 0.5 {
                continue;
            }
            // Colours at a and b along the zone's gradient (scaled by the gradient setting).
            let at = |y: f32| mix(ct, mix(ct, cb, grad), (y - z0) / (z1 - z0).max(1.0));
            let (ca, cb2) = (at(a), at(b));
            let r = Rect::new(x, a, w, b - a);
            ui.c.fill_rect_gradient(r, ca, cb2, 0.0);
            if w >= 4.0 {
                let hl = |c: Color| mix(c, Color::rgb(0xffffff), 0.04);
                ui.c.fill_rect_gradient(Rect::new(x + 1.0, a, 1.0, b - a), hl(ca), hl(cb2), 0.0);
                ui.c.fill_rect_gradient(Rect::new(x + w - 2.0, a, 1.0, b - a), hl(ca), hl(cb2), 0.0);
                ui.c.fill_rect_gradient(Rect::new(x, a, 1.0, b - a), shade(ca, edge), shade(cb2, edge), 0.0);
                ui.c.fill_rect_gradient(Rect::new(x + w - 1.0, a, 1.0, b - a), shade(ca, edge), shade(cb2, edge), 0.0);
            }
        }
        if cfg.v[k::CAP] >= 0.5 && top - 1.0 > full_top {
            ui.c.fill_rect(Rect::new(x, top - 1.0, w, 1.0), ui.t.s(0x181c1e));
        }
    }
    if cfg.v[k::HOLD_ON] >= 0.5 && hold > meters::SILENT {
        let y = ys(hold).min(bottom - 1.0);
        let c = ui.t.warning;
        ui.c.fill_rect(Rect::new(x, y, w, 1.0), c);
        if w >= 4.0 {
            ui.c.fill_rect(Rect::new(x, y, 1.0, 1.0), shade(c, edge));
            ui.c.fill_rect(Rect::new(x + w - 1.0, y, 1.0, 1.0), shade(c, edge));
        }
    }
}

/// The effect rack's parts in a column `r`: the equaliser's graph at the top (when the column has the room for it — the
/// tall Extra large rack does, a squeezed one does not) and the ten slot rows under it. Split out so the layout is
/// testable without a canvas.
fn rack_rows(r: Rect) -> (Option<Rect>, Vec<Rect>) {
    let room = (r.h - 10.0).max(0.0);
    // Ten rows at least 19 px tall come first; the graph only takes what is left over, so no slot is lost to it. Its
    // height is fixed, not a share of the column, so Extra large and V2 start their slots at the same place.
    let graph_h = if r.w >= 44.0 && room >= 234.0 { 56.0 } else { 0.0 };
    let graph = (graph_h > 0.0).then(|| Rect::new(r.x + 2.0, r.y + 5.0, r.w - 4.0, graph_h));
    let top = r.y + 5.0 + if graph_h > 0.0 { graph_h + 5.0 } else { 0.0 };
    let rh = ((r.bottom() - 5.0 - top) / 10.0).clamp(15.0, 54.0);
    let rows = (0..10).map(|i| Rect::new(r.x + 2.0, top + rh * i as f32, r.w - 4.0, rh - 2.0)).collect();
    (graph, rows)
}

/// The strip's equaliser, drawn: the three bands' summed response over the frequency knob's range (10 Hz .. 16 kHz,
/// ±18 dB) with a dot on each band's own frequency and gain. It is the same response the inspector's larger graph
/// shows, computed from the track's real settings (`os_dsp::mixer::eq_response_db`), so a strip's tone reads without
/// opening anything.
fn eq_chart(ui: &mut Ui, s: &State, n: u16, r: Rect) {
    let ink = ui.t.tx(0xc9cfd3);
    ui.c.fill_rrect(r, 2.0, ui.t.s(0x101315));
    ui.c.stroke_rrect(r, 2.0, 0.8, ui.t.s(0x2a2f31));
    for k in [0.25, 0.75] {
        let y = r.y + r.h * k;
        ui.c.line(r.x + 1.0, y, r.right() - 1.0, y, 0.5, ui.t.s(0x23282b));
    }
    let zero = r.cy();
    ui.c.line(r.x + 1.0, zero, r.right() - 1.0, zero, 0.8, ui.t.s(0x3a4145));
    let Some(track) = s.project.mixer.iter().find(|m| m.index == n) else { return };
    // The graph is a control, not a picture: a press grabs the band nearest the cursor and drags its gain and
    // frequency (`MHit::StripEq`), the same drag the inspector's larger graph takes.
    let hov = ui.hit(r.inset(1.0), Action::Mix(MHit::StripEq(n)), "Track equaliser: drag a band's dot to set its frequency and gain");
    let settings = track.eq;
    let half = (r.h / 2.0 - 2.0).max(1.0);
    let steps = ((r.w - 2.0) as usize).clamp(8, 160);
    let y_of = |db: f32| zero - (db / 18.0).clamp(-1.0, 1.0) * half;
    let pts: Vec<(f32, f32)> = (0..=steps)
        .map(|i| {
            let p = i as f32 / steps as f32;
            let hz = os_dsp::mixer::eq_frequency((p * 65536.0).round() as i32);
            let db = os_dsp::mixer::eq_response_db(settings, 44100, f64::from(hz)) as f32;
            (r.x + 1.0 + p * (r.w - 2.0), y_of(db))
        })
        .collect();
    ui.c.push_clip(r.inset(1.0));
    ui.c.polyline(&pts, 1.6, ink);
    ui.c.pop_clip();
    // A dot per band, where its own frequency and gain put it: the graph's three handles, without the handles.
    for band in 0..3 {
        let p = (settings[band][1].clamp(0, 65536) as f32 / 65536.0).clamp(0.0, 1.0);
        let db = settings[band][0] as f32 / 100.0;
        let (x, y) = (r.x + 1.0 + p * (r.w - 2.0), y_of(db));
        // The dot under the cursor is the one a press would grab, so it lights up first.
        let hot = hov && (ui.mouse.0 - x).abs() < 7.0;
        ui.c.fill_circle(x, y, if hot { 3.6 } else { 2.2 }, ui.t.s(0x101315));
        ui.c.fill_circle(x, y, if hot { 2.8 } else { 1.5 }, if hot { ui.t.tx(0xffffff) } else { ui.t.accent });
    }
}

/// Extra large (layout 7): one strip's effect rack — the track's equaliser at the top, then its ten slots as rows in a
/// recessed bay. A filled row carries the plugin's colour, its name (without the house's "Lemon" prefix, so a native
/// effect fits) and its LED; an empty one is an intentional `+ Add effect` button that opens the palette for that slot.
fn plugin_list(ui: &mut Ui, s: &State, n: u16, r: Rect) {
    if r.h < 22.0 {
        return;
    }
    ui.c.fill_rrect(r, 3.0, ui.t.s(0x141719));
    ui.c.stroke_rrect(r, 3.0, 1.0, ui.t.s(0x272b2d));
    let track = s.project.mixer.iter().find(|m| m.index == n);
    let (graph, slots) = rack_rows(r);
    if let Some(g) = graph {
        eq_chart(ui, s, n, g);
    }
    for (i, b) in slots.iter().copied().enumerate() {
        let k = i as u8;
        let filled = mx::slot_name(s, n, k);
        let name = filled.as_deref().map(crate::plugins::slot_short).unwrap_or_else(|| "Add effect".to_string());
        let hov = ui.hovering(b);
        let hint = match &filled {
            Some(full) => format!("{full}  (click for its window)"),
            None => format!("Add an effect to slot {}", i + 1),
        };
        let face = if hov { ui.t.s(0x262b2d) } else if filled.is_some() { ui.t.s(0x1e2224) } else { ui.t.s(0x181b1d) };
        ui.c.fill_rrect(b, 2.0, face);
        if let Some(c) = mx::slot_color(s, n, k) {
            ui.c.fill_rect(Rect::new(b.x + 2.0, b.y + 3.0, 2.0, b.h - 6.0), super::rgb(c));
        }
        let text_x = b.x + 7.0;
        let text_w = (b.w - 20.0).max(8.0);
        if filled.is_some() {
            ui.c.stroke_rrect(b, 2.0, 0.8, ui.t.s(0x353a3c));
            let f = if b.h >= 23.0 { Font::new(12) } else { Font::new(11) };
            ui.c.text_clip(f, text_x, b.y + ((b.h - 13.0) * 0.5).max(1.0), text_w, ui.t.tx(0xc9d0d4), &name);
            // Its LED at the row's right: on while the slot is on (the slot's own switch is in the inspector).
            let on = track.is_none_or(|m| m.slot_on[i]);
            ui.c.fill_circle(b.right() - 6.0, b.cy(), 2.2, if on { ui.t.h(0x8ec63a) } else { ui.t.s(0x2f3436) });
        } else {
            // A dashed bay with its plus: an empty slot reads as a place to put an effect.
            ui.c.stroke_rrect(b, 2.0, 0.8, ui.t.h(0xf09a28).with_alpha(if hov { 160 } else { 55 }));
            let mut g = Glyph::new();
            let (px, py) = (b.x + 10.0, b.cy());
            g.rect(px - 4.0, py - 0.7, 8.0, 1.4).rect(px - 0.7, py - 4.0, 1.4, 8.0);
            ui.c.glyph(if hov { ui.t.tx(0xffffff) } else { ui.t.h(0xf09a28) }, &g);
            let f = if b.h >= 23.0 { Font::new(11) } else { Font::new(10) };
            ui.c.text_clip(f, text_x + 7.0, b.y + ((b.h - 12.0) * 0.5).max(1.0), text_w - 7.0, ui.t.s(0x8c9498), &name);
        }
        ui.hit(b, Action::Mix(MHit::TrackSlot(n, k)), hint);
    }
}

/// The console's palette for the chosen Mixer style (Settings ▸ General ▸ Mixer style): the four section inks, the row
/// plate's two stops and its edge, and the STRIP switch's faceplate. The Edition's finishes keep the Edition's colours
/// (`views/mixer/analog.rs`); Lemon, this program's own look, tints each section with its accent colours.
struct ConsoleStyle {
    cut: Color,
    drive: Color,
    comp: Color,
    ds: Color,
    plate: (Color, Color),
    top: Color,
    edge: Color,
    switch: (Color, Color),
}

fn console_style(ui: &Ui) -> ConsoleStyle {
    let t = ui.t;
    use crate::views::mixer_finish::Finish;
    match Finish::current() {
        Finish::Original => ConsoleStyle {
            cut: Color::rgb(0xc8c2ad),
            drive: Color::rgb(0xd1a77e),
            comp: Color::rgb(0xcbb578),
            ds: Color::rgb(0x91b9c9),
            plate: (Color::rgb(0x383c3a), Color::rgb(0x292d2b)),
            top: Color::rgb(0x858a80).with_alpha(48),
            edge: Color::rgb(0x111315).with_alpha(180),
            switch: (Color::rgb(0x798682), Color::rgb(0x10181a)),
        },
        Finish::CleanModern => ConsoleStyle {
            cut: Color::rgb(0xe2e2df),
            drive: Color::rgb(0xe2e2df),
            comp: Color::rgb(0xe2e2df),
            ds: Color::rgb(0xe2e2df),
            plate: (Color::rgb(0x2e2e2e), Color::rgb(0x292929)),
            top: Color::rgb(0x5a5a5a),
            edge: Color::rgb(0x191919),
            switch: (Color::rgb(0x8a8a8a), Color::rgb(0x191919)),
        },
        Finish::Analog => ConsoleStyle {
            cut: Color::rgb(0xd8e0e5),
            drive: Color::rgb(0xe8c74f),
            comp: Color::rgb(0xedf1ed),
            ds: Color::rgb(0xa8b0b5),
            plate: (Color::rgb(0x2a2e31), Color::rgb(0x1d2124)),
            top: Color::rgb(0x77828a).with_alpha(110),
            edge: Color::rgb(0x0a0c0d).with_alpha(200),
            switch: (Color::rgb(0x3c4348), Color::rgb(0x0a0c0d)),
        },
        Finish::Lemon => ConsoleStyle {
            cut: t.tx(0xb7c3ca),
            drive: t.h(0xf09a28),
            comp: t.h(0xe8c74f),
            ds: t.a(0x5fb6f0),
            plate: (t.s(0x24282a), t.s(0x1c1f21)),
            top: t.tx(0xffffff).with_alpha(14),
            edge: t.s(0x141719),
            switch: (t.s(0x2b2e30), t.s(0x181b1d)),
        },
    }
}

/// One console row's plate: the style's recessed strip, a highlight along its top and an edge below.
fn console_plate(ui: &mut Ui, r: Rect, st: &ConsoleStyle) {
    ui.c.fill_rect_gradient(r, st.plate.0, st.plate.1, 2.0);
    ui.c.line(r.x + 2.0, r.y, r.right() - 2.0, r.y, 0.8, st.top);
    ui.c.line(r.x + 2.0, r.bottom(), r.right() - 2.0, r.bottom(), 1.0, st.edge);
}

/// A row's label, centred in the `frac`..`frac + span` part of the row (the Edition's halves: 0..0.54 and 0.54..1).
fn console_label(ui: &mut Ui, row: Rect, frac: f32, span: f32, text: &str, ink: Color) {
    ui.c.text_center(Font::new(9), Rect::new(row.x + row.w * frac, row.y + 2.0, row.w * span, 11.0), ink, text);
}

/// The mixer's knob family: a machined cap in a recessed well, a ring of graduations, a cream pointer and the value's
/// arc around the rim in the control's own ink. One drawing for every parameter, so the console, the pan and the
/// effect racks feel like the same hardware; `bipolar` arcs from twelve o'clock (pan, separation), `ink` distinguishes
/// a section's control from its neighbours'.
fn analog_knob(ui: &mut Ui, cx: f32, cy: f32, r: f32, value: f32, bipolar: bool, ink: Color, hov: bool) {
    let t = ui.t;
    let v = value.clamp(0.0, 1.0);
    let start = 0.75 * PI;
    let sweep = 1.5 * PI;
    let a = start + sweep * v;
    let (a0, a1) = if bipolar { (start + sweep / 2.0, a) } else { (start, a) };
    // The well: a dark recess with a lit lower edge, so the cap sits in something.
    ui.c.fill_circle(cx, cy + 0.9, r + 2.4, t.s(0x0b0d0e));
    ui.c.fill_circle(cx, cy, r + 2.0, t.s(if hov { 0x22262a } else { 0x1c2023 }));
    // Graduations around the sweep: ends brighter than the middle, as on a hardware scale.
    let ticks = if r >= 9.0 { 11 } else { 7 };
    for i in 0..ticks {
        let ang = start + sweep * (i as f32 / (ticks - 1) as f32);
        let (r0, r1) = (r + 0.6, r + 2.0);
        let end = i == 0 || i + 1 == ticks;
        let c = if end { t.s(0x6a7276) } else { t.s(0x454b4f) };
        ui.c.line(cx + ang.cos() * r0, cy + ang.sin() * r0, cx + ang.cos() * r1, cy + ang.sin() * r1, 1.0, c);
    }
    // The cap: a dark disc with a brushed highlight along its top-left and a shadow under its edge.
    ui.c.fill_circle(cx, cy, r, t.s(0x23272a));
    ui.c.fill_circle(cx, cy - 0.6, (r - 1.0).max(1.0), if hov { t.s(0x52575a) } else { t.s(0x43484b) });
    ui.c.fill_circle(cx, cy + r * 0.28, (r - 1.0).max(1.0), t.s(0x2c3033).with_alpha(120));
    ui.c.arc(cx, cy, (r - 1.4).max(1.0), 1.15 * PI, 0.7 * PI, (r * 0.16).clamp(1.0, 2.2), t.tx(0xffffff).with_alpha(30));
    // The pointer: cream, from a quarter of the cap to its edge.
    let p0 = r * 0.28;
    let p1 = (r - 1.5).max(1.0);
    ui.c.line(cx + p0 * a.cos(), cy + p0 * a.sin(), cx + p1 * a.cos(), cy + p1 * a.sin(), (r * 0.22).clamp(1.2, 2.6), t.tx(0xf5efdc));
    // The value's arc just outside the graduations, in the control's ink.
    if (a1 - a0).abs() > 0.02 {
        ui.c.arc(cx, cy, r + 2.6, a0.min(a1), (a1 - a0).abs(), 1.8, ink);
    }
}

/// A section's knob: the analog family's cap in its well, at the largest size its row allows; `frac` across the row,
/// its value written under it when the row is tall enough ("OUT" at zero, as the Edition reads).
fn console_knob(ui: &mut Ui, s: &State, row: Rect, frac: f32, id: KnobId, ink: Color, values: bool) {
    let radius = (row.h * 0.32).min((row.h - 14.0) * 0.5).min(row.w * 0.26).clamp(4.0, 15.0);
    let cx = row.x + row.w * frac;
    let cy = row.y + 11.0 + (row.h - 13.0) * 0.46;
    ui.hit(
        Rect::new(cx - radius - 4.0, row.y + 9.0, 2.0 * radius + 8.0, (row.h - 10.0).max(1.0)),
        Action::Knob(id),
        s.knob_text(id),
    );
    let hov = ui.hovering(Rect::new(cx - radius - 2.0, cy - radius - 2.0, 2.0 * radius + 4.0, 2.0 * radius + 4.0));
    let value = s.knob(id);
    if crate::views::mixer_finish::Finish::current().on() {
        mixer_knob(ui, cx, cy, radius, radius + 2.5, value, Some(false));
    } else {
        analog_knob(ui, cx, cy, radius, value, false, ink, hov);
    }
    if values && row.h >= 40.0 {
        let text = s.knob_text(id);
        let read = if value == 0.0 { "OUT".to_string() } else { text.split(": ").nth(1).unwrap_or("").to_string() };
        // The readout sits in its knob's half of the row, so it fits too in a narrow column.
        let bw = (row.w * 0.5 - 2.0).min(40.0);
        let bx = (cx - bw * 0.5).clamp(row.x, row.right() - bw);
        let f = if bw < 30.0 { Font::new(8) } else { Font::new(9) };
        ui.c.text_center(f, Rect::new(bx, row.y + row.h - 12.0, bw, 10.0), ui.t.tx(0xd7d4c6), &read);
    }
}

/// The strip's STRIP switch: the style's labelled faceplate with its two-ended rocker (the Edition's `knobs_switch`).
fn console_switch(ui: &mut Ui, r: Rect, on: bool, st: &ConsoleStyle) {
    let hov = ui.hovering(r);
    ui.c.text_center(Font::new(8).bold(), Rect::new(r.x, r.y, r.w, 10.0), ui.t.tx(0xe1ddd1), "STRIP");
    let plate = Rect::new(r.x + 1.0, r.y + 11.0, r.w - 2.0, (r.h - 12.0).max(8.0));
    ui.c.fill_rrect(plate.outset(1.0), 2.0, st.switch.1);
    ui.c.fill_rrect(plate, 2.0, if hov { st.switch.0.mix(ui.t.tx(0xffffff), 0.2) } else { st.switch.0 });
    let rocker = plate.inset(2.0);
    let h = rocker.h / 2.0;
    for i in 0..2 {
        let raised = (i == 1) == on;
        ui.c.fill_rect_gradient(
            Rect::new(rocker.x, rocker.y + i as f32 * h, rocker.w, h),
            if raised { ui.t.tx(0xe9e7d9) } else { ui.t.s(0x767c71) },
            if raised { ui.t.tx(0xa6ada0) } else { ui.t.s(0x4a5147) },
            1.0,
        );
    }
    ui.c.line(rocker.x, rocker.cy(), rocker.right(), rocker.cy(), 1.0, st.edge);
}

/// A grid module's colour: orange retriggers, yellow crushes, blue reverses, grey gates and green rings; a step that
/// fires nothing is the plate's own dark.
fn glitch_ink(ui: &Ui, module: usize) -> Color {
    match module {
        1 => ui.t.h(0xf09a28),
        2 => ui.t.h(0xe8c74f),
        3 => ui.t.h(0x5fb6f0),
        4 => ui.t.h(0x9aa3a8),
        5 => ui.t.h(0x7bd88f),
        _ => ui.t.s(0x3a4042),
    }
}

/// A module's console ink: the analog sections keep the Edition's colours (low cut and OTT cream, drive amber, the
/// compressor and TAME green-gold, the table blue) and the grid the glitch modules' orange.
fn chain_ink(ui: &Ui, st: &ConsoleStyle, module: u8) -> Color {
    match module {
        1 | 2 => st.cut,
        3 => st.drive,
        4 | 5 => st.comp,
        6 => st.ds,
        _ => glitch_ink(ui, 1),
    }
}

/// The `GRID` header's height in a glitch tile: its name and the grid's two buttons.
const GRID_HEAD: f32 = 16.0;

/// The glitch grid's area in a console body `r` and one cell's size: the header, the amount row's 26 px and a 3 px
/// gap, then the 4×4. `glitch_panel` draws from it, so a drag's `glitch_at` finds the cells it sees.
fn glitch_area(r: Rect) -> (Rect, f32, f32) {
    let top = r.y + GRID_HEAD + 29.0;
    let grid = Rect::new(r.x + 1.0, top, (r.w - 2.0).max(16.0), (r.bottom() - top).max(24.0));
    let cw = grid.w / 4.0;
    let ch = (grid.h / 4.0).min(cw * 1.5).max(9.0);
    (grid, cw, ch)
}

/// The step under `at`, given the grid cell painted for step `step` (a hit rect, so the grid is known by any of its
/// cells): a drag paints the cells the pointer crosses. A point outside the grid takes the nearest cell in it.
pub fn glitch_at(cell: Rect, step: u8, (x, y): (f32, f32)) -> u8 {
    let (cw, ch) = (cell.w + 1.0, cell.h + 1.0);
    let (ox, oy) = (cell.x - cw * f32::from(step % 4), cell.y - ch * f32::from(step / 4));
    let col = ((x - ox) / cw).floor().clamp(0.0, 3.0) as u8;
    let row = ((y - oy) / ch).floor().clamp(0.0, 3.0) as u8;
    row * 4 + col
}

/// The glitch console: a panel of its own — warm near-black, a hairline of the glitch orange and lit corner ticks,
/// not the analog sections' plates — with its name and two buttons (`RND` a new random pattern, `CLR` an empty one)
/// over the amount as a bar, and the sixteen steps as a 4×4. A cell that fires is filled in its module's colour with a
/// lit top edge and its two letters; an empty one is a dark socket. A click cycles a cell (`MCmd::GlitchStep`); a drag
/// paints; the step the audio thread rendered last is ringed in cream.
fn glitch_panel(ui: &mut Ui, s: &State, n: u16, m: &os_model::MixerTrack, r: Rect) {
    let ink = glitch_ink(ui, 1);
    let surface = ui.t.s(0x171314);
    ui.c.fill_rrect(r, 3.0, surface);
    ui.c.stroke_rrect(r, 3.0, 1.0, ink.with_alpha(if m.strip_bypassed { 55 } else { 125 }));
    for (x, y, dx, dy) in [
        (r.x + 3.0, r.y + 3.0, 1.0, 1.0),
        (r.right() - 3.0, r.y + 3.0, -1.0, 1.0),
        (r.x + 3.0, r.bottom() - 3.0, 1.0, -1.0),
        (r.right() - 3.0, r.bottom() - 3.0, -1.0, -1.0),
    ] {
        ui.c.line(x, y, x + 6.0 * dx, y, 1.0, ink.with_alpha(180));
        ui.c.line(x, y, x, y + 6.0 * dy, 1.0, ink.with_alpha(180));
    }
    // The header: the grid's name, then its two buttons at the right (below 50 px a 7 px button is not clickable, so
    // the name alone is drawn and the track menu has both).
    ui.c.text_center(Font::new(8).bold(), Rect::new(r.x + 4.0, r.y + 3.0, 24.0, 11.0), ink, "GRID");
    if r.w >= 50.0 {
        let bw = ((r.w - 26.0) / 2.0).clamp(11.0, 15.0);
        let clear = Rect::new(r.right() - 4.0 - bw, r.y + 3.0, bw, 11.0);
        let hov = ui.hit(clear, Action::Mix(MHit::GlitchClear(n)), "Empty the grid (one undo step)");
        ui.c.fill_rrect(clear, 2.0, if hov { ui.t.button_hi } else { ui.t.s(0x241b18) });
        ui.c.stroke_rrect(clear, 2.0, 0.8, ink.with_alpha(150));
        ui.c.text_center(Font::new(7).bold(), clear, ink, "CLR");
        let rnd = Rect::new(clear.x - bw - 3.0, clear.y, bw, 11.0);
        let hov = ui.hit(rnd, Action::Mix(MHit::GlitchRandom(n)), "A new random pattern (one undo step)");
        ui.c.fill_rrect(rnd, 2.0, if hov { ui.t.button_hi } else { ui.t.s(0x241b18) });
        ui.c.stroke_rrect(rnd, 2.0, 0.8, ink.with_alpha(150));
        ui.c.text_center(Font::new(7).bold(), rnd, ink, "RND");
    }
    // The amount: its knob, then the bar under it, so the level is a length as well as a number.
    let amount = Rect::new(r.x + 1.0, r.y + GRID_HEAD, r.w - 2.0, 24.0);
    console_knob(ui, s, amount, 0.5, KnobId::Glitch(n), ink, r.w >= 50.0);
    let level = Rect::new(amount.x + 4.0, amount.bottom() - 3.0, (amount.w - 8.0).max(11.0), 3.0);
    let frac = os_dsp::glitch::glitch_amount(m.glitch_amount).clamp(0.0, 1.0);
    ui.c.fill_rrect(level, 1.5, ui.t.s(0x2b2421));
    if frac > 0.0 {
        ui.c.fill_rrect(Rect::new(level.x, level.y, (level.w * frac).max(1.5), level.h), 1.5, ink);
    }
    let (grid, cw, ch) = glitch_area(r);
    // A faint grid of its own behind the cells, so the 4×4 has a surface instead of floating on the strip.
    for k in 1..4 {
        let y = grid.y + grid.h * k as f32 / 4.0;
        ui.c.line(grid.x, y, grid.right(), y, 0.5, ink.with_alpha(22));
        let x = grid.x + grid.w * k as f32 / 4.0;
        ui.c.line(x, grid.y, x, grid.bottom(), 0.5, ink.with_alpha(22));
    }
    // The playhead: the step the audio thread rendered last (`MixerUi::steps`), its cell ringed in cream.
    let play = s.mixer.steps.get(usize::from(n)).copied().flatten();
    ui.live(Live::Glitch, grid);
    let count = os_dsp::glitch::MODULES.len();
    for i in 0..os_dsp::glitch::STEPS {
        let b = Rect::new(
            grid.x + cw * (i % 4) as f32 + 0.75,
            grid.y + ch * (i / 4) as f32 + 0.75,
            (cw - 1.5).max(4.0),
            (ch - 1.5).max(6.0),
        );
        let module = usize::from(m.glitch[i]).min(count - 1);
        let cell = glitch_ink(ui, module);
        let hov = ui.hovering(b);
        let here = play == Some(i as u8);
        if module > 0 {
            ui.c.fill_rrect(b, 2.5, cell.mix(ui.t.s(0x120f10), if here { 0.3 } else { 0.58 }));
            // A lit top edge and a soft ring: a firing step reads as a lit key, not a checked box.
            ui.c.line(b.x + 2.0, b.y + 1.0, b.right() - 2.0, b.y + 1.0, 1.0, cell.with_alpha(220));
            ui.c.stroke_rrect(b, 2.5, 1.0, cell.with_alpha(if hov || here { 255 } else { 165 }));
        } else {
            ui.c.fill_rrect(b, 2.5, ui.t.s(0x100e0f));
            ui.c.stroke_rrect(b, 2.5, 1.0, ui.t.s(if hov { 0x4a423f } else { 0x282321 }));
        }
        if here {
            ui.c.stroke_rrect(Rect::new(b.x - 1.0, b.y - 1.0, b.w + 2.0, b.h + 2.0), 3.5, 1.6, ui.t.tx(0xfff4d6));
        }
        ui.c.text_center(Font::new(9).bold(), b, if module > 0 { cell } else { ui.t.s(0x453d39) }, os_dsp::glitch::SHORT[module]);
        let hint = if module == 0 {
            format!("Step {}: off (click to place {})", i + 1, os_dsp::glitch::MODULES[1])
        } else {
            format!("Step {}: {} (click for the next module)", i + 1, os_dsp::glitch::MODULES[module])
        };
        ui.hit(b, Action::Mix(MHit::GlitchCell(n, i as u8)), hint);
    }
}

/// The Analog Channel Strip's console: the STRIP switch, then the Edition's four rows of section controls — low cut
/// and OTT, drive with its character's button, compressor (its profile cycles from the label, as the Edition's little
/// triangle says) and TAME, and the table. Every control writes a word of the track's strip, so the panel and the
/// right-click menu change the same state (`crate::mixer::MCmd`).
fn console_panel(ui: &mut Ui, s: &State, n: u16, r: Rect) {
    let st = console_style(ui);
    let track = mx::track(s, n);
    let bypass = track.is_some_and(|m| m.strip_bypassed);
    let top = Rect::new(r.x, r.y, r.w, 28.0);
    let _ = ui.hit(
        top,
        Action::Mix(MHit::StripPower(n)),
        format!("Analog channel strip: {}", if bypass { "bypassed (click to switch on)" } else { "on" }),
    );
    console_switch(ui, top, !bypass, &st);
    let Some(m) = track else { return };
    // The Edition's rows are 57 px at most, one per section; the long labels and the value readouts need a panel wide
    // enough for them (Wide 2's 64 px, Wide 3's 80 px), so a narrow one takes the short labels the Edition uses on its
    // own compact strips.
    let wide = r.w >= 64.0;
    let values = r.w >= 50.0;
    // The chain bar: one chip per module the track runs, in signal order, so the two whole strips read through the same
    // bar (six chips, or one). A click takes a module out — the Modules menu puts it back and reorders — and the bar's
    // empty foot switches the track between the two whole strips, as it always did. Below 50 px a handful of 6 px chips
    // would not be clickable, so the narrow form is "MOD" and the count, and the menu is the way in.
    let chain = m.chain;
    let mut modules = [0u8; 8];
    let mut count = 0;
    for &module in &chain {
        if module != 0 {
            modules[count] = module;
            count += 1;
        }
    }
    let modules = &modules[..count];
    let bar = Rect::new(r.x, top.bottom(), r.w, 14.0);
    let hov = ui.hit(
        bar,
        Action::Mix(MHit::GlitchMode(n)),
        if m.glitch_style { "Glitch strip (click for the analog strip)" } else { "Analog strip (click for the glitch strip)" },
    );
    ui.c.fill_rrect(bar.inset(1.0), 2.0, if hov { ui.t.button_hi } else { ui.t.s(0x202628) });
    ui.c.line(bar.x + 2.0, bar.bottom(), bar.right() - 2.0, bar.bottom(), 1.0, st.edge);
    if r.w >= 50.0 {
        let cw = (bar.w - 2.0) / modules.len().max(1) as f32;
        for (i, &module) in modules.iter().enumerate() {
            let chip = Rect::new(bar.x + 1.0 + cw * i as f32, bar.y + 2.0, (cw - 1.0).max(3.0), bar.h - 4.0);
            let ink = chain_ink(ui, &st, module);
            ui.c.fill_rrect(chip, 1.5, ink.mix(ui.t.s(0x14171a), if ui.hovering(chip) { 0.2 } else { 0.5 }));
            ui.c.text_center(Font::new(7).bold(), chip, ink, os_dsp::strip::CHAIN_SHORT[usize::from(module)]);
            let what = os_dsp::strip::CHAIN[usize::from(module)];
            ui.hit(chip, Action::Mix(MHit::ChainSlot(n, module)), format!("{what} in the chain (click to take it out; the menu reorders)"));
        }
    } else {
        ui.c.text_center(Font::new(8).bold(), bar, st.cut, &format!("MOD {count}"));
    }
    let body = Rect::new(r.x, bar.bottom() + 3.0, r.w, (r.bottom() - bar.bottom() - 3.0).max(40.0));
    // One tile per module, top to bottom in signal order: an analog section is one row (its label, its knob, and the
    // drive character's or the compressor's profile where the module has one), and the grid is its amount over the 4×4,
    // four rows tall. The rows share the body's height, up to the Edition's 76 px; a chain too tall for the console
    // squeezes them and is clipped there, and the menu still reaches every module.
    let units: f32 = modules.iter().map(|&x| if x == 7 { 4.0 } else { 1.0 }).sum::<f32>().max(1.0);
    let unit = (body.h / units).min(76.0);
    ui.c.push_clip(body);
    let mut y = body.y;
    for &module in modules {
        if module == 7 {
            glitch_panel(ui, s, n, m, Rect::new(body.x, y, body.w, unit * 4.0));
            y += unit * 4.0;
            continue;
        }
        let row = Rect::new(body.x, y, body.w, (unit - 2.0).max(6.0));
        y += unit;
        console_plate(ui, row, &st);
        match module {
            // Drive, with the character's button where the Edition keeps it (the row's right half).
            3 => {
                console_label(ui, row, 0.0, 0.54, "DRIVE", st.drive);
                console_knob(ui, s, row, 0.27, KnobId::Strip(n, 2), st.drive, values);
                let kind = os_dsp::strip::drive_kind(m.strip[6]).min(os_dsp::strip::DRIVE_TYPES.len() - 1);
                let kind_name = os_dsp::strip::DRIVE_TYPES[kind];
                let bt = Rect::new(row.x + row.w * 0.54, row.cy() - 9.0, (row.w * 0.46 - 1.0).max(10.0), 18.0);
                let hov = ui.hit(bt, Action::Mix(MHit::StripKind(n)), format!("Drive character: {kind_name} (click to change)"));
                ui.c.fill_rrect(bt, 3.0, if hov { ui.t.button_hi } else { ui.t.s(0x202729) });
                ui.c.stroke_rrect(bt, 3.0, 1.0, ui.t.s(0x59636a));
                ui.c.text_center(Font::new(9), Rect::new(bt.x, bt.y, bt.w - 6.0, bt.h), st.drive, kind_name);
                ui.c.fill_polygon(&[(bt.right() - 6.0, bt.cy() - 1.0), (bt.right() - 2.0, bt.cy() - 1.0), (bt.right() - 4.0, bt.cy() + 2.0)], st.drive);
            }
            // The compressor, its profile cycling from the label, as the Edition's little triangle says.
            5 => {
                let profile = os_dsp::strip::comp_profile(m.strip[5]).min(os_dsp::strip::PROFILES.len() - 1);
                let profile_name = os_dsp::strip::PROFILES[profile].name;
                let label = Rect::new(row.x, row.y, row.w * 0.54, 12.0);
                ui.c.text_center(Font::new(9), Rect::new(label.x, label.y + 2.0, label.w, 11.0), st.comp, "COMP");
                ui.c.fill_polygon(&[(label.right() - 5.0, row.y + 5.0), (label.right() - 1.0, row.y + 5.0), (label.right() - 3.0, row.y + 8.0)], st.comp);
                ui.hit(label, Action::Mix(MHit::StripProfile(n)), format!("Compressor profile: {profile_name} (click to change)"));
                console_knob(ui, s, row, 0.73, KnobId::Strip(n, 1), st.comp, values);
            }
            1 | 2 | 4 | 6 => {
                let (name, word) = match module {
                    1 => (if wide { "LOW CUT" } else { "CUT" }, 0),
                    2 => ("OTT", 3),
                    4 => ("TAME", 4),
                    _ => (if wide { "TABLE" } else { "TBL" }, 7),
                };
                let ink = chain_ink(ui, &st, module);
                ui.c.text_center(Font::new(9), Rect::new(row.x, row.y + 2.0, row.w, 11.0), ink, name);
                console_knob(ui, s, row, 0.5, KnobId::Strip(n, word), ink, values);
            }
            _ => {}
        }
    }
    ui.c.pop_clip();
}

/// One strip; `left` is its left edge (44 px pitch). Columns: knobs at +22, meter / fader at +32,
/// name and small controls at +12 (spec §7).
fn strip(ui: &mut Ui, s: &State, n: u16, left: f32, bottom: f32, f: &FrameInfo, ly: &Ly) {
    let t = ui.t;
    let track = s.project.mixer.iter().find(|m| m.index == n);
    let selected = s.selected_mixer == n;
    let also = !selected && mx::is_selected(s, n);
    // The strip's columns follow its width (the chosen layout's): the knobs and routing sit on its centre line, the
    // meter 10 px right of it and the name / small controls 10 px left.
    let w = crate::mixer::strip_w(s);
    // Extra large (layout 7) keeps the strip's controls on its left and gives the rest to its plugin list column.
    let extra = s.mixer.layout >= 7;
    let c = strip_cx(s, left);
    let (kx, mx, sx) = (c, c + 10.0, c - 10.0);
    let _ = f;

    // Body and the strip divider.
    // The current strip's body is a shade lighter (measured: 0x26292b vs 0x232527); an armed one is tinted (0x312236 vs
    // 0x2d1f31; spec: audio/track-io.md §1.5).
    let armed = track.is_some_and(|m| m.armed);
    let body = match (armed, selected) {
        (true, true) => ui.t.s(0x312236),
        (true, false) => ui.t.s(0x2d1f31),
        (false, true) => ui.t.s(0x26292b),
        (false, false) => ui.t.s(0x232527),
    };
    // The track's colour: the strip's body is tinted with it (spec: ui/mixer.md §2, the Michael Lane Edition's
    // traditional mixer), so a coloured track reads at a glance.
    let tint = track.and_then(|m| m.color).map(super::rgb);
    let body = tint.map_or(body, |c| body.mix(c, 0.22));
    let lemon = super::lemon(ui);
    let finish = crate::views::mixer_finish::Finish::current();
    if finish.on() {
        // One of the console finishes from the Michael Lane Edition (Settings ▸ General ▸ Mixer style).
        let card = Rect::new(left, ly.y(HEAD_Y1) + 1.0, w, bottom - ly.y(HEAD_Y1) - 1.0);
        crate::views::mixer_finish::panel(ui, card, selected);
        // A finish draws its own face, so its tint is laid over it rather than mixed into the body.
        if let Some(c) = tint {
            ui.c.fill_rect(card, c.with_alpha(26));
        }
    } else if lemon {
        // Lemon (spec: ui/lemon-design.md §8): each strip is a rounded card on the dark gap colour.
        let card = Rect::new(left, ly.y(HEAD_Y1) + 1.0, w, bottom - ly.y(HEAD_Y1) - 1.0);
        ui.c.fill_rect(card, ui.t.s(0x131517));
        ui.c.fill_rrect(Rect::new(card.x + 1.5, card.y + 1.0, card.w - 3.0, card.h - 4.0), lk::r_panel(t), body);
    } else {
        ui.c.fill_rect(Rect::new(left, ly.y(HEAD_Y1) + 1.0, w, bottom - ly.y(HEAD_Y1) - 1.0), body);
        ui.c.fill_rect(Rect::new(left + w - 1.0, ly.y(HEAD_Y1) + 1.0, 1.0, bottom - ly.y(HEAD_Y1) - 1.0), ui.t.s(0x1c1f21));
    }
    // The whole strip selects (Ctrl adds, Shift renames); controls register on top of it (spec: ui/mixer.md §2).
    ui.hit(Rect::new(left, ly.y(HEAD_Y0), w, bottom - ly.y(HEAD_Y0)), Action::Mix(MHit::Strip(n)), mx::name(s, n));
    let hov = ui.hovering(Rect::new(left, ly.y(HEAD_Y0), w, ly.y(PANEL_Y1) - ly.y(HEAD_Y0)));

    // Header number (insert strips only; the master's "M" sits in the fixed header band).
    if n != 0 {
        let hr = Rect::new(left, ly.y(HEAD_Y0), w, ly.y(HEAD_Y1) - ly.y(HEAD_Y0) + 1.0);
        ui.c.fill_rect(hr, tint.map_or(ui.t.s(0x2b2e30), |c| ui.t.s(0x2b2e30).mix(c, 0.35)));
        if selected && lemon {
            lk::lit_in(ui.c, ui.t, Rect::new(hr.x + 3.0, hr.y + 2.0, hr.w - 6.0, hr.h - 4.0), lk::r_btn(t), false);
        } else if selected {
            // Hatched header for the selected strip.
            ui.c.push_clip(hr);
            for k in 0..14 {
                let x = hr.x - 20.0 + 5.0 * k as f32;
                ui.c.line(x, hr.bottom(), x + 21.0, hr.y, 1.0, ui.t.s(0x34383a));
            }
            ui.c.pop_clip();
        }
        let c = if selected && lemon { lk::ink_on_lit(ui.t) } else if selected || hov { ui.t.tx(0x9aa1a5) } else { ui.t.s(0x6d7275) };
        ui.c.text_center(Font::new(17), Rect::new(left, ly.y(HEAD_Y0), w, 21.0), c, &n.to_string());
    }

    // Dark head panel (name, LED, pan) with a rounded bottom.
    ui.c.fill_rrect(Rect::new(left + 1.0, ly.y(PANEL_Y0), w - 2.0, ly.y(PANEL_Y1) - ly.y(PANEL_Y0)), 9.0, ui.t.s(0x1d1f21));
    ui.c.fill_rect(Rect::new(left + 1.0, ly.y(PANEL_Y0), w - 2.0, 12.0), ui.t.s(0x1d1f21));

    // The strip's panel button: the Analog Channel Strip's console shown or hidden for this channel alone, whatever
    // the layout (`mx::console_shown`).
    let panel = Rect::new(left + 2.0, ly.y(HEAD_Y0) + 3.0, 13.0, 15.0);
    let shown = mx::console_shown(s, n);
    let hov = ui.hit(
        panel,
        Action::Mix(MHit::StripPanel(n)),
        format!("Analog strip panel: {}  (shown by the wide layouts)", if shown { "shown" } else { "hidden" }),
    );
    ui.c.fill_rrect(panel, 2.5, if shown { ui.t.h(0x8ec63a) } else if hov { ui.t.button_hi } else { ui.t.s(0x2b2e30) });
    let mut g = Glyph::new();
    for k in 0..3 {
        g.rect(panel.x + 3.0, panel.y + 4.0 + 3.0 * k as f32, 7.0, 1.2);
    }
    ui.c.glyph(if shown { ui.t.s(0x141719) } else if hov { ui.t.tx(0xffffff) } else { ui.t.s(0x9aa1a5) }, &g);

    // Wide layouts (Wide 2 and up) draw the Analog Channel Strip's console in the head panel instead of the meter and
    // the rotated name: the tracks' sections at a glance, as the Michael Lane Edition's wide strips do. Narrow strips
    // keep the meter until their panel button asks for the console (spec: ui/mixer.md §2).
    let console = shown;
    let v2 = s.mixer.layout >= crate::mixer::EXTRA_LARGE_V2;
    let track_y1 = bottom - 99.0;
    let meter = Rect::new(mx - 8.5, ly.y(METER_Y0), 18.0, ly.y(METER_Y1) - ly.y(METER_Y0));
    if console {
        // Extra large keeps its plugin list column on the right, so the console takes the strip's 44 px control column.
        // Wide 2, Wide 3 and Wide alt are wide enough to keep the meter beside the console, as their narrow relatives
        // do, so the level stays visible there too; a strip whose panel button shows the console at a narrower layout
        // gives it the whole strip.
        let beside = !extra
            && matches!(s.mixer.layout, crate::mixer::WIDE_2 | crate::mixer::WIDE_3 | crate::mixer::WIDE_ALT);
        // Extra large's console gets a real control width — its section labels, keys and value readouts need it — and
        // its plugin rack a column beside it; the wide layouts yield 20 px to the meter at the strip's right edge.
        let sw = if extra { if v2 { 55.0 } else { 76.0_f32.min(w - 56.0) } } else { w - 3.0 - if beside { 20.0 } else { 0.0 } };
        console_panel(ui, s, n, Rect::new(left + 1.0, ly.y(PANEL_Y0) + 2.0, sw, (ly.y(METER_Y1) - ly.y(PANEL_Y0) - 4.0).max(24.0)));
        if beside {
            let m = Rect::new(left + w - 21.0, ly.y(PANEL_Y0) + 2.0, 20.0, (ly.y(METER_Y1) - ly.y(PANEL_Y0) - 4.0).max(24.0));
            meter_body(ui, s, n, m, lemon, Some(ly));
        }
        // The console takes the meter's column, but a wide strip has room for the level elsewhere: Extra large puts it
        // beside the fader (the tall room the plugin list leaves free), V2 beside the console's section knobs, where
        // the list used to be.
        if extra {
            if v2 {
                let m = Rect::new(left + 57.0, ly.y(PANEL_Y0) + 4.0, 20.0, (ly.y(METER_Y1) - ly.y(PANEL_Y0) - 8.0).max(24.0));
                meter_body(ui, s, n, m, lemon, Some(ly));
            } else {
                let top = ly.y(TRACK_Y0);
                meter_body(ui, s, n, Rect::new(mx + 13.0, top, 22.0, (track_y1 - top).max(24.0)), lemon, None);
            }
        }
    } else {
        // Meter slot with its tick marks; the selected strip gets a green frame.
        meter_body(ui, s, n, meter, lemon, Some(ly));
        if selected {
            let (rim, rim_w) = if lemon { (t.accent_dim, 1.0) } else { (t.accent, 2.0) };
            ui.c.stroke_rrect(Rect::new(meter.x - 1.0, meter.y - 2.0, meter.w + 2.0, meter.h + 4.0), if lemon { lk::r_btn(t) } else { 1.0 }, rim_w, rim);
        }

        // Name, rotated, reading upward from the bottom of the meter.
        let name = track.and_then(|m| m.name.clone()).unwrap_or_else(|| if n == 0 { "Master".into() } else { format!("Bus {n}") });
        let name_c = track
            .and_then(|m| m.color)
            .map(super::rgb)
            .map_or(if selected { ui.t.tx(0xaab2b7) } else if also { ui.t.tx(0xe4e8ea) } else { ui.t.s(0x7f868a) }, |c| {
                if also { c.mix(ui.t.tx(0xffffff), 0.5) } else { c.mix(ui.t.s(0x7f868a), 0.4) }
            });
        // Change icon... (Lemon Studio's design): the icon at the bottom of the name's column, the name above it.
        let icon = crate::playlist::window_icon(&s.project, &s.playlist.icons, crate::playlist::TRACK_ICON_KEY + usize::from(n));
        if let Some(ic) = icon {
            icons::draw(ui.c, ic, Rect::new(sx - 9.0, meter.bottom() - 12.0, 12.0, 12.0), name_c);
        }
        let lead = if icon.is_some() { 15.0 } else { 0.0 };
        // The clip follows the name's column, not a fixed 22 px from the strip's edge: a wider layout (Wide 2, Wide 3,
        // Wide alt) pushed the rotated name past the old clip and cut it off.
        ui.c.push_clip(Rect::new(sx - 7.0, ly.y(PANEL_Y0), 20.0, (ly.y(METER_Y1) - ly.y(PANEL_Y0)).max(0.0)));
        ui.c.text_vertical(Font::new(15), sx - 7.0, meter.bottom() - 1.0 - lead, name_c, &name);
        ui.c.pop_clip();
    }
    // The strip's own plugin list, right of its controls (the Michael Lane Edition's column). Extra large keeps it up
    // beside the console, down to the mute / pan row; V2 drops it into the room beside the fader, which frees the
    // console's side for the meter.
    if extra {
        // Extra large fills the strip's right column from the top. V2 keeps the level beside the console's knobs and
        // gives the plugin list the room beside it — its old slot, under the meter, left most of that column empty —
        // while plain Extra large gives the list the whole column and the level a lane beside the fader.
        let (top, h) = if v2 {
            (ly.y(PANEL_Y0) + 4.0, (track_y1 - ly.y(PANEL_Y0) - 4.0).max(19.0))
        } else {
            let top = ly.y(PANEL_Y0) + 4.0;
            (top, (ly.y(LED_Y) - 12.0 - top).max(19.0))
        };
        let (x, width) = (left + 79.0, (w - 82.0).max(18.0));
        plugin_list(ui, s, n, Rect::new(x, top, width, h));
    }

    let enabled = track.is_none_or(|m| m.enabled);
    ui.hit(Rect::new(kx - 7.0, ly.y(LED_Y) - 7.0, 14.0, 14.0), Action::Mix(MHit::Led(n)), "Mute / solo");
    crate::views::mixer_finish::lamp(ui, kx, ly.y(LED_Y), enabled);
    let pan = KnobId::MixPan(n);
    ui.hit(Rect::new(kx - 14.0, ly.y(PAN_Y) - 14.0, 28.0, 28.0), Action::Knob(pan), s.knob_text(pan));
    mixer_knob(ui, kx, ly.y(PAN_Y), 10.5, 15.0, s.knob(pan), Some(true));
    mod_arcs(ui, s, os_model::target::mixer(n, os_model::target::MIXER_PAN), kx, ly.y(PAN_Y), 15.0, s.knob(pan).clamp(0.0, 1.0));

    // Reverse polarity (⇕) and swap left / right (⇔) buttons, lit when on, then the separation knob (§4).
    // They keep their size and spacing below the fader's top at any mixer height (FL's short mixer stacks them the
    // same way), so they are placed from the fader's top, not scaled with it.
    // View ▸ Extra properties off hides them (Lemon Studio's design).
    if s.mixer.extra {
        let (pol, swp) = (track.is_some_and(|m| m.polarity), track.is_some_and(|m| m.swap));
        let top = ly.y(TRACK_Y0);
        let (sep_y, swap_y, ring_y) = (top + SEP_Y - TRACK_Y0, top + SWAP_Y - TRACK_Y0, top + RING_Y - TRACK_Y0);
        let pr = Rect::new(sx - 8.0, sep_y - 8.0, 16.0, 16.0);
        let hp = ui.hit(pr, Action::Mix(MHit::Polarity(n)), "Reverse polarity");
        let mut g = Glyph::new();
        g.polygon(&[(sx - 5.0, sep_y - 1.5), (sx, sep_y - 6.0), (sx + 5.0, sep_y - 1.5)]);
        g.polygon(&[(sx - 5.0, sep_y + 1.5), (sx, sep_y + 6.0), (sx + 5.0, sep_y + 1.5)]);
        ui.c.glyph(if pol { ui.t.h(0xf3b234) } else if hp { ui.t.tx(0xffffff) } else { ui.t.tx(0xb7c3ca) }, &g);
        let wr = Rect::new(sx - 8.0, swap_y - 8.0, 16.0, 16.0);
        let hw = ui.hit(wr, Action::Mix(MHit::Swap(n)), "Swap left & right channels");
        let mut g = Glyph::new();
        g.polygon(&[(sx - 1.0, swap_y - 6.0), (sx - 7.0, swap_y), (sx - 1.0, swap_y + 6.0)]);
        g.polygon(&[(sx + 1.5, swap_y - 6.0), (sx + 7.5, swap_y), (sx + 1.5, swap_y + 6.0)]);
        ui.c.glyph(if swp { Color::rgb(0x5fb6f0) } else if hw { ui.t.tx(0xffffff) } else { ui.t.tx(0xb7c3ca) }, &g);
        ui.c.fill_rect(Rect::new(sx - 0.5, swap_y - 6.0, 1.0, 12.0), ui.t.s(0x5a6266));
        let sep = KnobId::MixSep(n);
        ui.hit(Rect::new(sx - 9.0, ring_y - 9.0, 18.0, 18.0), Action::Knob(sep), s.knob_text(sep));
        mixer_knob(ui, sx, ring_y, 6.0, 9.5, s.knob(sep), Some(true));
        mod_arcs(ui, s, os_model::target::mixer(n, os_model::target::MIXER_SEPARATION), sx, ring_y, 10.5, s.knob(sep).clamp(0.0, 1.0));
    }

    // Fader: a recessed 6 px channel with a darker groove down its middle, gain marks, and a handle that stands off
    // the strip (green on the selected strip, with the track lit below it).
    let track_y0 = ly.y(TRACK_Y0);
    ui.c.fill_rrect(Rect::new(mx - 3.0, track_y0, 6.0, track_y1 - track_y0), 3.0, ui.t.s(0x101213));
    ui.c.fill_rect(Rect::new(mx - 1.0, track_y0 + 2.0, 2.0, (track_y1 - track_y0 - 4.0).max(0.0)), ui.t.s(0x191b1d));
    fader_marks(ui, mx, track_y1, ly);
    let v = track.and_then(|m| m.volume).unwrap_or(0.8) as f32;
    // max/min, not clamp: a very short window gives track_y1 < ly.y(TRACK_Y0) (clamp would panic).
    let hy = (ly.y(HANDLE_0DB + (0.8 - v) * 440.0)).min(track_y1 - 5.0).max(ly.y(TRACK_Y0) + 5.0);
    if selected {
        if lemon {
            ui.c.fill_rrect(Rect::new(mx - 1.0, hy, 2.0, track_y1 - hy), 1.0, t.accent.with_alpha(185));
        } else {
            ui.c.fill_rect(Rect::new(mx - 1.0, hy, 2.0, track_y1 - hy), ui.t.a(0x8ec63a));
        }
    }
    let hov = ui.hit(Rect::new(mx - 9.0, ly.y(TRACK_Y0) - 5.0, 18.0, track_y1 - ly.y(TRACK_Y0) + 10.0), Action::Fader(n), mx::level_text("Volume", f64::from(v)));
    fader_handle(ui, mx, hy, selected, hov);
    // A driven fader's range beside the track, on the fader's own law; its handle drags the depth.
    let volume = os_model::target::mixer(n, os_model::target::MIXER_VOLUME);
    mod_fader(ui, s, volume, mx, v, |val| ly.y(HANDLE_0DB + (0.8 - val) * 440.0).clamp(track_y0 + 1.0, track_y1 - 1.0));

    // Input pin: not available yet, greyed out. The latency clock under it sets the track latency (right click); the
    // disk-record button below them arms the track.
    let master = n == 0;
    let by = bottom;
    let pin_c = if master { ui.t.h(0xf09a28) } else { ui.t.s(0x4d5357) };
    let mut g = Glyph::new();
    g.rrect(Rect::new(sx - 4.0, by - 146.0, 8.0, 9.0), 3.0);
    g.polygon(&[(sx - 2.5, by - 138.0), (sx + 2.5, by - 138.0), (sx, by - 133.0)]);
    ui.c.glyph(pin_c, &g);
    ui.c.fill_rect(Rect::new(sx - 1.0, by - 145.0, 2.0, 9.0), if master { ui.t.h(0x7a3c08) } else { ui.t.s(0x23272a) });
    ui.off(Rect::new(sx - 8.0, by - 148.0, 16.0, 18.0), "Track audio input");
    ui.veil(Rect::new(sx - 9.0, by - 149.0, 18.0, 20.0), body);
    // Orange on the master and on a track with a latency setting (spec: ui/mixer.md §4).
    let lat = track.map_or(0.0, |m| m.latency_out);
    let hov = ui.hit(Rect::new(sx - 8.0, by - 125.0, 16.0, 16.0), Action::Mix(MHit::Latency(n)), format!("Track latency  {}", mx::latency_text(lat)));
    let clk = if hov { ui.t.tx(0xffffff) } else if master || lat != 0.0 { ui.t.h(0xf09a28) } else { ui.t.tx(0xc4d4e0) };
    let mut g = Glyph::new();
    g.ring(sx, by - 117.0, 6.0, 1.6).rect(sx - 0.7, by - 121.0, 1.4, 4.5).rect(sx - 0.7, by - 117.7, 3.5, 1.4);
    ui.c.glyph(clk, &g);
    let hint = if armed { format!("Record to: {}", crate::track_rec::armed_file(s, n)) } else { "Arm disk recording".into() };
    let hov = ui.hit(Rect::new(sx - 7.0, by - 107.0, 14.0, 14.0), Action::Mix(MHit::Arm(n)), hint);
    if armed {
        // An orange-red ring about 8 px across with a 3 px hole.
        ui.c.stroke_circle(sx, by - 100.0, 2.75, 2.5, ui.t.h(0xff633f));
    } else {
        ui.c.fill_circle(sx, by - 100.0, 2.6, if hov { ui.t.tx(0xffffff) } else { ui.t.tx(0xd8e0e6) });
    }

    // Routing (spec: ui/mixer.md §5): the current track's green ⬇ (its cables are drawn over all strips: `cables`),
    // a send knob where the current track sends, else the ▲ "route to this track" button; an orange "^" marks tracks that receive a route.
    let ay = by - 38.0;
    let cur = s.selected_mixer;
    let cur_name = mx::name(s, cur);
    if selected {
        down_arrow(ui, kx, ay);
    } else if let Some(level) = mx::send(s, cur, n) {
        // Send knob (fader law) with its arc, the green "^" that removes the route; its cable is drawn with the others.
        let ky = by - 40.0;
        let id = KnobId::Send(cur, n);
        ui.hit(Rect::new(kx - 13.0, ky - 13.0, 26.0, 26.0), Action::Knob(id), s.knob_text(id));
        send_knob(ui, s, n, kx, ky, level);
        let ur = Rect::new(kx - 6.0, by - 61.0, 12.0, 8.0);
        let hov = ui.hit(ur, Action::Mix(MHit::Unroute(n)), format!("Remove the send from {cur_name} to {}", mx::name(s, n)));
        ui.c.fill_polygon(&[(kx - 4.0, by - 55.0), (kx, by - 59.0), (kx + 4.0, by - 55.0)], if hov { ui.t.tx(0xffffff) } else { t.accent });
    } else if !(master && cur == 0) {
        let hint = format!("Enable send from {cur_name} to {}  Ctrl/Shift", mx::name(s, n));
        let hov = ui.hit(Rect::new(kx - 8.0, ay - 8.0, 16.0, 14.0), Action::Mix(MHit::Route(n)), hint);
        let mut g = Glyph::new();
        g.polygon(&[(kx - 6.5, ay + 3.0), (kx, ay - 3.5), (kx + 6.5, ay + 3.0)]);
        ui.c.glyph(if hov { t.accent } else { ui.t.tx(0xd8e0e6) }, &g);
        if !master && mx::receives(s, n) {
            ui.c.fill_polygon(&[(kx - 3.5, ay - 9.0), (kx, ay - 12.5), (kx + 3.5, ay - 9.0)], ui.t.h(0xf09a28));
        }
    }
}

/// The current track's green ⬇ (centred on `ay`): where its cables leave.
fn down_arrow(ui: &mut Ui, kx: f32, ay: f32) {
    let a = [(kx - 3.0, ay - 6.0), (kx + 3.0, ay - 6.0), (kx + 3.0, ay - 1.0), (kx + 7.0, ay - 1.0), (kx, ay + 6.0), (kx - 7.0, ay - 1.0), (kx - 3.0, ay - 1.0)];
    ui.c.fill_polygon(&a, ui.t.accent);
}

/// The send knob from the current track into track `n`, with its level arc (none at zero).
fn send_knob(ui: &mut Ui, s: &State, n: u16, kx: f32, ky: f32, level: f64) {
    let id = KnobId::Send(s.selected_mixer, n);
    mixer_knob(ui, kx, ky, 10.0, 13.0, s.knob(id), None);
    mod_arcs(ui, s, os_model::target::send(s.selected_mixer, n), kx, ky, 14.0, s.knob(id).clamp(0.0, 1.0));
    if level > 0.0 {
        // From the knob's zero to its pointer (the arc takes a sweep, not an end angle).
        let sweep = (level as f32).min(1.0) * 1.5 * PI;
        ui.c.arc(kx, ky, 12.5, 0.75 * PI, sweep, 2.0, ui.t.accent);
    }
}

/// The dark round mixer knob: a shaded ball inside a darker halo.
/// `arc`: Some(true) draws the red value arc from the top (pan style), Some(false) from the start.
fn mixer_knob(ui: &mut Ui, cx: f32, cy: f32, r: f32, halo: f32, value: f32, arc: Option<bool>) {
    let hov = ui.hovering(Rect::new(cx - halo, cy - halo, 2.0 * halo, 2.0 * halo));
    let finish = crate::views::mixer_finish::Finish::current();
    if finish.on() {
        return crate::views::mixer_finish::knob(ui, cx, cy, r, value, arc.unwrap_or(false), hov);
    }
    let _ = halo;
    analog_knob(ui, cx, cy, r, value, arc.unwrap_or(false), ui.t.accent, hov);
}

/// Gain marks beside a fader track (spec: ui/mixer.md §5, measured from FL): 6 px ticks on both sides at 0, −6, −12 and
/// −18 dB (2 px clear of the track), and faint 4 px rails between consecutive ticks (1 px clear of each tick). They
/// belong to the track: the handle covers them as it moves.
fn fader_marks(ui: &mut Ui, mx: f32, track_y1: f32, ly: &Ly) {
    let ys: Vec<f32> = [0.0, -3.0, -6.0, -9.0, -12.0, -15.0, -18.0, -24.0]
        .iter()
        .map(|&db| ly.y(HANDLE_0DB + (0.8 - fader_value(db)) * 440.0).floor())
        .filter(|&y| y < track_y1 - 1.0)
        .collect();
    let (tick, rail) = (ui.t.s(0x181a1c), ui.t.s(0x202324));
    for w in ys.windows(2) {
        let (a, b) = (w[0] + 2.0, w[1] - 1.0);
        if b > a {
            ui.c.fill_rect(Rect::new(mx - 7.5, a, 4.0, b - a), rail);
            ui.c.fill_rect(Rect::new(mx + 3.5, a, 4.0, b - a), rail);
        }
    }
    for y in ys {
        ui.c.fill_rect(Rect::new(mx - 9.5, y, 6.0, 1.0), tick);
        ui.c.fill_rect(Rect::new(mx + 3.5, y, 6.0, 1.0), tick);
    }
}

/// Fader value (0..1, 0.8 = 0 dB) of a gain in dB: the inverse of `laws::fader_gain`.
fn fader_value(db: f32) -> f32 {
    0.8 * (10.0 * 10f32.powf(db / 20.0) + 1.0).ln() / 11f32.ln()
}

/// Fader handle centred on (cx, cy): a 19 × 26 cap with three milled grooves and a centre line, seated in a recess —
/// a physical fader's cap rather than a small slider. Green on the selected strip.
fn fader_handle(ui: &mut Ui, cx: f32, cy: f32, selected: bool, hov: bool) {
    if crate::views::mixer_finish::Finish::current().on() {
        return crate::views::mixer_finish::fader(ui, cx, cy, selected, hov);
    }
    let t = ui.t;
    let h = Rect::new(cx - 9.0, cy - 12.5, 18.0, 25.0);
    ui.c.fill_rrect(h.outset(1.5), 4.0, t.s(0x0c0e0f));
    let (top, mid, low) = if selected {
        (Color::rgb(0xc8f56e), Color::rgb(0x8fd236), Color::rgb(0x3f7a10))
    } else if hov {
        (t.s(0x7a7e81), t.s(0x55595c), t.s(0x2a2d30))
    } else {
        (t.s(0x6a6e71), t.s(0x46494c), t.s(0x222528))
    };
    ui.c.fill_rect_gradient(Rect::new(h.x, h.y, h.w, 12.0), top, mid, 3.0);
    ui.c.fill_rect_gradient(Rect::new(h.x, h.y + 12.0, h.w, 13.0), mid.shade(-0.25), low, 3.0);
    let groove = mid.shade(-0.45);
    for k in 0..3 {
        ui.c.fill_rect(Rect::new(h.x + 3.0, h.y + 4.5 + 3.5 * k as f32, h.w - 6.0, 1.0), groove);
    }
    ui.c.fill_rect(Rect::new(h.x, cy, h.w, 1.5), if selected { t.s(0x1f3d08) } else { t.s(0x151719) });
}

fn slot_panel(ui: &mut Ui, s: &State, r: Rect) {
    let t = ui.t;
    ui.c.fill_rect(r, ui.t.s(0x2e3133));
    ui.c.fill_rect(Rect::new(r.x - 1.0, r.y, 1.0, r.h), ui.t.s(0x1b1e20));
    ui.hit(r, Action::Swallow, "");
    let n = s.selected_mixer;
    let track = s.project.mixer.iter().find(|m| m.index == n);
    let title = match track.and_then(|m| m.name.clone()) {
        Some(nm) => format!("Mixer - {nm}"),
        None if n == 0 => "Mixer - Master".into(),
        None => format!("Mixer - Bus {n}"),
    };
    ui.c.fill_rect(Rect::new(r.x, r.y, r.w, 23.0), t.window_title);
    ui.c.text_clip(Font::new(17), r.x + 1.0, r.y + 2.0, r.w - 70.0, ui.t.tx(0xd4dade), &title);
    ui.hit(Rect::new(r.x, r.y, r.w, 23.0), Action::WinDrag(Win::Mixer), "Drag to move");
    window_buttons(ui, Win::Mixer, r.right() - 58.0, r.y, ui.t.tx(0xd4dade), ui.t.tx(0xffffff));
    if r.h < 2.0 * TITLE_H {
        return;
    }
    // Squish: the slot rows and the equalizer graph shrink with the height (the knob rows under
    // the graph are dropped when they no longer fit); the top and bottom rows keep their size.
    let room = r.h - 125.0;
    let kp = ((room - 15.0) / 503.0).clamp(0.3, 1.0);
    let knobs = room >= 579.0;
    let q = kp.max(0.55);
    // Input row: the record-source icon with its label, the input combo, the input latency clock (not available yet:
    // greyed out). Spec: audio/track-io.md §1.1, §2.3.
    let input_bg = ui.t.s(0x2b2e30);
    ui.c.fill_rect(Rect::new(r.x, r.y + 23.0, r.w, 36.0), input_bg);
    let io = track.map_or((-1, os_model::REC_POST_EQ), |m| (m.audio_in, m.rec_source));
    let icon = Rect::new(r.x + 3.0, r.y + 28.0, 21.0, 23.0);
    let hov = ui.hit(icon, Action::Mix(MHit::RecSource), "Select what is included when recording audio");
    // Grey while no input is chosen, orange once one is.
    let ink = if hov { ui.t.tx(0xffffff) } else if io.0 >= 0 { ui.t.h(0xf09a28) } else { ui.t.s(0x5a6266) };
    let mut g = Glyph::new();
    g.polygon(&[(r.x + 7.0, r.y + 32.0), (r.x + 13.0, r.y + 32.0), (r.x + 18.0, r.y + 37.5), (r.x + 13.0, r.y + 43.0), (r.x + 7.0, r.y + 43.0)]);
    ui.c.glyph(ink, &g);
    let label = mx::REC_SOURCES.get(usize::from(io.1)).map_or("", |s| s.1);
    let lf = Font::new(9).bold();
    let lw = ui.c.text_width(lf, label);
    ui.c.text(lf, r.x + 12.5 - lw / 2.0, r.y + 43.0, ink, label);
    let (text, any) = mx::input_label(s, n);
    let combo = Rect::new(r.x + 27.0, r.y + 28.0, 159.0, 23.0);
    if any {
        let hov = ui.hit(combo, Action::Mix(MHit::Input), "Audio input source");
        os_ui::kit::combo(ui.c, ui.t, combo, &text, Font::new(17), os_ui::kit::State::hover(hov));
    } else {
        off_combo(ui, combo, "Audio input source", &text, input_bg);
    }
    let clock = Rect::new(r.x + 192.0, r.y + 29.0, 21.0, 21.0);
    let lat = mx::track(s, n).map_or(0.0, |m| m.latency_in);
    let hov = ui.hit(clock, Action::Mix(MHit::InLatency), format!("Input latency  {}", mx::latency_text(lat)));
    let mut g = Glyph::new();
    g.ring(r.x + 202.0, r.y + 39.5, 8.5, 1.8).rect(r.x + 201.2, r.y + 34.0, 1.6, 6.5).rect(r.x + 201.2, r.y + 38.9, 5.0, 1.6);
    ui.c.glyph(if hov { ui.t.tx(0xffffff) } else if lat != 0.0 { ui.t.h(0xf09a28) } else { ui.t.tx(0xd4dade) }, &g);

    // Ten effect slots.
    for i in 0..10 {
        let row = Rect::new(r.x + 1.0, r.y + 61.0 + 33.0 * kp * i as f32, r.w - 2.0, 28.0 * kp);
        let k = i as u8;
        let plugin = mx::slot_name(s, n, k);
        let fs = (18.0 * kp.max(0.62)).round() as u16;
        let hint = format!("FX slot {} options", i + 1);
        let ha = ui.hit(Rect::new(row.x, row.y, 19.0, row.h), Action::Mix(MHit::SlotArrow(k)), hint.clone());
        let hn = ui.hit(Rect::new(row.x + 19.0, row.y, row.w - 64.0, row.h), Action::Mix(MHit::SlotName(k)), hint);
        let hov = ha || hn;
        if super::lemon(ui) {
            let face = if hov { ui.t.s(0x2f3335) } else { ui.t.s(0x2a2d2f) };
            lk::outlined(ui.c, Rect::new(row.x + 2.0, row.y, row.w - 4.0, row.h), lk::r_btn(t).min(row.h / 2.0), ui.t.s(0x1f2224), face);
        } else {
            ui.c.fill_rect(row, ui.t.s(0x1f2224));
            ui.c.fill_rect(row.inset(1.0), if hov { ui.t.s(0x2f3335) } else { ui.t.s(0x2a2d2f) });
        }
        // A coloured slot (Lemon Studio's design): a 3 px bar of its colour at the row's left edge.
        if let Some(c) = mx::slot_color(s, n, k) {
            ui.c.fill_rect(Rect::new(row.x + 3.0, row.y + 4.0 * kp, 3.0, row.h - 8.0 * kp), super::rgb(c));
        }
        chevron(ui, row.x + 10.0, row.cy(), true, if ha { ui.t.tx(0xffffff) } else { ui.t.tx(0xc4ccd0) });
        // An empty slot asks for an effect: its whole row is the add button (the palette opens for that slot), so it
        // says so instead of naming a blank, and its mix knob and LED stay out of the way until it holds something.
        let empty = plugin.is_none();
        let (f, name, ink) = match &plugin {
            Some(p) => (Font::new(fs), crate::plugins::slot_short(p), ui.t.tx(0xc9d0d4)),
            None => (Font::new(fs), "+  Add effect".to_string(), ui.t.s(0x7d858a)),
        };
        // Lemon centres the name's capitals on the row, level with the chevron, knob and LED: the baseline sits half a
        // cap height (the face's is about 0.72 of its size) under the middle, measured on the face drawn (the italic
        // one has its own ascent). Classic keeps FL's place.
        let ty = if super::lemon(ui) { (row.cy() + 0.36 * f32::from(fs) - ui.c.ascent(f)).round() } else { row.y + 4.0 * kp };
        // Change icon... (Lemon Studio's design): the icon before the name.
        let icon = crate::playlist::window_icon(&s.project, &s.playlist.icons, crate::plugins::fx_key(n, k));
        if let Some(ic) = icon {
            let sz = (12.0 * kp.max(0.62)).round();
            icons::draw(ui.c, ic, Rect::new(row.x + 19.0, row.cy() - sz / 2.0, sz, sz), ink);
        }
        let lead = if icon.is_some() { (16.0 * kp.max(0.62)).round() } else { 0.0 };
        ui.c.text_clip(f, row.x + 19.0 + lead, ty, 140.0 - lead, ink, &name);
        if !empty {
            let mix = KnobId::SlotMix(n, k);
            ui.hit(Rect::new(row.right() - 46.0, row.y, 26.0, row.h), Action::Knob(mix), s.knob_text(mix));
            mixer_knob(ui, row.right() - 33.0, row.cy(), 10.5 * q, 12.0 * q, s.knob(mix), Some(false));
            ui.c.stroke_circle(row.right() - 33.0, row.cy(), 11.8 * q, 1.5, ui.t.s(0x55595c));
            ui.hit(Rect::new(row.right() - 18.0, row.y, 18.0, row.h), Action::Mix(MHit::SlotLed(k)), "Mute / solo  (ctrl)0..9");
            led(ui, row.right() - 10.0, row.cy(), track.is_none_or(|m| m.slot_on[i]));
            // Last, so a depth handle wins the pointer where its arc reaches the LED's column.
            mod_arcs(ui, s, os_model::target::slot(n, k, os_model::target::SLOT_MIX), row.right() - 33.0, row.cy(), 15.0 * q, s.knob(mix).clamp(0.0, 1.0));
        }
    }
    let sep = r.y + 61.0 + 330.0 * kp;
    ui.c.fill_rect(Rect::new(r.x, sep, r.w, 1.0), ui.t.s(0x1f2224));

    // Equalizer: response graph, three band gain faders (±18 dB), band frequency and width knobs (spec: ui/mixer.md §8).
    let eq = Rect::new(r.x + 4.0, sep + 11.0, 146.0, 173.0 * kp);
    let settings = track.map_or(os_model::EQ_DEFAULT, |m| m.eq);
    // Its hint follows the cursor (App::mouse_move).
    let hint = if eq.contains(ui.mouse.0, ui.mouse.1) { mx::eq_graph_hint(s, eq, ui.mouse.0) } else { String::new() };
    ui.hit(eq, Action::Mix(MHit::EqGraph), hint);
    if super::lemon(ui) {
        lk::outlined(ui.c, eq, lk::r_btn(t), ui.t.s(0x1f2224), ui.t.s(0x272a2c));
    } else {
        ui.c.fill_rect(eq, ui.t.s(0x1f2224));
        ui.c.fill_rect(eq.inset(1.0), ui.t.s(0x272a2c));
    }
    for k in 1..5 {
        ui.c.fill_rect(Rect::new(eq.x + 14.0 + 31.0 * k as f32, eq.y + 1.0, 1.0, eq.h - 2.0), ui.t.s(0x2b2e30));
    }
    // The response curve: the three bands' summed gain over the graph's width, which spans the frequency knob's
    // 10 Hz .. 16 kHz law; ±18 dB fills the graph's half-height.
    let half = eq.h / 2.0 - 4.0;
    let steps = (eq.w - 2.0) as usize;
    let pts: Vec<(f32, f32)> = (0..=steps)
        .map(|i| {
            let p = i as f32 / steps as f32;
            let hz = os_dsp::mixer::eq_frequency((p * 65536.0).round() as i32);
            let db = os_dsp::mixer::eq_response_db(settings, 44100, f64::from(hz)) as f32;
            (eq.x + 1.0 + p * (eq.w - 2.0), eq.cy() - (db / 18.0).clamp(-1.0, 1.0) * half) // as mx::eq_graph_db
        })
        .collect();
    ui.c.push_clip(eq.inset(1.0));
    ui.c.polyline(&pts, 2.0, ui.t.tx(0xc9cfd3));
    ui.c.pop_clip();
    for k in 0..3 {
        let fx = r.x + 162.0 + 22.0 * k as f32;
        ui.c.fill_rect(Rect::new(fx - 2.0, eq.y + 1.0, 4.0, eq.h - 2.0), ui.t.s(0x17191b));
        ui.c.fill_rect(Rect::new(fx - 9.0, eq.y + 36.0 * kp, 18.0, 1.0), ui.t.s(0x3a3e41));
        ui.c.fill_rect(Rect::new(fx - 9.0, eq.y + 136.0 * kp, 18.0, 1.0), ui.t.s(0x3a3e41));
        ui.c.fill_rect(Rect::new(fx - 9.0, eq.cy(), 18.0, 1.0), ui.t.s(0x111315));
        let id = KnobId::Eq(n, k as u8, 0);
        let hov = ui.hit(Rect::new(fx - 9.0, eq.y, 18.0, eq.h), Action::Knob(id), s.knob_text(id));
        // The handle travels ±(half-height − 8) for ±18 dB.
        let hy = eq.cy() - (s.knob(id) * 2.0 - 1.0) * (eq.h / 2.0 - 8.0);
        let h = Rect::new(fx - 6.0, hy - 6.0, 12.0, 13.0);
        let face = if hov { 0x585c5f } else { 0x4a4e51 };
        if super::lemon(ui) {
            ui.c.fill_rrect(h.outset(1.0), lk::r_small(t), ui.t.s(0x121416));
            ui.c.fill_rrect(h, lk::r_small(t) - 1.0, ui.t.s(face));
        } else {
            ui.c.fill_rrect(h.outset(1.0), 2.0, ui.t.s(0x121416));
            ui.c.fill_rect_gradient(h, ui.t.s(if hov { 0x6c7073 } else { 0x5e6265 }), ui.t.s(0x2a2d30), 2.0);
        }
        ui.c.fill_rect(Rect::new(fx - 5.0, hy, 10.0, 1.0), ui.t.s(0x111315));
        // The band's gain fader: its range lies along the fader's own value law.
        let gain = os_model::target::mixer(n, os_model::target::EQ_GAIN + k as u8);
        mod_fader(ui, s, gain, fx, s.knob(id).clamp(0.0, 1.0), |v| eq.cy() - (v.clamp(0.0, 1.0) * 2.0 - 1.0) * (eq.h / 2.0 - 8.0));
    }
    let ly = eq.bottom() + 8.0;
    if knobs {
        ui.c.text(Font::new(17), r.x + 4.0, ly, ui.t.tx(0xa9b2b7), "Equalizer");
    }
    for row in 0..2 {
        if !knobs {
            break;
        }
        for k in 0..3 {
            let (kx, ky) = (r.x + 162.0 + 22.0 * k as f32, ly + 20.0 + 20.0 * row as f32);
            let id = KnobId::Eq(n, k as u8, 1 + row as u8);
            ui.hit(Rect::new(kx - 9.0, ky - 9.0, 18.0, 18.0), Action::Knob(id), s.knob_text(id));
            mixer_knob(ui, kx, ky, 7.0, 8.0, s.knob(id), Some(false));
            let param = if row == 0 { os_model::target::EQ_FREQ } else { os_model::target::EQ_WIDTH } + k as u8;
            mod_arcs(ui, s, os_model::target::mixer(n, param), kx, ky, 11.5, s.knob(id).clamp(0.0, 1.0));
        }
    }
    let mut g = Glyph::new();
    for k in 0..3 {
        g.rect(r.x + 141.0 + 3.0 * k as f32, ly + 16.0, 1.4, 6.0);
    }
    g.rect(r.x + 141.0, ly + 21.0, 8.0, 1.4);
    let ay = ly + 40.0;
    g.line(r.x + 137.0, ay, r.x + 149.0, ay, 1.2);
    g.polygon(&[(r.x + 135.0, ay), (r.x + 139.0, ay - 3.0), (r.x + 139.0, ay + 3.0)]).polygon(&[(r.x + 151.0, ay), (r.x + 147.0, ay - 3.0), (r.x + 147.0, ay + 3.0)]);
    if knobs {
        ui.c.glyph(ui.t.tx(0x8c969c), &g);
    }
    let b = r.bottom();

    // Output rows at the bottom: the track latency combo, then the output combo.
    ui.c.fill_rect(Rect::new(r.x, b - 66.0, r.w, 1.0), ui.t.s(0x1f2224));
    ui.c.fill_rect(Rect::new(r.x, b - 33.0, r.w, 1.0), ui.t.s(0x1f2224));
    let lat = mx::track(s, n).map_or(0.0, |m| m.latency_out);
    let hov = ui.hit(Rect::new(r.x + 3.0, b - 60.0, r.w - 6.0, 22.0), Action::Mix(MHit::OutLatency), format!("Track latency  {}", mx::latency_text(lat)));
    let mut g = Glyph::new();
    g.ring(r.x + 14.0, b - 48.5, 8.0, 1.5).rect(r.x + 13.3, b - 53.0, 1.4, 5.0).rect(r.x + 13.3, b - 49.2, 4.0, 1.4);
    ui.c.glyph(if lat != 0.0 { ui.t.h(0xf09a28) } else { ui.t.tx(0xc4ccd0) }, &g);
    ui.c.text(Font::new(17), r.x + 30.0, b - 59.0, if hov { ui.t.tx(0xffffff) } else { ui.t.tx(0xd4dade) }, &mx::latency_text(lat));
    chevron(ui, r.right() - 11.0, b - 48.5, true, if hov { ui.t.tx(0xffffff) } else { ui.t.tx(0x8c969c) });
    let mut g = Glyph::new();
    g.ring(r.x + 13.0, b - 16.0, 8.0, 1.6);
    g.erase(true).rect(r.x + 13.0, b - 26.0, 10.0, 20.0);
    g.erase(false).polygon(&[(r.x + 11.0, b - 20.5), (r.x + 16.0, b - 20.5), (r.x + 21.0, b - 16.0), (r.x + 16.0, b - 11.5), (r.x + 11.0, b - 11.5)]);
    ui.c.glyph(ui.t.tx(0xc4ccd0), &g);
    let out = Rect::new(r.x + 26.0, b - 28.0, 189.0, 24.0);
    let hov = ui.hit(out, Action::Mix(MHit::Output), "Audio output target");
    os_ui::kit::combo(ui.c, ui.t, out, &mx::output_label(s, n), Font::new(17), os_ui::kit::State::hover(hov));
}

/// An input combo with nothing to choose (no device inputs): drawn greyed out with `text`, its hit only says so.
fn off_combo(ui: &mut Ui, r: Rect, name: &'static str, text: &str, bg: Color) {
    ui.off(r, name);
    let st = os_ui::kit::State::NORMAL.with_disabled(true);
    os_ui::kit::combo(ui.c, ui.t, r, text, Font::new(17), st);
    ui.veil(r, bg);
}

/// Small solid chevron centred on (x, y), pointing right or left.
pub fn chevron(ui: &mut Ui, x: f32, y: f32, right: bool, c: Color) {
    let d = if right { 1.0 } else { -1.0 };
    let mut g = Glyph::new();
    g.polygon(&[(x - 2.5 * d, y - 4.5), (x + 2.5 * d, y), (x - 2.5 * d, y + 4.5)]);
    ui.c.glyph(c, &g);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::control::command::Command;

    fn app() -> App {
        let mut app = App::new(None);
        app.state.project = crate::app::test_project();
        app
    }

    fn track_create(s: &mut crate::app::State, kind: &str) {
        crate::control::apply(s, &crate::control::By::User, None, vec![Command::TrackCreate { kind: kind.into(), name: None, color: None }]).unwrap();
    }

    /// Wide layouts (Wide 2 and up) draw the strip's console panel: its switch and its section knobs are hit targets;
    /// the default Wide layout keeps the meter until the strip's own panel button asks for the console.
    #[test]
    fn wide_strips_show_the_console() {
        let mut app = app();
        let Ok(mut c) = os_ui::Canvas::new(1920, 1080, 1.0) else { return };
        let ws = &mut app.state.wins[crate::app::Win::Mixer.slot().unwrap()];
        (ws.maxed, ws.rect) = (false, os_ui::Rect::new(100.0, 83.0, 1200.0, 900.0));
        app.state.z = vec![crate::app::Win::Mixer];
        let has = |a: &App, want: Action| a.hits.iter().any(|h| h.action == want);
        let paint = |app: &mut App, c: &mut os_ui::Canvas| app.paint(c, None);
        app.state.mixer.layout = 4; // Wide 2, 64 px
        paint(&mut app, &mut c);
        assert!(has(&app, Action::Mix(MHit::StripPower(2))), "the strip's switch");
        assert!(has(&app, Action::Knob(KnobId::Strip(2, 7))), "the table's knob");
        assert!(has(&app, Action::Knob(KnobId::Strip(2, 1))), "the compressor's, too");
        app.state.mixer.layout = 3; // Wide, 44 px
        paint(&mut app, &mut c);
        assert!(!has(&app, Action::Mix(MHit::StripPower(2))), "narrow strips keep the meter");
        assert!(has(&app, Action::Mix(MHit::StripPanel(2))), "the panel button is on every strip");
        // The button shows the compact console on that one channel, and hides a wide layout's again.
        crate::mixer::hit(&mut app, MHit::StripPanel(2));
        paint(&mut app, &mut c);
        assert!(has(&app, Action::Mix(MHit::StripPower(2))), "shown for that channel alone");
        assert!(has(&app, Action::Knob(KnobId::Strip(2, 0))), "its low cut");
        assert!(!has(&app, Action::Mix(MHit::StripPower(3))), "the neighbour is untouched");
        app.state.mixer.layout = 4;
        paint(&mut app, &mut c);
        crate::mixer::hit(&mut app, MHit::StripPanel(2));
        paint(&mut app, &mut c);
        assert!(!has(&app, Action::Mix(MHit::StripPower(2))), "hidden on that channel");
        assert!(has(&app, Action::Mix(MHit::StripPower(3))), "the rest follow the layout");
        assert!(has(&app, Action::Fader(2)));
        // A strip's style bar swaps the analog sections for the glitch grid: sixteen cells and the grid's amount, and
        // no section knob, since a glitch module is placed in the grid rather than turned up.
        crate::mixer::hit(&mut app, MHit::GlitchMode(3));
        paint(&mut app, &mut c);
        assert!(has(&app, Action::Mix(MHit::GlitchCell(3, 0))), "the grid's first step");
        assert!(has(&app, Action::Mix(MHit::GlitchCell(3, 15))), "its last");
        assert!(has(&app, Action::Knob(KnobId::Glitch(3))), "the grid's amount");
        assert!(!has(&app, Action::Knob(KnobId::Strip(3, 0))), "the analog sections are away");
        assert!(has(&app, Action::Mix(MHit::GlitchMode(3))), "and the bar stays, to go back");
        // The playhead: the audio thread's step rings its cell, and a drag over the grid finds the step under the
        // pointer from any cell's rect (`glitch_at`), which is what paints the cells it crosses.
        app.state.mixer.steps = vec![None, None, None, Some(5)];
        paint(&mut app, &mut c);
        let cell = |app: &App, i: u8| app.hits.iter().find(|h| h.action == Action::Mix(MHit::GlitchCell(3, i))).map(|h| h.rect).expect("a grid cell");
        let five = cell(&app, 5);
        assert_eq!(glitch_at(five, 5, (five.cx(), five.cy())), 5, "its own cell");
        assert_eq!(glitch_at(five, 5, (five.cx() + five.w * 2.0, five.cy())), 7, "two cells to the right");
        assert_eq!(glitch_at(five, 5, (five.cx(), five.cy() - five.h * 2.0)), 1, "a row up");
        assert_eq!(glitch_at(five, 5, (five.cx() - five.w * 9.0, five.cy())), 4, "and a point past the left edge takes its column's first cell");
    }

    /// A driven mixer control draws its depth handle where the modulation is set, and dragging it changes that depth
    /// (the strip's knob and its volume fader both).
    #[test]
    fn a_driven_knob_and_fader_have_depth_handles() {
        use crate::modulation::{ModAct, ModCmd};
        let mut app = app();
        let Ok(mut c) = os_ui::Canvas::new(1600, 1000, 1.0) else { return };
        let ws = &mut app.state.wins[crate::app::Win::Mixer.slot().unwrap()];
        (ws.maxed, ws.rect) = (false, os_ui::Rect::new(100.0, 83.0, 1200.0, 900.0));
        app.state.z = vec![crate::app::Win::Mixer];
        let id = app.state.modulation.add(os_model::modulation::ModSource::default());
        let pan = os_model::target::mixer(2, os_model::target::MIXER_PAN);
        let volume = os_model::target::mixer(2, os_model::target::MIXER_VOLUME);
        crate::modulation::run(&mut app, ModCmd::Assign(id, pan)).expect("pan");
        crate::modulation::run(&mut app, ModCmd::Assign(id, volume)).expect("volume");
        app.paint(&mut c, None);
        let handle = |app: &App, target: u32| {
            app.hits
                .iter()
                .find(|h| h.action == Action::Mod(ModAct::DepthDrag(id, target)))
                .map(|h| h.rect)
                .unwrap_or_else(|| panic!("a depth handle for {target:#x}"))
        };
        let pan_h = handle(&app, pan);
        handle(&app, volume);
        let depth = |app: &App| {
            let path = crate::modulation::path_in(&app.state, pan).expect("the pan's path");
            app.state.modulation.get(id).and_then(|m| m.targets.iter().find(|t| t.path == path)).map_or(0.0, |t| t.depth)
        };
        let before = depth(&app);
        app.mouse_move(pan_h.cx(), pan_h.cy());
        app.mouse_down();
        app.mouse_move(pan_h.cx(), pan_h.cy() - 30.0);
        app.mouse_up();
        assert!(depth(&app) > before, "dragging the handle up raises the depth ({} -> {})", before, depth(&app));
    }

    /// A modulator chip in the title bar, dropped on a pan knob, assigns that modulator to the knob's parameter: the
    /// engine gets it, and the drop is one undo step.
    #[test]
    fn a_modulator_chip_dropped_on_a_knob_drives_it() {
        let mut app = app();
        let Ok(mut c) = os_ui::Canvas::new(1600, 1000, 1.0) else { return };
        let ws = &mut app.state.wins[crate::app::Win::Mixer.slot().unwrap()];
        (ws.maxed, ws.rect) = (false, os_ui::Rect::new(100.0, 83.0, 1200.0, 900.0));
        app.state.z = vec![crate::app::Win::Mixer];
        let id = app.state.modulation.add(os_model::modulation::ModSource::default());
        app.paint(&mut c, None);
        let find = |app: &App, want: &crate::app::Action| app.hits.iter().find(|h| &h.action == want).map(|h| h.rect).expect("a hit");
        let chip = find(&app, &crate::app::Action::Modulator(id));
        let pan = find(&app, &crate::app::Action::Knob(KnobId::MixPan(2)));
        app.mouse_move(chip.cx(), chip.cy());
        app.mouse_down();
        assert!(matches!(app.drag, Some(crate::app::Drag::Modulator { .. })), "the chip starts a drag");
        app.mouse_move(pan.cx(), pan.cy());
        app.mouse_up();
        assert_eq!(app.state.modulation.get(id).map(|m| m.targets.len()), Some(1), "dropped on the pan knob");
        let heard = crate::modulation::engine_spec(&app.state, &app.plugins).0[0].targets[0].target;
        assert_eq!(heard, os_model::target::decode(os_model::target::mixer(2, os_model::target::MIXER_PAN)));
        // One step back takes the assignment away; the modulator stays.
        crate::playlist::run(&mut app, crate::playlist::Cmd::Undo);
        assert_eq!(app.state.modulation.get(id).map(|m| m.targets.len()), Some(0));
    }

    /// The chain bar is the chain: a chip per module in signal order, and one tile per slot in the body — a section's
    /// row, or the grid's 4×4 four rows tall — so a mixed strip is visible as well as audible. A chip's click takes its
    /// module out, one undo step, and a narrow console shows the count instead of chips it could not make clickable.
    #[test]
    fn the_chain_bar_and_tiles_follow_the_chain() {
        let mut app = app();
        let Ok(mut c) = os_ui::Canvas::new(1920, 1080, 1.0) else { return };
        let ws = &mut app.state.wins[crate::app::Win::Mixer.slot().unwrap()];
        (ws.maxed, ws.rect) = (false, os_ui::Rect::new(100.0, 83.0, 1200.0, 900.0));
        app.state.z = vec![crate::app::Win::Mixer];
        // Wide 3's 80 px strip leaves the console 60 px beside the meter: wide enough for the chips, where Wide 2's
        // 64 px strip leaves it 44 and the bar reads its count instead.
        app.state.mixer.layout = 5;
        let has = |a: &App, want: Action| a.hits.iter().any(|h| h.action == want);
        let chip = |a: &App, m: u8| a.hits.iter().find(|h| h.action == Action::Mix(MHit::ChainSlot(2, m))).map(|h| h.rect).expect("a chain chip");
        let knob = |a: &App, word: u8| a.hits.iter().find(|h| h.action == Action::Knob(KnobId::Strip(2, word))).map(|h| h.rect);
        app.paint(&mut c, None);
        // The analog strip is its six sections, in the Edition's order.
        for module in 1..=6u8 {
            assert!(has(&app, Action::Mix(MHit::ChainSlot(2, module))), "chip {module}");
        }
        assert!(!has(&app, Action::Mix(MHit::ChainSlot(2, 7))), "no grid on the analog chain");
        assert!(chip(&app, 1).x < chip(&app, 3).x, "the chips follow the chain");
        let (cut, drive) = (knob(&app, 0).expect("low cut"), knob(&app, 2).expect("drive"));
        assert!(cut.y < drive.y, "and the tiles do: low cut before drive");
        // A mixed chain: the grid joins and its 4×4 draws in the body beside the sections.
        app.state.project.mixer.iter_mut().find(|m| m.index == 2).unwrap().chain = [1, 3, 7, 0, 0, 0, 0, 0];
        app.paint(&mut c, None);
        assert!(has(&app, Action::Mix(MHit::GlitchCell(2, 0))), "the grid's tile");
        assert!(has(&app, Action::Mix(MHit::GlitchCell(2, 15))), "its last step");
        assert!(has(&app, Action::Mix(MHit::GlitchRandom(2))), "its new-pattern button");
        assert!(has(&app, Action::Mix(MHit::GlitchClear(2))), "and its clear button");
        assert!(has(&app, Action::Mix(MHit::ChainSlot(2, 7))), "its chip too");
        assert!(knob(&app, 2).is_some(), "the drive's tile stays");
        // A chip takes its module out, one undo step, and its tile goes with it.
        let at = app.state.history.at();
        crate::mixer::hit(&mut app, MHit::ChainSlot(2, 3));
        assert_eq!(app.state.project.mixer.iter().find(|m| m.index == 2).unwrap().chain, [1, 7, 0, 0, 0, 0, 0, 0]);
        assert_eq!(app.state.history.at(), at + 1);
        app.paint(&mut c, None);
        assert!(knob(&app, 2).is_none(), "the drive's tile is gone");
        // Narrow: seven chips would not be clickable, so the bar reads its count and the menu is the way in.
        app.state.mixer.layout = 3; // Wide, 44 px
        app.state.mixer.console.insert(2, true);
        app.paint(&mut c, None);
        assert!(!has(&app, Action::Mix(MHit::ChainSlot(2, 1))), "no chips at 44 px");
        assert!(has(&app, Action::Mix(MHit::StripPower(2))), "the console still shows");
    }

    /// Wide 2, Wide 3 and Wide alt draw the console and the strip's own meter side by side: the console gives up the
    /// meter's column at the strip's right edge, so the level stays visible behind the console's sections.
    #[test]
    fn wide_layouts_keep_the_meter_beside_the_console() {
        let mut app = app();
        let Ok(mut c) = os_ui::Canvas::new(1920, 1080, 1.0) else { return };
        let ws = &mut app.state.wins[crate::app::Win::Mixer.slot().unwrap()];
        (ws.maxed, ws.rect) = (false, os_ui::Rect::new(100.0, 83.0, 1400.0, 900.0));
        app.state.z = vec![crate::app::Win::Mixer];
        let rect = |app: &App, a: Action| app.hits.iter().find(|h| h.action == a).map(|h| h.rect);
        // Wide alt is narrower than the console is shown at by itself (the layout only splits the strip once the panel
        // button has asked for the console), so ask for it there.
        app.state.mixer.console.insert(2, true);
        for layout in [crate::mixer::WIDE_2, crate::mixer::WIDE_3, crate::mixer::WIDE_ALT] {
            app.state.mixer.layout = layout;
            app.paint(&mut c, None);
            let m = rect(&app, Action::Mix(MHit::Meter(2))).expect("the strip's meter");
            let strip = rect(&app, Action::Mix(MHit::Strip(2))).expect("the strip");
            let console = rect(&app, Action::Mix(MHit::StripPower(2))).expect("the console");
            assert_eq!(m.w, 20.0);
            assert!((m.right() - (strip.right() - 1.0)).abs() < 2.0, "at the strip's right edge: {m:?} in {strip:?}");
            assert!(console.right() < m.x, "the console leaves it room: {console:?} {m:?}");
        }
    }

    /// Extra large's effect rack: every slot is a button of its own whether it holds a plugin or not, so an empty
    /// channel always offers the add target — and a full ten-slot chain does not lose it.
    #[test]
    fn extra_large_slots_offer_the_add_button() {
        let mut app = app();
        let Ok(mut c) = os_ui::Canvas::new(1920, 1080, 1.0) else { return };
        let ws = &mut app.state.wins[crate::app::Win::Mixer.slot().unwrap()];
        (ws.maxed, ws.rect) = (false, os_ui::Rect::new(100.0, 83.0, 1400.0, 900.0));
        app.state.z = vec![crate::app::Win::Mixer];
        let has = |a: &App, want: Action| a.hits.iter().any(|h| h.action == want);
        app.state.mixer.layout = crate::mixer::EXTRA_LARGE;
        app.paint(&mut c, None);
        for k in 0..10u8 {
            assert!(has(&app, Action::Mix(MHit::TrackSlot(2, k))), "slot {} is a target", k + 1);
        }
        // The rows are not just targets: they are laid out down the strip, not stacked on one another.
        let row = |k: u8| app.hits.iter().find(|h| h.action == Action::Mix(MHit::TrackSlot(2, k))).map(|h| h.rect).expect("a slot row");
        assert!(row(0).y < row(1).y && row(1).y < row(9).y, "the slots follow one another");
        assert!(row(0).h >= 15.0, "and each is tall enough to read and click: {}", row(0).h);
        // The rack's equaliser is a control too: a press grabs the band nearest the cursor, and it is that strip's own
        // bands (the first band is the low end, at the graph's left).
        let graph = app.hits.iter().find(|h| h.action == Action::Mix(MHit::StripEq(2))).map(|h| h.rect).expect("the rack's equaliser");
        assert!(graph.h >= 24.0, "with room to be dragged: {graph:?}");
        assert_eq!(crate::mixer::eq_band_of(&app.state, 2, graph, graph.x + 2.0), 0);
        assert_eq!(crate::mixer::eq_band_of(&app.state, 2, graph, graph.right() - 2.0), 2);
    }

    /// The effect rack gives its top to the track's equaliser and keeps all ten slots under it; a column too short for
    /// both keeps the slots and drops the graph rather than squeezing a slot away.
    #[test]
    fn the_effect_rack_puts_its_eq_graph_above_the_slots() {
        let tall = Rect::new(0.0, 0.0, 50.0, 430.0);
        let (graph, rows) = rack_rows(tall);
        let g = graph.expect("a tall column shows the equaliser");
        assert_eq!(rows.len(), 10);
        assert!(rows[0].y > g.bottom(), "the slots start under the graph");
        assert!(rows[9].bottom() <= tall.bottom(), "and all ten still fit: {:?}", rows[9]);
        let short = Rect::new(0.0, 0.0, 50.0, 210.0);
        let (graph, rows) = rack_rows(short);
        assert!(graph.is_none(), "no room for the graph");
        assert!(rows.iter().all(|b| b.h >= 15.0));
    }

    /// The two Extra large layouts share their columns and both keep the plugin list up beside the console. V2 puts the
    /// level meter in the strip's right column with the list beside it, using that column's whole height; Extra large
    /// gives the list the whole column and the meter a lane beside the fader.
    #[test]
    fn extra_large_v2_uses_its_right_column_down_the_strip() {
        let mut app = app();
        let Ok(mut c) = os_ui::Canvas::new(1920, 1080, 1.0) else { return };
        let ws = &mut app.state.wins[crate::app::Win::Mixer.slot().unwrap()];
        (ws.maxed, ws.rect) = (false, os_ui::Rect::new(100.0, 83.0, 1400.0, 900.0));
        app.state.z = vec![crate::app::Win::Mixer];
        let rect = |app: &App, a: Action| app.hits.iter().find(|h| h.action == a).map(|h| h.rect);
        let slot = |app: &App| rect(app, Action::Mix(MHit::TrackSlot(2, 0)));
        let meter = |app: &App| rect(app, Action::Mix(MHit::Meter(2)));
        app.state.mixer.layout = crate::mixer::EXTRA_LARGE;
        app.paint(&mut c, None);
        let (up, up_meter) = (slot(&app).expect("Extra large's first slot"), meter(&app).expect("its meter"));
        app.state.mixer.layout = crate::mixer::EXTRA_LARGE_V2;
        app.paint(&mut c, None);
        let (down, down_meter) = (slot(&app).expect("V2's first slot"), meter(&app).expect("its meter"));
        assert!(down.y < up.y + 20.0, "V2 keeps the list up beside the console: {up:?} {down:?}");
        assert!(down_meter.x < down.x, "V2's list sits beside its meter: {down_meter:?} {down:?}");
        assert!(down_meter.y < up_meter.y, "V2's meter is up beside the console's knobs: {down_meter:?} {up_meter:?}");
        assert!(up_meter.y > 400.0, "Extra large keeps its meter down by the fader: {up_meter:?}");
    }

    /// Classic mode's strips: in index order, as always — the view ordering and the docks are unchanged.
    #[test]
    fn classic_orders_by_index() {
        let app = app();
        let n = mx::inserts(&app.state);
        assert!(!app.state.project.linear_mode);
        assert_eq!(strip_order(&app.state), (1..=n).collect::<Vec<u16>>());
        let d = docks(&app.state);
        let parts: Vec<Vec<u16>> = d.parts.iter().map(|p| p.iter().map(|e| e.0).collect()).collect();
        assert_eq!(parts[1], (1..=n).collect::<Vec<u16>>(), "the inserts are the middle part");
        assert_eq!(strip_order(&app.state), d.parts.iter().flat_map(|p| p.iter().map(|e| e.0)).collect::<Vec<_>>());
    }

    /// Linear mode's strips (spec: model/track-model.md §5): the tracks' strips in their Track order, then the
    /// inserts no track names; Classic mode's mixer logic is untouched (`crate::mixer` has no order of its own).
    #[test]
    fn linear_orders_by_track() {
        let mut app = app();
        track_create(&mut app.state, "audio");
        track_create(&mut app.state, "midi");
        track_create(&mut app.state, "bus");
        app.state.project.linear_mode = true;
        let n = mx::inserts(&app.state);
        let tracks: Vec<u16> = app.state.project.tracks.iter().map(|t| t.mixer_strip).collect();
        let mut want = tracks.clone();
        want.extend((1..=n).filter(|m| !want.contains(m)).collect::<Vec<_>>());
        assert_eq!(strip_order(&app.state), want);
        // The docks lay the tracks' strips first, in their Track order, the unlinked inserts after.
        let d = docks(&app.state);
        let ordered: Vec<u16> = d.parts.iter().flat_map(|p| p.iter().map(|e| e.0)).collect();
        assert_eq!(ordered.iter().filter(|m| tracks.contains(m)).cloned().collect::<Vec<_>>(), tracks);
        // Every insert is there exactly once, then Classic is unchanged again when the project is not Linear.
        let mut all = ordered.clone();
        all.sort();
        assert_eq!(all, (1..=n).collect::<Vec<u16>>());
        app.state.project.linear_mode = false;
        assert_eq!(strip_order(&app.state), (1..=n).collect::<Vec<u16>>());
    }
}
