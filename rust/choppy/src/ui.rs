//! The windows (step 4): Theme, Controls, Format, Helpers, the copy window (compact / extended), Settings, Conflict,
//! the dialogs, the Benchmark window and the tray. Built on look test A's Direct2D layer (sys/gfx.rs, sys/win.rs)
//! with a small WPF-like layout (layout.rs) so each window follows its XAML element by element.
#![deny(unsafe_code)]

pub mod bench_window;
pub mod conflict_window;
pub mod controls;
pub mod dialogs;
pub mod format;
pub mod host;
pub mod kit;
pub mod layout;
pub mod main_window;
pub mod settings_window;
pub mod theme;
pub mod tray_menu;

/// Choppy.ico (the C# exe carries it as a resource too).
pub const ICO: &[u8] = include_bytes!("../res/Choppy.ico");

/// ChoppyIcon.Frame's scale: the system DPI (the primary screen's), which picks the icon's frame.
pub fn icon_scale() -> f64 {
    (crate::sys::win::system_dpi() as f64 / 96.0).max(1.0)
}
