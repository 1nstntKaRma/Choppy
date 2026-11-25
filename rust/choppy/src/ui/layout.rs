//! A small WPF: the windows are trees of the elements their XAML uses (Border, StackPanel, Grid, WrapPanel,
//! TextBlock, shapes, ScrollViewer), laid out with WPF's own measure / arrange rules and its UseLayoutRounding
//! arithmetic (DIP doubles, Math.Round to even at the screen's scale, margins and borders rounded, offsets rounded
//! relative to the parent, a Grid's sizes rounded with its rounding errors spread), so every element lands on the
//! pixels WPF puts it on. A tree is built again for each paint from the window's state (immediate mode): hover,
//! pressed and focus looks are chosen while building, and the arranged tree gives the hit rectangles.

use crate::sys::gfx::{Argb, Canvas, Font, Gfx, Image, Rect, Text, TextAlign, TextOpts};
use crate::sys::win::Cursor;

pub const INF: f64 = f64::INFINITY;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Th {
    pub l: f64,
    pub t: f64,
    pub r: f64,
    pub b: f64,
}

impl Th {
    pub const fn new(l: f64, t: f64, r: f64, b: f64) -> Th {
        Th { l, t, r, b }
    }
    pub const fn all(v: f64) -> Th {
        Th { l: v, t: v, r: v, b: v }
    }
    /// "h,v" as XAML writes two numbers.
    pub const fn hv(h: f64, v: f64) -> Th {
        Th { l: h, t: v, r: h, b: v }
    }
    pub fn w(&self) -> f64 {
        self.l + self.r
    }
    pub fn h(&self) -> f64 {
        self.t + self.b
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Al {
    #[default]
    Stretch,
    Start,
    Center,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Len {
    Auto,
    Px(f64),
    Star(f64),
    /// Auto, the same width as every other column of its group in this grid (SharedSizeGroup).
    Shared(u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Vis {
    #[default]
    Visible,
    /// Takes its room, isn't drawn.
    Hidden,
    Collapsed,
}

#[derive(Clone, Debug)]
pub enum Fill {
    Solid(Argb),
    /// A linear gradient over the element: stops, start and end as fractions of its box (WPF's defaults).
    Linear(Vec<(f64, Argb)>, (f64, f64), (f64, f64)),
}

pub type DrawFn = Box<dyn Fn(&mut Canvas, Rect)>;

pub enum Kind {
    /// A Grid without rows or columns: every child fills it (overlapping).
    Panel,
    Stack { horiz: bool },
    Grid { cols: Vec<Len>, rows: Vec<Len> },
    /// A horizontal WrapPanel.
    Wrap,
    /// `thick`: BorderThickness (one for all sides) unless `sides` gives each side (the settings' rows: 0,0,0,1).
    Border { bg: Option<Fill>, border: Option<Argb>, thick: f64, radius: f64, pad: Th, sides: Option<Th> },
    Text { runs: Vec<(String, Font, Argb)>, wrap: bool, trim: bool, align: TextAlign, line_h: f64, pad: Th },
    /// Drawn by code into its arranged box; `size` is what it asks for (0 = only what Width/Height say).
    Draw { size: (f64, f64), draw: DrawFn },
    Image { img: Option<Image>, opacity: f64, fill: bool },
    /// A vertical ScrollViewer (VerticalScrollBarVisibility Auto) around its one child; `offset` = scrolled DIPs.
    Scroll { offset: f64 },
}

/// One element of the tree.
pub struct El {
    pub kind: Kind,
    pub name: &'static str,
    pub margin: Th,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub min_w: f64,
    pub min_h: f64,
    pub max_w: f64,
    pub max_h: f64,
    pub ha: Al,
    pub va: Al,
    pub vis: Vis,
    pub opacity: f64,
    pub clip: bool,
    // input
    pub id: u32,
    pub cursor: Option<Cursor>,
    pub tip: Option<String>,
    pub focusable: bool,
    pub enabled: bool,
    // grid cell
    pub row: usize,
    pub col: usize,
    pub row_span: usize,
    pub col_span: usize,
    pub children: Vec<El>,
    // layout results
    desired: (f64, f64),
    unclipped: (f64, f64),
    /// Where it was arranged (absolute DIPs; render size).
    pub rect: Rect,
    text: Option<Text>,
    text_w: f64,
    /// Scroll: the content's height and the viewport's (set by arrange).
    pub extent: (f64, f64),
}

impl El {
    pub fn new(kind: Kind) -> El {
        El {
            kind,
            name: "",
            margin: Th::default(),
            width: None,
            height: None,
            min_w: 0.0,
            min_h: 0.0,
            max_w: INF,
            max_h: INF,
            ha: Al::Stretch,
            va: Al::Stretch,
            vis: Vis::Visible,
            opacity: 1.0,
            clip: false,
            id: 0,
            cursor: None,
            tip: None,
            focusable: false,
            enabled: true,
            row: 0,
            col: 0,
            row_span: 1,
            col_span: 1,
            children: Vec::new(),
            desired: (0.0, 0.0),
            unclipped: (0.0, 0.0),
            rect: Rect::default(),
            text: None,
            text_w: -1.0,
            extent: (0.0, 0.0),
        }
    }

    // ---------- builders ----------
    pub fn named(mut self, n: &'static str) -> El {
        self.name = n;
        self
    }
    pub fn margin(mut self, m: Th) -> El {
        self.margin = m;
        self
    }
    pub fn m(self, l: f64, t: f64, r: f64, b: f64) -> El {
        self.margin(Th::new(l, t, r, b))
    }
    pub fn w(mut self, w: f64) -> El {
        self.width = Some(w);
        self
    }
    pub fn h(mut self, h: f64) -> El {
        self.height = Some(h);
        self
    }
    pub fn size(self, w: f64, h: f64) -> El {
        self.w(w).h(h)
    }
    pub fn min_w(mut self, v: f64) -> El {
        self.min_w = v;
        self
    }
    pub fn min_h(mut self, v: f64) -> El {
        self.min_h = v;
        self
    }
    pub fn max_h(mut self, v: f64) -> El {
        self.max_h = v;
        self
    }
    pub fn max_w(mut self, v: f64) -> El {
        self.max_w = v;
        self
    }
    pub fn ha(mut self, a: Al) -> El {
        self.ha = a;
        self
    }
    pub fn va(mut self, a: Al) -> El {
        self.va = a;
        self
    }
    pub fn center(self) -> El {
        self.ha(Al::Center).va(Al::Center)
    }
    pub fn vcenter(self) -> El {
        self.va(Al::Center)
    }
    pub fn left(self) -> El {
        self.ha(Al::Start)
    }
    pub fn right(self) -> El {
        self.ha(Al::End)
    }
    pub fn top(self) -> El {
        self.va(Al::Start)
    }
    pub fn bottom(self) -> El {
        self.va(Al::End)
    }
    pub fn vis(mut self, v: Vis) -> El {
        self.vis = v;
        self
    }
    /// Visible or Collapsed.
    pub fn shown(mut self, on: bool) -> El {
        self.vis = if on { Vis::Visible } else { Vis::Collapsed };
        self
    }
    pub fn opacity(mut self, o: f64) -> El {
        self.opacity = o;
        self
    }
    pub fn clip(mut self) -> El {
        self.clip = true;
        self
    }
    pub fn id(mut self, id: u32) -> El {
        self.id = id;
        self
    }
    pub fn cursor(mut self, c: Cursor) -> El {
        self.cursor = Some(c);
        self
    }
    pub fn hand(self) -> El {
        self.cursor(Cursor::Hand)
    }
    pub fn tip(mut self, t: impl Into<String>) -> El {
        let t = t.into();
        self.tip = if t.is_empty() { None } else { Some(t) };
        self
    }
    pub fn tip_opt(mut self, t: Option<String>) -> El {
        self.tip = t.filter(|t| !t.is_empty());
        self
    }
    pub fn focusable(mut self, f: bool) -> El {
        self.focusable = f;
        self
    }
    pub fn enabled(mut self, e: bool) -> El {
        self.enabled = e;
        self
    }
    pub fn cell(mut self, row: usize, col: usize) -> El {
        self.row = row;
        self.col = col;
        self
    }
    pub fn row(mut self, r: usize) -> El {
        self.row = r;
        self
    }
    pub fn col(mut self, c: usize) -> El {
        self.col = c;
        self
    }
    pub fn span(mut self, rows: usize, cols: usize) -> El {
        self.row_span = rows.max(1);
        self.col_span = cols.max(1);
        self
    }
    pub fn col_span(mut self, c: usize) -> El {
        self.col_span = c.max(1);
        self
    }
    pub fn child(mut self, c: El) -> El {
        self.children.push(c);
        self
    }
    pub fn kids(mut self, cs: impl IntoIterator<Item = El>) -> El {
        self.children.extend(cs);
        self
    }
    pub fn push(&mut self, c: El) {
        self.children.push(c);
    }

    fn collapsed(&self) -> bool {
        self.vis == Vis::Collapsed
    }
}

// ---------- element makers ----------

pub fn panel() -> El {
    El::new(Kind::Panel)
}
pub fn vstack() -> El {
    El::new(Kind::Stack { horiz: false })
}
pub fn hstack() -> El {
    El::new(Kind::Stack { horiz: true })
}
pub fn wrap() -> El {
    El::new(Kind::Wrap)
}
pub fn grid(cols: &[Len], rows: &[Len]) -> El {
    El::new(Kind::Grid { cols: cols.to_vec(), rows: rows.to_vec() })
}
pub fn border(bg: Option<Fill>, border_c: Option<Argb>, thick: f64, radius: f64, pad: Th) -> El {
    El::new(Kind::Border { bg, border: border_c, thick, radius, pad, sides: None })
}
/// A Border with only a background (and corner radius).
pub fn fill(c: Argb, radius: f64) -> El {
    border(Some(Fill::Solid(c)), None, 0.0, radius, Th::default())
}
pub fn text(s: impl Into<String>, f: Font, c: Argb) -> El {
    runs(vec![(s.into(), f, c)])
}
pub fn runs(r: Vec<(String, Font, Argb)>) -> El {
    El::new(Kind::Text { runs: r, wrap: false, trim: false, align: TextAlign::Left, line_h: 0.0, pad: Th::default() })
}
pub fn draw(size: (f64, f64), f: impl Fn(&mut Canvas, Rect) + 'static) -> El {
    El::new(Kind::Draw { size, draw: Box::new(f) })
}
pub fn ellipse(w: f64, h: f64, c: Argb) -> El {
    draw((0.0, 0.0), move |cv, r| cv.ellipse(r.x + r.w / 2.0, r.y + r.h / 2.0, r.w / 2.0, r.h / 2.0, c)).size(w, h)
}
pub fn rect_el(w: f64, h: f64, c: Argb) -> El {
    draw((0.0, 0.0), move |cv, r| cv.fill(r, c)).size(w, h)
}
pub fn image(img: Option<Image>) -> El {
    El::new(Kind::Image { img, opacity: 1.0, fill: false })
}
pub fn scroll(offset: f64, content: El) -> El {
    El::new(Kind::Scroll { offset }).child(content)
}

impl El {
    /// A Border's BorderThickness side by side ("0,0,0,1").
    pub fn sides(mut self, t: Th) -> El {
        if let Kind::Border { sides, .. } = &mut self.kind {
            *sides = Some(t);
        }
        self
    }

    /// TextBlock: TextWrapping="Wrap".
    pub fn wrapped(mut self) -> El {
        if let Kind::Text { wrap, .. } = &mut self.kind {
            *wrap = true;
        }
        self
    }
    /// TextBlock: TextTrimming="CharacterEllipsis".
    pub fn trimmed(mut self) -> El {
        if let Kind::Text { trim, .. } = &mut self.kind {
            *trim = true;
        }
        self
    }
    pub fn align(mut self, a: TextAlign) -> El {
        if let Kind::Text { align, .. } = &mut self.kind {
            *align = a;
        }
        self
    }
    pub fn line_height(mut self, h: f64) -> El {
        if let Kind::Text { line_h, .. } = &mut self.kind {
            *line_h = h;
        }
        self
    }
    /// TextBlock.Padding.
    pub fn tpad(mut self, p: Th) -> El {
        if let Kind::Text { pad, .. } = &mut self.kind {
            *pad = p;
        }
        self
    }
    /// Image: Stretch="UniformToFill" (else Fill to its size), and its opacity.
    pub fn uniform_to_fill(mut self, o: f64) -> El {
        if let Kind::Image { fill, opacity, .. } = &mut self.kind {
            *fill = true;
            *opacity = o;
        }
        self
    }
}

// ================= layout =================

/// A Border's thickness on each side, rounded like the layout rounds it.
fn border_sides(lx: &Lx, thick: f64, sides: Option<Th>) -> Th {
    let t = sides.unwrap_or(Th::all(thick));
    Th::new(lx.rnd(t.l), lx.rnd(t.t), lx.rnd(t.r), lx.rnd(t.b))
}

/// The scale (pixels per DIP) and what measures text.
pub struct Lx<'a> {
    pub g: &'a Gfx,
    pub scale: f64,
    /// UseLayoutRounding (the XAML windows); windows built in code (the dialogs) have it off.
    pub round: bool,
    /// Text in WPF's Ideal formatting (no TextFormattingMode="Display": the dialogs).
    pub ideal: bool,
}

impl Lx<'_> {
    /// RoundLayoutValue: to whole pixels, halves to even.
    pub fn rnd(&self, v: f64) -> f64 {
        if !v.is_finite() || !self.round {
            return v;
        }
        if (self.scale - 1.0).abs() < 1e-9 {
            v.round_ties_even()
        } else {
            let n = (v * self.scale).round_ties_even() / self.scale;
            if n.is_finite() { n } else { v }
        }
    }
}

fn close(a: f64, b: f64) -> bool {
    if a == b {
        return true;
    }
    let eps = (a.abs() + b.abs() + 10.0) * 2.220446049250313e-16;
    let d = a - b;
    -eps < d && eps > d
}

/// FrameworkElement's MinMax with Width / Height, rounded like the layout.
fn min_max(e: &El, lx: &Lx) -> (f64, f64, f64, f64) {
    let mut max_h = e.max_h;
    let mut min_h = e.min_h;
    let hh = e.height.unwrap_or(INF);
    max_h = hh.min(max_h).max(min_h);
    let hh = e.height.unwrap_or(0.0);
    min_h = max_h.min(hh).max(min_h);
    let mut max_w = e.max_w;
    let mut min_w = e.min_w;
    let ww = e.width.unwrap_or(INF);
    max_w = ww.min(max_w).max(min_w);
    let ww = e.width.unwrap_or(0.0);
    min_w = max_w.min(ww).max(min_w);
    (lx.rnd(min_w), lx.rnd(max_w), lx.rnd(min_h), lx.rnd(max_h))
}

impl El {
    pub fn desired(&self) -> (f64, f64) {
        self.desired
    }
    /// What the element itself wanted before the layout rounded it (tests).
    pub fn unclipped(&self) -> (f64, f64) {
        self.unclipped
    }

    /// MeasureCore.
    pub fn measure(&mut self, lx: &Lx, avail: (f64, f64)) {
        if self.collapsed() {
            self.desired = (0.0, 0.0);
            self.unclipped = (0.0, 0.0);
            return;
        }
        let mw = lx.rnd(self.margin.w());
        let mh = lx.rnd(self.margin.h());
        let (min_w, max_w, min_h, max_h) = min_max(self, lx);
        let fw = (avail.0 - mw).max(0.0).min(max_w).max(min_w);
        let fh = (avail.1 - mh).max(0.0).min(max_h).max(min_h);
        let d = self.measure_override(lx, (fw, fh));
        let mut d = (d.0.max(min_w), d.1.max(min_h));
        self.unclipped = d;
        if d.0 > max_w {
            d.0 = max_w;
        }
        if d.1 > max_h {
            d.1 = max_h;
        }
        let mut cw = d.0 + mw;
        let mut ch = d.1 + mh;
        if cw > avail.0 {
            cw = avail.0;
        }
        if ch > avail.1 {
            ch = avail.1;
        }
        self.desired = (lx.rnd(cw).max(0.0), lx.rnd(ch).max(0.0));
    }

    /// ArrangeCore: `fr` is the slot in the parent's own coordinates (as WPF passes it, so offsets round exactly
    /// as WPF rounds them); `parent` = the parent's absolute corner (drawing and hits use absolute DIPs).
    pub fn arrange(&mut self, lx: &Lx, fr: Rect, parent: (f64, f64)) {
        if self.collapsed() {
            self.rect = Rect::new(parent.0 + fr.x, parent.1 + fr.y, 0.0, 0.0);
            return;
        }
        let mw = lx.rnd(self.margin.w());
        let mh = lx.rnd(self.margin.h());
        let mut aw = (fr.w - mw).max(0.0);
        let mut ah = (fr.h - mh).max(0.0);
        if self.ha != Al::Stretch {
            aw = self.unclipped.0;
        }
        if self.va != Al::Stretch {
            ah = self.unclipped.1;
        }
        let (_, max_w, _, max_h) = min_max(self, lx);
        let emw = self.unclipped.0.max(max_w);
        if emw < aw {
            aw = emw;
        }
        let emh = self.unclipped.1.max(max_h);
        if emh < ah {
            ah = emh;
        }
        // the corner first (children are placed relative to it), then the content
        let client = (lx.rnd((fr.w - mw).max(0.0)), lx.rnd((fr.h - mh).max(0.0)));
        let ink_guess = (lx.rnd(aw).min(max_w), lx.rnd(ah).min(max_h));
        let rel = (fr.x, fr.y);
        let off = self.offset(lx, client, ink_guess, rel);
        let corner = (parent.0 + off.0, parent.1 + off.1);
        // WPF hands ArrangeOverride a size already on whole pixels (24.4 DIPs = 30.5 px becomes 31 px at 125 %)
        let ink = self.arrange_override(lx, (lx.rnd(aw), lx.rnd(ah)), corner);
        let rs = (lx.rnd(ink.0), lx.rnd(ink.1));
        let clipped = (rs.0.min(max_w), rs.1.min(max_h));
        let corner = if clipped != ink_guess {
            // the content came out another size than asked: placed again by its real size
            let off = self.offset(lx, client, clipped, rel);
            let c2 = (parent.0 + off.0, parent.1 + off.1);
            if c2 != corner {
                self.shift(c2.0 - corner.0, c2.1 - corner.1);
            }
            c2
        } else {
            corner
        };
        self.rect = Rect::new(corner.0, corner.1, rs.0, rs.1);
    }

    fn offset(&self, lx: &Lx, client: (f64, f64), ink: (f64, f64), rel: (f64, f64)) -> (f64, f64) {
        let mut ha = self.ha;
        let mut va = self.va;
        if ha == Al::Stretch && ink.0 > client.0 {
            ha = Al::Start;
        }
        if va == Al::Stretch && ink.1 > client.1 {
            va = Al::Start;
        }
        let ox = match ha {
            Al::Center | Al::Stretch => (client.0 - ink.0) * 0.5,
            Al::End => client.0 - ink.0,
            Al::Start => 0.0,
        };
        let oy = match va {
            Al::Center | Al::Stretch => (client.1 - ink.1) * 0.5,
            Al::End => client.1 - ink.1,
            Al::Start => 0.0,
        };
        (lx.rnd(ox + rel.0 + self.margin.l), lx.rnd(oy + rel.1 + self.margin.t))
    }

    /// Moves an arranged subtree.
    fn shift(&mut self, dx: f64, dy: f64) {
        self.rect.x += dx;
        self.rect.y += dy;
        for c in self.children.iter_mut() {
            c.shift(dx, dy);
        }
    }

    fn measure_override(&mut self, lx: &Lx, c: (f64, f64)) -> (f64, f64) {
        match &mut self.kind {
            Kind::Panel => {
                let mut d = (0f64, 0f64);
                for ch in self.children.iter_mut() {
                    ch.measure(lx, c);
                    d.0 = d.0.max(ch.desired.0);
                    d.1 = d.1.max(ch.desired.1);
                }
                d
            }
            Kind::Stack { horiz } => {
                let horiz = *horiz;
                let mut d = (0f64, 0f64);
                for ch in self.children.iter_mut() {
                    if horiz {
                        ch.measure(lx, (INF, c.1));
                        d.0 += ch.desired.0;
                        d.1 = d.1.max(ch.desired.1);
                    } else {
                        ch.measure(lx, (c.0, INF));
                        d.0 = d.0.max(ch.desired.0);
                        d.1 += ch.desired.1;
                    }
                }
                d
            }
            Kind::Wrap => {
                let (mut line_w, mut line_h, mut w, mut h) = (0f64, 0f64, 0f64, 0f64);
                for ch in self.children.iter_mut() {
                    ch.measure(lx, c);
                    let s = ch.desired;
                    if line_w + s.0 > c.0 && !close(line_w + s.0, c.0) && line_w > 0.0 {
                        w = w.max(line_w);
                        h += line_h;
                        line_w = s.0;
                        line_h = s.1;
                    } else {
                        line_w += s.0;
                        line_h = line_h.max(s.1);
                    }
                }
                (w.max(line_w), h + line_h)
            }
            Kind::Border { thick, pad, sides, .. } => {
                let b = border_sides(lx, *thick, *sides);
                let comb = (b.w() + pad.w(), b.h() + pad.h());
                if let Some(ch) = self.children.first_mut() {
                    ch.measure(lx, ((c.0 - comb.0).max(0.0), (c.1 - comb.1).max(0.0)));
                    (ch.desired.0 + comb.0, ch.desired.1 + comb.1)
                } else {
                    comb
                }
            }
            Kind::Text { runs, wrap, trim, align, line_h, pad } => {
                let avail = (c.0 - pad.w()).max(0.0);
                let r: Vec<(&str, Font, Argb)> = runs.iter().map(|r| (r.0.as_str(), r.1, r.2)).collect();
                let o = TextOpts {
                    max_w: if (*wrap || *trim) && avail.is_finite() { avail.max(0.01) } else { 0.0 },
                    trim: *trim,
                    wrap: *wrap,
                    align: *align,
                    line_height: *line_h,
                    ideal: lx.ideal,
                };
                let t = lx.g.text(&r, &o, lx.scale);
                let w = if *wrap && avail.is_finite() { t.w.min(avail) } else { t.w };
                let w = if *trim && avail.is_finite() { w.min(avail) } else { w };
                let d = (w + pad.w(), t.h + pad.h());
                self.text_w = o.max_w;
                self.text = Some(t);
                d
            }
            Kind::Draw { size, .. } => *size,
            Kind::Image { .. } => (0.0, 0.0),
            Kind::Scroll { .. } => {
                let Some(ch) = self.children.first_mut() else { return (0.0, 0.0) };
                // without the bar first; a content taller than the room gets the bar (8 wide) and is measured again
                ch.measure(lx, (c.0, INF));
                if ch.desired.1 > c.1 && !close(ch.desired.1, c.1) {
                    ch.measure(lx, ((c.0 - 8.0).max(0.0), INF));
                    (ch.desired.0 + 8.0, ch.desired.1.min(c.1))
                } else {
                    (ch.desired.0, ch.desired.1.min(c.1))
                }
            }
            Kind::Grid { .. } => self.grid_measure(lx, c),
        }
    }

    fn arrange_override(&mut self, lx: &Lx, s: (f64, f64), at: (f64, f64)) -> (f64, f64) {
        match &mut self.kind {
            Kind::Panel => {
                for ch in self.children.iter_mut() {
                    ch.arrange(lx, Rect::new(0.0, 0.0, s.0, s.1), at);
                }
                s
            }
            Kind::Stack { horiz } => {
                let horiz = *horiz;
                let mut pos = 0.0;
                for ch in self.children.iter_mut() {
                    if horiz {
                        let w = ch.desired.0;
                        ch.arrange(lx, Rect::new(pos, 0.0, w, s.1.max(ch.desired.1)), at);
                        pos += w;
                    } else {
                        let h = ch.desired.1;
                        ch.arrange(lx, Rect::new(0.0, pos, s.0.max(ch.desired.0), h), at);
                        pos += h;
                    }
                }
                s
            }
            Kind::Wrap => {
                // lines as measured; each child its own width, the line's height
                let mut lines: Vec<(usize, usize, f64)> = Vec::new();
                let (mut start, mut line_w, mut line_h) = (0usize, 0f64, 0f64);
                for (i, ch) in self.children.iter().enumerate() {
                    let d = ch.desired;
                    if line_w + d.0 > s.0 && !close(line_w + d.0, s.0) && line_w > 0.0 {
                        lines.push((start, i, line_h));
                        start = i;
                        line_w = d.0;
                        line_h = d.1;
                    } else {
                        line_w += d.0;
                        line_h = line_h.max(d.1);
                    }
                }
                lines.push((start, self.children.len(), line_h));
                let mut y = 0.0;
                for (a, b, lh) in lines {
                    let mut x = 0.0;
                    for ch in self.children[a..b].iter_mut() {
                        let w = ch.desired.0;
                        ch.arrange(lx, Rect::new(x, y, w, lh), at);
                        x += w;
                    }
                    y += lh;
                }
                s
            }
            Kind::Border { thick, pad, sides, .. } => {
                let b = border_sides(lx, *thick, *sides);
                let pad = *pad;
                if let Some(ch) = self.children.first_mut() {
                    // WPF's HelperDeflateRect twice (the border, then the padding), each side in its order, so the
                    // float error is WPF's too
                    let (iw, ih) = ((s.0 - b.l - b.r).max(0.0), (s.1 - b.t - b.b).max(0.0));
                    let inner = Rect::new(b.l + pad.l, b.t + pad.t, (iw - pad.l - pad.r).max(0.0), (ih - pad.t - pad.b).max(0.0));
                    ch.arrange(lx, inner, at);
                }
                s
            }
            Kind::Text { runs, wrap, trim, align, line_h, pad } => {
                // a trimmed or wrapped text is laid out again for the width it really got
                let avail = (s.0 - pad.w()).max(0.0);
                if (*wrap || *trim || *align != TextAlign::Left) && (avail - self.text_w).abs() > 1e-6 {
                    let r: Vec<(&str, Font, Argb)> = runs.iter().map(|r| (r.0.as_str(), r.1, r.2)).collect();
                    let o = TextOpts { max_w: avail.max(0.01), trim: *trim, wrap: *wrap, align: *align, line_height: *line_h, ideal: lx.ideal };
                    self.text = Some(lx.g.text(&r, &o, lx.scale));
                    self.text_w = avail;
                }
                s
            }
            Kind::Draw { .. } | Kind::Image { .. } => s,
            Kind::Scroll { offset } => {
                let off = *offset;
                let Some(ch) = self.children.first_mut() else { return s };
                let content_h = ch.desired.1;
                let bar = content_h > s.1 && !close(content_h, s.1);
                let vw = if bar { (s.0 - 8.0).max(0.0) } else { s.0 };
                let max_off = (content_h - s.1).max(0.0);
                let off = lx.rnd(off.clamp(0.0, max_off));
                ch.arrange(lx, Rect::new(0.0, -off, vw.max(ch.desired.0), content_h), at);
                self.extent = (content_h, s.1);
                if let Kind::Scroll { offset } = &mut self.kind {
                    *offset = off;
                }
                s
            }
            Kind::Grid { .. } => self.grid_arrange(lx, s, at),
        }
    }

    // ---------- Grid ----------

    fn grid_defs(&self) -> (Vec<Len>, Vec<Len>) {
        match &self.kind {
            Kind::Grid { cols, rows } => {
                let c = if cols.is_empty() { vec![Len::Star(1.0)] } else { cols.clone() };
                let r = if rows.is_empty() { vec![Len::Star(1.0)] } else { rows.clone() };
                (c, r)
            }
            _ => (vec![Len::Star(1.0)], vec![Len::Star(1.0)]),
        }
    }

    /// Column (or row) sizes for `avail`: pixels, autos from the children's desired sizes, stars share the rest
    /// (with `avail` infinite a star sizes to its content like Auto).
    /// `px` rounds a pixel definition (measure: WPF's layout rounding of the user size; arrange: as given, then all
    /// rounded together).
    fn resolve(defs: &[Len], avail: f64, kids: &[(usize, usize, f64)], px: &dyn Fn(f64) -> f64) -> Vec<f64> {
        let mut size: Vec<f64> = defs.iter().map(|d| if let Len::Px(v) = d { px(*v) } else { 0.0 }).collect();
        let star_like = |d: &Len| matches!(d, Len::Star(_)) && avail.is_finite();
        // single-cell children first, then the spanning ones grow the auto definitions they cover
        // (a pixel definition too, as WPF's UpdateMinSize: a child clipped to 46 DIPs and rounded to 58 px at 125 %
        // makes its row 58 px)
        for &(i, _, d) in kids.iter().filter(|k| k.1 == 1) {
            if i < defs.len() && !star_like(&defs[i]) {
                size[i] = size[i].max(d);
            }
        }
        for &(i, span, d) in kids.iter().filter(|k| k.1 > 1) {
            let end = (i + span).min(defs.len());
            if (i..end).any(|j| matches!(defs[j], Len::Star(_))) && avail.is_finite() {
                continue;
            }
            let have: f64 = size[i..end].iter().sum();
            if d > have {
                let autos: Vec<usize> = (i..end).filter(|&j| !matches!(defs[j], Len::Px(_))).collect();
                if !autos.is_empty() {
                    let add = (d - have) / autos.len() as f64;
                    for j in autos {
                        size[j] += add;
                    }
                }
            }
        }
        // SharedSizeGroup: every column of a group as wide as the widest
        for g in 0..=255u8 {
            let m = defs.iter().zip(size.iter()).filter(|(d, _)| **d == Len::Shared(g)).map(|(_, s)| *s).fold(-1.0, f64::max);
            if m >= 0.0 {
                for (i, d) in defs.iter().enumerate() {
                    if *d == Len::Shared(g) {
                        size[i] = m;
                    }
                }
            }
            if !defs.iter().any(|d| matches!(d, Len::Shared(x) if *x > g)) {
                break;
            }
        }
        if avail.is_finite() {
            let fixed: f64 = defs.iter().zip(size.iter()).filter(|(d, _)| !matches!(d, Len::Star(_))).map(|(_, s)| *s).sum();
            let stars: f64 = defs.iter().map(|d| if let Len::Star(w) = d { *w } else { 0.0 }).sum();
            let rest = (avail - fixed).max(0.0);
            for (i, d) in defs.iter().enumerate() {
                if let Len::Star(w) = d {
                    size[i] = if stars > 0.0 { rest * w / stars } else { 0.0 };
                }
            }
        }
        size
    }

    fn grid_measure(&mut self, lx: &Lx, c: (f64, f64)) -> (f64, f64) {
        let (cols, rows) = self.grid_defs();
        // widths: auto-only cells measured with all the width they like
        let mut kids_w = Vec::new();
        for ch in self.children.iter_mut() {
            let (cs, ce) = (ch.col.min(cols.len() - 1), (ch.col + ch.col_span).min(cols.len()));
            let auto_only = (cs..ce).all(|j| matches!(cols[j], Len::Auto | Len::Shared(_))) || (cs..ce).all(|j| !matches!(cols[j], Len::Star(_)));
            if auto_only || !c.0.is_finite() {
                ch.measure(lx, (INF, INF));
                kids_w.push((cs, ce - cs, ch.desired.0));
            }
        }
        let r = |v: f64| v;
        let cw = Self::resolve(&cols, c.0, &kids_w, &r);
        let mut kids_h = Vec::new();
        for ch in self.children.iter_mut() {
            let (cs, ce) = (ch.col.min(cols.len() - 1), (ch.col + ch.col_span).min(cols.len()));
            let (rs, re) = (ch.row.min(rows.len() - 1), (ch.row + ch.row_span).min(rows.len()));
            let w: f64 = cw[cs..ce].iter().sum();
            let auto_rows = (rs..re).all(|j| !matches!(rows[j], Len::Px(_)));
            let h = if auto_rows || !c.1.is_finite() { INF } else { (rs..re).map(|j| if let Len::Px(v) = rows[j] { v } else { 0.0 }).sum() };
            ch.measure(lx, (w, h));
            kids_h.push((rs, re - rs, ch.desired.1));
        }
        // the widths again with the star cells' real sizes when the width is open
        let rh = Self::resolve(&rows, c.1, &kids_h, &r);
        let dw: f64 = if c.0.is_finite() {
            // a star column wants what its cells want
            let mut sizes = cw.clone();
            for (i, d) in cols.iter().enumerate() {
                if matches!(d, Len::Star(_)) {
                    sizes[i] = 0.0;
                }
            }
            for ch in self.children.iter() {
                if ch.col_span == 1 && ch.col < cols.len() && matches!(cols[ch.col], Len::Star(_)) {
                    sizes[ch.col] = sizes[ch.col].max(ch.desired.0);
                }
            }
            sizes.iter().sum()
        } else {
            cw.iter().sum()
        };
        let dh: f64 = if c.1.is_finite() {
            let mut sizes = rh.clone();
            for (i, d) in rows.iter().enumerate() {
                if matches!(d, Len::Star(_)) {
                    sizes[i] = 0.0;
                }
            }
            for ch in self.children.iter() {
                if ch.row_span == 1 && ch.row < rows.len() && matches!(rows[ch.row], Len::Star(_)) {
                    sizes[ch.row] = sizes[ch.row].max(ch.desired.1);
                }
            }
            sizes.iter().sum()
        } else {
            rh.iter().sum()
        };
        (dw, dh)
    }

    /// Grid.SetFinalSize's rounding: each size rounded, the rounding errors spread so they add up to the final
    /// size (only when a star definition takes up the rest).
    fn round_defs(lx: &Lx, defs: &[Len], sizes: &mut [f64], final_size: f64) {
        let mut err = vec![0.0; sizes.len()];
        let mut taken = 0.0;
        for i in 0..sizes.len() {
            let r = lx.rnd(sizes[i]);
            err[i] = r - sizes[i];
            sizes[i] = r;
            taken += r;
        }
        if !defs.iter().any(|d| matches!(d, Len::Star(_))) || close(taken, final_size) {
            return;
        }
        let mut idx: Vec<usize> = (0..sizes.len()).collect();
        idx.sort_by(|&a, &b| err[a].partial_cmp(&err[b]).unwrap_or(std::cmp::Ordering::Equal));
        let inc = 1.0 / lx.scale;
        let mut adj = taken;
        if taken > final_size {
            let mut i = sizes.len() as isize - 1;
            while adj > final_size && !close(adj, final_size) && i >= 0 {
                let d = idx[i as usize];
                let f = (sizes[d] - inc).max(0.0);
                if f < sizes[d] {
                    adj -= inc;
                }
                sizes[d] = f;
                i -= 1;
            }
        } else {
            let mut i = 0;
            while adj < final_size && !close(adj, final_size) && i < sizes.len() {
                let d = idx[i];
                sizes[d] += inc;
                adj += inc;
                i += 1;
            }
        }
    }

    fn grid_arrange(&mut self, lx: &Lx, s: (f64, f64), at: (f64, f64)) -> (f64, f64) {
        let (cols, rows) = self.grid_defs();
        let kids_w: Vec<(usize, usize, f64)> = self
            .children
            .iter()
            .filter(|ch| !ch.collapsed())
            .map(|ch| {
                let cs = ch.col.min(cols.len() - 1);
                (cs, (ch.col + ch.col_span).min(cols.len()) - cs, ch.desired.0)
            })
            .collect();
        let kids_h: Vec<(usize, usize, f64)> = self
            .children
            .iter()
            .filter(|ch| !ch.collapsed())
            .map(|ch| {
                let rs = ch.row.min(rows.len() - 1);
                (rs, (ch.row + ch.row_span).min(rows.len()) - rs, ch.desired.1)
            })
            .collect();
        let same = |v: f64| v;
        let mut cw = Self::resolve(&cols, s.0, &kids_w, &same);
        let mut rh = Self::resolve(&rows, s.1, &kids_h, &same);
        Self::round_defs(lx, &cols, &mut cw, s.0);
        Self::round_defs(lx, &rows, &mut rh, s.1);
        let cx: Vec<f64> = cw.iter().scan(0.0, |a, w| {
            let v = *a;
            *a += w;
            Some(v)
        }).collect();
        let ry: Vec<f64> = rh.iter().scan(0.0, |a, h| {
            let v = *a;
            *a += h;
            Some(v)
        }).collect();
        for ch in self.children.iter_mut() {
            let (cs, ce) = (ch.col.min(cols.len() - 1), (ch.col + ch.col_span).min(cols.len()));
            let (rs, re) = (ch.row.min(rows.len() - 1), (ch.row + ch.row_span).min(rows.len()));
            let w: f64 = cw[cs..ce].iter().sum();
            let h: f64 = rh[rs..re].iter().sum();
            ch.arrange(lx, Rect::new(cx[cs], ry[rs], w, h), at);
        }
        s
    }

    // ================= drawing and input =================

    pub fn draw(&self, cv: &mut Canvas) {
        if self.vis != Vis::Visible {
            return;
        }
        let r = self.rect;
        cv.push_opacity(self.opacity);
        if self.clip {
            cv.push_clip(r);
        }
        match &self.kind {
            Kind::Border { bg, border, sides: Some(sd), .. } => {
                // a border with sides of their own (no corners): each side a filled strip, the background inside
                let rd = |v: f64| if cv.snap { (v * cv.scale).round_ties_even() / cv.scale } else { v };
                let sd = Th::new(rd(sd.l), rd(sd.t), rd(sd.r), rd(sd.b));
                if let Some(Fill::Solid(c)) = bg {
                    cv.fill(r.inset(sd.l, sd.t, sd.r, sd.b), *c);
                }
                if let Some(c) = border {
                    for s in [
                        Rect::new(r.x, r.y, sd.l, r.h),
                        Rect::new(r.x, r.y, r.w, sd.t),
                        Rect::new(r.r() - sd.r, r.y, sd.r, r.h),
                        Rect::new(r.x, r.b() - sd.b, r.w, sd.b),
                    ] {
                        if s.w > 0.0 && s.h > 0.0 {
                            cv.fill(s, *c);
                        }
                    }
                }
            }
            Kind::Border { bg, border, thick, radius, .. } => {
                let b = if *thick > 0.0 && cv.snap { (thick * cv.scale).round_ties_even().max(0.0) / cv.scale } else { *thick };
                // Border.OnRender: the pen half its width inside the edge (corner radius as given), the background
                // inside the border with the inner radius
                if let (Some(bc), true) = (border, b > 0.0) {
                    let half = b / 2.0;
                    let rr = Rect::new(r.x + half, r.y + half, (r.w - b).max(0.0), (r.h - b).max(0.0));
                    cv.stroke_round(rr, *radius, *bc, b);
                }
                if let Some(bg) = bg {
                    let (inner, rad) = if border.is_some() && b > 0.0 {
                        (r.inset(b, b, b, b), (radius - 0.5 * thick).max(0.0))
                    } else {
                        (r, *radius)
                    };
                    if inner.w > 0.0 && inner.h > 0.0 {
                        match bg {
                            Fill::Solid(c) => cv.fill_round(inner, rad, *c),
                            Fill::Linear(stops, f, t) => cv.fill_gradient(inner, rad, stops, *f, *t),
                        }
                    }
                }
            }
            Kind::Text { pad, .. } => {
                if let Some(t) = &self.text {
                    cv.text(t, r.x + pad.l, r.y + pad.t);
                }
            }
            Kind::Draw { draw, .. } => draw(cv, r),
            Kind::Image { img: Some(img), opacity, fill } => {
                if *fill {
                    // UniformToFill: the picture covers the box, centered, cut by the box
                    let k = (r.w / img.w as f64).max(r.h / img.h as f64);
                    let (w, h) = (img.w as f64 * k, img.h as f64 * k);
                    cv.push_clip(r);
                    cv.image(img, Rect::new(r.x + (r.w - w) / 2.0, r.y + (r.h - h) / 2.0, w, h), *opacity);
                    cv.pop_clip();
                } else {
                    cv.image(img, r, *opacity);
                }
            }
            _ => {}
        }
        if let Kind::Scroll { offset } = &self.kind {
            cv.push_clip(r);
            for ch in self.children.iter() {
                ch.draw(cv);
            }
            cv.pop_clip();
            let (content, view) = self.extent;
            if content > view + 0.5 {
                // the thumb (Theme.xaml ThumbStyle: #454C56, corners 3, Margin 2,0) in an 8 wide bar
                let th = (view * view / content).max(16.0).min(view);
                let ty = r.y + (view - th) * (offset / (content - view)).clamp(0.0, 1.0);
                cv.fill_round(Rect::new(r.r() - 8.0 + 2.0, ty, 4.0, th), 3.0, 0xFF454C56);
            }
        } else {
            for ch in self.children.iter() {
                ch.draw(cv);
            }
        }
        if self.clip {
            cv.pop_clip();
        }
        cv.pop_opacity();
    }

    /// Every element with an id, in drawing order (the last on top), with its rectangle cut to scroll viewports.
    pub fn hits(&self, out: &mut Vec<Hit>, clip: Rect) {
        if self.vis != Vis::Visible {
            return;
        }
        let r = self.rect.intersect(&clip);
        if self.id != 0 && self.enabled && r.w > 0.0 && r.h > 0.0 {
            out.push(Hit { id: self.id, rect: r, cursor: self.cursor.unwrap_or_default(), tip: self.tip.clone(), focusable: self.focusable });
        } else if self.id == 0 && self.tip.is_some() && r.w > 0.0 {
            // a tooltip on a plain element (a TextBlock): hoverable, not clickable
            out.push(Hit { id: 0, rect: r, cursor: self.cursor.unwrap_or_default(), tip: self.tip.clone(), focusable: false });
        }
        if !self.enabled {
            return;
        }
        let inner = if matches!(self.kind, Kind::Scroll { .. }) { r } else { clip };
        for ch in self.children.iter() {
            ch.hits(out, inner);
        }
    }

    /// Finds an element by name (tests, dumps).
    pub fn find(&self, n: &str) -> Option<&El> {
        if self.name == n {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(n))
    }

    /// Every named element's rectangle in pixels, like tools/wpflayout.ps1 prints WPF's.
    pub fn dump(&self, scale: f64, depth: usize, out: &mut Vec<String>) {
        if self.vis == Vis::Collapsed {
            return;
        }
        let label = if !self.name.is_empty() {
            Some(self.name.to_string())
        } else if let Kind::Text { runs, .. } = &self.kind {
            Some(format!("TB:{}", runs.iter().map(|r| r.0.as_str()).collect::<String>()))
        } else {
            None
        };
        if let Some(l) = label {
            out.push(format!(
                "{:<16} x={:>7.2} y={:>7.2} w={:>7.2} h={:>7.2}",
                format!("{}{}", "  ".repeat(depth.min(6)), l),
                self.rect.x * scale,
                self.rect.y * scale,
                self.rect.w * scale,
                self.rect.h * scale
            ));
        }
        for c in self.children.iter() {
            c.dump(scale, depth + 1, out);
        }
    }
}

#[derive(Clone)]
pub struct Hit {
    pub id: u32,
    pub rect: Rect,
    pub cursor: Cursor,
    pub tip: Option<String>,
    pub focusable: bool,
}

/// Lays a whole window out: `w` x `h` DIPs (h infinite = SizeToContent height: the window is the content's height
/// rounded up to whole pixels, as WPF sizes it, and the content is arranged in that). Returns its height.
pub fn layout(root: &mut El, lx: &Lx, w: f64, h: f64) -> f64 {
    root.measure(lx, (w, h));
    let hh = if h.is_finite() { h } else { content_px(root.desired.1, lx.scale) / lx.scale };
    root.arrange(lx, Rect::new(0.0, 0.0, w, hh), (0.0, 0.0));
    hh
}

/// A SizeToContent window's height in pixels: its content's, rounded up (a float error above a whole pixel, as
/// pixels -> DIPs -> pixels leaves at 175 %, doesn't count).
pub fn content_px(dip: f64, scale: f64) -> f64 {
    (dip * scale - 1e-7).ceil()
}

/// Like `layout`, and a width that is infinite becomes the content's own (pop-ups, tooltips). Returns (w, h).
pub fn layout_auto(root: &mut El, lx: &Lx, w: f64, h: f64) -> (f64, f64) {
    root.measure(lx, (w, h));
    let ww = if w.is_finite() { w } else { root.desired.0 };
    let hh = if h.is_finite() { h } else { content_px(root.desired.1, lx.scale) / lx.scale };
    root.arrange(lx, Rect::new(0.0, 0.0, ww, hh), (0.0, 0.0));
    (ww, hh)
}
