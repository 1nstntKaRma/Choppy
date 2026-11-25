//! Theme.xaml's control styles as element builders, each with its triggers (mouse over, pressed, keyboard focus,
//! checked, disabled): Btn, Btn.Primary, Btn.Cancel, Btn.Stop, Btn.Title, Btn.TitleClose, Btn.Link, Toggle, Check,
//! Pill, Seg, QuickDot, Quick, QuickPill, Box, Box.Small; and the text box itself (caret, selection, clipboard,
//! WholeNumberBox's digits only).

use crate::sys::gfx::{Argb, Canvas, Font, Rect, TextAlign, TextOpts};
use crate::sys::win::{self, Cursor};
use crate::ui::layout::*;
use crate::ui::theme::*;

/// The interaction state the builders read: what is under the mouse, pressed, focused.
#[derive(Default, Clone)]
pub struct UiState {
    pub hot: u32,
    pub pressed: u32,
    pub focus: u32,
    /// Space held on the focused control.
    pub key_pressed: bool,
    pub caret_on: bool,
}

impl UiState {
    pub fn is_hot(&self, id: u32) -> bool {
        id != 0 && self.hot == id
    }
    pub fn is_pressed(&self, id: u32) -> bool {
        id != 0 && ((self.pressed == id && self.hot == id) || (self.key_pressed && self.focus == id))
    }
    pub fn is_focused(&self, id: u32) -> bool {
        id != 0 && self.focus == id
    }
}

/// A button's look (ControlTemplate + triggers).
#[derive(Clone, Copy)]
pub struct BtnStyle {
    pub bg: Argb,
    pub border: Argb,
    pub fg: Argb,
    pub hover_bg: Argb,
    pub hover_border: Option<Argb>,
    pub hover_fg: Option<Argb>,
    pub pressed_bg: Argb,
    pub focus_border: Option<Argb>,
    pub radius: f64,
    pub thick: f64,
    pub disabled: f64,
}

pub const BTN: BtnStyle = BtnStyle {
    bg: 0xFF262B32,
    border: 0xFF383E47,
    fg: TEXT,
    hover_bg: 0xFF2F353D,
    hover_border: Some(0xFF454C56),
    hover_fg: None,
    pressed_bg: 0xFF22262C,
    focus_border: Some(0xFF5CB8FF),
    radius: 6.0,
    thick: 1.0,
    disabled: 0.45,
};
pub const BTN_PRIMARY: BtnStyle = BtnStyle {
    bg: ACCENT,
    border: 0xFF2A86EE,
    fg: WHITE,
    hover_bg: ACCENT_HOVER,
    hover_border: None,
    hover_fg: None,
    pressed_bg: 0xFF0B62C6,
    focus_border: Some(0xFFBFE3FF),
    ..BTN
};
/// Btn.Cancel: Btn's template, its own resting colors.
pub const BTN_CANCEL: BtnStyle = BtnStyle { bg: 0xFF23272D, border: 0xFF3A4048, fg: 0xFFF4B8B3, ..BTN };
/// Btn.Stop: a solid red button that stops something that runs (the Benchmark).
pub const BTN_STOP: BtnStyle = BtnStyle {
    bg: 0xFFB3261E,
    border: 0xFFCF3D33,
    fg: WHITE,
    hover_bg: 0xFFC42B1C,
    hover_border: None,
    hover_fg: None,
    pressed_bg: 0xFF961F18,
    focus_border: Some(0xFFFFC9C4),
    ..BTN
};
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
    thick: 0.0,
    disabled: 1.0,
};
pub const BTN_TITLE_CLOSE: BtnStyle = BtnStyle { hover_bg: 0xFFC42B1C, hover_fg: Some(WHITE), pressed_bg: 0xFFA3271B, ..BTN_TITLE };
/// Btn.Link: flat clickable text, a soft background on hover, no pressed or focus look.
pub const BTN_LINK: BtnStyle = BtnStyle {
    bg: 0,
    border: 0,
    fg: DIM,
    hover_bg: 0xFF262B32,
    hover_border: None,
    hover_fg: None,
    pressed_bg: 0,
    focus_border: None,
    radius: 5.0,
    thick: 0.0,
    disabled: 1.0,
};
/// The Benchmark's small buttons beside DRIVES (Mini).
pub const BTN_MINI: BtnStyle = BtnStyle {
    bg: 0xFF22262C,
    border: 0xFF383E47,
    fg: DIM,
    hover_bg: 0xFF2F353D,
    hover_border: Some(0xFF454C56),
    hover_fg: Some(TEXT),
    pressed_bg: 0xFF1E2227,
    focus_border: Some(0xFF5CB8FF),
    radius: 5.0,
    thick: 1.0,
    disabled: 0.4,
};

