// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! FreqBat native rack panel view.

use os_ui::{Canvas, Color, Rect, Point, Font, TextAlign};
use crate::params::FreqBatParams;

/// The FreqBat rack panel size.
pub const W: u32 = 336;
pub const H: u32 = 280;

/// Draws the FreqBat rack panel.
pub fn paint(canvas: &mut Canvas, params: &FreqBatParams) {
    let theme = os_theme::active();
    
    // Background
    canvas.fill_rect(Rect::new(0.0, 0.0, W as f32, H as f32), theme.panel_bg);
    
    // Title bar
    canvas.fill_rect(Rect::new(0.0, 0.0, W as f32, 28.0), theme.header_bg);
    canvas.draw_text(
        "FreqBat",
        Point::new(12.0, 18.0),
        Font::bold(14.0),
        theme.text_primary,
        TextAlign::Left,
    );
    
    // Panel sections
    let mut y = 36.0;
    let section_h = 56.0;
    let gap = 8.0;
    
    // Oscillators section
    draw_osc_section(canvas, params, 8.0, y, W as f32 - 16.0, section_h, theme);
    y += section_h + gap;
    
    // Mix / Filter section
    draw_mix_filter_section(canvas, params, 8.0, y, W as f32 - 16.0, section_h, theme);
    y += section_h + gap;
    
    // Envelopes section (compact)
    draw_env_section(canvas, params, 8.0, y, W as f32 - 16.0, section_h, theme);
    y += section_h + gap;
    
    // Global controls
    draw_global_section(canvas, params, 8.0, y, W as f32 - 16.0, section_h, theme);
}

fn draw_osc_section(canvas: &mut Canvas, params: &FreqBatParams, x: f32, y: f32, w: f32, h: f32, theme: &os_theme::Theme) {
    let font = Font::regular(10.0);
    let label_font = Font::regular(9.0);
    
    // Section background
    canvas.fill_rect(Rect::new(x, y, w, h), theme.section_bg);
    canvas.draw_text(
        "OSCILLATORS",
        Point::new(x + 8.0, y + 14.0),
        label_font,
        theme.text_muted,
        TextAlign::Left,
    );
    
    // Three oscillators
    let osc_w = (w - 16.0) / 3.0;
    for i in 0..3 {
        let ox = x + 8.0 + i as f32 * (osc_w + 8.0);
        let oy = y + 20.0;
        
        // Shape selector (simplified as text for rack)
        let shape_names = ["Sine", "Tri", "Square", "Saw", "RSaw", "Noise"];
        let shape = params.v(OSC_BASE + i * OSC_STRIDE + SHAPE).clamp(0, 5) as usize;
        canvas.draw_text(
            &format!("OSC {}", i + 1),
            Point::new(ox, oy),
            label_font,
            theme.text_secondary,
            TextAlign::Left,
        );
        canvas.draw_text(
            shape_names[shape],
            Point::new(ox, oy + 14.0),
            font,
            theme.text_primary,
            TextAlign::Left,
        );
        
        // Coarse pitch
        let coarse = params.v(OSC_BASE + i * OSC_STRIDE + COARSE) as f32 / 64.0;
        canvas.draw_text(
            &format!("{:+.0}st", coarse * 12.0),
            Point::new(ox, oy + 28.0),
            Font::regular(9.0),
            theme.text_muted,
            TextAlign::Left,
        );
    }
}

