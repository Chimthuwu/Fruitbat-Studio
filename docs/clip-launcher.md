# The clip launcher — what works, and the plan

Written 2026-11-08, after the first landing. Read this before touching
`crates/os-app/src/launcher.rs`, `crates/os-app/src/views/launcher.rs`, or anything in
`crates/os-engine` about launching clips.

Reference implementation to borrow from: `../lemon-study/refs/suhina-daw` (a clean-room,
GPL-3.0-or-later clip-launching DAW in Rust — the same licence as this repo). It is not part of
this workspace; read it there.

---

## 1. What ships today

**Tab** flips the Playlist window between the Arranger and the Clip Launcher. The launcher keeps
the window's own frame, so it sits exactly where the arrangement was.

- Scenes down the left edge (8 of them, `launcher::SCENES`), each with its own **▶** launch
  button; the corner has **■ Stop all**.
- One **column per track, in the mixer's order** (`Track::mixer_strip`), each headed by the
  track's name, its strip colour and its own **■** stop button, so a column sits over the strip
  that plays it.
- One cell per (track, scene). A cell holds a **pattern** — the project's own pattern, so the
  piano roll, the device rack and the mixer all see the same notes.
- **Left-click** a filled cell: that track lights up for the scene and the pattern starts.
  An empty cell stops the track. **Right-click** a cell: the current pattern goes in, or out if
  it is already there. **Double-click** a filled cell: that clip's pattern opens in the Piano
  Roll (the first click launched it; the second opens it, as a double-click on a playlist MIDI
  clip does).
- Launching switches the transport to PAT mode at the clip's pattern (song playback would
  ignore the launcher) and starts the transport when it is stopped.

### The honest limit

**Tracks do not play independently yet.** The engine plays one pattern at a time, so launching
"drums/scene 2" while "bass/scene 1" loops does not stack them: the grid records what each track
*is* playing (`Launcher::playing`) and the newest launch wins on the air. That is the first thing
§2 fixes.

---

## 2. The plan, in order

Each step should land on its own, green, with tests. `cargo test -p os-app` is currently
1089 passed / 0 failed / 62 ignored — keep it that way.

### S0 — Editing from the launcher (done)

Double-clicking a filled cell opens the clip's pattern in the Piano Roll (`launcher::open`,
`Act::Launch` twice within 450 ms, tracked by `App::launcher_click`). Nothing else about the
clip is editable in place yet: a right-click holds only "the current pattern, or nothing". A
real slot menu (rename, duplicate, colour, delete) belongs with S4/S5.

### S1 — Quantised launch (S)

Today a launch starts the pattern at once. Bitwig launches at the next **bar** (its default
quantisation). Needs the transport's tick position where the launch is pressed, which
`Transport`/`Engine` already track.

- Add `LaunchQuantization { None, Beat, Bar, Scene }` to `launcher.rs`, with a per-clip setting
  (`Slot.quantize`) and a grid-wide default.
- A launch while stopped starts at once; a launch while running records
  `Launcher::pending: BTreeMap<u32, (usize, u64 /* tick */)>` and applies at the boundary.
- Draw pending cells with a pulsing border (the view already has `playing` state to copy).

Suhina spells this out: `crates/model/src/lib.rs` (`LaunchQuantization`), and
`crates/session/src/launch.rs` for the command surface.

### S2 — Independent playback per track (L) — the real feature

This is the engine work, and it is the step that makes the launcher a launcher.

`crates/os-engine/src/engine.rs` renders from `self.triggers` (`next_trigger`,
`notes_under_way`, `run_controls`, `seek_tick`). Triggers are built from the arrangement's clips.
A live clip is a second source that must not disturb the arrangement's:

1. `Engine::set_live_clip(track, Option<LiveClip>)` where
   `LiveClip { pattern: u16, channel: u16, start_tick: u64, length_ticks: u64, quantize }`.
2. Keep live triggers in their own `Vec<Trigger>` per track, rebuilt when a launch lands, and
   merge them into the per-tick dispatch (`notes_under_way` is the place that already walks
   `triggers[..next_trigger]`; a live clip loops, so its triggers are generated for the window
   `[start_tick, start_tick + length)` and regenerated when the window passes).
