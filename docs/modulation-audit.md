# Project-level modulation — Phase 1 audit (2026-11-12)

Handoff: *Lemonwig Studio — project-level modulation system*. This audit is the Phase 1 gate. It records
what already exists, what the engine path actually is, and every `[DECIDE]` in the spec with the choice made
and why. **No code changed before this file existed.**

Branch: `modulation-m0`. Baseline before this work: `cargo check -p os-model -p os-engine -p os-app`
green (with a portable MinGW/binutils + libclang; see *Build environment* at the end).

> **Important finding.** The repository already contains M0 (persistent parameter identity) and M1
> (a modulation graph, its `.lemon` persistence, an engine evaluator, and knob-menu/drag assignment).
> This work extends that system; it does not replace it. Two pieces of M1 contradict the handoff and are
> superseded by the routing-mode interaction (see §4): (a) dragging a modulator chip onto a knob, and
> (b) assigning only from a knob's right-click menu. Both remain as secondary paths, but the required
> primary mechanic is routing mode + drag-on-parameter.

---

## 1. Verified paths

All twelve paths in the handoff exist. Line counts are at the start of this work.

| Path | Lines | What it is |
|---|---|---|
| `crates/os-model/src/lib.rs` | 3199 | `Project`, channels, mixer tracks, automation. Holds no modulation state itself; the graph is a sibling type. |
| `crates/os-model/src/modulation.rs` | 571 | M0 `PersistentObjectId` / `OwnerLocator` / `ParameterKey` / `ParameterPath`, `IdentityRegistry`, and M1 `ModGraph` / `Modulator` / `ModTarget` / `ModSource`. |
| `crates/os-model/src/clip_lfo.rs` | 397 | FL's per-automation-clip LFO (a legacy controller shape). **Not** the project modulator. Kept, not absorbed (§3.4). |
| `crates/os-app/src/device_rack.rs` | 2513 | Rack model/actions: `Device`, `DeviceKind`, `RackParam`, `RackAct`, `run`, chain order, tile reorder, `set_param_value`. |
| `crates/os-app/src/views/device_rack.rs` | 846 | Rack painting: `layout`, `controls` (parameter grid), `editor_body` (docked plugin editors), `cell_paint`, header buttons, tile menus. |
| `crates/os-app/src/rack_ui.rs` | 1057 | Native re-drawings of built-in editors (Trio, Drum, EQ, …) and their hit regions. Not used for modulator UI: the modulator tiles are new native canvas drawing. |
| `crates/os-app/src/browser.rs` | 1489 | Current-project browser: tree rows, `Item`, row actions, history rows, jumps. |
| `crates/os-app/src/views/browser.rs` | 1047 | Browser tree construction: `ROOTS`, the Current-project sections list, row painting. |
| `crates/os-app/src/lemon_state.rs` | 1343 | `.lemon` extension state: event 206 = identity registry, event 207 = modulation graph, layout 1. |
| `crates/os-engine/src/engine/controllers.rs` | 272 | FL controller links (peak controller / formula controller outputs). Separate, frozen path. |
| `crates/os-engine/src/engine/param_auto.rs` | 582 | Parameter automation: samples curves every `PARAM_STEP = 64` frames and pushes `ParamEvent`s at frame offsets. |
| `crates/os-engine/src/effects/peak_ctrl.rs` | 422 | Lemon Peak Controller *device*: a per-tick peak follower + LFO producing controller-link outputs. A device, not the new follower; **not** wrapped. |

Also inspected and used:

- `crates/os-engine/src/engine/modulation.rs` (712) — M1 evaluator (`source_value`, `step_values`, `run_host_modulation`).
- `crates/os-app/src/modulation.rs` (744) — M1 app layer: owner registry, `target_for`/`path_of`, `engine_spec`, `ModCmd`, knob editor menu.
- `crates/os-app/src/params.rs` (1073) — `ParamUi`, `Entry { steps, hidden, default }`, `target_of`, `set_param_value`, `automation`.
- `crates/os-app/src/app.rs` (23294) — `Action`, `State`, `Drag`, `act`, `release`, `animating`.
- `crates/os-app/src/transport.rs` (3435) — `Shared` (lock-free UI state), `Queue`, engine construction, `set_modulation`.

## 2. How the existing pieces actually work

### 2.1 Parameter target encoding

`os_model::target::Target` is a `u32`. `Target::encode` / `target::decode`:

- `ChannelParam(channel_index, k)` — an instrument parameter: `channel_index << 16 | (PLUGIN_PARAM | k)`.
- `SlotParam(track, slot, k)` — a hosted effect parameter: `(0x7000 + track*64 + slot) << 16 | (PLUGIN_PARAM | k)`.
- Host parameters (volume, pan, EQ, send, slot mix) use their existing lower-16-bit host codes.
- Bit 31 is used by existing mixer targets. **No modulation state is ever written into these bytes.**

Parameter enumeration: `os_app::params::ParamUi` keeps a `List` per hosted key (`params.lists`), with `Entry
{ id, name, hidden, default, steps }`. `params::target_of(state, key, index)` converts a hosted key +
positional index into the FLP `Target`; `params::key_of(state, target)` is the inverse. Native ids are
available from `Plugins::hosted[key].param_ids()` and are what M0's `ParameterKey::PluginNativeId` stores.

### 2.2 Parameter change actions and delivery

The delivery path is **value + `ParamAuto` events** (not a UI-side push):

1. A control edit changes the project / hosted plugin directly (`device_rack::set_param_value`,
   `plugins::set_param`, `history` commands).
2. During playback, `ParamAuto` (in `param_auto.rs`) walks automation clips each tick in `PARAM_STEP`-frame
   steps, produces `ParamEvent { dest, offset, index, value }`, and the engine hands each event to
   `Instrument::param` / `Fx::param` at its frame offset. Plugins smooth the values themselves.
3. `ParamAuto::base` remembers the last value a clip (or a pattern control point) gave each target. That is
   the base modulation moves from.
4. Host mixer values (fader, pan, separation, EQ, sends, slot mix) do **not** go through plugin events;
   `Engine::run_host_modulation` writes them into `MixTrack::mod_*` once a tick.

The new modulation uses **this same path**. Plugin-parameter modulation pushes `ParamEvent`s through
`ParamAuto::push` (inside the automation step loop when clips exist, otherwise from `run_modulation`).
Host-parameter modulation stays in `run_host_modulation`. There is no second delivery path.

### 2.3 Engine project compilation

`Engine::new(&Project, …)` compiles the project into owned runtime state. Edits are sent as `LiveEdit`s.
The modulation graph never reaches the engine as a `Project`; the app resolves `ParameterPath`s to concrete
`Target`s and calls `Engine::set_modulation(Vec<os_engine::Modulator>)`. A running engine receives
`Cmd::Modulation`. This is the pattern the new work follows: owned runtime state, no project snapshots per
audio callback.

### 2.4 Undo / redo

`control::edit_app(app, "Modulation", Region::Project, …)` snapshots the project, the identity registry and
the modulation graph, applies the edit, and makes one undo step. `history::Side` already carries
`modulation` and `identity`. All graph edits go through `modulation::run(ModCmd)`.

### 2.5 Existing audio follower / spectrum

There is a **device** (Lemon Peak Controller) and FL controller-link machinery. Neither is the project
modulator, and neither exposes master RMS/peak as a modulation source. There is no reusable master-bus
band-filtered follower. The new Audio Follower therefore gets its own cheap detector in
`engine/modulation.rs`, fed from the master buffer at the end of each tick.

### 2.6 Device rack layout and hit-testing

`views::device_rack::layout(s, r)` returns `(Device, Rect)` tiles, starting at `r.y + HEADER_H`, offset
horizontally by `scroll_x`. Every tile is a header (`HEADER_H`) over a body. The body either:
- draws a generic parameter grid (`grid` / `cell_paint`, one `Action::DeviceRack(RackAct::ParamSet)` per cell), or
- blits a native editor image (`editor_body`) with hit rects mapped back to parameters.

`RackAct::ParamSet` is dispatched in `device_rack::run`, which calls `set_param_value` on press and starts
`Drag::DeviceRackParam`. This is the interception point for drag-to-set-depth.

The rack panel is resizable (`RackAct::Splitter`), collapsible (`ToggleCollapsed`), horizontally
scrollable (`ScrollBy`, wheel), and its tiles are drag-reorderable. The new modulator row must not change
any of that: only the body's top edge moves down.

---

## 3. Decisions

Every `[DECIDE]` in the handoff, plus the ones the existing code forces.

### 3.1 Cross-modulation — **out of scope in v1** (confirmed)

A modulator cannot drive another modulator's parameters. The target abstraction is
`ParameterPath { owner, parameter }`, where an owner is a device/track/slot/member. Modulators have no
ownable parameter namespace, so cross-modulation cannot be expressed accidentally. A future scope would
add a `ModulatorParam` owner variant; the comment in `os-model::modulation` says so. No code path allows it.

