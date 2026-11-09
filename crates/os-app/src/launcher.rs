// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The clip launcher, Bitwig-style: the scenes are columns (their launch buttons run along the top), one row per
//! track in the mixer's order, a clip per cell. A clip plays one of the project's patterns, so a launched cell puts
//! that pattern on the air through the channel the track already has — the device rack, the mixer and the piano roll
//! all see the same notes.
//!
//! Every clip and every scene carries Bitwig's launcher parameters (`LaunchQ`, `PlayMode`, `OnRelease`, main and
//! ALT), a next action with its own timing, a name and a colour; a scene may override the launch settings of the
//! clips it starts. Clips, scenes and track headers are renamed and coloured from their right-click menus.
//!
//! ## Recording into a cell
//!
//! A clip's menu records into its cell ([`record`]): Record is armed, the cell is the take's target (`Rec::slot`) and
//! playback starts, so what is played lands in the cell's own clip. An empty cell gets a pattern and a clip of its own
//! as the first note comes ([`take_pattern`], [`bind_take`]) and shows the clip while it is recorded; one that already
//! holds a clip records into that clip's pattern. The clip's length is its pattern's (whole bars past the last note),
//! so it loops like any other clip, and stopping the track or the transport ends the take (`launcher::stop`,
//! `record::finish_slot_take`).
//!
//! ## Clips play independently
//!
//! Tracks do not share one pattern any more. While launcher clips run, the project the transport plays is
//! [`shadow`]: the arrangement's clips on those tracks are replaced by each playing clip's loop (a copy of its
//! pattern holding only that track's channel's notes, repeated at its own length from the tick it was launched at),
//! so three tracks with three clips at three lengths play together, each looping on its own. The transport loops the
//! whole window ([`window`]) so the set never runs out; when a track launches a different clip only that track's row
//! changes.
//!
//! The limit of that shortcut: the window is a common multiple of the clips' lengths, so a clip launched part-way
//! into a longer one re-aligns at the window's end (Bitwig's engine loops each clip on its own forever). See
//! docs/clip-launcher.md.
//!
//! Tab flips the Playlist window between the arrangement and the launcher ([`crate::app::State::launcher_open`]).

use std::collections::BTreeMap;

use os_model::{Clip, ClipSource, Project};

use crate::app::{Action, App, State};

/// How many scenes a launcher has (Bitwig's scenes are columns; these are its slots).
pub const SCENES: usize = 8;

/// How long the launcher's play window grows at most, in bars: the common multiple of the playing clips' lengths
/// (so the transport's loop is seamless) capped here.
const MAX_WINDOW_BARS: u32 = 64;

// ---------------------------------------------------------------------------------------------------------------
// The launch parameters (Bitwig: "Launcher Clip Parameters")

/// Launch quantization: the grid line a launch waits for (`Lexicon` in docs/clip-launcher.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LaunchQ {
    /// `Off`: the clip starts the moment it is triggered.
    Off,
    Sixteenth,
    Eighth,
    Quarter,
    Half,
    #[default]
    Bar,
    Bars2,
    Bars4,
    Bars8,
}

impl LaunchQ {
    pub const ALL: [LaunchQ; 9] =
        [LaunchQ::Off, LaunchQ::Sixteenth, LaunchQ::Eighth, LaunchQ::Quarter, LaunchQ::Half, LaunchQ::Bar, LaunchQ::Bars2, LaunchQ::Bars4, LaunchQ::Bars8];

    /// The name the menus show ("1/16", "1 bar", "Off").
    pub fn name(self) -> &'static str {
        match self {
            LaunchQ::Off => "Off",
            LaunchQ::Sixteenth => "1/16",
            LaunchQ::Eighth => "1/8",
            LaunchQ::Quarter => "1/4",
            LaunchQ::Half => "1/2",
            LaunchQ::Bar => "1 bar",
            LaunchQ::Bars2 => "2 bars",
            LaunchQ::Bars4 => "4 bars",
            LaunchQ::Bars8 => "8 bars",
        }
    }

    /// The next step of the cycle (a menu item walks the list).
    pub fn next(self) -> LaunchQ {
        let i = LaunchQ::ALL.iter().position(|&q| q == self).unwrap_or(5);
        LaunchQ::ALL[(i + 1) % LaunchQ::ALL.len()]
    }

    /// Ticks between two lines of this grid (0 for `Off`).
    pub fn ticks(self, p: &Project) -> u64 {
        let bar = u64::from(p.bar_ticks().max(1));
        match self {
            LaunchQ::Off => 0,
            LaunchQ::Sixteenth => (bar / 16).max(1),
            LaunchQ::Eighth => (bar / 8).max(1),
            LaunchQ::Quarter => (bar / 4).max(1),
            LaunchQ::Half => (bar / 2).max(1),
            LaunchQ::Bar => bar,
            LaunchQ::Bars2 => bar * 2,
            LaunchQ::Bars4 => bar * 4,
            LaunchQ::Bars8 => bar * 8,
        }
    }
}

/// Where a launched clip begins playing (Bitwig's Play Mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayFrom {
    /// "Trigger from Start": from its beginning.
    #[default]
    Start,
    /// "Legato from Clip (or Start)": at the playing clip's position (from the start when nothing played).
    LegatoClip,
    /// "Legato from Project": at the transport's own position.
    LegatoProject,
}

impl PlayFrom {
    pub const ALL: [PlayFrom; 3] = [PlayFrom::Start, PlayFrom::LegatoClip, PlayFrom::LegatoProject];

    pub fn name(self) -> &'static str {
        match self {
            PlayFrom::Start => "From start",
            PlayFrom::LegatoClip => "Legato (clip)",
            PlayFrom::LegatoProject => "Legato (project)",
        }
    }

    pub fn next(self) -> PlayFrom {
        let i = PlayFrom::ALL.iter().position(|&m| m == self).unwrap_or(0);
        PlayFrom::ALL[(i + 1) % PlayFrom::ALL.len()]
    }
}

/// What the release of a triggering gesture does (Bitwig's "on Release").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OnRelease {
    /// "Continue": the clip plays on.
    #[default]
    Continue,
    /// "Stop": the clip stops.
    Stop,
    /// "Return": the clip that played before comes back (the arrangement when it was playing last).
    Return,
    /// "Next Action": the clip's next action runs at once.
    NextAction,
}

impl OnRelease {
    pub const ALL: [OnRelease; 4] = [OnRelease::Continue, OnRelease::Stop, OnRelease::Return, OnRelease::NextAction];

    pub fn name(self) -> &'static str {
        match self {
            OnRelease::Continue => "Continue",
            OnRelease::Stop => "Stop",
            OnRelease::Return => "Return",
            OnRelease::NextAction => "Next action",
        }
    }

    pub fn next(self) -> OnRelease {
        let i = OnRelease::ALL.iter().position(|&m| m == self).unwrap_or(0);
        OnRelease::ALL[(i + 1) % OnRelease::ALL.len()]
    }
}

/// One launch setting set: the Main column, or its ALT twin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Launch {
    pub q: LaunchQ,
    pub play: PlayFrom,
    pub release: OnRelease,
    /// "Q to Loop": quantize to the loop's start rather than the clip's.
    pub q_to_loop: bool,
}

/// What happens when a clip has played its while (Bitwig's Next Action functions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NextAction {
    /// Not enabled (or "none"): the clip plays on.
    #[default]
    None,
    /// Local: the clip stops.
    Stop,
    /// Return to the arrangement on this track.
    ReturnToArrangement,
    /// The clip that played before this one comes back (none: it stops).
    ReturnToLastClip,
    PlayNext,
    PlayPrevious,
    PlayFirst,
    PlayLast,
    PlayRandom,
    /// Any other clip of the track at random.
    PlayOther,
    /// The next clip, wrapping round at the track's last one.
    RoundRobin,
    FirstInBlock,
    LastInBlock,
    RandomInBlock,
    OtherInBlock,
    RoundRobinInBlock,
    FirstInNextBlock,
    RandomInNextBlock,
    FirstInPreviousBlock,
    RandomInPreviousBlock,
    FirstInOtherBlock,
    RandomInOtherBlock,
}

impl NextAction {
    pub const ALL: [NextAction; 22] = [
        NextAction::None,
        NextAction::Stop,
        NextAction::ReturnToArrangement,
        NextAction::ReturnToLastClip,
        NextAction::PlayNext,
        NextAction::PlayPrevious,
        NextAction::PlayFirst,
        NextAction::PlayLast,
        NextAction::PlayRandom,
        NextAction::PlayOther,
        NextAction::RoundRobin,
        NextAction::FirstInBlock,
        NextAction::LastInBlock,
        NextAction::RandomInBlock,
        NextAction::OtherInBlock,
        NextAction::RoundRobinInBlock,
        NextAction::FirstInNextBlock,
        NextAction::RandomInNextBlock,
        NextAction::FirstInPreviousBlock,
        NextAction::RandomInPreviousBlock,
        NextAction::FirstInOtherBlock,
        NextAction::RandomInOtherBlock,
    ];

    /// The function's name (Bitwig's own words).
    pub fn name(self) -> &'static str {
        match self {
            NextAction::None => "Off",
            NextAction::Stop => "Stop",
            NextAction::ReturnToArrangement => "Return to arrangement",
            NextAction::ReturnToLastClip => "Return to last clip",
            NextAction::PlayNext => "Play next",
            NextAction::PlayPrevious => "Play previous",
            NextAction::PlayFirst => "Play first",
            NextAction::PlayLast => "Play last",
            NextAction::PlayRandom => "Play random",
            NextAction::PlayOther => "Play other",
            NextAction::RoundRobin => "Round-robin",
            NextAction::FirstInBlock => "First in block",
            NextAction::LastInBlock => "Last in block",
            NextAction::RandomInBlock => "Random in block",
            NextAction::OtherInBlock => "Other in block",
            NextAction::RoundRobinInBlock => "Round-robin in block",
            NextAction::FirstInNextBlock => "First in next block",
            NextAction::RandomInNextBlock => "Random in next block",
            NextAction::FirstInPreviousBlock => "First in previous block",
            NextAction::RandomInPreviousBlock => "Random in previous block",
            NextAction::FirstInOtherBlock => "First in other block",
            NextAction::RandomInOtherBlock => "Random in other block",
        }
    }

    pub fn next(self) -> NextAction {
        let i = NextAction::ALL.iter().position(|&a| a == self).unwrap_or(0);
        NextAction::ALL[(i + 1) % NextAction::ALL.len()]
    }
}

/// When a clip's next action fires: after `loops` whole loops of it, or `ticks` after it started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextTiming {
    /// "Loop Count before Next Action": whole loops of the clip.
    Loops(u32),
    /// "Next Action Time": ticks (bars and beats) after the clip started.
    After(u64),
}

impl Default for NextTiming {
    fn default() -> Self {
        NextTiming::Loops(1)
    }
}

impl NextTiming {
    /// The name the menu shows ("2 loops", "4 bars").
    pub fn text(self, bar: u64) -> String {
        match self {
            NextTiming::Loops(n) => format!("{n} loop{}", if n == 1 { "" } else { "s" }),
            NextTiming::After(t) => {
                let bars = t / bar.max(1);
                if bars > 0 {
                    format!("{bars} bar{}", if bars == 1 { "" } else { "s" })
                } else {
                    format!("{t} ticks")
                }
            }
        }
    }
}

/// One clip: the pattern its slot plays (1-based, as `Project::current_pattern` is) or the audio channel it plays, the
/// name it was given in the launcher, its own colour, and its launch parameters.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Slot {
    /// The pattern the cell plays; `0` when it holds audio only (`audio`).
    pub pattern: u16,
    /// The `AudioClip` channel whose sample the cell plays (`ClipSource::Channel`, `playlist::audio_channel`): a
    /// dropped sample. Its length is the sample's own (`playlist::source_len`), so it follows the tempo.
    pub audio: Option<u16>,
    pub name: Option<String>,
    pub color: Option<os_model::Rgb>,
    /// The MAIN launch settings (a right-click cycles them).
    pub main: Launch,
    /// The ALT launch settings, for a controller's ALT gesture (and the clip menu's ALT section).
    pub alt: Launch,
    /// The clip's next action (runs when its timing is up).
    pub next: NextAction,
    pub next_timing: NextTiming,
}

impl Eq for Slot {}

