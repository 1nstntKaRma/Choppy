//! Direct2D + DirectWrite (look test A's `gfx`, step 4): the factories and fonts, laid-out text (colored runs on one
//! baseline like WPF Runs, trimming, wrapping), and the drawing calls the windows use. Coordinates are WPF units
//! (DIPs); a target's DPI maps them to pixels. The layout (ui/layout.rs) already rounds every position to whole
//! pixels the way WPF's UseLayoutRounding does, so what is drawn here lands on the same pixels as WPF's.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use windows::Win32::Foundation::{GENERIC_WRITE, HWND, RECT};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Gdi::HDC;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::core::*;
use windows_numerics::{Matrix3x2, Vector2};

/// 0xAARRGGBB.
pub type Argb = u32;

fn color(c: Argb) -> D2D1_COLOR_F {
    let f = |s: u32| ((c >> s) & 0xFF) as f32 / 255.0;
    D2D1_COLOR_F { r: f(16), g: f(8), b: f(0), a: f(24) }
}

fn v2(x: f64, y: f64) -> Vector2 {
    Vector2 { X: x as f32, Y: y as f32 }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn r(&self) -> f64 {
        self.x + self.w
    }
    pub fn b(&self) -> f64 {
        self.y + self.h
    }
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.r() && y >= self.y && y < self.b()
    }
    pub fn inset(&self, l: f64, t: f64, r: f64, b: f64) -> Rect {
        Rect::new(self.x + l, self.y + t, (self.w - l - r).max(0.0), (self.h - t - b).max(0.0))
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        Rect::new(x, y, (self.r().min(o.r()) - x).max(0.0), (self.b().min(o.b()) - y).max(0.0))
    }
    fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F { left: self.x as f32, top: self.y as f32, right: self.r() as f32, bottom: self.b() as f32 }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Fam {
    /// F.Ui: "Segoe UI Variable Text, Segoe UI"
    Ui,
    /// F.Display: "Segoe UI Variable Display, Segoe UI"
    Display,
    /// F.Icons: "Segoe Fluent Icons, Segoe MDL2 Assets"
    Icons,
    /// The tray menu's font (SystemFonts.MenuFont: Segoe UI)
    Menu,
}

/// A font as WPF names it: family, size in DIPs, weight (400 normal, 600 SemiBold, 700 Bold).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Font {
    pub fam: Fam,
    pub size: f64,
    pub weight: u16,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Right,
}

/// How a text is laid out: `max_w` > 0 limits the width (trim = "…", wrap = more lines); `line_height` > 0 is
/// TextBlock.LineHeight (each line that tall).
#[derive(Clone, Copy, Debug, Default)]
pub struct TextOpts {
    pub max_w: f64,
    pub trim: bool,
    pub wrap: bool,
    pub align: TextAlign,
    pub line_height: f64,
    /// FormattedText's Ideal formatting (PathText): natural widths, the line LineSpacing x size tall, unrounded.
    pub ideal: bool,
}

/// FontFamily.Baseline and LineSpacing as WPF reports them (Segoe UI and Segoe UI Variable share them; the icon
/// fonts are 1 / 1.33).
fn wpf_metrics(f: Fam) -> (f64, f64) {
    match f {
        Fam::Ui | Fam::Display | Fam::Menu => (1.0791015625, 1.330078125),
        Fam::Icons => (1.0, 1.0),
    }
}

/// A WPF line's height and baseline in DIPs at `scale` (TextFormattingMode Display: ascent and descent each
/// rounded to whole pixels; the tallest run wins).
pub fn line_metrics(fonts: &[Font], scale: f64) -> (f64, f64) {
    let (mut asc, mut desc) = (0f64, 0f64);
    for f in fonts {
        let (b, ls) = wpf_metrics(f.fam);
        asc = asc.max((f.size * b * scale + 0.5).floor());
        desc = desc.max((f.size * (ls - b) * scale + 0.5).floor());
    }
    ((asc + desc) / scale, asc / scale)
}

/// Factories and fonts, shared by every window of the UI thread.
pub struct Gfx {
    d2d: ID2D1Factory1,
    dw: IDWriteFactory,
    wic: IWICImagingFactory,
    fam_ui: HSTRING,
    fam_display: HSTRING,
    fam_icons: HSTRING,
    fam_menu: HSTRING,
    formats: RefCell<HashMap<(Fam, u64, u16, bool, bool, u8), IDWriteTextFormat>>,
    ellipsis: RefCell<HashMap<(Fam, u64, u16), IDWriteInlineObject>>,
    params: RefCell<Option<IDWriteRenderingParams>>,
}

