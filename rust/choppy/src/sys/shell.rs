//! Explorer's side (step 5): which folder an Explorer window shows (Takeover.cs ShellLocation, through the shell's own
//! interfaces and the old way by name through IDispatch), Explorer's windows, the shell folders, and telling Explorer
//! that Choppy's icon changed (IconRefresh.cs).

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{
    CLSCTX_SERVER, CLSIDFromProgID, CoCreateInstance, CoTaskMemFree, DISPATCH_FLAGS, DISPATCH_METHOD, DISPATCH_PROPERTYGET,
    DISPPARAMS, IDispatch, IServiceProvider,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_ProgramFiles, FOLDERID_Programs, FOLDERID_RoamingAppData, FOLDERID_System, IFolderView, IPersistFolder2,
    IShellBrowser, IShellWindows, KF_FLAG_DEFAULT, SHCNE_ASSOCCHANGED, SHCNE_UPDATEITEM, SHCNF_FLAGS, SHChangeNotify,
    SHGetKnownFolderPath, SHGetNameFromIDList, SHUpdateImageW, SIGDN_FILESYSPATH, ShellWindows, Shell_GetCachedImageIndexW,
};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowExW, GA_ROOT, GetAncestor};
use windows::core::{BSTR, GUID, HSTRING, Interface, PCWSTR};

use crate::wstr::WString;

const SID_STOP_LEVEL_BROWSER: GUID = GUID::from_u128(0x4c96be40_915c_11cf_99d3_00aa004ae837);
const LOCALE_USER_DEFAULT: u32 = 0x400;

/// FindWindowEx: the next window of that class after `after` (0 = the first) under `parent` (0 = top-level windows).
pub fn find_window(parent: isize, after: isize, class: &str) -> isize {
    let c = HSTRING::from(class);
    unsafe {
        FindWindowExW(
            (parent != 0).then_some(HWND(parent as *mut _)),
            (after != 0).then_some(HWND(after as *mut _)),
            &c,
            PCWSTR::null(),
        )
        .map(|h| h.0 as isize)
        .unwrap_or(0)
    }
}

/// GetClassName as a string (64 characters at most, as the C# StringBuilder(64)).
pub fn class_of(h: isize) -> String {
    let mut buf = [0u16; 64];
    let n = super::class_name(h, &mut buf);
    String::from_utf16_lossy(&buf[..n])
}

/// The window's active tab asked through the shell's own interfaces (ShellLocation.Ask): (the folder's file system
/// path, answered). answered = the window was found and said what it shows (no path then = not a folder on a disk).
pub fn ask_folder(hwnd: isize, active_tab: isize) -> (Option<WString>, bool) {
    unsafe {
        let Ok(windows) = CoCreateInstance::<_, IShellWindows>(&ShellWindows, None, CLSCTX_SERVER) else {
            return (None, false);
        };
        let Ok(count) = windows.Count() else { return (None, false) };
        for i in 0..count {
            // C#: any failure on the way to the browser's window = the next one (its catch inside the loop)
            let Ok(w) = windows.Item(&VARIANT::from(i)) else { continue };
            let Ok(sp) = w.cast::<IServiceProvider>() else { continue };
            let Ok(browser) = sp.QueryService::<IShellBrowser>(&SID_STOP_LEVEL_BROWSER) else { continue };
            let Ok(h) = browser.GetWindow() else { continue };
            let h = h.0 as isize;
            // the browser's window is the tab (Windows 11 has several per window; the active one comes first)
            if h != hwnd && GetAncestor(HWND(h as *mut _), GA_ROOT).0 as isize != hwnd {
                continue;
            }
            if active_tab != 0 && h != hwnd && h != active_tab {
                continue;
            }
            let Ok(view) = browser.QueryActiveShellView() else { return (None, false) };
            let Ok(fv) = view.cast::<IFolderView>() else { continue }; // C#'s cast threw: caught, the next one
            let Ok(folder) = fv.GetFolder::<IPersistFolder2>() else { return (None, false) };
            let pidl = match folder.GetCurFolder() {
                Ok(p) if !p.is_null() => p,
                _ => return (None, false),
            };
            let name = SHGetNameFromIDList(pidl, SIGDN_FILESYSPATH); // This PC, a library...: no path
            CoTaskMemFree(Some(pidl as *const _));
            return match name {
                Ok(p) if !p.is_null() => {
                    let s = WString::from_units(p.as_wide().to_vec());
                    CoTaskMemFree(Some(p.0 as *const _));
                    (Some(s), true)
                }
                _ => (None, true),
            };
        }
        (None, false)
    }
}

