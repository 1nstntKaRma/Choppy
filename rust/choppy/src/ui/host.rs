//! What every window shares: it paints its element tree (built again from its state each time), finds the control
//! under the mouse, keeps hover / pressed / keyboard focus (Tab, Space, Enter like WPF buttons), edits text boxes,
//! shows tooltips (Theme.xaml's ToolTip), sizes itself to its content (SizeToContent="Height"), follows DPI
//! changes, and draws itself into a PNG for the test hook (`shot`). Pop-ups (the queue, the "?" card, the tray
//! menu) are layered windows that use the same host.

use std::cell::RefCell;
use std::rc::Rc;

use crate::sys::gfx::{Canvas, Gfx, Rect, Target};
use crate::sys::win::{self, Cursor, Ev, Handler, HitData, Spec, Win};
use crate::ui::kit::{BoxStyle, TextBox, UiState};
use crate::ui::layout::{El, Hit, INF, Lx, layout};

pub const T_CARET: usize = 1;
pub const T_TIP: usize = 2;
pub const T_TIP_HIDE: usize = 3;
/// Window timers of their own start here.
pub const T_USER: usize = 100;

const VK_TAB: u16 = 0x09;
const VK_RETURN: u16 = 0x0D;
const VK_SPACE: u16 = 0x20;
const VK_SHIFT: u16 = 0x10;
const VK_CONTROL: u16 = 0x11;

/// A window's content: its tree and what its controls do.
pub trait Content {
    /// The whole client area's tree (DIPs).
    fn build(&mut self, ui: &UiState) -> El;
    fn click(&mut self, _h: &mut Host, _id: u32) {}
    /// A key before the host's own handling; true = handled.
    fn key(&mut self, _h: &mut Host, _vk: u16) -> bool {
        false
    }
    fn text_box(&mut self, _id: u32) -> Option<(&mut TextBox, BoxStyle)> {
        None
    }
    fn text_changed(&mut self, _h: &mut Host, _id: u32) {}
    /// The text box lost the keyboard focus (LostKeyboardFocus).
    fn focus_lost(&mut self, _h: &mut Host, _id: u32) {}
    fn timer(&mut self, _h: &mut Host, _id: usize) {}
    fn wheel(&mut self, _h: &mut Host, _delta: i32, _x: f64, _y: f64) {}
    /// The mouse came onto the window or left it (WPF's IsMouseOver).
    fn mouse_inside(&mut self, _h: &mut Host, _inside: bool) {}
    /// Alt+F4 / the system menu: true keeps it open.
    fn closing(&mut self, _h: &mut Host) -> bool {
        false
    }
    fn closed(&mut self, _h: &mut Host) {}
    fn activated(&mut self, _h: &mut Host, _on: bool) {}
    fn moved(&mut self, _h: &mut Host) {}
    fn device_change(&mut self, _h: &mut Host, _w: usize, _volume: bool) {}
    /// Moving / sizing by the user: started, an edge dragged (WM_SIZING's wParam), ended.
    fn size_move(&mut self, _h: &mut Host, _ev: Ev) {}
    /// Minimized / maximized / restored (SIZE_* in WM_SIZE).
    fn state(&mut self, _h: &mut Host, _st: u32) {}
    fn right_up(&mut self, _h: &mut Host, _x: f64, _y: f64) {}
    /// After each paint (WPF's LayoutUpdated).
    fn after_paint(&mut self, _h: &Host) {}
}

/// How the window sizes itself.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Sizing {
    /// Width fixed (DIPs), height = the content's (SizeToContent="Height").
    ContentHeight(f64),
    /// The user sizes it (the Benchmark): its width as dragged; its height follows the content (SizeToContent) while
    /// `true`, as dragged when `false`.
    Free(bool),
}

