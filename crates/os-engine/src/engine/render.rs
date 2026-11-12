// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The tick's rendering: sampler voices read through a windowed sinc into their track, and one mixer track's turn in the graph.

use std::time::Instant;
use crate::laws;
use crate::sample::SampleLibrary;
use super::{multicore, rec};
use super::{RecordPoint, RecordTap};
use super::channel::{ChannelState, stretched_frames};
use super::jobs::run_slot;
use super::mix::{MixTrack, input_channels};
use super::voice::{Voice};

/// Resampling window half-width in source samples.
pub(crate) const SINC_W: f64 = 13.5;

/// Adds one voice into its channel's insert bus for block frames `offset..end`.
/// Returns false when the voice has finished.
///
/// The gain ramps from the voice's last gain to the channel's current one over the span (no
/// change = constant). Rate 1 copies samples exactly. Other rates use a Blackman-windowed sinc with a fixed
/// 13.5-sample window; above rate 1 the cutoff drops to the output Nyquist (`1/rate`).
// spec: audio/sampler-playback.md (resampling)
/// The place in `ch.keyed` of the copy of its sample stretched for `key`, made when missing (realtime stretch methods
/// only): the channel's pitch with its fine tune, root and main pitch, and the stretch's own included.
pub(crate) fn keyed_copy(ch: &mut ChannelState, library: &mut SampleLibrary, key: u16) -> Option<usize> {
    let (Some((factor, cents)), Some(s)) = (ch.realtime, ch.sample.clone()) else { return None };
    Some(match ch.keyed.iter().position(|k| k.0 == key) {
        Some(i) => i,
        None => {
            let transpose = laws::playback_rate(key, 0) * laws::cents_rate(ch.pitch_from_60()) * 2f64.powf(cents / 1200.0);
            ch.keyed.push((key, library.stretched(&s, stretched_frames(s.frames(), factor), transpose)));
            ch.keyed.len() - 1
        }
    })
}

pub(crate) fn render_voice(v: &mut Voice, channels: &[ChannelState], bus: &mut [Vec<f32>], span: usize, offset: usize, end: usize, w: f64) -> bool {
    render_voice_into(v, channels, &mut bus[channels[v.channel].insert], span, offset, end, w)
}

