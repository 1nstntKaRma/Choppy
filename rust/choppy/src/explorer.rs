//! Explorer and system (step 5): the Ctrl+V hook (PasteHook.cs: paste_hook, paste_log, clipboard_files), which folder
//! an Explorer window shows (Takeover.cs ShellLocation), the registry entries and the other copier (Takeover.cs:
//! takeover) and the icon cache (IconRefresh.cs).
#![deny(unsafe_code)]

pub mod clipboard_files;
pub mod icon_refresh;
pub mod paste_hook;
pub mod paste_log;
pub mod shell_location;
pub mod takeover;

pub use takeover::{set as takeover_set, ultracopier_installed};