pub struct Host {
    pub win: Win,
    pub g: Rc<Gfx>,
    target: Option<Target>,
    pub dpi: u32,
    pub ui: UiState,
    hits: Vec<Hit>,
    /// The mouse is on the window (between its first move and WM_MOUSELEAVE).
    mouse_in: bool,
    pub sizing: Sizing,
    /// The caption's height (DIPs; 0 = no title bar) and the resize border (Benchmark: 6).
    pub caption: f64,
    pub resize: f64,
    /// The last laid-out height the content wanted (DIPs).
    pub content_h: f64,
    /// The size the window has in DIPs (after the last paint).
    pub size: (f64, f64),
    pub active: bool,
    /// A layered pop-up: where it is on screen (pixels) and its fade.
    pub layered: Option<Layered>,
    tip: Option<TipState>,
    /// The last tree (tests, dumps).
    pub last: Option<El>,
    /// A fixed scale for pictures at 100 % (tests), else the screen's.
    pub fixed_dpi: Option<u32>,
    pub closed: bool,
    /// A window built in code like the C# dialogs: WPF's defaults (no layout rounding, Ideal text).
    pub plain: bool,
}

pub struct Layered {
    pub pos: (i32, i32),
    pub alpha: u8,
}

struct TipState {
    text: String,
    win: Option<Rc<RefCell<TipWindow>>>,
    shown: bool,
}

/// A window = its host + its content.
pub struct Window<C: Content> {
    pub host: Host,
    pub c: C,
}

impl<C: Content + 'static> Window<C> {
    /// Makes the window (hidden; `show` puts it on screen).
    pub fn create(g: &Rc<Gfx>, spec: &Spec, sizing: Sizing, caption: f64, c: C) -> Option<Rc<RefCell<Window<C>>>> {
        let me = Rc::new(RefCell::new(Window { host: Host::new(g.clone(), sizing, caption), c }));
        let h: Rc<RefCell<dyn Handler>> = me.clone();
        let w = Win::create(spec, h)?;
        {
            let mut m = me.borrow_mut();
            m.host.win = w;
            m.host.dpi = w.dpi();
            if spec.popup {
                m.host.layered = Some(Layered { pos: (0, 0), alpha: 255 });
            }
        }
        Some(me)
    }
}

impl<C: Content> Handler for Window<C> {
    fn event(&mut self, w: Win, ev: Ev) -> bool {
        let Window { host, c } = self;
        host.win = w;
        host.handle(c, ev)
    }
}

impl Window<()> {}

impl Content for () {
    fn build(&mut self, _ui: &UiState) -> El {
        crate::ui::layout::panel()
    }
}

impl Host {
    pub fn new(g: Rc<Gfx>, sizing: Sizing, caption: f64) -> Host {
        Host {
            win: Win::default(),
            g,
            target: None,
            dpi: 96,
            ui: UiState { caret_on: true, ..Default::default() },
            hits: Vec::new(),
            mouse_in: false,
            sizing,
            caption,
            resize: 0.0,
            content_h: 0.0,
            size: (0.0, 0.0),
            active: false,
            layered: None,
            tip: None,
            last: None,
            fixed_dpi: None,
            closed: false,
            plain: false,
        }
    }

    pub fn scale(&self) -> f64 {
        self.fixed_dpi.unwrap_or(self.dpi) as f64 / 96.0
    }

    pub fn invalidate(&self) {
        if self.layered.is_some() {
            // a layered window has no WM_PAINT: it is asked to paint once the UI thread is free
            win::request_paint(self.win);
        } else {
            self.win.invalidate();
        }
    }

    /// The window's size in pixels for this content (SizeToContent: a layout pass that draws nothing).
    pub fn measure<C: Content + ?Sized>(&mut self, c: &mut C) -> (i32, i32) {
        let s = self.scale();
        let lx = Lx { g: &self.g, scale: s, round: !self.plain, ideal: self.plain };
        let mut root = c.build(&self.ui);
        match self.sizing {
            Sizing::ContentHeight(w) => {
                let h = layout(&mut root, &lx, w, INF);
                self.content_h = h;
                ((w * s).round() as i32, crate::ui::layout::content_px(h, s) as i32)
            }
            Sizing::Free(auto) => {
                let (cw, ch) = self.win.client_size();
                if auto {
                    let w = cw as f64 / s;
                    let h = layout(&mut root, &lx, w, INF);
                    self.content_h = h;
                    (cw as i32, crate::ui::layout::content_px(h, s) as i32)
                } else {
                    (cw as i32, ch as i32)
                }
            }
        }
    }

