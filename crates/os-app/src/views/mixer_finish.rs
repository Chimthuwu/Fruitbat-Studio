// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Optional mixer console finishes, taken from the Michael Lane Edition
//! (`Dorfhaus/Lemon-Studio-Michael-Lane-Edition`: Original, Clean Modern and Analog). Painter code only: control
//! geometry, hit targets and audio laws stay in `views::mixer`. The default, [`Finish::Lemon`], is this program's own
//! look, so the console finishes are opt-in: they change nothing until one is picked as Settings ▸ General ▸ Mixer
//! style. Spec: ui/mixer.md.

use std::f32::consts::PI;

use os_ui::{Color, Rect};

use crate::app::Ui;

/// Which finish the mixer's strips are drawn in (`crate::prefs::MIXER_STYLE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Finish {
    /// Lemon Studio's own look (the default): the finishes below stay off.
    Lemon,
    /// The Edition's vintage console: warm molded caps, ivory fader caps, cream VU-lit lamps.
    Original,
    /// Flat, minimal, high contrast grey.
    CleanModern,
    /// Dark graphite metal with machined silver rims and an amber value arc.
    Analog,
}

impl Finish {
    /// The finish Settings ▸ General ▸ Mixer style names (0 = Lemon, see `crate::prefs::MIXER_STYLE`).
    pub fn current() -> Finish {
        Finish::of(crate::prefs::index(&crate::prefs::MIXER_STYLE))
    }

    /// The finish a `crate::prefs::MIXER_STYLE` choice stands for; an unknown one is Lemon, this program's own look.
    pub fn of(index: usize) -> Finish {
        match index {
            1 => Finish::Original,
            2 => Finish::CleanModern,
            3 => Finish::Analog,
            _ => Finish::Lemon,
        }
    }

    /// Whether one of the Edition's finishes is in use (Lemon keeps `views::mixer`'s own drawing).
    pub fn on(self) -> bool {
        self != Finish::Lemon
    }
}

const DARK: Color = Color::rgb(0x111315);
const INK: Color = Color::rgb(0xe1ddd1);

/// A strip's body.
pub fn panel(ui: &mut Ui, r: Rect, selected: bool) {
    match Finish::current() {
        Finish::Original => {
            ui.c.fill_rect(r, DARK);
            let face = r.inset(1.0);
            let top = Color::rgb(0x23231f);
            let bottom = Color::rgb(0x30312e);
            ui.c.fill_rect_gradient(face, top, bottom, 0.0);
            ui.c.line(face.x, face.y, face.x, face.bottom(), 0.7, INK.with_alpha(44));
            if selected {
                ui.c.fill_rect(Rect::new(r.x + 1.0, r.y, 1.5, r.h), Color::rgb(0xd4bc77));
            }
        }
        Finish::CleanModern => {
            ui.c.fill_rect(r, Color::rgb(0x191919));
            ui.c.fill_rect(r.inset(1.0), if selected { Color::rgb(0x2e2e2e) } else { Color::rgb(0x292929) });
            if selected {
                ui.c.stroke_rrect(r.inset(1.0), 0.0, 1.0, Color::rgb(0xe6ce65));
            }
        }
        Finish::Analog => {
            ui.c.fill_rect(r, Color::rgb(0x101214));
            let face = r.inset(1.0);
            ui.c.fill_rect_gradient(face, Color::rgb(0x2a2e31), Color::rgb(0x1d2124), 0.0);
            ui.c.stroke_rrect(face, 2.0, 0.7, Color::rgb(0x77828a).with_alpha(110));
            if selected {
                ui.c.stroke_rrect(face, 2.0, 1.0, Color::rgb(0xe4c74f));
            }
        }
        Finish::Lemon => {}
    }
}

