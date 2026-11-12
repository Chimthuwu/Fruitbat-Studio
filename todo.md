# Lemon Studio — todo.md (roadmap reference)

## ✅ DONE 2026-11-12 — the glitcher: a second strip style per track
- [x] **The strip's de-esser is a Filtertable-style table** (`e8eb539b`): sixteen constant-Q band-passes summed over the
      dry signal, their gains a hand-drawn curve that drifts through the spectrum (12 s a cycle); the knob scales them
      and is the on/off, so 0 is sample-exact. The de-esser DSP is gone.
- [x] **The glitch grid** (`8beb7783`): `os-dsp/src/glitch.rs`, sixteen steps on their own clock (125 ms), each firing
      STUT / CRSH / RVRS / GATE / RING from an 8192-frame ring, with `Pattern { steps: [u8; 16], amount }`.
- [x] **The glitch mixer itself**: every track chooses Analog or Glitch (`MixerTrack::glitch_style`, `glitch`,
      `glitch_amount`); the engine builds both and runs the chosen one, and the strip's chunk is version 8 (4 and 7
      still read, 5 and 6 are the fork's SnapTune ones). The console's style bar swaps the analog sections for the grid:
      a 4×4 of coloured cells, a click cycling a step's module, and one amount knob — no knob per module. Right-click
      menu: Strip style, Amount, Clear grid. `KnobId::Glitch` is scriptable as `glitch:<track>`.
