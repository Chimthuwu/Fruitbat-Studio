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

- **The scenes run along the top** (8 of them, `launcher::SCENES`), one **▶** launch button
  each — Bitwig-style, scenes horizontally; the corner has **■ Stop all**.
- One **row per track, in the mixer's order** (`Track::mixer_strip`), each headed by the
  track's name, its colour, its own **■** stop button and, on a MIDI track with no instrument,
  an accent **!**.
- **The grid scrolls when it outgrows the window** (S3, finished): a scene column keeps at least
  `views::launcher::MIN_COL` px (a clip's name stays readable) and a track row `MIN_ROW` px, and
  what does not fit moves — the wheel scrolls the rows, **Shift**+wheel (or a sideways wheel) the
  scene columns (`Launcher::scroll_x` / `scroll_y`, `views::launcher::wheel`, clamped to the
  content). The scene buttons and the track column stay put while the cells scroll under them, so
  a track's name never leaves its clips, and a cell scrolled out of the grid registers no hit (it
  cannot take a click meant for the track column or the scene buttons).
- **Right-click a clip, a scene button or a track row**: its menu offers Rename..., Select in
  the playlist / Set current pattern / Clear clip, and a colour swatch (the crossed one takes a
  colour away). Renaming opens the playlist's inline text box over the grid. A clip's colour
  falls back to its track's; a track's name and colour are the arrangement's own
  (`playlist::track_color`, `EditTarget::TrackName`), so the header and the launcher agree.
- One cell per (track, scene). A cell holds a **pattern** — the project's own pattern, so the
  piano roll, the device rack and the mixer all see the same notes — plus a name and a colour
  of its own (`launcher::Slot`). A cell can hold **an audio clip** instead: see "Samples" below.
- **Samples** (dropped on a cell). Dragging an audio file from Explorer onto a cell
  (`App::drop_file` → `launcher::add_audio`) makes an `AudioClip` channel playing it
  (`playlist::audio_channel`, the same channel a drop on the arranger makes) and a clip in the
  cell that plays it (`Slot::audio`, `Slot::pattern` = 0: no pattern). Its length is the
  sample's own (`playlist::source_len`, so it follows the tempo), the shadow lays
  `ClipSource::Channel` clips on the track's lane, and the cell draws the sample's peaks behind
  its name. Double-clicking it plays it again (there is no pattern to open), and a recording
  into that cell replaces the sample with the take's pattern (`launcher::take_pattern`).
- **A cell records** (S4, finished). Its menu's `Record into this clip` (`Act::Record`,
  `launcher::record`) arms Record with the cell as the take's target (`Rec::slot`) and starts
  playback, so what is played lands in the cell: an empty cell gets a pattern and a clip of its
  own as the first note comes (`launcher::take_pattern` / `bind_take`) and shows the clip being
  recorded (ringed in the record colour, ● in its corner), and a cell that already holds a clip
  records into that clip's pattern. The take records from its own start, so the first note is at
  the clip's start; the clip's length is its pattern's (whole bars past the last note), so it
  loops at that length like any other. Stopping the track (`launcher::stop`) or the transport ends
  the take, and a take that recorded nothing leaves the cell as it was.
- **Left-click** a filled cell: that track lights up for the scene and the pattern starts.
  An empty cell stops the track. **Double-click** an empty cell: it gets a clip of its own (a new
  pattern, `launcher::make_clip`) and opens in the Piano Roll to be filled — a double click makes
  clips, as in Bitwig, rather than the menu. **Double-click** a filled cell: that clip's pattern
  opens in the Piano Roll (the first click launched it; the second opens it, as a double-click on
  a playlist MIDI clip does). **Right-click** a cell: its menu — launch settings, next action,
  rename, colour, `Set current pattern` (the current pattern goes in, or out if it is already
  there), `Clear clip`.
- **Launches are quantised** (S1, finished): every clip and every scene has its own
  `LaunchQ` — Off, 1/16, 1/8, 1/4, 1/2, 1 bar, 2, 4 or 8 bars — and `Q to Loop`. While the
  transport runs a click queues the clip for the next line of *its own* grid
  (`Launcher::pending`) and the cell is outlined until it lands; a launch while stopped starts at
  once, and so does one whose transport stops first (no line is coming).
