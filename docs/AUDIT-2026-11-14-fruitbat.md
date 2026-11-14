# Fruitbat Studios / Lemon Studio — full audit, 2026-11-14

This audit is written against the tree that became commit `cf52e2be` on
`chimthuwu/main` (parent `1d4c4091`, branch `modulation-m0`). It is deliberately
honest about the difference between **code that exists**, **behaviour covered by
tests**, and **behaviour a user can actually reach in the running app**.

Baselines used:

| Check | Result |
|---|---|
| `cargo test -p os-app --bin lemonstudio` | **1245 passed / 0 failed / 62 ignored** (before the final focus/palette edits; rerun recorded below) |
| `cargo test -p os-engine` | 385 passed + 5 integration (last recorded baseline) |
| `cargo test -p os-theme` | 28 passed |
| `cargo test -p os-plugkit` | 6 passed |
| Release build | `Z:\lemon-target\release\lemonstudio.exe`, Windows x64 |
| Placeholder controls | ~146 `.off()` / `Action::Placeholder` sites across `crates/os-app/src` |

The suite is a strong signal for model, persistence and paint logic. It is **not**
a GUI acceptance test: several bugs reported from the running app (launcher drop,
window stacking, plugin editor colours) were never covered by it.

---

## 1. What was fixed in this pass

### 1.1 Keyboard focus leaked between sections — FIXED (core)

**Report:** clicking the modulator strip and pressing Ctrl+A selected notes or
clips outside the strip. The same class of leak affected the device rack, which
kept Ctrl+B / Ctrl+C / Ctrl+X / Ctrl+V after a tile had been clicked once.

**Cause:** `App::key` dispatched section shortcuts by *selection state*, not by
the section that was clicked. The device-rack block ran before the piano roll's
block whenever `device_rack_sels` was non-empty, so a selected device swallowed
the note editing keys. The modulator block only ran when an editor, menu or
selection already existed, so Ctrl+A in an empty strip fell through to the piano
roll's global Ctrl+A arm.

**Fix (landed):**

- `KeyboardFocus` gained `Piano` (`crates/os-app/src/app.rs:68`), so the three
  editing surfaces (arrangement/mixer, device rack, modulator strip, piano roll)
  are explicitly tracked.
- `App::act` now maps every section action to its focus before dispatching
  (`app.rs:9209`): `Mod`/`Modulator` → modulators, `DeviceRack` → rack, piano
  actions → piano, playlist/mixer actions → tracks.
- The modulator strip registers a full-row `ModAct::FocusStrip` hit, so clicking
  empty strip space takes the keys too (`views/device_rack.rs`, `mod_row_paint`).
- `modulation::key` consumes Ctrl+A / C / X / V, Delete and the arrows **even
  when there is nothing to act on** (`crates/os-app/src/modulation.rs:1149`), so
  they cannot fall through to another section.
- The device-rack key block now requires `KeyboardFocus::Rack` (`app.rs:8449`
  area), and the piano-roll shortcuts use `App::piano_keys()`, which excludes the
  rack and modulator focus (`app.rs:8909`).
- Clicking a window in `raise_at_point` also moves the focus to that window's
  section (piano roll, channel rack, playlist/mixer).

This is the architectural fix the user asked for: **the section last clicked
owns Ctrl+A / C / X / V / B / D, Delete and the arrow keys**. It is not a
per-shortcut patch.

### 1.2 Duplicate aliases

- Piano roll: Ctrl+B and **Ctrl+D** both duplicate selected notes
  (`piano_tools.rs:166`). Ctrl+D previously cleared the selection.
- Device rack: Ctrl+B and Ctrl+D both duplicate selected devices
  (`device_rack.rs`, `key`).
- Playlist: Ctrl+B duplicates clips and Ctrl+D duplicates the selected track,
  unchanged.

### 1.3 Notes no longer stack on the same key — PARTIAL (intentional)

`piano::clear_overlaps` (`crates/os-app/src/piano.rs:653`) implements Bitwig's
rule: the note being drawn/moved/resized wins; an existing note on the same
channel+key is trimmed at its start, split around the new note with its tail
kept, or removed.

Where it is applied:

