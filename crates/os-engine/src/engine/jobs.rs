// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The per-tick jobs the generators run: an instrument channel's turn, its effect slots, and the notes handed to it.

use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};
use crate::instrument::{Instrument, TransportInfo};
use super::{auto_bypass, multicore, notes, param_auto, pdc};
use super::channel::{ChannelState};
use super::mix::{Fx, FxSlot, MixTrack, Route};
use super::triggers::{InstClip, InstTuning};

pub(crate) struct InstCx<'a> {
    pub(crate) n: usize,
    pub(crate) rate: u32,
    pub(crate) song: &'a TransportInfo,
    pub(crate) export: bool,
    pub(crate) wait: Duration,
    pub(crate) channels: &'a [ChannelState],
    /// This tick's notes by channel, the notes instruments missed, controller messages waiting.
    pub(crate) notes: &'a [(usize, u32, notes::Note)],
    pub(crate) missed: &'a [(usize, notes::Note)],
    pub(crate) midi: &'a [(usize, [u8; 3])],
    pub(crate) params: &'a param_auto::ParamAuto,
    pub(crate) shapes: multicore::Shapes,
    /// Per channel: a note held or due to end (`Engine::inst_held`); the program's Auto-bypass when silent setting.
    pub(crate) held: &'a [bool],
    pub(crate) auto_bypass: bool,
}

// SAFETY: `shapes` is shared under its own rule (each channel's turn touches its own clips); the rest is read only.
unsafe impl Sync for InstCx<'_> {}

/// A generator channel's own state for its turn.
pub(crate) struct InstLane<'a> {
    pub(crate) c: usize,
    pub(crate) fines: &'a mut InstTuning,
    pub(crate) owed: &'a mut usize,
    pub(crate) pad: Option<&'a mut pdc::Pad>,
    pub(crate) until: &'a mut Option<Instant>,
    pub(crate) out: &'a mut multicore::InstOut,
}

