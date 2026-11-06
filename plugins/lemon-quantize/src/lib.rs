// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Quantize: Lemon Studio's note quantizer as a VST3 / CLAP MIDI plugin (nih-plug).
//!
//! spec: plugins/lemon-quantize.md

#![allow(semicolon_in_expressions_from_non_local_macros)]

pub mod dsp;
pub mod editor;
pub mod params;

use crate::dsp::Quantize;
use crate::params::{QuantizeParams, Settings};

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

pub struct LemonQuantize {
    params: Arc<QuantizeParams>,
    shared: Arc<Shared>,
    dsp: Quantize,
    inbuf: Vec<NoteEvent<()>>,
    outbuf: Vec<NoteEvent<()>>,
}

impl Default for LemonQuantize {
    fn default() -> Self {
        LemonQuantize { params: Arc::default(), shared: Arc::default(), dsp: Quantize::default(), inbuf: Vec::with_capacity(256), outbuf: Vec::with_capacity(256) }
    }
}

impl Plugin for LemonQuantize {
    const NAME: &'static str = "Lemon Quantize";
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
        Some(Box::new(editor::QuantizeEditor::new(self.params.clone(), self.shared.clone())))
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
        // Held-over events need the transport; without a position the notes pass straight through.
        let len = buffer.samples() as u32;
        self.inbuf.clear();
        while let Some(ev) = ctx.next_event() {
            self.inbuf.push(ev);
        }
        let t = ctx.transport();
        let (Some(pos), tempo) = (t.pos_beats(), t.tempo.unwrap_or(120.0)) else {
            for ev in self.inbuf.drain(..) {
                ctx.send_event(ev);
            }
            return ProcessStatus::Normal;
        };
        let sr = ctx.transport().sample_rate as f64;
        self.outbuf.clear();
        self.dsp.block(&s, pos, tempo, sr, len, &self.inbuf, &mut self.outbuf);
        for ev in self.outbuf.drain(..) {
            ctx.send_event(ev);
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for LemonQuantize {
    const CLAP_ID: &'static str = "org.openstudio.lemon-quantize";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Lemon Studio's note quantizer");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::NoteEffect, ClapFeature::Utility];
}

impl Vst3Plugin for LemonQuantize {
    const VST3_CLASS_ID: [u8; 16] = *b"LemonStudioQuant";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx];
}

// Built as a plugin module: the C entry points hosts look for. Linked into Lemon Studio (`linked` feature): only the
// symbol-free table, so several Lemon plugins fit in one program (spec: plugins/plugin-formats.md).
#[cfg(not(feature = "linked"))]
nih_export_clap!(LemonQuantize);
#[cfg(not(feature = "linked"))]
nih_export_vst3!(LemonQuantize);
os_plugkit::linked_clap_entry!(LemonQuantize);
