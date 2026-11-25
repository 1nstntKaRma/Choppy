//! Benchmark.cs (1.48): Choppy against Windows' own copy (the shell copy engine Explorer uses) on every drive of this
//! PC: from the system drive to each other drive and back, and within each drive; one big file and many small files.
//! Both copy the same source, which is not in memory, in turns (A-B-B-A), and are timed until they say "done" and
//! until the copies are on the disk. The test files live only in a temporary folder per drive, removed at the end.
//! (Step 3's three-way harness - Rust against C# - stays in the C# on the rust branch; this is 1.48's Benchmark.)

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::engine::copy_job::CopyJob;
use crate::engine::model::{ConflictAnswer, ConflictPolicy, JobPhase, JobSpec, OpKind};
use crate::engine::paths;
use crate::engine::volume_probe::{self, DeviceKind};
use crate::engine::{Stopwatch, lock};
use crate::sys;
use crate::wstr::WString;

use super::files;
use super::report::{BenchCase, BenchDrive, BenchReport, BenchResult, BenchRun, Stamp};
use super::{BenchErr, BenchResult as Res, shell_copy};

pub const MB: i64 = 1 << 20;
/// Less than 4% of the time apart is a Marginal Tie.
pub const TIE_MARGIN: f64 = 0.04;

/// Runs each, unless changed: Quick 2, Thorough 3.
pub fn default_rounds(thorough: bool) -> i32 {
    if thorough { 3 } else { 2 }
}

pub type Tweak = Arc<dyn Fn(&mut JobSpec) + Send + Sync>;

#[derive(Default)]
pub struct Benchmark {
    /// Bigger files, a mixed folder, three runs each.
    pub thorough: bool,
    /// Mapped network drives too.
    pub network: bool,
    /// Only these drive letters ("C:", "D:"): the ones checked in the window, or a test's.
    pub only: Option<HashSet<String>>,
    /// Tests: very small sizes.
    pub tiny: bool,
    /// A line per step (the command line's --log).
    pub log: Option<Arc<dyn Fn(&str) + Send + Sync>>,
    /// Tests: engine switches for Choppy's side.
    pub tweak: Option<Tweak>,
    /// Tests: only these cases ("big", "small", "mixed").
    pub cases: Option<HashSet<String>>,
    /// Tests: this big file size on every route.
    pub big_mb: i32,
    /// Runs each other than the default (the window's Runs, the command line's --rounds).
    pub rounds_override: i32,
    /// Tests: only these routes ("C>D", "D>D").
    pub routes_only: Option<HashSet<String>>,

    report: Mutex<BenchReport>,
    cancel: AtomicBool,
    prog: Mutex<Prog>,
    clock: Stopwatch,
}

struct Prog {
    est_total: f64,
    est_done: f64,
    step_est: f64,
    step_clock: Instant,
    step_text: String,
}

impl Default for Prog {
    fn default() -> Self {
        Prog { est_total: 1.0, est_done: 0.0, step_est: 0.0, step_clock: Instant::now(), step_text: "Starting".into() }
    }
}

/// Where the run is: the fraction done, what runs now, time so far and an estimate of the rest.
#[derive(Clone, Debug, Default)]
pub struct BenchStatus {
    pub fraction: f64,
    pub text: String,
    pub elapsed: Duration,
    pub remaining: Duration,
    pub results: usize,
}

/// A route: from one drive to another (or within one), its sizes, cases and estimate.
#[derive(Clone, Debug, Default)]
pub struct Route {
    pub from: usize,
    pub to: usize,
    /// The big file.
    pub big: i64,
    /// How many small files.
    pub small: usize,
    pub cases: Vec<BenchCase>,
    /// Seconds, with what is known of the drives.
    pub est: f64,
}

impl Route {
    pub fn within(&self) -> bool {
        self.from == self.to
    }
}

const FILE_READ_ONLY_VOLUME: u32 = 0x00080000;

const BUS_NAMES: [&str; 20] = [
    "", "SCSI", "ATAPI", "ATA", "FireWire", "SSA", "Fibre Channel", "USB", "RAID", "iSCSI", "SAS", "SATA", "SD card", "MMC", "Virtual",
    "Virtual (file)", "Storage Spaces", "NVMe", "SCM", "UFS",
];

// Sizes go by the route's speed in three steps (not a continuous scale): the same PC gets the same sizes every time, so
// runs of different versions compare. Quick: 2 GB + 1,000 small files where it goes at 350 MB/s or more, 1 GB + 500
// below that, 512 MB + 250 under 40 MB/s. Thorough: 4 GB / 2 GB / 1 GB and 2,000 / 1,000 / 500 small files, a mixed
// folder, three rounds. A big file is big: a copy of a few seconds is mostly its start and where on the platter it
// lands (512 MB within a hard disk moved by up to 20% from one run to the next).
const FAST_MBPS: f64 = 350.0;
const SLOW_MBPS: f64 = 40.0;
const MIXED_BIG: [i64; 3] = [96 * MB, 160 * MB, 256 * MB];
const MIXED_SMALL: usize = 400;

