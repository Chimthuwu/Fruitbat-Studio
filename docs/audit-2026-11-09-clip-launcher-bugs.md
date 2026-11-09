# Audit — Clip Launcher bugs (2026-11-09)

Read-only audit of the three reported bugs. Exact file:line references, root causes where
confirmed, and proposed fix plans (none implemented).

Confidence labels: **[CONFIRMED]** = mechanism traced end-to-end in code; **[STRONG]** =
mechanism narrowed to one code path with concrete evidence; **[HYPOTHESIS]** = ranked suspect,
needs a debug run to confirm.

---

## Bug 3 — Duplicated tracks (Ctrl+D) never appear in the clip launcher **[CONFIRMED]**

### Reproduction path

- Ctrl+D → `Cmd::DuplicateTrack` (`crates/os-app/src/playlist.rs:253`, menu item :3122,
  dispatched :4912-4914) → `playlist::duplicate_tracks` (`crates/os-app/src/playlist.rs:2207`).
- Ctrl+T / Ctrl+Shift+T → `Cmd::TrackMode(.., Mode::LinearMidi/LinearAudio)` →
  `track_links::insert_linear` (`crates/os-app/src/track_links.rs:263`) → control plane
  `Command::TrackCreate` (`crates/os-app/src/control/command.rs:1256`, `:1351`).

### The launcher's source of tracks

`launcher::columns` (`crates/os-app/src/launcher.rs:472-498`) enumerates
**`s.project.tracks`** — the *Track-model* records — and filters:

```rust
.filter(|t| t.kind != os_model::TrackKind::Bus && t.playlist_lane > 0)   // launcher.rs:479
```

i.e. a launcher column exists **only for an `os_model::Track` whose `playlist_lane > 0`**
(`Track` struct: `crates/os-model/src/lib.rs:187-206`). Arrangement lanes
(`PlaylistTrack` in `arrangement.tracks`) alone produce no column.

### Root cause

The two creation paths diverge exactly here:

- **Linear inserts (work):** `Command::TrackCreate` pushes a real `os_model::Track` into
  `project.tracks` with `playlist_lane = lane + 1` (and strip/channel wiring), so
  `columns()` sees it.
- **Duplicate (broken):** `duplicate_tracks` clones only the **arrangement** lane. It calls
  `playlist::insert_track` (`crates/os-app/src/playlist.rs:2185-2201`) which copies
  `PlaylistTrack`s and then calls the model remap
  `s.project.track_lane_inserted(t)` (`playlist.rs:2199`). That model method
  (`crates/os-model/src/lib.rs:2254-2262`) **only renumbers the `playlist_lane` of the
  *existing* `Track`s** above the insertion point (lib.rs:2256-2258). It **never creates a
  `Track` for the new lane**. The duplicated lane therefore has no `Track` record →
  `columns()` has no column for it → it is invisible in the launcher, although the playlist
  header and clips are fine.

This also explains why duplicating a track that *has* a link in Classic mode still doesn't
show: the lane exists in the arrangement, but the Track-model row does not.

### Proposed fix (smoothest)

Make duplication a first-class control-plane command so the Track model is updated in the
same undo step (mirrors how `TrackDelete`/`TrackCreate` behave):

1. Add `Command::TrackDuplicate { lane }` to `crates/os-app/src/control/command.rs`
   (near `TrackCreate`, ~:232).
2. In its handler: after inserting the arrangement lane (reuse
   `playlist::insert_track`), push a matching `os_model::Track`:
   `Track { kind: src.kind, playlist_lane: at + 1, mixer_strip: mixer::add_track(s),
   channel: None }` — the duplicate intentionally takes no link (per the doc comment at
   `playlist.rs:2202-2206`), so `channel: None` and a *fresh* strip is correct; copy name via
   the arrangement lane (it already carries it).
3. Re-point `Cmd::DuplicateTrack` (`playlist.rs:4912`) and the Ctrl+D key path at the new
   command (one undo step for free, consistent with `TrackCreate`).
4. Test: duplicate a linear-mode track, assert
   `launcher::columns(s).len()` grew and the new column's `lane` is `row + 1`.

Alternative (smaller but hackier): relax `launcher::columns` to derive columns from the
arrangement lanes and *join* to `project.tracks` when present. Not recommended — it would
invent ghost columns for unlinked Classic lanes and duplicate the strip ordering logic.

---

