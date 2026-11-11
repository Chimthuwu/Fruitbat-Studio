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
//! - Lemon Reverb through `lemon_reverb::view::paint` with its `Model` and room visual.
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

use os_ui::{Canvas, Rect};

use crate::app::State;
use crate::plugins::Plugins;
use crate::device_rack::RackParam;
use crate::osc3::LEMON_TRIO;
use crate::plugins::LEMON_SAMPLER;

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
    /// Lemon Sampler: the 16-pad grid in the dock (its card editing stays in the pop-out window).
    Drum,
}

/// Reserved `id` values in an editor hit that name an editor-only action rather than a hosted parameter: a tile
/// switches its page, chooses a drum pad, etc. No hosted parameter may legitimately carry these ids.
pub const EDITOR_TAB: u32 = u32::MAX;
pub const EDITOR_SUBTAB: u32 = u32::MAX - 1;
pub const EDITOR_PAD: u32 = u32::MAX - 2;
pub const EDITOR_BANK: u32 = u32::MAX - 3;

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
        LEMON_SAMPLER => Ui::Drum,
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
            Ui::Drum => LEMON_SAMPLER,
        }
    }

    /// The editor's natural size in logical px (its window's).
    pub fn size(&self) -> (f32, f32) {
        match self {
            Ui::Simple(info) => (info.w as f32, info.h as f32),
            Ui::Small(spec) => (spec.size.0 as f32, spec.size.1 as f32),
            Ui::Reverb => (lemon_reverb::view::W as f32, lemon_reverb::view::H as f32),
            Ui::Trio => (lemon_3x_osc::view::EDITOR_W as f32, lemon_3x_osc::view::EDITOR_H as f32),
            // The dock shows only the pad grid (the cards stay in the pop-out), cropped at the grid's right edge.
            Ui::Drum => (lemon_drum_sampler::view::GRID.right() + lemon_drum_sampler::view::GRID.x, lemon_drum_sampler::view::H as f32),
        }
    }

    /// The editor's parameters as its own painting names them, with the name a host shows: the map from a view id
    /// ("hcut", "pattern") to the hosted parameter's name ("High cut", "Pattern").
    pub fn settings(&self) -> Vec<(&'static str, &'static str)> {
        match self {
            Ui::Simple(info) => info.settings.iter().map(|s| (s.id, s.name)).collect(),
            Ui::Small(spec) => spec.settings.iter().map(|s| (s.id, s.name)).collect(),
            Ui::Reverb => {
                use lemon_reverb::params::{DEFS, MID_SIDE_ID, MID_SIDE_NAME, TEMPO_ID, TEMPO_NAME};
                let mut v: Vec<(&'static str, &'static str)> = DEFS.iter().map(|d| (d.id, d.name)).collect();
                v.push((TEMPO_ID, TEMPO_NAME));
                v.push((MID_SIDE_ID, MID_SIDE_NAME));
                v
            }
            Ui::Trio => lemon_3x_osc::params::ui_settings(),
            // The drum sampler exposes no host parameters: its pads are drawn from its saved state, not the
            // parameter list, so the tile has no (param-based) settings to name.
            Ui::Drum => Vec::new(),
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
    pub fn paint_into(&self, c: &mut Canvas, params: &[RackParam], view: &TileView) -> Vec<(Rect, u32, Option<f64>)> {
        let vals = Values::new(self, params);
        let t = os_plugkit::palette();
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
                let hits = lemon_3x_osc::view::paint(c, &t, &vals, vst, "", true);
                let mut out = Vec::new();
                for hit in hits {
                    match hit.t {
                        lemon_3x_osc::view::Target::Tab(n) => out.push((hit.r, EDITOR_TAB, Some(n as f64))),
                        lemon_3x_osc::view::Target::SubTab(n) => out.push((hit.r, EDITOR_SUBTAB, Some(n as f64))),
                        lemon_3x_osc::view::Target::Knob(s)
                        | lemon_3x_osc::view::Target::Toggle(s)
                        | lemon_3x_osc::view::Target::Set(s, _)
                        | lemon_3x_osc::view::Target::List(s) => {
                            if let Some(code) = ids.get(s.as_str()).copied() {
                                let v = params.iter().find(|p| p.id == code).map(|p| p.value).unwrap_or(0.0);
                                out.push((hit.r, code, Some(v)));
                            }
                        }
                        lemon_3x_osc::view::Target::Flag(_) => {
                            if let Some(code) = ids.get("flags").copied() {
                                let v = params.iter().find(|p| p.id == code).map(|p| p.value).unwrap_or(0.0);
                                out.push((hit.r, code, Some(v)));
                            }
                        }
                        _ => {}
                    }
                }
                out
            }
            Ui::Drum => {
                let TileView::Drum { view: vst, state, loaded, lit } = view else { return Vec::new() };
                let look = lemon_drum_sampler::view::Look { state, loaded: &loaded[..], lit: &lit[..] };
                let hits = lemon_drum_sampler::view::paint(c, &t, vst, &look, "");
                let mut out = Vec::new();
                for hit in hits {
                    match hit.t {
                        lemon_drum_sampler::view::Target::Pad(n) => out.push((hit.r, EDITOR_PAD, Some(n as f64))),
                        lemon_drum_sampler::view::Target::Bank(n) => out.push((hit.r, EDITOR_BANK, Some(n as f64))),
                        _ => {}
                    }
                }
                out
            }
        }
    }
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
        Entry { name: name.to_string(), sig: 0, factor: 0.0, canvas: None, buf: Vec::new(), image, view: TileView::Plain }
    }

    /// Make sure `view` matches the UI about to be painted: if the UI changed (or was never set), reset it to that
    /// UI's default interactive state so we don't hand the wrong editor the wrong state.
    fn ensure_view(&mut self, ui: Ui) {
        let matches = matches!(
            (&self.view, ui),
            (TileView::Trio(_), Ui::Trio)
                | (TileView::Drum { .. }, Ui::Drum)
                | (TileView::Plain, _)
        );
        if !matches {
            self.view = match ui {
                Ui::Trio => TileView::Trio(lemon_3x_osc::view::ViewState::default()),
                Ui::Drum => TileView::Drum {
                    view: lemon_drum_sampler::view::ViewState::default(),
                    state: lemon_drum_sampler::state::State::default(),
                    loaded: vec![false; 32],
                    lit: vec![false; 32],
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
}

/// The image of the editor of the device hosted under `key`: `name` is the plugin's name, `factor` the device px per
/// editor px. The cached image is handed back when neither the values nor the scale moved.
pub fn image(s: &State, plugins: &Plugins, key: usize, name: &str, params: &[RackParam], factor: f32) -> Option<Rc<Image>> {
    let ui = ui(name)?;
    let sig = signature(name, params);
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
    if e.name == name && e.sig == sig && (e.factor - factor).abs() < 0.005 && !matches!(ui, Ui::Drum) {
        return Some(e.image.clone());
    }
    let (dw, dh) = ui.pixels(factor);
    // Painting in the editor's own logical px with the canvas's device pixels scaled: the glyphs and strokes are
    // rasterised at the tile's size, so a tile smaller than the editor stays sharp.
    let scale = dw as f32 / ui.size().0.max(1.0);
    match &mut e.canvas {
        Some(c) => c.resize(dw, dh, scale),
        None => e.canvas = Canvas::new(dw, dh, scale).ok(),
    }
    let canvas = e.canvas.as_mut()?;
    let hits = ui.paint_into(canvas, params, &view);
    e.buf.resize((dw * dh) as usize, 0);
    canvas.copy_to_xrgb(&mut e.buf);
    e.image = Rc::new(Image { w: dw as usize, h: dh as usize, px: e.buf.clone(), hits });
    e.name = name.to_string();
    e.sig = sig;
    e.factor = factor;
    if matches!(ui, Ui::Drum) {
        e.view = view;
    }
    Some(e.image.clone())
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
            let k = key(name);
            let found = params
                .iter()
                .find(|p| key(&p.name) == k)
                // A host may decorate a name ("Predelay (ms)"): fall back to the closest start.
                .or_else(|| params.iter().find(|p| key(&p.name).starts_with(&k) && !p.name.is_empty()))
                .or_else(|| params.iter().find(|p| key(&p.name) == key(id)));
            if let Some(p) = found {
                ids.insert(id.to_string(), p.id);
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

/// Per-tile interactive state for the native editors that keep some of it (the Trio's selected page, the drum's
/// chosen bank/pad). The rack used to paint every tile from `ViewState::default()`, which is why an instrument
/// never kept its page; this lives in the cache entry so the choice survives repaints.
#[derive(Clone)]
pub enum TileView {
    /// The simple / small / reverb editors have no tile-local state.
    Plain,
    /// Lemon Trio: the selected page and sub-page in its editor.
    Trio(lemon_3x_osc::view::ViewState),
    /// Lemon Sampler: the chosen bank/pad, plus a snapshot of its saved state so the pads can render in the dock.
    Drum {
        view: lemon_drum_sampler::view::ViewState,
        state: lemon_drum_sampler::state::State,
        loaded: Vec<bool>,
        lit: Vec<bool>,
    },
}

impl Default for TileView {
    fn default() -> Self {
        TileView::Plain
    }
}

/// A drum tile's per-paint view: a snapshot of the hosted sampler's state, carrying over the user's chosen
/// bank/pad so a repaint doesn't reset the selection.
fn drum_view(plugins: &Plugins, key: usize, prev: &TileView) -> TileView {
    let view = match prev {
        TileView::Drum { view, .. } => view.clone(),
        _ => lemon_drum_sampler::view::ViewState::default(),
    };
    let state = plugins.state(key)
        .and_then(|blob| lemon_drum_sampler::params::fl_from_plugin_state(&blob))
        .and_then(|raw| lemon_drum_sampler::state::State::parse(&raw))
        .unwrap_or_default();
    TileView::Drum {
        view,
        state,
        loaded: vec![false; 32],
        lit: vec![false; 32],
    }
}

/// The cache's cell, as `State` holds it (painting has the state by reference).
pub type Cell = RefCell<Cache>;

#[cfg(test)]
mod tests {
    use super::*;

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
        ];
        for name in names {
            let ui = ui(name).unwrap_or_else(|| panic!("{name} has no editor UI"));
            assert_eq!(ui.name(), name, "the registry key is the editor's own name");
            let (w, h) = ui.size();
            assert!(w >= 100.0 && h >= 60.0, "{name} is {w}x{h}");
            assert!(!ui.settings().is_empty(), "{name} names parameters");
        }
        assert!(ui(LEMON_SAMPLER).is_some(), "the drum sampler shows its pad grid in the dock");
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
        let hits = ui.paint_into(&mut c, &params, &TileView::Plain);
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
        assert_eq!(ui.paint_into(&mut c, &params, &TileView::Plain).len(), hits.len());
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
        let hits = ui.paint_into(&mut c, &params, &TileView::Plain);
        assert_eq!((w as f32, h as f32), ui.size());
        assert!(!hits.is_empty(), "its knobs and rows");
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
            let p = vals.param(id).unwrap_or_else(|| panic!("{id} resolved"));
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
        let hits = ui.paint_into(&mut c, &params, &TileView::Trio(Default::default()));
        assert!(!hits.is_empty(), "the Trio paints its oscillators and tabs");
        let tabs = hits.iter().filter(|(_, id, _)| *id == EDITOR_TAB).count();
        assert!(tabs > 0, "the Trio's tab strip is hit-testable");
        let subtabs = hits.iter().filter(|(_, id, _)| *id == EDITOR_SUBTAB).count();
        eprintln!("trio editor: {} hits ({} tabs, {} sub-tabs)", hits.len(), tabs, subtabs);
        // The Trio's editor is allowed to emit editor-only hits (tabs/sub-tabs, sentinels) and real hosted
        // parameter hits. Surface anything else rather than guess.
        let unknown: Vec<u32> = hits
            .iter()
            .filter(|(_, id, _)| *id != EDITOR_TAB && *id != EDITOR_SUBTAB && !params.iter().any(|p| p.id == *id))
            .map(|(_, id, _)| *id)
            .collect();
        if !unknown.is_empty() {
            eprintln!("note: Trio editor emitted {} non-param/non-tab hit ids: {:?}", unknown.len(), unknown);
        }
    }

    /// The drum machine shows just its 16 pads in the dock; the pad grid is hit-testable and the card editing
    /// stays in the pop-out window.
    #[test]
    fn the_drum_machine_pads_render_in_the_dock() {
        let ui = ui(LEMON_SAMPLER).expect("Lemon Sampler");
        let (w, h) = ui.pixels(1.0);
        let mut c = Canvas::new(w, h, 1.0).expect("a canvas");
        let view = TileView::Drum {
            view: lemon_drum_sampler::view::ViewState::default(),
            state: lemon_drum_sampler::state::State::default(),
            loaded: vec![false; 32],
            lit: vec![false; 32],
        };
        let hits = ui.paint_into(&mut c, &[], &view);
        assert!(!hits.is_empty(), "the pads paint");
        let pads = hits.iter().filter(|(_, id, _)| *id == EDITOR_PAD).count();
        assert_eq!(pads, 16, "one hit per pad");
    }
}