- Draw tool click and Paint tool strokes: always.
- Drag/resize release: yes, **except** a copied drag (Shift+drag or
  click-and-hold). Those copies intentionally start overlapping their original
  one snap step later, and trimming the original would be destructive. This is
  why hold-clone tests still pass.
- Paste: not applied. Pasting onto a selected region intentionally re-creates
  the selected material; `paste_tick` already shifts a paste that would land on
  an *unselected* note.

Verdict: newly drawn notes never stack. Moving one note over another trims the
one underneath. Copies remain allowed to overlap their source, which is a
deliberate exception rather than a missing case.

### 1.4 Pattern clips no longer stack on one track — PARTIAL

`playlist::clear_pattern_overlaps` (`playlist.rs:2098`) runs at drag release for
moved/resized clips. The dropped clip wins; the overlapped pattern clip is cut,
its tail preserved, or removed. Audio and automation clips are deliberately
excluded because their overlaps are the automatic crossfades.

This covers MIDI/pattern clips in the arranger. It does **not** cover:

- clips dragged in the launcher (there is no clip-move gesture there at all);
- pasted clips (paste can still overlap);
- audio/automation clips (by design, crossfades).

### 1.5 Popped-out windows

Detached app windows and detached plugin editors are now native top-level
windows set to **AlwaysOnTop** (`detached_win.rs:84`, `plugin_window/win32.rs`
`CreateWindowExW` with `WS_EX_TOPMOST`, and `set_detached` switching
`HWND_TOPMOST`/`HWND_NOTOPMOST`). They can no longer sink under the docked
device rack.

**Colour:** plugin editors cache the app palette. A skin/theme change published
the palette but did not repaint already-open editors. `Plugins::sync_palette`
(`plugins.rs:683`) now compares the published palette each tick and calls
`redraw()` on every open plugin window (`win32`, `cocoa`, `x11` implementations),
so a popped-out Lemon Trio follows the current skin after the next tick.

**Remaining limitation:** modulation arcs/targets are drawn by the device rack
tiles, not inside a native plugin editor. A popped-out editor can therefore be
used and themed, but modulation assignment still happens on its rack tile, not
on the plugin's own bitmap. Native VST/CLAP editors cannot be overlaid by the
host in general; a built-in-editor-in-app solution would be needed.

### 1.6 Clip launcher keyboard navigation — LANDED (new)

The launcher had mouse-only editing. `launcher::key` (`launcher.rs:506`) now
implements Bitwig-style navigation: arrows walk cells, Home/End jump scenes,
Enter launches the focused clip, Delete clears it. The focused cell is ringed and
`views::launcher::reveal` scrolls it into view. Mouse selection syncs the
keyboard cursor.

---

## 2. Area-by-area: what has landed, what has not

### 2.1 Desktop shell, windows and layouts

| Status | Detail |
|---|---|
| ✅ | Main window, toolbar, menu bar, browser, settings pages, dialogs, winit event loop, softbuffer/CPU rendering. |
| ✅ | Child windows (playlist, mixer, piano roll, channel rack, plugin editor, container, tool windows) share one stacking order; raise-on-click works. |
| ✅ | Options ▸ Detached for the mixer and channel rack; detached frames persist in `settings.txt`; plugin editor detach/dock. |
| ✅ | Popouts are hard-detached native, always-on-top windows now. |
| 🚧 | **The piano roll is still a movable/resizable floating window.** It opens maximized (`arrange::default_wins`, `wins[2].maxed = true`) and has a title bar, drag and resize zones. The user asked for it to stop behaving like a floating window; that layout change (docked bottom panel vs. maximized overlay) has **not** been made. |
| 🚧 | Window layout is hard-coded around a 1920×1080 workspace; small screens rely on clamping. |
| ❌ | No per-monitor DPI/window-placement management beyond `ui_scale`/`win_clamp`. |

### 2.2 Clip launcher

