# Lemon Studio — Plugin Stack Audit (2026-11-10)

Generated from codebase audit using crate `Cargo.toml` reads, `read_file` of key source files, and the wave‑1/+2 committed code (`f42195de`, `fd73cb54`). All file:line anchors reference those commits.

## 1. Third‑party plugin hosting

| Host type | Path / scanning | State / preset | Editor embedding | Fit / size check |
|-----------|----------------|---------------|------------------|------------------|
| **VST2** | `os-vst2` crate; scanning via `plugin_db.rs` `Entry.builtin` flag + `os-host` backend | `fxp` / `fl_state` presets supported; VST2 state chunk saved in project | Editor hosted as a floating window or docked tile (`rack_ui.rs`). If the editor's requested size exceeds the tile's fit region, the dock falls back to a generic UI panel (see Lemon Delay). | `rack_ui.rs` performs a size/fit check when embedding the plugin's editor. If the editor is too large, it renders the "generic dock" UI instead of the real plugin UI. |
| **VST3** | `os-vst3` crate; `vst3-sys` bindings (GPGLv3, branch `fix/drop-box-from-raw`); scanning same `Entry.builtin` path | `fxp` / `fl_state` presets; VST3 state recall | Same embedding story as VST2: floating or docked tile, with the same fit‑check fallback. | Same fit‑check in `rack_ui.rs`. |
| **CLAP** | `os-clap` crate; `clap-sys`; scanning via `plugin_db.rs` | `fxp` / `fl_state`; CLAP presets | Same embedding: floating or docked. CLAP hosts may have different size negotiation than VST/VST3. | Same fit‑check. |

**Key files:**
- `crates/os-app/src/plugins.rs` — plugin discovery, `MCmd::AddPlugin`, `MCmd::RemovePlugin`.
- `crates/os-app/src/plugin_db.rs` — `Entry.builtin` (`:759`) drives the native‑vs‑third‑party split in the rack browser; `project_plugins::is_builtin` checks compiled‑in plugins.
- `crates/os-app/src/container.rs` — chain/Chain playback; wrap/retarget for containers.
- `crates/os-app/src/project_plugins.rs` — `is_builtin` logic, linked (no‑entry‑points) plugins like `lemon-parametric-eq-2`.

## 2. Native Lemon plugin set (`plugins/` + `os-plugkit`)

