# Rough plan — Bitwig/Ableton-style track workflow

**Status: rough draft for review.** Goal: make "insert a track" behave like Bitwig/Ableton —
an Audio or MIDI track that is *simultaneously* a playlist lane, a mixer strip, and (for MIDI)
an instrument channel — static and always visible — while keeping the FL-compatible
pattern/playlist engine underneath and `.flp` round-trip intact.

## What already exists (from docs/status.md + feature-imports audit)

- Playlist has pattern, audio and automation clips; several arrangements; audio tracks and
  instrument tracks exist (FL 24.x semantics): Track mode ▸ links a playlist track to a mixer
  track or channel ("A"/"I" headers), audio clips dropped on an audio track route through its
  mixer track, recordings land on it, an instrument track creates/holds its channel.
- The mixer has sends, slots, routing, disk recording, input monitoring.
- Recording into mixer tracks and into playlist audio tracks already works.
- So the missing piece is not capability — it's that this is an *opt-in link between three
  unrelated-looking panels*, not the primary workflow.

## Design: the "Track" workflow layer

### 1. A first-class Track object (thin, not a new engine)
Add a `Track` layer in os-model that bundles what FL stores separately, using the existing
linkage fields (FL already stores these links the same way — that's the compat key):

    Track { kind: Audio|Midi|Bus, playlist_lane, mixer_strip, channel: Option<ChannelId> }

- No new audio path. Routing, voices, PDC, automation all stay as-is; Track is a bookkeeping
  + presentation layer over existing channel/mixer/playlist records.
- Insert Audio Track → creates an audio-clip-capable channel (or a bare "audio track" like
  FL's) + a new mixer insert + a playlist track, linked, named, coloured together.
- Insert MIDI Track → creates an instrument channel (user picks plugin, defaults Lemon Trio /
  Sampler) + mixer insert + playlist track linked as instrument track.
- Bus track = mixer strip with no lane content (foldable send group), v2.

### 2. Make it the default project mode, keep FL mode
- New project template "Linear/DAW mode": every track is created via the Track layer, the
  channel rack is collapsed/hidden by default, patterns still exist underneath for MIDI
  editing but clips on lanes can hold notes directly (see 3).
- "Classic (FL) mode": today's behaviour untouched. Mode is a project property saved in
  `.lemon` (`.flp` files always open in Classic; a "Convert to track project…" migration
  command builds Track objects from existing links).
- Compatibility rule: a Track that links channel+mixer+lane round-trips through `.flp` as
  FL's own audio/instrument track records (Lemon already reads/writes those). No new FLP
  chunks required for v1. Nice — the feature can be shipped without breaking FL interop.

### 3. Notes on lanes (the Ableton part that hurts)
Bitwig/Ableton MIDI tracks hold arrangement clips of *notes*, not pattern references. In FL
the playlist holds pattern clips. Options, in order of comfort:
- v1: keep pattern clips. Insert MIDI Track still "feels" linear because the pattern is
  per-track, auto-created, auto-named after the track, and its piano roll opens on double-
  click of the lane region — the pattern becomes an implementation detail (like Bitwig's
  inline clips being an implementation detail).
- v2: allow "inline" note clips on a MIDI track (a pattern auto-owned by the clip). The
  existing audio-clip ownership machinery (clip source picker, track locks) is the model.

### 4. Static tracks
- Tracks don't disappear when empty; they're scrollable lanes like the mixer (which already
  docks/sizes). Empty MIDI tracks show an idle instrument strip; deleting a track is
  explicit (with the existing delete-confirmation machinery).
- Group tracks (fold) reuse the mixer's Group/Separator mechanics for the strip side.

### 5. UI changes
- New default arranger view = today's Playlist with: persistent track headers showing
  kind, arm, monitor, mixer-strip mini-fader/meter (data exists in the mixer), track colour.
- Insert ▸ Audio Track / MIDI Track / Bus replaces the current multi-step ADD flow as the
  prominent action; FL's ADD menu stays for classic mode.
- Mixer reorders to track order when a project is in Linear mode (fork precedent shows
  track-order mixer is feasible; we do our own implementation).
- Recording: arm on the track header (already exists for audio tracks; extend to MIDI
  track record-into-clip).

### 6b. Automation lanes (Bitwig 5.2-style) — replaces free-floating automation clips
Problem today: automation clips are channels that render as movable clips anywhere in the
playlist, so the automation for a thing can sit visually on any track. Bitwig instead pins
automation to the thing it drives, in expandable lanes under the track (or clip).

Design (Linear mode; Classic mode keeps free clips untouched):

- **Per-track automation lanes.** Each Track gets a stack of automation lanes beneath it
  (collapse/expand like Bitwig's ▸). A lane is bound to one automation target: a channel
  knob (volume/pan/pitch), a mixer parameter on the track's strip (fader, pan, EQ, send,
  slot mix/switch), or a plugin parameter on the track's instrument or slots.
- **Storage stays FL-compatible.** Automation is still automation-clip channels + clips
  underneath — we only change *where they render and live*. A lane is a view+constraint over
  an existing automation channel: creating automation for a target in Linear mode
  auto-creates the clip channel, binds it to the owning track's lane, and locks its clips
  to that track. This keeps undo, the event editor, LFO/range machinery, control plane and
  `.flp` round-trip exactly as they are (FL stores automation clips as channels in the
  global rack — that is preserved; only Lemon-side placement/display changes).
- **Track-order source of truth.** A lane's vertical position follows its track. Lanes are
  named after their target ("Filter Cutoff", "Vol", "Send 1 → Reverb") and inherit track
  colour. Multiple lanes per track stack; collapsed by default except when the user adds one.
- **Clip behaviour inside a lane:** the automation clip's placement is constrained to its
  track's x-axis (time) — no vertical dragging between tracks. Delete/unlink returns it to
  a free clip (visible in Classic mode / the old view). Draw/edit interaction stays the
  existing automation editor (event editor / in-lane editing later).
- **Per-clip automation (Bitwig's inside-a-clip curves)** is a v3 idea: allow an automation
  curve owned by one clip (moves with it, no channel). Defer — it's the only piece that
  genuinely needs new data; v1/v2 need none.
- **Mixer automation on unlinked strips** (Classic-style mixer targets that have no Track):
  these keep their free clips, rendered in an "Unassigned" automation lane group at the
  bottom of the arranger, so nothing becomes invisible in the migration.
- **Migration/interop (LOW priority — deferred per chimi420, 2026-11-04):** auto-binding
  of automation clips on `.flp`/Classic project open is a later phase, not part of v1.
  Until then Linear-mode features target new/converted projects; old projects just keep
  their free clips (Unassigned group shows them when opened in Linear mode, no binding).
  Saving stays legal either way since storage is unchanged.

Impact on the open question in §6: automation-clip placement is no longer a v2 question —
lanes *are* the answer, and they're a view-layer change with no engine cost.

### 7. Device rack at the bottom (Bitwig-style) — kill floating windows by default
Problem today: plugin/setting windows are free-floating by default (FL heritage). Goal:
every device (instrument on a MIDI track, effects on a track's mixer strip) lives embedded
in a device rack docked at the bottom of the main window; floating becomes opt-in per
plugin, not the default.

Design:

- **Device rack panel** docked at the bottom (resizable, collapsible like the mixer).
  Contents = the devices of the *selected track*, in signal-flow order: its MIDI FX
  (MIDI tracks, §8) → instrument (MIDI track) → the mixer strip's effect slots.
- **Embedding first, pop-out optional.** Every device shows embedded by default:
  - Built-in Lemon plugins: their os-ui editors render directly in the rack (os-hostlink
    already wires plugin editors into app chrome — extend it with an embedded surface).
  - Hosted CLAP/VST3/VST2: embed the plugin's own editor view (CLAP GUI embed is
    straightforward; VST3 embed on Windows/macOS/Linux is the work item — macOS out-of-
    process plugins can't embed, fall back to a thumbnail + pop-out there).
  - Any device can be popped out via its header menu ("Pop out" / "Dock") — the existing
    detached-window machinery is reused for the floating state. State (docked vs floating)
    saved per plugin in `.lemon` projects, default docked.
- **Track selection drives the rack.** Selecting a track swaps the rack's device chain with
  a light crossfade; a "pin" toggle keeps a device chain visible while editing elsewhere
  (Bitwig's inspect-style behaviour).
- **Channel settings windows** (the FL channel settings pages) fold into the rack as a
  device detail view instead of a floating window; the piano roll and other *editors* stay
  as-is (they're workspaces, not devices — revisit later).
- **Compat:** plugin window state is stored per-plugin already (Scaling, docked/detached
  menus exist); we add the rack's default in Linear mode and keep FL-style floating in
  Classic mode. Classic mode = zero behaviour change.
- **Risks for reviewers:** VST3 embed effort per platform (the big one); very large plugin
  UIs in a short rack (mitigate: expandable rack height + pop-out); window chrome
  /keyboard-focus rules for embedded editors (existing "Set keyboard focus of plugins"
  settings apply); crash containment — an embedded plugin crashing must degrade to a
  placeholder slot, not take the app (hostlink scanner/sandbox precedent exists).

Milestone: after the track model (needs track/strip selection to exist), before or in
parallel with 3b. Order: (a) rack shell + built-in plugins embedded, (b) CLAP embed,
(c) VST3 embed, (d) pop-out/dock per device + state persistence, (e) channel settings
fold-in.

Milestone 3c: MIDI FX (§8) — engine note-path stage + CLAP hosting + built-in Arp/Chord/
Echo; slot list lives on the track, renders in the rack chain between MIDI FX and audio FX.

### 8. MIDI FX — new capability (does not exist in FL or in Lemon today)
Goal: Bitwig-style MIDI FX slots on MIDI tracks. Signal flow in the device rack and in
audio is: **MIDI FX → instrument → audio FX** — MIDI effects transform/produce note events
before the instrument consumes them.

Design:

- **Slot type.** MIDI tracks gain a MIDI-FX slot list, ordered before the instrument.
  Audio tracks and buses have none. In the rack the chain renders left-to-right
  (or top-down): MIDI FX, then instrument, then the strip's audio FX.
- **Host support.** CLAP is the first-class format: CLAP plugins receive and emit note
  events, so a MIDI FX is just a plugin whose input and output are MIDI (CLAP event
  pipeline already flows through os-engine's plugin hosting — the new work is inserting a
  processing stage into the note path). VST3 MIDI FX exist (note/JSON-based) but are less
  standard: support opportunistically, not day one. No VST2 MIDI FX.
- **Engine change (the real work, unlike the rest of the track plan).** The note path
  pattern/channel → instrument gains an intermediate event-processing stage:
  `notes → MIDI FX chain → instrument`. Per-tick note batches pass through each MIDI FX
  in order; timing must stay sample/tick-exact and respect the existing tick-start voice
  change discipline in engine.rs. Plugin delay compensation treats MIDI FX as zero-audio-
  delay (they can add latency in note terms only).
- **Built-in MIDI FX v1** (Lemon-branded, nih-plug CLAP so they double as plugins
  elsewhere): Arpeggiator, Chord/Scale, Note Echo (the existing echo-delay/arp channel
  features migrate conceptually here — keep channel-level features untouched for FL
  compat and offer MIDI FX as the new path).
- **Routing/compat rules:**
  - FL compatibility: an `.flp` has no MIDI-FX slots; the data lives in `.lemon`-side
    records only. Saving to an `.flp` name omits them (same policy as the other
    `.lemon`-only data). Classic mode never shows MIDI-FX slots.
  - Recording: notes recorded are the *input* notes; a "commit/flatten MIDI FX output"
    command (bounce the transformed notes into the pattern) for freezing ideas — v2.
  - Control plane: expose the chain read/propose/edit like other plugin slots.
  - MPE interplay: MIDI FX pass per-note expression through untouched by default
    (sequence after or with MPE work; same note-struct territory).
- **Risks:** note-path engine refactor in the RT-sensitive engine.rs (audit flagged its
  size — same split prerequisite as MPE); hosts + plugins disagreeing on MIDI dialect
  (14-bit CC, note release semantics) — v1 passes raw events through with a strict
  pass-through contract; UI: MIDI FX editors are small (arp grids, chord pads), good
  embed fit for the rack.

### 9. The Channel Rack / step sequencer in Linear mode — integrate, don't delete
Tension: once tracks are first-class and the device rack lives at the bottom, the floating
channel rack loses its home — but the step sequencer is an iconic FL/Lemon workflow and the
rack also hosts non-track things (layer channels, cut groups, per-channel step editing).

Design for Linear mode (Classic mode keeps the rack exactly as is):

- **Tracks absorb the rack's per-channel role.** Every channel in Linear mode is owned by a
  track (that's the point of the Track layer). The rack as a *window* goes away; its
  per-channel capabilities move to where the channel now lives: the device rack (§7) for
  instrument/settings, the track header for name/colour/mute/icon/swing.
- **Step sequencer becomes an inline drum-lane view.** A MIDI track whose channel is a
  drum/percussion kit (Lemon Drum Sampler, FPC-style) can switch its lane display to a
  step-sequencer grid (rows = kit pieces, columns = steps, mini piano rolls per row as
  today). Same data (pattern notes), same engine — a different *presentation* of the lane,
  toggled per track. This is the "iconic part" preserved: it becomes a view mode on the
  track, not a separate window.
- **Melodic step entry** stays in the piano roll; the rack's mini piano-roll preview per
  channel maps to the lane's collapsed preview.
- **Rack-only concepts find new homes:**
  - Layer channels → device-rack device ("Layer" device with children, like Bitwig's
    instrument containers) or a track group playing the same pattern.
  - Cut groups / cut itself → per-instrument device settings (they belong to the
    instrument, not the rack).
  - Swing, mute-LED, mixer-box menus → track header + strip controls (they're already
    exposed on mixer tracks).
  - "Receive notes from" locks → track input settings (input routing section on the
    track header).
- **Pattern-first users are not stranded:** PAT mode and the pattern blocks keep working
  in Linear mode; the step-sequence lane is just the fastest way to program a beat now.
- **Risks for reviewers:** discoverability of rack-only features (need a feature-parity
  audit list of every rack menu item and its Linear-mode home — good subagent task);
  density of inline step lanes at track height (lane rows need vertical space — collapse
  to a single row and expand on click, like automation lanes); drag-and-drop flows that
  assume rack order (samples dropped on the rack create a track now).

### 6. Risks / open questions for reviewers
- Pattern-clip vs note-clip model: does v1 (pattern-as-detail) satisfy? This is the main
  UX-decision risk.
- Track order vs channel-rack order vs mixer order: propose track order as the single
  source of truth in Linear mode; Classic mode unchanged. Where do automation clips live?
  (Proposal: automation lanes per track, Ableton-style, but v1 keeps automation clips as
  channels and just shows them on their track's lane.)
- Undo/control-plane: Track create/delete must be atomic undo steps and exposed to the
  control plane (it's the scripting surface agents use).
- MPE + track model land in the same code neighbourhood (note input) — sequence after MPE
  to avoid double-refactor of midi_in/engine seams.
- Sizing: mostly os-app/os-ui + os-model records; engine untouched except none. Biggest
  single risk is playlist edit UX complexity, not audio.

### Suggested milestone order
1. Track data model + create/delete/undo + `.flp`/`.lemon` round-trip of links
2. Linear-mode project template + insert flows + track headers with A/I semantics reused
3. Track-header mini mixer strip + track-ordered mixer view
3b. Automation lanes per track (§6b): bind auto-clips to lanes, Unassigned group for
    orphaned targets; classic mode keeps free clips
4. MIDI record into track + inline-note-clip decision spike (v2 gate)
5. (Deferred — much later) Migration command for existing/FL projects. v1 ships without it;
   Linear mode targets new projects first.
   Control-plane commands + docs/status.md update still belong in v1.
