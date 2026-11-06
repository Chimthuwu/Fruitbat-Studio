# Lemon Studio — todo.md

## 🎯 Engine Split (2026-11-06) — COMPLETE
- ✅ `os-engine/src/engine.rs` split into per-concern modules under `engine/`:
  `channel` (channel state, ramps, control events), `jobs` (instrument/mix job contexts),
  `live_edit`, `mix` (mixer graph, device io, fx slots), `render` (voice/track render lanes),
  `sched` (scheduling, automation clips), `transport`, `triggers` (triggers/tunings/clips),
  `voice` (voice + glide slide), plus pre-existing `auto_bypass`/`controllers`/`layers`/
  `marks`/`multicore`/`notes`/`param_auto`/`pdc`/`rec`.
- ✅ engine.rs is now the `Engine` struct + core `impl` + the engine test suite (~7.3k lines,
  down from ~9.5k); data structs live in their concern modules with `pub(crate)` fields.
- ✅ Pure structural refactor: no behavior change (verified — every non-visibility diff line
  is an import statement); `cargo test --workspace` 2248 passed / 0 failed / 79 ignored.
- ✅ Fixed split-file headers (CRCRLF endings broke the `watermark` header check).
- Note: working-tree line endings normalized to LF for touched files (git autocrlf handles
  conversion); remaining repo warnings are pre-existing os-app/os-theme dead-code ones.

## Current Session (2026-11-04) — Rack Rework Complete
- ✅ `＋ Add` header button → plugin browser (native built-ins first per section, filter radios in footer)
- ✅ DELETE/Backspace removes selected effect tile (instrument never removable)
- ✅ Left/Right arrows move selection across the chain
- ✅ Hosted plugin tiles replace MIDI FX placeholder
- ✅ `MoveEffect` / `RemoveInstrument` / `HostReq::RemovePlugin` in control plane (undoable)
- ✅ Workspace `cargo test` green, release build OK
- ✅ Commit pushed (`b25bf8fc`)
- 📋 **Next session starts at M9 MIDI-FX crates** (see below)

## In flight / partial (from README capabilities)
- [ ] Playback engine: render-ahead hardening, voice limits, cut groups, glides, layer channels
- [ ] Automation: record into automation clips (clip curves + LFO exist)
- [ ] Project files: older FL layouts (<24.2) — keep improving the conversion path
- [ ] Built-in instruments/effects: finish the 🚧 list (see README capabilities table)
- [ ] Note input: recording into automation clips, step edit polish
- [ ] Undo: remaining gaps (see 🚧 note)
- [ ] Scripting/assistants: expand control-plane command coverage (docs/control-plane.md)
- [ ] Audio drivers: JACK support
- [ ] Plugin delay compensation: finish playback/export path
- [ ] Linux/Wayland: remaining plugin-window gaps (docking, drag-drop, clipboard)

## Interop research queue (Ableton/Bitwig ideas)
- [ ] Read .als (Ableton Live sets): gzip+XML — reference abletoolz / dawtool (both Python, format-level)
- [ ] Adopt DAWproject in/out (bitwig/dawproject, MIT) — interop with Bitwig/Cubase/Studio One
- [ ] Study bitwig-nitro-tools format docs for device/Grid architecture ideas (clean-room: specs only)
- [ ] Evaluate .flp ↔ DAWproject ↔ .als conversion via DawVert as reference
- [ ] Session/clip-launcher view: survey suhina-daw, beatlayer, nxtakt for clean-room design ideas

## New workflow: Bitwig/Ableton-style tracks (in progress)
- [ ] See `docs/track-model-plan.md` and `docs/sequencing-plan.md` — unified Audio/MIDI/Bus
      Track layer; default "Linear mode" with Classic (FL) mode kept; `.flp` round-trip via
      FL's own audio/instrument-track records. Sequencing plan adopted 2026-11-04: track
      model first (no engine), MPE + MIDI FX land as one note-path refactor after the
      engine.rs split, automation lanes are view-layer only.

## 🎯 M9 — MIDI FX Plugin Crates (NEXT SESSION START)
Read `plugins/lemon-small-fx` first for layout/param/editor conventions.

| Crate | Purpose | DSP module | Editor | Tests |
|-------|---------|------------|--------|-------|
| `lemon-arp` | Arpeggiator (patterns, octave, rate, gate) | `arp.rs` | minimal os-ui | pattern gen, scale map |
| `lemon-chord` | Chord/Scale mapper (incoming note → chord voicing) | `chord.rs` | minimal os-ui | scale mapping |
| `lemon-echo` | Note Echo (delay lines, feedback, sync) | `echo.rs` | minimal os-ui | echo scheduling |

