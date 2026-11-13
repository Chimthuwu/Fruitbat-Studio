// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Studio's own state saved in a `.lemon` file's `LMex` chunk, where an FLP has no place for it (Lemon Studio's
//! design, spec: flp/lemon-format.md "LMex"): the icons chosen with Change icon..., the effect slots' names and colours,
//! Performance mode's settings, the mixer's Plugin delay compensation ▸ Automatic and View ▸ checks, the plugin windows'
//! Scaling ▸, the channels' Show waveform preview and MIDI channel through checks, Set keyboard focus of all plugins
//! to ▸, the playlist tracks' locks to their clips and the Receive notes from ▸ locks. Each kind is one event; its
//! payload starts with a layout version (1), and numbers are little-endian. Save writes them all (empty ones too, so a
//! file saved again holds no stale state); an FLP target drops them with the rest of `LMex`. Open reads them back from
//! a `.lemon` file only; an FLP's icons come from its icon numbers instead.
//!
//! Two of them belong to the modulation system: the persistent identity registry (M0, event 206) and the graph
//! itself (M1, event 207). The graph names parameters by that identity, so the two are written together and neither
//! is ever an FLP target (audit K.1/K.3).

use crate::app::State;
use crate::playlist::{ICONS, IconOwner};
use crate::plugins::Plugins;
use os_flp::{Event, FlpFile, Value, lemon};
use os_model::modulation::{
    Curve, Envelope, IdentityRegistry, Lfo, LfoShape, ModGraph, ModSource, ModTarget, Modulator, ModulatorId, OwnerLocator,
    ParameterKey, ParameterPath, PersistentObjectId, Polarity, Steps, Timebase,
};
use std::collections::BTreeMap;

/// The payload layout written (the first byte of each event). A reader ignores an event with a layout it does not know.
const LAYOUT: u8 = 2;
/// The layout written before project slots, random follow shape edits and per-routing enable. Still read.
const LAYOUT_1: u8 = 1;

/// Icon owner kinds in [`lemon::ICONS`].
const CHANNEL: u8 = 0;
const TRACK: u8 = 1;
const SLOT: u8 = 2;

/// Performance flags in [`lemon::PERFORMANCE`].
const CLIP_PROGRESS: u8 = 1;
const TRACK_PROGRESS: u8 = 2;
const PLUS_SCENE: u8 = 4;
const VELOCITY_SUBCLIPS: u8 = 8;

/// Mixer flags in [`lemon::MIXER`].
const PDC_AUTO: u8 = 1;
const WAVEFORMS: u8 = 2;
const EXTRA_PROPS: u8 = 4;

/// Channel flags in [`lemon::CHANNEL_CHECKS`].
const WAVE_PREVIEW: u8 = 1;
const MIDI_THROUGH: u8 = 2;

/// Clip sources in [`lemon::TRACK_LOCKS`].
const PATTERN: u8 = 0;
const CLIP_CHANNEL: u8 = 1;

/// The `LMex` events holding the Lemon Studio state of `s` and `plugins`, in ID order. Set keyboard focus of all
/// plugins to ▸ is written only where it works (Windows and macOS); elsewhere the file's own event is kept.
pub fn events(s: &State, plugins: &Plugins) -> Vec<Event> {
    let mut v = vec![icons(s), slot_names(s), slot_colors(s), performance(s), mixer(s), scaling(s, plugins), channel_checks(s)];
    #[cfg(any(windows, target_os = "macos"))]
    v.push(Event::new(lemon::KEY_FOCUS, Value::var([LAYOUT, u8::from(editor_keys())])));
    v.push(track_locks(s));
    v.push(note_locks(s));
    v.push(linear_tracks(s));
    v.push(linear_mode(s));
    // The lanes only when there are any: Classic mode's file has none (zero change).
    if !s.project.auto_lanes.is_empty() {
        v.push(lane_bindings(s));
    }
    v.push(owner_ids(s));
    v.push(modulation(s));
    v
}

/// The bytes [`events`] would write: a change to them is an unsaved change, as an edit of the project is. Set keyboard
/// focus of all plugins to ▸ is left out: it is a switch for the whole program, with no undo step, so it marks no
/// project changed.
pub fn saved_form(s: &State, plugins: &Plugins) -> Vec<u8> {
    let mut b = Vec::new();
    for e in events(s, plugins).into_iter().filter(|e| e.id != lemon::KEY_FOCUS) {
        b.push(e.id);
        b.extend_from_slice(e.data().unwrap_or_default());
    }
    b
}

/// The `LMex` event holding the modulation identity registry (K.2/K.3).
pub fn owner_ids(s: &State) -> Event {
    Event::new(lemon::OWNER_IDS, Value::var(write_identity(&s.identity)))
}

/// Serializes the identity registry: layout version, id counter, the `id -> locator` map, then any
/// explicit native bindings. Little-endian; readers ignore a layout they do not know.
fn write_identity(reg: &IdentityRegistry) -> Vec<u8> {
    let mut b = vec![LAYOUT];
    b.extend_from_slice(&reg.next.to_le_bytes());
    b.extend_from_slice(&(reg.owners.len() as u32).to_le_bytes());
    for (id, loc) in &reg.owners {
        b.extend_from_slice(&id.0.to_le_bytes());
        match loc {
            OwnerLocator::Channel(i) => {
                b.push(0);
                b.extend_from_slice(&i.to_le_bytes());
            }
            OwnerLocator::Track(i) => {
                b.push(1);
                b.extend_from_slice(&i.to_le_bytes());
            }
            OwnerLocator::Slot(t, k) => {
                b.push(2);
                b.extend_from_slice(&t.to_le_bytes());
                b.push(*k);
            }
            OwnerLocator::Member(t, k, m) => {
                b.push(3);
                b.extend_from_slice(&t.to_le_bytes());
                b.push(*k);
                b.extend_from_slice(&m.to_le_bytes());
            }
        }
    }
    b.extend_from_slice(&(reg.native_bindings.len() as u32).to_le_bytes());
    for (owner, map) in &reg.native_bindings {
        b.extend_from_slice(&owner.0.to_le_bytes());
        b.extend_from_slice(&(map.len() as u32).to_le_bytes());
        for (idx, native) in map {
            b.extend_from_slice(&idx.to_le_bytes());
            b.extend_from_slice(&native.to_le_bytes());
        }
    }
    b
}

/// Inverse of [`write_identity`]; `None` for an unrecognized layout or a truncated payload.
fn read_identity(d: &[u8]) -> Option<IdentityRegistry> {
    let mut r = Reader(d);
    if r.u8()? != LAYOUT {
        return None;
    }
    let next = r.u32()?;
    let count = r.u32()? as usize;
    let mut owners = BTreeMap::new();
    for _ in 0..count {
        let id = PersistentObjectId(r.u32()?);
        let loc = match r.u8()? {
            0 => OwnerLocator::Channel(r.u16()?),
            1 => OwnerLocator::Track(r.u16()?),
            2 => OwnerLocator::Slot(r.u16()?, r.u8()?),
            3 => OwnerLocator::Member(r.u16()?, r.u8()?, r.u32()?),
            _ => return None,
        };
        owners.insert(id, loc);
    }
    let nb_count = r.u32()? as usize;
    let mut native_bindings = BTreeMap::new();
    for _ in 0..nb_count {
        let owner = PersistentObjectId(r.u32()?);
        let mc = r.u32()? as usize;
        let mut map = BTreeMap::new();
        for _ in 0..mc {
            map.insert(r.u16()?, r.u32()?);
        }
        native_bindings.insert(owner, map);
    }
    Some(IdentityRegistry { owners, next, native_bindings })
}

/// The `LMex` event holding the modulation graph (M1).
pub fn modulation(s: &State) -> Event {
    Event::new(lemon::MODULATION, Value::var(write_graph(&s.modulation)))
}