    /// Sizes the window to its content now (after a change that makes it taller or shorter).
    pub fn fit<C: Content + ?Sized>(&mut self, c: &mut C) {
        if self.layered.is_some() {
            return;
        }
        let (pw, ph) = self.measure(c);
        let (cw, ch) = self.win.client_size();
        if (pw, ph) != (cw as i32, ch as i32) && matches!(self.sizing, Sizing::ContentHeight(_)) {
            self.win.resize(pw, ph);
        }
    }

    /// Places a new window: centered in the work area of the screen with the mouse (CenterScreen), or on its
    /// owner (CenterOwner), or at (x, y).
    pub fn place<C: Content + ?Sized>(&mut self, c: &mut C, at: Place) {
        // the DPI of where it will be: size it for that screen first
        let (wa, dpi) = match at {
            Place::Screen => {
                let p = win::cursor_screen();
                win::work_area_at(p.0, p.1)
            }
            Place::Owner(o) => {
                let r = o.rect();
                win::work_area_at((r.0 + r.2) / 2, (r.1 + r.3) / 2)
            }
            Place::At(x, y) => win::work_area_at(x + 8, y + 8),
        };
        self.dpi = dpi;
        let (pw, ph) = self.measure(c);
        let (x, y) = match at {
            Place::Screen => (wa.0 + (wa.2 - wa.0 - pw) / 2, wa.1 + (wa.3 - wa.1 - ph) / 2),
            Place::Owner(o) => {
                let r = o.rect();
                (r.0 + (r.2 - r.0 - pw) / 2, r.1 + (r.3 - r.1 - ph) / 2)
            }
            Place::At(x, y) => (x, y),
        };
        self.win.set_pos(x, y, pw, ph);
        // the DPI may differ where it landed
        let d = self.win.dpi();
        if d != self.dpi && self.fixed_dpi.is_none() {
            self.dpi = d;
            self.win.set_icons();
            let (pw, ph) = self.measure(c);
            self.win.resize(pw, ph);
        }
    }

    pub fn close(&mut self) {
        self.win.destroy();
    }

    fn ensure_target(&mut self) {
        if self.target.is_none() {
            let (w, h) = self.win.client_size();
            self.target = if self.layered.is_some() { self.g.dc_target() } else { self.g.hwnd_target(self.win_hwnd(), w, h, self.dpi as f64) };
        }
    }

    fn win_hwnd(&self) -> windows::Win32::Foundation::HWND {
        windows::Win32::Foundation::HWND(self.win.raw() as *mut core::ffi::c_void)
    }

    /// The state the builders see: keyboard focus only shows while the window is active (IsKeyboardFocused).
    fn view_state(&self) -> UiState {
        let mut u = self.ui.clone();
        if !self.active && self.layered.is_none() {
            u.focus = 0;
            u.key_pressed = false;
        }
        u
    }

    /// Lays the content out for the window's size and draws it.
    pub fn paint<C: Content + ?Sized>(&mut self, c: &mut C) {
        if self.closed {
            return;
        }
        let s = self.scale();
        let st = self.view_state();
        let mut root = c.build(&st);
        let lx = Lx { g: &self.g, scale: s, round: !self.plain, ideal: self.plain };
        let (cw, ch) = if let Some(l) = &self.layered {
            let _ = l;
            (INF, INF)
        } else {
            let (w, h) = self.win.client_size();
            (w as f64 / s, h as f64 / s)
        };
        let maxed = self.layered.is_none() && self.win.is_maximized();
        let (w, h) = match self.sizing {
            Sizing::ContentHeight(w) => (w, INF),
            Sizing::Free(auto) => (cw, if auto && !maxed { INF } else { ch }),
        };
        let (w, hh) = crate::ui::layout::layout_auto(&mut root, &lx, w, h);
        self.content_h = root.desired().1;
        self.size = (w, hh);
        // SizeToContent: the window follows its content's height
        if self.layered.is_none() && (matches!(self.sizing, Sizing::ContentHeight(_)) || (self.sizing == Sizing::Free(true) && !maxed)) {
            let want = ((w * s).round() as i32, crate::ui::layout::content_px(hh, s) as i32);
            let have = self.win.client_size();
            if want != (have.0 as i32, have.1 as i32) && !self.win.is_maximized() {
                self.win.resize(want.0, want.1);
            }
        }
        self.hits.clear();
        root.hits(&mut self.hits, Rect::new(-1e6, -1e6, 2e6, 2e6));
        self.publish_hits(s);
        self.ensure_target();
        let g = self.g.clone();
        if let Some(lay) = &self.layered {
            let (pos, alpha) = (lay.pos, lay.alpha);
            let size = (((root.rect.w) * s).ceil() as i32, ((root.rect.h) * s).ceil() as i32);
            if let Some(t) = self.target.as_mut() {
                self.win.update_layered(&g, t, pos, size, s, alpha, |cv| root.draw(cv));
            }
        } else if let Some(t) = self.target.as_mut() {
            let (pw, ph) = self.win.client_size();
            t.resize(pw, ph);
            t.set_dpi(96.0 * s);
            t.begin();
            {
                let mut cv = Canvas::new(&g, t, s);
                cv.snap = !self.plain;
                cv.ideal = self.plain;
                root.draw(&mut cv);
            }
            if !t.end() {
                self.target = None;
            }
        }
        self.last = Some(root);
        c.after_paint(self);
    }

