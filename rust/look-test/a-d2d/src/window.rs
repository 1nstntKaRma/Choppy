//! The copy window (MainWindow.xaml / .cs): compact and extended views, refreshed every 100 ms from the demo job.
//! Layout follows the XAML element by element (margins, rows, columns); see the comments with each block.

use std::time::Instant;

use look_common::color as c;
use look_common::demo::{self, Args, Demo, View};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

use crate::controls;
use crate::gfx::*;
use crate::icon;
use crate::popup::Popup;
use crate::textbox::{BOX_SMALL, TextBox};
use crate::ui::*;

// control ids
const SETTINGS: u32 = 1;
const CLOSE: u32 = 2;
const UNIT: u32 = 3;
const TG_CLOSE: u32 = 4;
const TG_VERIFY: u32 = 5;
const TG_LIMIT: u32 = 6;
const LIMIT_BOX: u32 = 7;
const PAUSE: u32 = 8;
const SKIP: u32 = 9;
const CANCEL: u32 = 10;
const QUEUE: u32 = 11;
const VIEW: u32 = 12;

const WIDTH: f32 = 576.0;
const TITLE_H: f32 = 46.0;

const T_TICK: usize = 1;
const T_CARET: usize = 2;
const T_FADE: usize = 3;
const T_EXIT: usize = 4;

const CAP: Font = sf(11.5, 600);
const WM_MOUSELEAVE: u32 = 0x02A3;

pub struct Main {
    hwnd: HWND,
    g: Gfx,
    hrt: Option<ID2D1HwndRenderTarget>,
    surf: Option<Surface>,
    dpi: u32,
    ui: Ui,
    demo: Demo,
    view: View,
    limit: TextBox,
    popup: Option<Popup>,
    icon: Option<(u32, ID2D1Bitmap)>,
    tracking: bool,
    last: Instant,
    /// Window height in pixels (SizeToContent="Height").
    height: f32,
    queue_rect: Rect,
    /// --dpi: a fixed DPI (pictures at 100 %), else the screen's.
    fixed_dpi: Option<u32>,
}

fn scale_of(dpi: u32) -> f32 {
    dpi as f32 / 96.0
}