/// A mute / solo lamp.
pub fn lamp(ui: &mut Ui, x: f32, y: f32, on: bool) {
    let finish = Finish::current();
    if !finish.on() {
        return super::led(ui, x, y, on);
    }
    let _ = finish;
    let col = Color::rgb(0x8ec63a);
    ui.c.fill_circle(x, y + 0.6, 6.5, DARK);
    ui.c.stroke_circle(x, y, 5.6, 1.0, Color::rgb(0x686c64));
    if on {
        ui.c.fill_circle(x, y, 8.0, col.with_alpha(26));
    }
    ui.c.fill_circle(x, y, 5.0, if on { col } else { col.mix(DARK, 0.79) });
    ui.c.fill_circle(
        x - 1.2,
        y - 1.5,
        1.6,
        if on { Color::rgb(0xfff5d5) } else { Color::rgb(0x929489).with_alpha(70) },
    );
}

/// A knob (pan, separation and the sends): `value` 0..1, `bipolar` a centred default (pan), `hover` lit.
pub fn knob(ui: &mut Ui, cx: f32, cy: f32, r: f32, value: f32, bipolar: bool, hover: bool) {
    let value = value.clamp(0.0, 1.0);
    let angle = (-225.0 + 270.0 * value) * PI / 180.0;
    match Finish::current() {
        Finish::Original => {
            let cap = if hover { Color::rgb(0xd1cbb4) } else { Color::rgb(0xc8c2ad) };
            ui.c.fill_circle(cx, cy + r * 0.12, r * 1.08, Color::rgb(0x0a0b0a));
            ui.c.fill_circle(cx, cy, r * 1.06, Color::rgb(0x6f7266));
            ui.c.fill_circle(cx, cy, r * 0.92, cap);
            ui.c.fill_circle(cx - r * 0.28, cy - r * 0.32, r * 0.42, cap.mix(Color::rgb(0xffffff), 0.34));
            ui.c.line(
                cx + angle.cos() * r * 0.30,
                cy + angle.sin() * r * 0.30,
                cx + angle.cos() * r * 0.86,
                cy + angle.sin() * r * 0.86,
                (r * 0.15).clamp(0.9, 2.0),
                Color::rgb(0xece0bc),
            );
        }
        Finish::CleanModern => {
            let face = Color::rgb(if hover { 0x555555 } else { 0x424242 });
            ui.c.fill_circle(cx, cy, r, face);
            ui.c.stroke_circle(cx, cy, r, 0.7, Color::rgb(0x5a5a5a));
            ui.c.line(
                cx + angle.cos() * r * 0.30,
                cy + angle.sin() * r * 0.30,
                cx + angle.cos() * r * 0.92,
                cy + angle.sin() * r * 0.92,
                1.35,
                Color::rgb(0xe2e2df),
            );
        }
        Finish::Analog => {
            ui.c.fill_circle(cx, cy, r, Color::rgb(0x15181a));
            ui.c.stroke_circle(cx, cy, r - 0.4, 0.9, Color::rgb(0x8d959b));
            ui.c.fill_circle(cx, cy, r * 0.74, Color::rgb(if hover { 0x30353a } else { 0x2b2f32 }));
            let neutral = if bipolar { 0.5 } else { 0.0 };
            if (value - neutral).abs() > 0.001 {
                let start = (-225.0 + 270.0 * neutral) * PI / 180.0;
                let sweep = 270.0 * (value - neutral).abs() * PI / 180.0;
                ui.c.arc(cx, cy, r + 2.1, start.min(angle), sweep, 1.0, Color::rgb(0xe8c74f));
            }
            ui.c.line(
                cx + angle.cos() * r * 0.22,
                cy + angle.sin() * r * 0.22,
                cx + angle.cos() * r * 0.74,
                cy + angle.sin() * r * 0.74,
                1.5,
                Color::rgb(0xedf1ed),
            );
        }
        Finish::Lemon => {}
    }
}

