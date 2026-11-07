# Handoff — Device Rack tiles show no UI (needs pop-out)

Written 2026-11-08 by Stooge (Hermes). Session went in circles; this doc is everything
measured, so you do not repeat it.

Repo: `Z:\GITHUB\Lemon-Studio`, branch `main`.
HEAD = `36b9b3e1` (clean tree, **unpushed**). Release exe =
`Z:\lemon-target\release\lemonstudio.exe` (2026-11-08 03:49, 49,394,436 B).

---

## 1. The bug, in the user's words

> "Device Rack UI wont appear till Pop out is clicked .... \"Dock\" mode is obsolete and
> shows blank device so its shit."

> "yeah but the device rack stiull doesnt show paramets. for anything... the fuck?"

> "when i add the MIDI effect in , it appears with zero knobs"

A device's editor is pinned into its rack tile (geometry is correct), but the tile
renders as a flat dark fill `(8,7,6)`. Hitting pop-out shows the UI, so the *content*
exists — only the docked copy never paints.

---

## 2. What is already committed

| commit | what |
|---|---|
| `fc17c4ec` | shared `simple::Editor` for all 18 stub plugins |
| `37e9eca1` | vertical-drag dials replace horizontal sliders |
| `54a692cf` | **round 1**: `State::size()`/`area_y()` now return the bare editor rect when `pinned`; win32 `layout()` area inset 0 when pinned; `pinned()` getters ×3 platforms; per-tick `set_editor_size` guarded `if !w.pinned()` |
| `a38cc3e1` | test `a_pinned_frame_is_exactly_the_editor` |
| `101ff8f2` | todo.md |
| `36b9b3e1` | **round 2**: `layout()` repaints frame + children. **See §4 — this did NOT fix it.** |

Tests: `cargo test -p os-app` → **1082 passed / 0 failed** (before `36b9b3e1`;
re-run it after your own changes). Workspace total 2248.

---

## 3. Ruled OUT — do not re-investigate

Every item below was disproven with a direct measurement on the running exe:

- **Geometry.** Frames pinned at the tile origin, 280×(tile body), editors at local
  (0,0). Example measured hierarchy (all `WS_VISIBLE`, correct parent chain):
  `OpenStudioPluginFrame(-1208,861,-928,1080)`
  → `OpenStudioPluginArea(same rect)`
  → `LemonVelocityCurveView(-1208,861,-888,1007)`.
- **Window regions.** `GetWindowRgn` → ERROR (no region) on frame/area/editor.
  Clearing them changed nothing.
- **Occlusion by other apps.** `WindowFromPoint` at the tile centre returned the editor
  itself in the early runs.
- **Clipping.** `clip_docked` (`app.rs` ~3115) sets `rgn = null` for in-tile windows.
- **Visibility calls.** `layout()` does `ShowWindow(w.area, if !st.misc {SW_SHOW} else {SW_HIDE})`;
  `State.misc` defaults `false`. `raise()` does `SW_SHOWNORMAL`. `refit` is maximized-only.
- **Stale exe / stale shortcut / stale process.** Desktop and Start Menu `.lnk`s both
  parse to `Z:\lemon-target\release\lemonstudio.exe`; exe was rebuilt.
- **DPI.** `IsProcessDPIAware: False`, `GetDpiForSystem: 96`, `GetDpiForWindow: 96`
  → 100% scale, no coordinate virtualization.
- **Repaint-on-expose theory.** (see §4)

### Editor paint path (`crates/os-plugkit/src/simple.rs`)
- editor created `WS_CHILD|WS_VISIBLE` at (0,0) of `area` (line ~524)
- paints **only on `WM_PAINT`** (line 581); `WM_ERASEBKGND => 1`
- **no `WM_SIZE` arm, no timer** → it depends entirely on someone sending it `WM_PAINT`
- `paint()` fills `c_body()` as the backdrop

### The `WS_CLIPCHILDREN` chain
`main`, `OpenStudioPluginFrame` and `OpenStudioPluginArea` **all** carry `WS_CLIPCHILDREN`.
So a parent's repaint never reaches its children.

---

## 4. The round-2 fix — plausible, but proven insufficient

`crates/os-app/src/plugin_window/win32.rs`, in `layout()` (~line 626):

```rust
// WAS: InvalidateRect(hwnd, null, 0)   // only the frame -> children excluded by WS_CLIPCHILDREN
RedrawWindow(hwnd, std::ptr::null_mut(), std::ptr::null_mut(),
             RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_UPDATENOW);
```

