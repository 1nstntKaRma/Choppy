//! Direct2D + DirectWrite: the factories, fonts, text (with colored runs on one baseline, like WPF Runs) and the
//! drawing calls the controls use. Coordinates are WPF units (DIPs); the render target's DPI maps them to pixels.

use std::cell::RefCell;
use std::collections::HashMap;


use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::*;
use windows_numerics::Vector2;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn r(&self) -> f32 {
        self.x + self.w
    }
    pub fn b(&self) -> f32 {
        self.y + self.h
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.r() && y >= self.y && y < self.b()
    }
    pub fn inset(&self, l: f32, t: f32, r: f32, b: f32) -> Rect {
        Rect::new(self.x + l, self.y + t, (self.w - l - r).max(0.0), (self.h - t - b).max(0.0))
    }
}

pub fn color(c: u32) -> D2D1_COLOR_F {
    let f = |s: u32| ((c >> s) & 0xFF) as f32 / 255.0;
    D2D1_COLOR_F { r: f(16), g: f(8), b: f(0), a: f(24) }
}

pub fn v2(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Fam {
    Ui,
    Display,
    Icons,
}

/// A font as WPF names it: family, size in DIPs, weight (400 normal, 600 SemiBold, 700 Bold).
#[derive(Clone, Copy, Debug)]
pub struct Font {
    pub fam: Fam,
    pub size: f32,
    pub weight: u16,
}

pub const fn sf(size: f32, weight: u16) -> Font {
    Font { fam: Fam::Ui, size, weight }
}
pub const fn display(size: f32, weight: u16) -> Font {
    Font { fam: Fam::Display, size, weight }
}
pub const fn icons(size: f32) -> Font {
    Font { fam: Fam::Icons, size, weight: 400 }
}

/// Factories and fonts, shared by every window.
pub struct Gfx {
    pub d2d: ID2D1Factory,
    pub dw: IDWriteFactory,
    fam_ui: HSTRING,
    fam_display: HSTRING,
    fam_icons: HSTRING,
    formats: RefCell<HashMap<(Fam, u32, u16, bool), IDWriteTextFormat>>,
    ellipsis: RefCell<HashMap<(Fam, u32, u16), IDWriteInlineObject>>,
}

impl Gfx {
    pub fn new() -> Result<Gfx> {
        unsafe {
            let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let mut coll = None;
            dw.GetSystemFontCollection(&mut coll, false)?;
            let coll = coll.unwrap();
            let has = |name: &str| {
                let (mut i, mut ok) = (0u32, BOOL(0));
                coll.FindFamilyName(&HSTRING::from(name), &mut i, &mut ok).is_ok() && ok.as_bool()
            };
            // Theme.xaml: "Segoe UI Variable Text, Segoe UI" / "Segoe Fluent Icons, Segoe MDL2 Assets"
            let pick = |a: &str, b: &str| HSTRING::from(if has(a) { a } else { b });
            Ok(Gfx {
                fam_ui: pick("Segoe UI Variable Text", "Segoe UI"),
                fam_display: pick("Segoe UI Variable Display", "Segoe UI"),
                fam_icons: pick("Segoe Fluent Icons", "Segoe MDL2 Assets"),
                d2d,
                dw,
                formats: RefCell::new(HashMap::new()),
                ellipsis: RefCell::new(HashMap::new()),
            })
        }
    }

    fn family(&self, f: Fam) -> &HSTRING {
        match f {
            Fam::Ui => &self.fam_ui,
            Fam::Display => &self.fam_display,
            Fam::Icons => &self.fam_icons,
        }
    }

    fn format(&self, f: Font, trim: bool) -> IDWriteTextFormat {
        let key = (f.fam, (f.size * 100.0) as u32, f.weight, trim);
        if let Some(t) = self.formats.borrow().get(&key) {
            return t.clone();
        }
        let t = unsafe {
            let t = self
                .dw
                .CreateTextFormat(
                    self.family(f.fam),
                    None,
                    DWRITE_FONT_WEIGHT(f.weight as i32),
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    f.size,
                    w!("en-us"),
                )
                .unwrap();
            let _ = t.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
            if trim {
                let sign = self.ellipsis_for(f, &t);
                let tr = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
                let _ = t.SetTrimming(&tr, &sign);
            }
            t
        };
        self.formats.borrow_mut().insert(key, t.clone());
        t
    }

    fn ellipsis_for(&self, f: Font, t: &IDWriteTextFormat) -> IDWriteInlineObject {
        let key = (f.fam, (f.size * 100.0) as u32, f.weight);
        if let Some(e) = self.ellipsis.borrow().get(&key) {
            return e.clone();
        }
        let e = unsafe { self.dw.CreateEllipsisTrimmingSign(t).unwrap() };
        self.ellipsis.borrow_mut().insert(key, e.clone());
        e
    }

    /// A laid-out text: one or more runs (each its own font and color) on one line, as WPF lays out TextBlock
    /// Runs: one shared baseline. `max_w` > 0 cuts it with "…" (TextTrimming.CharacterEllipsis).
    pub fn layout(&self, runs: &[(&str, Font, u32)], max_w: f32, scale: f32) -> Text {
        let text: String = runs.iter().map(|r| r.0).collect();
        let wide: Vec<u16> = text.encode_utf16().collect();
        let base = runs.first().map(|r| r.1).unwrap_or(sf(14.0, 400));
        let trim = max_w > 0.0;
        unsafe {
            let fmt = self.format(base, trim);
            // WPF TextFormattingMode="Display": GDI-compatible widths at the screen's pixel size
            let lay = self
                .dw
                .CreateGdiCompatibleTextLayout(&wide, &fmt, if trim { max_w } else { 100000.0 }, 1000.0, scale, None, false)
                .unwrap();
            let mut pos = 0u32;
            for (s, f, _) in runs.iter() {
                let len = s.encode_utf16().count() as u32;
                let range = DWRITE_TEXT_RANGE { startPosition: pos, length: len };
                if f.fam != base.fam {
                    let _ = lay.SetFontFamilyName(self.family(f.fam), range);
                }
                if f.size != base.size {
                    let _ = lay.SetFontSize(f.size, range);
                }
                if f.weight != base.weight {
                    let _ = lay.SetFontWeight(DWRITE_FONT_WEIGHT(f.weight as i32), range);
                }
                pos += len;
            }
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = lay.GetMetrics(&mut m);
            let mut lines = [DWRITE_LINE_METRICS::default(); 1];
            let mut n = 0u32;
            let _ = lay.GetLineMetrics(Some(&mut lines), &mut n);
            // WPF (TextFormattingMode Display) sizes a line from the font's own numbers, each part rounded to whole
            // pixels: ascent = size x Baseline, descent = size x (LineSpacing - Baseline); the tallest run wins
            let (mut asc, mut desc) = (0f32, 0f32);
            for (_, f, _) in runs.iter() {
                let (b, ls) = wpf_metrics(f.fam);
                asc = asc.max((f.size * b * scale + 0.5).floor());
                desc = desc.max((f.size * (ls - b) * scale + 0.5).floor());
            }
            Text {
                layout: lay,
                runs: runs.iter().map(|r| (r.0.encode_utf16().count() as u32, r.2)).collect(),
                w: m.widthIncludingTrailingWhitespace,
                h: (asc + desc) / scale,
                asc: asc / scale,
                baseline: lines[0].baseline,
                trimmed: trim && m.widthIncludingTrailingWhitespace > max_w,
            }
        }
    }
}

/// FontFamily.Baseline and LineSpacing as WPF reports them (Segoe UI and Segoe UI Variable share them).
fn wpf_metrics(f: Fam) -> (f32, f32) {
    match f {
        Fam::Ui | Fam::Display => (1.0791, 1.3301),
        Fam::Icons => (1.0, 1.0),
    }
}

pub struct Text {
    pub layout: IDWriteTextLayout,
    runs: Vec<(u32, u32)>,
    pub w: f32,
    /// The TextBlock's height and where its baseline sits (WPF's rounding).
    pub h: f32,
    pub asc: f32,
    /// DirectWrite's own baseline in the layout.
    pub baseline: f32,
    #[allow(dead_code)]
    pub trimmed: bool,
}

impl Text {
    /// The character position under x (for the text box), and whether it hit the trailing half.
    pub fn hit(&self, x: f32) -> u32 {
        unsafe {
            let (mut trailing, mut inside) = (BOOL(0), BOOL(0));
            let mut m = DWRITE_HIT_TEST_METRICS::default();
            let _ = self.layout.HitTestPoint(x, 1.0, &mut trailing, &mut inside, &mut m);
            m.textPosition + if trailing.as_bool() { m.length } else { 0 }
        }
    }
    /// The x of the caret before character `pos`.
    pub fn caret_x(&self, pos: u32) -> f32 {
        unsafe {
            let (mut x, mut y) = (0f32, 0f32);
            let mut m = DWRITE_HIT_TEST_METRICS::default();
            let _ = self.layout.HitTestTextPosition(pos, false, &mut x, &mut y, &mut m);
            x
        }
    }
}

/// A window's render target with its brushes (they belong to the target).
pub struct Surface {
    pub rt: ID2D1RenderTarget,
    brushes: HashMap<u32, ID2D1SolidColorBrush>,
    strokes: HashMap<u32, ID2D1StrokeStyle>,
}

impl Surface {
    pub fn new(rt: ID2D1RenderTarget) -> Surface {
        Surface { rt, brushes: HashMap::new(), strokes: HashMap::new() }
    }
}

/// Drawing in device pixels (layout is done in pixels, rounded the way WPF's UseLayoutRounding rounds: see `rp`
/// and `at`). Font sizes, corner radii and line widths stay in WPF units (DIPs). With no surface it only measures.
pub struct Canvas<'a> {
    pub g: &'a Gfx,
    pub s: Option<&'a mut Surface>,
    pub scale: f32,
    /// Multiplies every color's alpha (disabled controls: WPF Opacity).
    pub opacity: f32,
}