impl Benchmark {
    pub fn new() -> Benchmark {
        Benchmark { report: Mutex::new(BenchReport::new()), ..Default::default() }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, SeqCst);
    }
    pub fn is_canceled(&self) -> bool {
        self.cancel.load(SeqCst)
    }
    fn canceled(&self) -> bool {
        self.cancel.load(SeqCst)
    }

    fn rounds(&self) -> i32 {
        if self.rounds_override > 0 { self.rounds_override } else { default_rounds(self.thorough) }
    }

    // ================= drives =================

    /// Every drive letter with what it is, and why it isn't tested when it isn't.
    pub fn find_drives(network: bool, only: Option<&HashSet<String>>) -> Vec<BenchDrive> {
        let mut list = Vec::new();
        let sys_letter = std::env::var("SystemRoot").ok().and_then(|s| s.get(..2).map(|x| x.to_uppercase())).unwrap_or_else(|| "C:".into());
        let mut seen: HashMap<String, String> = HashMap::new();
        for root in crate::cli::drive_roots() {
            let r = root.to_string_lossy();
            let mut d = BenchDrive::new(&r[..2].to_uppercase(), &r);
            d.system = d.letter == sys_letter;
            Self::look_at(&mut d, &root, network, only, &mut seen);
            list.push(d);
        }
        for d in list.iter_mut() {
            d.found_skip = d.skip.clone();
        }
        list
    }

    fn look_at(d: &mut BenchDrive, root: &WString, network: bool, only: Option<&HashSet<String>>, seen: &mut HashMap<String, String>) {
        let ty = sys::get_drive_type(root);
        if ty == sys::DRIVE_REMOTE {
            d.network = true;
            d.remote = sys::wnet_remote_name(&d.letter);
            d.model = d.remote.clone();
            if !network {
                d.skip = Some("a network drive - turn on Network drives to include it".into());
                return;
            }
        }
        if ty == sys::DRIVE_CDROM {
            d.skip = Some("a disc drive".into());
            return;
        }
        let Some(vi) = sys::get_volume_information(root, true) else {
            d.skip = Some("not ready (no disk in it, or locked)".into());
            return;
        };
        d.label = Some(vi.label.to_string_lossy());
        d.file_system = Some(vi.file_system.to_string_lossy());
        if let Some((total, free)) = sys::disk_space(root) {
            d.size = total as i64;
            d.free = free as i64;
        }
        if let Some(o) = only
            && !o.contains(&d.letter)
        {
            d.skip = Some("not chosen for this run".into());
            return;
        }
        let vol = volume_probe::probe(&paths::to_long(root));
        if !d.network {
            Self::read_model(d);
        } else {
            d.model = d.remote.clone();
        }
        let flags = vol.fs_flags;
        let kind = vol.kind;
        let id = vol.id.to_string_lossy();
        d.vol = Some(vol);
        if flags & FILE_READ_ONLY_VOLUME != 0 {
            d.skip = Some("read-only".into());
            return;
        }
        // a cloud drive (Google Drive, for one) shows up as a fixed drive: it is no disk, and what is written there is uploaded
        if !d.network && !d.real_disk && kind != DeviceKind::Ram {
            d.skip = Some("not a disk of this PC (a virtual or cloud drive)".into());
            return;
        }
        if !id.is_empty() {
            if let Some(other) = seen.get(&id) {
                d.skip = Some(format!("the same drive as {other}"));
                return;
            }
            seen.insert(id, d.letter.clone());
        }
    }

    /// The disk's model and bus from the drive itself (no administrator rights needed).
    fn read_model(d: &mut BenchDrive) {
        let dev = WString::from(format!(r"\\.\{}", d.letter));
        let Ok(h) = sys::create_file(&dev, 0, sys::FILE_SHARE_READ | sys::FILE_SHARE_WRITE, sys::OPEN_EXISTING, 0) else { return };
        // a disk answers as a disk device (a cloud drive's volume answers as a file system) and describes itself
        let disk = volume_probe::StorageDeviceNumber::query(&h).map(|dn| volume_probe::disk_number(dn) >= 0).unwrap_or(false);
        let mut q = [0u8; 12]; // STORAGE_PROPERTY_QUERY: StorageDeviceProperty, PropertyStandardQuery
        q[0..4].copy_from_slice(&0i32.to_le_bytes());
        let mut buf = vec![0u8; 1024];
        let (ok, ret) = sys::device_io_control(&h, sys::IOCTL_STORAGE_QUERY_PROPERTY, &q, &mut buf);
        let ret = ret as usize;
        if !ok || ret < 32 {
            return;
        }
        let rd = |at: usize| i32::from_le_bytes(buf[at..at + 4].try_into().unwrap());
        let (vendor, product, bus) = (rd(12), rd(16), rd(28));
        let model = format!("{} {}", ascii(&buf, vendor, ret), ascii(&buf, product, ret)).trim().to_string();
        d.model = (!model.is_empty()).then_some(model);
        d.bus = (bus > 0 && (bus as usize) < BUS_NAMES.len()).then(|| BUS_NAMES[bus as usize].to_string());
        d.real_disk = disk;
    }

    // ================= the plan =================

    /// System drive to each other drive and back, and within each drive (the system drive first).
    pub fn plan_routes(usable: &[usize], drives: &[BenchDrive]) -> Vec<Route> {
        let mut r = Vec::new();
        let Some(hub) = usable.iter().copied().find(|&i| drives[i].system).or_else(|| usable.first().copied()) else { return r };
        r.push(Route { from: hub, to: hub, ..Default::default() });
        let mut others: Vec<usize> = usable.iter().copied().filter(|&i| i != hub).collect();
        others.sort_by(|&a, &b| (drives[a].network, &drives[a].letter).cmp(&(drives[b].network, &drives[b].letter)));
        for o in others {
            r.push(Route { from: hub, to: o, ..Default::default() });
            r.push(Route { from: o, to: hub, ..Default::default() });
            r.push(Route { from: o, to: o, ..Default::default() });
        }
        r
    }

    fn budget_seconds(&self) -> f64 {
        if self.thorough { 45.0 * 60.0 } else { 15.0 * 60.0 }
    }

    /// Speed class of a route: 0 = fast, 1 = medium, 2 = slow.
    fn speed_class(r: &Route, d: &[BenchDrive]) -> i32 {
        let sp = big_mbps(r, d);
        if sp >= FAST_MBPS {
            0
        } else if sp >= SLOW_MBPS {
            1
        } else {
            2
        }
    }

    fn big_for(&self, step: i32) -> i64 {
        if self.tiny {
            64 * MB
        } else {
            (if self.thorough { 4096 * MB } else { 2048 * MB }) >> step.min(3)
        }
    }
    fn small_for(&self, step: i32) -> usize {
        if self.tiny { 100 } else { 125.max((if self.thorough { 2000 } else { 1000 }) >> step.min(3)) }
    }

    /// Space a drive needs at most: one source and one copy of everything, and room to spare.
    fn need_bytes(&self) -> i64 {
        2 * (self.big_for(0)
            + files::small_sizes(self.small_for(0)).iter().sum::<i64>()
            + if self.thorough { MIXED_BIG.iter().sum::<i64>() + files::small_sizes(MIXED_SMALL).iter().sum::<i64>() } else { 0 })
            + 1024 * MB
    }

    /// Sizes per route from its speed class (one step smaller for every route when that is over the time budget);
    /// the estimate of each. The budget counts the default runs: more runs take longer, with the same files.
    pub fn size_routes(&self, routes: &mut [Route], d: &[BenchDrive]) {
        for down in 0..=1 {
            for r in routes.iter_mut() {
                let step = Self::speed_class(r, d) + down;
                r.big = self.big_for(step);
                r.small = self.small_for(step);
            }
            if self.tiny || routes.iter().map(|x| self.estimate(x, d, default_rounds(self.thorough))).sum::<f64>() <= self.budget_seconds() {
                break;
            }
        }
        for r in routes.iter_mut() {
            r.cases.clear();
            r.cases.push(BenchCase { key: "big".into(), big: vec![r.big], smalls: vec![] });
            r.cases.push(BenchCase { key: "small".into(), big: vec![], smalls: files::small_sizes(r.small) });
            if self.thorough {
                r.cases.push(BenchCase { key: "mixed".into(), big: MIXED_BIG.to_vec(), smalls: files::small_sizes(MIXED_SMALL) });
            }
            if self.big_mb > 0 {
                r.cases[0].big = vec![self.big_mb as i64 * MB];
            }
            if let Some(c) = &self.cases {
                r.cases.retain(|x| c.contains(&x.key));
            }
            r.est = self.estimate(r, d, self.rounds());
        }
    }

    /// Seconds a route takes: its sources, then every run of every case with the flush, the delete and a pause.
    fn estimate(&self, r: &Route, d: &[BenchDrive], rounds: i32) -> f64 {
        let (f, t) = (&d[r.from], &d[r.to]);
        let sp = big_mbps(r, d);
        let slow = slow_drive(f) || slow_drive(t);
        let one = one_disk(r, d);
        let rtt = f.vol.as_ref().map(|v| v.rtt_ms).unwrap_or(0.0).max(t.vol.as_ref().map(|v| v.rtt_ms).unwrap_or(0.0));
        let per_file = if f.network || t.network {
            0.004 + 3.0 * rtt.max(0.0) / 1000.0
        } else if slow {
            if one { 0.007 } else { 0.004 }
        } else {
            0.0004
        };
        let tidy = (if slow { 0.75 } else { 0.25 }) + 0.4; // pause between runs + the other program's start, the warm-up
        let runs = 2 * rounds;
        let mut total = 0.0;
        let mut cases = vec![(r.big, 0usize), (files::small_sizes(r.small).iter().sum::<i64>(), r.small)];
        if self.thorough {
            cases.push((MIXED_BIG.iter().sum::<i64>() + files::small_sizes(MIXED_SMALL).iter().sum::<i64>(), MIXED_SMALL + MIXED_BIG.len()));
        }
        for (bytes, nfiles) in cases {
            let bytes = bytes as f64 / MB as f64;
            let make = bytes / f.write_mbps + bytes / f.read_mbps + nfiles as f64 * if slow_drive(f) { 0.005 } else { 0.0005 };
            let copy = bytes / sp + nfiles as f64 * per_file;
            let after = nfiles as f64 * if slow { 0.0015 } else { 0.0002 } + nfiles as f64 * 0.0002; // flush + delete; purge of the source
            total += make + runs as f64 * (copy + after + tidy);
        }
        total
    }

    fn probe_seconds(&self, d: &BenchDrive) -> f64 {
        let size = self.probe_size(d) as f64 / MB as f64;
        size / d.write_mbps + 2.0 * size / d.read_mbps + 1.0
    }

    fn probe_size(&self, d: &BenchDrive) -> i64 {
        if self.tiny {
            32 * MB
        } else if d.network {
            64 * MB
        } else if d.vol.as_ref().is_some_and(|v| v.is_slow_seek()) {
            128 * MB
        } else {
            256 * MB
        }
    }

    /// The drives that would be tested and the whole run's seconds, before anything is written.
    pub fn preview(&self, drives: &mut [BenchDrive]) -> f64 {
        for d in drives.iter_mut() {
            d.skip = d.found_skip.clone();
        }
        let need = self.need_bytes();
        for d in drives.iter_mut().filter(|d| d.skip.is_none()) {
            assume(d);
            if d.free < need {
                d.skip = Some(format!("only {} free; the test needs {}", gb(d.free), gb(need)));
            }
        }
        let mut usable: Vec<usize> = (0..drives.len()).filter(|&i| drives[i].skip.is_none()).collect();
        if let Some(o) = &self.only {
            usable.retain(|&i| o.contains(&drives[i].letter)); // the drives chosen (the others can still be)
        }
        let mut routes = Self::plan_routes(&usable, drives);
        self.size_routes(&mut routes, drives);
        routes.iter().map(|r| r.est).sum::<f64>() + usable.iter().map(|&i| self.probe_seconds(&drives[i])).sum::<f64>()
    }

    // ================= progress =================

    fn step(&self, text: &str, est: f64) {
        {
            let mut p = lock(&self.prog);
            p.est_done += p.step_est;
            p.step_est = est.max(0.0);
            p.step_text = text.into();
            p.step_clock = Instant::now();
        }
        if let Some(l) = &self.log {
            l(&format!("{}s {text}", super::report::inv(self.clock.elapsed().as_secs_f64(), 1, 1)));
        }
    }

    /// Where the run is: the fraction done, what runs now, time so far and an estimate of the rest.
    pub fn status(&self) -> BenchStatus {
        let p = lock(&self.prog);
        let in_step = p.step_clock.elapsed().as_secs_f64().min(p.step_est * 0.95);
        let now = p.est_done + in_step;
        let frac = (now / p.est_total.max(1.0)).clamp(0.0, 1.0);
        let elapsed = self.clock.elapsed();
        // the estimate learns from how long the steps so far took against their guesses
        let pace = if now > 20.0 { (elapsed.as_secs_f64() / now).clamp(0.5, 3.0) } else { 1.0 };
        BenchStatus {
            fraction: frac,
            text: p.step_text.clone(),
            elapsed,
            remaining: Duration::from_secs_f64(((p.est_total - now) * pace).max(0.0)),
            results: lock(&self.report).results.len(),
        }
    }

    /// The cases finished so far (a copy of the list: the run adds to it from its own thread).
    pub fn results_so_far(&self) -> Vec<BenchResult> {
        lock(&self.report).results.clone()
    }

    /// The report as it is now (the window's state, tests).
    pub fn report(&self) -> BenchReport {
        lock(&self.report).clone()
    }

    // ================= the run =================

    /// Runs everything (on the calling thread) and returns the report; the test files are gone after it.
    pub fn run(&self) -> BenchReport {
        self.clock.start();
        {
            let mut r = lock(&self.report);
            r.started = Stamp::now();
            r.mode = if self.tiny {
                "tiny"
            } else if self.thorough {
                "thorough"
            } else {
                "quick"
            }
            .into();
            r.rounds = self.rounds();
            r.read_machine();
        }
        sys::set_thread_execution_state(sys::ES_CONTINUOUS | sys::ES_SYSTEM_REQUIRED); // no sleep while it runs
        let held = files::lock();
        let mut drives: Vec<BenchDrive> = Vec::new();
        let r = (|| -> Res<()> {
            if held.is_none() {
                return Err(BenchErr::Error("Another Benchmark is running (in another Choppy window or on the command line).".into()));
            }
            self.step("Removing what an earlier run left behind", 1.0);
            files::clean_leftovers();
            self.step("Looking at the drives", 2.0);
            drives = Self::find_drives(self.network, self.only.as_ref());
            lock(&self.report).drives = Some(drives.clone());
            for d in drives.iter_mut().filter(|d| d.skip.is_none()) {
                assume(d);
            }
            let est = self.preview(&mut drives) + 3.0;
            lock(&self.prog).est_total = est;
            let usable: Vec<usize> = (0..drives.len()).filter(|&i| drives[i].skip.is_none()).collect();
            // a temporary folder per drive, then each drive's own speed (and whether a file can be dropped from the cache there)
            for &i in usable.iter() {
                if self.canceled() {
                    return Err(BenchErr::Canceled);
                }
                let d = &mut drives[i];
                match files::make_folder(Path::new(&d.root), d.system) {
                    Ok(dir) => d.test_dir = Some(dir),
                    Err(why) => d.skip = Some(format!("Choppy can't make a folder there ({why})")),
                }
                lock(&self.report).drives = Some(drives.clone());
            }
            let usable: Vec<usize> = usable.into_iter().filter(|&i| drives[i].skip.is_none()).collect();
            for &i in usable.iter() {
                let (l, k) = (drives[i].letter.clone(), drives[i].kind());
                self.step(&format!("Measuring {l} ({k})"), self.probe_seconds(&drives[i]));
                self.measure_drive(&mut drives[i])?;
                lock(&self.report).drives = Some(drives.clone());
            }
            if usable.is_empty() {
                return Err(BenchErr::Error("No drive of this PC can be tested (each one's reason is listed).".into()));
            }
            let mut routes = Self::plan_routes(&usable, &drives);
            if let Some(ro) = &self.routes_only {
                routes.retain(|r| ro.contains(&format!("{}>{}", &drives[r.from].letter[..1], &drives[r.to].letter[..1])));
            }
            self.size_routes(&mut routes, &drives);
            {
                let mut p = lock(&self.prog);
                p.est_total = p.est_done + p.step_est + routes.iter().map(|r| r.est).sum::<f64>() + usable.len() as f64 * 1.5;
            }
            lock(&self.report).planned = routes.iter().map(|r| r.cases.len()).sum();
            if let Some(l) = &self.log {
                let plan: Vec<String> =
                    routes.iter().map(|r| format!("{}>{} {} MB + {} files", drives[r.from].letter, drives[r.to].letter, r.big / MB, r.small)).collect();
                l(&format!("plan: {}; about {}", plan.join(", "), super::report::text::duration(routes.iter().map(|r| r.est).sum())));
            }
            let mut copy_no = 0;
            for (i, r) in routes.iter().enumerate() {
                self.run_route(r, i + 1, &drives, &mut copy_no)?;
            }
            Ok(())
        })();
        {
            let mut rep = lock(&self.report);
            match r {
                Err(BenchErr::Canceled) => rep.canceled = true,
                Err(BenchErr::Error(e)) => rep.error = Some(e),
                Ok(()) => {}
            }
        }
        self.step("Removing the test files", 3.0);
        let mut notes = Vec::new();
        for d in drives.iter() {
            if let Some(dir) = &d.test_dir
                && !files::remove(dir)
            {
                notes.push(format!("Some test files on {} couldn't be removed yet ({}); Choppy removes them at its next start.", d.letter, dir.display()));
            }
        }
        drop(held);
        sys::set_thread_execution_state(sys::ES_CONTINUOUS);
        self.clock.stop();
        let mut rep = lock(&self.report);
        if !drives.is_empty() {
            rep.drives = Some(drives);
        }
        rep.notes.extend(notes);
        rep.seconds = self.clock.elapsed().as_secs_f64();
        rep.clone()
    }

    /// Writes a file past the cache (its write speed), reads it twice (the disk, then the cache), drops it from the
    /// cache and reads it again.
    fn measure_drive(&self, d: &mut BenchDrive) -> Res<()> {
        let dir = d.test_dir.clone().unwrap_or_default();
        let f = dir.join("Drive speed.bin");
        let size = self.probe_size(d);
        let canceled = || self.canceled();
        let r = (|| -> Res<()> {
            // a drive that sleeps (a USB drive, a hard disk that spun down) wakes first: measured here, E: answered at
            // 41 MB/s for its first seconds and copied at 440-900 MB/s after
            let wake = dir.join("Wake up.bin");
            files::write_big(&wake, 8 * MB, &canceled, None)?;
            files::purge(std::slice::from_ref(&wake), &canceled)?;
            self.read_speed(&wake, 8 * MB)?;
            let _ = std::fs::remove_file(&wake);
            let sw = Instant::now();
            files::write_big(&f, size, &canceled, None)?;
            d.write_mbps = size as f64 / MB as f64 / sw.elapsed().as_secs_f64().max(0.001);
            let disk = self.read_speed(&f, size)?;
            let cached = self.read_speed(&f, size)?;
            files::purge(std::slice::from_ref(&f), &canceled)?;
            let again = self.read_speed(&f, size)?;
            d.read_mbps = disk;
            // dropped from the cache: the third read is back near the disk's speed. When the cache was no faster than
            // the drive there is nothing to tell: the copies then start from a freshly written source each time anyway
            d.purge_ok = cached > disk * 1.5 && again < (disk + cached) / 2.0;
            d.purge_note = Some(if cached <= disk * 1.5 {
                "unclear"
            } else if d.purge_ok {
                "yes"
            } else {
                "no"
            }
            .into());
            if d.purge_ok {
                d.read_mbps = disk.max(again); // both came from the disk: the better one
            }
            d.measured = true;
            Ok(())
        })();
        let _ = std::fs::remove_file(&f);
        r
    }

    fn read_speed(&self, f: &Path, size: i64) -> Res<f64> {
        use std::io::Read;
        let mut buf = vec![0u8; 1 << 20];
        let sw = Instant::now();
        let mut fh = sys::open_sequential(f).map_err(|e| BenchErr::Error(files::io_message(&e)))?;
        loop {
            let n = fh.read(&mut buf).map_err(|e| BenchErr::Error(files::io_message(&e)))?;
            if n == 0 {
                break;
            }
            if self.canceled() {
                return Err(BenchErr::Canceled);
            }
        }
        Ok(size as f64 / MB as f64 / sw.elapsed().as_secs_f64().max(0.001))
    }

    fn run_route(&self, r: &Route, no: usize, d: &[BenchDrive], copy_no: &mut i32) -> Res<()> {
        let (from, to) = (&d[r.from], &d[r.to]);
        let src_dir = from.test_dir.clone().unwrap_or_default().join(format!("Source {no}"));
        std::fs::create_dir_all(&src_dir).map_err(|e| BenchErr::Error(files::io_message(&e)))?;
        // a tiny file each program copies first, untimed (the first copy of a process loads what it needs)
        let warm = src_dir.join("Warm-up.bin");
        std::fs::write(&warm, vec![0u8; 64 << 10]).map_err(|e| BenchErr::Error(files::io_message(&e)))?;
        for (case_no, c) in r.cases.iter().enumerate() {
            if self.canceled() {
                return Err(BenchErr::Canceled);
            }
            let mut res = BenchResult { from: from.clone(), to: to.clone(), from_i: r.from, to_i: r.to, case: c.clone(), ..Default::default() };
            let label = format!("{} → {} · {}", from.letter, to.letter, c.title());
            let out = (|| -> Res<()> {
                self.step(&format!("{label} · writing the test files"), make_seconds(r, c, d));
                let (mut src, mut list) = self.make_source(c, &src_dir)?;
                self.prime(&list)?;
                // A-B-B-A (-A-B): each one goes first and last as often, and the next case starts with the other one
                let rounds = self.rounds() as usize;
                let order: String = (if case_no % 2 == 0 { "CWWC" } else { "WCCW" }).repeat(rounds).chars().take(2 * rounds).collect();
                let per_run = (r.est - r.cases.iter().map(|x| make_seconds(r, x, d)).sum::<f64>()) / (2.0 * rounds as f64 * r.cases.len() as f64);
                let mut seen = [0; 2];
                for (k, ch) in order.chars().enumerate() {
                    let choppy = ch == 'C';
                    seen[if choppy { 0 } else { 1 }] += 1;
                    let nth = seen[if choppy { 0 } else { 1 }];
                    self.step(&format!("{label} · {}, run {nth} of {rounds}", if choppy { "Choppy" } else { "Windows" }), per_run.max(0.5));
                    // the source is read from the disk every time: dropped from the cache, or written again where that can't be done
                    if k > 0 {
                        if from.purge_ok {
                            files::purge(&list, &|| self.canceled())?;
                        } else {
                            files::delete_tree(&paths::to_long(&WString::from(src.display().to_string())));
                            let (s2, l2) = self.make_source(c, &src_dir)?;
                            src = s2;
                            list = l2;
                            self.prime(&list)?;
                        }
                    }
                    sys::sleep_ms(if slow_drive(from) || slow_drive(to) { 750 } else { 250 }); // the disks settle (what the last delete left to write)
                    *copy_no += 1;
                    let run = self.copy_once(to, c, choppy, &src, &warm, &mut res, *copy_no)?;
                    res.runs.push(run);
                }
                res.score();
                Ok(())
            })();
            match out {
                Err(BenchErr::Canceled) => {
                    self.clean_source(&src_dir, &warm);
                    return Err(BenchErr::Canceled);
                }
                Err(BenchErr::Error(e)) => res.error = Some(e),
                Ok(()) => {}
            }
            self.clean_source(&src_dir, &warm);
            lock(&self.report).results.push(res);
        }
        files::delete_tree(&paths::to_long(&WString::from(src_dir.display().to_string())));
        Ok(())
    }

    fn clean_source(&self, src_dir: &Path, warm: &Path) {
        if let Ok(rd) = std::fs::read_dir(src_dir) {
            for e in rd.flatten() {
                let p = e.path();
                if !p.as_os_str().eq_ignore_ascii_case(warm.as_os_str()) {
                    files::delete_tree(&paths::to_long(&WString::from(p.display().to_string())));
                }
            }
        }
    }

    /// The case's source in the route's source folder: a file (one big file) or a folder. Returns it and its files.
    fn make_source(&self, c: &BenchCase, src_dir: &Path) -> Res<(PathBuf, Vec<PathBuf>)> {
        let canceled = || self.canceled();
        let mut list = Vec::new();
        if c.key == "big" {
            let src = src_dir.join("Big file.bin");
            files::write_big(&src, c.big[0], &canceled, None)?;
            list.push(src.clone());
            return Ok((src, list));
        }
        let src = src_dir.join(if c.key == "small" { "Small files" } else { "Mixed folder" });
        std::fs::create_dir_all(&src).map_err(|e| BenchErr::Error(files::io_message(&e)))?;
        for (i, &b) in c.big.iter().enumerate() {
            let f = src.join(format!("Video {}.bin", i + 1));
            files::write_big(&f, b, &canceled, None)?;
            list.push(f);
        }
        list.extend(files::write_small(&src, &c.smalls, &canceled, None)?);
        Ok((src, list))
    }

    /// A new file's first open is slower (a virus scanner reads it then): every source file is read once, untimed, and
    /// then dropped from the cache, so every run - the first one too - starts from the same state. Measured before:
    /// the first of four runs was up to 4x slower (1 GB within the NVMe: 2.22 s, then 0.52 s).
    fn prime(&self, list: &[PathBuf]) -> Res<()> {
        use std::io::Read;
        let mut buf = vec![0u8; 1 << 20];
        for f in list {
            if self.canceled() {
                return Err(BenchErr::Canceled);
            }
            let mut fh = sys::open_sequential(f).map_err(|e| BenchErr::Error(files::io_message(&e)))?;
            while fh.read(&mut buf).map_err(|e| BenchErr::Error(files::io_message(&e)))? > 0 {}
        }
        files::purge(list, &|| self.canceled())
    }

    /// One timed copy into a fresh folder on the destination, then flushed (the time to the disk), checked and deleted.
    fn copy_once(&self, to: &BenchDrive, c: &BenchCase, choppy: bool, src: &Path, warm: &Path, res: &mut BenchResult, n: i32) -> Res<BenchRun> {
        let tdir = to.test_dir.clone().unwrap_or_default();
        let dest = tdir.join(format!("Copy {n}"));
        let warm_dest = tdir.join(format!("Warm-up {n}"));
        let r = (|| -> Res<BenchRun> {
            std::fs::create_dir_all(&dest).map_err(|e| BenchErr::Error(files::io_message(&e)))?;
            std::fs::create_dir_all(&warm_dest).map_err(|e| BenchErr::Error(files::io_message(&e)))?;
            let done = if choppy {
                self.choppy_copy(warm, &warm_dest)?;
                let (secs, m) = self.choppy_copy(src, &dest)?;
                if m.is_some() {
                    res.method = m;
                }
                secs
            } else {
                shell_copy::run(Some(warm), Some(&warm_dest), src, &dest, &|| self.canceled())?
            };
            let copies = files::list(&dest);
            let sw = Instant::now();
            let paths_: Vec<PathBuf> = copies.iter().map(|x| x.0.clone()).collect();
            files::flush(&paths_, &|| self.canceled())?;
            let flush = sw.elapsed().as_secs_f64();
            let bytes: i64 = copies.iter().map(|x| x.1).sum();
            if copies.len() != c.files() || bytes != c.bytes() {
                return Err(BenchErr::Error(format!(
                    "{} copy has {} of {} files, {} of {} bytes",
                    if choppy { "Choppy's" } else { "Windows'" },
                    copies.len(),
                    c.files(),
                    bytes,
                    c.bytes()
                )));
            }
            Ok(BenchRun { choppy, done, on_disk: done + flush })
        })();
        files::delete_tree(&paths::to_long(&WString::from(dest.display().to_string())));
        files::delete_tree(&paths::to_long(&WString::from(warm_dest.display().to_string())));
        r
    }

    /// Choppy's engine as any copy runs it (Adaptive picks the way; the pipeline and its tuner as usual). Seconds until
    /// it says done, and the way it picked.
    fn choppy_copy(&self, src: &Path, dest: &Path) -> Res<(f64, Option<String>)> {
        let mut spec = JobSpec {
            op: OpKind::Copy,
            origin: "Benchmark".into(),
            policy: ConflictPolicy::Skip,
            dest: paths::to_long(&WString::from(dest.display().to_string())),
            ..JobSpec::new(OpKind::Copy)
        };
        spec.sources.push(paths::to_long(&WString::from(src.display().to_string())));
        if let Some(t) = &self.tweak {
            t(&mut spec);
        }
        let mut job = CopyJob::new(Arc::new(spec));
        job.ask_conflict = Some(Box::new(|_, _| Some(ConflictAnswer { choice: ConflictPolicy::Skip, apply_to_all: false, cancel_job: false })));
        job.ask_low_space = Some(Box::new(|_, _, _| false));
        job.ask_elevate = Some(Box::new(|_| false));
        let job = Arc::new(job);
        let done: Arc<(Mutex<Option<f64>>, std::sync::Condvar)> = Arc::new((Mutex::new(None), std::sync::Condvar::new()));
        let sw = Arc::new(Mutex::new(Instant::now()));
        {
            let done = done.clone();
            let sw = sw.clone();
            job.on_finished(Arc::new(move |_| {
                let secs = lock(&sw).elapsed().as_secs_f64();
                *lock(&done.0) = Some(secs);
                done.1.notify_all();
            }));
        }
        *lock(&sw) = Instant::now();
        job.start();
        let secs = loop {
            let g = lock(&done.0);
            if let Some(s) = *g {
                break s;
            }
            let g = crate::engine::wait_timeout(&done.1, g, Duration::from_millis(50));
            if let Some(s) = *g {
                break s;
            }
            drop(g);
            if self.canceled() {
                job.cancel();
            }
        };
        if self.canceled() {
            return Err(BenchErr::Canceled);
        }
        let method = job.plan().map(|p| {
            let tune = job.tune_summary().filter(|t| !t.is_empty());
            format!("{}{}", p.badge(), tune.map(|t| format!(" ({t})")).unwrap_or_default())
        });
        if job.phase() != JobPhase::Done || job.failed_files.load(SeqCst) > 0 || job.skipped_files.load(SeqCst) > 0 {
            let fi = lock(&job.failed_items).first().and_then(|f| f.error());
            let why = fi.or_else(|| job.status()).unwrap_or_else(|| job.phase().name().to_string());
            return Err(BenchErr::Error(format!("Choppy's copy didn't finish: {why}")));
        }
        Ok((secs, method))
    }
}

