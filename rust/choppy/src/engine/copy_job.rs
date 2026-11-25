//! One copy or move (CopyJob.cs). Never loses a file:
//!  - every file is written to "<name>.chpart" and renamed into place only when complete (and verified, if on),
//!  - a move deletes a source file only after its copy is in place, on the disk (flushed) and verified, if on - and
//!    only if the original didn't change meanwhile,
//!  - errors are retried with backoff; a network drop waits for the network to come back.
//!
//! C# changed the job's fields from any thread (workers, the monitor, the flusher, the UI); here they are atomics or
//! sit behind locks. Every thread the job starts catches panics: a panic in a file's copy fails that file (C#'s
//! "Unexpected error"), and nothing is renamed into place or deleted after one.

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::big_copy::{self, PipeTuner, PipeTuning};
use super::handoff;
use super::model::{
    ConflictAnswer, ConflictInfo, ConflictPolicy, DirItem, FileItem, FileState, JobPhase, JobSpec, OpKind, ScanResult,
    file_times,
};
use super::plan::Plan;
use super::scanner::Scanner;
use super::settings::app_paths;
use super::volume_probe::{self, DeviceKind, VolumeInfo};
use super::xxhash64::{self, XxHash64};
use super::{Stopwatch, lock, panic_message, paths, wait_timeout};
use crate::sys;
use crate::wstr::{WString, w};

/// A file being written is named "<name>.chpart" until it is complete.
pub const TEMP_SUFFIX: &str = ".chpart";

const SEQ: Ordering = Ordering::SeqCst;

/// One file in flight (for the "current file" card, Skip and progress).
pub struct Transfer {
    pub file: Mutex<Arc<FileItem>>,
    pub done: AtomicI64,          // bytes copied of this file
    pub skip: AtomicBool,
    pub verifying: AtomicBool,
    pub verify_done: AtomicI64,   // bytes hashed (source + copy)
    pub started: Instant,
    pub times_set: AtomicBool,    // creation date already set through the copy's own handle
    pub rename_to: Mutex<Option<WString>>, // plain final name: rename through the copy's handle once every byte is written
    pub renamed: AtomicBool,
    pub has_src_hash: AtomicBool, // the pipeline hashed the source as it read it: verify reads only the copy
    pub src_hash: AtomicU64,
    pub durable: AtomicBool,      // the copy was flushed to the disk itself (a move may delete the original at once)
}

impl Transfer {
    fn new(file: Arc<FileItem>) -> Transfer {
        Transfer {
            file: Mutex::new(file),
            done: AtomicI64::new(0),
            skip: AtomicBool::new(false),
            verifying: AtomicBool::new(false),
            verify_done: AtomicI64::new(0),
            started: Instant::now(),
            times_set: AtomicBool::new(false),
            rename_to: Mutex::new(None),
            renamed: AtomicBool::new(false),
            has_src_hash: AtomicBool::new(false),
            src_hash: AtomicU64::new(0),
            durable: AtomicBool::new(false),
        }
    }
    pub fn file(&self) -> Arc<FileItem> {
        lock(&self.file).clone()
    }
}

/// Speed history (bytes/s), one sample per 250 ms.
#[derive(Default)]
pub struct Speed {
    pub history: Vec<f64>, // last 600 samples (2.5 min)
    pub samples: i64,      // samples taken so far - keeps counting when the history is full
    pub now: f64,
    pub smooth: f64,
    pub peak: f64,
    pub second: f64, // average of the last second (4 samples): the "now" number, less jumpy
}

/// ManualResetEventSlim: open or closed; waiters wait until it opens (or for a while).
struct Gate {
    open: Mutex<bool>,
    cv: Condvar,
}

impl Gate {
    fn new(open: bool) -> Gate {
        Gate { open: Mutex::new(open), cv: Condvar::new() }
    }
    fn set(&self) {
        *lock(&self.open) = true;
        self.cv.notify_all();
    }
    fn reset(&self) {
        *lock(&self.open) = false;
    }
    fn wait(&self, d: Duration) {
        let g = lock(&self.open);
        if !*g {
            drop(wait_timeout(&self.cv, g, d));
        }
    }
    /// Wait(): until it opens.
    fn wait_open(&self) {
        let mut g = lock(&self.open);
        while !*g {
            g = super::wait(&self.cv, g);
        }
    }
}

/// Small files read ahead into memory: where each one sits in the batch buffer.
struct SmallBatch {
    mem: sys::Memory,
    cap: i64,
    end: usize,                // index after the batch's last file
    at: HashMap<usize, usize>, // file (its address) -> where its bytes start
}

#[derive(Default)]
struct FlushState {
    to_flush: Vec<Arc<FileItem>>,
    flusher: Option<JoinHandle<()>>,
    end: bool,
    dead: bool, // the flusher ended on a panic: the copies are flushed (then their originals go) where they finish
    bytes: i64,
    oldest: Option<Instant>,
}

#[derive(Default)]
struct Dirs {
    known: HashSet<WString>,   // destination folders that exist (upper case: OrdinalIgnoreCase)
    created: HashSet<WString>, // the ones this copy made
}

/// The admin hand-off can't happen (a rights problem or the user said no): why, to note.
struct JobError(String);

pub type AskConflict = Box<dyn Fn(&Arc<CopyJob>, &ConflictInfo) -> Option<ConflictAnswer> + Send + Sync>;
pub type AskLowSpace = Box<dyn Fn(&Arc<CopyJob>, i64, i64) -> bool + Send + Sync>;
pub type AskElevate = Box<dyn Fn(&Arc<CopyJob>) -> bool + Send + Sync>;
pub type JobEvent = Arc<dyn Fn(&Arc<CopyJob>) + Send + Sync>;

/// A copy or move job.
pub struct CopyJob {
    pub spec: Arc<JobSpec>,
    policy: Mutex<ConflictPolicy>, // Spec.Policy ("Apply to all" changes it)
    plan: Mutex<Option<Plan>>,
    src_vol: OnceLock<Option<VolumeInfo>>,
    dst_vol: OnceLock<Option<VolumeInfo>>,
    scan: OnceLock<ScanResult>,
    scanner: Mutex<Option<Arc<Scanner>>>,

    // ---- live state (read by the UI) ----
    phase: AtomicU8,
    status: Mutex<Option<String>>,     // extra line: "Waiting for the network...", "The destination is full..."
    retry_file: Mutex<Option<WString>>, // a file waiting to be tried again (in use, network hiccup)
    retry_why: Mutex<Option<String>>,
    pub total_bytes: AtomicI64,
    pub done_bytes: AtomicI64,
    pub skipped_bytes: AtomicI64,
    pub failed_bytes: AtomicI64,
    pub total_files: AtomicI32,
    pub done_files: AtomicI32,
    pub skipped_files: AtomicI32,
    pub failed_files: AtomicI32,
    pub verified_files: AtomicI32,
    pub already_there_files: AtomicI32,
    pub conflicts_left: AtomicI32,
    pub copied_bytes: AtomicI64, // bytes of files that finished
    pub clock: Stopwatch,        // running time (stops while paused)
    pub copy_clock: Stopwatch,   // copying time only (no analysis, no pauses): the average speed
    started_at: AtomicI64,       // FILETIME (UTC); shown in local time
    finished_at: AtomicI64,
    pub failed_items: Mutex<Vec<Arc<FileItem>>>,
    pub skipped_items: Mutex<Vec<Arc<FileItem>>>,
    pub notes: Mutex<Vec<String>>,
    log_path: Mutex<Option<PathBuf>>,
    verify: AtomicBool, // effective for this job (the setting; can be switched live)
    /// C#'s Spec.Verify, which the copy window's quick toggle changes too (the job takes it when it starts).
    spec_verify: AtomicBool,
    pub speed: Mutex<Speed>,

    // ---- callbacks from the UI (set before Start) ----
    pub ask_conflict: Option<AskConflict>,
    pub ask_low_space: Option<AskLowSpace>, // (need, free) -> continue?
    pub ask_elevate: Option<AskElevate>,    // the destination needs administrator rights: copy the rest as administrator?
    as_admin: AtomicBool,                   // an administrator copy is doing the rest (in its own window)
    pub admin_files: AtomicI32,             // files the administrator copy did
    finished: Mutex<Vec<JobEvent>>,
    phase_changed: Mutex<Vec<JobEvent>>,

    // ---- control ----
    canceled: Arc<AtomicBool>,
    paused: AtomicBool,
    run_gate: Gate,
    conflict_lock: Mutex<()>,
    active: Mutex<Vec<Arc<Transfer>>>,
    dirs: Mutex<Dirs>,
    waiting_net: AtomicBool,
    fast_path_off: AtomicBool,
    fast_checks: AtomicI32,
    files: OnceLock<Vec<Arc<FileItem>>>,
    next_index: AtomicI32,
    active_workers: AtomicI32,
    big_in_flight: Mutex<i32>,
    big_cv: Condvar,
    pending_big: AtomicI32,
    pending_small: AtomicI32,
    started: AtomicBool,
    limit_bps: AtomicU64, // f64 bits
    tokens: Mutex<(f64, f64)>, // (tokens, when they were counted)
    token_clock: Instant,
    elev_state: Mutex<i32>, // 0 = not asked yet, 1 = yes (hand over), 2 = no
    hand_off: AtomicBool,   // the rest goes to the administrator copy: no new files here
    batch: Mutex<Option<SmallBatch>>,
    pipe_off_at_dest: AtomicBool, // the destination folder is compressed or encrypted: the pipeline would fight it
    pipe_tuner: Mutex<Option<PipeTuner>>, // finds the fastest way for big files on this copy (see PipeTuner)
    tune_summary: Mutex<Option<String>>,  // what it measured, as of the last big file (read without waiting for one)
    flush: Mutex<FlushState>,
    flush_cv: Condvar,
    job_dests: Mutex<Option<HashSet<WString>>>,
    worker_panic: Mutex<Option<Box<dyn std::any::Any + Send>>>,
}

fn phase_of(v: u8) -> JobPhase {
    match v {
        1 => JobPhase::Analyzing,
        2 => JobPhase::Copying,
        3 => JobPhase::Waiting,
        4 => JobPhase::Finishing,
        5 => JobPhase::Done,
        6 => JobPhase::Canceled,
        7 => JobPhase::Failed,
        _ => JobPhase::Queued,
    }
}

fn phase_index(p: JobPhase) -> u8 {
    match p {
        JobPhase::Queued => 0,
        JobPhase::Analyzing => 1,
        JobPhase::Copying => 2,
        JobPhase::Waiting => 3,
        JobPhase::Finishing => 4,
        JobPhase::Done => 5,
        JobPhase::Canceled => 6,
        JobPhase::Failed => 7,
    }
}

fn add(a: &AtomicI64, v: i64) {
    a.fetch_add(v, SEQ);
}

fn upper(p: &WString) -> WString {
    p.to_upper_invariant()
}

/// A thread of the job; a panic in it is kept for whoever joins it.
fn spawn(name: String, stack: Option<usize>, f: impl FnOnce() + Send + 'static) -> JoinHandle<()> {
    let mut b = std::thread::Builder::new().name(name);
    if let Some(s) = stack {
        b = b.stack_size(s);
    }
    b.spawn(f).unwrap_or_else(|_| panic!("{}", big_copy::OUT_OF_MEMORY))
}

impl CopyJob {
    pub fn new(spec: Arc<JobSpec>) -> CopyJob {
        let limit = if spec.speed_limit_mbps > 0.0 { spec.speed_limit_mbps * 1000000.0 / 8.0 } else { 0.0 };
        CopyJob {
            policy: Mutex::new(spec.policy),
            plan: Mutex::new(None),
            src_vol: OnceLock::new(),
            dst_vol: OnceLock::new(),
            scan: OnceLock::new(),
            scanner: Mutex::new(None),
            phase: AtomicU8::new(phase_index(JobPhase::Queued)),
            status: Mutex::new(None),
            retry_file: Mutex::new(None),
            retry_why: Mutex::new(None),
            total_bytes: AtomicI64::new(0),
            done_bytes: AtomicI64::new(0),
            skipped_bytes: AtomicI64::new(0),
            failed_bytes: AtomicI64::new(0),
            total_files: AtomicI32::new(0),
            done_files: AtomicI32::new(0),
            skipped_files: AtomicI32::new(0),
            failed_files: AtomicI32::new(0),
            verified_files: AtomicI32::new(0),
            already_there_files: AtomicI32::new(0),
            conflicts_left: AtomicI32::new(0),
            copied_bytes: AtomicI64::new(0),
            clock: Stopwatch::default(),
            copy_clock: Stopwatch::default(),
            started_at: AtomicI64::new(0),
            finished_at: AtomicI64::new(0),
            failed_items: Mutex::new(Vec::new()),
            skipped_items: Mutex::new(Vec::new()),
            notes: Mutex::new(Vec::new()),
            log_path: Mutex::new(None),
            verify: AtomicBool::new(false),
            spec_verify: AtomicBool::new(spec.verify),
            speed: Mutex::new(Speed::default()),
            ask_conflict: None,
            ask_low_space: None,
            ask_elevate: None,
            as_admin: AtomicBool::new(false),
            admin_files: AtomicI32::new(0),
            finished: Mutex::new(Vec::new()),
            phase_changed: Mutex::new(Vec::new()),
            canceled: Arc::new(AtomicBool::new(false)),
            paused: AtomicBool::new(false),
            run_gate: Gate::new(true),
            conflict_lock: Mutex::new(()),
            active: Mutex::new(Vec::new()),
            dirs: Mutex::new(Dirs::default()),
            waiting_net: AtomicBool::new(false),
            fast_path_off: AtomicBool::new(false),
            fast_checks: AtomicI32::new(0),
            files: OnceLock::new(),
            next_index: AtomicI32::new(0),
            active_workers: AtomicI32::new(0),
            big_in_flight: Mutex::new(0),
            big_cv: Condvar::new(),
            pending_big: AtomicI32::new(0),
            pending_small: AtomicI32::new(0),
            started: AtomicBool::new(false),
            limit_bps: AtomicU64::new(limit.to_bits()),
            tokens: Mutex::new((0.0, 0.0)),
            token_clock: Instant::now(),
            elev_state: Mutex::new(0),
            hand_off: AtomicBool::new(false),
            batch: Mutex::new(None),
            pipe_off_at_dest: AtomicBool::new(false),
            pipe_tuner: Mutex::new(None),
            tune_summary: Mutex::new(None),
            flush: Mutex::new(FlushState::default()),
            flush_cv: Condvar::new(),
            job_dests: Mutex::new(None),
            worker_panic: Mutex::new(None),
            spec,
        }
    }