    fn publish_hits(&self, s: f64) {
        let px = |r: Rect| Rect::new(r.x * s, r.y * s, r.w * s, r.h * s);
        let cap = self.caption * s;
        let hd = HitData {
            caption: if self.caption > 0.0 { (cap).round() + 1.0 } else { 0.0 },
            no_drag: self.hits.iter().filter(|h| h.id != 0 && h.rect.y * s < cap).map(|h| px(h.rect)).collect(),
            resize: self.resize * s,
            cursors: self.hits.iter().filter(|h| h.cursor != Cursor::Arrow).map(|h| (px(h.rect), h.cursor)).collect(),
        };
        self.win.set_hit(hd);
    }

    /// The control at a point (DIPs): the last one drawn wins.
    pub fn hit_at(&self, x: f64, y: f64) -> Option<&Hit> {
        self.hits.iter().rev().find(|h| h.id != 0 && h.rect.contains(x, y))
    }
    pub fn rect_of(&self, id: u32) -> Option<Rect> {
        self.hits.iter().rev().find(|h| h.id == id).map(|h| h.rect)
    }
    fn tip_at(&self, x: f64, y: f64) -> Option<String> {
        self.hits.iter().rev().find(|h| h.rect.contains(x, y) && h.tip.is_some()).and_then(|h| h.tip.clone())
    }
    pub fn focusable(&self, id: u32) -> bool {
        self.hits.iter().any(|h| h.id == id && h.focusable)
    }

    /// Moves the keyboard focus (Focus()); a text box gets its text selected by the caller when it wants.
    pub fn set_focus<C: Content + ?Sized>(&mut self, c: &mut C, id: u32) {
        let old = self.ui.focus;
        if old == id {
            return;
        }
        self.ui.focus = id;
        if old != 0 && c.text_box(old).is_some() {
            if let Some((tb, _)) = c.text_box(old) {
                tb.mouse_up();
            }
            c.focus_lost(self, old);
        }
        self.ui.caret_on = true;
        self.invalidate();
    }

    fn next_focus<C: Content + ?Sized>(&mut self, c: &mut C, back: bool) {
        let t: Vec<u32> = self.hits.iter().filter(|h| h.focusable).map(|h| h.id).collect();
        if t.is_empty() {
            return;
        }
        let i = t.iter().position(|&f| f == self.ui.focus);
        let n = t.len();
        let next = match (i, back) {
            (None, false) => t[0],
            (None, true) => t[n - 1],
            (Some(i), false) => t[(i + 1) % n],
            (Some(i), true) => t[(i + n - 1) % n],
        };
        self.set_focus(c, next);
        if let Some((tb, _)) = c.text_box(next) {
            tb.select_all();
        }
    }

