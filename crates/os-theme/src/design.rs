// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Skins: the shapes and base colours of a look over the same layout. A theme (`.flstheme`) colours whichever skin is
//! in use.
//!
//! `Citrus` (formerly Classic) is the measured default look. `Lemon` is Lemon Studio's own look: neutral greys, one
//! lemon-yellow brand colour and rounded-square controls (knobs stay round). It re-colours every token and every
//! role-mapped literal (`Palette::s/a/h/tx/led`) through a `DesignMap` (a neutral lightness ramp plus key colours), sets
//! the tokens the map can't get right outright, and tells the widget kit which shape language to draw (`Style`). A
//! theme other than the stock Dark one is laid over Lemon's colours by a `Retint`.
// spec: ui/lemon-design.md

// This code is a part of L e m o n S t u d i o.

use crate::fl::{self, FlTheme};
use crate::{Color, Palette};

/// A skin: the look's shapes and base colours (Settings > Theme > Skins).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Design {
    /// Meridian: Lemonwig's default look. Ink and bone surfaces, a brass signature, squared instrument faces and
    /// editorial display type.
    #[default]
    Meridian,
    /// The measured look (named Classic before 2026-10-17).
    Citrus,
    /// Lemon Studio's original look: pale lemon on neutral greys, rounded squares.
    Lemon,
    /// The graphite, turquoise-signal Lemon redesign.
    Industrial,
}

impl Design {
    /// In the order Settings > Theme > Skins shows them.
    pub const ALL: [Design; 4] = [Design::Meridian, Design::Lemon, Design::Industrial, Design::Citrus];

    /// The skins that contribute an own-colours entry to the Themes list. Meridian is a skin only: its look is not
    /// presented as a colour theme, so selecting a theme never appears to select a skin.
    pub const THEME_SKINS: [Design; 3] = [Design::Lemon, Design::Industrial, Design::Citrus];

    /// The skins Settings ▸ Theme lists (Meridian and Industrial are set from elsewhere).
    pub const SETTINGS_SKINS: [Design; 2] = [Design::Lemon, Design::Citrus];

    /// The Themes-list index of this skin's own colours, when it has one.
    pub fn theme_index(self) -> Option<usize> {
        Self::THEME_SKINS.iter().position(|d| *d == self)
    }

    pub fn name(self) -> &'static str {
        match self {
            Design::Meridian => "Meridian",
            Design::Citrus => "Citrus",
            Design::Lemon => "Lemon",
            Design::Industrial => "Lemon Industrial",
        }
    }

    /// A skin by name; `Classic` (Citrus's old name, still in older `design.txt` files) is Citrus.
    pub fn parse(s: &str) -> Option<Design> {
        if s.trim().eq_ignore_ascii_case("classic") {
            return Some(Design::Citrus);
        }
        Design::ALL.into_iter().find(|d| d.name().eq_ignore_ascii_case(s.trim()))
    }

    pub fn next(self) -> Design {
        let i = Design::ALL.iter().position(|d| *d == self).unwrap_or(0);
        Design::ALL[(i + 1) % Design::ALL.len()]
    }

    pub fn style(self) -> Style {
        match self {
            Design::Meridian => Style::MERIDIAN,
            Design::Citrus => Style::CITRUS,
            Design::Lemon => Style::LEMON,
            Design::Industrial => Style::INDUSTRIAL,
        }
    }
}

/// Shape language of a design. Sizes that affect layout are not here: the layout never changes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    pub design: Design,
    /// Corner radius of buttons, boxes and combos.
    pub radius: f32,
    /// Corner radius of small parts (steps, slots, LEDs, notes).
    pub radius_small: f32,
    /// Corner radius of panels, popups and floating windows.
    pub radius_panel: f32,
}

impl Style {
    pub const MERIDIAN: Style = Style { design: Design::Meridian, radius: 2.0, radius_small: 1.5, radius_panel: 5.0 };
    pub const CITRUS: Style = Style { design: Design::Citrus, radius: 3.0, radius_small: 2.5, radius_panel: 4.0 };
    pub const LEMON: Style = Style { design: Design::Lemon, radius: 6.0, radius_small: 4.0, radius_panel: 10.0 };
    pub const INDUSTRIAL: Style = Style { design: Design::Industrial, radius: 2.5, radius_small: 2.0, radius_panel: 4.0 };

    pub fn is(&self, d: Design) -> bool {
        self.design == d
    }

    /// The modern widget kit's treatment. Meridian, Lemon and Industrial draw it, each with its own geometry and
    /// colours; Citrus keeps the measured kit.
    pub fn lemon(&self) -> bool {
        matches!(self.design, Design::Meridian | Design::Lemon | Design::Industrial)
    }

    /// Meridian's own component geometry: squared faces, rail edges and editorial displays.
    pub fn meridian(&self) -> bool {
        self.design == Design::Meridian
    }
}

impl Default for Style {
    fn default() -> Self {
        Style::CITRUS
    }
}

/// How a design re-colours the measured (Classic) colours, by role.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DesignMap {
    /// Surface lightness: (classic lightness, design lightness) points, ascending input.
    pub ramp: &'static [(f32, f32)],
    /// Hue (degrees) and saturation of the design's neutrals.
    pub tint: (f32, f32),
    /// Classic key colours -> design key colours.
    pub accent: Color,
    pub highlight: Color,
    pub mute: Color,
    /// Text: the design's text colour and the surface it is read on (dimmer classic text moves toward it).
    pub text: Color,
    pub text_bg: Color,
}

/// Classic text and its toolbar background (the reference for `DesignMap::text_of`).
const CLASSIC_TEXT_L: f32 = 0.85;
const CLASSIC_BG_L: f32 = 0.19;

impl DesignMap {
    fn ramp_l(&self, l: f32) -> f32 {
        let r = self.ramp;
        if l <= r[0].0 {
            return r[0].1;
        }
        for w in r.windows(2) {
            let ((a, fa), (b, fb)) = (w[0], w[1]);
            if l <= b {
                let t = if b > a { (l - a) / (b - a) } else { 1.0 };
                return fa + (fb - fa) * t;
            }
        }
        r[r.len() - 1].1
    }

    /// A classic surface colour in this design: lightness through the ramp, neutrals take the design's tint;
    /// clearly coloured surfaces keep their hue with reduced saturation.
    pub fn surface(&self, c: Color) -> Color {
        let (h, s, l) = c.hsl();
        let l2 = self.ramp_l(l).clamp(0.0, 1.0);
        let (th, ts) = self.tint;
        // Coloured = visible chroma (HSL saturation overrates the stock look's blue-tinted dark greys).
        let chroma = f32::from(c.r.max(c.g).max(c.b) - c.r.min(c.g).min(c.b)) / 255.0;
        let (h2, s2) = if chroma > 0.09 { (h, (s * 0.6).max(ts)) } else { (th, ts) };
        Color::from_hsl_pub(h2, s2.clamp(0.0, 1.0), l2, c.a)
    }