- **Bitwig's launch parameters, Main and ALT.** A clip's menu carries `Quantize`, `Play mode`
  (from start / legato from clip / legato from project), `On release` (Continue / Stop / Return /
  Next action), `Q to Loop`, and the same three again as `ALT ...` for a controller's ALT gesture
  (`Act::LaunchAlt`). A click walks each row to its next value. Scenes carry the same sets plus
  **Override Launch Settings**: when it is on, the scene's settings, not the clips', are what its
  launches use.
- **Next actions** (S5, finished): a clip's menu sets a function — Stop, Return to arrangement,
  Return to last clip, Play next / previous / first / last / random / other, Round-robin, and the
  block functions (first / last / random / other / round-robin in block, first / random in the
  next, previous or other block) — and its timing (`NextTiming::Loops(n)` or `After(ticks)`).
  `launcher::next_actions` runs them from `App::tick`. Clips show a **→** mark when they follow.
- **Tracks play independently** (S2, done through §1.1 below): three tracks with three clips at
  three lengths run together, each looping at its own length.
- Launching starts the transport when it is stopped and plays the clips' shadow project
  (`launcher::shadow`, via `rack::play_project`); the transport loops the launcher's window.

### 1.1 How independent playback works without engine changes

The engine still plays one project, so the launcher hands it a **shadow project** instead of
teaching `os-engine` about live clips: `launcher::shadow` clones the played project, gives each
playing track a copy of its clip's pattern holding only that track's channel's notes, and lays
those copies along the track's lane at the clip's own length, from the tick the clip was launched
at, across the window. The project's real patterns are untouched, so the piano roll, the channel
rack and a save never see them; the transport loops `[0, window)` (`App::selection_loop`) so the
set never runs out. A cell holding a sample (`Slot::audio`) needs no copy: its lane gets
`ClipSource::Channel` clips of the sample itself, repeated at its own length.

- `window` is the **common multiple** of the playing clips' lengths, capped at 64 bars
  (`MAX_WINDOW_BARS`).
- A rebuild (`transport.update`) is what makes a launch audible, so only the track that launched
  changes row; `continue_from` keeps the voices of the others.
- **The limit:** at the window's end a clip launched part-way into a longer one re-aligns. Bitwig
  loops each clip on its own forever; ours shares one timeline whose length must serve every
  clip. Independent playback with no re-alignment is the engine work S2 describes — worth doing
  when something else needs the engine touched anyway.
- Tracks with no clip playing keep their arrangement clips in the shadow, so stopping a track
  (its **■** button) is Bitwig's "Stop clips": it returns to the arrangement.

---

## 2. The plan, in order

Each step should land on its own, green, with tests. `cargo test -p os-app` is currently
1124 passed / 0 failed / 62 ignored — keep it that way.

### S0 — Editing from the launcher (done)

Double-clicking a filled cell opens the clip's pattern in the Piano Roll (`launcher::open`,
`Act::Launch` twice within 450 ms, tracked by `App::launcher_click`), and a double click on an
empty cell makes the clip first (`launcher::make_clip`) and opens it — the grid's own gesture
for making clips. The rest is the slot menu (rename, colour, `Set current pattern`, `Clear
clip`, the launch sets, the next action), plus a drop of an audio file on a cell
(`launcher::add_audio`).

### S1 — Quantised launch (done)

A launch while the transport runs waits for the next line of the clip's **own** `LaunchQ`
(`launch` / `launch_scene` → `next_line`, which takes the grid's step from `grid_step`: the
quantization, or the window when `Q to Loop` is on). `launcher::advance` applies them at that
tick, when the transport stops first, or when the looping window wraps past the line (the
transport never reports a tick *at* its end). `LaunchQ::ticks` is built from
`Project::bar_ticks`, so the time signature is respected. The cell is outlined while it waits.

Still open: a pulsing rather than static outline.

Suhina spells this out: `crates/model/src/lib.rs` (`LaunchQuantization`), and
`crates/session/src/launch.rs` for the command surface.