fn draw_mix_filter_section(canvas: &mut Canvas, params: &FreqBatParams, x: f32, y: f32, w: f32, h: f32, theme: &os_theme::Theme) {
    let font = Font::regular(10.0);
    let label_font = Font::regular(9.0);
    
    canvas.fill_rect(Rect::new(x, y, w, h), theme.section_bg);
    canvas.draw_text(
        "MIX / FILTER",
        Point::new(x + 8.0, y + 14.0),
        label_font,
        theme.text_muted,
        TextAlign::Left,
    );
    
    let oy = y + 20.0;
    let knob_w = (w - 24.0) / 4.0;
    
    // Mix 2
    let m2 = params.v(MIX2) as f32 / 128.0;
    draw_knob(canvas, x + 8.0, oy, knob_w, 36.0, "MIX 2", m2, theme);
    
    // Mix 3
    let m3 = params.v(MIX3) as f32 / 128.0;
    draw_knob(canvas, x + 8.0 + knob_w + 8.0, oy, knob_w, 36.0, "MIX 3", m3, theme);
    
    // Cutoff
    let cutoff = params.v(CUTOFF) as f32 / 256.0;
    draw_knob(canvas, x + 8.0 + 2.0 * (knob_w + 8.0), oy, knob_w, 36.0, "CUT", cutoff, theme);
    
    // Resonance
    let res = params.v(RESONANCE) as f32 / 256.0;
    draw_knob(canvas, x + 8.0 + 3.0 * (knob_w + 8.0), oy, knob_w, 36.0, "RES", res, theme);
}

fn draw_env_section(canvas: &mut Canvas, params: &FreqBatParams, x: f32, y: f32, w: f32, h: f32, theme: &os_theme::Theme) {
    let label_font = Font::regular(9.0);
    
    canvas.fill_rect(Rect::new(x, y, w, h), theme.section_bg);
    canvas.draw_text(
        "ENVELOPES",
        Point::new(x + 8.0, y + 14.0),
        label_font,
        theme.text_muted,
        TextAlign::Left,
    );
    
    let oy = y + 20.0;
    let knob_w = (w - 32.0) / 5.0;
    
    let targets = ["VOL", "PAN", "CUT", "RES", "PITCH"];
    for (i, name) in targets.iter().enumerate() {
        let on = params.v(ENV_BASE + i * ENV_STRIDE + R_ON) != 0;
        let color = if on { theme.accent } else { theme.text_muted };
        draw_mini_knob(canvas, x + 8.0 + i as f32 * (knob_w + 8.0), oy, knob_w, 32.0, name, color, theme);
    }
}

fn draw_global_section(canvas: &mut Canvas, params: &FreqBatParams, x: f32, y: f32, w: f32, h: f32, theme: &os_theme::Theme) {
    let label_font = Font::regular(9.0);
    
    canvas.fill_rect(Rect::new(x, y, w, h), theme.section_bg);
    canvas.draw_text(
        "GLOBAL",
        Point::new(x + 8.0, y + 14.0),
        label_font,
        theme.text_muted,
        TextAlign::Left,
    );
    
    let oy = y + 20.0;
    let knob_w = (w - 24.0) / 4.0;
    
    // Detune
    let detune = params.v(DETUNE) as f32 / 64.0;
    draw_knob(canvas, x + 8.0, oy, knob_w, 36.0, "DETUNE", detune, theme);
    
    // Random phase
    let rand = params.v(RAND) as f32 / 64.0;
    draw_knob(canvas, x + 8.0 + knob_w + 8.0, oy, knob_w, 36.0, "RAND", rand, theme);
    
    // HQ
    let hq = params.v(HQ) != 0;
    draw_switch(canvas, x + 8.0 + 2.0 * (knob_w + 8.0), oy, knob_w, 36.0, "HQ", hq, theme);
    
    // AM
    let am = params.v(AM_BYTE) != 0;
    draw_switch(canvas, x + 8.0 + 3.0 * (knob_w + 8.0), oy, knob_w, 36.0, "AM", am, theme);
}