    pub fn accent_of(&self, c: Color) -> Color {
        fl::rekey(c, classic_keys().selected, self.accent)
    }
    pub fn highlight_of(&self, c: Color) -> Color {
        fl::rekey(c, classic_keys().highlight, self.highlight)
    }
    pub fn mute_of(&self, c: Color) -> Color {
        fl::rekey(c, classic_keys().mute, self.mute)
    }

    /// A classic text colour: its position between the classic background and text is kept between the
    /// design's text background and text colour (so dim text stays dim).
    pub fn text_of(&self, c: Color) -> Color {
        let (_, s, l) = c.hsl();
        let t = ((l - CLASSIC_BG_L) / (CLASSIC_TEXT_L - CLASSIC_BG_L)).clamp(-0.2, 1.25);
        let (th, ts, tl) = self.text.hsl();
        let bl = self.text_bg.hsl().2;
        let l2 = (bl + (tl - bl) * t).clamp(0.0, 1.0);
        Color::from_hsl_pub(th, if s > 0.25 { s * 0.6 } else { ts }, l2, c.a)
    }
}

/// Key colours of the stock look (Classic was measured with them).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Keys {
    pub selected: Color,
    pub highlight: Color,
    pub text: Color,
    pub mute: Color,
}

pub fn classic_keys() -> Keys {
    let d = FlTheme::dark();
    Keys {
        selected: d.selected.unwrap_or(Color::rgb(0xa8e44a)),
        highlight: d.highlight.unwrap_or(Color::rgb(0xfb9244)),
        text: d.text_color.unwrap_or(Color::rgb(0xc9d0d3)),
        mute: d.mute.unwrap_or(Color::rgb(0x78d008)),
    }
}

/// The original Lemon brand palette (owner's base colours, 2026-10-12).
pub mod lemon {
    use crate::Color;
    /// Main colour (pale yellow): "on" controls, selection, the playhead, lit steps.
    pub const MAIN: Color = Color::rgb(0xffe066);
    /// Highlight: the bright top of lit controls, hover on lit controls, selected notes.
    pub const HIGHLIGHT: Color = Color::rgb(0xfff1b8);
    /// Dark: the main surface (toolbar).
    pub const DARK: Color = Color::rgb(0x3b3b3b);
    /// Dark highlight: raised faces on hover, hairlines, knob tracks.
    pub const DARK_HIGHLIGHT: Color = Color::rgb(0x666666);
    /// Coloured dark: glyphs and text on lemon, unlit LEDs, hovered menu rows.
    pub const COLORED_DARK: Color = Color::rgb(0x473900);
    /// In-between: a darker lemon for edges of lit controls and meter bodies.
    pub const DEEP: Color = Color::rgb(0xc9a92c);
    /// Text on the greys.
    pub const TEXT: Color = Color::rgb(0xececec);
}

/// The Lemon Industrial palette: graphite surfaces, a restrained lime signature and turquoise for signal flow and
/// modulation; orange and coral are reserved for warnings and clipping.
pub mod industrial {
    use crate::Color;
    pub const MAIN: Color = Color::rgb(0xb8f34a);
    pub const HIGHLIGHT: Color = Color::rgb(0xd9ff96);
    pub const DARK: Color = Color::rgb(0x171e20);
    pub const DARK_HIGHLIGHT: Color = Color::rgb(0x2e3a3d);
    pub const COLORED_DARK: Color = Color::rgb(0x101416);
    pub const DEEP: Color = Color::rgb(0x86a62f);
    pub const TEXT: Color = Color::rgb(0xe5e9e5);
    pub const SIGNAL: Color = Color::rgb(0x00d5c0);
    pub const MODULATION: Color = Color::rgb(0x00d5c0);
    pub const WARNING: Color = Color::rgb(0xf49b48);
    pub const CLIPPING: Color = Color::rgb(0xe65e72);
}

/// Meridian: ink, bone and brass. It is the default skin but not a colour theme: a theme recolours it through the same
/// surfaces, text and key roles as every other skin.
pub mod meridian {
    use crate::Color;
    pub const MAIN: Color = Color::rgb(0xd8b26a);
    pub const HIGHLIGHT: Color = Color::rgb(0xf2e2ba);
    pub const DARK: Color = Color::rgb(0x14171b);
    pub const DARK_HIGHLIGHT: Color = Color::rgb(0x2c3239);
    pub const COLORED_DARK: Color = Color::rgb(0x1a1408);
    pub const DEEP: Color = Color::rgb(0x997238);
    pub const TEXT: Color = Color::rgb(0xe9e4d8);
    pub const SIGNAL: Color = Color::rgb(0x72d3c2);
    pub const MODULATION: Color = Color::rgb(0xbf9cf2);
    pub const WARNING: Color = Color::rgb(0xe39b55);
    pub const CLIPPING: Color = Color::rgb(0xe26872);
}

/// Meridian's ramp: the toolbar lands on ink (#14171b, 0.09), lists sink toward black-blue and raised faces lift to a
/// soft slate. The whole ladder is compressed so panels read as one machined surface rather than stacked cards.
const MERIDIAN_RAMP: &[(f32, f32)] = &[
    (0.0, 0.025),
    (0.03, 0.035),
    (0.06, 0.05),
    (0.10, 0.07),
    (0.13, 0.085),
    (0.155, 0.10),
    (0.19, 0.12),
    (0.23, 0.145),
    (0.27, 0.17),
    (0.33, 0.21),
    (0.5, 0.40),
    (0.8, 0.78),
    (1.0, 1.0),
];

/// Original Lemon: Classic's toolbar (0.19) lands on DARK (#3b3b3b, 0.231) and the raised 0.33 greys on
/// DARK_HIGHLIGHT (#666666, 0.40); the near-black lists and rulers lift to about #151515.
const LEMON_RAMP: &[(f32, f32)] = &[
    (0.0, 0.07),
    (0.03, 0.085),
    (0.06, 0.10),
    (0.10, 0.125),
    (0.13, 0.15),
    (0.155, 0.18),
    (0.19, 0.231),
    (0.23, 0.285),
    (0.27, 0.33),
    (0.33, 0.40),
    (0.5, 0.56),
    (0.8, 0.84),
    (1.0, 1.0),
];

