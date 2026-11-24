//! A pop-up like WPF's Popup with AllowsTransparency: its own window (so it can reach past the main window's edge),
//! see-through corners (a layered window drawn by Direct2D into a 32-bit bitmap), grayscale text like WPF's
//! transparent pop-ups, and PopupAnimation="Fade".

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

use crate::gfx::*;

pub struct Popup {
    pub hwnd: HWND,
    rt: ID2D1DCRenderTarget,
    pub alpha: u8,
    /// Screen position and size in pixels.
    pub pos: (i32, i32),
    pub size: (i32, i32),
}

extern "system" fn proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if m == WM_MOUSEACTIVATE {
            return LRESULT(MA_NOACTIVATE as isize);
        }
        DefWindowProcW(h, m, w, l)
    }
}

impl Popup {
    pub fn new(g: &Gfx, owner: HWND) -> Result<Popup> {
        unsafe {
            let inst = GetModuleHandleW(None)?;
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(proc),
                hInstance: inst.into(),
                lpszClassName: w!("ChoppyLookPopup"),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            };
            RegisterClassExW(&wc);
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                w!("ChoppyLookPopup"),
                w!(""),
                WS_POPUP,
                0,
                0,
                1,
                1,
                Some(owner),
                None,
                Some(inst.into()),
                None,
            )?;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                usage: D2D1_RENDER_TARGET_USAGE_GDI_COMPATIBLE,
                minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
            };
            let rt = g.d2d.CreateDCRenderTarget(&props)?;
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            Ok(Popup { hwnd, rt, alpha: 0, pos: (0, 0), size: (1, 1) })
        }
    }

    pub fn visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }

    pub fn hide(&mut self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.alpha = 0;
    }

    /// Draws `paint` (in DIPs, at `scale`) into a w x h pixel bitmap and puts it on screen at `pos`.
    pub fn render(&mut self, g: &Gfx, scale: f32, pos: (i32, i32), size: (i32, i32), paint: impl FnOnce(&mut Canvas)) {
        self.pos = pos;
        self.size = size;
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: size.0,
                    biHeight: -size.1,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let Ok(dib) = CreateDIBSection(Some(mem), &bi, DIB_RGB_COLORS, &mut bits, None, 0) else {
                let _ = DeleteDC(mem);
                ReleaseDC(None, screen);
                return;
            };
            let old = SelectObject(mem, dib.into());
            let rc = RECT { left: 0, top: 0, right: size.0, bottom: size.1 };
            self.rt.SetDpi(96.0 * scale, 96.0 * scale);
            if self.rt.BindDC(mem, &rc).is_ok() {
                self.rt.BeginDraw();
                self.rt.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
                let mut surf = Surface::new(self.rt.cast().unwrap());
                {
                    let mut cv = Canvas { g, s: Some(&mut surf), scale, opacity: 1.0 };
                    paint(&mut cv);
                }
                let _ = self.rt.EndDraw(None, None);
            }
            let pt = POINT { x: pos.0, y: pos.1 };
            let sz = SIZE { cx: size.0, cy: size.1 };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: self.alpha, AlphaFormat: AC_SRC_ALPHA as u8 };
            let _ = UpdateLayeredWindow(self.hwnd, Some(screen), Some(&pt), Some(&sz), Some(mem), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA);
            SelectObject(mem, old);
            let _ = DeleteObject(dib.into());
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
            if !IsWindowVisible(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
        }
    }

    /// Fade: only the window's overall alpha changes.
    pub fn set_alpha(&mut self, a: u8) {
        self.alpha = a;
        unsafe {
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: a, AlphaFormat: AC_SRC_ALPHA as u8 };
            let _ = UpdateLayeredWindow(self.hwnd, None, None, None, None, None, COLORREF(0), Some(&blend), ULW_ALPHA);
        }
    }
}
