// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Quick Sidechain (ported from the Michael Lane Edition): one command on a mixer routing arrow loads a keyed Lemon
//! Limiter after the destination's last effect, on COMP with the source as its external input, and gives the source a
//! sidechain-only send. Opt-in: Settings ▸ Miscellaneous ▸ Quick Sidechain in the mixer
//! (`prefs::QUICK_SIDECHAIN`). Nothing changes when the chain is full or the routing would loop.
// spec: ui/mixer.md "Quick Sidechain"

use crate::app::App;
use crate::mixer as mx;
use lemon_limiter::params as lp;
use os_engine::effects::dynamics as dsp;

/// Effects a mixer track has (the model's slot array).
const SLOTS: usize = 10;

/// The limiter's state: COMP page, threshold -30 dB, ratio 4:1, release 200 ms, keyed from `port` (1 = the
/// destination's first input).
fn preset(port: usize) -> Vec<u8> {
    let mut values = dsp::DEFAULTS;
    for (k, text) in [(dsp::COMP_THRES, "-30"), (dsp::COMP_RATIO, "4:1"), (dsp::COMP_REL, "200")] {
        values[k] = lp::parse_text(lp::DEFS[k].text, text, dsp::RANGES[k].0, dsp::RANGES[k].1).unwrap();
    }
    let mut raw = lp::to_fl(&values, &lp::default_state());
    raw[lp::PAGE_BYTE] = 1;
    raw[lp::SIDECHAIN_BYTE..lp::SIDECHAIN_BYTE + 4].copy_from_slice(&(port as i32).to_le_bytes());
    lp::plugin_state_from_fl(&raw)
}

/// Runs Quick Sidechain from the current track to `dst`.
pub fn run(app: &mut App, dst: u16) -> Option<String> {
    let src = app.state.selected_mixer;
    if src == 0 || src == dst || mx::track(&app.state, dst).is_none() {
        return Some("Select the trigger track, then Quick Sidechain on a different destination".into());
    }
    // The first empty slot after the last used one: existing effects keep their order.
    let slot = {
        let m = mx::track(&app.state, dst)?;
        (0..SLOTS).rfind(|&i| m.slots[i].is_some()).map_or(0, |i| i + 1)
    };
    if slot >= SLOTS {
        return Some("This channel has reached its 10-effect limit".into());
    }
    let Some(index) = app.plugins.effect_index("Lemon Limiter") else {
        return Some("Lemon Limiter is not available yet; try again after plugin loading finishes".into());
    };
    let old_inputs = mx::inputs(&app.state, dst);
    let mut inputs = old_inputs.clone();
    if !inputs.contains(&src) {
        inputs.push(src);
        inputs.sort_unstable();
    }
    let port = inputs.iter().position(|t| *t == src).unwrap() + 1;
    let state = preset(port);
    let reference = crate::project_plugins::plugin_ref(&app.plugins.effects[index].desc, state.clone());
    let wrapper = os_project::wrapper::encode(None, &reference);
    let Some(wrapper) = os_project::wrapper::set_input_port(&wrapper, 1, port as i32) else {
        return Some("Could not configure the sidechain input".into());
    };
    // The whole project changes (the destination's chain and the source's routing): one undo step.
    crate::control::edit_app(app, "Quick Sidechain", crate::control::command::Region::Project, |app| {
        if let Err(e) = app.plugins.create_effect(dst, slot as u8, index, Some(&state)) {
            return Some(format!("Quick Sidechain: {e}"));
        }
        if let Some(m) = mx::track_mut(&mut app.state, dst) {
            // Effects already keyed from a destination input keep their source when the list grows.
            for effect in m.slots.iter_mut().flatten() {
                if effect.name == os_project::wrapper::WRAPPER {
                    for (bus, old) in os_project::wrapper::input_ports(&effect.state).iter().enumerate() {
                        if *old > 0 {
                            if let Some(source) = old_inputs.get(*old as usize - 1) {
                                if let Some(new) = inputs.iter().position(|t| t == source) {
                                    if let Some(state) = os_project::wrapper::set_input_port(&effect.state, bus, (new + 1) as i32) {
                                        effect.state = state;
                                    }
                                }
                            }
                        }
                    }
                } else if effect.name == "Fruity Limiter" && effect.state.len() >= 168 {
                    let old = i32::from_le_bytes(effect.state[164..168].try_into().unwrap());
                    if old > 0 {
                        if let Some(source) = old_inputs.get(old as usize - 1) {
                            if let Some(new) = inputs.iter().position(|t| t == source) {
                                effect.state[164..168].copy_from_slice(&((new + 1) as i32).to_le_bytes());
                            }
                        }
                    }
                }
            }
            m.slots[slot] = Some(os_model::SlotPlugin { id: os_model::Id::NONE, name: os_project::wrapper::WRAPPER.into(), state: wrapper });
            m.slot_on[slot] = true;
            m.slot_mix[slot] = 1.0;
            m.slots_enabled = true;
        }
        if let Some(m) = mx::track_mut(&mut app.state, src) {
            if !m.routes.contains(&dst) {
                m.routes.push(dst);
                m.routes.sort_unstable();
            }
            m.send_levels.retain(|(t, _)| *t != dst);
            m.send_levels.push((dst, 0.0));
        }
        app.plugins.added(crate::plugins::fx_key(dst, slot as u8));
        app.state.piano.dirty = true;
        app.invalidate();
        Some(format!(
            "Quick Sidechain: {} → {}, slot {}. COMP, −30 dB, 4:1, 200 ms release",
            mx::name(&app.state, src),
            mx::name(&app.state, dst),
            slot + 1
        ))
    })
}
