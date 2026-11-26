//! Windows on the UI thread (step 4): our own frameless windows (WPF's WindowChrome look: the whole window is client
//! area, DWM still gives the shadow and the rounded corners on Windows 11), layered pop-ups, the UI thread's work
//! queue (C#'s Dispatcher.BeginInvoke) and timers, monitors, the tray icon, the taskbar button's progress, the
//! clipboard and the shell (open a folder / file, Explorer's thumbnails).
//!
//! A window's handler is an `Rc<RefCell<dyn Handler>>`. Windows sends some messages while the handler is busy
//! (SetWindowPos inside a click sends WM_SIZE, WM_DPICHANGED...): those wait in the window's queue and reach the
//! handler right after it returns. Hit testing and cursors come from data the handler leaves on the window after
//! each layout, so they never need the handler.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::Mutex;

use core::ffi::c_void;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

use super::gfx::{Canvas, Gfx, Rect, Target};

const WM_MOUSELEAVE_: u32 = 0x02A3;
const WM_DISPATCH: u32 = WM_APP + 1;
const WM_TRAY: u32 = WM_APP + 2;
/// The resource id of the app icon (build.rs: `32512 ICON`).
const ICON_ID: u16 = 32512;

/// What a window gets from Windows. Coordinates are client pixels.
#[derive(Clone, Copy, Debug)]
pub enum Ev {
    Paint,
    Size(u32, u32),
    /// The new DPI and the rectangle Windows suggests (left, top, right, bottom).
    Dpi(u32, (i32, i32, i32, i32)),
    Timer(usize),
    MouseMove(f64, f64),
    MouseLeave,
    Down(f64, f64, bool),
    Up(f64, f64),
    RightUp(f64, f64),
    /// Wheel delta (120 a notch) at a client point.
    Wheel(i32, f64, f64),
    KeyDown(u16),
    KeyUp(u16),
    Char(u32),
    Activate(bool),
    Moved,
    /// The user asked to close (Alt+F4, the system menu): the handler decides (true = handled, stays open).
    Close,
    Destroyed,
    CaptureLost,
    /// A device change (WM_DEVICECHANGE's wParam, and whether a volume came or went: DBT_DEVTYP_VOLUME).
    DeviceChange(usize, bool),
    /// The user starts / ends moving or sizing the window; which edge is being dragged (WM_SIZING's wParam).
    EnterSizeMove,
    ExitSizeMove,
    Sizing(u32),
    /// The tray icon's callback (the mouse message).
    Tray,
    /// Minimized / maximized / restored (WM_SIZE's wParam).
    State(u32),
}

