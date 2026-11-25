//! Theme.xaml: the palette, the fonts and the text styles (the control styles are in kit.rs).

use crate::sys::gfx::{Argb, Fam, Font};
use crate::ui::layout::Fill;

// ===== palette (from the Choppy concept) =====
pub const WIN_TOP: Argb = 0xFF232931;
pub const WIN_BOTTOM: Argb = 0xFF1B1F25;
pub const WIN_BORDER: Argb = 0xFF333A43;
pub const CARD: Argb = 0xFF171A1F;
pub const CARD_BORDER: Argb = 0xFF262B33;
pub const TEXT: Argb = 0xFFF1F3F5;
pub const TEXT2: Argb = 0xFFDDE1E6;
pub const DIM: Argb = 0xFF9AA2AD;
pub const LABEL: Argb = 0xFF8A929E;
pub const FAINT: Argb = 0xFF6B737E;
pub const ACCENT: Argb = 0xFF0E72E6;
pub const ACCENT_HOVER: Argb = 0xFF1C80F2;
pub const ACCENT_TEXT: Argb = 0xFF5CB8FF;
pub const TRACK: Argb = 0xFF2A2F37;
pub const GOOD: Argb = 0xFF4CC38A;
pub const WARN: Argb = 0xFFE8B04B;
pub const BAD: Argb = 0xFFF07A72;
/// speed color code: right now = the graph's blue, average = lavender (also the dashed line in the graph)
pub const NOW_SPEED: Argb = 0xFF5AC8FA;
pub const AVG_SPEED: Argb = 0xFFC4A7FF;
/// sizes: done = green, total = peach; times: elapsed = amber, remaining = pink
pub const SIZE_DONE: Argb = 0xFF7DD3A0;
pub const SIZE_TOTAL: Argb = 0xFFF0B68C;
pub const TIME: Argb = 0xFFF2C46D;
pub const REMAIN: Argb = 0xFFF59BC6;
/// quick toggles: green dot = on, red dot = off
pub const ON: Argb = 0xFF3DD27A;
pub const OFF: Argb = 0xFFF0645F;
pub const SEP: Argb = 0xFF2E343C;
pub const LINK: Argb = 0xFF6CC4F5;
pub const WHITE: Argb = 0xFFFFFFFF;

/// B.Progress: #1A7FE6 -> #5AC8FA, left to right.
pub fn progress() -> Fill {
    Fill::Linear(vec![(0.0, 0xFF1A7FE6), (1.0, 0xFF5AC8FA)], (0.0, 0.0), (1.0, 0.0))
}
/// B.Window: #232931 -> #1B1F25, top to bottom.
pub fn window_bg() -> Fill {
    Fill::Linear(vec![(0.0, WIN_TOP), (1.0, WIN_BOTTOM)], (0.0, 0.0), (0.0, 1.0))
}

// ===== fonts =====
pub const fn sf(size: f64, weight: u16) -> Font {
    Font { fam: Fam::Ui, size, weight }
}
pub const fn display(size: f64, weight: u16) -> Font {
    Font { fam: Fam::Display, size, weight }
}
pub const fn icons(size: f64) -> Font {
    Font { fam: Fam::Icons, size, weight: 400 }
}
pub const NORMAL: u16 = 400;
pub const SEMI: u16 = 600;
pub const BOLD: u16 = 700;

/// T.Label: 11 SemiBold, B.Label.
pub const T_LABEL: Font = sf(11.0, SEMI);
/// T.Cap: section caps (FROM, TO, SPEED, CURRENT FILE...): 11.5 SemiBold, B.Label.
pub const T_CAP: Font = sf(11.5, SEMI);

// glyphs of Segoe Fluent Icons / MDL2 Assets
pub const G_SETTINGS: &str = "\u{E713}";
pub const G_CLOSE: &str = "\u{E8BB}";
pub const G_CHEVRON_DOWN: &str = "\u{E70D}";
pub const G_CHEVRON_UP: &str = "\u{E70E}";
pub const G_CHECK: &str = "\u{E73E}";
pub const G_CANCEL: &str = "\u{E711}";
pub const G_MINIMIZE: &str = "\u{E921}";
pub const G_MAXIMIZE: &str = "\u{E922}";
pub const G_RESTORE: &str = "\u{E923}";
pub const G_REFRESH: &str = "\u{E72C}";
pub const G_REMOVE: &str = "\u{E738}";
pub const G_ADD: &str = "\u{E710}";
pub const G_WARNING: &str = "\u{E7BA}";
