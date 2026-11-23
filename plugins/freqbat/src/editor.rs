// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! FreqBat full editor window.

use std::sync::Arc;

use nih_plug::prelude::*;
use os_ui::{Window, Widget, Layout, Button, Knob, Switch, Selector, Label};

use crate::params::FreqBatParams;
use crate::Shared;

pub struct FreqBatEditor {
    params: Arc<FreqBatParams>,
    shared: Arc<Shared>,
    window: Window,
}

impl FreqBatEditor {
    pub fn new(params: Arc<FreqBatParams>, shared: Arc<Shared>) -> Self {
        let mut window = Window::new("FreqBat", 600, 500);
        window.set_resizable(true);
        
        FreqBatEditor {
            params,
            shared,
            window,
        }
    }
}

impl Editor for FreqBatEditor {
    fn spawn(&mut self, _async_executor: AsyncExecutor<Self>) -> Box<dyn Window> {
        Box::new(self.window.clone())
    }
    
    fn size(&self) -> (u32, u32) {
        self.window.size()
    }
    
    fn set_size(&mut self, width: u32, height: u32) {
        self.window.set_size(width, height);
    }
    
    fn param_changed(&mut self, _id: &str, _normalized_value: f32) {
        // The window will repaint automatically when parameters change
    }
}

impl Drop for FreqBatEditor {
    fn drop(&mut self) {
        // Clean up
    }
}