/// A scene: its name, colour and comment, and the launch settings it forces on the clips it launches.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    pub name: String,
    pub color: Option<os_model::Rgb>,
    pub comment: String,
    /// "Override Launch Settings": the scene's own launch settings are used for the clips it starts.
    pub override_launch: bool,
    pub main: Launch,
    pub alt: Launch,
}

impl Eq for Scene {}

/// What a launcher context menu acts on: a clip cell, a scene's launch button, or a track's row header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuTarget {
    Clip(u32, usize),
    Scene(usize),
    Track(u32),
}

/// An open context menu (`Launcher::menu`): its target and where it was opened (screen px).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Menu {
    pub target: MenuTarget,
    pub at: (f32, f32),
}

/// The palette the launcher's colour swatches offer, as 0xRRGGBB.
pub const PALETTE: [u32; 8] = [0xc8504a, 0xd0803c, 0xc8b44a, 0x5a9c4a, 0x3c8c88, 0x4a78b4, 0x8a5aa0, 0x7a8288];

/// Palette entry `i`.
pub fn palette(i: usize) -> os_model::Rgb {
    let c = PALETTE[i.min(PALETTE.len() - 1)];
    os_model::Rgb::new((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// A clip on the air on one track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Playing {
    pub scene: usize,
    /// The transport tick the clip started at (its own loop's phase).
    pub since: u64,
    /// The scene it replaced, for "Return to last clip" (`None`: nothing was playing on the track).
    pub previous: Option<usize>,
}

/// A clip waiting for its grid line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pending {
    pub scene: usize,
    pub pattern: u16,
    /// The transport tick it launches at.
    pub at: u64,
    /// The ALT launch settings (a controller's shift gesture), else the MAIN ones.
    pub alt: bool,
}

/// The launcher's grid: the clips by (track id, scene), the scene each track is playing, and the open menu.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Launcher {
    pub clips: BTreeMap<(u32, usize), Slot>,
    /// By the track's id: the clip it plays.
    pub playing: BTreeMap<u32, Playing>,
    /// By the track's id: the clip waiting for its grid line (a launch made while the transport runs).
    pub pending: BTreeMap<u32, Pending>,
    /// The scenes by index.
    pub scenes: Vec<Scene>,
    /// The context menu a right-click opened, if any.
    pub menu: Option<Menu>,
    /// The clip a mouse press launched, for the release gesture (`OnRelease`): (track, scene).
    pub pressed: Option<(u32, usize)>,
    /// How far the grid's scene columns are scrolled sideways, in content px (0: the first column at the left of the
    /// cells). The track column and the scene buttons stay put, so it moves the cells only (`views::launcher`).
    pub scroll_x: f32,
    /// How far the grid's track rows are scrolled up, in content px (0: the first row under the scene buttons).
    pub scroll_y: f32,
    /// The transport tick [`advance`] last saw, so the wrap at the looping window's end counts as its grid line.
    last_tick: f64,
}

/// A launcher column: one track, in the mixer's order.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub id: u32,
    /// The track's playlist lane, 0-based (its name and colour live there).
    pub lane: u16,
    pub name: String,
    pub color: Option<os_model::Rgb>,
    /// A MIDI track whose instrument slot is empty: it has nothing to play its clips through, so its header says so.
    pub no_instrument: bool,
    /// The track's channel (`Track::channel`), by its index: what its clips' notes are picked out by.
    pub channel: Option<u16>,
}

/// The launcher's actions (its cells, scene buttons and track headers).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Act {
    /// Launch the clip at (track id, scene); an empty slot stops the track.
    Launch(u32, usize),
    /// The ALT launch settings of the same clip (a controller's ALT gesture).
    LaunchAlt(u32, usize),
    /// The press on a clip was released: `OnRelease` does its thing.
    Release(u32, usize),
    /// Stop one track's clip.
    Stop(u32),
    /// Launch every clip of a scene.
    LaunchScene(usize),
    /// Stop every track.
    StopAll,
    /// Put the current pattern in the slot, or clear it when it already holds that pattern.
    Set(u32, usize),
    /// Record into the clip at (track id, scene): what is played lands in the cell (`record::open_take`).
    Record(u32, usize),
    /// A click on a track's row header: its lane is selected in the arrangement (so the device rack follows it).
    SelectTrack(u32),
    /// A right-click: open the context menu of a clip, a scene or a track.
    Menu(MenuTarget, (f32, f32)),
    /// Close the open context menu.
    MenuClose,
    /// Empty a clip's slot.
    Clear(u32, usize),
    /// Rename a clip in place (an inline text box, `crate::playlist::start_edit`).
    RenameClip(u32, usize),
    RenameScene(usize),
    RenameTrack(u32),
    /// Give a clip, a scene or a track a colour (`None`: back to the default).
    ColorClip(u32, usize, Option<os_model::Rgb>),
    ColorScene(usize, Option<os_model::Rgb>),
    ColorTrack(u32, Option<os_model::Rgb>),
    /// Walk a clip's launch setting to its next value: quantize (`0`), play mode (`1`), on release (`2`).
    CycleLaunch(u32, usize, u8),
    /// Walk a clip's next action to the next function.
    CycleNext(u32, usize),
    /// Walk a clip's next action timing: 1 loop, 2, 4, 8, then 1, 2, 4, 8, 16 bars.
    CycleNextTiming(u32, usize),
    /// Turn the scene's "Override Launch Settings" on / off.
    SceneOverride(usize),
}

/// The tracks the launcher has a column for, in the mixer's order (`mixer_strip`), so a column sits over the strip
/// that plays it.
pub fn columns(s: &State) -> Vec<Column> {
    let mut v: Vec<(u16, Column)> = s
        .project
        .tracks
        .iter()
        .filter(|t| t.kind != os_model::TrackKind::Bus && t.playlist_lane > 0)
        .map(|t| {
            let lane = t.playlist_lane as u16 - 1;
            (
                t.mixer_strip,
                Column {
                    id: t.id.0,
                    lane,
                    name: crate::playlist::track_name(s, lane),
                    color: crate::playlist::track_info(s, lane).and_then(|i| i.color),
                    no_instrument: instrument_missing(s, t),
                    channel: t.channel,
                },
            )
        })
        .collect();
    v.sort_by_key(|(strip, _)| *strip);
    v.into_iter().map(|(_, c)| c).collect()
}

/// Whether a MIDI track's instrument slot is empty: no channel plays it (the same test the device rack's
/// `device_rack::DeviceKind::NoInstrument` tile makes, so the two never disagree).
fn instrument_missing(s: &State, t: &os_model::Track) -> bool {
    t.kind == os_model::TrackKind::Midi
        && t.channel
            .and_then(|ch| s.project.channels.iter().position(|c| c.index == ch))
            .is_none()
}

/// The clip in (track, scene), when the slot holds one.
pub fn clip(s: &State, track: u32, scene: usize) -> Option<Slot> {
    s.launcher.clips.get(&(track, scene)).cloned()
}

/// The track's playlist lane (`Track::playlist_lane`), by its id.
pub fn lane_of(s: &State, track: u32) -> Option<u16> {
    let tr = s.project.tracks.iter().find(|t| t.id.0 == track && t.playlist_lane > 0)?;
    Some(tr.playlist_lane as u16 - 1)
}

/// The track's channel, by its id.
pub fn channel_of(s: &State, track: u32) -> Option<u16> {
    s.project.tracks.iter().find(|t| t.id.0 == track)?.channel
}

/// A clip's name: the one it was given, else its pattern's (or its sample's) name.
pub fn clip_name(s: &State, track: u32, scene: usize) -> String {
    match clip(s, track, scene) {
        Some(sl) => sl.name.clone().unwrap_or_else(|| crate::playlist::source_name(s, source(&sl))),
        None => String::new(),
    }
}

/// What a cell plays, as the playlist names it: its sample's channel for an audio clip, else its pattern.
pub fn source(slot: &Slot) -> ClipSource {
    match slot.audio {
        Some(ch) => ClipSource::Channel(ch),
        None => ClipSource::Pattern(slot.pattern),
    }
}

/// A clip's colour: its own, else its source's (a sample's tint), else its track's.
pub fn clip_color(s: &State, track: u32, scene: usize) -> Option<os_model::Rgb> {
    let Some(slot) = clip(s, track, scene) else { return None };
    slot.color
        .or_else(|| crate::playlist::clip_tint(s, source(&slot)))
        .or_else(|| crate::playlist::source_color(s, source(&slot)))
        .or_else(|| lane_of(s, track).and_then(|lane| crate::playlist::track_color(s, lane)))
}

/// A cell's own length in ticks: its sample's for an audio clip (`playlist::source_len`), else its pattern's.
pub fn slot_len(s: &State, slot: &Slot) -> u64 {
    match slot.audio {
        Some(ch) => u64::from(crate::playlist::source_len(s, ClipSource::Channel(ch)).max(1)),
        None => clip_len(s, slot.pattern),
    }
}

/// A scene's colour.
pub fn scene_color(s: &State, scene: usize) -> Option<os_model::Rgb> {
    s.launcher.scenes.get(scene).and_then(|sc| sc.color)
}

/// The scene's name ("Scene 3" when it has none).
pub fn scene_name(s: &State, scene: usize) -> String {
    match s.launcher.scenes.get(scene).map(|sc| sc.name.as_str()).filter(|n| !n.is_empty()) {
        Some(n) => n.to_string(),
        None => format!("Scene {}", scene + 1),
    }
}

/// A clip's name as the text box opens with it (an empty one lets the pattern's name show through).
pub fn set_clip_name(s: &mut State, track: u32, scene: usize, text: &str) {
    if let Some(sl) = s.launcher.clips.get_mut(&(track, scene)) {
        sl.name = (!text.trim().is_empty()).then(|| text.trim().to_string());
    }
}

/// A scene's name: an empty one reads "Scene N" again.
pub fn set_scene_name(s: &mut State, scene: usize, text: &str) {
    let scenes = &mut s.launcher.scenes;
    while scenes.len() <= scene {
        scenes.push(Scene::default());
    }
    scenes[scene].name = text.trim().to_string();
    normalize(s);
}

/// Grows the scene list to `SCENES` and drops trailing empty ones.
fn normalize(s: &mut State) {
    let v = &mut s.launcher.scenes;
    while v.len() < SCENES {
        v.push(Scene::default());
    }
    while v.len() > SCENES && v.last().is_some_and(|sc| sc == &Scene::default()) {
        v.pop();
    }
}

/// The rows of the context menu a right-click opened: the target's own commands (the colour swatches follow them in
/// the view, `views::launcher::menu_paint`).
pub fn menu_items(s: &State, target: MenuTarget) -> Vec<(&'static str, String, Act)> {
    match target {
        MenuTarget::Clip(track, scene) => {
            let Some(sl) = clip(s, track, scene) else {
                return vec![("Set current pattern".into(), String::new(), Act::Set(track, scene))];
            };
            vec![
                ("Set current pattern", String::new(), Act::Set(track, scene)),
                ("Record into this clip", String::new(), Act::Record(track, scene)),
                // Bitwig's Launch section: each row shows what it holds and a click walks it to the next value.
                ("Quantize", sl.main.q.name().to_string(), Act::CycleLaunch(track, scene, 0)),
                ("Play mode", sl.main.play.name().to_string(), Act::CycleLaunch(track, scene, 1)),
                ("On release", sl.main.release.name().to_string(), Act::CycleLaunch(track, scene, 2)),
                ("Next action", sl.next.name().to_string(), Act::CycleNext(track, scene)),
                ("Next action after", sl.next_timing.text(u64::from(s.project.bar_ticks())), Act::CycleNextTiming(track, scene)),
                ("Q to loop", if sl.main.q_to_loop { "On".into() } else { "Off".into() }, Act::CycleLaunch(track, scene, 3)),
                // Bitwig duplicates the whole launch section for the ALT gesture.
                ("ALT quantize", sl.alt.q.name().to_string(), Act::CycleLaunch(track, scene, 4)),
                ("ALT play mode", sl.alt.play.name().to_string(), Act::CycleLaunch(track, scene, 5)),
                ("ALT on release", sl.alt.release.name().to_string(), Act::CycleLaunch(track, scene, 6)),
                ("Rename...", String::new(), Act::RenameClip(track, scene)),
                ("Clear clip", String::new(), Act::Clear(track, scene)),
            ]
        }
        MenuTarget::Scene(scene) => {
            let sc = s.launcher.scenes.get(scene).cloned().unwrap_or_default();
            vec![
                ("Launch scene", String::new(), Act::LaunchScene(scene)),
                ("Override launch settings", if sc.override_launch { "On".into() } else { "Off".into() }, Act::SceneOverride(scene)),
                ("Rename...", String::new(), Act::RenameScene(scene)),
            ]
        }
        MenuTarget::Track(track) => vec![
            ("Rename...", String::new(), Act::RenameTrack(track)),
            ("Select in the playlist", String::new(), Act::SelectTrack(track)),
            ("Stop", String::new(), Act::Stop(track)),
        ],
    }
}

