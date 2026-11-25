//! One Choppy per user (App.cs Instance + Ipc): its mutex, or handing a job to the one already running. A second
//! start (Explorer paste, right-click, command line) hands its command line to the first through a per-user pipe and
//! exits at once, so the window opens instantly in the running copy.
#![deny(unsafe_code)]

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use crate::engine::paths;
use crate::engine::settings::app_paths;
use crate::sys::{self, NamedMutex, Wait};
use crate::wstr::WString;

/// "Choppy-<the user's SID>" (+ "-test" in test mode, + "-admin" for the administrator copy): the mutex and the pipe.
/// A test instance never meets the installed Choppy's, an administrator copy never the user's own.
pub fn name(elevated: bool) -> String {
    let sid = sys::user_sid().unwrap_or_else(|| "user".into());
    format!("Choppy-{sid}{}{}", if app_paths::test_mode() { "-test" } else { "" }, if elevated { "-admin" } else { "" })
}

pub enum Claimed {
    /// this process runs Choppy: the instance mutex is its own
    Run(NamedMutex),
    /// nothing more to do here (the exit code)
    Exit(i32),
}

/// This process's instance mutex, or the exit code when there is nothing more to do here.
pub fn claim(args: &[WString], name: &str) -> Claimed {
    let Some((mutex, created)) = NamedMutex::create(&format!(r"Local\{name}"), true) else { return Claimed::Exit(1) };
    if created && args.first().is_some_and(|a| a.eq_str("--quit")) {
        // nothing running
        mutex.release();
        return Claimed::Exit(0);
    }
    if created {
        return Claimed::Run(mutex);
    }
    // hand the job to the running Choppy
    for _ in 0..40 {
        if send(name, args) {
            return Claimed::Exit(0);
        }
        sys::sleep_ms(100);
        // it just exited: take over (left behind by a Choppy that ended on an error counts too)
        match mutex.wait0() {
            Wait::Owned | Wait::Abandoned => return Claimed::Run(mutex),
            Wait::Busy => {}
        }
    }
    Claimed::Exit(1)
}

const SEP: char = '\u{1f}';

/// True once the job is written to the running Choppy. After that it never sends again: a lost "ok" is harmless, a
/// job sent twice is a duplicate copy.
pub fn send(name: &str, args: &[WString]) -> bool {
    let Ok(c) = sys::pipe_client(name, 1500) else { return false }; // nothing was sent: safe to try again
    // we were started by the user's click: let the running Choppy bring its window to the front
    sys::allow_set_foreground_any();
    // relative paths are made absolute here, in the folder the second copy was started in
    let abs: Vec<String> = args
        .iter()
        .enumerate()
        .map(|(i, a)| if i > 0 && !a.starts_with("-") { paths::make_abs(a) } else { a.clone() })
        .map(|a| a.to_string_lossy())
        .collect();
    let mut data = abs.join(&SEP.to_string());
    data.push('\n');
    if sys::pipe_write(&c, data.as_bytes()).is_err() {
        return false;
    }
    // the "ok": waited for up to 3 s
    let c = Arc::new(c);
    let (tx, rx) = mpsc::channel::<()>();
    let reader = c.clone();
    let _ = std::thread::Builder::new().name("ipc reply".into()).spawn(move || {
        let mut buf = [0u8; 8];
        let _ = sys::pipe_read(&reader, &mut buf);
        let _ = tx.send(());
    });
    let _ = rx.recv_timeout(Duration::from_millis(3000));
    true
}

/// Listens on the pipe for good (a background thread): each command line received goes to `on_args`.
pub fn listen(name: &str, on_args: fn(Vec<WString>)) {
    let name = name.to_string();
    let _ = std::thread::Builder::new().name("ipc".into()).spawn(move || {
        loop {
            let r = std::panic::catch_unwind(|| serve_one(&name, on_args));
            if !matches!(r, Ok(Ok(()))) {
                sys::sleep_ms(200);
            }
        }
    });
}

/// One server instance: waits for a client, reads its line, answers "ok", hands the line on.
fn serve_one(name: &str, on_args: fn(Vec<WString>)) -> Result<(), u32> {
    let sid = sys::user_sid().ok_or(sys::ERROR_ACCESS_DENIED)?;
    let s = sys::pipe_server(name, &sid)?;
    sys::pipe_wait_for_client(&s)?;
    let mut ms = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = sys::pipe_read(&s, &mut buf)?;
        if n == 0 {
            break;
        }
        ms.extend_from_slice(&buf[..n]);
        if buf[n - 1] == b'\n' {
            break;
        }
    }
    let text = String::from_utf8_lossy(&ms);
    let text = text.trim_end_matches('\n');
    sys::pipe_write(&s, b"ok")?;
    let _ = sys::pipe_drain(&s);
    on_args(if text.is_empty() { Vec::new() } else { text.split(SEP).map(WString::from).collect() });
    Ok(())
}