| Status | Detail |
|---|---|
| ✅ | Bitwig-style scene columns + track rows; clips map to patterns; per-track and per-scene launch/stop; launch quantization, play mode, on-release, next action and ALT settings are modelled. |
| ✅ | Audio clips in cells (`launcher::add_audio`); double-click empty cell creates and selects a clip without opening an editor, double-click a filled cell opens it in the piano roll / clip editor; record-into-cell. |
| ✅ | Multi-selection (plain / Ctrl+click / Shift+click / Ctrl+A) and bulk copy, paste, duplicate and delete (`duplicate_selection`, `copy_selection`, `paste_selection`, `clear_selection`), each one undo step through `history::Snap::Launcher`. Duplicate leaves the originals untouched and makes independent patterns. |
| ✅ | Per-track Stop / Stop all and header mute/solo now rebuild the playing project (remaining clips, arrangement layer or silence); the master Stop stops every launcher clip and queued launch as well as the arrangement. |
| ✅ | Clip/scene/track rename and colours; per-clip play/stop button; progress bar; queued/recording rings; drag-and-drop from OS to a cell has a code path (`App::drop_file`). |
| ✅ | Keyboard navigation (new in this pass), including Ctrl+A/C/V/D and Delete for the selection. |
| 🚧 | **Track headers are not at playlist parity:** no volume fader, no pan fader; only mute/solo LEDs, name, colour and stop. This is roadmap items F2–F4 and remains open. |
| 🚧 | **Clips cannot be dragged between cells or from the arranger into the launcher.** Copy/paste and duplicate now work, but pointer drag is still missing. |
| 🚧 | Drop-from-OS has a hit-test path but has repeatedly been reported as not landing in the live app (roadmap priority 2); this audit did not run a live DnD trace. |
| 🚧 | Scene follow actions/next actions are modelled but there is no dedicated visual editor beyond cycling menu values. |
| ❌ | **No launcher persistence:** the grid lives only in `State::launcher`, so saving and reopening loses it (`docs/clip-launcher.md` S6). |
| ❌ | No launcher follow/legato mode beyond the implemented play-mode fields; no scene tempo/time-signature override. |

### 2.3 Modulation system

| Status | Detail |
|---|---|
| ✅ | Graph model (`os-model::modulation`), persistent parameter identity, `.lemon` persistence, 8 project slots expanded by rows, LFO/Random/Audio Follower sources. |
| ✅ | Engine evaluation (`os-engine::engine::modulation`), host parameters (fader/pan/separation) and plugin parameters; live snapshot drives tiles and plots. |
| ✅ | Audio Follower's low band is now real (2026-11-14): its two cascaded high-pass stages had shared one `prev` sample; each stage keeps its own history now. |
| ✅ | Routing mode, drag-to-assign, depth arcs, fader ranges, browser Modulation section, multi-select (Shift+click, Ctrl+A, Delete), copy/cut/paste, editor window. |
| ✅ | Modulation survives engine rebuilds (`transport.rs` re-installs graph + snapshot in `swap`); New Project clears the graph and selection. |
| ✅ | Keyboard isolation of the strip (this pass). |
| 🚧 | **Native popped-out plugin editors do not show modulation overlays.** Assignment works on the rack tile only. |
| 🚧 | Detached modulator editor is always-on-top and native; it cannot be docked back into the rack. |
| 🚧 | The browser's Modulation section exists but is not a full graph editor (no routing curve editing there). |
| ❌ | No MIDI/MPE modulation sources, no per-voice modulation. |

### 2.4 Device rack

| Status | Detail |
|---|---|
| ✅ | Docked at the bottom in Linear mode; signal-flow order MIDI FX → instrument → audio FX; tiles draw each device's own controls; scrolling; parameter groups; enabled/bypass; drag to reorder; mixer-slot re-targeting. |
| ✅ | Per-device pop-out/dock, editor window, container window, device selection and clipboard (Ctrl+C/X/V/D/B/G), Delete; modulation targets tint and accept drags. |
| ✅ | Device editor popouts are native and now always-on-top; palette changes repaint open editors. |
| 🚧 | No inline device rename (roadmap item). |
| 🚧 | Docked editor fit for long devices is still a known issue (Trio/Sampler/Slicer/EQ fader bank) — `Dock UI fitting` roadmap items are open. |
| 🚧 | MIDI FX devices are hosted in the UI but the engine note path does not call them (C3 blocked on M9). |

### 2.5 Piano roll

