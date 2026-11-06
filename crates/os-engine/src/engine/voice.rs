// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! A sampler voice mid-playback: its position, rate, gain ramp and release, and the slide a glide note moves it by.

use super::triggers::{ClipShape, Origin, Via};

#[derive(Debug, Clone, Copy)]
pub(crate) struct Voice {
    pub(crate) channel: usize,
    pub(crate) pos: f64,
    /// Key rate; times the channel's base rate (so pitch edits reach ringing voices).
    pub(crate) step: f64,
    /// Tick at which the voice is cut (the end of an audio clip), if any.
    pub(crate) cut: Option<u64>,
    /// Tick at which the note is released (the envelopes' release starts); `None` = released with the song.
    pub(crate) release_at: Option<u64>,
    pub(crate) vel: f64,
    pub(crate) pan: f64,
    /// Gain applied at the end of the last span; ramps to the channel's current gain, so
    /// volume / pan edits reach ringing voices without clicks.
    pub(crate) gain: (f32, f32),
    /// Gain the ramp is heading for, its per-frame step and the frames left.
    pub(crate) dest: (f32, f32),
    pub(crate) delta: (f32, f32),
    pub(crate) ramp_left: u32,
    /// The channel's glide count when the voice started.
    pub(crate) serial: u32,
    /// Key of a live note (`note_on`), which `note_off` releases.
    pub(crate) live: Option<u16>,
    /// Release fade after a live note off: (frames left, fade length).
    pub(crate) release: Option<(u32, u32)>,
    /// Frames played so far, and the age at which the note ended (the channel envelopes' clock).
    pub(crate) age: u64,
    pub(crate) off: Option<u64>,
    /// An audio clip's fades and gain.
    pub(crate) shape: Option<ClipShape>,
    /// The channel buffer it plays (`Trigger::buf`).
    pub(crate) buf: u16,
    /// Its note's key, and where slide notes have moved it.
    pub(crate) key: u16,
    pub(crate) slide: VoiceSlide,
    /// Its note's colour group: slide notes of that colour move it.
    pub(crate) colour: u8,
    /// The layer its note came through.
    pub(crate) via: Option<Via>,
    /// The note or audio clip that started it (`None`: a live note).
    pub(crate) origin: Option<Origin>,
}

/// A voice's levels as slide notes move them (spec: audio/song-structure.md "Slide notes", audio/note-rules.md
/// "Glides"): the pitch moved in cents, the velocity byte and the note pan, singles that each tick of a glide adds its
/// step to; the key rate the voice started with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct VoiceSlide {
    pub(crate) cents: f32,
    pub(crate) velocity: f32,
    pub(crate) pan: f32,
    pub(crate) rate: f64,
    /// Steps per tick of a glide under way, and the steps left.
    pub(crate) step: (f32, f32, f32),
    pub(crate) left: u32,
}

impl VoiceSlide {
    pub(crate) fn new(rate: f64, velocity: u8, pan: f64) -> Self {
        VoiceSlide { cents: 0.0, velocity: f32::from(velocity), pan: pan as f32, rate, step: (0.0, 0.0, 0.0), left: 0 }
    }
}

/// Release fade of a live note (a Lemon Studio choice, spec: ui/typing-keyboard.md open questions).
pub(crate) const RELEASE_SECONDS: f64 = 0.015;
