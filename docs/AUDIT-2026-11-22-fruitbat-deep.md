# Fruitbat Studio deep audit — 2026-11-22

Scope: `Z:/GITHUB/_APPS/Fruitbat-Studio/studio` (worktree at the audit date), companion
`study/`, build cache `build-target/`. Requested focus: launcher ↔ playlist/mixer sync,
docked vs popped-out native-plugin theming, DSP/engine risk, build/dependency hygiene,
docs drift, tests/stability, consolidation. **No code was modified and nothing was
committed.** Line references are to the worktree as read on 2026-11-22. The whole tree
has uncommitted rename churn (`git status --short` lists ~200 `M` files for the
`Lemon-Studio` → `Fruitbat-Studio/studio` move); the launcher view edits described in
§1.1 are *uncommitted work on top of HEAD*, so they can disappear on a clean checkout.

Method: read the listed docs, inspected the named crates/plugins, ran targeted
commands (`cargo metadata --locked --offline`, cached test binary `--list` and test
filters, `git` history/refs), and rebuilt the current worktree's `os-app` test binary
with the documented GNU toolchain (`cargo test -p os-app --bin lemonstudio --no-run`:
**success** in 8m08s, 56 warnings). No audio device and no GUI session were used: this
is a static audit plus test runs. The fresh binary lists 1333 tests (1271 passed,
62 ignored in a full run).

## Verdict

Pre-alpha but genuinely ambitious: the launcher, theme/plugkit pipeline, plugin crash
guard, parallel engine, and 60+ workspace crates/44 plugins are real working systems.
The highest-value problems today are (1) an uncommitted launcher header rewrite with no
tests, (2) two independent theme publication paths that can desynchronise docked tiles
from popped-out editors, (3) a playlist `TrackDelete` path that leaves launcher state
orphaned and can corrupt the project chunk, (4) docs that are one or two renames behind,
and (5) a 15 GB stale build cache / stale git refs and study paths.

---

## 1. Clip launcher ↔ playlist/mixer sync

### 1.1 The launcher now has playlist-style headers — but only in the worktree

`crates/os-app/src/views/launcher.rs` paints a colour strip (253-261), mute and solo
LEDs (262-274), a volume fader bound to `Action::Fader(mixer_strip)` (313-328), a pan
fader bound to `Action::PlPan(mixer_strip)` (329-338) and a per-track Stop button
(339-346). `MIN_ROW` was raised from 22 to 44 px (31) to fit them; `git diff --ignore-
space-at-eol -- crates/os-app/src/views/launcher.rs` shows these as 55 uncommitted
insertions/3 deletions. `slot_paint` gained a per-clip play/stop button (401-423) in
the same uncommitted `launcher.rs` change (41 insertions/3 deletions).

Consequences and risks:

* **Solo LED reads a different state than the playlist's.** The launcher lights the
  LED from `playlist::track_soloed` (`views/launcher.rs:271`), which only checks
  `s.playlist.solo` (`playlist.rs:1187-1190`). The playlist header's Linear-mode S LED
  also lights for the mixer strip's own solo (`views/playlist.rs:1228-1229,1316-1318`).
  Solo from the mixer therefore shows in the playlist but not in the launcher. Mute is
  consistent: both read `playlist::track_enabled`, which resolves the strip's `enabled`
  flag in Linear mode (`playlist.rs:1175-1185`).
* **The faders reuse mixer drag semantics at the wrong scale.** `Action::Fader`'s drag
  rate is `views::mixer::FADER_PER_PX / scale(Win::Mixer)` = `(1/440)/mixer_scale` per
  pixel (`app.rs:6421-6430`, `views/mixer.rs:112,160-162`). The launcher's fader is
  `row_h-8` ≈ 36 px tall, so a full-range drag takes hundreds of pixels. The same
  compromise exists in the playlist mini-fader (`views/playlist.rs:1383`), but it is
  much more visible in the launcher. The pan fader is consistent with the playlist's
  knob (`knobs.rs:260-264,333-337`), and the last hit wins (`app.rs:6124-6126`), so the
  tiny controls do receive their clicks ahead of the whole-row SelectTrack hit
  (`views/launcher.rs:275`).