(`lprgnupdate` is `*mut c_void`, hence `null_mut()` — `null()` fails to compile.)

**Evidence for the theory:** on a blank tile, forcing
`RedrawWindow(hwnd, null, null_mut(), RDW_INVALIDATE|RDW_ALLCHILDREN|RDW_UPDATENOW)`
flipped the screen region from **1 colour `(8,7,6)` → 363 colours with structured rows**,
stable over 3 s. That isolated the cause as "content exists, never painted".

**Evidence that it still fails:** after `36b9b3e1`, with Lemon held **foreground from
launch** (never occluded), all 5 tiles captured as:

```
'Lemon Velocity Curve'  280x219 at (-1014,861)  unique=1  dominant=(8,7,6) 100%  rows=[1,1,1,1,1]
'Lemon Note Delay'      280x219 at (-1316,861)  unique=1  dominant=(8,7,6) 100%
'Lemon Humanize'        280x219 at (-1920,861)  unique=1  dominant=(8,7,6) 100%
'Lemon Sampler'         280x219 at ( -712,861)  unique=1  dominant=(8,7,6) 100%
```

…while a **coordinate sanity check passed** in the same run:

```
top-left of main (titlebar)  400x120: unique=839  rows=[3,15,39,32,46]   <-- real UI
a rack tile                  280x219: unique=1    rows=[1,1,1,1,1]       <-- blank
rack strip around tiles     1920x240: unique=261
```

So the captures land in the right place and the tiles really are empty.

### The unresolved anomaly (start here)

In those same runs, `WindowFromPoint(tile_centre)` returns the **main window**
(`class='Window Class'`, `title='Lemon Studio'`) — *not* the visible child frame/area/editor
that `GetWindowRect` says occupy that exact point. In the very first (pre-fix) run it
returned the full chain `editor → area → frame → main`.

Something about the child windows' actual on-screen presence changed. Candidate leads:
- whether the children are really being hit-tested (`IsWindowVisible` said `True`,
  but verify with `GetWindowRgnBox`/`DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)`
  and compare screen vs. client mapping)
- whether `pin_window`'s rect is in *physical* screen px vs. canvas coords — the call is
  `rect = [(r.x*k).floor()+ox, ...]` with `k = app.plugins.scale` and `ox,oy = app.canvas_off`
  (`crates/os-app/src/device_rack.rs` ~690). **A `scale != 1` or a stale `canvas_off`
  would put the frame where `GetWindowRect` says but where it isn't actually drawn.**
- whether the frame is being moved *after* paint and the movement is not repainting
  (re-run the forced-`RedrawWindow` experiment on `36b9b3e1` — if it still flips
  1 → 363, the invalidation fix is being undone somewhere)

---

## 5. Design decisions the user already made (do not re-ask)

1. **Native MIDI FX devices → show their real native UI inside the rack tile.**
   Their UIs are small (Quantize 320×146, Velocity-Curve 320×146, Note-Length 320×224,
   Transpose 390×224). *This is the current priority.*
2. Then the same for **all Lemon native Audio FX** (Lemon Reverb, Lemon Parametric EQ, …).
3. **Instruments and third-party plugins → do NOT squeeze.** Show a parameter list in the
   tile, plus a **wrench button in the top-right of the tile banner** (where the name is
   shown) that opens the full plugin UI.
4. Variable tile sizes for native devices are explicitly acceptable:
   > "I dont mind if you sometimes make the containers for the Devices in Device rack
   > different sizes for these and other native devices."

### Why (3) is right — the Lemon Sampler
The user's screenshot was **Lemon Studio running Lemon Sampler**, a multi-tab native UI:
Volume tab (ADSR Delay/Att/Hold/Dec/Sus/Rel each with Tension, plus LFO Delay/Att/Amt/Speed),
tabs for MOD A/B, Pitch, Filter; root note + piano keyboard; and a bottom param strip
(Keep on disk / Resample / Load regions / Load slice mode / Click removal / Sample start /
Loop start / Loop end). Natural size **560×390** into a **280×219** tile.

User's question: *"what are we to do when the device has MANY knobs … i cant even see the
ADSR for the volume (or the MOD A or B or Pitch) or the Filter Knobs or LFO controls…"*

→ Answer: don't cram. Tabbed UIs are fundamentally unreadable at 50% scale. Follow rule (3):
param list + wrench. Do not attempt to fit the Sampler in a tile.

---

## 6. Other known-open work (not part of the paint bug)