For each:
- [ ] Create `plugins/lemon-arp/`, `lemon-chord/`, `lemon-echo/` with `Cargo.toml`, `src/lib.rs`, `src/dsp/*.rs`, `src/editor.rs`
- [ ] Unit tests in `src/dsp/*.rs` (no audio, pure logic)
- [ ] Minimal editor per OS UI conventions (os-ui)
- [ ] Register in workspace `Cargo.toml` + `tools/notices` if that's the convention
- [ ] `cargo test` green, no new warnings

## M8 — Track Model & Linear Mode (per `docs/track-model-plan.md` §7 + brief)
- [ ] `os-model/src/lib.rs`: `Track { kind: Audio|Midi|Bus, playlist_lane, mixer_strip, channel: Option<ChannelId> }`
- [ ] Insert Track → creates playlist lane + mixer strip + channel atomically
- [ ] Round-trip through `.flp` (LMex events 203 TRACKS, 204 LINEAR_MODE)
- [ ] Linear mode = default project template; Classic (FL) mode untouched
- [ ] Mode stored in `.lemon` project property
- [ ] Automation lanes per track (Bitwig 5.2-style) — view + placement constraint, storage stays FL-compatible (automation-clip channels + clips)
- [ ] Unassigned lane group for orphaned targets
- [ ] `.lemon`-only data; Classic never shows MIDI FX slots

## Device Rack — Remaining Polish
- [ ] Wire drag-to-reorder in `views/device_rack.rs` → `MoveEffect` command
- [ ] Dedicated drag handle on each tile header
- [ ] Focus delegation to plugin editors (global keyboard rules already in `app::act`)
- [ ] Container-nesting seam: `DeviceKind::Container` in chain for future Instrument Rack / Drum Machine step sequencer
- [ ] Rack geometry in prefs/session (no new `.lemon` data this milestone)

## Channel Rack Integration (Linear mode)
- [ ] Step sequencer becomes inline drum-lane view on MIDI tracks whose channel is a drum kit
- [ ] Presentation toggle per track
- [ ] Pattern-first users keep PAT mode working

## Master Assistant DSP (offline first)
- [ ] Rust port of oXygen BSD-3 DSP chain (gated-LUFS/band/width analysis → heuristic proposal)
- [ ] No JUCE, no bundled engine binary
- [ ] Fits into master-slot effect crate + `os-dsp` analysis crate
- [ ] Offline `--render` entry point first

## Import queue — features from Michael's Lemon
Source: https://github.com/Dorfhaus/Lemon-Studio-Michael-Lane-Edition/tree/michael-lane
(cloned at `Z:\github\Lemon-Studio-MLE`, HEAD c452b0d, release 2026.11.03.6)

**Deep audits done 2026-11-04:**
- Our side: `docs/feature-imports-audit.md` (architecture fit per feature)
- Fork side: `Z:\github\Lemon-Studio-MLE-audit\1-master-assistant.md` … `6-community.md`
  (spec-level only, clean-room rules respected)

**Key audit facts driving the plan:**
- The fork shares our crate skeleton (os-model / os-flp / os-engine / os-app /
  nih-plug plugins), so porting surfaces map almost 1:1.
- ⚠ Open licensing question before ANY porting: the fork is a public fork of the
  same upstream. If upstream+fork licenses permit, direct code reuse may be legal —
  otherwise everything below is spec-level clean-room reimplementation. Decide first.
- oXygen in the fork is a **C++/JUCE 8 standalone** (Wamphyre/oXygen v1.2.1 pinned),
  BSD-3 for the DSP but **JUCE is AGPLv3** — embedding it as-is would poison our
  license. Our integration must be a Rust re-implementation of oXygen's DSP
  algorithms (BSD-3 lets us do that), not a JUCE binary bundle.
- MPE in the fork is host-layer complete but synths only honor pitch bend; ours
  already has per-note-vs-channel bend modeling in `instrument.rs` — extend, don't
  redesign.
- Chop = one ~5.9k-LOC crate with 3+2 slice modes, 32 pads/voices, 16 aux outs.
  Wave = ~9k-LOC nih-plug crate, RIFF-WAV-only WT import, 9-level mip chain,
  linear frame morph. Both are solid spec targets but still marked WIP upstream.
- Do NOT port the fork's mixer — keep our existing mixer. None of the six queued
  features may pull in fork mixer changes (check for mixer-side diff contamination
  when porting anything engine/master-chain related).
- Community panel in the fork = custom self-hosted PHP+SQLite (HMAC/proof-of-work
  tokens) + ureq client; changelog is compile-time bundled JSON (no network).
  Our $0 equivalent: GitHub Discussions API (or reuse our Oracle VM for a tiny
  self-hosted service) — decide when we build it.

