//! Which folder an Explorer window (or the desktop) is showing - the active tab on Windows 11 (ShellLocation).

use crate::engine::paths;
use crate::sys::{self, Folder};
use crate::wstr::WString;

/// A real folder path, or None. is_virtual = the window shows a place that isn't a folder (This PC, a library, a zip:
/// Explorer keeps those pastes); None without it = Explorer didn't answer (ask again). The desktop's folder comes back
/// as Windows names it ("" when it doesn't).
pub fn folder_of(hwnd: isize) -> (Option<WString>, bool) {
    let cls = sys::class_of(hwnd);
    if cls == "Progman" || cls == "WorkerW" {
        return (Some(sys::known_folder(Folder::Desktop)), false);
    }
    let active_tab = sys::find_window(hwnd, 0, "ShellTabWindowClass");
    let (mut p, mut answered) = sys::ask_folder(hwnd, active_tab);
    if !answered {
        // the old way, in case this Windows answers only that one
        (p, answered) = sys::ask_folder_by_name(hwnd, active_tab);
    }
    if !answered {
        return (None, false);
    }
    if is_folder(p.as_ref()) { (p, false) } else { (None, true) }
}

/// A folder Choppy can paste into ("::{...}" is a virtual place, a zip or an FTP place isn't a folder on a disk).
fn is_folder(p: Option<&WString>) -> bool {
    let Some(p) = p else { return false };
    if p.is_empty() || p.starts_with("::") {
        return false;
    }
    paths::dir_exists(&paths::to_long(p))
}
