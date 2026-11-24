//! Step 2's tests that need a whole job: the administrator copy's side of the hand-off (its record, progress, control
//! and result files), the job queue (JobManager), cleaning up after administrator copies whose Choppy is gone, and the
//! tuner's choices. They run in test mode (CHOPPY_DATA in a scratch folder), never in the user's own data folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::time::{Duration, Instant};

use super::big_copy::{PipeTuner, PipeTuning};
use super::copy_job::CopyJob;
use super::handoff;
use super::model::{JobPhase, JobSpec, OpKind};
use super::settings::app_paths;
use crate::job_manager::JobManager;
use crate::sys;
use crate::wstr::WString;

/// One test at a time may switch the process to test mode (the settings test reads the registry otherwise).
pub static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn long(p: &Path) -> WString {
    super::paths::to_long(&WString::from_os(p.as_os_str()))
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("choppy-step2-test-{name}-{}", super::model::new_id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn wait_done(job: &Arc<CopyJob>) {
    let t = Instant::now();
    while !job.is_finished() {
        assert!(t.elapsed() < Duration::from_secs(60), "the job didn't end");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn source(dir: &Path, n: usize, size: usize) -> PathBuf {
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    for i in 0..n {
        std::fs::write(src.join(format!("f{i}.bin")), vec![i as u8; size]).unwrap();
    }
    src
}

fn read(p: &Path) -> String {
    let b = std::fs::read(p).unwrap();
    assert_eq!(&b[..3], &[0xEF, 0xBB, 0xBF], "UTF-8 with a byte order mark: {}", p.display());
    String::from_utf8(b[3..].to_vec()).unwrap()
}

fn with_ext(p: &Path, ext: &str) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(ext);
    PathBuf::from(s)
}

#[test]
fn admin_side_queue_and_stale_records() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let data = scratch("data");
    sys::set_env_var("CHOPPY_DATA", &data.to_string_lossy());
    assert!(app_paths::test_mode());
    let work = scratch("work");

    // ---- the administrator's side: its record, checked by its SHA-256, the progress and the result ----
    let src = source(&work, 3, 1000);
    let dst = work.join("dst");
    let spec = JobSpec { op: OpKind::Copy, sources: vec![long(&src)], dest: long(&dst), origin: "Administrator".into(), ..Default::default() };
    let record = handoff::save(&spec).unwrap();
    assert_eq!(record, app_paths::job_dir().join(format!("{}.admin", spec.id)));
    let text = std::fs::read_to_string(&record).unwrap();
    assert!(text.contains(&format!("parent={}\r\n", sys::current_pid())), "{text}");
    assert!(text.contains(&format!("testdata={}\r\n", data.to_string_lossy())), "{text}");
    let fp: String = handoff::sha256(&std::fs::read(&record).unwrap()).iter().map(|b| format!("{b:02x}")).collect();
    // only the record the user's Choppy wrote runs: a wrong or missing fingerprint loads nothing
    assert!(handoff::load(&record, Some("00")).is_none());
    assert!(handoff::load(&record, None).is_none());
    let s = handoff::load(&record, Some(&format!(" {} ", fp.to_uppercase()))).expect("the genuine record");
    assert!(s.elevated && s.handoff_path.is_some());
    let job = Arc::new(CopyJob::new(Arc::new(s)));
    handoff::report(&job);
    job.start();
    wait_done(&job);
    std::thread::sleep(Duration::from_millis(300));
    let res = read(&with_ext(&record, ".result"));
    assert!(res.starts_with("phase=Done\r\n"), "{res}");
    assert!(res.contains("doneFiles=3\r\n") && res.contains("totalBytes=3000\r\n") && res.ends_with("end\r\n"), "{res}");
    assert!(res.contains("log="), "an administrator copy always leaves its log: {res}");
    let r = handoff::read_result(&record).expect("a whole result");
    assert_eq!((r.phase.as_deref(), r.done_files, r.total_bytes, r.copied_bytes), (Some("Done"), 3, 3000, 3000));
    let pr = handoff::read_progress(&record).expect("progress");
    assert_eq!(pr.get("pid").map(|s| s.as_str()), Some(sys::current_pid().to_string().as_str()));
    assert_eq!(pr.get("donefiles").map(|s| s.as_str()), Some("3"));
    for i in 0..3 {
        assert_eq!(std::fs::read(dst.join("src").join(format!("f{i}.bin"))).unwrap(), vec![i as u8; 1000]);
    }
    handoff::clean(&record, true);
    assert!(!record.exists() && !with_ext(&record, ".result").exists() && !with_ext(&record, ".progress").exists());

    // the user's Choppy pauses, runs and cancels it through the control file (the newest number wins)
    let src2 = source(&work.join("two"), 20, 1 << 20);
    let spec2 = JobSpec {
        op: OpKind::Copy,
        sources: vec![long(&src2)],
        dest: long(&work.join("dst2")),
        speed_limit_mbps: 16.0, // 2 MB/s: long enough to steer
        ..Default::default()
    };
    let record2 = handoff::save(&spec2).unwrap();
    let fp2: String = handoff::sha256(&std::fs::read(&record2).unwrap()).iter().map(|b| format!("{b:02x}")).collect();
    let job2 = Arc::new(CopyJob::new(Arc::new(handoff::load(&record2, Some(&fp2)).unwrap())));
    handoff::report(&job2);
    job2.start();
    std::thread::sleep(Duration::from_millis(600));
    handoff::control(&record2, 1, "pause");
    let t = Instant::now();
    while !job2.is_paused() {
        assert!(t.elapsed() < Duration::from_secs(5), "pause didn't arrive");
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(handoff::read_progress(&record2).unwrap().get("paused").map(|s| s.as_str()), Some("1"));
    handoff::control(&record2, 2, "run");
    let t = Instant::now();
    while job2.is_paused() {
        assert!(t.elapsed() < Duration::from_secs(5), "run didn't arrive");
        std::thread::sleep(Duration::from_millis(20));
    }
    handoff::control(&record2, 1, "pause"); // an older number: ignored
    std::thread::sleep(Duration::from_millis(500));
    assert!(!job2.is_paused());
    handoff::control(&record2, 3, "cancel");
    wait_done(&job2);
    assert_eq!(job2.phase(), JobPhase::Canceled);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(handoff::read_result(&record2).unwrap().phase.as_deref(), Some("Canceled"));
    // nothing half written is left
    for e in std::fs::read_dir(work.join("dst2").join("src")).unwrap() {
        let p = e.unwrap().path();
        assert!(!p.to_string_lossy().ends_with(".chpart"), "{}", p.display());
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 1 << 20);
    }
    handoff::clean(&record2, true);

    // ---- administrator copies whose Choppy is gone: their files go; a live one's stay ----
    let jobs = app_paths::job_dir();
    let gone = jobs.join("gone.admin");
    let mine = jobs.join("mine.admin");
    let noparent = jobs.join("noparent.admin");
    std::fs::write(&gone, "op=Copy\r\nparent=2147483000\r\n").unwrap();
    std::fs::write(with_ext(&gone, ".progress"), "pid=1").unwrap();
    std::fs::write(&mine, format!("op=Copy\r\nparent={}\r\n", sys::current_pid())).unwrap();
    std::fs::write(&noparent, "op=Copy\r\n").unwrap();
    handoff::clean_stale();
    assert!(!gone.exists() && !with_ext(&gone, ".progress").exists() && !noparent.exists());
    assert!(mine.exists());
    std::fs::remove_file(&mine).unwrap();

    // ---- the queue: one job at a time in the order they came, records removed when they end ----
    let mgr = Arc::new(JobManager::default());
    let changes = Arc::new(AtomicI32::new(0));
    let c = changes.clone();
    mgr.on_changed(Arc::new(move || {
        c.fetch_add(1, Ordering::SeqCst);
    }));
    let a = source(&work.join("qa"), 5, 200_000);
    let b = source(&work.join("qb"), 5, 200_000);
    let qa = JobSpec { op: OpKind::Copy, sources: vec![long(&a)], dest: long(&work.join("qdst")), ..Default::default() };
    let qb = JobSpec { op: OpKind::Copy, sources: vec![long(&b)], dest: long(&work.join("qdst2")), ..Default::default() };
    let (ida, idb) = (qa.id.clone(), qb.id.clone());
    mgr.enqueue(qa);
    mgr.enqueue(qb);
    let t = Instant::now();
    while mgr.busy() || mgr.finished_snapshot().len() < 2 {
        assert!(t.elapsed() < Duration::from_secs(60), "the queue didn't empty");
        std::thread::sleep(Duration::from_millis(20));
    }
    let fin = mgr.finished_snapshot();
    assert_eq!((fin[0].spec.id.clone(), fin[1].spec.id.clone()), (idb.clone(), ida.clone()), "newest first");
    // the second started only after the first ended
    assert!(fin[1].finished_at() <= fin[0].started_at());
    assert!(fin.iter().all(|j| j.phase() == JobPhase::Done && j.done_files.load(Ordering::SeqCst) == 5));
    std::thread::sleep(Duration::from_millis(300));
    assert!(!jobs.join(format!("{ida}.job")).exists() && !jobs.join(format!("{idb}.job")).exists());
    assert!(changes.load(Ordering::SeqCst) > 0);

    sys::remove_env_var("CHOPPY_DATA");
    let _ = std::fs::remove_dir_all(&work);
    let _ = std::fs::remove_dir_all(&data);
}

#[test]
fn tuner_warms_up_tries_each_way_and_keeps_a_clear_winner() {
    let start = PipeTuning { chunk: 8 << 20, slots: 4, ..Default::default() };
    let other = PipeTuning { chunk: 32 << 20, slots: 4, ..Default::default() };
    let mb = 1i64 << 20;
    let mut t = PipeTuner::new(start.clone(), vec![other.clone()], 0.75, 1.03);
    // warm-up: 64 MB of the starting way, then trials sized to 0.75 s at that speed (whole 32 MB units, 64-512 MB)
    let (way, end, measured) = t.next(0, 4096 * mb, 0);
    assert_eq!((way, end, measured), (start.clone(), 64 * mb, false));
    t.report(64 * mb, 0.1, false); // 640 MB/s: 480 MB -> 480 MB trials
    let (way, end, measured) = t.next(64 * mb, 4096 * mb, 0);
    assert_eq!((way, end - 64 * mb, measured), (start.clone(), 480 * mb, true));
    t.report(480 * mb, 0.05, true); // too short to tell: tried again
    assert!(!t.settled());
    t.report(480 * mb, 1.0, true);
    let (way, _, measured) = t.next(1024 * mb, 4096 * mb, 0);
    assert_eq!((way, measured), (other.clone(), true));
    // 2% faster is noise: the starting way stays
    t.report(480 * mb, 1.0 / 1.02, true);
    assert!(t.settled() && !t.changed && t.best() == &start);
    assert_eq!(t.summary(), "8x4 480 MB/s <- | 32x4 490 MB/s");
    // a clear winner takes over
    let mut t = PipeTuner::new(start.clone(), vec![other.clone()], 0.75, 1.03);
    t.report(64 * mb, 0.1, false);
    t.report(480 * mb, 1.0, true);
    t.report(480 * mb, 0.5, true);
    assert!(t.settled() && t.changed && t.best() == &other);
    // no room in this file for a trial (the cached end stays out of it): the next big file tries
    let t = PipeTuner::new(start.clone(), vec![other], 0.75, 1.03);
    assert_eq!(t.next(0, 100 * mb, 10 * mb), (start, 100 * mb, false));
}
