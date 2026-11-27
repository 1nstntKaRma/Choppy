//! Choppy: the Rust port of the C# app in ../../src (the spec). See ../../PLAN.md.
//! Modules fill up step by step; `sys` is the only place with `unsafe`.
#![windows_subsystem = "windows"]
#![deny(unsafe_code)]
#![allow(clippy::too_many_arguments, clippy::type_complexity)]

mod app;
mod bench;
mod cli;
mod engine;
mod explorer;
mod guard;
mod instance;
mod job_manager;
mod sys; // the one module with unsafe code (it allows it itself)
mod testhook;
mod ui;
mod wstr;

#[cfg(test)]
mod step5_tests;

use std::sync::OnceLock;
use std::time::Instant;

use engine::paths;
use engine::settings::app_paths;
use wstr::WString;

static STARTED: OnceLock<Instant> = OnceLock::new();

/// When this process started (Process.StartTime: a crash right at the start doesn't revive).
pub fn started() -> Instant {
    *STARTED.get_or_init(Instant::now)
}

/// Program.Main
fn main() {
    started();
    let mut args: Vec<WString> = std::env::args_os().skip(1).map(|a| WString::from_os(&a)).collect();
    if let Ok(cwd) = std::env::current_dir() {
        paths::set_start_dir(WString::from_os(cwd.as_os_str()));
    }
    engine::handoff::prepare(&args); // an administrator copy takes test mode from its record (a fresh environment)
    // Never work "in" a random folder: anything relative can only ever land in Choppy's own data folder.
    let data = app_paths::data_dir();
    if std::fs::create_dir_all(&data).is_ok() {
        let _ = std::env::set_current_dir(&data);
    }
    let first = args.first().map(|a| a.to_string_lossy()).unwrap_or_default();
    if matches!(
        first.as_str(),
        "--cli" | "--selftest" | "--where" | "--usb" | "--benchmark" | "--shellcopy" | "--plan-dump" | "--benchcopy" | "--diag"
    ) {
        std::process::exit(cli::run(&args));
    }
    // the tray Choppy's guard (guard.rs): only waits for it to end; no window, no instance of its own
    if args.len() == 2 && args[0].eq_str(guard::ARG) {
        std::process::exit(guard::run(&args[1].to_string_lossy()));
    }
    // a Choppy that ended on an error started this one: wait until it is gone, then watch Ctrl+V from the tray
    if args.len() == 2
        && args[0].eq_str("--revive")
        && let Some(dead) = engine::netfmt::parse_int(&args[1].to_string_lossy())
    {
        if dead > 0 {
            sys::wait_process(dead as u32, 20000);
        }
        args = vec![WString::from("--tray")];
    }
    // a Choppy is already running: hand it the job and exit, before any window is made
    let elevated = args.len() >= 2 && args[0].eq_str(engine::handoff::ARG);
    let name = instance::name(elevated);
    match instance::claim(&args, &name) {
        instance::Claimed::Exit(code) => std::process::exit(code),
        instance::Claimed::Run(mutex) => std::process::exit(app::run(&args, mutex, &name, elevated)),
    }
}
