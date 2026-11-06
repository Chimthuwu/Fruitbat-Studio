// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Key Filter editor: the shared `os-plugkit::simple` editor over its parameters.

use os_plugkit::simple::{Info, Setting};

/// The editor's static layout: one dial (or toggle, for an on / off setting) per parameter, in display order.
pub static INFO: Info = Info {
    name: "Lemon Key Filter",
    mark: "KEY",
    class: "LemonKeyFilterView",
    w: 320,
    h: 224,
    settings: &[
        Setting { id: "root", name: "Root", min: 0, max: 11 },
        Setting { id: "scale", name: "Scale", min: 0, max: 13 },
        Setting { id: "mode", name: "Mode", min: 0, max: 1 },
    ],
};