/// Serializes the graph: layout version, the modulator count, the id counter, then per modulator its id, enabled
/// flag, name, source and targets; numbers are little-endian and a source's kind picks the fields that follow.
/// Readers ignore a layout they do not know, so a later record can grow.
fn write_graph(g: &ModGraph) -> Vec<u8> {
    let mut b = start(g.modulators.len());
    b.extend_from_slice(&g.next.to_le_bytes());
    for m in &g.modulators {
        b.extend_from_slice(&m.id.0.to_le_bytes());
        b.push(u8::from(m.enabled));
        b.push(m.slot);
        let name = &m.name.as_bytes()[..m.name.len().min(usize::from(u16::MAX))];
        b.extend_from_slice(&(name.len() as u16).to_le_bytes());
        b.extend_from_slice(name);
        match &m.source {
            ModSource::Lfo(l) => {
                b.push(LFO);
                b.push(lfo_code(l.shape));
                write_timebase(&mut b, l.period);
                b.extend_from_slice(&l.phase.to_le_bytes());
                b.extend_from_slice(&l.offset.to_le_bytes());
                b.extend_from_slice(&l.amount.to_le_bytes());
                b.push(u8::from(l.bipolar));
            }
            ModSource::Steps(st) => {
                b.push(STEPS);
                write_timebase(&mut b, st.period);
                b.push(u8::from(st.smooth));
                b.extend(st.values.iter().map(|&v| v as u8));
            }
            ModSource::Envelope(e) => {
                b.push(ENVELOPE);
                for v in [e.attack, e.decay, e.sustain, e.release] {
                    b.extend_from_slice(&v.to_le_bytes());
                }
            }
            ModSource::Random(r) => {
                b.push(RANDOM);
                write_timebase(&mut b, r.period);
                b.extend_from_slice(&r.amount.to_le_bytes());
                b.push(u8::from(r.distribution == os_model::modulation::Distribution::Gaussian));
                b.extend_from_slice(&r.smooth.to_le_bytes());
                b.push(u8::from(r.interpolate));
                b.push(u8::from(r.bipolar));
                b.extend_from_slice(&r.seed.to_le_bytes());
            }
            ModSource::AudioFollower(f) => {
                b.push(FOLLOWER);
                // v1 has one source (master); the byte is the scope field for later track sources.
                b.push(0);
                for v in [f.amount, f.attack, f.release, f.gain_db] {
                    b.extend_from_slice(&v.to_le_bytes());
                }
                b.push(u8::from(f.detection == os_model::modulation::Detection::Peak));
                let (kind, low, high) = match f.band {
                    os_model::modulation::Band::Full => (0u8, 0.0f32, 0.0f32),
                    os_model::modulation::Band::Low => (1, 0.0, 0.0),
                    os_model::modulation::Band::Mid => (2, 0.0, 0.0),
                    os_model::modulation::Band::High => (3, 0.0, 0.0),
                    os_model::modulation::Band::Custom { low, high } => (4, low, high),
                };
                b.push(kind);
                if kind == 4 {
                    b.extend_from_slice(&low.to_le_bytes());
                    b.extend_from_slice(&high.to_le_bytes());
                }
            }
        }
        b.extend_from_slice(&(m.targets.len() as u16).to_le_bytes());
        for t in &m.targets {
            b.extend_from_slice(&t.path.owner.0.to_le_bytes());
            match t.path.parameter {
                ParameterKey::PluginNativeId(n) => {
                    b.push(NATIVE_ID);
                    b.extend_from_slice(&n.to_le_bytes());
                }
                ParameterKey::LegacyPluginIndex(i) => {
                    b.push(LEGACY_INDEX);
                    b.extend_from_slice(&i.to_le_bytes());
                }
                ParameterKey::HostParameterCode(c) => {
                    b.push(HOST_CODE);
                    b.extend_from_slice(&c.to_le_bytes());
                }
            }
            b.extend_from_slice(&t.depth.to_le_bytes());
            b.push(curve_code(t.curve));
            b.push(polarity_code(t.polarity));
            b.push(u8::from(t.enabled));
        }
    }
    // The strip's rows trail the modulators: a reader that stops after the count ignores them, and a payload cut
    // short reads as one row.
    b.push(g.rows as u8);
    b
}

/// A source kind in [`write_graph`].
const LFO: u8 = 0;
const STEPS: u8 = 1;
const ENVELOPE: u8 = 2;
const RANDOM: u8 = 3;
const FOLLOWER: u8 = 4;

/// A [`Timebase`] kind in [`write_graph`]: 0 hertz (f32), 1 synced (u16 sixteenths).
const FREE: u8 = 0;
const SYNC: u8 = 1;

/// A [`ParameterKey`] kind in [`write_graph`].
const NATIVE_ID: u8 = 0;
const LEGACY_INDEX: u8 = 1;
const HOST_CODE: u8 = 2;

fn lfo_code(s: LfoShape) -> u8 {
    match s {
        LfoShape::Sine => 0,
        LfoShape::Triangle => 1,
        LfoShape::SawUp => 2,
        LfoShape::SawDown => 3,
        LfoShape::Square => 4,
        LfoShape::Random => 5,
    }
}

fn lfo_shape(c: u8) -> Option<LfoShape> {
    Some(match c {
        0 => LfoShape::Sine,
        1 => LfoShape::Triangle,
        2 => LfoShape::SawUp,
        3 => LfoShape::SawDown,
        4 => LfoShape::Square,
        5 => LfoShape::Random,
        _ => return None,
    })
}

fn curve_code(c: Curve) -> u8 {
    match c {
        Curve::Linear => 0,
        Curve::Exp => 1,
        Curve::Log => 2,
        Curve::S => 3,
    }
}

fn curve_of(c: u8) -> Option<Curve> {
    Some(match c {
        0 => Curve::Linear,
        1 => Curve::Exp,
        2 => Curve::Log,
        3 => Curve::S,
        _ => return None,
    })
}

fn polarity_code(p: Polarity) -> u8 {
    match p {
        Polarity::Bipolar => 0,
        Polarity::Unipolar => 1,
    }
}

fn polarity_of(p: u8) -> Option<Polarity> {
    Some(match p {
        0 => Polarity::Bipolar,
        1 => Polarity::Unipolar,
        _ => return None,
    })
}

fn write_timebase(b: &mut Vec<u8>, t: Timebase) {
    match t {
        Timebase::Free(hz) => {
            b.push(FREE);
            b.extend_from_slice(&hz.to_le_bytes());
        }
        Timebase::Sync(beats) => {
            b.push(SYNC);
            b.extend_from_slice(&beats.to_le_bytes());
        }
    }
}

fn read_timebase(r: &mut Reader) -> Option<Timebase> {
    Some(match r.u8()? {
        FREE => Timebase::Free(r.f32()?),
        SYNC => Timebase::Sync(r.u16()?),
        _ => return None,
    })
}

/// Inverse of [`write_graph`]. A payload cut short keeps the modulators before the cut (their incomplete targets
/// left out); `None` for an unrecognized layout or one too short to hold the counter.
fn read_graph(d: &[u8]) -> Option<ModGraph> {
    let mut r = Reader(d);
    let layout = r.u8()?;
    if layout != LAYOUT && layout != LAYOUT_1 {
        return None;
    }
    let count = r.u32()? as usize;
    let next = r.u32()?;
    let mut modulators: Vec<Modulator> = Vec::new();
    for index in 0..count {
        let (Some(id), Some(enabled)) = (r.u32(), r.u8()) else { break };
        // Layout 2 stores the fixed project slot; layout 1 had no slots, so the creation order is the only place it had.
        let slot = if layout >= 2 {
            let Some(slot) = r.u8() else { break };
            slot
        } else if index < os_model::modulation::PROJECT_SLOTS {
            index as u8
        } else {
            os_model::modulation::NO_SLOT
        };
        let Some(len) = r.u16() else { break };
        let Some(name) = r.take(usize::from(len)) else { break };
        let Some(kind) = r.u8() else { break };
        let Some(source) = read_source(&mut r, kind, layout) else { break };
        let Some(tcount) = r.u16() else { break };
        let mut targets = Vec::new();
        let mut cut = false;
        for _ in 0..tcount {
            let Some(t) = read_target(&mut r, layout) else {
                cut = true;
                break;
            };
            targets.push(t);
        }
        let name = String::from_utf8_lossy(name).into_owned();
        modulators.push(Modulator { id: ModulatorId(id), name, source, targets, enabled: enabled != 0, slot });
        if cut {
            break;
        }
    }
    // Rows trail the modulators; a payload written before they existed (or cut short) is one row.
    let rows = r.u8().unwrap_or(0);
    Some(ModGraph { modulators, next, rows })
}

fn read_source(r: &mut Reader, kind: u8, layout: u8) -> Option<ModSource> {
    Some(match kind {
        LFO => {
            let shape = lfo_shape(r.u8()?)?;
            let period = read_timebase(r)?;
            let phase = r.f32()?;
            // Layout 1 ended the LFO here: it had no amount, offset or polarity of its own.
            let (offset, amount, bipolar) = if layout >= 2 { (r.f32()?, r.f32()?, r.u8()? != 0) } else { (0.0, 1.0, true) };
            ModSource::Lfo(Lfo { shape, period, phase, offset, amount, bipolar })
        }
        STEPS => {
            let period = read_timebase(r)?;
            let smooth = r.u8()? != 0;
            let mut values = [0i8; 16];
            for v in &mut values {
                *v = r.u8()? as i8;
            }
            ModSource::Steps(Steps { values, period, smooth })
        }
        ENVELOPE => {
            ModSource::Envelope(Envelope { attack: r.f32()?, decay: r.f32()?, sustain: r.f32()?, release: r.f32()? })
        }
        RANDOM => {
            let period = read_timebase(r)?;
            let amount = r.f32()?;
            let distribution = if r.u8()? != 0 { os_model::modulation::Distribution::Gaussian } else { os_model::modulation::Distribution::Uniform };
            let smooth = r.f32()?;
            let interpolate = r.u8()? != 0;
            let bipolar = r.u8()? != 0;
            let seed = r.u64()?;
            ModSource::Random(os_model::modulation::Random { period, amount, distribution, smooth, interpolate, seed, bipolar })
        }
        FOLLOWER => {
            let _source = r.u8()?;
            let amount = r.f32()?;
            let attack = r.f32()?;
            let release = r.f32()?;
            let gain_db = r.f32()?;
            let detection = if r.u8()? != 0 { os_model::modulation::Detection::Peak } else { os_model::modulation::Detection::Rms };
            let band = match r.u8()? {
                0 => os_model::modulation::Band::Full,
                1 => os_model::modulation::Band::Low,
                2 => os_model::modulation::Band::Mid,
                3 => os_model::modulation::Band::High,
                4 => os_model::modulation::Band::Custom { low: r.f32()?, high: r.f32()? },
                _ => return None,
            };
            ModSource::AudioFollower(os_model::modulation::AudioFollower { source: os_model::modulation::FollowerSource::Master, amount, attack, release, gain_db, detection, band })
        }
        _ => return None,
    })
}

