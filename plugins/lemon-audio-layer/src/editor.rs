// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Audio Layer editor: the shared `os-plugkit::simple` editor over its parameters.

use os_plugkit::simple::{Info, Setting};

/// The editor's static layout: one dial (or toggle, for an on / off setting) per parameter, in display order.
pub static INFO: Info = Info {
    name: "Lemon Audio Layer",
    mark: "LAYER",
    class: "LemonAudioLayerView",
    w: 626,
    h: 380,
    settings: &[
        Setting { id: "dry", name: "Master Dry", min: 0, max: 6000 },
        Setting { id: "wet", name: "Master Wet", min: 0, max: 6000 },
        Setting { id: "s1active", name: "Slot 1 Active", min: 0, max: 1 },
        Setting { id: "s1type", name: "Slot 1 Type", min: 0, max: 4 },
        Setting { id: "s1level", name: "Slot 1 Level", min: 0, max: 6000 },
        Setting { id: "s1param", name: "Slot 1 Param", min: 0, max: 6000 },
        Setting { id: "s2active", name: "Slot 2 Active", min: 0, max: 1 },
        Setting { id: "s2type", name: "Slot 2 Type", min: 0, max: 4 },
        Setting { id: "s2level", name: "Slot 2 Level", min: 0, max: 6000 },
        Setting { id: "s2param", name: "Slot 2 Param", min: 0, max: 6000 },
        Setting { id: "s3active", name: "Slot 3 Active", min: 0, max: 1 },
        Setting { id: "s3type", name: "Slot 3 Type", min: 0, max: 4 },
        Setting { id: "s3level", name: "Slot 3 Level", min: 0, max: 6000 },
        Setting { id: "s3param", name: "Slot 3 Param", min: 0, max: 6000 },
        Setting { id: "s4active", name: "Slot 4 Active", min: 0, max: 1 },
        Setting { id: "s4type", name: "Slot 4 Type", min: 0, max: 4 },
        Setting { id: "s4level", name: "Slot 4 Level", min: 0, max: 6000 },
        Setting { id: "s4param", name: "Slot 4 Param", min: 0, max: 6000 },
    ],
};
