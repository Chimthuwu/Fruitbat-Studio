// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Audio Selector editor: the shared `os-plugkit::simple` editor over its parameters.

use os_plugkit::simple::{Info, Setting};

/// The editor's static layout: one dial (or toggle, for an on / off setting) per parameter, in display order.
pub static INFO: Info = Info {
    name: "Lemon Audio Selector",
    mark: "SELECTOR",
    class: "LemonAudioSelectorView",
    w: 508,
    h: 380,
    settings: &[
        Setting { id: "mode", name: "Selection mode", min: 0, max: 1 },
        Setting { id: "manual", name: "Manual slot", min: 0, max: 4 },
        Setting { id: "s1_active", name: "Slot 1 Filter active", min: 0, max: 1 },
        Setting { id: "s1_level", name: "Slot 1 Filter level", min: 0, max: 6000 },
        Setting { id: "s1_param", name: "Slot 1 Filter param", min: 0, max: 6000 },
        Setting { id: "s2_active", name: "Slot 2 Delay active", min: 0, max: 1 },
        Setting { id: "s2_level", name: "Slot 2 Delay level", min: 0, max: 6000 },
        Setting { id: "s2_param", name: "Slot 2 Delay param", min: 0, max: 6000 },
        Setting { id: "s3_active", name: "Slot 3 Chorus active", min: 0, max: 1 },
        Setting { id: "s3_level", name: "Slot 3 Chorus level", min: 0, max: 6000 },
        Setting { id: "s3_param", name: "Slot 3 Chorus param", min: 0, max: 6000 },
        Setting { id: "s4_active", name: "Slot 4 Drive active", min: 0, max: 1 },
        Setting { id: "s4_level", name: "Slot 4 Drive level", min: 0, max: 6000 },
        Setting { id: "s4_param", name: "Slot 4 Drive param", min: 0, max: 6000 },
    ],
};
