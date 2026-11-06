// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Velocity Curve editor: the shared `os-plugkit::simple` editor over its parameters.

use os_plugkit::simple::{Info, Setting};

/// The editor's static layout: one dial (or toggle, for an on / off setting) per parameter, in display order.
pub static INFO: Info = Info {
    name: "Lemon Velocity Curve",
    mark: "VELOCITY",
    class: "LemonVelocityCurveView",
    w: 320,
    h: 146,
    settings: &[
        Setting { id: "curve", name: "Curve", min: 0, max: 2 },
        Setting { id: "amount", name: "Amount", min: 0, max: 100 },
    ],
};