impl<'a> Canvas<'a> {
    /// A WPF length (size or margin) in whole pixels: RoundLayoutValue = Math.Round, halves to even.
    pub fn rp(&self, dip: f32) -> f32 {
        (dip * self.scale).round_ties_even()
    }
    /// A WPF length in pixels, not rounded (an offset that is rounded only together with its slot, see `at`).
    pub fn fp(&self, dip: f32) -> f32 {
        dip * self.scale
    }
    /// An element's place: its parent's corner + its offset inside the parent, rounded as one number (WPF rounds
    /// the offset relative to the parent, so 200.5 -> 200 but 201.5 -> 202).
    pub fn at(&self, parent: f32, rel: f32) -> f32 {
        parent + rel.round_ties_even()
    }
    /// Centered in a slot: the top margin + (slot - margins - len) / 2, as ArrangeCore does.
    pub fn center(&self, slot: f32, m0: f32, m1: f32, len: f32) -> f32 {
        self.fp(m0) + (slot - self.rp(m0 + m1) - len) / 2.0
    }

    pub fn text(&self, runs: &[(&str, Font, u32)]) -> Text {
        self.g.layout(runs, 0.0, self.scale)
    }
    pub fn text_trim(&self, runs: &[(&str, Font, u32)], max_w_px: f32) -> Text {
        self.g.layout(runs, (max_w_px / self.scale).max(1.0), self.scale)
    }
    /// A TextBlock's size in pixels (WPF rounds its desired size).
    pub fn tw(&self, t: &Text) -> f32 {
        (t.w * self.scale).round_ties_even()
    }
    pub fn th(&self, t: &Text) -> f32 {
        (t.h * self.scale).round()
    }
    /// The height of one line in `f`, in pixels.
    pub fn line_h(&self, f: Font) -> f32 {
        self.th(&self.text(&[("Ag", f, 0)]))
    }