/// Lemon Industrial: Classic's toolbar (0.19) lands on the graphite panel (#171e20, 0.17), raised 0.33 greys on the
/// elevated surface (#20282b, 0.22) and the near-black lists sink to about #0c1012.
const INDUSTRIAL_RAMP: &[(f32, f32)] = &[
    (0.0, 0.035),
    (0.03, 0.055),
    (0.06, 0.075),
    (0.10, 0.10),
    (0.13, 0.12),
    (0.155, 0.14),
    (0.19, 0.17),
    (0.23, 0.20),
    (0.27, 0.22),
    (0.33, 0.26),
    (0.5, 0.45),
    (0.8, 0.80),
    (1.0, 1.0),
];

fn map_of(d: Design) -> Option<DesignMap> {// This code is a part of L e m o n S t u d i o.
    match d {
        Design::Citrus => None,
        Design::Meridian => Some(DesignMap {
            ramp: MERIDIAN_RAMP,
            tint: (216.0, 0.10),
            accent: meridian::MAIN,
            highlight: meridian::HIGHLIGHT,
            mute: meridian::MAIN,
            text: meridian::TEXT,
            text_bg: meridian::DARK,
        }),
        Design::Lemon => Some(DesignMap {
            ramp: LEMON_RAMP,
            tint: (48.0, 0.0),
            accent: lemon::MAIN,
            highlight: lemon::MAIN,
            mute: lemon::MAIN,
            text: lemon::TEXT,
            text_bg: lemon::DARK,
        }),
        Design::Industrial => Some(DesignMap {
            ramp: INDUSTRIAL_RAMP,
            tint: (168.0, 0.0),
            accent: industrial::MAIN,
            highlight: industrial::MAIN,
            mute: industrial::MAIN,
            text: industrial::TEXT,
            text_bg: industrial::DARK,
        }),
    }
}

