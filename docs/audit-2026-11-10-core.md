# Lemon Studio — Core Architecture Audit (2026-11-10)

Generated from workspace audit using cargo metadata + targeted reads. All file:line anchors reference the `f42195de` commit (wave 1 + wave 2 shipped).

## 1. Workspace layout

| Crate | Role | ~LOC (rs) | Key deps |
|-------|------|-----------|----------|
| `os-app` | Main DAW application (binary `lemonstudio`) | ~22,000 (app.rs + views/) | `os-model`, `os-dsp`, `os-engine`, `os-flp`, `os-hostlink`, `os-theme`, `os-ui`, `os-clap`, `os-native`, `os-host` |
| `os-model` | Project data model (Track, Clip, Pattern, playlist, undo/redo, FLP round-trip) | 136567 bytes (lib.rs) | workspace deps only (no external crates) |
| `os-dsp` | DSP building blocks: level, curve, pan, gain, EQ, mixer matrix | 1842 bytes (lib.rs) | `signalsmith-stretch` |
| `os-engine` | Audio engine: sample loading, mixing, rendering, chains | 1610 bytes (lib.rs) | `os-model`, `os-dsp`, `miniz_oxide`, `thiserror`, `symphonia` (format I/O) |
| `os-host` | Plugin hosting layer (CLAP, VST3, native backends) | — | `os-engine`, `libloading`, `windows-sys` (Windows) |
| `os-hostlink` | Host-link: how a Lemon plugin UI shares the window strip | — | `os-engine`, `windows-sys` (Win32 UI/Messaging) |
| `os-clap` | CLAP plugin host | — | `clap-sys`, `libloading`, `os-engine`, `os-host` |
| `os-native` | Native plugin interoperability (public SDK interface) | — | `libloading`, `thiserror`, `os-engine`, `os-dsp`, `os-host` |
| `os-paths` | Platform‑specific file locations (settings, themes, presets) | — | none (pure config) |
| `os-plugkit` | Shared plugin‑editor pieces: live param edits, menus, popups, type‑in boxes | — | `nih-plug`, `os-hostlink`, `os-ui`, `os-theme` |
| `os-theme` | Theme tokens + HSL adjustment model | — | none |
| `os-ui` | Painting primitives: shapes and text on a CPU canvas | — | `os-theme`, `tiny-skia`, `fontdue`; `windows-sys` on Windows |
| `os-smf` | Standard MIDI Files: reading/writing types 0 and 1 | — | none |
| `os-flp` | Lossless FLP project file reader/writer | — | `os-model`, `thiserror`, `chrono` |

**Root-level files**: `todo.md` (33982 bytes), `HANDOFF.md` (15810 bytes), `README.md` (34655 bytes), `Cargo.lock` (134769 bytes), `Cargo.toml` (3064 bytes).

## 2. os‑model: project data model (key structs & API)

- **`Track`** (`crates/os-model/src/lib.rs`): central record in the project. Has `playlist_lane: i32`, `grouped: bool` (FLP grouping semantics: "grouped with track above", leader = nearest ungrouped track above per `os-model/src/lib.rs:2114`), `name`, `color`, `midi_channels`, input/output buses. Serialized in FLP format.
- **`Clip`** + **`Pattern`**: event containers. Clips have a `lane` (playlist lane) and reference a `Pattern`. Patterns hold note data.
- **`playlist_lane`**: an integer identifying which lane a track/clip belongs to. The Clip Launcher (`launcher.rs:472-498`) only shows columns where `playlist_lane > 0` — this is the root cause of the duplicate‑track bug (see audit‑2026‑11‑09‑clip-launcher-bugs.md).
- **Undo / snapshot system**: mutation API through `project_*` functions (e.g. `project.track_lane_inserted` at `os-model/src/lib.rs:2254`). Each command allocates a snapshot; `cargo test -p os-app` baseline: 1129 passed / 62 ignored.
- **FLP round‑trip**: `os-flp` reads/writes Ableton Live project files. Key marker: event 237 (project start time, spec: `flp/events/project-info.md`). Serialization is lossless for the model subset Lemon supports.
- **Undo‑snapshot API** (high‑level): `Command` enum in `crates/os-app/src/app.rs` drives state transitions. Every UI action (track create, delete, duplicate, group, etc.) funnels through `app.rs` command routing → `os-model` mutation → new snapshot.

