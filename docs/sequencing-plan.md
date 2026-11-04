# Sequencing plan — track model + MIDI FX + MPE (one plan, three features)

Status: adopted 2026-11-04. Supersedes the three independent orderings in
`todo.md` / `track-model-plan.md` / audit notes for anything touching the note path.

## Why one plan

Three features touch the same seam — `crates/os-engine/src/engine/notes.rs` and the
RT-sensitive `crates/os-engine/src/engine.rs`:

- **Track model** — no engine work at all (bookkeeping + presentation over
  `PlaylistTrack.link`, mixer, channels). Safe first.
- **MPE** — note input path (`midi_in.rs` → `engine.midi()`), per-note expression in
  `Note`/voice structs, CLAP/VST3 wrapper event emission.
- **MIDI FX** — a new stage in the note path: `notes → MIDI FX chain → instrument`,
  per-tick note batches, tick-start voice change discipline, zero-audio-delay PDC.

Doing them as three independent refactors of `notes.rs`/`engine.rs` means three passes
over the same RT-critical seam and three merge conflicts with ourselves. Instead:

## Phases

### Phase 0 — engine.rs split (prerequisite, before any note-path change)
- Split `crates/os-engine/src/engine.rs` (audit: 4,800+ lines, RT-critical) into
  focused modules (`engine/voice.rs`, `engine/sched.rs`, `engine/mix.rs`, …) with
  **zero behaviour change**: `cargo test` green before and after, no diff in rendered
  offline output of the `os-render` corpus.
- This is the prerequisite both MPE (todo.md) and MIDI FX already name. Nothing else
  in phases 1–4 needs it, so it can run in parallel with phases 1–2 (different crates).

### Phase 1 — Track data model (M1, no engine)
- `os-model`: `Track { kind: Audio|Midi|Bus, playlist_lane, mixer_strip,
  channel: Option<ChannelId> }` — a record over existing linkage
  (`PlaylistTrack.link` = FL's audio/instrument-track records, event 238 bytes 49–60).
- `.lemon`-side: Linear/Classic mode flag + Track list in `lemon_state` events;
  `.flp` save omits them (existing `.lemon`-only policy). A linked Track
  round-trips as FL's own audio/instrument track record — no new FLP chunks.
- Undo + control plane: `TrackCreate`, `TrackDelete`, `TrackSet` commands following
  the existing Region take/put pattern in `os-app/src/control/command.rs`.

### Phase 2 — Linear mode UI (M2+M3, presentation only)
- Linear template, Insert Audio/MIDI/Bus Track flows, track headers (A/I semantics
  reused), track-ordered mixer view. Classic mode untouched.
- Parallel-safe with Phase 0 (os-app/os-ui vs os-engine).

### Phase 3 — Automation lanes (M4, view-layer only)
- Lanes = view + placement constraint over existing automation-clip channels;
  Unassigned group for orphaned targets; Classic mode keeps free clips.
- No engine cost (per plan doc §6b).

### Phase 4 — the one note-path refactor: MPE + MIDI FX in the same seam
After Phase 0 lands, do a single refactor of the note path that lands both:
1. Extend `Note`/voice structs with per-note expression (MPE zone config, 3 lanes
   in os-model, `.lemon`-only side data for recorded expression).
2. Insert the MIDI-FX stage into the same pipeline: per-tick note batches flow
   `notes → [MPE decode → MIDI FX chain] → instrument`; MIDI FX is zero-audio-delay.
3. Built-in MIDI FX v1 (nih-plug CLAP: Arpeggiator, Chord/Scale, Note Echo) consume
   the same events CLAP wrappers emit for MPE — one dialect pass, not two.
4. `midi_in.rs` record path lands in the same pass (M5 record-into-track).

### Phases 5+ — device rack (M6–M8) and beyond
- Rack shell + built-in embed → CLAP embed → VST3 embed (macOS fallback) →
  pop-out/dock state in `.lemon`. Independent of the note path; can run parallel
  with Phase 4 (os-hostlink/os-app vs os-engine).

## Constraint checklist carried through every milestone
- No fork (Lemon-Studio-MLE) code: spec-level only, per audit docs.
- No fork mixer port; `os-app/src/mixer.rs` + `views/mixer.rs` stay ours.
- `.flp` round-trip: no new FLP chunks in v1; Lemon-only data omitted on `.flp` save.
- CLAP-first hosting; VST3 embed falls back to thumbnail + pop-out on macOS.
- `THIRD-PARTY-NOTICES.txt` synced via `cargo run -p notices` (cargo test enforces).
- Re-run the 2026-11-04 audit after each feature lands.