/// The colors a button shows now: (background, border, foreground).
pub fn btn_colors(ui: &UiState, id: u32, st: &BtnStyle, enabled: bool) -> (Argb, Argb, Argb) {
    let (mut bg, mut bd, mut fg) = (st.bg, st.border, st.fg);
    if enabled && ui.is_hot(id) {
        bg = st.hover_bg;
        if let Some(b) = st.hover_border {
            bd = b;
        }
        if let Some(f) = st.hover_fg {
            fg = f;
        }
    }
    if enabled && ui.is_pressed(id) && st.pressed_bg != 0 {
        bg = st.pressed_bg;
    }
    if ui.is_focused(id)
        && let Some(f) = st.focus_border
    {
        bd = f;
    }
    (bg, bd, fg)
}

fn opt(c: Argb) -> Option<Argb> {
    (c >> 24 != 0).then_some(c)
}

/// A button: its Border (the template's root, `b`) with the content centered at Margin = Padding. `content` gets
/// the foreground color.
pub fn button(ui: &UiState, id: u32, st: &BtnStyle, pad: Th, enabled: bool, content: impl FnOnce(Argb) -> El) -> El {
    let (bg, bd, fg) = btn_colors(ui, id, st, enabled);
    let c = content(fg).margin(pad).center();
    border(opt(bg).map(Fill::Solid), (st.thick > 0.0).then_some(bd).and_then(opt), st.thick, st.radius, Th::default())
        .child(c)
        .id(id)
        .hand()
        .focusable(st.focus_border.is_some())
        .enabled(enabled)
        .opacity(if enabled { 1.0 } else { st.disabled })
}

/// Btn / Btn.Primary / Btn.Cancel / Btn.Stop with a text: 14 SemiBold, Height 36 (or given), Padding 18,0,
/// MinWidth 74 (or given).
pub fn text_button(ui: &UiState, id: u32, st: &BtnStyle, label: &str, enabled: bool) -> El {
    let l = label.to_string();
    button(ui, id, st, Th::hv(18.0, 0.0), enabled, move |fg| text(l, sf(14.0, SEMI), fg)).h(36.0).min_w(74.0)
}

/// Btn.Link: Border Padding 6,3, corners 5; the content as given (it sets its own colors).
pub fn link_button(ui: &UiState, id: u32, content: El) -> El {
    let hot = ui.is_hot(id);
    border(hot.then_some(Fill::Solid(0xFF262B32)), None, 0.0, 5.0, Th::hv(6.0, 3.0)).child(content).id(id).hand().focusable(true)
}

/// Btn.Title / Btn.TitleClose: 46 wide, a glyph centered; not focusable.
pub fn title_button(ui: &UiState, id: u32, close: bool, glyph: &str, size: f64, h: f64) -> El {
    let st = if close { &BTN_TITLE_CLOSE } else { &BTN_TITLE };
    let (bg, _, fg) = btn_colors(ui, id, st, true);
    border(opt(bg).map(Fill::Solid), None, 0.0, 0.0, Th::default())
        .child(text(glyph, icons(size), fg).center())
        .id(id)
        .size(46.0, h)
}