fn read_target(r: &mut Reader, layout: u8) -> Option<ModTarget> {
    let owner = PersistentObjectId(r.u32()?);
    let parameter = match r.u8()? {
        NATIVE_ID => ParameterKey::PluginNativeId(r.u32()?),
        LEGACY_INDEX => ParameterKey::LegacyPluginIndex(r.u16()?),
        HOST_CODE => ParameterKey::HostParameterCode(r.u16()?),
        _ => return None,
    };
    let depth = r.f32()?;
    let curve = curve_of(r.u8()?)?;
    let polarity = polarity_of(r.u8()?)?;
    // A routing's own switch arrived in layout 2; before that every routing was on.
    let enabled = if layout >= 2 { r.u8()? != 0 } else { true };
    Some(ModTarget { path: ParameterPath { owner, parameter }, depth, curve, polarity, enabled })
}

/// Puts the state a `.lemon` file holds into `s` and `plugins` (a project just opened, its ids given). An FLP has
/// none; its channels and mixer tracks show the icons their numbers stand for instead ([`from_flp`]).
pub fn apply(s: &mut State, plugins: &mut Plugins, file: &FlpFile) {
    if !lemon::is_lemon(file) {
        from_flp(s);
        return;
    }
    let read = |id| lemon::data(file, id).filter(|d| d.first() == Some(&LAYOUT));
    if let Some(&on) = read(lemon::LINEAR_MODE).as_deref().and_then(|d| d.get(1)) {
        s.project.linear_mode = on != 0;
    }
    // The modulation system's persistent device/owner identity registry (M0, K.2/K.3) and its graph (M1): the graph
    // names parameters by the ids the registry holds, so they are read together.
    if let Some(d) = read(lemon::OWNER_IDS) {
        if let Some(reg) = read_identity(&d) {
            s.identity = reg;
        }
    }
    if let Some(d) = read(lemon::MODULATION) {
        if let Some(g) = read_graph(&d) {
            s.modulation = g;
        }
    }
    if let Some(d) = read(lemon::TRACKS) {
        s.project.tracks = read_tracks(&d[1..]);
    }
    if let Some(d) = read(lemon::LANE_BINDINGS) {
        s.project.auto_lanes = read_lane_bindings(&d[1..], &s.project.tracks);
    }
    if let Some(&flags) = read(lemon::MIXER).as_deref().and_then(|d| d.get(1)) {
        s.project.pdc_off = flags & PDC_AUTO == 0;
        s.mixer.waveforms = flags & WAVEFORMS != 0;
        s.mixer.extra = flags & EXTRA_PROPS != 0;
    }
    if let Some(d) = read(lemon::SCALING) {
        let mut r = Reader(&d[1..]);
        for _ in 0..r.u32().unwrap_or(0) {
            let (Some(kind), Some(n), Some(k), Some(pct)) = (r.u8(), r.u16(), r.u8(), r.u16()) else { break };
            let key = match kind {
                CHANNEL => usize::from(n),
                SLOT => crate::plugins::fx_key(n, k),
                _ => continue,
            };
            if pct > 0 {
                plugins.editor_scales.insert(key, f32::from(pct) / 100.0);
            }
        }
    }
    if let Some(d) = read(lemon::CHANNEL_CHECKS) {
        let mut r = Reader(&d[1..]);
        for _ in 0..r.u32().unwrap_or(0) {
            let (Some(n), Some(flags)) = (r.u16(), r.u8()) else { break };
            let Some(id) = s.project.channels.get(usize::from(n)).map(|c| c.id.0) else { continue };
            if flags & WAVE_PREVIEW != 0 {
                s.rack.wave_preview.insert(id);
            }
            if flags & MIDI_THROUGH == 0 {
                s.rack.through_off.insert(id);
            }
        }
    }
    #[cfg(any(windows, target_os = "macos"))]
    if let Some(&on) = read(lemon::KEY_FOCUS).as_deref().and_then(|d| d.get(1)) {
        set_editor_keys(on != 0);
    }
    if let Some(d) = read(lemon::TRACK_LOCKS) {
        let mut r = Reader(&d[1..]);
        for _ in 0..r.u32().unwrap_or(0) {
            let (Some(a), Some(t), Some(kind), Some(v)) = (r.u16(), r.u16(), r.u8(), r.u16()) else { break };
            let src = match kind {
                PATTERN => os_model::ClipSource::Pattern(v),
                CLIP_CHANNEL => os_model::ClipSource::Channel(v),
                _ => continue,
            };
            if let Some(tr) = s.project.arrangements.get_mut(usize::from(a)).and_then(|a| a.tracks.get_mut(usize::from(t))) {
                tr.clip_lock = Some(src);
            }
        }
    }
    if let Some(d) = read(lemon::NOTE_LOCKS) {
        let mut r = Reader(&d[1..]);
        for _ in 0..r.u32().unwrap_or(0) {
            let (Some(src), Some(row)) = (r.u8(), r.u16()) else { break };
            let row = usize::from(row);
            if let Some(lock) = s.rack.note_lock.get_mut(usize::from(src)).filter(|_| row < s.project.channels.len()) {
                *lock = Some(row);
            }
        }
    }
    if let Some(d) = read(lemon::ICONS) {
        read_icons(s, &d[1..]);
    }
    if let Some(d) = read(lemon::SLOT_NAMES) {
        let mut r = Reader(&d[1..]);
        for _ in 0..r.u32().unwrap_or(0) {
            let (Some(t), Some(k), Some(len)) = (r.u16(), r.u8(), r.u16()) else { break };
            let Some(text) = r.take(usize::from(len)) else { break };
            s.mixer.slot_names.insert((t, k), String::from_utf8_lossy(text).into_owned());
        }
    }
    if let Some(d) = read(lemon::SLOT_COLORS) {
        let mut r = Reader(&d[1..]);
        for _ in 0..r.u32().unwrap_or(0) {
            let (Some(t), Some(k), Some(c)) = (r.u16(), r.u8(), r.take(3)) else { break };
            s.mixer.slot_colors.insert((t, k), os_model::Rgb::new(c[0], c[1], c[2]));
        }
    }
    if let Some(&flags) = read(lemon::PERFORMANCE).as_deref().and_then(|d| d.get(1)) {
        let pl = &mut s.playlist;
        pl.perf_clip_progress = flags & CLIP_PROGRESS != 0;
        pl.perf_track_progress = flags & TRACK_PROGRESS != 0;
        pl.perf_plus_scene = flags & PLUS_SCENE != 0;
        pl.perf_velocity_subclips = flags & VELOCITY_SUBCLIPS != 0;
    }
}

/// The icons of an opened FLP: each channel's and mixer track's icon number (events 155 and 95) that has a picture in
/// the Change icon... list ([`crate::playlist::FLP_ICONS`]). A `.lemon` file's icons are its own (event 193), so a
/// choice made in Lemon Studio, None too, wins over the numbers its FLP events keep.
fn from_flp(s: &mut State) {
    // An FLP has no Linear mode of its own: the defaults (Classic, no tracks), as for every other kind of state.
    s.project.linear_mode = false;
    s.project.tracks.clear();
    let p = &s.project;
    let channels = p.channels.iter().filter_map(|c| Some((IconOwner::Channel(c.id.0), crate::playlist::flp_icon(c.rack.icon)?)));
    let tracks = p.mixer.iter().filter_map(|m| Some((IconOwner::Track(m.index), crate::playlist::flp_icon(u32::from(m.icon?))?)));
    let found: Vec<_> = channels.chain(tracks).collect();
    s.playlist.icons.extend(found);
}