fn ascii(b: &[u8], at: i32, len: usize) -> String {
    if at <= 0 || at as usize >= len {
        return String::new();
    }
    let at = at as usize;
    let mut end = at;
    while end < len && b[end] != 0 {
        end += 1;
    }
    String::from_utf8_lossy(&b[at..end]).trim().to_string()
}

/// Both ends on one disk with heads: every byte is read and written by them.
fn one_disk(r: &Route, d: &[BenchDrive]) -> bool {
    (r.within() || volume_probe::same_disk(d[r.from].vol.as_ref(), d[r.to].vol.as_ref())) && (slow_drive(&d[r.from]) || slow_drive(&d[r.to]))
}

/// MB/s a big file can go at on this route: the slower end, both shared on one hard disk, or an SSD that reads and
/// writes at once.
fn big_mbps(r: &Route, d: &[BenchDrive]) -> f64 {
    let (rd, wr) = (d[r.from].read_mbps, d[r.to].write_mbps);
    if one_disk(r, d) {
        return 1.0 / (1.0 / rd + 1.0 / wr);
    }
    if r.within() {
        return rd.min(wr) * 0.7;
    }
    rd.min(wr)
}

fn slow_drive(d: &BenchDrive) -> bool {
    d.network || d.vol.as_ref().is_some_and(|v| v.is_slow_seek())
}

