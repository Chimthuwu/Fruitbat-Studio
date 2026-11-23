// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! FreqBat: Fruitbat Studio's polyphonic synthesizer with per-voice modulation using os-dsp voice pool.
//!
//! spec: plugins/freqbat.md, plugins/plugin-formats.md

#![allow(semicolon_in_expressions_from_non_local_macros)]

pub mod dsp;
pub mod editor;
pub mod params;
pub mod view;

use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nih_plug::prelude::*;
use os_engine::envelope;
use os_plugkit::levels::Levels;

use params::FreqBatParams;

/// State shared by the audio thread and the editor.
#[derive(Default)]
pub struct Shared {
    /// Notes the editor plays itself (key, velocity, on) when the host does not play them.
    pub previews: Mutex<Vec<(u8, f32, bool)>>,
    /// Keys sounding (bit per key).
    pub sounding: [AtomicU64; 2],
    /// The patch version for the audio thread.
    pub patch_version: AtomicU64,
    /// The patch being edited.
    pub patch: Mutex<Option<dsp::Patch>>,
}

impl Shared {
    /// Sets a new patch from the editor.
    pub fn set_patch(&self, patch: dsp::Patch) {
        if let Ok(mut p) = self.patch.lock() {
            *p = Some(patch);
        }
        self.patch_version.fetch_add(1, Ordering::AcqRel);
    }

    /// Gets the current patch version.
    pub fn patch_version(&self) -> u64 {
        self.patch_version.load(Ordering::Acquire)
    }

    /// Gets the current patch.
    pub fn patch(&self) -> Option<dsp::Patch> {
        self.patch.lock().ok().and_then(|p| p.clone())
    }

    pub fn sounding_keys(&self) -> Vec<i32> {
        let mut out = Vec::new();
        for (w, a) in self.sounding.iter().enumerate() {
            let bits = a.load(Ordering::Relaxed);
            for b in 0..64 {
                if bits & (1 << b) != 0 {
                    out.push((w * 64 + b) as i32);
                }
            }
        }
        out
    }
}

/// Seconds in a tick at `tempo` BPM: the plugin is not told the project's PPQ and takes the original's default of 96.
pub fn tick_seconds(tempo: f64) -> f64 {
    60.0 / (tempo.max(1.0) * 96.0)
}

pub struct FreqBatPlugin {
    params: Arc<FreqBatParams>,
    shared: Arc<Shared>,
    engine: dsp::Engine,
    patch_seen: u64,
    /// Pitch bend in cents.
    bend: f64,
}

impl Default for FreqBatPlugin {
    fn default() -> Self {
        FreqBatPlugin {
            params: Arc::new(FreqBatParams::default()),
            shared: Arc::new(Shared::default()),
            engine: dsp::Engine::new(44100.0),
            patch_seen: u64::MAX,
            bend: 0.0,
        }
    }
}

impl FreqBatPlugin {
    /// The engine takes the patch last built (never waits: a busy lock is tried again next block).
    fn take_patch(&mut self) {
        let v = self.shared.patch_version();
        if v != self.patch_seen
            && let Ok(p) = self.shared.patch.try_lock()
        {
            let patch = p.clone();
            drop(p);
            if let Some(patch) = patch {
                self.engine.set_patch(patch);
            }
            self.patch_seen = v;
        }
    }

    fn publish_sounding(&self) {
        let mut bits = [0u64; 2];
        for k in self.engine.sounding() {
            let k = usize::from(k).min(127);
            bits[k / 64] |= 1 << (k % 64);
        }
        for (a, b) in self.shared.sounding.iter().zip(bits) {
            a.store(b, Ordering::Relaxed);
        }
    }
}

