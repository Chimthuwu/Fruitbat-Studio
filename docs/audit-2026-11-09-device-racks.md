# Device-rack audit — 2026-11-09 (prep for three changes; NOT implemented)

Scope: the audit behind the plan to (1) give each device tile its own wrench (pop out *that* device's
editor alone), (2) make each tile's top bar selectable (click = select, highlight, and Ctrl+D /
Ctrl+X / Ctrl+V act on devices), and (3) MIDI FX / Audio FX / Instrument Racks as container devices,
insertable from the browser and buildable with Ctrl+G over multi-selection. Everything below is
read from the tree; no code was changed.

## 1. How the rack works today (the plumbing the changes plug into)

### 1.1 Model & actions — `crates/os-app/src/device_rack.rs`
- `DeviceKind` (:83): `MidiFx(usize)` / `Instrument(usize)` / `Fx(u16 /*strip*/, u8 /*slot*/)` /
  `NoInstrument`. `Device` (:113): `{ kind, key: Option<usize>, name, on, builtin }`.
- `chain()` (:155) builds the flat signal-flow list for the rack track: MIDI FX (via
  `midi_fx::filled`, :160) → instrument (channel row, :180) or `NoInstrument` placeholder (:190) →
  the mixer strip's effect slots (:198). `rack_track()` (:148) = pinned lane or selected lane;
  `rack_chain()` (:217) wraps it. The doc-comment at :12-20 already reserves the seam:
  “`(future) Container { children }` — a group device holding sub-nodes … the view layer can choose
  to render a device with children as a nested container”. There is **no `Container` variant yet**;
  this is where change (3) lands.
- `RackAct` (:340): the full action enum — `Splitter`, `TogglePin`, `ToggleCollapsed`,
  `SelectTile(usize)` (:349, currently `#[allow(dead_code)]` — defined, dispatched, but never
  constructed by the view!), `Menu(usize)`, `MenuClose`, `PopOut(usize)`, `ToggleEditors`,
  `Dock(usize)`, `ToggleOn(DeviceKind)`, `Add`/`AddAt(Option<u8>)`, `TilePress(DeviceKind)`,
  `AddDevice`, `Reorder{from,to}`, `ScrollBy(f32)`, `ParamSet{key,id,v}`, `ParamTap{key,id,v}`,
  `Remove(DeviceKind)`, `Replace`.
- `run()` (:476) dispatches every action; note `SelectTile(idx)` already writes
  `s.device_rack_sel = Some(idx)` (:483) — half of change (2)'s selection store already exists.
- Per-device pop-out exists and works: `pop_out(app, key)` (:436) sets
  `device_rack_float.insert(key)` + `plugins.set_detached(key, true)` and opens the editor if
  closed; `dock()` (:450) reverses it (`set_detached(false)`, `close_if_open`). `toggle_editors()`
  (:460) is the header wrench: if `device_rack_float` is empty it pops out **every** key in
  `rack_chain()` (:462), else docks all floated ones. The per-device wrench is literally a
  subset call of the same function per `key`.
- Selection + delete: `rack_sel()`/:223, `rack_sel_mut()`/:228, `delete_selected()`/:687 reads
  `device_rack_sel`, `remove_device()`/:696 routes by kind (`RemoveEffect` command through the
  control plane for FX :703, `midi_fx::remove` :711, `rack::ask_delete` for the instrument :714)
  and clears the selection (:717).
- `reorder()` (:237) + `tile_drop()` (:614) show the established move patterns (MIDI FX reorder
  among slots, FX via adjacent `MCmd::SlotMove`, instrument immovable) — change (3)'s group
  un-group and change (2)'s cut/paste must respect the same segment rules.

### 1.2 View — `crates/os-app/src/views/device_rack.rs` (639 lines)
- `layout()` (:31) lays tiles; `tile_paint()` (:281) draws a tile: header gradient
  (`header_gradient(d.kind)`), LED hit → `RackAct::ToggleOn` (:307), name, a **single** menu `v`
  hit → `RackAct::Menu(key)` (:324-333), and — importantly for (1) and (2) —
  the **whole header is one hit rect** bound to `RackAct::TilePress(d.kind)` (:297-299, drag to
  reorder). Body: `controls()` (:425) draws either the plugin's own editor
  (`rack_ui::ui`, :435, `editor_body` :478) or the parameter grid; the `NoInstrument` tile opens
  the browser on click (:336-343).