### 3.2 Slot editor: inline expanded view or popover — **popover panel anchored to the slot**

The rack is a horizontal strip with fixed-height tiles; expanding a slot inline would reflow the whole
chain and fight the existing layout/scroll. A popover drawn over the rack (same immediate-mode canvas,
dismissed by a click elsewhere or Esc) keeps the eight slots fixed and leaves the device chain untouched.
It is not a floating OS window and not a modal dialog.

### 3.3 LFO retrigger: free / transport / note — **free + transport sync in v1; note mode deferred**

The engine has no note-triggered modulation clock yet (M9's note path is incomplete). `Timebase` already
expresses free Hz and tempo-synced sixteenths; the transport's own clock is the phase reference for both.
"Sync to note" would need a per-voice/per-note event and is documented as deferred rather than faked.
`Lfo.phase` and an offset are available, so the user can line a cycle up manually meanwhile.

### 3.4 Existing `os-model/src/clip_lfo.rs` — **kept, not deleted or absorbed**

It is FL's per-automation-clip LFO and part of the FLP round-trip. The project modulator is a different
concept with different persistence. Deleting it would break FLP compatibility; absorbing it would conflate
a clip's shape with a project source. `ModGraph` is the new system.

### 3.5 Random distribution — **Uniform + Gaussian**

Minimum viable set. Uniform is the default; Gaussian uses two hashed uniforms (Box–Muller) clamped to
±3σ and mapped to 0..1. Both are deterministic from `(seed, cycle)`.

### 3.6 Discrete / boolean parameters — **excluded, and visibly not eligible**

`steps == 0` (continuous) is required. Enums, switches and other stepped parameters are not tinted in
routing mode and cannot receive an assignment. Rationale: snapping a continuous LFO to a 2-step enum
produces an unmusical square wave and hides the parameter's real range; the handoff allows either choice,
and exclusion is the less surprising one. The eligibility predicate reports why (see §3.8), and the target
is left un-tinted rather than silently snapping.

### 3.7 Audio Follower input source — **Master post-FX**

Recommended by the handoff and the simplest correct tap. The detector reads the final master buffer for
the tick, after insert/master effects and the main volume. Because modulation for the tick is evaluated at
the start of the tick, the follower has a one-tick latency; this is documented and inaudible at control rate.
Per-track/pre-FX sources are a future scope field on `AudioFollower` (the model stores a source enum, so
adding them later does not change the assignment format).

### 3.8 `is_modulatable(target) -> bool`

Implemented in `os-app/src/modulation.rs`. Rules:

1. A hosted plugin parameter (`ChannelParam` / `SlotParam` / container member item) is modulatable when the
   parameter list resolves, the entry is not hidden, and `entry.steps == 0`.
2. Read-only / unresolved targets (no hosted instance, no metadata, index ≥ 0x8000) are excluded.
3. Host mixer controls (volume, pan, separation, EQ gain/freq/width, send level, slot mix) are modulatable.
4. Everything else (tempo, transport, pattern switches, channel enable, etc.) is excluded in v1.

The predicate is the single source of truth for the blue tint, the routing-mode drag and the browser.

### 3.9 Evaluation rate — **fixed 64-frame control block, existing `PARAM_STEP`**

The repo already samples parameter curves every 64 frames (1.33 ms at 48 kHz). Reusing it keeps automation
and modulation in one step, preserves zipper-free behaviour through the plugin's own smoothing, and avoids
a second clock. Per-sample evaluation is unnecessary; per-buffer evaluation would alias fast LFOs at large
buffers. This is the handoff's recommendation with the repo's existing constant.

### 3.10 Stable target identities — **`ParameterPath` (owner UID + parameter UID/code), not indices**

Already M0. The owner is a persistent `u32` (`PersistentObjectId`) that follows mixer moves and container
wraps via `IdentityRegistry`; the parameter is a native plugin id, a legacy positional index, or a host
code. No array indices, and the legacy FL controller-link path is untouched.

### 3.11 Stable modulator ids — **`ModulatorId(u32)`, monotonic, never reused** (already M0/M1)

`ModGraph.next` mints ids; removal never rewinds `next`. The handoff asked for `u64`; the existing system
uses `u32` and persists it. A `u32` holds 4 billion modulators, is already in event 207, and changing it
would break saved projects for no practical gain. Documented deviation.

### 3.12 Eight slots — **a fixed `slot: u8` (0..8) on each modulator**

Adding an eight-slot row over an unbounded `Vec<Modulator>` needs stable slot placement, not list order.
Each modulator gains `slot: u8`; `0..=7` are the project slots, `255` is "no slot" (a modulator created
before slots existed). `ModGraph::add_in_slot` fills a named slot; `add` fills the lowest free slot, else
`255`. Removing a modulator frees its slot; slots never shift. Switching tracks changes nothing because
the graph is project state, not rack state.

### 3.13 Per-routing enable — **`enabled: bool` on `ModTarget`**

The handoff requires an enable toggle per routing in the browser while a modulator's own switch stays.
A disabled routing stays in the graph and contributes nothing; it is restored by the same undo step rules.

### 3.14 Live value delivery — **lock-free `ModSnapshot` (atomics), written by the audio thread**

`os_engine::ModSnapshot` is preallocated when the app installs a graph: one `AtomicU32` per modulator
output and one per modulated target's effective normalized value. The audio thread only stores; the UI
reads at frame rate. No locks, no allocations in the callback. The tile's scrolling history is sampled
by the UI from the same snapshot into a fixed ring buffer (about 2 s at 30–60 Hz); history is view state,
never project state.

### 3.15 Discrete snapping

Because discrete parameters are excluded (§3.6), there is no snapping rule to define in v1. Ranged floats
are clamped to the target's normalized 0..1 (which the plugin maps to its real min/max). Booleans are
excluded with discrete parameters. This is recorded here as the answer the handoff asked for.

### 3.16 Assignment mechanic

Routing mode is **UI state only** (`State.mod_routing`), never persisted and never in undo history.
Entering: click the patch-point button on a slot; it flashes, and every `is_modulatable` control is tinted
blue. Leaving: click it again, click another slot's patch point, Esc, or complete an assignment.
Assigning: press a tinted control and drag vertically. The base value does not change; the drag delta is
the depth (`±1` of the parameter range), and the press mutates the graph live so the sound follows. On
release the pre-drag graph is restored and one `ModCmd::AssignDepth` undo step applies the final depth.
Only one modulator is in routing mode at a time.

### 3.17 Target removal / modulator removal cleanup

- Removing a modulator removes its assignments (`ModGraph::remove` already does).
- Removing a device/track: the identity registry follows the same move/remove closures as `Target`s
  (`IdentityRegistry::retarget_mixer_places`, `follow_mixer_places`), so an owner that disappears makes
  its assignments unresolved. Unresolved assignments are **not** silently dropped; the browser lists them
  as missing, and the engine skips them. The new code additionally prunes assignments owned by a locator
  the project no longer has when the user removes the device (one undo step), so the browser does not
  accumulate dead rows.
- Undo restores all of the above because the graph and registry are both in `history::Side`.

---

## 4. Scope boundaries (v1)

In scope: eight project slots; LFO, Random, Audio Follower; routing-mode assignment; per-target arcs and
live cursor; multiple sources per target; both removal paths; browser Modulation section; persistence;
undo; master-bus follower; live tile plots.

Out of scope, per the handoff: device-level / track-level / per-voice modulation; cross-modulation;
note-triggered LFO retrigger; non-master follower sources; an FFT spectrum display (see *Known limitations*).
The Audio Follower's **detection band is filtered before detection** with cheap IIR filters, which is the
mandatory real-time part; the spectrum drawing is the optional visual part that v1 does not ship. The
model keeps `Band` (Full / Low / Mid / High / Custom with low/high Hz), so the band handles and the
greying-out of out-of-band frequencies can be added without a format change.

## 5. Implementation status (M1b)

Delivered on `modulation-m0`:

- **Model** (`os-model/src/modulation.rs`): `Random` (persisted `seed`, Uniform/Gaussian, smooth, hold/interpolate),
  `AudioFollower` (attack, release, gain, RMS/peak, Full/Low/Mid/High/Custom band), LFO `offset` / `amount` /
  `bipolar`, `ModTarget.enabled`, `Modulator.slot` with `PROJECT_SLOTS = 8` / `NO_SLOT`.
- **Engine** (`os-engine/src/engine/modulation.rs`): Random and follower DSP, per-routing enable, the follower fed
  from the master post-gain bus; the UI snapshot is a preallocated `ModSnapshot` of atomics written by the audio
  thread, sampled into an app-side history ring at 30–60 Hz. No locks or allocations on the audio thread.
- **App** (`os-app/src/modulation.rs`): graph history (one undo step per edit), eligibility, routing state, depth
  drags, snapshot sync, parameter-side listing and removal.
- **Rack** (`os-app/src/views/device_rack.rs`): the 92 px project-modulator row above the device headers — eight
  fixed slots, live mini-plots, on/off LED, routing patch point, empty `+` add menu, anchored slot editor and slot
  context menu; the editor's title is its inline name field. The tile body, scrolling, resizing and collapse are
  shifted by that row and unchanged otherwise.
- **Assignment**: click a slot's patch point (or a knob's Route item) to arm routing mode; every eligible control on
  the selected track is tinted blue and flashes; dragging a control sets its depth (the base value is unchanged).
  Each routed control draws an arc, a draggable depth endpoint and the live cursor; the mixer strips' knobs and
  faders and the docked shared plugin editors' knobs and faders draw the same arcs and handles. Multiple sources can
  drive one control; removal works from the control ("Modulation routings") and from the slot ("Clear routings").