pub trait Handler {
    /// Returns true when the event was handled (Windows' default handling is skipped).
    fn event(&mut self, w: Win, ev: Ev) -> bool;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Cursor {
    #[default]
    Arrow,
    Hand,
    IBeam,
}

/// Where the window can be dragged and resized, and which cursor goes where (pixels). Set after each layout.
#[derive(Default, Clone)]
pub struct HitData {
    /// The title bar's height (drags the window); 0 = none.
    pub caption: f64,
    /// Rectangles inside the title bar that are buttons, not the bar.
    pub no_drag: Vec<Rect>,
    /// Resize border thickness (0 = not resizable).
    pub resize: f64,
    pub cursors: Vec<(Rect, Cursor)>,
}

struct Slot {
    handler: Rc<RefCell<dyn Handler>>,
    queue: RefCell<VecDeque<Ev>>,
    repaint: Cell<bool>,
    hit: RefCell<HitData>,
    /// Smallest size the user can drag it to (pixels), and whether a maximize fills only the work area.
    min_track: Cell<(i32, i32)>,
    tracking: Cell<bool>,
    destroyed: Cell<bool>,
    paint_pending: Cell<bool>,
    drain_posted: Cell<bool>,
}

/// A window handle on the UI thread (Copy, like HWND).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[derive(Default)]
pub struct Win(HWND);


/// How a window is made.
#[derive(Clone, Default)]
pub struct Spec {
    pub title: String,
    pub owner: Option<Win>,
    /// Frameless with WPF WindowChrome's look (captioned, DWM shadow): true. A layered pop-up: false.
    pub resizable: bool,
    pub minimize: bool,
    pub maximize: bool,
    pub taskbar: bool,
    pub topmost: bool,
    /// A layered pop-up that never takes the focus (tooltips, the queue, the "?" card).
    pub popup: bool,
    /// A pop-up that does take the focus (the tray menu: it closes when another window is clicked).
    pub activate: bool,
}

fn inst() -> HINSTANCE {
    unsafe { GetModuleHandleW(None).map(|m| m.into()).unwrap_or_default() }
}

fn register(name: PCWSTR, proc: WNDPROC, dbl: bool) {
    unsafe {
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: if dbl { CS_DBLCLKS } else { WNDCLASS_STYLES(0) },
            lpfnWndProc: proc,
            hInstance: inst(),
            lpszClassName: name,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: LoadIconW(Some(inst()), PCWSTR(ICON_ID as usize as *const u16)).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassExW(&wc);
    }
}

thread_local! {
    static CLASSES: Cell<bool> = const { Cell::new(false) };
}

fn ensure_classes() {
    if CLASSES.with(|c| c.replace(true)) {
        return;
    }
    register(w!("ChoppyWindow"), Some(wndproc), true);
    register(w!("ChoppyPopup"), Some(wndproc), false);
    register(w!("ChoppyDispatch"), Some(dispatch_proc), false);
}

impl Win {
    pub fn create(spec: &Spec, handler: Rc<RefCell<dyn Handler>>) -> Option<Win> {
        ensure_classes();
        let slot = Box::new(Slot {
            handler,
            queue: RefCell::new(VecDeque::new()),
            repaint: Cell::new(false),
            hit: RefCell::new(HitData::default()),
            min_track: Cell::new((0, 0)),
            tracking: Cell::new(false),
            destroyed: Cell::new(false),
            paint_pending: Cell::new(false),
            drain_posted: Cell::new(false),
        });
        let raw = Box::into_raw(slot);
        unsafe {
            let (class, style, ex) = if spec.popup {
                let mut ex = WS_EX_LAYERED | WS_EX_TOOLWINDOW;
                if !spec.activate {
                    ex |= WS_EX_NOACTIVATE;
                }
                if spec.topmost {
                    ex |= WS_EX_TOPMOST;
                }
                (w!("ChoppyPopup"), WS_POPUP, ex)
            } else {
                // like WPF's WindowChrome: a captioned window whose frame is all client area
                let mut st = WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN;
                if spec.minimize {
                    st |= WS_MINIMIZEBOX;
                }
                if spec.maximize {
                    st |= WS_MAXIMIZEBOX;
                }
                if spec.resizable {
                    st |= WS_THICKFRAME;
                }
                let mut ex = WINDOW_EX_STYLE(0);
                if spec.taskbar {
                    ex |= WS_EX_APPWINDOW;
                } else {
                    ex |= WS_EX_TOOLWINDOW;
                }
                if spec.topmost {
                    ex |= WS_EX_TOPMOST;
                }
                (w!("ChoppyWindow"), st, ex)
            };
            let title = HSTRING::from(spec.title.as_str());
            let hwnd = CreateWindowExW(
                ex,
                class,
                &title,
                style,
                -32000,
                -32000,
                100,
                100,
                spec.owner.map(|o| o.0),
                None,
                Some(inst()),
                Some(raw as *const c_void),
            );
            let Ok(hwnd) = hwnd else {
                drop(Box::from_raw(raw));
                return None;
            };
            let w = Win(hwnd);
            if !spec.popup {
                // dark frame, rounded corners and the border color (Windows 11; harmless on 10) - Native2.Rounded
                let on: i32 = 1;
                let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4);
                let round: i32 = 2;
                let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(33), &round as *const _ as _, 4);
                let border: u32 = 0x00433A33;
                let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(34), &border as *const _ as _, 4);
                // GlassFrameThickness="0,0,0,1": the DWM frame (and its shadow) stays
                let m = MARGINS { cxLeftWidth: 0, cxRightWidth: 0, cyTopHeight: 0, cyBottomHeight: 1 };
                let _ = DwmExtendFrameIntoClientArea(hwnd, &m);
                w.set_icons();
            }
            Some(w)
        }
    }

    pub fn raw(&self) -> isize {
        self.0.0 as isize
    }
    pub fn is_null(&self) -> bool {
        self.0.0.is_null()
    }
    pub fn exists(&self) -> bool {
        !self.is_null() && unsafe { IsWindow(Some(self.0)).as_bool() }
    }

    fn slot(&self) -> Option<&Slot> {
        let p = unsafe { GetWindowLongPtrW(self.0, GWLP_USERDATA) } as *const Slot;
        if p.is_null() { None } else { Some(unsafe { &*p }) }
    }

    pub fn set_hit(&self, h: HitData) {
        if let Some(s) = self.slot() {
            *s.hit.borrow_mut() = h;
        }
    }
    pub fn set_min_track(&self, w: i32, h: i32) {
        if let Some(s) = self.slot() {
            s.min_track.set((w, h));
        }
    }

    /// The window's icons (taskbar, Alt+Tab): the exe's icon at the sizes this DPI wants.
    pub fn set_icons(&self) {
        unsafe {
            let dpi = GetDpiForWindow(self.0);
            for (which, metric) in [(ICON_BIG, SM_CXICON), (ICON_SMALL, SM_CXSMICON)] {
                let px = GetSystemMetricsForDpi(metric, dpi);
                if let Ok(h) = LoadImageW(Some(inst()), PCWSTR(ICON_ID as usize as *const u16), IMAGE_ICON, px, px, LR_DEFAULTCOLOR) {
                    SendMessageW(self.0, WM_SETICON, Some(WPARAM(which as usize)), Some(LPARAM(h.0 as isize)));
                }
            }
        }
    }

    pub fn dpi(&self) -> u32 {
        unsafe { GetDpiForWindow(self.0) }.max(96)
    }

    /// GetWindowRect: (left, top, right, bottom) in screen pixels.
    pub fn rect(&self) -> (i32, i32, i32, i32) {
        let mut r = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.0, &mut r);
        }
        (r.left, r.top, r.right, r.bottom)
    }
    pub fn client_size(&self) -> (u32, u32) {
        let mut r = RECT::default();
        unsafe {
            let _ = GetClientRect(self.0, &mut r);
        }
        ((r.right - r.left).max(0) as u32, (r.bottom - r.top).max(0) as u32)
    }

    pub fn set_pos(&self, x: i32, y: i32, w: i32, h: i32) {
        unsafe {
            let _ = SetWindowPos(self.0, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
        }
    }
    pub fn move_to(&self, x: i32, y: i32) {
        unsafe {
            let _ = SetWindowPos(self.0, None, x, y, 0, 0, SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOSIZE | SWP_NOOWNERZORDER);
        }
    }
    pub fn resize(&self, w: i32, h: i32) {
        unsafe {
            let _ = SetWindowPos(self.0, None, 0, 0, w, h, SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOOWNERZORDER);
        }
    }

    /// Shows it; `activate` = it takes the focus (WPF's ShowActivated).
    pub fn show(&self, activate: bool) {
        unsafe {
            let _ = ShowWindow(self.0, if activate { SW_SHOW } else { SW_SHOWNOACTIVATE });
        }
    }
    /// Shown without painting first: hidden from DWM until its first frame is drawn (no white flash).
    pub fn show_drawn(&self, activate: bool) {
        unsafe {
            let cloak: i32 = 1;
            let _ = DwmSetWindowAttribute(self.0, DWMWA_CLOAK, &cloak as *const _ as _, 4);
            self.show(activate);
            let _ = UpdateWindow(self.0);
            let cloak: i32 = 0;
            let _ = DwmSetWindowAttribute(self.0, DWMWA_CLOAK, &cloak as *const _ as _, 4);
        }
    }
    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.0, SW_HIDE);
        }
    }
    pub fn is_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.0).as_bool() }
    }
    pub fn is_minimized(&self) -> bool {
        unsafe { IsIconic(self.0).as_bool() }
    }
    pub fn is_maximized(&self) -> bool {
        unsafe { IsZoomed(self.0).as_bool() }
    }
    pub fn minimize(&self) {
        unsafe {
            let _ = ShowWindow(self.0, SW_MINIMIZE);
        }
    }
    pub fn maximize(&self) {
        unsafe {
            let _ = ShowWindow(self.0, SW_MAXIMIZE);
        }
    }
    pub fn restore(&self) {
        unsafe {
            let _ = ShowWindow(self.0, SW_RESTORE);
        }
    }
    /// Maximize as a title-bar double click does (WM_SYSCOMMAND SC_MAXIMIZE).
    pub fn sys_maximize(&self) {
        unsafe {
            let _ = SendMessageW(self.0, WM_SYSCOMMAND, Some(WPARAM(SC_MAXIMIZE as usize)), Some(LPARAM(0)));
        }
    }
    pub fn destroy(&self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
    pub fn set_title(&self, t: &str) {
        unsafe {
            let _ = SetWindowTextW(self.0, &HSTRING::from(t));
        }
    }
    pub fn set_topmost(&self, on: bool) {
        unsafe {
            let after = if on { HWND_TOPMOST } else { HWND_NOTOPMOST };
            let _ = SetWindowPos(self.0, Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
    /// Window.Activate: true when it got the focus.
    pub fn activate(&self) -> bool {
        unsafe { SetForegroundWindow(self.0).as_bool() }
    }
    pub fn enable(&self, on: bool) {
        unsafe {
            let _ = EnableWindow(self.0, on);
        }
    }
    pub fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.0), None, false);
        }
    }
    pub fn capture(&self) {
        unsafe {
            SetCapture(self.0);
        }
    }
    pub fn release_capture(&self) {
        unsafe {
            let _ = ReleaseCapture();
        }
    }
    pub fn set_timer(&self, id: usize, ms: u32) {
        unsafe {
            SetTimer(Some(self.0), id, ms, None);
        }
    }
    pub fn kill_timer(&self, id: usize) {
        unsafe {
            let _ = KillTimer(Some(self.0), id);
        }
    }
    /// The mouse cursor in client pixels.
    pub fn cursor_pos(&self) -> (f64, f64) {
        let mut p = POINT::default();
        unsafe {
            let _ = GetCursorPos(&mut p);
            let _ = ScreenToClient(self.0, &mut p);
        }
        (p.x as f64, p.y as f64)
    }

    /// The work area (the taskbar left out) of the monitor the window is on: (left, top, right, bottom) pixels.
    pub fn work_area(&self) -> (i32, i32, i32, i32) {
        unsafe { monitor_work(MonitorFromWindow(self.0, MONITOR_DEFAULTTONEAREST)) }
    }

    /// Draws a layered pop-up: `paint` draws into a `w` x `h` pixel picture at `scale`, shown at (x, y) with `alpha`.
    pub fn update_layered(&self, g: &Gfx, t: &mut Target, pos: (i32, i32), size: (i32, i32), scale: f64, alpha: u8, paint: impl FnOnce(&mut Canvas)) {
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: size.0.max(1),
                    biHeight: -size.1.max(1),
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
            t.set_dpi(96.0 * scale);
            if t.bind_dc(mem, size.0, size.1) {
                t.begin();
                t.clear(0);
                {
                    let mut cv = Canvas::new(g, t, scale);
                    paint(&mut cv);
                }
                t.end();
            }
            let pt = POINT { x: pos.0, y: pos.1 };
            let sz = SIZE { cx: size.0, cy: size.1 };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: alpha, AlphaFormat: AC_SRC_ALPHA as u8 };
            let _ = UpdateLayeredWindow(self.0, Some(screen), Some(&pt), Some(&sz), Some(mem), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA);
            SelectObject(mem, old);
            let _ = DeleteObject(dib.into());
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
        }
    }

    /// The taskbar button's progress (TaskbarItemInfo): 0 none, 1 indeterminate, 2 normal, 8 paused.
    pub fn taskbar_progress(&self, state: u32, value: f64) {
        TASKBAR.with(|t| {
            let mut t = t.borrow_mut();
            if t.is_none() {
                *t = unsafe { CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_INPROC_SERVER).ok() };
                if let Some(tb) = t.as_ref() {
                    unsafe {
                        let _ = tb.HrInit();
                    }
                }
            }
            if let Some(tb) = t.as_ref() {
                unsafe {
                    let _ = tb.SetProgressState(self.0, TBPFLAG(state as i32));
                    if state == 2 || state == 8 {
                        let _ = tb.SetProgressValue(self.0, (value.clamp(0.0, 1.0) * 1000.0) as u64, 1000);
                    }
                }
            }
        });
    }
}

