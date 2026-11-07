// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The clip launcher (Bitwig-style; the plan for the rest is docs/clip-launcher.md): scenes down the left edge, one
//! column per track in the mixer's order, a clip per cell. A clip plays one of the project's patterns, so a launched
//! cell puts that pattern on the air through the channel the track already has — the device rack, the mixer and the
//! piano roll all see the same notes.
//!
//! Tab flips the Playlist window between the arrangement and the launcher ([`crate::app::State::launcher_open`]).
//!
//! What is *not* here yet (docs/clip-launcher.md §2): independent playback per track. The engine plays one pattern
//! at a time, so launching clip B on the drum track while clip A loops on the bass does not stack them; the launcher
//! records what each track is playing and starts the pattern, and the stacking is the engine work the plan stages.

use std::collections::BTreeMap;

use crate::app::{Action, App, State};

/// How many scenes a launcher has.
pub const SCENES: usize = 8;

/// One clip: the pattern its slot plays (1-based, as `Project::current_pattern` is).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub pattern: u16,
}

/// The launcher's grid: the clips by (track id, scene), and the scene each track is playing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Launcher {
    pub clips: BTreeMap<(u32, usize), Slot>,
    /// By the track's id: the scene whose clip it plays.
    pub playing: BTreeMap<u32, usize>,
    /// The scenes' names by index; an empty entry reads "Scene N".
    pub scenes: Vec<String>,
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
}

/// The launcher's actions (its cells, scene buttons and track headers).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Act {
    /// Launch the clip at (track id, scene); an empty slot stops the track.
    Launch(u32, usize),
    /// Stop one track's clip.
    Stop(u32),
    /// Launch every clip of a scene.
    LaunchScene(usize),
    /// Stop every track.
    StopAll,
    /// Put the current pattern in the slot, or clear it when it already holds that pattern (right-click).
    Set(u32, usize),
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
                },
            )
        })
        .collect();
    v.sort_by_key(|(strip, _)| *strip);
    v.into_iter().map(|(_, c)| c).collect()
}

/// The clip in (track, scene), when the slot holds one.
/// Whether a MIDI track's instrument slot is empty: no channel plays it (the same test the device rack's
/// `device_rack::DeviceKind::NoInstrument` tile makes, so the two never disagree).
fn instrument_missing(s: &State, t: &os_model::Track) -> bool {
    t.kind == os_model::TrackKind::Midi
        && t.channel
            .and_then(|ch| s.project.channels.iter().position(|c| c.index == ch))
            .is_none()
}

pub fn clip(s: &State, track: u32, scene: usize) -> Option<Slot> {
    s.launcher.clips.get(&(track, scene)).copied()
}

/// The scene a track is playing, when it is playing one.
pub fn playing(s: &State, track: u32) -> Option<usize> {
    s.launcher.playing.get(&track).copied()
}

/// The scene's name ("Scene 3" when it has none).
pub fn scene_name(s: &State, scene: usize) -> String {
    s.launcher.scenes.get(scene).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| format!("Scene {}", scene + 1))
}

/// Puts `pattern` in (track, scene), or empties the slot with `None`.
pub fn set(app: &mut App, track: u32, scene: usize, pattern: Option<u16>) {
    match pattern {
        Some(p) => {
            app.state.launcher.clips.insert((track, scene), Slot { pattern: p });
        }
        None => {
            app.state.launcher.clips.remove(&(track, scene));
        }
    }
    // A cleared or replaced slot cannot keep playing.
    if playing(&app.state, track) == Some(scene) && clip(&app.state, track, scene).is_none() {
        app.state.launcher.playing.remove(&track);
    }
    app.invalidate();
}

/// Launches (track, scene): the track's column lights up for that scene and the clip's pattern starts. An empty slot
/// stops the track (Bitwig's empty-slot stop).
pub fn launch(app: &mut App, track: u32, scene: usize) {
    let Some(slot) = clip(&app.state, track, scene) else {
        stop(app, track);
        return;
    };
    app.state.launcher.playing.insert(track, scene);
    play_pattern(app, slot.pattern);
}

/// Stops a track's clip.
pub fn stop(app: &mut App, track: u32) {
    app.state.launcher.playing.remove(&track);
    app.invalidate();
}

/// Launches every clip of a scene (one pattern, so the first clip's pattern is what the engine plays).
pub fn launch_scene(app: &mut App, scene: usize) {
    let mut first: Option<u16> = None;
    for c in columns(&app.state) {
        if let Some(slot) = clip(&app.state, c.id, scene) {
            app.state.launcher.playing.insert(c.id, scene);
            first.get_or_insert(slot.pattern);
        }
    }
    match first {
        Some(pattern) => play_pattern(app, pattern),
        None => app.invalidate(),
    }
}

/// Stops every track.
pub fn stop_all(app: &mut App) {
    app.state.launcher.playing.clear();
    app.invalidate();
}

/// The current pattern goes on the air: PAT mode at that pattern's number (song playback would ignore the launcher),
/// starting the transport when it is stopped.
fn play_pattern(app: &mut App, pattern: u16) {
    app.state.pattern = pattern;
    app.state.project.current_pattern = Some(pattern);
    if app.state.song_mode {
        app.act(Action::PatSong(false));
    }
    if !app.transport.playing() {
        app.act(Action::Play);
    }
    app.invalidate();
}