/// Tokens a design sets outright (the ones the role map can't get right: signal colours, notes, keys, steps).
fn overrides(d: Design) -> Vec<(&'static str, Color)> {
    let c = Color::rgb;
    match d {
        Design::Citrus => vec![],
        Design::Meridian => {
            use meridian::*;
            vec![
                ("toolbar", DARK),
                ("toolbar_line", c(0x0c0e11)),
                ("toolbar_edge", c(0x333a42)),
                ("sunken", c(0x0e1114)),
                ("sunken_dark", c(0x0a0c0f)),
                ("time_panel", c(0x101317)),
                ("menu_panel", c(0x181c21)),
                ("menu_text", c(0xb9b1a1)),
                ("news_panel", c(0x1b2026)),
                ("button", c(0x1d2228)),
                ("button_hi", c(0x2a3138)),
                ("glyph", TEXT),
                ("glyph_dim", c(0x8e887c)),
                ("text", TEXT),
                ("text_dim", c(0xb4ac9d)),
                ("text_faint", c(0x79746a)),
                ("on_top", HIGHLIGHT),
                ("on_bottom", MAIN),
                ("on_glyph", COLORED_DARK),
                ("pill", MAIN),
                ("pill_text", COLORED_DARK),
                ("record", CLIPPING),
                ("tempo_box", c(0x0e1114)),
                ("accent", MAIN),
                ("accent_dim", c(0x5c4a29)),
                ("workspace", c(0x0d1013)),
                ("watermark", c(0x171b20)),
                ("version_text", c(0x4a4b4a)),
                ("browser_title", c(0x171b20)),
                ("browser_tabs", c(0x11151a)),
                ("browser_tab_outline", MAIN),
                ("browser_list", c(0x0b0e11)),
                ("browser_row_hi", c(0x20262d)),
                ("browser_folder", c(0xd8d2c6)),
                ("browser_footer", c(0x14181d)),
                ("browser_search", MAIN),
                ("cat_project", MAIN),
                ("cat_recent", SIGNAL),
                ("cat_plugin_db", c(0x9aa4ad)),
                ("cat_plugin_presets", c(0x9d92b0)),
                ("cat_channel_presets", c(0x92a59b)),
                ("cat_mixer_presets", c(0x8e9baa)),
                ("splitter", c(0x11151a)),
                ("splitter_dark", c(0x090b0e)),
                ("window_title", c(0x151a1f)),
                ("window_title_float", c(0x151a1f)),
                ("window_title_hi", c(0x29313a)),
                ("window_body", c(0x14181d)),
                ("window_border", c(0x080a0d)),
                ("picker", c(0x151a1f)),
                ("track_header", c(0x1a2026)),
                ("overview", c(0x4b5157)),
                ("ruler", c(0x0c0f12)),
                ("ruler_text", c(0xb4ac9d)),
                ("grid", c(0x12161a)),
                ("grid_alt", c(0x171c21)),
                ("grid_gap", c(0x0b0e11)),
                ("grid_bar", c(0x080a0d)),
                ("scrollbar", c(0x11151a)),
                ("playhead", MAIN),
                ("mixer_bg", c(0x0d1013)),
                ("mixer_strip", c(0x171c21)),
                ("mixer_strip_dark", c(0x111519)),
                ("mixer_panel", c(0x1a2026)),
                ("slot", c(0x1b2128)),
                ("slot_text", c(0x79746a)),
                ("slot_text_used", c(0xe3ddd1)),
                ("fader_handle", c(0x9a927f)),
                ("meter_low", c(0x2c7f74)),
                ("meter_high", SIGNAL),
                ("meter_clip", CLIPPING),
                ("signal", SIGNAL),
                ("modulation", MODULATION),
                ("warning", WARNING),
                ("clipping", CLIPPING),
                ("rack_row_box", c(0x181d23)),
                ("name_button", c(0x20262d)),
                ("name_button_sel", c(0x2b323a)),
                ("name_text", TEXT),
                ("rack_inset", c(0x0d1013)),
                ("rack_inset_border", c(0x080a0d)),
                ("rack_control_border", c(0x080a0d)),
                ("step_a", c(0x3d434a)),
                ("step_b", c(0x4b4437)),
                ("step_dim_a", c(0x2a3036)),
                ("step_dim_b", c(0x322e27)),
                ("step_dim_border", c(0x242a30)),
                ("step_on", MAIN),
                ("add_hover", MAIN),
                ("menu_drop", c(0x151a1f)),
                ("menu_drop_hi", c(0x2a3138)),
                ("menu_drop_text", TEXT),
                ("menu_drop_disabled", c(0x79746a)),
                ("menu_pop", c(0x151a1f)),
                ("menu_pop_border", c(0x080a0d)),
                ("menu_pop_hi", c(0x2a3138)),
                ("menu_pop_text", TEXT),
                ("menu_pop_shortcut", c(0xb4ac9d)),
                ("menu_pop_dim", c(0x79746a)),
                ("menu_pop_head", c(0x101317)),
                ("menu_pop_head_text", c(0xb4ac9d)),
                ("clip", c(0x9c7c46)),
                ("pr_row", c(0x1a2026)),
                ("pr_row_black", c(0x14191e)),
                ("pr_row_sel", c(0x2a2721)),
                ("pr_row_black_sel", c(0x231f19)),
                ("pr_bar", c(0x0b0e11)),
                ("note_top", c(0xd8b26a)),
                ("note_bottom", c(0xa9843f)),
                ("note_rim", c(0x5c4520)),
                ("note_text", c(0x1a1408)),
                ("note_sel_top", c(0xf2e2ba)),
                ("note_sel_bottom", c(0xd8b26a)),
                ("note_sel_rim", c(0x5c4520)),
                ("note_sel_text", c(0x1a1408)),
                ("key_white", c(0xe7e0d3)),
                ("key_white_left", c(0xc9c1b2)),
                ("key_black", c(0x2f353c)),
                ("key_black_left", c(0x1b2026)),
                ("key_c", c(0xd6cec0)),
                ("key_label", c(0x6d675c)),
                ("key_lit", c(0xa9e4d8)),
                ("key_lit_left", c(0x62bfb0)),
                ("key_lit_edge", c(0x2a7a70)),
                ("key_black_lit", c(0x3f9e91)),
                ("key_black_lit_left", c(0x1d5c55)),
                ("velocity", SIGNAL),
                ("velocity_dot", c(0x1f5c55)),
            ]
        }
        Design::Lemon => {
            use lemon::*;
            vec![
                ("toolbar", DARK),
                ("toolbar_line", c(0x2a2a2a)),
                ("toolbar_edge", c(0x4a4a4a)),
                ("sunken", c(0x2b2b2b)),
                ("sunken_dark", c(0x232323)),
                ("time_panel", c(0x232323)),
                ("menu_panel", c(0x333333)),
                ("menu_text", TEXT),
                ("news_panel", c(0x333333)),
                ("button", c(0x474747)),
                ("button_hi", c(0x555555)),
                ("glyph", c(0xe6e6e6)),
                ("glyph_dim", c(0x8c8c8c)),
                ("text", TEXT),
                ("text_dim", c(0xa3a3a3)),
                ("text_faint", c(0x7f7f7f)),
                ("on_top", HIGHLIGHT),
                ("on_bottom", MAIN),
                ("on_glyph", COLORED_DARK),
                ("pill", MAIN),
                ("pill_text", COLORED_DARK),
                ("record", c(0xff6159)),
                ("tempo_box", c(0x2b2b2b)),
                ("accent", MAIN),
                ("accent_dim", COLORED_DARK),
                ("workspace", c(0x262626)),
                ("watermark", c(0x2e2e2e)),
                ("version_text", c(0x5a5a5a)),
                ("browser_title", c(0x333333)),
                ("browser_tabs", c(0x2b2b2b)),
                ("browser_tab_outline", MAIN),
                ("browser_list", c(0x1f1f1f)),
                ("browser_row_hi", c(0x333333)),
                ("browser_folder", c(0xd9d9d9)),
                ("browser_footer", c(0x2b2b2b)),
                ("browser_search", MAIN),
                ("cat_project", MAIN),
                ("cat_recent", c(0xc8e3a6)),
                ("cat_plugin_db", c(0xa9cbe6)),
                ("cat_plugin_presets", c(0xe6b3cf)),
                ("cat_channel_presets", c(0xd4bce8)),
                ("cat_mixer_presets", c(0xbcc2ec)),
                ("splitter", c(0x262626)),
                ("splitter_dark", c(0x1c1c1c)),
                ("window_title", c(0x333333)),
                ("window_title_float", c(0x333333)),
                ("window_title_hi", c(0x3d3d3d)),
                ("window_body", c(0x2e2e2e)),
                ("window_border", c(0x1a1a1a)),
                ("picker", c(0x2b2b2b)),
                ("track_header", c(0x383838)),
                ("overview", c(0x555555)),
                ("ruler", c(0x232323)),
                ("ruler_text", c(0x9e9e9e)),
                ("grid", c(0x282828)),
                ("grid_alt", c(0x2d2d2d)),
                ("grid_gap", c(0x202020)),
                ("grid_bar", c(0x161616)),
                ("scrollbar", c(0x2b2b2b)),
                ("playhead", MAIN),
                ("mixer_bg", c(0x262626)),
                ("mixer_strip", c(0x303030)),
                ("mixer_strip_dark", c(0x242424)),
                ("mixer_panel", c(0x333333)),
                ("slot", c(0x2b2b2b)),
                ("slot_text", c(0x707070)),
                ("slot_text_used", c(0xdedede)),
                ("fader_handle", c(0x5c5c5c)),
                ("meter_low", DEEP),
                ("meter_high", HIGHLIGHT),
                ("meter_clip", c(0xff6159)),
                ("rack_row_box", c(0x3a3a3a)),
                ("name_button", c(0x474747)),
                ("name_button_sel", c(0x5e5e5e)),
                ("name_text", c(0xe2e2e2)),
                ("rack_inset", c(0x333333)),
                ("rack_inset_border", c(0x262626)),
                ("rack_control_border", c(0x222222)),
                ("step_a", c(0x4d4d4d)),
                ("step_b", c(0x5a4d1f)),
                ("step_dim_a", c(0x3a3a3a)),
                ("step_dim_b", c(0x3d3827)),
                ("step_dim_border", c(0x2e2e2e)),
                ("step_on", MAIN),
                ("add_hover", MAIN),
                ("menu_drop", c(0x333333)),
                ("menu_drop_hi", c(0x4d4d4d)),
                ("menu_drop_text", TEXT),
                ("menu_drop_disabled", c(0x7f7f7f)),
                ("menu_pop", c(0x333333)),
                ("menu_pop_border", c(0x1f1f1f)),
                ("menu_pop_hi", COLORED_DARK),
                ("menu_pop_text", TEXT),
                ("menu_pop_shortcut", c(0xa3a3a3)),
                ("menu_pop_dim", c(0x7f7f7f)),
                ("menu_pop_head", c(0x2b2b2b)),
                ("menu_pop_head_text", c(0x8c8c8c)),
                ("clip", c(0x8a7a3a)),
                ("pr_row", c(0x2b2b2b)),
                ("pr_row_black", c(0x242424)),
                ("pr_row_sel", c(0x2f2c22)),
                ("pr_row_black_sel", c(0x29261c)),
                ("pr_bar", c(0x141414)),
                ("note_top", MAIN),
                ("note_bottom", c(0xf2d052)),
                ("note_rim", c(0x8a7420)),
                ("note_text", COLORED_DARK),
                ("note_sel_top", c(0xffffff)),
                ("note_sel_bottom", HIGHLIGHT),
                ("note_sel_rim", COLORED_DARK),
                ("note_sel_text", COLORED_DARK),
                ("key_white", c(0xededed)),
                ("key_white_left", c(0xd2d2d2)),
                ("key_black", c(0x3b3b3b)),
                ("key_black_left", c(0x262626)),
                ("key_c", c(0xdddddd)),
                ("key_label", c(0x8a7420)),
                ("key_lit", HIGHLIGHT),
                ("key_lit_left", MAIN),
                ("key_lit_edge", DEEP),
                ("key_black_lit", MAIN),
                ("key_black_lit_left", c(0xa88f25)),
                ("velocity", MAIN),
                ("velocity_dot", COLORED_DARK),
            ]
        }
        Design::Industrial => {
            use industrial::*;
            vec![
                ("toolbar", DARK),
                ("toolbar_line", c(0x0f1416)),
            ("toolbar_edge", c(0x283337)),
            ("sunken", c(0x0d1113)),
            ("sunken_dark", c(0x0a0e10)),
            ("time_panel", c(0x20282b)),
            ("menu_panel", DARK),
            ("menu_text", c(0x9aa7a5)),
            ("news_panel", c(0x20282b)),
            ("button", c(0x20282b)),
            ("button_hi", c(0x2b3538)),
            ("glyph", TEXT),
            ("glyph_dim", c(0x657378)),
            ("text", TEXT),
            ("text_dim", c(0x9aa7a5)),
            ("text_faint", c(0x657378)),
            ("on_top", HIGHLIGHT),
            ("on_bottom", MAIN),
            ("on_glyph", COLORED_DARK),
            ("pill", MAIN),
            ("pill_text", COLORED_DARK),
            ("record", c(0xe65e72)),
            ("tempo_box", c(0x0d1113)),
            ("accent", MAIN),
            ("accent_dim", c(0x4f6b1f)),
            ("workspace", c(0x101416)),
            ("watermark", c(0x1a2224)),
            ("version_text", c(0x465257)),
            ("browser_title", c(0x171e20)),
            ("browser_tabs", c(0x101416)),
            ("browser_tab_outline", MAIN),
            ("browser_list", c(0x0c1012)),
            ("browser_row_hi", c(0x20282b)),
            ("browser_folder", c(0xd7dfd8)),
            ("browser_footer", c(0x171e20)),
            ("browser_search", c(0x00d5c0)),
            ("cat_project", MAIN),
            ("cat_recent", c(0x00d5c0)),
            ("cat_plugin_db", c(0x9aa7a5)),
            ("cat_plugin_presets", c(0x7f9aa0)),
            ("cat_channel_presets", c(0x8fa59a)),
            ("cat_mixer_presets", c(0x8d9ea8)),
            ("splitter", c(0x101416)),
            ("splitter_dark", c(0x090d0f)),
            ("window_title", c(0x171e20)),
            ("window_title_float", c(0x171e20)),
            ("window_title_hi", c(0x263135)),
            ("window_body", c(0x171e20)),
            ("window_border", c(0x090d0f)),
            ("picker", c(0x171e20)),
            ("track_header", c(0x1a2224)),
            ("overview", c(0x555555)),
            ("ruler", c(0x0c1012)),
            ("ruler_text", c(0x9aa7a5)),
            ("grid", c(0x151d1f)),
            ("grid_alt", c(0x1a2325)),
            ("grid_gap", c(0x0e1315)),
            ("grid_bar", c(0x0b0f11)),
            ("scrollbar", c(0x171e20)),
            ("playhead", MAIN),
            ("mixer_bg", c(0x101416)),
            ("mixer_strip", c(0x171e20)),
            ("mixer_strip_dark", c(0x12181a)),
            ("mixer_panel", c(0x1b2427)),
            ("slot", c(0x1e272a)),
            ("slot_text", c(0x657378)),
            ("slot_text_used", c(0xd7dfd8)),
            ("fader_handle", c(0x6f7a76)),
            ("meter_low", c(0x0f7c72)),
            ("meter_high", SIGNAL),
            ("meter_clip", CLIPPING),
            ("signal", SIGNAL),
            ("modulation", MODULATION),
            ("warning", WARNING),
            ("clipping", CLIPPING),
            ("rack_row_box", c(0x1a2224)),
            ("name_button", c(0x262626)),
            ("name_button_sel", c(0x303030)),
            ("name_text", TEXT),
            ("rack_inset", c(0x0d1113)),
            ("rack_inset_border", c(0x090d0f)),
            ("rack_control_border", c(0x090d0f)),
            ("step_a", c(0x4d4d4d)),
            ("step_b", c(0x5a4d1f)),
            ("step_dim_a", c(0x3a3a3a)),
            ("step_dim_b", c(0x3d3827)),
            ("step_dim_border", c(0x2e2e2e)),
            ("step_on", MAIN),
            ("add_hover", MAIN),
            ("menu_drop", c(0x171e20)),
            ("menu_drop_hi", c(0x2b3538)),
            ("menu_drop_text", TEXT),
            ("menu_drop_disabled", c(0x657378)),
            ("menu_pop", c(0x171e20)),
            ("menu_pop_border", c(0x090d0f)),
            ("menu_pop_hi", c(0x2b3538)),
            ("menu_pop_text", TEXT),
            ("menu_pop_shortcut", c(0x9aa7a5)),
            ("menu_pop_dim", c(0x657378)),
            ("menu_pop_head", c(0x101416)),
            ("menu_pop_head_text", c(0x9aa7a5)),
            ("clip", c(0x7d8f35)),
            ("pr_row", c(0x2b2b2b)),
            ("pr_row_black", c(0x242424)),
            ("pr_row_sel", c(0x2f2c22)),
            ("pr_row_black_sel", c(0x29261c)),
            ("pr_bar", c(0x141414)),
            // Notes are signal: turquoise bodies with a calm, high-contrast label; selection is the lemon signature.
            ("note_top", c(0x3fc4b2)),
            ("note_bottom", c(0x1d8579)),
            ("note_rim", c(0x0b4842)),
            ("note_text", c(0xeafffb)),
            ("note_sel_top", c(0xd9ff96)),
            ("note_sel_bottom", c(0xa8d94a)),
            ("note_sel_rim", c(0x4d661d)),
            ("note_sel_text", c(0x17200b)),
            ("key_white", c(0xededed)),
            ("key_white_left", c(0xd2d2d2)),
            ("key_black", c(0x3b3b3b)),
            ("key_black_left", c(0x262626)),
            ("key_c", c(0xdddddd)),
            ("key_label", c(0x6f7c78)),
            ("key_lit", c(0x8ff0e3)),
            ("key_lit_left", c(0x3fbfae)),
            ("key_lit_edge", c(0x0d6b60)),
            ("key_black_lit", c(0x249a8d)),
            ("key_black_lit_left", c(0x0b4842)),
            ("velocity", SIGNAL),
            ("velocity_dot", c(0x0f5c54)),
            ]
        }
    }
}