/// A generator channel's turn in a tick: its instrument gets the tick's notes and renders; what the channel adds to its
/// insert (its volume and pan, its pad) goes to `out` (`Engine::instruments_out` adds it in). An instrument busy
/// elsewhere after the tick's wait misses the tick (`out.ran` stays false) and owes its frames.
pub(crate) fn instrument_job(cx: &InstCx, lane: InstLane, sc: &mut multicore::MixScratch) {
    let InstLane { c, fines, owed, pad, until, out } = lane;
    let n = cx.n;
    let ch = &cx.channels[c];
    (out.ran, out.active) = (false, false);
    out.buf.clear();
    let Some(inst) = ch.instrument.0.as_ref() else { return };
    let Some(mut inst) = lock_busy(inst, until, cx.wait, cx.export) else {
        *owed += n;
        out.quiet.note(false, n);
        return;
    };
    out.ran = true;
    if sc.l.len() < n {
        sc.l.resize(n, 0.0);
        sc.r.resize(n, 0.0);
    }
    let first = cx.notes.partition_point(|e| e.0 < c);
    // Auto-bypass when silent (spec: audio/auto-bypass.md "Instruments", Lemon Studio's design): an instrument that has
    // had nothing to play and put out silence for the hosted hold is not run, until a note, a controller message, a
    // parameter value, an audio clip or a pitch change reaches it, or it says it still sounds. It adds nothing to its
    // insert meanwhile.
    let quiet_in = ch.smart
        && cx.auto_bypass
        && *owed == 0
        && !cx.held.get(c).copied().unwrap_or(false)
        && cx.notes.get(first).is_none_or(|e| e.0 != c)
        && !cx.missed.iter().any(|m| m.0 == c)
        && !cx.midi.iter().any(|m| m.0 == c)
        && cx.params.channel_ticks(c).next().is_none()
        && cx.params.channel_span(c, 0, n).next().is_none()
        && fines.shift == ch.hosted_shift()
        // SAFETY: only this channel's turn touches its clips.
        && unsafe { cx.shapes.of_channel(c) }.next().is_none()
        && !inst.active();
    if quiet_in && out.quiet.skip(true, Some(auto_bypass::hold_frames(auto_bypass::HOSTED_HOLD, cx.rate, out.delay))) {
        out.active = false;
        return;
    }
    let notes = cx.notes[first..].iter().take_while(|e| e.0 == c);
    // The channel's pitch (knob, control points, automation, fine tune, root note, main pitch; a sampler channel's
    // instrument applies root and fine itself) reaches the instrument as each note's tuning, with the note's fine
    // pitch, on top of any glide; notes keep the pitch they started with. A plugin channel's instrument that ignores a
    // tuning hears its whole semitones as moved keys instead.
    let shift = ch.hosted_shift();
    fines.moves = !ch.own_tuning && !inst.takes_tuning();
    // A changed channel pitch reaches the notes already sounding.
    if fines.shift != shift {
        fines.shift = shift;
        for key in (0..128u8).filter(|&k| fines.keys >> k & 1 == 1) {
            inst.tune(0, fines.sent(key), fines.tuning(key));
        }
    }
    // Ticks it missed are made up first: their notes at once, then their frames, unheard (at most a second; an
    // instance held longer than that loses that much of its place).
    let owed = std::mem::take(owed);
    let mut missed = cx.missed.iter().filter(|m| m.0 == c).peekable();
    if owed > 0 || missed.peek().is_some() {
        inst.transport(cx.song);
        for m in missed {
            send_note(&mut *inst, 0, m.1, fines);
        }
        let mut left = owed.min(cx.rate as usize);
        while left > 0 {
            let k = left.min(sc.l.len());
            if k == 0 {
                break;
            }
            inst.process(cx.rate, &mut sc.l[..k], &mut sc.r[..k]);
            left -= k;
        }
    }
    inst.transport(cx.song);
    // Controller messages before this tick's notes.
    for &(_, bytes) in cx.midi.iter().filter(|m| m.0 == c) {
        inst.midi(0, bytes);
    }
    // The layer whose notes it plays adds its levels to the channel's.
    let layer = ch.via.and_then(|w| cx.channels.get(w.layer).map(|l| (l.layer_adds(), w.gain)));
    let mut levels = ch.levels();
    if let Some((a, gain)) = layer {
        levels.volume = (f64::from(levels.volume) * a.volume * gain) as f32;
        levels.pan += a.pan as f32;
        levels.mod_x += a.mods.0;
        levels.mod_y += a.mods.1;
    }
    let own_levels = inst.channel_levels(levels);
    inst.channel_voicing(&ch.voicing);
    // Parameter values before the notes at the same frame (spec: plugins/plugin-parameters.md §2); one per tick at its
    // start for a plugin that takes them so (spec: plugins/fruity-abi.md "Parameters"), parameter by parameter (a
    // channel's container: each member as its plugin takes them).
    for (index, value) in cx.params.channel_ticks(c) {
        if inst.tick_params_at(index) {
            inst.param(0, index, value);
        }
    }
    for (off, index, value) in cx.params.channel_span(c, 0, n) {
        if !inst.tick_params_at(index) {
            inst.param(off, index, value);
        }
    }
    for &(_, off, note) in notes {
        send_note(&mut *inst, off.min(n.saturating_sub(1) as u32), note, fines);
    }
    // Audio clip fades and gain on this channel (spec: audio/audio-clips.md "Fades and clip gain"): on each clip's
    // voice when the instrument takes voice gains, so clips that overlap (a crossfade) each have their own. A voice
    // gets its gain now at its start, then every `VOICE_GAIN_FRAMES` frames the gain due that many frames on, which it
    // glides to; a gain that stays is not sent again.
    let voiced = inst.voice_gains();
    // SAFETY: only this channel's turn touches its clips.
    let shapes = || unsafe { cx.shapes.of_channel(c) };
    if voiced {
        let step = crate::instrument::VOICE_GAIN_FRAMES;
        for s in shapes() {
            if s.sent.is_none() {
                let g = s.shape.at(s.age);
                inst.voice_gain(0, s.voice, g);
                s.sent = Some(g);
            }
            for off in (0..n as u32).step_by(step as usize) {
                let g = s.shape.at(s.age + u64::from(off + step));
                if s.sent != Some(g) {
                    inst.voice_gain(off, s.voice, g);
                    s.sent = Some(g);
                }
            }
        }
    }
    let (l, r) = (&mut sc.l[..n], &mut sc.r[..n]);
    inst.process(cx.rate, l, r);
    out.delay = inst.reported_delay();
    // An instrument without voice gains: the clips' shapes on its whole output. A clip that has ended keeps the channel
    // silent through its choke's tail only while no other clip of the channel plays; then the playing clips' gains
    // alone count. Clips that overlap still share one gain, so a crossfade between them dips.
    if !voiced && shapes().next().is_some() {
        for f in 0..n {
            let ended = |s: &InstClip| s.shape.since + s.age + f as u64 >= s.shape.total;
            let playing = shapes().any(|s| !ended(&*s));
            let g: f32 = shapes().filter(|s| !(playing && ended(&**s))).map(|s| s.shape.at(s.age + f as u64)).product();
            l[f] *= g;
            r[f] *= g;
        }
    }
    out.active = inst.active();
    // An audio clip channel's voices are its playlist clips: their own level is on the channel's output, since a
    // hosted sampler takes velocities only up to 128 (spec: audio/audio-clips.md "Level").
    let cl = ch.clip_level;
    let (gl, gr) = if own_levels { (cl, cl) } else { ch.gain_after(n as u32, f64::from(cl), layer) };
    // A level that changed smooths over the tick, as a voice's does: it has arrived when the curve says so (spec:
    // audio/automation.md).
    let (sl, sr) = out.gain.replace((gl, gr)).unwrap_or((gl, gr));
    if (sl, sr) == (gl, gr) {
        out.buf.extend(l.iter().zip(r.iter()).flat_map(|(&a, &b)| [a * gl, b * gr]));
    } else {
        let (dl, dr) = ((gl - sl) / n as f32, (gr - sr) / n as f32);
        out.buf.extend(l.iter().zip(r.iter()).enumerate().flat_map(|(f, (&a, &b))| {
            let x = (f + 1) as f32;
            [a * (sl + dl * x), b * (sr + dr * x)]
        }));
    }
    out.quiet.note(quiet_in && !out.active && auto_bypass::silent(&out.buf), n);
    // Delay compensation: the channel waits for the latest path into its track.
    if let Some(pad) = pad.filter(|p| p.len() > 0) {
        pad.run(&mut out.buf);
    }
}