    fn d(&self, px: f32) -> f32 {
        px / self.scale
    }
    fn dr(&self, r: Rect) -> D2D_RECT_F {
        D2D_RECT_F { left: self.d(r.x), top: self.d(r.y), right: self.d(r.r()), bottom: self.d(r.b()) }
    }

    fn brush(&mut self, c: u32) -> Option<ID2D1SolidColorBrush> {
        let op = self.opacity;
        let c = if op < 1.0 { (c & 0x00FF_FFFF) | ((((c >> 24) as f32 * op) as u32) << 24) } else { c };
        let s = self.s.as_mut()?;
        if let Some(b) = s.brushes.get(&c) {
            return Some(b.clone());
        }
        let b = unsafe { s.rt.CreateSolidColorBrush(&color(c), None).ok()? };
        s.brushes.insert(c, b.clone());
        Some(b)
    }

    /// Text with the top of its TextBlock at (x, y) pixels.
    pub fn draw_text(&mut self, t: &Text, x: f32, y: f32) {
        if self.s.is_none() {
            return;
        }
        let mut pos = 0;
        let mut first = None;
        for &(len, c) in t.runs.iter() {
            let b = self.brush(c);
            if first.is_none() {
                first = b.clone();
            }
            if let Some(b) = b {
                unsafe {
                    let _ = t.layout.SetDrawingEffect(&b, DWRITE_TEXT_RANGE { startPosition: pos, length: len });
                }
            }
            pos += len;
        }
        if let Some(b) = first {
            // put DirectWrite's baseline where WPF's is
            let oy = self.d(y) + t.asc - t.baseline;
            let s = self.s.as_ref().unwrap();
            unsafe { s.rt.DrawTextLayout(v2(self.d(x), oy), &t.layout, &b, D2D1_DRAW_TEXT_OPTIONS_NONE) };
        }
    }