    // ================= what the UI reads =================

    pub fn phase(&self) -> JobPhase {
        phase_of(self.phase.load(SEQ))
    }
    pub fn status(&self) -> Option<String> {
        lock(&self.status).clone()
    }
    fn set_status(&self, s: Option<String>) {
        *lock(&self.status) = s;
    }
    pub fn retry_file(&self) -> Option<WString> {
        lock(&self.retry_file).clone()
    }
    pub fn retry_why(&self) -> Option<String> {
        lock(&self.retry_why).clone()
    }
    /// The plan Adaptive picked (None before the drives were looked at).
    pub fn plan(&self) -> Option<Plan> {
        lock(&self.plan).clone()
    }
    /// The plan, to change it (C# changed Plan's fields in place).
    fn plan_mut(&self) -> MutexGuard<'_, Option<Plan>> {
        lock(&self.plan)
    }
    fn plan_get<R>(&self, f: impl FnOnce(&Plan) -> R) -> R {
        f(lock(&self.plan).as_ref().expect("the plan"))
    }
    pub fn src_vol(&self) -> Option<&VolumeInfo> {
        self.src_vol.get().and_then(|v| v.as_ref())
    }
    pub fn dst_vol(&self) -> Option<&VolumeInfo> {
        self.dst_vol.get().and_then(|v| v.as_ref())
    }
    pub fn scan(&self) -> Option<&ScanResult> {
        self.scan.get()
    }
    pub fn scanner(&self) -> Option<Arc<Scanner>> {
        lock(&self.scanner).clone()
    }
    pub fn policy(&self) -> ConflictPolicy {
        *lock(&self.policy)
    }
    pub fn verify(&self) -> bool {
        self.verify.load(SEQ)
    }
    /// The setting can be switched while the copy runs.
    pub fn set_verify(&self, on: bool) {
        self.verify.store(on, SEQ);
    }
    /// Spec.Verify: what the job takes when it starts (the copy window sets it with Verify).
    pub fn spec_verify(&self) -> bool {
        self.spec_verify.load(SEQ)
    }
    pub fn set_spec_verify(&self, on: bool) {
        self.spec_verify.store(on, SEQ);
    }
    pub fn as_admin(&self) -> bool {
        self.as_admin.load(SEQ)
    }
    pub fn log_path(&self) -> Option<PathBuf> {
        lock(&self.log_path).clone()
    }
    pub fn started_at(&self) -> i64 {
        self.started_at.load(SEQ)
    }
    pub fn finished_at(&self) -> i64 {
        self.finished_at.load(SEQ)
    }
    pub fn speed_now(&self) -> f64 {
        lock(&self.speed).now
    }
    pub fn speed_peak(&self) -> f64 {
        lock(&self.speed).peak
    }

    /// Average speed of this copy so far (bytes/s). Steadier than the live speed: the time left uses it.
    pub fn average_speed(&self) -> f64 {
        let s = self.copy_clock.elapsed().as_secs_f64();
        if s < 0.5 {
            return 0.0;
        }
        self.done_bytes.load(SEQ).max(0) as f64 / s
    }

    /// What the tuner measured on this copy (log, tests).
    pub fn tune_summary(&self) -> Option<String> {
        lock(&self.tune_summary).clone()
    }

    pub fn is_canceled(&self) -> bool {
        self.canceled.load(SEQ)
    }
    pub fn is_paused(&self) -> bool {
        self.paused.load(SEQ)
    }
    pub fn is_running(&self) -> bool {
        self.started.load(SEQ)
            && matches!(self.phase(), JobPhase::Analyzing | JobPhase::Copying | JobPhase::Waiting | JobPhase::Finishing)
    }
    pub fn is_finished(&self) -> bool {
        matches!(self.phase(), JobPhase::Done | JobPhase::Canceled | JobPhase::Failed)
    }

    /// Runs `f` once the job ends (C#'s Finished event).
    pub fn on_finished(&self, f: JobEvent) {
        lock(&self.finished).push(f);
    }
    /// Runs `f` whenever the phase or the badge changes (C#'s PhaseChanged event).
    pub fn on_phase_changed(&self, f: JobEvent) {
        lock(&self.phase_changed).push(f);
    }

    fn canceled(&self) -> bool {
        self.canceled.load(SEQ)
    }
    fn paused(&self) -> bool {
        self.paused.load(SEQ)
    }
    fn files(&self) -> &[Arc<FileItem>] {
        self.files.get().map_or(&[], |v| v.as_slice())
    }
    fn note(&self, n: String) {
        lock(&self.notes).push(n);
    }
    fn limit(&self) -> f64 {
        f64::from_bits(self.limit_bps.load(SEQ))
    }

    // ================= control =================

    pub fn start(self: &Arc<Self>) {
        if self.started.swap(true, SEQ) {
            return;
        }
        let me = self.clone();
        spawn(format!("CopyJob {}", self.spec.id), None, move || me.run());
    }

    pub fn pause(self: &Arc<Self>) {
        if self.paused() || self.is_finished() {
            return;
        }
        self.paused.store(true, SEQ);
        self.run_gate.reset();
        self.clock.stop();
        self.copy_clock.stop();
        self.raise_phase();
    }

    pub fn resume(self: &Arc<Self>) {
        if !self.paused() {
            return;
        }
        self.paused.store(false, SEQ);
        if self.status().is_some_and(|s| s.starts_with("The destination is full")) {
            self.set_status(None);
        }
        if !self.is_finished() {
            self.clock.start();
        }
        if matches!(self.phase(), JobPhase::Copying | JobPhase::Waiting) {
            self.copy_clock.start();
        }
        self.run_gate.set();
        self.raise_phase();
    }

    pub fn cancel(self: &Arc<Self>) {
        self.canceled.store(true, SEQ);
        self.run_gate.set();
        if !self.started.load(SEQ) {
            self.phase.store(phase_index(JobPhase::Canceled), SEQ);
            self.raise_phase();
        }
    }

    pub fn set_speed_limit(&self, mbps: f64) {
        let v = if mbps > 0.0 { mbps * 1000000.0 / 8.0 } else { 0.0 };
        self.limit_bps.store(v.to_bits(), SEQ);
    }

    /// Skips the file shown as "current file".
    pub fn skip_current(&self) {
        if let Some(t) = self.current_transfer() {
            t.skip.store(true, SEQ);
        }
    }

    /// The biggest file in flight (the first one of that size).
    pub fn current_transfer(&self) -> Option<Arc<Transfer>> {
        let a = lock(&self.active);
        let mut best: Option<&Arc<Transfer>> = None;
        for t in a.iter() {
            if best.is_none_or(|b| t.file().size() > b.file().size()) {
                best = Some(t);
            }
        }
        best.cloned()
    }

    pub fn active_count(&self) -> usize {
        lock(&self.active).len()
    }

    fn raise_phase(self: &Arc<Self>) {
        let hs: Vec<JobEvent> = lock(&self.phase_changed).clone();
        for h in hs {
            let _ = catch_unwind(AssertUnwindSafe(|| h(self)));
        }
    }

    fn set_phase(self: &Arc<Self>, p: JobPhase) {
        self.phase.store(phase_index(p), SEQ);
        self.raise_phase();
    }

    // ================= main =================

    fn run(self: &Arc<Self>) {
        self.started_at.store(sys::now_file_time(), SEQ);
        self.clock.start();
        let r = catch_unwind(AssertUnwindSafe(|| self.run_job()));
        match r {
            Ok(Ok(())) => {}
            Ok(Err(JobError(m))) => {
                self.note(m.clone());
                self.set_status(Some(m));
                self.set_phase(if self.canceled() { JobPhase::Canceled } else { JobPhase::Failed });
            }
            Err(e) => {
                let m = panic_message(&*e);
                self.note(format!("Unexpected error: {m}"));
                self.set_status(Some(format!("Unexpected error: {m}")));
                self.set_phase(JobPhase::Failed);
            }
        }
        self.clock.stop();
        self.copy_clock.stop();
        self.finished_at.store(sys::now_file_time(), SEQ);
        self.write_log();
        let hs: Vec<JobEvent> = lock(&self.finished).clone();
        for h in hs {
            let _ = catch_unwind(AssertUnwindSafe(|| h(self)));
        }
    }

    fn run_job(self: &Arc<Self>) -> Result<(), JobError> {
        self.verify.store(self.spec_verify.load(SEQ), SEQ);
        self.set_phase(JobPhase::Analyzing);
        let me = self.clone();
        spawn("CopyJob monitor".into(), None, move || {
            let _ = catch_unwind(AssertUnwindSafe(|| me.monitor_loop()));
        });
        // its record is on the disk before any file is touched (a crash can then be resumed)
        if let Some(rec) = &self.spec.recorded {
            rec.wait(Duration::from_millis(15000));
        }

        let _ = self.src_vol.set(self.spec.sources.first().map(volume_probe::probe));
        let mut all_as_admin = false;
        if !paths::dir_exists(&self.spec.dest) {
            let e = self.create_dir_tree(&self.spec.dest);
            // the destination folder can't even be made without administrator rights: the whole copy goes there
            if e == sys::ERROR_ACCESS_DENIED && self.ask_to_elevate() {
                all_as_admin = true;
            } else if e != 0 {
                return Err(JobError(format!(
                    "Can't open the destination folder {}: {}",
                    paths::to_short(&self.spec.dest),
                    sys::error_text(e)
                )));
            }
        }
        let _ = self.dst_vol.set(Some(volume_probe::probe(&self.spec.dest)));

        *self.plan_mut() = Some(Plan::choose(&self.spec, self.src_vol(), self.dst_vol(), None));
        let instant = self.plan_get(|p| p.instant_move);
        if all_as_admin && !self.canceled() {
            self.total_files.store(self.spec.sources.len() as i32, SEQ);
            self.set_phase(JobPhase::Copying);
            if !self.paused() {
                self.copy_clock.start();
            }
            let items: Option<Vec<Arc<FileItem>>> =
                self.spec.explicit_items.as_ref().map(|v| v.iter().map(|x| Arc::new(copy_item(x))).collect());
            self.run_elevated(items, self.spec.explicit_dirs.clone(), Some(self.spec.sources.clone()));
            self.copy_clock.stop();
        } else if instant && !self.canceled() {
            self.run_instant_move();
        } else if !self.canceled() {
            let explicit: Option<Vec<FileItem>> = self.spec.explicit_items.as_ref().map(|v| v.iter().map(copy_item).collect());
            self.run_copy(self.spec.sources.clone(), explicit);
        }
        if self.canceled() {
            self.set_phase(JobPhase::Canceled);
        } else {
            self.set_phase(JobPhase::Done);
        }
        Ok(())
    }

    // ================= instant move (same drive) =================

    fn run_instant_move(self: &Arc<Self>) {
        self.set_phase(JobPhase::Copying);
        self.total_files.store(self.spec.sources.len() as i32, SEQ);
        let mut fallback: Vec<WString> = Vec::new();
        let mut as_admin: Vec<WString> = Vec::new();
        for src in &self.spec.sources {
            if self.canceled() {
                return;
            }
            self.run_gate.wait_open();
            if self.hand_off.load(SEQ) {
                as_admin.push(src.clone());
                continue;
            }
            let name = paths::file_name(src);
            let target = paths::combine(&self.spec.dest, &name);
            let a = match sys::get_file_attributes_err(src) {
                Ok(a) => a,
                Err(e) => {
                    self.fail_note(src, sys::error_text(e));
                    continue;
                }
            };
            let is_dir = (a & sys::FILE_ATTRIBUTE_DIRECTORY) != 0;
            if paths::same_path(src, &target) {
                self.note(format!("Already in this folder: {}", paths::to_short(src)));
                self.skipped_files.fetch_add(1, SEQ);
                continue;
            }
            if is_dir && paths::is_same_or_inside(&self.spec.dest, src) {
                self.fail_note(src, "A folder can't be moved into itself.".into());
                continue;
            }
            if self.move_item(src, &target, is_dir, &mut fallback, true) {
                self.done_files.fetch_add(1, SEQ);
            } else {
                as_admin.push(src.clone());
            }
        }
        if !as_admin.is_empty() && !self.canceled() {
            // the rest is renamed in place by an administrator copy
            self.run_elevated(None, None, Some(as_admin));
        }
        if !fallback.is_empty() && !self.canceled() {
            // something on this "same drive" refused a rename (another volume mounted inside): copy + delete instead
            *self.plan_mut() = Some(Plan::choose(&JobSpec::new(OpKind::Copy), self.src_vol(), self.dst_vol(), None));
            self.run_copy(fallback, None);
        }
    }

    /// False = the destination needs administrator rights and the user said yes: an administrator copy moves it.
    fn move_item(self: &Arc<Self>, src: &WString, target: &WString, is_dir: bool, fallback: &mut Vec<WString>, top: bool) -> bool {
        if self.canceled() {
            return true;
        }
        let ta = sys::get_file_attributes(target);
        if ta == sys::INVALID_FILE_ATTRIBUTES {
            let e = match sys::move_file_ex(src, target, 0) {
                Ok(()) => return true,
                Err(e) => e,
            };
            if e == sys::ERROR_NOT_SAME_DEVICE {
                // copied + deleted instead - only a top item: the fallback copy lands in the destination folder itself,
                // so an item from deeper in a merged folder would land in the wrong place. That one stays put, said so.
                if top {
                    fallback.push(src.clone());
                } else {
                    self.fail_note(src, "It is on another drive mounted inside this folder - move it on its own.".into());
                }
                return true;
            }
            if e == sys::ERROR_ACCESS_DENIED && dest_refuses(paths::parent(target).as_ref()) && self.hand_over(None) {
                return false;
            }
            self.fail_note(src, sys::error_text(e));
            return true;
        }
        let target_is_dir = (ta & sys::FILE_ATTRIBUTE_DIRECTORY) != 0;
        if is_dir && target_is_dir {
            // merge folders: move the children one by one. One the destination refuses: the administrator copy merges
            // what is left of this whole item
            for (name, child_is_dir) in list_names(src) {
                if self.canceled() {
                    return true;
                }
                if !self.move_item(&paths::combine(src, &name), &paths::combine(target, &name), child_is_dir, fallback, false) {
                    return false;
                }
            }
            sys::remove_directory(src);
            return true;
        }
        // name conflict
        let mut fi = FileItem { src: src.clone(), rel: paths::file_name(target), ..Default::default() };
        if let Ok(sd) = sys::get_file_attributes_ex(src) {
            fi.set_size(sd.size());
            fi.mtime = sd.write;
        }
        if let Ok(dd) = sys::get_file_attributes_ex(target) {
            fi.dest_exists = true;
            fi.dest_is_dir = target_is_dir;
            fi.dest_size = dd.size();
            fi.dest_mtime = dd.write;
        }
        let fi = Arc::new(fi);
        self.conflicts_left.fetch_add(1, SEQ);
        let choice = self.resolve_conflict(&fi, target);
        match choice {
            ConflictPolicy::Skip => {
                self.skipped_files.fetch_add(1, SEQ);
                lock(&self.skipped_items).push(fi);
                true
            }
            ConflictPolicy::KeepBoth => {
                let free = free_name(target, is_dir);
                if let Err(e) = sys::move_file_ex(src, &free, 0) {
                    self.fail_note(src, sys::error_text(e));
                }
                true
            }
            _ => {
                // Replace
                if is_dir || target_is_dir {
                    self.fail_note(src, "A file and a folder have the same name - kept both untouched.".into());
                    return true;
                }
                clear_read_only(target);
                if let Err(e) = sys::move_file_ex(src, target, sys::MOVEFILE_REPLACE_EXISTING) {
                    self.fail_note(src, sys::error_text(e));
                }
                true
            }
        }
    }

    fn fail_note(&self, src: &WString, why: String) {
        let fi = FileItem::new(src.clone(), paths::file_name(src), 0);
        fi.set_error(Some(why));
        fi.set_st(FileState::Failed);
        self.failed_files.fetch_add(1, SEQ);
        lock(&self.failed_items).push(Arc::new(fi));
    }

    // ================= copy (and cross-drive move) =================

    fn run_copy(self: &Arc<Self>, sources: Vec<WString>, explicit_items: Option<Vec<FileItem>>) {
        self.set_phase(JobPhase::Analyzing);
        let canceled = self.canceled.clone();
        let scanner = Arc::new(Scanner::new(move || canceled.load(SEQ)));
        *lock(&self.scanner) = Some(scanner.clone());
        let src_threads = if self.src_vol().is_some_and(|v| v.is_network()) { 8 } else { 1 };
        let dst_threads = if self.dst_vol().is_some_and(|v| v.is_network()) { 8 } else { 2 };
        let scan = if let Some(explicit) = explicit_items {
            let mut scan = ScanResult::default();
            for f in explicit {
                let mut c = FileItem {
                    src: f.src.clone(),
                    rel: f.rel.clone(),
                    attr: f.attr,
                    ctime: f.ctime,
                    atime: f.atime,
                    mtime: f.mtime,
                    ..Default::default()
                };
                c.set_size(f.size());
                if let Ok(d) = sys::get_file_attributes_ex(&c.src) {
                    c.set_size(d.size());
                    c.mtime = d.write;
                    c.ctime = d.creation;
                    c.attr = d.attributes;
                }
                if let Ok(dd) = sys::get_file_attributes_ex(&paths::combine(&self.spec.dest, &c.rel)) {
                    c.dest_exists = true;
                    c.dest_size = dd.size();
                    c.dest_mtime = dd.write;
                }
                scan.total_bytes += c.size();
                if c.dest_exists {
                    scan.conflicts += 1;
                }
                scan.files.push(Arc::new(c));
            }
            if let Some(dirs) = &self.spec.explicit_dirs {
                for d in dirs {
                    let mut c = DirItem { src: d.src.clone(), rel: d.rel.clone(), depth: d.depth, ..Default::default() };
                    if let Ok(a) = sys::get_file_attributes_ex(&c.src) {
                        c.attr = a.attributes;
                        c.ctime = a.creation;
                        c.mtime = a.write;
                    }
                    let t = sys::get_file_attributes(&paths::combine(&self.spec.dest, &c.rel));
                    if t != sys::INVALID_FILE_ATTRIBUTES {
                        if (t & sys::FILE_ATTRIBUTE_DIRECTORY) != 0 {
                            c.dest_existed = true;
                        } else {
                            c.dest_is_file = true;
                        }
                    }
                    scan.dirs.push(c);
                }
            }
            // OrderByDescending(Size).Take(4): a stable sort, the largest first
            let mut order: Vec<&Arc<FileItem>> = scan.files.iter().collect();
            order.sort_by_key(|x| std::cmp::Reverse(x.size()));
            scan.largest = order.iter().take(4).map(|x| (x.name(), x.size())).collect();
            scan
        } else {
            let sub = JobSpec { op: self.spec.op, sources, dest: self.spec.dest.clone(), ..Default::default() };
            scanner.run(&sub, src_threads, dst_threads)
        };
        if self.canceled() {
            return;
        }
        for p in &scan.problems {
            self.note(p.clone());
        }
        let files: Vec<Arc<FileItem>> = scan.files.clone();
        let _ = self.files.set(files);
        self.total_files.store(self.files().len() as i32, SEQ);
        self.total_bytes.store(scan.total_bytes, SEQ);
        self.conflicts_left.store(scan.conflicts, SEQ);
        let mut plan = Plan::choose(&JobSpec::new(OpKind::Copy), self.src_vol(), self.dst_vol(), Some(&scan));
        let s = &self.spec;
        if s.force_workers > 0 {
            plan.workers = s.force_workers;
            plan.can_tune = false;
        }
        if s.no_tune {
            plan.can_tune = false;
        }
        if s.small_mode.as_deref() == Some("batch") {
            plan.can_tune = false; // batches are read and written by one worker
        }
        let fixed_pipe = s.pipe_chunk_mb > 0
            || s.pipe_slots > 0
            || s.pipe_batch > 0
            || s.pipe_depth > 0
            || s.pipe_write_step_mb > 0
            || s.no_tune
            || s.no_prealloc
            || s.force_prealloc
            || s.pipe_phased >= 0;
        // with a speed limit the trials would measure the limit, not the drives
        let tuner = if plan.pipeline && !fixed_pipe && self.limit() <= 0.0 && !plan.pipe_ways.is_empty() {
            Some(PipeTuner::new(plan.pipe.clone(), plan.pipe_ways.clone(), plan.tune_seconds, plan.tune_margin))
        } else {
            None
        };
        *lock(&self.tune_summary) = tuner.as_ref().map(|t| t.summary());
        *lock(&self.pipe_tuner) = tuner;
        let da = sys::get_file_attributes(&s.dest);
        self.pipe_off_at_dest.store(
            da != sys::INVALID_FILE_ATTRIBUTES && (da & (sys::FILE_ATTRIBUTE_COMPRESSED | sys::FILE_ATTRIBUTE_ENCRYPTED)) != 0,
            SEQ,
        );
        let (big_threshold, workers, can_tune, max_workers) = (plan.big_threshold, plan.workers, plan.can_tune, plan.max_workers);
        *self.plan_mut() = Some(plan);

        // known destination folders
        {
            let mut d = lock(&self.dirs);
            d.known.insert(upper(&s.dest));
            for dir in &scan.dirs {
                if dir.dest_existed {
                    d.known.insert(upper(&paths::combine(&s.dest, &dir.rel)));
                }
            }
        }
        let _ = self.scan.set(scan);

        // FAT32 (USB sticks, cards) holds no file of 4 GB or more: such a file fails now, with the reason, instead of
        // after 4 GB were written; the others are copied as usual
        let max_file = if s.max_file_test > 0 { s.max_file_test } else { self.dst_vol().map_or(0, |v| v.max_file_size()) };
        if max_file > 0 {
            let fs = match self.dst_vol() {
                Some(v) => v.file_system.clone().unwrap_or_default(),
                None => "FAT32".to_string(),
            };
            for f in self.files() {
                if f.size() > max_file {
                    self.mark_failed(
                        f,
                        format!("Too big for this drive: its {fs} file system holds files of up to 4 GB (NTFS and exFAT have no such limit)."),
                    );
                }
            }
        }

        // space check (bytes that will land there; conflicts may overwrite, count them anyway)
        let free = self.free_at_dest();
        if free >= 0 {
            let need = self.total_bytes.load(SEQ) - self.failed_bytes.load(SEQ);
            if need > 0
                && need > free
                && let Some(ask) = &self.ask_low_space
                && !ask(self, need, free)
            {
                self.canceled.store(true, SEQ);
                return;
            }
        }

        for f in self.files() {
            if f.size() >= big_threshold {
                self.pending_big.fetch_add(1, SEQ);
            } else {
                self.pending_small.fetch_add(1, SEQ);
            }
        }

        self.set_phase(JobPhase::Copying);
        if !self.paused() {
            self.copy_clock.start();
        }
        self.active_workers.store(workers, SEQ);
        let threads = if can_tune { workers.max(max_workers) } else { workers };
        let mut ws = Vec::new();
        for i in 0..threads.max(1) {
            let me = self.clone();
            let made = std::thread::Builder::new().name(format!("copy {i}")).spawn(move || {
                if let Err(e) = catch_unwind(AssertUnwindSafe(|| me.worker_loop(i))) {
                    // outside a file's own copy (C# crashed here): the job fails once the rest is wrapped up
                    let mut wp = lock(&me.worker_panic);
                    if wp.is_none() {
                        *wp = Some(e);
                    }
                }
            });
            match made {
                Ok(t) => ws.push(t),
                // no more threads (memory): the workers there do the rest (they take files by index), none at all =
                // the job fails as C#'s Thread.Start did - and no worker keeps running after the job ended
                Err(_) if !ws.is_empty() => break,
                Err(_) => panic!("{}", big_copy::OUT_OF_MEMORY),
            }
        }
        for t in ws {
            let _ = t.join();
        }
        *lock(&self.batch) = None;
        // a move: the last copies onto the disk, then their originals go (also when canceled: those files are done)
        if s.op == OpKind::Move {
            let waiting = lock(&self.flush).flusher.is_some();
            if waiting && !self.canceled() {
                self.set_status(Some("Making sure the moved files are on the disk...".into()));
            }
            self.finish_durable();
            if waiting {
                self.set_status(None);
            }
        }
        if let Some(e) = lock(&self.worker_panic).take() {
            resume_unwind(e);
        }

        if self.hand_off.load(SEQ) && !self.canceled() {
            // the rest (files never started + those the destination refused) goes to the administrator copy
            let rest: Vec<Arc<FileItem>> = self.files().iter().filter(|x| x.st() == FileState::Pending).cloned().collect();
            if !rest.is_empty() {
                let dirs = self.scan().map(|s| s.dirs.clone());
                self.run_elevated(Some(rest), dirs, None);
            }
        }
        self.copy_clock.stop();

        if self.canceled() {
            return;
        }
        self.set_phase(JobPhase::Finishing);
        self.finish_folders();
    }

    fn worker_loop(self: &Arc<Self>, idx: i32) {
        let files = self.files();
        while !self.canceled() {
            if idx >= self.active_workers.load(SEQ) {
                if self.next_index.load(SEQ) as usize >= files.len() {
                    return;
                }
                sys::sleep_ms(100);
                continue;
            }
            self.wait_to_run();
            if self.canceled() || self.hand_off.load(SEQ) {
                return;
            }
            let i = self.next_index.fetch_add(1, SEQ) as usize;
            if i >= files.len() {
                return;
            }
            let f = &files[i];
            let big = f.size() >= self.plan_get(|p| p.big_threshold);
            if big {
                self.pending_big.fetch_sub(1, SEQ);
            } else {
                self.pending_small.fetch_sub(1, SEQ);
            }
            if f.st() == FileState::Failed {
                continue; // failed before it started (too big for the drive)
            }
            if big {
                let mut n = lock(&self.big_in_flight);
                while *n >= self.plan_get(|p| p.big_slots) && !self.canceled() {
                    n = wait_timeout(&self.big_cv, n, Duration::from_millis(200));
                }
                *n += 1;
            }
            // one disk at both ends: small files are read ahead in batches (read everything, then write everything)
            if self.batch_on()
                && f.size() < self.plan_get(|p| p.pipe_threshold)
                && lock(&self.batch).as_ref().is_none_or(|b| i >= b.end)
            {
                self.fill_batch(i);
            }
            if let Err(e) = catch_unwind(AssertUnwindSafe(|| self.process_file(f))) {
                self.mark_failed(f, format!("Unexpected error: {}", panic_message(&*e)));
            }
            if big {
                let mut n = lock(&self.big_in_flight);
                *n -= 1;
                self.big_cv.notify_all();
            }
        }
    }

    fn wait_to_run(&self) {
        while !self.canceled() && (self.paused() || self.waiting_net.load(SEQ)) {
            self.run_gate.wait(Duration::from_millis(250));
            if self.waiting_net.load(SEQ) {
                sys::sleep_ms(250);
            }
        }
    }

    // ---------- one file ----------

    fn process_file(self: &Arc<Self>, f: &Arc<FileItem>) {
        f.set_st(FileState::Active);
        let mut final_ = paths::combine(&self.spec.dest, &f.rel);
        let parent = paths::parent(&final_).unwrap_or_default();

        // the destination folder for this file
        let mut de = self.ensure_dir(&parent);
        if de != 0 {
            if is_net_error(de) && self.wait_for_network() {
                de = self.ensure_dir(&parent);
            }
            if de == sys::ERROR_ACCESS_DENIED && self.hand_over(Some(f)) {
                return;
            }
            if de != 0 {
                self.mark_failed(f, format!("Can't create the folder {}: {}", paths::to_short(&parent), sys::error_text(de)));
                return;
            }
        }

        let mut replace = false;
        if f.dest_exists {
            let mut resumed = self.spec.resume_mode && !f.dest_is_dir && f.dest_size == f.size() && f.dest_mtime == f.mtime;
            // a move deletes the original: there, size and date aren't enough (a power cut can leave a copy with its
            // name and dates but not its data) - the copy must read back the same as the original
            if resumed && self.spec.op == OpKind::Move && !self.same_content(&f.src, &final_) {
                resumed = false;
                replace = true;
                self.conflicts_left.fetch_sub(1, SEQ);
            }
            if resumed {
                // already copied by the interrupted run
                f.already_there.store(true, SEQ);
                f.set_final_path(final_.clone());
                self.conflicts_left.fetch_sub(1, SEQ);
                add(&self.done_bytes, f.size());
                add(&self.copied_bytes, f.size());
                self.already_there_files.fetch_add(1, SEQ);
                self.done_files.fetch_add(1, SEQ);
                f.set_st(FileState::Done);
                if self.spec.op == OpKind::Move {
                    self.moved(f, false);
                }
                return;
            }
            if !replace {
                let choice = self.resolve_conflict(f, &final_);
                if self.canceled() {
                    return;
                }
                if choice == ConflictPolicy::Skip {
                    self.mark_skipped(f);
                    return;
                }
                if choice == ConflictPolicy::KeepBoth {
                    final_ = free_name(&final_, false);
                } else {
                    if f.dest_is_dir {
                        self.mark_failed(f, "A folder with this name is already there.".into());
                        return;
                    }
                    replace = true;
                }
            }
        }

        // "<name>.chpart" taken already: a leftover of this very job's interrupted run goes; anything else (a file that
        // only looks like one, e.g. copied from a folder that held one) is never touched - another temp name is used
        let mut temp = temp_name(&final_);
        let mut alt = 0;
        if f.temp_exists {
            if self.spec.resume_mode && !self.is_job_dest(&temp) {
                delete_quiet(&temp);
            } else {
                temp = free_temp(&final_, &mut alt);
            }
        }

        let mut attempt = 0;
        let mut verify_retried = false;
        loop {
            if self.canceled() {
                return;
            }
            self.wait_to_run();
            if self.canceled() {
                return;
            }

            // whether this file is checked: decided once per try (the switch can change while it copies)
            let check = self.verify();
            let t = Arc::new(Transfer::new(f.clone()));
            // fast path for a brand-new file without verify: one rename through the copy's own handle instead of open
            // + rename + close afterwards (saves round trips per file on a NAS)
            if !replace && !check && !self.spec.no_fast_path && !self.fast_path_off.load(SEQ) {
                *lock(&t.rename_to) = Some(final_.clone());
            }
            lock(&self.active).push(t.clone());
            let r = catch_unwind(AssertUnwindSafe(|| self.copy_file(f, &temp, &t)));
            lock(&self.active).retain(|x| !Arc::ptr_eq(x, &t));
            let (ok, err) = match r {
                Ok(v) => v,
                Err(e) => resume_unwind(e),
            };

            if !ok {
                add(&self.done_bytes, -t.done.load(SEQ));
                // it had already taken its real name (a brand-new name, so the file there is ours): remove it
                if t.renamed.load(SEQ) {
                    delete_quiet(&final_);
                }
                if err == sys::ERROR_REQUEST_ABORTED {
                    delete_quiet(&temp);
                    if self.canceled() {
                        return;
                    }
                    if t.skip.load(SEQ) {
                        self.mark_skipped(f);
                        return;
                    }
                }
                if err == sys::ERROR_FILE_EXISTS || err == sys::ERROR_ALREADY_EXISTS {
                    // the temp name got taken (not by this try - it never replaces): another one, the file there stays
                    temp = free_temp(&final_, &mut alt);
                    let a = attempt;
                    attempt += 1;
                    if a < 3 {
                        continue;
                    }
                }
                if err == sys::ERROR_DISK_FULL || err == sys::ERROR_HANDLE_DISK_FULL {
                    delete_quiet(&temp);
                    // really full: wait for room. Room enough and still refused is a limit of the drive (the largest
                    // file it takes, a quota): pausing would never help - this file fails, the rest go on
                    let room = self.free_at_dest();
                    if room < 0 || room < f.size() + (64 << 20) {
                        self.set_status(Some("The destination is full - free some space, then press Resume.".into()));
                        self.pause();
                        continue;
                    }
                    self.mark_failed(f, "The destination refused the file as too big, though it has room for it.".into());
                    return;
                }
                if is_net_error(err) && attempt < 6 {
                    attempt += 1;
                    if !self.wait_for_network() {
                        return;
                    }
                    self.backoff(attempt, Some(f), "The network hiccuped");
                    continue;
                }
                if is_busy(err) && attempt < 3 {
                    attempt += 1;
                    self.backoff(attempt, Some(f), "In use by another program");
                    continue;
                }
                delete_quiet(&temp);
                // the destination folder refuses (only administrators may add files there), not the source
                if err == sys::ERROR_ACCESS_DENIED && dest_refuses(Some(&parent)) && self.hand_over(Some(f)) {
                    return;
                }
                self.mark_failed(f, sys::error_text(err));
                return;
            }

            // verify before the file gets its real name
            if check {
                t.verifying.store(true, SEQ);
                lock(&self.active).push(t.clone());
                let r = catch_unwind(AssertUnwindSafe(|| self.verify_one(f, &temp, &t)));
                lock(&self.active).retain(|x| !Arc::ptr_eq(x, &t));
                t.verifying.store(false, SEQ);
                let vr = match r {
                    Ok(v) => v,
                    Err(e) => resume_unwind(e),
                };
                let same = match vr {
                    Ok(same) => same,
                    Err(ve) => {
                        if ve == sys::ERROR_REQUEST_ABORTED {
                            delete_quiet(&temp);
                            if self.canceled() {
                                return;
                            }
                            if t.skip.load(SEQ) {
                                add(&self.done_bytes, -t.done.load(SEQ));
                                self.mark_skipped(f);
                                return;
                            }
                        }
                        if is_net_error(ve) && attempt < 6 {
                            attempt += 1;
                            if !self.wait_for_network() {
                                return;
                            }
                            self.backoff(attempt, None, "");
                            add(&self.done_bytes, -t.done.load(SEQ));
                            delete_quiet(&temp);
                            continue;
                        }
                        add(&self.done_bytes, -t.done.load(SEQ));
                        delete_quiet(&temp);
                        self.mark_failed(f, format!("Couldn't check the copy: {}", sys::error_text(ve)));
                        return;
                    }
                };
                if !same {
                    add(&self.done_bytes, -t.done.load(SEQ));
                    delete_quiet(&temp);
                    if !verify_retried {
                        verify_retried = true;
                        self.note(format!("Check failed once, copied again: {}", paths::to_short(&f.src)));
                        continue;
                    }
                    self.mark_failed(f, "The copy didn't match the original twice (xxHash64). Not kept.".into());
                    return;
                }
                f.verified.store(true, SEQ);
                self.verified_files.fetch_add(1, SEQ);
            }

            if t.renamed.load(SEQ) {
                // already under its real name (renamed through the copy's handle after the last byte).
                // Guard: the first few of every job are checked where they must be; if one isn't, the fast path is off.
                if self.fast_checks.fetch_add(1, SEQ) < 3 && !paths::exists(&final_) {
                    self.fast_path_off.store(true, SEQ);
                    self.note(format!(
                        "Fast rename switched off: a file wasn't where expected ({}).",
                        paths::to_short(&final_)
                    ));
                    self.mark_failed(f, "The copy wasn't found under its final name - not counted as done.".into());
                    return;
                }
                if !t.times_set.load(SEQ) {
                    set_times(&final_, f.ctime, f.mtime, false);
                }
                f.set_final_path(final_.clone());
                f.set_st(FileState::Done);
                add(&self.copied_bytes, f.size());
                self.done_files.fetch_add(1, SEQ);
                if self.spec.op == OpKind::Move {
                    self.moved(f, t.durable.load(SEQ));
                }
                return;
            }

            // keep the original creation date too (Windows' copy keeps only the modified date)
            if !t.times_set.load(SEQ) {
                set_times(&temp, f.ctime, f.mtime, false);
            }

            // into place
            let mut re = rename_into_place(&temp, &mut final_, replace);
            if re != 0 {
                if is_net_error(re) && attempt < 6 {
                    attempt += 1;
                    if !self.wait_for_network() {
                        return;
                    }
                    self.backoff(attempt, None, "");
                    re = rename_into_place(&temp, &mut final_, replace);
                }
                if re != 0 {
                    add(&self.done_bytes, -t.done.load(SEQ));
                    delete_quiet(&temp);
                    self.mark_failed(f, format!("Couldn't put the file in place: {}", sys::error_text(re)));
                    return;
                }
            }
            // done
            f.set_final_path(final_.clone());
            f.set_st(FileState::Done);
            add(&self.copied_bytes, f.size());
            self.done_files.fetch_add(1, SEQ);
            if replace {
                self.conflicts_left.fetch_sub(1, SEQ);
            }
            if self.spec.op == OpKind::Move {
                self.moved(f, t.durable.load(SEQ));
            }
            return;
        }
    }

    // ---------- one disk at both ends, many small files: read a batch, then write it ----------

    fn batch_on(&self) -> bool {
        let s = &self.spec;
        let plan = lock(&self.plan);
        let Some(p) = plan.as_ref() else { return false };
        p.pipeline
            && (p.small_batch || s.small_mode.as_deref() == Some("batch"))
            && p.workers == 1
            && !p.can_tune
            && s.engine.is_none()
            && !self.pipe_off_at_dest.load(SEQ)
            && s.small_mode.as_deref() != Some("off")
    }

    /// Reads the small files from index i on (up to 256 MB, 1,024 files) into memory, in the order they sit on the
    /// disk: the heads sweep once over the sources; then they are written through the cache, whose own writer puts
    /// them out in one sweep while the next batch is read.
    fn fill_batch(self: &Arc<Self>, i: usize) {
        let mut guard = lock(&self.batch);
        if guard.is_none() {
            let mut cap: i64 = 256 << 20;
            if let Some((phys, _)) = sys::memory_status() {
                cap = (32i64 << 20).max(cap.min((phys / 8) as i64));
            }
            let Some(mem) = sys::Memory::commit(cap as usize) else { return };
            *guard = Some(SmallBatch { mem, cap, end: 0, at: HashMap::new() });
        }
        let batch = guard.as_mut().unwrap();
        batch.at.clear();
        let (pipe_threshold, pipe_align) = self.plan_get(|p| (p.pipe_threshold, p.pipe.align));
        let align = 4096.max(pipe_align);
        let files = self.files();
        let mut open: Vec<(usize, sys::Handle, i64)> = Vec::new(); // (file index, handle, lcn)
        let mut place: HashMap<usize, (usize, usize)> = HashMap::new(); // file index -> (where, room)
        let mut used: i64 = 0;
        let mut j = i;
        while j < files.len() && open.len() < 1024 && !self.canceled() {
            let f = &files[j];
            if f.size() >= pipe_threshold || (f.attr & big_copy::SKIP_ATTRS) != 0 {
                break; // a big one: the batch ends here
            }
            let mut room = (f.size() + align as i64 - 1) / align as i64 * align as i64;
            if used + room > batch.cap {
                break;
            }
            let Some((h, size, lcn)) = big_copy::open_small(&f.src) else {
                j += 1;
                continue; // this one goes the usual way
            };
            room = (size + align as i64 - 1) / align as i64 * align as i64;
            if size != f.size() && used + room > batch.cap {
                drop(h);
                break;
            }
            place.insert(j, (used as usize, room as usize));
            used += room;
            open.push((j, h, lcn));
            j += 1;
        }
        batch.end = j;
        // in disk order: one sweep of the heads instead of a jump per file
        open.sort_by_key(|o| o.2);
        for (k, h, _) in &open {
            if self.canceled() {
                break;
            }
            self.wait_to_run();
            let f = &files[*k];
            // the size again (C# ignored a failure here and copied an empty file: it can't fail on an open handle, but
            // if it does the file goes the usual way)
            let Ok(size) = sys::file_size(h) else { continue };
            let (start, room) = place[k];
            // never past the room this file was given (C# read the new size in whole: it can't grow while it is open
            // without sharing writes, but a neighbour's bytes must never be at stake)
            if ((size + align as i64 - 1) / align as i64 * align as i64) as usize > room {
                continue;
            }
            let mem = batch.mem.bytes_mut();
            if big_copy::read_small(h, &mut mem[start..start + room], size, align) {
                if size != f.size() {
                    add(&self.total_bytes, size - f.size());
                    f.set_size(size);
                }
                batch.at.insert(Arc::as_ptr(f) as usize, start);
            }
        }
        // the handles close here
    }

    /// A file of the batch: written from memory in one go (else false: the usual way). Some(error) = it went this way.
    fn copy_from_batch(self: &Arc<Self>, f: &Arc<FileItem>, temp: &WString, t: &Transfer) -> Option<u32> {
        let mut guard = lock(&self.batch);
        let batch = guard.as_mut()?;
        let at = batch.at.remove(&(Arc::as_ptr(f) as usize))?;
        if self.canceled() || t.skip.load(SEQ) {
            return Some(sys::ERROR_REQUEST_ABORTED);
        }
        let s = &self.spec;
        // through the cache (measured on one disk: 67-69 MB/s, straight to the disk 51, Explorer 52-55): the cache's
        // own writer puts many small files out in one sweep while the next batch is read
        let cached = match s.small_mode.as_deref() {
            None | Some("cached") | Some("batch") => true,
            Some("tiny") => f.size() < (64 << 10),
            _ => false,
        };
        let align = 4096.max(self.plan_get(|p| p.pipe.align));
        let size = f.size();
        let room = ((size + align as i64 - 1) / align as i64 * align as i64) as usize;
        let mem = batch.mem.bytes_mut();
        let end = (at + room).min(mem.len());
        let data = &mut mem[at..end];
        let rename_to = lock(&t.rename_to).clone();
        let (err, renamed, times_set) =
            big_copy::write_small(temp, data, size, align, f.ctime, f.mtime, f.attr, rename_to.as_ref(), cached);
        if err == big_copy::NOT_SUPPORTED {
            return None;
        }
        if err != 0 {
            return Some(err);
        }
        t.renamed.store(renamed, SEQ);
        t.times_set.store(times_set, SEQ);
        if self.verify() {
            let mut x = XxHash64::new();
            x.update(&data[..size as usize]);
            t.has_src_hash.store(true, SEQ);
            t.src_hash.store(x.digest(), SEQ);
        }
        drop(guard);
        t.done.store(size, SEQ);
        add(&self.done_bytes, size);
        self.throttle(size);
        Some(0)
    }

    /// Big local files take the pipeline; everything else (and whatever the pipeline can't take) CopyFileEx.
    /// (it went, the error).
    fn copy_file(self: &Arc<Self>, f: &Arc<FileItem>, temp: &WString, t: &Arc<Transfer>) -> (bool, u32) {
        if let Some(err) = self.copy_from_batch(f, temp, t) {
            return (err == 0, err);
        }
        if self.use_pipe(f) {
            let err = self.copy_big(f, temp, t);
            if err == 0 {
                return (true, 0);
            }
            if err != big_copy::NOT_SUPPORTED {
                return (false, err);
            }
            // this file can't go that way (a file system or driver that refuses unbuffered I/O): CopyFileEx copies it
            add(&self.done_bytes, -t.done.load(SEQ));
            t.done.store(0, SEQ);
            delete_quiet(temp);
        }
        self.copy_one(f, temp, t)
    }

    fn use_pipe(&self, f: &FileItem) -> bool {
        let (pipeline, threshold) = self.plan_get(|p| (p.pipeline, p.pipe_threshold));
        pipeline
            && self.spec.engine.is_none()
            && !self.pipe_off_at_dest.load(SEQ)
            && f.size() >= threshold
            && (f.attr & big_copy::SKIP_ATTRS) == 0
    }

    /// 0 = copied; else the error (NOT_SUPPORTED: CopyFileEx takes it).
    fn copy_big(self: &Arc<Self>, f: &Arc<FileItem>, temp: &WString, t: &Arc<Transfer>) -> u32 {
        let s = &self.spec;
        let mut tune = self.plan_get(|p| p.pipe.clone());
        if s.pipe_chunk_mb > 0
            || s.pipe_slots > 0
            || s.pipe_batch > 0
            || s.pipe_depth > 0
            || s.pipe_write_step_mb > 0
            || s.no_prealloc
            || s.force_prealloc
            || s.cached_write_mb > 0
            || s.pipe_phased >= 0
        {
            tune = PipeTuning {
                align: tune.align,
                chunk: if s.pipe_chunk_mb > 0 { s.pipe_chunk_mb << 20 } else { tune.chunk },
                slots: if s.pipe_slots > 0 { s.pipe_slots } else { tune.slots },
                write_batch: if s.pipe_batch > 0 { s.pipe_batch } else { tune.write_batch },
                phased: if s.pipe_phased >= 0 { s.pipe_phased == 1 } else { tune.phased },
                prealloc: s.force_prealloc || (tune.prealloc && !s.no_prealloc),
                cached_write_chunk: if s.cached_write_mb > 0 { s.cached_write_mb << 20 } else { tune.cached_write_chunk },
                tail_after_read: tune.tail_after_read,
                read_depth: if s.pipe_depth > 0 { s.pipe_depth } else { tune.read_depth },
                write_step: if s.pipe_write_step_mb > 0 { s.pipe_write_step_mb << 20 } else { tune.write_step },
            };
        }
        let mut progress = |done: i64| -> bool {
            let delta = done - t.done.load(SEQ);
            if delta != 0 {
                t.done.store(done, SEQ);
                add(&self.done_bytes, delta);
                if delta > 0 {
                    self.throttle(delta);
                }
            }
            if self.canceled() || t.skip.load(SEQ) {
                return false;
            }
            if self.paused() {
                while self.paused() && !self.canceled() && !t.skip.load(SEQ) {
                    self.run_gate.wait(Duration::from_millis(200));
                }
                if self.canceled() || t.skip.load(SEQ) {
                    return false;
                }
            }
            true
        };
        // the last big file of a copy: its end goes through the Windows cache, so the copy is done when the last bytes
        // are in memory - like Explorer's copy (a move and a checked copy read the disk, they don't)
        let mut tail: i64 = 0;
        let dst = self.dst_vol();
        if s.tail_mb >= 0 {
            tail = (s.tail_mb as i64) << 20;
        }
        // never for a USB stick: it may be pulled out right after "done". Nor when the source is the slow end (a hard
        // disk or a USB drive into an internal SSD): the copy waits on the reads anyway, so a tail in memory makes
        // "done" no sooner and leaves writing for after it. Five runs each into the NVMe, time to the disk against
        // Windows' copy, with the tail / without: 1 GB from a hard disk -1.7% / +1.6%, 2 GB from a 10 Gbps USB drive
        // -2.4% / +5.2%, from a 10 Gbps USB SSD +5.3% / +5.3% (every run within 0.03 s without it; one took twice as
        // long with it)
        else if !self.verify()
            && s.op == OpKind::Copy
            && !s.no_cached_tail
            && dst.is_none_or(|d| d.kind != DeviceKind::Usb && !d.removable)
            && !self.source_is_slow_end()
        {
            let after = self.total_bytes.load(SEQ) - self.done_bytes.load(SEQ) - (f.size() - t.done.load(SEQ));
            if after < (64 << 20) {
                tail = cache_budget();
            }
        }
        // a move flushes the copy to the disk before the original can go
        let mut tuner_guard = Some(lock(&self.pipe_tuner));
        let mut use_tuner = tuner_guard.as_ref().unwrap().as_ref().is_some_and(|x| !x.settled());
        if let Some(best) = tuner_guard.as_ref().unwrap().as_ref().filter(|x| x.settled()).map(|x| x.best().clone()) {
            tune = best;
        }
        // to a hard disk, the last file goes there the way Explorer's copy sends it: through the Windows cache, its size
        // reserved first, done once it is in memory - Windows writes it out right after (512 MB NVMe to hard disk:
        // Explorer done in 0.15 s, the copy straight to the disk 2.7-3.1 s). From an SSD any size goes that way:
        // Windows holds the writer back by itself while too much waits in memory, as it does for its own copy.
        // Measured, NVMe to each of two hard disks about 80% full (D: / X:), five runs each, time to the disk against
        // Windows': 2 GB +5% / -1%, 4 GB 0% / -2%. Before, a file bigger than a quarter of the free memory went through
        // the pipeline and its trials: 2 GB -8% / -22%, 4 GB +10% / -20% (where the file lands decides; -5% / -24% in a
        // Benchmark run). With at most 256 MB waiting in memory, flushed as it goes: 2 GB +4% / -1%, 4 GB -5% / -5%.
        // From a hard disk only a file that fits in memory, once the whole batch is read (the plan says so), so the
        // writing never competes with the reads. Nothing to try then.
        let src = self.src_vol();
        let fast_source = src.is_some_and(|v| !v.is_slow_seek() && !v.is_network()); // not a hard disk, USB stick or disc
        if tail > 0
            && (tail >= f.size() || fast_source)
            && s.tail_mb < 0
            && dst.is_some_and(|d| d.kind == DeviceKind::Hdd)
            && s.pipe_chunk_mb == 0
        {
            tail = f.size();
            use_tuner = false;
            tune = PipeTuning { prealloc: true, ..tune };
        }
        // while it still measures, at most a quarter of the file's end waits in memory: the rest is room for the trials
        else if use_tuner {
            tail = tail.min(f.size() / 4);
        }
        if !use_tuner {
            tuner_guard = None;
        }
        let rename_to = lock(&t.rename_to).clone();
        let step = || self.limit_step();
        let r = big_copy::copy(
            &f.src,
            temp,
            f.ctime,
            f.mtime,
            f.attr,
            &tune,
            self.verify(),
            s.op == OpKind::Move,
            rename_to.as_ref(),
            tail,
            &mut progress,
            tuner_guard.as_mut().and_then(|g| g.as_mut()),
            Some(&step),
        );
        let picked = tuner_guard.as_ref().and_then(|g| g.as_ref()).filter(|x| x.settled() && x.changed).map(|x| x.best().clone());
        if let Some(g) = &tuner_guard {
            *lock(&self.tune_summary) = g.as_ref().map(|x| x.summary());
        }
        drop(tuner_guard);
        if let Some(best) = picked {
            let mut plan = self.plan_mut();
            let p = plan.as_mut().expect("the plan");
            if !p.tuned {
                // the measurements picked another way: the badge says so, the next big files use it
                p.pipe = best;
                p.tuned = true;
                p.tune_note = Some("tuned".into());
                drop(plan);
                self.raise_phase();
            }
        }
        if r.error != 0 {
            return r.error;
        }
        t.times_set.store(r.times_set, SEQ);
        t.renamed.store(r.renamed, SEQ);
        t.durable.store(s.op == OpKind::Move, SEQ); // a move's big file was flushed before it got its name
        if self.verify() {
            t.has_src_hash.store(true, SEQ);
            t.src_hash.store(r.hash, SEQ);
        }
        if r.bytes != f.size() {
            add(&self.total_bytes, r.bytes - f.size());
            f.set_size(r.bytes);
        }
        0
    }

    /// A hard disk, disc or USB drive into an internal SSD: the reads set the pace.
    fn source_is_slow_end(&self) -> bool {
        match (self.src_vol(), self.dst_vol()) {
            (Some(s), Some(d)) => {
                !s.is_network() && !d.is_network() && !d.is_slow_seek() && !d.on_usb && (s.is_slow_seek() || s.on_usb)
            }
            _ => false,
        }
    }

    /// Free bytes at the destination for this user (quotas count); -1 = couldn't tell.
    fn free_at_dest(&self) -> i64 {
        let dir = paths::to_short(&self.spec.dest).trim_end_char('\\') + "\\";
        sys::disk_free_space_ex(&dir).map_or(-1, |f| f.min(i64::MAX as u64) as i64)
    }

    /// (it went, the error).
    fn copy_one(self: &Arc<Self>, f: &Arc<FileItem>, temp: &WString, t: &Arc<Transfer>) -> (bool, u32) {
        let mut flags = sys::COPY_FILE_FAIL_IF_EXISTS | sys::COPY_FILE_ALLOW_DECRYPTED_DESTINATION;
        if self.spec.engine.as_deref() == Some("nobuf") && f.size() >= (256 << 20) {
            flags |= sys::COPY_FILE_NO_BUFFERING; // tests: the old way, to compare
        }
        let mut cb = |p: &sys::CopyProgress| -> u32 {
            let delta = p.transferred - t.done.load(SEQ);
            if delta != 0 {
                t.done.store(p.transferred, SEQ);
                add(&self.done_bytes, delta);
                if delta > 0 {
                    self.throttle(delta);
                }
            }
            if self.canceled() || t.skip.load(SEQ) {
                return sys::PROGRESS_CANCEL;
            }
            if self.paused() {
                while self.paused() && !self.canceled() && !t.skip.load(SEQ) {
                    self.run_gate.wait(Duration::from_millis(200));
                }
                if self.canceled() || t.skip.load(SEQ) {
                    return sys::PROGRESS_CANCEL;
                }
            }
            if let Some(hd) = p.dest {
                // original creation date, set on the copy's own open handle (no extra round trip later)
                if !t.times_set.load(SEQ)
                    && p.reason == sys::CALLBACK_STREAM_SWITCH
                    && f.ctime != 0
                    && sys::set_file_time(hd, Some(f.ctime), None)
                {
                    t.times_set.store(true, SEQ);
                }
                // every byte is written: give it its real name now (never replaces anything)
                let mut rt = lock(&t.rename_to);
                if let Some(to) = rt.as_ref()
                    && !t.renamed.load(SEQ)
                    && p.transferred >= p.total
                {
                    if sys::rename_by_handle(hd, to) {
                        t.renamed.store(true, SEQ);
                    } else {
                        *rt = None;
                    }
                }
            }
            sys::PROGRESS_CONTINUE
        };
        let r = sys::copy_file_ex(&f.src, temp, flags, &mut cb);
        let ok = r.is_ok();
        let err = r.err().unwrap_or(0);
        if ok && t.done.load(SEQ) < f.size() {
            // the file grew/shrank between analysis and copy: count what was really copied
            add(&self.total_bytes, t.done.load(SEQ) - f.size());
            f.set_size(t.done.load(SEQ));
        }
        (ok, err)
    }

    /// Ok(same) or Err(the error).
    fn verify_one(&self, f: &FileItem, temp: &WString, t: &Transfer) -> Result<bool, u32> {
        let h1;
        let half;
        if t.has_src_hash.load(SEQ) {
            // hashed on the way in: only the copy is read back (from the disk, not the cache)
            h1 = t.src_hash.load(SEQ);
            half = f.size();
            t.verify_done.store(half, SEQ);
        } else {
            let mut prog = |n: i64| -> bool {
                t.verify_done.store(n, SEQ);
                self.wait_to_run_in_callback(t);
                !(self.canceled() || t.skip.load(SEQ))
            };
            h1 = xxhash64::hash_file(&f.src, false, Some(&mut prog))?;
            half = t.verify_done.load(SEQ);
        }
        let mut prog2 = |n: i64| -> bool {
            t.verify_done.store(half + n, SEQ);
            self.wait_to_run_in_callback(t);
            !(self.canceled() || t.skip.load(SEQ))
        };
        let h2 = xxhash64::hash_file(temp, true, Some(&mut prog2))?;
        Ok(h1 == h2)
    }

    fn wait_to_run_in_callback(&self, t: &Transfer) {
        while self.paused() && !self.canceled() && !t.skip.load(SEQ) {
            self.run_gate.wait(Duration::from_millis(200));
        }
    }

    fn delete_source(&self, f: &FileItem) {
        // another program saved the original after it was read: the copy is older, so the original stays
        if let Ok(d) = sys::get_file_attributes_ex(&f.src)
            && (d.size() != f.size() || d.write != f.mtime)
        {
            f.source_kept.store(true, SEQ);
            self.note(format!("Moved, but the original changed meanwhile - it was kept: {}", paths::to_short(&f.src)));
            return;
        }
        let mut e = match sys::delete_file(&f.src) {
            Ok(()) => return,
            Err(e) => e,
        };
        if e == sys::ERROR_FILE_NOT_FOUND {
            return;
        }
        if e == sys::ERROR_ACCESS_DENIED {
            let a = sys::get_file_attributes(&f.src);
            if clear_read_only(&f.src) {
                match sys::delete_file(&f.src) {
                    Ok(()) => return,
                    Err(e2) => e = e2,
                }
                sys::set_file_attributes(&f.src, a); // still there: read-only again, as it was
            }
        }
        f.source_kept.store(true, SEQ);
        self.note(format!(
            "Moved, but the original couldn't be removed: {} - {}",
            paths::to_short(&f.src),
            sys::error_text(e)
        ));
    }

    // ---------- moves: an original goes only once its copy is on the disk ----------
    // Small copies are written through the Windows cache, and every drive writes its cache out on its own schedule:
    // a power cut right after the original's delete could leave neither. So the copies wait here and are flushed in
    // big batches, many flushes at once - the disk merges them - and only then are their originals deleted.
    // Measured, 2,000 files (1 GB) onto a hard disk, until all of it is on the disk: 8 flushes at once 27 s after the
    // copy, 32 19 s, 128 10 s; flushing while copying slows the copy (11.7-12.9 s in all); copy, then 256 at once:
    // 10.0 s - about what the disk needs to write it anyway. In Choppy: batches of 2,000 / 256 at once 12.1-12.6 s,
    // 500 / 512 at once 10.9-11.1 s, 250 / 1,024 12.0 s (Explorer: 11.0-11.6 s, with the originals gone before that).

    fn flush_batch_files(&self) -> usize {
        if self.spec.flush_files > 0 { self.spec.flush_files as usize } else { 500 }
    }
    fn flush_batch_bytes(&self) -> i64 {
        (self.flush_batch_files() as i64) << 19 // 500 files or 256 MB
    }

    /// A move's copy is complete: its original goes now (durable = the copy was flushed already) or after the flush.
    fn moved(self: &Arc<Self>, f: &Arc<FileItem>, durable: bool) {
        if durable {
            self.delete_source(f);
            return;
        }
        let mut st = lock(&self.flush);
        if st.dead {
            drop(st);
            self.flush_then_delete(vec![f.clone()]);
            return;
        }
        if st.to_flush.is_empty() {
            st.oldest = Some(Instant::now());
        }
        st.to_flush.push(f.clone());
        st.bytes += f.size();
        if st.flusher.is_none() {
            let me = self.clone();
            st.flusher = Some(spawn("move flush".into(), None, move || {
                if let Err(e) = catch_unwind(AssertUnwindSafe(|| me.flush_loop())) {
                    // the job fails once the rest is wrapped up; what was waiting is flushed by FinishDurable
                    let mut wp = lock(&me.worker_panic);
                    if wp.is_none() {
                        *wp = Some(e);
                    }
                    drop(wp);
                    lock(&me.flush).dead = true;
                    me.flush_cv.notify_all();
                }
            }));
        }
        if st.to_flush.len() >= self.flush_batch_files() || st.bytes >= self.flush_batch_bytes() {
            self.flush_cv.notify_all();
        }
        // never far ahead of the disk: at most two batches wait
        while (st.to_flush.len() >= 2 * self.flush_batch_files() || st.bytes >= 2 * self.flush_batch_bytes()) && !st.end && !st.dead {
            st = wait_timeout(&self.flush_cv, st, Duration::from_millis(500));
        }
    }

    fn flush_loop(self: &Arc<Self>) {
        loop {
            let work;
            {
                let mut st = lock(&self.flush);
                // a full batch, the end of the move, or copies waiting 10 s already (a slow source)
                while !st.end
                    && st.to_flush.len() < self.flush_batch_files()
                    && st.bytes < self.flush_batch_bytes()
                    && (st.to_flush.is_empty() || st.oldest.is_some_and(|o| o.elapsed() < Duration::from_secs(10)))
                {
                    st = wait_timeout(&self.flush_cv, st, Duration::from_millis(1000));
                }
                if st.to_flush.is_empty() {
                    if st.end {
                        return;
                    }
                    continue;
                }
                work = std::mem::take(&mut st.to_flush);
                st.bytes = 0;
                self.flush_cv.notify_all(); // room again for a worker that waits
            }
            self.flush_then_delete(work);
        }
    }

    fn flush_then_delete(self: &Arc<Self>, work: Vec<Arc<FileItem>>) {
        // many at once: the disk merges flushes that arrive together (a network share: fewer, each one is a round trip)
        let cap = if self.spec.flush_threads > 0 {
            self.spec.flush_threads as usize
        } else if self.dst_vol().is_some_and(|v| v.is_network()) {
            32
        } else {
            512
        };
        let threads = work.len().min(cap);
        let work = Arc::new(work);
        let next = Arc::new(AtomicI64::new(-1));
        let one = {
            let (me, work, next) = (self.clone(), work.clone(), next.clone());
            move || {
                loop {
                    let k = next.fetch_add(1, SEQ) + 1;
                    if k as usize >= work.len() {
                        break;
                    }
                    let f = &work[k as usize];
                    let e = flush_path(&f.final_path().unwrap_or_default());
                    if e == 0 {
                        me.delete_source(f);
                    } else {
                        f.source_kept.store(true, SEQ);
                        me.note(format!(
                            "Moved, but the copy couldn't be confirmed on the disk - the original was kept: {} - {}",
                            paths::to_short(&f.src),
                            sys::error_text(e)
                        ));
                    }
                }
            }
        };
        let mut ts = Vec::new();
        for _ in 1..threads {
            let o = one.clone();
            let made = std::thread::Builder::new().name("flush".into()).stack_size(256 * 1024).spawn(move || {
                let _ = catch_unwind(AssertUnwindSafe(o));
            });
            match made {
                Ok(t) => ts.push(t),
                Err(_) => break, // no more threads (memory): the ones there and this one do the rest
            }
        }
        let r = catch_unwind(AssertUnwindSafe(one));
        // every flush that started ends before this returns (FinishDurable waits for all of them)
        for t in ts {
            let _ = t.join();
        }
        if let Err(e) = r {
            resume_unwind(e);
        }
    }

    /// Waits until every copy of this move is on the disk and its original handled.
    fn finish_durable(self: &Arc<Self>) {
        let t = {
            let mut st = lock(&self.flush);
            st.end = true;
            self.flush_cv.notify_all();
            st.flusher.take()
        };
        if let Some(t) = t {
            let _ = t.join();
        }
        // a flusher that died left copies waiting: flushed here, then their originals go
        let rest = std::mem::take(&mut lock(&self.flush).to_flush);
        if !rest.is_empty() {
            self.flush_then_delete(rest);
        }
    }

    // ---------- administrator rights ----------

    /// Asks once per copy (the first file the destination refuses). True = hand the rest over.
    fn ask_to_elevate(self: &Arc<Self>) -> bool {
        let Some(ask) = &self.ask_elevate else { return false };
        if self.spec.elevated || handoff::is_admin() || self.canceled() {
            return false;
        }
        let mut st = lock(&self.elev_state);
        if *st == 0 {
            *st = if ask(self) { 1 } else { 2 };
        }
        *st == 1
    }

    /// The destination refused this file for lack of rights: true = it goes to the administrator copy.
    fn hand_over(self: &Arc<Self>, f: Option<&Arc<FileItem>>) -> bool {
        if !self.ask_to_elevate() {
            return false;
        }
        if let Some(f) = f {
            f.set_st(FileState::Pending);
        }
        self.hand_off.store(true, SEQ);
        true
    }

    /// Runs the rest of this copy in Choppy started as administrator (its own window) and mirrors its progress here
    /// until it ends; then its counts, problems and notes join this copy's report.
    /// items + dirs: the files left (and the folders around them); sources: whole items (same-drive move, or nothing
    /// started).
    fn run_elevated(self: &Arc<Self>, items: Option<Vec<Arc<FileItem>>>, dirs: Option<Vec<DirItem>>, sources: Option<Vec<WString>>) {
        let limit = self.limit();
        let mut spec = JobSpec {
            id: format!("{}a", self.spec.id),
            op: self.spec.op,
            dest: self.spec.dest.clone(),
            verify: self.verify(),
            policy: self.policy(),
            speed_limit_mbps: if limit > 0.0 { limit * 8.0 / 1e6 } else { 0.0 },
            origin: "Administrator".into(),
            engine: self.spec.engine.clone(),
            sources: sources.clone().unwrap_or_else(|| self.spec.sources.clone()),
            ..Default::default()
        };
        let mut handed_bytes = 0i64;
        if let Some(items) = &items {
            spec.explicit_items = Some(items.iter().map(|x| FileItem::new(x.src.clone(), x.rel.clone(), x.size())).collect());
            spec.explicit_dirs =
                dirs.map(|ds| ds.iter().map(|d| DirItem { src: d.src.clone(), rel: d.rel.clone(), depth: d.depth, ..Default::default() }).collect());
            for x in items {
                handed_bytes += x.size();
            }
        }
        let record = match handoff::save(&spec) {
            Ok(r) => r,
            Err(m) => {
                self.hand_back(items.as_deref(), sources.as_deref(), format!("Couldn't write the administrator copy's record: {m}"));
                return;
            }
        };

        let p = match handoff::launch(&record) {
            Ok(p) => p,
            Err(e) => {
                handoff::clean(&record, true);
                let why = if handoff::declined(e) {
                    "Administrator rights weren't given.".to_string()
                } else {
                    format!("Couldn't start Choppy as administrator: {}", sys::win32_message(e))
                };
                self.hand_back(items.as_deref(), sources.as_deref(), why);
                return;
            }
        };

        self.as_admin.store(true, SEQ);
        self.set_status(Some("Copying as administrator in its own window".into()));
        self.raise_phase();
        let base_done = self.done_bytes.load(SEQ);
        let base_files = self.done_files.load(SEQ);
        let mirror = Arc::new(Transfer::new(Arc::new(FileItem::default())));
        let mut mirrored = false;
        let (mut seq, mut owner_pid) = (0, 0);
        let (mut sent_pause, mut sent_cancel) = (false, false);
        let mut res: Option<handoff::HandoffResult> = None;
        let quiet = Stopwatch::start_new();
        let looped = catch_unwind(AssertUnwindSafe(|| loop {
            res = handoff::read_result(&record);
            if res.is_some() {
                break;
            }
            if let Some(pr) = handoff::read_progress(&record) {
                let get = |k: &str| pr.get(&k.to_ascii_lowercase());
                if let Some(v) = get("pid") {
                    owner_pid = super::netfmt::parse_int(v).unwrap_or(0);
                }
                if let Some(n) = get("doneBytes").and_then(|v| super::netfmt::parse_long(v)) {
                    self.done_bytes.store(base_done + n, SEQ);
                }
                if let Some(n) = get("doneFiles").and_then(|v| super::netfmt::parse_long(v)) {
                    self.done_files.store(base_files + n as i32, SEQ);
                }
                match get("cur") {
                    Some(cur) if !cur.is_empty() => {
                        let cs = get("curSize").and_then(|v| super::netfmt::parse_long(v)).unwrap_or(0);
                        let cd = get("curDone").and_then(|v| super::netfmt::parse_long(v)).unwrap_or(0);
                        let cur_w = WString::from(cur.as_str());
                        {
                            let mut mf = lock(&mirror.file);
                            if mf.rel != cur_w {
                                let src = get("curSrc").map_or(cur_w.clone(), |v| WString::from(v.as_str()));
                                *mf = Arc::new(FileItem::new(src, cur_w.clone(), cs));
                            }
                            mf.set_size(cs);
                        }
                        mirror.done.store(cd, SEQ);
                        mirror.verifying.store(get("verifying").is_some_and(|v| v == "1"), SEQ);
                        if !mirrored {
                            lock(&self.active).push(mirror.clone());
                            mirrored = true;
                        }
                    }
                    _ => {
                        if mirrored {
                            lock(&self.active).retain(|x| !Arc::ptr_eq(x, &mirror));
                            mirrored = false;
                        }
                    }
                }
                self.set_status(Some(if get("paused").is_some_and(|v| v == "1") {
                    "Paused in the administrator window".into()
                } else {
                    "Copying as administrator in its own window".into()
                }));
                quiet.restart();
            }
            // Pause / Resume / Cancel here reach it too
            if self.canceled() && !sent_cancel {
                seq += 1;
                handoff::control(&record, seq, "cancel");
                sent_cancel = true;
            } else if !self.canceled() && self.paused() != sent_pause {
                seq += 1;
                handoff::control(&record, seq, if self.paused() { "pause" } else { "run" });
                sent_pause = self.paused();
            }
            // it ended (or was closed) without leaving a result
            let alive = !p.has_exited() || handoff::alive(owner_pid);
            if !alive && quiet.elapsed().as_secs_f64() > 2.0 {
                res = handoff::read_result(&record);
                break;
            }
            sys::sleep_ms(200);
        }));
        // C#'s finally: the mirror and "as administrator" go however the loop ended
        lock(&self.active).retain(|x| !Arc::ptr_eq(x, &mirror));
        self.as_admin.store(false, SEQ);
        drop(p);
        if let Err(e) = looped {
            resume_unwind(e);
        }

        let Some(res) = res else {
            handoff::clean(&record, true);
            self.set_status(None);
            self.hand_back(items.as_deref(), sources.as_deref(), "The administrator copy stopped before it finished.".into());
            return;
        };
        // its numbers join this copy's
        self.done_bytes.store(base_done + res.done_bytes, SEQ);
        self.done_files.store(base_files + res.done_files, SEQ);
        if let Some(items) = &items {
            add(&self.total_bytes, res.total_bytes - handed_bytes);
            self.total_files.fetch_add(res.total_files - items.len() as i32, SEQ);
            for x in items {
                x.set_st(FileState::Done); // the ones that weren't are in its failed / skipped lists
            }
        }
        add(&self.copied_bytes, res.copied_bytes);
        self.admin_files.fetch_add(res.done_files, SEQ);
        self.verified_files.fetch_add(res.verified, SEQ);
        self.already_there_files.fetch_add(res.already, SEQ);
        self.skipped_files.fetch_add(res.skipped, SEQ);
        add(&self.skipped_bytes, res.skipped_bytes);
        self.failed_files.fetch_add(res.failed, SEQ);
        add(&self.failed_bytes, res.failed_bytes);
        lock(&self.failed_items).extend(res.failed_items.into_iter().map(Arc::new));
        lock(&self.skipped_items).extend(res.skipped_items.into_iter().map(Arc::new));
        {
            let mut notes = lock(&self.notes);
            notes.push(format!(
                "{} as administrator: {}{}{}",
                if self.spec.op == OpKind::Move { "Moved" } else { "Copied" },
                res.done_files,
                if res.done_files == 1 { " file" } else { " files" },
                res.log.as_ref().map_or(String::new(), |l| format!(" (its log: {l})"))
            ));
            for n in &res.notes {
                notes.push(format!("As administrator: {n}"));
            }
        }
        self.set_status(if res.phase.as_deref() == Some("Failed") { res.status.clone() } else { None });
        if res.phase.as_deref() == Some("Canceled") {
            self.canceled.store(true, SEQ);
        }
        handoff::clean(&record, true);
    }

    /// No administrator copy after all: what was meant for it is reported as failed, like before.
    fn hand_back(&self, items: Option<&[Arc<FileItem>]>, sources: Option<&[WString]>, why: String) {
        self.note(why.clone());
        if let Some(items) = items {
            for f in items {
                self.mark_failed(f, format!("Access denied. {why}"));
            }
        } else if let Some(sources) = sources {
            for s in sources {
                self.fail_note(s, format!("Access denied. {why}"));
            }
        }
    }

    // ---------- conflicts ----------

    fn resolve_conflict(self: &Arc<Self>, f: &Arc<FileItem>, final_: &WString) -> ConflictPolicy {
        let _g = lock(&self.conflict_lock);
        let mut p = self.policy();
        if p == ConflictPolicy::Ask {
            match &self.ask_conflict {
                None => p = ConflictPolicy::Skip,
                Some(ask) => {
                    let info = ConflictInfo {
                        file: f.clone(),
                        incoming_path: paths::to_short(&f.src),
                        existing_path: paths::to_short(final_),
                        incoming_size: f.size(),
                        existing_size: f.dest_size,
                        incoming_time: file_times::to_local(f.mtime),
                        existing_time: file_times::to_local(f.dest_mtime),
                        remaining: 1.max(self.conflicts_left.load(SEQ)),
                        op: self.spec.op,
                    };
                    let a = ask(self, &info);
                    match a {
                        Some(a) if !a.cancel_job => {
                            p = a.choice;
                            if a.apply_to_all {
                                *lock(&self.policy) = p;
                            }
                        }
                        _ => {
                            self.cancel();
                            return ConflictPolicy::Skip;
                        }
                    }
                }
            }
        }
        if p == ConflictPolicy::ReplaceOlder {
            p = if f.mtime > f.dest_mtime { ConflictPolicy::Replace } else { ConflictPolicy::Skip };
        }
        if p != ConflictPolicy::Replace {
            self.conflicts_left.fetch_sub(1, SEQ);
        }
        p
    }

    /// A file of this job lands under this name (so it is a real file, not a leftover temp).
    fn is_job_dest(&self, path: &WString) -> bool {
        let mut g = lock(&self.job_dests);
        let set = g.get_or_insert_with(|| self.files().iter().map(|x| upper(&paths::combine(&self.spec.dest, &x.rel))).collect());
        set.contains(&upper(path))
    }

    /// The copy reads back the same as the original (xxHash64 of both; the copy past the cache).
    fn same_content(&self, src: &WString, copy: &WString) -> bool {
        let mut go = |_: i64| !self.canceled();
        let a = xxhash64::hash_file(src, false, Some(&mut go));
        let mut go2 = |_: i64| !self.canceled();
        match a {
            Ok(a) => xxhash64::hash_file(copy, true, Some(&mut go2)).is_ok_and(|b| a == b),
            Err(_) => false,
        }
    }

    // ---------- bookkeeping ----------

    fn mark_skipped(&self, f: &Arc<FileItem>) {
        f.set_st(FileState::Skipped);
        self.skipped_files.fetch_add(1, SEQ);
        add(&self.skipped_bytes, f.size());
        lock(&self.skipped_items).push(f.clone());
    }

    fn mark_failed(&self, f: &Arc<FileItem>, why: String) {
        f.set_st(FileState::Failed);
        f.set_error(Some(why));
        self.failed_files.fetch_add(1, SEQ);
        add(&self.failed_bytes, f.size());
        lock(&self.failed_items).push(f.clone());
    }

    // ---------- folders ----------

    fn ensure_dir(&self, dir: &WString) -> u32 {
        if lock(&self.dirs).known.contains(&upper(dir)) {
            return 0;
        }
        let e = self.create_dir_tree(dir);
        if e == 0 {
            lock(&self.dirs).known.insert(upper(dir));
        }
        e
    }

    fn create_dir_tree(&self, dir: &WString) -> u32 {
        let mut e = match sys::create_directory(dir) {
            Ok(()) => {
                lock(&self.dirs).created.insert(upper(dir));
                return 0;
            }
            Err(e) => e,
        };
        if e == sys::ERROR_ALREADY_EXISTS {
            return if paths::dir_exists(dir) { 0 } else { sys::ERROR_ALREADY_EXISTS };
        }
        if e == sys::ERROR_PATH_NOT_FOUND {
            let Some(p) = paths::parent(dir) else { return e };
            if p.len() <= paths::LONG_PREFIX.len() + 3 {
                return e;
            }
            let pe = self.create_dir_tree(&p);
            if pe != 0 {
                return pe;
            }
            match sys::create_directory(dir) {
                Ok(()) => {
                    lock(&self.dirs).created.insert(upper(dir));
                    return 0;
                }
                Err(e2) => e = e2,
            }
            if e == sys::ERROR_ALREADY_EXISTS {
                return 0;
            }
        }
        e
    }

    fn finish_folders(self: &Arc<Self>) {
        let Some(scan) = self.scan() else { return };
        // empty folders exist too, folders get their original dates and attributes
        let dirs = &scan.dirs;
        let mut work: Vec<DirItem> = Vec::new();
        for d in dirs {
            if self.canceled() {
                return;
            }
            if d.dest_is_file {
                self.note(format!(
                    "A file with this folder's name is in the way: {}",
                    paths::to_short(&paths::combine(&self.spec.dest, &d.rel))
                ));
                continue;
            }
            let target = paths::combine(&self.spec.dest, &d.rel);
            if !d.dest_existed {
                self.ensure_dir(&target);
                work.push(d.clone());
            }
        }
        let threads = if self.dst_vol().is_some_and(|v| v.is_network()) { 8 } else { 1 };
        let work = Arc::new(work);
        let next = Arc::new(AtomicI64::new(-1));
        let run = {
            let (me, work, next) = (self.clone(), work.clone(), next.clone());
            move || loop {
                let k = next.fetch_add(1, SEQ) + 1;
                if k as usize >= work.len() || me.canceled() {
                    break;
                }
                let d = &work[k as usize];
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    let target = paths::combine(&me.spec.dest, &d.rel);
                    set_times(&target, d.ctime, d.mtime, true);
                    let keep = d.attr & (0x2 /*hidden*/ | 0x4 /*system*/ | 0x20/*archive*/);
                    if (keep & 0x6) != 0 {
                        sys::set_file_attributes(&target, keep | sys::FILE_ATTRIBUTE_DIRECTORY);
                    }
                }));
            }
        };
        // C# made `threads` of them and waited; here this thread is one of them
        let mut ts = Vec::new();
        for _ in 1..threads.min(work.len().max(1)) {
            let r = run.clone();
            match std::thread::Builder::new().name("folders".into()).spawn(r) {
                Ok(t) => ts.push(t),
                Err(_) => break,
            }
        }
        run();
        for t in ts {
            let _ = t.join();
        }

        if self.spec.op == OpKind::Move {
            // remove the now-empty source folders, deepest first (a folder that still holds anything stays)
            // (not one whose place was taken by a file: it wasn't moved, so it stays even when empty)
            let mut order: Vec<&DirItem> = dirs.iter().collect();
            order.sort_by_key(|d| std::cmp::Reverse(d.depth));
            for d in order {
                if !d.dest_is_file {
                    sys::remove_directory(&d.src);
                }
            }
        }
    }

    // ---------- errors / network ----------

    fn backoff(&self, attempt: i32, f: Option<&FileItem>, why: &str) {
        let ms = 16000.min(1000u64 << (attempt - 1).clamp(0, 4) as u32);
        let sw = Instant::now();
        if let Some(f) = f {
            *lock(&self.retry_file) = Some(f.name());
            *lock(&self.retry_why) = Some(why.to_string());
        }
        while (sw.elapsed().as_millis() as u64) < ms && !self.canceled() {
            if f.is_some() {
                let left = ms.saturating_sub(sw.elapsed().as_millis() as u64);
                *lock(&self.retry_why) = Some(format!("{why} - trying again in {} s", 1.max(left.div_ceil(1000))));
            }
            sys::sleep_ms(100);
        }
        if let Some(f) = f {
            let mut rf = lock(&self.retry_file);
            if rf.as_ref() == Some(&f.name()) {
                *rf = None;
                *lock(&self.retry_why) = None;
            }
        }
    }

    /// Waits (up to 15 min) until both sides answer again. False = canceled or gave up.
    fn wait_for_network(self: &Arc<Self>) -> bool {
        let a = self.spec.dest.clone();
        let b = if self.src_vol().is_some_and(|v| v.is_network()) && !self.spec.sources.is_empty() {
            paths::parent(&self.spec.sources[0])
        } else {
            None
        };
        let reachable = || paths::dir_exists(&a) && b.as_ref().is_none_or(paths::exists);
        if reachable() {
            return true;
        }
        let mut owner = false;
        {
            let _g = lock(&self.active);
            if !self.waiting_net.load(SEQ) {
                self.waiting_net.store(true, SEQ);
                owner = true;
            }
        }
        if owner {
            self.set_status(Some("Waiting for the network to come back...".into()));
            self.set_phase(JobPhase::Waiting);
        }
        let sw = Instant::now();
        let mut ok = false;
        while !self.canceled() && sw.elapsed() < Duration::from_secs(15 * 60) {
            if reachable() {
                ok = true;
                break;
            }
            sys::sleep_ms(2000);
        }
        if owner {
            self.waiting_net.store(false, SEQ);
            self.set_status(if ok { None } else { Some("The network didn't come back in 15 minutes.".into()) });
            if !self.canceled() {
                self.set_phase(JobPhase::Copying);
            }
        }
        ok && !self.canceled()
    }

    // ---------- speed limit ----------

    /// With a speed limit, one write is at most an eighth of a second of it (at least 256 KB): steady, not bursts.
    fn limit_step(&self) -> i32 {
        let rate = self.limit();
        if rate > 0.0 { (256.0f64 * 1024.0).max((64.0f64 * 1048576.0).min(rate / 8.0)) as i32 } else { 0 }
    }

    fn throttle(&self, bytes: i64) {
        let rate = self.limit();
        if rate <= 0.0 {
            return;
        }
        let mut wait = 0.0;
        {
            let mut tk = lock(&self.tokens);
            let now = self.token_clock.elapsed().as_secs_f64();
            tk.0 += (now - tk.1) * rate;
            tk.1 = now;
            if tk.0 > rate * 0.25 {
                tk.0 = rate * 0.25;
            }
            tk.0 -= bytes as f64;
            if tk.0 < 0.0 {
                wait = -tk.0 / rate;
            }
        }
        if wait > 0.0 {
            let ms = (2000.0f64.min(wait * 1000.0)) as u64;
            let sw = Instant::now();
            while (sw.elapsed().as_millis() as u64) < ms && !self.canceled() {
                sys::sleep_ms(50.min(ms) as u32);
            }
        }
    }

    // ---------- monitor: speed history + one-time tuning ----------

    fn monitor_loop(self: &Arc<Self>) {
        let mut last_bytes = 0i64;
        let sw = Instant::now();
        let mut last_t = 0.0f64;
        let mut small = Probe::default();
        let mut big_p = Probe::default();
        let mut awake = false;
        while !self.is_finished() {
            // the PC doesn't go to sleep in the middle of a copy (the screen may turn off); a paused copy lets it
            if awake == self.paused() {
                awake = !self.paused();
                sys::set_thread_execution_state(if awake { sys::ES_CONTINUOUS | sys::ES_SYSTEM_REQUIRED } else { sys::ES_CONTINUOUS });
            }
            sys::sleep_ms(250);
            let t = sw.elapsed().as_secs_f64();
            let b = self.done_bytes.load(SEQ);
            let dt = t - last_t;
            if dt <= 0.0 {
                continue;
            }
            let s = 0.0f64.max((b - last_bytes) as f64 / dt);
            last_bytes = b;
            last_t = t;
            let running = self.phase() == JobPhase::Copying && !self.paused() && !self.waiting_net.load(SEQ);
            {
                let mut sp = lock(&self.speed);
                sp.now = if running { s } else { 0.0 };
                if running {
                    sp.smooth = if sp.smooth <= 0.0 { s } else { sp.smooth * 0.92 + s * 0.08 };
                    if s > sp.peak {
                        sp.peak = s;
                    }
                }
                let now = sp.now;
                sp.history.push(now);
                if sp.history.len() > 600 {
                    let extra = sp.history.len() - 600;
                    sp.history.drain(0..extra);
                }
                sp.samples += 1;
                let k = 4.min(sp.history.len());
                let sum: f64 = sp.history[sp.history.len() - k..].iter().sum();
                sp.second = if running && k > 0 { sum / k as f64 } else { 0.0 };
            }

            // Adaptive: measure the real path, try more in flight, keep only what is faster
            let Some((can_tune, network)) = lock(&self.plan).as_ref().map(|p| (p.can_tune, p.network)) else { continue };
            if !can_tune {
                continue;
            }
            let big_active = *lock(&self.big_in_flight) > 0;
            if !running {
                self.interrupt(&mut small, false);
                self.interrupt(&mut big_p, true);
                continue;
            }
            if !big_active {
                self.interrupt(&mut big_p, true);
                self.tune_small(&mut small, t, self.done_files.load(SEQ));
            } else {
                self.interrupt(&mut small, false);
                if network {
                    self.tune_big(&mut big_p, t, b); // local big files tune inside the pipeline
                }
            }
        }
        if awake {
            sys::set_thread_execution_state(sys::ES_CONTINUOUS);
        }
    }

    // small files are limited by round trips, not bandwidth: measure files per second
    fn tune_small(self: &Arc<Self>, p: &mut Probe, t: f64, done_files: i32) {
        if p.stage == 3 {
            return;
        }
        let pend = self.pending_small.load(SEQ);
        let max_workers = self.plan_get(|x| x.max_workers);
        let active = self.active_workers.load(SEQ);
        match p.stage {
            0 => {
                if pend < 150 || active >= max_workers {
                    return;
                }
                if p.start == 0.0 {
                    p.start = t;
                    return;
                }
                if t - p.start >= 2.0 {
                    p.stage = 1;
                    p.start = t;
                    p.start_value = done_files as f64;
                }
            }
            1 => {
                if t - p.start < 5.0 {
                    return;
                }
                p.base_rate = (done_files as f64 - p.start_value) / (t - p.start);
                if pend < 100 {
                    p.stage = 3;
                    return;
                }
                p.saved = active;
                self.active_workers.store(max_workers.min(active * 2), SEQ);
                p.stage = 2;
                p.start = t;
                p.start_value = done_files as f64;
            }
            2 => {
                if t - p.start < 5.0 {
                    return;
                }
                let rate = (done_files as f64 - p.start_value) / (t - p.start);
                if rate > p.base_rate * 1.15 {
                    {
                        let mut plan = self.plan_mut();
                        let x = plan.as_mut().expect("the plan");
                        x.workers = active;
                        x.tuned = true;
                    }
                    self.raise_phase();
                    if pend >= 150 && active < max_workers {
                        // it helped: try one more step up
                        p.base_rate = rate;
                        p.saved = active;
                        self.active_workers.store(max_workers.min(active * 2), SEQ);
                        p.start = t;
                        p.start_value = done_files as f64;
                        return;
                    }
                } else {
                    self.active_workers.store(p.saved, SEQ);
                }
                p.stage = 3;
            }
            _ => {}
        }
    }

    // big files are limited by bandwidth: measure bytes per second with a second big file in flight
    fn tune_big(self: &Arc<Self>, p: &mut Probe, t: f64, bytes: i64) {
        if p.stage == 3 {
            return;
        }
        match p.stage {
            0 => {
                if self.pending_big.load(SEQ) < 1 || self.plan_get(|x| x.big_slots) != 1 {
                    return;
                }
                if p.start == 0.0 {
                    p.start = t;
                    return;
                }
                if t - p.start >= 3.0 {
                    p.stage = 1;
                    p.start = t;
                    p.start_value = bytes as f64;
                }
            }
            1 => {
                if t - p.start < 8.0 {
                    return;
                }
                p.base_rate = (bytes as f64 - p.start_value) / (t - p.start);
                if self.pending_big.load(SEQ) < 1 {
                    p.stage = 3;
                    return;
                }
                self.set_big_slots(2);
                p.stage = 2;
                p.start = t;
                p.start_value = bytes as f64;
            }
            2 => {
                if t - p.start < 8.0 {
                    return;
                }
                let rate = (bytes as f64 - p.start_value) / (t - p.start);
                if rate > p.base_rate * 1.15 {
                    {
                        let mut plan = self.plan_mut();
                        let x = plan.as_mut().expect("the plan");
                        x.tuned = true;
                        x.tune_note = Some("2 big files at once".into());
                    }
                    self.raise_phase();
                } else {
                    self.set_big_slots(1);
                }
                p.stage = 3;
            }
            _ => {}
        }
    }

    fn set_big_slots(&self, n: i32) {
        let _g = lock(&self.big_in_flight);
        self.plan_mut().as_mut().expect("the plan").big_slots = n;
        self.big_cv.notify_all();
    }

    // the situation changed mid-measurement (big file started/ended, pause): undo the try, measure again later
    fn interrupt(&self, p: &mut Probe, big: bool) {
        if p.stage == 0 || p.stage == 3 {
            if p.stage == 0 {
                p.start = 0.0;
            }
            return;
        }
        if p.stage == 2 {
            if big {
                self.set_big_slots(1);
            } else {
                self.active_workers.store(p.saved, SEQ);
            }
        }
        p.interrupts += 1;
        p.stage = if p.interrupts >= 4 { 3 } else { 0 };
        p.start = 0.0;
    }

    // ---------- log ----------

    fn write_log(&self) {
        let failed = lock(&self.failed_items).clone();
        let skipped = lock(&self.skipped_items).clone();
        let notes = lock(&self.notes).clone();
        if failed.is_empty() && skipped.is_empty() && notes.is_empty() && !self.spec.elevated {
            return; // an administrator copy always leaves its log
        }
        let dir = app_paths::log_dir();
        let r = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(&dir)?;
            let now = sys::file_time_to_local(sys::now_file_time()).unwrap_or_default();
            let path = dir.join(format!(
                "{:04}-{:02}-{:02} {:02}-{:02}-{:02} {}.txt",
                now.year, now.month, now.day, now.hour, now.minute, now.second, self.spec.id
            ));
            *lock(&self.log_path) = Some(path.clone());
            let local = |ft: i64| sys::file_time_to_local(ft).map_or(String::new(), |t| sys::general_date_time(&t));
            let mut sb = String::new();
            let mut line = |l: &str| {
                sb.push_str(l);
                sb.push_str("\r\n");
            };
            line(&format!("Choppy - {}", self.spec.title()));
            line(&format!("To: {}", paths::to_short(&self.spec.dest)));
            line(&format!(
                "Started {}, finished {} ({})",
                local(self.started_at()),
                local(self.finished_at()),
                self.phase().name()
            ));
            line(&format!(
                "Done {}, skipped {}, failed {}, verified {}",
                self.done_files.load(SEQ),
                self.skipped_files.load(SEQ),
                self.failed_files.load(SEQ),
                self.verified_files.load(SEQ)
            ));
            if !failed.is_empty() {
                line("");
                line("FAILED:");
                for f in &failed {
                    line(&format!("  {}  ->  {}", paths::to_short(&f.src), f.error().unwrap_or_default()));
                }
            }
            if !skipped.is_empty() {
                line("");
                line("SKIPPED:");
                for f in &skipped {
                    line(&format!("  {}", paths::to_short(&f.src)));
                }
            }
            if !notes.is_empty() {
                line("");
                line("NOTES:");
                for n in &notes {
                    line(&format!("  {n}"));
                }
            }
            let mut bytes = vec![0xEF, 0xBB, 0xBF]; // Encoding.UTF8 writes a byte order mark
            bytes.extend_from_slice(sb.as_bytes());
            std::fs::write(&path, bytes)
        })();
        if r.is_err() {
            *lock(&self.log_path) = None;
        }
    }
}