impl Gfx {
    pub fn new() -> Option<Rc<Gfx>> {
        unsafe {
            let d2d: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).ok()?;
            let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).ok()?;
            let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
            let mut coll = None;
            dw.GetSystemFontCollection(&mut coll, false).ok()?;
            let coll = coll?;
            let has = |name: &str| {
                let (mut i, mut ok) = (0u32, BOOL(0));
                coll.FindFamilyName(&HSTRING::from(name), &mut i, &mut ok).is_ok() && ok.as_bool()
            };
            let pick = |a: &str, b: &str| HSTRING::from(if has(a) { a } else { b });
            Some(Rc::new(Gfx {
                fam_ui: pick("Segoe UI Variable Text", "Segoe UI"),
                fam_display: pick("Segoe UI Variable Display", "Segoe UI"),
                fam_icons: pick("Segoe Fluent Icons", "Segoe MDL2 Assets"),
                fam_menu: HSTRING::from("Segoe UI"),
                d2d,
                dw,
                wic,
                formats: RefCell::new(HashMap::new()),
                ellipsis: RefCell::new(HashMap::new()),
                params: RefCell::new(None),
            }))
        }
    }

    fn family(&self, f: Fam) -> &HSTRING {
        match f {
            Fam::Ui => &self.fam_ui,
            Fam::Display => &self.fam_display,
            Fam::Icons => &self.fam_icons,
            Fam::Menu => &self.fam_menu,
        }
    }

    fn format(&self, f: Font, trim: bool, wrap: bool, align: TextAlign) -> Option<IDWriteTextFormat> {
        let key = (f.fam, (f.size * 1000.0) as u64, f.weight, trim, wrap, align as u8);
        if let Some(t) = self.formats.borrow().get(&key) {
            return Some(t.clone());
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
                    f.size as f32,
                    w!("en-us"),
                )
                .ok()?;
            let _ = t.SetWordWrapping(if wrap { DWRITE_WORD_WRAPPING_WRAP } else { DWRITE_WORD_WRAPPING_NO_WRAP });
            let _ = t.SetTextAlignment(match align {
                TextAlign::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                TextAlign::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            if trim {
                let key = (f.fam, (f.size * 1000.0) as u64, f.weight);
                let sign = self.ellipsis.borrow().get(&key).cloned();
                let sign = match sign {
                    Some(s) => s,
                    None => {
                        let e = self.dw.CreateEllipsisTrimmingSign(&t).ok()?;
                        self.ellipsis.borrow_mut().insert(key, e.clone());
                        e
                    }
                };
                let tr = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
                let _ = t.SetTrimming(&tr, &sign);
            }
            t
        };
        self.formats.borrow_mut().insert(key, t.clone());
        Some(t)
    }

    /// A laid-out text: one or more runs (each its own font and color) as WPF lays out a TextBlock's Runs.
    pub fn text(&self, runs: &[(&str, Font, Argb)], o: &TextOpts, scale: f64) -> Text {
        let text: String = runs.iter().map(|r| r.0).collect();
        let wide: Vec<u16> = text.encode_utf16().collect();
        let base = runs.first().map(|r| r.1).unwrap_or(Font { fam: Fam::Ui, size: 12.0, weight: 400 });
        let limited = o.max_w > 0.0 && (o.trim || o.wrap);
        // a line that neither wraps nor trims still sits right or centered in the width it got (TextAlignment)
        let aligned = o.max_w > 0.0 && o.align != TextAlign::Left;
        let fonts: Vec<Font> = runs.iter().map(|r| r.1).collect();
        let (line_h, asc) = if o.ideal {
            let (b, ls) = wpf_metrics(base.fam);
            (base.size * ls, base.size * b)
        } else {
            line_metrics(&fonts, scale)
        };
        // hard line breaks make more lines even without wrapping (a tooltip's "\n")
        let multi = (o.wrap && limited) || text.contains('\n');
        let run_lens: Vec<(u32, Argb)> = runs.iter().map(|r| (r.0.encode_utf16().count() as u32, r.2)).collect();
        let Some(fmt) = self.format(base, o.trim && limited, o.wrap && limited, if limited || aligned { o.align } else { TextAlign::Left })
        else {
            return Text::empty(line_h, asc, run_lens);
        };
        unsafe {
            // WPF TextFormattingMode="Display": GDI-compatible widths at the screen's pixel size
            let max_w = if limited || aligned { o.max_w } else { 100000.0 };
            let made = if o.ideal {
                self.dw.CreateTextLayout(&wide, &fmt, max_w as f32, 100000.0)
            } else {
                self.dw.CreateGdiCompatibleTextLayout(&wide, &fmt, max_w as f32, 100000.0, scale as f32, None, false)
            };
            let Ok(lay) = made else {
                return Text::empty(line_h, asc, run_lens);
            };
            let mut pos = 0u32;
            for (s, f, _) in runs.iter() {
                let len = s.encode_utf16().count() as u32;
                let range = DWRITE_TEXT_RANGE { startPosition: pos, length: len };
                if f.fam != base.fam {
                    let _ = lay.SetFontFamilyName(self.family(f.fam), range);
                }
                if f.size != base.size {
                    let _ = lay.SetFontSize(f.size as f32, range);
                }
                if f.weight != base.weight {
                    let _ = lay.SetFontWeight(DWRITE_FONT_WEIGHT(f.weight as i32), range);
                }
                pos += len;
            }
            let step = if o.line_height > 0.0 { o.line_height } else { line_h };
            let mut lines = vec![DWRITE_LINE_METRICS::default(); 1];
            let mut n = 0u32;
            if lay.GetLineMetrics(Some(&mut lines), &mut n).is_err() && n > 1 {
                lines = vec![DWRITE_LINE_METRICS::default(); n as usize];
                let _ = lay.GetLineMetrics(Some(&mut lines), &mut n);
            }
            let dw_base = lines.first().map(|l| l.baseline).unwrap_or(0.0) as f64;
            if multi {
                // every line WPF's height apart, its baseline where WPF puts it
                // with LineHeight the baseline stays where the font puts it; the extra room goes below
                let base_in_line = asc;
                let _ = lay.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, step as f32, base_in_line as f32);
            }
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = lay.GetMetrics(&mut m);
            let count = m.lineCount.max(1);
            let w = if o.wrap && limited { m.widthIncludingTrailingWhitespace.min(max_w as f32) } else { m.widthIncludingTrailingWhitespace };
            let w = if multi && !(o.wrap && limited) { m.widthIncludingTrailingWhitespace.max(m.width) } else { w };
            Text {
                layout: Some(lay),
                runs: run_lens,
                w: w as f64,
                h: if multi { step * count as f64 } else if o.line_height > 0.0 { o.line_height } else { line_h },
                asc,
                dw_baseline: if multi { -1.0 } else { dw_base },
                line_shift: 0.0,
            }
        }
    }

    fn rendering_params(&self) -> Option<IDWriteRenderingParams> {
        if self.params.borrow().is_none() {
            *self.params.borrow_mut() = unsafe { self.dw.CreateRenderingParams().ok() };
        }
        self.params.borrow().clone()
    }

    /// Decodes an image file in memory (.ico, .png) into `px` x `px` pixels: the smallest frame at least that big
    /// (else the largest), scaled with WIC's Fant filter like WPF's BitmapScalingMode.HighQuality.
    pub fn icon_image(&self, bytes: &[u8], px: u32) -> Option<Image> {
        unsafe {
            let stream = SHCreateMemStream(Some(bytes))?;
            let dec = self.wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?;
            let n = dec.GetFrameCount().ok()?;
            let mut pick: Option<(u32, IWICBitmapFrameDecode)> = None;
            let mut largest: Option<(u32, IWICBitmapFrameDecode)> = None;
            for i in 0..n {
                let f = dec.GetFrame(i).ok()?;
                let (mut w, mut h) = (0, 0);
                f.GetSize(&mut w, &mut h).ok()?;
                if largest.as_ref().is_none_or(|l| w > l.0) {
                    largest = Some((w, f.clone()));
                }
                if w >= px && pick.as_ref().is_none_or(|p| w < p.0) {
                    pick = Some((w, f));
                }
            }
            let (_, frame) = pick.or(largest)?;
            self.convert(&frame.cast().ok()?, px, px)
        }
    }

    /// Premultiplied BGRA pixels (`premul` false = straight alpha) as an image scaled to `w` x `h` with Fant.
    pub fn bgra_image(&self, pixels: &[u8], sw: u32, sh: u32, premul: bool, w: u32, h: u32) -> Option<Image> {
        unsafe {
            let fmt = if premul { GUID_WICPixelFormat32bppPBGRA } else { GUID_WICPixelFormat32bppBGRA };
            let bmp = self.wic.CreateBitmapFromMemory(sw, sh, &fmt, sw * 4, pixels).ok()?;
            self.convert(&bmp.cast().ok()?, w, h)
        }
    }

    unsafe fn convert(&self, src: &IWICBitmapSource, w: u32, h: u32) -> Option<Image> {
        unsafe {
            let conv = self.wic.CreateFormatConverter().ok()?;
            conv.Initialize(src, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).ok()?;
            let (mut sw, mut sh) = (0, 0);
            conv.GetSize(&mut sw, &mut sh).ok()?;
            let out: IWICBitmapSource = if sw != w || sh != h {
                let sc = self.wic.CreateBitmapScaler().ok()?;
                sc.Initialize(&conv, w.max(1), h.max(1), WICBitmapInterpolationModeFant).ok()?;
                sc.cast().ok()?
            } else {
                conv.cast().ok()?
            };
            let (w, h) = (w.max(1), h.max(1));
            let mut px = vec![0u8; (w * h * 4) as usize];
            out.CopyPixels(std::ptr::null(), w * 4, &mut px).ok()?;
            Some(Image::new(px, w, h))
        }
    }

    // ---------- render targets ----------

    /// A window's target (grayscale text like WPF draws these windows).
    pub fn hwnd_target(&self, hwnd: HWND, w: u32, h: u32, dpi: f64) -> Option<Target> {
        unsafe {
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE },
                dpiX: dpi as f32,
                dpiY: dpi as f32,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let hp = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd,
                pixelSize: D2D_SIZE_U { width: w.max(1), height: h.max(1) },
                presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY,
            };
            let rt = self.d2d.CreateHwndRenderTarget(&props, &hp).ok()?;
            let t = Target::new(rt.cast().ok()?, Kind::Hwnd(rt));
            self.prepare(&t);
            Some(t)
        }
    }

    /// A target that draws into a GDI device context (the layered pop-ups: see-through corners).
    pub fn dc_target(&self) -> Option<Target> {
        unsafe {
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_GDI_COMPATIBLE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let rt = self.d2d.CreateDCRenderTarget(&props).ok()?;
            let t = Target::new(rt.cast().ok()?, Kind::Dc(rt));
            self.prepare(&t);
            Some(t)
        }
    }

    /// An off-screen picture `w` x `h` pixels at `dpi` (the test hook's `shot`).
    pub fn picture_target(&self, w: u32, h: u32, dpi: f64) -> Option<Target> {
        unsafe {
            let bmp = self.wic.CreateBitmap(w.max(1), h.max(1), &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnDemand).ok()?;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: dpi as f32,
                dpiY: dpi as f32,
                usage: D2D1_RENDER_TARGET_USAGE_NONE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let rt = self.d2d.CreateWicBitmapRenderTarget(&bmp, &props).ok()?;
            let t = Target::new(rt, Kind::Wic(bmp));
            self.prepare(&t);
            Some(t)
        }
    }

    fn prepare(&self, t: &Target) {
        unsafe {
            t.rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            if let Some(p) = self.rendering_params() {
                t.rt.SetTextRenderingParams(&p);
            }
        }
    }

    /// Saves an off-screen picture as a PNG file.
    pub fn save_png(&self, t: &Target, path: &std::path::Path) -> bool {
        let Kind::Wic(bmp) = &t.kind else { return false };
        unsafe {
            let run = || -> Result<()> {
                let stream = self.wic.CreateStream()?;
                stream.InitializeFromFilename(&HSTRING::from(path.as_os_str()), GENERIC_WRITE.0)?;
                let enc = self.wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
                enc.Initialize(&stream, WICBitmapEncoderNoCache)?;
                let mut frame = None;
                enc.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
                let frame = frame.ok_or_else(Error::empty)?;
                frame.Initialize(None)?;
                let (mut w, mut h) = (0, 0);
                bmp.GetSize(&mut w, &mut h)?;
                frame.SetSize(w, h)?;
                let mut fmt = GUID_WICPixelFormat32bppPBGRA;
                frame.SetPixelFormat(&mut fmt)?;
                let src: IWICBitmapSource = bmp.cast()?;
                frame.WriteSource(&src, std::ptr::null())?;
                frame.Commit()?;
                enc.Commit()?;
                Ok(())
            };
            run().is_ok()
        }
    }
}

