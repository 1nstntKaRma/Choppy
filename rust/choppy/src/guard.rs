//! The guard (2.0): a tray Choppy (Default file manager on) starts a second, tiny Choppy.exe that only waits for it to
//! end. It never looks inside it, never asks it anything and never closes it - a Choppy that isn't answering may still
//! come back. Only once that process is completely gone without having quit (ended from Task Manager, a hard stop,
//! Windows ending it) does the guard start Choppy again in the tray, once, and only within limits:
//!   - a quit on purpose (the tray's Quit, the installer, Default file manager turned off, Windows signing out) never
//!     starts anything: the tray Choppy says so first (`quitting`) and the guard leaves with it;
//!   - the Choppy that ended ran at least 30 s (one that ends right at the start would only end again; as the C#'s
//!     OnCrash);
//!   - fewer than 3 were started again in the last hour;
//!   - Default file manager is still on.
//!
//! Each ending without a quit is noted in errors.txt (with Windows' exit code) and in the Ctrl+V log.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

use crate::engine::settings::{Settings, app_paths};
use crate::explorer::paste_log;
use crate::sys::{self, GuardEnd, NamedEvent};

pub const ARG: &str = "--guard";

/// The Choppy that ended ran at least this long (seconds) to be started again.
const MIN_RAN: f64 = 30.0;
/// At most this many starts again within the last hour.
const MAX_PER_HOUR: usize = 3;

static STARTED: AtomicBool = AtomicBool::new(false);
static QUIT: Mutex<Option<NamedEvent>> = Mutex::new(None);

fn event_name(pid: u32) -> String {
    format!(r"Local\{}-quit-{pid}", crate::instance::name(false))
}

/// The tray Choppy (not the administrator copy) gets its guard, once.
pub fn start() {
    if STARTED.swap(true, SeqCst) {
        return;
    }
    let pid = sys::current_pid();
    let Some(ev) = NamedEvent::create(&event_name(pid)) else { return };
    *QUIT.lock().unwrap_or_else(|e| e.into_inner()) = Some(ev);
    let exe = sys::current_exe();
    if std::process::Command::new(exe.to_os()).arg(ARG).arg(pid.to_string()).spawn().is_err() {
        STARTED.store(false, SeqCst);
    }
}

/// A guard waits for this Choppy.
pub fn running() -> bool {
    STARTED.load(SeqCst)
}

/// This Choppy ends on purpose: its guard leaves without starting anything.
pub fn quitting() {
    if let Some(ev) = QUIT.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        ev.set();
    }
}

/// `--guard <pid>`: the guard process.
pub fn run(pid: &str) -> i32 {
    let Some(pid) = crate::engine::netfmt::parse_int(pid).filter(|p| *p > 0) else { return 0 };
    let pid = pid as u32;
    sys::background_quiet();
    // the event is the tray Choppy's: gone already = nothing to guard
    let Some(quit) = NamedEvent::open(&event_name(pid)) else { return 0 };
    let GuardEnd::Gone { code, ran } = sys::wait_guarded(pid, &quit) else { return 0 };
    if sys::shutting_down() {
        return 0;
    }
    let what = format!("Choppy ended without quitting (exit code 0x{code:08X}, after {})", ran_text(ran));
    if ran < MIN_RAN {
        crate::app::note_error(&format!("{what}; not started again: it ended right after it started"), true);
        return 0;
    }
    if !Settings::current().explorer_takeover {
        crate::app::note_error(&format!("{what}; not started again: Default file manager is off"), true);
        return 0;
    }
    let recent = restarts_last_hour();
    if recent.len() >= MAX_PER_HOUR {
        crate::app::note_error(&format!("{what}; not started again: it was started again {MAX_PER_HOUR} times within an hour"), true);
        paste_log::write_now("Choppy ended again; not started again this time (start it from the Start menu).");
        return 0;
    }
    save_restarts(&recent, sys::now_file_time());
    crate::app::note_error(&format!("{what}; started again in the tray"), true);
    paste_log::write_now("Choppy ended without quitting; started again in the tray.");
    let _ = std::process::Command::new(sys::current_exe().to_os()).arg("--tray").spawn();
    0
}

fn ran_text(s: f64) -> String {
    if s < 120.0 {
        format!("{} s", s.round() as i64)
    } else if s < 7200.0 {
        format!("{} min", (s / 60.0).round() as i64)
    } else {
        format!("{:.1} h", s / 3600.0)
    }
}

/// When it was started again lately (FILETIMEs), kept in the data folder.
fn restarts_file() -> std::path::PathBuf {
    app_paths::data_dir().join("restarts.txt")
}

fn restarts_last_hour() -> Vec<i64> {
    let hour_ago = sys::now_file_time() - 3600 * 10_000_000;
    std::fs::read_to_string(restarts_file())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().parse::<i64>().ok())
        .filter(|t| *t > hour_ago)
        .collect()
}

fn save_restarts(recent: &[i64], now: i64) {
    let mut text: String = recent.iter().map(|t| format!("{t}\r\n")).collect();
    text.push_str(&format!("{now}\r\n"));
    let _ = std::fs::write(restarts_file(), text);
}
