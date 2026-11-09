# Audit 2026-11-09 — Device-rack browser ordering & playlist track operations

Scope: `crates/os-app` (browser palette ordering; playlist track selection / delete / move / group).
Status: read-only audit, no code changed. Prepares the four changes below.

---

## 1. Browser inside the device rack — device/plugin section ordering

### Where the ordering is decided

- The rack's device browser is **the plugin palette in `Target::Rack` mode**, not the file browser
  (`browser.rs` is a file-browser; it has no device ordering logic).
- `crates/os-app/src/plugin_palette.rs`
  - `rows()` — `plugin_palette.rs:203`. Entry point: when `matches!(p.target, Target::Rack(_))` it
    delegates to `rack_rows()` (line 205–206).
  - `rack_rows()` — `plugin_palette.rs:287` (doc comment at 285–286): "MIDI FX first (all-native), then
    Instruments and Effects, each led by a 'Lemon' group of the built-ins before the installed ones".
    Structure, in the no-query branch:
      1. `MIDI FX` header + `Row::MidiFx { at }` rows from `p.midi_fx` (`plugin_palette.rs:313–321`)
      2. `Instruments` header + `rack_section(s, false)` (gens) (`plugin_palette.rs:322–323`)
      3. `Effects` header + `rack_section(s, true)` (fx) (`plugin_palette.rs:324–325`)
    On a MIDI track all three sections show; on an audio track only `Effects` (`midi` flag,
    `plugin_palette.rs:290` via `crate::device_rack::rack_track`).
  - `rack_section()` — `plugin_palette.rs:336`: for each list, splits `builtin` vs rest; renders
    a `Lemon` sub-header with all built-ins first, then the rest by the palette's display order
    (Favourites, then `MenuView::Simple | Categories | Vendor`, `plugin_palette.rs:355–384`).
  - Query path (`q` non-empty) sorts merged hits by `(rank, !favorite, label)` — no sectioning;
    `plugin_palette.rs:291–311`.
- The `builtin` flag comes from `crates/os-app/src/plugin_db.rs`: `Entry.builtin` (`plugin_db.rs:759`),
  set via `crate::project_plugins::is_builtin(d)` (`plugin_db.rs:782`). `is_builtin` lives in
  `crates/os-app/src/project_plugins.rs:24`. Native effects that are compiled in but have no own file
  are covered by `crate::plugins::LINKED_EFFECTS` (`crates/os-app/src/plugins.rs:1928`;
  also used by `builtin_file` in `plugin_db.rs:489–492` and `project_plugins.rs:67/74`).
- Related but separate surfaces (unchanged by this task):
  - Global palette (non-Rack) ordering: `plugin_palette.rs:207–283` — `Effects`/`Instruments` heads by
    `shown_kinds`, no built-in-first split.
  - Format/category ranking inside the plugin db: `plugin_db.rs:658` (`db_rank`: built-ins first, then
    preferred format, then VST3/CLAP/VST2/native).
  - F8 plugin menu: `plugin_menu.rs`; plugin manager/library listing: `project_plugins.rs:570`.
  - MIDI FX registry itself: `midi_fx.rs` (`Slots`, `add()` at `midi_fx.rs:52`); the rack shows them as
    `Row::MidiFx { at }` picking into `midi_fx::add`.

### Current ordering as-implemented

```
MIDI FX                    (all native, from Plugins::midi_fx)
Instruments
  Lemon          (built-in generators)
  Favourites     (third-party)
  Others/Categories/Vendor  (third-party)
Effects
  Lemon          (built-in audio effects)
  Favourites     (third-party)
  Others/…       (third-party)
```

### Gap vs. the requested order

Requested: MIDI FX → **all Lemon instruments** → **immediately the Lemon native audio effects** →
**then all third-party VSTs**. Today the third-party *instruments* (Favourites/Others) sit between the
Lemon instruments and the Lemon effects, i.e. third-party content is split per-section rather than
collected after everything native.

### Minimal change

Rewrite the tail of `rack_rows()` (`plugin_palette.rs:312–326`) to emit four flat groups:

1. `MIDI FX` (unchanged);
2. `Instruments` head + `builtin` rows of `plugdb.lists.gens` (favours included by list order, or
   keep a Favourites subgroup *inside* the Lemon group only);
3. `Effects` head + `builtin` rows of `plugdb.lists.fx` (same treatment);
4. a single third-party tail: the non-builtin gens then non-builtin fx (each in the current
   Favourites/others or Categories/Vendor arrangement via `rack_section`'s tail logic).

Cheapest shape: refactor `rack_section()` into two helpers — `native_rows(list)` (built-in subset) and
`third_party_rows(list)` (the current favs/view code) — and have `rack_rows` call
`native_rows(gens)`, `native_rows(fx)`, then `third_party_rows(gens)`, `third_party_rows(fx)`.
`Entry.builtin` (`plugin_db.rs:759`) already provides the split; no new data, no changes to
`plugin_db.rs` / `project_plugins.rs` / `plugin_menu.rs`. Suggested labels for the tails:
"Instruments" / "Third-party instruments" style headers (`Row::Head { major: … }`).
Query branch (`plugin_palette.rs:291–311`) can stay as-is (ranked search results) — optionally bump
native hits ahead by seeding `hits` with rank `0` for built-ins, mirroring `db_rank`.

Also keep `rack_rows`'s audio-track case correct: `midi == false` renders only `Effects` +
third-party tail (skip step 2 entirely).

Recommended step order: (a) refactor `rack_section` into the two helpers with no behaviour change →
(b) reroute `rack_rows` to the four-group order → (c) add a unit test next to the existing palette
tests (`plugin_palette.rs:1403` `plugin_database_by_category_and_format`) asserting the sequence
MIDI-FX, Lemon-instr, Lemon-fx, third-party.

---

## 2. Playlist track operations

### Selection model today

- `State.playlist.track` = active lane; `State.playlist.track_anchor: Option<u16>`
  (`playlist.rs:647`) — open Shift-range anchor; cleared in `default()` (`playlist.rs:808`).
- `track_selected()` — `playlist.rs:4317–4323`: anchor set ⇒ every lane in `min(a,t)..=max(a,t)`,
  else only the active lane.
- `select_track()` — `playlist.rs:4326–4330`: plain click clears the anchor, Shift sets it
  (`extend && anchor != row`).
- `press_header()` — `playlist.rs:4332` wires clicks; Shift+click extend already works.

### 2a. Delete deletes selected tracks (single & multi)

- Keyboard: `playlist::key()` — `playlist.rs:5549`. `Key::Named(NamedKey::Delete)` ⇒ `Cmd::Delete`
  (`playlist.rs:5657`), which deletes **clips**: `Cmd::Delete` at `playlist.rs:4820–4823`
  (`delete(s, s.playlist.sel)`). There is no track-side Delete key today; tracks are deleted only via
  the track menu ("Delete one", `Cmd::TrackDelete(t)`, `playlist.rs:3124`, handled at
  `playlist.rs:5254–5258` → `snapshot` + `delete_track`).
- Deletion primitive: `delete_track(s, t)` — `playlist.rs:2243–2262`: removes track `t`'s clips via
  `delete()` (`playlist.rs:1732`, index-remapping), shifts clips above `t` up, rotates the lane
  descriptors (`PlaylistTrack`), blanks the last lane, and calls
  `s.project.track_lane_deleted(t)` (Linear-mode lane remap, spec `model/track-model.md`
  "Remap discipline"). Undo is a whole-state snapshot (`snapshot()`, `playlist.rs:1591` /
  `history.rs`) — the same mechanism `Cmd::DuplicateTrack` uses (`playlist.rs:4912–4914`), so a new
  multi-track command needs no separate undo plumbing.
- **Proposed command**: add `Cmd::DeleteTracks` to `Cmd` (`playlist.rs` enum at ~line 237/325 block
  where `DuplicateTrack` (`playlist.rs:253`) and `TrackDelete(u16)` (`playlist.rs:337`) live), handled
  in `run()` beside `Cmd::TrackDelete`:
  ```rust
  Cmd::DeleteTracks => {
      snapshot(s);
      let rows: Vec<u16> = (0..TRACKS).filter(|&r| track_info(s, r).is_some() && track_selected(s, r))
          .collect();
      for &t in rows.iter().rev() { delete_track(s, t); }
      if let Some(last) = rows.first() { select_track(s, *last.saturating_sub(1), false); }
      None
  }
  ```
  Delete bottom-up so each `delete_track`'s index remap stays valid (same trick as
  `duplicate_tracks`, `playlist.rs:2207–2237`). Reuse the collection pattern from
  `duplicate_tracks` (`playlist.rs:2210–2213`) and the guard from the track menu.