/// One frame of an .ico as a PNG file's bytes (the Benchmark report's icon): the smallest frame at least `min_px`
/// wide (else the largest), decoded and encoded again by WIC as WPF's IconBitmapDecoder + PngBitmapEncoder do.
pub fn ico_frame_png(bytes: &[u8], min_px: u32) -> Option<Vec<u8>> {
    use windows::Win32::System::Com::STREAM_SEEK_SET;
    unsafe {
        let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let stream = SHCreateMemStream(Some(bytes))?;
        let dec = wic.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?;
        let n = dec.GetFrameCount().ok()?;
        let mut best: Option<(u32, IWICBitmapFrameDecode)> = None;
        let mut largest: Option<(u32, IWICBitmapFrameDecode)> = None;
        for i in 0..n {
            let f = dec.GetFrame(i).ok()?;
            let (mut w, mut h) = (0, 0);
            f.GetSize(&mut w, &mut h).ok()?;
            if largest.as_ref().is_none_or(|l| w > l.0) {
                largest = Some((w, f.clone()));
            }
            if w >= min_px && best.as_ref().is_none_or(|b| w < b.0) {
                best = Some((w, f));
            }
        }
        let (_, frame) = best.or(largest)?;
        let out = SHCreateMemStream(None)?;
        let enc = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null()).ok()?;
        enc.Initialize(&out, WICBitmapEncoderNoCache).ok()?;
        let mut fe = None;
        enc.CreateNewFrame(&mut fe, std::ptr::null_mut()).ok()?;
        let fe = fe?;
        fe.Initialize(None).ok()?;
        let (mut w, mut h) = (0, 0);
        frame.GetSize(&mut w, &mut h).ok()?;
        fe.SetSize(w, h).ok()?;
        let mut fmt = frame.GetPixelFormat().ok()?;
        fe.SetPixelFormat(&mut fmt).ok()?;
        let src: IWICBitmapSource = frame.cast().ok()?;
        fe.WriteSource(&src, std::ptr::null()).ok()?;
        fe.Commit().ok()?;
        enc.Commit().ok()?;
        let mut len = 0u64;
        out.Seek(0, windows::Win32::System::Com::STREAM_SEEK_END, Some(&mut len)).ok()?;
        out.Seek(0, STREAM_SEEK_SET, None).ok()?;
        let mut buf = vec![0u8; len as usize];
        let mut read = 0u32;
        out.Read(buf.as_mut_ptr() as *mut _, len as u32, Some(&mut read)).ok().ok()?;
        buf.truncate(read as usize);
        Some(buf)
    }
}

