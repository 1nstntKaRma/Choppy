//! ShellCopy.cs: Windows' own copy for the Benchmark - IFileOperation, the shell copy engine Explorer itself uses for
//! a paste, run with no UI. Each timed copy runs in a child process (choppy.exe --shellcopy job.txt), so a cancel stops
//! it at once, even in the middle of a big file.

use std::path::Path;

use super::{BenchErr, BenchResult};

fn text_of(b: &[u8]) -> String {
    String::from_utf8_lossy(b.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(b)).to_string()
}

/// The child: choppy.exe --shellcopy job.txt. Its lines: a tiny warm-up file, a folder for it, the source, the
/// destination folder. The first copy in a process loads the shell's copy engine (Explorer has it loaded already):
/// that happens on the warm-up file, untimed. Writes "seconds=..." or "error=..." to job.txt.result.
pub fn child(job_file: &str) -> i32 {
    crate::sys::com_init_sta();
    let result = (|| -> Result<String, String> {
        let b = std::fs::read(job_file).map_err(|e| super::files::io_message(&e))?;
        let t = text_of(&b);
        let lines: Vec<&str> = t.split("\r\n").flat_map(|l| l.split('\n')).collect();
        let get = |i: usize| lines.get(i).copied().unwrap_or("");
        if !get(0).is_empty() {
            crate::sys::shell_copy_item(get(0), get(1))?;
        }
        let sw = std::time::Instant::now();
        let aborted = crate::sys::shell_copy_item(get(2), get(3))?;
        let secs = sw.elapsed().as_secs_f64();
        Ok(format!("seconds={secs}\r\naborted={}", if aborted { "1" } else { "0" }))
    })()
    .unwrap_or_else(|e| format!("error={}", e.replace(['\r', '\n'], " ")));
    let mut out = vec![0xEF, 0xBB, 0xBF];
    out.extend_from_slice(result.as_bytes());
    match std::fs::write(format!("{job_file}.result"), out) {
        Ok(_) => 0,
        Err(_) => 1,
    }
}

/// One timed copy by Windows, in a child process. Returns its seconds; an error or a cancel otherwise.
pub fn run(warm_src: Option<&Path>, warm_dir: Option<&Path>, src: &Path, dest_dir: &Path, canceled: &dyn Fn() -> bool) -> BenchResult<f64> {
    let id = crate::engine::model::new_id();
    let job = std::env::temp_dir().join(format!("choppy-shellcopy-{}.txt", &id[..8]));
    let result = std::path::PathBuf::from(format!("{}.result", job.display()));
    let lines = [
        warm_src.map(|p| p.display().to_string()).unwrap_or_default(),
        warm_dir.map(|p| p.display().to_string()).unwrap_or_default(),
        src.display().to_string(),
        dest_dir.display().to_string(),
    ];
    let mut text = vec![0xEF, 0xBB, 0xBF];
    for l in lines.iter() {
        text.extend_from_slice(l.as_bytes());
        text.extend_from_slice(b"\r\n");
    }
    let r = (|| -> BenchResult<f64> {
        std::fs::write(&job, text).map_err(|e| BenchErr::Error(super::files::io_message(&e)))?;
        let exe = std::env::current_exe().map_err(|e| BenchErr::Error(e.to_string()))?;
        let mut p = crate::sys::spawn_hidden(&exe, &["--shellcopy", &job.display().to_string()]).map_err(|e| BenchErr::Error(super::files::io_message(&e)))?;
        // Process.WaitForExit(50): back the moment it ends
        while !crate::sys::wait_child(&p, 50) {
            if canceled() {
                let _ = p.kill();
                crate::sys::wait_child(&p, 10_000);
                return Err(BenchErr::Canceled);
            }
        }
        let res = match std::fs::read(&result) {
            Ok(b) => text_of(&b),
            Err(_) => "error=Windows' copy ended without a result".into(),
        };
        let mut secs = -1.0;
        for line in res.split(['\r', '\n']).filter(|l| !l.is_empty()) {
            if let Some(e) = line.strip_prefix("error=") {
                return Err(BenchErr::Error(format!("Windows' copy failed: {e}")));
            }
            if line == "aborted=1" {
                return Err(BenchErr::Error("Windows' copy stopped part way".into()));
            }
            if let Some(v) = line.strip_prefix("seconds=") {
                secs = crate::engine::netfmt::parse_double(v).unwrap_or(-1.0);
            }
        }
        if secs < 0.0 {
            return Err(BenchErr::Error("Windows' copy gave no time".into()));
        }
        Ok(secs)
    })();
    let _ = std::fs::remove_file(&job);
    let _ = std::fs::remove_file(&result);
    r
}