fn draw_knob(canvas: &mut Canvas, x: f32, y: f32, w: f32, h: f32, label: &str, value: f32, theme: &os_theme::Theme) {
    let cx = x + w / 2.0;
    let cy = y + h / 2.0 - 4.0;
    let r = (w.min(h) / 2.0) - 2.0;
    
    // Knob track
    canvas.fill_circle(Point::new(cx, cy), r + 2.0, theme.knob_bg);
    canvas.stroke_circle(Point::new(cx, cy), r + 2.0, 1.0, theme.knob_border);
    
    // Knob indicator
    let angle = -0.75 * PI + value * 1.5 * PI;
    let ix = cx + angle.cos() as f32 * (r - 4.0);
    let iy = cy + angle.sin() as f32 * (r - 4.0);
    canvas.stroke_line(Point::new(cx, cy), Point::new(ix, iy), 2.0, theme.accent);
    
    // Label
    canvas.draw_text(
        label,
        Point::new(cx, y + h - 8.0),
        Font::regular(8.0),
        theme.text_secondary,
        TextAlign::Center,
    );
    
    // Value
    canvas.draw_text(
        &format!("{:.0}%", value * 100.0),
        Point::new(cx, y + 10.0),
        Font::regular(8.0),
        theme.text_primary,
        TextAlign::Center,
    );
}

fn draw_mini_knob(canvas: &mut Canvas, x: f32, y: f32, w: f32, h: f32, label: &str, color: Color, theme: &os_theme::Theme) {
    let cx = x + w / 2.0;
    let cy = y + h / 2.0 - 2.0;
    let r = (w.min(h) / 2.0) - 1.0;
    
    canvas.fill_circle(Point::new(cx, cy), r + 1.0, theme.knob_bg);
    canvas.stroke_circle(Point::new(cx, cy), r + 1.0, 1.0, theme.knob_border);
    
    canvas.draw_text(
        label,
        Point::new(cx, y + h - 6.0),
        Font::regular(7.0),
        theme.text_secondary,
        TextAlign::Center,
    );
    
    // Active indicator
    canvas.fill_circle(Point::new(cx, cy), 3.0, color);
}

fn draw_switch(canvas: &mut Canvas, x: f32, y: f32, w: f32, h: f32, label: &str, on: bool, theme: &os_theme::Theme) {
    let track_w = w - 4.0;
    let track_h = 18.0;
    let tx = x + 2.0;
    let ty = y + (h - track_h) / 2.0;
    
    let track_color = if on { theme.accent } else { theme.knob_bg };
    canvas.fill_rounded_rect(Rect::new(tx, ty, track_w, track_h), track_h / 2.0, track_color);
    
    let thumb_x = if on { tx + track_w - track_h + 2.0 } else { tx + 2.0 };
    canvas.fill_rounded_rect(Rect::new(thumb_x, ty + 2.0, track_h - 4.0, track_h - 4.0), (track_h - 4.0) / 2.0, theme.panel_bg);
    
    canvas.draw_text(
        label,
        Point::new(x + w / 2.0, y + h - 6.0),
        Font::regular(8.0),
        theme.text_secondary,
        TextAlign::Center,
    );
}

// Parameter indices (matching the 3x-osc layout)
const OSC_BASE: usize = 0;
const OSC_STRIDE: usize = 16;
const SHAPE: usize = 0;
const COARSE: usize = 1;
const FINE: usize = 2;
const PAN: usize = 3;
const PHASE: usize = 4;
const DETUNE: usize = 5;

const MIX2: usize = 48;
const MIX3: usize = 49;
const AM_BYTE: usize = 50;
const HQ: usize = 51;
const RAND: usize = 52;
const DETUNE_GLOBAL: usize = 53;

const FILTER: usize = 54;
const CUTOFF: usize = 55;
const RESONANCE: usize = 56;

const ENV_BASE: usize = 57;
const ENV_STRIDE: usize = 16;
const R_ON: usize = 0;
const R_DELAY: usize = 1;
const R_ATT: usize = 2;
const R_HOLD: usize = 3;
const R_DEC: usize = 4;
const R_SUS: usize = 5;
const R_REL: usize = 6;
const R_AMT: usize = 7;
const R_ATT_T: usize = 8;
const R_DEC_T: usize = 9;
const R_REL_T: usize = 10;
const R_LFO_SHAPE: usize = 11;
const R_LFO_DELAY: usize = 12;
const R_LFO_ATT: usize = 13;
const R_LFO_AMT: usize = 14;
const R_LFO_SPEED: usize = 15;