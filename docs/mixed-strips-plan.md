# Mixed strips — one track holding modules from both consoles

**Status: plan for review, 2026-11-12.** Asked for alongside the glitcher. Today a track picks one
strip (`MixerTrack::glitch_style`): the Analog Channel Strip's six sections, or the Glitch grid in
their place. This plan makes the strip a **chain of modules** — any mix of the six analog sections
and the grid, in signal order — so a track can crush before it drives, or gate after it tables.

Roughly: DSP split + model + a side chunk (P1), commands and menus (P2), the console (P3).

## The model: a chain, not a pick-list

`MixerTrack.chain: [u8; 8]` — module ids in signal order, `0` is an empty slot. Ids are fixed and
never renumbered (they are written to files):

| id | module | its word (`strip[]`) | console control |
|----|--------|----------------------|-----------------|
| 1 | CUT   | 0 (low cut)      | knob, 20..350 Hz, 0 = off |
| 2 | OTT   | 3 (depth)        | knob, 0 = off |
| 3 | DRIVE | 2 + 6 (character)| knob + character button |
| 4 | TAME  | 4                | knob, 0 = off |
| 5 | COMP  | 1 + 5 (profile)  | knob + profile button |
| 6 | TABLE | 7                | knob, 0 = off |
| 7 | GRID  | `glitch_amount`  | the 4×4 grid, painting, playhead |

- **Order is the point.** A chain is a list, not a set: `[3, 7]` (drive, then grid) and `[7, 3]`
  (grid, then drive) are different sounds and must render differently.
- **A slot is on when its own word is non-zero**, as a section already is. So adding a module from
  the menu sets a musical default word in the same undo step (CUT 80 Hz, OTT 40 %, DRIVE 40 %,
  TAME 40 %, COMP 50 %, TABLE 40 %, GRID amount 70 % — the grid's existing click precedent), and
  switching a chip off zeroes it. Nothing added ever does nothing silently.
- **Each module at most once** in v1. The array has room for eight, so duplicates (two grids, two
  drives) stay possible later without a new format; they need per-instance DSP state and a bigger
  file record, so they are explicitly out.
- **Defaults and presets.** A new track: `[1,2,3,4,5,6]` with every word 0 — idle, sample-exact
  pass-through, exactly as a bypassed strip is today. "Analog channel strip" preset → that chain;
  "Glitch strip" preset → `[7]`; anything else is mixed. Switching presets keeps every word, so a
  round trip through Analog and back restores the grid untouched.
- **`strip_bypassed` stays the chain's master bypass** (the STRIP switch). It is the A/B of the
  whole chain, not a module toggle, so muscle memory and the file format both hold.

## DSP: the sections become callable one at a time

`os_dsp::strip::Strip::process` currently runs all six sections in the Edition's order (low cut →
OTT → drive → TAME → comp → table) inside one frame loop, with one 10 ms bypass crossfade. Split
that loop into per-section steps, `Strip::run(&mut self, module: u8, buf, rate)`, keeping the
existing per-sample code and each section's own `block` plan, idle skip and glide. Then:

- `Strip::process` = the bypass crossfade plus `run` in Edition order for ids 1..6, so the analog
  preset is **bit-for-bit what it renders today** (a test, not a hope).
- `MixTrack` gains `chain: [u8; 8]`; `render.rs` walks it: `run(module)` for 1..6, `Glitch::process`
  for 7, skipping whatever is idle. Both objects stay constructed as they are now — the grid keeps
  its clock and ring whether or not it is in the chain, so its playhead readback
  (`Engine::glitch_steps`) and step position do not jump when it is switched in.
- **No latency changes**: every analog section is zero-latency and the grid is too (its reverse
  reads a step of history, not a delay). PDC, `latency_out`/`latency_in` and the slot path are
  untouched.

CPU: one more branch per block; a module whose word is 0 costs a `word == 0` test, as today.

## File format: a side chunk, not a version bump

The strip chunk `LSCS` is at version 8 (index, 8 words, bypass, 16 steps, amount, style). Changing
its record layout would make **older builds drop every track's strip state**, analog included, because
an unknown version is ignored whole. Instead:

- `LSCS` stays at version 8 and keeps being written exactly as now (style = `[7]` for a glitch track,
  `false` for analog and mixed). An old build therefore reads a mixed track as analog with a grid,
  and its analog console shows the right words.
- New chunk **`LSMX`**, version 1: count `u16`, then per track `index u16` + 8 chain bytes. Written
  only when a track's chain is not the analog default `[1..6]`, and dropped from the file when no
  track needs it (the same `strip_payload`/`store_strip` shape). Reading it is a separate pass over
  `MixerTrack`; a file without it derives the chain from `glitch_style` (glitch → `[7]`, else
  `[1,2,3,4,5,6]`).