thread_local! {
    static TASKBAR: RefCell<Option<ITaskbarList3>> = const { RefCell::new(None) };
}

unsafe fn monitor_work(mon: HMONITOR) -> (i32, i32, i32, i32) {
    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let _ = GetMonitorInfoW(mon, &mut mi);
    }
    let w = mi.rcWork;
    (w.left, w.top, w.right, w.bottom)
}

/// The system DPI (the primary screen's at sign-in): what WinForms and ChoppyIcon.Frame go by.
/// Test hook only (CHOPPY_DPI96): this thread's windows DPI-unaware, laid out and drawn at 96 DPI (100 %).
pub fn thread_dpi_unaware() {
    unsafe {
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_UNAWARE);
    }
}

pub fn system_dpi() -> u32 {
    unsafe { GetDpiForSystem() }.max(96)
}

/// The primary screen's work area (SystemParameters.WorkArea), pixels.
pub fn primary_work_area() -> (i32, i32, i32, i32) {
    unsafe { monitor_work(MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY)) }
}

/// The work area of the monitor at a point, and that monitor's DPI.
pub fn work_area_at(x: i32, y: i32) -> ((i32, i32, i32, i32), u32) {
    unsafe {
        let mon = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        (monitor_work(mon), dx.max(96))
    }
}

