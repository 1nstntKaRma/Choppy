//! Windows integration, per user (HKCU) - no admin rights, nothing machine-wide (Takeover).
//!  - Right-click "Paste with Choppy": always there while Choppy is installed (the installed copy registers it;
//!    uninstall removes it).
//!  - "Default file manager" (set): ON = Ctrl+V of files in Explorer goes to Choppy (tray copy with the keyboard hook,
//!    started with Windows). Only one copier may take Ctrl+V: Ultracopier is closed and its startup entry is parked in
//!    Choppy's settings. OFF = Windows is left alone (Ultracopier's entry is put back); Choppy then works only through
//!    the right-click menu.

use std::sync::OnceLock;

use crate::engine::settings::{Settings, app_paths};
use crate::sys::{self, Folder};
use crate::wstr::WString;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VERB_NAME: &str = "ChoppyPaste";
const VERB_PARENTS: [&str; 3] =
    [r"Software\Classes\Directory\Background\shell", r"Software\Classes\Directory\shell", r"Software\Classes\Drive\shell"];

/// Test switch (on the rust branch, in both programs, for comparing what they write): with test mode (CHOPPY_DATA),
/// CHOPPY_REGTEST=<scratch key> runs the registry steps as an installed Choppy does, but every key lands under
/// HKCU\<scratch key>\... (never the real Run key, right-click entries or settings), "installed" means the exe sits in
/// <data folder>\Programs\Choppy, and Explorer's icon cache is left alone.
pub fn test_root() -> Option<String> {
    if !app_paths::test_mode() {
        return None;
    }
    std::env::var("CHOPPY_REGTEST").ok().filter(|r| !r.is_empty())
}

/// An HKCU key path, under the test root when there is one.
pub fn key(path: &str) -> String {
    match test_root() {
        Some(r) => format!(r"{r}\{path}"),
        None => path.to_string(),
    }
}

/// Tests never touch the real Explorer, startup or Ultracopier (unless CHOPPY_REGTEST points them at a scratch key).
pub fn registry_off() -> bool {
    app_paths::test_mode() && test_root().is_none()
}

static EXE: OnceLock<WString> = OnceLock::new();

pub fn exe_path() -> WString {
    EXE.get_or_init(sys::current_exe).clone()
}

/// RegistryKey.GetValue(name) as string: REG_SZ (one closing NUL dropped) or REG_EXPAND_SZ (expanded); None for any
/// other type or no value.
pub fn reg_string(subkey: &str, name: &str) -> Option<String> {
    let values = sys::reg_read_values(subkey).ok()??;
    let (_, v) = values.into_iter().find(|(n, _)| n.eq_ignore_case_str(name))?;
    let units: Vec<u16> = v.data.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    let mut s = WString::from_units(units);
    if !s.is_empty() && s.at(s.len() - 1) == 0 {
        s = s.slice(0, s.len() - 1);
    }
    match v.kind {
        1 => Some(s.to_string_lossy()),
        2 => Some(sys::expand_environment_strings(&s).to_string_lossy()),
        _ => None,
    }
}

/// What .NET's registry calls throw, as their message (the Settings switch shows it).
fn reg_error(subkey: &str, code: u32) -> String {
    if code == sys::ERROR_ACCESS_DENIED {
        format!("Access to the registry key 'HKEY_CURRENT_USER\\{subkey}' is denied.")
    } else {
        sys::win32_message(code)
    }
}

fn set_values(subkey: &str, values: &[(&str, String)]) -> Result<(), String> {
    sys::reg_write_values(subkey, values, &[]).map_err(|e| reg_error(subkey, e))
}

pub fn ultracopier_installed() -> bool {
    if !sys::process_ids("ultracopier.exe").is_empty() {
        return true;
    }
    if sys::reg_value_exists(&key(RUN_KEY), "ultracopier") {
        return true;
    }
    let pf = sys::known_folder(Folder::ProgramFiles);
    let p = crate::engine::paths::combine(&crate::engine::paths::combine(&pf, &WString::from("Ultracopier")), &WString::from("ultracopier.exe"));
    let a = sys::get_file_attributes(&p);
    a != sys::INVALID_FILE_ATTRIBUTES && (a & sys::FILE_ATTRIBUTE_DIRECTORY) == 0
}

