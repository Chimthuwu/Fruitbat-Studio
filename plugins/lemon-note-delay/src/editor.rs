// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Note Delay editor: the shared `os-plugkit::simple` editor over its parameters.

use os_plugkit::simple::{Info, Setting};

/// The editor's static layout: one dial (or toggle, for an on / off setting) per parameter, in display order.
pub static INFO: Info = Info {
    name: "Lemon Note Delay",
    mark: "DELAY",
    class: "LemonNoteDelayView",
    w: 390,
    h: 224,
    settings: &[
        Setting { id: "division", name: "Division", min: 0, max: 17 },
        Setting { id: "unit", name: "Unit", min: 0, max: 1 },
        Setting { id: "ms", name: "Time", min: 0, max: 2000 },
        Setting { id: "feedback", name: "Feedback", min: 0, max: 16 },
        Setting { id: "decay", name: "Decay", min: 0, max: 100 },
    ],
};
