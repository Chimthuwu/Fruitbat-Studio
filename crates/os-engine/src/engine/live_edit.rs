// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! A project edit a playing engine takes in place, so moving a knob does not build a new engine, and the diff that finds them.

use os_model::Project;

/// A project edit a playing engine can take in place ([`Engine::apply_edit`]), so that moving a knob does not build a new
/// engine: the channels' volume, pan and pitch knobs and the mixer tracks' volume, pan and separation, by index, as
/// `Channel` and `MixerTrack` hold them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LiveEdit {
    ChannelVolume(usize, f64),
    ChannelPan(usize, f64),
    ChannelPitch(usize, i32),
    MixerVolume(usize, f64),
    MixerPan(usize, f64),
    MixerSeparation(usize, f64),
}

impl LiveEdit {
    /// `project` as this edit leaves it.
    pub fn apply_to(self, project: &mut Project) {
        let channel = |p: &mut Project, c: usize, f: &mut dyn FnMut(&mut os_model::Channel)| {
            if let Some(ch) = p.channels.get_mut(c) {
                f(ch);
            }
        };
        match self {
            LiveEdit::ChannelVolume(c, v) => channel(project, c, &mut |ch| ch.volume = v),
            LiveEdit::ChannelPan(c, p) => channel(project, c, &mut |ch| ch.pan = p),
            LiveEdit::ChannelPitch(c, cents) => channel(project, c, &mut |ch| ch.pitch_cents = cents),
            LiveEdit::MixerVolume(t, v) => {
                if let Some(m) = project.mixer.get_mut(t) {
                    m.volume = Some(v);
                }
            }
            LiveEdit::MixerPan(t, v) => {
                if let Some(m) = project.mixer.get_mut(t) {
                    m.pan = Some(v);
                }
            }
            LiveEdit::MixerSeparation(t, v) => {
                if let Some(m) = project.mixer.get_mut(t) {
                    m.stereo_separation = Some(v);
                }
            }
        }
    }
}

/// The edits that turn `sent` into `new`, when those are all that differ and there is one at least; `sent` is left with
/// them applied. `None`: something else differs, or nothing does, and a new engine is built as ever (it also reads the
/// program's settings). A track's level that goes unset is not such an edit.
pub fn live_edits(sent: &mut Project, new: &Project) -> Option<Vec<LiveEdit>> {
    if sent.channels.len() != new.channels.len() || sent.mixer.len() != new.mixer.len() {
        return None;
    }
    let mut edits = Vec::new();
    for (c, (a, b)) in sent.channels.iter().zip(&new.channels).enumerate() {
        if a.volume != b.volume {
            edits.push(LiveEdit::ChannelVolume(c, b.volume));
        }
        if a.pan != b.pan {
            edits.push(LiveEdit::ChannelPan(c, b.pan));
        }
        if a.pitch_cents != b.pitch_cents {
            edits.push(LiveEdit::ChannelPitch(c, b.pitch_cents));
        }
    }
    for (t, (a, b)) in sent.mixer.iter().zip(&new.mixer).enumerate() {
        if let (true, Some(v)) = (a.volume != b.volume, b.volume) {
            edits.push(LiveEdit::MixerVolume(t, v));
        }
        if let (true, Some(v)) = (a.pan != b.pan, b.pan) {
            edits.push(LiveEdit::MixerPan(t, v));
        }
        if let (true, Some(v)) = (a.stereo_separation != b.stereo_separation, b.stereo_separation) {
            edits.push(LiveEdit::MixerSeparation(t, v));
        }
    }
    if edits.is_empty() {
        return None;
    }
    for e in &edits {
        e.apply_to(sent);
    }
    (*sent == *new).then_some(edits)
}