## Bug 2 — Browser context menus render behind the browser panel (in Clip Launcher) **[STRONG]**

### Layering map (verified)

All menus/popups paint into the **same main canvas**, in this order inside
`views::paint` (`crates/os-app/src/views/mod.rs:171-306`):

| Order | What | Where |
|---|---|---|
| 1 | Browser panel (`Seg::Browser`) | `views/mod.rs:203` |
| 2 | Docked windows incl. the launcher (`Seg::Win(..)`) | `views/mod.rs:220-247` |
| 3 | Toolbar band (`Seg::Toolbar`) | `views/mod.rs:251-255` |
| 4 | Dropdown menus (`s.menu_open`) — `menus::paint_dropdown` | `views/mod.rs:268-271` |
| 5 | Playlist popups (`s.playlist.popup`) — `views::playlist::paint_overlays` | `views/mod.rs:276-278`, painter at `views/playlist.rs:1282` |
| 6 | `rack::paint_fill`, `device_rack::paint_menu`, `patterns::paint`, settings | `views/mod.rs:279-289` |

Both browser menus open into the **shared playlist popup state**, not a browser-specific one:

- Row right-click → `s.playlist.popup = Some(Popup { items: row_menu(s, &row), .. })`
  (`crates/os-app/src/browser.rs:637`; items builder :980).
- Browser ⚙ options → same state, `options_menu(s)` (`browser.rs:44`, builder :1081).

So in a *full* frame the popup is painted after the browser (step 5) and is on top. Paint
z-order is **not** the defect.

### Root cause (where the bug must live)

The failure is in the **partial-repaint / damage interaction**, which differs between views:

1. `playlist::paint_overlays` runs *unclipped by any part* and is re-executed on every paint,
   but `Ui::part` (`crates/os-app/src/app.rs:691-737`) "keeps" (skips) any part whose
   `area.union` misses the damage clip, reusing the previous frame's hit list. The popup's
   pixels live *outside* every part's registered area, so when a repaint of the
   browser/window parts happens with damage that intersects the popup rect but the overlay
   paint is reached through a code path that skips it, the popup is simply not re-drawn while
   the (opaque) browser part is — the browser fills over the stale popup pixels, i.e. the
   menu visually goes *behind* the panel.
2. The overlay paint is gated and duplicated across **four** call sites
   (`views/mod.rs:276-278`, `:289-291` for the settings window, `paint_detached` :326-332,
   and the `over_dialog` tail :304-306). The gates are
   `!s.settings.open && !over_dialog && here`, where
   `here = !s.overlay_on.is_some_and(|w| s.is_detached(w) && s.shown(w))`
   (`views/mod.rs:275`). A stale `overlay_on` (a detached window that was closed/hidden
   without clearing `overlay_on`) makes `here == false` and the popup is never painted.
3. The Clip-Launcher relevance: the launcher replaced the playlist as the `Win::Playlist`
   painter (`views/mod.rs:222`) and uses its **own** menu state (`s.launcher.menu`, painted
   inside the window part by `views/launcher.rs::menu_paint`), while browser menus still use
   `s.playlist.popup`. Launcher actions blanket-call `app.invalidate()`
   (`crates/os-app/src/launcher.rs:754-1488`), so frames around launcher clicks are
   full repaints in which the popup draws *before* ... nothing covers it — but the *first*
   partial repaint that follows (any hover/damage under the browser) hits case 1 above.

### Proposed fix (smoothest)

1. Make popup painting unconditional and single-site: hoist `playlist::paint_overlays`'
   popup half into one call at the end of `views::paint` (drop the four duplicated call
   sites; keep only the `overlay_on`-detached delegation in `paint_detached`).
2. Audit `overlay_on` lifecycle: clear it whenever the overlay's owner window is hidden or
   detached state changes (`App::hide`/detach toggles, app.rs:1329-1333) so `here` cannot go
   stale.
3. Cheap regression test: paint frame with `s.playlist.popup = Some(..)` and
   `launcher_open = true`, force a partial repaint whose damage covers the popup rect, and
   assert the popup's pixels (or at least its hit) survive — mirrors
   `partial_repaint_matches_full` (`app.rs:12294`).

---

## Bug 1 — Top bar (transport/master/pitch) disappears in the Clip Launcher **[STRONG]**

### What the code establishes

