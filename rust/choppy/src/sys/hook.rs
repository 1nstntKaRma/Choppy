//! The Ctrl+V hook's Windows side (PasteHook.cs, step 5): the low-level keyboard hook and its thread's message loop,
//! the questions it asks about the window in front (none of them waits on anything), Explorer's own paste handed back
//! with SendInput, and the files on the clipboard (ClipboardFiles).

use std::sync::OnceLock;

use windows::Win32::Foundation::{GetLastError, HGLOBAL, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Threading::{GetCurrentThread, GetCurrentThreadId, SetThreadPriority, THREAD_PRIORITY_HIGHEST};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    VIRTUAL_KEY,
};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GUITHREADINFO, GetClassNameW, GetForegroundWindow, GetGUIThreadInfo, GetMessageW, GetParent,
    GetWindowThreadProcessId, HHOOK, KBDLLHOOKSTRUCT, KillTimer, MSG, PostThreadMessageW, SetTimer, SetWindowsHookExW,
    UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_QUIT, WM_TIMER,
};
use windows::core::{BOOL, w};

use crate::wstr::WString;

/// What the hook does with a key (vk, the message, dwExtraInfo): true = Choppy takes it (Explorer never sees it).
pub type KeyFn = fn(u32, u32, usize) -> bool;
static KEY_FN: OnceLock<KeyFn> = OnceLock::new();

extern "system" fn ll_proc(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0
        && let Some(f) = KEY_FN.get()
    {
        // KBDLLHOOKSTRUCT: vkCode, flags, dwExtraInfo. Nothing may unwind into Windows (C# caught everything here).
        let k = unsafe { &*(l.0 as *const KBDLLHOOKSTRUCT) };
        let (vk, extra) = (k.vkCode, k.dwExtraInfo);
        if std::panic::catch_unwind(|| f(vk, w.0 as u32, extra)).unwrap_or(false) {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, w, l) }
}

/// SetWindowsHookEx(WH_KEYBOARD_LL) for this thread's message loop; every key goes to `f`. Err = the Win32 error.
pub fn keyboard_hook_set(f: KeyFn) -> Result<isize, u32> {
    let _ = KEY_FN.set(f);
    unsafe {
        let module = GetModuleHandleW(None).map(|m| HINSTANCE(m.0)).ok();
        match SetWindowsHookExW(WH_KEYBOARD_LL, Some(ll_proc), module, 0) {
            Ok(h) => Ok(h.0 as isize),
            Err(_) => Err(GetLastError().0),
        }
    }
}

/// UnhookWindowsHookEx: false = Windows had already taken it out (it answered too late once).
pub fn keyboard_hook_remove(h: isize) -> bool {
    unsafe { UnhookWindowsHookEx(HHOOK(h as *mut _)).is_ok() }
}

pub fn current_thread_id() -> u32 {
    unsafe { GetCurrentThreadId() }
}

/// ThreadPriority.Highest for the calling thread.
pub fn set_thread_priority_highest() {
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
    }
}

/// A timer of the calling thread (no window): WM_TIMER every `ms` in its message loop.
pub fn thread_timer(ms: u32) -> usize {
    unsafe { SetTimer(None, 0, ms, None) }
}

pub fn thread_timer_kill(id: usize) {
    unsafe {
        let _ = KillTimer(None, id);
    }
}

/// GetMessage until WM_QUIT (or an error): `on_timer` for every WM_TIMER. (A hook's calls arrive inside GetMessage.)
pub fn thread_message_loop(mut on_timer: impl FnMut()) {
    let mut m = MSG::default();
    loop {
        let r: BOOL = unsafe { GetMessageW(&mut m, None, 0, 0) };
        if r.0 <= 0 {
            break;
        }
        if m.message == WM_TIMER {
            on_timer();
        }
    }
}

/// Ends that thread's message loop.
pub fn post_quit_to(thread_id: u32) -> bool {
    unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)).is_ok() }
}

/// The key is down right now (GetAsyncKeyState's high bit).
pub fn async_key_down(vk: u32) -> bool {
    (unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000) != 0
}

pub fn foreground_window() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}

/// GetClassName into the caller's buffer (no allocation: the hook asks this for every Ctrl+V). Its length.
pub fn class_name(h: isize, buf: &mut [u16; 64]) -> usize {
    let n = unsafe { GetClassNameW(HWND(h as *mut _), buf) };
    n.max(0) as usize
}

