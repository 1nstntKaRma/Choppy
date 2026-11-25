//! Ctrl+V in Explorer -> Choppy (PasteHook). A low-level keyboard hook that takes the key ONLY when: an Explorer
//! window or the desktop is in front, its file list has focus (not a rename box, the address bar or search), and the
//! clipboard holds files. Everything else passes straight through.
//!
//! It must never miss: Windows gives a hook a moment to answer (LowLevelHooksTimeout, at most 1 s) and when it answers
//! late, passes the key on (Explorer's own paste) and may drop the hook for good without a word. So the hook lives on
//! its own thread that does nothing else (never behind the window's work), answers without waiting on anything, and
//! is put in again every few seconds (a dropped hook is back at once).

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU32, AtomicU64, Ordering::SeqCst};
use std::sync::mpsc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::paste_log;
use crate::sys;

const WM_KEYDOWN: u32 = 0x100;
const WM_KEYUP: u32 = 0x101;
const WM_SYSKEYDOWN: u32 = 0x104;
const VK_V: u32 = 0x56;
const VK_INSERT: u32 = 0x2D;
const VK_CONTROL: u32 = 0x11;
const VK_SHIFT: u32 = 0x10;
const VK_MENU: u32 = 0x12;
const VK_LWIN: u32 = 0x5B;
const VK_RWIN: u32 = 0x5C;
/// marks the paste Choppy hands back to Explorer (dwExtraInfo)
const OWN_KEYS: usize = 0x43685079;
const REFRESH_MS: u32 = 3000;

/// the hook thread: Some while it runs; it says when it ended
static THREAD: Mutex<Option<mpsc::Receiver<()>>> = Mutex::new(None);
static THREAD_ID: AtomicU32 = AtomicU32::new(0);
static HOOK: AtomicIsize = AtomicIsize::new(0);
/// the key whose release is taken too (the press was)
static SWALLOW_UP: AtomicU32 = AtomicU32::new(0);
/// when the last paste taken was pressed (µs since EPOCH)
static PRESSED_AT: AtomicU64 = AtomicU64::new(0);
static OBSERVE_ONLY: AtomicBool = AtomicBool::new(false);
/// what a paste taken runs on the UI thread (the window under the key); None = nothing (the self-test)
static ON_PASTE: Mutex<Option<fn(isize)>> = Mutex::new(None);
static EPOCH: OnceLock<Instant> = OnceLock::new();

/// Times the hook was put in again; and how many of those found the old one already dropped by Windows.
pub static REFRESHES: AtomicI32 = AtomicI32::new(0);
pub static DROPPED: AtomicI32 = AtomicI32::new(0);

fn micros() -> u64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_micros() as u64
}

pub fn running() -> bool {
    THREAD.lock().unwrap_or_else(|e| e.into_inner()).is_some() && HOOK.load(SeqCst) != 0
}

/// observe = the self-test: the hook runs but never takes a key. True = the hook is in.
pub fn start(on_paste: Option<fn(isize)>, observe: bool) -> bool {
    *ON_PASTE.lock().unwrap_or_else(|e| e.into_inner()) = on_paste;
    let mut t = THREAD.lock().unwrap_or_else(|e| e.into_inner());
    if t.is_some() {
        return HOOK.load(SeqCst) != 0;
    }
    OBSERVE_ONLY.store(observe, SeqCst);
    EPOCH.get_or_init(Instant::now);
    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let (done_tx, done_rx) = mpsc::channel::<()>();
    let spawned = std::thread::Builder::new().name("Ctrl+V hook".into()).spawn(move || {
        sys::set_thread_priority_highest();
        let _ = std::panic::catch_unwind(move || run(ready_tx));
        let _ = done_tx.send(());
    });
    if spawned.is_err() {
        return false;
    }
    *t = Some(done_rx);
    drop(t);
    let _ = ready_rx.recv_timeout(Duration::from_millis(5000));
    HOOK.load(SeqCst) != 0
}

pub fn stop() {
    let Some(done) = THREAD.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    sys::post_quit_to(THREAD_ID.load(SeqCst));
    let _ = done.recv_timeout(Duration::from_millis(3000));
}

fn run(ready: mpsc::Sender<()>) {
    sys::com_init_sta(); // the C# thread was an STA
    THREAD_ID.store(sys::current_thread_id(), SeqCst);
    match sys::keyboard_hook_set(on_key) {
        Ok(h) => HOOK.store(h, SeqCst),
        Err(e) => paste_log::write(&format!("The Ctrl+V hook couldn't start (error {e}); it tries again in a moment.")),
    }
    let timer = sys::thread_timer(REFRESH_MS);
    let _ = ready.send(());
    sys::thread_message_loop(refresh);
    sys::thread_timer_kill(timer);
    let h = HOOK.swap(0, SeqCst);
    if h != 0 {
        sys::keyboard_hook_remove(h);
    }
}

/// A fresh hook in front, then the old one out (on this thread, so no key is seen twice or missed).
fn refresh() {
    let old = HOOK.load(SeqCst);
    let Ok(h) = sys::keyboard_hook_set(on_key) else { return }; // keep the old one
    HOOK.store(h, SeqCst);
    REFRESHES.fetch_add(1, SeqCst);
    if old != 0 && !sys::keyboard_hook_remove(old) {
        DROPPED.fetch_add(1, SeqCst);
        paste_log::write("Windows had dropped the Ctrl+V hook (it answered late once); it is back.");
    } else if old == 0 {
        paste_log::write("The Ctrl+V hook is running.");
    }
}