/// The DPI of the monitor with the mouse on it.
pub fn dpi_at_cursor() -> u32 {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    work_area_at(p.x, p.y).1
}

pub fn cursor_screen() -> (i32, i32) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

/// The system's hover time (a tooltip's delay, WPF's InitialShowDelay) and caret blink time, ms.
pub fn hover_time() -> u32 {
    let mut v = 400u32;
    unsafe {
        let _ = SystemParametersInfoW(SPI_GETMOUSEHOVERTIME, 0, Some(&mut v as *mut u32 as *mut c_void), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    }
    v
}
pub fn caret_blink_ms() -> u32 {
    unsafe { GetCaretBlinkTime() }.clamp(100, 2000)
}

pub fn key_down(vk: u16) -> bool {
    unsafe { GetKeyState(vk as i32) < 0 }
}

/// Every top-level window of this thread that is visible and enabled (WPF's ShowDialog disables them all).
pub fn thread_windows() -> Vec<Win> {
    let mut out: Vec<Win> = Vec::new();
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        let v = unsafe { &mut *(l.0 as *mut Vec<Win>) };
        if unsafe { IsWindowVisible(h).as_bool() && IsWindowEnabled(h).as_bool() } {
            v.push(Win(h));
        }
        BOOL(1)
    }
    unsafe {
        let _ = EnumThreadWindows(GetCurrentThreadId(), Some(cb), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

fn lparam_pt(l: LPARAM) -> (f64, f64) {
    ((l.0 & 0xFFFF) as i16 as f64, ((l.0 >> 16) & 0xFFFF) as i16 as f64)
}

fn decode(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> Option<Ev> {
    Some(match msg {
        WM_PAINT => Ev::Paint,
        WM_SIZE => Ev::Size((l.0 & 0xFFFF) as u32, ((l.0 >> 16) & 0xFFFF) as u32),
        WM_DPICHANGED => {
            let r = unsafe { &*(l.0 as *const RECT) };
            Ev::Dpi((w.0 & 0xFFFF) as u32, (r.left, r.top, r.right, r.bottom))
        }
        WM_TIMER => Ev::Timer(w.0),
        WM_MOUSEMOVE => {
            let (x, y) = lparam_pt(l);
            Ev::MouseMove(x, y)
        }
        WM_MOUSELEAVE_ => Ev::MouseLeave,
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let (x, y) = lparam_pt(l);
            Ev::Down(x, y, msg == WM_LBUTTONDBLCLK)
        }
        WM_LBUTTONUP => {
            let (x, y) = lparam_pt(l);
            Ev::Up(x, y)
        }
        WM_RBUTTONUP => {
            let (x, y) = lparam_pt(l);
            Ev::RightUp(x, y)
        }
        WM_MOUSEWHEEL => {
            let mut p = POINT { x: (l.0 & 0xFFFF) as i16 as i32, y: ((l.0 >> 16) & 0xFFFF) as i16 as i32 };
            unsafe {
                let _ = ScreenToClient(h, &mut p);
            }
            Ev::Wheel(((w.0 >> 16) & 0xFFFF) as i16 as i32, p.x as f64, p.y as f64)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => Ev::KeyDown(w.0 as u16),
        WM_KEYUP | WM_SYSKEYUP => Ev::KeyUp(w.0 as u16),
        WM_CHAR => Ev::Char(w.0 as u32),
        WM_ACTIVATE => Ev::Activate((w.0 & 0xFFFF) as u32 != WA_INACTIVE),
        WM_MOVE => Ev::Moved,
        WM_CLOSE => Ev::Close,
        WM_DESTROY => Ev::Destroyed,
        WM_CAPTURECHANGED => Ev::CaptureLost,
        WM_DEVICECHANGE => {
            // DEV_BROADCAST_HDR: its size, then the device type (2 = a volume)
            let vol = l.0 != 0 && unsafe { *((l.0 as *const u8).add(4) as *const u32) } == 2;
            Ev::DeviceChange(w.0, vol)
        }
        WM_ENTERSIZEMOVE => Ev::EnterSizeMove,
        WM_EXITSIZEMOVE => Ev::ExitSizeMove,
        WM_SIZING => Ev::Sizing(w.0 as u32),
        WM_TRAY => Ev::Tray,
        _ => return None,
    })
}

/// Messages that can wait until the handler is free (the rest get Windows' default when it is busy).
fn deferrable(ev: &Ev) -> bool {
    !matches!(ev, Ev::Paint | Ev::Close)
}

extern "system" fn wndproc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(l.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(h, GWLP_USERDATA, cs.lpCreateParams as isize);
        }
        let p = GetWindowLongPtrW(h, GWLP_USERDATA) as *const Slot;
        if p.is_null() {
            return DefWindowProcW(h, msg, w, l);
        }
        if msg == WM_NCDESTROY {
            SetWindowLongPtrW(h, GWLP_USERDATA, 0);
            let r = DefWindowProcW(h, msg, w, l);
            let slot = Box::from_raw(p as *mut Slot);
            // events that waited for a busy handler (its Destroyed among them) still reach it, once it is free
            if !slot.queue.borrow().is_empty() || slot.handler.try_borrow_mut().is_err() {
                finish_later(slot, Win(h));
            }
            return r;
        }
        let slot = &*p;
        let win = Win(h);
        match msg {
            WM_NCCALCSIZE if w.0 != 0 => {
                // a maximized window without a frame: Windows still puts its border past the screen's edge
                if IsZoomed(h).as_bool() {
                    let params = &mut *(l.0 as *mut NCCALCSIZE_PARAMS);
                    let (l_, t_, r_, b_) = monitor_work(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST));
                    params.rgrc[0] = RECT { left: l_, top: t_, right: r_, bottom: b_ };
                }
                return LRESULT(0);
            }
            WM_NCHITTEST => return LRESULT(hit_test(slot, h, l) as isize),
            WM_SETCURSOR if (l.0 & 0xFFFF) as u32 == HTCLIENT => {
                let (x, y) = win.cursor_pos();
                let cur = slot.hit.borrow().cursors.iter().rev().find(|c| c.0.contains(x, y)).map(|c| c.1).unwrap_or_default();
                let id = match cur {
                    Cursor::Hand => IDC_HAND,
                    Cursor::IBeam => IDC_IBEAM,
                    Cursor::Arrow => IDC_ARROW,
                };
                SetCursor(LoadCursorW(None, id).ok());
                return LRESULT(1);
            }
            WM_GETMINMAXINFO => {
                let mmi = &mut *(l.0 as *mut MINMAXINFO);
                let (mw, mh) = slot.min_track.get();
                if mw > 0 {
                    mmi.ptMinTrackSize.x = mw;
                }
                if mh > 0 {
                    mmi.ptMinTrackSize.y = mh;
                }
                // Native2.MaximizeToWorkArea: a maximized window fills the work area of its screen, not the screen
                let mon = MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST);
                let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
                if GetMonitorInfoW(mon, &mut mi).as_bool() {
                    mmi.ptMaxPosition.x = mi.rcWork.left - mi.rcMonitor.left;
                    mmi.ptMaxPosition.y = mi.rcWork.top - mi.rcMonitor.top;
                    mmi.ptMaxSize.x = mi.rcWork.right - mi.rcWork.left;
                    mmi.ptMaxSize.y = mi.rcWork.bottom - mi.rcWork.top;
                }
                return LRESULT(0);
            }
            WM_ERASEBKGND => return LRESULT(1),
            WM_MOUSEACTIVATE if GetWindowLongPtrW(h, GWL_EXSTYLE) as u32 & WS_EX_NOACTIVATE.0 != 0 => {
                return LRESULT(MA_NOACTIVATE as isize);
            }
            WM_MOUSEMOVE if !slot.tracking.get() => {
                let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                let _ = TrackMouseEvent(&mut t);
                slot.tracking.set(true);
            }
            WM_MOUSELEAVE_ => slot.tracking.set(false),
            WM_SIZE => {
                let st = w.0 as u32;
                if st == SIZE_MINIMIZED || st == SIZE_MAXIMIZED || st == SIZE_RESTORED {
                    deliver(slot, win, Ev::State(st));
                }
            }
            WM_DESTROY => slot.destroyed.set(true),
            _ => {}
        }
        let Some(ev) = decode(h, msg, w, l) else {
            return DefWindowProcW(h, msg, w, l);
        };
        if let Ev::Paint = ev {
            let mut ps = PAINTSTRUCT::default();
            BeginPaint(h, &mut ps);
            let _ = EndPaint(h, &ps);
        }
        let handled = deliver(slot, win, ev);
        match ev {
            Ev::Paint => LRESULT(0),
            Ev::Dpi(..) => LRESULT(0),
            Ev::Close if handled => LRESULT(0),
            _ if handled => LRESULT(0),
            _ => DefWindowProcW(h, msg, w, l),
        }
    }
}

