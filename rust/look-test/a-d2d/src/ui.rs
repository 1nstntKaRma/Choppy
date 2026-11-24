//! A small immediate-mode UI layer: each paint lays the window out, draws it and records where the controls are;
//! the mouse and keyboard are matched against that. Hover / pressed / focus states and the control styles follow
//! Theme.xaml (Btn, Btn.Primary, Btn.Cancel, Btn.Title, Btn.TitleClose, Btn.Link, Quick, QuickDot, QuickPill, Box).
//! Positions are device pixels, rounded like WPF's layout (Canvas::rp / at / center).
//! Built to be reused by the other windows (step 4): Settings, Conflict, Benchmark.

use crate::gfx::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cursor {
    Arrow,
    Hand,
    IBeam,
}

#[derive(Clone, Copy)]
pub struct Hit {
    pub id: u32,
    pub rect: Rect,
    pub cursor: Cursor,
}

#[derive(Default)]
pub struct Ui {
    /// Under the mouse.
    pub hot: u32,
    /// The mouse went down on it and is still down.
    pub pressed: u32,
    /// Keyboard focus (WPF: a click focuses a button too, and its border shows it).
    pub focus: u32,
    /// Space held on the focused button.
    pub key_pressed: bool,
    hits: Vec<Hit>,
    last: Vec<Hit>,
    /// Focusable controls in tab order, from the last paint.
    tab: Vec<u32>,
    last_tab: Vec<u32>,
}

impl Ui {
    pub fn begin(&mut self) {
        self.hits.clear();
        self.tab.clear();
    }
    pub fn end(&mut self) {
        std::mem::swap(&mut self.hits, &mut self.last);
        std::mem::swap(&mut self.tab, &mut self.last_tab);
    }
    /// Registers a control (disabled ones take no mouse or focus).
    pub fn add(&mut self, id: u32, rect: Rect, cursor: Cursor, focusable: bool, enabled: bool) {
        if !enabled {
            return;
        }
        self.hits.push(Hit { id, rect, cursor });
        if focusable {
            self.tab.push(id);
        }
    }
    /// The control at (x, y) pixels: the last one drawn wins (it's on top).
    pub fn at(&self, x: f32, y: f32) -> Option<Hit> {
        self.last.iter().rev().find(|h| h.rect.contains(x, y)).copied()
    }
    pub fn is_hot(&self, id: u32) -> bool {
        self.hot == id && id != 0
    }
    /// Pressed = the mouse went down on it and is over it still, or Space is held on it.
    pub fn is_pressed(&self, id: u32) -> bool {
        id != 0 && ((self.pressed == id && self.hot == id) || (self.key_pressed && self.focus == id))
    }
    pub fn is_focused(&self, id: u32) -> bool {
        self.focus == id && id != 0
    }
    /// Tab / Shift+Tab.
    pub fn next_focus(&mut self, back: bool) {
        let t = &self.last_tab;
        if t.is_empty() {
            return;
        }
        let i = t.iter().position(|&f| f == self.focus);
        let n = t.len();
        self.focus = match (i, back) {
            (None, false) => t[0],
            (None, true) => t[n - 1],
            (Some(i), false) => t[(i + 1) % n],
            (Some(i), true) => t[(i + n - 1) % n],
        };
    }
    pub fn focusable(&self, id: u32) -> bool {
        self.last_tab.contains(&id)
    }
}

/// Button looks (ControlTemplate + triggers).
pub struct BtnStyle {
    pub bg: u32,
    pub border: u32,
    pub fg: u32,
    pub hover_bg: u32,
    pub hover_border: Option<u32>,
    pub hover_fg: Option<u32>,
    pub pressed_bg: u32,
    pub focus_border: Option<u32>,
    /// Corner radius in DIPs.
    pub radius: f32,
    /// Border thickness in DIPs (0 = none).
    pub border_w: f32,
}

pub const BTN: BtnStyle = BtnStyle {
    bg: 0xFF262B32,
    border: 0xFF383E47,
    fg: 0xFFF1F3F5,
    hover_bg: 0xFF2F353D,
    hover_border: Some(0xFF454C56),
    hover_fg: None,
    pressed_bg: 0xFF22262C,
    focus_border: Some(0xFF5CB8FF),
    radius: 6.0,
    border_w: 1.0,
};
pub const BTN_PRIMARY: BtnStyle = BtnStyle {
    bg: 0xFF0E72E6,
    border: 0xFF2A86EE,
    fg: 0xFFFFFFFF,
    hover_bg: 0xFF1C80F2,
    hover_border: None,
    hover_fg: None,
    pressed_bg: 0xFF0B62C6,
    focus_border: Some(0xFFBFE3FF),
    radius: 6.0,
    border_w: 1.0,
};
/// Btn.Cancel: Btn's template (its hover / pressed / focus colors), its own resting colors.
pub const BTN_CANCEL: BtnStyle = BtnStyle { bg: 0xFF23272D, border: 0xFF3A4048, fg: 0xFFF4B8B3, ..BTN };
pub const BTN_TITLE: BtnStyle = BtnStyle {
    bg: 0,
    border: 0,
    fg: 0xFFC9CED6,
    hover_bg: 0xFF2C323A,
    hover_border: None,
    hover_fg: None,
    pressed_bg: 0xFF262B32,
    focus_border: None,
    radius: 0.0,
    border_w: 0.0,
};
pub const BTN_TITLE_CLOSE: BtnStyle =
    BtnStyle { hover_bg: 0xFFC42B1C, hover_fg: Some(0xFFFFFFFF), pressed_bg: 0xFFA3271B, ..BTN_TITLE };