/// Pixels in memory (premultiplied BGRA), drawn by any target (each target keeps its own copy on the GPU).
#[derive(Clone)]
pub struct Image {
    id: u64,
    pub w: u32,
    pub h: u32,
    px: Rc<Vec<u8>>,
}

impl Image {
    pub fn new(px: Vec<u8>, w: u32, h: u32) -> Image {
        thread_local!(static NEXT: std::cell::Cell<u64> = const { std::cell::Cell::new(1) });
        let id = NEXT.with(|n| {
            let v = n.get();
            n.set(v + 1);
            v
        });
        Image { id, w, h, px: Rc::new(px) }
    }
}

enum Kind {
    Hwnd(ID2D1HwndRenderTarget),
    Dc(ID2D1DCRenderTarget),
    Wic(IWICBitmap),
}

/// A render target with what belongs to it (brushes, dashes, uploaded images).
pub struct Target {
    rt: ID2D1RenderTarget,
    kind: Kind,
    brushes: HashMap<Argb, ID2D1SolidColorBrush>,
    strokes: HashMap<u64, ID2D1StrokeStyle>,
    bitmaps: HashMap<u64, ID2D1Bitmap>,
}

impl Target {
    fn new(rt: ID2D1RenderTarget, kind: Kind) -> Target {
        Target { rt, kind, brushes: HashMap::new(), strokes: HashMap::new(), bitmaps: HashMap::new() }
    }
    pub fn set_dpi(&self, dpi: f64) {
        unsafe { self.rt.SetDpi(dpi as f32, dpi as f32) };
    }
    /// A window's target follows the window's size (pixels).
    pub fn resize(&self, w: u32, h: u32) {
        if let Kind::Hwnd(h_rt) = &self.kind {
            unsafe {
                let _ = h_rt.Resize(&D2D_SIZE_U { width: w.max(1), height: h.max(1) });
            }
        }
    }
    /// A DC target draws into this device context's `w` x `h` pixels.
    pub fn bind_dc(&self, dc: HDC, w: i32, h: i32) -> bool {
        if let Kind::Dc(d) = &self.kind {
            let rc = RECT { left: 0, top: 0, right: w, bottom: h };
            unsafe { d.BindDC(dc, &rc).is_ok() }
        } else {
            false
        }
    }
    pub fn begin(&self) {
        unsafe { self.rt.BeginDraw() };
    }
    /// False when the device was lost (the target must be made again).
    pub fn end(&self) -> bool {
        unsafe { self.rt.EndDraw(None, None).is_ok() }
    }
    pub fn clear(&self, c: Argb) {
        unsafe { self.rt.Clear(Some(&color(c))) };
    }
}