/// How soon a handler that was still busy is looked at again. The handler stays busy while Windows runs a message
/// loop of its own inside it (ShellExecuteEx opening a folder, a COM call to Explorer); a retry posted at once came
/// straight back inside that loop, again and again, and never let it end: the window froze (fix list 6.14).
const BUSY_RETRY_MS: u32 = 15;

/// A destroyed window's last events, delivered when its handler is free.
fn finish_later(slot: Box<Slot>, win: Win) {
    post_local(Box::new(move || finish_now(slot, win)));
}

fn finish_now(slot: Box<Slot>, win: Win) {
    let busy = slot.handler.try_borrow_mut().is_err();
    if busy {
        return after(BUSY_RETRY_MS, Box::new(move || finish_now(slot, win)));
    }
    if let Ok(mut hd) = slot.handler.try_borrow_mut() {
        loop {
            let next = slot.queue.borrow_mut().pop_front();
            match next {
                Some(e) => {
                    hd.event(win, e);
                }
                None => break,
            }
        }
    }
}

/// The handler was busy (something outside its own events holds it): what waited is delivered soon after.
fn schedule_drain(slot: &Slot, win: Win) {
    if slot.drain_posted.replace(true) {
        return;
    }
    post_local(Box::new(move || drain(win)));
}

fn drain(win: Win) {
    if !win.exists() {
        return;
    }
    let Some(s) = win.slot() else { return };
    let Ok(mut hd) = s.handler.try_borrow_mut() else {
        // still busy: a moment later (drain_posted stays set, so this is the only retry waiting)
        return after(BUSY_RETRY_MS, Box::new(move || drain(win)));
    };
    s.drain_posted.set(false);
    loop {
        let next = s.queue.borrow_mut().pop_front();
        match next {
            Some(e) => {
                hd.event(win, e);
            }
            None => break,
        }
    }
    drop(hd);
    if s.repaint.replace(false) && !s.destroyed.get() {
        win.invalidate();
    }
}