/// `render_voice` into `out`, its track's input (or a buffer of its own, added into it later).
pub(crate) fn render_voice_into(v: &mut Voice, channels: &[ChannelState], out: &mut [f32], span: usize, offset: usize, end: usize, w: f64) -> bool {
    let ch = &channels[v.channel];
    let Some(s) = ch.buffer(v.buf) else { return false };
    let n = s.frames();
    let glide = ch.ramp.filter(|r| v.serial < r.id);
    // A layer's note: the layer's volume and crossfade gain multiply, its pan and pitch add (spec: audio/note-rules.md
    // "Layer channels").
    let layer = v.via.and_then(|w| channels.get(w.layer).map(|l| (l.layer_adds(), w.gain)));
    let (lv, lp) = layer.map_or((1.0, 0.0), |(a, g)| (a.volume * g, a.pan));
    let target = ch.start_gain_via(v.vel, v.pan, layer);
    if glide.is_none() && target != v.dest {
        // Any edit smooths over this span (a control point's glide is handled per frame below).
        let len = end.saturating_sub(offset).max(1) as u32;
        v.dest = target;
        v.delta = ((target.0 - v.gain.0) / len as f32, (target.1 - v.gain.1) / len as f32);
        v.ramp_left = len;
    }
    let (mut gl, mut gr) = v.gain;
    let (dl, dr) = v.delta;
    let mut left = v.ramp_left;
    let base_step = ch.base_rate * v.step * layer.map_or(1.0, |(a, _)| laws::cents_rate(a.cents));
    let mut step = base_step;
    // Rate 1 copies samples only while the position is whole (a voice that was pitched keeps its fraction).
    let exact = step == 1.0 && v.pos.fract() == 0.0 && ch.pitch_env.is_none();
    let mut scale = step.max(1.0);
    let off_s = v.off.map(|o| o as f64 * ch.frame_s);
    let mut pos = v.pos;
    for f in offset..end {
        if pos >= n as f64 {
            v.pos = pos;
            v.gain = (gl, gr);
            return false;
        }
        let t_s = v.age as f64 * ch.frame_s;
        let shape = v.shape.map_or(1.0, |s| s.at(v.age));
        v.age += 1;
        let env_gain = match &ch.vol_env {
            None => 1.0,
            Some(p) => match p.level(t_s, off_s) {
                Some(l) => l as f32,
                None => {
                    v.pos = pos;
                    v.gain = (gl, gr);
                    v.ramp_left = left;
                    return false;
                }
            },
        };
        if let Some(p) = &ch.pitch_env {
            step = base_step * 2f64.powf(p.level(t_s, off_s).unwrap_or(0.0) * p.amount / 1200.0);
            scale = step.max(1.0);
        }
        let fade = match v.release.as_mut() {
            None => 1.0,
            Some((0, _)) => return false,
            Some((left, len)) => {
                *left -= 1;
                *left as f32 / *len as f32
            }
        };
        let (l, r) = if exact {
            let i = pos as usize;
            (s.left[i], s.right[i])
        } else if w <= 1.0 {
            // Linear (an export's 2-point resampling).
            let i = pos as usize;
            let (j, x) = ((i + 1).min(n - 1), (pos - i as f64) as f32);
            (s.left[i] + (s.left[j] - s.left[i]) * x, s.right[i] + (s.right[j] - s.right[i]) * x)
        } else {
            let i0 = (pos - w).ceil().max(0.0) as usize;
            let i1 = ((pos + w).floor() as usize).min(n - 1);
            let (mut l, mut r) = (0.0f64, 0.0f64);
            for i in i0..=i1 {
                let t = i as f64 - pos;
                let k = sinc(t / scale) / scale * blackman(t, w);
                l += f64::from(s.left[i]) * k;
                r += f64::from(s.right[i]) * k;
            }
            (l as f32, r as f32)
        };
        if let Some(r) = glide {
            // The volume parameter glides linearly and the volume law applies to it; the pan glides linearly
            // in the left / right gains (spec: audio/pattern-controls.md).
            let frames = f64::from(r.pos) + (f - span) as f64 + 1.0;
            let g = ch.vol_gain(r.v0 + (r.v1 - r.v0) * r.volume_at(frames)) * v.vel * lv;
            let law = ch.pan_law;
            let (a, b) = (laws::channel_pan_law(ch.pan_of(r.p0) + v.pan + lp, law), laws::channel_pan_law(ch.pan_of(r.p1) + v.pan + lp, law));
            let x = r.pan_at(frames);
            let (pl, pr) = (a.0 + (b.0 - a.0) * x, a.1 + (b.1 - a.1) * x);
            (gl, gr) = ((g * pl) as f32, (g * pr) as f32);
            v.dest = (gl, gr);
            left = 0;
        } else if left > 0 {
            gl += dl;
            gr += dr;
            left -= 1;
            if left == 0 {
                (gl, gr) = v.dest;
            }
        }
        out[f * 2] += l * gl * fade * env_gain * shape;
        out[f * 2 + 1] += r * gr * fade * env_gain * shape;
        pos += step;
    }
    v.pos = pos;
    v.gain = (gl, gr);
    v.ramp_left = left;
    pos < n as f64
}

pub(crate) fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else {
        let px = std::f64::consts::PI * x;
        px.sin() / px
    }
}

fn blackman(t: f64, w: f64) -> f64 {
    if t.abs() >= w {
        return 0.0;
    }
    let x = std::f64::consts::PI * t / w;
    0.42 + 0.5 * x.cos() + 0.08 * (2.0 * x).cos()
}

