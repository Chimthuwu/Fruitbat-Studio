// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The native plugins' own editors, drawn by the app: a device rack tile shows its device's real UI — the same
//! panels, dials, switches, captions, value texts and wordmark its window shows — rather than a generic knob grid of
//! its parameters.
//!
//! Every native Lemon plugin paints its editor from a value source, never from its own parameters directly:
//!
//! - the MIDI FX and the note / audio containers through [`os_plugkit::simple::paint`] with a
//!   [`os_plugkit::simple::Values`] (their window uses the plugin's [`os_plugkit::vals::Vals`], this draws from the
//!   host's parameter list),
//! - the small effects (Lemon Filter, Balance, Chorus, …) through `lemon_small_fx::view::paint` with its `Model`,
//! - Lemon Reverb through `lemon_reverb::view::paint` with its `Model` and room visual,
//! - the larger native editors (Delay, Limiter, Slicer, Sampler, Parametric EQ, Press, Pulp Filter, Tesla) through
//!   their own `view::paint`; the parameter-driven ones edit the host's parameters, while Press, Pulp Filter and
//!   Tesla keep their edits in their saved state and stay read-only in the dock (the pop-out edits them).
//!
//! So the app renders an editor into an offscreen canvas at the tile's size ([`Ui::render`]), blits it in the tile
//! (`views::device_rack::controls`) and keeps the editor's hit regions, which come back with the pixels: the tile's
//! dials, switches and selectors are live and set the hosted parameter they name. Rendering is cached by the
//! parameters' values ([`Cache`]), so repainting a tile whose values did not move is a blit.
//!
//! A plugin whose editor is not one of those ([`ui`] has no entry: an instrument, a third-party plugin) keeps the
//! parameter grid and gets its window from the tile's menu (Pop out).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use os_ui::{Canvas, Rect};

use crate::app::State;
use crate::plugins::Plugins;
use crate::device_rack::RackParam;
use crate::osc3::LEMON_TRIO;
use crate::plugins::LEMON_DRUM_SAMPLER;

