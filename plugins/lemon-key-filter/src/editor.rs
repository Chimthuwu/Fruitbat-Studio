// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! Lemon Key Filter editor stub.

use std::any::Any;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::Arc;

use nih_plug::prelude::*;

use crate::params::KeyFilterParams;
use crate::Shared;

pub struct KeyFilterEditor {
    params: Arc<KeyFilterParams>,
    shared: Arc<Shared>,
    scale: AtomicU32,
    hwnd: Arc<AtomicIsize>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    slot: os_plugkit::gui::Slot,
}

impl KeyFilterEditor {
    pub fn new(params: Arc<KeyFilterParams>, shared: Arc<Shared>) -> Self {
        KeyFilterEditor {
            params,
            shared,
            scale: AtomicU32::new(1f32.to_bits()),
            hwnd: Arc::new(AtomicIsize::new(0)),
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            slot: Default::default(),
        }
    }

    fn notify(&self) {
        #[cfg(windows)]
        win32::post_changed(self.hwnd.load(Ordering::Acquire));
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        os_plugkit::gui::notify(&self.slot);
    }
}

impl Editor for KeyFilterEditor {
    fn spawn(&self, parent: ParentWindowHandle, context: Arc<dyn GuiContext>) -> Box<dyn Any + Send> {
        let scale = f32::from_bits(self.scale.load(Ordering::Relaxed));
        match parent {
            #[cfg(windows)]
            ParentWindowHandle::Win32Hwnd(hwnd) => {
                Box::new(win32::open(hwnd, scale, self.params.clone(), self.shared.clone(), context, self.hwnd.clone()))
            }
            #[cfg(target_os = "linux")]
            ParentWindowHandle::X11Window(window) => Box::new(gui::open(window, scale, self.params.clone(), self.shared.clone(), context, &self.slot)),
            #[cfg(target_os = "macos")]
            ParentWindowHandle::AppKitNsView(view) => {
                let scale = os_plugkit::gui::backing_scale(view as usize);
                Box::new(gui::open(view as usize, scale, self.params.clone(), self.shared.clone(), context, &self.slot))
            }
            _ => {
                let _ = (context, scale);
                Box::new(())
            }
        }
    }

    fn size(&self) -> (u32, u32) {
        (360, 260)
    }

    fn set_scale_factor(&self, factor: f32) -> bool {
        self.scale.store(factor.to_bits(), Ordering::Relaxed);
        true
    }

    fn param_value_changed(&self, _id: &str, _normalized_value: f32) {
        self.notify();
    }
    fn param_modulation_changed(&self, _id: &str, _modulation_offset: f32) {
        self.notify();
    }
    fn param_values_changed(&self) {
        self.notify();
    }
}

// Minimal platform stubs
#[cfg(windows)]
mod win32 {
    use std::any::Any;
    use std::sync::Arc;
    use std::sync::atomic::AtomicIsize;
    use nih_plug::prelude::*;
    use crate::params::KeyFilterParams;
    use crate::Shared;

    pub fn open(
        _hwnd: *mut std::ffi::c_void,
        _scale: f32,
        _params: Arc<KeyFilterParams>,
        _shared: Arc<Shared>,
        _context: Arc<dyn GuiContext>,
        _hwnd_atomic: Arc<AtomicIsize>,
    ) -> Box<dyn Any + Send> {
        Box::new(())
    }

    pub fn post_changed(_hwnd: isize) {}
}

#[cfg(target_os = "linux")]
mod gui {
    use std::any::Any;
    use std::sync::Arc;
    use nih_plug::prelude::*;
    use crate::params::KeyFilterParams;
    use crate::Shared;
    use os_plugkit::gui::Slot;

    pub fn open(
        _window: u64,
        _scale: f32,
        _params: Arc<KeyFilterParams>,
        _shared: Arc<Shared>,
        _context: Arc<dyn GuiContext>,
        _slot: &Slot,
    ) -> Box<dyn Any + Send> {
        Box::new(())
    }
}

#[cfg(target_os = "macos")]
mod gui {
    use std::any::Any;
    use std::sync::Arc;
    use nih_plug::prelude::*;
    use crate::params::KeyFilterParams;
    use crate::Shared;
    use os_plugkit::gui::Slot;

    pub fn open(
        _view: usize,
        _scale: f32,
        _params: Arc<KeyFilterParams>,
        _shared: Arc<Shared>,
        _context: Arc<dyn GuiContext>,
        _slot: &Slot,
    ) -> Box<dyn Any + Send> {
        Box::new(())
    }
}
