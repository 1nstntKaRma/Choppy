//! `--cli copy|move <src...> <destDir> [--verify] [--policy ask|replace|skip|keepboth|newer] [--limit Mbps] [--resume]
//! [--json out.json] [--log progress.txt] [--cancel-after sec] [--pause at,for] [--skip-at name]` plus the engine's
//! test switches (Cli.Run's job part, EngineOption, ParsePolicy, Esc, Report). Headless: conflicts are skipped, a
//! destination without room cancels. Exit code: 0 = all done, 2 = some files failed/skipped, 3 = canceled, 1 = error.

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use crate::engine::big_copy::PipeTuner;
use crate::engine::copy_job::CopyJob;
use crate::engine::model::{ConflictAnswer, ConflictPolicy, JobPhase, JobSpec, OpKind};
use crate::engine::settings::journal;
use crate::engine::{lock, netfmt, paths};
use crate::sys;
use crate::wstr::WString;

const SEQ: Ordering = Ordering::SeqCst;

/// The next argument (C#'s args[++i]: past the end it threw).
fn arg(args: &[WString], i: &mut usize) -> String {
    *i += 1;
    match args.get(*i) {
        Some(a) => a.to_string_lossy(),
        None => panic!("Index was outside the bounds of the array."),
    }
}

/// double.Parse(s, CultureInfo.InvariantCulture)
fn double(s: &str) -> f64 {
    netfmt::parse_double(s).unwrap_or_else(|| panic!("Input string was not in a correct format."))
}

/// int.Parse(s)
fn int(s: &str) -> i32 {
    netfmt::parse_int(s).unwrap_or_else(|| panic!("Input string was not in a correct format."))
}

/// Test switches for the engine (fixed ways instead of Adaptive's choice). False = not one of them.
pub fn engine_option(args: &[WString], i: &mut usize, spec: &mut JobSpec) -> bool {
    match args[*i].to_string_lossy().as_str() {
        "--workers" => spec.force_workers = int(&arg(args, i)),
        "--no-fast" => spec.no_fast_path = true,
        "--always-temp" => spec.always_temp = true,
        "--engine" => {
            let e = arg(args, i);
            spec.engine = if e == "auto" { None } else { Some(e) };
        }
        "--chunk" => spec.pipe_chunk_mb = int(&arg(args, i)),
        "--slots" => spec.pipe_slots = int(&arg(args, i)),
        "--batch" => spec.pipe_batch = int(&arg(args, i)),
        "--depth" => spec.pipe_depth = int(&arg(args, i)),
        "--wstep" => spec.pipe_write_step_mb = int(&arg(args, i)),
        "--phased" => spec.pipe_phased = int(&arg(args, i)),
        "--flush-files" => spec.flush_files = int(&arg(args, i)),
        "--flush-threads" => spec.flush_threads = int(&arg(args, i)),
        "--no-prealloc" => spec.no_prealloc = true,
        "--no-tune" => spec.no_tune = true,
        "--prealloc" => spec.force_prealloc = true,
        "--small" => spec.small_mode = Some(arg(args, i)),
        "--wchunk" => spec.cached_write_mb = int(&arg(args, i)),
        "--no-tail" => spec.no_cached_tail = true,
        "--tail" => spec.tail_mb = int(&arg(args, i)),
        "--max-file" => {
            spec.max_file_test = netfmt::parse_long(&arg(args, i)).unwrap_or_else(|| panic!("Input string was not in a correct format."))
        }
        _ => return false,
    }
    true
}

pub fn parse_policy(s: &str) -> ConflictPolicy {
    match WString::from(s).to_lower_invariant().to_string_lossy().as_str() {
        "replace" => ConflictPolicy::Replace,
        "skip" => ConflictPolicy::Skip,
        "keepboth" => ConflictPolicy::KeepBoth,
        "newer" => ConflictPolicy::ReplaceOlder,
        _ => ConflictPolicy::Ask,
    }
}

