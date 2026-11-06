# Session audit — 2026-11-07 (scatter cleanup)

Consolidated the scattered Lemon-Studio working copies. Nothing of value was
lost: everything removed was a byte-identical duplicate, a git worktree on an
already-merged+pushed branch, or a regenerable build cache.

## Before (the scatter)

One git repo (`Lemon-Studio`) plus **five** extra git worktrees, **six** stale
local branches, **three** loose planning/doc folders (two of them duplicating
each other byte-for-byte), and **four** `CARGO_TARGET_DIR` build caches strewn
across `Z:\`.

## What was done

### 1. Removed five merged worktrees (`git worktree remove`)

| Worktree | Branch | State |
|---|---|---|
| `Lemon-Studio-engine` | `orchestra/engine-split` | merged to main, pushed |
| `Lemon-Studio-midifx` | `orchestra/midi-fx-plugins` | merged, pushed |
| `Lemon-Studio-midifx-a/b/c` | `orchestra/midifx-a/b/c` | merged, pushed |

Verified first: every branch was `git branch --merged` into main **and** pushed
to `origin`; no file existed in any worktree that wasn't already in main (full
`comm` diff of every copy against main was empty). The `.git` files confirmed
they were real worktrees, so `git worktree remove` (not `rm`) cleaned both the
directories and the git metadata.

### 2. Deleted six stale local branches

`orchestra/engine-split`, `orchestra/midi-fx-plugins`, `orchestra/midifx-a/b/c`,
`temp/merge-midifx` — all confirmed merged into main and pushed to origin, so
recoverable. Local branch list is now just `main`.

### 3. Consolidated planning/study/audit docs into `lemon-study/`

- `Lemon-Studio-MLE-audit/` (6 files) was byte-identical to
  `Lemon-Planning/fork-audit/` → deleted the duplicate.
- `Lemon-Planning/` → its unique files moved into `lemon-study/`:
  `PLAN-SUMMARY.md`, `todo.md` → `planning-todo.md`, `fork-audit/`. Its other
  three files (`agent-handoff-goals.md`, `feature-imports-audit.md`,
  `track-model-plan.md`) were byte-identical to `Lemon-Studio/docs/` → deleted.
- `lemon-study/` is now the single planning/study/audit home (a local git repo,
  no remote). See its `INDEX.md`.

### 4. Deleted regenerable build caches

| Dir | Size | Why safe |
|---|---|---|
| `Z:\lemon-target-engine` | 1.3 GB | stale (engine worktree removed) |
| `Z:\lemon-target-midifx` | 745 MB | stale (midifx worktree removed) |
| `Z:\lemon-target-midifx-a` | — | stale (midifx-a worktree removed) |
| `Lemon-Studio-midifx/target` | ~670 MB | in-worktree build dir |

Kept: `Z:\lemon-target` (14 GB) — the active `main` build cache. (The 6.1 GB
`Z:\lemon-target-rack` was already deleted in a prior session.)

## After — the clean layout

```
Z:\github\
  Lemon-Studio/        THE repo — code, git, remote Chimthuwu/Lemon-Studio
  lemon-study/         planning + study + audit docs (local git)
  Lemon-Studio-MLE/    reference fork code (import-queue source, c452b0d)
  suhina-daw/          reference Rust DAW (device-SDK study source)
  bitwig-nitro-tools/  reference Bitwig reverse-engineering

Z:\lemon-target\       one build cache (main)
```

## Open item 1 — build toolchain is broken from git-bash

`cargo check` fails here: the rustup default is `stable-x86_64-pc-windows-msvc`,
but from the MSYS/git-bash shell `link` resolves to GNU coreutils (`/usr/bin/link`),
not MSVC's linker, so the linker stage dies with `link: extra operand`.
`RUSTUP_HOME` must point at `Z:\rustup` (not `~/.rustup`, which has no
toolchains), and `CARGO_TARGET_DIR=Z:\lemon-target`.

The repo's own notes say the **GNU** toolchain was used (`RUSTUP_HOME=/z/rustup`),
but rustup currently has only the MSVC toolchain installed. Two ways out, pick one:

1. `rustup toolchain install stable-x86_64-pc-windows-gnu` and build with
   `+stable-x86_64-pc-windows-gnu` (matches the repo notes, links fine from bash).
2. Build from a "x64 Native Tools" cmd prompt (vcvars) so MSVC's `link.exe` wins.

Not a scatter item — a separate fix to schedule.

## Open item 2 — uncommitted WIP in main

Three files are modified but uncommitted (in-flight work from a prior session):

- `crates/os-app/src/app.rs`
- `crates/os-app/src/device_rack.rs`
- `crates/os-app/src/views/device_rack.rs`

Device-rack right-click menu + undoable remove + tests (~246 insertions). Left
uncommitted on purpose — it is half-finished (a larger model change is noted as
still pending) and was not build-verified because of Open item 1. Finish it,
`cargo test`, then commit.

## Path forward

1. Fix the toolchain (Open item 1) so `cargo test` runs at all.
2. Finish + build-verify + commit the device-rack WIP (Open item 2).
3. Resume the plan of record (`lemon-study/PLAN-SUMMARY.md` + `docs/status.md`):
   rack parameter list/set/automate, MIDI FX editor UIs, engine note-path,
   `.flp` persistence of MIDI FX slots → M8 track model.
