// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Quantize editor: the shared `os-plugkit::simple` editor over its parameters.

use os_plugkit::simple::{Info, Setting};

/// The editor's static layout: one dial (or toggle, for an on / off setting) per parameter, in display order.
pub static INFO: Info = Info {
    name: "Lemon Quantize",
    mark: "QUANTIZE",
    class: "LemonQuantizeView",
    w: 320,
    h: 146,
    settings: &[
        Setting { id: "division", name: "Division", min: 0, max: 10 },
        Setting { id: "strength", name: "Strength", min: 0, max: 100 },
    ],
};