/// The hook's answer for every key (on the hook thread). Keys typed by other programs count too (a clipboard
/// manager pastes by sending Shift+Insert or Ctrl+V); only Choppy's own hand-back doesn't.
fn on_key(vk: u32, msg: u32, extra: usize) -> bool {
    if (vk != VK_V && vk != VK_INSERT) || extra == OWN_KEYS {
        return false;
    }
    if msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN {
        match check(vk) {
            (Why::Take, fg) if !OBSERVE_ONLY.load(SeqCst) => {
                PRESSED_AT.store(micros(), SeqCst);
                SWALLOW_UP.store(vk, SeqCst);
                let f = *ON_PASTE.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(f) = f {
                    crate::sys::win::post(Box::new(move || f(fg)));
                }
                return true;
            }
            // files on the clipboard and Explorer in front, but left to Windows: say why
            (Why::Say(why), _) => {
                paste_log::write(&format!("{} left to Windows: {why}", if vk == VK_V { "Ctrl+V" } else { "Shift+Insert" }))
            }
            _ => {}
        }
    } else if msg == WM_KEYUP && SWALLOW_UP.load(SeqCst) == vk {
        SWALLOW_UP.store(0, SeqCst);
        return true;
    }
    false
}

/// Milliseconds since the last paste Choppy took was pressed (the paste log notes how quick the window was).
pub fn ms_since_press() -> u64 {
    micros().saturating_sub(PRESSED_AT.load(SeqCst)) / 1000
}

fn down(vk: u32) -> bool {
    sys::async_key_down(vk)
}

enum Why {
    /// plain Ctrl+V or Shift+Insert in an Explorer file list or on the desktop, files on the clipboard
    Take,
    /// not ours, nothing to say (another program, other keys)
    Quiet,
    /// why Explorer keeps it
    Say(String),
}

/// The class name in `buf` is `name` (compared without allocating: this runs inside the hook's answer).
fn is(buf: &[u16; 64], n: usize, name: &str) -> bool {
    let mut it = buf[..n].iter();
    name.encode_utf16().all(|c| it.next() == Some(&c)) && it.next().is_none()
}

fn check(vk: u32) -> (Why, isize) {
    if down(VK_MENU) || down(VK_LWIN) || down(VK_RWIN) {
        return (Why::Quiet, 0);
    }
    let wrong = if vk == VK_V { !down(VK_CONTROL) || down(VK_SHIFT) } else { !down(VK_SHIFT) || down(VK_CONTROL) };
    if wrong {
        return (Why::Quiet, 0);
    }
    let fg = sys::foreground_window();
    if fg == 0 {
        return (Why::Quiet, 0);
    }
    let mut c = [0u16; 64];
    let n = sys::class_name(fg, &mut c);
    if !is(&c, n, "CabinetWClass") && !is(&c, n, "ExploreWClass") && !is(&c, n, "Progman") && !is(&c, n, "WorkerW") {
        return (Why::Quiet, fg);
    }
    if !sys::clipboard_has_files() {
        return (Why::Quiet, fg); // text or nothing: Explorer's business
    }
    let Some(focus) = sys::focus_in_thread_of(fg) else {
        return (Why::Say("no focus in the Explorer window".into()), fg);
    };
    // the file view itself: DirectUIHWND / SysListView32 inside SHELLDLL_DefView
    let mut f = [0u16; 64];
    let fname = sys::class_name(focus, &mut f);
    if is(&f, fname, "SHELLDLL_DefView") {
        return (Why::Take, fg);
    }
    if is(&f, fname, "DirectUIHWND") || is(&f, fname, "SysListView32") {
        let mut p = [0u16; 64];
        let pn = sys::class_name(sys::parent_window(focus), &mut p);
        if is(&p, pn, "SHELLDLL_DefView") {
            return (Why::Take, fg);
        }
    }
    let why = if is(&f, fname, "SysTreeView32") {
        "the folder tree on the left has focus (the paste goes to the folder picked there)".to_string()
    } else if is(&f, fname, "Edit") {
        "a text box has focus (address bar, search or a rename)".to_string()
    } else {
        format!("focus is on {}", String::from_utf16_lossy(&f[..fname]))
    };
    (Why::Say(why), fg)
}

/// Fallback: give Explorer its own paste back (virtual folders like This PC, Libraries).
pub fn replay_paste() {
    let ctrl_held = down(VK_CONTROL);
    sys::send_paste(ctrl_held, OWN_KEYS);
}

#[cfg(test)]
mod tests {
    #[test]
    fn class_names_compare_exactly() {
        let mut b = [0u16; 64];
        for (i, c) in "Edit".encode_utf16().enumerate() {
            b[i] = c;
        }
        assert!(super::is(&b, 4, "Edit"));
        assert!(!super::is(&b, 4, "Edi"));
        assert!(!super::is(&b, 4, "EditX"));
        assert!(!super::is(&b, 3, "Edit"));
        assert!(!super::is(&b, 4, "edit"));
    }
}