/// Toggle (a switch): the content, then a 44 x 24 track with an 18 x 18 knob.
pub fn toggle(ui: &UiState, id: u32, on: bool, content: Option<El>) -> El {
    let hot = ui.is_hot(id);
    let (bg, mut bd) = if on { (ACCENT, 0xFF2A86EE) } else { (0xFF3A3F47, 0xFF4A515C) };
    if hot {
        bd = 0xFF6B737E;
    }
    let knob = ellipse(18.0, 18.0, if on { WHITE } else { 0xFFF1F3F5 });
    let knob = if on { knob.right().m(0.0, 0.0, 2.0, 0.0) } else { knob.left().m(2.0, 0.0, 0.0, 0.0) };
    let track = border(Some(Fill::Solid(bg)), Some(bd), 1.0, 12.0, Th::default()).size(44.0, 24.0).vcenter().col(1).child(knob.vcenter());
    let mut g = grid(&[Len::Star(1.0), Len::Auto], &[]).id(id).hand().focusable(true);
    if let Some(c) = content {
        g.push(c.vcenter().m(0.0, 0.0, 14.0, 0.0));
    }
    g.child(track)
}

/// Check (the conflict's "apply to all"): a 20 x 20 box with a check mark, the content 10 to its right.
pub fn check(_ui: &UiState, id: u32, on: bool, content: El) -> El {
    let (bg, bd) = if on { (ACCENT, ACCENT) } else { (0xFF1F2329, 0xFF59616C) };
    let mark = text(G_CHECK, icons(12.0), WHITE).center().vis(if on { Vis::Visible } else { Vis::Collapsed });
    hstack()
        .id(id)
        .hand()
        .focusable(true)
        .child(border(Some(Fill::Solid(bg)), Some(bd), 1.0, 4.0, Th::default()).size(20.0, 20.0).child(mark))
        .child(content.vcenter().m(10.0, 0.0, 0.0, 0.0))
}

/// Seg (settings, the Benchmark's mode): the chosen one is filled. Margin 0,0,6,6 unless given.
pub fn seg(ui: &UiState, id: u32, on: bool, label: &str, tip: &str) -> El {
    let hot = ui.is_hot(id);
    let (bg, mut bd, fg, w) = if on { (ACCENT, 0xFF2A86EE, WHITE, SEMI) } else { (0xFF262B32, 0xFF383E47, TEXT2, NORMAL) };
    if hot && !on {
        bd = 0xFF59616C;
    }
    border(Some(Fill::Solid(bg)), Some(bd), 1.0, 6.0, Th::new(12.0, 6.0, 12.0, 7.0))
        .child(text(label, sf(13.5, w), fg).ha(Al::Center))
        .id(id)
        .hand()
        .focusable(true)
        .tip(tip)
        .m(0.0, 0.0, 6.0, 6.0)
}

/// QuickDot's dot: 8 x 8, Margin 0,1,8,0, green when on, red when off.
fn quick_dot_el(on: bool) -> El {
    ellipse(8.0, 8.0, if on { ON } else { OFF }).vcenter().m(0.0, 1.0, 8.0, 0.0)
}

/// QuickDot: the dot and the label, no frame (inside the speed-limit pill).
pub fn quick_dot(id: u32, on: bool, label: &str, tip: &str) -> El {
    hstack()
        .id(id)
        .hand()
        .focusable(true)
        .tip(tip)
        .child(quick_dot_el(on))
        .child(text(label, sf(13.5, NORMAL), TEXT2).vcenter())
}

/// Quick: a pill 32 high with a dot and a label (Padding 11,0,13,0), Margin 0,0,7,6.
pub fn quick(ui: &UiState, id: u32, on: bool, label: &str, tip: &str) -> El {
    let bd = if ui.is_hot(id) { 0xFF59616C } else { 0xFF383E47 };
    border(Some(Fill::Solid(0xFF22262C)), Some(bd), 1.0, 16.0, Th::new(11.0, 0.0, 13.0, 0.0))
        .h(32.0)
        .child(hstack().child(quick_dot_el(on)).child(text(label, sf(13.5, NORMAL), TEXT2).vcenter()))
        .id(id)
        .hand()
        .focusable(true)
        .tip(tip)
        .m(0.0, 0.0, 7.0, 6.0)
}

