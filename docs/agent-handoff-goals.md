# Goals & plan — handoff doc for review/study agents

Read alongside: `todo.md` (checklist), `docs/track-model-plan.md` (full design),
`docs/feature-imports-audit.md` (our architecture), `Z:\github\lemon-study\fork-audit\*.md`
(Michael's fork, spec-level).

## Who we are
Lemon Studio (`Z:\github\Lemon-Studio`): pattern-based clean-room DAW in Rust, FL-style,
opens/saves .flp with lossless round-trip. ~26-crate workspace: os-model (plain data) →
os-flp (file I/O) → os-engine (playback) → os-app/os-ui; built-in plugins are separate
nih-plug (VST3+CLAP) crates linked in via os-hostlink. Clean-room rules apply: no code from
Michael's fork — spec-level study only.

## Goal 1 — Bitwig/Ableton-style track workflow (user's top priority)
User is a Bitwig user and hates FL's floating windows + channel→mixer→playlist workflow.
Plan (docs/track-model-plan.md):
1. First-class Track layer (Audio/MIDI/Bus): playlist lane + mixer strip + channel bound in
   one static object. Insert Track creates all three. Built on FL's existing
   audio/instrument-track links → `.flp` round-trip survives with no new chunks.
2. "Linear mode" default for new projects; Classic (FL) mode untouched. No migration of old
   projects (explicitly deferred by the user — much later).
3. Automation lanes (Bitwig 5.2-style) per track replace free-floating automation clips in
   Linear mode; storage stays FL-compatible (automation-clip channels), lanes are a
   view+placement constraint. Unbound targets land in an "Unassigned" lane group.
4. Device rack docked at the bottom: selected track's chain embedded by default —
   signal flow **MIDI FX → instrument → audio FX**. Pop-out to float is per-device opt-in.
   Built-ins embed via os-hostlink; CLAP embed first, VST3 embed is the big lift; docked/
   floating state per plugin in `.lemon`.
5. MIDI FX (§8) — genuinely new capability, absent from FL and Lemon: MIDI-FX slots on MIDI
   tracks feeding the instrument (notes → MIDI FX chain → instrument). CLAP-first hosting;
   built-ins v1 = Arpeggiator, Chord/Scale, Note Echo (nih-plug, reusable as plugins).
   `.lemon`-only data; Classic mode never shows the slots. This is the one piece touching
   the real-time note path in engine.rs — same engine-split prerequisite as MPE.

## Goal 2 — Feature imports from Michael's fork
(see todo.md import queue; audits done) Order: Intel Mac CI → MPE → Lemon Wave →
Lemon Chop → Master Assistant (Rust port of BSD-3 oXygen DSP; NO JUCE/AGPL, no bundled
engine binary) → community voting/forum/changelog (GitHub Discussions $0 for v1).
Standing constraint: do NOT port the fork's mixer — ours stays.

## What study agents should return
Per area: concrete implementation instructions — crate/file touch points, data-structure
changes, undo + control-plane exposure, `.flp`/`.lemon` round-trip impact, test plan,
sizing. Flag anything that violates the clean-room rule or the RT-hardening prerequisites
(engine.rs split, sinc/voice-path work in AUDIT-2026-11-04.md). Sequencing constraint:
MIDI FX + MPE + track model all touch the note path — propose a single sequencing plan
for all three rather than independent ones.