/// One measured experiment: baseline window, then a try window; keep the winner, no flapping.
#[derive(Default)]
struct Probe {
    stage: i32, // 0 = waiting, 1 = baseline, 2 = trying, 3 = finished
    start: f64,
    start_value: f64,
    base_rate: f64,
    saved: i32,
    interrupts: i32,
}

/// A copy of a job record's file (scan data only).
fn copy_item(f: &FileItem) -> FileItem {
    FileItem {
        src: f.src.clone(),
        rel: f.rel.clone(),
        size: f.size().into(),
        attr: f.attr,
        ctime: f.ctime,
        atime: f.atime,
        mtime: f.mtime,
        ..Default::default()
    }
}

fn list_names(dir: &WString) -> Vec<(WString, bool)> {
    let mut r = Vec::new();
    let Ok(list) = sys::find_first(&paths::combine(dir, &w("*"))) else { return r };
    for fd in list {
        if fd.name.eq_str(".") || fd.name.eq_str("..") {
            continue;
        }
        r.push((fd.name, (fd.attributes & sys::FILE_ATTRIBUTE_DIRECTORY) != 0));
    }
    r
}

/// True when a new file can't be made in this folder because of rights (as opposed to an unreadable source).
fn dest_refuses(dir: Option<&WString>) -> bool {
    let Some(dir) = dir.filter(|d| !d.is_empty()) else { return false };
    let g = sys::new_guid();
    let probe = paths::combine(dir, &WString::from(format!("~ch-{:08x}.tmp", g.data1)));
    match sys::create_file(
        &probe,
        sys::GENERIC_WRITE,
        0,
        sys::CREATE_NEW,
        sys::FILE_ATTRIBUTE_TEMPORARY | sys::FILE_FLAG_DELETE_ON_CLOSE,
    ) {
        Ok(_) => false, // closed (and so deleted) right here
        Err(e) => e == sys::ERROR_ACCESS_DENIED,
    }
}