- `strip_style` stays on the model so `.lemon`, undo snapshots and the style bar keep working.

Round trips to test: v9-less (old) file → derived chain; mixed chain → `LSMX` → back; an old build's
`LSCS` v4/v7/v8 still read; a project with no chains writes no `LSMX`.

## Control plane and undo

- `MCmd::Chain(n, [u8; 8])` — one command, one undo step, validated on the way in (ids 0..=7, unique,
  zeros trailing). Add, remove, reorder and the presets all compute an array and send it, so undo is
  uniform and the control plane stays small.
- Menus (`mixer::track_items`): a **Modules** submenu — one radio per module, in chain order first;
  **Move earlier / Move later** for the slot the menu was opened on; the two presets at the top.
- Knobs keep their ids: `KnobId::Strip(n, word)` addresses a word wherever its module sits, and
  `KnobId::Glitch(n)` the grid's amount. Scripted as `strip:<track>:chain` (eight ids, comma
  separated) beside the existing `strip:<track>:<word>` and `glitch:<track>`.

## Console (the visual half)

The strip is 44 px wide and the console body is four rows of sections.

- **The style bar becomes a chain bar.** Above 50 px: one small chip per chain slot, in signal
  order, two letters in the module's colour (`glitch_ink` gives the grid's, a per-module ink for the
  analog six), filled when the module is on, outlined when it is in the chain but off. Click
  switches it in or out, drag reorders (`Drag::ChainMove`), right-click opens Modules. Below 50 px the
  bar is `MOD` and a count ("5") instead, and the menu is the only way — seven 6 px chips are not
  clickable and should not pretend to be.
- **One tile per slot, top to bottom in signal order.** An analog tile is one row: label, its knob,
  its character/profile button where it has one, and its readout — the existing `console_label`,
  `console_knob` and row plates. A GRID tile is the 4×4 plus its amount row, reusing `glitch_area`,
  `glitch_panel`'s cell drawing (extracted as `grid_cells`) and `glitch_at`, so painting and the
  step-ring playhead work unchanged.
- **Pages.** Four rows fit; a grid tile takes four. A chain taller than a page pages from the bar
  (dot pair, wheel or click), and a grid tile always starts a page so it is never cut in half.
- **Damage.** The chain bar and tiles register `Live::Chain` (the analog panel's damage) with
  `Live::Glitch` over the grid area, so a playhead ring still repaints alone.

## Phases

**P1 — DSP, model, file (no visible change).** Split `Strip` into per-section runs; add
`MixerTrack.chain` + `os_dsp::strip::CHAIN_MODULES`/`CHAIN_SHORT`; engine chain walk; `LSMX` read and
write; tests below. Old projects load identical; the analog and glitch presets sound identical.

**P2 — commands, menus, scripting.** `MCmd::Chain`, Modules submenu, preset switching preserving
words, `strip:<track>:chain`, undo tests. Mixed strips are usable from the menu before the console
catches up; the style bar still shows the two presets.

**P3 — the console.** Chain bar, tiles, paging, reorder drag, the narrow `MOD n` form, playhead and
painting tests at several layouts.

**P4 — polish (optional).** Copy a chain between tracks, "move to front/back", starter mixed presets
(CRUSH→DRIVE, GATE→TABLE), and — only if the user asks — duplicate slots with per-instance state.

## Tests

- DSP: analog chain in Edition order == `Strip::process` sample for sample; `[7]` == `Glitch::process`
  sample for sample; a chain of zero words is bit-identical to the input; `[3,7]` ≠ `[7,3]`; a drive
  into the grid stays finite at every amount (the glitcher's own `the_grid_stays_bounded` plus one).
- Model/FLP: `LSMX` round trip, absent when default, derived from `glitch_style` when missing, `LSCS`
  v4/v7/v8 still read, mixed file keeps its words.
- Engine: an offline render of a mixed chain equals the same modules applied in sequence by hand
  (`os-render` corpus), and `glitch_steps` still reports a grid in slot 3 of 5.
- App: menu add sets a default word in one undo step; remove keeps the word for re-adding; presets
  round trip; a knob's hit lands on the right word for every chain order; chain bar chips at 64 px
  and the `MOD n` form at 44 px; grid painting still hits the right step when the tile is paged.

## Open questions for the user

1. At 44 px, is the chip bar worth it, or is `MOD 5` plus the menu the right narrow form? (Plan:
   `MOD n` below 50 px, chips above.)
2. Should a mixed chain ever hold the grid twice? (Plan: no in v1; the format allows it later.)
3. Should mixed be the default for new tracks, or stay opt-in behind the style bar? (Plan: opt-in;
   Analog stays the default until mixed has shipped and been lived with.)