impl Main {
    pub fn create(args: &Args) -> Result<Box<Main>> {
        unsafe {
            let inst = GetModuleHandleW(None)?;
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                style: CS_DBLCLKS,
                lpfnWndProc: Some(wndproc),
                hInstance: inst.into(),
                lpszClassName: w!("ChoppyLookA"),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                ..Default::default()
            };
            RegisterClassExW(&wc);
            let mut demo = args.demo();
            let view = demo.view();
            let mut limit = TextBox::new(6, true);
            limit.set(&view.limit_text);
            let mut me = Box::new(Main {
                hwnd: HWND::default(),
                g: Gfx::new()?,
                hrt: None,
                surf: None,
                dpi: 96,
                ui: Ui::default(),
                demo,
                view,
                limit,
                popup: None,
                icon: None,
                tracking: false,
                last: Instant::now(),
                height: 300.0,
                queue_rect: Rect::default(),
                fixed_dpi: args.dpi,
            });
            // like WPF's WindowChrome: a captioned window whose frame is all client area (DWM still gives the shadow)
            let style = WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_CLIPCHILDREN;
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("ChoppyLookA"),
                w!("Choppy"),
                style,
                0,
                0,
                100,
                100,
                None,
                None,
                Some(inst.into()),
                Some(&mut *me as *mut Main as *const _),
            )?;
            me.hwnd = hwnd;
            me.dpi = args.dpi.unwrap_or_else(|| GetDpiForWindow(hwnd));
            // dark frame, rounded corners and the border color (Windows 11; harmless on 10) - Native2.Rounded
            let on: i32 = 1;
            let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4);
            let round: i32 = 2;
            let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(33), &round as *const _ as _, 4);
            let border: u32 = 0x00433A33;
            let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(34), &border as *const _ as _, 4);
            let m = MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 0, cyBottomHeight: 1 };
            let _ = DwmExtendFrameIntoClientArea(hwnd, &m);
            me.set_icons();
            me.height = me.measure();
            let s = scale_of(me.dpi);
            let (pw, ph) = ((WIDTH * s).round_ties_even() as i32, me.height as i32);
            let (x, y) = match args.pos {
                Some(p) => p,
                None => {
                    // WindowStartupLocation="CenterScreen": the work area of the screen with the mouse
                    let mut pt = POINT::default();
                    let _ = GetCursorPos(&mut pt);
                    let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTOPRIMARY);
                    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
                    let _ = GetMonitorInfoW(mon, &mut mi);
                    let wa = mi.rcWork;
                    (wa.left + (wa.right - wa.left - pw) / 2, wa.top + (wa.bottom - wa.top - ph) / 2)
                }
            };
            SetWindowPos(hwnd, None, x, y, pw, ph, SWP_NOZORDER | SWP_NOACTIVATE)?;
            // the dpi may differ where it landed
            let d = args.dpi.unwrap_or_else(|| GetDpiForWindow(hwnd));
            if d != me.dpi {
                me.dpi = d;
                me.set_icons();
                me.fit_height(false);
            }
            // no white flash: hidden from DWM until the first frame is drawn
            let cloak: i32 = 1;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_CLOAK, &cloak as *const _ as _, 4);
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = UpdateWindow(hwnd);
            let cloak: i32 = 0;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_CLOAK, &cloak as *const _ as _, 4);
            SetTimer(Some(hwnd), T_TICK, 100, None);
            SetTimer(Some(hwnd), T_CARET, GetCaretBlinkTime().max(100), None);
            if let Some(secs) = args.exit_after {
                SetTimer(Some(hwnd), T_EXIT, (secs * 1000.0) as u32, None);
            }
            Ok(me)
        }
    }

    fn set_icons(&self) {
        unsafe {
            let big = GetSystemMetricsForDpi(SM_CXICON, self.dpi) as u32;
            let small = GetSystemMetricsForDpi(SM_CXSMICON, self.dpi) as u32;
            if let Some(h) = icon::hicon(big) {
                SendMessageW(self.hwnd, WM_SETICON, Some(WPARAM(ICON_BIG as usize)), Some(LPARAM(h.0 as isize)));
            }
            if let Some(h) = icon::hicon(small) {
                SendMessageW(self.hwnd, WM_SETICON, Some(WPARAM(ICON_SMALL as usize)), Some(LPARAM(h.0 as isize)));
            }
        }
    }

    fn scale(&self) -> f32 {
        scale_of(self.dpi)
    }

    /// The window height in pixels for the current view (a layout pass that draws nothing).
    fn measure(&mut self) -> f32 {
        let scale = self.scale();
        let mut cv = Canvas { g: &self.g, s: None, scale, opacity: 1.0 };
        let mut ui = Ui::default();
        let mut st = St { demo: &mut self.demo, view: &self.view, limit: &mut self.limit, queue_rect: Rect::default(), popup_open: false };
        build(&mut cv, &mut ui, &mut st)
    }

    /// SizeToContent="Height": resize when the content's height changed; keep it on its screen (KeepOnScreen).
    fn fit_height(&mut self, keep_on_screen: bool) {
        let h = self.measure();
        let s = self.scale();
        unsafe {
            let mut r = RECT::default();
            let _ = GetWindowRect(self.hwnd, &mut r);
            let (pw, ph) = ((WIDTH * s).round_ties_even() as i32, h as i32);
            let mut top = r.top;
            if keep_on_screen {
                let mon = MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST);
                let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
                let _ = GetMonitorInfoW(mon, &mut mi);
                if top + ph > mi.rcWork.bottom {
                    top = mi.rcWork.top.max(mi.rcWork.bottom - ph - (8.0 * s) as i32);
                }
            }
            if h != self.height || r.right - r.left != pw || r.bottom - r.top != ph || top != r.top {
                self.height = h;
                let _ = SetWindowPos(self.hwnd, None, r.left, top, pw, ph, SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
    }

    fn paint(&mut self) {
        unsafe {
            let mut ps = PAINTSTRUCT::default();
            BeginPaint(self.hwnd, &mut ps);
            if self.hrt.is_none() {
                let mut rc = RECT::default();
                let _ = GetClientRect(self.hwnd, &mut rc);
                let props = D2D1_RENDER_TARGET_PROPERTIES {
                    r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                    pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE },
                    dpiX: self.dpi as f32,
                    dpiY: self.dpi as f32,
                    usage: D2D1_RENDER_TARGET_USAGE_NONE,
                    minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
                };
                let hp = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                    hwnd: self.hwnd,
                    pixelSize: D2D_SIZE_U { width: rc.right as u32, height: rc.bottom as u32 },
                    presentOptions: D2D1_PRESENT_OPTIONS_IMMEDIATELY,
                };
                if let Ok(rt) = self.g.d2d.CreateHwndRenderTarget(&props, &hp) {
                    // grayscale, like WPF draws this window (its text has no ClearType color fringes on screen)
                    rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
                    if let Ok(p) = self.g.dw.CreateRenderingParams() {
                        rt.SetTextRenderingParams(&p);
                    }
                    self.surf = Some(Surface::new(rt.cast().unwrap()));
                    self.hrt = Some(rt);
                    self.icon = None;
                }
            }
            if let (Some(rt), Some(_)) = (self.hrt.clone(), self.surf.as_ref()) {
                rt.SetDpi(self.dpi as f32, self.dpi as f32);
                let px = (20.0 * self.scale()).round() as u32;
                if self.icon.as_ref().is_none_or(|i| i.0 != px) {
                    self.icon = icon::bitmap(&rt.cast().unwrap(), px).map(|b| (px, b));
                }
                rt.BeginDraw();
                let scale = self.scale();
                let icon = self.icon.as_ref().map(|i| i.1.clone());
                let mut cv = Canvas { g: &self.g, s: self.surf.as_mut(), scale, opacity: 1.0 };
                self.ui.begin();
                frame(&mut cv, &mut self.ui, self.height, icon.as_ref(), "");
                let open = self.popup.as_ref().is_some_and(|p| p.visible());
                let mut st = St { demo: &mut self.demo, view: &self.view, limit: &mut self.limit, queue_rect: Rect::default(), popup_open: open };
                build(&mut cv, &mut self.ui, &mut st);
                self.queue_rect = st.queue_rect;
                self.ui.end();
                if rt.EndDraw(None, None).is_err() {
                    self.hrt = None;
                    self.surf = None;
                }
            }
            let _ = EndPaint(self.hwnd, &ps);
        }
    }

    fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f64().min(0.5);
        self.last = now;
        self.demo.advance(dt);
        self.view = self.demo.view();
        // SyncQuick: the box shows the setting unless it is being typed in
        if self.ui.focus != LIMIT_BOX && self.limit.text != self.view.limit_text {
            self.limit.set(&self.view.limit_text);
        }
        unsafe {
            let _ = SetWindowTextW(self.hwnd, &HSTRING::from(self.view.title.as_str()));
        }
        self.invalidate();
    }

    fn refresh(&mut self) {
        self.view = self.demo.view();
        if self.ui.focus != LIMIT_BOX {
            self.limit.set(&self.view.limit_text);
        }
        self.invalidate();
    }

    fn click(&mut self, id: u32) {
        match id {
            CLOSE => unsafe {
                let _ = DestroyWindow(self.hwnd);
                return;
            },
            UNIT => self.demo.mbps_unit = !self.demo.mbps_unit,
            TG_CLOSE => self.demo.close_when_done = !self.demo.close_when_done,
            TG_VERIFY => self.demo.verify = !self.demo.verify,
            TG_LIMIT => {
                let on = self.demo.limit_mbps <= 0.0;
                self.demo.set_limit_on(on);
                if on {
                    // LimitBox.Focus(); SelectAll()
                    self.ui.focus = LIMIT_BOX;
                    self.view = self.demo.view();
                    self.limit.set(&self.view.limit_text);
                    self.limit.select_all();
                } else if self.ui.focus == LIMIT_BOX {
                    self.ui.focus = 0;
                }
            }
            PAUSE => self.demo.paused = !self.demo.paused,
            SKIP => self.demo.skip(),
            CANCEL => self.demo.restart(),
            QUEUE => {
                if self.popup.as_ref().is_some_and(|p| p.visible()) {
                    self.close_popup();
                } else {
                    self.open_popup();
                }
            }
            VIEW => {
                self.close_popup();
                self.demo.extended = !self.demo.extended;
                self.view = self.demo.view();
                if !self.ui.focusable(self.ui.focus) {
                    // a hidden control loses focus
                }
                self.fit_height(true);
            }
            _ => {}
        }
        self.refresh();
    }

    // ---------- the Queue pop-up ----------

    fn popup_height(&self) -> f32 {
        let cv = Canvas { g: &self.g, s: None, scale: self.scale(), opacity: 1.0 };
        popup_h(&cv)
    }

    fn open_popup(&mut self) {
        if self.popup.is_none() {
            self.popup = Popup::new(&self.g, self.hwnd).ok();
        }
        let s = self.scale();
        let h = self.popup_height();
        let q = self.queue_rect;
        let mut wr = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.hwnd, &mut wr);
        }
        // Placement="Top", VerticalOffset="-6": its bottom 6 DIPs above the button, left edges lined up
        let x = wr.left + q.x as i32;
        let y = wr.top + (q.y - (6.0 * s).round_ties_even() - h) as i32;
        let w = (400.0 * s).round_ties_even();
        let size = (w as i32, h as i32);
        let g = &self.g;
        if let Some(p) = self.popup.as_mut() {
            p.alpha = 0;
            p.render(g, s, (x, y), size, |cv| draw_popup(cv, w, h));
            unsafe { SetTimer(Some(self.hwnd), T_FADE, 15, None) };
        }
    }

    fn close_popup(&mut self) {
        if let Some(p) = self.popup.as_mut()
            && p.visible()
        {
            p.hide();
            unsafe {
                let _ = KillTimer(Some(self.hwnd), T_FADE);
            }
            self.invalidate();
        }
    }

    fn popup_open(&self) -> bool {
        self.popup.as_ref().is_some_and(|p| p.visible())
    }

    // ---------- input ----------

    fn pt(&self, l: LPARAM) -> (f32, f32) {
        ((l.0 & 0xFFFF) as i16 as f32, ((l.0 >> 16) & 0xFFFF) as i16 as f32)
    }

    fn mouse_move(&mut self, x: f32, y: f32) {
        if !self.tracking {
            let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: self.hwnd, dwHoverTime: 0 };
            unsafe {
                let _ = TrackMouseEvent(&mut t);
            }
            self.tracking = true;
        }
        let hot = self.ui.at(x, y).map(|h| h.id).unwrap_or(0);
        if self.ui.pressed == LIMIT_BOX {
            let cv = Canvas { g: &self.g, s: None, scale: self.scale(), opacity: 1.0 };
            self.limit.mouse_move(&cv, &BOX_SMALL, x);
            self.invalidate();
        }
        if hot != self.ui.hot {
            self.ui.hot = hot;
            self.invalidate();
        }
    }

    fn mouse_down(&mut self, x: f32, y: f32, double: bool) {
        if self.popup_open() {
            // StaysOpen="False": a click outside closes it (the click itself goes nowhere)
            let on_queue = self.ui.at(x, y).is_some_and(|h| h.id == QUEUE);
            self.close_popup();
            if !on_queue {
                return;
            }
            // the Queue button toggles it closed
            self.ui.pressed = 0;
            return;
        }
        let Some(hit) = self.ui.at(x, y) else { return };
        unsafe { SetCapture(self.hwnd) };
        self.ui.pressed = hit.id;
        if self.ui.focusable(hit.id) {
            self.ui.focus = hit.id;
        }
        if hit.id == LIMIT_BOX {
            let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
            let cv = Canvas { g: &self.g, s: None, scale: self.scale(), opacity: 1.0 };
            self.limit.mouse_down(&cv, &BOX_SMALL, x, shift, double);
        }
        self.invalidate();
    }

    fn mouse_up(&mut self, x: f32, y: f32) {
        unsafe {
            let _ = ReleaseCapture();
        }
        let p = self.ui.pressed;
        self.ui.pressed = 0;
        self.limit.mouse_up();
        let over = self.ui.at(x, y).map(|h| h.id).unwrap_or(0);
        if p != 0 && p == over && p != LIMIT_BOX {
            self.click(p);
        }
        self.invalidate();
    }

    fn key_down(&mut self, vk: u16) -> bool {
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        if vk == VK_TAB.0 {
            self.ui.next_focus(shift);
            if self.ui.focus == LIMIT_BOX {
                self.limit.select_all();
            }
            self.invalidate();
            return true;
        }
        if self.ui.focus == LIMIT_BOX {
            if vk == VK_RETURN.0 || vk == VK_ESCAPE.0 {
                self.ui.focus = 0;
            } else if self.limit.key(vk, shift, ctrl) {
                let t = self.limit.text.clone();
                self.demo.set_limit_text(&t);
            }
            self.invalidate();
            return true;
        }
        let f = self.ui.focus;
        if f != 0 && (vk == VK_SPACE.0 || vk == VK_RETURN.0) {
            if vk == VK_RETURN.0 {
                self.click(f);
            } else {
                self.ui.key_pressed = true;
            }
            self.invalidate();
            return true;
        }
        // Space anywhere else: pause / resume
        if vk == VK_SPACE.0 {
            self.click(PAUSE);
            return true;
        }
        false
    }

    fn key_up(&mut self, vk: u16) {
        if vk == VK_SPACE.0 && self.ui.key_pressed {
            self.ui.key_pressed = false;
            let f = self.ui.focus;
            if f != 0 {
                self.click(f);
            }
            self.invalidate();
        }
    }

    fn handle(&mut self, msg: u32, w: WPARAM, l: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_NCCALCSIZE if w.0 != 0 => return Some(LRESULT(0)),
            WM_NCHITTEST => {
                let (sx, sy) = ((l.0 & 0xFFFF) as i16 as i32, ((l.0 >> 16) & 0xFFFF) as i16 as i32);
                let mut p = POINT { x: sx, y: sy };
                unsafe {
                    let _ = ScreenToClient(self.hwnd, &mut p);
                }
                let (x, y) = (p.x as f32, p.y as f32);
                // CaptionHeight="46": the title bar drags the window, except its buttons
                let on_button = self.ui.at(x, y).is_some_and(|h| h.id == SETTINGS || h.id == CLOSE);
                let cap = (TITLE_H * self.scale()).round_ties_even() + 1.0;
                return Some(LRESULT(if y < cap && !on_button { HTCAPTION as isize } else { HTCLIENT as isize }));
            }
            WM_ERASEBKGND => return Some(LRESULT(1)),
            WM_PAINT => {
                self.paint();
                return Some(LRESULT(0));
            }
            WM_SIZE => {
                if let Some(rt) = self.hrt.as_ref() {
                    let size = D2D_SIZE_U { width: (l.0 & 0xFFFF) as u32, height: ((l.0 >> 16) & 0xFFFF) as u32 };
                    unsafe {
                        let _ = rt.Resize(&size);
                    }
                }
                self.invalidate();
            }
            WM_DPICHANGED if self.fixed_dpi.is_none() => {
                self.dpi = (w.0 & 0xFFFF) as u32;
                let r = unsafe { &*(l.0 as *const RECT) };
                let h = self.measure();
                let s = self.scale();
                unsafe {
                    let _ = SetWindowPos(self.hwnd, None, r.left, r.top, (WIDTH * s).round_ties_even() as i32, h as i32, SWP_NOZORDER | SWP_NOACTIVATE);
                }
                self.height = h;
                self.set_icons();
                self.close_popup();
                self.invalidate();
                return Some(LRESULT(0));
            }
            WM_TIMER => match w.0 {
                T_TICK => self.tick(),
                T_CARET => {
                    self.limit.caret_on = !self.limit.caret_on;
                    if self.ui.focus == LIMIT_BOX {
                        self.invalidate();
                    }
                }
                T_FADE => {
                    if let Some(p) = self.popup.as_mut() {
                        let a = (p.alpha as u32 + 40).min(255) as u8;
                        p.set_alpha(a);
                        if a == 255 {
                            unsafe {
                                let _ = KillTimer(Some(self.hwnd), T_FADE);
                            }
                        }
                    }
                }
                T_EXIT => unsafe {
                    let _ = DestroyWindow(self.hwnd);
                },
                _ => {}
            },
            WM_MOUSEMOVE => {
                let (x, y) = self.pt(l);
                self.mouse_move(x, y);
            }
            WM_MOUSELEAVE => {
                self.tracking = false;
                if self.ui.hot != 0 {
                    self.ui.hot = 0;
                    self.invalidate();
                }
            }
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
                let (x, y) = self.pt(l);
                self.mouse_down(x, y, msg == WM_LBUTTONDBLCLK);
            }
            WM_LBUTTONUP => {
                let (x, y) = self.pt(l);
                self.mouse_up(x, y);
            }
            WM_SETCURSOR if (l.0 & 0xFFFF) as u32 == HTCLIENT => {
                let mut p = POINT::default();
                unsafe {
                    let _ = GetCursorPos(&mut p);
                    let _ = ScreenToClient(self.hwnd, &mut p);
                }
                let cur = self.ui.at(p.x as f32, p.y as f32).map(|h| h.cursor).unwrap_or(Cursor::Arrow);
                let id = match cur {
                    Cursor::Hand => IDC_HAND,
                    Cursor::IBeam => IDC_IBEAM,
                    Cursor::Arrow => IDC_ARROW,
                };
                unsafe {
                    SetCursor(LoadCursorW(None, id).ok());
                }
                return Some(LRESULT(1));
            }
            WM_KEYDOWN => {
                if self.key_down(w.0 as u16) {
                    return Some(LRESULT(0));
                }
            }
            WM_KEYUP => self.key_up(w.0 as u16),
            WM_CHAR => {
                if self.ui.focus == LIMIT_BOX
                    && let Some(ch) = char::from_u32(w.0 as u32)
                    && self.limit.char(ch)
                {
                    let t = self.limit.text.clone();
                    self.demo.set_limit_text(&t);
                    self.invalidate();
                }
                return Some(LRESULT(0));
            }
            WM_ACTIVATE => {
                if (w.0 & 0xFFFF) as u32 == WA_INACTIVE {
                    self.close_popup();
                }
            }
            WM_MOVE => self.close_popup(),
            WM_DESTROY => unsafe {
                PostQuitMessage(0);
            },
            _ => {}
        }
        None
    }
}