- The rack header (:166-246): `＋ Add`, Pin, Hide and the **wrench**
  (`header_icon_button` :235-246 → `RackAct::ToggleEditors`, lit green while
  `!device_rack_float.is_empty()`). This is the button change (1) per-tile-izes.
- The per-device menu (`paint_menu` :594) currently offers only Pop out/Dock (+Replace for the
  instrument, Remove): the per-tile wrench duplicates the Pop out item visually in the header.

### 1.3 App layer — `crates/os-app/src/app.rs`
- `Action::DeviceRack(RackAct)` variant (:270); `act()` routes it at :8723-8725 straight to
  `device_rack::run`. Cursor logic keys off `Drag::DeviceRackSplit/Tile` (:6600-6605).
- State fields: `device_rack_drag/drop` (:1137-1138), **`device_rack_sel: Option<usize>`**
  (:1140, init :1601), `midi_fx: midi_fx::Slots` (:1143), plus `device_rack_float`,
  `device_rack_menu`, `device_rack_params/names/tile_scroll`, `rack_ui_cache`.
- Keyboard (`App::key`, :7866): the device-rack key block sits at :7969-8008 — gated on
  `device_rack_sel.is_some() && playlist.edit.is_none() && menu_open.is_none() &&
  device_rack::visible()`; handles ArrowLeft/ArrowRight (move selection) and
  Delete/Backspace (`device_rack::delete_selected`, :8000). **This is the exact block change (2)
  extends with Ctrl+D/X/V and change (3) adds Ctrl+G.** It runs *before* `playlist::key`
  (:8015), so a rack-selected device wins over the playlist's Ctrl+D — good.
  The playlist's Ctrl+D lives in `playlist.rs` (`Cmd::DuplicateTrack` :253, `duplicate_tracks()`
  :2207, hint :3122; range semantics via `track_selected`/`select_track` :4316/:4326 with
  `track_anchor` — the multi-select pattern change (2) should mirror for tiles).

### 1.4 Window keying — `crates/os-app/src/plugins.rs`
- Key spaces: `FX_KEY = 1<<24` (:315, `fx_key(track,slot)` :446), `MEMBER_KEY` (container members,
  :317-ish, keys ≥ MEMBER_KEY), `MIDI_FX_KEY = 1<<27` (:324, `midi_fx_key(track,slot)` :327);
  rack rows use the bare row index (container *channels*: `container::place_key` returns the row
  itself for `k == CHANNEL`, container.rs :236).
- `set_detached` (:1573), `window_of` (:1567), `close_if_open` (:1681), `drop_key` (:883),
  `rekey(&[(old,new)])` (used by `midi_fx::reorder` :99) — everything the pop-out and
  cut/paste-through-rehost story needs. `rekey` keeps plugin objects (values and open editors
  survive a move) — the mechanism to reuse when a device moves between tracks or into a rack.

### 1.5 MIDI FX — `crates/os-app/src/midi_fx.rs`
In-memory slots per track id (`Slots = BTreeMap<u32, Vec<Option<Slot>>>` :29, `SLOTS=8` :16);
`add` (:43) hosts under `midi_fx_key`, `reorder` (:74) compacts and `rekey`s all slots,
`remove` (:104), `toggle` (:134), `info` (:115). MIDI FX never cross the instrument.

### 1.6 Containers (Patcher) — `crates/os-app/src/container.rs`
- `CONTAINER = "Lemon Container"` (:44); state is a slot plugin's (or channel plugin's) byte blob:
  `encode/decode` of `Member { id, instrument, uid, name, wet, dry, state, cpu, label, color, icon }`
  (:68-123, version 4). Places: `(track, slot)` in the mixer, `(rack_row, CHANNEL=0xFF)` on a
  channel (:64, :186). `place_key` (:236) / `key_place` (:245) map place↔hosted key.
- `wrap_in_container` (:962, slot menu “Wrap in container”/“Wrap all in container”, undo names
  WRAP/WRAP_ALL :338) and `wrap_channel` (:1113, “Patcherize”) already produce containers;
  `Moved`/`Ids` (:663-774) retarget automation/links onto members.