/// [`lemon::ICONS`]: a u32 count, then per icon its owner's kind (u8: 0 a channel, 1 a mixer track, 2 an effect slot),
/// number (u16: the channel's position in the rack, or the mixer track) and slot (u8, 0 unless an effect slot), and the
/// icon's name (u8 length, UTF-8; the names of the Change icon... menu). A channel is kept by its position, since the
/// file holds the channels in rack order and ids are not saved.
fn icons(s: &State) -> Event {
    let mut v: Vec<(u8, u16, u8, &str)> = Vec::new();
    for (owner, icon) in &s.playlist.icons {
        let Some(&(name, _)) = ICONS.iter().find(|(_, i)| i == icon) else { continue };
        let at = match *owner {
            IconOwner::Channel(id) => match s.project.channels.iter().position(|c| c.id.0 == id) {
                Some(i) => (CHANNEL, i as u16, 0),
                None => continue,
            },
            IconOwner::Track(t) => (TRACK, t, 0),
            IconOwner::Slot(t, k) => (SLOT, t, k),
        };
        v.push((at.0, at.1, at.2, name));
    }
    v.sort();
    let mut b = start(v.len());
    for (kind, n, k, name) in v {
        b.push(kind);
        b.extend_from_slice(&n.to_le_bytes());
        b.push(k);
        b.push(name.len() as u8);
        b.extend_from_slice(name.as_bytes());
    }
    Event::new(lemon::ICONS, Value::var(b))
}

fn read_icons(s: &mut State, d: &[u8]) {
    let mut r = Reader(d);
    for _ in 0..r.u32().unwrap_or(0) {
        let (Some(kind), Some(n), Some(k), Some(len)) = (r.u8(), r.u16(), r.u8(), r.u8()) else { break };
        let Some(name) = r.take(usize::from(len)) else { break };
        // A name this version does not know (one added later) is left out.
        let Some(&(_, icon)) = ICONS.iter().find(|(nm, _)| nm.as_bytes() == name) else { continue };
        let owner = match kind {
            CHANNEL => match s.project.channels.get(usize::from(n)) {
                Some(c) => IconOwner::Channel(c.id.0),
                None => continue,
            },
            TRACK => IconOwner::Track(n),
            SLOT => IconOwner::Slot(n, k),
            _ => continue,
        };
        s.playlist.icons.insert(owner, icon);
    }
}

/// [`lemon::SLOT_NAMES`]: a u32 count, then per slot its mixer track (u16), slot (u8) and name (u16 length, UTF-8).
fn slot_names(s: &State) -> Event {
    let mut v: Vec<_> = s.mixer.slot_names.iter().collect();
    v.sort();
    let mut b = start(v.len());
    for (&(t, k), name) in v {
        let name = &name.as_bytes()[..name.len().min(usize::from(u16::MAX))];
        b.extend_from_slice(&t.to_le_bytes());
        b.push(k);
        b.extend_from_slice(&(name.len() as u16).to_le_bytes());
        b.extend_from_slice(name);
    }
    Event::new(lemon::SLOT_NAMES, Value::var(b))
}

/// [`lemon::SLOT_COLORS`]: a u32 count, then per slot its mixer track (u16), slot (u8) and colour (u8 red, green, blue).
fn slot_colors(s: &State) -> Event {
    let mut v: Vec<_> = s.mixer.slot_colors.iter().collect();
    v.sort_by_key(|(k, _)| **k);
    let mut b = start(v.len());
    for (&(t, k), c) in v {
        b.extend_from_slice(&t.to_le_bytes());
        b.extend_from_slice(&[k, c.r, c.g, c.b]);
    }
    Event::new(lemon::SLOT_COLORS, Value::var(b))
}

/// [`lemon::PERFORMANCE`]: one u8 of flags: 1 Performance clip progress, 2 Performance track progress, 4 Ctrl+click
/// triggers +Scene, 8 Velocity picks sub-clips.
fn performance(s: &State) -> Event {
    let pl = &s.playlist;
    let flags = [
        (pl.perf_clip_progress, CLIP_PROGRESS),
        (pl.perf_track_progress, TRACK_PROGRESS),
        (pl.perf_plus_scene, PLUS_SCENE),
        (pl.perf_velocity_subclips, VELOCITY_SUBCLIPS),
    ]
    .iter()
    .filter(|(on, _)| *on)
    .fold(0, |f, (_, bit)| f | bit);
    Event::new(lemon::PERFORMANCE, Value::var([LAYOUT, flags]))
}

/// [`lemon::KEY_FOCUS`]: Set keyboard focus of all plugins to ▸ (`plugin_window::editor_keys`). The switch is one for
/// the whole program, so tests keep their own, one per test thread.
#[cfg(any(windows, target_os = "macos"))]
fn editor_keys() -> bool {
    #[cfg(test)]
    return tests::KEYS.with(|k| k.get());
    #[cfg(not(test))]
    crate::plugin_window::editor_keys()
}

#[cfg(any(windows, target_os = "macos"))]
fn set_editor_keys(on: bool) {
    #[cfg(test)]
    tests::KEYS.with(|k| k.set(on));
    #[cfg(not(test))]
    crate::plugin_window::set_editor_keys(on);
}

/// [`lemon::MIXER`]: one u8 of flags: 1 Plugin delay compensation ▸ Automatic, 2 View ▸ Waveforms, 4 View ▸ Extra
/// properties.
fn mixer(s: &State) -> Event {
    let flags = [(!s.project.pdc_off, PDC_AUTO), (s.mixer.waveforms, WAVEFORMS), (s.mixer.extra, EXTRA_PROPS)];
    Event::new(lemon::MIXER, Value::var([LAYOUT, flags.iter().filter(|(on, _)| *on).fold(0, |f, (_, bit)| f | bit)]))
}

/// [`lemon::SCALING`]: a u32 count, then per plugin window its owner's kind (u8: 0 a channel, 2 an effect slot, as in
/// [`lemon::ICONS`]), number (u16: the channel's rack position, or the mixer track), slot (u8, 0 for a channel) and
/// scale (u16 percent). Windows at Main are not listed; a container's members are not kept.
fn scaling(s: &State, plugins: &Plugins) -> Event {
    let mut v = Vec::new();
    for (&key, &k) in &plugins.editor_scales {
        let pct = (k * 100.0).round() as u16;
        match crate::plugins::fx_of(key) {
            Some((t, slot)) => v.push((SLOT, t, slot, pct)),
            None if key < s.project.channels.len() => v.push((CHANNEL, key as u16, 0, pct)),
            None => {}
        }
    }
    v.sort();
    let mut b = start(v.len());
    for (kind, n, slot, pct) in v {
        b.push(kind);
        b.extend_from_slice(&n.to_le_bytes());
        b.push(slot);
        b.extend_from_slice(&pct.to_le_bytes());
    }
    Event::new(lemon::SCALING, Value::var(b))
}

/// [`lemon::CHANNEL_CHECKS`]: a u32 count, then per channel in rack order its position (u16) and flags (u8: 1 Show
/// waveform preview, 2 MIDI channel through).
fn channel_checks(s: &State) -> Event {
    let mut b = start(s.project.channels.len());
    for (i, c) in s.project.channels.iter().enumerate() {
        let mut flags = 0;
        if s.rack.wave_preview.contains(&c.id.0) {
            flags |= WAVE_PREVIEW;
        }
        if !s.rack.through_off.contains(&c.id.0) {
            flags |= MIDI_THROUGH;
        }
        b.extend_from_slice(&(i as u16).to_le_bytes());
        b.push(flags);
    }
    Event::new(lemon::CHANNEL_CHECKS, Value::var(b))
}

/// [`lemon::TRACK_LOCKS`]: a u32 count, then per locked playlist track its arrangement (u16, 0-based), track (u16,
/// 0-based) and source (u8: 0 a pattern, 1 an audio or automation channel; u16: the 1-based pattern number or the
/// channel's rack position).
fn track_locks(s: &State) -> Event {
    let mut v = Vec::new();
    for (a, arr) in s.project.arrangements.iter().enumerate() {
        for (t, tr) in arr.tracks.iter().enumerate() {
            match tr.clip_lock {
                Some(os_model::ClipSource::Pattern(p)) => v.push((a as u16, t as u16, PATTERN, p)),
                Some(os_model::ClipSource::Channel(c)) => v.push((a as u16, t as u16, CLIP_CHANNEL, c)),
                None => {}
            }
        }
    }
    let mut b = start(v.len());
    for (a, t, kind, n) in v {
        b.extend_from_slice(&a.to_le_bytes());
        b.extend_from_slice(&t.to_le_bytes());
        b.push(kind);
        b.extend_from_slice(&n.to_le_bytes());
    }
    Event::new(lemon::TRACK_LOCKS, Value::var(b))
}

/// [`lemon::NOTE_LOCKS`]: a u32 count, then per locked note source its kind (u8: 0 the computer keyboard, 1 the Touch
/// controller, as `rack::NoteSrc` orders them) and the channel's rack position (u16).
fn note_locks(s: &State) -> Event {
    let locks: Vec<(u8, u16)> = s.rack.note_lock.iter().enumerate().filter_map(|(src, row)| Some((src as u8, (*row)? as u16))).collect();
    let mut b = start(locks.len());
    for (src, row) in locks {
        b.push(src);
        b.extend_from_slice(&row.to_le_bytes());
    }
    Event::new(lemon::NOTE_LOCKS, Value::var(b))
}