/// The act that gives `target` `color`, for the menu's swatches.
pub fn color_act(target: MenuTarget, color: Option<os_model::Rgb>) -> Act {
    match target {
        MenuTarget::Clip(track, scene) => Act::ColorClip(track, scene, color),
        MenuTarget::Scene(scene) => Act::ColorScene(scene, color),
        MenuTarget::Track(track) => Act::ColorTrack(track, color),
    }
}

/// The clip a track plays, when it plays one.
pub fn playing(s: &State, track: u32) -> Option<usize> {
    s.launcher.playing.get(&track).map(|p| p.scene)
}

/// The scene a track is waiting to launch at its grid line, when it has one queued.
pub fn pending(s: &State, track: u32) -> Option<usize> {
    s.launcher.pending.get(&track).map(|p| p.scene)
}

/// Whether any track plays a launcher clip: the transport then plays the launcher's window ([`shadow`]).
pub fn live(s: &State) -> bool {
    !s.launcher.playing.is_empty()
}

/// The tick of the next grid line `step` ticks apart after `tick`: a launch made on a line waits for the next one, as
/// Bitwig's quantization does.
fn next_line(tick: f64, step: u64) -> u64 {
    if step == 0 {
        return tick.max(0.0) as u64;
    }
    let step = step as f64;
    (((tick / step).floor() + 1.0) * step).max(0.0) as u64
}

/// The grid a launch waits for: its own quantization, or the loop's start when `Q to Loop` is on (the whole window is
/// the launcher's loop, so the clip waits for the window to wrap).
fn grid_step(s: &State, l: Launch) -> u64 {
    if l.q_to_loop {
        window(s).max(1)
    } else {
        l.q.ticks(&s.project)
    }
}

/// Whether a launch has to wait: the transport runs (a stopped transport starts the clip at once — there is no grid
/// line to wait for).
fn waiting(app: &App) -> bool {
    app.transport.playing() && !app.transport.paused()
}

/// A pattern's length in ticks (`Project::pattern_length`).
pub fn clip_len(s: &State, pattern: u16) -> u64 {
    u64::from(s.project.pattern_length(pattern).max(1))
}

/// Where a new clip starts inside itself (Bitwig's Play Mode): 0 from its start, the playing clip's phase for
/// "legato from clip", the transport's own position for "legato from project".
fn legato_phase(app: &App, track: u32, play: PlayFrom, len: u64) -> u64 {
    let len = len.max(1);
    let now = app.transport.tick().max(0.0) as u64;
    match play {
        PlayFrom::Start => 0,
        PlayFrom::LegatoClip => match app.state.launcher.playing.get(&track) {
            Some(p) => now.saturating_sub(p.since) % len,
            None => 0,
        },
        PlayFrom::LegatoProject => now % len,
    }
}

/// The launch settings a clip launches with: the scene's when it overrides, else the clip's own (MAIN or ALT).
fn settings(s: &State, track: u32, scene: usize, alt: bool) -> Launch {
    let sc = s.launcher.scenes.get(scene).cloned().unwrap_or_default();
    if sc.override_launch {
        return if alt { sc.alt } else { sc.main };
    }
    match clip(s, track, scene) {
        Some(sl) => {
            let l = if alt { sl.alt } else { sl.main };
            // "Use Project Setting": a default clip takes the project's own launch quantization.
            l
        }
        None => Launch::default(),
    }
}

/// Puts (track, scene) on the air now, at tick `at`.
fn play_now(app: &mut App, track: u32, scene: usize, slot: &Slot, alt: bool, at: u64) {
    let previous = app.state.launcher.playing.get(&track).map(|p| p.scene);
    // Bitwig's Play Mode: where in itself the clip starts.
    let play = settings(&app.state, track, scene, alt).play;
    let len = slot_len(&app.state, slot);
    let offset = legato_phase(app, track, play, len);
    let since = at.saturating_sub(offset);
    app.state.launcher.pending.remove(&track);
    app.state.launcher.playing.insert(track, Playing { scene, since, previous });
    play_cell(app, slot);
}

/// The clip goes on the air: a pattern clip's pattern becomes the current one (`if pattern > 0`: an audio cell has
/// none), the launcher's mode takes over playback and the transport starts when it is stopped. The engine's project
/// is rebuilt from the launcher's window (`shadow`).
fn play_cell(app: &mut App, slot: &Slot) {
    if slot.pattern > 0 {
        app.state.pattern = slot.pattern;
        app.state.project.current_pattern = Some(slot.pattern);
    }
    if !app.transport.playing() {
        app.act(Action::Play);
    } else {
        // Clips already run: the playing project's tracks change, so the engine is given the new one.
        let p = crate::rack::play_project(&app.state);
        app.transport.update(&p);
    }
    app.invalidate();
}

/// Puts `pattern` in (track, scene), or empties the slot with `None`.
pub fn set(app: &mut App, track: u32, scene: usize, pattern: Option<u16>) {
    match pattern {
        Some(p) => {
            let old = clip(&app.state, track, scene);
            // A cell holding a sample takes the pattern instead of it (`audio: None`).
            app.state.launcher.clips.insert(
                (track, scene),
                Slot { pattern: p, audio: None, ..old.unwrap_or_default() },
            );
        }
        None => {
            app.state.launcher.clips.remove(&(track, scene));
            // A cell a take records into is no clip any more: the take goes with it.
            if app.state.rec.slot == Some((track, scene)) {
                app.state.rec.slot = None;
            }
        }
    }
    // A cleared or replaced slot cannot keep playing.
    if playing(&app.state, track) == Some(scene) && clip(&app.state, track, scene).is_none() {
        app.state.launcher.playing.remove(&track);
    }
    app.invalidate();
}

/// Arms a cell for recording (`Act::Record`): Record is armed, the cell is the take's target and playback starts when
/// it is stopped, so what is played lands in the cell's clip. A cell with a clip records into that clip's pattern
/// (over it, as the record options say); an empty one gets a pattern and a clip of its own when the first note comes
/// ([`take_pattern`], `record::open_take`).
///
/// Stopping the transport (or the track, `stop`) ends the take: the clip keeps what it captured, and its length is its
/// pattern's (whole bars past the last note), so it loops at that length like any other clip.
pub fn record(app: &mut App, track: u32, scene: usize) {
    app.state.launcher.menu = None;
    app.state.launcher.pressed = None;
    app.state.rec.slot = Some((track, scene));
    app.state.rec.armed = true;
    if !app.transport.playing() {
        app.play_counted_if_armed();
    }
    app.invalidate();
}

/// The pattern a cell's take records into, and whether it was made for the take: the clip's own pattern, or a new empty
/// one the cell is bound to (so the clip shows on the grid while it is recorded). An empty take leaves the cell empty
/// again (`record::finish_slot_take`).
///
/// A pattern number past every one the project has keeps the take out of the project's own patterns (its notes stay
/// visible as a clip's, not swapped into the channel rack's pattern).
pub fn take_pattern(s: &State, track: u32, scene: usize) -> (u16, bool) {
    // An audio clip has no pattern to record into: the take makes one and the cell takes it instead of the sample.
    if let Some(sl) = clip(s, track, scene).filter(|sl| sl.audio.is_none()) {
        return (sl.pattern, false);
    }
    (next_pattern(s), true)
}

/// A pattern number no pattern has: past every one the project has.
pub fn next_pattern(s: &State) -> u16 {
    s.project.patterns.iter().map(|p| p.number).max().unwrap_or(0).saturating_add(1).max(1)
}

/// Makes the pattern [`take_pattern`] named for a cell the take records into, and binds the cell to it.
pub fn bind_take(s: &mut State, track: u32, scene: usize, pattern: u16) {
    if !s.project.patterns.iter().any(|p| p.number == pattern) {
        s.project.patterns.push(os_model::Pattern { number: pattern, ..Default::default() });
    }
    s.launcher.clips.insert((track, scene), Slot { pattern, ..Default::default() });
    s.launcher.playing.remove(&track);
}

/// Whether a cell is the one a take records into.
pub fn recording(s: &State, track: u32, scene: usize) -> bool {
    s.rec.slot == Some((track, scene))
}

/// Launches (track, scene) with the clip's MAIN launch settings: it waits for the next grid line of its
/// quantization while the transport runs, and starts at once when it is stopped. An empty slot stops the track.
pub fn launch(app: &mut App, track: u32, scene: usize) {
    launch_with(app, track, scene, false);
}

/// [`launch`] with the ALT launch settings.
pub fn launch_alt(app: &mut App, track: u32, scene: usize) {
    launch_with(app, track, scene, true);
}

fn launch_with(app: &mut App, track: u32, scene: usize, alt: bool) {
    let Some(slot) = clip(&app.state, track, scene) else {
        stop(app, track);
        return;
    };
    let l = settings(&app.state, track, scene, alt);
    if waiting(app) {
        let at = next_line(app.transport.tick(), grid_step(&app.state, l));
        app.state.launcher.pending.insert(track, Pending { scene, pattern: slot.pattern, at, alt });
        app.invalidate();
        return;
    }
    play_now(app, track, scene, &slot, alt, app.transport.tick().max(0.0) as u64);
}

/// The press on a clip was released: Bitwig's "on Release". `Continue` leaves it playing, `Stop` stops the track,
/// `Return` brings back the clip it replaced, `Next action` runs the clip's next action now.
pub fn release(app: &mut App, track: u32, scene: usize) {
    if app.state.launcher.pressed.take() != Some((track, scene)) {
        return;
    }
    let Some(sl) = clip(&app.state, track, scene) else { return };
    let l = settings(&app.state, track, scene, false);
    match l.release {
        OnRelease::Continue => {}
        OnRelease::Stop => stop(app, track),
        OnRelease::Return => return_to_last(app, track),
        OnRelease::NextAction => run_next(app, track, sl.next),
    }
    app.invalidate();
}

/// Stops a track's clip, and drops one it was waiting to launch. Recording into one of its cells ends with it (the
/// clip keeps what the take captured).
pub fn stop(app: &mut App, track: u32) {
    if app.state.rec.slot.is_some_and(|(t, _)| t == track) {
        app.end_take();
    }
    app.state.launcher.playing.remove(&track);
    app.state.launcher.pending.remove(&track);
    app.invalidate();
}

/// Brings back the clip a track played before the current one (the track stops when there was none).
fn return_to_last(app: &mut App, track: u32) {
    let previous = app.state.launcher.playing.get(&track).and_then(|p| p.previous);
    match previous.and_then(|sc| clip(&app.state, track, sc).map(|sl| (sc, sl))) {
        Some((sc, sl)) => {
            let at = app.transport.tick().max(0.0) as u64;
            let old = playing(&app.state, track);
            app.state.launcher.playing.insert(track, Playing { scene: sc, since: at, previous: old });
            play_cell(app, &sl);
        }
        None => stop(app, track),
    }
}

/// Launches every clip of a scene: at the next grid line while the transport runs, at once when it is stopped. The
/// scene's launch settings apply when it overrides them.
pub fn launch_scene(app: &mut App, scene: usize) {
    let cols = columns(&app.state);
    let clips: Vec<(u32, Slot)> = cols
        .iter()
        .filter_map(|c| clip(&app.state, c.id, scene).map(|slot| (c.id, slot)))
        .collect();
    if clips.is_empty() {
        app.invalidate();
        return;
    }
    let l = settings(&app.state, clips[0].0, scene, false);
    if waiting(app) {
        let at = next_line(app.transport.tick(), grid_step(&app.state, l));
        for (track, slot) in clips {
            app.state.launcher.pending.insert(track, Pending { scene, pattern: slot.pattern, at, alt: false });
        }
        app.invalidate();
        return;
    }
    let at = app.transport.tick().max(0.0) as u64;
    let mut first: Option<Slot> = None;
    for (track, slot) in clips {
        let previous = playing(&app.state, track);
        app.state.launcher.pending.remove(&track);
        app.state.launcher.playing.insert(track, Playing { scene, since: at, previous });
        first.get_or_insert(slot);
    }
    match first {
        Some(slot) => play_cell(app, &slot),
        None => app.invalidate(),
    }
}