/// One tick of a mixer slot over interleaved `buf`: skipped when its switch is off, blended with the dry signal by its
/// dry level, which glides to its mix level by at most `limit` a sample (spec: audio/mixer.md "An effect slot", "A
/// slot's mix level while it moves"). A slot at mix 0 still runs its effect.
///
/// `(t, routes, out)`: the slot's track, its routes and its outbox, for an own effect's extra output, which is
/// added to the input of one of the track's outputs (spec: audio/sidechain.md "Extra output"). It is not padded for
/// delay compensation (audio/delay-compensation.md); it is connected from the first tick (the cleaner choice of
/// audio/quirks.md for the original's late connection).
///
/// `key`: the output of the track the slot listens to as its sidechain input, and a buffer for it after the slot's
/// pad, which runs also while the slot is off (spec: audio/sidechain.md "While the slot is off"). It is connected from
/// the first tick too.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_slot(
    fx: &mut FxSlot,
    buf: &mut [f32],
    rate: u32,
    limit: f32,
    (song, tick): (&TransportInfo, (u64, u16)),
    dry: &mut Vec<f32>,
    l: &mut Vec<f32>,
    r: &mut Vec<f32>,
    (until, wait, export, missed): (&mut Option<Instant>, Duration, bool, &mut u64),
    moved: &mut bool,
    (t, routes, out): (usize, &[Route], &mut multicore::Outbox),
    (key, main): (Option<(&[f32], &mut Vec<f32>)>, Option<(&[f32], &mut Vec<f32>)>),
    auto: bool,
) -> bool {
    let n = buf.len() / 2;
    let ramp = fx.dry.block(os_dsp::mixer::dry_level(fx.mix), n, limit);
    let fill = |(src, padded): (&[f32], &mut Vec<f32>), pad: &mut pdc::Pad| {
        padded.clear();
        padded.extend_from_slice(src);
        pad.run(padded);
    };
    let key = key.map(|k| {
        fill((k.0, &mut *k.1), &mut fx.key_pad);
        &k.1[..]
    });
    let main = main.map(|m| {
        fill((m.0, &mut *m.1), &mut fx.main_pad);
        &m.1[..]
    });
    if !fx.on {
        return false;
    }
    // Auto-bypass when silent (spec: audio/auto-bypass.md): a slot that has heard and put out silence for its hold time
    // passes its silent input on without running the effect, until sound reaches the effect again (its input, the
    // sidechain or the main bus it takes).
    let smart = fx.smart && auto;
    let quiet_in = smart && auto_bypass::silent(buf) && key.is_none_or(auto_bypass::silent) && main.is_none_or(auto_bypass::silent);
    if smart {
        let hold = match &fx.fx {
            Fx::Own(e) => e.quiet_hold(),
            Fx::Hosted(_) => Some(auto_bypass::HOSTED_HOLD),
        };
        if fx.quiet.skip(quiet_in, hold.map(|h| auto_bypass::hold_frames(h, rate, fx.delay))) {
            return true;
        }
    }
    let mut loud_extra = false;
    let blend = !(ramp.is_flat() && ramp.first() == 0.0);
    let fresh = std::mem::take(&mut fx.fresh);
    // The dry part waits for the effect's delay (spec: audio/delay-compensation.md "A slot's dry part"): every tick,
    // also at a mix of 100 %, and emptied with the effect when the slot is switched on.
    let padded = fx.dry_pad.len() > 0;
    if fresh {
        fx.dry_pad.clear();
    }
    if blend || padded {
        dry.clear();
        dry.extend_from_slice(buf);
        fx.dry_pad.run(dry);
    }
    // The delay the effect reports, read right after it processes.
    let delay = match &mut fx.fx {
        Fx::Own(e) => {
            if fresh {
                e.reset();
            }
            e.transport(song, tick);
            match key {
                Some(key) => e.process_keyed(buf, key),
                None => e.process(buf),
            }
            if let Some((k, signal)) = e.extra() {
                loud_extra = smart && !auto_bypass::silent(signal);
                // On a track routed nowhere the extra output joins the track's own audio behind the effect, or the dry
                // part when the slot's mix level is below 100 %.
                let dest = output(routes, t, k);
                if dest != t {
                    out.put(multicore::Dest::Track(dest)).extend_from_slice(signal);
                } else {
                    let into: &mut [f32] = if blend { &mut dry[..] } else { &mut *buf };
                    for (d, s) in into.iter_mut().zip(signal) {
                        *d += *s;
                    }
                }
            }
            e.latency() as u32
        }
        Fx::Hosted(h) => {
            // An instance still busy elsewhere after the tick's wait passes the sound through for this tick (a
            // poisoned one always does).
            let Some(mut h) = lock_busy(h, until, wait, export) else {
                *missed += u64::from(!h.is_poisoned());
                fx.fresh |= fresh;
                fx.quiet.note(false, n);
                return false;
            };
            if fresh {
                h.reset_fx();
            }
            // A main port takes another input instead of the track's own audio; an auxiliary port without an input hears
            // the track's own audio too (spec: audio/sidechain.md "Hosted plugins' input buses").
            let src: &[f32] = main.unwrap_or(buf);
            l.clear();
            r.clear();
            l.extend(src.chunks_exact(2).map(|f| f[0]));
            r.extend(src.chunks_exact(2).map(|f| f[1]));
            h.transport_fx(song);
            // The key's two sides go behind the input's in `l` and `r`.
            match key {
                Some(k) => {
                    l.extend(k.chunks_exact(2).map(|f| f[0]));
                    r.extend(k.chunks_exact(2).map(|f| f[1]));
                }
                None => {
                    l.extend_from_within(..n);
                    r.extend_from_within(..n);
                }
            }
            let ((il, kl), (ir, kr)) = (l.split_at_mut(n), r.split_at_mut(n));
            h.process_fx_keyed(rate, il, ir, (kl, kr));
            for (i, f) in buf.chunks_exact_mut(2).enumerate() {
                f[0] = l[i];
                f[1] = r[i];
            }
            h.reported_delay_fx()
        }
    };
    if delay != fx.delay {
        fx.delay = delay;
        *moved = true;
    }
    if blend {
        os_dsp::mixer::slot_blend(buf, dry, ramp);
    }
    if smart {
        fx.quiet.note(quiet_in && !loud_extra && auto_bypass::silent(buf), n);
    }
    false
}

