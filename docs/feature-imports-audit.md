# Architecture audit for the planned feature imports (2026-11-04)

Companion to `AUDIT-2026-11-04.md` (code-quality audit). This one maps the existing
architecture to the six features queued in `todo.md` ("Import queue — features from
Michael's Lemon"), and says which subsystems each would touch.

## The shape of the codebase (what the imports plug into)

Cargo workspace, ~26 crates. Data flows one way:

- `os-model` — project data model, no I/O. `Note` (lib.rs:1362) is a plain Copy
  struct (position, length, channel, key, fine_pitch, release, pan, velocity,
  mod_x, mod_y, flags, group, repeat + kept-unknown bytes for FLP round-trip).
  `Channel` (lib.rs:680) holds `plugin: Option<String>` — channels are
  plugin-name-keyed, so a new instrument is mostly a new plugin crate plus
  registration, not model surgery.
- `os-project` — lossless `.flp` event reader/writer; anything unknown is kept and
  written back. `os-smf` for MIDI files.
- `os-engine` — `engine.rs` (9,515 lines) owns channels, voices, envelopes,
  channel `midi(channel, bytes, bend_cents)` (engine.rs:1905) for live MIDI,
  and an `Instrument` trait (instrument.rs) with tuning-as-pitch-bend logic
  (`TuningBend`, instrument.rs:317) for instruments that can't take per-note
  tuning.
- `os-app` — `midi_in.rs` (429 lines: parse → `MidiIn::poll` → events), all UI.
  MIDI note input is 🚧 (README: pitch bend + aftertouch already handled).
- Plugins — each built-in instrument/effect is its own crate under `plugins/`,
  built on `nih-plug` (VST3+CLAP), sharing `os-ui`, `os-theme`, `os-plugkit`,
  `os-engine`, `os-dsp`. The app links them in with a `features = ["linked"]`
  dependency (`crates/os-app/Cargo.toml`) so one binary is both the DAW and the
  plugin host of its own plugins; `os-hostlink` wires their editors into the
  app's plugin-window chrome. Lemon Slicer and Lemon Drum Sampler are the
  closest precedents for sampler-style channels; Citrus is the precedent for a
  large synth (separate voice.rs / engine.rs / wave.rs / patch.rs).
- CI (`.github/workflows/ci.yml`) — one build matrix: windows-latest,
  ubuntu-24.04, macos-latest (Apple silicon), then build + bundle-plugins +
  test. Platform arch guards in code are few: `os-ui` SIMD under
  `cfg(target_arch = "x86_64")` (lib.rs:1169, 1236), VST2/VST3 arch paths and a
  mac-aarch64 "Intel-only plugin" filter (`os-vst2/files.rs:144`,
  `os-vst3/lib.rs:149`), plugin-scan macOS-arch check (`plugin_scan.rs:336`).
- Networking — there is none. No HTTP client anywhere in the manifests; the
  only sockets are the local JSON-RPC control plane (`docs/control-plane.md`,
  `os-app/src/control/`). Settings/dirs live in `os-paths`.

## Feature by feature

### 1. Master Assistant (oXygen integration)
Touches: `os-engine` (offline render + a tap on the mix), `os-dsp` (analysis
chain), `os-app` (assistant UI), `os-project`/`os-model` (storing applied settings),
plugin system (could alternatively ship as a Lemon effect plugin).
- The `--render` / `os-render` offline path gives the analysis pass a clean
  entry point without touching the audio thread; real-time later hits the same
  render-ahead engine the audit flagged for RT-hardening.
- The plugin-crate pattern fits "analyze → suggest → apply" as a master-slot
  effect, and the control plane already has `apply`/`propose` semantics for an
  assistant-driven change (undoable, named steps) — this feature can reuse it.
- License/vendoring gate (todo.md step 1) lands in `THIRD-PARTY-NOTICES.txt`;
  `cargo test` enforces notices ↔ Cargo.lock sync.

### 2. MPE support
Touches: `os-app/src/midi_in.rs` (parse MPE zone, per-channel bend/pressure),
`os-engine` (`Voice`/channel state, `instrument.rs` MIDI routing), `os-model`
(Note, if per-note expression is recorded), `os-project` (round-trip of any new
per-note data).
- The seam already exists: `engine.midi()` takes raw 3-byte messages with a
  `bend_cents` scale, and `instrument.rs` already models per-note tuning vs
  channel bend (`TuningBend`) — MPE per-note bend is a generalization of that
  path, not a new one.
- The `Note` struct deliberately keeps unknown record bytes (`unk14`, spare
  group nibbles); FL's format has no MPE fields, so recorded per-note
  expression needs either those spare bytes or `.lemon`-only side data, and
  must stay FLP-lossless.
- Recording (🚧 note-input path) and the plugin-wrapper bend-range plumbing
  (`set_bend_range`, instrument.rs:91) are the other two consumers.

### 3. Intel Mac support
Touches: CI only + two small code areas. Least invasive of the six.
- Add `x86_64-apple-darwin` to the ci.yml matrix (and a universal2 bundling
  step in `tools/bundle-app` if shipping .dmg).
- Code risk is concentrated and known: the x86_64-only SIMD paths in `os-ui`
  need an aarch64 fallback (they must already have one — macos-latest is ARM —
  so verify rather than port), and `os-vst2`/`os-vst3`/`plugin_scan` arch
  guards flip behavior per-arch and should be smoke-tested on Intel.
- `bundle-plugins`/`bundle-app` arch naming (`tools/bundle-plugins/main.rs:128`)
  needs the new triple; Intel-only VST2 plugins will load on Intel Macs where
  the Apple-silicon filter (os-vst2/files.rs:144) currently rejects them.

### 4. Lemon Chop sampler
Touches: a new `plugins/` crate (or an extension of lemon-sampler), `os-model`
Channel/sampler settings, `os-project` mapping if it plays an FL channel type,
`os-app` (browser/drop integration, piano-roll dump).
- Lemon Slicer (`plugins/lemon-slicer`: slicing.rs, dump.rs) already does
  load → slice by markers/beats/transients → play slice n on C5+n → dump to
  piano roll, and Tesla auto-slice shares the transient detector. Chop is a
  workflow variant of an existing, working pipeline.
- Slicer channels round-trip Fruity Slicer byte-for-byte; the same "play an FL
  channel type, save back unchanged" pattern is the template if Chop maps to an
  FL counterpart. todo.md correctly gates this on upstream stabilizing.

### 5. Lemon Wave wavetable synth
Touches: new `plugins/lemon-wave` crate on the nih-plug/os-ui/os-hostlink
pattern, `os-dsp` (wavetable playback/morph), `os-model` + `os-project` only if it
must play an FL/Sytrus-style channel.
- Citrus is the direct precedent: a six-op synth with engine.rs/voice.rs/
  wave.rs/patch.rs/preset.rs, playing Sytrus state byte-for-byte. Lemon Wave
  clones that crate shape; WT import reuses the sample-loading paths Lemon
  Sampler already has (`sample.rs`, keep-on-disk streaming is irrelevant for
  WT tables — they must be in RAM).
- The morph UI is `os-ui` widget kit + `os-hostlink` page tabs (a new page in
  the plugin-window strip, like the Sampler's sample/envelope pages).

### 6. Community voting / bug forum / in-app changelog
Touches: `os-app` (new panel), `os-paths` (changelog cache), plus the first
networked code in the repo.
- No HTTP dependency exists; this feature adds the first one (or raw sockets).
  The standing rule ($0 / self-host first) + the todo.md note point at
  GitHub Issues/Discussions API; that needs a TLS client crate and a GitHub
  token story — nothing in the current tree handles credentials.
- Natural seams: `os-app` panel + `os-paths` user dir for cached
  changelog JSON; releases already have a version in
  `[workspace.package] version`. The control plane's security posture
  (127.0.0.1, token, parse-error-then-close) is a good in-repo precedent for
  how the project treats network input.

## Cross-cutting cautions (from the code-quality audit, where they bite these imports)

- Any feature that renders through the live engine (1, 2, 4, 5) sits on the
  engine the quality audit marked "needs RT-hardening before product work" —
  per-note MPE and a wavetable voice add per-voice cost to the sinc-resampler
  hot path (engine.rs:4020-4091).
- `engine.rs` (9,515 lines) and `os-project/project.rs` (7,098 lines) are already
  at the size where two of these imports (MPE, any new FLP mapping) make the
  split urgent rather than optional.
- The clean-room rule applies to all six: Michael's fork and oXygen are
  specs-only sources; oXygen's license must be vetted before any vendoring.
