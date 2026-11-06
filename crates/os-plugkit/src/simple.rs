// Copyright (C) 2026 ItsAlphaNeon
// SPDX-License-Identifier: GPL-3.0-or-later
// This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

//! The simple plugin editor: the shared editor window for the Lemon plugins whose parameters are plain linear
//! integers with ids (`Vals`) — the MIDI FX and layer/selector containers. One column-packed grid of blocks, a
//! dial (or a toggle for on / off settings) per parameter with its caption and value text under it, the hint well
//! and the wordmark along the foot. Drags hold the cursor in place, the wheel steps, Alt / middle click resets and
//! the right click opens the parameter menu (reset / copy / paste / type-in). A plugin adopts it with a
//! [`Info`] and an [`Editor`](nih_plug) impl that spawns [`open`].
//!
//! spec: plugins/host-link.md, plugins/plugin-parameters.md

#![cfg_attr(not(windows), allow(dead_code))] // Linux / macOS keep the plugins' stub spawn until their editors land.

use std::sync::Arc;

use nih_plug::prelude::GuiContext;
use crate::vals::Vals;
use crate::MenuDef;
use os_theme::design::lemon as brand;
use os_theme::Palette;
use os_ui::kit::lemon::{self, plug};
use os_ui::kit::State;
use os_ui::{Canvas, Font, Rect};

/// One parameter the editor shows.
pub struct Setting {
    /// The parameter id (`param_map`).
    pub id: &'static str,
    /// The caption above the block.
    pub name: &'static str,
    /// The setting's integer range (an on / off is 0..1 and draws as a toggle).
    pub min: i32,
    pub max: i32,
}

/// A plugin's static editor metadata.
pub struct Info {
    /// The window title and the hint well's fallback ("Lemon Arp").
    pub name: &'static str,
    /// The foot's big wordmark ("ARP").
    pub mark: &'static str,
    /// The Win32 window class ("LemonArpView").
    pub class: &'static str,
    /// The window size in logical px (`size(n)` gives a fitting one).
    pub w: u32,
    pub h: u32,
    /// The parameters, in display order.
    pub settings: &'static [Setting],
}

/// A window size that fits `n` blocks in a near-square grid of [`CELL_W`] x 78 blocks.
pub fn size(n: usize) -> (u32, u32) {
    let cols = ((n as f32).sqrt().ceil() as usize).clamp(2, 6);
    let rows = n.div_ceil(cols).max(1);
    let w = (24 + cols as u32 * CELL_W as u32 + 12).max(320);
    let h = 34 + rows as u32 * 78 + 34;
    (w, h)
}

/// A block's width; a knob block's height (a toggle's is 78 too, centred).
const CELL_W: f32 = 118.0;
const KNOB_H: f32 = 78.0;
const HEADER: f32 = 30.0;
const FOOT: f32 = 26.0;
const DIAL_R: f32 = 17.0;
/// Pixels of drag for a dial's whole range.
const KNOB_PX: f32 = 200.0;
const FINE: f32 = 0.25;
const LABEL: Font = Font::new(10).bold();
const HINT: Font = Font::new(11);
const MARK_BIG: Font = Font::condensed(18, 0).bold();
const MARK_SMALL: Font = Font::condensed(9, 0).bold();

/// The blocks, laid out column-packed within `w` x `h`.
struct Block {
    id: &'static str,
    name: &'static str,
    /// The dial's centre (or the toggle's).
    cx: f32,
    cy: f32,
    toggle: bool,
}

fn blocks(info: &Info, w: f32, h: f32) -> Vec<Block> {
    let usable = (h - HEADER - FOOT - 6.0).max(KNOB_H);
    let per_col = (usable / KNOB_H).floor().max(1.0) as usize;
    let cols = info.settings.len().div_ceil(per_col).max(1);
    let span = (w - 24.0).max(cols as f32 * CELL_W);
    let cw = span / cols as f32;
    let mut v = Vec::new();
    for (i, s) in info.settings.iter().enumerate() {
        let col = i / per_col;
        let row = i % per_col;
        // A toggle when the setting is an on / off (0..1).
        let toggle = s.min == 0 && s.max == 1;
        let x0 = 12.0 + col as f32 * cw;
        v.push(Block {
            id: s.id,
            name: s.name,
            cx: x0 + cw / 2.0,
            cy: HEADER + 8.0 + row as f32 * KNOB_H + KNOB_H / 2.0 - 8.0,
            toggle,
        });
    }
    v
}