fn deliver(slot: &Slot, win: Win, ev: Ev) -> bool {
    let Ok(mut hd) = slot.handler.try_borrow_mut() else {
        match ev {
            Ev::Paint => slot.repaint.set(true),
            e if deferrable(&e) => slot.queue.borrow_mut().push_back(e),
            _ => {}
        }
        schedule_drain(slot, win);
        return false;
    };
    let handled = guard(|| hd.event(win, ev)).unwrap_or(false);
    loop {
        let next = slot.queue.borrow_mut().pop_front();
        match next {
            Some(e) => {
                guard(|| hd.event(win, e));
            }
            None => break,
        }
    }
    drop(hd);
    if slot.repaint.replace(false) && !slot.destroyed.get() {
        win.invalidate();
    }
    handled
}

static ON_PANIC: std::sync::OnceLock<fn(String)> = std::sync::OnceLock::new();

/// Where the UI thread's errors are noted (App: DispatcherUnhandledException, errors.txt).
pub fn on_ui_error(f: fn(String)) {
    let _ = ON_PANIC.set(f);
}

/// Runs UI-thread work; an error in it is noted and the UI goes on (WPF's DispatcherUnhandledException with Handled:
/// nothing may unwind into Windows' window procedures).
fn guard<R>(f: impl FnOnce() -> R) -> Option<R> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => Some(r),
        Err(e) => {
            let text = e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "panic".into());
            if let Some(n) = ON_PANIC.get() {
                let _ = std::panic::catch_unwind(|| n(text));
            }
            None
        }
    }
}

fn hit_test(slot: &Slot, h: HWND, l: LPARAM) -> u32 {
    let mut p = POINT { x: (l.0 & 0xFFFF) as i16 as i32, y: ((l.0 >> 16) & 0xFFFF) as i16 as i32 };
    unsafe {
        let _ = ScreenToClient(h, &mut p);
    }
    let (x, y) = (p.x as f64, p.y as f64);
    let hit = slot.hit.borrow();
    let zoomed = unsafe { IsZoomed(h).as_bool() };
    if hit.resize > 0.0 && !zoomed {
        let (cw, ch) = Win(h).client_size();
        let (cw, ch) = (cw as f64, ch as f64);
        let b = hit.resize;
        let (l_, r_, t_, b_) = (x < b, x >= cw - b, y < b, y >= ch - b);
        let ht = match (l_, r_, t_, b_) {
            (true, _, true, _) => HTTOPLEFT,
            (_, true, true, _) => HTTOPRIGHT,
            (true, _, _, true) => HTBOTTOMLEFT,
            (_, true, _, true) => HTBOTTOMRIGHT,
            (true, ..) => HTLEFT,
            (_, true, ..) => HTRIGHT,
            (_, _, true, _) => HTTOP,
            (.., true) => HTBOTTOM,
            _ => HTCLIENT,
        };
        if ht != HTCLIENT {
            return ht;
        }
    }
    if y < hit.caption && !hit.no_drag.iter().any(|r| r.contains(x, y)) {
        return HTCAPTION;
    }
    HTCLIENT
}

/// A layered window has no WM_PAINT: its handler gets Ev::Paint once the UI thread is free (one for many asks).
pub fn request_paint(w: Win) {
    let Some(s) = w.slot() else { return };
    if s.paint_pending.replace(true) {
        return;
    }
    post_local(Box::new(move || {
        if !w.exists() {
            return;
        }
        if let Some(s) = w.slot() {
            s.paint_pending.set(false);
            if !s.destroyed.get() {
                deliver(s, w, Ev::Paint);
            }
        }
    }));
}

// ================= the UI thread: work queue, timers, message loop =================

type Job = Box<dyn FnOnce() + Send>;
type LocalJob = Box<dyn FnOnce()>;

static QUEUE: Mutex<VecDeque<Job>> = Mutex::new(VecDeque::new());
static DISPATCH_HWND: Mutex<isize> = Mutex::new(0);