/// A brand colour as skin `design` has it, for the widget kit's direct `design::lemon` constants. The original Lemon
/// skin is the constants themselves; the industrial skin re-keys the seven brand colours and leaves anything else alone.
pub fn own_brand(design: Design, c: Color) -> Color {
    let (main, high, dark, dark_hi, colored, deep, text) = match design {
        Design::Industrial => (industrial::MAIN, industrial::HIGHLIGHT, industrial::DARK, industrial::DARK_HIGHLIGHT, industrial::COLORED_DARK, industrial::DEEP, industrial::TEXT),
        Design::Meridian => (meridian::MAIN, meridian::HIGHLIGHT, meridian::DARK, meridian::DARK_HIGHLIGHT, meridian::COLORED_DARK, meridian::DEEP, meridian::TEXT),
        _ => return c,
    };
    let pairs = [
        (lemon::MAIN, main),
        (lemon::HIGHLIGHT, high),
        (lemon::DARK, dark),
        (lemon::DARK_HIGHLIGHT, dark_hi),
        (lemon::COLORED_DARK, colored),
        (lemon::DEEP, deep),
        (lemon::TEXT, text),
    ];
    pairs.iter().find(|(from, _)| *from == c).map_or(c, |(_, to)| *to)
}

/// The palette of skin `design` coloured by `theme` (a `.flstheme`). The stock Dark theme (whatever its options), or
/// none, leaves a skin its own colours: Citrus the measured look, Lemon the brand palette; any other theme colours
/// Citrus through `fl::palette` and is laid over Lemon's colours by a `Retint`. The theme's workspace picture goes with
/// it on either skin, also where it keeps the skin's own colours (spec: ui/flstheme.md §3).
pub fn palette(design: Design, theme: Option<&FlTheme>) -> Palette {
    let mut p = skin_palette(design, theme);
    p.wallpaper = theme.and_then(fl::wallpaper);
    p
}