    pub fn fill(&mut self, r: Rect, c: u32) {
        let rr = self.dr(r);
        if let Some(b) = self.brush(c) {
            unsafe { self.s.as_ref().unwrap().rt.FillRectangle(&rr, &b) };
        }
    }

    /// Rounded rectangle, radius in DIPs.
    pub fn fill_round(&mut self, r: Rect, radius: f32, c: u32) {
        let rr = D2D1_ROUNDED_RECT { rect: self.dr(r), radiusX: radius, radiusY: radius };
        if let Some(b) = self.brush(c) {
            unsafe { self.s.as_ref().unwrap().rt.FillRoundedRectangle(&rr, &b) };
        }
    }

    /// A WPF Border: background and a border `thick` pixels wide inside the rectangle; radius in DIPs.
    pub fn border(&mut self, r: Rect, radius: f32, bg: Option<u32>, border: Option<u32>, thick: f32) {
        let t = thick.max(1.0);
        let half = t / 2.0;
        if let Some(bg) = bg {
            match border {
                // the background reaches under the border's inner half, like WPF
                Some(_) => self.fill_round(r.inset(half, half, half, half), (radius - self.d(half)).max(0.0), bg),
                None => self.fill_round(r, radius, bg),
            }
        }
        if let Some(bc) = border {
            let i = r.inset(half, half, half, half);
            let rad = (radius - self.d(half)).max(0.0);
            let rr = D2D1_ROUNDED_RECT { rect: self.dr(i), radiusX: rad, radiusY: rad };
            let w = self.d(t);
            if let Some(b) = self.brush(bc) {
                unsafe { self.s.as_ref().unwrap().rt.DrawRoundedRectangle(&rr, &b, w, None) };
            }
        }
    }