- **Browser**: the Current Project tree has a sibling **Modulation** section, grouped per modulator, live-updating,
  with an inline depth cell (vertical drag), an enable checkbox and a remove button; the row menu also offers go-to,
  enable, +10% / -10% / invert, and remove. A modulator click focuses it and opens its editor; the pencil on its
  group row renames it in place.
- **Persistence**: `.lemon` layout 2 (layout 1 still reads), including slots, seeds, bands and per-routing enable.
  Routing mode itself is UI state and is not persisted or undoable, as decided in §3.16.

Verification (2026-11-13, portable MinGW/libclang):

| Check | Result |
|---|---|
| `cargo test -p os-model` | 49 passed |
| `cargo test -p os-app` | 1212 passed, 0 failed, 62 ignored |
| `cargo test -p os-engine` | 383 passed, 1 failed (the pre-existing 1-ULP assertion in `engine::tests::level_adjustments_scale_and_offset_the_channel`), 1 ignored |
| `cargo build --workspace` | finished green (warnings only) |

The engine failure is a floating-point last-bit difference in an unrelated level-adjustment test (line 3293):
`1.189328688090249` vs `1.1893286880902487`. It is reported here rather than hidden; no modulation test fails.

### Known limitations (v1)

- Cross-modulation, device/track/voice-scope modulation and note-triggered LFO retrigger are not implemented (§3.1,
  §3.3, §4).