/// A native plugin's editor, as the rack draws it.
#[derive(Clone, Copy)]
pub enum Ui {
    /// One of the plugins whose editor is the shared `os_plugkit::simple` one: the MIDI FX and the containers.
    Simple(&'static os_plugkit::simple::Info),
    /// One of the small effects (`lemon-small-fx`): its `Spec` is the whole layout.
    Small(&'static lemon_small_fx::Spec),
    /// Lemon Reverb, room visual included.
    Reverb,
    /// Lemon Trio: the full editor (oscillators / envelopes / sample), drawn from the host's parameters.
    Trio,
    /// Lemon Drum Sampler (the drum machine): the 16-pad grid in the dock, or — flipped — its pad and
    /// layer cards (the card editing stays in the pop-out window). Not the plain `Lemon Sampler`.
    Drum,
    /// Lemon Parametric EQ: the curve, band faders / FREQ / BW knobs and main level.
    Eq,
    /// Lemon Delay: the cards, its faders and its selector rows.
    Delay,
    /// Lemon Limiter: its display and LIMIT / COMP pages.
    Limiter,
    /// Lemon Sampler: its sample page (the waveform stays empty in the dock; the pop-out shows it).
    Sampler,
    /// Lemon Slicer: its slice card and play / options controls.
    Slicer,
    /// Press (Maximus): the full maximiser editor, drawn from its saved state.
    Press,
    /// Pulp Filter (Love Philter): the full filter editor, drawn from its saved state.
    Pulp,
    /// Tesla (Edison): the audio editor, drawn from its saved state.
    Tesla,
}

/// Reserved `id` values in an editor hit that name an editor-only action rather than a hosted parameter: a tile
/// switches its page, chooses a drum pad, etc. No hosted parameter may legitimately carry these ids.
pub const EDITOR_TAB: u32 = u32::MAX;
pub const EDITOR_SUBTAB: u32 = u32::MAX - 1;
pub const EDITOR_PAD: u32 = u32::MAX - 2;
pub const EDITOR_BANK: u32 = u32::MAX - 3;
/// The editor's own Load button (the drum's flipped cards): the full editor's window has the picker.
pub const EDITOR_OPEN: u32 = u32::MAX - 4;
/// The Parametric EQ's band token in the graph: the value is the band. Pressing it starts the same
/// frequency / level drag the plugin's window has, so the bands can be moved in the dock's graph.
pub const EDITOR_EQ_TOKEN: u32 = u32::MAX - 5;
/// The Parametric EQ's graph well (outside the tokens): a vertical drag moves the main level, as in the
/// plugin's window.
pub const EDITOR_EQ_GRAPH: u32 = u32::MAX - 6;

/// The plugins whose editors the rack draws: the hosted plugin's name (its descriptor's) to the UI that paints it.
///
/// A plugin missing here is not an error — the tile falls back to the parameter grid — so this grows as plugins get
/// a shared view. The name of every entry is asserted against its own `Info` / `Spec` in the tests.
pub fn ui(name: &str) -> Option<Ui> {
    Some(match name {
        // MIDI FX (spec: plugins/lemon-arp.md, lemon-chord.md, …) and the note / audio layer + selector containers.
        "Lemon Arp" => Ui::Simple(&lemon_arp::editor::INFO),
        "Lemon Chord" => Ui::Simple(&lemon_chord::editor::INFO),
        "Lemon Echo" => Ui::Simple(&lemon_echo::editor::INFO),
        "Lemon Transpose" => Ui::Simple(&lemon_transpose::editor::INFO),
        "Lemon Quantize" => Ui::Simple(&lemon_quantize::editor::INFO),
        "Lemon Note Length" => Ui::Simple(&lemon_note_length::editor::INFO),
        "Lemon Key Filter" => Ui::Simple(&lemon_key_filter::editor::INFO),
        "Lemon Humanize" => Ui::Simple(&lemon_humanize::editor::INFO),
        "Lemon Velocity Curve" => Ui::Simple(&lemon_velocity_curve::editor::INFO),
        "Lemon Note Delay" => Ui::Simple(&lemon_note_delay::editor::INFO),
        "Lemon Note Repeats" => Ui::Simple(&lemon_note_repeats::editor::INFO),
        "Lemon Note Filter" => Ui::Simple(&lemon_note_filter::editor::INFO),
        "Lemon Random" => Ui::Simple(&lemon_random::editor::INFO),
        "Lemon Strum" => Ui::Simple(&lemon_strum::editor::INFO),
        "Lemon Note Layer" => Ui::Simple(&lemon_note_layer::editor::INFO),
        "Lemon Note Selector" => Ui::Simple(&lemon_note_selector::editor::INFO),
        "Lemon Audio Layer" => Ui::Simple(&lemon_audio_layer::editor::INFO),
        "Lemon Audio Selector" => Ui::Simple(&lemon_audio_selector::editor::INFO),
        // Native audio FX (spec: plugins/lemon-small-effects.md, lemon-reverb.md).
        "Lemon Filter" => Ui::Small(&lemon_small_fx::filter::SPEC),
        "Lemon Balance" => Ui::Small(&lemon_small_fx::balance::SPEC),
        "Lemon Chorus" => Ui::Small(&lemon_small_fx::chorus::SPEC),
        "Lemon Fast Distortion" => Ui::Small(&lemon_small_fx::fast_distortion::SPEC),
        "Lemon Flanger" => Ui::Small(&lemon_small_fx::flanger::SPEC),
        "Lemon Phaser" => Ui::Small(&lemon_small_fx::phaser::SPEC),
        "Lemon Soft Clipper" => Ui::Small(&lemon_small_fx::soft_clipper::SPEC),
        "Lemon Stereo Enhancer" => Ui::Small(&lemon_small_fx::stereo_enhancer::SPEC),
        "Lemon Stereo Shaper" => Ui::Small(&lemon_small_fx::stereo_shaper::SPEC),
        "Lemon Transient Processor" => Ui::Small(&lemon_small_fx::transient_processor::SPEC),
        "Lemon Vocoder" => Ui::Small(&lemon_small_fx::vocoder::SPEC),
        "Lemon Formula Controller" => Ui::Small(&lemon_small_fx::formula_controller::SPEC),
        "Lemon Peak Controller" => Ui::Small(&lemon_small_fx::peak_controller::SPEC),
        "Lemon Reverb" => Ui::Reverb,
        "Lemon Trio" => Ui::Trio,
        "Lemon Parametric EQ" => Ui::Eq,
        "Lemon Delay" => Ui::Delay,
        "Lemon Limiter" => Ui::Limiter,
        "Lemon Sampler" => Ui::Sampler,
        "Lemon Slicer" => Ui::Slicer,
        "Press" => Ui::Press,
        "Pulp Filter" => Ui::Pulp,
        "Tesla" => Ui::Tesla,
        // The drum machine — NOT the plain `Lemon Sampler` (a different plugin, which keeps the
        // parameter grid): the pads and their cards come from its saved state, not its parameters.
        LEMON_DRUM_SAMPLER => Ui::Drum,
        _ => return None,
    })
}

impl Ui {
    /// The plugin's name ("Lemon Arp").
    pub fn name(&self) -> &'static str {
        match self {
            Ui::Simple(info) => info.name,
            Ui::Small(spec) => spec.name,
            Ui::Reverb => "Lemon Reverb",
            Ui::Trio => LEMON_TRIO,
            Ui::Drum => LEMON_DRUM_SAMPLER,
            Ui::Eq => "Lemon Parametric EQ",
            Ui::Delay => "Lemon Delay",
            Ui::Limiter => "Lemon Limiter",
            Ui::Sampler => "Lemon Sampler",
            Ui::Slicer => "Lemon Slicer",
            Ui::Press => "Press",
            Ui::Pulp => "Pulp Filter",
            Ui::Tesla => "Tesla",
        }
    }

    /// The editor's natural size in logical px (its window's).
    pub fn size(&self) -> (f32, f32) {
        match self {
            Ui::Simple(info) => (info.w as f32, info.h as f32),
            Ui::Small(spec) => (spec.size.0 as f32, spec.size.1 as f32),
            Ui::Reverb => (lemon_reverb::view::W as f32, lemon_reverb::view::H as f32),
            // The whole editor, its own page strip included: the dock shows the same oscillators / envelopes /
            // sample buttons the window does (as the Sampler's tile does its strip).
            Ui::Trio => (lemon_3x_osc::view::EDITOR_W as f32, lemon_3x_osc::view::EDITOR_H as f32),
            // The dock shows only the pad grid (the cards stay in the pop-out), cropped at the grid's right edge.
            Ui::Drum => (lemon_drum_sampler::view::GRID.right() + lemon_drum_sampler::view::GRID.x, lemon_drum_sampler::view::H as f32),
            Ui::Eq => (lemon_parametric_eq_2::view::W as f32, lemon_parametric_eq_2::view::H as f32),
            Ui::Delay => (lemon_delay::view::W as f32, lemon_delay::view::H as f32),
            Ui::Limiter => (lemon_limiter::view::W as f32, lemon_limiter::view::H as f32),
            Ui::Sampler => (lemon_sampler::view::W as f32, lemon_sampler::view::H as f32),
            Ui::Slicer => (lemon_slicer::view::W as f32, lemon_slicer::view::H as f32),
            Ui::Press => (press::view::W as f32, press::view::H as f32),
            Ui::Pulp => (pulp_filter::view::W as f32, pulp_filter::view::H as f32),
            Ui::Tesla => (tesla::view::W as f32, tesla::view::H as f32),
        }
    }

    /// The editor's parameters as its own painting names them, with the name a host shows: the map from a view id
    /// ("hcut", "pattern") to the hosted parameter's name ("High cut", "Pattern"). Names are owned because the
    /// EQ's and the sampler's are built from their band / parameter lists rather than written as literals.
    pub fn settings(&self) -> Vec<(String, String)> {
        match self {
            Ui::Simple(info) => info.settings.iter().map(|s| (s.id.to_string(), s.name.to_string())).collect(),
            Ui::Small(spec) => spec.settings.iter().map(|s| (s.id.to_string(), s.name.to_string())).collect(),
            Ui::Reverb => {
                use lemon_reverb::params::{DEFS, MID_SIDE_ID, MID_SIDE_NAME, TEMPO_ID, TEMPO_NAME};
                let mut v: Vec<(String, String)> =
                    DEFS.iter().map(|d| (d.id.to_string(), d.name.to_string())).collect();
                v.push((TEMPO_ID.to_string(), TEMPO_NAME.to_string()));
                v.push((MID_SIDE_ID.to_string(), MID_SIDE_NAME.to_string()));
                v
            }
            Ui::Trio => lemon_3x_osc::params::ui_settings()
                .into_iter()
                .map(|(id, name)| (id.to_string(), name.to_string()))
                .collect(),
            // The drum sampler exposes no host parameters: its pads are drawn from its saved state, not the
            // parameter list, so the tile has no (param-based) settings to name.
            Ui::Drum => Vec::new(),
            Ui::Eq => {
                let mut v = Vec::with_capacity(36);
                for (id, name) in [
                    ("level", "level"),
                    ("freq", "freq"),
                    ("width", "width"),
                    ("type", "type"),
                    ("order", "order"),
                ] {
                    for k in 0..os_engine::effects::eq::BANDS {
                        v.push((format!("{id}{}", k + 1), format!("Band {} {name}", k + 1)));
                    }
                }
                v.push(("main".to_string(), "Main level".to_string()));
                v
            }
            Ui::Delay => lemon_delay::params::DEFS.iter().map(|d| (d.id.to_string(), d.name.to_string())).collect(),
            Ui::Limiter => lemon_limiter::params::DEFS.iter().map(|d| (d.id.to_string(), d.name.to_string())).collect(),
            Ui::Slicer => lemon_slicer::params::NAMES.iter().map(|(id, name)| (id.to_string(), name.to_string())).collect(),
            Ui::Sampler => lemon_sampler::params::ui_settings().to_vec(),
            // The state-driven editors expose no host parameters: their controls live in the saved state, not in
            // the parameter list, so there is nothing for the tile's hit regions to name.
            Ui::Press | Ui::Pulp | Ui::Tesla => Vec::new(),
        }
    }

    /// The device-pixel size an editor is drawn at for `factor` device px per editor px.
    pub fn pixels(&self, factor: f32) -> (u32, u32) {
        let (w, h) = self.size();
        let factor = factor.max(0.02);
        ((w * factor).round().max(1.0) as u32, (h * factor).round().max(1.0) as u32)
    }

    /// Paints the editor into `c` (already sized by [`Ui::pixels`], which sets its scale so the glyphs are rasterised
    /// at the tile's size and a scaled-down tile stays sharp — ui/ui-scaling.md) and gives every control's hit region
    /// with the hosted parameter id it sets.
    pub fn paint_into(
        &self,
        c: &mut Canvas,
        params: &[RackParam],
        view: &TileView,
        plugins: &Plugins,
        key: usize,
    ) -> Vec<(Rect, u32, Option<f64>)> {
        let vals = Values::new(self, params);
        // The active palette, not the stock Lemon one: a native editor in the rack follows the app's theme like
        // the rest of the tile (its window does too, through `os_theme::current`).
        let t = os_theme::current();
        match *self {
            Ui::Simple(info) => {
                let st = os_plugkit::simple::ViewState::default();
                os_plugkit::simple::paint(info, c, &vals, &st, "")
                    .into_iter()
                    .filter_map(|(r, id)| {
                        let p = vals.param(&id)?;
                        let v = p.toggle.then(|| 1.0 - p.value);
                        Some((r, p.id, v))
                    })
                    .collect()
            }
            Ui::Small(spec) => {
                let st = lemon_small_fx::view::ViewState::default();
                lemon_small_fx::view::paint(spec, c, &t, &vals, &st, "")
                    .into_iter()
                    .filter_map(|hit| {
                        let id = hit.t.id();
                        let p = vals.param(id)?;
                        let v = match &hit.t {
                            lemon_small_fx::view::Target::Choice(_, i) => leaned(p, f64::from(*i)),
                            _ => None,
                        };
                        Some((hit.r, p.id, v))
                    })
                    .collect()
            }
            Ui::Reverb => {
                use os_engine::effects::reverb::PARAMS;
                let knobs: [i32; PARAMS] =
                    std::array::from_fn(|k| <Values as os_plugkit::simple::Values>::plain(&vals, lemon_reverb::params::DEFS[k].id).round() as i32);
                let look = lemon_reverb::view::Look { room: lemon_reverb::room::look(&knobs), angle: 0.0 };
                let st = lemon_reverb::view::ViewState::default();
                lemon_reverb::view::paint(c, &t, &vals, &st, &look, "")
                    .into_iter()
                    .filter_map(|hit| {
                        use lemon_reverb::view::Target;
                        let p = vals.param(match &hit.t {
                            Target::Knob(id) | Target::Fader(id) | Target::Switch(id) => id,
                            Target::Room | Target::Logo => return None,
                        })?;
                        let v = matches!(hit.t, Target::Switch(_)).then(|| 1.0 - p.value);
                        Some((hit.r, p.id, v))
                    })
                    .collect()
            }
            Ui::Trio => {
                let TileView::Trio(vst) = view else { return Vec::new() };
                // The editor names its controls by string id; the rack's `Values` bridge resolves each to the
                // hosted parameter code. Snapshot the map before `paint` takes `vals` by value.
                let ids = vals.ids.clone();
                // `hosted: false`: the editor draws its own page strip, so the dock has the same oscillators /
                // envelopes / sample buttons (and the same envelope access) as its window.
                let hits = lemon_3x_osc::view::paint(c, &t, &vals, vst, "", false);
                use lemon_3x_osc::view::Target;
                let mut out = Vec::new();
                for hit in hits {
                    let lemon_3x_osc::view::Hit { r, t } = hit;
                    match t {
                        Target::Tab(n) => out.push((r, EDITOR_TAB, Some(n as f64))),
                        Target::SubTab(n) => out.push((r, EDITOR_SUBTAB, Some(n as f64))),
                        // A dial keeps its drag: the press sets where it is and the drag follows the mouse
                        // (`ParamSet`); a list drags too.
                        Target::Knob(s) | Target::List(s) => {
                            let Some(code) = ids.get(s.as_str()).copied() else { continue };
                            if params.iter().any(|p| p.id == code) {
                                out.push((r, code, None));
                            }
                        }
                        // A switch flips: set once, no drag.
                        Target::Toggle(s) => {
                            let Some(code) = ids.get(s.as_str()).copied() else { continue };
                            let Some(p) = params.iter().find(|p| p.id == code) else { continue };
                            out.push((r, code, Some(1.0 - p.value)));
                        }
                        // A selector row picks its own position: set once, no drag.
                        Target::Set(s, i) => {
                            let Some(code) = ids.get(s.as_str()).copied() else { continue };
                            let Some(p) = params.iter().find(|p| p.id == code) else { continue };
                            out.push((r, code, leaned(p, f64::from(i))));
                        }
                        // The switches row (oscillator on/off and friends): one `flags` parameter, a bit
                        // per switch — the press flips its own bit.
                        Target::Flag(n) => {
                            let Some(code) = ids.get("flags").copied() else { continue };
                            let Some(p) = params.iter().find(|p| p.id == code) else { continue };
                            out.push((r, code, leaned(p, f64::from((p.plain as i64 ^ (1 << n)) as i32))));
                        }
                        _ => {}
                    }
                }
                out
            }
            Ui::Drum => {
                let TileView::Drum { view: vst, state, loaded, lit, .. } = view else { return Vec::new() };
                let look = lemon_drum_sampler::view::Look { state, loaded: &loaded[..], lit: &lit[..] };
                let hits = lemon_drum_sampler::view::paint(c, &t, vst, &look, "");
                let mut out = Vec::new();
                for hit in hits {
                    let lemon_drum_sampler::view::Hit { r, t } = hit;
                    match t {
                        // A pad picks (and plays); a bank picks its bank. Both only land in the pads
                        // half — the flip crop below drops whatever is outside the shown half.
                        lemon_drum_sampler::view::Target::Pad(n) => out.push((r, EDITOR_PAD, Some(n as f64))),
                        lemon_drum_sampler::view::Target::Bank(n) => out.push((r, EDITOR_BANK, Some(n as f64))),
                        // The flipped half's Load: the picker lives in the full editor's window.
                        lemon_drum_sampler::view::Target::Load => out.push((r, EDITOR_OPEN, None)),
                        _ => {}
                    }
                }
                out
            }
            Ui::Eq => {
                use lemon_parametric_eq_2::params;
                let extra = plugins.eq_file_state(key).unwrap_or_else(params::default_state);
                let settings = eq_settings(&vals, &extra);
                let look = lemon_parametric_eq_2::view::Look {
                    s: settings,
                    extra: &extra,
                    spectrum: None,
                    rate: 48_000.0,
                    faders: matches!(view, TileView::Eq { faders: true }),
                };
                let st = lemon_parametric_eq_2::view::ViewState::default();
                let mut out = Vec::new();
                for hit in lemon_parametric_eq_2::view::paint(c, &look, &st) {
                    use lemon_parametric_eq_2::view::{Target, MAIN_FADER};
                    match hit.t {
                        // The tokens are dragged as one control (frequency across, level up / down): the
                        // tile's action handles both parameters, as the plugin's own token drag does.
                        Target::Token(k) => {
                            out.push((hit.r, EDITOR_EQ_TOKEN, Some(k as f64)));
                            continue;
                        }
                        // Empty graph: the plugin's window drags the main level there.
                        Target::Graph => {
                            out.push((hit.r, EDITOR_EQ_GRAPH, None));
                            continue;
                        }
                        _ => {}
                    }
                    let (id, v) = match hit.t {
                        Target::Fader(k) if k == MAIN_FADER => ("main".to_string(), None),
                        Target::Fader(k) => (format!("level{}", k + 1), None),
                        Target::Freq(k) => (format!("freq{}", k + 1), None),
                        Target::Bw(k) => (format!("width{}", k + 1), None),
                        _ => continue,
                    };
                    let Some(p) = vals.param(&id) else { continue };
                    out.push((hit.r, p.id, v));
                }
                out
            }
            Ui::Delay => {
                let st = lemon_delay::view::ViewState::default();
                lemon_delay::view::paint(c, &t, &vals, &st, "")
                    .into_iter()
                    .filter_map(|hit| {
                        use lemon_delay::view::Target;
                        let p = vals.param(hit.t.id())?;
                        let v = match &hit.t {
                            Target::Switch(_) => Some(1.0 - p.value),
                            Target::Choice(_, i) => leaned(p, f64::from(*i)),
                            _ => None,
                        };
                        Some((hit.r, p.id, v))
                    })
                    .collect()
            }
            Ui::Limiter => {
                let st = lemon_limiter::view::ViewState::default();
                let look = lemon_limiter::view::Look { sidechain: 0, history: &[] };
                lemon_limiter::view::paint(c, &t, &vals, &st, &look, "")
                    .into_iter()
                    .filter_map(|hit| {
                        use lemon_limiter::view::Target;
                        let id = match &hit.t {
                            Target::Knob(id) | Target::Spin(id) => id,
                            _ => return None,
                        };
                        let p = vals.param(id)?;
                        Some((hit.r, p.id, None))
                    })
                    .collect()
            }
            Ui::Slicer => {
                let raw = plugins
                    .state(key)
                    .and_then(|b| lemon_slicer::params::fl_from_plugin_state(&b))
                    .unwrap_or_default();
                let state = lemon_slicer::state::State::parse(&raw).unwrap_or_default();
                let look = lemon_slicer::view::Look { state: &state, frames: 0, peaks: &[], can_dump: false };
                let st = lemon_slicer::view::ViewState::default();
                lemon_slicer::view::paint(c, &t, &vals, &st, &look, "")
                    .into_iter()
                    .filter_map(|hit| {
                        use lemon_slicer::view::Target;
                        let id = match &hit.t {
                            Target::Knob(id) | Target::Switch(id) => id,
                            _ => return None,
                        };
                        let p = vals.param(id)?;
                        let v = matches!(hit.t, Target::Switch(_)).then(|| 1.0 - p.value);
                        Some((hit.r, p.id, v))
                    })
                    .collect()
            }
            Ui::Sampler => {
                let st = lemon_sampler::view::ViewState::default();
                let sv = lemon_sampler::view::SampleView {
                    sample: None,
                    path: "",
                    bits: 32,
                    spectrum: None,
                    tones: None,
                };
                let mut out = Vec::new();
                for hit in lemon_sampler::view::paint(c, &t, &vals, &st, &sv, "", false) {
                    use lemon_sampler::view::Target;
                    let Some(id) = hit.t.param() else { continue };
                    let Some(p) = vals.param(id) else { continue };
                    let v = match &hit.t {
                        Target::Toggle(_) => Some(1.0 - p.value),
                        Target::Set(_, n) => leaned(p, f64::from(*n)),
                        Target::Knob(_) | Target::Slider(_, _) | Target::LoopHandle(_) | Target::Key(_) => None,
                        _ => continue,
                    };
                    out.push((hit.r, p.id, v));
                }
                out
            }
            Ui::Press => {
                let raw = plugins.maximus_file_state(key).unwrap_or_default();
                let state = press::state_of(&raw);
                let history: [[[f32; 2]; os_engine::effects::maximus::state::BANDS]; 0] = [];
                let model = press::view::Model { st: &state, history: &history, version: "0.0.0" };
                let _ = press::view::paint(c, &model, &press::view::ViewState::default(), "");
                Vec::new()
            }
            Ui::Pulp => {
                let raw = plugins.love_philter_file_state(key).unwrap_or_default();
                let state = pulp_filter::state_of(&raw);
                let model = pulp_filter::view::Model { st: &state, positions: [512, 512, 0], version: "0.0.0" };
                let _ = pulp_filter::view::paint(c, &model, &pulp_filter::view::ViewState::default(), "");
                Vec::new()
            }
            Ui::Tesla => {
                let raw = plugins.edison_file_state(key).unwrap_or_default();
                let saved = tesla::state::from_edison(&raw).unwrap_or_default();
                let doc = tesla::doc::Doc::from_saved(saved);
                let channels: Vec<&[f32]> = doc.sample.ch.iter().map(|c| c.as_slice()).collect();
                let peaks = tesla::peaks::Peaks::new(&channels);
                let shown = tesla::view::Shown {
                    parts: vec![tesla::view::Part { ch: channels, peaks: &peaks, at: 0 }],
                    rate: doc.sample.rate.max(1),
                    channels: doc.sample.channels(),
                };
                let st = tesla::view::ViewState::default();
                let look = tesla::view::Look {
                    doc: &doc,
                    shown: &shown,
                    st: &st,
                    hint: "",
                    hosted: true,
                    free: String::new(),
                    playing: false,
                    pos: 0.0,
                    armed: false,
                    capturing: false,
                    level: 0.0,
                    rec: &doc.rec,
                    looping: false,
                    can_undo: false,
                    spectrum: None,
                };
                let _ = tesla::view::paint(c, &look, tesla::view::W, tesla::view::H);
                Vec::new()
            }
        }
    }
}

/// The docked EQ's band settings, from each parameter's *normalized* value and the range its editor documents (a
/// band level is ±18 dB, a type 0..=7, an order -3..=3). The host reports the built-in EQ's `ParamInfo.min/max` as
/// 0..1 (a step count for the two integer parameters), so `plain` there is just the normalized value: reading it as
/// dB flattened the docked graph and left every band token on the 0 dB line while the pop-out showed the real curve.
fn eq_settings(vals: &Values, extra: &[u8]) -> os_engine::effects::eq::Settings {
    use os_engine::effects::eq;
    let nrm = |id: &str| vals.param(id).map_or(0.0, |p| p.value);
    let bands: [eq::Band; eq::BANDS] = std::array::from_fn(|k| {
        let n = k + 1;
        eq::Band {
            db: -18.0 + 36.0 * nrm(&format!("level{n}")),
            freq: nrm(&format!("freq{n}")),
            width: nrm(&format!("width{n}")),
            kind: (nrm(&format!("type{n}")) * 7.0).round() as i32,
            order: (-3.0 + 6.0 * nrm(&format!("order{n}"))).round() as i32,
        }
    });
    eq::Settings { bands, main_db: -18.0 + 36.0 * nrm("main"), mask: extra.get(eq::MASK_BYTE).copied().unwrap_or(0x7f) }
}

/// A selector row's normalized value: position `i` of a parameter whose plain range is `p.min ..= p.max`.
fn leaned(p: &RackParam, i: f64) -> Option<f64> {
    (p.max > p.min).then(|| ((i - p.min) / (p.max - p.min)).clamp(0.0, 1.0))
}

/// A rendered editor: `w` x `h` device pixels (`0x00RRGGBB`) and its controls' hit regions in the editor's logical
/// px, each with the hosted parameter it sets and the value a press gives it (`None`: a drag from where it is).
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
    pub hits: Vec<(Rect, u32, Option<f64>)>,
}