| Status | Detail |
|---|---|
| ✅ | Full editing toolset: Draw, Paint, Select, Delete, Mute, Slice, Zoom, Playback; velocity/event panel; note properties; quantize/chop/legato/glue, strum, arpeggiate, flam, flip, levels, limit, LFO, randomize, claw; snap, scrollbars, zoom, keyboard shortcuts. |
| ✅ | F7 opens/focuses it; note selection/copy/paste/duplicate; focus now owns its shortcuts (this pass). |
| ✅ | New notes/drag/resize no longer stack on the same key (this pass). |
| 🚧 | Still a floating/movable window, not a docked editor panel. |
| 🚧 | Piano roll keyboard/mouse focus is now scoped, but the window itself still overlaps the arrangement and device rack by design. |

### 2.6 Arranger / playlist

| Status | Detail |
|---|---|
| ✅ | Pattern, audio and automation clips; drag/move/resize/slip/slice/erase/mute/paint; groups; multiple arrangements; markers; time selection; zoom; clip colours. |
| ✅ | Pattern-clip overlap trimming on drop/resize (this pass); audio crossfade overlaps preserved. |
| 🚧 | Paste, duplicate and some automation edits can still create overlaps. |
| 🚧 | Automation is clip-based in Classic mode; Linear-mode automation lanes are partially implemented. |
| 🚧 | No clip-launch integration from the arranger (drag to launcher cell missing). |

### 2.7 Mixer

| Status | Detail |
|---|---|
| ✅ | Strips, faders, pan, sends, inserts, meters, mute/solo, routing, sidechain, EQ graph, glitch strip, cable view, channel inspector, detached mixer. |
| ✅ | Console visuals and hardware-style knobs; per-track strip styles; mixer keyboard shortcuts. |
| 🚧 | Not every strip control is scriptable/automated; some EQ/limiter controls remain read-only UI. |
| ❌ | No full console channel-strip parity with Bitwig (per-band dynamics, etc.). |

### 2.8 Browser

| Status | Detail |
|---|---|
| ✅ | Tabs, tree, search, folders, drag to playlist/rack, current-project tree, modulation section, reorder. |
| 🚧 | Some editorial rules and preview metadata are still incomplete; no audio audition of every item type. |

### 2.9 Plugins and built-in instruments/effects

| Status | Detail |
|---|---|
| ✅ | CLAP/VST3/VST2 hosting, editors, presets, crash containment, scan cache, plugin manager. |
| ✅ | Built-ins: Lemon Sampler, Trio, Citrus, Drum Sampler, Slicer, Tesla and the effect suite. |
| ✅ | Plugin editors follow the app palette, including open/popped-out editors after a skin change (this pass). |
| 🚧 | Native editor UIs cannot host our modulation overlay. |
| 🚧 | Pop-out colour matches only after the next tick/repaint; a window that is not ticking during a frozen UI may show the old palette briefly. |

### 2.10 Project I/O, engine, export

| Status | Detail |
|---|---|
| ✅ | FL Studio `.flp` read/write with lossless round-trip, `.lemon` extension for fork state, MIDI import/export, WAV/FLAC/MP3/OGG export, stems, offline render. |
| 🚧 | Known file-format risks from the 2026-11-04 audit are still open (ANSI/Latin-1 text, silent short-record writes). |
| 🚧 | Engine RT hardening (SIMD, sinc cost, allocations on audio thread) remains a quality-audit item, not a feature regression. |

### 2.11 Settings, themes and skins

| Status | Detail |
|---|---|
| ✅ | All Settings pages exist; theme library, `.flstheme`, live adjustments, colour picker, PPI, shortcuts, MIDI/audio/recording/plugin pages. |
| ✅ | Four skins exist: Meridian (default), Lemon, Lemon Industrial, Citrus. `Design::ALL` lists all four; `THEME_SKINS` lists the three with own-colour entries. |
| 🚧 | **Settings ▸ Theme exposes only Lemon and Citrus** (`Design::SETTINGS_SKINS`). Meridian and Industrial are reachable from other surfaces but not that page. If the intent is "all four in Settings", this is a gap. |
| 🚧 | Plugin frames are themed; the plugin editor's own rendering follows the published palette, with the limitations above. |

### 2.12 Placeholders and unfinished controls

There are roughly **146** greyed-out placeholder/`.off()` sites. They are
intentionally visible and say "not available yet" instead of silently doing
nothing. The largest clusters are:

- toolbar/menu utilities and layout commands;
- plugin window Misc pages and some strip controls;
- settings pages' advanced options;
- launcher/device-rack polish items listed above.

This is healthy for a pre-alpha, but it is also the main source of "the UI has a
button that does nothing" reports.

---

## 3. Cross-cutting audits still valid

The 2026-11-04 code audit (`AUDIT-2026-11-04.md`) remains the reference for:

- CPU rendering / per-frame allocation costs and the 4K ceiling;
- hard-coded layout assumptions;
- engine RT-thread spikes and unsafe multicore;
- FLP text encoding and silent short-record writes;
- plugin-host pointer/queue hardening.

None of those were addressed in this pass; they are quality/robustness work, not
feature wiring.

---

## 4. Prioritized next steps

1. **Piano roll window model.** Decide docked bottom editor vs. native detached
   popout, then remove the floating title-bar drag/resize path. This is the one
   reported UI behaviour still untouched.
2. **Launcher drag/move.** Add a launcher `Act`/`Drag` for moving and copying
   clips between cells, and from the arranger into an empty cell.
3. **Launcher track headers.** Volume/pan faders and the remaining playlist
   header controls (roadmap F2–F4).
4. **Modulation in popouts.** Either add a host-drawn parameter overlay for
   built-in editors in their own windows, or document rack-tile-only assignment
   as the supported path.
5. **Settings skins.** Show all four skins in Settings ▸ Theme, or remove the
   `SETTINGS_SKINS` restriction and update the theme list.
6. **Live DnD trace.** Run the launcher drop path against a real window; unit
   tests alone have not caught the reported failure.
7. **RT hardening + FLP data-integrity fixes** from the 2026-11-04 audit.

---

## 5. Verification for this audit

Commands run from `Z:\github\Lemon-Studio`:

```bash
source ./env.sh
export OPENSTUDIO_NO_PLUGIN_BUILD=1
cargo test -p os-app --bin lemonstudio
cargo build --release -p os-app --bin lemonstudio
```

The release binary is `Z:\lemon-target\release\lemonstudio.exe`. The previous
running instance had to be renamed aside because Windows locks the executable
while it is open; close the app before rebuilding next time.

---

## 6. Addendum, 2026-11-14 (launcher correctness + follower DSP)

This pass implemented and tested the following (working tree, not committed at the time of writing):

- **Launcher selection**: `Launcher::selected` / `anchor` / `clipboard`; plain click selects one cell,
  Ctrl+click toggles, Shift+click ranges, Ctrl+A selects every clip. Ctrl/Shift clicks do not launch.
- **Bulk edits**: Ctrl+C / Ctrl+V (relative layout, independent patterns), Ctrl+D (copies one
  selection-width to the right, originals untouched), Delete clears the selection. Each is one undo
  step via `history::Snap::Launcher`; `App::undo_key` rebuilds the engine project when an undo changes
  a playing grid.
- **Engine sync**: `launcher::stop` / `stop_all` and the header mute/solo actions rebuild and push the
  playing project. Stopping the last clip keeps the transport clock running (later launches still
  quantise) and silences the launcher, or hands back to the arrangement when its layer is on. The
  master Stop now stops every launcher clip and queued launch too (`master_stop_stops_launcher_clips`).
- **Double-click**: an empty cell creates and selects a clip without opening the piano roll; the clip
  menu's **Open clip** opens a pattern in the piano roll or an audio clip in the clip editor.
- **Audio Follower**: the two cascaded high-pass stages shared one previous sample; each stage now has
  its own (`hp_prev: [f32; 2]`), with a 5 Hz vs 100 Hz low-band regression test.
- **Modulator tiles**: every populated tile now draws a visible `×` remove button beside its enable LED
  (the menu and Delete already worked).

Verified in this pass:

```bash
source ./env.sh
cargo test -p os-app --bin lemonstudio            # 1248 passed, 0 failed, 62 ignored
cargo test -p os-engine followers_low_band        # 1 passed
```

Still not done: launcher persistence (`docs/clip-launcher.md` S6), pointer drag between cells, full
playlist-style headers, and a runtime DnD trace. The engine build/transport wiring was verified only
through unit tests and the model's project snapshot; no audio device was run for this addendum.