fn skin_palette(design: Design, theme: Option<&FlTheme>) -> Palette {
    // A skin's own theme gives that skin its own colours; another skin's own theme gives that skin's colours with this
    // skin's shapes. The stock Citrus theme laid over a modern skin is the stock Dark colours on that skin, as usual.
    if let Some(th) = theme {
        if let Some(other) = th.colors {
            if other == design {
                return palette(design, None);
            }
            if other == Design::Citrus {
                let mut p = palette(design, None);
                retint(&mut p, &FlTheme { colors: None, ..th.clone() });
                return p;
            }
            let mut p = palette(other, None);
            p.style = design.style();
            return p;
        }
    }
    let Some(map) = map_of(design) else {
        return match theme {
            Some(th) => fl::palette(th),
            None => Palette::default(),
        };
    };
    let classic = Palette::default();
    let mut p = classic.clone();
    for name in Palette::NAMES {
        let c = classic.get(name).expect("token");
        let v = match fl::role(name) {
            fl::Role::Surface | fl::Role::StepEven | fl::Role::StepOdd | fl::Role::Fixed => map.surface(c),
            fl::Role::Accent => map.accent_of(c),
            fl::Role::Highlight => map.highlight_of(c),
            fl::Role::Text => map.text_of(c),
            fl::Role::Mute => map.mute_of(c),
        };
        *p.get_mut(name).expect("token") = v;
    }
    for (name, c) in overrides(design) {
        if let Some(slot) = p.get_mut(name) {
            *slot = c;
        }
    }
    p.xf = fl::Xform { design: Some(map), ..fl::Xform::default() };
    p.style = design.style();
    if let Some(th) = theme.filter(|t| !t.is_stock()) {
        retint(&mut p, th);
    }
    p
}

/// the original's base palette toolbar (spec: ui/flstheme.md §5): its slight blue tint is what a theme's Hue and Saturation turn.
const BASE_TINT: Color = Color::rgb(0x434c51);

/// A theme laid over a skin's own colours: the theme's surface adjustment tints and lights the skin's greys (read as
/// stock Dark surfaces, given the original's base tint), its selection / highlight / mute colours take the place of the skin's
/// key colour, and its text colour the skin's text, keeping each text's place between the skin's background and text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Retint {
    /// The theme's surface adjustment.
    pub surface: fl::Surface,
    /// The skin's key colour (Lemon: Main) and the theme's keys (`None` = keep the skin's).
    pub key: Color,
    pub accent: Option<Color>,
    pub highlight: Option<Color>,
    pub mute: Option<Color>,
    /// (text, background) of the skin, and of the theme.
    pub text_from: (Color, Color),
    pub text_to: (Color, Color),
}

impl Retint {
    fn brand(d: Design) -> (Color, Color, Color, Color) {
        match d {
            Design::Industrial => (industrial::MAIN, industrial::HIGHLIGHT, industrial::TEXT, industrial::DARK),
            Design::Meridian => (meridian::MAIN, meridian::HIGHLIGHT, meridian::TEXT, meridian::DARK),
            _ => (lemon::MAIN, lemon::HIGHLIGHT, lemon::TEXT, lemon::DARK),
        }
    }

    /// `th` over skin `d`.
    pub fn of(d: Design, th: &FlTheme) -> Retint {
        let (main, _, text, dark) = Self::brand(d);
        let mut r = Retint {
            surface: th.surface,
            key: main,
            accent: th.selected,
            highlight: th.highlight,
            mute: th.mute,
            text_from: (text, dark),
            text_to: (text, dark),
        };
        let bg = r.surface(dark);
        let text = th.text_color.unwrap_or(if bg.hsl().2 > 0.5 { Color::rgb(0x1e1e1e) } else { text });
        r.text_to = (fl::readable(text, bg, 0.38), bg);
        r
    }

    /// `th` over the original Lemon skin.
    pub fn lemon(th: &FlTheme) -> Retint {
        Self::of(Design::Lemon, th)
    }