    fn handle<C: Content + ?Sized>(&mut self, c: &mut C, ev: Ev) -> bool {
        let s = self.scale();
        match ev {
            Ev::Paint => {
                self.paint(c);
                true
            }
            Ev::Size(w, h) => {
                if let Some(t) = &self.target {
                    t.resize(w, h);
                }
                self.invalidate();
                false
            }
            Ev::Dpi(dpi, r) => {
                if self.fixed_dpi.is_none() {
                    self.dpi = dpi;
                    self.target = None;
                    self.win.set_icons();
                    let (pw, ph) = match self.sizing {
                        Sizing::Free(_) => (r.2 - r.0, r.3 - r.1),
                        _ => self.measure(c),
                    };
                    self.win.set_pos(r.0, r.1, pw, ph);
                    self.invalidate();
                }
                true
            }
            Ev::Timer(id) => {
                match id {
                    T_CARET => {
                        self.ui.caret_on = !self.ui.caret_on;
                        if c.text_box(self.ui.focus).is_some() {
                            self.invalidate();
                        }
                    }
                    T_TIP => {
                        self.win.kill_timer(T_TIP);
                        self.show_tip();
                    }
                    T_TIP_HIDE => {
                        self.win.kill_timer(T_TIP_HIDE);
                        self.hide_tip();
                    }
                    _ => c.timer(self, id),
                }
                true
            }
            Ev::MouseMove(x, y) => {
                if !self.mouse_in {
                    self.mouse_in = true;
                    c.mouse_inside(self, true);
                }
                let (x, y) = (x / s, y / s);
                let hot = self.hit_at(x, y).map(|h| h.id).unwrap_or(0);
                let focus = self.ui.focus;
                if self.ui.pressed == focus
                    && focus != 0
                    && let Some(r) = self.rect_of(focus)
                {
                    let g = self.g.clone();
                    if let Some((tb, st)) = c.text_box(focus)
                        && tb.dragging()
                    {
                        let p = tb.pos_at(&g, s, r, &st, x);
                        tb.mouse_move(p);
                        self.invalidate();
                    }
                }
                if hot != self.ui.hot {
                    self.ui.hot = hot;
                    self.invalidate();
                }
                self.track_tip(x, y);
                false
            }
            Ev::MouseLeave => {
                if self.mouse_in {
                    self.mouse_in = false;
                    c.mouse_inside(self, false);
                }
                if self.ui.hot != 0 {
                    self.ui.hot = 0;
                    self.invalidate();
                }
                self.cancel_tip();
                false
            }
            Ev::Down(x, y, dbl) => {
                let (x, y) = (x / s, y / s);
                self.cancel_tip();
                let Some(hit) = self.hit_at(x, y).cloned() else {
                    // a click on nothing takes the focus off a text box (WPF keeps it; the main window clears it)
                    return false;
                };
                self.win.capture();
                self.ui.pressed = hit.id;
                if hit.focusable {
                    self.set_focus(c, hit.id);
                }
                let g = self.g.clone();
                if let Some((tb, st)) = c.text_box(hit.id) {
                    let p = tb.pos_at(&g, s, hit.rect, &st, x);
                    tb.mouse_down(p, win::key_down(VK_SHIFT), dbl);
                    self.ui.caret_on = true;
                }
                self.invalidate();
                true
            }
            Ev::Up(x, y) => {
                let (x, y) = (x / s, y / s);
                self.win.release_capture();
                let p = self.ui.pressed;
                self.ui.pressed = 0;
                if let Some((tb, _)) = c.text_box(p) {
                    tb.mouse_up();
                }
                let over = self.hit_at(x, y).map(|h| h.id).unwrap_or(0);
                self.invalidate();
                if p != 0 && p == over && c.text_box(p).is_none() {
                    c.click(self, p);
                }
                true
            }
            Ev::RightUp(x, y) => {
                c.right_up(self, x / s, y / s);
                false
            }
            Ev::CaptureLost => {
                if self.ui.pressed != 0 {
                    self.ui.pressed = 0;
                    self.invalidate();
                }
                false
            }
            Ev::Wheel(d, x, y) => {
                c.wheel(self, d, x / s, y / s);
                true
            }
            Ev::KeyDown(vk) => {
                if c.key(self, vk) {
                    self.invalidate();
                    return true;
                }
                let shift = win::key_down(VK_SHIFT);
                let ctrl = win::key_down(VK_CONTROL);
                if vk == VK_TAB {
                    self.next_focus(c, shift);
                    self.invalidate();
                    return true;
                }
                let f = self.ui.focus;
                if let Some((tb, _)) = c.text_box(f) {
                    let changed = tb.key(vk, shift, ctrl);
                    self.ui.caret_on = true;
                    if changed {
                        c.text_changed(self, f);
                    }
                    self.invalidate();
                    return true;
                }
                if f != 0 && (vk == VK_SPACE || vk == VK_RETURN) && self.focusable(f) {
                    if vk == VK_RETURN {
                        c.click(self, f);
                    } else {
                        self.ui.key_pressed = true;
                    }
                    self.invalidate();
                    return true;
                }
                false
            }
            Ev::KeyUp(vk) => {
                if vk == VK_SPACE && self.ui.key_pressed {
                    self.ui.key_pressed = false;
                    let f = self.ui.focus;
                    if f != 0 {
                        c.click(self, f);
                    }
                    self.invalidate();
                }
                false
            }
            Ev::Char(ch) => {
                let f = self.ui.focus;
                if let Some(ch) = char::from_u32(ch)
                    && let Some((tb, _)) = c.text_box(f)
                {
                    if tb.char(ch) {
                        c.text_changed(self, f);
                    }
                    self.ui.caret_on = true;
                    self.invalidate();
                    return true;
                }
                f != 0
            }
            Ev::Activate(on) => {
                self.active = on;
                if on {
                    self.win.set_timer(T_CARET, win::caret_blink_ms());
                } else {
                    self.ui.key_pressed = false;
                }
                c.activated(self, on);
                self.invalidate();
                false
            }
            Ev::Moved => {
                c.moved(self);
                false
            }
            Ev::Close => c.closing(self),
            Ev::Destroyed => {
                self.closed = true;
                self.hide_tip();
                self.tip = None;
                c.closed(self);
                false
            }
            Ev::DeviceChange(w, vol) => {
                c.device_change(self, w, vol);
                false
            }
            Ev::EnterSizeMove | Ev::ExitSizeMove | Ev::Sizing(_) => {
                c.size_move(self, ev);
                false
            }
            Ev::State(st) => {
                c.state(self, st);
                false
            }
            Ev::Tray => false,
        }
    }