* **No tests cover any of it.** `grep -n '#\[test\]' crates/os-app/src/views/launcher.rs`
  returns nothing; neither launcher file mentions `Action::Fader`/`PlPan` in a test.
  The cached binary's `launcher::tests` filter passes (35 tests, 2026-11-16 build)
  but exercises the *old* code. This is the largest untested surface in the worktree.
* `docs/clip-launcher.md:180` still specifies `MIN_ROW = 22`; `todo.md:194,355,361-369`
  and `docs/AUDIT-2026-11-14-fruitbat.md:295` still call full headers, volume/pan faders
  and per-clip buttons missing. They are now present (uncommitted).

### 1.2 Launcher → engine synchronisation is a rebuilt shadow project, not a live clip bus

`columns()` takes every non-bus track with `playlist_lane > 0` and sorts by `mixer_strip`
(`launcher.rs:581-607`), so the launcher column sits over the strip that plays it;
`rack::play_project` clones the project and replaces the arrangement with
`launcher::shadow` when clips are live (`rack.rs:2698-2710`, `launcher.rs:1637-1733`).
Every live state change (launch/pending at `launcher.rs:930-962`, advance at 1335-1377,
Stop/sync at 1084-1121, mute/solo at 1866-1883) calls `transport.update(&p)`
(`transport.rs:696`). That is a coherent, simple model: the launcher *owns* the engine
while clips play, and the arrangement is layered back through `add_arrangement`
(`launcher.rs:1734-1755`). Known risks:

* `add_arrangement` positions layer clips with `(s.engine_tick - s.arr_tick) as u32`
  (`launcher.rs:1642-1644`). Both are `f64` (`app.rs:1268-1271`); a negative delta
  saturates to 0 on the `as` cast. `advance` only adds a positive delta to `arr_tick`
  (`launcher.rs:1355-1360`), and loop wrap detection is `tick + 0.5 < last`
  (`launcher.rs:1385`), so after a seek/rewind while clips are live the arrangement layer
  can be positioned at 0 rather than at its own phase. This is worth a targeted test.
* Launch quantisation is driven from `App::tick` (UI thread) via `launcher::advance`
  (`app.rs:4511-4513`), not from the audio callback. A busy UI thread can delay a
  bar-line launch; the docs already note the window-end re-alignment compromise
  (`docs/clip-launcher.md §1.1`, `todo.md:465-466`).
* `docs/AUDIT-2026-11-14-fruitbat.md:165` says "no launcher persistence", and line 354
  repeats it, but `lemon_state.rs:75-86,997-1073` writes/reads event 208 and the test
  `lemon_state::modulation_graph_tests::the_launcher_round_trips_through_a_lemon_file`
  passes on the cached binary; `todo.md:181-186` marks it done. The audit contradicts
  itself (line 350 reports 1248 tests from the same date).

### 1.3 Track deletion leaks launcher state and can corrupt the saved chunk

`Command::TrackDelete` (`control/command.rs:1349-1397`) removes the track, its
automation lanes, arrangement clips and mixer strip, but never touches
`s.launcher.clips`, `playing`, `pending`, `selected` or `scenes`. Undo cleanup exists in
`history.rs:266-268`, but that path is not taken by this command.
`lemon_state::launcher_state` writes its clip *count* as `s.launcher.clips.len()` and
then skips clips whose track position is missing (`lemon_state.rs:997-1008`). With an
orphaned clip the declared count exceeds the records written; `read_launcher`
(1035-1073) then parses scene/name bytes as the next clip's track position. This is a
real project-corruption path for control-plane/RPC track deletion after launcher use.
A cheap invariant to test: `TrackDelete` ⇒ no launcher entries for that id, count ==
records.

---

## 2. Docked vs popped-out native plugin theming

Both paths ultimately use `os_plugkit` palette, but they diverge in three places.

1. **Docked tiles are double-coloured.** `rack_ui::paint_into` takes
   `t = os_theme::current()` (`rack_ui.rs:253-255`) and passes it to palette-aware
   painters (`Ui::Small` 268-270, Reverb 283-289, Trio 302-309), then every rendered
   image is recoloured again by `retint` (`rack_ui.rs:742-747,767-780`). A popped-out
   editor calls the *same* view painter with `os_plugkit::palette()` (e.g.
   `plugins/lemon-3x-osc/src/editor.rs:818-821,1109-1112`) and is not retinted. With any
   non-Lemon theme the dock and the window therefore disagree by construction.
