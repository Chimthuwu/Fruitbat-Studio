// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Audio Selector: stereo audio selector (nih-plug).
//!
//! Only the selected slot processes audio: 0=Bypass 1=Filter 2=Delay 3=Chorus 4=Drive.
//! Selection modes: Manual / Note. Slot type is fixed per slot index.

#![allow(semicolon_in_expressions_from_non_local_macros)]

pub mod dsp;
pub mod editor;
pub mod params;

use crate::dsp::Selector;
use crate::params::Settings;

use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use nih_plug::prelude::*;
use params::SelectorParams;

/// What the audio thread and the editor share.
pub struct Shared {
    pub rate: AtomicU32,
    pub editor_open: AtomicBool,
}

impl Default for Shared {
    fn default() -> Self {
        Shared { rate: AtomicU32::new(44100), editor_open: AtomicBool::new(false) }
    }
}

pub struct LemonAudioSelector {
    params: Arc<SelectorParams>,
    shared: Arc<Shared>,
    dsp: Option<Selector>,
    raw: Vec<u8>,
    raw_seen: u64,
}

impl Default for LemonAudioSelector {
    fn default() -> Self {
        LemonAudioSelector {
            params: Arc::new(SelectorParams::default()),
            shared: Arc::new(Shared::default()),
            dsp: None,
            raw: params::default_state(),
            raw_seen: u64::MAX,
        }
    }
}

impl LemonAudioSelector {
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

impl Plugin for LemonAudioSelector {
    const NAME: &'static str = "Lemon Audio Selector";
    const VENDOR: &'static str = "Lemon Studio";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
        AudioIOLayout {
            main_input_channels: NonZeroU32::new(2),
            main_output_channels: NonZeroU32::new(2),
            ..AudioIOLayout::const_default()
        },
    ];
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        Some(Box::new(editor::LemonAudioSelectorEditor::new(self.params.clone(), self.shared.clone())))
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
            None => self.dsp = Some(Selector::with_settings(rate, &s)),
        }
        true
    }

    fn reset(&mut self) {
        if let Some(d) = &mut self.dsp {
            d.reset();
        }
    }

    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, ctx: &mut impl ProcessContext<Self>) -> ProcessStatus {
        let s = self.settings();
        let Some(dsp) = &mut self.dsp else { return ProcessStatus::Normal };
        dsp.set(&s);
        while let Some(ev) = ctx.next_event() {
            if let NoteEvent::NoteOn { note, velocity, channel, .. } = ev {
                dsp.note_on(note, velocity, channel);
            }
        }
        let out = buffer.as_slice();
        if out.len() < 2 {
            return ProcessStatus::Normal;
        }
        let (left, right) = out.split_at_mut(1);
        dsp.process(left[0], right[0]);
        ProcessStatus::Normal
    }
}

impl ClapPlugin for LemonAudioSelector {
    const CLAP_ID: &'static str = "org.openstudio.lemon-audio-selector";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Lemon Studio's stereo audio selector");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect, ClapFeature::Stereo];
}

impl Vst3Plugin for LemonAudioSelector {
    const VST3_CLASS_ID: [u8; 16] = *b"LemonAudSelector";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx];
}

#[cfg(not(feature = "linked"))]
nih_export_clap!(LemonAudioSelector);
#[cfg(not(feature = "linked"))]
nih_export_vst3!(LemonAudioSelector);
os_plugkit::linked_clap_entry!(LemonAudioSelector);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::SlotKind;

    #[test]
    fn manual_selection_routing() {
        let mut p = LemonAudioSelector::default();
        // Default manual = Bypass.
        let s = p.settings();
        assert_eq!(s.manual, SlotKind::Bypass);
        // Simulate buffer processing through dsp directly.
        let mut sel = Selector::with_settings(44100, &s);
        let mut l = vec![0.5; 4];
        let mut r = vec![0.5; 4];
        sel.process(&mut l, &mut r);
        assert!(l.iter().all(|x| x.is_finite()));
        // Manual Drive selection changes output.
        let s2 = Settings { manual: SlotKind::Drive, ..s };
        let mut sel2 = Selector::with_settings(44100, &s2);
        let mut l2 = vec![0.8; 4];
        let mut r2 = vec![0.8; 4];
        sel2.process(&mut l2, &mut r2);
        assert!(l2[0] != 0.8 && l2[0].abs() <= 1.5);
    }

    #[test]
    fn state_roundtrip() {
        let s = Settings::default();
        let blob = s.to_state();
        assert_eq!(Settings::from_state(&blob), Some(s));
    }
}