/// Stops every track, and drops every queued launch (a take recording into a cell ends with them).
pub fn stop_all(app: &mut App) {
    if app.state.rec.slot.is_some() {
        app.end_take();
    }
    app.state.launcher.playing.clear();
    app.state.launcher.pending.clear();
    app.invalidate();
}

/// Applies the clips waiting for their grid line (called from `App::tick`). True when one went on the air.
pub fn advance(app: &mut App) -> bool {
    let running = waiting(app);
    let now = app.transport.tick();
    let before = app.state.launcher.playing.clone();
    let launched = advance_at(app, now, running);
    let fired = next_actions(app, now);
    // A launch that changed what plays needs the engine's project rebuilt (the window's tracks changed).
    if app.state.launcher.playing != before {
        // The window (and so the transport's loop) follows what is on the air.
        app.sync_loop_range();
        if live(&app.state) {
            let p = crate::rack::play_project(&app.state);
            app.transport.update(&p);
        }
    }
    launched || fired
}

/// [`advance`] at a given transport tick, for the tests and for `advance` itself. A launch lands when its grid line
/// is reached, when a looping window wrapped past it, or as soon as the transport stops.
fn advance_at(app: &mut App, tick: f64, running: bool) -> bool {
    let last = app.state.launcher.last_tick;
    app.state.launcher.last_tick = tick;
    if app.state.launcher.pending.is_empty() {
        return false;
    }
    let wrapped = running && tick + 0.5 < last;
    let due: Vec<(u32, Pending)> = app
        .state
        .launcher
        .pending
        .iter()
        .filter(|(_, p)| !running || wrapped || tick >= p.at as f64)
        .map(|(track, p)| (*track, *p))
        .collect();
    if due.is_empty() {
        return false;
    }
    let mut pattern = None;
    let at = tick.max(0.0) as u64;
    for (track, p) in due {
        let previous = playing(&app.state, track);
        app.state.launcher.pending.remove(&track);
        app.state.launcher.playing.insert(track, Playing { scene: p.scene, since: at, previous });
        pattern = Some(p.pattern);
    }
    // An audio cell has no pattern to make current (its `pattern` is 0).
    if let Some(pattern) = pattern.filter(|&p| p > 0) {
        app.state.pattern = pattern;
        app.state.project.current_pattern = Some(pattern);
    }
    app.invalidate();
    true
}

/// Runs the next actions whose time is up (Bitwig: "Next Action ... executed at a set time after the clip started
/// playing"): after the clip's whole loops, or the ticks it was given. True when one ran.
fn next_actions(app: &mut App, tick: f64) -> bool {
    let due: Vec<(u32, usize, NextAction)> = app
        .state
        .launcher
        .playing
        .iter()
        .filter_map(|(track, p)| {
            let sl = clip(&app.state, *track, p.scene)?;
            if sl.next == NextAction::None {
                return None;
            }
            let len = slot_len(&app.state, &sl);
            let at = match sl.next_timing {
                NextTiming::Loops(n) => p.since + len * u64::from(n.max(1)),
                NextTiming::After(t) => p.since + t.max(1),
            };
            (tick >= at as f64).then_some((*track, p.scene, sl.next))
        })
        .collect();
    if due.is_empty() {
        return false;
    }
    for (track, _, action) in due {
        run_next(app, track, action);
    }
    app.invalidate();
    true
}

/// Runs a clip's next action on its track.
fn run_next(app: &mut App, track: u32, action: NextAction) {
    let Some(current) = playing(&app.state, track) else { return };
    match action {
        NextAction::None => {}
        NextAction::Stop => stop(app, track),
        NextAction::ReturnToArrangement => {
            // The arrangement takes this track back: its launcher clip stops (the arrangement's own clips play on).
            stop(app, track);
        }
        NextAction::ReturnToLastClip => return_to_last(app, track),
        NextAction::PlayNext => follow(app, track, current, Follow::Next),
        NextAction::PlayPrevious => follow(app, track, current, Follow::Previous),
        NextAction::PlayFirst => follow(app, track, current, Follow::First),
        NextAction::PlayLast => follow(app, track, current, Follow::Last),
        NextAction::PlayRandom => follow(app, track, current, Follow::Random),
        NextAction::PlayOther => follow(app, track, current, Follow::Other),
        NextAction::RoundRobin => follow(app, track, current, Follow::RoundRobin),
        NextAction::FirstInBlock => follow(app, track, current, Follow::InBlock(BlockRule::First)),
        NextAction::LastInBlock => follow(app, track, current, Follow::InBlock(BlockRule::Last)),
        NextAction::RandomInBlock => follow(app, track, current, Follow::InBlock(BlockRule::Random)),
        NextAction::OtherInBlock => follow(app, track, current, Follow::InBlock(BlockRule::Other)),
        NextAction::RoundRobinInBlock => follow(app, track, current, Follow::InBlock(BlockRule::RoundRobin)),
        NextAction::FirstInNextBlock => follow(app, track, current, Follow::NextBlock(BlockRule::First)),
        NextAction::RandomInNextBlock => follow(app, track, current, Follow::NextBlock(BlockRule::Random)),
        NextAction::FirstInPreviousBlock => follow(app, track, current, Follow::PreviousBlock(BlockRule::First)),
        NextAction::RandomInPreviousBlock => follow(app, track, current, Follow::PreviousBlock(BlockRule::Random)),
        NextAction::FirstInOtherBlock => follow(app, track, current, Follow::OtherBlock(BlockRule::First)),
        NextAction::RandomInOtherBlock => follow(app, track, current, Follow::OtherBlock(BlockRule::Random)),
    }
}

/// A rule inside a block of clips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockRule {
    First,
    Last,
    Random,
    Other,
    RoundRobin,
}

/// How a next action picks the next clip of its track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Follow {
    Next,
    Previous,
    First,
    Last,
    Random,
    Other,
    RoundRobin,
    InBlock(BlockRule),
    NextBlock(BlockRule),
    PreviousBlock(BlockRule),
    OtherBlock(BlockRule),
}

/// The scenes a track has clips in, as clip blocks: runs of neighbouring scenes holding clips.
fn blocks(s: &State, track: u32) -> Vec<Vec<usize>> {
    let scenes: Vec<usize> = (0..SCENES).filter(|&sc| clip(s, track, sc).is_some()).collect();
    let mut out: Vec<Vec<usize>> = Vec::new();
    for sc in scenes {
        match out.last_mut() {
            Some(b) if b.last().is_some_and(|&last| last + 1 == sc) => b.push(sc),
            _ => out.push(vec![sc]),
        }
    }
    out
}

/// The block a scene sits in, with the list of blocks.
fn block_of(s: &State, track: u32, scene: usize) -> (Vec<Vec<usize>>, usize) {
    let bl = blocks(s, track);
    let i = bl.iter().position(|b| b.contains(&scene)).unwrap_or(0);
    (bl, i)
}

/// A deterministic "random" choice for `n` items (the transport tick keeps it from flickering).
fn pick(n: usize, salt: u64) -> usize {
    if n == 0 {
        return 0;
    }
    (salt.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) >> 33) as usize % n
}    /// Runs a follow action: which clip the track plays next, out of its own clips.
fn follow(app: &mut App, track: u32, current: usize, how: Follow) {
    let all: Vec<usize> = (0..SCENES).filter(|&sc| clip(&app.state, track, sc).is_some()).collect();
    if all.is_empty() {
        stop(app, track);
        return;
    }
    let at = app.transport.tick().max(0.0) as u64;
    let salt = at / u64::from(app.state.project.bar_ticks().max(1));
    let target: Option<usize> = match how {
        Follow::Next => all.iter().find(|&&sc| sc > current).copied(),
        Follow::Previous => all.iter().rev().find(|&&sc| sc < current).copied(),
        Follow::First => all.first().copied(),
        Follow::Last => all.last().copied(),
        Follow::Random => Some(all[pick(all.len(), salt)]),
        Follow::Other => {
            let others: Vec<usize> = all.iter().copied().filter(|&sc| sc != current).collect();
            others.get(pick(others.len(), salt)).copied()
        }
        Follow::RoundRobin => Some(*all.iter().find(|&&sc| sc > current).unwrap_or(&all[0])),
        Follow::InBlock(rule) => {
            let (bl, i) = block_of(&app.state, track, current);
            pick_in(&bl[i], current, rule, salt)
        }
        Follow::NextBlock(rule) => {
            let (bl, i) = block_of(&app.state, track, current);
            bl.get(i + 1).and_then(|b| pick_in(b, usize::MAX, rule, salt))
        }
        Follow::PreviousBlock(rule) => {
            let (bl, i) = block_of(&app.state, track, current);
            let j = i.saturating_sub(1);
            bl.get(j).and_then(|b| pick_in(b, usize::MAX, rule, salt))
        }
        Follow::OtherBlock(rule) => {
            let (bl, i) = block_of(&app.state, track, current);
            let others: Vec<usize> = (0..bl.len()).filter(|&j| j != i).collect();
            others.get(pick(others.len(), salt)).and_then(|&j| pick_in(&bl[j], usize::MAX, rule, salt))
        }
    };
    match target.and_then(|sc| clip(&app.state, track, sc).map(|sl| (sc, sl))) {
        Some((sc, sl)) => {
            let previous = playing(&app.state, track);
            app.state.launcher.playing.insert(track, Playing { scene: sc, since: at, previous });
            play_cell(app, &sl);
        }
        None => stop(app, track),
    }
}

/// A clip inside one block, by the rule (`current` = the clip playing, `usize::MAX` for "none").
fn pick_in(block: &[usize], current: usize, rule: BlockRule, salt: u64) -> Option<usize> {
    let at = |i: usize| block.get(i).copied();
    match rule {
        BlockRule::First => at(0),
        BlockRule::Last => block.last().copied(),
        BlockRule::Random => at(pick(block.len(), salt)),
        BlockRule::Other => {
            let others: Vec<usize> = block.iter().copied().filter(|&sc| sc != current).collect();
            others.get(pick(others.len(), salt)).copied()
        }
        BlockRule::RoundRobin => {
            let i = block.iter().position(|&sc| sc == current);
            match i {
                Some(i) => at((i + 1) % block.len()),
                None => at(0),
            }
        }
    }
}

/// The common multiple of two lengths (the loop window's step).
fn lcm(a: u64, b: u64) -> u64 {
    if a == 0 || b == 0 {
        return a.max(b);
    }
    let (mut x, mut y) = (a, b);
    while y != 0 {
        let t = x % y;
        x = y;
        y = t;
    }
    a / x * b
}

/// The window the launcher's tracks loop over: the common multiple of the playing clips' lengths (so the transport's
/// own loop is seamless), capped at [`MAX_WINDOW_BARS`] bars.
pub fn window(s: &State) -> u64 {
    let bar = u64::from(s.project.bar_ticks().max(1));
    let mut l = bar;
    for (track, p) in &s.launcher.playing {
        if let Some(sl) = clip(s, *track, p.scene) {
            l = lcm(l, slot_len(s, &sl));
        }
    }
    l.min(bar * u64::from(MAX_WINDOW_BARS))
}

