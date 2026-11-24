//! Look test A: Choppy's copy window drawn with Direct2D + DirectWrite (the official `windows` crate), no UI
//! framework. Shows the README demo copy, animated every 100 ms. See ../../../PLAN.md (UI look test).
#![windows_subsystem = "windows"]

mod clip;
mod controls;
mod gfx;
mod icon;
mod popup;
mod textbox;
mod ui;
mod window;

use windows::Win32::System::Com::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

fn main() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let args = look_common::demo::Args::parse();
    let _main = window::Main::create(&args).expect("window");
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