    // ---------- tooltips ----------

    fn track_tip(&mut self, x: f64, y: f64) {
        let t = self.tip_at(x, y);
        let cur = self.tip.as_ref().map(|t| t.text.clone());
        if t == cur {
            return;
        }
        self.hide_tip();
        match t {
            Some(text) => {
                let win = self.tip.take().and_then(|t| t.win);
                self.tip = Some(TipState { text, win, shown: false });
                self.win.set_timer(T_TIP, win::hover_time().max(400));
            }
            None => {
                self.win.kill_timer(T_TIP);
                if let Some(t) = self.tip.as_mut() {
                    t.text.clear();
                }
                self.tip = self.tip.take().filter(|_| false);
            }
        }
    }

    fn cancel_tip(&mut self) {
        self.win.kill_timer(T_TIP);
        self.hide_tip();
        if let Some(t) = self.tip.as_mut() {
            t.text = String::new();
        }
    }

    fn show_tip(&mut self) {
        let s = self.scale();
        let Some(t) = self.tip.as_mut() else { return };
        if t.text.is_empty() || t.shown || self.closed {
            return;
        }
        if t.win.is_none() {
            t.win = TipWindow::make(&self.g, self.win);
        }
        if let Some(w) = &t.win {
            let (cx, cy) = win::cursor_screen();
            // placed under the mouse pointer (WPF's Placement Mouse: below the cursor's box)
            TipWindow::show_at(w, &t.text, (cx, cy + (20.0 * s).round() as i32), s);
            t.shown = true;
            self.win.set_timer(T_TIP_HIDE, 5000);
        }
    }

    fn hide_tip(&mut self) {
        if let Some(t) = self.tip.as_mut()
            && t.shown
        {
            t.shown = false;
            if let Some(w) = &t.win
                && let Ok(w) = w.try_borrow()
            {
                w.host.win.hide();
            }
        }
    }