- **MIDI FX do not process MIDI.** User: *"the MIDI fx dont actually send midi signal to
  instruments yet"* — UI/data only; `process()` never called. Plan = note-path refactor
  (todo `M9`): `MidiFx` trait in `os-engine`, `Plugins::midi_fx()`,
  `notes_under_way` → `send_note` left→right, 18 impls reusing `dsp::process`.
- Audio FX chain already works (`crates/os-engine/src/engine/mix.rs:31`).
- Audio FX layer/selector containers not separately verified.
- `body_mode` = `Embedded` iff `s.device_rack_editor.contains(&key)`
  (`views/device_rack.rs` ~232); `device_rack.rs` ~316 fills the blank body.

---

## 7. Environment — these will bite you

**Build/test must run through git-bash `terminal()`.** Python `subprocess` spawns die with
`0xc0000135` (DLL not found). Full env (no `.cargo/config.toml` exists; default rustup
toolchain is **MSVC** and rejects `linker-flavor=ld`):

```bash
export RUSTUP_HOME=/z/rustup
export CARGO_TARGET_DIR='Z:\lemon-target'
export PATH="/z/mingw64/mingw64/bin:$PATH"
export LIBCLANG_PATH='Z:\LLVM\bin'
export BINDGEN_EXTRA_CLANG_ARGS='--target=x86_64-pc-windows-gnu -resource-dir=Z:/LLVM/lib/clang/20 -mmmx -IZ:/mingw64/mingw64/x86_64-w64-mingw32/include -IZ:/mingw64/mingw64/include'
export RUSTFLAGS="-C link-self-contained=yes -C linker-flavor=ld -C linker=Z:/mingw64/mingw64/x86_64-w64-mingw32/bin/ld.exe"
cargo +stable-x86_64-pc-windows-gnu test -p os-app
cargo +stable-x86_64-pc-windows-gnu build --release -p os-app
```

**Launching / probing the exe:**
- Kill with `taskkill /IM lemonstudio.exe /F`, **wait 5 s**, then launch. Launching a
  second instance over a live one silently kills it — this caused two false "the app
  crashed" readings.
- The window **drifts between monitors**. It was at `(63,1,1910,1081)` (primary) and later
  `(-1928,-8,8,1088)` (left monitor). **Rack tiles then have negative x** — a naive
  `if x0 < 0: skip` capture guard wrongly discards every tile (this cost a whole round).
- **Bring it to foreground BEFORE frames are created**, or exposing it later repaints and
  masks the bug. Foreground lock workaround that works:
  ```python
  u.keybd_event(0x12,0,0,0); u.keybd_event(0x12,0,2,0)  # ALT down/up
  u.SetForegroundWindow(hwnd)
  ```
  Plain `SetForegroundWindow` is usually denied (`GetForegroundWindow()` stays `Hermes`).
- Frames appear ~3–17 s after launch. Sometimes none appear at all (`sync_embed` found no
  tiles — the rack was empty/collapsed); a force-killed restart may land in that state.

**Probing tools:** ctypes GDI `BitBlt` from screen DC + colour histogram
(`unique`, dominant share, per-row unique counts) is the working substitute for vision.
`vision_analyze` was returning `404 - Model 'z-ai…'` (out of credits) — retest it, a real
screenshot would settle §4 immediately. `PrintWindow(flags=0/1)` *forces* a paint of
children, so it **cannot** be used to ask "is it painted on screen" — it always shows
content. `PrintWindow(flags=2)` fails (`ok=0`) on this app.

**Other:** guard hook blocks `rm -r/-f` — delete single files with Python `os.remove`.
`write_file`/`patch` refuse `Z:\HERMES\.hermes\config.yaml` (use `hermes config set … --force`).
**Never touch `Z:\BITWIG LIBRARY`.** `Z:\GITHUB\DAW` is out of scope ("DAW is something else").
No credentials/tokens/passwords/connection strings may appear in any file.

---

## 8. Suggested first moves

1. `git push` (HEAD is 1 commit ahead) — or revert `36b9b3e1` first if you decide it is wrong;
   decide which by re-running the forced-`RedrawWindow` A/B on a `36b9b3e1` build.
2. Re-check whether `36b9b3e1`'s invalidation is being *undone* (something must be
   re-blanking the tile — check who else calls `InvalidateRect`/`SetWindowPos` on the frame
   after `layout()`, especially the per-tick pin loop in `device_rack.rs` ~690 and
   `plugins.rs` ~1463/~1480).
3. Resolve the `WindowFromPoint` anomaly — it is the sharpest inconsistency in the data.
4. Only once tiles paint: implement §5.1 (MIDI FX real UI, auto-sized tiles).