/// QuickPill: the frame of the speed-limit pill (Padding 11,0,4,0), Margin 0,0,8,6.
pub fn quick_pill(content: El) -> El {
    border(Some(Fill::Solid(0xFF22262C)), Some(0xFF383E47), 1.0, 16.0, Th::new(11.0, 0.0, 4.0, 0.0)).h(32.0).m(0.0, 0.0, 8.0, 6.0).child(content)
}

/// The Benchmark's drive checkbox (Tick): a 20 x 20 box, corners 5.
pub fn tick(ui: &UiState, id: u32, on: bool, enabled: bool) -> El {
    let hot = enabled && ui.is_hot(id);
    let (mut bg, mut bd) = if on { (ACCENT, 0xFF2A86EE) } else { (0xFF15181C, 0xFF4A515C) };
    if hot && !on {
        bd = 0xFF6A737F;
    }
    if hot && on {
        bg = ACCENT_HOVER;
    }
    if ui.is_focused(id) {
        bd = 0xFFBFE3FF;
    }
    border(Some(Fill::Solid(bg)), Some(bd), 1.0, 5.0, Th::default())
        .size(20.0, 20.0)
        .child(text(G_CHECK, icons(12.0), WHITE).center().vis(if on { Vis::Visible } else { Vis::Hidden }))
        .id(id)
        .cursor(if enabled { Cursor::Hand } else { Cursor::Arrow })
        .focusable(true)
        .enabled(enabled)
        .opacity(if enabled { 1.0 } else { 0.45 })
        .vcenter()
        .left()
}

// ================= text boxes =================

/// Box / Box.Small: colors, font, padding, alignment.
#[derive(Clone, Copy)]
pub struct BoxStyle {
    pub bg: Argb,
    pub font: Font,
    pub pad: Th,
    pub center: bool,
}

pub const BOX: BoxStyle = BoxStyle { bg: 0xFF1F2329, font: sf(14.0, NORMAL), pad: Th::hv(8.0, 5.0), center: false };
pub const BOX_SMALL: BoxStyle = BoxStyle { bg: 0xFF15181C, font: sf(14.0, SEMI), pad: Th::hv(2.0, 1.0), center: true };

/// A one-line text box: caret, selection (drag, Shift+arrows, double click, Ctrl+A), Backspace / Delete, Home / End,
/// the clipboard, a length limit (MaxLength) and WholeNumberBox (digits only; a paste with anything else is ignored,
/// Space does nothing).
#[derive(Clone, Default)]
pub struct TextBox {
    pub text: String,
    pub caret: usize,
    pub anchor: usize,
    pub max_len: usize,
    pub digits_only: bool,
    dragging: bool,
}