    pub fn surface(&self, c: Color) -> Color {
        let l = fl::DARK.invert(c).hsl().2;
        let (h, s, _) = BASE_TINT.hsl();
        let (h2, s2, l2) = self.surface.apply(Color::from_hsl_pub(h, s, l, c.a)).hsl();
        // A coloured surface (Lemon's lemon-tinted wells and hovers) takes the theme's selection hue.
        let chroma = f32::from(c.r.max(c.g).max(c.b) - c.r.min(c.g).min(c.b)) / 255.0;
        let (h2, s2) = if chroma > 0.09 {
            let (kh, ks, _) = self.accent(c).hsl();
            (kh, ks)
        } else {
            (h2, s2)
        };
        Color::from_hsl_pub(h2, s2, l2, c.a)
    }

    fn key(&self, c: Color, to: Option<Color>) -> Color {
        to.map_or(c, |t| fl::rekey(c, self.key, t))
    }
    pub fn accent(&self, c: Color) -> Color {
        self.key(c, self.accent)
    }
    pub fn highlight(&self, c: Color) -> Color {
        self.key(c, self.highlight)
    }
    pub fn mute(&self, c: Color) -> Color {
        self.key(c, self.mute)
    }

    pub fn text(&self, c: Color) -> Color {
        let (_, s, l) = c.hsl();
        let ((ft, fb), (tt, tb)) = (self.text_from, self.text_to);
        let (fl_, bl) = (ft.hsl().2, fb.hsl().2);
        let t = if (fl_ - bl).abs() > 1e-3 { ((l - bl) / (fl_ - bl)).clamp(-0.2, 1.25) } else { 1.0 };
        let (th, ts, tl) = tt.hsl();
        let l2 = tb.hsl().2 + (tl - tb.hsl().2) * t;
        Color::from_hsl_pub(th, if s > 0.25 { s * 0.6 } else { ts }, l2.clamp(0.0, 1.0), c.a)
    }
}

/// A skin's own colours as theme text: where Settings ▸ Theme ▸ Adjustments start with no theme applied. Citrus: the
/// stock Dark theme. Lemon: the stock Dark surface with saturation at its lowest (Lemon's neutral greys), Lemon's Main
/// as selection, highlight and mute, Lemon's text, and no step colours, so it colours Lemon as Lemon is (spec:
/// ui/theme-presets.md §C).
pub fn default_text(d: Design) -> String {
    match d {
        Design::Citrus => fl::DARK_TEXT.to_string(),
        Design::Meridian | Design::Lemon | Design::Industrial => {
            let (hue, main, text) = match d {
                Design::Industrial => (168, industrial::MAIN, industrial::TEXT),
                Design::Meridian => (216, meridian::MAIN, meridian::TEXT),
                _ => (0, lemon::MAIN, lemon::TEXT),
            };
            let v = |key: &str, c: Color| format!("{key}={}\n", fl::colour_value(key, c));
            format!(
                "Hue={hue}\nSaturation=-256\nLightness=-153\nContrast=-8\nText=0\n{}{}{}{}BackMode=2\n",
                v("Selected", main),
                v("Highlight", main),
                v("Mute", main),
                v("TextColor", text)
            )
        }
    }
}

/// The Lemon and Citrus themes (Settings ▸ Theme ▸ Themes): skin `d`'s own colours as a theme file, marked with
/// `fl::COLORS_KEY` so either skin draws them exactly (spec: ui/theme-presets.md §B). The rest of the file is
/// `default_text(d)`, which is what FL reads.
pub fn skin_text(d: Design) -> String {
    format!("{}{}={}\n", default_text(d), fl::COLORS_KEY, d.name())
}

