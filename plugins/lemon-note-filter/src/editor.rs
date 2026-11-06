// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Note Filter editor: the shared `os-plugkit::simple` editor over its parameters.

use os_plugkit::simple::{Info, Setting};

/// The editor's static layout: one dial (or toggle, for an on / off setting) per parameter, in display order.
pub static INFO: Info = Info {
    name: "Lemon Note Filter",
    mark: "FILTER",
    class: "LemonNoteFilterView",
    w: 390,
    h: 224,
    settings: &[
        Setting { id: "low", name: "Low note", min: 0, max: 127 },
        Setting { id: "high", name: "High note", min: 0, max: 127 },
        Setting { id: "vel_low", name: "Vel low", min: 1, max: 127 },
        Setting { id: "vel_high", name: "Vel high", min: 1, max: 127 },
        Setting { id: "mode", name: "Mode", min: 0, max: 1 },
        Setting { id: "channel", name: "Channel", min: 0, max: 16 },
    ],
};
