// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Arp editor: the shared `os-plugkit::simple` editor over its seven parameters.

use os_plugkit::simple::{Info, Setting};

/// The editor's static layout: one dial (or toggle, for an on / off setting) per parameter, in display order.
pub static INFO: Info = Info {
    name: "Lemon Arp",
    mark: "ARP",
    class: "LemonArpView",
    w: 390,
    h: 302,
    settings: &[
        Setting { id: "pattern", name: "Pattern", min: 0, max: 3 },
        Setting { id: "octave", name: "Octave", min: 0, max: 4 },
        Setting { id: "rate", name: "Rate", min: 0, max: 4 },
        Setting { id: "gate", name: "Gate", min: 0, max: 100 },
        Setting { id: "swing", name: "Swing", min: -50, max: 50 },
        Setting { id: "hold", name: "Hold", min: 0, max: 1 },
        Setting { id: "latch", name: "Latch", min: 0, max: 1 },
    ],
};