extern "system" fn wndproc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(l.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(h, GWLP_USERDATA, cs.lpCreateParams as isize);
            let me = cs.lpCreateParams as *mut Main;
            (*me).hwnd = h;
        }
        let me = GetWindowLongPtrW(h, GWLP_USERDATA) as *mut Main;
        if !me.is_null()
            && let Some(r) = (*me).handle(msg, w, l)
        {
            return r;
        }
        DefWindowProcW(h, msg, w, l)
    }
}

// ================= layout + drawing =================
//
// Everything is in device pixels and rounded like WPF's UseLayoutRounding: a length is rounded on its own
// (cv.rp), an element's offset inside its parent is rounded once, slot + margin + alignment together (cv.at,
// cv.center). The comments name the XAML each block follows.

struct St<'a> {
    demo: &'a mut Demo,
    view: &'a View,
    limit: &'a mut TextBox,
    queue_rect: Rect,
    popup_open: bool,
}

/// The window's width in pixels, its 1 DIP border (rounded) and the title row's height.
fn chrome(cv: &Canvas) -> (f32, f32, f32) {
    (cv.rp(WIDTH), cv.rp(1.0), cv.rp(TITLE_H))
}

/// Lays the content out (and draws it when the canvas has a surface). Returns the window height in pixels.
fn build(cv: &mut Canvas, ui: &mut Ui, st: &mut St) -> f32 {
    let ext = st.demo.extended;
    let v = st.view;
    let (ww, b, title_h) = chrome(cv);
    let h14 = cv.line_h(sf(14.0, 400));
    let cap_h = cv.line_h(CAP);

    // <StackPanel Grid.Row="1" Margin="20,0,20,18">: its corner and width
    let x0 = cv.at(b, cv.fp(20.0));
    let cw = ww - 2.0 * b - cv.rp(40.0);
    let right = x0 + cw;
    let sp = b + title_h;
    let mut y = 0.0; // offset inside the StackPanel

    // ---------- RouteGrid: columns 48 | 30 | Auto | * | Auto | Auto ----------
    let g0 = sp;
    let head = sf(15.5, 600);
    let from_name = cv.text(&[(demo::FROM_NAME, head, c::TEXT)]);
    let to_name = cv.text(&[(demo::TO_NAME, head, c::TEXT)]);
    let (col0, col1) = (cv.rp(48.0), cv.rp(30.0));
    let col2 = cv.tw(&from_name).max(cv.tw(&to_name));
    let pct_c = cv.text(&[(&v.percent, display(25.0, 700), c::PERCENT)]);
    // compact: the percent's column (MinWidth 70, Margin 12 left); the bar, stats and speeds end before it
    let col5 = if ext { 0.0 } else { cv.rp(12.0) + cv.tw(&pct_c).max(cv.rp(70.0)) };
    let span5_right = right - col5;
    let row_h = cv.rp(27.0);
    let path_font = sf(14.5, 400);
    let mut gy = 0.0; // offset inside the grid
    for (label, name, path) in [("FROM", &from_name, demo::FROM_PATH), ("TO", &to_name, demo::TO_PATH)] {
        let t = cv.text(&[(label, CAP, c::LABEL)]);
        cv.draw_text(&t, x0, cv.at(g0, gy + cv.center(row_h, 0.0, 0.0, cap_h)));
        // the drive icon: its geometry's size (17.65 x 13.65 with the stroke), centered
        let icon_h = cv.rp(13.65);
        controls::drive_icon(cv, x0 + col0, cv.at(g0, gy + cv.center(row_h, 0.0, 0.0, icon_h)), false);
        cv.draw_text(name, x0 + col0 + col1, cv.at(g0, gy + cv.center(row_h, 0.0, 0.0, cv.th(name))));
        // PathText: Margin 10,1,0,0, Height 20, centered
        let px0 = cv.at(x0, col0 + col1 + col2 + cv.fp(10.0));
        let pt = cv.text_trim(&[(path, path_font, c::DIM)], right - px0);
        cv.draw_text(&pt, px0, cv.at(g0, gy + cv.center(row_h, 1.0, 0.0, cv.rp(20.0))));
        gy += row_h;
    }

    // row 2, compact: speeds on their own row, right where the bar ends (Margin 0,4,0,-6)
    if !ext {
        let t = cv.text(&[
            (&v.c_now, sf(14.0, 600), c::NOW_SPEED),
            (&v.c_avg_label, sf(12.5, 400), c::DIM),
            (&v.c_avg, sf(14.0, 600), c::AVG_SPEED),
        ]);
        let th = cv.th(&t);
        let rh = th + cv.rp(4.0 - 6.0);
        cv.draw_text(&t, span5_right - cv.tw(&t), cv.at(g0, gy + cv.center(rh, 4.0, -6.0, th)));
        gy += rh;
    }

    // row 3, extended: the badge (Margin 0,12,0,0, centered) and the big percent (Margin 0,8,0,-4, bottom)
    if ext {
        let bf = sf(13.5, 600);
        let mode = cv.text(&[(&v.badge_mode, bf, c::ACCENT_TEXT)]);
        let detail = cv.text(&[(&v.badge_detail, sf(13.5, 400), c::ACCENT_TEXT)]);
        let has_detail = !v.badge_detail.is_empty();
        let dot = cv.rp(7.0);
        let content_w = dot + cv.rp(8.0) + cv.tw(&mode) + if has_detail { cv.rp(12.0) + cv.tw(&detail) } else { 0.0 };
        let content_h = cv.th(&mode).max(dot + cv.rp(1.0));
        // Border: thickness rounded, Padding 12,4,14,5 not
        let bw = (2.0 * b + cv.fp(12.0 + 14.0) + content_w).round_ties_even();
        let bh = (2.0 * b + cv.fp(4.0 + 5.0) + content_h).round_ties_even();
        let pct = cv.text(&[(&v.percent, display(30.0, 700), c::PERCENT)]);
        let ph = cv.th(&pct);
        let rh = (bh + cv.rp(12.0)).max(ph + cv.rp(8.0 - 4.0));
        let by = cv.at(g0, gy + cv.center(rh, 12.0, 0.0, bh));
        let br = Rect::new(x0, by, bw, bh);
        cv.border(br, 15.0, Some(c::BADGE_BG), Some(c::BADGE_BORDER), b);
        let cx = cv.at(br.x, b + cv.fp(12.0));
        let cy = cv.at(br.y, b + cv.fp(4.0));
        let ch = bh - 2.0 * b - cv.fp(4.0 + 5.0);
        let dy = cv.at(cy, cv.center(ch, 1.0, 0.0, dot));
        cv.ellipse(cx + dot / 2.0, dy + dot / 2.0, dot / 2.0, dot / 2.0, if v.badge_warn { c::WARN } else { c::NOW_SPEED });
        let ty = cv.at(cy, (ch - cv.th(&mode)) / 2.0);
        let mx = cv.at(cx, dot + cv.fp(8.0));
        cv.draw_text(&mode, mx, ty);
        if has_detail {
            cv.draw_text(&detail, cv.at(cx, dot + cv.rp(8.0) + cv.tw(&mode) + cv.fp(12.0)), ty);
        }
        // VerticalAlignment Bottom: the slot minus the bottom margin (-4)
        let py = cv.at(g0, gy + cv.fp(8.0) + (rh - cv.rp(8.0 - 4.0) - ph));
        cv.draw_text(&pct, right - cv.tw(&pct), py);
        gy += rh;
    }

    // row 4: the bar (Height 7, Margin 0,14,0,0) and, compact, the percent beside it (Margin 12,12,0,0)
    {
        let pch = cv.th(&pct_c);
        let bar_h = cv.rp(7.0);
        let rh = if ext { bar_h + cv.rp(14.0) } else { (bar_h + cv.rp(14.0)).max(pch + cv.rp(12.0)) };
        let ty = cv.at(g0, gy + cv.center(rh, 14.0, 0.0, bar_h));
        let track = Rect::new(x0, ty, span5_right - x0, bar_h);
        cv.fill_round(track, 3.5, c::TRACK);
        if v.frac > 0.0 {
            // Bar.Width = max(7, track x fraction)
            let bw = cv.rp((track.w / cv.scale * v.frac as f32).max(7.0));
            let bar = Rect::new(track.x, track.y, bw, bar_h);
            if let Some(gr) = cv.gradient(&[(0.0, c::PROGRESS_FROM), (1.0, c::PROGRESS_TO)], (bar.x, 0.0), (bar.r(), 0.0)) {
                cv.fill_round_brush(bar, 3.5, &gr);
            }
        }
        if !ext {
            // MinWidth 70, TextAlignment Right
            cv.draw_text(&pct_c, right - cv.tw(&pct_c), cv.at(g0, gy + cv.center(rh, 12.0, 0.0, pch)));
        }
        gy += rh;
    }

    // row 5: sizes · files · elapsed / remaining (Margin 0,9,0,0)
    {
        let sy = cv.at(g0, gy + cv.fp(9.0));
        let sizes = cv.text(&[
            (&v.size_done, sf(14.0, 600), c::SIZE_DONE),
            (&v.size_of, sf(14.0, 400), c::DIM),
            (&v.size_total, sf(14.0, 600), c::SIZE_TOTAL),
        ]);
        cv.draw_text(&sizes, x0, sy);
        let times = cv.text(&[
            ("Elapsed ", sf(14.0, 400), c::DIM),
            (&v.elapsed, sf(14.0, 600), c::TIME),
            (&v.remain_label, sf(14.0, 400), c::DIM),
            (&v.remain, sf(14.0, 600), c::REMAIN),
        ]);
        let tx = span5_right - cv.tw(&times);
        cv.draw_text(&times, tx, sy);
        if ext {
            // column 1 (between the two Auto columns), Margin 10,0, centered
            let files = cv.text(&[(&v.files, sf(14.0, 400), c::DIM)]);
            let l = x0 + cv.tw(&sizes);
            cv.draw_text(&files, cv.at(l, cv.center(tx - l, 10.0, 10.0, cv.tw(&files))), sy);
        }
        gy += h14 + cv.rp(9.0);
    }
    y += gy;

    if !ext {
        // compact: what is happening right now (Margin 0,12,0,0)
        let t = cv.text_trim(
            &[(&v.c2_label, sf(14.0, 400), if v.c2_warn { c::WARN } else { c::DIM }), (&v.c2_name, sf(14.0, 400), c::TEXT2)],
            cw,
        );
        cv.draw_text(&t, x0, cv.at(sp, y + cv.fp(12.0)));
        y += h14 + cv.rp(12.0);
    } else {
        // ---------- speed card (Card: Margin 0,14,0,0; Padding 16,10,16,10; border 1) ----------
        let card_y = cv.at(sp, y + cv.fp(14.0));
        let avg = cv.text(&[("AVG  ", sf(12.0, 600), c::DIM), (&v.avg_num, sf(15.0, 600), c::AVG_SPEED), (&v.avg_unit, sf(15.0, 600), c::AVG_SPEED)]);
        let now = cv.text(&[(&v.speed_num, display(24.0, 600), c::NOW_SPEED), (&v.speed_unit, display(24.0, 600), c::NOW_SPEED)]);
        let (ah, nh) = (cv.th(&avg), cv.th(&now));
        let content_w = cv.tw(&avg) + cv.rp(20.0) + cv.tw(&now);
        let content_h = ah.max(nh);
        // BtnUnit: Btn.Link (Border Padding 6,3), Margin 0,-4,-6,-4
        let ubw = (cv.fp(12.0) + content_w).round_ties_even();
        let ubh = (cv.fp(6.0) + content_h).round_ties_even();
        let row = cap_h.max(ubh + cv.rp(-8.0));
        let graph_h = cv.rp(70.0);
        let inner_h = row + graph_h + cv.rp(6.0);
        let card_h = (2.0 * b + cv.fp(20.0) + inner_h).round_ties_even();
        cv.border(Rect::new(x0, card_y, cw, card_h), 9.0, Some(c::CARD), Some(c::CARD_BORDER), b);
        let ix = cv.at(x0, b + cv.fp(16.0));
        let iy = cv.at(card_y, b + cv.fp(10.0));
        let iw = cw - 2.0 * b - cv.fp(32.0);
        let t = cv.text(&[("SPEED", CAP, c::LABEL)]);
        cv.draw_text(&t, ix, cv.at(iy, cv.center(row, 0.0, 0.0, cap_h)));
        // HorizontalAlignment Right with a -6 right margin; stretched to the row + 4 above and below
        let ux = cv.at(ix, iw - cv.rp(-6.0) - ubw);
        let uy = cv.at(iy, cv.fp(-4.0));
        let uh = row - cv.rp(-8.0);
        let ur = Rect::new(ux, uy, ubw, uh);
        button(cv, ui, UNIT, ur, &BTN_LINK, true, true);
        let cx = cv.at(ux, cv.fp(6.0));
        let cy = cv.at(uy, cv.fp(3.0));
        let ch = uh - cv.fp(6.0);
        cv.draw_text(&avg, cx, cv.at(cy, (ch - ah) / 2.0));
        cv.draw_text(&now, cv.at(cx, cv.tw(&avg) + cv.fp(20.0)), cv.at(cy, (ch - nh) / 2.0));
        let gy2 = cv.at(iy, row + cv.fp(6.0));
        controls::speed_graph(cv, &st.demo.graph, Rect::new(ix, gy2, iw, graph_h));
        y += cv.rp(14.0) + card_h;

        // ---------- current file (Margin 0,16,0,0) ----------
        let cur = cv.at(sp, y + cv.fp(16.0));
        let row = cap_h.max(h14);
        let t = cv.text(&[(&v.cur_label, CAP, c::LABEL)]);
        cv.draw_text(&t, x0, cv.at(cur, cv.center(row, 0.0, 0.0, cap_h)));
        let sz = cv.text(&[(&v.cur_done, sf(14.0, 600), c::SIZE_DONE), (&v.cur_of, sf(14.0, 400), c::DIM), (&v.cur_total, sf(14.0, 600), c::SIZE_TOTAL)]);
        cv.draw_text(&sz, right - cv.tw(&sz), cv.at(cur, cv.center(row, 0.0, 0.0, h14)));
        let name_y = cv.at(cur, row + cv.fp(4.0));
        let name = cv.text_trim(&[(&v.cur_name, sf(16.0, 600), c::TEXT)], cw);
        cv.draw_text(&name, x0, name_y);
        let name_h = cv.line_h(sf(16.0, 600));
        let bar_y = cv.at(cur, row + cv.rp(4.0) + name_h + cv.fp(9.0));
        let bar_h = cv.rp(4.0);
        cv.fill_round(Rect::new(x0, bar_y, cw, bar_h), 2.0, c::TRACK);
        let fw = (cw * v.cur_frac as f32).round();
        if fw > 0.0 {
            cv.fill_round(Rect::new(x0, bar_y, fw, bar_h), 2.0, c::NOW_SPEED);
        }
        y += cv.rp(16.0) + row + cv.rp(4.0) + name_h + cv.rp(9.0) + bar_h;

        // ---------- largest files (Margin 0,18,0,0) ----------
        let lg = cv.at(sp, y + cv.fp(18.0));
        let t = cv.text(&[("LARGEST FILES", CAP, c::LABEL)]);
        cv.draw_text(&t, x0, lg);
        let grid_y = cv.at(lg, cap_h + cv.fp(10.0));
        let parts = Demo::largest();
        let sq = cv.rp(10.0);
        let row_h = h14.max(sq + cv.rp(1.0));
        let gap = cv.rp(6.0);
        let legend_h = row_h * parts.len() as f32 + gap * (parts.len() as f32 - 1.0);
        let donut = cv.rp(76.0);
        let grid_h = legend_h.max(donut);
        let donut_parts: Vec<(f64, u32)> = parts.iter().map(|p| (p.1.max(1) as f64, p.2)).collect();
        controls::donut(cv, &donut_parts, Rect::new(x0, cv.at(grid_y, (grid_h - donut) / 2.0), donut, donut));
        // the legend: column 1 (80 wide column 0), Margin 18,0,0,0, centered
        let lx = cv.at(x0, cv.rp(80.0) + cv.fp(18.0));
        let ly0 = cv.at(grid_y, (grid_h - legend_h) / 2.0);
        let mut ry = 0.0;
        for (name, size, col) in parts.iter() {
            // each row: a Grid at Margin 0,6,0,0 (none for the first): square (Margin 0,1,10,0) | name | size (Margin 14 left)
            let rt = ly0 + ry;
            let sy = cv.at(rt, cv.center(row_h, 1.0, 0.0, sq));
            cv.fill_round(Rect::new(lx, sy, sq, sq), 2.0, *col);
            let st_ = cv.text(&[(&look_common::format::size(*size), sf(14.0, 400), c::DIM)]);
            let sx = right - cv.tw(&st_);
            cv.draw_text(&st_, sx, cv.at(rt, cv.center(row_h, 0.0, 0.0, h14)));
            let nx = lx + sq + cv.rp(10.0);
            let nt = cv.text_trim(&[(name, sf(14.0, 400), c::TEXT2)], sx - cv.rp(14.0) - nx);
            cv.draw_text(&nt, nx, cv.at(rt, cv.center(row_h, 0.0, 0.0, h14)));
            ry += row_h + gap;
        }
        y += cv.rp(18.0) + cap_h + cv.rp(10.0) + grid_h;

        // ---------- quick toggles (Margin 0,18,0,0; WrapPanel Margin 0,9,0,-6) ----------
        let qt = cv.at(sp, y + cv.fp(18.0));
        let t = cv.text(&[("QUICK TOGGLES", CAP, c::LABEL)]);
        cv.draw_text(&t, x0, qt);
        let qy = cv.at(qt, cap_h + cv.fp(9.0));
        let ph = cv.rp(32.0);
        let mut qx = 0.0;
        let w1 = quick_w(cv, "Close at completion");
        quick(cv, ui, TG_CLOSE, Rect::new(cv.at(x0, qx), qy, w1, ph), "Close at completion", st.demo.close_when_done);
        qx += w1 + cv.rp(7.0);
        let w2 = quick_w(cv, "Verify files");
        quick(cv, ui, TG_VERIFY, Rect::new(cv.at(x0, qx), qy, w2, ph), "Verify files", st.demo.verify);
        qx += w2 + cv.rp(7.0);
        // QuickPill (Padding 11,0,4,0): TgLimit (QuickDot) | a 1 x 18 line (Margin 10,0,8,0) | LimitBox 56 x 26 |
        // the unit (Margin 7,0,8,0); all centered
        let limit_on = st.demo.limit_mbps > 0.0;
        let label = cv.text(&[("Speed limit", sf(13.5, 400), 0)]);
        let unit = cv.text(&[(v.limit_unit, sf(13.5, 400), c::DIM)]);
        let dot_w = cv.rp(8.0) + cv.rp(8.0);
        let (line_w, line_h, box_w, box_h) = (cv.rp(1.0), cv.rp(18.0), cv.rp(56.0), cv.rp(26.0));
        let stack_w = dot_w + cv.tw(&label) + line_w + cv.rp(18.0) + box_w + cv.tw(&unit) + cv.rp(15.0);
        let pw = (2.0 * b + cv.fp(11.0 + 4.0) + stack_w).round_ties_even();
        let pill = Rect::new(cv.at(x0, qx), qy, pw, ph);
        cv.border(pill, 16.0, Some(0xFF22262C), Some(0xFF383E47), b);
        let sx = cv.at(pill.x, b + cv.fp(11.0));
        let (sy, sh) = (pill.y + b, ph - 2.0 * b);
        let w_dot = quick_dot(cv, ui, TG_LIMIT, sx, sy, sh, "Speed limit", limit_on);
        let mut off = w_dot;
        cv.fill(Rect::new(cv.at(sx, off + cv.fp(10.0)), cv.at(sy, cv.center(sh, 0.0, 0.0, line_h)), line_w, line_h), 0xFF3A4048);
        off += line_w + cv.rp(18.0);
        let bx = Rect::new(cv.at(sx, off), cv.at(sy, cv.center(sh, 0.0, 0.0, box_h)), box_w, box_h);
        st.limit.draw(cv, ui, LIMIT_BOX, bx, &BOX_SMALL, limit_on);
        off += box_w;
        cv.draw_text(&unit, cv.at(sx, off + cv.fp(7.0)), cv.at(sy, cv.center(sh, 0.0, 0.0, cv.th(&unit))));
        y += cv.rp(18.0) + cap_h + cv.rp(9.0 - 6.0) + ph + cv.rp(6.0);
    }

    // ---------- actions (Margin 0,16,0,0): buttons 36 high, the view switch on the right ----------
    let ay = cv.at(sp, y + cv.fp(16.0));
    let bh = cv.rp(36.0);
    let bf = sf(14.0, 600);
    let gap = cv.rp(8.0);
    let mut ax = 0.0;
    let wp = text_button_w(cv, &BTN_PRIMARY, &v.pause_text, bf, 18.0, 84.0);
    text_button(cv, ui, PAUSE, Rect::new(cv.at(x0, ax), ay, wp, bh), &BTN_PRIMARY, &v.pause_text, bf, 18.0, true);
    ax += wp;
    if ext {
        let ws = text_button_w(cv, &BTN, "Skip", bf, 18.0, 0.0);
        text_button(cv, ui, SKIP, Rect::new(cv.at(x0, ax + cv.fp(8.0)), ay, ws, bh), &BTN, "Skip", bf, 18.0, true);
        ax += gap + ws;
    }
    let wc = text_button_w(cv, &BTN_CANCEL, "Cancel", bf, 18.0, 0.0);
    text_button(cv, ui, CANCEL, Rect::new(cv.at(x0, ax + cv.fp(8.0)), ay, wc, bh), &BTN_CANCEL, "Cancel", bf, 18.0, true);
    ax += gap + wc;
    if ext {
        // Queue: "Queue   " dim + the count (FontWeight Normal), a chevron (Margin 10,2,0,0); Padding 14,0
        let q = cv.text(&[("Queue   ", sf(14.0, 400), c::DIM), (&st.demo.queue.to_string(), sf(14.0, 600), c::TEXT)]);
        let chev = cv.text(&[(if st.popup_open { "\u{E70E}" } else { "\u{E70D}" }, icons(9.0), c::DIM)]);
        let (qh, chh) = (cv.th(&q), cv.th(&chev));
        let content_w = cv.tw(&q) + cv.rp(10.0) + cv.tw(&chev);
        let content_h = qh.max(chh + cv.rp(2.0));
        let wq = 2.0 * b + cv.rp(28.0) + content_w;
        let r = Rect::new(cv.at(x0, ax + cv.fp(8.0)), ay, wq, bh);
        button(cv, ui, QUEUE, r, &BTN, true, true);
        let cx = cv.at(r.x + b, cv.center(wq - 2.0 * b, 14.0, 14.0, content_w));
        let cy = cv.at(r.y + b, cv.center(bh - 2.0 * b, 0.0, 0.0, content_h));
        cv.draw_text(&q, cx, cv.at(cy, cv.center(content_h, 0.0, 0.0, qh)));
        cv.draw_text(&chev, cv.at(cx, cv.tw(&q) + cv.fp(10.0)), cv.at(cy, cv.center(content_h, 2.0, 0.0, chh)));
        st.queue_rect = r;
    }
    // Compact / Extended: Btn.Link (Padding 6,3), right, centered; text 14.5 + chevron (Margin 9,2,0,0)
    {
        let vt = cv.text(&[(if ext { "Compact" } else { "Extended" }, sf(14.5, 400), c::TEXT2)]);
        let chev = cv.text(&[(if ext { "\u{E70E}" } else { "\u{E70D}" }, icons(10.0), c::DIM)]);
        let (vh, chh) = (cv.th(&vt), cv.th(&chev));
        let content_w = cv.tw(&vt) + cv.rp(9.0) + cv.tw(&chev);
        let content_h = vh.max(chh + cv.rp(2.0));
        let vbw = (cv.fp(12.0) + content_w).round_ties_even();
        let vbh = (cv.fp(6.0) + content_h).round_ties_even();
        let r = Rect::new(right - vbw, cv.at(ay, (bh - vbh) / 2.0), vbw, vbh);
        button(cv, ui, VIEW, r, &BTN_LINK, true, true);
        let cx = cv.at(r.x, cv.fp(6.0));
        let cy = cv.at(r.y, cv.fp(3.0));
        let ch = vbh - cv.fp(6.0);
        cv.draw_text(&vt, cx, cv.at(cy, (ch - vh) / 2.0));
        cv.draw_text(&chev, cv.at(cx, cv.tw(&vt) + cv.fp(9.0)), cv.at(cy, cv.center(ch, 2.0, 0.0, chh)));
    }
    y += cv.rp(16.0) + bh;
    // the StackPanel's bottom margin (18) and the window border
    sp + y + cv.rp(18.0) + b
}