/// A tile's rendered editor, and what it was rendered from.
pub struct Entry {
    name: String,
    /// The parameters' values when it was drawn (a change redraws it).
    sig: u64,
    /// The applied palette when it was drawn (a theme change redraws it).
    theme: usize,
    /// The tile's own editor state when it was drawn (the Trio's page, the drum's pad): a page switch redraws it.
    vsig: u64,
    factor: f32,
    /// The offscreen editor canvas, kept between redraws: a knob drag redraws every frame and building the canvas
    /// (its font engine) each time would cost more than the painting.
    canvas: Option<Canvas>,
    /// The pixels, kept for the same reason.
    buf: Vec<u32>,
    image: Rc<Image>,
    /// Per-tile state for the native-instrument editors (the Trio's page, the drum's bank/pad); `Plain` otherwise.
    pub view: TileView,
}

impl Entry {
    fn new(name: &str) -> Self {
        let image = Rc::new(Image { w: 1, h: 1, px: vec![0], hits: Vec::new() });
        Entry { name: name.to_string(), sig: 0, theme: 0, vsig: 0, factor: 0.0, canvas: None, buf: Vec::new(), image, view: TileView::Plain }
    }

    /// Make sure `view` matches the UI about to be painted: if the UI changed (or was never set), reset it to that
    /// UI's default interactive state so we don't hand the wrong editor the wrong state. `Plain` satisfies only
    /// the plain editors (`Simple`/`Reverb`): a Trio or Drum entry starting `Plain` (the `Entry::new` default)
    /// must be upgraded, or its paint arm bails and the tile stays black.
    fn ensure_view(&mut self, ui: Ui) {
        let matches = matches!(
            (&self.view, ui),
            (TileView::Trio(_), Ui::Trio)
                | (TileView::Drum { .. }, Ui::Drum)
                | (TileView::Eq { .. }, Ui::Eq)
                | (
                    TileView::Plain,
                    Ui::Simple(_)
                        | Ui::Small(_)
                        | Ui::Reverb
                        | Ui::Delay
                        | Ui::Limiter
                        | Ui::Sampler
                        | Ui::Slicer
                        | Ui::Press
                        | Ui::Pulp
                        | Ui::Tesla
                )
        );
        if !matches {
            self.view = match ui {
                Ui::Eq => TileView::Eq { faders: false },
                Ui::Trio => TileView::Trio(lemon_3x_osc::view::ViewState::default()),
                Ui::Drum => TileView::Drum {
                    view: lemon_drum_sampler::view::ViewState::default(),
                    flip: false,
                    state: lemon_drum_sampler::state::State::default(),
                    loaded: vec![false; 32],
                    lit: vec![false; 32],
                    mode: preferred_mode(),
                    page: 0,
                    lane: 0,
                },
                _ => TileView::Plain,
            };
        }
    }
}