    /// Ellipse, center and radii in pixels.
    pub fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, c: u32) {
        let e = D2D1_ELLIPSE { point: v2(self.d(cx), self.d(cy)), radiusX: self.d(rx), radiusY: self.d(ry) };
        if let Some(b) = self.brush(c) {
            unsafe { self.s.as_ref().unwrap().rt.FillEllipse(&e, &b) };
        }
    }

    /// A line between pixel points, `w` DIPs wide.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, c: u32, w: f32, dash: Option<&[f32]>) {
        let style = dash.and_then(|d| self.stroke(d));
        let (p0, p1) = (v2(self.d(x0), self.d(y0)), v2(self.d(x1), self.d(y1)));
        if let Some(b) = self.brush(c) {
            unsafe { self.s.as_ref().unwrap().rt.DrawLine(p0, p1, &b, w, style.as_ref()) };
        }
    }

    fn stroke(&mut self, dashes: &[f32]) -> Option<ID2D1StrokeStyle> {
        let key = dashes.iter().fold(0u32, |a, d| a.wrapping_mul(31).wrapping_add((d * 100.0) as u32));
        let g = self.g;
        let s = self.s.as_mut()?;
        if let Some(st) = s.strokes.get(&key) {
            return Some(st.clone());
        }
        let p = D2D1_STROKE_STYLE_PROPERTIES {
            startCap: D2D1_CAP_STYLE_FLAT,
            endCap: D2D1_CAP_STYLE_FLAT,
            dashCap: D2D1_CAP_STYLE_FLAT,
            lineJoin: D2D1_LINE_JOIN_MITER,
            miterLimit: 10.0,
            dashStyle: D2D1_DASH_STYLE_CUSTOM,
            dashOffset: 0.0,
        };
        let st = unsafe { g.d2d.CreateStrokeStyle(&p, Some(dashes)).ok()? };
        s.strokes.insert(key, st.clone());
        Some(st)
    }

    pub fn round_stroke(&self) -> Option<ID2D1StrokeStyle> {
        let p = D2D1_STROKE_STYLE_PROPERTIES {
            startCap: D2D1_CAP_STYLE_ROUND,
            endCap: D2D1_CAP_STYLE_ROUND,
            dashCap: D2D1_CAP_STYLE_FLAT,
            lineJoin: D2D1_LINE_JOIN_ROUND,
            miterLimit: 10.0,
            dashStyle: D2D1_DASH_STYLE_SOLID,
            dashOffset: 0.0,
        };
        unsafe { self.g.d2d.CreateStrokeStyle(&p, None).ok() }
    }

    pub fn fill_geometry_brush(&mut self, g: &ID2D1Geometry, b: &ID2D1Brush) {
        if let Some(s) = self.s.as_ref() {
            unsafe { s.rt.FillGeometry(g, b, None) };
        }
    }

    /// Strokes a path (its points are in DIPs, see `path`), `w` DIPs wide.
    pub fn stroke_geometry(&mut self, g: &ID2D1Geometry, c: u32, w: f32, style: Option<&ID2D1StrokeStyle>) {
        if let Some(b) = self.brush(c) {
            unsafe { self.s.as_ref().unwrap().rt.DrawGeometry(g, &b, w, style) };
        }
    }

    /// A linear gradient between two pixel points (sRGB interpolation, like WPF's default).
    pub fn gradient(&self, stops: &[(f32, u32)], from: (f32, f32), to: (f32, f32)) -> Option<ID2D1Brush> {
        let s = self.s.as_ref()?;
        let st: Vec<D2D1_GRADIENT_STOP> = stops.iter().map(|&(p, c)| D2D1_GRADIENT_STOP { position: p, color: color(c) }).collect();
        unsafe {
            let coll = s.rt.CreateGradientStopCollection(&st, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP).ok()?;
            let p = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: v2(self.d(from.0), self.d(from.1)),
                endPoint: v2(self.d(to.0), self.d(to.1)),
            };
            let b = s.rt.CreateLinearGradientBrush(&p, None, &coll).ok()?;
            b.cast().ok()
        }
    }

    pub fn fill_round_brush(&mut self, r: Rect, radius: f32, b: &ID2D1Brush) {
        let rr = D2D1_ROUNDED_RECT { rect: self.dr(r), radiusX: radius, radiusY: radius };
        if let Some(s) = self.s.as_ref() {
            unsafe { s.rt.FillRoundedRectangle(&rr, b) };
        }
    }

    pub fn clip(&mut self, r: Rect) {
        let rr = self.dr(r);
        if let Some(s) = self.s.as_ref() {
            unsafe { s.rt.PushAxisAlignedClip(&rr, D2D1_ANTIALIAS_MODE_ALIASED) };
        }
    }
    pub fn unclip(&mut self) {
        if let Some(s) = self.s.as_ref() {
            unsafe { s.rt.PopAxisAlignedClip() };
        }
    }

    pub fn bitmap(&mut self, bmp: &ID2D1Bitmap, r: Rect) {
        let rr = self.dr(r);
        if let Some(s) = self.s.as_ref() {
            unsafe { s.rt.DrawBitmap(bmp, Some(&rr), self.opacity, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None) };
        }
    }

    /// A path built with `f`; its points are in DIPs (pixels / scale).
    pub fn path(&self, f: impl FnOnce(&ID2D1GeometrySink)) -> Option<ID2D1Geometry> {
        unsafe {
            let g = self.g.d2d.CreatePathGeometry().ok()?;
            let sink = g.Open().ok()?;
            f(&sink);
            sink.Close().ok()?;
            g.cast().ok()
        }
    }
}

/// Helpers for building paths.
pub mod pth {
    use super::*;
    pub fn begin(s: &ID2D1GeometrySink, x: f32, y: f32, filled: bool) {
        unsafe { s.BeginFigure(v2(x, y), if filled { D2D1_FIGURE_BEGIN_FILLED } else { D2D1_FIGURE_BEGIN_HOLLOW }) };
    }
    pub fn line(s: &ID2D1GeometrySink, x: f32, y: f32) {
        unsafe { s.AddLine(v2(x, y)) };
    }
    pub fn bezier(s: &ID2D1GeometrySink, c1: (f32, f32), c2: (f32, f32), p: (f32, f32)) {
        let b = D2D1_BEZIER_SEGMENT { point1: v2(c1.0, c1.1), point2: v2(c2.0, c2.1), point3: v2(p.0, p.1) };
        unsafe { s.AddBezier(&b) };
    }
    pub fn arc(s: &ID2D1GeometrySink, x: f32, y: f32, r: f32, large: bool) {
        let a = D2D1_ARC_SEGMENT {
            point: v2(x, y),
            size: D2D_SIZE_F { width: r, height: r },
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
            arcSize: if large { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
        };
        unsafe { s.AddArc(&a) };
    }
    pub fn end(s: &ID2D1GeometrySink, closed: bool) {
        unsafe { s.EndFigure(if closed { D2D1_FIGURE_END_CLOSED } else { D2D1_FIGURE_END_OPEN }) };
    }
}