    /// Draws the window into a PNG at `dpi` (the test hook's `shot`): laid out at the screen's scale, drawn at
    /// `dpi`, like WPF's RenderTargetBitmap of the live window. The frame as the C# draws it: the window background
    /// with a 1-DIP border pen on its edge, then the content.
    pub fn shot<C: Content + ?Sized>(&mut self, c: &mut C, path: &std::path::Path, dpi: f64) -> bool {
        let s = self.scale();
        let st = self.view_state();
        let mut root = c.build(&st);
        let lx = Lx { g: &self.g, scale: s, round: !self.plain, ideal: self.plain };
        let (w, h) = match self.sizing {
            Sizing::ContentHeight(w) => (w, INF),
            Sizing::Free(_) => {
                let (cw, ch) = self.win.client_size();
                (cw as f64 / s, ch as f64 / s)
            }
        };
        let (w, hh) = crate::ui::layout::layout_auto(&mut root, &lx, w, h);
        let k = dpi / 96.0;
        let (pw, ph) = ((w * k).ceil() as u32, (hh * k).ceil() as u32);
        let Some(mut t) = self.g.picture_target(pw, ph, dpi) else { return false };
        t.begin();
        t.clear(0);
        {
            let mut cv = Canvas::new(&self.g, &mut t, k);
            cv.snap = !self.plain;
            cv.ideal = self.plain;
            let r = Rect::new(0.0, 0.0, w, hh);
            cv.fill_gradient(r, 0.0, &[(0.0, crate::ui::theme::WIN_TOP), (1.0, crate::ui::theme::WIN_BOTTOM)], (0.0, 0.0), (0.0, 1.0));
            cv.stroke_round(r, 0.0, crate::ui::theme::WIN_BORDER, 1.0);
            // the root is the window's Border: its content only
            for ch in root.children.iter() {
                ch.draw(&mut cv);
            }
        }
        t.end();
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        if std::env::var("CHOPPY_DUMP").as_deref() == Ok("1") {
            // tests: every named element's and text's pixel rectangle, as the C# hook on the rust branch writes its own
            let mut lines = vec![format!("window {:.2} x {:.2} scale {} (content {:.6} px)", w * s, hh * s, s, root.unclipped().1 * s)];
            root.dump(s, 0, &mut lines);
            let mut f = path.as_os_str().to_owned();
            f.push(".layout.txt");
            let _ = std::fs::write(f, lines.join("\r\n") + "\r\n");
        }
        self.g.save_png(&t, path)
    }
}

/// Where a new window goes.
#[derive(Clone, Copy)]
pub enum Place {
    Screen,
    Owner(Win),
    At(i32, i32),
}

// ---------- the tooltip window ----------

pub struct TipContent {
    text: String,
}

impl Content for TipContent {
    fn build(&mut self, _ui: &UiState) -> El {
        crate::ui::kit::tooltip_el(&self.text)
    }
}

pub type TipWindow = Window<TipContent>;

impl TipWindow {
    fn make(g: &Rc<Gfx>, owner: Win) -> Option<Rc<RefCell<TipWindow>>> {
        let spec = Spec { owner: Some(owner), popup: true, topmost: true, ..Default::default() };
        Window::create(g, &spec, Sizing::ContentHeight(INF), 0.0, TipContent { text: String::new() })
    }

    fn show_at(me: &Rc<RefCell<TipWindow>>, text: &str, at: (i32, i32), scale: f64) {
        let Ok(mut m) = me.try_borrow_mut() else { return };
        let Window { host, c } = &mut *m;
        c.text = text.to_string();
        host.fixed_dpi = Some((scale * 96.0).round() as u32);
        // kept on the screen
        let (wa, _) = win::work_area_at(at.0, at.1);
        let lx = Lx { g: &host.g, scale, round: true, ideal: false };
        let mut root = c.build(&host.ui);
        root.measure(&lx, (INF, INF));
        let (pw, ph) = ((root.desired().0 * scale).ceil() as i32, (root.desired().1 * scale).ceil() as i32);
        let x = at.0.min(wa.2 - pw).max(wa.0);
        let y = if at.1 + ph > wa.3 { at.1 - ph - (24.0 * scale) as i32 } else { at.1 };
        if let Some(l) = host.layered.as_mut() {
            l.pos = (x, y);
            l.alpha = 255;
        }
        host.sizing = Sizing::ContentHeight(INF);
        host.paint(c);
        host.win.show(false);
    }
}