Order (cheapest/highest-certainty first; engine-touching items after RT hardening):

- [ ] Intel Mac support (day 1-2, no engine risk)
      1. Add x86_64-apple-darwin + universal2 jobs to `.github/workflows/ci.yml`
         (fork ships DMGs but has zero CI coverage for Intel — we do better).
      2. Verify the two SSE2 fast paths in os-ui have working fallbacks.
      3. Smoke-test plugin hosting + drivers on Intel hardware or CI.
- [ ] MPE support (seam already exists)
      1. Add MPE zone config + 3 expression lanes to os-model (fork proves the
         shape; implement our own) — reuse spare `Note` bytes or `.lemon`-only
         side data for recorded expression (FL has no MPE fields).
      2. Shared MPE decoder in os-engine; extend `midi_in.rs` → `engine.midi()`
         path for per-channel bend/pressure on member channels.
      3. Emit note expressions in CLAP/VST3 wrappers; add pressure/timbre (CC74)
         to lemon-wave/sampler/3x-osc (the fork skipped this — we can too, v1
         pitch-bend-only parity, then CC74).
      4. Smoothing for live-recorded curves (fork ships raw steps — do better).
      5. Test with MPE controller + per-note-bend offline renders.
      - Prereq: engine.rs is at splitting-point size — split before/while adding.
- [ ] Lemon Wave wavetable synth (biggest single feature, spec is fully auditable)
      1. Design doc: engine/voice/wave/patch/preset crate modeled on our Citrus
         crate-shape precedent.
      2. WT import: RIFF WAV first (fork-parity), resample to 1024-sample cycles,
         9-level mip chain; consider Serum .wt as an add since fork lacks it.
      3. DSP: 2 oscs + sub/noise, TPT SVF, morph = linear frame crossfade +
         phase-warp pivot; mod matrix v1 minimal.
      4. UI: os-ui widgets + os-hostlink editor page; stacked-frames display,
         defer the 64-bar harmonic editor to v2.
      5. Skip the fork's embedded LWPK pack (provenance unverified).
- [ ] Lemon Chop sampler (port after Wave; upstream still stabilizing)
      1. Spec from audit doc; watch Michael's changelog until hands-on review
         lands (his docs say the build awaits review).
      2. Build on our Lemon Slicer pipeline (load→slice→C5 mapping→piano-roll)
         rather than a new engine: autoset (transient), beat-grid, manual slice
         modes v1; 32 pads on notes 36–67; own voices (share
         storage/stretch with Lemon Sampler).
      3. Aux outs + offline WAV export v2; skip chromatic-dup/MIDI-capture extras
         until base is solid.
- [ ] Master Assistant (last: biggest dependency + license-sensitive)
      1. Rust port of oXygen's BSD-3 DSP chain: gated-LUFS/band/width analysis →
         heuristic proposal (EQ, dyn-EQ, comp, imager, maximizer). No JUCE, no
         bundled engine binary (fork's approach is AGPL-tainted).
      2. Fit into master-slot effect crate + os-dsp analysis; offline
         `--render` entry point first, real-time later.
      3. Suggest → confirm → apply via existing control-plane propose/apply-undo
         semantics. No auto-learn/ML v1 (fork stubs ONNX too).
      4. Reference-track matching: analyze user-supplied reference, same chain.
      5. Record BSD-3 attribution in THIRD-PARTY-NOTICES.txt (cargo test enforces).
      - Prereq: RT hardening items from AUDIT-2026-11-04.md (sinc recompute,
        voice-path SIMD) — Master Assistant rides the same engine path.
- [ ] Community feature voting + bug forum + in-app changelog
      1. Changelog first (zero network): compile-time bundled JSON in os-paths
         cache dir + simple in-app panel — direct port of the concept.
      2. Voting/forum backing: decide GitHub Discussions ($0, no infra) vs tiny
         self-hosted service on our Oracle VM (fork's PHP+SQLite is a proven
         pattern we'd re-implement). GitHub-only recommended for v1.
      3. Client: ureq (fork precedent) on a background worker, refresh throttle,
         offline cache fallback — this adds our repo's first HTTP dep + first
         credential handling: keep anonymous, store nothing sensitive.
      4. Add a privacy-policy note in-app before shipping anything networked.

Cross-cutting standing notes:
- engine.rs and os-flp/project.rs are at sizes where MPE/FLP work forces a split —
  schedule that before MPE.
- All six items touch code the 2026-11-04 quality audit flagged; re-run that audit
  after each feature lands.

## Standing rules
- Clean-room only: no decompiled code or pseudocode derived from decompilation (see README contributing).
- Keep `THIRD-PARTY-NOTICES.txt` in sync with Cargo.lock (`cargo run -p notices`) — cargo test enforces it.