/// The project the transport plays while launcher clips run: `base` with the playing tracks' arrangement clips
/// replaced by their clips' loops. `None` when no clip plays.
///
/// For every playing track the track's lane gets copies of a shadow pattern — a copy of the clip's pattern holding
/// only that track's channel's notes — repeated at the clip's own length from the tick it was launched at, over the
/// window [`window`]. So the tracks play together, each looping at its own length, and the shadows stay out of the
/// project's own patterns (the piano roll, the channel rack and a save see the real ones).
pub fn shadow(s: &State, base: &Project) -> Option<Project> {
    if !live(s) {
        return None;
    }
    let w = window(s);
    if w == 0 {
        return None;
    }
    let mut p = base.clone();
    // A project without an arrangement (a new one) gets one: the launcher's clips need somewhere to run.
    if p.arrangements.is_empty() {
        p.arrangements.push(os_model::Arrangement { number: 1, ..Default::default() });
    }
    // Number the shadow patterns past every pattern the project has (they must not collide), and build them off the
    // project's own patterns before the arrangement is touched.
    let mut next_no = p.patterns.iter().map(|q| q.number).max().unwrap_or(0).saturating_add(1).max(1);
    // What a playing track puts on its lane: a shadow pattern (with the number it was given) or the sample the cell
    // plays itself.
    enum Lane {
        Pattern(u16, os_model::Pattern),
        Audio(u16),
    }
    let mut shadows: Vec<(u16, u64, u64, Lane)> = Vec::new();
    for (track, now) in s.launcher.playing.clone() {
        let Some(sl) = clip(s, track, now.scene) else { continue };
        let Some(lane) = lane_of(s, track) else { continue };
        let len = slot_len(s, &sl);
        // An audio clip is the sample itself: its channel's clip, as the arranger holds it (`ClipSource::Channel`).
        if let Some(ch) = sl.audio {
            shadows.push((lane, now.since, len, Lane::Audio(ch)));
            continue;
        }
        let channel = channel_of(s, track);
        let Some(src) = p.patterns.iter().find(|pat| pat.number == sl.pattern) else { continue };
        let trimmed: Vec<os_model::Note> = src
            .notes
            .iter()
            .filter(|n| channel.is_none_or(|c| n.channel == c) && u64::from(n.position) < len)
            .copied()
            .collect();
        if trimmed.is_empty() {
            continue;
        }
        let no = next_no;
        next_no += 1;
        let mut pat = src.clone();
        pat.number = no;
        pat.notes = trimmed;
        pat.length = Some(len as u32);
        shadows.push((lane, now.since, len, Lane::Pattern(no, pat)));
    }
    if shadows.is_empty() {
        return None;
    }
    let idx = p.arrangement_index();
    let arr = p.arrangements.get_mut(idx)?;
    // A lane per playing track (an arrangement may be shorter than the playlist).
    for &(lane, ..) in &shadows {
        while arr.tracks.len() <= usize::from(lane) {
            let n = arr.tracks.len() as u32 + 1;
            arr.tracks.push(os_model::PlaylistTrack::new(n));
        }
    }
    for (lane, since, len, what) in shadows {
        // The arrangement's own clips leave this lane: the launcher owns it while its clip plays.
        arr.clips.retain(|c| c.track != lane);
        let source = match what {
            Lane::Pattern(no, pat) => {
                p.patterns.push(pat);
                ClipSource::Pattern(no)
            }
            Lane::Audio(ch) => ClipSource::Channel(ch),
        };
        // From the tick it was launched at, one clip per loop, over the window.
        let mut at = since % w;
        while at < w {
            let length = (w - at).min(len) as u32;
            let mut clip = Clip::new(at as u32, length, lane, source);
            clip.id = os_model::Id::NONE;
            if let ClipSource::Channel(_) = source {
                // Gain 1.0, as `playlist::place` gives a new audio clip (spec: flp/events/playlist.md, byte 52).
                clip.set_gain(1.0);
            }
            arr.clips.push(clip);
            at += len;
        }
    }
    arr.clips.sort_by_key(|c| c.position);
    Some(p)
}

/// The project a launcher clip's notes start relative to (`PlayFrom::LegatoProject`): the transport's position.

/// Runs a launcher action.
pub fn run(app: &mut App, act: Act) {
    // Anything but opening a menu, or picking one of its commands, closes the open one.
    if !matches!(
        act,
        Act::Menu(..)
            | Act::ColorClip(..)
            | Act::ColorScene(..)
            | Act::ColorTrack(..)
            | Act::Set(..)
            | Act::CycleLaunch(..)
            | Act::CycleNext(..)
            | Act::CycleNextTiming(..)
            | Act::SceneOverride(_)
    ) {
        app.state.launcher.menu = None;
    }
    match act {
        Act::Launch(track, scene) => {
            // A second click on the same slot opens its clip in the piano roll; the first one launches it, as Bitwig's
            // press does.
            let now = std::time::Instant::now();
            let double = app
                .launcher_click
                .is_some_and(|((t, sc), at)| t == track && sc == scene && at.elapsed().as_millis() < 450);
            app.launcher_click = if double { None } else { Some(((track, scene), now)) };
            if double {
                // An empty slot gets its clip from the double click itself (Bitwig's way of making one), then opens
                // in the piano roll to be filled.
                if clip(&app.state, track, scene).is_none() {
                    make_clip(app, track, scene);
                }
                if open(app, track, scene) {
                    return;
                }
            }
            app.state.launcher.pressed = Some((track, scene));
            launch(app, track, scene)
        }
        Act::LaunchAlt(track, scene) => launch_alt(app, track, scene),
        Act::Release(track, scene) => release(app, track, scene),
        Act::Stop(track) => stop(app, track),
        Act::LaunchScene(scene) => launch_scene(app, scene),
        Act::StopAll => stop_all(app),
        Act::Set(track, scene) => {
            let current = app.state.pattern;
            let next = (clip(&app.state, track, scene).map(|c| c.pattern) != Some(current)).then_some(current);
            set(app, track, scene, next);
        }
        Act::SelectTrack(track) => {
            // The row's lane is selected in the arrangement; Shift+click extends the selection from the last track
            // clicked (the playlist's own Shift+click range), so several rows are highlighted together.
            if let Some(lane) = lane_of(&app.state, track) {
                crate::playlist::select_track(&mut app.state, lane, app.shift);
            }
            app.invalidate();
        }
        Act::Menu(target, at) => {
            app.state.launcher.menu = Some(Menu { target, at });
            app.invalidate();
        }
        Act::MenuClose => {
            app.state.launcher.menu = None;
            app.invalidate();
        }
        Act::Clear(track, scene) => set(app, track, scene, None),
        Act::Record(track, scene) => record(app, track, scene),
        Act::RenameClip(track, scene) => rename(app, crate::playlist::EditTarget::LauncherClip(track, scene)),
        Act::RenameScene(scene) => rename(app, crate::playlist::EditTarget::LauncherScene(scene)),
        Act::RenameTrack(track) => {
            if let Some(lane) = lane_of(&app.state, track) {
                rename(app, crate::playlist::EditTarget::TrackName(lane));
            }
        }
        Act::ColorClip(track, scene, color) => {
            if let Some(sl) = app.state.launcher.clips.get_mut(&(track, scene)) {
                sl.color = color;
            }
            app.invalidate();
        }
        Act::ColorScene(scene, color) => {
            let scenes = &mut app.state.launcher.scenes;
            while scenes.len() <= scene {
                scenes.push(Scene::default());
            }
            scenes[scene].color = color;
            normalize(&mut app.state);
            app.invalidate();
        }
        Act::ColorTrack(track, color) => {
            if let Some(lane) = lane_of(&app.state, track) {
                crate::playlist::set_track_color(&mut app.state, lane, color);
            }
            app.invalidate();
        }
        Act::CycleLaunch(track, scene, which) => {
            if let Some(sl) = app.state.launcher.clips.get_mut(&(track, scene)) {
                match which {
                    0 => sl.main.q = sl.main.q.next(),
                    1 => sl.main.play = sl.main.play.next(),
                    2 => sl.main.release = sl.main.release.next(),
                    3 => sl.main.q_to_loop = !sl.main.q_to_loop,
                    4 => sl.alt.q = sl.alt.q.next(),
                    5 => sl.alt.play = sl.alt.play.next(),
                    _ => sl.alt.release = sl.alt.release.next(),
                }
            }
            app.invalidate();
        }
        Act::CycleNext(track, scene) => {
            if let Some(sl) = app.state.launcher.clips.get_mut(&(track, scene)) {
                sl.next = sl.next.next();
            }
            app.invalidate();
        }
        Act::CycleNextTiming(track, scene) => {
            if let Some(sl) = app.state.launcher.clips.get_mut(&(track, scene)) {
                let bar = u64::from(app.state.project.bar_ticks().max(1));
                sl.next_timing = match sl.next_timing {
                    NextTiming::Loops(n) if n < 8 => NextTiming::Loops(n * 2),
                    NextTiming::Loops(_) => NextTiming::After(bar),
                    NextTiming::After(t) if t < bar * 16 => NextTiming::After(t * 2),
                    NextTiming::After(_) => NextTiming::Loops(1),
                };
            }
            app.invalidate();
        }
        Act::SceneOverride(scene) => {
            let scenes = &mut app.state.launcher.scenes;
            while scenes.len() <= scene {
                scenes.push(Scene::default());
            }
            scenes[scene].override_launch = !scenes[scene].override_launch;
            app.invalidate();
        }
    }
}

/// Opens the inline name box for `target`, where the menu that asked for it was opened.
fn rename(app: &mut App, target: crate::playlist::EditTarget) {
    let at = app.state.launcher.menu.map_or(app.mouse, |m| m.at);
    crate::playlist::start_edit(&mut app.state, target, at);
    app.invalidate();
}

/// Opens (track, scene)'s clip in the piano roll: its pattern becomes the current one and the window comes up. False
/// when the slot is empty.
/// Makes an empty clip in a cell — a new pattern of its own — as a double click on an empty slot does, so the grid
/// makes clips without the menu.
pub fn make_clip(app: &mut App, track: u32, scene: usize) {
    let no = next_pattern(&app.state);
    if !app.state.project.patterns.iter().any(|p| p.number == no) {
        app.state.project.patterns.push(os_model::Pattern { number: no, ..Default::default() });
    }
    app.state.launcher.clips.insert((track, scene), Slot { pattern: no, ..Default::default() });
    app.invalidate();
}

/// Drops the sample at `path` into a cell (`App::drop_file`): an audio channel playing it (`playlist::audio_channel`)
/// and a clip in the cell that plays it, looped at the sample's own length. Returns what to tell the user.
pub fn add_audio(app: &mut App, path: &str, track: u32, scene: usize) -> String {
    let Some(ch) = crate::playlist::audio_channel(&mut app.state, path) else {
        return format!("Cannot read {path} as audio");
    };
    let src = ClipSource::Channel(ch);
    let name = crate::playlist::source_name(&app.state, src);
    app.state.launcher.clips.insert((track, scene), Slot { pattern: 0, audio: Some(ch), name: Some(name.clone()), ..Default::default() });
    // The cell may have been the one playing: its track plays the sample now.
    if app.state.launcher.playing.contains_key(&track) {
        let p = crate::rack::play_project(&app.state);
        app.transport.update(&p);
    }
    app.state.piano.dirty = true;
    app.invalidate();
    format!("{name} in the launcher cell")
}

