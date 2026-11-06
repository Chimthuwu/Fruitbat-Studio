// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Note Selector editor: the shared `os-plugkit::simple` editor over its parameters.

use os_plugkit::simple::{Info, Setting};

/// The editor's static layout: one dial (or toggle, for an on / off setting) per parameter, in display order.
pub static INFO: Info = Info {
    name: "Lemon Note Selector",
    mark: "SELECTOR",
    class: "LemonNoteSelectorView",
    w: 508,
    h: 302,
    settings: &[
        Setting { id: "mode", name: "Mode", min: 0, max: 3 },
        Setting { id: "manual_slot", name: "Manual Slot", min: 0, max: 3 },
        Setting { id: "slot1_type", name: "Slot 1 Type", min: 0, max: 3 },
        Setting { id: "slot1_active", name: "Slot 1 Active", min: 0, max: 1 },
        Setting { id: "slot2_type", name: "Slot 2 Type", min: 0, max: 3 },
        Setting { id: "slot2_active", name: "Slot 2 Active", min: 0, max: 1 },
        Setting { id: "slot3_type", name: "Slot 3 Type", min: 0, max: 3 },
        Setting { id: "slot3_active", name: "Slot 3 Active", min: 0, max: 1 },
        Setting { id: "slot4_type", name: "Slot 4 Type", min: 0, max: 3 },
        Setting { id: "slot4_active", name: "Slot 4 Active", min: 0, max: 1 },
    ],
};
