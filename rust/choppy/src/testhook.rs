//! The opt-in test driver (TestHook.cs, CHOPPY_TESTHOOK=<dir>): reads <dir>\cmd.txt, runs the lines, appends results
//! to out.txt. Never moves the real mouse or keyboard: buttons are pressed in code, pictures are drawn off-screen.
//! CHOPPY_PARK=1 keeps every window off-screen with no taskbar button. The same commands and answers as the C#:
//!   shot main|conflict|settings|dialog|benchmark|unithelp file.png     press pause|skip|cancel|verify|unit|queue|retry
//!   conflict replace|skip|keepboth|compare [all]   dialog 1|2   about   settings open|close   set key value
//!   benchmark ...   state file.json   enqueue copy|move|paste ...args separated by |   quit
#![deny(unsafe_code)]

use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::sys::win;
use crate::ui::host::Window;

/// With the test hook, CHOPPY_DPI96=1 makes the UI thread DPI-unaware before any window exists: every window at
/// 96 DPI (100 %), as the C# test hook on the rust branch does, for the step-4 pictures at 100 %.
pub fn dpi96_if_asked() {
    if std::env::var("CHOPPY_DPI96").as_deref() == Ok("1") && std::env::var("CHOPPY_TESTHOOK").is_ok_and(|d| !d.is_empty()) {
        win::thread_dpi_unaware();
    }
}

pub fn park() -> bool {
    std::env::var("CHOPPY_PARK").as_deref() == Ok("1")
}

pub struct TestHook {
    dir: PathBuf,
    timer: usize,
}

/// The pictures are drawn at 1.5 x (144 DPI), like the C#'s RenderTargetBitmap.
const SHOT_DPI: f64 = 144.0;

impl TestHook {
    pub fn start(dir: &str) -> TestHook {
        let dir = PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        let now = crate::ui::format::When::from_local_file_time(crate::sys::now_local_file_time());
        let ms = (crate::sys::now_local_file_time() / 10_000) % 1000;
        let sec = (crate::sys::now_local_file_time() / 10_000_000) % 60;
        append(&dir.join("out.txt"), &format!("ready {:02}:{:02}:{:02}.{:03}\r\n", now.hour, now.minute, sec, ms));
        let d = dir.clone();
        let timer = win::timer(150, false, Rc::new(move || poll(&d)));
        TestHook { dir, timer }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for TestHook {
    fn drop(&mut self) {
        win::kill(self.timer);
    }
}

fn append(f: &Path, text: &str) {
    if let Ok(mut fh) = std::fs::OpenOptions::new().create(true).append(true).open(f) {
        let _ = fh.write_all(text.as_bytes());
    }
}

fn poll(dir: &Path) {
    let f = dir.join("cmd.txt");
    if !f.exists() {
        return;
    }
    // File.ReadAllLines (UTF-8, a BOM skipped), then the file goes
    let Ok(bytes) = std::fs::read(&f) else { return };
    if std::fs::remove_file(&f).is_err() {
        return;
    }
    let text = String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes)).to_string();
    for raw in text.split('\n') {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let result = match catch_unwind(AssertUnwindSafe(|| run(line))) {
            Ok(r) => r,
            Err(e) => format!("error {}", crate::engine::panic_message(e.as_ref())),
        };
        append(&dir.join("out.txt"), &format!("{line} => {result}\r\n"));
    }
}

