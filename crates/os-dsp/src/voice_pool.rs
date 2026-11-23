// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! A per-channel voice pool for tracking note context per voice.
//!
//! The modulation system's [`NoteBank`] (in `os-engine`) is per-channel and holds only the newest
//! note. For per-voice ADSR and other per-voice modulation sources, we need to track the note
//! context (key, velocity, gate) for each individual voice. This module provides a voice pool
//! that can be used by both the articulation system (channel voices) and the modulation system
//! (per-voice modulation sources like ADSR).
//!
//! The pool is designed to be shared: the engine's `VoiceLedger` (per plugin instance) and the
//! modulation system's per-voice note context can both reference the same voice identities.

use std::collections::HashMap;

/// Maximum voices per channel (matches `os-engine::MAX_VOICES`).
pub const MAX_VOICES_PER_CHANNEL: usize = 512;

/// A note event for a voice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceNote {
    /// MIDI key (0-127).
    pub key: u8,
    /// MIDI velocity (0-127).
    pub velocity: u8,
    /// The voice's unique id within this channel.
    pub voice_id: u32,
    /// Whether the note is currently held (gate high).
    pub gate: bool,
}

/// Per-voice note context: the key, velocity, and gate state for a single voice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoiceContext {
    pub key: u8,
    pub velocity: u8,
    pub gate: bool,
    /// The voice's age in ticks (for voice stealing priority).
    pub age_ticks: u32,
}

impl VoiceContext {
    /// A new voice context from a note-on event.
    pub fn from_note_on(note: VoiceNote) -> Self {
        VoiceContext {
            key: note.key,
            velocity: note.velocity,
            gate: true,
            age_ticks: 0,
        }
    }

    /// Update from a note-off event (sets gate low, keeps key/velocity for release stage).
    pub fn note_off(&mut self) {
        self.gate = false;
    }

    /// The velocity as a normalized 0..1 value.
    pub fn velocity_norm(&self) -> f32 {
        self.velocity as f32 / 127.0
    }

    /// The key as a normalized 0..1 value (for key tracking).
    pub fn key_norm(&self) -> f32 {
        self.key as f32 / 127.0
    }
}

/// A pool of voices for a single channel.
#[derive(Debug, Default, Clone)]
pub struct ChannelVoicePool {
    /// Active voices, keyed by voice_id.
    voices: HashMap<u32, VoiceContext>,
    /// Next voice id to assign (monotonic within the channel).
    next_voice_id: u32,
    /// The channel index this pool belongs to.
    channel: usize,
}

impl ChannelVoicePool {
    /// Create a new voice pool for a channel.
    pub fn new(channel: usize) -> Self {
        ChannelVoicePool {
            voices: HashMap::new(),
            next_voice_id: 1, // 0 = no voice
            channel,
        }
    }

    /// The channel this pool belongs to.
    pub fn channel(&self) -> usize {
        self.channel
    }

    /// Handle a note-on event: allocate a new voice or reuse a stolen one.
    /// Returns the voice_id of the started voice.
    pub fn note_on(&mut self, key: u8, velocity: u8) -> u32 {
        let voice_id = self.next_voice_id;
        self.next_voice_id = self.next_voice_id.wrapping_add(1);
        if self.next_voice_id == 0 {
            self.next_voice_id = 1;
        }

        // If at capacity, steal the oldest released voice.
        if self.voices.len() >= MAX_VOICES_PER_CHANNEL {
            self.steal_oldest_released();
        }

        let ctx = VoiceContext::from_note_on(VoiceNote {
            key,
            velocity,
            voice_id,
            gate: true,
        });
        self.voices.insert(voice_id, ctx);
        voice_id
    }

    /// Handle a note-off event: mark the voice's gate as low.
    /// The voice stays in the pool until explicitly removed (by the voice ledger or modulation system).
    pub fn note_off(&mut self, key: u8) {
        // Find the oldest held voice with this key and release it.
        // This matches the behavior of `VoiceLedger::release`.
        let mut best: Option<u32> = None;
        let mut best_age = 0u32;
        for (id, ctx) in &self.voices {
            if ctx.key == key && ctx.gate {
                if best.is_none() || ctx.age_ticks > best_age {
                    best = Some(*id);
                    best_age = ctx.age_ticks;
                }
            }
        }
        if let Some(id) = best {
            if let Some(ctx) = self.voices.get_mut(&id) {
                ctx.note_off();
            }
        }
    }

    /// Handle a choke event: immediately remove all voices with this key.
    pub fn choke(&mut self, key: u8) {
        self.voices.retain(|_, ctx| ctx.key != key);
    }