impl TextBox {
    pub fn new(max_len: usize, digits_only: bool) -> TextBox {
        TextBox { max_len, digits_only, ..Default::default() }
    }
    /// Sets the text (TextBox.Text = ...): the caret stays where it can.
    pub fn set(&mut self, s: &str) {
        if self.text != s {
            self.text = s.to_string();
            self.caret = self.caret.min(self.text.len());
            self.anchor = self.anchor.min(self.text.len());
        }
    }
    fn sel(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }
    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }

    /// The box: Border (corners 5, border 1; focused = #5CB8FF) with the text, its selection and caret.
    pub fn el(&self, ui: &UiState, id: u32, st: &BoxStyle, enabled: bool) -> El {
        let focused = ui.is_focused(id);
        let bd = if focused { 0xFF5CB8FF } else { 0xFF3A4048 };
        let me = self.clone();
        let st2 = *st;
        let caret_on = ui.caret_on;
        let inner = draw((0.0, 0.0), move |cv, r| me.paint(cv, r, &st2, focused, caret_on));
        border(Some(Fill::Solid(st.bg)), Some(bd), 1.0, 5.0, Th::default())
            .child(inner)
            .id(id)
            .cursor(Cursor::IBeam)
            .focusable(true)
            .enabled(enabled)
            .opacity(if enabled { 1.0 } else { 0.45 })
    }

    /// Where the text starts and its line's top inside the box's inner rectangle `r` (DIPs).
    fn origin(&self, cv_scale: f64, g: &crate::sys::gfx::Gfx, r: Rect, st: &BoxStyle) -> (f64, f64, crate::sys::gfx::Text) {
        let t = g.text(&[(&self.text, st.font, TEXT)], &TextOpts::default(), cv_scale);
        let rnd = |v: f64| (v * cv_scale).round_ties_even() / cv_scale;
        // the ScrollViewer (Margin = Padding) centered in the border; TextAlignment Center centers the line in it,
        // a left-aligned line starts 2 DIPs in (the text view's own margin)
        let x = if st.center { r.x + rnd(st.pad.l + (r.w - st.pad.w() - rnd(t.w)) / 2.0) } else { r.x + rnd(st.pad.l + 2.0) };
        let y = r.y + rnd(st.pad.t + (r.h - st.pad.h() - rnd(t.h)) / 2.0);
        (x, y, t)
    }

    fn paint(&self, cv: &mut Canvas, r: Rect, st: &BoxStyle, focused: bool, caret_on: bool) {
        let (x, y, t) = self.origin(cv.scale, cv.g, r, st);
        cv.push_clip(r);
        let (a, b) = self.sel();
        if focused && a != b {
            let x0 = x + t.caret_x(utf16_pos(&self.text, a));
            let x1 = x + t.caret_x(utf16_pos(&self.text, b));
            // SelectionBrush #2A79C8 at WPF's SelectionOpacity 0.4
            cv.fill(Rect::new(x0, y, x1 - x0, t.h), 0x662A79C8);
        }
        cv.text(&t, x, y);
        if focused && caret_on && a == b {
            let px = 1.0 / cv.scale;
            let cx = ((x + t.caret_x(utf16_pos(&self.text, self.caret))) * cv.scale).round() / cv.scale;
            cv.fill(Rect::new(cx, y, px, t.h), WHITE);
        }
        cv.pop_clip();
    }

    /// The character position under x, for a box whose border is at `outer`.
    pub fn pos_at(&self, g: &crate::sys::gfx::Gfx, scale: f64, outer: Rect, st: &BoxStyle, x: f64) -> usize {
        let b = (scale.round_ties_even().max(1.0)) / scale;
        let r = outer.inset(b, b, b, b);
        let (ox, _, t) = self.origin(scale, g, r, st);
        let p = t.hit(x - ox) as usize;
        byte_pos(&self.text, p)
    }

    pub fn mouse_down(&mut self, p: usize, shift: bool, double: bool) {
        if double {
            self.select_all();
            return;
        }
        self.caret = p;
        if !shift {
            self.anchor = p;
        }
        self.dragging = true;
    }
    pub fn mouse_move(&mut self, p: usize) {
        if self.dragging {
            self.caret = p;
        }
    }
    pub fn mouse_up(&mut self) {
        self.dragging = false;
    }
    pub fn dragging(&self) -> bool {
        self.dragging
    }

    fn insert(&mut self, s: &str) -> bool {
        let s: String = if self.digits_only { s.chars().filter(|c| c.is_ascii_digit()).collect() } else { s.to_string() };
        let (a, b) = self.sel();
        let room = if self.max_len > 0 { self.max_len.saturating_sub(self.text.chars().count() - self.text[a..b].chars().count()) } else { usize::MAX };
        let s: String = s.chars().take(room).collect();
        if s.is_empty() && a == b {
            return false;
        }
        self.text.replace_range(a..b, &s);
        self.caret = a + s.len();
        self.anchor = self.caret;
        true
    }

    /// A typed character (WM_CHAR). True when the text changed.
    pub fn char(&mut self, c: char) -> bool {
        if c < ' ' || c == '\u{7f}' {
            return false;
        }
        if self.digits_only && !c.is_ascii_digit() {
            return false;
        }
        self.insert(&c.to_string())
    }

    /// A key (WM_KEYDOWN). True when the text changed.
    pub fn key(&mut self, vk: u16, shift: bool, ctrl: bool) -> bool {
        let len = self.text.len();
        let (a, b) = self.sel();
        let prev = |s: &str, i: usize| s[..i].char_indices().last().map(|(p, _)| p).unwrap_or(0);
        let next = |s: &str, i: usize| s[i..].char_indices().nth(1).map(|(p, _)| i + p).unwrap_or(s.len());
        let mv = |me: &mut TextBox, p: usize| {
            me.caret = p;
            if !shift {
                me.anchor = p;
            }
        };
        match vk {
            0x25 => {
                // Left: a selection collapses to its start
                if !shift && a != b {
                    mv(self, a)
                } else {
                    let p = prev(&self.text, self.caret);
                    mv(self, p)
                }
            }
            0x27 => {
                if !shift && a != b {
                    mv(self, b)
                } else {
                    let p = next(&self.text, self.caret);
                    mv(self, p)
                }
            }
            0x24 => mv(self, 0),
            0x23 => mv(self, len),
            0x08 => {
                if a != b {
                    self.text.replace_range(a..b, "");
                    self.caret = a;
                } else if a > 0 {
                    let p = prev(&self.text, a);
                    self.text.replace_range(p..a, "");
                    self.caret = p;
                } else {
                    return false;
                }
                self.anchor = self.caret;
                return true;
            }
            0x2E => {
                if a != b {
                    self.text.replace_range(a..b, "");
                    self.caret = a;
                } else if a < len {
                    let n = next(&self.text, a);
                    self.text.replace_range(a..n, "");
                } else {
                    return false;
                }
                self.anchor = self.caret;
                return true;
            }
            0x41 if ctrl => self.select_all(),
            0x43 if ctrl => {
                if a != b {
                    win::set_clipboard_text(&self.text[a..b]);
                }
            }
            0x58 if ctrl => {
                if a != b {
                    win::set_clipboard_text(&self.text[a..b]);
                    self.text.replace_range(a..b, "");
                    self.caret = a;
                    self.anchor = a;
                    return true;
                }
            }
            0x56 if ctrl => {
                if let Some(s) = win::clipboard_text() {
                    // WholeNumberBox: a paste with anything but digits is ignored
                    if self.digits_only && (s.is_empty() || !s.chars().all(|c| c.is_ascii_digit())) {
                        return false;
                    }
                    let s = s.replace(['\r', '\n'], "");
                    return self.insert(&s);
                }
            }
            _ => {}
        }
        false
    }
}

