// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The linear mode's tracks and a legacy project: the file keeps the playlist tracks' links (the original's audio/instrument
//! track records) and nothing of the Lemon-only state (spec: model/track-model.md).

use os_project::lemon;
use os_project::project;

/// A project with tracks round trips through a legacy project: the PlaylistTrack links come back (no new chunks needed), and
/// `tracks` and `linear_mode` do not leak into the file's bytes (a `.lemon` file keeps them, in its own events).
#[test]
fn an_flp_keeps_the_links_and_drops_the_linear_tracks() {
    // Channels first (the file's regions follow the original's order: prelude, channels, arrangements, mixer).
    let sampler = |index: u16, name: &str, track: u16| os_model::Channel {
        id: os_model::Id::NONE,
        index,
        kind: os_model::ChannelKind::Sampler,
        name: Some(name.into()),
        plugin: None,
        plugin_state: None,
        color: None,
        enabled: true,
        volume: 10000.0 / 12800.0,
        pan: 0.0,
        pitch_cents: 0,
        mixer_track: track,
        sample_path: None,
        layer_children: vec![],
        envelopes: Default::default(),
        rack: Default::default(),
        misc: Default::default(),
        sound: Default::default(),
    };
    let mut p = os_model::Project::default();
    p.channels.push(sampler(0, "Kick", 5));
    p.channels.push(sampler(1, "Keys", 6));
    p.mixer = (0..=6).map(os_model::MixerTrack::new).collect();
    p.current_arrangement = Some(0);
    let mut arr = os_project::project::default_arrangement();
    arr.tracks[2].link = os_model::TrackLink::instrument(1);
    arr.tracks[2].name = Some("Lead".into());
    arr.tracks[0].link = os_model::TrackLink::audio(4);
    p.arrangements.push(arr);
    p.linear_mode = true;
    p.tracks.push(os_model::Track::new(os_model::TrackKind::Midi, 3, 5, Some(1)));
    p.tracks.push(os_model::Track::new(os_model::TrackKind::Bus, 0, 6, None));

    let file = project::create(&p).unwrap();
    assert!(!lemon::is_lemon(&file), "no LMex chunk without a .lemon save");
    let bytes = file.to_bytes().unwrap();
    assert!(!bytes.windows(4).any(|w| w == b"LMex"), "the tracks' event ids are not written either");

    let back = project::load(&os_project::FlpFile::parse(&bytes).unwrap()).unwrap();
    let lane = |link: os_model::TrackLink| back.arrangements[0].tracks.iter().find(|t| t.link == link).unwrap();
    assert_eq!(lane(os_model::TrackLink::instrument(1)).name.as_deref(), Some("Lead"));
    assert!(lane(os_model::TrackLink::audio(4)).name.is_none(), "the audio track kept its link, not the midi one's name");
    assert!(back.tracks.is_empty() && !back.linear_mode, "the Lemon-only state is gone");
}