fn free_name(path: &WString, is_dir: bool) -> WString {
    let dir = paths::parent(path).unwrap_or_default();
    let name = paths::file_name(path);
    for n in 2..10000 {
        let cand = paths::combine(&dir, &paths::numbered_name(&name, n, is_dir));
        if !paths::exists(&cand) && !paths::exists(&(cand.clone() + TEMP_SUFFIX)) {
            return cand;
        }
    }
    paths::combine(&dir, &paths::numbered_name(&name, (net_ticks() % 100000) as i32, is_dir))
}

/// DateTime.Now.Ticks (100 ns since year 1, local time).
fn net_ticks() -> i64 {
    // FILETIME counts from 1601; DateTime from year 1: 504,911,232,000,000,000 ticks apart
    sys::now_local_file_time() + 504_911_232_000_000_000
}

/// The copy's name while it is written: "<name>.chpart".
pub fn temp_name(final_: &WString) -> WString {
    temp_name_n(final_, 0)
}

/// The copy's name while it is written; n > 0 = another one, for when "<name>.chpart" is taken.
pub fn temp_name_n(final_: &WString, n: i32) -> WString {
    let name = paths::file_name(final_);
    let extra = if n > 0 { format!(".~{n}") } else { String::new() };
    if name.len() + extra.encode_utf16().count() + TEMP_SUFFIX.len() <= 255 {
        return final_.clone() + extra.as_str() + TEMP_SUFFIX;
    }
    // Encoding.UTF8.GetBytes: a lone surrogate becomes EF BF BD, as the lossy conversion writes it
    let h = XxHash64::of_bytes(name.to_string_lossy().as_bytes());
    paths::combine(&paths::parent(final_).unwrap_or_default(), &WString::from(format!("~ch{h:016x}{extra}{TEMP_SUFFIX}")))
}