## 3. Audio engine path

- **`os-engine`** (`crates/os-engine/src/lib.rs`): owns the render loop, sample loading, and mixing. Calls `os-dsp` primitives for each device in a chain.
- **Chains**: a signal path through devices. Each device has slots (instrument, MIDI FX, audio FX). The chain plays through `os_engine::chain::Chain`; `Lemon Container` (`container.rs`) wraps slot chains with `wrap_in_container` (`device_rack.rs:962`) / `wrap_channel` (`:1113`) and supports `Moved::retarget` for when devices are moved.
- **Device processing order**: top‑down through the device rack; each device's `process()` is called in rack order. MIDI FX devices are **UI‑hosted only**: the engine never calls their `process()` (the "M9 note‑path milestone" — they pass notes through the in‑memory UI layer only; a MIDI FX Rack cannot pass notes through until that milestone lands).
- **Transport / clock**: global BPM, PPQ, record arm, cycle mode. Drives all Pattern/Clip playback.
- **MIDI routing**: runs through `os-hostlink` and the CLAP/VST3 hosts; see plugin stack section for the editor‑hosting detail.

## 4. Control plane (UI → state flow)

- **`Command` enum** lives in `crates/os-app/src/app.rs`. Every UI gesture (button click, key press, drag) is routed through `app.rs` command dispatch.
- **Rack keyboard block** (`app.rs:7969‑8008`): handles per‑device shortcuts (Ctrl+D duplicate, Ctrl+X/V cut/paste, SelectTile, PopOut/Dock). Runs **before** `playlist::key`, so it wins over the playlist's Ctrl+D.
- **`os-hostlink`**: mediates how a plugin's UI window strip integrates with the Lemon UI (window list, shared drag‑region). Used by the editor‑hosting story for native Lemon plugins and third‑party VSTs.
- **Persistence**: `app.rs` `drop_file` (`:4784`) + `add_audio` (`launcher.rs:1475`) handles OS file drops into the launcher; the `drop_file` path was wired at `launcher.rs:2116` but live behaviour still has the drop‑into‑cell bug (re‑audit needed, see user report).

## 5. Persistence / save‑load

- Save format built around the `os-model` data model + FLP‑compatible headers.
- `cargo test -p os-app` runs the full test suite; baseline 1129 passed / 62 ignored (post‑wave 1 + wave 2). The two new tests for rack ordering and container groups are included.

## 6. Threading / audio‑callback architecture

- **`os-native`** runloop + `guard` primitives ensure the audio callback runs on a dedicated thread without drop‑outs.
- DSP graph is lock‑free during the callback; state mutations (parameter changes, device add/remove) go through the control plane and are applied between callbacks.

## 7. What's solid / what's fragile

| Solid (verified by test suite) | Fragile (needs runtime confirmation / known broken) |
|--------------------------------|-----------------------------------------------------|
| Track model, undo/snapshot, FLP read/write | Drop‑into‑launcher‑cell (OS file drop) — unit tests pass but live behaviour unchanged |
| Per‑device pop‑out/dock (A1, committed) | Lemon Delay generic‑UI fallback in dock (fit check) |
| Browser reorder (MIDI FX → Lemon inst → Lemon FX → 3rd‑party) (A3, committed) | MIDI FX Rack (C3) — blocked on M9 note‑path milestone |
| Device selection + Ctrl+D/X/V (B1, committed) | Launcher paint bugs (E1/E2) — regression tests written but not fully green in all configurations |
| Ctrl+D track duplicate + launcher Track creation (A2, committed) | Clip‑launcher drag‑and‑drop — needs fresh runtime trace |
| Ctrl+G / Alt+G track grouping (D3, committed) | Launcher missing `window_body` swallow hit (incidental) |
| Delete selected tracks (D1, committed) | Toolbar vanishes on launcher clicks — fix in wave 2, test `launcher_click_repaint_matches_full` written |

---
*End of core architecture audit. File:line anchors reference the `f42195de` commit and the specific crates listed. All `cargo test -p os-app` runs mentioned are against the wave‑1/+2 committed code.*