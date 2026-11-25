//! `--where [out.txt]` (Cli.Where, step 5): a read-only check - every open Explorer window and the folder a paste would
//! go to, with how long each lookup took; then the desktop. Written as UTF-8 with a byte order mark (nothing without a
//! file).

use std::time::Instant;

use super::{append_line, write_all_text};
use crate::explorer::shell_location;
use crate::sys;
use crate::wstr::WString;

pub fn where_(out_file: Option<&WString>) -> i32 {
    sys::com_init_sta(); // C#'s Main is [STAThread]
    let mut sb = String::new();
    let mut h = 0isize;
    let mut n = 0;
    loop {
        h = sys::find_window(0, h, "CabinetWClass");
        if h == 0 {
            break;
        }
        n += 1;
        let sw = Instant::now();
        let (dest, is_virtual) = shell_location::folder_of(h);
        let what = match dest {
            Some(d) => d.to_string_lossy(),
            None if is_virtual => "(not a folder - Explorer keeps that paste)".into(),
            None => "(Explorer didn't answer)".into(),
        };
        append_line(&mut sb, &format!("window {h:X} -> {what}  [{} ms]", sw.elapsed().as_millis()));
    }
    let desk = sys::find_window(0, 0, "Progman");
    let d = shell_location::folder_of(desk).0.map(|d| d.to_string_lossy()).unwrap_or_default();
    append_line(&mut sb, &format!("desktop -> {d}"));
    append_line(&mut sb, &format!("{n} Explorer windows"));
    if let Some(f) = out_file {
        let _ = write_all_text(f, &sb, true);
    }
    0
}