/// A temp name nothing has (the next free one after alt).
fn free_temp(final_: &WString, alt: &mut i32) -> WString {
    for n in *alt + 1..1000 {
        let cand = temp_name_n(final_, n);
        if !paths::exists(&cand) {
            *alt = n;
            return cand;
        }
    }
    *alt = 1000 + (net_ticks() % 100000) as i32;
    temp_name_n(final_, *alt)
}

fn rename_into_place(temp: &WString, final_: &mut WString, replace: bool) -> u32 {
    let mut last = 0u32;
    for i in 0u64..4 {
        let e = match sys::move_file_ex(temp, final_, if replace { sys::MOVEFILE_REPLACE_EXISTING } else { 0 }) {
            Ok(()) => return 0,
            Err(e) => e,
        };
        last = e;
        if !replace && (e == sys::ERROR_ALREADY_EXISTS || e == sys::ERROR_FILE_EXISTS) {
            // something new appeared under that name while we copied: keep both, never overwrite silently
            *final_ = free_name(final_, false);
            last = sys::ERROR_FILE_NOT_FOUND; // C#: the last call was FreeName's look for the free name
            continue;
        }
        if replace && e == sys::ERROR_ACCESS_DENIED && clear_read_only(final_) {
            continue;
        }
        if is_busy(e) {
            sys::sleep_ms((500 * (i + 1)) as u32);
            continue;
        }
        return e;
    }
    if last == 0 { sys::ERROR_ACCESS_DENIED } else { last }
}