/// A JSON string (or null): quotes and backslashes escaped, control characters as \u00xx.
pub fn esc(s: Option<&str>) -> String {
    let Some(s) = s else { return "null".into() };
    let mut sb = String::from("\"");
    for c in s.chars() {
        if c == '"' || c == '\\' {
            sb.push('\\');
            sb.push(c);
        } else if (c as u32) < 32 {
            sb.push_str(&format!("\\u{:04x}", c as u32));
        } else {
            sb.push(c);
        }
    }
    sb.push('"');
    sb
}

fn report(j: &CopyJob, secs: f64) -> String {
    let plan = j.plan();
    let n = |a: &std::sync::atomic::AtomicI32| a.load(SEQ);
    let mut sb = String::from("{");
    sb += &format!("\"phase\":{}", esc(Some(j.phase().name())));
    sb += &format!(",\"seconds\":{}", netfmt::f(secs, 2));
    sb += &format!(",\"badge\":{}", esc(plan.as_ref().map(|p| p.badge()).as_deref()));
    sb += &format!(",\"tune\":{}", esc(j.tune_summary().as_deref()));
    let engine = match &j.spec.engine {
        Some(e) => e.clone(),
        None => match &plan {
            Some(p) if p.pipeline => format!("pipeline {}", PipeTuner::describe(&p.pipe)),
            _ => "copyfile".into(),
        },
    };
    sb += &format!(",\"engine\":{}", esc(Some(&engine)));
    sb += &format!(",\"srcKind\":{}", esc(j.src_vol().map(|v| v.kind_text())));
    sb += &format!(",\"dstKind\":{}", esc(j.dst_vol().map(|v| v.kind_text())));
    sb += &format!(",\"rttMs\":{}", netfmt::f(j.dst_vol().map_or(-1.0, |v| v.rtt_ms), 1));
    sb += &format!(
        ",\"totalFiles\":{},\"doneFiles\":{},\"skipped\":{},\"failed\":{}",
        n(&j.total_files),
        n(&j.done_files),
        n(&j.skipped_files),
        n(&j.failed_files)
    );
    sb += &format!(",\"verified\":{},\"alreadyThere\":{}", n(&j.verified_files), n(&j.already_there_files));
    sb += &format!(",\"totalBytes\":{},\"copiedBytes\":{}", j.total_bytes.load(SEQ), j.copied_bytes.load(SEQ));
    let run = j.clock.elapsed().as_secs_f64();
    sb += &format!(",\"runSeconds\":{}", netfmt::f(run, 2));
    let copied = j.copied_bytes.load(SEQ);
    sb += &format!(",\"avgMbps\":{}", netfmt::f(if run > 0.0 { copied as f64 * 8.0 / 1e6 / run } else { 0.0 }, 1));
    sb += &format!(",\"peakMbps\":{}", netfmt::f(j.speed_peak() * 8.0 / 1e6, 1));
    sb += ",\"largest\":[";
    if let Some(scan) = j.scan() {
        let parts: Vec<String> = scan.largest.iter().map(|(k, v)| esc(Some(&format!("{k}={v}")))).collect();
        sb += &parts.join(",");
    }
    sb += "],\"failedItems\":[";
    let failed: Vec<String> = lock(&j.failed_items)
        .iter()
        .map(|f| esc(Some(&format!("{} :: {}", paths::to_short(&f.src), f.error().unwrap_or_default()))))
        .collect();
    sb += &failed.join(",");
    sb += "],\"notes\":[";
    let notes: Vec<String> = lock(&j.notes).iter().map(|x| esc(Some(x))).collect();
    sb += &notes.join(",");
    sb += &format!("],\"log\":{}", esc(j.log_path().map(|p| p.to_string_lossy().to_string()).as_deref()));
    sb += "}";
    sb
}

