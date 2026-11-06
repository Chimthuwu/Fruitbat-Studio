// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Note Layer editor: the shared `os-plugkit::simple` editor over its parameters.

use os_plugkit::simple::{Info, Setting};

/// The editor's static layout: one dial (or toggle, for an on / off setting) per parameter, in display order.
pub static INFO: Info = Info {
    name: "Lemon Note Layer",
    mark: "LAYER",
    class: "LemonNoteLayerView",
    w: 626,
    h: 380,
    settings: &[
        Setting { id: "layer1_active", name: "Layer 1 Active", min: 0, max: 1 },
        Setting { id: "layer1_type", name: "Layer 1 Type", min: 0, max: 3 },
        Setting { id: "layer1_channel", name: "Layer 1 Channel", min: 0, max: 16 },
        Setting { id: "layer1_level", name: "Layer 1 Level", min: 0, max: 100 },
        Setting { id: "layer2_active", name: "Layer 2 Active", min: 0, max: 1 },
        Setting { id: "layer2_type", name: "Layer 2 Type", min: 0, max: 3 },
        Setting { id: "layer2_channel", name: "Layer 2 Channel", min: 0, max: 16 },
        Setting { id: "layer2_level", name: "Layer 2 Level", min: 0, max: 100 },
        Setting { id: "layer3_active", name: "Layer 3 Active", min: 0, max: 1 },
        Setting { id: "layer3_type", name: "Layer 3 Type", min: 0, max: 3 },
        Setting { id: "layer3_channel", name: "Layer 3 Channel", min: 0, max: 16 },
        Setting { id: "layer3_level", name: "Layer 3 Level", min: 0, max: 100 },
        Setting { id: "layer4_active", name: "Layer 4 Active", min: 0, max: 1 },
        Setting { id: "layer4_type", name: "Layer 4 Type", min: 0, max: 3 },
        Setting { id: "layer4_channel", name: "Layer 4 Channel", min: 0, max: 16 },
        Setting { id: "layer4_level", name: "Layer 4 Level", min: 0, max: 100 },
        Setting { id: "dry_level", name: "Dry Level", min: 0, max: 100 },
    ],
};