/// The rack's editor images, by plugin-window key (`State::device_rack_params`'s): one per device.
#[derive(Default)]
pub struct Cache {
    entries: BTreeMap<usize, Entry>,
}

impl Cache {
    /// Drops the entries whose key is no longer a device tile.
    pub fn retain(&mut self, keys: &[usize]) {
        self.entries.retain(|k, _| keys.contains(k));
    }
    /// The tile's cache entry, to read or change its per-tile state (the Trio's page, the drum's chosen pad).
    pub fn get_mut(&mut self, key: &usize) -> Option<&mut Entry> {
        self.entries.get_mut(key)
    }

    /// The tile's cached view (its editor's own selection state), for the header's buttons.
    pub fn view(&self, key: &usize) -> Option<&TileView> {
        self.entries.get(key).map(|e| &e.view)
    }

    /// Makes sure a tile has its editor state before the view reads or changes it. The drum's step interface is drawn
    /// by the app, not by `image`, so it must create its cache entry itself; otherwise the first STEP / PADS click
    /// would have no tile state to change.
    pub fn ensure(&mut self, key: &usize, ui: Ui) {
        let e = self.entries.entry(*key).or_insert_with(|| Entry::new(ui.name()));
        e.ensure_view(ui);
    }
}

/// The image of the editor of the device hosted under `key`: `name` is the plugin's name, `factor` the
/// cache key (the device px per editor px the caller stretched to), `cw`/`ch` the canvas in device px to
/// cover — the tile body's own size, so the editor never letterboxes. The cached image is handed back
/// when neither the values nor the canvas moved.
///
/// The editor's own background covers the whole canvas (`paint` clears with it), so a canvas wider or
/// taller than the editor shows the editor whole at a uniform scale with its background running on —
/// no bars, no squash. The hits come back in canvas device px, origin at the canvas's top-left.
pub fn image(
    s: &State,
    plugins: &Plugins,
    key: usize,
    name: &str,
    params: &[RackParam],
    factor: f32,
    cw: u32,
    ch: u32,
) -> Option<Rc<Image>> {
    let ui = ui(name)?;
    let sig = signature(name, params);
    // A fresh palette is installed on every theme load / adjustment, so its address identifies the theme without
    // hashing it. A changed address misses the cache and repaints the tile in the new colours.
    let theme = Arc::as_ptr(&os_theme::current()) as usize;
    let mut cache = s.rack_ui_cache.borrow_mut();
    let e = cache.entries.entry(key).or_insert_with(|| Entry::new(name));
    // The drum sampler's pads reflect its live state, which the pop-out edits, so its tile is never cached:
    // rebuild it every paint so a pad load/swap shows up.
    let view = if let Ui::Drum = ui {
        drum_view(plugins, key, &e.view)
    } else {
        e.ensure_view(ui);
        e.view.clone()
    };
    let vsig = view_sig(&view);
    if e.name == name && e.sig == sig && e.theme == theme && e.vsig == vsig && (e.factor - factor).abs() < 0.005 && !matches!(ui, Ui::Drum) {
        return Some(e.image.clone());
    }
    // The drum machine's editor is painted whole (its cards sit right of its pad grid) and the tile
    // shows one half of it: the pads, or — flipped — the cards. Everything else covers its canvas whole.
    let drum = matches!(ui, Ui::Drum);
    let flip = matches!(&view, TileView::Drum { flip: true, .. });
    let (dw, dh) = if drum {
        let f = factor.max(0.02);
        (
            (lemon_drum_sampler::view::W as f32 * f).round().max(1.0) as u32,
            (lemon_drum_sampler::view::H as f32 * f).round().max(1.0) as u32,
        )
    } else {
        (cw.max(1), ch.max(1))
    };
    // Painting in the editor's own logical px with the canvas's device pixels scaled: the glyphs and strokes are
    // rasterised at the tile's size, so a tile smaller than the editor stays sharp. The drum covers its whole
    // editor (to crop the shown half out of); everything else fits whole and lets its background cover the rest.
    let scale = if drum {
        dw as f32 / lemon_drum_sampler::view::W as f32
    } else {
        (dw as f32 / ui.size().0.max(1.0))
            .min(dh as f32 / ui.size().1.max(1.0))
            .max(0.02)
    };
    match &mut e.canvas {
        Some(c) => c.resize(dw, dh, scale),
        None => e.canvas = Canvas::new(dw, dh, scale).ok(),
    }
    let canvas = e.canvas.as_mut()?;
    let hits = ui.paint_into(canvas, params, &view, plugins, key);
    e.buf.resize((dw * dh) as usize, 0);
    canvas.copy_to_xrgb(&mut e.buf);
    // The native editors' own painters use the fixed Lemon plugin palette (their windows are Lemon-branded), so
    // recolour the rendered pixels through the active theme: greys take its surfaces / text roles, the Lemon
    // accent takes its accent. This is what makes the device's panels, wells and highlights follow the theme.
    retint(&mut e.buf, &os_theme::current());
    // The drum's shown half, cropped out of the whole it painted.
    let (px, w, h, hits) = if drum {
        drum_half(&e.buf, dw, scale, flip, hits)
    } else {
        (e.buf.clone(), dw, dh, scaled_hits(hits, scale))
    };
    e.image = Rc::new(Image { w: w as usize, h: h as usize, px, hits });
    e.name = name.to_string();
    e.sig = sig;
    e.theme = theme;
    e.vsig = vsig;
    e.factor = factor;
    if matches!(ui, Ui::Drum) {
        e.view = view;
    }
    Some(e.image.clone())
}