/// The window that has the keyboard focus in `h`'s thread (GetGUIThreadInfo's hwndFocus); None = it can't be asked or
/// nothing has the focus.
pub fn focus_in_thread_of(h: isize) -> Option<isize> {
    unsafe {
        let tid = GetWindowThreadProcessId(HWND(h as *mut _), None);
        let mut gti = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        GetGUIThreadInfo(tid, &mut gti).ok()?;
        (!gti.hwndFocus.0.is_null()).then_some(gti.hwndFocus.0 as isize)
    }
}

pub fn parent_window(h: isize) -> isize {
    unsafe { GetParent(HWND(h as *mut _)).map(|p| p.0 as isize).unwrap_or(0) }
}

pub const CF_HDROP: u32 = 15;

/// IsClipboardFormatAvailable(CF_HDROP): files are on the clipboard (asked without opening it).
pub fn clipboard_has_files() -> bool {
    unsafe { IsClipboardFormatAvailable(CF_HDROP).is_ok() }
}

/// Presses Ctrl+V for Explorer (Ctrl only when the user isn't holding it), marked with `extra` so the hook lets it
/// through. The number of key events Windows took.
pub fn send_paste(ctrl_held: bool, extra: usize) -> u32 {
    const VK_CONTROL: u16 = 0x11;
    const VK_V: u16 = 0x56;
    let key = |vk: u16, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                time: 0,
                dwExtraInfo: extra,
            },
        },
    };
    let mut list = Vec::with_capacity(4);
    if !ctrl_held {
        list.push(key(VK_CONTROL, false));
    }
    list.push(key(VK_V, false));
    list.push(key(VK_V, true));
    if !ctrl_held {
        list.push(key(VK_CONTROL, true));
    }
    unsafe { SendInput(&list, size_of::<INPUT>() as i32) }
}

/// What the clipboard held when it was opened.
pub enum Clip {
    /// the files (as DragQueryFile gives them) and "Preferred DropEffect", when there is one
    Files(Vec<WString>, Option<u32>),
    NoFiles,
}

/// Opens the clipboard, reads its CF_HDROP list and "Preferred DropEffect", closes it. Err = another program has the
/// clipboard open (WPF's CLIPBRD_E_CANT_OPEN).
pub fn clipboard_files() -> Result<Clip, ()> {
    unsafe {
        if OpenClipboard(None).is_err() {
            return Err(());
        }
        let r = read_open_clipboard();
        let _ = CloseClipboard();
        Ok(r)
    }
}

unsafe fn read_open_clipboard() -> Clip {
    unsafe {
        if IsClipboardFormatAvailable(CF_HDROP).is_err() {
            return Clip::NoFiles;
        }
        let Ok(h) = GetClipboardData(CF_HDROP) else { return Clip::NoFiles };
        let drop = HDROP(h.0);
        let n = DragQueryFileW(drop, u32::MAX, None);
        let mut files = Vec::with_capacity(n as usize);
        for i in 0..n {
            let len = DragQueryFileW(drop, i, None);
            let mut buf = vec![0u16; len as usize + 1];
            let got = DragQueryFileW(drop, i, Some(&mut buf));
            files.push(WString::from_units(buf[..got as usize].to_vec()));
        }
        let fmt = RegisterClipboardFormatW(w!("Preferred DropEffect"));
        let mut effect = None;
        if fmt != 0
            && let Ok(e) = GetClipboardData(fmt)
        {
            let g = HGLOBAL(e.0);
            if GlobalSize(g) >= 4 {
                let p = GlobalLock(g) as *const u8;
                if !p.is_null() {
                    effect = Some(u32::from_le_bytes([*p, *p.add(1), *p.add(2), *p.add(3)]));
                    let _ = GlobalUnlock(g);
                }
            }
        }
        Clip::Files(files, effect)
    }
}

/// Clipboard.Clear: empties the clipboard (WPF tries 10 times, 100 ms apart, while another program holds it).
pub fn clipboard_clear() -> bool {
    for i in 0..10 {
        unsafe {
            if OpenClipboard(None).is_ok() {
                let ok = EmptyClipboard().is_ok();
                let _ = CloseClipboard();
                return ok;
            }
        }
        if i < 9 {
            super::sleep_ms(100);
        }
    }
    false
}