/// Opens (track, scene)'s clip in the piano roll: its pattern becomes the current one and the window comes up. False
/// when the slot is empty.
pub fn open(app: &mut App, track: u32, scene: usize) -> bool {
    let Some(slot) = clip(&app.state, track, scene) else { return false };
    app.state.pattern = slot.pattern;
    app.state.project.current_pattern = Some(slot.pattern);
    app.state.launcher.playing.insert(track, scene);
    app.state.piano.dirty = true;
    app.state.raise(crate::app::Win::PianoRoll);
    app.invalidate();
    true
}

/// Runs a launcher action.
pub fn run(app: &mut App, act: Act) {
    match act {
        Act::Launch(track, scene) => {
            // A second click on the same slot opens its clip in the piano roll; the first one launches it, as Bitwig's
            // press does.
            let now = std::time::Instant::now();
            let double = app
                .launcher_click
                .is_some_and(|((t, sc), at)| t == track && sc == scene && at.elapsed().as_millis() < 450);
            app.launcher_click = if double { None } else { Some(((track, scene), now)) };
            if double && open(app, track, scene) {
                return;
            }
            launch(app, track, scene)
        }
        Act::Stop(track) => stop(app, track),
        Act::LaunchScene(scene) => launch_scene(app, scene),
        Act::StopAll => stop_all(app),
        Act::Set(track, scene) => {
            let current = app.state.pattern;
            let next = (clip(&app.state, track, scene).map(|c| c.pattern) != Some(current)).then_some(current);
            set(app, track, scene, next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::{Key, NamedKey};

    fn app() -> App {
        let mut app = App::new(None);
        app.state.project = crate::app::test_project();
        app.state.project.linear_mode = true;
        // Two linear tracks: the template arrives with none (the same fixture the device-rack tests use).
        use crate::{playlist::Cmd, track_links::Mode};
        crate::playlist::run(&mut app, Cmd::TrackMode(0, Mode::LinearMidi));
        crate::playlist::run(&mut app, Cmd::TrackMode(1, Mode::LinearMidi));
        app
    }

    /// A launched clip puts its pattern on the air: the track's column lights up for that scene, the pattern becomes
    /// the current one and playback is in PAT mode (song playback would ignore the launcher).
    #[test]
    fn launching_a_slot_plays_its_pattern() {
        let mut app = app();
        let col = columns(&app.state).first().cloned().expect("a track");
        set(&mut app, col.id, 1, Some(3));
        launch(&mut app, col.id, 1);
        assert_eq!(playing(&app.state, col.id), Some(1));
        assert_eq!(app.state.pattern, 3);
        assert_eq!(app.state.project.current_pattern, Some(3));
        assert!(!app.state.song_mode, "PAT mode: the launcher plays the pattern");
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

    /// A right-click sets the current pattern into a slot, and clears it when it already holds that pattern.
    #[test]
    fn a_slot_toggles_the_current_pattern() {
        let mut app = app();
        let id = columns(&app.state)[0].id;
        app.state.pattern = 4;
        run(&mut app, Act::Set(id, 2));
        assert_eq!(clip(&app.state, id, 2), Some(Slot { pattern: 4 }));
        run(&mut app, Act::Set(id, 2));
        assert_eq!(clip(&app.state, id, 2), None);
    }

    /// A scene launch lights every track that has a clip in it, and plays that scene's pattern.
    #[test]
    fn a_scene_launch_starts_its_clips() {
        let mut app = app();
        let cols = columns(&app.state);
        set(&mut app, cols[0].id, 1, Some(2));
        set(&mut app, cols[1].id, 1, Some(2));
        launch_scene(&mut app, 1);
        assert_eq!(playing(&app.state, cols[0].id), Some(1));
        assert_eq!(playing(&app.state, cols[1].id), Some(1));
        assert_eq!(app.state.pattern, 2);
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
        assert!(app.hits.iter().any(|h| h.action == Action::Launcher(Act::LaunchScene(0))), "scene 1 launches");
        assert!(app.hits.iter().any(|h| h.action == Action::Launcher(Act::StopAll)), "stop all");

        // Fill a slot, paint again, and click it.
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

        // Tab goes back to the arrangement.
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
        // An empty slot has nothing to open: the double click just stops the track.
        app.state.hide(crate::app::Win::PianoRoll);
        run(&mut app, Act::Launch(id, 6));
        run(&mut app, Act::Launch(id, 6));
        assert!(!app.state.shown(crate::app::Win::PianoRoll));
    }

    /// A MIDI track with no instrument channel is marked, so the launcher can say so on its header: clips launched
    /// there would have nothing to play.
    #[test]
    fn a_midi_track_without_an_instrument_is_marked() {
        let mut app = app();
        assert!(
            columns(&app.state).iter().all(|c| !c.no_instrument),
            "the inserted MIDI tracks have their channels"
        );
        let id = columns(&app.state)[0].id;
        let track = app.state.project.tracks.iter_mut().find(|t| t.id.0 == id).expect("the column's track");
        track.channel = None;
        assert!(
            columns(&app.state).iter().find(|c| c.id == id).unwrap().no_instrument,
            "the emptied instrument slot is marked"
        );
    }

    /// The columns follow the mixer: one per track that has a lane, ordered by its mixer strip.
    #[test]
    fn columns_follow_the_mixer_order() {
        let mut app = app();
        let cols = columns(&app.state);
        assert!(!cols.is_empty());
        let strips: Vec<u16> = cols
            .iter()
            .map(|c| {
                app.state
                    .project
                    .tracks
                    .iter()
                    .find(|t| t.id.0 == c.id)
                    .map_or(0, |t| t.mixer_strip)
            })
            .collect();
        assert!(strips.windows(2).all(|w| w[0] <= w[1]), "{strips:?}");
    }
}