/// A drag: `v` = the unrounded normalised value it has reached, `px` = pixels for the whole range.
#[derive(Clone)]
struct Drag {
    id: String,
    v: f32,
    px: f32,
}

/// Something the window has to do after an event.
enum Act {
    Repaint,
    Lock,
    Menu(String, f32, f32),
    TypeIn(String, f32, f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Btn {
    Left,
    Right,
    Middle,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Mods {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

/// The editor logic: the parameter values, the pointer, the pressed dial and any drag.
pub struct Logic {
    pub vals: Vals,
    pub info: &'static Info,
    pub view: ViewState,
    pub hits: Vec<(Rect, String)>,
    drag: Option<Drag>,
    pub hint: String,
    copied: Option<f32>,
    pub hosted: bool,
}

/// The pointer and the pressed control.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewState {
    pub mouse: Option<(f32, f32)>,
    pub pressed: Option<String>,
}

impl Logic {
    pub fn new(info: &'static Info, params: Arc<dyn nih_plug::prelude::Params>, live: Arc<crate::live::LiveEdits>, ctx: Option<Arc<dyn GuiContext>>) -> Self {
        let vals = Vals::new(params.as_ref(), live, ctx);
        Logic { vals, info, view: ViewState::default(), hits: Vec::new(), drag: None, hint: String::new(), copied: None, hosted: false }
    }

    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    fn hit(&self, x: f32, y: f32) -> Option<&(Rect, String)> {
        self.hits.iter().rev().find(|(r, _)| r.contains(x, y))
    }

    fn hint_for(&mut self, id: Option<&str>) {
        self.hint = id
            .and_then(|id| self.info.settings.iter().find(|s| s.id == id))
            .map(|s| format!("{}: {}", s.name, self.vals.text(s.id)))
            .unwrap_or_default();
    }

    fn edit(&mut self, id: &str, norm: f32) {
        self.vals.edit(id, norm);
    }

    fn reset(&mut self, id: &str) {
        let d = self.vals.default_norm(id);
        self.edit(id, d);
    }

    fn begin_drag(&mut self, id: &str, px: f32) -> Vec<Act> {
        self.vals.begin(id);
        self.drag = Some(Drag { id: id.into(), v: self.vals.norm(id), px });
        self.view.pressed = Some(id.into());
        vec![Act::Repaint, Act::Lock]
    }

    #[allow(dead_code)] // The plugins' editors call these once they adopt the simple editor.
    pub fn down(&mut self, x: f32, y: f32, btn: Btn, m: Mods) -> Vec<Act> {
        self.view.mouse = Some((x, y));
        let Some((_, id)) = self.hit(x, y).map(|(r, id)| (*r, id.clone())) else { return vec![] };
        let mut acts = vec![Act::Repaint];
        match btn {
            Btn::Right => acts.push(Act::Menu(id.clone(), x, y)),
            Btn::Middle => self.reset(&id),
            Btn::Left if m.alt => self.reset(&id),
            Btn::Left => {
                let toggle = self.info.settings.iter().find(|s| s.id == id).is_some_and(|s| s.min == 0 && s.max == 1);
                if toggle {
                    // A toggle: click flips it.
                    let on = self.vals.plain(&id) > 0.5;
                    self.edit(&id, if on { 0.0 } else { 1.0 });
                } else {
                    acts.extend(self.begin_drag(&id, KNOB_PX));
                }
            }
        }
        self.hint_for(Some(&id));
        acts
    }

    /// Mouse movement during a locked drag (logical px since the last event).
    #[allow(dead_code)] // As `down`.
    pub fn delta(&mut self, _dx: f32, dy: f32, m: Mods) -> Vec<Act> {
        let Some(Drag { id, v, px }) = self.drag.clone() else { return vec![] };
        let f = if m.ctrl { FINE } else { 1.0 };
        let v = (v - dy * f / px).clamp(0.0, 1.0);
        self.vals.set(&id, v);
        self.drag = Some(Drag { id: id.clone(), v, px });
        self.hint_for(Some(&id));
        vec![Act::Repaint]
    }

    pub fn up(&mut self, _btn: Btn) -> Vec<Act> {
        let Some(d) = self.drag.take() else { return vec![] };
        self.vals.end(&d.id);
        self.view.pressed = None;
        vec![Act::Repaint]
    }

    pub fn moved(&mut self, x: f32, y: f32, _m: Mods) -> Vec<Act> {
        self.view.mouse = Some((x, y));
        if self.drag.is_none() {
            let t = self.hit(x, y).map(|(_, id)| id.clone());
            self.hint_for(t.as_deref());
        }
        vec![Act::Repaint]
    }

    pub fn leave(&mut self) -> Vec<Act> {
        self.view.mouse = None;
        self.hint.clear();
        vec![Act::Repaint]
    }

    /// Wheel: one step per notch (1/128 of the range for the settings with many steps).
    pub fn wheel(&mut self, x: f32, y: f32, notches: f32, _m: Mods) -> Vec<Act> {
        let Some((_, id)) = self.hit(x, y).map(|(r, id)| (*r, id.clone())) else { return vec![] };
        let Some(s) = self.info.settings.iter().find(|s| s.id == id) else { return vec![] };
        let steps = (s.max - s.min) as f32;
        let step = if steps > 128.0 { (steps / 128.0).round() } else { 1.0 };
        let cur = self.vals.plain(&id);
        let plain = (cur + notches.signum() * step).clamp(s.min as f32, s.max as f32).round();
        self.vals.edit_plain(&id, plain);
        self.hint_for(Some(&id));
        vec![Act::Repaint]
    }

    /// The parameter menu for `id`.
    pub fn menu(&self, id: &str) -> MenuDef<Cmd> {
        let name = self.info.settings.iter().find(|s| s.id == id).map(|s| s.name).unwrap_or(id);
        let mut m = MenuDef::new(os_ui::kit::menu::MenuStyle::Context);
        m.head(name, Cmd::None);
        m.add(os_ui::kit::menu::MenuItem::item("Reset"), Cmd::Reset(id.into()));
        m.sep(Cmd::None);
        m.add(os_ui::kit::menu::MenuItem::item("Copy value"), Cmd::CopyValue(id.into()));
        m.add(os_ui::kit::menu::MenuItem::item("Paste value").enabled(self.copied.is_some()), Cmd::PasteValue(id.into()));
        m.add(os_ui::kit::menu::MenuItem::item("Enter value..."), Cmd::TypeIn(id.into()));
        m
    }

    pub fn pick(&mut self, cmd: Cmd, x: f32, y: f32) -> Vec<Act> {
        match cmd {
            Cmd::None => {}
            Cmd::Reset(id) => self.reset(&id),
            Cmd::CopyValue(id) => self.copied = Some(self.vals.norm(&id)),
            Cmd::PasteValue(id) => {
                if let Some(v) = self.copied {
                    self.edit(&id, v);
                }
            }
            Cmd::TypeIn(id) => return vec![Act::TypeIn(id, x, y)],
        }
        vec![Act::Repaint]
    }

    /// The value text a type-in box starts with.
    pub fn value_text(&self, id: &str) -> String {
        self.vals.text(id)
    }

    /// Applies a typed value.
    pub fn typed(&mut self, id: &str, text: &str) {
        if let Some(n) = self.vals.parse(id, text) {
            self.edit(id, n);
        }
    }

    /// Paints the editor; `hint` is drawn when the host does not show it.
    pub fn paint(&mut self, c: &mut Canvas, _t: &Palette, hint: &str) {
        let (w, h) = c.size();
        c.begin_frame(None);
        c.clear(plug::BG);
        let info = self.info;
        let bs = blocks(info, w, h);
        self.hits.clear();
        for b in &bs {
            let panel = Rect::new(b.cx - CELL_W / 2.0 + 4.0, b.cy - KNOB_H / 2.0 + 4.0, CELL_W - 8.0, KNOB_H - 8.0);
            lemon::plug_panel(c, panel);
            // The hit area covers the panel.
            self.hits.push((panel, b.id.to_string()));
            let pressed = self.view.pressed.as_deref() == Some(b.id);
            let hover = !pressed && self.view.pressed.is_none() && self.view.mouse.is_some_and(|(x, y)| panel.contains(x, y));
            let st = State { hover, pressed, ..State::NORMAL };
            // The caption above (the setting's name) and the value text below.
            c.text(LABEL, panel.x + 8.0, panel.y + 5.0, plug::DIM, b.name);
            if b.toggle {
                let on = self.vals.plain(b.id) > 0.5;
                lemon::toggle(c, b.cx, b.cy, on, State { on, ..st });
            } else {
                lemon::dial(c, b.cx, b.cy, DIAL_R, 2.8, self.vals.norm(b.id), false, brand::MAIN, st);
            }
            let vt = self.vals.text(b.id);
            let ink = if pressed || hover { brand::TEXT } else { plug::OFF };
            c.text_center(LABEL, Rect::new(panel.x, panel.bottom() - 18.0, panel.w, 14.0), ink, &vt);
        }
        // The foot: the hint well at the left and the wordmark at the right.
        let foot_c = h - FOOT / 2.0 - 4.0;
        let mark_w = lemon::spaced_width(c, MARK_BIG, info.mark, 2.0);
        let lemon_w = lemon::spaced_width(c, MARK_SMALL, "LEMON", 2.5);
        let bx = (w - 12.0 - mark_w).round();
        let (lh_b, lh_s) = (c.line_height(MARK_BIG), c.line_height(MARK_SMALL));
        lemon::spaced(c, MARK_BIG, bx, (foot_c - lh_b / 2.0).round(), plug::WORDMARK, info.mark, 2.0);
        let sx = (bx - 8.0 - lemon_w).round();
        lemon::spaced(c, MARK_SMALL, sx, (foot_c - lh_s / 2.0).round() + 1.0, brand::MAIN, "LEMON", 2.5);
        lemon::lemon_mark(c, sx - 14.0, foot_c, 3.5, brand::MAIN);
        let well = Rect::span(12.0, sx - 26.0, foot_c - 10.0, foot_c + 10.0);
        lemon::plug_well(c, well, 5.0);
        if !hint.is_empty() {
            let lh = c.line_height(HINT);
            c.text_clip(HINT, well.x + 8.0, (well.cy() - lh / 2.0).round(), well.w - 16.0, brand::TEXT, hint);
        }
    }
}

/// The parameter menu's commands.
#[derive(Clone, Debug, PartialEq)]
pub enum Cmd {
    None,
    Reset(String),
    CopyValue(String),
    PasteValue(String),
    TypeIn(String),
}

#[cfg(windows)]
mod win32 {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
        use std::sync::Arc;

    use nih_plug::prelude::GuiContext;
    use crate::live::LiveEdits;
    use crate::win::{self as pw, CursorLock};
    use os_theme::Palette;
    use os_ui::Canvas;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Controls::WM_MOUSELEAVE;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    use super::{Act, Btn, Info, Logic, Mods};

    const TIMER: usize = 1;
    const WM_CHANGED: u32 = WM_APP + 1;

    struct Win {
        hwnd: HWND,
        canvas: Canvas,
        buf: Vec<u32>,
        scale: f32,
        size: (u32, u32),
        logic: Logic,
        palette: Palette,
        lock: CursorLock,
        tracking: bool,
        slot: Arc<AtomicIsize>,
        /// The last hint shown (the host is told when it changes, spec: host-link.md).
        last_hint: String,
    }

    /// Owns the window; dropping it closes the editor.
    pub struct Handle(isize, Arc<AtomicIsize>, Arc<AtomicBool>);

    // SAFETY: the handle is only an HWND value; the wrapper drops it on the GUI thread.
    unsafe impl Send for Handle {}

    impl Drop for Handle {
        fn drop(&mut self) {
            self.1.store(0, Ordering::Release);
            self.2.store(false, Ordering::Relaxed);
            unsafe { DestroyWindow(self.0 as HWND) };
        }
    }

    pub fn post_changed(hwnd: isize) {
        if hwnd != 0 {
            unsafe { PostMessageW(hwnd as HWND, WM_CHANGED, 0, 0) };
        }
    }

    fn mods() -> Mods {
        let (ctrl, shift, alt) = pw::mods();
        Mods { ctrl, shift, alt }
    }

    /// Opens the simple editor as a child of `parent` (a Win32 HWND).
    pub fn open(
        parent: *mut c_void,
        scale: f32,
        info: &'static Info,
        params: Arc<dyn nih_plug::prelude::Params>,
        live: Arc<LiveEdits>,
        ctx: Arc<dyn GuiContext>,
        slot: Arc<AtomicIsize>,
        editor_open: Arc<AtomicBool>,
    ) -> Handle {
        let scale = if scale > 0.0 { scale } else { 1.0 };
        let px = |v: u32| (v as f32 * scale).round() as u32;
        let size = (px(info.w), px(info.h));
        editor_open.store(true, Ordering::Relaxed);
        unsafe {
            let class = pw::register(info.class, Some(wndproc));
            let win = Box::new(Win {
                hwnd: std::ptr::null_mut(),
                canvas: Canvas::new(size.0, size.1, scale).expect("canvas"),
                buf: vec![0; (size.0 * size.1) as usize],
                scale,
                size,
                logic: Logic::new(info, params, live, Some(ctx)),
                palette: crate::palette(),
                lock: CursorLock::default(),
                tracking: false,
                slot: slot.clone(),
                last_hint: String::new(),
            });
            let title = pw::wide(info.name);
            let hwnd = CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
                0,
                0,
                size.0 as i32,
                size.1 as i32,
                parent as HWND,
                std::ptr::null_mut(),
                GetModuleHandleW(std::ptr::null()),
                Box::into_raw(win) as *const c_void,
            );
            slot.store(hwnd as isize, Ordering::Release);
            // About 33 checks a second (edits the host confirms, the hint).
            SetTimer(hwnd, TIMER, 30, None);
            Handle(hwnd as isize, slot, editor_open)
        }
    }

    unsafe fn state(hwnd: HWND) -> Option<&'static mut Win> {
        let p = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut Win;
        if p.is_null() { None } else { Some(unsafe { &mut *p }) }
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        unsafe {
            if msg == WM_NCCREATE {
                let cs = lp as *const CREATESTRUCTW;
                let w = (*cs).lpCreateParams as *mut Win;
                (*w).hwnd = hwnd;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, w as isize);
                return DefWindowProcW(hwnd, msg, wp, lp);
            }
            if msg == WM_NCDESTROY {
                let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Win;
                if !p.is_null() {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    (*p).slot.store(0, Ordering::Release);
                    drop(Box::from_raw(p));
                }
                return DefWindowProcW(hwnd, msg, wp, lp);
            }
            let Some(w) = state(hwnd) else { return DefWindowProcW(hwnd, msg, wp, lp) };
            if msg == os_hostlink::message() {
                return w.host_link(wp, lp);
            }
            match msg {
                WM_PAINT => {
                    w.paint();
                    0
                }
                WM_ERASEBKGND => 1,
                WM_TIMER | WM_CHANGED => {
                    w.poll();
                    0
                }
                WM_LBUTTONDOWN | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_RBUTTONDBLCLK | WM_MBUTTONDOWN => {
                    SetFocus(hwnd);
                    let (x, y) = w.mouse(lp);
                    let btn = match msg {
                        WM_RBUTTONDOWN | WM_RBUTTONDBLCLK => Btn::Right,
                        WM_MBUTTONDOWN => Btn::Middle,
                        _ => Btn::Left,
                    };
                    let acts = w.logic.down(x, y, btn, mods());
                    if w.logic.dragging() && !w.lock.locked() {
                        SetCapture(hwnd);
                    }
                    w.run(acts);
                    0
                }
                WM_MOUSEMOVE => {
                    if !w.tracking {
                        let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                        TrackMouseEvent(&mut tme);
                        w.tracking = true;
                    }
                    if let Some((dx, dy)) = w.lock.delta() {
                        if dx != 0 || dy != 0 {
                            let acts = w.logic.delta(dx as f32 / w.scale, dy as f32 / w.scale, mods());
                            w.run(acts);
                        }
                        return 0;
                    }
                    let (x, y) = w.mouse(lp);
                    let acts = w.logic.moved(x, y, mods());
                    w.run(acts);
                    0
                }
                WM_MOUSELEAVE => {
                    w.tracking = false;
                    if !w.lock.locked() && !w.logic.dragging() {
                        let acts = w.logic.leave();
                        w.run(acts);
                    }
                    0
                }
                WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP => {
                    w.lock.unlock();
                    ReleaseCapture();
                    let acts = w.logic.up(Btn::Left);
                    w.run(acts);
                    0
                }
                WM_CAPTURECHANGED => {
                    if w.logic.dragging() && lp as HWND != hwnd {
                        w.lock.unlock();
                        let acts = w.logic.up(Btn::Left);
                        w.run(acts);
                    }
                    0
                }
                WM_MOUSEWHEEL => {
                    let mut r: RECT = std::mem::zeroed();
                    GetWindowRect(hwnd, &mut r);
                    let (sx, sy) = pw::xy(lp);
                    let (x, y) = ((sx - r.left) as f32 / w.scale, (sy - r.top) as f32 / w.scale);
                    let notches = ((wp >> 16) & 0xffff) as u16 as i16 as f32 / 120.0;
                    let acts = w.logic.wheel(x, y, notches, mods());
                    w.run(acts);
                    0
                }
                WM_DESTROY => {
                    KillTimer(hwnd, TIMER);
                    w.lock.unlock();
                    0
                }
                _ => DefWindowProcW(hwnd, msg, wp, lp),
            }
        }
    }

