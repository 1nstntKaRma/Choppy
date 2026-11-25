//! `--benchcopy job.txt`: one timed copy for the C# Benchmark (the move to Rust, step 3; Bench/EngineChild.cs on the
//! rust branch runs it, and the C# Choppy takes the same switch). Lines: the warm-up file, a folder for it, the source,
//! the destination folder, the engine's test switches (or empty).
//!
//! The time is counted as the Benchmark counts its own in-process copy (Benchmark.ChoppyCopy): the warm-up file is
//! copied first, untimed (both drives looked at once); then the clock runs from right before the job starts until
//! its Finished event. Starting the process is not counted. "ended=" is the moment of that event (a FILETIME to the
//! microsecond): the parent adds the time from then until its flush starts, so this process's end counts against
//! this side, and then flushes the copies (the time to the disk) as for every side.
//! Writes "seconds=...", "ended=...", "method=..." or "error=..." to job.txt.result.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};
use std::time::Instant;

use super::job::engine_option;
use crate::engine::copy_job::CopyJob;
use crate::engine::model::{ConflictAnswer, ConflictPolicy, JobPhase, JobSpec, OpKind};
use crate::engine::{lock, paths};
use crate::sys;
use crate::wstr::WString;

const SEQ: Ordering = Ordering::SeqCst;

/// Benchmark.ChoppyCopy: Choppy's engine as any copy runs it. Seconds until it says done, the moment it did
/// (FILETIME), and the way it picked.
fn engine_copy(src: &WString, dest: &WString, opts: &[WString]) -> Result<(f64, i64, Option<String>), String> {
    let mut spec = JobSpec {
        op: OpKind::Copy,
        origin: "Benchmark".into(),
        policy: ConflictPolicy::Skip,
        dest: paths::to_long(dest),
        ..Default::default()
    };
    spec.sources.push(paths::to_long(src));
    let mut k = 0;
    while k < opts.len() {
        engine_option(opts, &mut k, &mut spec);
        k += 1;
    }
    let mut job = CopyJob::new(Arc::new(spec));
    job.ask_conflict = Some(Box::new(|_, _| {
        Some(ConflictAnswer { choice: ConflictPolicy::Skip, apply_to_all: false, cancel_job: false })
    }));
    job.ask_low_space = Some(Box::new(|_, _, _| false));
    job.ask_elevate = Some(Box::new(|_| false));
    let job = Arc::new(job);
    let (tx, rx) = mpsc::channel::<(Instant, i64)>();
    job.on_finished(Arc::new(move |_| {
        let _ = tx.send((Instant::now(), sys::now_precise_file_time()));
    }));
    let sw = Instant::now();
    job.start();
    let (end, ended) = rx.recv().map_err(|_| "the copy ended without saying so".to_string())?;
    let secs = end.duration_since(sw).as_secs_f64();
    let method = job.plan().map(|p| {
        let tune = job.tune_summary().unwrap_or_default();
        if tune.is_empty() { p.badge() } else { format!("{} ({})", p.badge(), tune) }
    });
    if job.phase() != JobPhase::Done || job.failed_files.load(SEQ) > 0 || job.skipped_files.load(SEQ) > 0 {
        let first = lock(&job.failed_items).first().cloned();
        let why = match first {
            Some(f) => f.error().unwrap_or_default(),
            None => job.status().unwrap_or_else(|| job.phase().name().to_string()),
        };
        return Err(format!("Choppy's copy didn't finish: {why}"));
    }
    Ok((secs, ended, method))
}

/// File.ReadAllLines(path, Encoding.UTF8): a byte order mark dropped; lines end at \r\n, \n or \r.
fn read_lines(path: &std::path::Path) -> Result<Vec<String>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes)).into_owned();
    let mut lines: Vec<String> = text.replace("\r\n", "\n").replace('\r', "\n").split('\n').map(String::from).collect();
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    Ok(lines)
}

fn run(job_file: &std::path::Path) -> Result<String, String> {
    let lines = read_lines(job_file)?;
    let line = |i: usize| -> Result<WString, String> {
        lines.get(i).map(|s| WString::from(s.as_str())).ok_or_else(|| "Index was outside the bounds of the array.".to_string())
    };
    let opts: Vec<WString> =
        lines.get(4).map_or("", |s| s.as_str()).split(' ').filter(|s| !s.is_empty()).map(WString::from).collect();
    let warm = line(0)?;
    if !warm.is_empty() {
        engine_copy(&warm, &line(1)?, &opts)?;
    }
    let (secs, ended, method) = engine_copy(&line(2)?, &line(3)?, &opts)?;
    Ok(format!("seconds={secs}\r\nended={ended}\r\nmethod={}", method.unwrap_or_default()))
}

pub fn child(job_file: Option<&WString>) -> i32 {
    let Some(job_file) = job_file else { return 1 };
    let job_file = std::path::PathBuf::from(job_file.to_os());
    let result = match catch_unwind(AssertUnwindSafe(|| run(&job_file))) {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => format!("error={}", e.replace(['\r', '\n'], " ")),
        Err(p) => {
            let text = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "panic".into());
            format!("error={}", text.replace(['\r', '\n'], " "))
        }
    };
    let mut out = job_file.into_os_string();
    out.push(".result");
    // File.WriteAllText(path, text, Encoding.UTF8): a byte order mark first
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(result.as_bytes());
    if std::fs::write(&out, bytes).is_err() {
        return 1;
    }
    0
}