/// A laid-out text. Sizes in DIPs.
pub struct Text {
    layout: Option<IDWriteTextLayout>,
    runs: Vec<(u32, Argb)>,
    /// Width including trailing white space (what TextBlock measures).
    pub w: f64,
    /// The TextBlock's height (WPF's line rounding) and where its first baseline sits.
    pub h: f64,
    pub asc: f64,
    dw_baseline: f64,
    line_shift: f64,
}

impl Text {
    fn empty(h: f64, asc: f64, runs: Vec<(u32, Argb)>) -> Text {
        Text { layout: None, runs, w: 0.0, h, asc, dw_baseline: 0.0, line_shift: 0.0 }
    }
    /// The character position under x (DIPs from the text's start).
    pub fn hit(&self, x: f64) -> u32 {
        let Some(l) = &self.layout else { return 0 };
        unsafe {
            let (mut trailing, mut inside) = (BOOL(0), BOOL(0));
            let mut m = DWRITE_HIT_TEST_METRICS::default();
            let _ = l.HitTestPoint(x as f32, 1.0, &mut trailing, &mut inside, &mut m);
            m.textPosition + if trailing.as_bool() { m.length } else { 0 }
        }
    }
    /// The x of the caret before character `pos`.
    pub fn caret_x(&self, pos: u32) -> f64 {
        let Some(l) = &self.layout else { return 0.0 };
        unsafe {
            let (mut x, mut y) = (0f32, 0f32);
            let mut m = DWRITE_HIT_TEST_METRICS::default();
            let _ = l.HitTestTextPosition(pos, false, &mut x, &mut y, &mut m);
            x as f64
        }
    }
}

