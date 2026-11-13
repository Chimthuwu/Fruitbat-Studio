// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The Fruitbat Studio artwork: the full wordmark for the workspace watermark and the square fruit mark for the
//! toolbar and small icons. Both are embedded from `assets/fruitbat-logo.png` and `assets/fruitbat-mark.png` (made
//! from the supplied logo) and decoded once, then drawn with alpha so the theme shows through the transparent parts.

use std::sync::OnceLock;

use os_ui::{Canvas, Rect};

/// A decoded picture: `0xAARRGGBB`, straight alpha.
pub struct Picture {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

static FULL: OnceLock<Option<Picture>> = OnceLock::new();
static MARK: OnceLock<Option<Picture>> = OnceLock::new();

fn decode(bytes: &[u8]) -> Option<Picture> {
    let png = tiny_skia::Pixmap::decode_png(bytes).ok()?;
    let (w, h) = (png.width() as usize, png.height() as usize);
    let mut px = Vec::with_capacity(w * h);
    for p in png.pixels() {
        let c = p.demultiply();
        px.push((u32::from(c.alpha()) << 24) | (u32::from(c.red()) << 16) | (u32::from(c.green()) << 8) | u32::from(c.blue()));
    }
    Some(Picture { w, h, px })
}

/// The full wordmark, decoded once.
pub fn full() -> Option<&'static Picture> {
    FULL.get_or_init(|| decode(include_bytes!("../assets/fruitbat-logo.png"))).as_ref()
}

/// The square fruit mark, decoded once.
pub fn mark() -> Option<&'static Picture> {
    MARK.get_or_init(|| decode(include_bytes!("../assets/fruitbat-mark.png"))).as_ref()
}

/// Draws the full wordmark `width` logical px wide, centred on `(cx, cy)`, keeping its aspect.
pub fn draw_full(cv: &mut Canvas, cx: f32, cy: f32, width: f32) {
    let Some(p) = full() else { return };
    let height = width * p.h as f32 / p.w as f32;
    cv.image_argb(Rect::new(cx - width / 2.0, cy - height / 2.0, width, height), p.w, p.h, &p.px);
}

/// Draws the square mark `size` logical px across, centred on `(cx, cy)`.
pub fn draw_mark(cv: &mut Canvas, cx: f32, cy: f32, size: f32) {
    let Some(p) = mark() else { return };
    cv.image_argb(Rect::new(cx - size / 2.0, cy - size / 2.0, size, size), p.w, p.h, &p.px);
}