thread_local! {
    static LOCAL: RefCell<VecDeque<LocalJob>> = const { RefCell::new(VecDeque::new()) };
    static TIMERS: RefCell<HashMap<usize, (bool, Rc<dyn Fn()>)>> = RefCell::new(HashMap::new());
    static NEXT_TIMER: Cell<usize> = const { Cell::new(1) };
    static TRAY_HANDLER: RefCell<Option<Rc<dyn Fn(u32)>>> = const { RefCell::new(None) };
    static DEVICE_HANDLER: RefCell<Option<Rc<dyn Fn(usize)>>> = const { RefCell::new(None) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
    static UI_THREAD: Cell<bool> = const { Cell::new(false) };
}

fn dispatch_hwnd() -> HWND {
    HWND(*DISPATCH_HWND.lock().unwrap_or_else(|e| e.into_inner()) as *mut c_void)
}

/// Makes this thread the UI thread: its hidden window takes posted work, app timers and the tray's messages.
pub fn init_ui_thread() {
    ensure_classes();
    unsafe {
        let h = CreateWindowExW(WINDOW_EX_STYLE(0), w!("ChoppyDispatch"), w!("Choppy"), WINDOW_STYLE(0), 0, 0, 0, 0, None, None, Some(inst()), None);
        if let Ok(h) = h {
            *DISPATCH_HWND.lock().unwrap_or_else(|e| e.into_inner()) = h.0 as isize;
        }
        TASKBAR_CREATED.with(|t| t.set(RegisterWindowMessageW(w!("TaskbarCreated"))));
    }
    UI_THREAD.with(|u| u.set(true));
}

pub fn on_ui_thread() -> bool {
    UI_THREAD.with(|u| u.get())
}

/// Runs `f` on the UI thread soon (Dispatcher.BeginInvoke), from any thread.
pub fn post(f: Job) {
    QUEUE.lock().unwrap_or_else(|e| e.into_inner()).push_back(f);
    unsafe {
        let _ = PostMessageW(Some(dispatch_hwnd()), WM_DISPATCH, WPARAM(0), LPARAM(0));
    }
}

/// Runs `f` on the UI thread soon, from the UI thread (it may hold UI-thread things like Rc).
pub fn post_local(f: LocalJob) {
    LOCAL.with(|q| q.borrow_mut().push_back(f));
    unsafe {
        let _ = PostMessageW(Some(dispatch_hwnd()), WM_DISPATCH, WPARAM(0), LPARAM(0));
    }
}

/// the timer that runs the idle work (far from the ids `timer` hands out)
const IDLE_TIMER: usize = 0x7FFF_0001;

thread_local! {
    static IDLE: RefCell<VecDeque<LocalJob>> = const { RefCell::new(VecDeque::new()) };
}

/// Runs `f` on the UI thread once it has nothing more urgent to do (WPF's Background / ApplicationIdle priorities): a
/// timer's message comes only after the posted work, input and painting. Several run in the order given.
pub fn idle(f: LocalJob) {
    IDLE.with(|q| q.borrow_mut().push_back(f));
    unsafe {
        SetTimer(Some(dispatch_hwnd()), IDLE_TIMER, 0, None);
    }
}

/// A timer on the UI thread: `f` every `ms` (once = after `ms`, once). Returns its id.
pub fn timer(ms: u32, once: bool, f: Rc<dyn Fn()>) -> usize {
    let id = NEXT_TIMER.with(|n| {
        let v = n.get();
        n.set(v + 1);
        v
    });
    TIMERS.with(|t| t.borrow_mut().insert(id, (once, f)));
    unsafe {
        SetTimer(Some(dispatch_hwnd()), id, ms, None);
    }
    id
}

pub fn kill(id: usize) {
    TIMERS.with(|t| t.borrow_mut().remove(&id));
    unsafe {
        let _ = KillTimer(Some(dispatch_hwnd()), id);
    }
}

/// Runs `f` once on the UI thread after `ms`. A timer's message comes only when nothing more urgent waits, so work
/// that waits for something this way never keeps the thread spinning.
pub fn after(ms: u32, f: LocalJob) {
    let f = RefCell::new(Some(f));
    timer(
        ms,
        true,
        Rc::new(move || {
            let job = f.borrow_mut().take();
            if let Some(job) = job {
                job();
            }
        }),
    );
}

/// Windows considers the UI thread hung: it hasn't looked at its messages for about 5 seconds although some wait
/// (IsHungAppWindow; a thread that is only idle, or busy inside a message loop of Windows' own, isn't). Any thread
/// may ask; it costs nothing.
pub fn ui_hung() -> bool {
    let h = dispatch_hwnd();
    !h.0.is_null() && unsafe { IsHungAppWindow(h).as_bool() }
}

static SESSION_END: std::sync::OnceLock<fn()> = std::sync::OnceLock::new();

/// Called when Windows signs out or shuts down (WM_ENDSESSION).
pub fn on_session_end(f: fn()) {
    let _ = SESSION_END.set(f);
}

extern "system" fn dispatch_proc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_DISPATCH => {
                // only the work that waits now: what it posts runs on its own message (each post has one), after
                // what Windows has waiting - so work that posts work can never keep this loop going, and a message
                // loop of Windows' own that we run inside (a shell call, a COM call to Explorer) gets its turn back
                let n = QUEUE.lock().unwrap_or_else(|e| e.into_inner()).len();
                for _ in 0..n {
                    let job = QUEUE.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
                    let Some(job) = job else { break };
                    guard(job);
                }
                let n = LOCAL.with(|q| q.borrow().len());
                for _ in 0..n {
                    let job = LOCAL.with(|q| q.borrow_mut().pop_front());
                    let Some(job) = job else { break };
                    guard(job);
                }
                LRESULT(0)
            }
            WM_TIMER if w.0 == IDLE_TIMER => {
                let _ = KillTimer(Some(h), IDLE_TIMER);
                // the same: idle work added meanwhile set the timer again
                let n = IDLE.with(|q| q.borrow().len());
                for _ in 0..n {
                    let job = IDLE.with(|q| q.borrow_mut().pop_front());
                    let Some(job) = job else { break };
                    guard(job);
                }
                LRESULT(0)
            }
            WM_ENDSESSION if w.0 != 0 => {
                // Windows signs out or shuts down: Choppy ends with it, on purpose (nothing to start again)
                if let Some(f) = SESSION_END.get() {
                    guard(f);
                }
                LRESULT(0)
            }
            WM_TIMER => {
                let e = TIMERS.with(|t| t.borrow().get(&w.0).cloned());
                if let Some((once, f)) = e {
                    if once {
                        kill(w.0);
                    }
                    guard(|| f());
                }
                LRESULT(0)
            }
            WM_TRAY => {
                let f = TRAY_HANDLER.with(|t| t.borrow().clone());
                if let Some(f) = f {
                    guard(|| f((l.0 & 0xFFFF) as u32));
                }
                LRESULT(0)
            }
            WM_DEVICECHANGE => {
                let f = DEVICE_HANDLER.with(|t| t.borrow().clone());
                if let Some(f) = f {
                    guard(|| f(w.0));
                }
                LRESULT(1)
            }
            m if m != 0 && m == TASKBAR_CREATED.with(|t| t.get()) => {
                // Explorer started again: the tray icon has to be added again
                TRAY.with(|t| {
                    if let Some(d) = t.borrow().as_ref() {
                        let _ = Shell_NotifyIconW(NIM_ADD, d);
                    }
                });
                LRESULT(0)
            }
            _ => DefWindowProcW(h, msg, w, l),
        }
    }
}

/// The message loop; returns when `quit` was called.
pub fn run_loop() -> i32 {
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        msg.wParam.0 as i32
    }
}

pub fn quit(code: i32) {
    unsafe { PostQuitMessage(code) };
}

// ================= tray icon =================

thread_local! {
    static TRAY: RefCell<Option<NOTIFYICONDATAW>> = const { RefCell::new(None) };
}

