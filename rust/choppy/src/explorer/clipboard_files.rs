//! The files on the clipboard and whether they were cut (move) or copied (ClipboardFiles).

use crate::sys::{self, Clip};
use crate::wstr::WString;

/// The files and whether they were cut. busy = another program held the clipboard the whole time (tried for about
/// half a second).
pub struct Files {
    pub files: Vec<WString>,
    pub moved: bool,
}

pub fn read_busy() -> (Option<Files>, bool) {
    let mut attempt = 0;
    loop {
        match sys::clipboard_files() {
            Ok(Clip::NoFiles) => return (None, false),
            Ok(Clip::Files(files, effect)) => {
                // DROPEFFECT_MOVE only = cut
                let moved = effect.is_some_and(|de| (de & 2) != 0 && (de & 1) == 0);
                return if files.is_empty() { (None, false) } else { (Some(Files { files, moved }), false) };
            }
            Err(()) => {
                // another program has the clipboard open right now (CLIPBRD_E_CANT_OPEN): wait a moment
                if attempt >= 10 {
                    return (None, true);
                }
                sys::sleep_ms(50);
            }
        }
        attempt += 1;
    }
}

pub fn read() -> Option<Files> {
    read_busy().0
}
