// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Transpose: Lemon Studio's note transposer as a VST3 / CLAP MIDI plugin (nih-plug).
//!
//! spec: plugins/lemon-transpose.md

#![allow(semicolon_in_expressions_from_non_local_macros)]

pub mod dsp;
pub mod editor;
pub mod params;

use crate::dsp::Transpose;
use crate::params::{TransposeParams, Settings};

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use nih_plug::prelude::*;

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

pub struct LemonTranspose {
    params: Arc<TransposeParams>,
    shared: Arc<Shared>,
    dsp: Transpose,
}

impl Default for LemonTranspose {
    fn default() -> Self {
        LemonTranspose { params: Arc::default(), shared: Arc::default(), dsp: Transpose::default() }
    }
}

impl Plugin for LemonTranspose {
    const NAME: &'static str = "Lemon Transpose";
    const VENDOR: &'static str = "Lemon Studio";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // MIDI plugin: no audio I/O
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[];
    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::Basic;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        Some(Box::new(editor::TransposeEditor::new(self.params.clone(), self.shared.clone())))
    }

    fn initialize(&mut self, _layout: &AudioIOLayout, config: &BufferConfig, _ctx: &mut impl InitContext<Self>) -> bool {
        self.shared.rate.store(config.sample_rate as u32, Ordering::Relaxed);
        self.dsp.reset();
        true
    }

    fn reset(&mut self) {
        self.dsp.reset();
    }

    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, ctx: &mut impl ProcessContext<Self>) -> ProcessStatus {
        let s: Settings = self.params.settings();
        // Every note event: notes shift, everything else passes through. Note offs follow their note on.
        while let Some(ev) = ctx.next_event() {
            if let Some(out) = self.dsp.process(&s, ev) {
                ctx.send_event(out);
            }
        }
        let _ = buffer;
        ProcessStatus::Normal
    }
}

impl ClapPlugin for LemonTranspose {
    const CLAP_ID: &'static str = "org.openstudio.lemon-transpose";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Lemon Studio's note transposer");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::NoteEffect, ClapFeature::Utility];
}

impl Vst3Plugin for LemonTranspose {
    const VST3_CLASS_ID: [u8; 16] = *b"LemonStudioXpose";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx];
}

// Built as a plugin module: the C entry points hosts look for. Linked into Lemon Studio (`linked` feature): only the
// symbol-free table, so several Lemon plugins fit in one program (spec: plugins/plugin-formats.md).
#[cfg(not(feature = "linked"))]
nih_export_clap!(LemonTranspose);
#[cfg(not(feature = "linked"))]
nih_export_vst3!(LemonTranspose);
os_plugkit::linked_clap_entry!(LemonTranspose);
