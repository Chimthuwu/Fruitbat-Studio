// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Transpose editor: the shared `os-plugkit::simple` editor over its parameters.

use os_plugkit::simple::{Info, Setting};

/// The editor's static layout: one dial (or toggle, for an on / off setting) per parameter, in display order.
pub static INFO: Info = Info {
    name: "Lemon Transpose",
    mark: "TRANSPOSE",
    class: "LemonTransposeView",
    w: 390,
    h: 224,
    settings: &[
        Setting { id: "semitones", name: "Semitones", min: -24, max: 24 },
        Setting { id: "octaves", name: "Octaves", min: -4, max: 4 },
        Setting { id: "scale_aware", name: "Scale Aware", min: 0, max: 1 },
        Setting { id: "root", name: "Root", min: 0, max: 11 },
        Setting { id: "scale", name: "Scale", min: 0, max: 8 },
    ],
};