fn run(line: &str) -> String {
    let (cmd, arg) = match line.split_once(' ') {
        Some((c, a)) => (c, a),
        None => (line, ""),
    };
    match cmd {
        "shot" => {
            let (what, file) = arg.split_once(' ').unwrap_or((arg, ""));
            let file = PathBuf::from(file);
            // the settings' "?" card is a pop-up of its own: "set unithelp 1", then "shot unithelp file.png"
            if what == "unithelp" {
                let Some(s) = crate::app::with(|a| a.settings.clone()).flatten() else { return "settings not open".into() };
                let Ok(s) = s.try_borrow() else { return "busy".into() };
                s.c.shot_unit_help(&s.host, &file);
                return "ok".into();
            }
            let ok = match what {
                "conflict" => shot(crate::app::with(|a| a.conflict.clone()).flatten(), &file),
                "settings" => shot(crate::app::with(|a| a.settings.clone()).flatten(), &file),
                "dialog" => shot(crate::app::top_dialog(), &file),
                "benchmark" => shot(crate::app::with(|a| a.bench.clone()).flatten(), &file),
                _ => shot(crate::app::main_window(), &file),
            };
            if ok { "ok".into() } else { "no window".into() }
        }
        "press" => {
            let Some(m) = crate::app::main_window() else { return "no main window".into() };
            let Ok(mut m) = m.try_borrow_mut() else { return "busy".into() };
            let Window { host, c } = &mut *m;
            c.press_for_test(host, arg.trim());
            "ok".into()
        }
        "conflict" => {
            let Some(w) = crate::app::with(|a| a.conflict.clone()).flatten() else { return "no conflict window".into() };
            let Ok(mut w) = w.try_borrow_mut() else { return "busy".into() };
            let Window { host, c } = &mut *w;
            let p: Vec<&str> = arg.split(' ').collect();
            c.press_for_test(host, p[0], p.len() > 1 && p[1] == "all");
            "ok".into()
        }
        "dialog" => {
            let Some(d) = crate::app::top_dialog() else { return "no dialog".into() };
            let Ok(mut d) = d.try_borrow_mut() else { return "busy".into() };
            let Window { host, c } = &mut *d;
            match arg.trim().parse::<i32>() {
                Ok(r) => {
                    c.answer(host, r);
                    "ok".into()
                }
                Err(_) => "error Input string was not in a correct format.".into(),
            }
        }
        "about" => {
            // modal: opened after this command returns, so the hook keeps answering ("dialog 1" closes it)
            win::post_local(Box::new(|| {
                let owner = crate::app::with(|a| a.settings.clone())
                    .flatten()
                    .and_then(|s| s.try_borrow().ok().map(|s| s.host.win))
                    .or_else(|| crate::app::main_window().and_then(|m| m.try_borrow().ok().map(|m| m.host.win)));
                crate::ui::dialogs::about(owner);
            }));
            "ok".into()
        }
        "settings" => {
            if arg.trim() == "open" {
                let owner = crate::app::main_window().and_then(|m| m.try_borrow().ok().map(|m| m.host.win));
                crate::app::show_settings(owner);
            } else if let Some(s) = crate::app::with(|a| a.settings.clone()).flatten()
                && let Ok(mut s) = s.try_borrow_mut()
            {
                s.host.close();
            }
            "ok".into()
        }
        "benchmark" => {
            if arg.trim() == "open" {
                crate::app::show_benchmark();
                return "ok".into();
            }
            let Some(b) = crate::app::with(|a| a.bench.clone()).flatten() else { return "benchmark not open".into() };
            let Ok(mut b) = b.try_borrow_mut() else { return "busy".into() };
            let Window { host, c } = &mut *b;
            if arg.trim() == "close" {
                c.close_for_test(host);
                return "ok".into();
            }
            c.for_test(host, arg)
        }
        "set" => {
            let Some(s) = crate::app::with(|a| a.settings.clone()).flatten() else { return "settings not open".into() };
            let Ok(mut s) = s.try_borrow_mut() else { return "busy".into() };
            let Window { host, c } = &mut *s;
            let (k, v) = arg.split_once(' ').unwrap_or((arg, ""));
            c.set_for_test(host, k, v);
            "ok".into()
        }
        "state" => {
            let json = match crate::app::main_window() {
                Some(m) => match m.try_borrow() {
                    Ok(m) => m.c.state_for_test(&m.host),
                    Err(_) => "{\"phase\":\"busy\"}".into(),
                },
                None => "{\"phase\":\"no window\"}".into(),
            };
            // File.WriteAllText(..., Encoding.UTF8): with a byte order mark
            let mut b = vec![0xEF, 0xBB, 0xBF];
            b.extend_from_slice(json.as_bytes());
            match std::fs::write(arg.trim(), b) {
                Ok(_) => "ok".into(),
                Err(e) => format!("error {e}"),
            }
        }
        "enqueue" => {
            let args: Vec<crate::wstr::WString> = arg.split('|').map(|s| crate::wstr::WString::from(s.trim())).collect();
            crate::app::handle_args(&args);
            "ok".into()
        }
        "quit" => {
            crate::app::quit_for_test();
            "ok".into()
        }
        _ => "unknown command".into(),
    }
}

fn shot<C: crate::ui::host::Content>(w: Option<std::rc::Rc<std::cell::RefCell<Window<C>>>>, file: &Path) -> bool {
    let Some(w) = w else { return false };
    let Ok(mut w) = w.try_borrow_mut() else { return false };
    let Window { host, c } = &mut *w;
    host.shot(c, file, SHOT_DPI)
}
