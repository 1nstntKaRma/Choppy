//! The command line (Cli.cs), split by command: --usb + --plan-dump (step 1), --cli + --selftest (step 2; step 1's
//! checks are in already), --benchmark + --shellcopy (step 4), --where (step 5).
//! Exit code: 0 = all done, 2 = some files failed/skipped (self-test: a check failed), 3 = canceled, 1 = error.
#![deny(unsafe_code)]

mod plan_dump;
mod selftest;
mod usb;

use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::wstr::WString;

pub fn run(args: &[WString]) -> i32 {
    let r = catch_unwind(AssertUnwindSafe(|| {
        let first = args.first().map(|a| a.to_string_lossy()).unwrap_or_default();
        match first.as_str() {
            "--selftest" => selftest::self_test(args.get(1)),
            "--usb" => usb::usb(args.get(1)),
            "--plan-dump" => plan_dump::plan_dump(args),
            _ => {
                eprintln!("not ported yet: {first}");
                1
            }
        }
    }));
    match r {
        Ok(code) => code,
        Err(e) => {
            // as the C# catch: the error goes to %TEMP%\choppy-cli-error.txt
            let text = e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "panic".into());
            let _ = std::fs::write(std::env::temp_dir().join("choppy-cli-error.txt"), text);
            1
        }
    }
}

/// File.WriteAllText(path, text) (UTF-8 without a byte order mark), or with Encoding.UTF8 (with one).
fn write_all_text(path: &WString, text: &str, bom: bool) -> std::io::Result<()> {
    let mut bytes = Vec::with_capacity(text.len() + 3);
    if bom {
        bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    bytes.extend_from_slice(text.as_bytes());
    std::fs::write(path.to_os(), bytes)
}

/// StringBuilder.AppendLine: the line and "\r\n".
fn append_line(sb: &mut String, line: &str) {
    sb.push_str(line);
    sb.push_str("\r\n");
}

/// The drive roots in use, "A:\" to "Z:\" (DriveInfo.GetDrives).
fn drive_roots() -> Vec<WString> {
    let mask = crate::sys::get_logical_drives();
    (0..26u8).filter(|i| mask & (1 << i) != 0).map(|i| WString::from(format!("{}:\\", (b'A' + i) as char))).collect()
}
