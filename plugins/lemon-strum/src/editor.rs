// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Strum editor: the shared `os-plugkit::simple` editor over its parameters.

use os_plugkit::simple::{Info, Setting};

/// The editor's static layout: one dial (or toggle, for an on / off setting) per parameter, in display order.
pub static INFO: Info = Info {
    name: "Lemon Strum",
    mark: "STRUM",
    class: "LemonStrumView",
    w: 320,
    h: 224,
    settings: &[
        Setting { id: "direction", name: "Direction", min: 0, max: 1 },
        Setting { id: "spread", name: "Spread", min: 0, max: 500 },
        Setting { id: "window", name: "Window", min: 0, max: 100 },
    ],
};