/// Recolours a rendered editor image through `t`: chromatic pixels (the Lemon accent and its lit states) take the
/// theme accent, light neutrals its text, dark neutrals its surfaces. Lightness is kept, so the editor's hierarchy
/// and anti-aliasing survive.
fn retint(px: &mut [u32], t: &os_theme::Palette) {
    for p in px {
        let c = os_theme::Color::rgb(*p);
        let (_, s, l) = c.hsl();
        let out = if s > 0.18 {
            t.a(*p)
        } else if l > 0.45 {
            t.tx(*p)
        } else {
            t.s(*p)
        };
        *p = (u32::from(out.r) << 16) | (u32::from(out.g) << 8) | u32::from(out.b);
    }
}

/// A fingerprint of the tile's own editor state that the editor draws with: its page and sub-page, the drum's pad and
/// flipped half. A change redraws the tile (a page button changes nothing else, so without this the cached image would
/// come back and the buttons would look dead).
fn view_sig(v: &TileView) -> u64 {
    match v {
        TileView::Trio(v) => 1 + u64::from(v.tab) * 0x10 + u64::from(v.sub) * 0x100,
        TileView::Drum { view, flip, mode, page, lane, .. } => {
            2 + u64::from(*flip) * 0x10
                + view.bank as u64 * 0x100
                + view.pad as u64 * 0x1000
                + u64::from(*mode == DrumMode::Pads) * 0x1_0000
                + u64::from(*page) * 0x10_0000
                + *lane as u64 * 0x1000_0000
        }
        TileView::Eq { faders } => 4 + u64::from(*faders) * 0x10,
        TileView::Plain => 0,
    }
}

/// A fingerprint of the values an editor draws (they are all it shows besides its name).
fn signature(name: &str, params: &[RackParam]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    for p in params {
        h = (h ^ u64::from(p.id)).wrapping_mul(0x0100_0000_01b3);
        h = (h ^ p.value.to_bits()).wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// A parameter name as the two sides are compared: case, spaces and punctuation ignored ("Low cut" = "lowcut").
fn key(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// An editor's values, read from the tile's hosted parameters: the plugin's parameter ids as its own painting names
/// them, resolved to the host's parameter ids through the names the host reports.
struct Values<'a> {
    params: &'a [RackParam],
    ids: BTreeMap<String, u32>,
}

impl<'a> Values<'a> {
    fn new(ui: &Ui, params: &'a [RackParam]) -> Self {
        let mut ids = BTreeMap::new();
        for (id, name) in ui.settings() {
            let k = key(&name);
            let found = params
                .iter()
                .find(|p| key(&p.name) == k)
                // A host may decorate a name ("Predelay (ms)"): fall back to the closest start.
                .or_else(|| params.iter().find(|p| key(&p.name).starts_with(&k) && !p.name.is_empty()))
                .or_else(|| params.iter().find(|p| key(&p.name) == key(&id)));
            if let Some(p) = found {
                ids.insert(id, p.id);
            }
        }
        Values { params, ids }
    }

    fn param(&self, id: &str) -> Option<&'a RackParam> {
        let id = self.ids.get(id)?;
        self.params.iter().find(|p| p.id == *id)
    }
}

impl os_plugkit::simple::Values for Values<'_> {
    fn norm(&self, id: &str) -> f32 {
        self.param(id).map_or(0.0, |p| p.value as f32)
    }
    fn plain(&self, id: &str) -> f32 {
        self.param(id).map_or(0.0, |p| p.plain as f32)
    }
    fn text(&self, id: &str) -> String {
        self.param(id).map(|p| p.text.clone()).unwrap_or_default()
    }
}

impl lemon_small_fx::view::Model for Values<'_> {
    fn norm(&self, id: &str) -> f32 {
        <Self as os_plugkit::simple::Values>::norm(self, id)
    }
    fn plain(&self, id: &str) -> f32 {
        <Self as os_plugkit::simple::Values>::plain(self, id)
    }
    fn text(&self, id: &str) -> String {
        <Self as os_plugkit::simple::Values>::text(self, id)
    }
}

impl lemon_reverb::view::Model for Values<'_> {
    fn norm(&self, id: &str) -> f32 {
        <Self as os_plugkit::simple::Values>::norm(self, id)
    }
    fn plain(&self, id: &str) -> f32 {
        <Self as os_plugkit::simple::Values>::plain(self, id)
    }
    fn text(&self, id: &str) -> String {
        <Self as os_plugkit::simple::Values>::text(self, id)
    }
}

impl lemon_delay::view::Model for Values<'_> {
    fn norm(&self, id: &str) -> f32 {
        <Self as os_plugkit::simple::Values>::norm(self, id)
    }
    fn plain(&self, id: &str) -> f32 {
        <Self as os_plugkit::simple::Values>::plain(self, id)
    }
    fn text(&self, id: &str) -> String {
        <Self as os_plugkit::simple::Values>::text(self, id)
    }
}

impl lemon_limiter::view::Model for Values<'_> {
    fn norm(&self, id: &str) -> f32 {
        <Self as os_plugkit::simple::Values>::norm(self, id)
    }
    fn plain(&self, id: &str) -> f32 {
        <Self as os_plugkit::simple::Values>::plain(self, id)
    }
    fn text(&self, id: &str) -> String {
        <Self as os_plugkit::simple::Values>::text(self, id)
    }
}

impl lemon_slicer::view::Model for Values<'_> {
    fn norm(&self, id: &str) -> f32 {
        <Self as os_plugkit::simple::Values>::norm(self, id)
    }
    fn text(&self, id: &str) -> String {
        <Self as os_plugkit::simple::Values>::text(self, id)
    }
}

impl lemon_sampler::view::Model for Values<'_> {
    fn norm(&self, id: &str) -> f32 {
        <Self as os_plugkit::simple::Values>::norm(self, id)
    }
    fn plain(&self, id: &str) -> f32 {
        <Self as os_plugkit::simple::Values>::plain(self, id)
    }
    fn text(&self, id: &str) -> String {
        <Self as os_plugkit::simple::Values>::text(self, id)
    }
}

impl<'a> lemon_3x_osc::view::Model for Values<'a> {
    fn norm(&self, id: &str) -> f32 {
        let Some(code) = self.ids.get(id).copied() else { return 0.0 };
        let Some(p) = self.params.iter().find(|c| c.id == code) else { return 0.0 };
        p.value as f32
    }
    fn plain(&self, id: &str) -> f32 {
        let Some(code) = self.ids.get(id).copied() else { return 0.0 };
        let Some(p) = self.params.iter().find(|c| c.id == code) else { return 0.0 };
        p.plain as f32
    }
    fn text(&self, id: &str) -> String {
        let Some(code) = self.ids.get(id).copied() else { return String::new() };
        let Some(p) = self.params.iter().find(|c| c.id == code) else { return String::new() };
        p.text.clone()
    }
}

/// The drum machine's two interfaces over one instrument: the pattern programmer and the performance pads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DrumMode {
    #[default]
    Step,
    Pads,
}

/// Where the chosen interface is remembered between runs (a view preference, like the toolbar's choices).
const DRUM_MODE: &str = "drum_step_pads_view";

/// The interface a new drum tile opens with: the one last used.
pub fn preferred_mode() -> DrumMode {
    if crate::settings_file::flag(DRUM_MODE, false) { DrumMode::Pads } else { DrumMode::Step }
}

/// Keeps the chosen interface for the next run.
pub fn remember_mode(mode: DrumMode) {
    crate::settings_file::set_flag(DRUM_MODE, mode == DrumMode::Pads);
}