/// Builds a path (DIPs).
pub struct PathBuilder {
    sink: ID2D1GeometrySink,
    open: bool,
}

impl PathBuilder {
    pub fn begin(&mut self, x: f64, y: f64, filled: bool) {
        unsafe { self.sink.BeginFigure(v2(x, y), if filled { D2D1_FIGURE_BEGIN_FILLED } else { D2D1_FIGURE_BEGIN_HOLLOW }) };
        self.open = true;
    }
    pub fn line(&mut self, x: f64, y: f64) {
        unsafe { self.sink.AddLine(v2(x, y)) };
    }
    pub fn bezier(&mut self, c1: (f64, f64), c2: (f64, f64), p: (f64, f64)) {
        let b = D2D1_BEZIER_SEGMENT { point1: v2(c1.0, c1.1), point2: v2(c2.0, c2.1), point3: v2(p.0, p.1) };
        unsafe { self.sink.AddBezier(&b) };
    }
    /// A clockwise arc to (x, y) with radius r.
    pub fn arc(&mut self, x: f64, y: f64, r: f64, large: bool) {
        let a = D2D1_ARC_SEGMENT {
            point: v2(x, y),
            size: D2D_SIZE_F { width: r as f32, height: r as f32 },
            rotationAngle: 0.0,
            sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
            arcSize: if large { D2D1_ARC_SIZE_LARGE } else { D2D1_ARC_SIZE_SMALL },
        };
        unsafe { self.sink.AddArc(&a) };
    }
    pub fn end(&mut self, closed: bool) {
        unsafe { self.sink.EndFigure(if closed { D2D1_FIGURE_END_CLOSED } else { D2D1_FIGURE_END_OPEN }) };
        self.open = false;
    }
}

pub struct Path(ID2D1Geometry);

impl Path {
    /// The geometry's bounds (DIPs): where a gradient fill maps to, as in WPF.
    pub fn bounds(&self) -> Rect {
        unsafe {
            match self.0.GetBounds(None) {
                Ok(b) => Rect::new(b.left as f64, b.top as f64, (b.right - b.left) as f64, (b.bottom - b.top) as f64),
                Err(_) => Rect::default(),
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Cap {
    Flat,
    Round,
}

/// Drawing on a target in DIPs.
pub struct Canvas<'a> {
    pub g: &'a Gfx,
    t: &'a mut Target,
    pub scale: f64,
    layers: Vec<bool>,
    /// Layout rounding is on (WPF's UseLayoutRounding): border pens are whole pixels.
    pub snap: bool,
    /// Text in WPF's Ideal formatting (windows built in code have no TextFormattingMode="Display").
    pub ideal: bool,
}

impl<'a> Canvas<'a> {
    pub fn new(g: &'a Gfx, t: &'a mut Target, scale: f64) -> Canvas<'a> {
        Canvas { g, t, scale, layers: Vec::new(), snap: true, ideal: false }
    }

    fn brush(&mut self, c: Argb) -> Option<ID2D1SolidColorBrush> {
        if let Some(b) = self.t.brushes.get(&c) {
            return Some(b.clone());
        }
        let b = unsafe { self.t.rt.CreateSolidColorBrush(&color(c), None).ok()? };
        self.t.brushes.insert(c, b.clone());
        Some(b)
    }

    fn stroke(&mut self, dashes: &[f64], cap: Cap) -> Option<ID2D1StrokeStyle> {
        let key = dashes.iter().fold(cap as u64 + 1, |a, d| a.wrapping_mul(31).wrapping_add((d * 100.0) as u64));
        if let Some(s) = self.t.strokes.get(&key) {
            return Some(s.clone());
        }
        let c = if cap == Cap::Round { D2D1_CAP_STYLE_ROUND } else { D2D1_CAP_STYLE_FLAT };
        let p = D2D1_STROKE_STYLE_PROPERTIES {
            startCap: c,
            endCap: c,
            dashCap: D2D1_CAP_STYLE_FLAT,
            lineJoin: if cap == Cap::Round { D2D1_LINE_JOIN_ROUND } else { D2D1_LINE_JOIN_MITER },
            miterLimit: 10.0,
            dashStyle: if dashes.is_empty() { D2D1_DASH_STYLE_SOLID } else { D2D1_DASH_STYLE_CUSTOM },
            dashOffset: 0.0,
        };
        let d: Vec<f32> = dashes.iter().map(|x| *x as f32).collect();
        let f: &ID2D1Factory = &self.g.d2d;
        let s = unsafe { f.CreateStrokeStyle(&p, if d.is_empty() { None } else { Some(&d) }).ok()? };
        self.t.strokes.insert(key, s.clone());
        Some(s)
    }

    /// Text with the top of its TextBlock at (x, y).
    pub fn text(&mut self, t: &Text, x: f64, y: f64) {
        let Some(layout) = &t.layout else { return };
        let mut pos = 0;
        let mut first = None;
        for &(len, c) in t.runs.iter() {
            let b = self.brush(c);
            if first.is_none() {
                first = b.clone();
            }
            if let Some(b) = b {
                unsafe {
                    let _ = layout.SetDrawingEffect(&b, DWRITE_TEXT_RANGE { startPosition: pos, length: len });
                }
            }
            pos += len;
        }
        if let Some(b) = first {
            // put DirectWrite's baseline where WPF's is (wrapped text: the line spacing puts it there already)
            let oy = if t.dw_baseline < 0.0 { y } else { y + t.line_shift + t.asc - t.dw_baseline };
            unsafe { self.t.rt.DrawTextLayout(v2(x, oy), layout, &b, D2D1_DRAW_TEXT_OPTIONS_NONE) };
        }
    }

    pub fn fill(&mut self, r: Rect, c: Argb) {
        if let Some(b) = self.brush(c) {
            unsafe { self.t.rt.FillRectangle(&r.d2d(), &b) };
        }
    }

    pub fn fill_round(&mut self, r: Rect, radius: f64, c: Argb) {
        if radius <= 0.0 {
            return self.fill(r, c);
        }
        let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius as f32, radiusY: radius as f32 };
        if let Some(b) = self.brush(c) {
            unsafe { self.t.rt.FillRoundedRectangle(&rr, &b) };
        }
    }

    pub fn stroke_round(&mut self, r: Rect, radius: f64, c: Argb, w: f64) {
        if let Some(b) = self.brush(c) {
            unsafe {
                if radius > 0.0 {
                    let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius as f32, radiusY: radius as f32 };
                    self.t.rt.DrawRoundedRectangle(&rr, &b, w as f32, None);
                } else {
                    self.t.rt.DrawRectangle(&r.d2d(), &b, w as f32, None);
                }
            }
        }
    }