/// The window frame and the title bar. Drawn first; `build` draws the content over it. `height` in pixels.
pub fn frame(cv: &mut Canvas, ui: &mut Ui, height: f32, icon: Option<&ID2D1Bitmap>, title_extra: &str) {
    let (ww, b, title_h) = chrome(cv);
    let all = Rect::new(0.0, 0.0, ww, height);
    if let Some(g) = cv.gradient(&[(0.0, c::WIN_TOP), (1.0, c::WIN_BOTTOM)], (0.0, b), (0.0, height - b)) {
        cv.fill_round_brush(all, 0.0, &g);
    }
    cv.border(all, 0.0, None, Some(c::WIN_BORDER), b);
    // title: a StackPanel (Margin 18,0,0,0, centered): icon 20 x 20 | "Choppy" 15 SemiBold (Margin 10,0,0,1) |
    // the extra 13 (Margin 8,1,0,0)
    let title = cv.text(&[("Choppy", sf(15.0, 600), c::TITLE_TEXT)]);
    let extra = cv.text(&[(title_extra, sf(13.0, 400), c::FAINT)]);
    let img = cv.rp(20.0);
    let th = cv.th(&title);
    let stack_h = img.max(th + cv.rp(1.0)).max(if title_extra.is_empty() { 0.0 } else { cv.th(&extra) + cv.rp(1.0) });
    let sx = cv.at(b, cv.fp(18.0));
    let sy = cv.at(b, (title_h - stack_h) / 2.0);
    if let Some(bm) = icon {
        cv.bitmap(bm, Rect::new(sx, cv.at(sy, (stack_h - img) / 2.0), img, img));
    }
    let tx = cv.at(sx, img + cv.fp(10.0));
    cv.draw_text(&title, tx, cv.at(sy, cv.center(stack_h, 0.0, 1.0, th)));
    if !title_extra.is_empty() {
        let ex = cv.at(sx, img + cv.rp(10.0) + cv.tw(&title) + cv.fp(8.0));
        cv.draw_text(&extra, ex, cv.at(sy, cv.center(stack_h, 1.0, 0.0, cv.th(&extra))));
    }
    // gear + close: 46 x 46 each, at the right edge
    let bw = cv.rp(46.0);
    for (id, glyph, size, st, x) in [
        (SETTINGS, "\u{E713}", 16.0, &BTN_TITLE, ww - b - 2.0 * bw),
        (CLOSE, "\u{E8BB}", 12.0, &BTN_TITLE_CLOSE, ww - b - bw),
    ] {
        let r = Rect::new(x, b, bw, title_h);
        let fg = button(cv, ui, id, r, st, true, false);
        let t = cv.text(&[(glyph, icons(size), fg)]);
        cv.draw_text(&t, cv.at(r.x, (bw - cv.tw(&t)) / 2.0), cv.at(r.y, (title_h - cv.th(&t)) / 2.0));
    }
}

/// The Queue pop-up: #22262C, border #3A4048, corners 8, Padding 6; nothing waiting. `w`, `h` in pixels.
fn draw_popup(cv: &mut Canvas, w: f32, h: f32) {
    cv.border(Rect::new(0.0, 0.0, w, h), 8.0, Some(0xFF22262C), Some(0xFF3A4048), cv.rp(1.0));
    let t = cv.text_trim(&[("Nothing waiting. A new copy waits here while one is running.", sf(12.5, 400), c::FAINT)], w - cv.rp(2.0 + 12.0 + 12.0));
    let x = cv.at(cv.rp(1.0), cv.fp(6.0 + 8.0));
    let y = cv.at(cv.rp(1.0), cv.fp(6.0 + 4.0));
    cv.draw_text(&t, x, y);
}

/// The pop-up's height in pixels: border + Padding 6 + the text line (Margin 8,4,4,4).
fn popup_h(cv: &Canvas) -> f32 {
    (2.0 * cv.rp(1.0) + cv.fp(12.0) + cv.line_h(sf(12.5, 400)) + cv.rp(8.0)).round_ties_even()
}
