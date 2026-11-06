// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Chord editor: the shared `os-plugkit::simple` editor over its parameters.

use os_plugkit::simple::{Info, Setting};

/// The editor's static layout: one dial (or toggle, for an on / off setting) per parameter, in display order.
pub static INFO: Info = Info {
    name: "Lemon Chord",
    mark: "CHORD",
    class: "LemonChordView",
    w: 390,
    h: 302,
    settings: &[
        Setting { id: "root", name: "Root", min: 0, max: 11 },
        Setting { id: "scale", name: "Scale", min: 0, max: 8 },
        Setting { id: "chord_type", name: "Chord Type", min: 0, max: 6 },
        Setting { id: "inversion", name: "Inversion", min: 0, max: 3 },
        Setting { id: "voicing", name: "Voicing", min: 0, max: 3 },
        Setting { id: "strum", name: "Strum", min: 0, max: 3 },
        Setting { id: "strum_speed", name: "Strum Speed", min: 0, max: 10000 },
        Setting { id: "hold", name: "Hold", min: 0, max: 1 },
    ],
};
