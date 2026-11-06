// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Chord: Lemon Studio's chord/scale mapper as a VST3 / CLAP MIDI plugin (nih-plug).
//!
//! spec: plugins/lemon-chord.md

#![allow(semicolon_in_expressions_from_non_local_macros)]

pub mod dsp;
pub mod editor;
pub mod params;

use crate::dsp::Chord;
use crate::params::{ChordType, Inversion, Root, Scale, Settings, Strum, Voicing};

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use nih_plug::prelude::*;
use params::ChordParams;

/// What the MIDI thread and the editor share.
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

pub struct LemonChord {
    params: Arc<ChordParams>,
    shared: Arc<Shared>,
    /// Made in `initialize` (its state is sized for the sample rate).
    dsp: Option<Chord>,
    raw: Vec<u8>,
    raw_seen: u64,
}

impl Default for LemonChord {
    fn default() -> Self {
        let params = Arc::new(ChordParams::default());
        let shared = Arc::new(Shared::default());
        LemonChord { params, shared, dsp: None, raw: params::default_state(), raw_seen: u64::MAX }
    }
}

impl LemonChord {
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

impl Plugin for LemonChord {
    const NAME: &'static str = "Lemon Chord";
    const VENDOR: &'static str = "Lemon Studio";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // MIDI plugin: no audio I/O
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[];
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
            None => self.dsp = Some(Chord::with_settings(rate, &s)),
        }
        true
    }

    fn reset(&mut self) {
        if let Some(d) = &mut self.dsp {
            d.reset();
        }
    }

    fn process(&mut self, _buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _ctx: &mut impl ProcessContext<Self>) -> ProcessStatus {
        let s = self.settings();
        let Some(dsp) = &mut self.dsp else { return ProcessStatus::Normal };
        dsp.set(&s);
        ProcessStatus::Normal
    }

}

impl ClapPlugin for LemonChord {
    const CLAP_ID: &'static str = "org.openstudio.lemon-chord";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Lemon Studio's chord/scale mapper");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect, ClapFeature::AudioEffect];
}

impl Vst3Plugin for LemonChord {
    const VST3_CLASS_ID: [u8; 16] = *b"LemonStudioChord";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx];
}

// Built as a plugin module: the C entry points hosts look for. Linked into Lemon Studio (`linked` feature): only the
// symbol-free table, so several Lemon plugins fit in one program (spec: plugins/plugin-formats.md).
#[cfg(not(feature = "linked"))]
nih_export_clap!(LemonChord);
#[cfg(not(feature = "linked"))]
nih_export_vst3!(LemonChord);
os_plugkit::linked_clap_entry!(LemonChord);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_mapping() {
        let mut c = LemonChord::default();
        let expected = Settings {
            root: Root::C,
            scale: Scale::Major,
            chord_type: ChordType::Triad,
            inversion: Inversion::Root,
            voicing: Voicing::Close,
            strum: Strum::Off,
            strum_speed: 5000,
            hold: false,
        };
        assert_eq!(c.settings(), expected);
    }
}