    impl Win {
        fn mouse(&self, l: LPARAM) -> (f32, f32) {
            let (x, y) = pw::xy(l);
            (x as f32 / self.scale, y as f32 / self.scale)
        }

        fn screen(&self, x: f32, y: f32) -> (i32, i32) {
            pw::to_screen(self.hwnd, (x * self.scale) as i32, (y * self.scale) as i32)
        }

        fn invalidate(&self) {
            unsafe { InvalidateRect(self.hwnd, std::ptr::null(), 0) };
        }

        fn paint(&mut self) {
            let (hdc, ps) = pw::begin_paint(self.hwnd);
            let hint = if self.logic.hosted { String::new() } else { self.logic.hint.clone() };
            let mut palette = self.palette.clone();
            if palette != crate::palette() {
                palette = crate::palette();
                self.palette = palette.clone();
            }
            let canvas = &mut self.canvas;
            self.logic.paint(canvas, &palette, &hint);
            self.buf.resize((self.size.0 * self.size.1) as usize, 0);
            self.canvas.copy_to_xrgb(&mut self.buf);
            pw::blit(hdc, &self.buf, self.size.0, self.size.1);
            pw::end_paint(self.hwnd, &ps);
        }

        /// Timer / parameter change: expire confirmed overlay values and repaint.
        fn poll(&mut self) {
            self.logic.vals.expire();
            self.invalidate();
            if self.logic.hint != self.last_hint {
                self.last_hint = self.logic.hint.clone();
                if self.logic.hosted {
                    os_hostlink::to_host(self.hwnd as isize, os_hostlink::Cmd::Changed, 0);
                }
            }
        }