- Entering the launcher is a **full repaint**: Tab → `Action::ToggleLauncher` →
  `s.launcher_open = !s.launcher_open; self.invalidate()` (`crates/os-app/src/app.rs:8829-8831`,
  keybind :8035). `invalidate` = `Damage::Full` (app.rs:3179-3181).
- The toolbar paints **after** the windows in `views::paint`
  (`views/mod.rs:251-255`, area `Rect::new(0, 0, w, band_h(s))`), so on a full frame nothing
  painted before it can cover the band; the launcher itself is confined to the Playlist
  window rect (`win_rect(maxed) = work_rect`, app.rs:1304-1316, which starts at
  `work_y = band_h`, app.rs:1271-1277; `views::launcher::paint` draws only inside
  `framed`'s client, `views/launcher.rs:157-217`).
- Every launcher interaction issues `app.invalidate()` (dozens of sites,
  `crates/os-app/src/launcher.rs:754-1488`) → again full repaints.
- Hover repair: hovering the band damages it (hover lighting → `damage_band`, app.rs:3212)
  → a *partial* paint in which only the `Seg::Toolbar` part repaints — and the toolbar comes
  back correctly.

Together this pins the defect: **the toolbar paints correctly in isolation (partial paint)
but produces a blank band in full frames while the launcher is open.** A paint-order cause is
excluded (toolbar is last); the surviving mechanisms are, ranked:

1. **`Ui::part` keep/area accounting for `Seg::Toolbar` vs the maximized Playlist part
   [HYPOTHESIS → verify first].** `Part.area` = `drawn ∪ declared`
   (`app.rs:691-737`, `begin_drawn`/`end_drawn` os-ui `lib.rs:445-458`). In launcher frames
   the window part's declared area equals `win_rect` but its *drawn* extent can grow upward
   (frame painting reports drawn bounds); when `keep.parts[i].area` then unions over the
   band, a partial repaint whose damage touches the band may take the *keep* branch for the
   toolbar (restoring stale/empty hit state) or the window part may be re-painted with a
   clip that was already consumed — the band then shows whatever the canvas had before the
   clear skip (`set_cover` promise, os-ui `lib.rs:527-533`). A debug print of
   `keep.parts` areas on a full launcher frame will confirm in minutes.
2. **The `set_cover(cover)` promise** in `views::paint` (`views/mod.rs:175-177`):
   `cover = s.win_rect(maximized Playlist)` = the whole workspace. Fills between
   `set_cover` and `set_cover(None)` skip that rect — that window is the `clear` only, so it
   is sound — but any future code that fills while the cover is set would silently lose the
   band. Verify no such fill exists in the launcher path (currently none found).
3. A concrete wart found on the way (not the cause, fix anyway): `views::launcher::paint`
   fills the client **twice** — two identical
   `ui.c.fill_rect(client, t.s(0x16191b))` lines back to back (`views/launcher.rs:160-161`),
   likely a leftover of a debugging attempt on this same symptom.
4. The launcher omits the `window_body(ui, r)` swallow hit the playlist installs
   (`views/playlist.rs:65`; absent from `views/launcher.rs::paint`). This is a *hit-test*
   asymmetry, not the visual bug, but it is the exact class of bug the user suspects and
   should be aligned.

### Proposed plan

1. Reproduce under the existing paint harness (`partial_repaint_matches_full`, app.rs:12294)
   with `launcher_open = true`: paint full, then paint the same state again partially over
   the toolbar band, and diff pixels. That test decides between hypothesis 1 and a canvas
   bug instantly.
2. If it is the keep logic: clamp `Part.area` for window parts to their declared `a`
   (drop the `drawn`-union growth for maximized windows), or make `Seg::Toolbar` opt out of
   keeps when a maximized Playlist part was repainted in the same frame.
3. Remove the duplicate client fill (`views/launcher.rs:160-161`) and add the
   `window_body(ui, r)` swallow to `views::launcher::paint` for hit-test parity with
   `views/playlist.rs:65`.

---

## Files consulted (audit scope)

Fully read: `crates/os-app/src/app.rs`, `crates/os-app/src/launcher.rs`,
`crates/os-app/src/track_links.rs`, `crates/os-app/src/views/{mod,launcher,playlist,rack,patterns}.rs`,
`crates/os-model/src/lib.rs`. Targeted: `crates/os-app/src/{playlist,browser}.rs`,
`crates/os-app/src/control/command.rs`, `crates/os-ui/src/lib.rs`,
`crates/os-app/src/views/menus.rs`. No code was modified.
