//! Explorer and system: PasteHook, Takeover, IconRefresh (step 5). Step 4 has what the settings window needs:
//! whether another copier that takes Ctrl+V is installed, and turning "Default file manager" on / off in test mode.
#![deny(unsafe_code)]

use crate::engine::settings::{Settings, app_paths};

/// Takeover.UltracopierInstalled: running, in the Run key, or in Program Files.
pub fn ultracopier_installed() -> bool {
    if crate::sys::process_running("ultracopier.exe") {
        return true;
    }
    if crate::sys::reg_value_exists(r"Software\Microsoft\Windows\CurrentVersion\Run", "ultracopier") {
        return true;
    }
    let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
    std::path::Path::new(&pf).join("Ultracopier").join("ultracopier.exe").exists()
}

/// Takeover.Set: tests never touch the real Explorer, startup or Ultracopier (the rest comes with step 5).
/// Returns the error to show, or None.
pub fn takeover_set(on: bool) -> Option<String> {
    if app_paths::test_mode() {
        {
            let mut s = Settings::current();
            s.explorer_takeover = on;
            s.save();
        }
        crate::app::apply_takeover();
        return None;
    }
    Some("Not available in this build yet: Explorer integration comes with step 5 of the move to Rust.".into())
}
