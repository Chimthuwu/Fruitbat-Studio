# Session-state audit — 2026-11-06

Where the project actually stands across all local copies, and the plan forward.

## Layout (the truth about the "scattered folders")

One git repo, three working copies (git worktrees, all pointing at origin
`Chimthuwu/Lemon-Studio`, forked from `ItsAlphaNeon/Lemon-Studio`):

| Path | Branch | State on 2026-11-06 |
|---|---|---|
| `Z:\GITHUB\Lemon-Studio` | `main` @ b25bf8fc | In sync with origin. Device rack: ＋Add browser, MoveEffect/RemoveInstrument, undoable commands. |
| `Z:\GITHUB\Lemon-Studio-engine` | `orchestra/engine-split` @ de<hash> | engine.rs (~2,250 lines) split into `engine/{channel,jobs,live_edit,mix,render,sched,transport,triggers,voice}.rs`. WIP checkpoint pushed. |
| `Z:\GITHUB\Lemon-Studio-midifx` | `orchestra/midi-fx-plugins` @ (M9 checkpoint) | M9 crates: lemon-arp/chord/echo/note-layer/note-selector/audio-layer/audio-selector (58 tests pass per todo) + rack hosting wiring, editors still stubs. Pushed. |

Reference-only clones (import queue, do not delete casually):
`Z:\GITHUB\Lemon-Studio-MLE` (fork code), `Z:\GITHUB\lemon-study\fork-audit` (audit notes).

## C drive — clean

Only `C:\Users\Serge\AppData\Roaming\Lemon Studio` (settings, plugin cache,
project autosaves, ~324 KB) and the Desktop shortcut. Normal DAW behaviour,
nothing stray.

## Z:\ target dirs (the real "scatter")

Relocated Cargo build dirs created via explicit `CARGO_TARGET_DIR=` env vars —
no config file wires them, so they only exist because past sessions set the
variable manually:

| Dir | Size | Maps to | Verdict |
|---|---|---|---|
| `Z:\lemon-target` | 14 G | main | keep (consolidation candidate) |
| `Z:\lemon-target-rack` | 6.1 G | device-rack branch (merged 2026-11-04) | stale — deletable |
| `Z:\lemon-target-midifx` | 745 M | midi-fx worktree | in use |
| `Z:\lemon-target-engine` | 1.3 G | engine worktree | in use |

~22 GB total. Deleting the stale one reclaims 6.1 GB with zero risk; the others
are regenerable by `cargo build` but expensive to rebuild.

## Plan forward (order matters — per docs/sequencing-plan.md)

1. **Land the engine split.** `orchestra/engine-split`: finish split, `cargo
   test` green, merge to main. Everything else queues behind this — the
   sequencing plan requires MIDI FX to land as a note-path refactor *after* the
   split.
2. **Rebase/merge midi-fx branch onto post-split main**, then M9 next steps:
   real editor UIs (current ones are stubs) + full MIDI processing in note
   containers (arp timing, chord routing).
3. **M8 track model** (Bitwig/Ableton-style Audio/MIDI/Bus tracks) per
   `docs/track-model-plan.md`, after the split.
4. **Housekeeping:** one `CARGO_TARGET_DIR` convention documented (env var or
   per-worktree `.cargo/config`), stale rack target dir deleted.