/// Writes a file's data and its entry to the disk itself (FlushFileBuffers). 0 or the error.
fn flush_path(path: &WString) -> u32 {
    let open = || {
        sys::create_file(
            path,
            sys::GENERIC_WRITE,
            sys::FILE_SHARE_READ | sys::FILE_SHARE_WRITE | sys::FILE_SHARE_DELETE,
            sys::OPEN_EXISTING,
            0,
        )
    };
    for tries in 0..2 {
        let e = match open() {
            Ok(h) => return sys::flush(&h).err().unwrap_or(0),
            Err(e) => e,
        };
        // a read-only copy can't be opened for writing: briefly not, then read-only again
        let a = sys::get_file_attributes(path);
        if e != sys::ERROR_ACCESS_DENIED || a == sys::INVALID_FILE_ATTRIBUTES || (a & sys::FILE_ATTRIBUTE_READONLY) == 0 {
            if is_busy(e) && tries == 0 {
                sys::sleep_ms(200); // a scanner has it open for a moment
                continue;
            }
            return e;
        }
        if !sys::set_file_attributes(path, a & !sys::FILE_ATTRIBUTE_READONLY) {
            return e;
        }
        let r = match open() {
            Ok(h) => sys::flush(&h).err().unwrap_or(0),
            Err(e2) => e2,
        };
        sys::set_file_attributes(path, a);
        if r == 0 {
            return 0;
        }
        if !is_busy(r) {
            return r;
        }
        sys::sleep_ms(200);
    }
    sys::ERROR_ACCESS_DENIED
}