- `drop_container_on_slot` (:1163) / `drop_container_on_rack` (:1181) are the browser-entry drop
  paths; `Target::Container(k)` routes palette picks into members (`container_act` :1219,
  `plugin_menu.rs` pick() :82).
- Members are hosted under `MEMBER_KEY + n` and played by `os_engine::chain::Chain`
  (`sync_containers` :1327). **Containers are *state-of-a-slot-or-channel*, not `DeviceKind`
  variants** — the gap change (3) fills.
- Also relevant: `can_wrap` (:446)/`can_wrap_channel` (:453) guards, `MAX_MEMBERS =
  MEMBER_IDS` (:357, automation-target room — any new rack-container scheme that wants member
  parameters to automate must respect this or add its own item space).

### 1.7 Plugin palette / browser — `crates/os-app/src/plugin_menu.rs`
- `Target` enum: `Target::Rack(Option<u8>)` (:60) is the device-rack browser; `pick()` (:64-84):
  an effect pick → `PlugCmd::Effect { index, slot }` (:76), a generator pick →
  `PlugCmd::Generator` (refined by `plugin_palette::pick_cmd` to reuse the track's channel, :80-82).
  The palette lists rows via `plugin_palette::rows`. Change (3)'s three rack devices are new
  entries in this browser (and/or new `Target` handling).

### 1.8 The 18 os-plugkit simple plugins
`crates/os-plugkit/src/simple.rs` defines the shared `simple::paint(info, canvas, &Values,
&ViewState, hint)` editor framework; `lemon-note-layer`, `lemon-note-selector`,
`lemon-audio-layer`, `lemon-audio-selector` are the existing layer/selector *containers*
(todo.md :84-110). They are plugins the rack can host — i.e. change (3) can ship v1 by exposing
rack-container *devices* that wrap those plugins, but the Ctrl+G grouping should be app-side
(container state in the project), not a new plugin per group.

### 1.9 Docs
- `docs/track-model-plan.md` §7/§9: track selection drives the rack; the chain is MIDI FX →
  instrument → strip slots; the `Container { children }` future variant is anticipated here.
- `docs/clip-launcher.md`: launcher tracks feed the rack through `rack_track` (pinned lane).
- `todo.md` :47-50 documents today's header wrench (“pops every device in the chain out…”);
  :192-197 lists “Container-nesting seam: `DeviceKind::Container` in chain for future
  Instrument Rack…” as known remaining polish — change (3) is that item.

## 2. Change (1) — per-device wrench button in each tile's top bar

**Behavior spec.** Each tile's top bar gains a small wrench icon (Tabler `tool`,
`icons::Icon::Wrench`) at the header's right (between the name and the `v` menu). Lit green while
*this* device's editor is floating; click pops that device's editor out alone (`PopOut`) /
docks it back (`Dock`). The rack-header wrench keeps its current all-or-nothing meaning, but its
“lit” state stays “any editor out”.

**Reuse (almost everything).**
- State: `device_rack_float: BTreeSet<usize>` already keys floated editors per key; 
- Actions: `RackAct::PopOut(key)`/`Dock(key)` already implement exactly per-device toggle via
  `device_rack::pop_out`/:436 and `dock`/:450. Nothing new in `device_rack.rs` except a
  convenience `editor_floated(s, key) -> bool` (or just read `s.device_rack_float` in the view).
- Icon: reuse `header_icon_button` (views/device_rack.rs :551) — it is generic (rect, icon, on,
  action, hint).

**Edits (view only; zero app.rs changes).**
1. `views/device_rack.rs` `tile_paint()`: reserve header room — the name clip (:316-323) shrinks
   by another ~26 px (name clip currently `tile.w - 24 - 44`), then add, right-to-left from the
   menu `v` rect (:324):
   `let wr = Rect::new(tile.right() - 22.0 - 24.0, head.y + 4.0, 17.0, 15.0);`
   and, when `d.key.is_some()`, draw
   `header_icon_button(ui, wr, Icon::Wrench, s.device_rack_float.contains(&d.key.unwrap()),
       Action::DeviceRack(RackAct::PopOut(k)), "Pop this device's editor out into its own window")`
   — with the action flipping to `RackAct::Dock(k)` and the hint to “Dock” when floated
   (mirroring `paint_menu` :627-631). Since `header_icon_button` already registers its own
   `ui.hit`, place its hit *before* the header-wide `TilePress` hit conceptually is fine: hits are
   registered in paint order and later/inner hits win in this UI kit's stacking (the LED and `v`
   already take their clicks away from the header drag, :296-299 comment), so put the wrench
   hit after the `TilePress` hit like the LED and menu are.