/// Reads [`lemon::TRACKS`]'s payload after its layout byte. A cut entry and the ones after it are left out; an entry
/// whose kind byte this version does not know is left out too.
fn read_tracks(d: &[u8]) -> Vec<os_model::Track> {
    const NO_CHANNEL: u16 = 0xFFFF;
    let mut r = Reader(d);
    let mut v = Vec::new();
    for _ in 0..r.u32().unwrap_or(0) {
        let (Some(kind), Some(lane), Some(strip), Some(ch)) = (r.u8(), r.u32(), r.u16(), r.u16()) else { break };
        let Some(kind) = (match kind {
            0 => Some(os_model::TrackKind::Audio),
            1 => Some(os_model::TrackKind::Midi),
            2 => Some(os_model::TrackKind::Bus),
            _ => None,
        }) else {
            continue;
        };
        v.push(os_model::Track::new(kind, lane, strip, (ch != NO_CHANNEL).then_some(ch)));
    }
    v
}

/// [`lemon::TRACKS`]: a u32 count, then per linear mode track (in [`os_model::Project::tracks`] order) its kind (u8:
/// 0 audio, 1 midi, 2 bus), playlist lane (u32, 1-based [`os_model::PlaylistTrack::number`], 0 on a bus), mixer strip
/// (u16) and channel (u16, 0xFFFF = none). Ids are not saved (project files store none); `settle` gives new ones.
fn linear_tracks(s: &State) -> Event {
    const NO_CHANNEL: u16 = 0xFFFF;
    let mut b = start(s.project.tracks.len());
    for t in &s.project.tracks {
        b.push(t.kind as u8);
        b.extend_from_slice(&t.playlist_lane.to_le_bytes());
        b.extend_from_slice(&t.mixer_strip.to_le_bytes());
        b.extend_from_slice(&t.channel.unwrap_or(NO_CHANNEL).to_le_bytes());
    }
    Event::new(lemon::TRACKS, Value::var(b))
}

/// [`lemon::LINEAR_MODE`]: one u8, 1 = Linear mode.
fn linear_mode(s: &State) -> Event {
    Event::new(lemon::LINEAR_MODE, Value::var([LAYOUT, u8::from(s.project.linear_mode)]))
}

/// Reads [`lemon::LANE_BINDINGS`]'s payload after its layout byte, resolving each lane's track position against the
/// just-read [`os_model::Project::tracks`] (the layout check: a lane whose track is gone, or a target bound twice, is
/// left out).
fn read_lane_bindings(d: &[u8], tracks: &[os_model::Track]) -> Vec<os_model::AutoLane> {
    let mut r = Reader(d);
    let mut v = Vec::new();
    for _ in 0..r.u32().unwrap_or(0) {
        let (Some(at), Some(target), Some(collapsed)) = (r.u32(), r.u32(), r.u8()) else { break };
        let Some(track) = u16::try_from(at).ok().filter(|&at| usize::from(at) < tracks.len()) else { continue };
        if v.iter().any(|l: &os_model::AutoLane| l.target == target) {
            continue;
        }
        v.push(os_model::AutoLane { track, target, collapsed: collapsed != 0 });
    }
    v
}

/// [`lemon::LANE_BINDINGS`]: a u32 count, then per automation lane (in [`os_model::Project::auto_lanes`] order) the
/// owning track's position in [`os_model::Project::tracks`] (u32), the bound target (u32) and the collapsed flag (u8).
/// The track is stored by its position, the order the `TRACKS` event restores it in; the apply resolves it to the
/// track (a lane whose track is gone is dropped, the layout check).
fn lane_bindings(s: &State) -> Event {
    let mut b = start(s.project.auto_lanes.len());
    for l in &s.project.auto_lanes {
        b.extend_from_slice(&u32::from(l.track).to_le_bytes());
        b.extend_from_slice(&l.target.to_le_bytes());
        b.push(u8::from(l.collapsed));
    }
    Event::new(lemon::LANE_BINDINGS, Value::var(b))
}

/// A payload's layout version and entry count.
fn start(count: usize) -> Vec<u8> {
    let mut b = vec![LAYOUT];
    b.extend_from_slice(&(count as u32).to_le_bytes());
    b
}

