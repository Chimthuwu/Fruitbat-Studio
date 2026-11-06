// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Random editor: the shared `os-plugkit::simple` editor over its parameters.

use os_plugkit::simple::{Info, Setting};

/// The editor's static layout: one dial (or toggle, for an on / off setting) per parameter, in display order.
pub static INFO: Info = Info {
    name: "Lemon Random",
    mark: "RANDOM",
    class: "LemonRandomView",
    w: 320,
    h: 224,
    settings: &[
        Setting { id: "density", name: "Density", min: 0, max: 100 },
        Setting { id: "velocity", name: "Velocity", min: 0, max: 100 },
        Setting { id: "timing", name: "Timing", min: 0, max: 200 },
        Setting { id: "drop", name: "Drop", min: 0, max: 100 },
    ],
};