/// How much of a file's end may wait in memory: a quarter of the free memory, at most 2 GB.
fn cache_budget() -> i64 {
    let Some((phys, _)) = sys::memory_status() else { return 0 };
    let b = (phys / 4) as i64;
    if b < (64 << 20) { 0 } else { (2i64 << 30).min(b) }
}

fn delete_quiet(path: &WString) {
    if let Err(e) = sys::delete_file(path)
        && e == sys::ERROR_ACCESS_DENIED
        && clear_read_only(path)
    {
        let _ = sys::delete_file(path);
    }
}

fn clear_read_only(path: &WString) -> bool {
    let a = sys::get_file_attributes(path);
    if a == sys::INVALID_FILE_ATTRIBUTES || (a & sys::FILE_ATTRIBUTE_READONLY) == 0 {
        return false;
    }
    sys::set_file_attributes(path, a & !sys::FILE_ATTRIBUTE_READONLY)
}

fn set_times(path: &WString, ctime: i64, mtime: i64, dir: bool) {
    let flags = if dir { sys::FILE_FLAG_BACKUP_SEMANTICS } else { 0 };
    if let Ok(h) = sys::create_file(
        path,
        sys::FILE_WRITE_ATTRIBUTES,
        sys::FILE_SHARE_READ | sys::FILE_SHARE_WRITE | sys::FILE_SHARE_DELETE,
        sys::OPEN_EXISTING,
        flags,
    ) {
        sys::set_file_time(&h, Some(ctime), Some(mtime));
    }
}

fn is_net_error(e: u32) -> bool {
    matches!(
        e,
        sys::ERROR_BAD_NETPATH
            | sys::ERROR_NETWORK_BUSY
            | sys::ERROR_DEV_NOT_EXIST
            | sys::ERROR_UNEXP_NET_ERR
            | sys::ERROR_NETNAME_DELETED
            | sys::ERROR_BAD_NET_NAME
            | sys::ERROR_SEM_TIMEOUT
            | sys::ERROR_NETWORK_UNREACHABLE
            | sys::ERROR_HOST_UNREACHABLE
            | sys::ERROR_CONNECTION_ABORTED
            | sys::ERROR_CONNECTION_UNAVAIL
            | sys::ERROR_NOT_CONNECTED
            | sys::ERROR_NOT_READY
            | sys::ERROR_INVALID_HANDLE
    )
}

fn is_busy(e: u32) -> bool {
    e == sys::ERROR_SHARING_VIOLATION || e == sys::ERROR_LOCK_VIOLATION
}