2. **Simple MIDI FX/note editors ignore the palette entirely.**
   `os_plugkit::simple::Editor::paint` takes `_t: &Palette` and discards it
   (`crates/os-plugkit/src/simple.rs:311`); `simple::paint` uses fixed `plug::BG`,
   `brand::MAIN`, `brand::TEXT` constants (344-370). Docked they are retinted; popped
   out they stay Lemon-branded with a themed frame (`plugins/lemon-{arp,chord,...}` use
   `simple::Editor::new`). Only the single test `editor_pixels_follow_the_theme`
   (`rack_ui.rs:1099-1111`) checks the retint, and it checks neither popout parity nor
   palette-aware editors.
3. **Theme changes are published on two independent channels.** `set_design`, theme
   load and the Options toggle call both `os_theme::set_current` and
   `publish_palette` (`app.rs:5410-5411,5444-5445,5464-5465,5468+`). But
   `SAct::Button("Reset")` and `SAct::Preset(i)` (`app.rs:6009-6010,6022-6023`),
   `theme_edit` (`theme_adjust.rs:239-247`, used by live colour editing) and
   `theme_restore` (`theme_library.rs:245-247`) only set the theme-global. Docked tiles
   repaint immediately (the cache key is the `Arc` address, `rack_ui.rs:694-700`), while
   `Plugins::sync_palette` (`plugins.rs:711-725`, called from `app.rs:4511`) sees no
   palette change and does not call `Window::redraw` (`plugin_window/win32.rs:375-378`,
   x11:397-399, cocoa:528-530). Popped-out editors then stay on the old palette until
   some later action publishes it. This is a concrete, fixable docked/popout bug.

Known/limited-by-design: Press/Pulp/Tesla tiles render from saved state with a default
`ViewState` and no hits (`rack_ui.rs:488-533`), i.e. read-only in the dock, editable in
the popout; documented in `rack_ui.rs:16-18`. Crash containment is in-process with a
SEH/signal guard, not a separate process, and explicitly does not cover stack overflow,
`abort`, C++ exceptions on Unix or plugin-owned threads (`crates/os-host/src/guard.rs:5-21`).

---

## 3. DSP / audio-engine risk

This area is comparatively healthy. `os-dsp` guards the known sharp edges: denormals
are flushed in the 72-band TAME bank (`strip.rs:508,519`), the stretcher detects
non-finite peaks and handles samples shorter than input latency (`stretch.rs:238-250`
and the tests at 252+), resampling tables are `OnceLock`-initialised with clamped
indices (`resample.rs:73-89,195,217,245`), envelope divisions are guarded
(`pitch.rs:49`, `channel_filter.rs:59,137`). `os-engine` uses `try_lock` for hosted
plugins in the audio path (`chain.rs:90-112`, `controllers.rs:67`, `param_auto.rs:193`)
and a bounded 2 ms `lock_busy` spin for UI-busy plugins (`jobs.rs:426-444`,
`engine.rs:288`). Parallel jobs catch panics and resurface them on the render thread
without killing workers (`parallel.rs:429-436`, test at 565-578).

Residual risks:

* `tables()` in `resample.rs:73` builds the poly/sinc/window tables on first use
  (`OnceLock::get_or_init`); if that first use is the audio callback, the first note of
  a session can glitch. Prewarming at engine construction would remove it.
* Every launcher launch/stop/mute/solo clones the whole `Project` through
  `play_project` → `shadow` → `transport.update`; this is UI-thread work but it scales
  with project size and can stall interaction on large sessions.
* `Engine::process` renders a whole tick synchronously when the output buffer runs dry
  (`engine.rs:2017-2035`); a plugin that blocks or panics inside that call still
  threatens the stream; the crash guard covers memory faults, not hangs.
* `pdc.rs:631` `pop_front().unwrap()` in a test-only effect (line ~614), not a
  production path — recorded to avoid a false alarm.

---

## 4. Build system and dependency hygiene

* `cargo metadata --locked --offline --no-deps` succeeds; the workspace has 16 crates
  under `crates/`, 44 plugin packages and 11 tools. `Cargo.lock` is consistent.