2. Optional nicety: extend the device menu (`paint_menu`) unchanged — the wrench is the fast path,
   the menu keeps Pop out/Dock for discoverability.

**Tests to add** (in `device_rack.rs` tests / app tests, following `pop_out_and_dock_change_the_
window_state` :870): a painted tile registers a `RackAct::PopOut(key)` hit; clicking it floats
only that key (`device_rack_float` contains exactly one) while `toggle_editors` is untouched; a
second click docks it.

**Step order.** This is self-contained: view-only + maybe one helper. Do it first (smallest risk,
ships a user-visible win, no model changes).

## 3. Change (2) — selectable tile top bar + Ctrl+D / Ctrl+X / Ctrl+V on devices

**Behavior spec.** Clicking a tile's top bar (name / LED area / anywhere not taken by the LED,
wrench or menu) selects the device: selection is drawn as a slight brightness lift (e.g. the
header's veil alpha drops from 110 to ~70 and/or a 1 px accent stroke around the tile — mirror how
the launcher paints selected rows with an accent edge). Selection feeds `device_rack_sel`, which
already drives Arrow-key movement and Delete (app.rs :7971-8008). New keys, when a tile is
selected and the rack is visible: **Ctrl+D** duplicates the device (inserts the copy right after
it in its segment, selects the copy); **Ctrl+X** cuts (removes to an app-held clipboard),
**Ctrl+V** pastes into the selected position/track. `NoInstrument` is not selectable (it is the
“Add instrument” placeholder).

**Existing pieces to reuse.**
- `RackAct::SelectTile(usize)` exists and `run()` already stores into `device_rack_sel`
  (:483-486); only the view never emits it (dead-code allow at :348). Just wire the hit.
- The keyboard block at app.rs :7971-8008 is the insertion point for the new keys (add
  `Key::Character(c) if ctrl && c.eq_ignore_ascii_case("d")` arms etc. *inside* the
  `handled` match; note the rack block must also swallow Ctrl+D/X/V before `playlist::key` — it
  already sits before it, and the fall-through `_ => false` keeps unrelated keys global).
- Device removal already has one canonical path: `device_rack::remove_device` (:696) — cut = that
  plus a clipboard write; it clears selection (fine: cut selects the clipboard, not the tile).
