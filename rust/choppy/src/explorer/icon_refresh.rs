//! Explorer keeps every program's icon in its icon cache, by the exe's path. An update replaces Choppy.exe in the same
//! place, so the taskbar (a pinned Choppy), the Start menu and the right-click entry went on showing the old icon (seen
//! on Windows 11 after v1.37's new icon). Once per icon - the first start after it changes - Choppy tells Explorer the
//! picture behind its exe and its shortcuts changed, and has Windows rebuild the icon cache (IconRefresh).

use super::takeover;
use crate::engine::paths;
use crate::engine::xxhash64::XxHash64;
use crate::sys::{self, Folder};
use crate::wstr::WString;

const KEY: &str = r"Software\Choppy";
const VALUE: &str = "IconStamp";

/// The icon's own fingerprint (not the exe's: every update changes the exe, few change the icon): xxHash64 of the icon
/// file, as C# read it from the exe's resources ("choppy.ico"), 16 lower-case hex digits.
pub fn stamp() -> String {
    format!("{:016x}", XxHash64::of_bytes(crate::ui::ICO))
}

/// Refreshes the icons when Choppy's icon differs from the one last shown (cheap when it doesn't).
pub fn if_changed() {
    if takeover::registry_off() {
        return;
    }
    let stamp = stamp();
    let k = takeover::key(KEY);
    if takeover::reg_string(&k, VALUE).as_deref() == Some(stamp.as_str()) {
        return;
    }
    // the test switch only compares what is written: Explorer's own cache stays as it is
    if takeover::test_root().is_none() {
        refresh();
    }
    let _ = sys::reg_write_values(&k, &[(VALUE, stamp)], &[]);
}

fn refresh() {
    let exe = takeover::exe_path();
    // the cached picture of the exe's icon, then the exe and every shortcut to it (Start menu, taskbar pins)
    sys::update_cached_icon(&exe);
    sys::notify_item_updated(&exe);
    let appdata = sys::known_folder(Folder::AppData);
    for dir in [
        sys::known_folder(Folder::Programs),
        paths::combine(&appdata, &WString::from(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar")),
        sys::known_folder(Folder::Desktop),
    ] {
        for lnk in shortcuts(&dir) {
            sys::notify_item_updated(&lnk);
        }
    }
    sys::notify_assoc_changed();
    // Windows 10 / 11 keep the cache in a file as well: ie4uinit -show rebuilds it from the programs themselves
    let ie4 = paths::combine(&sys::known_folder(Folder::System), &WString::from("ie4uinit.exe"));
    if std::path::Path::new(&ie4.to_os()).is_file() {
        let _ = sys::spawn_hidden(std::path::Path::new(&ie4.to_os()), &["-show"]);
    }
}

/// Directory.GetFiles(dir, "Choppy*.lnk"): the files of that folder (not its subfolders) Windows' own pattern matches.
fn shortcuts(dir: &WString) -> Vec<WString> {
    if dir.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let pattern = paths::combine(dir, &WString::from("Choppy*.lnk"));
    if let Ok(f) = sys::find_first(&pattern) {
        for e in f {
            if e.attributes & sys::FILE_ATTRIBUTE_DIRECTORY == 0 {
                out.push(paths::combine(dir, &e.name));
            }
        }
    }
    out
}