fn copy_w(dst: &mut [u16], s: &str) {
    let v: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..v.len()].copy_from_slice(&v);
    dst[v.len()] = 0;
}

/// Adds the tray icon (the small app icon) with its tooltip; `on` gets its mouse messages.
pub fn tray_add(tip: &str, on: Rc<dyn Fn(u32)>) {
    TRAY_HANDLER.with(|t| *t.borrow_mut() = Some(on));
    unsafe {
        let px = GetSystemMetrics(SM_CXSMICON);
        let icon = LoadImageW(Some(inst()), PCWSTR(ICON_ID as usize as *const u16), IMAGE_ICON, px, px, LR_DEFAULTCOLOR)
            .map(|h| HICON(h.0))
            .unwrap_or_default();
        let mut d = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: dispatch_hwnd(),
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: icon,
            ..Default::default()
        };
        copy_w(&mut d.szTip, tip);
        let _ = Shell_NotifyIconW(NIM_ADD, &d);
        TRAY.with(|t| *t.borrow_mut() = Some(d));
    }
}

pub fn tray_tip(tip: &str) {
    TRAY.with(|t| {
        if let Some(d) = t.borrow_mut().as_mut() {
            d.uFlags = NIF_TIP;
            copy_w(&mut d.szTip, tip);
            unsafe {
                let _ = Shell_NotifyIconW(NIM_MODIFY, d);
            }
            d.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        }
    });
}

/// A balloon over the tray icon (NotifyIcon.ShowBalloonTip, no icon).
pub fn tray_balloon(title: &str, text: &str, ms: u32) {
    TRAY.with(|t| {
        if let Some(d) = t.borrow_mut().as_mut() {
            let mut b = *d;
            b.uFlags = NIF_INFO;
            copy_w(&mut b.szInfoTitle, title);
            copy_w(&mut b.szInfo, text);
            b.Anonymous.uTimeout = ms;
            b.dwInfoFlags = NIIF_NONE;
            unsafe {
                let _ = Shell_NotifyIconW(NIM_MODIFY, &b);
            }
        }
    });
}

pub fn tray_remove() {
    TRAY.with(|t| {
        if let Some(d) = t.borrow_mut().take() {
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &d);
            }
        }
    });
    TRAY_HANDLER.with(|t| *t.borrow_mut() = None);
}

/// The tray needs the menu's window in front so a click elsewhere closes it.
pub fn foreground(w: Win) {
    unsafe {
        let _ = SetForegroundWindow(w.0);
    }
}

// ================= clipboard, shell =================

const CF_UNICODETEXT: u32 = 13;

pub fn clipboard_text() -> Option<String> {
    unsafe {
        OpenClipboard(None).ok()?;
        let mut out = None;
        if let Ok(h) = GetClipboardData(CF_UNICODETEXT) {
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g) as *const u16;
            if !p.is_null() {
                let mut n = 0;
                while *p.add(n) != 0 {
                    n += 1;
                }
                out = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)));
                let _ = GlobalUnlock(g);
            }
        }
        let _ = CloseClipboard();
        out
    }
}

pub fn set_clipboard_text(s: &str) {
    unsafe {
        if OpenClipboard(None).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        let w: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
        if let Ok(g) = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2) {
            let p = GlobalLock(g) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
                let _ = GlobalUnlock(g);
                let _ = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(g.0)));
            }
        }
        let _ = CloseClipboard();
    }
}

/// Process.Start(file, args) with the shell (explorer.exe /select,..., notepad.exe, a web page).
pub fn shell_open(file: &str, args: Option<&str>) -> bool {
    unsafe {
        let f = HSTRING::from(file);
        let a = args.map(HSTRING::from);
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_FLAG_NO_UI,
            lpFile: PCWSTR(f.as_ptr()),
            lpParameters: a.as_ref().map(|a| PCWSTR(a.as_ptr())).unwrap_or(PCWSTR::null()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        ShellExecuteExW(&mut info).is_ok()
    }
}

/// Explorer's own thumbnail of a file (photos, videos, RAW with codecs) as BGRA pixels: (pixels, width, height,
/// premultiplied). None when the file type has none (ShellThumbs.Get). Call it on an STA thread.
pub fn shell_thumbnail(path: &str, size: i32) -> Option<(Vec<u8>, u32, u32, bool)> {
    unsafe {
        let item: IShellItemImageFactory = SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
        let hbm = item.GetImage(SIZE { cx: size, cy: size }, SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK).ok()?;
        let out = (|| {
            let mut ds = DIBSECTION::default();
            if GetObjectW(hbm.into(), size_of::<DIBSECTION>() as i32, Some(&mut ds as *mut _ as *mut c_void)) == 0
                || ds.dsBm.bmBits.is_null()
                || ds.dsBm.bmBitsPixel != 32
            {
                return None;
            }
            let (w, h, stride) = (ds.dsBm.bmWidth as usize, ds.dsBm.bmHeight as usize, ds.dsBm.bmWidthBytes as usize);
            let src = std::slice::from_raw_parts(ds.dsBm.bmBits as *const u8, stride * h);
            let mut bytes = vec![0u8; w * h * 4];
            for y in 0..h {
                // bottom-up DIB: rows flipped
                let sy = if ds.dsBmih.biHeight > 0 { h - 1 - y } else { y };
                bytes[y * w * 4..(y + 1) * w * 4].copy_from_slice(&src[sy * stride..sy * stride + w * 4]);
            }
            // thumbnails without real alpha come back with alpha 0: opaque
            let any_alpha = bytes.chunks(4).any(|p| p[3] != 0);
            if !any_alpha {
                for p in bytes.chunks_mut(4) {
                    p[3] = 255;
                }
            }
            Some((bytes, w as u32, h as u32, any_alpha))
        })();
        let _ = DeleteObject(hbm.into());
        out
    }
}