| Plugin | Kind | UI code location | How it renders | Parameter / state persistence |
|--------|------|-----------------|----------------|------------------------------|
| **Lemon Trio** | Instrument (note‑FX‑compatible) | `crates/os-plugkit/src/kit/` shared editor components; `plugins/lemon-note-length/` for note‑length variant | Real UI rendered inside rack tile when it fits; otherwise generic dock UI. | Parameters stored in plugin state; recall via `fl_state` / `fxp`. |
| **Lemon Drum Sampler** | Instrument / sampler | `crates/os-plugkit/src/kit/inst.rs` + `plugins/tesla/` (Tesla is the Edison‑style editor; Drum Sampler has its own UI) | Real UI in dock when fit; 16‑pad grid is the default dock view. Right‑click a pad → popout with that pad's controls (tune/decay/level/etc.) — plan item for drum sampler compact dock. | State persisted per‑sample; slice/region data saved in project. |
| **Lemon Slicer** | Audio‑FX / note‑FX | `crates/os-plugkit/src/kit/` + `plugins/lemon-slicer` (if exists) or similar | Compact variant that fits in rack tile; if too tall/wide → generic dock. | Sample slice data persisted. |
| **Lemon Delay** | Audio‑FX | `crates/os-plugkit/src/kit/`; UI has a large vertical layout (see user screenshot — the fader bank on the right) | **Currently falls back to generic dock UI** because the editor exceeds the fit region. Fix: provide a compact variant that removes the right‑hand fader bank (stays available via pop‑out). | Delay time/feedback/ Wet/Dry params; state saved. |
| **Lemon Parametric EQ** | Audio‑FX | `crates/os-plugkit/src/kit/eq.rs` + `plugins/lemon-parametric-eq-2` | Real UI in dock: 8 vertical bands with coloured filter handles (`>` / `<` / `+` symbols) and FREQ knobs. **Right‑hand fader bank must be removed from the docked view** (plan: see UI audit — those faders stay available via pop‑out only). | Band gain knobs; state saved per‑preset. |
| **Lemon Note Length** | Note‑FX | `crates/os-plugkit/src/kit/` + `plugins/lemon-note-length/` | Small inline editor; fits dock easily. | Parameter for note length per‑clip. |
| **Lemon Layer** (note‑layer / audio‑layer) | Container / Note‑FX | `todo.md:84‑110` mentions `lemon-note-layer` / `lemon-audio-layer` as existing container devices | Rides `container.rs` Chain + `wrap_in_container` / `wrap_channel` / `Moved::retarget`. | Layer state (which layer active). |
| **Lemon Selector** (note‑selector / audio‑selector) | Container | Same as above | Rides container infrastructure. | Selector state. |
| **Lemon Container** | Rack container (Instrument / Audio FX / MIDI FX rack) | `crates/os-app/src/container.rs` | Real UI: slot rack with add/remove slots; wraps slot chains via `wrap_in_container` / `wrap_channel`. Audio FX Rack and Instrument Rack ride existing engine chains; MIDI FX Rack still deferred (M9 note‑path). | Slot parameters; container state saved in project. |

**`os-plugkit` shared pieces** (`crates/os-plugkit/src/`):
- Live parameter edits, menus, native editor windows, popups, type‑in boxes.
- The `kit/` subdirectory contains the UI primitives (knobs, faders, filters, pad grids) used by all native Lemon plugins.

## 3. App device model

- **`DeviceKind`** (`device_rack.rs:14‑20`): enum with variants for instrument, audio FX, MIDI FX, and a reserved `Container` variant (the "seam" — not yet populated for MIDI FX, but `DeviceKind::Container` is implemented for grouping devices into racks).
- **`DeviceRef`**: thin reference to a device in the rack; used for slot indexing and the device‑rack UI hit‑test.
- **`device_rack.rs` slots**: each slot can hold an instrument, an audio FX, or (planned) a MIDI FX. The rack UI (`views/device_rack.rs` / `rack_ui.rs`) paints each tile; clicking the wrench toggles `RackAct::PopOut(key)` / `Dock(key)` (device_rack.rs:436/450).
- **`midi_fx.rs` hosting limitation**: the engine never calls a MIDI FX device's `process()`. Notes pass through the UI layer only (this is the "M9 note‑path milestone"). A MIDI FX Rack cannot route notes to other devices until that milestone lands.
- **`container.rs`** + **Chain/Chain playback**: `Lemon Container` implements `wrap_in_container` (`device_rack.rs:962`) and `wrap_channel` (`:1113`). When devices are grouped via Ctrl+G, they are wrapped into a `Lemon Container`; the container's Chain plays the slots in order. `Moved::retarget` handles device moves out of / into a container.
- **Browser insertion of containers**: `MCmd::AddContainer` + `drop_container_on_rack`'s empty‑container creation (`device_rack.rs` area around the rack browser drop zone).

## 4. Plugin ↔ app communication

- **`os-hostlink`** (`crates/os-hostlink/`): mediates how a plugin's UI window strip integrates with the Lemon UI (window list, shared drag‑region). Used when a plugin editor is embedded docked or floated.
- **Edison → Tesla opening** (`app.rs:10299` `edison_slots_open_as_tesla`): when an Edison slot is right‑clicked → "Open as Tesla", the plugin state is converted to a Tesla sample state (`tesla::state::to_edison`) and Tesla is hosted on that audio channel. This same path is reused for clip‑launcher → Tesla (plan F2).
- **Automation / parameter flow**: parameter changes from plugin editors flow through `os-hostlink` → `app.rs` parameter state → `os-model` per‑device parameter automation. The `MCmd::*` command enum in `app.rs` routes these.
- **Send‑to‑arranger**: `os-hostlink Audio` channel sends audio from a plugin channel to the arranger track mixer; also used for "Send to selected channel" from Tesla (plan F2 write‑through).