/// The same through Shell.Application, by name (ShellLocation.AskByName: the way Choppy asked before 1.36): the first
/// window with this handle, or the one whose tab is the active one.
pub fn ask_folder_by_name(hwnd: isize, active_tab: isize) -> (Option<WString>, bool) {
    unsafe {
        let Ok(clsid) = CLSIDFromProgID(&HSTRING::from("Shell.Application")) else { return (None, false) };
        let Ok(shell) = CoCreateInstance::<_, IDispatch>(&clsid, None, CLSCTX_SERVER) else { return (None, false) };
        let Some(list) = call(&shell, "Windows", DISPATCH_METHOD | DISPATCH_PROPERTYGET, vec![]).and_then(|v| IDispatch::try_from(&v).ok())
        else {
            return (None, false);
        };
        let Some(count) = call(&list, "Count", DISPATCH_PROPERTYGET, vec![]).and_then(|v| i32::try_from(&v).ok()) else {
            return (None, false);
        };
        let mut first: Option<WString> = None;
        for i in 0..count {
            let Some(w) = call(&list, "Item", DISPATCH_METHOD | DISPATCH_PROPERTYGET, vec![VARIANT::from(i)])
                .and_then(|v| IDispatch::try_from(&v).ok())
            else {
                continue;
            };
            let Some(h) = call(&w, "HWND", DISPATCH_PROPERTYGET, vec![]).and_then(|v| i64::try_from(&v).ok()) else { continue };
            if h != hwnd as i64 {
                continue;
            }
            let p = path_of(&w);
            if active_tab != 0 && tab_of(&w) == active_tab {
                let answered = p.is_some();
                return (p, answered);
            }
            if first.is_none() {
                first = p;
            }
        }
        let answered = first.is_some();
        (first, answered)
    }
}

/// w.Document.Folder.Self.Path
fn path_of(w: &IDispatch) -> Option<WString> {
    let get = |d: &IDispatch, name: &str| call(d, name, DISPATCH_PROPERTYGET, vec![]).and_then(|v| IDispatch::try_from(&v).ok());
    let doc = get(w, "Document")?;
    let folder = get(&doc, "Folder")?;
    let me = get(&folder, "Self")?;
    let v = call(&me, "Path", DISPATCH_PROPERTYGET, vec![])?;
    let b = BSTR::try_from(&v).ok()?;
    Some(WString::from_units(b.to_vec()))
}

/// The window of the browser behind a ShellWindows item (its tab).
fn tab_of(w: &IDispatch) -> isize {
    unsafe {
        let Ok(sp) = w.cast::<IServiceProvider>() else { return 0 };
        let Ok(b) = sp.QueryService::<IShellBrowser>(&SID_STOP_LEVEL_BROWSER) else { return 0 };
        b.GetWindow().map(|h| h.0 as isize).unwrap_or(0)
    }
}

/// A late-bound call (C#'s `dynamic`): GetIDsOfNames, then Invoke. None = it failed (C# threw).
fn call(d: &IDispatch, name: &str, flags: DISPATCH_FLAGS, mut args: Vec<VARIANT>) -> Option<VARIANT> {
    unsafe {
        let n = HSTRING::from(name);
        let names = [PCWSTR(n.as_ptr())];
        let mut id = 0i32;
        d.GetIDsOfNames(&GUID::zeroed(), names.as_ptr(), 1, LOCALE_USER_DEFAULT, &mut id).ok()?;
        args.reverse(); // DISPPARAMS takes the arguments last first
        let params = DISPPARAMS {
            rgvarg: if args.is_empty() { std::ptr::null_mut() } else { args.as_mut_ptr() },
            rgdispidNamedArgs: std::ptr::null_mut(),
            cArgs: args.len() as u32,
            cNamedArgs: 0,
        };
        let mut result = VARIANT::default();
        d.Invoke(id, &GUID::zeroed(), LOCALE_USER_DEFAULT, flags, &params, Some(&mut result), None, None).ok()?;
        Some(result)
    }
}