    /// A linear gradient over `r` from (x0,y0) to (x1,y1) given as fractions of `r` (WPF's relative StartPoint /
    /// EndPoint), filling `r` with corner radius `radius`. sRGB interpolation (gamma 2.2) like WPF.
    pub fn fill_gradient(&mut self, r: Rect, radius: f64, stops: &[(f64, Argb)], from: (f64, f64), to: (f64, f64)) {
        if let Some(br) = self.gradient(r, stops, from, to) {
            let rr = D2D1_ROUNDED_RECT { rect: r.d2d(), radiusX: radius as f32, radiusY: radius as f32 };
            unsafe { self.t.rt.FillRoundedRectangle(&rr, &br) };
        }
    }

    fn gradient(&mut self, r: Rect, stops: &[(f64, Argb)], from: (f64, f64), to: (f64, f64)) -> Option<ID2D1Brush> {
        let st: Vec<D2D1_GRADIENT_STOP> = stops.iter().map(|&(p, c)| D2D1_GRADIENT_STOP { position: p as f32, color: color(c) }).collect();
        unsafe {
            let coll = self.t.rt.CreateGradientStopCollection(&st, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP).ok()?;
            let p = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: v2(r.x + r.w * from.0, r.y + r.h * from.1),
                endPoint: v2(r.x + r.w * to.0, r.y + r.h * to.1),
            };
            let b = self.t.rt.CreateLinearGradientBrush(&p, None, &coll).ok()?;
            b.cast().ok()
        }
    }

    pub fn ellipse(&mut self, cx: f64, cy: f64, rx: f64, ry: f64, c: Argb) {
        let e = D2D1_ELLIPSE { point: v2(cx, cy), radiusX: rx as f32, radiusY: ry as f32 };
        if let Some(b) = self.brush(c) {
            unsafe { self.t.rt.FillEllipse(&e, &b) };
        }
    }

    pub fn stroke_ellipse(&mut self, cx: f64, cy: f64, r: f64, c: Argb, w: f64) {
        let e = D2D1_ELLIPSE { point: v2(cx, cy), radiusX: r as f32, radiusY: r as f32 };
        if let Some(b) = self.brush(c) {
            unsafe { self.t.rt.DrawEllipse(&e, &b, w as f32, None) };
        }
    }

    /// A line `w` DIPs wide, optionally dashed (dash lengths in line widths, as WPF's DashStyle).
    pub fn line(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, c: Argb, w: f64, dash: &[f64]) {
        let style = if dash.is_empty() { None } else { self.stroke(dash, Cap::Flat) };
        if let Some(b) = self.brush(c) {
            unsafe { self.t.rt.DrawLine(v2(x0, y0), v2(x1, y1), &b, w as f32, style.as_ref()) };
        }
    }

    pub fn path(&self, f: impl FnOnce(&mut PathBuilder)) -> Option<Path> {
        unsafe {
            let g = self.g.d2d.CreatePathGeometry().ok()?;
            let sink = g.Open().ok()?;
            let mut pb = PathBuilder { sink, open: false };
            f(&mut pb);
            if pb.open {
                pb.end(false);
            }
            pb.sink.Close().ok()?;
            Some(Path(g.cast().ok()?))
        }
    }

    pub fn fill_path(&mut self, p: &Path, c: Argb) {
        if let Some(b) = self.brush(c) {
            unsafe { self.t.rt.FillGeometry(&p.0, &b, None) };
        }
    }

    /// Fills a path with a vertical gradient over the path's own bounds (WPF maps a brush to the filled shape).
    pub fn fill_path_gradient(&mut self, p: &Path, stops: &[(f64, Argb)]) {
        let b = p.bounds();
        if let Some(br) = self.gradient(b, stops, (0.0, 0.0), (0.0, 1.0)) {
            unsafe { self.t.rt.FillGeometry(&p.0, &br, None) };
        }
    }

    pub fn stroke_path(&mut self, p: &Path, c: Argb, w: f64, cap: Cap) {
        let style = if cap == Cap::Round { self.stroke(&[], Cap::Round) } else { None };
        if let Some(b) = self.brush(c) {
            unsafe { self.t.rt.DrawGeometry(&p.0, &b, w as f32, style.as_ref()) };
        }
    }

    pub fn image(&mut self, img: &Image, r: Rect, opacity: f64) {
        let bmp = match self.t.bitmaps.get(&img.id) {
            Some(b) => b.clone(),
            None => {
                let props = D2D1_BITMAP_PROPERTIES {
                    pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                    dpiX: 96.0,
                    dpiY: 96.0,
                };
                let size = D2D_SIZE_U { width: img.w, height: img.h };
                let Ok(b) = (unsafe { self.t.rt.CreateBitmap(size, Some(img.px.as_ptr() as *const _), img.w * 4, &props) }) else {
                    return;
                };
                if self.t.bitmaps.len() > 64 {
                    self.t.bitmaps.clear();
                }
                self.t.bitmaps.insert(img.id, b.clone());
                b
            }
        };
        unsafe {
            self.t.rt.DrawBitmap(&bmp, Some(&r.d2d()), opacity as f32, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None);
        }
    }

    pub fn push_clip(&mut self, r: Rect) {
        unsafe { self.t.rt.PushAxisAlignedClip(&r.d2d(), D2D1_ANTIALIAS_MODE_ALIASED) };
    }
    pub fn pop_clip(&mut self) {
        unsafe { self.t.rt.PopAxisAlignedClip() };
    }

    /// WPF's Opacity on a part of the window: drawn as a group, then faded.
    pub fn push_opacity(&mut self, o: f64) {
        if o >= 1.0 {
            self.layers.push(false);
            return;
        }
        unsafe {
            let Ok(layer) = self.t.rt.CreateLayer(None) else {
                self.layers.push(false);
                return;
            };
            let p = D2D1_LAYER_PARAMETERS {
                contentBounds: D2D_RECT_F { left: -1e6, top: -1e6, right: 1e6, bottom: 1e6 },
                geometricMask: std::mem::ManuallyDrop::new(None),
                maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                maskTransform: Matrix3x2::identity(),
                opacity: o as f32,
                opacityBrush: std::mem::ManuallyDrop::new(None),
                layerOptions: D2D1_LAYER_OPTIONS_NONE,
            };
            self.t.rt.PushLayer(&p, &layer);
        }
        self.layers.push(true);
    }
    pub fn pop_opacity(&mut self) {
        if self.layers.pop() == Some(true) {
            unsafe { self.t.rt.PopLayer() };
        }
    }

    /// A soft shadow under a rounded rectangle (WPF's DropShadowEffect: BlurRadius, ShadowDepth down-right at
    /// 315°, Opacity, black): rings of fading alpha, the cheap stand-in for a Gaussian blur.
    pub fn shadow(&mut self, r: Rect, radius: f64, blur: f64, depth: f64, opacity: f64) {
        let d = depth / std::f64::consts::SQRT_2;
        let steps = (blur * self.scale).ceil().max(1.0) as i32;
        let sigma = blur / 3.0;
        for i in (0..steps).rev() {
            let spread = blur * (i as f64 + 0.5) / steps as f64;
            // the share of a Gaussian beyond `spread` (each ring adds its part)
            let a = opacity * (-(spread * spread) / (2.0 * sigma * sigma)).exp() / steps as f64 * 2.2;
            let c = ((a.clamp(0.0, 1.0) * 255.0) as u32) << 24;
            let rr = Rect::new(r.x + d - spread / 2.0, r.y + d - spread / 2.0, r.w + spread, r.h + spread);
            self.fill_round(rr, radius + spread / 2.0, c);
        }
    }
}