fn make_seconds(r: &Route, c: &BenchCase, d: &[BenchDrive]) -> f64 {
    let f = &d[r.from];
    let mb = c.bytes() as f64 / MB as f64;
    mb / f.write_mbps + mb / f.read_mbps + c.files() as f64 * if slow_drive(f) { 0.006 } else { 0.0006 } + 0.2
}

/// Speeds assumed before a drive is measured (only for the first estimate).
fn assume(d: &mut BenchDrive) {
    if d.measured {
        return;
    }
    let k = d.kind();
    let (r, w) = if d.network {
        (60.0, 40.0)
    } else if k == "NVMe SSD" {
        (2000.0, 1500.0)
    } else if k == "USB SSD" {
        (350.0, 300.0)
    } else if k.ends_with("SSD") {
        (450.0, 400.0)
    } else if k.contains("hard disk") || k == "Hard disk" {
        (170.0, 170.0)
    } else if k == "USB drive" {
        (100.0, 60.0)
    } else if k == "RAM disk" {
        (4000.0, 4000.0)
    } else {
        (300.0, 250.0)
    };
    d.read_mbps = r;
    d.write_mbps = w;
}

fn gb(b: i64) -> String {
    let v = b as f64 / (1u64 << 30) as f64;
    format!("{} GB", if b < (10i64 << 30) { super::report::inv(v, 1, 1) } else { super::report::inv(v, 0, 0) })
}