/// Per-tile interactive state for the native editors that keep some of it (the Trio's selected page, the drum's
/// chosen bank/pad). The rack used to paint every tile from `ViewState::default()`, which is why an instrument
/// never kept its page; this lives in the cache entry so the choice survives repaints.
#[derive(Clone)]
pub enum TileView {
    /// The simple / small / reverb editors have no tile-local state.
    Plain,
    /// Lemon Parametric EQ: whether the tile shows its fader bank (hidden by default, revealed from the header).
    Eq { faders: bool },
    /// Lemon Trio: the selected page and sub-page in its editor.
    Trio(lemon_3x_osc::view::ViewState),
    /// Lemon Drum Sampler: the chosen bank/pad, whether the tile shows its cards (the pads flip to
    /// them), plus a snapshot of its saved state so the pads can render in the dock.
    Drum {
        view: lemon_drum_sampler::view::ViewState,
        /// The tile's other half: `false` shows the 16 pads, `true` the pad and layer cards.
        flip: bool,
        state: lemon_drum_sampler::state::State,
        loaded: Vec<bool>,
        lit: Vec<bool>,
        /// The interface the user chose: the step programmer or the performance pads.
        mode: DrumMode,
        /// The first step shown in the programmer (16 per page).
        page: u32,
        /// The lane the pads selected, and the programmer highlights.
        lane: usize,
    },
}

impl Default for TileView {
    fn default() -> Self {
        TileView::Plain
    }
}

/// The drum machine's editor paints whole — its cards sit right of its pad grid — and its tile shows
/// one half of it: `px` is the whole it painted (`dw` device px wide, `scale` device px per editor px),
/// `flip` which half. The half comes back as its own image: its pixels, and its controls' hit regions
/// moved with it (a hit outside the half belongs to the half that is not showing).
fn drum_half(
    px: &[u32],
    dw: u32,
    scale: f32,
    flip: bool,
    hits: Vec<(Rect, u32, Option<f64>)>,
) -> (Vec<u32>, u32, u32, Vec<(Rect, u32, Option<f64>)>) {
    // The half's bounds in the editor's logical px.
    let (x0, x1) = if flip {
        (lemon_drum_sampler::view::GRID.right(), lemon_drum_sampler::view::W as f32)
    } else {
        (0.0, lemon_drum_sampler::view::GRID.right() + lemon_drum_sampler::view::GRID.x)
    };
    let c0 = (x0 * scale) as u32;
    let c1 = ((x1 * scale).round() as u32).min(dw);
    let cw = c1.saturating_sub(c0).max(1);
    let dh = px.len() / dw as usize;
    let mut half = Vec::with_capacity((cw as usize) * dh);
    for row in 0..dh {
        let y = row * dw as usize;
        half.extend_from_slice(&px[y + c0 as usize..y + (c0 + cw) as usize]);
    }
    let hits = hits
        .into_iter()
        .filter_map(|(mut r, id, v)| {
            let inside = r.x >= x0 && r.x + r.w <= x1;
            inside.then(|| {
                // In the half's device px, origin at the half's top-left: the crop moves the hit with
                // its pixels, and the scale takes it from the editor's logical px.
                r.x = (r.x - x0) * scale;
                r.y *= scale;
                r.w = (r.w * scale).max(1.0);
                r.h = (r.h * scale).max(1.0);
                (r, id, v)
            })
        })
        .collect();
    (half, cw, dh as u32, hits)
}

/// The editor's hit regions in the canvas's device px: the editors draw in logical px, and the canvas rasterises at
/// `scale` device px per logical px, so a hit rect must be stretched the same way as the picture. Left in logical px,
/// a hit lands up and left of the control it belongs to — a knob you have to click well right of to move.
fn scaled_hits(hits: Vec<(Rect, u32, Option<f64>)>, scale: f32) -> Vec<(Rect, u32, Option<f64>)> {
    hits.into_iter()
        .map(|(mut r, id, v)| {
            r.x *= scale;
            r.y *= scale;
            r.w = (r.w * scale).max(1.0);
            r.h = (r.h * scale).max(1.0);
            (r, id, v)
        })
        .collect()
}

/// A drum tile's per-paint view: a snapshot of the hosted sampler's state, carrying over the user's chosen
/// bank/pad so a repaint doesn't reset the selection.
fn drum_view(plugins: &Plugins, key: usize, prev: &TileView) -> TileView {
    let (view, flip, mode, page, lane) = match prev {
        TileView::Drum { view, flip, mode, page, lane, .. } => (view.clone(), *flip, *mode, *page, *lane),
        _ => (lemon_drum_sampler::view::ViewState::default(), false, preferred_mode(), 0, 0),
    };
    TileView::Drum {
        view,
        flip,
        state: drum_state(plugins, key),
        loaded: vec![false; 32],
        lit: vec![false; 32],
        mode,
        page,
        lane,
    }
}

/// The hosted drum sampler's saved state under `key` (a new kit when it is not one).
pub fn drum_state(plugins: &Plugins, key: usize) -> lemon_drum_sampler::state::State {
    plugins
        .state(key)
        .and_then(|blob| lemon_drum_sampler::params::fl_from_plugin_state(&blob))
        .and_then(|raw| lemon_drum_sampler::state::State::parse(&raw))
        .unwrap_or_default()
}

/// The cache's cell, as `State` holds it (painting has the state by reference).
pub type Cell = RefCell<Cache>;

#[cfg(test)]
mod tests {
    use super::*;

    /// A rendered editor's fixed Lemon pixels are recoloured through the applied palette, so panels and highlights
    /// follow a theme instead of staying Lemon grey / lemon.
    #[test]
    fn editor_pixels_follow_the_theme() {
        let text = os_theme::presets::PRESETS.iter().find(|(n, _)| *n == "Orchid").expect("Orchid preset").1;
        let fl = os_theme::FlTheme::parse(text);
        let p = os_theme::design::palette(os_theme::Design::Lemon, Some(&fl));
        let mut px = [0x00333333, 0x00ffe066];
        retint(&mut px, &p);
        assert_ne!(px[0], 0x00333333, "a grey panel takes the theme's surface hue");
        assert_ne!(px[1], 0x00ffe066, "the Lemon accent takes the theme's accent");
    }

    /// The docked EQ reads each band's level from its parameter's normalized value and the ±18 dB range its editor
    /// documents: with the host's 0..1 range a `plain` read made every band ~0 dB, so the graph was flat and the
    /// tokens sat on the zero line.
    #[test]
    fn the_docked_eq_bands_read_their_normalized_values() {
        use os_engine::effects::eq;
        let mut params: Vec<RackParam> = Vec::new();
        for k in 0..eq::BANDS {
            let n = k + 1;
            let level = if k == 0 { 0.8333 } else if k == 3 { 0.0 } else { 0.5 };
            for (name, value) in [
                (format!("Band {n} level"), level),
                (format!("Band {n} freq"), 0.5),
                (format!("Band {n} width"), 0.5),
                (format!("Band {n} type"), 0.0),
                (format!("Band {n} order"), 0.5),
            ] {
                params.push(RackParam {
                    id: params.len() as u32,
                    name,
                    group: String::new(),
                    value,
                    plain: value,
                    min: 0.0,
                    max: 1.0,
                    text: String::new(),
                    toggle: false,
                });
            }
        }
        params.push(RackParam {
            id: 900,
            name: "Main level".into(),
            group: String::new(),
            value: 0.5,
            plain: 0.5,
            min: 0.0,
            max: 1.0,
            text: String::new(),
            toggle: false,
        });
        let vals = Values::new(&Ui::Eq, &params);
        let s = eq_settings(&vals, &lemon_parametric_eq_2::params::default_state());
        assert!((s.bands[0].db - 12.0).abs() < 0.05, "band 1 at +12 dB, not {} dB", s.bands[0].db);
        assert!(s.bands[3].db <= -17.9, "band 4 at -18 dB, not {} dB", s.bands[3].db);
        assert!(s.main_db.abs() < 0.05, "main level at 0 dB");
        assert_eq!(s.bands[0].kind, 0, "a type reads its index, not a normalized value");
        assert_eq!(s.bands[0].order, 0, "an order reads -3..=3, not a shifted index");
    }

    /// The STEP / PADS choice is a remembered preference, not project data: switching it never touches a pattern.
    #[test]
    fn the_drum_interface_preference_is_remembered() {
        remember_mode(DrumMode::Pads);
        assert_eq!(preferred_mode(), DrumMode::Pads);
        remember_mode(DrumMode::Step);
        assert_eq!(preferred_mode(), DrumMode::Step);
    }