- Routing mode lives only in UI state: switching projects or reloading drops it (§3.16).
- The target arc has depth, endpoint dragging and a live cursor, but no curve-editing handle; the model keeps
  `Curve` and the engine honours it, so a curve UI can be added without a format change.
- Arcs and depth handles are drawn on the rack's parameter cells, the mixer strips' knobs and faders (volume, pan,
  separation, sends, slot mix, EQ gain / frequency / width) and the docked shared plugin editors' knobs and faders.
  Docked MIDI FX editors show none: MIDI FX parameters are hosted under their own keys and are not on the
  `ChannelParam` / `SlotParam` target path, so they are not modulation targets in v1.
- Routing-mode tint is still painted only on the rack's parameter grid; mixer strips and docked editor surfaces show
  the arcs of existing assignments but do not flash as eligible targets when routing mode is armed.
- The slot editor and slot menu expose type, waveform, rate, bipolar, offset/amount, seed reroll, distribution,
  hold/interpolate, detection, band, enable and clear/remove. The editor's title is the name field, and the Modulation
  section's group row has a pencil: either opens the shared inline text box in place, selected, and commits one undo
  step. The name saves with the graph and reads back in the browser rows.
- The Audio Follower is master-only and runs one control block (64 frames) behind; the band is filtered before
  detection, but the optional spectrum drawing (greying out-of-band frequencies) is not shipped.
- `ModulatorId` is `u32`, not the `u64` the handoff suggests (§3.11): it is monotonic and never reused, and the
  existing M0/M1 ids and persistence already use `u32`; changing the width would not buy anything at this scale.
- Slot count is a fixed eight; there is no “add slot” path, and a ninth modulator is refused rather than silently
  replacing one.

## 6. Build environment note

The machine's only Rust toolchain is `stable-x86_64-pc-windows-gnu`, and it ships a self-contained
`dlltool.exe` but no `as.exe`; `libloading`/`windows-link` use `raw-dylib`, so `dlltool` runs and fails
without an assembler. The previous `target/` artifacts are MSVC-built and there is no MSVC toolchain now.
For verification, a portable WinLibs MinGW-w64 (UCRT) toolchain and `libclang` were extracted under
`/tmp` and put on `PATH` / `LIBCLANG_PATH`. Nothing in the repository, the system, or global Cargo config
was changed. Baseline `cargo check -p os-model -p os-engine -p os-app` then finished green.