* Three git dependencies are pinned *only* in the lockfile: `nih_plug`
  (`git = ...nih-plug.git`, no `rev`) in all 44 plugin manifests, `clap-sys` with a
  `rev`, and `vst3-sys` on a branch (`Cargo.lock:672,2699,4810+`). Any `cargo update`
  silently moves to upstream HEAD; pinning revs in the manifests (or vendoring) is
  worth considering for reproducible plugin builds.
* `vendor/asio-sys` is a patched MSVC-layout copy with a local `Cargo.lock`; the
  workspace patch is in `Cargo.toml` and the SDK path comes from
  `.cargo/config.toml` (`CPAL_ASIO_DIR`, relative). `HANDOFF.md:235` says there is *no*
  `.cargo/config.toml`; `docs/build.md:68` says there is. The file exists and contains
  only that env entry. The GNU linker recipe in `docs/build.md:9-18` also differs from
  the active MSVC `env.sh:5-17` (portable `Z:/msvc`, `CARGO_TARGET_DIR` now pointing at
  `build-target`); `.prof.sh` (untracked) uses the GNU recipe plus `tee /tmp/probe.txt`.
  CI (`.github/workflows/ci.yml`) uses the default stable host toolchain on all three
  OSes, i.e. MSVC on `windows-latest`, so the documented GNU-only linker workaround is
  not part of the CI path — worth reconciling before trusting CI as a release gate.
* Remotes still point at the old identity: `origin` → `nyannonymous/Lemon-Studio`,
  `upstream` → `ItsAlphaNeon/Lemon-Studio`, new `chimthuwu/Fruitbat-Studio`; upstream is
  set to `origin/analog-channel-strip-m0`. Many remote-tracking refs are far ahead of
  `main` (e.g. `origin/cutting-edge` +360, `origin/undo-save-playback-fixes` +167,
  `origin/parity-docs` +120). They are fork history, not necessarily wanted work, but
  they should be pruned or explicitly archived so they are not mistaken for a backup.

---

## 5. Docs drift vs reality

| Claim | Reality (2026-11-22 worktree) |
|---|---|
| "No launcher persistence" (`docs/AUDIT-2026-11-14-fruitbat.md:165,354`) | Implemented in `lemon_state.rs:75-86,997-1073`; test passes; `todo.md:181-186` says done. The audit also reports 1248 tests at line 350, so it contradicts itself. |
| "Launcher headers / volume / pan faders missing" (`AUDIT-2026-11-14:295`, `todo.md:194,361-369`) | Present but uncommitted (`views/launcher.rs:253-346`). |
| `MIN_ROW = 22` (`docs/clip-launcher.md:180`) | `MIN_ROW = 44` (`views/launcher.rs:31`). |
| Test counts 1081 (`docs/build.md:82`), 1124 (`docs/clip-launcher.md:123`), 1129/1147/1248 (`todo.md:523`, roadmap:47/61/78, audit:350) | The current test binary lists **1333 tests**; static scan finds 1349 `#[test]` attributes in `crates/os-app/src` (some cfg-/target-gated). Pick one source of truth and generate it. |
| Release binary in `Z:\lemon-target\release` (`AUDIT-2026-11-14:16,319`, `HANDOFF.md:113`, roadmap:77) | Active cache is `build-target/` (`env.sh:12`); `build-target/release/lemonstudio.exe` still carries the old binary name. |
| "No `.cargo/config.toml`" (`HANDOFF.md:235`) | File exists (§4). |
| Stale paths across `docs/session-audit-2026-11-06.md`, `-07.md`, `docs/social.md`, `README.md`, `HANDOFF.md`, `todo.md` | `Z:\lemon-target*`, `Z:\GITHUB\Lemon-Studio`, old org names (17 hits in the 11-07 session audit alone). |
| `study/INDEX.md` paths `Z:\github\Lemon-Studio`, `Z:\github\lemon-study`, `Z:\lemon-target` | Repos now live under `_APPS/Fruitbat-Studio/{studio,study,build-target}`. |

`docs/clip-launcher.md` S6 is correctly marked done; most drift is in the older
roadmap/audits and in `todo.md`'s unchecked 2026-11-09 list. Those files are useful as
historical records; add a short "superseded by …" header rather than rewriting history.

---

## 6. Tests and stability