/// Reads a payload front to back; `None` once it runs out (a cut entry and the ones after it are left out).
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, rest) = self.0.split_at_checked(n)?;
        self.0 = rest;
        Some(head)
    }

    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }

    fn u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Option<u64> {
        self.take(8).map(|b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }

    fn f32(&mut self) -> Option<f32> {
        self.take(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::icons::Icon;

    std::thread_local! {
        /// This test thread's Set keyboard focus of all plugins to ▸ (`editor_keys`).
        pub(super) static KEYS: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    }

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("os_lemon_state_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// An app with every kind of state set: icons on the second channel, mixer track 2 and its slot 1; slot 1's name
    /// and slot 0's colour on track 2; Performance settings other than the defaults.
    fn with_state() -> App {
        let mut app = App::new(None);
        app.state.project = crate::app::test_project();
        app.state.control.settle(&mut app.state.project);
        let ch = app.state.project.channels[1].id.0;
        let s = &mut app.state;
        s.playlist.icons.insert(IconOwner::Channel(ch), Icon::Mic);
        s.playlist.icons.insert(IconOwner::Track(2), Icon::Star);
        s.playlist.icons.insert(IconOwner::Slot(2, 1), Icon::Plug);
        s.mixer.slot_names.insert((2, 1), "Crunch ✓".into());
        s.mixer.slot_colors.insert((2, 0), os_model::Rgb::new(10, 200, 30));
        s.playlist.perf_clip_progress = false;
        s.playlist.perf_track_progress = true;
        s.playlist.perf_plus_scene = true;
        s.playlist.perf_velocity_subclips = false;
        app
    }

    /// Saved to a .lemon file and opened again, each kind of state comes back: the channel's icon on the channel at
    /// the same rack position (ids are new), the track's and the slot's, the slot's name and colour, the Performance
    /// settings. Changing one of them is an unsaved change.
    #[test]
    fn state_round_trips_through_a_lemon_file() {
        let d = dir("round");
        let path = d.join("State.lemon");
        let mut app = with_state();
        app.save_to(path.clone());
        assert!(!app.unsaved());
        let file = os_flp::FlpFile::parse(&std::fs::read(&path).unwrap()).unwrap();
        let ids: Vec<u8> = lemon::events(&file).unwrap().iter().map(|e| e.id).collect();
        let mut want = vec![lemon::SAVED_BY, lemon::ICONS, lemon::SLOT_NAMES, lemon::SLOT_COLORS, lemon::PERFORMANCE, lemon::MIXER];
        want.extend([lemon::SCALING, lemon::CHANNEL_CHECKS]);
        #[cfg(any(windows, target_os = "macos"))]
        want.push(lemon::KEY_FOCUS);
        want.extend([lemon::TRACK_LOCKS, lemon::NOTE_LOCKS, lemon::TRACKS, lemon::LINEAR_MODE]);
        // The modulation identity registry (M0) and its graph (M1): a state with devices in it has owners, and
        // both are written (an empty graph too, so a saved file holds no stale one).
        want.push(lemon::OWNER_IDS);
        want.push(lemon::MODULATION);
        assert_eq!(ids, want);
        let mut back = App::new(None);
        back.open(path.clone());
        assert!(!back.unsaved(), "the state read back is the saved state");
        let s = &back.state;
        let ch = s.project.channels[1].id.0;
        assert_eq!(s.playlist.icons.get(&IconOwner::Channel(ch)), Some(&Icon::Mic));
        assert_eq!(s.playlist.icons.get(&IconOwner::Track(2)), Some(&Icon::Star));
        assert_eq!(s.playlist.icons.get(&IconOwner::Slot(2, 1)), Some(&Icon::Plug));
        assert_eq!(s.playlist.icons.len(), 3);
        assert_eq!(s.mixer.slot_names.get(&(2, 1)).map(String::as_str), Some("Crunch ✓"));
        assert_eq!(s.mixer.slot_colors.get(&(2, 0)), Some(&os_model::Rgb::new(10, 200, 30)));
        let pl = &s.playlist;
        assert_eq!((pl.perf_clip_progress, pl.perf_track_progress, pl.perf_plus_scene, pl.perf_velocity_subclips), (false, true, true, false));
        back.state.playlist.icons.remove(&IconOwner::Track(2));
        assert!(back.unsaved(), "an icon taken off is an unsaved change");
        // Saved again, what was taken off stays off (the events are replaced, not kept from the opened file).
        back.save_to(path.clone());
        let mut again = App::new(None);
        again.open(path);
        assert_eq!(again.state.playlist.icons.len(), 2);
        assert!(!again.state.playlist.icons.contains_key(&IconOwner::Track(2)));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// `with_state`, plus the settings of events 197-201: Plugin delay compensation ▸ Automatic off, View ▸ Waveforms
    /// on and Extra properties off; Scaling ▸ 150 % on channel 0's window and 125 % on track 3's slot 2; Show waveform
    /// preview on channel 1 and MIDI channel through off on channel 2; tracks 3 and 4 of the first arrangement locked to
    /// pattern 1 and to channel 0.
    fn with_settings() -> App {
        let mut app = with_state();
        let s = &mut app.state;
        s.project.pdc_off = true;
        s.mixer.waveforms = true;
        s.mixer.extra = false;
        s.rack.wave_preview.insert(s.project.channels[1].id.0);
        s.rack.through_off.insert(s.project.channels[2].id.0);
        if s.project.arrangements.is_empty() {
            s.project.arrangements.push(os_flp::project::default_arrangement());
        }
        let tracks = &mut s.project.arrangements[0].tracks;
        tracks[3].clip_lock = Some(os_model::ClipSource::Pattern(1));
        tracks[4].clip_lock = Some(os_model::ClipSource::Channel(0));
        app.plugins.editor_scales.insert(0, 1.5);
        app.plugins.editor_scales.insert(crate::plugins::fx_key(3, 2), 1.25);
        app
    }

    /// Saved to a .lemon file and opened again, the settings of events 197-201 come back (the checks on the channels
    /// at the same rack positions), Set keyboard focus of all plugins to ▸ too; a project opened from an FLP has the
    /// defaults (and a Scaling ▸ of the project opened before is gone). A changed Scaling ▸ is an unsaved change; the
    /// keyboard focus switch is not.
    #[test]
    fn settings_round_trip_through_a_lemon_file() {
        let d = dir("settings");
        let (lemon_path, flp_path) = (d.join("Settings.lemon"), d.join("Settings.flp"));
        let mut app = with_settings();
        #[cfg(any(windows, target_os = "macos"))]
        set_editor_keys(false);
        app.save_to(lemon_path.clone());
        app.save_to(flp_path.clone());
        assert!(app.state.message.as_deref().is_some_and(|m| m.starts_with("Saved")), "{:?}", app.state.message);
        #[cfg(any(windows, target_os = "macos"))]
        set_editor_keys(true);
        let mut back = App::new(None);
        back.open(lemon_path.clone());
        #[cfg(any(windows, target_os = "macos"))]
        assert!(!editor_keys(), "Disabled comes back");
        assert!(!back.unsaved());
        let s = &back.state;
        assert!(s.project.pdc_off && s.mixer.waveforms && !s.mixer.extra);
        let ids: Vec<u32> = s.project.channels.iter().map(|c| c.id.0).collect();
        assert_eq!(s.rack.wave_preview.iter().copied().collect::<Vec<_>>(), [ids[1]]);
        assert_eq!(s.rack.through_off.iter().copied().collect::<Vec<_>>(), [ids[2]]);
        let locks: Vec<_> = s.project.arrangements[0].tracks.iter().enumerate().filter_map(|(t, tr)| Some((t, tr.clip_lock?))).collect();
        assert_eq!(locks, [(3, os_model::ClipSource::Pattern(1)), (4, os_model::ClipSource::Channel(0))]);
        let scales: Vec<(usize, f32)> = back.plugins.editor_scales.iter().map(|(&k, &v)| (k, v)).collect();
        assert_eq!(scales, [(0, 1.5), (crate::plugins::fx_key(3, 2), 1.25)]);
        back.plugins.editor_scales.remove(&0);
        assert!(back.unsaved(), "a Scaling ▸ change is an unsaved change");
        #[cfg(any(windows, target_os = "macos"))]
        {
            back.plugins.editor_scales.insert(0, 1.5);
            set_editor_keys(true);
            assert!(!back.unsaved(), "the keyboard focus switch is not");
        }
        // The FLP saved from the same state: no LMex, so the defaults (opened over the .lemon project).
        back.open(flp_path);
        let s = &back.state;
        assert!(!s.project.pdc_off && !s.mixer.waveforms && s.mixer.extra);
        assert!(s.rack.wave_preview.is_empty() && s.rack.through_off.is_empty() && back.plugins.editor_scales.is_empty());
        assert!(s.project.arrangements.iter().flat_map(|a| &a.tracks).all(|t| t.clip_lock.is_none()));
        #[cfg(any(windows, target_os = "macos"))]
        assert!(editor_keys());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The linear mode's tracks (event 203) and mode flag (204) come back through a .lemon file (ids are not saved, so
    /// they are new); an FLP saved from the same state has none of them.
    #[test]
    fn linear_mode_and_its_tracks_round_trip_through_a_lemon_file() {
        let d = dir("linear");
        let (lemon_path, flp_path) = (d.join("Linear.lemon"), d.join("Linear.flp"));
        let mut app = with_state();
        app.state.project.linear_mode = true;
        app.state.project.tracks.push(os_model::Track::new(os_model::TrackKind::Midi, 3, 5, Some(1)));
        app.state.project.tracks.push(os_model::Track::new(os_model::TrackKind::Bus, 0, 6, None));
        app.save_to(lemon_path.clone());
        app.save_to(flp_path.clone());
        let file = os_flp::FlpFile::parse(&std::fs::read(&lemon_path).unwrap()).unwrap();
        let midi = [1u8, 3, 0, 0, 0, 5, 0, 1, 0];
        let bus = [2u8, 0, 0, 0, 0, 6, 0, 255, 255];
        let mut want = vec![LAYOUT, 2, 0, 0, 0];
        want.extend_from_slice(&midi);
        want.extend_from_slice(&bus);
        assert_eq!(lemon::data(&file, lemon::TRACKS), Some(want));
        assert_eq!(lemon::data(&file, lemon::LINEAR_MODE), Some(vec![LAYOUT, 1]));
        let mut back = App::new(None);
        back.open(lemon_path.clone());
        assert!(!back.unsaved(), "the tracks read back are the saved ones");
        let s = &back.state;
        assert!(s.project.linear_mode);
        assert_eq!(s.project.tracks.len(), 2);
        assert_eq!(
            (s.project.tracks[0].kind, s.project.tracks[0].playlist_lane, s.project.tracks[0].mixer_strip, s.project.tracks[0].channel),
            (os_model::TrackKind::Midi, 3, 5, Some(1))
        );
        assert_eq!((s.project.tracks[1].kind, s.project.tracks[1].playlist_lane), (os_model::TrackKind::Bus, 0));
        // The FLP saved from the same state: no tracks and no Linear mode (an FLP has no place for them).
        back.open(flp_path);
        let s = &back.state;
        assert!(!s.project.linear_mode && s.project.tracks.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// The automation lanes (event 205, spec: model/track-model.md §6b) come back through a .lemon file, their
    /// targets and collapsed flags with them; a lane whose track is gone (or a target bound twice) is left out; an
    /// FLP saved from the same state has none.
    #[test]
    fn lane_bindings_round_trip_through_a_lemon_file() {
        let d = dir("lanes");
        let (lemon_path, flp_path) = (d.join("Lanes.lemon"), d.join("Lanes.flp"));
        let mut app = with_state();
        app.state.project.linear_mode = true;
        app.state.project.tracks.push(os_model::Track::new(os_model::TrackKind::Midi, 1, 5, Some(0)));
        let tgt = os_model::target::channel(0, os_model::target::CHANNEL_VOLUME);
        let gone = os_model::target::mixer(9, os_model::target::MIXER_VOLUME);
        app.state.project.auto_lanes = vec![
            os_model::AutoLane { track: 0, target: tgt, collapsed: false },
            os_model::AutoLane { track: 0, target: gone, collapsed: true },
            os_model::AutoLane { track: 7, target: os_model::target::TEMPO, collapsed: false },
        ];
        app.save_to(lemon_path.clone());
        app.save_to(flp_path.clone());
        let file = os_flp::FlpFile::parse(&std::fs::read(&lemon_path).unwrap()).unwrap();
        let mut want = vec![LAYOUT, 3, 0, 0, 0];
        for (track, target, collapsed) in [(0u32, tgt, 0u8), (0, gone, 1), (7, os_model::target::TEMPO, 0)] {
            want.extend_from_slice(&track.to_le_bytes());
            want.extend_from_slice(&target.to_le_bytes());
            want.push(collapsed);
        }
        assert_eq!(lemon::data(&file, lemon::LANE_BINDINGS), Some(want));
        let mut back = App::new(None);
        back.open(lemon_path.clone());
        assert!(!back.unsaved(), "the lanes read back are the saved ones");
        // The lane whose track is gone is left out (the layout check).
        assert_eq!(
            back.state.project.auto_lanes,
            vec![os_model::AutoLane { track: 0, target: tgt, collapsed: false }, os_model::AutoLane { track: 0, target: gone, collapsed: true }]
        );
        // The FLP saved from the same state: no lanes (an FLP has no place for them).
        back.open(flp_path);
        assert!(back.state.project.auto_lanes.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Receive notes from ▸ locks (event 202) come back on the channels at the same rack positions; one taken off is an
    /// unsaved change and stays off once saved; an FLP saved from the same state opens with none.
    #[test]
    fn note_locks_round_trip_through_a_lemon_file() {
        use crate::rack::NoteSrc;
        let d = dir("note_locks");
        let (lemon_path, flp_path) = (d.join("Locks.lemon"), d.join("Locks.flp"));
        let mut app = with_state();
        app.state.rack.note_lock[NoteSrc::Typing as usize] = Some(3);
        app.state.rack.note_lock[NoteSrc::Touch as usize] = Some(1);
        app.save_to(lemon_path.clone());
        app.save_to(flp_path.clone());
        let file = os_flp::FlpFile::parse(&std::fs::read(&lemon_path).unwrap()).unwrap();
        assert_eq!(lemon::data(&file, lemon::NOTE_LOCKS), Some(vec![LAYOUT, 2, 0, 0, 0, 0, 3, 0, 1, 1, 0]));
        let mut back = App::new(None);
        back.open(lemon_path.clone());
        assert!(!back.unsaved());
        assert_eq!(back.state.rack.note_lock, [Some(3), Some(1)]);
        back.state.rack.note_lock[NoteSrc::Typing as usize] = None;
        assert!(back.unsaved(), "an unlock is an unsaved change");
        back.save_to(lemon_path.clone());
        let mut again = App::new(None);
        again.open(lemon_path);
        assert_eq!(again.state.rack.note_lock, [None, Some(1)]);
        // A lock on a channel the file does not have is left out.
        let mut f = file.clone();
        lemon::to_lemon(&mut f, &[Event::new(lemon::NOTE_LOCKS, Value::var([LAYOUT, 1, 0, 0, 0, 0, 99, 0]))]).unwrap();
        let mut s = with_state();
        apply(&mut s.state, &mut s.plugins, &f);
        assert_eq!(s.state.rack.note_lock, [None, None]);
        again.open(flp_path);
        assert_eq!(again.state.rack.note_lock, [None, None], "an FLP has no locks");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// An FLP's icon numbers (a channel's event 155, a mixer track's event 95) open as the icons with the same picture
    /// (`playlist::FLP_ICONS`), other numbers as none. Saved as .lemon, the FLP events keep the file's numbers and the
    /// icons come back from event 193, so one taken off in Lemon Studio stays off.
    #[test]
    fn flp_icon_numbers_open_as_icons() {
        let d = dir("flp_icons");
        let path = d.join("Icons.flp");
        let mut made = App::new(None);
        made.state.project = crate::app::test_project();
        made.state.control.settle(&mut made.state.project);
        let m = &mut made.state.project.mixer;
        m[2].icon = Some(75);
        m[3].icon = Some(74); // the automation clip's nodes: no picture of Lemon Studio's
        m[4].icon = Some(80);
        made.save_to(path.clone());
        let mut file = os_flp::FlpFile::parse(&std::fs::read(&path).unwrap()).unwrap();
        // The second channel's icon number, before its colour: 30, the microphone.
        let ev = file.chunks.iter_mut().find_map(|c| match &mut c.body {
            os_flp::ChunkBody::Events(v) => Some(v),
            _ => None,
        });
        let ev = ev.unwrap();
        let second = ev.iter().enumerate().filter(|(_, e)| e.id == 64).nth(1).unwrap().0;
        let color = second + ev[second..].iter().position(|e| e.id == 128).unwrap();
        ev.insert(color, Event::new(155, Value::U32(30)));
        std::fs::write(&path, file.to_bytes().unwrap()).unwrap();
        let mut app = App::new(None);
        app.open(path);
        let s = &app.state;
        let (first, second) = (s.project.channels[0].id.0, s.project.channels[1].id.0);
        assert_eq!(s.project.channels[1].rack.icon, 30);
        assert_eq!(s.playlist.icons.get(&IconOwner::Channel(second)), Some(&Icon::Mic));
        assert_eq!(s.playlist.icons.get(&IconOwner::Channel(first)), None);
        assert_eq!(s.playlist.icons.get(&IconOwner::Track(2)), Some(&Icon::Wave));
        assert_eq!(s.playlist.icons.get(&IconOwner::Track(4)), Some(&Icon::Kick));
        assert_eq!(s.playlist.icons.len(), 3, "74 has no picture here");
        assert_eq!(crate::playlist::window_icon(&s.project, &s.playlist.icons, 1), Some(Icon::Mic));
        assert!(!app.unsaved(), "the icons are the opened state");
        // Saved as .lemon with the track 2 icon taken off: the FLP events keep the original numbers.
        app.state.playlist.icons.remove(&IconOwner::Track(2));
        let lemon_path = d.join("Icons.lemon");
        app.save_to(lemon_path.clone());
        let saved = os_flp::project::load(&os_flp::FlpFile::parse(&std::fs::read(&lemon_path).unwrap()).unwrap()).unwrap();
        assert_eq!((saved.channels[1].rack.icon, saved.mixer[2].icon, saved.mixer[3].icon), (30, Some(75), Some(74)));
        let mut back = App::new(None);
        back.open(lemon_path);
        let s = &back.state;
        assert_eq!(s.playlist.icons.get(&IconOwner::Channel(s.project.channels[1].id.0)), Some(&Icon::Mic));
        assert_eq!(s.playlist.icons.get(&IconOwner::Track(2)), None, "event 193 holds the choice, not event 95");
        assert_eq!(s.playlist.icons.len(), 2);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A target named .flp has no place for the state: the save succeeds, the file is a plain FLP, and opening it gives
    /// the defaults.
    #[test]
    fn an_flp_target_drops_the_state() {
        let d = dir("flp");
        let path = d.join("State.flp");
        let mut app = with_state();
        app.save_to(path.clone());
        assert!(app.state.message.as_deref().is_some_and(|m| m.starts_with("Saved")), "{:?}", app.state.message);
        let file = os_flp::FlpFile::parse(&std::fs::read(&path).unwrap()).unwrap();
        assert!(!lemon::is_lemon(&file) && lemon::events(&file).unwrap().is_empty());
        let mut back = App::new(None);
        back.open(path);
        let s = &back.state;
        assert!(s.playlist.icons.is_empty() && s.mixer.slot_names.is_empty() && s.mixer.slot_colors.is_empty());
        let fresh = crate::playlist::PlaylistState::default();
        assert_eq!((s.playlist.perf_clip_progress, s.playlist.perf_velocity_subclips), (fresh.perf_clip_progress, fresh.perf_velocity_subclips));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// An `LMex` event this version does not know is kept through open and save; one of ours with a later layout is
    /// left unread, and a payload cut short keeps the entries before the cut.
    #[test]
    fn unknown_events_and_layouts() {
        let d = dir("unknown");
        let path = d.join("State.lemon");
        let mut app = with_state();
        app.save_to(path.clone());
        let mut file = os_flp::FlpFile::parse(&std::fs::read(&path).unwrap()).unwrap();
        let future = Event::new(240, Value::var([7, 7]));
        // A layout after this version's is left unread.
        let later = Event::new(lemon::PERFORMANCE, Value::var([LAYOUT + 1, 0]));
        lemon::to_lemon(&mut file, &[future.clone(), later]).unwrap();
        std::fs::write(&path, file.to_bytes().unwrap()).unwrap();
        let mut back = App::new(None);
        back.open(path.clone());
        assert!(back.state.playlist.perf_clip_progress, "a layout 2 Performance event is not read: the defaults stay");
        assert_eq!(back.state.playlist.icons.len(), 3);
        back.save_to(path.clone());
        let file = os_flp::FlpFile::parse(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(lemon::data(&file, 240), Some(vec![7, 7]), "the unknown event is kept");
        // Two colours, the second cut short: the first is read.
        let mut s = with_state();
        s.state.mixer.slot_colors.insert((3, 4), os_model::Rgb::new(1, 2, 3));
        let mut b = slot_colors(&s.state).data().unwrap().to_vec();
        b.truncate(b.len() - 1);
        s.state.mixer.slot_colors.clear();
        let mut f = file.clone();
        lemon::to_lemon(&mut f, &[Event::new(lemon::SLOT_COLORS, Value::var(b))]).unwrap();
        apply(&mut s.state, &mut s.plugins, &f);
        assert_eq!(s.state.mixer.slot_colors.len(), 1);
        assert_eq!(s.state.mixer.slot_colors.get(&(2, 0)), Some(&os_model::Rgb::new(10, 200, 30)));
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod modulation_identity_tests {
    use super::*;
    use os_model::modulation::{IdentityRegistry, OwnerLocator, PersistentObjectId};
    use std::collections::BTreeMap;

    #[test]
    fn identity_registry_round_trips_through_lemon_event() {
        // M0 persistence (K.3): the owner/parameter identity survives a `.lemon` save and load.
        let mut reg = IdentityRegistry::default();
        reg.next = 5;
        reg.owners.insert(PersistentObjectId(1), OwnerLocator::Channel(3));
        reg.owners.insert(PersistentObjectId(2), OwnerLocator::Slot(2, 0));
        reg.owners.insert(PersistentObjectId(3), OwnerLocator::Member(2, 0, 1));
        reg.owners.insert(PersistentObjectId(4), OwnerLocator::Track(5));
        let mut nb: BTreeMap<PersistentObjectId, BTreeMap<u16, u32>> = BTreeMap::new();
        let mut m = BTreeMap::new();
        m.insert(0u16, 100u32);
        nb.insert(PersistentObjectId(2), m);
        reg.native_bindings = nb;

        let bytes = write_identity(&reg);
        let back = read_identity(&bytes).expect("round trip");
        assert_eq!(back, reg);
    }

    #[test]
    fn identity_event_id_is_free_and_after_existing() {
        // 192–205 are taken by other Lemon Studio state (K.3); the identity registry uses a fresh id.
        assert!(lemon::OWNER_IDS > lemon::LANE_BINDINGS);
        assert_eq!(lemon::OWNER_IDS, 206);
    }

    #[test]
    fn identity_rejects_unknown_layout() {
        assert!(read_identity(&[0u8]).is_none());
        assert!(read_identity(&[]).is_none());
    }
}

#[cfg(test)]
mod modulation_graph_tests {
    use super::*;
    use crate::app::App;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("os_lemon_graph_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A graph with every kind of source and every kind of parameter: an LFO driving a host parameter, a step
    /// sequence driving a legacy positional parameter, and an envelope (off) driving a native plugin parameter.
    fn graph() -> ModGraph {
        let mut g = ModGraph::default();
        let lfo = g.add(ModSource::Lfo(Lfo { shape: LfoShape::Triangle, period: Timebase::Sync(16), phase: 0.25, ..Lfo::default() }));
        g.get_mut(lfo).unwrap().name = "Wobble".into();
        g.assign(
            lfo,
            ModTarget {
                path: ParameterPath { owner: PersistentObjectId(1), parameter: ParameterKey::HostParameterCode(0x0100) },
                depth: -0.5,
                curve: Curve::Exp,
                polarity: Polarity::Unipolar,
                enabled: true,
            },
        );
        let mut values = [0i8; 16];
        for (i, v) in values.iter_mut().enumerate() {
            *v = (i as i16 * 12 - 90) as i8;
        }
        let steps = g.add(ModSource::Steps(Steps { values, period: Timebase::Free(2.5), smooth: true }));
        g.assign(steps, ModTarget::new(ParameterPath { owner: PersistentObjectId(2), parameter: ParameterKey::LegacyPluginIndex(5) }));
        let env = g.add(ModSource::Envelope(Envelope { attack: 0.01, decay: 0.2, sustain: 0.5, release: 0.3 }));
        g.get_mut(env).unwrap().enabled = false;
        g.assign(env, ModTarget::new(ParameterPath { owner: PersistentObjectId(3), parameter: ParameterKey::PluginNativeId(42) }));
        // A second row of slots, so the round trips carry the strip's rows too.
        g.add_row();
        g
    }

    /// A layout-1 payload (before project slots, source amount/offset and per-routing enable) still loads: its
    /// modulators take slots by creation order, every routing is on, and the LFO gets the old defaults.
    #[test]
    fn a_layout_one_graph_still_loads() {
        // A payload written by layout 1: no slot byte, and the LFO without amount, offset or polarity.
        let mut b = vec![LAYOUT_1];
        b.extend_from_slice(&1u32.to_le_bytes()); // count
        b.extend_from_slice(&2u32.to_le_bytes()); // next
        b.extend_from_slice(&7u32.to_le_bytes()); // id
        b.push(1); // enabled
        b.extend_from_slice(&3u16.to_le_bytes());
        b.extend_from_slice(b"Old");
        b.push(LFO);
        b.push(1); // triangle
        write_timebase(&mut b, Timebase::Sync(4));
        b.extend_from_slice(&0.25f32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes()); // one target
        b.extend_from_slice(&9u32.to_le_bytes()); // owner
        b.push(HOST_CODE);
        b.extend_from_slice(&0x0100u16.to_le_bytes());
        b.extend_from_slice(&(-0.5f32).to_le_bytes());
        b.push(0); // curve
        b.push(1); // unipolar
        let g = read_graph(&b).expect("layout 1 reads");
        let m = g.get(ModulatorId(7)).expect("the modulator");
        assert_eq!(m.slot, 0, "creation order is the only slot a layout-1 file had");
        assert!(m.targets[0].enabled, "every routing was on before layout 2");
        assert!(matches!(m.source, ModSource::Lfo(Lfo { offset: 0.0, amount, bipolar: true, .. }) if amount == 1.0));
    }

    /// Random and Audio Follower sources travel with their seed, distribution and band.
    #[test]
    fn the_new_sources_round_trip() {
        let mut g = ModGraph::default();
        let r = g.add(ModSource::Random(os_model::modulation::Random {
            seed: 0xDEAD_BEEF_1234_5678,
            distribution: os_model::modulation::Distribution::Gaussian,
            smooth: 0.4,
            interpolate: true,
            ..os_model::modulation::Random::default()
        }));
        let f = g.add(ModSource::AudioFollower(os_model::modulation::AudioFollower {
            attack: 0.02,
            release: 0.3,
            gain_db: 6.0,
            detection: os_model::modulation::Detection::Peak,
            band: os_model::modulation::Band::Custom { low: 120.0, high: 3500.0 },
            ..os_model::modulation::AudioFollower::default()
        }));
        let back = read_graph(&write_graph(&g)).expect("round trip");
        assert_eq!(back, g);
        assert!(matches!(back.get(r).map(|m| m.source), Some(ModSource::Random(os_model::modulation::Random { seed: 0xDEAD_BEEF_1234_5678, .. }))));
        assert!(back.get(f).is_some());
    }

    /// The graph's event payload is its exact round trip; a layout this version does not know is ignored, and a
    /// payload cut anywhere still reads the modulators before the cut instead of panicking.
    #[test]
    fn the_graph_round_trips_through_its_lemon_event() {
        let g = graph();
        let bytes = write_graph(&g);
        assert_eq!(read_graph(&bytes), Some(g.clone()));
        assert_eq!(lemon::MODULATION, 207, "after the identity registry's 206 (K.3)");
        assert!(read_graph(&[]).is_none());
        assert!(read_graph(&[3, 0, 0, 0, 0]).is_none(), "a later layout is not read");
        for n in 1..bytes.len() {
            if let Some(part) = read_graph(&bytes[..n]) {
                assert!(part.modulators.len() <= 3, "a cut payload keeps at most what it held");
            }
        }
    }

    /// Saved to a .lemon file and opened again, every modulator comes back with its source, name, enabled flag and
    /// its targets' depth, curve and polarity; adding one is an unsaved change, and an FLP drops the graph.
    #[test]
    fn the_graph_round_trips_through_a_lemon_file() {
        let d = dir("round");
        let (lemon_path, flp_path) = (d.join("Mod.lemon"), d.join("Mod.flp"));
        let mut app = App::new(None);
        app.state.project = crate::app::test_project();
        app.state.control.settle(&mut app.state.project);
        app.state.modulation = graph();
        app.save_to(lemon_path.clone());
        app.save_to(flp_path.clone());
        let file = os_flp::FlpFile::parse(&std::fs::read(&lemon_path).unwrap()).unwrap();
        assert_eq!(lemon::data(&file, lemon::MODULATION), Some(write_graph(&graph())));
        let mut back = App::new(None);
        back.open(lemon_path.clone());
        assert!(!back.unsaved(), "the graph read back is the saved one");
        assert_eq!(back.state.modulation, graph());
        assert_eq!(back.state.modulation.get(ModulatorId(1)).map(|m| m.name.as_str()), Some("Wobble"));
        assert!(!back.state.modulation.get(ModulatorId(3)).unwrap().enabled, "a modulator saved off comes back off");
        // Adding a modulator is an unsaved change, and saving again writes the new one.
        back.state.modulation.add(ModSource::default());
        assert!(back.unsaved());
        back.save_to(lemon_path.clone());
        let mut again = App::new(None);
        again.open(lemon_path);
        assert_eq!(again.state.modulation.modulators.len(), 4);
        // An FLP has no place for the identity or the graph.
        let mut flp = App::new(None);
        flp.open(flp_path);
        assert!(flp.state.modulation.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// An assignment whose parameter does not resolve now (a native id with no plugin metadata) is reported, not
    /// dropped, and the rest of the graph still resolves.
    #[test]
    fn an_assignment_that_does_not_resolve_is_reported() {
        let mut app = App::new(None);
        app.state.project = crate::app::test_project();
        app.state.control.settle(&mut app.state.project);
        let owner = app.state.ensure_owner(OwnerLocator::Track(2));
        let id = app.state.modulation.add(ModSource::default());
        let good = ParameterPath { owner, parameter: ParameterKey::HostParameterCode(0x0100) };
        let native = ParameterPath { owner, parameter: ParameterKey::PluginNativeId(7) };
        assert!(app.state.modulation.assign(id, ModTarget::new(good)));
        assert!(app.state.modulation.assign(id, ModTarget::new(native)));
        assert_eq!(crate::modulation::unresolved(&app.state, &app.plugins), vec![(id, native)]);
    }
}