/// Turns the takeover on/off. None when it worked, else what went wrong.
pub fn set(on: bool) -> Option<String> {
    if registry_off() {
        // tests never touch the real Explorer, startup or Ultracopier
        {
            let mut s = Settings::current();
            s.explorer_takeover = on;
            s.save();
        }
        crate::app::apply_takeover();
        return None;
    }
    let run = key(RUN_KEY);
    let r = (|| -> Result<(), String> {
        if on {
            stop_ultracopier();
            ensure_menu();
            set_values(&run, &[("Choppy", format!("\"{}\" --tray", exe_path()))])?;
        } else {
            if sys::reg_value_exists(&run, "Choppy") {
                sys::reg_write_values(&run, &[], &["Choppy"]).map_err(|e| reg_error(&run, e))?;
            }
            restore_ultracopier();
        }
        Ok(())
    })();
    if let Err(e) = r {
        return Some(e);
    }
    {
        let mut s = Settings::current();
        s.explorer_takeover = on;
        s.save();
    }
    crate::app::apply_takeover();
    None
}

/// The installed copy (not a build or test exe) is the one that owns the right-click menu.
pub fn is_installed_copy() -> bool {
    let base = if test_root().is_some() {
        Some(WString::from_os(app_paths::data_dir().as_os_str()))
    } else {
        sys::local_app_data()
    };
    let Some(base) = base else { return false };
    let dir = crate::engine::paths::combine(&crate::engine::paths::combine(&base, &WString::from("Programs")), &WString::from("Choppy"));
    exe_path().starts_with_ignore_case(&(dir + "\\"))
}

/// Right-click "Paste with Choppy" (installed copy only).
pub fn ensure_menu() {
    if registry_off() || !is_installed_copy() {
        return;
    }
    let _ = write_verbs();
}

/// Keeps the registered paths pointing at this exe (if Choppy was moved/updated).
pub fn refresh() {
    ensure_menu();
    if registry_off() || !Settings::current().explorer_takeover {
        return;
    }
    let _ = set_values(&key(RUN_KEY), &[("Choppy", format!("\"{}\" --tray", exe_path()))]);
}

/// Uninstall: default off (Ultracopier's entry back) and the right-click menu removed.
pub fn remove_all() -> Option<String> {
    let err = set(false);
    if !registry_off() {
        remove_verbs();
    }
    err
}

fn write_verbs() -> Result<(), String> {
    let exe = exe_path();
    for parent in VERB_PARENTS {
        let k = key(&format!(r"{parent}\{VERB_NAME}"));
        set_values(
            &k,
            &[("MUIVerb", "Paste with Choppy".into()), ("Icon", format!("\"{exe}\",0")), ("Position", "Top".into())],
        )?;
        let arg = if parent.contains("Background") { "%V" } else { "%1" };
        set_values(&format!(r"{k}\command"), &[("", format!("\"{exe}\" paste \"{arg}\""))])?;
    }
    Ok(())
}

fn remove_verbs() {
    for parent in VERB_PARENTS {
        let _ = sys::reg_delete_tree(&key(&format!(r"{parent}\{VERB_NAME}")));
    }
}

fn stop_ultracopier() {
    // quit it FIRST: "ultracopier.exe quit" starts a second Ultracopier that writes its startup entry again
    for pid in sys::process_ids("ultracopier.exe") {
        if let Some(exe) = sys::process_exe(pid) {
            let dir = crate::engine::paths::parent(&exe).unwrap_or_default();
            if let Ok(q) = std::process::Command::new(exe.to_os()).arg("quit").current_dir(dir.to_os()).spawn() {
                sys::wait_child(&q, 5000);
            }
        }
        if !sys::wait_process(pid, 5000) {
            sys::close_main_window(pid);
        }
    }
    for _ in 0..20 {
        if sys::process_ids("ultracopier.exe").is_empty() {
            break;
        }
        sys::sleep_ms(250);
    }
    // then park its startup entry
    let run = key(RUN_KEY);
    if let Some(v) = reg_string(&run, "ultracopier") {
        Settings::current().ultracopier_run_backup = Some(v);
        let _ = sys::reg_write_values(&run, &[], &["ultracopier"]);
    }
}

fn restore_ultracopier() {
    let backup = Settings::current().ultracopier_run_backup.clone();
    let Some(b) = backup.filter(|b| !b.is_empty()) else { return };
    if set_values(&key(RUN_KEY), &[("ultracopier", b)]).is_ok() {
        Settings::current().ultracopier_run_backup = None;
    }
}