* Fresh-enough evidence from the cached binary (2026-11-16, before the launcher header
  *both* binaries): `--list` → 1333 tests; full run of the freshly built binary
  → **1271 passed / 0 failed / 62 ignored in 95.85 s**. Filtered runs: `launcher::tests` 35/35 pass,
  `rack_ui::tests` 14/14 pass, launcher round-trip 1/1 pass. No assertion failed.
  A GUI acceptance pass (real audio device/windows) was not performed.
* Coverage is strong for model/persistence/DSP/engine (397 `#[test]` in `os-engine`,
  103 in `os-dsp`, 118 in `os-project`, 1349 in `os-app`) and notably thin where the
  user feels the pain: *zero* tests in `views/launcher.rs`; no test for launcher mute /
  solo LED parity with the playlist, fader drag scale, TrackDelete + launcher
  persistence, or docked/popped palette parity. `plugins/lemon-vocoder`,
  `lemon-peak-controller` and `lemon-formula-controller` have 1 test each; the simple
  editor family has 7-9 per crate (mostly parameter parsing, not pixels).
* Many GUI/device tests are `#[ignore]` (dozens in `app.rs`, `plugins.rs`, `rt.rs`,
  `transport.rs`, device files) and need real audio hardware, a display, or bundled
  plugins. `ci.yml` bundles plugins before `cargo test --workspace`, so those tests
  can run on CI, but the dev build recipe in `docs/build.md` uses `-p os-app` only.
* The `AUDIT-2026-11-14` observation still holds: the suite is not a GUI acceptance
  test. The three reported user-visible bugs (launcher drop, stacking, editor colours)
  were all outside its reach.

---

## 7. Consolidation candidates (study repo, build cache, repo layout)

* `study/INDEX.md` is the right index but has stale absolute paths; update it to the
  three new directories and add links from `studio/README.md`/`docs/status.md`. It
  lists `devices-ui-process.md`, `engine-realtime.md`, `plugin-hosting.md`,
  `study-device-sdk.md`, `study-docs-process.md`, `study-mle-fork.md`, `MASTER-PLAN.md`,
  `PLAN-SUMMARY.md`, `planning-todo.md` and `fork-audit/{1..6}`. Several of these
  (`engine-realtime.md`, `plugin-hosting.md`, `fork-audit/*`) cover the same engine/
  plugin-host topics as `studio/docs/audit-2026-11-10-*.md`; pick one canonical home
  and leave a pointer in the other.
* `study/refs/` holds ~214 MB of gitignored reference clones (`Lemon-Studio-MLE`
  160 MB, `suhina-daw` 52 MB, `bitwig-nitro-tools` 2.3 MB). Fine to keep, but record
  their exact revisions in `INDEX.md` so they are reproducible after a cleanup.
* `build-target` is **15 GB** (debug `.pdb` 139 MB, debug `.exe` 42 MB, release 32 MB)
  and still emits `lemonstudio.exe` names; `studio/target` is a second 43 MB cache.
  `docs/session-audit-2026-11-06.md:33-36` and `-07.md:51-57` still describe the old
  `Z:\lemon-target*` caches (14 GB etc.). Keep one cache, re-point the docs, and
  decide whether the binary should be renamed to `fruitbat-studio.exe` (that changes
  bundle/help paths, so treat it as a deliberate release task, not cleanup).
* `AUDIT-2026-11-04.md` sits at the repo root while every newer audit lives in
  `docs/`; move or link it for a single audit index.
* `.prof.sh` is untracked and writes `/tmp/probe*.txt` outside the configured scratch
  directory; either track it as a tool with documented paths or delete it.

## Recommended order of work

1. Commit or discard the uncommitted launcher header work, with tests for TrackDelete
   cleanup, solo parity, and fader drag; this is unshipped code that already changes
   the UI.
2. Route every theme mutation through one function that calls both `set_current` and
   `publish_palette`; then decide whether `retint` should be applied only to painters
   that ignore the palette (simple/MIDI FX) or removed entirely.
3. Add a launcher-clip cleanup arm to `Command::TrackDelete` and a regression test that
   saves/reloads after deletion.
4. Refresh the docs index (counts, paths, superseded notices) and consolidate study
   pointers; then prune stale remotes/caches.