    /// Every registered UI is named the way its own editor names it, so a tile finds it by the plugin's name.
    #[test]
    fn every_ui_is_named_as_its_editor_is() {
        let names = [
            "Lemon Arp",
            "Lemon Chord",
            "Lemon Echo",
            "Lemon Transpose",
            "Lemon Quantize",
            "Lemon Note Length",
            "Lemon Key Filter",
            "Lemon Humanize",
            "Lemon Velocity Curve",
            "Lemon Note Delay",
            "Lemon Note Repeats",
            "Lemon Note Filter",
            "Lemon Random",
            "Lemon Strum",
            "Lemon Note Layer",
            "Lemon Note Selector",
            "Lemon Audio Layer",
            "Lemon Audio Selector",
            "Lemon Filter",
            "Lemon Balance",
            "Lemon Chorus",
            "Lemon Fast Distortion",
            "Lemon Flanger",
            "Lemon Phaser",
            "Lemon Soft Clipper",
            "Lemon Stereo Enhancer",
            "Lemon Stereo Shaper",
            "Lemon Transient Processor",
            "Lemon Vocoder",
            "Lemon Formula Controller",
            "Lemon Peak Controller",
            "Lemon Reverb",
            "Lemon Trio",
            "Lemon Parametric EQ",
            "Lemon Delay",
            "Lemon Limiter",
            "Lemon Sampler",
            "Lemon Slicer",
            "Press",
            "Pulp Filter",
            "Tesla",
        ];
        for name in names {
            let ui = ui(name).unwrap_or_else(|| panic!("{name} has no editor UI"));
            assert_eq!(ui.name(), name, "the registry key is the editor's own name");
            let (w, h) = ui.size();
            assert!(w >= 100.0 && h >= 60.0, "{name} is {w}x{h}");
            if matches!(ui, Ui::Press | Ui::Pulp | Ui::Tesla) {
                assert!(ui.settings().is_empty(), "{name} edits its state, not host parameters");
            } else {
                assert!(!ui.settings().is_empty(), "{name} names parameters");
            }
        }
        assert!(ui(LEMON_DRUM_SAMPLER).is_some(), "the drum machine shows its pad grid in the dock");
        assert!(ui("Lemon Sampler").is_some(), "the plain sampler draws its own sample page in the dock");
    }

    /// An editor renders its own pixels and reports its controls, each on a hosted parameter: the canvas is the
    /// editor's size at the asked scale and the hits sit inside it.
    #[test]
    fn an_editor_renders_its_controls() {
        let ui = ui("Lemon Arp").expect("Lemon Arp");
        // The hosted parameters the editor's seven settings name.
        let params: Vec<RackParam> = ["Pattern", "Octave", "Rate", "Gate", "Swing", "Hold", "Latch"]
            .iter()
            .enumerate()
            .map(|(i, name)| RackParam {
                id: i as u32 + 10,
                name: (*name).to_string(),
                group: String::new(),
                value: 0.5,
                text: "x".into(),
                plain: 1.0,
                min: 0.0,
                max: 3.0,
                toggle: i >= 5,
            })
            .collect();
        let (w, h) = ui.pixels(1.0);
        let mut c = Canvas::new(w, h, 1.0).expect("a canvas");
        let hits = ui.paint_into(&mut c, &params, &TileView::Plain, &Plugins::default(), 0);
        assert_eq!((w as f32, h as f32), ui.size());
        assert_eq!(hits.len(), 7, "a hit per setting");
        for (r, id, v) in &hits {
            assert!((10..17).contains(id), "each hit names a hosted parameter");
            assert!(r.x >= 0.0 && r.right() <= ui.size().0 + 0.5, "{r:?} inside the editor");
            if *id >= 15 {
                assert!(v.is_some(), "a switch flips on the press");
            } else {
                assert!(v.is_none(), "a dial drags");
            }
        }
        // Half the scale, half the pixels.
        let (sw, sh) = ui.pixels(0.5);
        assert_eq!((sw, sh), (w / 2, h / 2));
        let mut c = Canvas::new(sw, sh, 0.5).expect("a canvas");
        assert_eq!(ui.paint_into(&mut c, &params, &TileView::Plain, &Plugins::default(), 0).len(), hits.len());
    }

    /// A small effect (Lemon Filter) draws its own layout too, and its selector rows pick their value.
    #[test]
    fn a_small_effect_renders_its_own_layout() {
        let ui = ui("Lemon Filter").expect("Lemon Filter");
        let params: Vec<RackParam> = ui
            .settings()
            .iter()
            .enumerate()
            .map(|(i, (_, name))| RackParam {
                id: i as u32,
                name: (*name).to_string(),
                group: String::new(),
                value: 0.25,
                text: String::new(),
                plain: 512.0,
                min: 0.0,
                max: 2048.0,
                toggle: false,
            })
            .collect();
        let (w, h) = ui.pixels(1.0);
        let mut c = Canvas::new(w, h, 1.0).expect("a canvas");
        let hits = ui.paint_into(&mut c, &params, &TileView::Plain, &Plugins::default(), 0);
        assert_eq!((w as f32, h as f32), ui.size());
        assert!(!hits.is_empty(), "its knobs and rows");
    }

    /// The larger native editors (EQ, Delay, Limiter, Sampler, Slicer, Press, Pulp Filter, Tesla) draw their own
    /// layouts in a tile. The parameter-driven ones also report live controls; the state-driven ones stay read-only
    /// in the dock, so only their pixels are asserted.
    #[test]
    fn the_large_native_editors_render_in_the_dock() {
        for name in [
            "Lemon Parametric EQ",
            "Lemon Delay",
            "Lemon Limiter",
            "Lemon Sampler",
            "Lemon Slicer",
            "Press",
            "Pulp Filter",
            "Tesla",
        ] {
            let ui = ui(name).unwrap_or_else(|| panic!("{name} has no editor UI"));
            let params: Vec<RackParam> = ui
                .settings()
                .iter()
                .enumerate()
                .map(|(i, (_, pname))| RackParam {
                    id: i as u32,
                    name: pname.clone(),
                    group: String::new(),
                    value: 0.5,
                    text: String::new(),
                    plain: 1.0,
                    min: 0.0,
                    max: 2.0,
                    toggle: false,
                })
                .collect();
            let (w, h) = ui.pixels(1.0);
            let mut c = Canvas::new(w, h, 1.0).expect("a canvas");
            let hits = ui.paint_into(&mut c, &params, &TileView::Plain, &Plugins::default(), 0);
            assert_eq!((w as f32, h as f32), ui.size(), "{name} draws at its editor size");
            if !matches!(ui, Ui::Press | Ui::Pulp | Ui::Tesla) {
                assert!(!hits.is_empty(), "{name} reports its controls");
            }
            if matches!(ui, Ui::Eq) {
                assert!(
                    hits.iter().any(|(_, id, _)| *id == EDITOR_EQ_TOKEN),
                    "the EQ's graph tokens start a band drag"
                );
            }
            assert!(
                hits.iter().all(|(r, _, _)| r.x >= 0.0 && r.y >= 0.0 && r.right() <= ui.size().0 + 0.5 && r.bottom() <= ui.size().1 + 0.5),
                "{name}'s hits sit inside its editor"
            );
            let mut px = vec![0u32; (w * h) as usize];
            c.copy_to_xrgb(&mut px);
            px.sort_unstable();
            px.dedup();
            assert!(px.len() > 8, "{name} paints its editor, not a flat colour: {} shades", px.len());
        }
    }

    /// The values a tile draws are read through the editor's ids: a setting finds its hosted parameter by name,
    /// whatever the case and spacing.
    #[test]
    fn editor_ids_resolve_the_hosts_parameters() {
        let ui = ui("Lemon Reverb").expect("Lemon Reverb");
        let params: Vec<RackParam> = ui
            .settings()
            .iter()
            .enumerate()
            .map(|(i, (_, name))| RackParam {
                id: i as u32 + 100,
                name: name.to_uppercase(),
                group: String::new(),
                value: 0.5,
                text: String::new(),
                plain: 1.0,
                min: 0.0,
                max: 2.0,
                toggle: false,
            })
            .collect();
        let vals = Values::new(&ui, &params);
        for (id, _) in ui.settings() {
            let p = vals.param(&id).unwrap_or_else(|| panic!("{id} resolved"));
            assert!((100..100 + params.len() as u32).contains(&p.id));
        }
        assert_eq!(<Values as os_plugkit::simple::Values>::norm(&vals, "hcut"), 0.5);
        assert_eq!(<Values as os_plugkit::simple::Values>::plain(&vals, "hcut"), 1.0);
    }