    /// Advance all voices by one tick (increment age_ticks).
    pub fn advance_tick(&mut self) {
        for ctx in self.voices.values_mut() {
            ctx.age_ticks = ctx.age_ticks.saturating_add(1);
        }
    }

    /// Remove a specific voice by id (when the voice ledger reports it ended).
    pub fn remove_voice(&mut self, voice_id: u32) {
        self.voices.remove(&voice_id);
    }

    /// Get the context for a voice, if it exists.
    pub fn get(&self, voice_id: u32) -> Option<&VoiceContext> {
        self.voices.get(&voice_id)
    }

    /// Get a mutable reference to a voice's context.
    pub fn get_mut(&mut self, voice_id: u32) -> Option<&mut VoiceContext> {
        self.voices.get_mut(&voice_id)
    }

    /// Iterate over all active voices.
    pub fn iter(&self) -> impl Iterator<Item = (u32, &VoiceContext)> {
        self.voices.iter().map(|(id, ctx)| (*id, ctx))
    }

    /// Iterate over held (gate high) voices.
    pub fn iter_held(&self) -> impl Iterator<Item = (u32, &VoiceContext)> {
        self.voices.iter().filter(|(_, ctx)| ctx.gate).map(|(id, ctx)| (*id, ctx))
    }

    /// Iterate over released (gate low) voices.
    pub fn iter_released(&self) -> impl Iterator<Item = (u32, &VoiceContext)> {
        self.voices.iter().filter(|(_, ctx)| !ctx.gate).map(|(id, ctx)| (*id, ctx))
    }

    /// Number of active voices.
    pub fn len(&self) -> usize {
        self.voices.len()
    }

    /// Whether the pool is empty.
    pub fn is_empty(&self) -> bool {
        self.voices.is_empty()
    }

    /// Steal the oldest released voice to make room for a new one.
    fn steal_oldest_released(&mut self) {
        let mut oldest: Option<u32> = None;
        let mut oldest_age = 0u32;
        for (id, ctx) in &self.voices {
            if !ctx.gate {
                if oldest.is_none() || ctx.age_ticks > oldest_age {
                    oldest = Some(*id);
                    oldest_age = ctx.age_ticks;
                }
            }
        }
        // If no released voice, steal the oldest held voice (last resort).
        if oldest.is_none() {
            for (id, ctx) in &self.voices {
                if oldest.is_none() || ctx.age_ticks > oldest_age {
                    oldest = Some(*id);
                    oldest_age = ctx.age_ticks;
                }
            }
        }
        if let Some(id) = oldest {
            self.voices.remove(&id);
        }
    }

    /// Clear all voices (stop all notes).
    pub fn clear(&mut self) {
        self.voices.clear();
    }

    /// Get the newest note across all voices (for modulation sources that read "latest note").
    pub fn latest_note(&self) -> Option<VoiceNote> {
        self.voices
            .values()
            .max_by_key(|ctx| ctx.age_ticks)
            .map(|ctx| VoiceNote {
                key: ctx.key,
                velocity: ctx.velocity,
                voice_id: 0, // Not a specific voice
                gate: ctx.gate,
            })
    }
}

/// A voice pool for all channels in a project.
#[derive(Debug, Default, Clone)]
pub struct VoicePool {
    channels: Vec<ChannelVoicePool>,
}

impl VoicePool {
    /// Create a new voice pool with `channel_count` channels.
    pub fn new(channel_count: usize) -> Self {
        VoicePool {
            channels: (0..channel_count).map(ChannelVoicePool::new).collect(),
        }
    }

    /// Resize the pool to `channel_count` channels.
    pub fn set_channels(&mut self, channel_count: usize) {
        if self.channels.len() < channel_count {
            self.channels.extend(
                (self.channels.len()..channel_count).map(ChannelVoicePool::new),
            );
        } else if self.channels.len() > channel_count {
            self.channels.truncate(channel_count);
        }
    }

    /// Get a channel's voice pool.
    pub fn channel(&self, channel: usize) -> Option<&ChannelVoicePool> {
        self.channels.get(channel)
    }

    /// Get a mutable reference to a channel's voice pool.
    pub fn channel_mut(&mut self, channel: usize) -> Option<&mut ChannelVoicePool> {
        self.channels.get_mut(channel)
    }

    /// Iterate over every channel's voice pool, for sources that read voices globally.
    pub fn channels(&self) -> impl Iterator<Item = &ChannelVoicePool> {
        self.channels.iter()
    }