impl Plugin for FreqBatPlugin {
    const NAME: &'static str = "FreqBat";
    const VENDOR: &'static str = "Fruitbat Studio";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];
    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = Levels;
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        Some(Box::new(editor::FreqBatEditor::new(self.params.clone(), self.shared.clone())))
    }

    fn initialize(&mut self, _layout: &AudioIOLayout, config: &BufferConfig, _ctx: &mut impl InitContext<Self>) -> bool {
        self.engine.set_rate(config.sample_rate);
        // A restored state names a patch: load it now (initialize may block).
        self.take_patch();
        true
    }

    fn reset(&mut self) {
        self.engine.reset();
        self.publish_sounding();
    }

    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, ctx: &mut impl ProcessContext<Self>) -> ProcessStatus {
        self.take_patch();
        let tick = tick_seconds(ctx.transport().tempo.unwrap_or(140.0));
        let mut blk = dsp::block(&self.params, self.bend, tick);
        if let Ok(mut q) = self.shared.previews.try_lock() {
            for (key, vel, on) in q.drain(..) {
                if on {
                    self.engine.note_on(key, vel, None, &blk);
                } else {
                    self.engine.note_off(key);
                }
            }
        }
        let len = buffer.samples();
        let out = buffer.as_slice();
        for ch in out.iter_mut() {
            ch.fill(0.0);
        }
        if out.len() < 2 {
            return ProcessStatus::Normal;
        }
        let mut at = 0;
        while let Some(ev) = ctx.next_event() {
            let t = (ev.timing() as usize).min(len);
            if t > at {
                let (left, right) = out.split_at_mut(1);
                self.engine.render(&mut left[0][at..t], &mut right[0][at..t], &blk);
                at = t;
            }
            match ev {
                NoteEvent::NoteOn { note, velocity, voice_id, .. } => self.engine.host_note_on(note, velocity, voice_id, &blk),
                NoteEvent::NoteOff { note, voice_id, .. } => self.engine.note_off_id(note, voice_id),
                NoteEvent::Choke { note, .. } => self.engine.choke(note),
                // Portamento and slide notes: the host's tuning of a key's voices.
                NoteEvent::PolyTuning { note, voice_id, tuning, .. } => self.engine.tune(note, voice_id, f64::from(tuning)),
                // A note's own pan, and the channel's volume and pan from Fruitbat Studio.
                NoteEvent::PolyPan { note, voice_id, pan, .. } => self.engine.pan(note, voice_id, pan),
                NoteEvent::MidiSysEx { message: Levels(volume, pan), .. } => self.engine.set_levels(volume, pan),
                // A note's mod X / Y: the host's per-note modulation of the filter knobs, in mod units.
                NoteEvent::PolyModulation { voice_id, poly_modulation_id, normalized_offset, .. } => match poly_modulation_id {
                    params::MOD_X_POLY => self.engine.modulate(voice_id, 0, normalized_offset),
                    params::MOD_Y_POLY => self.engine.modulate(voice_id, 1, normalized_offset),
                    _ => {}
                },
                NoteEvent::MidiPitchBend { value, .. } => {
                    // ±2 semitones, as the other Fruitbat instruments.
                    self.bend = f64::from(value * 2.0 - 1.0) * 200.0;
                    blk.bend = self.bend;
                }
                NoteEvent::MidiCC { cc: 120 | 123, .. } => self.engine.all_off(),
                _ => {}
            }
        }
        if at < len {
            let (left, right) = out.split_at_mut(1);
            self.engine.render(&mut left[0][at..len], &mut right[0][at..len], &blk);
        }
        // Voice ends go back to the host, at the end of the block (spec: audio/voices.md §4).
        let last = (len as u32).saturating_sub(1);
        for (note, voice_id) in self.engine.take_ended() {
            ctx.send_event(NoteEvent::VoiceTerminated { timing: last, voice_id, channel: 0, note });
        }
        self.publish_sounding();
        if self.engine.has_voices() { ProcessStatus::KeepAlive } else { ProcessStatus::Normal }
    }
}

// CLAP id and VST3 class id keep the spelling of the plugin's name: hosts find saved sessions' plugins by them.
impl ClapPlugin for FreqBatPlugin {
    const CLAP_ID: &'static str = "org.fruitbatstudio.freqbat";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Fruitbat Studio's polyphonic synthesizer with per-voice modulation");
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::Instrument, ClapFeature::Synthesizer, ClapFeature::Stereo];
}

impl Vst3Plugin for FreqBatPlugin {
    const VST3_CLASS_ID: [u8; 16] = *b"FruitbatFreqBat!";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

// Built as a plugin module: the C entry points hosts look for. Linked into Lemon Studio (`linked` feature): only the
// symbol-free table, so several Fruitbat plugins fit in one program (spec: plugins/plugin-formats.md).
#[cfg(not(feature = "linked"))]
nih_export_clap!(FreqBatPlugin);
#[cfg(not(feature = "linked"))]
nih_export_vst3!(FreqBatPlugin);
os_plugkit::linked_clap_entry!(FreqBatPlugin);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_block_is_the_default_patch() {
        let p = FreqBatParams::default();
        let b = dsp::block(&p, 0.0, 0.0);
        let d = dsp::Block::default();
        assert_eq!((b.osc, b.m2, b.m3, b.am, b.hq, b.rand), (d.osc, d.m2, d.m3, d.am, d.hq, d.rand));
        assert_eq!((b.filter, b.cutoff, b.resonance), (0, 256.0, 0.0));
        assert!(b.filter_off());
        assert!(b.targets.iter().all(|t| t.env.is_none() && t.lfo.amount == 0.0));
    }
}