3. A launch that replaces a clip must **stop** the old one's notes (the engine has `choke` /
   `note_off` machinery; `Trigger.cut` is how the arrangement ends overlapping clips).
4. Pointer for the shape: `../lemon-study/refs/suhina-daw/crates/engine/src/launcher.rs`
   (`LaunchCommand::{LaunchClip, ReleaseClip, StopColumn}`, and its "pending jump" idea, which is
   the same quantisation problem as S1).

Risks: `seek_tick`, PDC (`self.pdc`) and layer containers (`start_notes`) all assume one trigger
list; the MIDI FX note path (see `HANDOFF.md` §6) also inserts itself before the instrument and
must be ordered *live clip → MIDI FX → instrument*.

### S3 — Mixer alignment (S/M)

The columns already follow `mixer_strip`. What is missing when a set grows:

- horizontal scroll for many tracks (the device rack's `device_rack_scroll_x` is the pattern to
  copy), and a minimum column width so names stay readable;
- grouping: a launcher column for a **bus** is currently skipped (a bus has no lane). Bitwig shows
  group tracks; decide whether a bus gets a column that only shows its return;
- the mixer itself should be able to show the same left-to-right order as the launcher when both
  are open — today the mixer orders by strip, which is what the launcher uses, so they agree;
  keep that invariant if either side ever gains reordering.

### S4 — Recording into slots (M/L)

Arm a track, press record on a slot, play: the notes land in a new pattern bound to that cell
(`Pattern` creation already exists; `crates/os-app/src/record.rs`). Suhina:
`crates/session/src/recording.rs`, `record_writer.rs`.

### S5 — Follow actions and scene chains (S)

Per clip: `FollowAction { None, Next, Previous, First, Last, Stop, Random }` and a count, run when
a loop finishes (only meaningful once S2 loops live clips). Suhina:
`crates/session/src/follow.rs`, `crates/model/src/lib.rs` (`Scene::tempo_bpm` — a scene can also
carry a tempo switch).

### S6 — Persistence (M)

The launcher lives only in memory (`State::launcher`), like the MIDI FX slots. The `.flp` format
has no launcher; two options, decide before writing code:

- store it as arrangement **markers + clips** (a launcher scene becomes a marker, a clip becomes a
  normal clip placed after the arrangement's end) — lossless for FL round-tripping, ugly;
- a sidecar file next to the project (`<project>.launcher.json`), keeping `.flp` untouched.

Suhina's `Project.launcher` (`scenes`, `columns[].slots[].clip`, `mappings`) is the model to copy
whichever storage wins.

### S7 — Launch by MIDI / key (S)

`LaunchMapping { key | midi note, target }` — Suhina has it in `crates/model/src/lib.rs` and
`crates/session/src/mapping.rs`. Cheap once S1–S2 settle.

---

## 3. File map

| file | what |
|---|---|
| `crates/os-app/src/launcher.rs` | `Launcher` (clips, playing, scene names), `Slot`, `Act`, `columns`, `clip`, `playing`, `set`, `launch`, `stop`, `launch_scene`, `stop_all`, `run` |
| `crates/os-app/src/views/launcher.rs` | the grid: frame + title "Clip Launcher", track headers, scene column, cells, the right-click hint |
| `crates/os-app/src/app.rs` | `Action::Launcher(Act)`, `Action::ToggleLauncher`, the **Tab** key, `State::launcher` / `State::launcher_open`, the right-click → `Act::Set` route in `mouse_down_right` |
| `crates/os-app/src/views/mod.rs` | `Win::Playlist` paints the launcher when `launcher_open` |

## 4. Testing habits this code base expects

- Model/unit tests live at the bottom of the module they test (`launcher.rs` has six).
- Paint tests make a `os_ui::Canvas`, call `app.paint(&mut c, None)` and read `app.hits` — that is
  how "a slot per track and scene" is asserted, and how a click is simulated
  (`mouse_move` → `mouse_down`).
- Build and test through git-bash with the env in `HANDOFF.md` §7; the default toolchain is MSVC
  and will not link this workspace.
