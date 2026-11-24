//! `--usb [out.txt]` (Cli.Usb): read-only check of how each USB drive is connected (the speed it runs at, the most it
//! can do, the port).

use std::time::Instant;

use super::{append_line, drive_roots, write_all_text};
use crate::engine::paths;
use crate::engine::usb_link::{self, UsbLink};
use crate::engine::volume_probe;
use crate::sys;
use crate::wstr::WString;

pub fn usb(out_file: Option<&WString>) -> i32 {
    let mut sb = String::new();
    for root in drive_roots() {
        let t = sys::get_drive_type(&root);
        if t == sys::DRIVE_REMOTE || t == sys::DRIVE_CDROM {
            continue;
        }
        let sw = Instant::now();
        let v = volume_probe::probe(&paths::to_long(&root));
        let link = UsbLink::for_disk(v.disk);
        let text = match &link {
            None => "not USB".to_string(),
            Some(l) => format!(
                "{}, can do {}{}, port {}, bcdUSB {:04X}{}{}",
                l.speed_text(),
                usb_link::short(l.capable),
                if l.capable_known { "" } else { " (unknown)" },
                if l.port_known { if l.port_usb3 { "USB 3" } else { "USB 2" } } else { "?" },
                l.bcd_usb,
                if l.uasp { ", UASP" } else { ", Bulk-Only" },
                if l.held_back() { format!(" - HELD BACK: {}", l.advice().unwrap_or_default()) } else { String::new() }
            ),
        };
        append_line(&mut sb, &format!("{} disk {} {}: {}  [{} ms]", root, v.disk, v.kind_text(), text, sw.elapsed().as_millis()));
    }
    match out_file {
        Some(f) => {
            write_all_text(f, &sb, true).expect("--usb: the output file");
        }
        None => print!("{sb}"),
    }
    0
}