- Key binding: in `playlist::key`, route plain `Delete` to `Cmd::DeleteTracks` **only when the range
  is open** (`s.playlist.track_anchor.is_some()`), otherwise keep the existing clip-Delete:
  ```rust
  Key::Named(NamedKey::Delete) if s.playlist.track_anchor.is_some() => Cmd::DeleteTracks,
  Key::Named(NamedKey::Delete) => Cmd::Delete,
  ```
  (match arm order matters — put the guarded arm first at `playlist.rs:5656`). Optionally also expose
  `Cmd::DeleteTracks` in the track menu (`track_menu`, `playlist.rs:3118–3125`).
- Control plane: track mutations today go through `crate::control::edit(...)` only for
  `TrackMode`/linear inserts (`playlist.rs:5201–5205`); `TrackDelete`/`DuplicateTrack` use plain
  snapshots. Follow that precedent — `Cmd::DeleteTracks` stays snapshot-based.

### 2b. Shift+wheel moves all selected tracks as a group

- Wheel routing: `App::wheel` — `app.rs:7822–7853`. The Shift branch at `app.rs:7845–7851`:
  over `Action::PlHeader(_)/PlLed(_)/PlGroupArrow(_)/PlLaneArrow(..)`, computes `d = ±1` and calls
  `crate::playlist::shift_tracks(s, d)` after `snapshot`. Same path handles Shift+Up/Down via
  `Cmd::ShiftTracks` (`playlist.rs:5629–5630`, handled at `playlist.rs:4841–4844`).
- `shift_tracks()` — `playlist.rs:2142–2166`: **clips first** (moves `s.playlist.sel` clips if the
  *clip* selection is non-empty) — so with no clips selected it swaps the active track with its
  neighbour via `swap_tracks(s, t, u)` (`playlist.rs:2169`). Track-selection awareness is absent.
- **Minimal change**: teach `shift_tracks` to prefer the *track* range. When
  `s.playlist.track_anchor.is_some()`, operate on the lane descriptors of the selected block:
  1. compute `lo..=hi` from `track_selected` over `0..TRACKS` (or `anchor..=active`);
  2. clamp `d` so `lo+d >= 0` and `hi+d < TRACKS`;
  3. move the whole block: rotate the `PlaylistTrack` slice by `d` and update every clip's
     `c.track` for lanes that had content in the block (mirror what `swap_tracks` does pairwise,
     `playlist.rs:2169–2180`, but for the range; easiest is `d.abs()` iterations of
     `swap_tracks(s, hi, hi+d)` then descending from `lo` for `d > 0` — preserves `number` relabelling
     logic exactly as written);
  4. keep the anchor semantics: after the move, shift both `track` and `track_anchor` by `d` so the
     group stays selected.
  Because `app.rs:7845` already funnels every Shift+wheel step through `shift_tracks` (one snapshot
  per notch is fine — matches the existing per-notch snapshot), **the only file to touch is
  `playlist.rs::shift_tracks`**; `Cmd::ShiftTracks` (Shift+Up/Down) gets grouped movement for free.
  Note the clip-selection branch must stay first when a *clip* selection exists, to avoid regressing
  Shift+arrow clip nudging (`playlist.rs:2144–2157`).

### 2c. Ctrl+G groups the selected playlist tracks

- Existing group model (FLP-native): `os_model::Track.grouped: bool`
  (`crates/os-model/src/lib.rs:2114–2116`, FLP event 238 "grouped with the track above").
  A group = a maximal run of `grouped` tracks below a leader; leader = nearest ungrouped track above
  (`playlist.rs:1261–1271`: `group_track`, `is_group_track`). Collapse:
  `Cmd::GroupCollapse(t)` → `toggle_collapse` (`playlist.rs:5192–5194`); menu
  ("Group with above track", `Cmd::TrackGroup(t)`, `playlist.rs:3118`), handled at
  `playlist.rs:5177–5181` (plain `tr.grouped = !tr.grouped` toggle with snapshot).