/// GetAncestor(h, GA_ROOT)
pub fn root_of(h: isize) -> isize {
    unsafe { GetAncestor(HWND(h as *mut _), GA_ROOT).0 as isize }
}

/// The shell folders C# took from Environment.GetFolderPath: the desktop, the Start menu's Programs, the roaming
/// AppData, System32, Program Files (64-bit, as the AnyCPU C#). "" when Windows doesn't say (as .NET).
pub enum Folder {
    Desktop,
    Programs,
    AppData,
    System,
    ProgramFiles,
}

pub fn known_folder(f: Folder) -> WString {
    let id = match f {
        Folder::Desktop => &FOLDERID_Desktop,
        Folder::Programs => &FOLDERID_Programs,
        Folder::AppData => &FOLDERID_RoamingAppData,
        Folder::System => &FOLDERID_System,
        Folder::ProgramFiles => &FOLDERID_ProgramFiles,
    };
    unsafe {
        match SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) {
            Ok(p) => {
                let s = WString::from_units(p.as_wide().to_vec());
                CoTaskMemFree(Some(p.0 as *const _));
                s
            }
            Err(_) => WString::new(),
        }
    }
}

const SHCNF_PATHW: u32 = 0x0005;
const SHCNF_IDLIST: u32 = 0x0000;
const SHCNF_FLUSHNOWAIT: u32 = 0x3000;

/// The cached picture of the exe's icon is renewed (Shell_GetCachedImageIndexW + SHUpdateImageW).
pub fn update_cached_icon(exe: &WString) {
    let p = exe.to_wide_z();
    unsafe {
        let index = Shell_GetCachedImageIndexW(PCWSTR(p.as_ptr()), 0, 0);
        if index >= 0 {
            SHUpdateImageW(PCWSTR(p.as_ptr()), 0, 0, index);
        }
    }
}

/// SHChangeNotify(SHCNE_UPDATEITEM, SHCNF_PATHW | SHCNF_FLUSHNOWAIT, path)
pub fn notify_item_updated(path: &WString) {
    let p = path.to_wide_z();
    unsafe { SHChangeNotify(SHCNE_UPDATEITEM, SHCNF_FLAGS(SHCNF_PATHW | SHCNF_FLUSHNOWAIT), Some(p.as_ptr() as *const _), None) }
}

/// SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST | SHCNF_FLUSHNOWAIT): Explorer looks at every icon again.
pub fn notify_assoc_changed() {
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_FLAGS(SHCNF_IDLIST | SHCNF_FLUSHNOWAIT), None, None) }
}

/// Tests: how many windows Explorer lists, asked both ways (the shell's interfaces; Shell.Application by name).
pub fn shell_window_counts() -> (Option<i32>, Option<i32>) {
    unsafe {
        let a = CoCreateInstance::<_, IShellWindows>(&ShellWindows, None, CLSCTX_SERVER).ok().and_then(|w| w.Count().ok());
        let b = CLSIDFromProgID(&HSTRING::from("Shell.Application"))
            .ok()
            .and_then(|c| CoCreateInstance::<_, IDispatch>(&c, None, CLSCTX_SERVER).ok())
            .and_then(|s| call(&s, "Windows", DISPATCH_METHOD | DISPATCH_PROPERTYGET, vec![]))
            .and_then(|v| IDispatch::try_from(&v).ok())
            .and_then(|l| call(&l, "Count", DISPATCH_PROPERTYGET, vec![]))
            .and_then(|v| i32::try_from(&v).ok());
        (a, b)
    }
}
