//! Shared by both look-test prototypes, so A and B show exactly the same numbers:
//! - `format`: Format.cs (sizes, speeds, times) as the copy window uses it;
//! - `graph`: SpeedGraph's math from Controls.cs (smoothing, moving scale, Catmull-Rom curve);
//! - `demo`: a fake copy job with the README demo (D:\Home Videos -> X:\Backup\Home Videos, 57 MB/s limit) that
//!   moves like the real engine and produces what MainWindow.Tick() shows, every 100 ms.

pub mod demo;
pub mod format;
pub mod graph;

/// Colors from Theme.xaml / the code (0xAARRGGBB).
pub mod color {
    pub const WIN_TOP: u32 = 0xFF232931;
    pub const WIN_BOTTOM: u32 = 0xFF1B1F25;
    pub const WIN_BORDER: u32 = 0xFF333A43;
    pub const CARD: u32 = 0xFF171A1F;
    pub const CARD_BORDER: u32 = 0xFF262B33;
    pub const TEXT: u32 = 0xFFF1F3F5;
    pub const TEXT2: u32 = 0xFFDDE1E6;
    pub const DIM: u32 = 0xFF9AA2AD;
    pub const LABEL: u32 = 0xFF8A929E;
    pub const FAINT: u32 = 0xFF6B737E;
    pub const ACCENT: u32 = 0xFF0E72E6;
    pub const ACCENT_HOVER: u32 = 0xFF1C80F2;
    pub const ACCENT_TEXT: u32 = 0xFF5CB8FF;
    pub const TRACK: u32 = 0xFF2A2F37;
    pub const WARN: u32 = 0xFFE8B04B;
    pub const NOW_SPEED: u32 = 0xFF5AC8FA;
    pub const AVG_SPEED: u32 = 0xFFC4A7FF;
    pub const SIZE_DONE: u32 = 0xFF7DD3A0;
    pub const SIZE_TOTAL: u32 = 0xFFF0B68C;
    pub const TIME: u32 = 0xFFF2C46D;
    pub const REMAIN: u32 = 0xFFF59BC6;
    pub const ON: u32 = 0xFF3DD27A;
    pub const OFF: u32 = 0xFFF0645F;
    pub const PROGRESS_FROM: u32 = 0xFF1A7FE6;
    pub const PROGRESS_TO: u32 = 0xFF5AC8FA;
    pub const TITLE_TEXT: u32 = 0xFFE9ECF0;
    pub const PERCENT: u32 = 0xFFF5F7FA;
    pub const DRIVE_ICON: u32 = 0xFFAEB6C2;
    pub const BADGE_BG: u32 = 0xFF132B44;
    pub const BADGE_BORDER: u32 = 0xFF245E9C;
    /// Largest files: the four biggest, then "Other".
    pub const SPLIT: [u32; 4] = [0xFF4FC3F7, 0xFF7FE3D4, 0xFFB79CFF, 0xFFF5C542];
    pub const OTHER: u32 = 0xFF5F6772;
}