/// Btn.Link: flat clickable text, a soft background on hover, no pressed or focus look.
pub const BTN_LINK: BtnStyle = BtnStyle {
    bg: 0,
    border: 0,
    fg: 0xFF9AA2AD,
    hover_bg: 0xFF262B32,
    hover_border: None,
    hover_fg: None,
    pressed_bg: 0xFF262B32,
    focus_border: None,
    radius: 5.0,
    border_w: 0.0,
};

/// Draws a button's frame for its state and registers it; returns the text color for its content. A disabled
/// button leaves the canvas at 45 % opacity: the caller restores it after drawing the content.
pub fn button(cv: &mut Canvas, ui: &mut Ui, id: u32, r: Rect, st: &BtnStyle, enabled: bool, focusable: bool) -> u32 {
    ui.add(id, r, Cursor::Hand, focusable, enabled);
    let hot = enabled && ui.is_hot(id);
    let pressed = enabled && ui.is_pressed(id);
    let (mut bg, mut border, mut fg) = (st.bg, st.border, st.fg);
    if hot {
        bg = st.hover_bg;
        if let Some(b) = st.hover_border {
            border = b;
        }
        if let Some(f) = st.hover_fg {
            fg = f;
        }
    }
    if pressed {
        bg = st.pressed_bg;
    }
    if ui.is_focused(id)
        && let Some(f) = st.focus_border
    {
        border = f;
    }
    if !enabled {
        cv.opacity = 0.45;
    }
    let bgc = (bg >> 24 != 0).then_some(bg);
    let bc = (st.border_w > 0.0 && border >> 24 != 0).then_some(border);
    let bw = cv.rp(st.border_w);
    cv.border(r, st.radius, bgc, bc, bw);
    fg
}

/// A button with one line of text: Border + ContentPresenter (Margin = Padding, centered).
pub fn text_button(cv: &mut Canvas, ui: &mut Ui, id: u32, r: Rect, st: &BtnStyle, label: &str, f: Font, pad: f32, enabled: bool) {
    let old = cv.opacity;
    let fg = button(cv, ui, id, r, st, enabled, true);
    let t = cv.text(&[(label, f, fg)]);
    let b = cv.rp(st.border_w);
    let x = cv.at(r.x + b, cv.center(r.w - 2.0 * b, pad, pad, cv.tw(&t)));
    let y = cv.at(r.y + b, cv.center(r.h - 2.0 * b, 0.0, 0.0, cv.th(&t)));
    cv.draw_text(&t, x, y);
    cv.opacity = old;
}

/// Width of a text button: border + padding + text, at least MinWidth (pixels).
pub fn text_button_w(cv: &Canvas, st: &BtnStyle, label: &str, f: Font, pad: f32, min_w: f32) -> f32 {
    let t = cv.text(&[(label, f, 0)]);
    let w = 2.0 * cv.rp(st.border_w) + cv.tw(&t) + cv.rp(2.0 * pad);
    w.max(cv.rp(min_w))
}

/// The quick toggle's dot (8 x 8, Margin 0,1,8,0): green when on, red when off. `top` = its row's top, `h` its height.
fn dot(cv: &mut Canvas, x: f32, top: f32, h: f32, on: bool) {
    let d = cv.rp(8.0);
    let y = cv.at(top, cv.center(h, 1.0, 0.0, d));
    cv.ellipse(x + d / 2.0, y + d / 2.0, d / 2.0, d / 2.0, if on { 0xFF3DD27A } else { 0xFFF0645F });
}

/// The dot and its gap before the label (8 + Margin 8), in pixels.
fn dot_w(cv: &Canvas) -> f32 {
    cv.rp(8.0) + cv.rp(8.0)
}

/// Quick (copy window): a pill 32 high with a dot and a label (Padding 11,0,13,0).
pub fn quick_w(cv: &Canvas, label: &str) -> f32 {
    let t = cv.text(&[(label, sf(13.5, 400), 0)]);
    (2.0 + cv.fp(11.0 + 13.0) + dot_w(cv) + cv.tw(&t)).round_ties_even()
}

pub fn quick(cv: &mut Canvas, ui: &mut Ui, id: u32, r: Rect, label: &str, on: bool) {
    ui.add(id, r, Cursor::Hand, true, true);
    let border = if ui.is_hot(id) { 0xFF59616C } else { 0xFF383E47 };
    cv.border(r, 16.0, Some(0xFF22262C), Some(border), 1.0);
    let t = cv.text(&[(label, sf(13.5, 400), 0xFFDDE1E6)]);
    let ix = cv.at(r.x + 1.0, cv.fp(11.0));
    let ih = r.h - 2.0;
    dot(cv, ix, r.y + 1.0, ih, on);
    let ty = cv.at(r.y + 1.0, cv.center(ih, 0.0, 0.0, cv.th(&t)));
    cv.draw_text(&t, ix + dot_w(cv), ty);
}

/// QuickDot (inside the speed-limit pill): the dot and the label, no frame, centered in a row `h` high at `top`.
/// Returns its width.
pub fn quick_dot(cv: &mut Canvas, ui: &mut Ui, id: u32, x: f32, top: f32, h: f32, label: &str, on: bool) -> f32 {
    let t = cv.text(&[(label, sf(13.5, 400), 0xFFDDE1E6)]);
    let w = dot_w(cv) + cv.tw(&t);
    let own_h = cv.th(&t).max(cv.rp(8.0) + cv.rp(1.0));
    let y = cv.at(top, cv.center(h, 0.0, 0.0, own_h));
    ui.add(id, Rect::new(x, y, w, own_h), Cursor::Hand, true, true);
    dot(cv, x, y, own_h, on);
    cv.draw_text(&t, x + dot_w(cv), y);
    w
}