- [x] **Mixing modules from both strips, P1 and P2** (`docs/mixed-strips-plan.md`): `MixerTrack.chain: [u8; 8]` — the
      six analog sections and the grid (7), in signal order, with `CHAIN_ANALOG` / `CHAIN_GLITCH` the two whole strips and
      `chain_add` / `chain_remove` / `chain_move` beside them. `Strip::run_section` runs one section over a block (a chain
      of the six in the edition's order renders the strip bit for bit — `a_chain_of_the_sections_matches_the_strip`), the
      engine walks the chain with the two whole strips keeping their old paths, and `Engine::glitch_steps` reports a grid
      wherever it sits in a chain. A mixed chain rides in its own `LSMX` chunk (version 1, index + eight ids), so a build
      from before mixed strips still reads every track's strip. Menu: Track ▸ Modules with the two presets and, per
      module, add / take out (coming in, a zero word takes a musical default) and move earlier / later — one undo step
      each (`MCmd::Chain`, `MCmd::ChainModule`).
- [ ] **P3, the chain console**: the style bar becomes a chain bar of module chips and the panel draws one tile per
      chain slot (the grid's 4×4 among them), paged when the chain is taller than four rows; a mixed chain also needs a
      chain-level bypass fade (today the bypass switch is hard on a mixed chain). The plan's §"Console" has the layout.
- [x] **A playhead on the grid, and painting it**: the feeder publishes each track's step (`Engine::glitch_steps`),
      `MixerUi::steps` carries it and the active cell is ringed in cream, repainting only the grid (`Live::Glitch`).
      Holding a cell down drags its module across the cells the pointer crosses (`App::Drag::GlitchPaint`,
      `glitch_at` reading the grid from any cell's rect), in the press's one undo step.
- [x] **Named patterns for the grid** (`crate::glitch_patterns`): Glitch Strip ▸ Pattern ▸ Save pattern as... writes the
      sixteen steps and the amount to `Glitch patterns` in the user folder as `<name>.osglitch` (JSON, module numbers as
      `MODULES` numbers them), and the submenu lists every one there by name — `(no saved patterns)` greyed when there
      are none. Taking one puts the grid and its amount back, switches the glitcher on, and is one undo step
      ("Load glitch pattern"); a hand-edited file loads too (a module past the list is clamped, a short one padded).
- [x] **The five modules tuned on audio**: each measured on a 1 kHz tone at -6 dBFS (RMS, peak, the largest frame-to-frame
      jump, DC). Every step that changed module, and every stutter repeat, was a splice with no fade — its boundary stood
      up to 22 dB out of the signal's own steps, the loudest thing the grid did — so each now crossfades in over 1.5 ms
      from the frame before (a linear crossfade, not a chase: chasing moved fastest a third of the way in). STUT's
      repeats went from -7.6 to -29 dBFS, RVRS from -12.7 to -29.4. CRSH's top is 3 bits instead of 2 (a fizz, not a
      buzz; its own hold steps -8.5 dBFS, down from -6.0), and GATE leaves an eighth of each step sounding instead of a
      twentieth (-9.3 dB at full, was -13.4). RVRS and RING needed nothing beside the crossfade; RING's 3 dB at full is
      what a ring mod does. Tests: `a_stutter_step_repeats_itself` (seams under -40 dBFS and the quarter still repeats),
      `a_gate_step_leaves_an_eighth_open`, `a_step_that_changes_module_fades_in` (a splice is no wider than the step's
      own module), and the empty grid still passes through sample for sample.

## ✅ DONE 2026-11-12 — strip channel: gate → OTT; piano roll resizes
- [x] **The strip's gate is a one-knob OTT** (`962b8418`): three bands at 250 Hz / 2.5 kHz, 8:1 down above -30 dB, 4:1
      up below it (+18 dB at most), the depth on word 3 and doubling as the dry / wet mix; the fork's gate code is gone.
      Console row label, the knob's text ("OTT: 40%"), the right-click submenu and the mixer/knob tests follow.
- [x] **A maximized window resizes from its edges** (`621a6e9a`): the piano roll opens maximized and so had no resize
      edges at all; dragging one now puts the window back on its floating rect and resizes from there.
- [x] ~~Soothe-2 style dynamic EQ; ANINA.vst3 / EB-FreakyTable~~ — **scrapped by the user 2026-11-12** ("forget the
      Soothe2 Anina EBFreakyTable shit"): TAME stays as it is, and those plugins are not something this project takes on.

## 🔝 TOP OF LIST — Modulation system (UN-PAUSED 2026-11-12, in progress)
- [x] **M0 persistent parameter identity is DONE & shipped** on `origin/modulation-m0` (milestones 1+2, 16 tests green:
      7 os-model + 9 os-app).
- [x] **M1a, the modulation graph model and its `.lemon` persistence** (2026-11-12, no engine wiring): `os_model::modulation`
      gains `ModGraph` / `Modulator` / `ModTarget` beside the M0 identity types — a modulator is one `ModSource`
      (`Lfo` with `LfoShape` + `Timebase`, `Steps` with sixteen `i8` values, or `Envelope` attack/decay/sustain/release),
      a user name, an enabled flag, and a list of targets. Each target is a `ParameterPath` (the M0 identity) with a
      `depth` (clamped to ±1, 25 % by default), a `Curve` (linear / exp / log / S) and a `Polarity` (bipolar / unipolar).
      One modulator drives a parameter at most once; several modulators may drive the same one (`ModGraph::assign`,
      `unassign`, `targets_on`, ids minted from `next` and never reused). The graph is saved as `LMex` event 207
      (`lemon::MODULATION`, layout 1: graph next + per modulator id / enabled / name / source / targets, parameter keys as
      native id, legacy index or host code), read back in `lemon_state::apply`, and always written so a removed graph
      leaves no stale one; `State::modulation`. `modulation::unresolved` lists the assignments that do not resolve
      against the current project and plugin metadata (a missing native id, a device that went) instead of dropping
      them. Tests: ids never reused, one assignment per parameter per modulator, depth clamping, event round trip,
      `.lemon` round trip (and the FLP dropping it), truncated payloads, unresolved reporting.
- [ ] **M1b, the app→engine update path** (Section K K.4 / K.5 §7 scoped this as an M1 decision): evaluate the sources on
      the engine's clock and write the values through the existing target path — no FLP `Target` bytes change.

This file tracks the open/executed state of the todo items. The canonical priority plan is in `docs/roadmap-2026-11-10.md` — reference that doc for the full rationale, audit findings, and ordering.

- [ ] **Docked knobs: user-set labels and colours** (2026-11-12): a tile header's channel knobs wear fixed words and
      colours (VOL green, PAN blue, PITCH orange with its semitones on the face). Let the user rename a knob and pick
      its colour — saved with the project, or as a view setting.

## 🎯 PLANNED — "Range" removed from every device (plan only, NOT started)
Requested 2026-11-12, right after the docked-strip work: the RANGE box goes away everywhere (the plugin windows' strip
and anything mirroring it). The pitch knob's range becomes fixed at 48 semitones: centre = 0, hard left = -48 st, hard
right = +48 st. This is broad (plugin strip, channel model, FLP save/load, docked tile strip, specs) — plan it before
writing code.
- [ ] Audit and plan: `plugin_window/mod.rs` (`Knob::Range`, `RANGE_MAX`, `range_box`/`range_rect`, `Part::Range`,
      `FrameEvent::PitchRange`, `ChannelInfo::range`), `app.rs` (`Request::PitchRange`, the pitch clamp),
      `os_model::Channel::misc.pitch_range` + its FLP read/write, the channel rack's pitch-range docs, and the new
      docked tile strip (`views/device_rack.rs::strip_paint`, which maps PITCH through `ch.misc.pitch_range`).
- [ ] Then make pitch a plain -4800..+4800 cent knob (0 = centre) and stop storing a range in new projects; old files
      still load, the stored range simply stops being read.
- [ ] Update the specs/docs and tests that mention the range.

## 🎯 NEW — Clip Launcher polish (2026-11-09 23:12, plan only — DO NOT EXECUTE yet)
Reported after testing the fd73cb54 build. Needs a short audit pass (launcher cell rendering + hit handling,
views/launcher.rs + launcher.rs) before executing — the F1 drop fix from wave 2 did NOT hold in practice.
- [ ] **Drag & drop of audio samples into launcher cells still doesn't work** — no clip appears. Wave 2 claimed
      a fix with a drop_file regression test, but live behaviour is unchanged: re-audit the whole chain (OS drop
      reaching the launcher window → cell hit-test → drop_file's launcher branch → add_audio) with a runtime
      trace, not just unit tests. Check whether the drop actually lands in the launcher window at all.
- [ ] **Clips hard to see / no play-state affordance.** Each occupied clip cell gets a little play/stop button in
      its top-right corner reflecting that clip's play state (playing / stopped / queued — Bitwig-style).
- [ ] **Launcher tracks should match playlist tracks:**
      1. Track headers/lanes fill with the track colour entirely, like the playlist.
      2. Same header controls as playlist tracks (mute/solo/etc. — audit the playlist header control set).
      3. A volume fader on each launcher track, like playlist tracks.
- [ ] **Pan fader on ALL tracks** (playlist and launcher tracks alike) — new control; audit the track model/mixer
      for where pan lives (os-model Track + mixer path) and both header layouts.
- [ ] **Launcher track Mute / Solo buttons do nothing** (reported 2026-11-12): audit the launcher's track-header
      hits and actions against the playlist's headers, then wire them.
- [ ] **Launcher tracks must look identical to playlist tracks** — same colour fill, same header control set, same
      text: share the playlist's track-header drawing instead of keeping a parallel one in views/launcher.rs.

## 🎯 ACTIVE — Dock UI fitting + rename (Trio + Lemon Drum Sampler ported; Slicer deferred)
All in the docked device-rack tile UI (rack_ui.rs) and the kit editors. Needs a short audit pass first to pin
exact file:line refs (rack_ui.rs size/fit logic, kit/eq.rs, kit/lemon.rs, kit/inst.rs).
- [x] **Trio + Lemon Drum Sampler docked (2026-11-12, 9a8fb7f9).** Trio = full editor via host-parameter bridge;
      Drum = 16 pads + flip button for its cards. Follow-up fixes (this commit): Lemon Sampler vs Lemon Drum
      Sampler name un-map (plain sampler keeps the parameter grid), Trio `hosted: false` (own tab strip, no squash)
      + aspect-fit blit + draggable knobs (`ParamSet`), drum flip button + crop helper (`drum_half`) + pop-out
      arrow icon (wrench stays the device wrapper's).
- [ ] **Device wrapper strip in the tile top bar.** ON/PAN/VOL/PITCH/RANGE/TRACK controls (as the plugin
      window's strip, plugin_window/mod.rs + `FrameEvent::*`) need a taller tile top bar (HEADER_H=23 won't
      fit knobs): new strip row between tile header and editor body, wired to the device's channel via the
      `open_editor` lookup + `channel_control` setters, with a `Drag::WrapKnob`-style drag. Design decided;
      not yet built.
- [ ] **Device rename missing.** The tile context menu has no Rename — add it (same UX as the arranger's
      audio-clip rename: inline edit on the device name, written back to the device/plugin state).
- [ ] **Lemon Delay still shows generic UI in the dock** — likely its editor fails the fit check (too big for the
      tile) and falls back to the generic panel. Fix the fit logic / provide a compact variant so it renders its
      real UI docked.
- [ ] **Parametric EQ dock UI slim-down.** The docked EQ must fit comfortably: remove the right-hand fader bank
      from the docked view entirely (see screenshot — the 8 vertical dB faders) — those stay available via
      pop-out only. The FREQ knob row and band handles stay docked.
- [ ] **Fit Lemon Trio, Lemon Drum Sampler and Lemon Slicer comfortably in the rack tile.** Per-device compact
      variants: Trio = fit as-is (audit what's overflowing); Drum Sampler = dock shows ONLY the 16 pads; right
      clicking a pad opens a popout window with that pad's controls (tune/decay/level/etc.); Slicer = compact
      variant that fits.

## 🎯 TODO — Rack / launcher / tracks work queue (audited 2026-11-09)
Full plans with file:line refs in `docs/audit-2026-11-09-device-racks.md`, `-clip-launcher-bugs.md`,
`-browser-and-tracks.md`. Order below is smoothest first; every step independently shippable.

### A. Quick launcher/UX fixes
- [x] **A1. Per-device wrench.** `RackAct::PopOut(key)`/`Dock(key)` + `device_rack_float` + `pop_out/dock`
      (device_rack.rs:436/450) already do per-device pop-out — only a button is missing. Add a wrench in
      `tile_paint` (views/device_rack.rs:281, right of the `v` menu) via `header_icon_button` (:551), lit from
      `device_rack_float.contains(key)`. View-only. A *container* tile's wrench opens the docked `Win::Container`
      instead (its UI isn't a hosted plugin window). ✅ wave 1 (f42195de)
- [x] **A2. Duplicate-track launcher bug (root-caused).** ✅ wave 1 (f42195de): `Command::TrackDuplicate`
      inserts the lane and the `Track` in one undo step; Ctrl+D routed through it.
- [x] **A3. Device-rack browser order.** ✅ wave 1: MIDI FX → Lemon instruments → Lemon fx → third-party tail.

### B. Device selection + clipboard
- [x] **B1. Tile top bar selectable.** ✅ wave 1: SelectTile wired, brightness highlight, Ctrl+D/X/V,
      BTreeSet multi-select.

### C. Rack containers (Instrument / Audio FX / MIDI FX racks)
- [x] **C1. `DeviceKind::Container`** — ✅ wave 1: variant implemented; browser insertion via
      `MCmd::AddContainer` + `drop_container_on_rack`.
- [x] **C2. Ctrl+G groups selected devices** into a rack — ✅ wave 1 (wrap_in_container/wrap_channel).
- [ ] **C3. MIDI FX Rack waits for the M9 note-path milestone** — MIDI FX is hosted for UI only; the engine never
      calls its `process()`, so a MIDI FX rack can't pass notes through until that lands.

### D. Playlist track operations
- [x] **D1. Delete deletes selected tracks.** ✅ wave 1: `Cmd::DeleteTracks`, bottom-up, one undo snapshot;
      clip-Delete preserved when no track range is selected.
- [x] **D2. Shift+wheel moves the whole selection.** ✅ wave 1: anchored-range branch in `shift_tracks`
      (block move via repeated `swap_tracks`, Shift+Up/Down included). ⚠ `track_links.rs` untouched (mixer links).
- [x] **D3. Ctrl+G groups tracks / Alt+G ungroups.** ✅ wave 1: `Cmd::TrackGroupSel`/`TrackUngroupSel`
      on the existing `Track.grouped` leader semantics, FLP round-trip compatible.

### E. Launcher paint bugs (need a runtime debug pass to confirm)
- [x] **E1. Browser context menus behind the browser.** ✅ wave 2 (fd73cb54): consolidated popup-paint site +
      `overlay_on` lifecycle cleanup; regression test `browser_menu_survives_launcher_repaints`.
- [x] **E2. Toolbar vanishes on launcher clicks.** ✅ wave 2: toolbar-band clip-leak fix; regression tests
      `launcher_click_repaint_matches_full` + launcher open/click partial-repaint parity.
- [x] Incidental: duplicate client fill at views/launcher.rs and the launcher's missing `window_body` swallow
      hit — ✅ wave 2.

### F. Clip launcher — audio clips × Tesla (added 2026-11-09)
- [x] **Dragging samples/audio into empty launcher cells.** ✅ wave 2 (fd73cb54): drop-into-cell fixed with a
      drop_file regression test.
- [x] **Double-clicking an audio clip cell opens it in Tesla inside the device-rack area.** ✅ wave 2: single
      click selects the cell; double click hosts Tesla (Edison-slot path) on the clip's audio channel in the
      rack area, with rename/recolour write-through — mirroring the pattern-clip double click → piano roll.
- [x] **Clicking a track (header or row) in the launcher returns the device rack** from the Tesla view. ✅ wave 2.

## 🎯 Current Session (2026-11-09) — Rack shows the plugins' own UI; the launcher is Bitwig's
- ✅ **Actual plugin UI in the device-rack tiles.** New `crates/os-app/src/rack_ui.rs`: a hosted plugin's name maps
  to its real editor — the 18 `os-plugkit` "simple" plugins (Lemon Arp, Lemon Transpose, Lemon Chord, the
  note/audio layer + selector containers) through a new paint-only `os_plugkit::simple::paint(info, canvas, &dyn
  Values, &ViewState, hint)` with a `Values` trait for the host's parameters (`Logic::paint` now delegates to it,
  so one paint path serves both), the 13 `lemon-small-fx` plugins (Lemon Filter, …) through `view::paint`, and
  Lemon Reverb through its own `view::paint` + `room::look`. Rendered into a reused offscreen `Canvas` at the
  tile's DPI scale, cached by parameter-value signature, blitted with `Canvas::image`; the paint's hit rects become
  `RackAct::ParamSet` (dial/fader drags) and the new `RackAct::ParamTap` (switches, selector rows). Tile widths now
  follow each editor (`device_rack::tile_w`, `DEFAULT_H` 336). `Canvas::scale()` added.
- ✅ **The clip launcher is Bitwig's launcher.** Scenes along the top; every clip and scene has Bitwig's launch
  parameters (`LaunchQ` Off/1-16…8 bars, `Q to Loop`, Play mode from-start / legato-clip / legato-project, On
  release Continue/Stop/Return/Next action) as **Main and ALT**, scenes carry Override Launch Settings, and every
  clip a full **next action** (21 functions incl. the in-block / next / previous / other block sets) with its own
  timing (Loop Count or Time). Clips, scenes and track headers rename in place and take a colour.
- ✅ **Tracks play independently** — the item `docs/clip-launcher.md` §S2 called engine work, done without touching
  the engine: `launcher::shadow` (from `rack::play_project`) gives the transport a project whose lanes hold each
  playing clip's pattern (that track's channel's notes only) repeated at its own length, and `App::selection_loop`
  loops their common multiple (`launcher::window`). **Limit:** a clip launched part-way into a longer one re-aligns
  at the window's end (Bitwig loops each clip on its own forever). See `docs/clip-launcher.md` §1.1.
- ✅ **The grid scrolls** (S3): a scene column keeps ≥ 96 px and a track row ≥ 22 px, and what does not fit moves —
  wheel for the rows, Shift+wheel (or a sideways wheel) for the scene columns, clamped to the content. Scene buttons
  and the track column stay put while the cells scroll under them, and a cell scrolled out of the grid registers no
  hit, so it cannot steal a click meant for the track column or the scene buttons.
- ✅ **Clips record into their cells** (S4, notes): a cell's menu arms it (`Act::Record` → `launcher::record`), playback
  starts, and what is played lands in the cell's own clip — an empty cell gets a pattern and a clip as the first note
  comes (the cell shows it being recorded), a filled one records over its own pattern, and the clip loops at its
  pattern's length (whole bars past the last note). Stopping the track or the transport ends the take; a take that
  recorded nothing leaves the cell as it was. Audio into a slot, across-loop takes and take comping are still open.
- ✅ **A double click makes a clip** — an empty cell gets a pattern of its own (`launcher::make_clip`) and opens in
  the piano roll, as Bitwig's double click does; the menu keeps `Set current pattern` for binding an existing one.
- ✅ **Samples drop into cells**: an audio file dragged from Explorer onto a cell (`App::drop_file` →
  `launcher::add_audio`) makes an `AudioClip` channel (`playlist::audio_channel`, shared with the arranger's drop)
  and a cell that plays it — looped at the sample's own length, drawn as its peaks, renamed/recoloured like any
  clip, and no pattern behind it (a take into that cell replaces the sample with its pattern).
- ✅ **A scrolled device tile stays in its body.** `views::device_rack::controls` clips the grid and the scroll bar to
  the tile body and clamps each control's hit rect to it too, so the row the body's top (or bottom) edge cuts through
  no longer paints its switches and value texts over the tile's header — the overlap seen on Lemon Sampler's docked
  tile.
- ✅ **A tile's editor fills its body.** `views::device_rack::editor_body` no longer aspect-fits the plugin's editor
  with the 1:1 cap (the bars beside and under it): it is drawn stretched into the tile body (2 px inset for the
  frame), rendered at the larger of the two stretches so glyphs are never drawn from fewer pixels than they cover.
  The tiles keep their size — only the UI inside them grows; the editor's hit regions scale with it, so its dials
  stay live.
- ✅ **The rack header's wrench.** A wrench button at the header's right (after Pin / Hide) with `RackAct::ToggleEditors`
  (`device_rack::toggle_editors`): lit green from the LED pair while any editor is out, its click pops every device in
  the chain out into its own floating editor window (opening the ones not open yet) and, when any is already out,
  docks them all back into their tiles. The icon is Tabler's `tool` (`icons::Icon::Wrench`).
- ✅ **The rack's height is bounded.** `MIN_H` is 180 and a new `MAX_H` 360 (`device_rack::max_h`: never more than
  half a short window), so the splitter can no longer take the panel to a sliver nor to half the screen
  (`split_drag` clamps both ends); the double click still gives back `DEFAULT_H` 336. New test
  `the_splitter_stays_within_the_racks_bounds` drags the splitter to both ends and checks a 500 px screen caps it
  at 250.
- ✅ **Shift+click selects several tracks.** `PlaylistState::track_anchor` (`None` = `track` alone) plus
  `playlist::select_track(s, row, extend)` / `track_selected(s, row)`: a plain click selects one track, Shift+click
  selects every lane from the anchor (the last plain click) to the one clicked, in both directions, and the last
  click stays the active `track` so the device rack and the mixer keep following it. Wired into `press_header` (the
  arrangement's headers) and `Act::SelectTrack` (the launcher's track column, which now paints the selected rows
  with an accent edge). The other places that move the selection (Ctrl+↑/↓, Shift+↑/↓ track moves, mixer
  Select channels and tracks, assigning audio / instrument tracks) go through `select_track`, so the range never
  outlives the click that made it. Tests: `shift_click_a_header_selects_a_range_of_tracks`,
  `shift_click_a_track_row_selects_a_range_of_tracks`.
- ✅ **Ctrl+D duplicates the selected track** (`Cmd::DuplicateTrack` → `playlist::duplicate_tracks`): a new lane
  right under the original with its name, colour, state and clips (new clip ids, copies regrouped, one undo step);
  the copy is a lane of its own — it does not join the original's group and takes no link — and the copy made last
  becomes the selected track. With a Shift+click range, every lane of the range is copied under its own original.
  Ctrl+D used to deselect in the playlist: Deselect moved to **Ctrl+Shift+D** (menu hint updated) and its tests use
  that; the piano roll keeps its own Ctrl+D. Test: `ctrl_d_duplicates_the_selected_track`.
- ✅ **A hosted instrument's tile no longer steals its audio thread.** Showing Vital's track ran `rack_controls` on
  every tick with a blocking `plugin.lock()` and a full `param_info` + `param_value` sweep (~900 parameters: each
  info asks VST3 for the title, units and two plain conversions), holding the instance's lock for milliseconds while
  its audio thread waited — distorted sound, high load. Now: the parameter list is read once per instance
  (`Hosted::infos`, a `OnceLock`), the values are read through `lock_for_tick` (skips the tick instead of making the
  audio thread wait, `prev` kept as-is), the sweep runs at most every 40 ms (`RACK_EVERY`, `Plugins::rack_due`,
  attention count or interval) and a control the user moves on the tile updates its own entry at once
  (`set_param_value` + `Plugins::param_text`). Test: `plugins::tests::a_tiles_parameter_list_is_read_once`.
- ✅ `cargo test -p os-app` 1129 passed / 0 failed / 62 ignored (24 launcher tests).
- 📋 Open on the launcher: audio / across-loop / comping *recording* (S4's rest), persistence (S6), MIDI / key
  mapping (S7), a scene's own tempo, launcher columns for bus tracks (S3).

## 🎯 Session (2026-11-07) — MIDI FX / container editors LIVE
- ✅ All 18 "simple" plugins (14 MIDI FX + 4 layer/selector containers) now render real parameter UI: added a
  reusable `os-plugkit::simple::Editor` (opens `simple::win32::open` on spawn, repaints on param changes) and
  replaced every stub `editor.rs` (whose `spawn()` returned `Box::new(())`, leaving the embedded rack frame
  empty) with its static `Info` layout. Net −2.6k lines of dead stub code.
- ✅ `cargo test -p os-app` 1081 passed / 0 failed / 62 ignored; all 18 plugin crates' suites green (166 tests).
- ✅ Commit `fc17c4ec` pushed to `main`.
- ✅ **Rack params are dials** (commit `37e9eca1`): horizontal sliders replaced with vertical-drag knobs
  (`kit::knob`, `Drag::DeviceRackParam { y0, v0 }`); `param_bar` → `param_dial`; 1081 tests green.
- ✅ **Embedded rack tiles now show the plugin's own UI** (commit `54a692cf`): the tiles were blank dark bodies,
  two bugs — `State::size()`/`area_y()` always added the floating window's chrome (title/strip/border) so a pinned
  frame was `tile + chrome` and `pin_window` pushed the editor down/right out of the tile body, and the per-tick
  `set_editor_size` sync (unlinked natural size + Lemon host-link size) had no `pinned` guard, re-inflating the
  frame past the tile every tick. A pinned frame is now exactly the editor (no chrome — the tile's own header is
  the name/LED/menu), its editor area sits at the origin, and the tick sync skips pinned windows; pop-out is
  unaffected (it un-pins first, then syncs as before). `cargo check` clean, `cargo test -p os-app` 1082/0/62
  (added `a_pinned_frame_is_exactly_the_editor` regression test, commit `a38cc3e1`).
- 📋 **Signal routing audit (2026-11-07):** audio FX chain ALREADY WORKS — mixer insert effects process in
  slot order (`engine/mix.rs:31`), so the rack's right-side FX tiles already drive the mixer. The remaining gap
  is the **engine note-path**: MIDI FX / note-layer / note-selector plugins are hosted for UI only and their
  `process()` is never called, so no MIDI reaches the instrument on the right. This is the staged
  "note-path refactor" milestone (todo `M9` → note-path).

## 🎯 Current Session (2026-11-05) — M9 MIDI + Audio Layer/Selector complete
- ✅ `lemon-arp`, `lemon-chord`, `lemon-echo` — 23 tests pass (GNU toolchain, `RUSTUP_HOME=/z/rustup`)
- ✅ `lemon-note-layer` (9 tests), `lemon-note-selector` (9 tests) — parallel/slot routing, channel/note/CC modes
- ✅ `lemon-audio-layer` (10 tests), `lemon-audio-selector` (7 tests) — parallel/selected Bypass/Filter/Delay/Chorus/Drive slots
- ✅ 58 tests pass across all 7 MIDI/Audio containers (`cargo test -p lemon-arp -p lemon-chord -p lemon-echo -p lemon-note-layer -p lemon-note-selector -p lemon-audio-layer -p lemon-audio-selector`)
- ✅ MIDI FX wired into the app (2026-11-06): `os-app` links `lemon-arp`/`lemon-chord`/`lemon-echo` (`builtin_midi_fx`),
  the device rack's MIDI FX region shows per-slot tiles + a "＋ Add" tile opening the MIDI FX browser
  (`midi_fx::add` hosts an instance under `MIDI_FX_KEY`); slots are in memory per track id (the `.flp` records
  them with the engine's MIDI FX milestone); `os-app` 1062 tests pass.
## 🎯 Engine Split (2026-11-06) — COMPLETE
- ✅ `os-engine/src/engine.rs` split into per-concern modules under `engine/` (18 modules: channel, jobs,
  live_edit, mix, render, sched, transport, triggers, voice + pre-existing auto_bypass/controllers/layers/
  marks/multicore/notes/param_auto/pdc/rec). engine.rs = Engine struct + core impl + tests (~9.5k → 7.3k lines).
- ✅ Pure structural refactor, no behavior change; `cargo test --workspace` 2248 passed / 0 failed / 79 ignored.
- ✅ Commit `57df0c97` pushed to `orchestra/engine-split`; merged to main.
## 🎯 Current Session (2026-11-04) — Rack Rework Complete
- ✅ `＋ Add` header button → plugin browser (native built-ins first per section, filter radios in footer)
- ✅ DELETE/Backspace removes selected effect tile (instrument never removable)
- ✅ Left/Right arrows move selection across the chain
- ✅ Hosted plugin tiles replace MIDI FX placeholder
- ✅ `MoveEffect` / `RemoveInstrument` / `HostReq::RemovePlugin` in control plane (undoable)
- ✅ Workspace `cargo test` green, release build OK
- ✅ Commit pushed (`b25bf8fc`, main in sync with origin)
- 📋 **Next session starts at M9 MIDI-FX crates** (see below)

## 🎯 Session (2026-11-06) — Audit
- ✅ Audit of all local copies + stray folders: `docs/session-audit-2026-11-06.md`
- ✅ WIP secured: `orchestra/engine-split` (engine.rs module split) and
  `orchestra/midi-fx-plugins` (M9 crates) committed and pushed to origin
- 📋 Order forward: land engine split → midi-fx editors + note processing → M8 track model

- Drag & drop plugins from the browser panel onto device-rack slots (drop inserts at that position; reuse the TilePress reorder-drag machinery)

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
- [x] Session/clip-launcher view: survey suhina-daw, beatlayer, nxtakt for clean-room design ideas (landed as
      `docs/clip-launcher.md` + the launcher described above; remaining steps S3, S4, S6, S7 are listed there)

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
- Fork side: `Z:\github\lemon-study\fork-audit\1-master-assistant.md` … `6-community.md`
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

## 🎯 Rebuild spec — the step sequencer / channel rack as a group instrument
The channel rack (and its step sequencer) is out of the app UI since 2026-11-06 (no default window, no
toolbar/VIEW/F6 toggles, no ADD ▸ Channel). The channel model, FLP import/export and the channel-rack
tests stay untouched. To bring it back, rebuild it as ONE self-contained group instrument, not a window:
- It is an instrument plugin (`plugins/lemon_group.rs`-style CLAP-linked module) whose hosts live in a
  track's instrument slot like any other device — the device rack, mixer and shortcuts all work on it.
- Its editor embeds per-channel lanes: each FLP channel is a lane row with the step sequencer grid, mute
  and mini-knobs, routed inside the plugin; the group's single channel output goes to the track's strip.
- Its own mixer group: a "Channel group" entry in the project's mixer view, one fader + FX chain for the
  whole rack, plus optional per-lane sends to other strips.
- Import: FLP channels build lanes (channel → lane mapping kept in the plugin state); export maps lanes
  back. The step-sequencer UI rules (mouse paint/drag, step prefs, high-visibility outlines) move into the
  lane editor per ui/step-sequencer.md.
- Remove `Win::ChannelRack`/`rack.rs` view code only after parity; until then it stays dormant but tested.

## 🎯 Session (2026-11-06) — Device-rack routing fixes
- [x] Rack ＋ (header, gaps, browser) routes by the rack track's chain: an instrument pick replaces the
      track's channel or (when the slot is empty) makes a channel ON the track — lane linked, channel routed
      to the track's strip (`track_links::new_instrument` now mirrors `Track.channel` + `mixer_track`, like
      track_create does).
- [ ] LARGER MODEL CHANGE (not half-fixed here): an instrument should be a first-class device of the track
      model — `Track.channel` is a mirror of `TrackLink::instrument` kept in sync by hand (track_create,
      new_instrument); every new route must remember both. Next step: a single `control` command
      (`TrackSetInstrument { track, index | none }`) that owns lane link + channel routing + mirror +
      hosting, and make the ADD-tab / classic paths delegate to it.

## Standing rules
- Clean-room only: no decompiled code or pseudocode derived from decompilation (see README contributing).
- Keep `THIRD-PARTY-NOTICES.txt` in sync with Cargo.lock (`cargo run -p notices`) — cargo test enforces it.