pub fn open(app: &mut App, track: u32, scene: usize) -> bool {
    // An audio clip has no pattern to edit (it plays on as it was).
    let Some(slot) = clip(&app.state, track, scene).filter(|sl| sl.pattern > 0) else { return false };
    app.state.launcher.pending.remove(&track);
    app.state.pattern = slot.pattern;
    app.state.project.current_pattern = Some(slot.pattern);
    app.state.piano.dirty = true;
    app.state.raise(crate::app::Win::PianoRoll);
    app.invalidate();
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::{Key, NamedKey};

    fn app() -> App {
        let mut app = App::new(None);
        app.state.project = crate::app::test_project();
        app.state.project.linear_mode = true;
        use crate::{playlist::Cmd, track_links::Mode};
        crate::playlist::run(&mut app, Cmd::TrackMode(0, Mode::LinearMidi));
        crate::playlist::run(&mut app, Cmd::TrackMode(1, Mode::LinearMidi));
        app
    }

    /// A click on a track row selects its lane in the arrangement; with Shift held it selects every lane between the
    /// row clicked last and this one, the same range the arrangement's headers highlight.
    #[test]
    fn shift_click_a_track_row_selects_a_range_of_tracks() {
        let mut app = app();
        let cols = columns(&app.state);
        assert!(cols.len() >= 2, "two tracks to range over");
        let (a, b) = (cols[0].clone(), cols[1].clone());
        assert_ne!(a.lane, b.lane, "two lanes to range over");
        run(&mut app, Act::SelectTrack(a.id));
        assert_eq!(app.state.playlist.track, a.lane);
        assert!(!crate::playlist::track_selected(&app.state, b.lane), "one row selected alone");
        app.shift = true;
        run(&mut app, Act::SelectTrack(b.id));
        assert_eq!(app.state.playlist.track, b.lane, "the row clicked is the active track");
        assert!(crate::playlist::track_selected(&app.state, a.lane), "the anchor's row stays selected");
        assert!(crate::playlist::track_selected(&app.state, b.lane), "and so does the one clicked");
        // Without Shift the range collapses to the row clicked.
        app.shift = false;
        run(&mut app, Act::SelectTrack(a.id));
        assert!(!crate::playlist::track_selected(&app.state, b.lane));
    }

    /// Gives `pattern` a one-step note on `channel` (so a clip of it has something to play).
    fn note(app: &mut App, pattern: u16, channel: u16) {
        if !app.state.project.patterns.iter().any(|p| p.number == pattern) {
            app.state.project.patterns.push(os_model::Pattern { number: pattern, ..Default::default() });
        }
        let p = app.state.project.patterns.iter_mut().find(|p| p.number == pattern).unwrap();
        p.notes.push(os_model::Note {
            position: 0,
            length: 24,
            channel,
            key: 36,
            fine_pitch: 120,
            release: 64,
            pan: 64,
            velocity: 100,
            mod_x: 128,
            mod_y: 128,
            flags: 0,
            group: 0,
            unk14: 0,
            repeat: 0,
        });
    }

    /// A launched clip puts its pattern on the air: the track's column lights up for that scene, the pattern becomes
    /// the current one and the launcher's own playback takes over.
    #[test]
    fn launching_a_slot_plays_its_pattern() {
        let mut app = app();
        let col = columns(&app.state).first().cloned().expect("a track");
        set(&mut app, col.id, 1, Some(3));
        launch(&mut app, col.id, 1);
        assert_eq!(playing(&app.state, col.id), Some(1));
        assert_eq!(app.state.pattern, 3);
        assert_eq!(app.state.project.current_pattern, Some(3));
        assert!(!app.state.song_mode, "the launcher plays the clips");
        assert!(app.transport.playing(), "the transport started");
    }

    /// An empty slot stops its track, and Stop / Stop all clear the columns.
    #[test]
    fn empty_slots_and_stops_clear_the_track() {
        let mut app = app();
        let cols = columns(&app.state);
        let (a, b) = (cols[0].id, cols[1].id);
        set(&mut app, a, 0, Some(1));
        set(&mut app, b, 0, Some(2));
        launch(&mut app, a, 0);
        launch(&mut app, b, 0);
        assert_eq!(playing(&app.state, a), Some(0));
        launch(&mut app, a, 5);
        assert_eq!(playing(&app.state, a), None, "an empty slot stops the track");
        stop(&mut app, b);
        assert!(app.state.launcher.playing.is_empty());
        launch(&mut app, b, 0);
        stop_all(&mut app);
        assert!(app.state.launcher.playing.is_empty());
    }

    /// Every launch waits for the next line of its own quantization — 1/16, 1/8, a beat, a bar, 2 or 8 bars — and
    /// then goes on the air.
    #[test]
    fn a_launch_waits_for_its_quantization_line() {
        let mut app = app();
        app.act(Action::Play);
        assert!(app.transport.playing(), "the transport runs");
        let id = columns(&app.state)[0].id;
        set(&mut app, id, 1, Some(3));
        let bar = u64::from(app.state.project.bar_ticks());
        for (q, want) in [
            (LaunchQ::Off, 0),
            (LaunchQ::Quarter, bar / 4),
            (LaunchQ::Bar, bar),
            (LaunchQ::Bars2, bar * 2),
            (LaunchQ::Bars8, bar * 8),
        ] {
            app.state.launcher.clips.get_mut(&(id, 1)).unwrap().main.q = q;
            launch(&mut app, id, 1);
            assert_eq!(pending(&app.state, id), Some(1));
            let queued = app.state.launcher.pending[&id];
            assert_eq!(queued.at, want, "{q:?} waits for its own line");
            assert!(!advance_at(&mut app, queued.at as f64 - 1.0, true) || q == LaunchQ::Off);
            if q != LaunchQ::Off {
                assert!(advance_at(&mut app, queued.at as f64, true));
                assert_eq!(playing(&app.state, id), Some(1));
                stop(&mut app, id);
            }
        }
    }

    /// A launch while the transport runs waits for the next bar line (the default quantization), then goes on the
    /// air; before the line the pattern is untouched, so nothing of it is heard early.
    #[test]
    fn a_launch_while_playing_waits_for_the_next_bar() {
        let mut app = app();
        app.act(Action::Play);
        let id = columns(&app.state)[0].id;
        set(&mut app, id, 1, Some(3));
        let before = app.state.project.current_pattern;
        launch(&mut app, id, 1);
        assert_eq!(playing(&app.state, id), None, "not on the air at once");
        assert_eq!(pending(&app.state, id), Some(1));
        let queued = app.state.launcher.pending[&id];
        assert_eq!(queued.at, u64::from(app.state.project.bar_ticks()), "the next bar line");
        assert_eq!(app.state.project.current_pattern, before, "the pattern waits too");
        assert!(!advance_at(&mut app, queued.at as f64 - 1.0, true));
        assert!(advance_at(&mut app, queued.at as f64, true));
        assert_eq!(playing(&app.state, id), Some(1));
        assert_eq!(app.state.pattern, 3);
        assert_eq!(app.state.project.current_pattern, Some(3));
        assert!(app.state.launcher.pending.is_empty());
    }

    /// A launch while the transport is stopped starts at once: there is no grid line to wait for.
    #[test]
    fn a_launch_while_stopped_starts_at_once() {
        let mut app = app();
        let id = columns(&app.state)[0].id;
        set(&mut app, id, 2, Some(2));
        launch(&mut app, id, 2);
        assert_eq!(playing(&app.state, id), Some(2));
        assert_eq!(app.state.pattern, 2);
        assert!(app.state.launcher.pending.is_empty());
    }

    /// Stop drops a queued launch, and Stop all drops every one of them.
    #[test]
    fn a_stop_drops_a_queued_launch() {
        let mut app = app();
        app.act(Action::Play);
        let cols = columns(&app.state);
        let (a, b) = (cols[0].id, cols[1].id);
        set(&mut app, a, 1, Some(2));
        set(&mut app, b, 1, Some(2));
        launch(&mut app, a, 1);
        launch(&mut app, b, 1);
        assert_eq!((pending(&app.state, a), pending(&app.state, b)), (Some(1), Some(1)));
        stop(&mut app, a);
        assert_eq!(pending(&app.state, a), None, "its queue went with it");
        assert_eq!(pending(&app.state, b), Some(1), "the other track still waits");
        stop_all(&mut app);
        assert!(app.state.launcher.pending.is_empty());
    }

    /// A scene launch queues every clip of the scene for the same line, and the scene's override setting passes its
    /// quantization on to the clips it starts.
    #[test]
    fn a_scene_launch_queues_its_clips_for_the_bar() {
        let mut app = app();
        app.act(Action::Play);
        let cols = columns(&app.state);
        let (a, b) = (cols[0].id, cols[1].id);
        set(&mut app, a, 1, Some(2));
        set(&mut app, b, 1, Some(2));
        launch_scene(&mut app, 1);
        assert_eq!((pending(&app.state, a), pending(&app.state, b)), (Some(1), Some(1)));
        assert_eq!(playing(&app.state, a), None, "nothing yet");
        let bar = f64::from(app.state.project.bar_ticks());
        assert!(advance_at(&mut app, bar, true));
        assert_eq!((playing(&app.state, a), playing(&app.state, b)), (Some(1), Some(1)));
        assert_eq!(app.state.pattern, 2);
        // The scene's own override wins over the clips' settings.
        stop_all(&mut app);
        run(&mut app, Act::SceneOverride(1));
        let mut sc = Scene::default();
        sc.override_launch = true;
        sc.main.q = LaunchQ::Bars4;
        app.state.launcher.scenes[1] = sc;
        assert_eq!(settings(&app.state, a, 1, false).q, LaunchQ::Bars4);
    }

    /// A looping window that wrapped past a launch's line lands it (the transport never reports a tick at its end).
    #[test]
    fn a_launch_lands_when_the_window_wraps() {
        let mut app = app();
        app.act(Action::Play);
        let id = columns(&app.state)[0].id;
        set(&mut app, id, 1, Some(3));
        launch(&mut app, id, 1);
        assert!(!advance_at(&mut app, 380.0, true), "still the bar before it");
        assert!(advance_at(&mut app, 4.0, true), "the wrap is the line");
        assert_eq!(playing(&app.state, id), Some(1));
        assert_eq!(app.state.pattern, 3);
    }

    /// Clips, scenes and tracks take names and colours of their own: the launcher's menu commands, with a track's
    /// name and colour being the arrangement's own (the launcher edits the same row).
    #[test]
    fn clips_scenes_and_tracks_take_names_and_colours() {
        let mut app = app();
        let id = columns(&app.state)[0].id;
        set(&mut app, id, 1, Some(3));
        run(&mut app, Act::RenameClip(id, 1));
        assert!(app.state.playlist.edit.is_some(), "the inline box opened");
        app.state.playlist.edit.as_mut().unwrap().text = "Verse".into();
        crate::playlist::commit_edit(&mut app.state);
        assert_eq!(clip_name(&app.state, id, 1), "Verse");
        run(&mut app, Act::RenameClip(id, 1));
        app.state.playlist.edit.as_mut().unwrap().text = String::new();
        crate::playlist::commit_edit(&mut app.state);
        assert_eq!(clip_name(&app.state, id, 1), crate::playlist::source_name(&app.state, os_model::ClipSource::Pattern(3)));
        let own = palette(5);
        run(&mut app, Act::ColorClip(id, 1, Some(own)));
        assert_eq!(clip_color(&app.state, id, 1), Some(own));
        let lane = lane_of(&app.state, id).expect("the track's lane");
        let track = palette(2);
        run(&mut app, Act::ColorTrack(id, Some(track)));
        assert_eq!(crate::playlist::track_color(&app.state, lane), Some(track));
        run(&mut app, Act::ColorClip(id, 1, None));
        assert_eq!(clip_color(&app.state, id, 1), Some(track), "the track's colour shows through");
        run(&mut app, Act::RenameScene(4));
        app.state.playlist.edit.as_mut().unwrap().text = "Chorus".into();
        crate::playlist::commit_edit(&mut app.state);
        assert_eq!(scene_name(&app.state, 4), "Chorus");
        run(&mut app, Act::ColorScene(4, Some(track)));
        assert_eq!(scene_color(&app.state, 4), Some(track));
        run(&mut app, Act::RenameScene(4));
        app.state.playlist.edit.as_mut().unwrap().text = String::new();
        crate::playlist::commit_edit(&mut app.state);
        assert_eq!(scene_name(&app.state, 4), "Scene 5");
        run(&mut app, Act::RenameTrack(id));
        app.state.playlist.edit.as_mut().unwrap().text = "Drums".into();
        crate::playlist::commit_edit(&mut app.state);
        assert_eq!(crate::playlist::track_name(&app.state, lane), "Drums");
    }

    /// A right-click opens the target's menu, and its commands act on that clip, scene or track; the clip menu
    /// carries Bitwig's launch parameters and a click walks each to its next value.
    #[test]
    fn a_right_click_opens_the_clips_menu() {
        let mut app = app();
        let id = columns(&app.state)[0].id;
        set(&mut app, id, 0, Some(2));
        run(&mut app, Act::Menu(MenuTarget::Clip(id, 0), (10.0, 10.0)));
        assert_eq!(app.state.launcher.menu, Some(Menu { target: MenuTarget::Clip(id, 0), at: (10.0, 10.0) }));
        let labels: Vec<&str> = menu_items(&app.state, MenuTarget::Clip(id, 0)).into_iter().map(|(l, _, _)| l).collect();
        assert_eq!(
            labels,
            [
                "Set current pattern",
                "Record into this clip",
                "Quantize",
                "Play mode",
                "On release",
                "Next action",
                "Next action after",
                "Q to loop",
                "ALT quantize",
                "ALT play mode",
                "ALT on release",
                "Rename...",
                "Clear clip"
            ]
        );
        let q = |s: &State| s.launcher.clips[&(id, 0)].main.q;
        let before = q(&app.state);
        run(&mut app, Act::CycleLaunch(id, 0, 0));
        assert_eq!(q(&app.state), before.next());
        run(&mut app, Act::CycleNext(id, 0));
        assert_eq!(app.state.launcher.clips[&(id, 0)].next, NextAction::Stop);
        run(&mut app, Act::CycleNextTiming(id, 0));
        assert_eq!(app.state.launcher.clips[&(id, 0)].next_timing, NextTiming::Loops(2));
        run(&mut app, Act::Clear(id, 0));
        assert_eq!(clip(&app.state, id, 0), None, "the clip went");
    }

    /// A right-click sets the current pattern into a slot, and clears it when it already holds that pattern.
    #[test]
    fn a_slot_toggles_the_current_pattern() {
        let mut app = app();
        let id = columns(&app.state)[0].id;
        app.state.pattern = 4;
        run(&mut app, Act::Set(id, 2));
        assert_eq!(clip(&app.state, id, 2).map(|c| c.pattern), Some(4));
        run(&mut app, Act::Set(id, 2));
        assert_eq!(clip(&app.state, id, 2), None);
    }

    /// **The launcher's own feature**: each track plays its own clip, so three tracks' clips run together — the
    /// transport's project is the shadow of the launcher (`shadow`), one lane per playing track, each looping at its
    /// own length.
    #[test]
    fn every_track_plays_its_own_clip() {
        let mut app = app();
        let cols = columns(&app.state);
        assert!(cols.len() >= 2);
        let ch0 = channel_of(&app.state, cols[0].id).expect("a channel");
        let ch1 = channel_of(&app.state, cols[1].id).expect("a channel");
        note(&mut app, 1, ch0);
        note(&mut app, 2, ch1);
        set(&mut app, cols[0].id, 0, Some(1));
        set(&mut app, cols[1].id, 0, Some(2));
        launch(&mut app, cols[0].id, 0);
        // The first clip starts the transport; the second waits for the bar line it was launched on.
        launch(&mut app, cols[1].id, 0);
        assert_eq!(playing(&app.state, cols[1].id), None, "queued for the next bar");
        let bar = f64::from(app.state.project.bar_ticks());
        assert!(advance_at(&mut app, bar, true));
        assert_eq!(playing(&app.state, cols[0].id), Some(0));
        assert_eq!(playing(&app.state, cols[1].id), Some(0));
        let base = app.state.project.clone();
        let p = shadow(&app.state, &base).expect("the launcher's project");
        let arr = p.arrangement().expect("an arrangement");
        let lanes: Vec<u16> = arr.clips.iter().map(|c| c.track).collect();
        for c in &cols {
            if let Some(lane) = lane_of(&app.state, c.id) {
                assert!(lanes.contains(&lane), "track {} plays on its own lane {lane}", c.name);
            }
        }
        // Each clip's pattern has a shadow of its own, holding only that track's channel's notes.
        let ch = channel_of(&app.state, cols[0].id).expect("a channel");
        let shadow_no = match arr.clips.iter().find(|c| c.track == lane_of(&app.state, cols[0].id).unwrap()).unwrap().source {
            ClipSource::Pattern(n) => n,
            _ => panic!("a pattern clip"),
        };
        let pat = p.patterns.iter().find(|x| x.number == shadow_no).unwrap();
        assert!(pat.notes.iter().all(|n| n.channel == ch), "only its track's notes");
        // The project's own patterns are untouched.
        assert!(app.state.project.patterns.iter().all(|x| x.number != shadow_no));
    }

    /// The window is the common multiple of the clips' lengths (so the transport loops seamlessly), capped.
    #[test]
    fn the_window_is_the_common_multiple_of_the_clips() {
        let mut app = app();
        let cols = columns(&app.state);
        let bar = u64::from(app.state.project.bar_ticks());
        set(&mut app, cols[0].id, 0, Some(1));
        set(&mut app, cols[1].id, 0, Some(2));
        launch(&mut app, cols[0].id, 0);
        let one = window(&app.state);
        launch(&mut app, cols[1].id, 0);
        let two = window(&app.state);
        assert_eq!(one, lcm(bar, clip_len(&app.state, 1)));
        assert_eq!(two, lcm(one, clip_len(&app.state, 2)));
        assert!(two <= bar * u64::from(MAX_WINDOW_BARS));
    }

    /// Next actions: a clip's function runs when its timing is up — after its loops, or after the ticks it was
    /// given — and "play next" moves the track on to the next clip it has.
    #[test]
    fn next_actions_move_a_track_on() {
        let mut app = app();
        let cols = columns(&app.state);
        let id = cols[0].id;
        set(&mut app, id, 0, Some(1));
        set(&mut app, id, 1, Some(2));
        launch(&mut app, id, 0);
        let len = clip_len(&app.state, 1);
        app.state.launcher.clips.get_mut(&(id, 0)).unwrap().next = NextAction::PlayNext;
        app.state.launcher.clips.get_mut(&(id, 0)).unwrap().next_timing = NextTiming::Loops(1);
        let since = app.state.launcher.playing[&id].since;
        assert!(!next_actions(&mut app, (since + len).saturating_sub(1) as f64), "not yet");
        assert!(next_actions(&mut app, (since + len) as f64), "its loop is up");
        assert_eq!(playing(&app.state, id), Some(1), "the next clip of the track");
        assert_eq!(app.state.pattern, 2);
        // "Return to last clip" comes back.
        app.state.launcher.clips.get_mut(&(id, 1)).unwrap().next = NextAction::ReturnToLastClip;
        next_actions(&mut app, 1e9);
        assert_eq!(playing(&app.state, id), Some(0));
        // A timed next action fires on the tick it names.
        app.state.launcher.clips.get_mut(&(id, 0)).unwrap().next = NextAction::Stop;
        app.state.launcher.clips.get_mut(&(id, 0)).unwrap().next_timing = NextTiming::After(len);
        let since = app.state.launcher.playing[&id].since;
        assert!(next_actions(&mut app, (since + len) as f64));
        assert_eq!(playing(&app.state, id), None, "it stopped");
    }

    /// Clips run in blocks: "first in next block" moves to the first clip of the next run of neighbouring clips.
    #[test]
    fn next_actions_work_on_clip_blocks() {
        let mut app = app();
        let id = columns(&app.state)[0].id;
        for sc in [0, 1, 3, 4] {
            set(&mut app, id, sc, Some(1));
        }
        assert_eq!(blocks(&app.state, id), vec![vec![0, 1], vec![3, 4]]);
        launch(&mut app, id, 0);
        app.state.launcher.clips.get_mut(&(id, 0)).unwrap().next = NextAction::FirstInNextBlock;
        app.state.launcher.clips.get_mut(&(id, 0)).unwrap().next_timing = NextTiming::Loops(1);
        let since = app.state.launcher.playing[&id].since;
        let len = clip_len(&app.state, 1);
        next_actions(&mut app, (since + len) as f64);
        assert_eq!(playing(&app.state, id), Some(3), "the next block's first clip");
        // "Other in block" never picks the clip playing, and stays inside the clip's block.
        for _ in 0..8 {
            let now = playing(&app.state, id).unwrap();
            follow(&mut app, id, now, Follow::InBlock(BlockRule::Other));
            let after = playing(&app.state, id).unwrap();
            assert_ne!(after, now, "it always moves on");
            assert!(after == 3 || after == 4, "inside the block, not {after}");
        }
    }

    /// Releasing the press runs the clip's "on Release": Stop stops it, Return brings the clip it replaced back.
    #[test]
    fn a_release_does_what_on_release_says() {
        let mut app = app();
        let id = columns(&app.state)[0].id;
        set(&mut app, id, 0, Some(1));
        set(&mut app, id, 1, Some(2));
        launch(&mut app, id, 0);
        app.state.launcher.clips.get_mut(&(id, 0)).unwrap().main.release = OnRelease::Stop;
        app.launcher_click = None;
        run(&mut app, Act::Launch(id, 0));
        assert_eq!(app.state.launcher.pressed, Some((id, 0)), "the press is remembered");
        run(&mut app, Act::Release(id, 0));
        assert_eq!(playing(&app.state, id), None, "Stop");
        // The ALT launch settings are their own set.
        app.state.launcher.clips.get_mut(&(id, 1)).unwrap().alt.release = OnRelease::Stop;
        launch_alt(&mut app, id, 1);
        app.state.launcher.pressed = Some((id, 1));
        release(&mut app, id, 1);
        assert_eq!(playing(&app.state, id), None, "the ALT release");
    }

    /// Letting go of the mouse runs the clip's "on Release": with Stop, the click's own release stops the clip it
    /// just started (Continue, the default, leaves it playing).
    #[test]
    fn the_mouse_release_runs_on_release() {
        let Ok(mut c) = os_ui::Canvas::new(1920, 1080, 1.0) else { return };
        let mut app = app();
        app.state.z = vec![crate::app::Win::Playlist];
        app.key(&Key::Named(NamedKey::Tab), false, false);
        let id = columns(&app.state)[0].id;
        set(&mut app, id, 1, Some(2));
        app.paint(&mut c, None);
        let cell = app
            .hits
            .iter()
            .find(|h| h.action == Action::Launcher(Act::Launch(id, 1)))
            .expect("the slot is painted")
            .rect;
        let at = (cell.cx(), cell.cy());
        app.mouse_move(at.0, at.1);
        app.mouse_down();
        app.mouse_up();
        assert_eq!(playing(&app.state, id), Some(1), "Continue: it keeps playing");
        // With Stop the release takes it away again; the transport is stopped first so the press starts at once.
        stop(&mut app, id);
        app.transport.stop();
        app.state.launcher.clips.get_mut(&(id, 1)).unwrap().main.release = OnRelease::Stop;
        app.launcher_click = None;
        app.mouse_down();
        assert_eq!(playing(&app.state, id), Some(1), "the press launched it");
        app.mouse_up();
        assert_eq!(playing(&app.state, id), None, "the release stopped it");
    }

    /// Tab flips the Playlist window from the arrangement to the launcher and back. The launcher paints a slot per
    /// track and scene, scene launch buttons, and a click on a filled slot launches it.
    #[test]
    fn tab_flips_the_playlist_to_the_launcher() {
        let Ok(mut c) = os_ui::Canvas::new(1920, 1080, 1.0) else { return };
        let mut app = app();
        app.state.z = vec![crate::app::Win::Playlist];
        app.paint(&mut c, None);
        assert!(app.hits.iter().all(|h| !matches!(h.action, Action::Launcher(_))), "the arrangement first");
        app.key(&Key::Named(NamedKey::Tab), false, false);
        assert!(app.state.launcher_open, "Tab shows the launcher");
        app.paint(&mut c, None);
        let cols = columns(&app.state).len();
        let slots = app.hits.iter().filter(|h| matches!(h.action, Action::Launcher(Act::Launch(..)))).count();
        assert_eq!(slots, cols * SCENES, "a slot per track and scene ({cols} tracks)");
        let id0 = columns(&app.state)[0].id;
        let head = app.hits.iter().find(|h| h.action == Action::Launcher(Act::LaunchScene(0))).expect("the first scene").rect;
        let cell = app.hits.iter().find(|h| h.action == Action::Launcher(Act::Launch(id0, 0))).expect("its first cell").rect;
        assert!(head.bottom() <= cell.y, "the scene buttons head the grid: {head:?} {cell:?}");
        assert!(app.hits.iter().any(|h| h.action == Action::Launcher(Act::SelectTrack(id0))), "the track row's header");
        assert!(app.hits.iter().any(|h| h.action == Action::Launcher(Act::StopAll)), "stop all");

        let id = columns(&app.state)[0].id;
        set(&mut app, id, 2, Some(1));
        app.paint(&mut c, None);
        let hit = app
            .hits
            .iter()
            .find(|h| h.action == Action::Launcher(Act::Launch(id, 2)))
            .expect("the slot is painted");
        let at = (hit.rect.cx(), hit.rect.cy());
        app.mouse_move(at.0, at.1);
        app.mouse_down();
        assert_eq!(playing(&app.state, id), Some(2), "the click launched it");
        app.mouse_up();

        app.key(&Key::Named(NamedKey::Tab), false, false);
        assert!(!app.state.launcher_open);
    }

    /// A double click on a filled slot opens its clip in the piano roll (the first click launched it).
    #[test]
    fn a_double_click_opens_the_clip_in_the_piano_roll() {
        let Ok(mut c) = os_ui::Canvas::new(1920, 1080, 1.0) else { return };
        let mut app = app();
        app.state.z = vec![crate::app::Win::Playlist];
        app.key(&Key::Named(NamedKey::Tab), false, false);
        let id = columns(&app.state)[0].id;
        set(&mut app, id, 1, Some(5));
        app.paint(&mut c, None);
        let cell = app
            .hits
            .iter()
            .find(|h| h.action == Action::Launcher(Act::Launch(id, 1)))
            .expect("the slot is painted")
            .rect;
        let at = (cell.cx(), cell.cy());
        app.mouse_move(at.0, at.1);
        app.mouse_down();
        app.mouse_up();
        assert_eq!(playing(&app.state, id), Some(1), "one click launches");
        assert!(!app.state.shown(crate::app::Win::PianoRoll), "one click does not open the piano roll");
        app.mouse_down();
        app.mouse_up();
        assert!(app.state.shown(crate::app::Win::PianoRoll), "the second click opens it");
        assert_eq!(app.state.pattern, 5, "the clip's own pattern");
        assert_eq!(app.state.project.current_pattern, Some(5));
    }

    /// An empty slot gets its clip from a double click (not the menu), and opens to be filled.
    #[test]
    fn a_double_click_on_an_empty_slot_makes_its_clip() {
        let Ok(mut c) = os_ui::Canvas::new(1920, 1080, 1.0) else { return };
        let mut app = app();
        app.state.z = vec![crate::app::Win::Playlist];
        app.key(&Key::Named(NamedKey::Tab), false, false);
        let id = columns(&app.state)[0].id;
        let before = app.state.project.patterns.len();
        app.paint(&mut c, None);
        let cell = app
            .hits
            .iter()
            .find(|h| h.action == Action::Launcher(Act::Launch(id, 3)))
            .expect("the slot is painted")
            .rect;
        let at = (cell.cx(), cell.cy());
        app.mouse_move(at.0, at.1);
        app.mouse_down();
        app.mouse_up();
        assert!(clip(&app.state, id, 3).is_none(), "one click makes nothing");
        assert!(!app.state.shown(crate::app::Win::PianoRoll));
        app.mouse_down();
        app.mouse_up();
        let sl = clip(&app.state, id, 3).expect("the double click made the clip");
        assert_eq!(app.state.project.patterns.len(), before + 1, "a pattern of its own");
        assert!(sl.pattern > 0 && app.state.project.patterns.iter().any(|p| p.number == sl.pattern));
        assert!(app.state.shown(crate::app::Win::PianoRoll), "and it opens to be filled");
        assert_eq!(app.state.pattern, sl.pattern);
    }

    /// A sample dropped on a cell becomes an audio clip there: it plays from the cell's lane, looped at the sample's
    /// own length, and it is no pattern.
    #[test]
    fn a_sample_dropped_on_a_cell_plays_from_it() {
        let dir = std::env::temp_dir().join(format!("ls_launcher_audio_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Loop.wav");
        let tone = tesla::sample::Sample::from_channels(44100, vec![vec![0.25; 4410], vec![-0.25; 4410]], tesla::sample::Format::Int16);
        std::fs::write(&file, tesla::wav::write(&tone, tesla::wav::Depth::Own)).unwrap();
        let path = file.to_string_lossy().into_owned();

        let Ok(mut c) = os_ui::Canvas::new(1920, 1080, 1.0) else { return };
        let mut app = app();
        app.state.z = vec![crate::app::Win::Playlist];
        app.key(&Key::Named(NamedKey::Tab), false, false);
        let id = columns(&app.state)[0].id;
        let patterns = app.state.project.patterns.len();
        app.paint(&mut c, None);
        let cell = app
            .hits
            .iter()
            .find(|h| h.action == Action::Launcher(Act::Launch(id, 2)))
            .expect("the slot is painted")
            .rect;
        app.mouse_move(cell.cx(), cell.cy());
        app.drop_file(std::path::PathBuf::from(&path));
        let sl = clip(&app.state, id, 2).expect("the drop made a clip");
        let ch = sl.audio.expect("an audio clip");
        assert_eq!(sl.pattern, 0, "the sample is the clip, there is no pattern");
        assert_eq!(app.state.project.patterns.len(), patterns);
        assert_eq!(clip_name(&app.state, id, 2), "Loop", "named after the file");
        assert_eq!(slot_len(&app.state, &sl), u64::from(crate::playlist::source_len(&app.state, ClipSource::Channel(ch))));
        assert!(!open(&mut app, id, 2), "there is no pattern to edit");
        launch(&mut app, id, 2);
        assert_eq!(playing(&app.state, id), Some(2));
        let p = shadow(&app.state, &app.state.project).expect("the sample plays");
        let lane = lane_of(&app.state, id).unwrap();
        assert!(
            p.arrangements.iter().flat_map(|a| &a.clips).any(|c| c.track == lane && c.source == ClipSource::Channel(ch)),
            "the cell's lane holds the sample"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A cell records (`Act::Record`): arming one starts playback, and what is played lands in the cell's own clip —
    /// an empty cell gets a pattern and a clip of its own as the take opens, and the clip's length is the bars it
    /// captured, so it loops like any other clip. A cell that already holds a clip records into that clip's pattern.
    #[test]
    fn a_cell_records_what_the_armed_track_plays() {
        use crate::record::{Extra, Opt};
        use crate::step_edit::Src;
        crate::prefs::set_check(Opt::Notes.check(), true);
        crate::prefs::set_check(Opt::Precount.check(), false);
        crate::prefs::set_check(Opt::QuantizeStart.check(), false);
        crate::prefs::set_check(Opt::QuantizeEnd.check(), false);
        let mut app = app();
        let id = columns(&app.state)[0].id;
        let channel = channel_of(&app.state, id).expect("the track's channel");
        let bar = app.state.project.bar_ticks();
        run(&mut app, Act::Record(id, 2));
        assert!(recording(&app.state, id, 2) && app.state.rec.armed, "the cell is the take's target");
        assert!(app.transport.playing(), "playback starts");
        app.state.rec.count = None;
        // A note at the take's start, let go a beat later.
        app.transport.pretend_at(0.0, false);
        app.record_tick();
        // No channel given: the cell's track says which one (a clip plays its track's channel's notes).
        let src = Src::Midi(60);
        app.record_down_with(src, 60, 100, Extra::default());
        app.transport.pretend_at(96.0, false);
        app.record_tick();
        app.record_up_with(src, None);
        app.transport.stop();
        app.record_tick();
        let sl = clip(&app.state, id, 2).expect("the take made the cell's clip");
        let notes = app.state.project.pattern(sl.pattern).expect("its pattern").notes.clone();
        assert_eq!(notes.len(), 1, "the note landed in the cell");
        assert_eq!((notes[0].position, notes[0].length, notes[0].key), (0, 96, 60), "at the clip's start");
        assert_eq!(notes[0].channel, channel, "the take's track's channel, whatever is selected");
        assert_eq!(app.state.project.pattern_length(sl.pattern), bar, "one bar: whole bars past the note");
        assert!(!recording(&app.state, id, 2), "the take ended with the stop");
        assert_eq!(app.state.launcher.playing.get(&id), None, "the cell is not on the air yet");

        // A cell that already holds a clip records into that clip's own pattern (over it): no second pattern is made.
        let patterns = app.state.project.patterns.len();
        run(&mut app, Act::Record(id, 2));
        app.state.rec.count = None;
        app.transport.pretend_at(0.0, false);
        app.record_tick();
        app.record_down_with(src, 64, 100, Extra { channel: Some(channel), ..Default::default() });
        app.transport.pretend_at(96.0, false);
        app.record_tick();
        app.record_up_with(src, None);
        app.transport.stop();
        app.record_tick();
        assert_eq!(clip(&app.state, id, 2).map(|c| c.pattern), Some(sl.pattern), "the same clip");
        assert_eq!(app.state.project.patterns.len(), patterns, "no pattern was made for it");
        let notes = app.state.project.pattern(sl.pattern).expect("its pattern").notes.clone();
        assert_eq!(notes.len(), 1, "the new take replaced the old one: {notes:?}");
        assert_eq!(notes[0].key, 64);
    }

    /// The grid keeps a least column and row size and scrolls what does not fit: a narrow window scrolls its scene
    /// columns sideways and a set of many tracks scrolls its rows, so every cell is reachable at any window size —
    /// and a cell scrolled out of the grid answers nowhere, so it never takes a click meant for the track column.
    #[test]
    fn the_grid_scrolls_to_reach_the_tracks_and_scenes_that_do_not_fit() {
        use crate::views::launcher::{self as view, MIN_COL, MIN_ROW};
        // A window too narrow for eight scene columns at their least width, and with more tracks than rows fit.
        let Ok(mut c) = os_ui::Canvas::new(760, 520, 1.0) else { return };
        let mut app = app();
        app.state.z = vec![crate::app::Win::Playlist];
        app.key(&Key::Named(NamedKey::Tab), false, false);
        app.state.device_rack_collapsed = true;
        for _ in 0..20 {
            crate::playlist::run(&mut app, crate::playlist::Cmd::TrackMode(0, crate::track_links::Mode::LinearMidi));
        }
        let cols = columns(&app.state);
        assert!(cols.len() > 10, "enough tracks to outgrow the rows ({} tracks)", cols.len());
        // The screen is the canvas's size, set by the paint (`App::paint_region`).
        app.paint(&mut c, None);
        let client = view::client_rect(&app.state);
        let g = view::geom(&app.state, client);
        assert!(g.scene_w >= MIN_COL, "the scene columns keep their least width ({})", g.scene_w);
        assert!(g.row_h >= MIN_ROW, "the track rows keep their least height ({})", g.row_h);
        assert!(g.max_sx() > 0.0, "the scene columns overflow a window this narrow");
        assert!(g.max_sy() > 0.0, "the track rows overflow it too");

        let at = (client.cx(), g.head().bottom() + 40.0);
        let cell = |app: &App, id: u32, scene: usize| {
            app.hits
                .iter()
                .find(|h| h.action == Action::Launcher(Act::Launch(id, scene)))
                .map(|h| h.rect)
        };
        let (first, last) = (cols[0].id, cols[cols.len() - 1].id);
        assert!(cell(&app, first, 0).is_some(), "the first track shows");
        assert!(cell(&app, last, 0).is_none(), "the last track is below the window");
        assert!(
            cell(&app, first, SCENES - 1).is_none_or(|r| r.right() <= client.right() + 0.5),
            "nor is the last scene"
        );

        // The wheel scrolls the rows: the last track comes into view, and no cell answers over the track column.
        assert!(view::wheel(&mut app.state, at.0, at.1, 0.0, -40.0, false), "the grid takes the wheel");
        assert!(app.state.launcher.scroll_y > 0.0, "scrolled down");
        app.paint(&mut c, None);
        let row = cell(&app, last, 0).expect("scrolled to the last track");
        assert!(row.bottom() <= client.bottom() + 0.5, "inside the grid: {row:?} {client:?}");
        assert!(cell(&app, first, 0).is_none(), "the first track scrolled away");
        for h in app.hits.iter().filter(|h| matches!(h.action, Action::Launcher(Act::Launch(..)))) {
            assert!(h.rect.x >= client.x + g.label_w - 0.5, "no cell over the track column: {:?}", h.rect);
        }

        // Sideways (a sideways wheel) the scene columns follow, up to their end.
        assert!(view::wheel(&mut app.state, at.0, at.1, -40.0, 0.0, false));
        assert_eq!(app.state.launcher.scroll_x, g.max_sx(), "clamped at the last column");
        app.paint(&mut c, None);
        let last_scene = cell(&app, last, SCENES - 1).expect("the last scene scrolled in");
        assert!(last_scene.x >= client.x + g.label_w - 0.5 && last_scene.right() <= client.right() + 0.5, "{last_scene:?}");
        // And back: the first column stops at the left of the cells.
        assert!(view::wheel(&mut app.state, at.0, at.1, 40.0, 0.0, false));
        assert_eq!(app.state.launcher.scroll_x, 0.0, "clamped at the first column");
        assert!(!view::wheel(&mut app.state, client.x - 5.0, client.y + 40.0, 0.0, -4.0, false), "off the grid the wheel is not the launcher's");

        // Through the real dispatch (`App::wheel` → `wheel_act`), not just the helper.
        app.state.launcher.scroll_x = 0.0;
        app.state.launcher.scroll_y = 0.0;
        app.mouse_move(at.0, at.1);
        app.wheel(0.0, -1.0, false, false);
        assert!(app.state.launcher.scroll_y > 0.0, "the wheel over the grid scrolls its rows");
        app.wheel(0.0, -1.0, true, false);
        assert!(app.state.launcher.scroll_x > 0.0, "Shift+wheel scrolls the scene columns");
    }

    /// The columns follow the mixer: one per track that has a lane, ordered by its mixer strip.
    #[test]
    fn columns_follow_the_mixer_order() {
        let mut app = app();
        let cols = columns(&app.state);
        assert!(!cols.is_empty());
        let strips: Vec<u16> = cols
            .iter()
            .map(|c| app.state.project.tracks.iter().find(|t| t.id.0 == c.id).map_or(0, |t| t.mixer_strip))
            .collect();
        assert!(strips.windows(2).all(|w| w[0] <= w[1]), "{strips:?}");
    }
}