/// Lemon's palette `p` coloured by `th` (see `Retint`). Step faces are the theme's own (as Citrus draws them); the
/// fixed tokens drawn in the lemon key colour (lit steps, notes, key labels, the default clip, the project category)
/// follow the theme's selection colour; then what the theme colours directly (`fl::finish`).
fn retint(p: &mut Palette, th: &FlTheme) {
    let r = Retint::of(p.style.design, th);
    let citrus = fl::palette(th);
    let skin = p.clone();
    for name in Palette::NAMES {
        let c = skin.get(name).expect("token");
        let v = match fl::role(name) {
            fl::Role::Surface => r.surface(c),
            fl::Role::Accent => r.accent(c),
            fl::Role::Highlight => r.highlight(c),
            fl::Role::Text => r.text(c),
            fl::Role::Mute => r.mute(c),
            // A theme without step colours leaves the skin's own step faces (only re-greyed like its surfaces).
            fl::Role::StepEven if th.step_even.is_none() => r.surface(c),
            fl::Role::StepOdd if th.step_odd.is_none() => r.surface(c),
            fl::Role::StepEven | fl::Role::StepOdd => citrus.get(name).expect("token"),
            fl::Role::Fixed => match *name {
                "step_on" | "note_top" | "note_bottom" | "note_rim" | "note_text" | "note_sel_top" | "note_sel_bottom"
                | "note_sel_rim" | "note_sel_text" | "key_label" | "clip" | "cat_project" | "velocity" | "velocity_dot" => r.accent(c),
                "meter_low" | "meter_high" => r.highlight(c),
                _ => c,
            },
        };
        *p.get_mut(name).expect("token") = v;
    }
    fl::finish(p, th, skin.note_top, true);
    p.xf.retint = Some(r);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contrast(a: Color, b: Color) -> f32 {
        (a.hsl().2 - b.hsl().2).abs()
    }

    /// Adjustments start from the skin's own colours as a theme: Citrus's is the stock Dark theme; Lemon's colours
    /// Lemon as it is (within rounding), so the first knob turn changes only what it turns.
    #[test]
    fn default_text_is_the_skins_own_colours() {
        assert!(FlTheme::parse(&default_text(Design::Citrus)).is_stock());
        let own = palette(Design::Lemon, None);
        let th = FlTheme::parse(&default_text(Design::Lemon));
        assert!(!th.is_stock());
        let p = palette(Design::Lemon, Some(&th));
        let worst = Palette::NAMES
            .iter()
            .map(|n| {
                let (a, b) = (own.get(n).unwrap(), p.get(n).unwrap());
                let d = [a.r.abs_diff(b.r), a.g.abs_diff(b.g), a.b.abs_diff(b.b)].into_iter().max().unwrap();
                (d, *n)
            })
            .max()
            .unwrap();
        // The cool secondary text moves toward the theme's text hue on the way back; 20 keeps that intended drift
        // bounded while still catching a broken surface ramp.
        assert!(worst.0 <= 20, "{worst:?}");
        assert_eq!(p.a(0x2d7cd0), own.a(0x2d7cd0), "accents stay Lemon's Main");
    }

    #[test]
    fn classic_is_unchanged() {
        assert_eq!(palette(Design::Citrus, None), Palette::default());
        let th = FlTheme::parse("Hue=120\nSaturation=60\nLightness=-90\nContrast=0\nSelected=16744576\n");
        assert_eq!(palette(Design::Citrus, Some(&th)), fl::palette(&th));
    }

    #[test]
    fn lemon_uses_the_brand_colours() {
        let p = palette(Design::Lemon, None);
        assert!(p.style.lemon());
        assert_eq!(p.toolbar, lemon::DARK);
        assert_eq!(p.pill, lemon::MAIN);
        assert_eq!(p.on_glyph, lemon::COLORED_DARK);
        // Role-mapped literals follow the design: Classic's raised grey lands on the dark highlight,
        // orange highlights on lemon.
        let g = p.s(0x505355);
        assert!(g.r.abs_diff(0x66) <= 6 && g.r == g.g && g.g == g.b, "{g}");
        let (h, s, _) = p.h(0xfb9244).hsl();
        assert!((h - lemon::MAIN.hsl().0).abs() < 4.0 && s > 0.8, "{}", p.h(0xfb9244));
        assert_eq!(p.a(0xa8e44a), lemon::MAIN);
    }

    /// The restored Lemon skin and the industrial redesign are two selectable looks: different surfaces, brand colour
    /// and corner radii, with the old kit translating its brand constants for the new one.
    #[test]
    fn industrial_is_a_separate_look() {
        let lemon = palette(Design::Lemon, None);
        let industrial = palette(Design::Industrial, None);
        assert_eq!(lemon.toolbar, lemon::DARK);
        assert_eq!(industrial.toolbar, industrial::DARK);
        assert_ne!(lemon.toolbar, industrial.toolbar);
        assert_eq!(industrial.accent, industrial::MAIN);
        assert_eq!((lemon.style.radius, industrial.style.radius), (6.0, 2.5));
        assert!(lemon.style.lemon() && industrial.style.lemon(), "one kit, two treatments");
        assert_eq!(industrial.brand_sel(lemon::MAIN), industrial::MAIN);
        assert_eq!(industrial.brand_lit(lemon::HIGHLIGHT), industrial::HIGHLIGHT);
        assert_eq!(industrial.brand_grey(lemon::DARK_HIGHLIGHT), industrial::DARK_HIGHLIGHT);
        assert_eq!(lemon.brand_sel(lemon::MAIN), lemon::MAIN, "the old skin is unchanged");
    }

    #[test]
    fn lemon_text_is_readable() {
        let p = palette(Design::Lemon, None);
        for (fg, bg) in [
            (p.text, p.window_title),
            (p.text_dim, p.window_title),
            (p.glyph, p.toolbar),
            (p.menu_text, p.menu_panel),
            (p.browser_folder, p.browser_list),
            (p.pill_text, p.pill),
            (p.on_glyph, p.on_bottom),
            (p.menu_pop_text, p.menu_pop_hi),
            (p.name_text, p.name_button),
            (p.ruler_text, p.ruler),
            (p.slot_text_used, p.slot),
            (p.note_text, p.note_top),
        ] {
            assert!(contrast(fg, bg) >= 0.38, "{fg} on {bg}");
        }
    }

    /// A theme colours the Lemon skin (spec: ui/theme-presets.md "Skins"): Lemon's shapes stay, the theme's
    /// selection replaces Main, its surfaces are light where Citrus's clearly are, and text stays readable on them.
    #[test]
    fn themes_colour_lemon() {
        for (name, text) in crate::presets::PRESETS {
            let th = FlTheme::parse(text);
            let p = palette(Design::Lemon, Some(&th));
            assert!(p.style.lemon(), "{name}");
            assert_eq!(p.accent, th.selected.unwrap(), "{name}");
            assert_eq!(p.a(0xa8e44a), th.selected.unwrap(), "{name}");
            let citrus = palette(Design::Citrus, Some(&th));
            // Light where Citrus is clearly light, dark where it is clearly dark (a mid-grey theme may sit either side).
            let (l, cl) = (p.toolbar.hsl().2, citrus.toolbar.hsl().2);
            assert!(!(cl > 0.6 && l <= 0.5) && !(cl < 0.4 && l >= 0.5), "{name}: light like Citrus ({l} vs {cl})");
            for (fg, bg) in [(p.text, p.window_title), (p.glyph, p.toolbar), (p.menu_text, p.menu_panel), (p.name_text, p.name_button)] {
                assert!(contrast(fg, bg) >= 0.3, "{name}: {fg} on {bg}");
            }
        }
    }

    /// Meridian is a fourth, independent look and the default for a fresh install. Its shapes are its own, its ink
    /// is not Industrial's graphite, and a theme still recolours it without changing those shapes.
    #[test]
    fn meridian_is_the_default_and_a_separate_look() {
        assert_eq!(Design::default(), Design::Meridian);
        let p = palette(Design::Meridian, None);
        assert!(p.style.meridian());
        assert_eq!(p.style.radius, 2.0);
        assert_eq!(p.accent, meridian::MAIN);
        assert_ne!(p.toolbar, palette(Design::Industrial, None).toolbar);
        assert_ne!(p.note_top, palette(Design::Industrial, None).note_top);
        assert_eq!(Design::Meridian.theme_index(), None, "a skin is not a colour theme");
        assert_eq!(Design::Lemon.theme_index(), Some(0));
        assert_eq!(Design::Citrus.theme_index(), Some(2));
        let th = FlTheme::parse(crate::presets::PRESETS[0].1);
        let themed = palette(Design::Meridian, Some(&th));
        assert!(themed.style.meridian(), "a theme keeps Meridian's geometry");
        assert_eq!(themed.accent, th.selected.unwrap());
    }

    #[test]
    fn meridian_text_is_readable() {
        let p = palette(Design::Meridian, None);
        for (fg, bg) in [
            (p.text, p.window_title),
            (p.text_dim, p.window_title),
            (p.glyph, p.toolbar),
            (p.menu_text, p.menu_panel),
            (p.browser_folder, p.browser_list),
            (p.pill_text, p.pill),
            (p.on_glyph, p.on_bottom),
            (p.menu_pop_text, p.menu_pop_hi),
            (p.name_text, p.name_button),
            (p.ruler_text, p.ruler),
            (p.slot_text_used, p.slot),
            (p.note_text, p.note_top),
        ] {
            assert!(contrast(fg, bg) >= 0.35, "{fg} on {bg}");
        }
    }

    #[test]
    fn names_round_trip() {
        for d in Design::ALL {
            assert_eq!(Design::parse(d.name()), Some(d));
        }
        assert_eq!(Design::Lemon.next(), Design::Industrial);
        assert_eq!(Design::Industrial.next(), Design::Citrus);
        assert_eq!(Design::parse("Classic"), Some(Design::Citrus));
    }
}