/// Input `k` (from 1) of track `t`: the tracks routed into it in track order, whatever the route's level, a number past
/// the end meaning the last one; `None` when nothing is routed into it (spec: audio/sidechain.md "Settings").
pub(crate) fn input(tracks: &[MixTrack], t: usize, k: usize) -> Option<usize> {
    let mut ins = (0..tracks.len()).filter(|&s| s != t && tracks[s].routes.iter().any(|r| r.dest == t));
    let last = ins.clone().last();
    ins.nth(k.max(1) - 1).or(last)
}

/// Input `k` (from 1) of track `t` with no stand-in: `None` past the last input, as for a hosted plugin's port (spec:
/// audio/sidechain.md "Hosted plugins' input buses").
pub(crate) fn input_at(tracks: &[MixTrack], t: usize, k: usize) -> Option<usize> {
    (0..tracks.len()).filter(|&s| s != t && tracks[s].routes.iter().any(|r| r.dest == t)).nth(k.checked_sub(1)?)
}

/// Output `k` (from 1) of track `t`: the tracks it is routed to in track order, a number past the end meaning the last
/// one; a track routed nowhere (the master) is its own only output (spec: audio/sidechain.md "Settings").
pub(crate) fn output(routes: &[Route], t: usize, k: usize) -> usize {
    let mut at = None;
    for _ in 0..k.max(1) {
        match routes.iter().map(|r| r.dest).filter(|&d| at.is_none_or(|a| d > a)).min() {
            Some(d) => at = Some(d),
            None => break,
        }
    }
    at.unwrap_or(t)
}