/// One mixer track's own state for its turn in a tick (`mix_track`).
pub(crate) struct Lane<'a> {
    pub(crate) t: usize,
    pub(crate) track: &'a mut MixTrack,
    /// What reached the track (its voices, channels and inputs); the track's output once it has run.
    pub(crate) buf: &'a mut Vec<f32>,
    /// While capturing, the track's capture and the lateness still to skip at its start (`set_track_capture`).
    pub(crate) capture: Option<&'a mut Vec<f32>>,
    pub(crate) skip: Option<&'a mut u64>,
    /// What it adds to other tracks and to the device's pairs.
    pub(crate) out: &'a mut multicore::Outbox,
    /// When the tick stops waiting for hosted plugins busy elsewhere, the ticks they missed, a slot's delay changed.
    pub(crate) until: &'a mut Option<Instant>,
    pub(crate) missed: &'a mut u64,
    pub(crate) moved: &'a mut bool,
}

/// A mixer track's turn in a tick (`Engine::mix`): a monitored input joins, its slots, its equaliser, its fader matrix;
/// its output for the slots that listen to it, its meters, its capture; then what it sends to other tracks and plays on
/// a device pair goes to its outbox.
pub(crate) fn mix_track(lane: Lane, cx: &multicore::MixCx, s: &mut multicore::MixScratch) {
    let Lane { t, track, buf, capture, skip, out, until, missed, moved } = lane;
    let (frames, limit) = (cx.frames, cx.limit);
    out.clear();
    // A monitored input joins what reaches the track, before its slots (spec: audio/track-io.md §2.2).
    if let Some((inputs, ch, _)) = cx.io {
        if let Some((l, r)) = input_channels(track.input, ch) {
            for (f, x) in buf.chunks_exact_mut(2).zip(inputs.chunks_exact(ch)) {
                f[0] += x[l];
                f[1] += x[r];
            }
        }
    }
    // Disk recording, while the song plays (spec: audio/track-io.md §2.5).
    let tap = cx.record.and_then(|(r, at)| Some((r, at, track.rec?)));
    if let Some((r, at, arm)) = tap.filter(|(_, _, a)| matches!(a.point, RecordPoint::External | RecordPoint::WithMixer)) {
        rec_input(r, t, at, arm, track.input, cx.io, buf, &mut s.rec);
    }
    // The track's strip, before the insert slots (plan v3 §6.1): the chain of modules the track runs
    // (`MixerTrack::chain`) — the Analog Channel Strip's six sections, the Glitch grid, or a mix of them in signal
    // order. The two whole strips keep their own paths, so `CHAIN_ANALOG` renders as the strip always has (its bypass
    // fade included) and `CHAIN_GLITCH` as the grid does; a track that has neither runs its modules in turn, with the
    // strip's bypass switch as the chain's. An idle module writes nothing, so the same sample-for-sample pass-through
    // holds (`Strip::idle`, `Glitch::idle`).
    let chain = track.chain;
    if chain == os_model::CHAIN_ANALOG {
        if !track.strip.idle() {
            track.strip.process(buf, cx.rate);
        }
    } else if chain == os_model::CHAIN_GLITCH {
        if !track.glitch.idle() {
            track.glitch.process(buf, cx.rate);
        }
    } else {
        // A mixed chain fades across the strip's bypass switch in 10 ms (`chain_ramp`): the modules keep running while
        // the gain goes down, so the switch does not click, and stop once it is at zero, when they can no longer be
        // heard anyway.
        let gain = chain_ramp(track.chain_gain, !track.strip.bypassed, frames, cx.rate as f32);
        track.chain_gain = gain;
        if gain > 0.0 {
            for &module in chain.iter().filter(|&&m| m != 0) {
                if module == 7 {
                    if !track.glitch.idle() {
                        track.glitch.process(buf, cx.rate);
                    }
                } else {
                    track.strip.run_section(module, buf, cx.rate);
                }
            }
            // The whole buffer fades as one: the chain's four modules each mix their own dry, so they see one gain.
            if gain < 1.0 {
                for x in buf[..frames * 2].iter_mut() {
                    *x *= gain;
                }
            }
        }
    }
    // A slot runs when its own switch is on and the track's slots are enabled (spec: audio/mixer.md).
    if track.slots_enabled && if t == 0 { cx.master_fx } else { cx.insert_fx } {
        let (dry, l, r, key_buf, main_buf) = (&mut s.dry, &mut s.l, &mut s.r, &mut s.key, &mut s.main);
        for fx in &mut track.effects {
            // SAFETY: the tracks a slot listens to do not run meanwhile (`multicore`).
            let feed = |s: Option<usize>| s.and_then(|s| unsafe { cx.keys.get(s) }).filter(|k| k.len() == buf.len());
            let key = feed(fx.key).map(|k| (k, &mut *key_buf));
            let main = feed(fx.main).map(|k| (k, &mut *main_buf));
            let extra = (t, &track.routes[..], &mut *out);
            fx.skipped = run_slot(fx, buf, cx.rate, limit, (cx.song, cx.tick), dry, l, r, (&mut *until, cx.wait, cx.export, &mut *missed), &mut *moved, extra, (key, main), cx.auto_bypass);
        }
    }
    if let Some((r, at, rec::Arm { point: RecordPoint::PostFx, .. })) = tap {
        r.push(t, at, buf);
    }
    track.eq.process(buf);
    if let Some((r, at, rec::Arm { point: RecordPoint::PostEq, .. })) = tap {
        r.push(t, at, buf);
    }
    // The four gains glide to their targets: a per-sample step for this tick, at most `limit`, applied as a running
    // sum (spec: audio/mixer.md "Gains ramp").
    let to = track.matrix;
    let mut ramps = [[track.gains[0][0].block(to[0][0], frames, limit), track.gains[0][1].block(to[0][1], frames, limit)], [
        track.gains[1][0].block(to[1][0], frames, limit),
        track.gains[1][1].block(to[1][1], frames, limit),
    ]];
    if ramps.iter().flatten().all(|r| r.is_flat()) {
        let [[ll, lr], [rl, rr]] = ramps.map(|r| r.map(|g| g.first()));
        for f in buf.chunks_exact_mut(2) {
            let (l, r) = (f[0], f[1]);
            f[0] = ll * l + lr * r;
            f[1] = rr * r + rl * l;
        }
    } else {
        for f in buf.chunks_exact_mut(2) {
            let [[ll, lr], [rl, rr]] = [[ramps[0][0].next(), ramps[0][1].next()], [ramps[1][0].next(), ramps[1][1].next()]];
            let (l, r) = (f[0], f[1]);
            f[0] = ll * l + lr * r;
            f[1] = rr * r + rl * l;
        }
    }
    if let Some((r, at, rec::Arm { point: RecordPoint::PostLevel, .. })) = tap {
        r.push(t, at, buf);
    }
    // What the track puts out, for the slots that listen to it (spec: audio/sidechain.md "Sidechain input").
    // SAFETY: no slot reads it meanwhile (`multicore`).
    if let Some(k) = unsafe { cx.keys.slot(t) } {
        k.clear();
        k.extend_from_slice(buf);
    }
    let pk = &mut track.peak;
    for f in buf.chunks_exact(2) {
        pk[0] = pk[0].max(f[0].abs());
        pk[1] = pk[1].max(f[1].abs());
    }
    // The direct output, after the fader matrix, besides the sends (the master's goes out in `route_out`).
    if let (Some((_, _, pairs)), true) = (cx.io, t != 0) {
        if let Some(p) = usize::try_from(track.output).ok().filter(|&p| p < pairs) {
            out.put(multicore::Dest::Pair(p)).extend_from_slice(buf);
        }
    }
    if let Some(c) = capture {
        // Each track's own lateness is skipped at the start (`take_track_audio`).
        let skip = skip.map_or(0, |s| {
            let k = (*s).min(frames as u64);
            *s -= k;
            k as usize
        });
        c.extend_from_slice(&buf[skip * 2..frames * 2]);
    }
    // Sends glide like the matrix gains; a route at 0 passes nothing.
    for route in &mut track.routes {
        let mut ramp = route.glide.block(route.effective(), frames, limit);
        if route.pad.len() > 0 {
            // Delay compensation: the send's level, then its pad, which runs every tick (also at level 0).
            let padded = out.put(multicore::Dest::Track(route.dest));
            if ramp.is_flat() {
                let g = ramp.first();
                padded.extend(buf.iter().map(|s| *s * g));
            } else {
                for s in buf.chunks_exact(2) {
                    let g = ramp.next();
                    padded.extend([s[0] * g, s[1] * g]);
                }
            }
            route.pad.run(padded);
        } else if !ramp.is_flat() {
            let into = out.put(multicore::Dest::Track(route.dest));
            for s in buf.chunks_exact(2) {
                let g = ramp.next();
                into.extend([s[0] * g, s[1] * g]);
            }
        } else if ramp.first() != 0.0 {
            let g = ramp.first();
            out.put(multicore::Dest::Track(route.dest)).extend(buf.iter().map(|s| *s * g));
        }
    }
}