/// A fader handle centred on (`cx`, `cy`).
pub fn fader(ui: &mut Ui, cx: f32, cy: f32, selected: bool, hover: bool) {
    match Finish::current() {
        Finish::Original => {
            let r = Rect::new(cx - 9.5, cy - 14.0, 19.0, 28.0);
            ui.c.fill_rrect(Rect::new(r.x + 0.8, r.y + 2.2, r.w + 0.8, r.h), 2.8, Color::rgb(0x131410).with_alpha(210));
            ui.c.fill_rrect(r, 2.5, Color::rgb(0x4c4d40));
            let face = r.inset(1.0);
            ui.c.fill_rect_gradient(
                face,
                if hover { Color::rgb(0xf2edd3) } else { Color::rgb(0xd5d1ae) },
                Color::rgb(0x848670),
                1.7,
            );
            for dy in [-5.0, 0.0, 5.0] {
                ui.c.line(face.x + 1.0, cy + dy - 0.6, face.right() - 1.0, cy + dy - 0.6, 1.0, Color::rgb(0xdedbc4));
                ui.c.line(
                    face.x + 1.0,
                    cy + dy + 0.55,
                    face.right() - 1.0,
                    cy + dy + 0.55,
                    if dy == 0.0 { 1.7 } else { 1.0 },
                    Color::rgb(if dy == 0.0 { 0x373a30 } else { 0x707361 }),
                );
            }
            ui.c.line(face.x + 1.0, face.y + 0.5, face.right() - 1.0, face.y + 0.5, 0.9, Color::rgb(0xf1ead1));
            if selected {
                ui.c.line(face.x + 2.0, face.bottom() - 1.0, face.right() - 2.0, face.bottom() - 1.0, 0.8, Color::rgb(0xdac780));
            }
        }
        Finish::CleanModern => {
            let r = Rect::new(cx - 8.0, cy - 13.0, 16.0, 26.0);
            ui.c.fill_rrect(r, 2.0, Color::rgb(0x191919));
            ui.c.fill_rrect(r.inset(1.0), 1.5, Color::rgb(if hover { 0xc3c3c3 } else { 0xa3a3a3 }));
            ui.c.line(r.x + 1.0, cy, r.right() - 1.0, cy, 1.0, Color::rgb(0x292929));
            if selected {
                ui.c.stroke_rrect(r.outset(1.0), 2.0, 1.0, Color::rgb(0xe6ce65));
            }
        }
        Finish::Analog => {
            let r = Rect::new(cx - 10.5, cy - 15.0, 21.0, 31.0);
            ui.c.fill_rrect(Rect::new(r.x + 0.6, r.y + 1.8, r.w, r.h), 3.0, Color::rgb(0x080a0b).with_alpha(200));
            ui.c.fill_rrect(r, 2.6, Color::rgb(0x2b3033));
            let face = r.inset(1.2);
            ui.c.fill_rect_gradient(face, Color::rgb(0xa8b0b5), Color::rgb(0x636c71), 1.6);
            ui.c.fill_rect(Rect::new(face.x, cy - 3.0, face.w, 6.0), Color::rgb(0x3c4348));
            ui.c.line(face.x + 1.0, cy, face.right() - 1.0, cy, 1.0, Color::rgb(0xd8e0e5));
            ui.c.line(face.x + 1.0, face.y + 0.5, face.right() - 1.0, face.y + 0.5, 0.8, Color::rgb(0xe6edf1));
            if hover {
                ui.c.line(cx - 7.0, r.y - 2.0, cx + 7.0, r.y - 2.0, 0.8, Color::rgb(0xf8fafb));
            }
            if selected {
                ui.c.stroke_rrect(r, 2.6, 1.0, Color::rgb(0xe4c74f));
            }
        }
        Finish::Lemon => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Settings ▸ General ▸ Mixer style: each choice is its finish, and anything unknown keeps Lemon's own look.
    #[test]
    fn mix_styles_map_to_their_finishes() {
        assert_eq!(Finish::current(), Finish::of(crate::prefs::index(&crate::prefs::MIXER_STYLE)));
        assert_eq!(
            [0, 1, 2, 3, 9].map(Finish::of),
            [Finish::Lemon, Finish::Original, Finish::CleanModern, Finish::Analog, Finish::Lemon]
        );
        assert!(!Finish::of(0).on(), "the default finish leaves the mixer's own drawing in place");
        assert!(Finish::of(3).on());
    }
}