    /// The Trio's full editor draws in the dock: its oscillators, tabs and knobs, with drag/drop hits on real
    /// parameters and editor-only hits on its tab strip (the user keeps the rest in the pop-out window).
    #[test]
    fn the_trio_editor_renders_in_the_dock() {
        let ui = ui("Lemon Trio").expect("Lemon Trio");
        let params: Vec<RackParam> = ui
            .settings()
            .iter()
            .enumerate()
            .map(|(i, (_, name))| RackParam {
                id: i as u32,
                name: name.to_string(),
                group: String::new(),
                value: 0.5,
                text: String::new(),
                plain: 1.0,
                min: 0.0,
                max: 1.0,
                toggle: false,
            })
            .collect();
        let (w, h) = ui.pixels(1.0);
        let mut c = Canvas::new(w, h, 1.0).expect("a canvas");
        let hits = ui.paint_into(&mut c, &params, &TileView::Trio(Default::default()), &Plugins::default(), 0);
        assert!(!hits.is_empty(), "the Trio paints its oscillators");
        // The editor draws its own page strip, so the envelopes page is reachable from the dock (as in its
        // window and the Sampler's docked tile).
        let pages: Vec<u8> = hits
            .iter()
            .filter(|(_, id, _)| *id == EDITOR_TAB)
            .filter_map(|(_, _, v)| v.map(|v| v as u8))
            .collect();
        assert!((0..3).all(|n| pages.contains(&n)), "all three pages are hit-testable: {pages:?}");
        // Its parameter controls are drawn and hit-testable too.
        let knobs = hits
            .iter()
            .filter(|(_, id, _)| *id != EDITOR_SUBTAB && params.iter().any(|p| p.id == *id))
            .count();
        assert!(knobs > 0, "the Trio's knobs are hit-testable ({} hits)", hits.len());
    }

    /// The editor paints real pixels into a body-sized canvas (cover mode): the glyphs and knobs land on the
    /// canvas, not just their hit regions — a blank tile with live hits would otherwise pass the suites above.
    #[test]
    fn the_trio_editor_paints_pixels_into_a_body_sized_canvas() {
        let ui = ui(LEMON_TRIO).expect("Lemon Trio");
        let params: Vec<RackParam> = ui
            .settings()
            .iter()
            .enumerate()
            .map(|(i, (_, name))| RackParam {
                id: i as u32,
                name: (*name).to_string(),
                group: String::new(),
                value: 0.25,
                text: String::new(),
                plain: 512.0,
                min: 0.0,
                max: 2048.0,
                toggle: false,
            })
            .collect();
        // The production shape: a 280x300 logical tile body at app scale 2 is a 560x600 device-px canvas,
        // and the cover scale is the smaller stretch (here 1.0: the width matches the editor exactly).
        let (ew, eh) = ui.size();
        let (cw, ch) = (560u32, 600u32);
        let scale = ((cw as f32 / ew).min(ch as f32 / eh)).max(0.02);
        let mut c = Canvas::new(cw, ch, scale).expect("a canvas");
        let hits = ui.paint_into(&mut c, &params, &TileView::Trio(Default::default()), &Plugins::default(), 0);
        assert!(!hits.is_empty(), "its knobs and tabs");
        let mut px = vec![0u32; (cw * ch) as usize];
        c.copy_to_xrgb(&mut px);
        let unique = {
            let mut v = px.clone();
            v.sort_unstable();
            v.dedup();
            v.len()
        };
        assert!(unique > 8, "the editor paints more than a flat colour: {unique} shades");
        // The realistic small tile: 236x180 logical body at app scale 2 -> 472x360 device px, and the
        // cover scale is fractional (0.82). A flat canvas here is the blank-tile bug.
        let (cw, ch) = (472u32, 360u32);
        let scale = ((cw as f32 / ew).min(ch as f32 / eh)).max(0.02);
        let mut c = Canvas::new(cw, ch, scale).expect("a canvas");
        ui.paint_into(&mut c, &params, &TileView::Trio(Default::default()), &Plugins::default(), 0);
        let mut px = vec![0u32; (cw * ch) as usize];
        c.copy_to_xrgb(&mut px);
        let unique = {
            let mut v = px.clone();
            v.sort_unstable();
            v.dedup();
            v.len()
        };
        assert!(unique > 8, "the small fractional-scale tile paints too: {unique} shades");
    }

    /// The docked Trio renders end to end through `image` with a real hosted instance (a real plugin, its
    /// real parameters, the tile's cache): a blank tile with passing paint probes would hide in the suites
    /// above, so this one follows the app's own path. A 236x180 logical body at app scale 2.
    #[test]
    fn the_trio_renders_end_to_end_through_image() {
        let mut app = crate::app::App::new(None);
        // The bundled Lemon instruments (the app fills `available` from the installed plugin DB at
        // startup; a test builds the same list straight from the builtin table).
        app.plugins.available = crate::plugins::builtin();
        let idx = app
            .plugins
            .available
            .iter()
            .position(|a| a.desc.name == LEMON_TRIO)
            .expect("Lemon Trio bundled");
        app.plugins.create_with(0, idx, None).expect("the Trio hosted");
        let params = app.plugins.rack_controls(0, &[]);
        assert!(!params.is_empty(), "the Trio names its parameters");
        let s = &app.state;
        let img = image(s, &app.plugins, 0, LEMON_TRIO, &params, 0.84, 472, 360).expect("the Trio's image");
        assert!(!img.hits.is_empty(), "its controls are hit-testable");
        assert_eq!((img.w, img.h), (472, 360), "the image is the body");
        assert!(
            img.hits.iter().any(|(_, id, _)| *id == EDITOR_TAB),
            "the page strip (envelopes included) is in the docked image"
        );
        let unique = {
            let mut v = img.px.clone();
            v.sort_unstable();
            v.dedup();
            v.len()
        };
        assert!(unique > 8, "the tile paints the editor, not a flat colour: {unique} shades");
    }

    /// The drum machine shows just its 16 pads in the dock; the pad grid is hit-testable and the card
    /// editing stays in the pop-out window.
    #[test]
    fn the_drum_machine_pads_render_in_the_dock() {
        let ui = ui(LEMON_DRUM_SAMPLER).expect("Lemon Drum Sampler");
        let (w, h) = ui.pixels(1.0);
        let mut c = Canvas::new(w, h, 1.0).expect("a canvas");
        let view = TileView::Drum {
            view: lemon_drum_sampler::view::ViewState::default(),
            flip: false,
            state: lemon_drum_sampler::state::State::default(),
            loaded: vec![false; 32],
            lit: vec![false; 32],
            mode: DrumMode::Pads,
            page: 0,
            lane: 0,
        };
        let hits = ui.paint_into(&mut c, &[], &view, &Plugins::default(), 0);
        assert!(!hits.is_empty(), "the pads paint");
        let pads = hits.iter().filter(|(_, id, _)| *id == EDITOR_PAD).count();
        assert_eq!(pads, 16, "one hit per pad");
    }

    /// The drum machine's other half — its pad and layer cards — renders in the dock flipped to, with
    /// its Load button hit-testable (the picker itself stays in the pop-out window).
    #[test]
    fn the_drum_machine_cards_flip_into_the_dock() {
        let ui = ui(LEMON_DRUM_SAMPLER).expect("Lemon Drum Sampler");
        let view = TileView::Drum {
            view: lemon_drum_sampler::view::ViewState::default(),
            flip: true,
            state: lemon_drum_sampler::state::State::default(),
            loaded: vec![false; 32],
            lit: vec![false; 32],
            mode: DrumMode::Pads,
            page: 0,
            lane: 0,
        };
        let (w, h) = ui.pixels(1.0);
        let mut c = Canvas::new(w, h, 1.0).expect("a canvas");
        let hits = ui.paint_into(&mut c, &[], &view, &Plugins::default(), 0);
        assert!(hits.iter().any(|(_, id, _)| *id == EDITOR_OPEN), "the Load button is hit-testable");
    }

    /// The drum machine's tile crops the whole it paints down to the half it shows: the pads' half keeps
    /// the pad grid and drops the cards (and vice versa), and the kept hits move with the crop.
    #[test]
    fn the_drum_half_keeps_only_its_own_half() {
        use lemon_drum_sampler::view as dv;
        let pad = Rect::new(10.0, 10.0, 20.0, 20.0);
        let bank = Rect::new(228.0, 10.0, 32.0, 20.0);
        let load = Rect::new(540.0, 221.0, 64.0, 20.0);
        let hit = |r: Rect, id: u32| (r, id, None);
        let whole = (0..dv::W).map(|x| (x as u32 % 2) as u32).collect::<Vec<_>>();
        // The pads' half: the pads and banks stay, the cards' Load drops.
        let (_, w, _, hits) = drum_half(&whole, dv::W, 1.0, false, vec![hit(pad, EDITOR_PAD), hit(bank, EDITOR_BANK), hit(load, EDITOR_OPEN)]);
        assert_eq!(w as usize, (dv::GRID.right() + dv::GRID.x) as usize);
        assert_eq!(hits.len(), 2, "the Load belongs to the other half");
        assert!(hits.iter().all(|(r, _, _)| r.x == pad.x || r.x == bank.x), "the kept hits do not move: nothing was cropped off the left");
        // The cards' half: the Load stays (moved left by the crop), the pads drop.
        let (_, w, _, hits) = drum_half(&whole, dv::W, 1.0, true, vec![hit(pad, EDITOR_PAD), hit(load, EDITOR_OPEN)]);
        assert_eq!(w as usize, (dv::W as f32 - dv::GRID.right()) as usize);
        assert_eq!(hits.len(), 1, "the pads belong to the other half");
        let (r, id, _) = hits[0];
        assert_eq!(id, EDITOR_OPEN);
        assert_eq!(r.x, load.x - dv::GRID.right(), "the Load moves with the crop");
    }
}