### S2 — Independent playback per track (done, by the shadow; engine version open)

Ships as §1.1's shadow project: no engine change, one shared timeline. The engine version below
is what removes the re-alignment at the window's end, and is not needed for the launcher to be
a launcher any more.

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

The columns already follow `mixer_strip`, and the grid scrolls on both axes at a least column and
row size (§1). What is still missing when a set grows:

- ~~horizontal scroll for many tracks, and a minimum column width so names stay readable~~ done:
  in this grid a track is a row, so many tracks scroll up and down and many scenes sideways (8
  scenes, `MIN_COL` = 96 px, `MIN_ROW` = 22 px, wheel and Shift+wheel in `views::launcher::wheel`);
- grouping: a launcher column for a **bus** is currently skipped (a bus has no lane). Bitwig shows
  group tracks; decide whether a bus gets a column that only shows its return;
- the mixer itself should be able to show the same left-to-right order as the launcher when both
  are open — today the mixer orders by strip, which is what the launcher uses, so they agree;
  keep that invariant if either side ever gains reordering.

### S4 — Recording into slots (done for notes)

~~Arm a track, press record on a slot, play: the notes land in a new pattern bound to that cell~~
done: `Act::Record` / `launcher::record` arm the cell, `record::open_take` records into the cell's
pattern from the take's own start, and `record::rec_pattern_tick` keeps a launcher take out of the
arranger's clip placement. Still open:

- **audio** into a launcher slot (only notes and automation are captured; a sample dropped on
  a cell plays from it but is not recorded);
- recording **across loops**: the transport's loop wrap ends the take (one pass per take, as the
  arranger's takes already did), where Bitwig keeps layering passes into the same clip;
- take **comping** (several takes of one cell, picked between) — Suhina's
  `crates/session/src/recording.rs` / `record_writer.rs` are the pointer for that part;
- the clip the take makes is not launched by the end of the take: click it to hear it.

### S5 — Follow actions and scene chains (done)

Bitwig's whole next-action list (`NextAction`, 21 functions) with `NextTiming`, run by
`launcher::next_actions`; clip blocks are the runs of neighbouring filled slots on a track
(`launcher::blocks`). Random picks are seeded from the transport's bar, so a take repeats.
Still open: a scene's own tempo (`Scene::tempo_bpm`); the scene struct has name, colour, comment
and the launch sets today.

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
| `crates/os-app/src/launcher.rs` | `Launcher` (clips, playing, pending, scenes), `Slot` (pattern or `audio`), `Scene`, `Launch`, `LaunchQ`, `PlayFrom`, `OnRelease`, `NextAction`, `NextTiming`, `Act`, `columns`, `clip`, `playing`, `set`, `make_clip`, `add_audio`, `slot_len`, `launch`, `release`, `stop`, `launch_scene`, `stop_all`, `advance`, `shadow`, `window`, `run` |
| `crates/os-app/src/views/launcher.rs` | the grid: frame + title "Clip Launcher", track headers, scene buttons, cells (and a sample's peaks in one), the hint, and its layout and scroll (`geom`, `wheel`) |
| `crates/os-app/src/app.rs` | `Action::Launcher(Act)`, `Action::ToggleLauncher`, the **Tab** key, `State::launcher` / `State::launcher_open`, the right-click → `Act::Menu` route in `mouse_down_right` |
| `crates/os-app/src/record.rs` | `Rec::slot` and the take's `slot` / `created` / `start`: a launcher cell's take records into the cell's pattern from its own start, and ends with the cell's clip (`finish_slot_take`) |
| `crates/os-app/src/views/mod.rs` | `Win::Playlist` paints the launcher when `launcher_open` |

## 4. Testing habits this code base expects

- Model/unit tests live at the bottom of the module they test (`launcher.rs` has twenty-four).
- Paint tests make a `os_ui::Canvas`, call `app.paint(&mut c, None)` and read `app.hits` — that is
  how "a slot per track and scene" is asserted, and how a click is simulated
  (`mouse_move` → `mouse_down`).
- Build and test through git-bash with the env in `HANDOFF.md` §7; the default toolchain is MSVC
  and will not link this workspace.
