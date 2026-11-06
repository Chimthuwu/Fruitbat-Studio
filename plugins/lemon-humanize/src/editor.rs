// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Humanize editor: the shared `os-plugkit::simple` editor over its parameters.

use os_plugkit::simple::{Info, Setting};

/// The editor's static layout: one dial (or toggle, for an on / off setting) per parameter, in display order.
pub static INFO: Info = Info {
    name: "Lemon Humanize",
    mark: "HUMANIZE",
    class: "LemonHumanizeView",
    w: 320,
    h: 224,
    settings: &[
        Setting { id: "timing", name: "Timing", min: 0, max: 100 },
        Setting { id: "velocity", name: "Velocity", min: 0, max: 64 },
        Setting { id: "length", name: "Length", min: 0, max: 100 },
        Setting { id: "seed", name: "Seed", min: 0, max: 9999 },
    ],
};