/// Disk recording from the device input (External input only), or with what reaches the track before its slots
/// (External and mixer input; `buf` holds the input already when the track hears it: its `input` is set). A single
/// input is on both sides; a track with no input, or with one the device does not have, records silence or the mixer
/// alone. `take` is the lane's scratch.
#[allow(clippy::too_many_arguments)]
fn rec_input(tap: &dyn RecordTap, t: usize, at: u64, arm: rec::Arm, input: i32, io: Option<(&[f32], usize, usize)>, buf: &[f32], take: &mut Vec<f32>) {
    take.clear();
    if arm.point == RecordPoint::WithMixer {
        take.extend_from_slice(buf);
    } else {
        take.resize(buf.len(), 0.0);
    }
    let heard = input >= 0 && arm.point == RecordPoint::WithMixer;
    if let (Some((inputs, ch, _)), false) = (io, heard) {
        if let Some((l, r)) = input_channels(arm.input, ch) {
            for (f, x) in take.chunks_exact_mut(2).zip(inputs.chunks_exact(ch)) {
                f[0] += x[l];
                f[1] += x[r];
            }
        }
    }
    tap.push(t, at, take);
}

/// A mixed chain's gain on a block of `frames`, toward 1 (`on`: the chain runs) or 0 (the strip is bypassed): a 10 ms
/// ramp, so the bypass switch does not click, clamped at both ends so a bypassed chain stops at exactly zero and can.
fn chain_ramp(gain: f32, on: bool, frames: usize, rate: f32) -> f32 {
    let step = frames as f32 / (0.010 * rate.max(1.0));
    let target = if on { 1.0 } else { 0.0 };
    if gain < target {
        (gain + step).min(target)
    } else {
        (gain - step).max(target)
    }
}

#[cfg(test)]
mod tests {
    use super::chain_ramp;

    /// A mixed chain's bypass is a ramp, not a switch: 10 ms either way at 48 kHz, clamped at both ends.
    #[test]
    fn a_mixed_chain_fades_across_its_bypass() {
        // 480 frames is 10 ms: one block takes the gain all the way, in either direction.
        assert_eq!(chain_ramp(1.0, false, 480, 48_000.0), 0.0);
        assert_eq!(chain_ramp(0.0, true, 480, 48_000.0), 1.0);
        // 240 frames is half of it: 0.5 each way.
        assert!((chain_ramp(1.0, false, 240, 48_000.0) - 0.5).abs() < 1e-6);
        assert!((chain_ramp(0.0, true, 240, 48_000.0) - 0.5).abs() < 1e-6);
        // No overshoot: a chain switched off stops at zero and one switched on stops at one.
        assert_eq!(chain_ramp(0.2, false, 480, 48_000.0), 0.0);
        assert_eq!(chain_ramp(0.9, true, 960, 48_000.0), 1.0);
    }
}