/// The time decides: saved = how much less time Choppy took than Windows, as a share of Windows' time (their average
/// times; below 0 = Choppy took longer). +1 = Faster, -1 = Slower, 0 = a Marginal Tie (less than 4% apart).
pub fn judge(choppy_secs: &[f64], windows_secs: &[f64]) -> (i32, f64) {
    if choppy_secs.is_empty() || windows_secs.is_empty() {
        return (0, 0.0);
    }
    let (c, w) = (mean(choppy_secs), mean(windows_secs));
    if c <= 0.0 || w <= 0.0 {
        return (0, 0.0);
    }
    let saved = (w - c) / w;
    if saved.abs() < TIE_MARGIN {
        return (0, saved);
    }
    (if saved > 0.0 { 1 } else { -1 }, saved)
}

/// Enumerable.Average (0 for none).
pub fn mean(x: &[f64]) -> f64 {
    if x.is_empty() { 0.0 } else { x.iter().sum::<f64>() / x.len() as f64 }
}

/// The drives a run would use, for tests (BenchChecks build them by hand).
pub fn test_drive(letter: &str, system: bool, kind: DeviceKind, disk: i32, rd: f64, wr: f64) -> BenchDrive {
    let mut d = BenchDrive::new(letter, &format!("{letter}\\"));
    d.system = system;
    d.measured = true;
    d.read_mbps = rd;
    d.write_mbps = wr;
    d.file_system = Some("NTFS".into());
    d.vol = Some(crate::engine::volume_probe::VolumeInfo {
        id: WString::from(format!("vol:{letter}")),
        kind,
        disk,
        file_system: Some("NTFS".into()),
        ..Default::default()
    });
    d.bus = Some(if kind == DeviceKind::Ssd { "NVMe".into() } else { "SATA".into() });
    d
}
