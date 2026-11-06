// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Note Repeats editor: the shared `os-plugkit::simple` editor over its parameters.

use os_plugkit::simple::{Info, Setting};

/// The editor's static layout: one dial (or toggle, for an on / off setting) per parameter, in display order.
pub static INFO: Info = Info {
    name: "Lemon Note Repeats",
    mark: "REPEATS",
    class: "LemonNoteRepeatsView",
    w: 320,
    h: 224,
    settings: &[
        Setting { id: "rate", name: "Rate", min: 0, max: 14 },
        Setting { id: "gate", name: "Gate", min: 1, max: 100 },
        Setting { id: "taper", name: "Taper", min: 0, max: 100 },
    ],
};
