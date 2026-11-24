//! Choppy: the Rust port of the C# app in ../../src (the spec). See ../../PLAN.md.
//! Modules fill up step by step; `sys` is the only place with `unsafe`.
#![windows_subsystem = "windows"]
#![deny(unsafe_code)]
// the port fills in step by step: some parts are only used by the steps after this one
#![allow(dead_code)]

mod bench;
mod cli;
mod engine;
mod explorer;
mod sys; // the one module with unsafe code (it allows it itself)
mod testhook;
mod ui;
mod wstr;

use engine::paths;
use engine::settings::app_paths;
use wstr::WString;

/// Program.Main (step 5 ports the rest: the app, the single instance, the hand-over).
fn main() {
    let args: Vec<WString> = std::env::args_os().skip(1).map(|a| WString::from_os(&a)).collect();
    if let Ok(cwd) = std::env::current_dir() {
        paths::set_start_dir(WString::from_os(cwd.as_os_str()));
    }
    // (step 2: Handoff.Prepare(args) - an administrator copy takes test mode from its record)
    // Never work "in" a random folder: anything relative can only ever land in Choppy's own data folder.
    let data = app_paths::data_dir();
    if std::fs::create_dir_all(&data).is_ok() {
        let _ = std::env::set_current_dir(&data);
    }
    let first = args.first().map(|a| a.to_string_lossy()).unwrap_or_default();
    if matches!(
        first.as_str(),
        "--cli" | "--selftest" | "--where" | "--usb" | "--benchmark" | "--shellcopy" | "--plan-dump"
    ) {
        std::process::exit(cli::run(&args));
    }
    // (step 5: --revive, Instance.Claim, App.Start)
    std::process::exit(1);
}