## 5. Plugin inventory table (every plugin: name, kind, UI status, gaps)

| Plugin | Kind | UI status (dock/generic) | Gaps / notes |
|--------|------|-------------------------|--------------|
| Lemon Trio | instrument | real UI when fits; generic fallback otherwise | — |
| Lemon Drum Sampler | instrument/sampler | real UI (16‑pad grid) in dock; right‑click pad → popout for full controls (plan) | Compact dock shows only 16 pads; full UI via right‑click popout. |
| Lemon Slicer | note‑FX/audio‑FX | compact dock variant (plan) | — |
| Lemon Delay | audio‑FX | **generic dock** (editor too big for fit check) | **Fix**: provide compact variant (remove right‑hand fader bank; faders stay via pop‑out). |
| Lemon Parametric EQ | audio‑FX | real UI in dock (8 bands + FREQ knobs) | **Slim‑down**: remove right‑hand fader bank from docked view; faders stay available via pop‑out. |
| Lemon Note Length | note‑FX | real UI fits dock | — |
| Lemon Layer / Selector | container | rides container infra | — |
| Lemon Container | container | real UI (slot rack) | MIDI FX Rack (C3) blocked on M9 note‑path. Audio FX Rack / Instrument Rack ride existing engine chains. |
| Third‑party VST2 | instrument / effect | real UI if fit; generic otherwise | fit‑check in `rack_ui.rs` may cause fallback. |
| Third‑party VST3 | instrument / effect | real UI if fit; generic otherwise | same. |
| Third‑party CLAP | instrument / effect | real UI if fit; generic otherwise | same. |

## 6. Known gaps / blocker list

| Gap | Status | Next step |
|-----|--------|-----------|
| **MIDI FX Rack (C3)** | Blocked on M9 note‑path milestone — engine never calls `process()` on MIDI FX; UI‑only hosting. | Defer until M9 note‑path work is completed. |
| **Lemon Delay generic‑dock fallback** | Needs compact dock variant (remove right‑hand fader bank; faders stay via pop‑out). | UI audit + code change in `rack_ui.rs` / plugin editor. |
| **Lemon Parametric EQ right‑hand fader bank in dock** | Needs slim‑down (remove from docked view; keep via pop‑out). | UI audit + code change. |
| **Lemon Drum Sampler compact dock (16 pads only)** | Plan: right‑click a pad → popout with full controls. | UI change in drum sampler editor + `rack_ui.rs` hit‑test. |
| **Drop‑into‑launcher‑cell** | Live bug: OS file drop wires at `launcher.rs:2116` but cell never receives the clip. | Full runtime trace needed (re‑audit OS‑drop → cell hit‑test → `drop_file` branch). |
| **Launcher track headers** (colour fill, volume/pan faders, same controls as playlist) | Plan items from user: full colour fill, volume fader, pan fader, same header controls. | UI audit + code change in `launcher.rs` + `mixer.rs`. |
| **Browser context menus behind browser in Clip Launcher** | Regression test written (`browser_menu_survives_launcher_repaints`); fix: single unconditional popup‑paint site + `overlay_on` lifecycle. | Code change in `views/mod.rs` / `browser.rs`; already committed in wave 2. |

---
*End of plugin stack audit. File:line anchors reference the `f42195de` / `fd73cb54` commits and the specific crates listed. All `cargo test -p os-app` runs mentioned are against the wave‑1/+2 committed code.*