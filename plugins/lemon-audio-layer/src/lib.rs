// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Audio Layer: Lemon Studio's stereo parallel audio layer as a VST3 / CLAP plugin (nih-plug).
//!
//! The output is the master dry signal plus the master wet sum of 4 parallel slots
//! (Bypass / Filter / Delay / Chorus / Drive). The sound is `dsp::Layer`.

#![allow(semicolon_in_expressions_from_non_local_macros)]

pub mod dsp;
pub mod editor;
pub mod params;

use dsp::Layer;
use params::Settings;

use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use nih_plug::prelude::*;
use params::AudioLayerParams;

/// What the audio thread and the editor share.
pub struct Shared {
    pub rate: AtomicU32,
    /// Set while an editor is open.
    pub editor_open: AtomicBool,
}

impl Default for Shared {
    fn default() -> Self {
        Shared { rate: AtomicU32::new(44100), editor_open: AtomicBool::new(false) }
    }
}

pub struct LemonAudioLayer {
    params: Arc<AudioLayerParams>,
    shared: Arc<Shared>,
    /// Made in `initialize` (its delay line is sized for the sample rate).
    dsp: Option<Layer>,
    raw: Vec<u8>,
    raw_seen: u64,
}

impl Default for LemonAudioLayer {
    fn default() -> Self {
        let params = Arc::new(AudioLayerParams::default());
        let shared = Arc::new(Shared::default());
        LemonAudioLayer { params, shared, dsp: None, raw: params::default_state(), raw_seen: u64::MAX }
    }
}

impl LemonAudioLayer {
    /// The saved state's copy (never blocks: keeps the last one while the editor writes).
    fn read_raw(&mut self) {
        let v = self.params.raw_version.load(Ordering::Acquire);
        if v == self.raw_seen {
            return;
        }
        if let Ok(b) = self.params.raw.try_read() {
            self.raw.clone_from(&b);
            self.raw_seen = v;
        }
    }

    fn settings(&mut self) -> Settings {
        self.read_raw();
        self.params.settings(&self.raw)
    }
}

impl Plugin for LemonAudioLayer {
    const NAME: &'static str = "Lemon Audio Layer";
    const VENDOR: &'static str = "Lemon Studio";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] =
        &[AudioIOLayout { main_input_channels: NonZeroU32::new(2), main_output_channels: NonZeroU32::new(2), ..AudioIOLayout::const_default() }];
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        Some(Box::new(os_plugkit::simple::Editor::new(&editor::INFO, self.params.clone(), self.params.live.clone())))
    }

    fn initialize(&mut self, _layout: &AudioIOLayout, config: &BufferConfig, _ctx: &mut impl InitContext<Self>) -> bool {
        self.shared.rate.store(config.sample_rate as u32, Ordering::Relaxed);
        self.raw_seen = u64::MAX;
        let s = self.settings();
        let rate = config.sample_rate.round() as u32;
        match &mut self.dsp {
            Some(d) => {
                d.set_sample_rate(rate);
                d.set(&s);
            }
            None => self.dsp = Some(Layer::with_settings(rate, &s)),
        }
        true
    }

    fn reset(&mut self) {
        if let Some(d) = &mut self.dsp {
            d.reset();
        }
    }

    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _ctx: &mut impl ProcessContext<Self>) -> ProcessStatus {
        let s = self.settings();
        let Some(dsp) = &mut self.dsp else { return ProcessStatus::Normal };
        dsp.set(&s);
        let out = buffer.as_slice();
        if out.len() < 2 {
            return ProcessStatus::Normal;
        }
        let (left, right) = out.split_at_mut(1);
        dsp.process(left[0], right[0]);
        // The delay feedback rings on after the input stops.
        ProcessStatus::KeepAlive
    }
}

impl ClapPlugin for LemonAudioLayer {
    const CLAP_ID: &'static str = "org.openstudio.lemon-audio-layer";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Lemon Studio's stereo parallel audio layer");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect, ClapFeature::Stereo];
}

impl Vst3Plugin for LemonAudioLayer {
    const VST3_CLASS_ID: [u8; 16] = *b"LemonAudioLayer ";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx];
}

// Built as a plugin module: the C entry points hosts look for. Linked into Lemon Studio (`linked` feature): only the
// symbol-free table, so several Lemon plugins fit in one program.
#[cfg(not(feature = "linked"))]
nih_export_clap!(LemonAudioLayer);
#[cfg(not(feature = "linked"))]
nih_export_vst3!(LemonAudioLayer);
os_plugkit::linked_clap_entry!(LemonAudioLayer);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_are_passthrough() {
        let mut p = LemonAudioLayer::default();
        assert_eq!(p.settings(), Settings::default());
        assert_eq!(p.settings().dry, 6000);
        assert_eq!(p.settings().wet, 6000);
        assert!(p.settings().slots.iter().all(|s| !s.active));
    }
}
