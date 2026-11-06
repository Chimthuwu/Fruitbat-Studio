// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Note Length editor: the shared `os-plugkit::simple` editor over its parameters.

use os_plugkit::simple::{Info, Setting};

/// The editor's static layout: one dial (or toggle, for an on / off setting) per parameter, in display order.
pub static INFO: Info = Info {
    name: "Lemon Note Length",
    mark: "LENGTH",
    class: "LemonNoteLengthView",
    w: 320,
    h: 224,
    settings: &[
        Setting { id: "mode", name: "Mode", min: 0, max: 1 },
        Setting { id: "amount", name: "Amount", min: 1, max: 400 },
        Setting { id: "length", name: "Length", min: 0, max: 10 },
    ],
};
