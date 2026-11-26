//! What happened to each Ctrl+V of files in Explorer, and the hook's own state: %LOCALAPPDATA%\Choppy\Logs\ctrl-v.txt
//! (the newest lines; kept small). Written in the background, never from inside the hook's answer (PasteLog).

use std::collections::VecDeque;
use std::io::Write;
use std::sync::Mutex;

use crate::engine::settings::{app_paths, read_all_lines};
use crate::ui::format::When;

struct State {
    pending: VecDeque<String>,
    writing: bool,
}

static STATE: Mutex<State> = Mutex::new(State { pending: VecDeque::new(), writing: false });

/// DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss.fff")
fn now_text() -> String {
    let t = crate::sys::now_local_file_time();
    let w = When::from_local_file_time(t);
    let sec = (t / 10_000_000) % 60;
    let ms = (t / 10_000) % 1000;
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}", w.year, w.month, w.day, w.hour, w.minute, sec, ms)
}

pub fn write(line: &str) {
    {
        let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
        if s.pending.len() < 200 {
            let l = format!("{}  {line}", now_text());
            s.pending.push_back(l);
        }
        if s.writing {
            return;
        }
        s.writing = true;
    }
    // ThreadPool.QueueUserWorkItem: a thread of its own (it ends when nothing is left)
    if std::thread::Builder::new().name("ctrl-v log".into()).spawn(flush).is_err() {
        STATE.lock().unwrap_or_else(|e| e.into_inner()).writing = false;
    }
}

/// Written at once, on this thread (a process about to end: the guard).
pub fn write_now(line: &str) {
    let _ = append(&[format!("{}  {line}", now_text())]);
}

fn flush() {
    loop {
        let lines: Vec<String> = {
            let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
            if s.pending.is_empty() {
                s.writing = false;
                return;
            }
            s.pending.drain(..).collect()
        };
        let _ = std::panic::catch_unwind(|| append(&lines));
    }
}

/// File.AppendAllLines(file, lines, Encoding.UTF8): a new file starts with a byte order mark; past 256 KB the
/// newest half is kept first (File.WriteAllLines, a new file with its mark).
fn append(lines: &[String]) -> std::io::Result<()> {
    let dir = app_paths::log_dir();
    std::fs::create_dir_all(&dir)?;
    let file = dir.join("ctrl-v.txt");
    if let Ok(m) = std::fs::metadata(&file)
        && m.len() > 256 * 1024
    {
        // keep the newest half
        let all = read_all_lines(&file)?;
        let half = &all[all.len() / 2..];
        let mut text = vec![0xEF, 0xBB, 0xBF];
        for l in half {
            text.extend_from_slice(l.as_bytes());
            text.extend_from_slice(b"\r\n");
        }
        std::fs::write(&file, text)?;
    }
    let mut f = std::fs::OpenOptions::new().append(true).create(true).open(&file)?;
    let mut text = Vec::new();
    if f.metadata()?.len() == 0 {
        text.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    for l in lines {
        text.extend_from_slice(l.as_bytes());
        text.extend_from_slice(b"\r\n");
    }
    f.write_all(&text)
}