pub fn run(args: &[WString]) -> i32 {
    let mut rest: Vec<WString> = Vec::new(); // as they came (UTF-16: nothing lost on odd names)
    let (mut json, mut log): (Option<String>, Option<String>) = (None, None);
    let mut spec = JobSpec { origin: "CLI".into(), ..Default::default() };
    let (mut cancel_after, mut pause_at, mut pause_for) = (0.0, 0.0, 0.0);
    let mut skip_at: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].to_string_lossy().as_str() {
            "--verify" => spec.verify = true,
            "--resume" => spec.resume_mode = true,
            "--policy" => spec.policy = parse_policy(&arg(args, &mut i)),
            "--limit" => spec.speed_limit_mbps = double(&arg(args, &mut i)),
            "--json" => json = Some(arg(args, &mut i)),
            "--log" => log = Some(arg(args, &mut i)),
            "--cancel-after" => cancel_after = double(&arg(args, &mut i)),
            "--pause" => {
                let v = arg(args, &mut i);
                let p: Vec<&str> = v.split(',').collect();
                pause_at = double(p[0]);
                pause_for = double(p.get(1).unwrap_or_else(|| panic!("Index was outside the bounds of the array.")));
            }
            "--skip-at" => skip_at = Some(arg(args, &mut i)),
            _ => {
                if !engine_option(args, &mut i, &mut spec) {
                    rest.push(args[i].clone());
                }
            }
        }
        i += 1;
    }
    if rest.len() < 3 {
        eprint!("usage: --cli copy|move <src...> <dest>\r\n");
        return 1;
    }
    spec.op = if rest[0].eq_ignore_case_str("move") { OpKind::Move } else { OpKind::Copy };
    for r in &rest[1..rest.len() - 1] {
        spec.sources.push(paths::to_long(r));
    }
    spec.dest = paths::to_long(&rest[rest.len() - 1]);

    let mut job = CopyJob::new(Arc::new(spec));
    job.ask_conflict = Some(Box::new(|_, _| {
        Some(ConflictAnswer { choice: ConflictPolicy::Skip, apply_to_all: false, cancel_job: false })
    }));
    job.ask_low_space = Some(Box::new(|_, _, _| false));
    let job = Arc::new(job);
    journal::save(&job.spec);
    let sw = Instant::now();
    job.start();
    // StreamWriter(log, false, Encoding.UTF8) { AutoFlush = true }: a byte order mark first
    let mut lw = log.map(|l| {
        let mut f = std::fs::File::create(&l).unwrap_or_else(|e| panic!("{e}"));
        let _ = f.write_all(&[0xEF, 0xBB, 0xBF]);
        f
    });
    let (mut paused_once, mut skipped) = (false, false);
    while !job.is_finished() {
        sys::sleep_ms(50);
        let t = sw.elapsed().as_secs_f64();
        if cancel_after > 0.0 && t >= cancel_after && !job.is_canceled() {
            job.cancel();
        }
        if pause_at > 0.0 && !paused_once && t >= pause_at {
            job.pause();
            paused_once = true;
        }
        if paused_once && job.is_paused() && t >= pause_at + pause_for {
            job.resume();
        }
        if let Some(name) = &skip_at
            && !skipped
            && let Some(cur) = job.current_transfer()
            && cur.file().name().eq_ignore_case(&WString::from(name.as_str()))
            && cur.done.load(SEQ) > 0
        {
            job.skip_current();
            skipped = true;
        }
        if let Some(f) = lw.as_mut() {
            let cur = job.current_transfer();
            let line = format!(
                "{}s {} {}/{} files {}/{} bytes {} Mbps workers={} badge='{}' cur={} status={}\r\n",
                netfmt::f(t, 1),
                job.phase().name(),
                job.done_files.load(SEQ),
                job.total_files.load(SEQ),
                job.done_bytes.load(SEQ),
                job.total_bytes.load(SEQ),
                netfmt::f(job.speed_now() * 8.0 / 1e6, 1),
                job.active_count(),
                job.plan().map(|p| p.badge()).unwrap_or_default(),
                cur.map_or("-".to_string(), |c| c.file().name().to_string()),
                job.status().unwrap_or_default()
            );
            let _ = f.write_all(line.as_bytes());
        }
    }
    let secs = sw.elapsed().as_secs_f64();
    drop(lw);
    if job.phase() != JobPhase::Canceled {
        journal::remove(&job.spec);
    }
    if let Some(j) = json {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(report(&job, secs).as_bytes());
        std::fs::write(&j, bytes).unwrap_or_else(|e| panic!("{e}"));
    }
    match job.phase() {
        JobPhase::Canceled => 3,
        JobPhase::Failed => 1,
        _ => {
            if job.failed_files.load(SEQ) > 0 || job.skipped_files.load(SEQ) > 0 {
                2
            } else {
                0
            }
        }
    }
}