- This is a **binary adjacent-grouping** model (FL-style), not Bitwig-style arbitrary named groups;
  "group selected tracks" therefore maps to: keep the topmost selected lane ungrouped (leader),
  mark every other selected lane `grouped = true`. Non-contiguous selections either become separate
  adjacent groups or get a warning — recommend: group each maximal run separately (each run's top
  lane is its leader), which degrades gracefully and is undoable in one snapshot.
- Ctrl+G is free: `playlist::key` binds only `Shift+G` ⇒ `Cmd::GroupClips` (clip grouping,
  `playlist.rs:5607`) and `Alt+G/U` ⇒ `Cmd::UngroupClips` (`playlist.rs:5608`);
  `app.rs:7905–7912` uses Shift+Alt+G for the global Enable-groups switch. No conflict.
- **Proposed command**: `Cmd::TrackGroupSel` (enum near `playlist.rs:316`), handled beside
  `Cmd::TrackGroup`:
  ```rust
  Cmd::TrackGroupSel => {
      snapshot(s);
      let rows: Vec<u16> = (0..TRACKS).filter(|&r| track_selected(s, r)).collect();
      // group each maximal run: every lane except the run's first becomes grouped
      for w in rows.windows(2) {
          // within a contiguous run (w[1] == w[0]+1): set grouped on the lower lane
          if w[1] == w[0] + 1 { arr_mut(s).tracks[usize::from(w[1])].grouped = true; }
      }
  }
  ```
  Also add `Cmd::TrackUngroupSel` for Alt+G parity (clear `grouped` on all selected, then fix up:
  FL keeps the leader the track above the former group). Mirror the wording used in the existing
  clip group messages (`playlist.rs:4876–4893`).
- Note: `duplicate_tracks` deliberately resets `grouped: false` on copies (`playlist.rs:2223`) —
  multi-group operations must preserve that invariant.

### Files to touch (part 2)

| File | Change |
|---|---|
| `crates/os-app/src/playlist.rs` | new cmds `DeleteTracks` (+ optional `TrackGroupSel`/`TrackUngroupSel`) in the `Cmd` enum (~237/316/337), their arms in `run()` (besides 5177–5181, 5254–5258), `shift_tracks` range support (2142), one guarded `Delete` match arm in `key()` (5656) |
| `crates/os-app/src/app.rs` | none required — wheel & key routing already land in `playlist` (app.rs:7845, playlist::key 5549) |

### Suggested step order (part 2)

1. Add `Cmd::DeleteTracks` + `run()` arm (bottom-up multi `delete_track`), unit-test with two
   adjacent selected tracks asserting clips/lanes/lane-remap — pattern the existing
   `Cmd::DuplicateTrack` tests (`app.rs:18822` area, `playlist.rs` tests near 5899–5916).
2. Guard the `Delete` keybinding on `track_anchor.is_some()` so plain Delete still removes clips.
3. Extend `shift_tracks` for anchored ranges; verify Shift+wheel and Shift+Up/Down both move the
   group and keep the selection.
4. Add `Cmd::TrackGroupSel` (+ `TrackUngroupSel`) and bind Ctrl+G (and Alt+G) in `playlist::key`;
   wire "Group selected" into the track menu (`track_menu`, `playlist.rs:3118`).

## Misc observations

- `note_fx.rs` (piano-roll note effects) is unrelated to both tasks despite the filename.
- `browser_index.rs` is the file-browser disk index (`build`, `match_dirs`); irrelevant to device
  ordering.
- `track_links.rs` models mixer/channel ↔ track links (`TrackLink`, `assign_audio`,
  `link_instrument`), not visual grouping — do not conflate with `grouped`.
- `enable_groups` switch gates *clip* grouping only (`playlist.rs:1754 with_groups`), not track
  grouping; `Cmd::TrackGroupSel` need not check it.