/// Hands one note event to an instrument at frame `off` of its next block. `fines`: the channel's pitch in semitones
/// and, per key, the fine pitch of the note last started there and the key it was given as (notes keep the pitch they
/// started with).
fn send_note(inst: &mut dyn Instrument, off: u32, note: notes::Note, fines: &mut InstTuning) {
    match note {
        notes::Note::Choke(key, None) => inst.choke(off, fines.sent(key)),
        notes::Note::Choke(key, Some(voice)) => inst.voice_choke(off, fines.sent(key), voice),
        notes::Note::Off(key) => inst.note_off(off, fines.sent(key)),
        notes::Note::On { key, vel, start, end, fine, levels, voice } => {
            // An instrument that ignores a tuning plays the note's pitch to the nearest semitone on a moved key.
            let sent = if fines.moves { (i32::from(key) + (fines.shift + fine).round() as i32).clamp(0, 127) as u8 } else { key };
            // The note's own pan and mod X / Y go with it (spec: audio/voice-levels.md "The note's own levels").
            match voice {
                Some(v) => inst.voice_start(off, sent, vel, start, end, levels, v),
                None => inst.note_start(off, sent, vel, start, levels),
            }
            let k = key & 127;
            (fines.fine[usize::from(k)], fines.glide[usize::from(k)], fines.sent[usize::from(k)]) = (fine, 0.0, sent);
            fines.keys |= 1 << k;
            let tuning = fines.tuning(key);
            if tuning.abs() > 1e-3 {
                inst.tune(off, sent, tuning);
            }
        }
        notes::Note::Tune(key, semitones) => {
            fines.glide[usize::from(key & 127)] = semitones;
            inst.tune(off, fines.sent(key), fines.tuning(key));
        }
    }
}

/// Locks a hosted plugin the UI thread also uses. While it is busy there the tick waits, until `until` (set at the
/// tick's first wait, `wait` from then); `None` once that has passed. An export waits as long as it takes.
pub(crate) fn lock_busy<'a, T: ?Sized>(m: &'a Mutex<T>, until: &mut Option<Instant>, wait: Duration, export: bool) -> Option<MutexGuard<'a, T>> {
    if export {
        return m.lock().ok();
    }
    loop {
        match m.try_lock() {
            Ok(g) => return Some(g),
            Err(TryLockError::Poisoned(_)) => return None,
            Err(TryLockError::WouldBlock) => {}
        }
        let now = Instant::now();
        if now >= *until.get_or_insert(now + wait) {
            return None;
        }
        std::thread::yield_now();
    }
}