    /// Handle a note-on event for a channel.
    pub fn note_on(&mut self, channel: usize, key: u8, velocity: u8) -> Option<u32> {
        self.channel_mut(channel).map(|c| c.note_on(key, velocity))
    }

    /// Handle a note-off event for a channel.
    pub fn note_off(&mut self, channel: usize, key: u8) {
        if let Some(c) = self.channel_mut(channel) {
            c.note_off(key);
        }
    }

    /// Handle a choke event for a channel.
    pub fn choke(&mut self, channel: usize, key: u8) {
        if let Some(c) = self.channel_mut(channel) {
            c.choke(key);
        }
    }

    /// Advance all channels by one tick.
    pub fn advance_tick(&mut self) {
        for c in &mut self.channels {
            c.advance_tick();
        }
    }

    /// Remove a specific voice from a channel.
    pub fn remove_voice(&mut self, channel: usize, voice_id: u32) {
        if let Some(c) = self.channel_mut(channel) {
            c.remove_voice(voice_id);
        }
    }

    /// Clear all voices in all channels.
    pub fn clear(&mut self) {
        for c in &mut self.channels {
            c.clear();
        }
    }

    /// Get the newest note across all channels (for modulation sources that read "latest note anywhere").
    pub fn latest_note_any(&self) -> Option<VoiceNote> {
        self.channels.iter().filter_map(|c| c.latest_note()).max_by_key(|n| n.velocity as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_pool_allocates_voices() {
        let mut pool = ChannelVoicePool::new(0);
        let id1 = pool.note_on(60, 100);
        let id2 = pool.note_on(64, 80);
        assert_ne!(id1, id2);
        assert_eq!(pool.len(), 2);
        assert!(pool.get(id1).is_some());
        assert!(pool.get(id2).is_some());
    }

    #[test]
    fn channel_pool_note_off_clears_gate() {
        let mut pool = ChannelVoicePool::new(0);
        let id = pool.note_on(60, 100);
        pool.note_off(60);
        let ctx = pool.get(id).expect("voice still exists after note_off");
        assert!(!ctx.gate);
        assert_eq!(ctx.key, 60);
        assert_eq!(ctx.velocity, 100);
    }

    #[test]
    fn channel_pool_choke_removes_voices() {
        let mut pool = ChannelVoicePool::new(0);
        pool.note_on(60, 100);
        pool.note_on(64, 80);
        pool.choke(60);
        assert_eq!(pool.len(), 1);
        assert_eq!(pool.get_mut(1).map(|ctx| ctx.key), Some(64));
    }

    #[test]
    fn channel_pool_steals_oldest_released() {
        let mut pool = ChannelVoicePool::new(0);
        // Fill to capacity
        for i in 0..MAX_VOICES_PER_CHANNEL {
            pool.note_on(i as u8, 100);
        }
        // Release half
        for i in 0..(MAX_VOICES_PER_CHANNEL / 2) {
            pool.note_off(i as u8);
        }
        pool.advance_tick(); // Age the released voices
        // Add one more - should steal oldest released
        let new_id = pool.note_on(100, 100);
        assert_eq!(pool.len(), MAX_VOICES_PER_CHANNEL);
        assert!(pool.get(new_id).is_some());
    }

    #[test]
    fn voice_pool_multichannel() {
        let mut pool = VoicePool::new(4);
        let id1 = pool.note_on(0, 60, 100).expect("channel 0");
        let id2 = pool.note_on(1, 64, 80).expect("channel 1");
        assert_ne!(id1, id2);
        assert_eq!(pool.channel(0).unwrap().len(), 1);
        assert_eq!(pool.channel(1).unwrap().len(), 1);
        assert_eq!(pool.channel(2).unwrap().len(), 0);
    }

    #[test]
    fn voice_pool_advance_tick() {
        let mut pool = VoicePool::new(1);
        pool.note_on(0, 60, 100);
        assert_eq!(pool.channel(0).unwrap().get(1).unwrap().age_ticks, 0);
        pool.advance_tick();
        assert_eq!(pool.channel(0).unwrap().get(1).unwrap().age_ticks, 1);
    }

    #[test]
    fn latest_note_returns_newest() {
        let mut pool = VoicePool::new(1);
        pool.note_on(0, 60, 100);
        pool.advance_tick();
        pool.advance_tick();
        pool.note_on(0, 64, 80); // newer
        let latest = pool.latest_note_any().expect("has latest");
        assert_eq!(latest.key, 64);
        assert_eq!(latest.velocity, 80);
    }
}