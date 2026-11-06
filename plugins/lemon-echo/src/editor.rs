// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Echo editor: the shared `os-plugkit::simple` editor over its parameters.

use os_plugkit::simple::{Info, Setting};

/// The editor's static layout: one dial (or toggle, for an on / off setting) per parameter, in display order.
pub static INFO: Info = Info {
    name: "Lemon Echo",
    mark: "ECHO",
    class: "LemonEchoView",
    w: 508,
    h: 302,
    settings: &[
        Setting { id: "time", name: "Time", min: 0, max: 6000 },
        Setting { id: "feedback", name: "Feedback", min: 0, max: 6000 },
        Setting { id: "mix", name: "Mix", min: 0, max: 6000 },
        Setting { id: "mode", name: "Mode", min: 0, max: 2 },
        Setting { id: "sync", name: "Sync", min: 0, max: 1 },
        Setting { id: "filter", name: "Filter", min: 0, max: 2 },
        Setting { id: "cutoff", name: "Cutoff", min: 0, max: 6000 },
        Setting { id: "pan", name: "Pan", min: 0, max: 6000 },
        Setting { id: "width", name: "Width", min: 0, max: 6000 },
        Setting { id: "mod_rate", name: "Mod Rate", min: 0, max: 6000 },
        Setting { id: "mod_depth", name: "Mod Depth", min: 0, max: 6000 },
    ],
};