        /// Answers the host (spec `plugins/host-link.md`): one page, the hint line. Unknown commands return 0.
        unsafe fn host_link(&mut self, wp: WPARAM, lp: LPARAM) -> LRESULT {
            use os_hostlink::{Cmd, PageIcon};
            let Some(cmd) = Cmd::from_usize(wp) else { return 0 };
            match cmd {
                Cmd::Hello => {
                    self.logic.hosted = true;
                    self.invalidate();
                    os_hostlink::VERSION as LRESULT
                }
                Cmd::PageCount => 1,
                Cmd::PageIcon => PageIcon::Plugin as LRESULT,
                Cmd::GetPage | Cmd::SetPage => 0,
                Cmd::Metrics => os_hostlink::pack_size(self.size.0, self.size.1),
                Cmd::Hint => {
                    if lp != 0 {
                        // SAFETY: the host passes a `HintBuf` of its own process (spec: host-link.md).
                        unsafe { &mut *(lp as *mut os_hostlink::HintBuf) }.set(&self.logic.hint);
                    }
                    1
                }
                Cmd::Changed => {
                    self.invalidate();
                    1
                }
                _ => 0,
            }
        }

        fn run(&mut self, acts: Vec<Act>) {
            for act in acts {
                match act {
                    Act::Repaint => self.invalidate(),
                    Act::Lock => self.lock.lock(self.hwnd),
                    Act::Menu(id, x, y) => {
                        let def = self.logic.menu(&id);
                        let (sx, sy) = self.screen(x, y);
                        let palette = self.palette.clone();
                        let scale = self.scale;
                        if let Some(cmd) = pw::popup(self.hwnd, sx, sy, &def, &palette, scale) {
                            let acts = self.logic.pick(cmd, x, y);
                            self.run(acts);
                        }
                    }
                    Act::TypeIn(id, x, y) => {
                        let (sx, sy) = self.screen(x, y);
                        let text = self.logic.value_text(&id);
                        let scale = self.scale;
                        if let Some(t) = pw::type_in(self.hwnd, sx, sy, &text, scale) {
                            self.logic.typed(&id, &t);
                        }
                        self.invalidate();
                    }
                }
            }
        }
    }
}