- Device creation paths to reuse for duplicate/paste:
  - FX: control-plane `Command::AddEffect` / mixer `MCmd::AddEffect` (see `two_effects` test in
    container.rs :1534) + copying the slot's `SlotPlugin { name, state }` (os_model) and hosted
    instance — best via a new control command `DuplicateEffect { track, slot }` that snapshots
    `SlotPlugin` and re-hosts under the *new* `fx_key` (the old instance stays at its key;
    `Plugins::create_with`-style host of the same plugin index with `plugin.save_state()`.
  - MIDI FX: `midi_fx::add` (:43) with `slot: Some(next)` and, instead of defaults, restore the
    copied state after hosting (a small `midi_fx::duplicate(app, track, slot)` helper mirroring
    `add` but seeding state from the source instance).
  - Instrument: duplicating a device that *is the track's channel* is a track-level act — v1:
    Ctrl+D on an instrument duplicates the *channel* (`rack::` add-channel with the plugin +
    state + name) and retargets the rack's instrument slot to the copy? No — a track holds one
    instrument; simplest correct v1: Ctrl+D on the instrument tile is a no-op with a hint-bar
    message (“Duplicate the track instead (Ctrl+D in the playlist)”), exactly like the instrument
    is immovable in `reorder`/:297. Cut of an instrument likewise no-op. This keeps the segment
    rules intact.
- Multi-select groundwork: the playlist's `track_anchor` pattern (:4316-4329) is the template if
  tile selection later needs Shift ranges — not needed for v1 (single selection is already
  modeled by `Option<usize>`).

**Edits.**
1. `device_rack.rs`: new `RackAct` arms or free fns: `duplicate_selected(app)` /
   `cut_selected(app)` / `paste(app)`; add `Command`-level support where undo matters (a
   `DuplicateEffect`/`AddEffect`+state pair through `crate::control::apply` like `RemoveEffect`
   at :704, so duplicate/undo stay one step).
2. `views/device_rack.rs` `tile_paint()`: replace the unconditional header `TilePress` hit
   (:297-299) with: if pressed without drag movement → select. Simplest faithful approach:
   register `ui.hit(head, Action::DeviceRack(RackAct::TilePress(d.kind)), ...)` as today (drag)
   *and* a click-on-select via `SelectTile(idx)` — the kit distinguishes click vs drag on the
   same rect via `Drag::DeviceRackTile` lifecycle (the drop clears `device_rack_drag`/
   `device_rack_drop`, `tile_drop` :614); so: emit `SelectTile(i)` on press, keep
   `TilePress` for the drag start — i.e. in `run()`, `RackAct::SelectTile(i)` sets the selection
   *and* the existing `TilePress` arm (:510) continues to start the drag; alternatively bind
   selection to press and let the drag simply also have selected first (Bitwig selects on press
   too). Recommend: `SelectTile` on left-press, `TilePress` behavior unchanged beneath it; a
   click that never moves leaves the selection set — which is the wanted UX.
   Paint: if `s.device_rack_sel == Some(index-in-layout)`, lift the header veil (:291 alpha 110 →
   ~70) and stroke the tile with `t.glyph`/accent at low alpha; body unchanged.
3. Clipboard: `State` field `device_rack_clip: Option<ClipDevice>` (enum over
   `Fx{track,slot}` snapshot incl. `SlotPlugin` state, `MidiFx` snapshot, nothing for
   instrument). Paste targets: the selected FX's strip/next slot, or the gap slot from
   `AddAt` semantics; cross-track paste = same helpers with the target strip/track id.
4. Keyboard arms in app.rs :7977 match (see above). Menu parity: add Duplicate/Cut/Paste to
   `paint_menu` (PItem with `.key("Ctrl+D")` like playlist's :3122) — cheap and keeps the menu
   honest.

**Tests:** selection on click (`SelectTile` sets `device_rack_sel`), brightness affects paint
(hit rect for the wrench/menu still wins), Ctrl+D duplicates an FX and a MIDI FX (state copied,
source intact, copy selected), Ctrl+X then Ctrl+V restores (same state bytes, new slot), undo
steps named, instrument no-ops.

**Step order.** After (1): it touches device_rack.rs, views, app.rs keys and possibly a new
control command, but stays within the flat chain — no model surgery. Land *before* (3), because
(3)'s Ctrl+G needs the same selection plumbing and the duplicate/clipboard machinery is the
natural place for group/ungroup to reuse remove+insert.

## 4. Change (3) — MIDI FX / Audio FX / Instrument Racks as container devices

**Behavior spec (Bitwig/Ableton style, clean-room).** A rack container is one device tile in the
chain that *holds* other devices: its body shows its own nested chain (MIDI FX Rack = MIDI-FX-slot
devices before an optional internal instrument...; Audio FX Rack = effect slots; Instrument Rack =
instrument devices), with add/remove/reorder of its children, on/off per child, and the container
tile participates in the outer chain as one device. The three racks differ only in what children
they accept (notes-transforming MIDI FX / audio effects / instruments). Insertable from the
device browser (like any device), and **Ctrl+G** with ≥2 selected tiles groups those tiles into a
new rack container at the first selected device's position; a container's menu/group action can
ungroup it back in place. Nested containers are allowed (Bitwig nests freely) — v1 may forbid
nesting; recommend allowing it but flagging engine scope.

**The decisive architecture fact.** The codebase already has *two* container notions that must
now meet:
- `container.rs`'s `Lemon Container`: mixer-slot / channel-wrapper state, members played by
  `os_engine::chain::Chain`, wet/dry per member, automation via member items (`MEMBER_IDS` cap),
  `ContainerUi` window. It exists per *place* (slot or channel), not as a rack-chain node.
- `device_rack::DeviceKind`'s reserved-but-unimplemented `Container { children }` seam
  (:14-20) — the rack-chain node.
And it already has parallel data homes for chain devices:
- MIDI FX: `midi_fx::Slots` in-memory per track (:29) — *not yet in the .flp* (todo.md :100-104:
  the engine note-path milestone owns persistence).
- Audio FX: mixer `slots` (os_model, persisted).
- Instrument: the track's channel.

**Recommended build: a real `DeviceKind::Container` on top of `Lemon Container` state, layered in
three steps.**

**Step 3a — model + chain plumbing (the risky part, do it alone).**
1. Extend `DeviceKind` (:83) with `Container { place: (u16, u8) }`-shaped variant — reuse
   container.rs's addressing: for an **Audio FX Rack** the natural home is a mixer slot whose
   `SlotPlugin.name == CONTAINER` (already plays through `Chain`!); for an **Instrument Rack** a
   channel whose plugin is `CONTAINER` (`is_container_channel` :181); the **MIDI FX Rack** has no
   engine home yet — mirror `midi_fx::Slots` with an in-memory `midi_racks` map until the
   note-path milestone lands (its members are note processors, not audio, so they cannot reuse
   `os_engine::chain::Chain` as-is; the engine work belongs to the M9 note-path refactor — see
   todo.md :100-104).
2. `chain()` (:155): a filled FX slot holding `CONTAINER` currently appears as one `Fx` device
   named by `slot_title`. Route it to `DeviceKind::Container` (kind-aware header gradient — add
   `HEADER_GRAD_CONTAINER` next to :97-99). Children come from `container::members(s, t, k)`
   rendered as nested mini-tiles (the view decides nesting; `chain()` stays flat with a
   `parent`/`depth` or the view unpacks children itself — recommend `chain()` gains a nested
   representation only at the view: keep `Device.kind = Container` and have the view call
   `container::members` for the body, exactly like the `ContainerUi` window does).
   For the Instrument Rack, the rack track's *channel being a container channel* maps to
   `DeviceKind::Container` instead of `Instrument` (its member instruments play the notes —
   `sync_containers` :1336 already hosts channel containers).
3. Storage: audio FX racks and instrument racks persist for free (slot / channel plugin state).
   Undo: reuse the container.rs edit pattern (`control::edit` + `set_members`, :477).

**Step 3b — insert from browser + Ctrl+G.**
1. Browser: add three entries — either as new palette section heads (rows in
   `plugin_palette::rows` under Target::Rack) or as `plug_menu` items producing new commands
   `Cmd::MakeRack{Kind, at}`: for Audio FX Rack, `MCmd::AddContainer(slot)` already exists
   (container.rs :1171 `drop_container_on_slot` uses it) — the browser pick just calls the same;
   for Instrument Rack, the empty-container-channel creation inside `drop_container_on_rack`
   (:1181-1196) becomes a callable; MIDI FX Rack needs a new in-memory slot-keeper
   (a `midi_racks: BTreeMap<u32 /*track*/, ...>` sibling of `midi_fx::Slots`, plus later
   persistence with the note-path milestone).
2. Ctrl+G (keyboard block app.rs :7977, after change (2) lands): operates on the rack's
   multi-selection. Requires upgrading selection from `Option<usize>` to a `BTreeSet<usize>`
   (click = single-select, Ctrl+click toggles, Shift+click range — mirror
   `playlist::select_track` :4326; keep `rack_sel()` as “last clicked” for Delete/arrows). Then
   `device_rack::group_selected(app)`:
   - audio devices spanning *one strip* → wrap their slots: `wrap_in_container`-style member
     extraction but written as a new function that lifts the selected slots (in chain order)
     into one `CONTAINER` slot at the first one's position, clearing the others (reuse
     `wrap_in_container` :962 nearly verbatim with `slots` = the selection, then move the
     resulting container slot to the first position via adjacent `SlotMove`s);
   - instrument selection: only meaningful when the track's instrument + selected MIDI FX are
     grouped → an Instrument Rack channel wrap (reuse `wrap_channel` :1113); MIDI-FX-only
     grouping → a MIDI FX Rack entry in the in-memory keeper.
   - Undo: one step named `"Group in container"`; retarget automation with the existing
     `Moved` machinery (:751) — that is the hard-won part, do not reinvent it.
3. Ungroup: mirror — a container device whose all-members-selected (or menu “Ungroup”) dissolves
   in place (inverse of the wrap; reuse `Moved` retargeting in reverse via a fresh move set).

**Step 3c — rack tile UX.**
- Nested body: reuse the `ContainerUi` list visuals at tile scale, or — better fit for the rack —
  render children as sub-tiles in the body using the existing `tile_paint` recursion guarded by
  depth; children get LED/params as today; the container tile's header shows “Instrument Rack”
  etc.
- Wrench (change 1) must work per-child and per-container (child keys are `MEMBER_KEY`-based
  hosted keys — `open_member_key` :1270 shows the pattern for opening member windows).
- Drag-to-reorder: children reorder within the container (container.rs `Op::Up/Down`/ids already
  move members keeping instances); dragging a child *out* of a container and a whole container
  reorder are stretch goals, defer.

**Open risks / decisions to confirm before implementing.**
- **Engine note path**: MIDI FX Rack (and instrument-member processing) depends on M9's
  note-path refactor; ship Audio FX Rack + Instrument Rack (both ride existing engine chains)
  first, MIDI FX Rack as UI + in-memory state.
- **Parameter/automation room**: reusing `Member` items caps members at `MEMBER_IDS` and couples
  rack containers to the slot automation model — acceptable, and consistent.
- **`Device.key` for containers**: a container tile needs a window key for its own UI; for a slot
  container that is `fx_key(t,k)` (`place_key` :236); for a channel container the row. Define
  `DeviceKind::Container`'s key accordingly and let `pop_out` work unchanged — the container's
  *window* is its member-list `ContainerUi`... which today is a `Win::Container` docked window,
  not a hosted plugin window; decide: v1 = per-tile wrench opens `Win::Container` via
  `container::show` (:389), not the floating-plugin machinery. Document this divergence.
- **Selection widening** (`BTreeSet`) touches `delete_selected` and arrows — small, but do it in
  its own commit before Ctrl+G.

## 5. Smoothest overall order

1. **(1) per-tile wrench** — views/device_rack.rs only; tests; commit. No model change.
2. **(2) selection + clipboard** — first upgrade selection to the multi-select set
   (`BTreeSet` + `track_anchor`-style anchor), paint highlight, then Ctrl+D/X/V with the
   duplicate helpers (`midi_fx::duplicate`, FX slot-copy control command), instrument no-ops.
3. **(3a) Container device kind** for audio FX rack + instrument rack (both have engine chains
   already); nested tile rendering; browser entries via `AddContainer`/empty-container-channel.
4. **(3b) Ctrl+G group/ungroup** reusing `wrap_in_container`/`wrap_channel` + `Moved` retargeting.
5. **(3c) MIDI FX Rack** once the M9 note-path milestone lands (members need engine processing
   to matter); until then its browser entry can exist but warn.

Each step is independently shippable and lands `cargo test -p os-app` green (currently 1129
passing, todo.md :79).

## 6. Quick symbol index (for the implementer)
`RackAct` device_rack.rs:340 · `run` :476 · `pop_out` :436 · `dock` :450 · `toggle_editors` :460 ·
`SelectTile` :349/:483 · `remove_device` :696 · `delete_selected` :687 · `reorder` :237 ·
`tile_drop` :614 · `chain` :155 · `rack_track` :148 · `header_gradient` :102 ·
`tile_paint` views/device_rack.rs:281 · header hits :297-333 · `paint_menu` :594 ·
`header_icon_button` :551 · app.rs `Action::DeviceRack` :270, dispatch :8723, state :1137-1143,
rack keys :7969-8008, `act` :8701 · plugins.rs `FX_KEY` :315, `MIDI_FX_KEY` :324, `midi_fx_key`
:327, `fx_key` :446, `set_detached` :1573, `window_of` :1567, `close_if_open` :1681, `drop_key`
:883 · midi_fx.rs `add` :43, `reorder` :74, `remove` :104, `toggle` :134 ·
container.rs `CONTAINER` :44, `CHANNEL` :64, `Member` :68, `encode/decode` :98/:136,
`place_key` :236, `members` :206, `wrap_in_container` :962, `wrap_channel` :1113,
`drop_container_on_slot` :1163, `drop_container_on_rack` :1181, `sync_containers` :1327,
`Moved::retarget` :751, `MAX_MEMBERS` :357 · plugin_menu.rs `Target::Rack` :60, `pick` :64.