fn utf16_pos(s: &str, byte: usize) -> u32 {
    s[..byte.min(s.len())].encode_utf16().count() as u32
}

fn byte_pos(s: &str, u16pos: usize) -> usize {
    let mut n = 0;
    for (i, c) in s.char_indices() {
        if n >= u16pos {
            return i;
        }
        n += c.len_utf16();
    }
    s.len()
}

// ================= common pieces =================

/// The tooltip (Theme.xaml ToolTip): #2A2F37, border #3F4650, corners 5, Padding 9,5, 12.5 #E6E9EE.
pub fn tooltip_el(t: &str) -> El {
    border(Some(Fill::Solid(0xFF2A2F37)), Some(0xFF3F4650), 1.0, 5.0, Th::hv(9.0, 5.0)).child(text(t, sf(12.5, NORMAL), 0xFFE6E9EE))
}

/// Text left / center / right helpers.
pub fn txt(s: impl Into<String>, size: f64, weight: u16, c: Argb) -> El {
    text(s, sf(size, weight), c)
}

pub fn run(s: impl Into<String>, size: f64, weight: u16, c: Argb) -> (String, Font, Argb) {
    (s.into(), sf(size, weight), c)
}

pub fn align_right(e: El) -> El {
    e.align(TextAlign::Right)
}
