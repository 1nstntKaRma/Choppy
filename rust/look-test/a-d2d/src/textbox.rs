//! A one-line text box (Theme.xaml "Box" / "Box.Small"): caret, selection (mouse drag, Shift+arrows, double
//! click, Ctrl+A), Backspace / Delete, Home / End, clipboard (Ctrl+C / X / V), a length limit and a filter
//! (WholeNumberBox: digits only, pastes with anything else are ignored). Selection = #2A79C8 at 40 % over the
//! text (WPF's SelectionOpacity), caret white, 1 px, blinking at the system rate.

use crate::gfx::*;
use crate::ui::*;

pub struct TextBox {
    pub text: String,
    /// Caret and selection anchor, in UTF-16 positions (the box only holds ASCII digits).
    pub caret: usize,
    pub anchor: usize,
    pub max_len: usize,
    pub digits_only: bool,
    pub caret_on: bool,
    dragging: bool,
    /// Where the text was drawn last (for mouse hits): x of the text start.
    text_x: f32,
}

pub struct BoxStyle {
    pub bg: u32,
    pub border: u32,
    pub focus_border: u32,
    pub fg: u32,
    pub font: Font,
    pub pad: (f32, f32),
    pub center: bool,
}

pub const BOX_SMALL: BoxStyle = BoxStyle {
    bg: 0xFF15181C,
    border: 0xFF3A4048,
    focus_border: 0xFF5CB8FF,
    fg: 0xFFF1F3F5,
    font: crate::gfx::sf(14.0, 600),
    pad: (2.0, 1.0),
    center: true,
};

impl TextBox {
    pub fn new(max_len: usize, digits_only: bool) -> TextBox {
        TextBox { text: String::new(), caret: 0, anchor: 0, max_len, digits_only, caret_on: true, dragging: false, text_x: 0.0 }
    }

    pub fn set(&mut self, s: &str) {
        self.text = s.to_string();
        self.caret = self.caret.min(self.text.len());
        self.anchor = self.anchor.min(self.text.len());
    }

    fn sel(&self) -> (usize, usize) {
        (self.caret.min(self.anchor), self.caret.max(self.anchor))
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }

    pub fn draw(&mut self, cv: &mut Canvas, ui: &mut Ui, id: u32, r: Rect, st: &BoxStyle, enabled: bool) {
        ui.add(id, r, Cursor::IBeam, true, enabled);
        let old = cv.opacity;
        if !enabled {
            cv.opacity = 0.45;
        }
        let focused = ui.is_focused(id);
        cv.border(r, 5.0, Some(st.bg), Some(if focused { st.focus_border } else { st.border }), 1.0);
        let t = cv.text(&[(&self.text, st.font, st.fg)]);
        let (tw, th) = (cv.tw(&t), cv.th(&t));
        // the ScrollViewer (Margin = Padding) centered in the border; the line centered in it (TextAlignment Center)
        let x = if st.center {
            cv.at(r.x + 1.0, cv.center(r.w - 2.0, st.pad.0, st.pad.0, tw))
        } else {
            cv.at(r.x + 1.0, cv.fp(st.pad.0 + 2.0))
        };
        let y = cv.at(r.y + 1.0, cv.center(r.h - 2.0, st.pad.1, st.pad.1, th));
        self.text_x = x;
        let s = cv.scale;
        cv.clip(r.inset(1.0, 1.0, 1.0, 1.0));
        let (a, b) = self.sel();
        if focused && a != b {
            let x0 = x + t.caret_x(a as u32) * s;
            let x1 = x + t.caret_x(b as u32) * s;
            cv.fill(Rect::new(x0, y, x1 - x0, th), 0x662A79C8);
        }
        cv.draw_text(&t, x, y);
        if focused && self.caret_on && a == b {
            let cx = (x + t.caret_x(self.caret as u32) * s).round();
            cv.fill(Rect::new(cx, y, 1.0, th), 0xFFFFFFFF);
        }
        cv.unclip();
        cv.opacity = old;
    }

    fn pos_at(&self, cv: &Canvas, st: &BoxStyle, x: f32) -> usize {
        let t = cv.text(&[(&self.text, st.font, st.fg)]);
        (t.hit((x - self.text_x) / cv.scale) as usize).min(self.text.len())
    }

    pub fn mouse_down(&mut self, cv: &Canvas, st: &BoxStyle, x: f32, shift: bool, double: bool) {
        if double {
            self.select_all();
            return;
        }
        let p = self.pos_at(cv, st, x);
        self.caret = p;
        if !shift {
            self.anchor = p;
        }
        self.dragging = true;
        self.caret_on = true;
    }
    pub fn mouse_move(&mut self, cv: &Canvas, st: &BoxStyle, x: f32) {
        if self.dragging {
            self.caret = self.pos_at(cv, st, x);
        }
    }
    pub fn mouse_up(&mut self) {
        self.dragging = false;
    }

    fn insert(&mut self, s: &str) -> bool {
        let s: String = if self.digits_only { s.chars().filter(|c| c.is_ascii_digit()).collect() } else { s.to_string() };
        if s.is_empty() {
            return false;
        }
        let (a, b) = self.sel();
        let room = self.max_len.saturating_sub(self.text.len() - (b - a));
        let s: String = s.chars().take(room).collect();
        if s.is_empty() && a == b {
            return false;
        }
        self.text.replace_range(a..b, &s);
        self.caret = a + s.len();
        self.anchor = self.caret;
        true
    }

    /// A typed character (WM_CHAR). Returns true when the text changed.
    pub fn char(&mut self, c: char) -> bool {
        if c < ' ' {
            return false;
        }
        if self.digits_only && !c.is_ascii_digit() {
            return false;
        }
        self.caret_on = true;
        self.insert(&c.to_string())
    }

    /// A key (WM_KEYDOWN). Returns true when the text changed.
    pub fn key(&mut self, vk: u16, shift: bool, ctrl: bool) -> bool {
        self.caret_on = true;
        let len = self.text.len();
        let (a, b) = self.sel();
        let mv = |s: &mut TextBox, p: usize| {
            s.caret = p;
            if !shift {
                s.anchor = p;
            }
        };
        match vk {
            0x25 => {
                // Left: a selection collapses to its start
                if !shift && a != b { mv(self, a) } else { mv(self, self.caret.saturating_sub(1)) }
            }
            0x27 => {
                if !shift && a != b { mv(self, b) } else { mv(self, (self.caret + 1).min(len)) }
            }
            0x24 => mv(self, 0),
            0x23 => mv(self, len),
            0x08 => {
                if a != b {
                    self.text.replace_range(a..b, "");
                    self.caret = a;
                } else if a > 0 {
                    self.text.remove(a - 1);
                    self.caret = a - 1;
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
                    self.text.remove(a);
                } else {
                    return false;
                }
                self.anchor = self.caret;
                return true;
            }
            0x41 if ctrl => self.select_all(),
            0x43 if ctrl => {
                if a != b {
                    crate::clip::set(&self.text[a..b]);
                }
            }
            0x58 if ctrl => {
                if a != b {
                    crate::clip::set(&self.text[a..b]);
                    self.text.replace_range(a..b, "");
                    self.caret = a;
                    self.anchor = a;
                    return true;
                }
            }
            0x56 if ctrl => {
                if let Some(s) = crate::clip::get() {
                    // WholeNumberBox: a paste with anything but digits is ignored
                    if self.digits_only && !s.chars().all(|c| c.is_ascii_digit()) {
                        return false;
                    }
                    return self.insert(&s);
                }
            }
            _ => {}
        }
        false
    }
}
