//! MainWindow.xaml / .cs: the copy window. Compact view (from / to, bar, sizes, times, current file) or extended
//! view (badge, speed graph, current file, largest files, quick toggles, queue). Refreshed every 100 ms straight
//! from the engine. Color code: blue = speed right now, lavender = average of this copy, green = done, peach =
//! total, amber = time.
//!
//! The C# sets element properties in Tick() and the XAML shows them; here Tick() fills `V` (the same properties)
//! and `build` makes the XAML's tree from it.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::Ordering::SeqCst;
use std::time::{Duration, Instant};

use crate::engine::copy_job::CopyJob;
use crate::engine::model::{JobPhase, OpKind};
use crate::engine::paths;
use crate::engine::settings::{Settings, app_paths};
use crate::engine::usb_link;
use crate::engine::volume_probe::VolumeInfo;
use crate::job_manager::JobManager;
use crate::sys::gfx::{Argb, Image, TextAlign};
use crate::sys::win::{self, Spec};
use crate::ui::controls::{PathText, SpeedGraph, donut, drive_icon};
use crate::ui::format;
use crate::ui::host::{Content, Host, Place, Sizing, T_USER, Window};
use crate::ui::kit::*;
use crate::ui::layout::*;
use crate::ui::theme::*;
use crate::wstr::WString;

// control ids
const SETTINGS: u32 = 1;
const CLOSE: u32 = 2;
const UNIT: u32 = 3;
const TG_CLOSE: u32 = 4;
const TG_VERIFY: u32 = 5;
const TG_LIMIT: u32 = 6;
const LIMIT_BOX: u32 = 7;
const PAUSE: u32 = 8;
const SKIP: u32 = 9;
const CANCEL: u32 = 10;
const QUEUE: u32 = 11;
const VIEW: u32 = 12;
const OPEN: u32 = 13;
const RETRY: u32 = 14;
const LOG: u32 = 15;

const T_TICK: usize = T_USER + 1;
const T_LIFT: usize = T_USER + 2;

pub const WIDTH: f64 = 576.0;

const SPLIT: [Argb; 4] = [0xFF4FC3F7, 0xFF7FE3D4, 0xFFB79CFF, 0xFFF5C542];
const OTHER: Argb = 0xFF5F6772;

pub type MainWindow = Window<MainContent>;

/// What Tick() puts on screen (the XAML's named elements' properties).
#[derive(Default, Clone)]
struct V {
    title_extra: String,
    from_name: String,
    from_warn: Option<String>,
    from_tip: Option<String>,
    from_path: String,
    from_server: bool,
    to_name: String,
    to_warn: Option<String>,
    to_tip: Option<String>,
    to_path: String,
    to_server: bool,
    badge_mode: String,
    badge_detail: String,
    badge_tip: Option<String>,
    badge_warn: bool,
    percent: String,
    frac: f64,
    sp_num: String,
    sp_unit: String,
    sp_avg_num: String,
    sp_avg_unit: String,
    c_now: String,
    c_avg_label: String,
    c_avg: String,
    size_done: String,
    size_of: String,
    size_total: String,
    files_text: String,
    elapsed_label: String,
    elapsed_val: String,
    remain_label: String,
    remain_val: String,
    cur_label: String,
    cur_name: String,
    cur_name_tip: Option<String>,
    cur_done: String,
    cur_of: String,
    cur_total: String,
    /// CurBar.Width as a share of its track (1 = the whole track).
    cur_frac: f64,
    c2_label: String,
    c2_label_c: Argb,
    c2_name: String,
    c2_name_c: Argb,
    pause_text: String,
    skip_enabled: bool,
    cancel_text: String,
    // report
    current_visible: bool,
    report_visible: bool,
    run_buttons: bool,
    done_buttons: bool,
    rep_label: String,
    rep_label_c: Argb,
    rep_title: String,
    chips: Vec<(String, Argb)>,
    rep_note: String,
    rep_note_visible: bool,
    rep_note_c: Argb,
    retry_visible: bool,
    log_visible: bool,
    open_visible: bool,
    // largest files
    legend: Vec<(Argb, String, i64)>,
    donut: Vec<(f64, Argb)>,
    // quick toggles
    tg_close: bool,
    tg_verify: bool,
    tg_limit: bool,
    limit_unit: String,
    limit_enabled: bool,
    queue_count: String,
}

pub struct MainContent {
    pub manager: Arc<JobManager>,
    /// The job on screen.
    job: Option<Arc<CopyJob>>,
    v: V,
    graph: SpeedGraph,
    limit: TextBox,
    icon: Option<(u32, Image)>,
    last_samples: i64,
    close_at: Option<Instant>,
    shown_cur: Option<WString>,
    shown_cur_at: Instant,
    report_shown: bool,
    route_labeled: bool,
    last_tray: Option<Instant>,
    split_shown: bool,
    problem_note: Option<String>,
    /// A finished job with problems stays on screen until it is dismissed, even when the next copy starts.
    dismissed: HashSet<usize>,
    loading_quick: bool,
    /// The speed-limit box has the keyboard focus (IsKeyboardFocusWithin): it keeps what is typed.
    limit_focused: bool,
    queue_popup: Option<Rc<RefCell<QueueWindow>>>,
    pub mouse_over: bool,
    /// The last time the window's height changed (KeepOnScreen after the view switch).
    keep_on_screen: bool,
    /// drawn once off-screen and closed (App's warm-up): not the app's copy window
    warm_up: bool,
}

fn key(j: &Arc<CopyJob>) -> usize {
    Arc::as_ptr(j) as usize
}

fn had_problems(j: &CopyJob) -> bool {
    j.is_finished() && (j.failed_files.load(SeqCst) > 0 || j.skipped_files.load(SeqCst) > 0 || j.phase() == JobPhase::Failed)
}

impl MainContent {
    pub fn new(manager: Arc<JobManager>) -> MainContent {
        let mut m = MainContent {
            manager,
            job: None,
            v: V { cancel_text: "Cancel".into(), pause_text: "Pause".into(), current_visible: true, run_buttons: true, ..Default::default() },
            graph: SpeedGraph::default(),
            limit: TextBox::new(6, true),
            icon: None,
            last_samples: -1,
            close_at: None,
            shown_cur: None,
            shown_cur_at: Instant::now(),
            report_shown: false,
            route_labeled: false,
            last_tray: None,
            split_shown: false,
            problem_note: None,
            dismissed: HashSet::new(),
            loading_quick: false,
            limit_focused: false,
            queue_popup: None,
            mouse_over: false,
            keep_on_screen: false,
            warm_up: false,
        };
        if crate::app::elevated() {
            m.v.title_extra = "· Administrator".into();
        }
        m.v.c2_label_c = DIM;
        m.v.c2_name_c = TEXT2;
        m.sync_quick();
        m
    }

    fn extended(&self) -> bool {
        Settings::current().extended
    }

    fn pinned(&self) -> bool {
        self.job.as_ref().is_some_and(|j| had_problems(j) && !self.dismissed.contains(&key(j)))
    }

    // ================= job binding =================

    pub fn on_manager_changed(&mut self, h: &mut Host) {
        let cur = self.manager.current().or_else(|| self.manager.last_finished());
        let same = match (&cur, &self.job) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same && !self.pinned() {
            self.show_job(cur);
        }
        self.refresh_queue();
        self.tick(h);
        self.update_close_button();
    }

    fn update_close_button(&mut self) {
        let Some(j) = &self.job else { return };
        if !j.is_finished() {
            return;
        }
        let next = self.manager.current();
        self.v.cancel_text = if next.as_ref().is_some_and(|n| !Arc::ptr_eq(n, j)) { "Next copy ›".into() } else { "Close".into() };
    }

    /// Report "Close": go on to the copy that is running now, or close the window.
    fn leave_report(&mut self, h: &mut Host) {
        if let Some(j) = &self.job {
            self.dismissed.insert(key(j));
        }
        let next = self.manager.current();
        match next {
            Some(n) if !self.job.as_ref().is_some_and(|j| Arc::ptr_eq(j, &n)) => {
                self.show_job(Some(n));
                self.tick(h);
            }
            _ => h.close(),
        }
    }

    // ---------- from / to ----------

    /// The folder the files come from (their common folder when there are several).
    fn source_folder(j: &CopyJob) -> String {
        let mut parents: Vec<WString> = Vec::new();
        for s in j.spec.sources.iter() {
            let sh = paths::to_short(s);
            let p = paths::parent(&sh).unwrap_or(sh);
            if !parents.iter().any(|q| q.eq_ignore_case(&p)) {
                parents.push(p);
            }
        }
        if parents.is_empty() {
            return String::new();
        }
        let mut common = Some(parents[0].clone());
        for p in parents.iter().skip(1) {
            while let Some(c) = &common {
                if paths::is_same_or_inside(p, c) {
                    break;
                }
                common = paths::parent(c);
            }
        }
        common.unwrap_or_else(|| parents[0].clone()).to_string_lossy()
    }

    /// "D: Media", "N: NAS", or the server of a \\server\share path.
    fn drive_name(short_path: &str, v: Option<&VolumeInfo>) -> String {
        if short_path.is_empty() {
            return "-".into();
        }
        if short_path.starts_with("\\\\") {
            return short_path.trim_start_matches('\\').split('\\').next().unwrap_or(short_path).to_string();
        }
        let b = short_path.as_bytes();
        let letter = if b.len() >= 2 && b[1] == b':' { short_path[..2].to_uppercase() } else { short_path.to_string() };
        match v.and_then(|v| v.label.clone()).filter(|l| !l.is_empty()) {
            Some(l) => format!("{letter} {l}"),
            None => letter,
        }
    }

    /// The drive's name, and after it a warning when its USB port (or cable, or a hub) holds it back: "H: Stick  ⚠
    /// USB 2.0, can do 5 Gbps". Returns what helps (for the tooltip), or None.
    fn show_drive(name: String, v: Option<&VolumeInfo>) -> (String, Option<String>, Option<String>) {
        let Some(link) = v.and_then(|v| v.usb.as_ref()).filter(|l| l.held_back()) else { return (name, None, None) };
        let warn = format!("{}, can do {}", usb_link::short(link.speed), usb_link::short(link.capable));
        (name, Some(warn), link.advice())
    }

    fn show_route(&mut self, j: &CopyJob) {
        let from = Self::source_folder(j);
        let to = paths::to_short(&j.spec.dest).to_string_lossy();
        let (fname, fwarn, flink) = Self::show_drive(Self::drive_name(&from, j.src_vol()), j.src_vol());
        let (tname, twarn, tlink) = Self::show_drive(Self::drive_name(&to, j.dst_vol()), j.dst_vol());
        self.v.from_name = fname;
        self.v.from_warn = fwarn;
        self.v.to_name = tname;
        self.v.to_warn = twarn;
        self.v.from_server = from.starts_with("\\\\") || j.src_vol().is_some_and(|v| v.is_network());
        self.v.to_server = to.starts_with("\\\\") || j.dst_vol().is_some_and(|v| v.is_network());
        let list: Vec<String> = j.spec.sources.iter().take(12).map(|s| paths::to_short(s).to_string_lossy()).collect();
        self.v.from_tip = Some(format!(
            "{}{}{}",
            flink.map(|l| l + "\n\n").unwrap_or_default(),
            list.join("\n"),
            if j.spec.sources.len() > 12 { "\n…" } else { "" }
        ));
        self.v.to_tip = tlink;
        self.v.from_path = from;
        self.v.to_path = to;
        self.route_labeled = j.src_vol().is_some() && j.dst_vol().is_some();
    }

    fn show_job(&mut self, j: Option<Arc<CopyJob>>) {
        self.job = j.clone();
        self.close_at = None;
        self.report_shown = false;
        self.last_samples = -1;
        self.graph.clear();
        self.shown_cur = None;
        let Some(j) = j else { return };
        self.show_route(&j);
        let mut extra = Vec::new();
        if j.spec.op == OpKind::Move {
            extra.push("Move");
        }
        if j.spec.elevated {
            extra.push("Administrator");
        }
        self.v.title_extra = if extra.is_empty() { String::new() } else { format!("· {}", extra.join(" · ")) };
        self.v.legend.clear();
        self.v.donut.clear();
        self.split_shown = false;
        self.v.current_visible = true;
        self.v.report_visible = false;
        self.v.run_buttons = true;
        self.v.done_buttons = false;
        self.v.cancel_text = "Cancel".into();
        self.v.pause_text = "Pause".into();
        self.v.c2_label_c = DIM;
        self.v.c2_name_c = TEXT2;
        self.sync_quick();
    }

    // ---------- largest files ----------

    fn fill_split(&mut self) {
        let Some(j) = self.job.clone() else { return };
        let Some(scan) = j.scan() else { return };
        if self.split_shown || matches!(j.phase(), JobPhase::Analyzing | JobPhase::Queued) {
            return;
        }
        self.split_shown = true;
        self.v.legend.clear();
        let mut parts = Vec::new();
        let mut top = 0i64;
        let mut i = 0;
        for (name, size) in scan.largest.iter() {
            if i >= 4 {
                break;
            }
            let c = SPLIT[i];
            parts.push((*size as f64, c));
            self.v.legend.push((c, name.to_string_lossy(), *size));
            top += size;
            i += 1;
        }
        let others = scan.files.len() as i64 - i as i64;
        let other_bytes = scan.total_bytes - top;
        if others > 0 {
            parts.push((other_bytes.max(1) as f64, OTHER));
            self.v.legend.push((OTHER, format!("Other {}{}", format::count(others), if others == 1 { " file" } else { " files" }), other_bytes));
        }
        if scan.files.is_empty() && j.plan().is_some_and(|p| p.instant_move) {
            self.v.legend.push((OTHER, "Renamed in place - nothing to copy".into(), -1));
        }
        self.v.donut = parts;
    }

    // ================= live refresh =================

    fn use_mbps(j: Option<&Arc<CopyJob>>) -> bool {
        let u = Settings::current().speed_unit.clone();
        match u.as_deref() {
            Some("mbps") => true,
            Some("mbs") => false,
            _ => j.is_some_and(|j| j.src_vol().is_some_and(|v| v.is_network()) || j.dst_vol().is_some_and(|v| v.is_network())),
        }
    }

    fn speed(bps: f64, mbps: bool) -> String {
        let (n, u) = format::speed(bps, mbps);
        format!("{n} {u}")
    }

    pub fn tick(&mut self, h: &mut Host) {
        let Some(j) = self.job.clone() else { return };
        let phase = j.phase();
        let finished = j.is_finished();
        if !self.route_labeled && j.src_vol().is_some() && j.dst_vol().is_some() {
            self.show_route(&j);
        }
        self.sync_quick();
        let v = &mut self.v;

        // badge: "Adaptive  1 stream"
        if j.as_admin() {
            v.badge_mode = "Administrator".into();
            v.badge_detail = "own window".into();
        } else if j.is_paused() {
            v.badge_mode = "Paused".into();
            v.badge_detail = String::new();
        } else if phase == JobPhase::Waiting {
            v.badge_mode = "Waiting".into();
            v.badge_detail = "for the network".into();
        } else if let Some(p) = j.plan() {
            v.badge_mode = p.mode().into();
            v.badge_detail = p.detail();
            v.badge_tip = Some(p.why());
        } else {
            v.badge_mode = "Adaptive".into();
            v.badge_detail = if phase == JobPhase::Queued { "waiting in the queue".into() } else { "checking the drives".into() };
        }
        v.badge_warn = j.is_paused() || phase == JobPhase::Waiting;

        // counters
        let total = (j.total_bytes.load(SeqCst) - j.skipped_bytes.load(SeqCst) - j.failed_bytes.load(SeqCst)).max(0);
        let done = j.done_bytes.load(SeqCst).min(total).max(0);
        let files_total = (j.total_files.load(SeqCst) - j.skipped_files.load(SeqCst) - j.failed_files.load(SeqCst)).max(0);
        let done_files = j.done_files.load(SeqCst);
        let files_done = done_files.min(files_total.max(done_files));
        let mut frac = if phase == JobPhase::Analyzing {
            0.0
        } else if total > 0 {
            done as f64 / total as f64
        } else if files_total > 0 {
            files_done as f64 / files_total as f64
        } else if finished {
            1.0
        } else {
            0.0
        };
        if phase == JobPhase::Done {
            frac = 1.0;
        }
        let frac = frac.clamp(0.0, 1.0);
        v.percent = if phase == JobPhase::Analyzing { "…".into() } else { format!("{}%", (frac * 100.0).floor() as i64) };
        v.frac = frac;

        // speeds: now (blue) + average (lavender); the time left uses the average
        let mbps = Self::use_mbps(Some(&j));
        let (now, hist) = {
            let sp = crate::engine::lock(&j.speed);
            // redraw on every new sample: once the history is full (600) its size stops changing, the counter doesn't
            let hist = if sp.samples != self.last_samples {
                self.last_samples = sp.samples;
                Some(sp.history.clone())
            } else {
                None
            };
            (sp.second, hist)
        };
        let avg = j.average_speed();
        let running = !finished && phase == JobPhase::Copying && !j.is_paused();
        let mut left: Option<Duration> = None;
        if running && avg > 1.0 && total > done && j.copy_clock.elapsed().as_secs_f64() >= 2.0 {
            let l = (total - done) as f64 / avg;
            if l < 7.0 * 86400.0 {
                left = Some(Duration::from_secs_f64(l));
            }
        }
        let (sn, su) = format::speed(if running { now } else { 0.0 }, mbps);
        let (an, au) = format::speed(avg, mbps);
        v.sp_num = if finished || !running { "–".into() } else { sn.clone() };
        v.sp_unit = if finished || !running { String::new() } else { format!(" {su}") };
        v.sp_avg_num = if avg > 0.0 { an.clone() } else { "–".into() };
        v.sp_avg_unit = if avg > 0.0 { format!(" {au}") } else { String::new() };
        v.c_now = if running { format!("{sn} {su}") } else { String::new() };
        v.c_avg_label = if avg > 0.0 { if running { "    AVG  ".into() } else { "AVG  ".into() } } else { String::new() };
        v.c_avg = if avg > 0.0 { format!("{an} {au}") } else { String::new() };
        if let Some(hist) = hist
            && !finished
        {
            self.graph.set_samples(&hist);
        }
        if avg > 0.0 {
            self.graph.set_average(avg);
        }
        let v = &mut self.v;

        // sizes (green done, peach total), files, times (amber)
        if phase == JobPhase::Analyzing
            && let Some(sc) = j.scanner()
        {
            v.size_done = format::size(sc.bytes_seen.load(SeqCst));
            v.size_of = " found".into();
            v.size_total = String::new();
            v.files_text = format!("{} files", format::count(sc.files_seen.load(SeqCst) as i64));
        } else if phase == JobPhase::Queued {
            v.size_done = String::new();
            v.size_of = "Waiting in the queue".into();
            v.size_total = String::new();
            v.files_text = String::new();
        } else {
            let (a, b) = format::pair(if finished { done.min(total) } else { done }, total);
            v.size_done = a;
            v.size_of = " of ".into();
            v.size_total = b;
            v.files_text = format!("{} of {}{}", format::count(files_done as i64), format::count(files_total as i64), if files_total == 1 { " file" } else { " files" });
        }
        v.elapsed_label = "Elapsed ".into();
        v.elapsed_val = format::short(j.clock.elapsed());
        if finished {
            v.remain_label = String::new();
            v.remain_val = String::new();
        } else {
            v.remain_label = "    Remaining ".into();
            v.remain_val = if j.is_paused() {
                "paused".into()
            } else if let Some(l) = left {
                format::short(l)
            } else {
                "…".into()
            };
        }

        // taskbar + title + tray
        let state = if finished {
            0
        } else if j.is_paused() || phase == JobPhase::Waiting {
            8
        } else if phase == JobPhase::Analyzing {
            1
        } else {
            2
        };
        if !crate::testhook::park() {
            h.win.taskbar_progress(state, frac);
        }
        let speeds = running.then(|| format!("{} (avg {})", Self::speed(now, mbps), Self::speed(avg, mbps)));
        let title = if finished {
            "Choppy".to_string()
        } else {
            format!("{}%{} - Choppy", (frac * 100.0) as i64, speeds.as_ref().map(|s| format!(" · {s}")).unwrap_or_default())
        };
        h.win.set_title(&title);
        if self.last_tray.is_none_or(|t| t.elapsed().as_millis() > 1000) {
            self.last_tray = Some(Instant::now());
            crate::app::set_tray_text(&if finished {
                "Choppy".to_string()
            } else {
                format!(
                    "Choppy {}%{}{}",
                    (frac * 100.0) as i64,
                    speeds.as_ref().map(|s| format!(" · {s}")).unwrap_or_default(),
                    left.map(|l| format!(" · {} left", format::short(l))).unwrap_or_default()
                )
            });
        }

        self.fill_split();
        if finished {
            self.show_report(h, &j);
            h.invalidate();
            return;
        }
        let v = &mut self.v;
        v.pause_text = if j.is_paused() { "Resume".into() } else { "Pause".into() };
        v.skip_enabled = j.active_count() > 0 && !j.is_paused() && !j.as_admin();

        // compact line + current file card
        v.c2_label_c = DIM;
        let t = j.current_transfer();
        let retry_file = j.retry_file();
        h.invalidate();
        if matches!(phase, JobPhase::Analyzing | JobPhase::Queued) {
            v.cur_label = "ANALYZING".into();
            v.cur_name = if phase == JobPhase::Queued {
                "Waiting for the copy before it".into()
            } else if let Some(sc) = j.scanner() {
                format!("{}…", sc.stage())
            } else {
                "Checking the drives…".into()
            };
            v.cur_done.clear();
            v.cur_of.clear();
            v.cur_total.clear();
            v.cur_frac = 0.0;
            v.c2_label = if phase == JobPhase::Queued { "Queued  ".into() } else { "Analyzing  ".into() };
            v.c2_name = v.cur_name.clone();
            return;
        }
        if phase == JobPhase::Finishing {
            v.cur_label = "FINISHING".into();
            v.cur_name = "Folder dates and attributes".into();
            v.cur_done.clear();
            v.cur_of.clear();
            v.cur_total.clear();
            v.cur_frac = 1.0;
            v.c2_label = "Finishing  ".into();
            v.c2_name = "folder dates and attributes".into();
            return;
        }
        if phase == JobPhase::Waiting {
            v.c2_label_c = WARN;
            v.c2_label = "Waiting for the network  ".into();
            v.c2_name = "it goes on by itself".into();
        }
        v.cur_label = if j.as_admin() { "CURRENT FILE · AS ADMINISTRATOR".into() } else { "CURRENT FILE".into() };
        if let (None, Some(rf)) = (&t, &retry_file) {
            let rf = rf.to_string_lossy();
            v.cur_name = rf.clone();
            v.cur_done.clear();
            v.cur_of = j.retry_why().unwrap_or_else(|| "Trying again".into());
            v.cur_total.clear();
            v.cur_frac = 0.0;
            if phase != JobPhase::Waiting {
                v.c2_label = "Waiting  ".into();
                v.c2_name = format!("{rf} - {}", j.retry_why().unwrap_or_else(|| "trying again".into()));
            }
        } else if t.is_none() && j.as_admin() {
            v.cur_name = j.status().unwrap_or_else(|| "Copying as administrator".into());
            v.cur_done.clear();
            v.cur_of.clear();
            v.cur_total.clear();
            v.cur_frac = 0.0;
            v.c2_label = "As administrator  ".into();
            v.c2_name = "in its own window".into();
        } else if let Some(t) = &t {
            let f = t.file();
            let name = f.name();
            // with many small files in flight, hold each name a moment so it can be read
            let same = self.shown_cur.as_ref().is_some_and(|s| *s == name);
            if self.shown_cur.is_none() || same || self.shown_cur_at.elapsed().as_millis() > 350 || f.size() > (32 << 20) {
                if !same {
                    self.shown_cur = Some(name.clone());
                    self.shown_cur_at = Instant::now();
                }
                let v = &mut self.v;
                v.cur_name = name.to_string_lossy();
                v.cur_name_tip = Some(paths::to_short(&f.src).to_string_lossy());
                
                let verb = if t.verifying.load(SeqCst) {
                    let vtotal = (f.size() * if j.as_admin() { 1 } else { 2 }).max(1);
                    let mut vf = (t.verify_done.load(SeqCst) as f64 / vtotal as f64).min(1.0);
                    if j.as_admin() {
                        vf = (t.done.load(SeqCst) as f64 / f.size().max(1) as f64).min(1.0);
                    }
                    v.cur_done.clear();
                    v.cur_of = format!("checking the copy {}%", (vf * 100.0) as i64);
                    v.cur_total.clear();
                    v.cur_frac = vf;
                    "Checking  "
                } else {
                    if f.size() > 0 {
                        let (a, b) = format::pair(t.done.load(SeqCst), f.size());
                        v.cur_done = a;
                        v.cur_of = " of ".into();
                        v.cur_total = b;
                    } else {
                        v.cur_done.clear();
                        v.cur_of = "empty file".into();
                        v.cur_total.clear();
                    }
                    v.cur_frac = if f.size() > 0 { (t.done.load(SeqCst) as f64 / f.size() as f64).min(1.0) } else { 1.0 };
                    if j.spec.op == OpKind::Move { "Moving  " } else { "Copying  " }
                };
                if phase != JobPhase::Waiting {
                    v.c2_label = if j.is_paused() { "Paused  ".into() } else { verb.into() };
                    v.c2_name = name.to_string_lossy();
                }
            }
        }
        let v = &mut self.v;
        if j.is_paused() {
            v.c2_label_c = WARN;
            if t.is_none() {
                v.c2_label = "Paused  ".into();
                v.c2_name = j.status().unwrap_or_else(|| "press Resume to go on".into());
            } else if let Some(s) = j.status().filter(|s| s.starts_with("The destination is full")) {
                v.c2_name = s;
            }
        }
    }

    fn final_average(j: &CopyJob) -> f64 {
        let mut s = j.copy_clock.elapsed().as_secs_f64();
        if s <= 0.0 {
            s = j.clock.elapsed().as_secs_f64();
        }
        if s > 0.0 { j.copied_bytes.load(SeqCst) as f64 / s } else { 0.0 }
    }

    // ================= end report =================

    fn show_report(&mut self, h: &mut Host, j: &Arc<CopyJob>) {
        let canceled = j.phase() == JobPhase::Canceled;
        let failed = j.phase() == JobPhase::Failed;
        let failed_files = j.failed_files.load(SeqCst);
        let skipped = j.skipped_files.load(SeqCst);
        let clean = !canceled && !failed && failed_files == 0 && skipped == 0;
        let mbps = Self::use_mbps(Some(j));
        let avg = Self::final_average(j);
        let done_files = j.done_files.load(SeqCst);
        let verb = if j.spec.op == OpKind::Move { "Moved" } else { "Copied" };
        if !self.report_shown {
            self.report_shown = true;
            let v = &mut self.v;
            v.current_visible = false;
            v.report_visible = true;
            v.run_buttons = false;
            v.done_buttons = true;
            v.cancel_text = "Close".into();
            if !crate::testhook::park() {
                h.win.taskbar_progress(0, 0.0);
            }
            if avg > 0.0 {
                let (an, au) = format::speed(avg, mbps);
                v.sp_avg_num = an.clone();
                v.sp_avg_unit = format!(" {au}");
                v.c_avg_label = "AVG  ".into();
                v.c_avg = format!("{an} {au}");
            }
            v.rep_label = if canceled {
                "CANCELED"
            } else if failed {
                "STOPPED"
            } else if clean {
                "DONE"
            } else {
                "DONE WITH PROBLEMS"
            }
            .into();
            v.rep_label_c = if canceled || failed { WARN } else if clean { GOOD } else { WARN };
            let dur = j.clock.elapsed();
            let copied = j.copied_bytes.load(SeqCst);
            let summary = if j.plan().is_some_and(|p| p.instant_move) && failed_files == 0 {
                format!("{verb} {}{} instantly (same drive)", format::count(done_files as i64), if done_files == 1 { " item" } else { " items" })
            } else if failed {
                j.status().unwrap_or_else(|| "The copy stopped.".into())
            } else if canceled {
                format!("{verb} {}{} · {} before canceling", format::count(done_files as i64), if done_files == 1 { " file" } else { " files" }, format::size(copied))
            } else {
                format!("{verb} {}{} · {} in {}", format::count(done_files as i64), if done_files == 1 { " file" } else { " files" }, format::size(copied), format::duration(dur))
            };
            v.rep_title = summary;
            v.chips.clear();
            if avg > 0.0 && !canceled && !failed {
                v.chips.push((format!("avg {}", Self::speed(avg, mbps)), AVG_SPEED));
            }
            let admin = j.admin_files.load(SeqCst);
            if admin > 0 {
                v.chips.push((format!("As administrator {}", format::count(admin as i64)), ACCENT_TEXT));
            }
            let verified = j.verified_files.load(SeqCst);
            if verified > 0 {
                v.chips.push((format!("Verified {}", format::count(verified as i64)), GOOD));
            }
            let there = j.already_there_files.load(SeqCst);
            if there > 0 {
                v.chips.push((format!("Already there {}", format::count(there as i64)), DIM));
            }
            if skipped > 0 {
                v.chips.push((format!("Skipped {}", format::count(skipped as i64)), WARN));
            }
            if failed_files > 0 {
                v.chips.push((format!("Failed {}", format::count(failed_files as i64)), BAD));
            }
            if canceled {
                let not = (j.total_files.load(SeqCst) - done_files - skipped - failed_files).max(0);
                v.chips.push((format!("Not copied {}", format::count(not as i64)), DIM));
            }

            self.problem_note = None;
            let fi = crate::engine::lock(&j.failed_items).clone();
            if let Some(f) = fi.first() {
                self.problem_note = Some(format!(
                    "{}: {}{}",
                    f.name(),
                    f.error().unwrap_or_default(),
                    if fi.len() > 1 { format!("  (+{} more in the log)", fi.len() - 1) } else { String::new() }
                ));
            } else if skipped > 0 {
                self.problem_note =
                    Some(format!("{}{} skipped (already there).", format::count(skipped as i64), if skipped == 1 { " file was" } else { " files were" }));
            } else {
                // "Copied as administrator: ..." is in the chips already, not a problem
                let notes: Vec<String> = crate::engine::lock(&j.notes)
                    .iter()
                    .filter(|n| !n.starts_with("Copied as administrator") && !n.starts_with("Moved as administrator"))
                    .cloned()
                    .collect();
                if let Some(n) = notes.first() {
                    self.problem_note =
                        Some(format!("{n}{}", if notes.len() > 1 { format!("  (+{} more in the log)", notes.len() - 1) } else { String::new() }));
                }
            }

            // compact: the outcome in the line under the numbers
            let v = &mut self.v;
            v.c2_label_c = if canceled || failed { WARN } else if clean { GOOD } else { WARN };
            v.c2_label = format!(
                "{}  ",
                if canceled {
                    "Canceled"
                } else if failed {
                    "Stopped"
                } else if clean {
                    "Done"
                } else {
                    "Done with problems"
                }
            );
            v.retry_visible = !fi.is_empty();
            v.log_visible = j.log_path().is_some();
            v.open_visible = !(canceled && done_files == 0);
            let s = Settings::current();
            if clean && s.close_when_done && !self.manager.has_waiting() && auto_close_allowed() {
                self.close_at = Some(Instant::now() + Duration::from_secs(s.close_after_seconds.max(0) as u64));
            }
            drop(s);
            self.update_close_button();
        }
        if let Some(at) = self.close_at {
            // the mouse on the window holds it open, unless it is set to close right away (0 s)
            let secs = Settings::current().close_after_seconds;
            if self.mouse_over && secs > 0 {
                self.close_at = Some(Instant::now() + Duration::from_secs(secs as u64));
            } else if Instant::now() >= at {
                self.close_at = None;
                h.close();
                return;
            }
        }
        let countdown = self.close_at.map(|at| {
            let left = at.saturating_duration_since(Instant::now()).as_secs_f64();
            format!("closing in {} s", (left.ceil() as i64).max(1))
        });
        let v = &mut self.v;
        if let Some(p) = &self.problem_note {
            v.c2_name = p.clone();
            v.c2_name_c = if failed_files > 0 { BAD } else { WARN };
        } else {
            v.c2_name = format!("{}{}", v.rep_title.replacen(&format!("{verb} "), "", 1), countdown.as_ref().map(|c| format!(" · {c}")).unwrap_or_default());
            if !v.rep_title.contains(&format!("{verb} ")) {
                v.c2_name = format!("{}{}", v.rep_title, countdown.as_ref().map(|c| format!(" · {c}")).unwrap_or_default());
            }
            v.c2_name_c = TEXT2;
        }
        let note = self.problem_note.clone().or_else(|| countdown.as_ref().map(|c| format!("Closing in {} (Close at completion)", &c[11..])));
        v.rep_note = note.clone().unwrap_or_default();
        v.rep_note_visible = note.is_some();
        v.rep_note_c = if self.problem_note.is_some() { DIM } else { FAINT };
    }

    // ================= quick toggles (the same settings as the Settings window) =================

    /// Shows the settings as they are now (they can also change in the Settings window).
    fn sync_quick(&mut self) {
        self.loading_quick = true;
        let s = Settings::current();
        self.v.tg_close = s.close_when_done;
        let verify = match &self.job {
            Some(j) if !j.is_finished() => j.verify() || (j.phase() == JobPhase::Queued && j.spec_verify()),
            _ => s.verify_copies,
        };
        self.v.tg_verify = verify;
        let limit_on = s.speed_limit_mbps > 0.0;
        self.v.tg_limit = limit_on;
        let (lim, val) = (s.speed_limit_mbps, s.limit_value_mbps);
        drop(s);
        let mbps = Self::use_mbps(self.job.as_ref());
        self.v.limit_unit = if mbps { "Mbps".into() } else { "MB/s".into() };
        if !self.limit_focused {
            let v = format::limit(if limit_on { lim } else { val }, mbps);
            if self.limit.text != v {
                self.limit.set(&v);
            }
        }
        self.v.limit_enabled = limit_on;
        self.loading_quick = false;
    }

    fn quick_changed(&mut self, h: &mut Host, which: u32) {
        if self.loading_quick {
            return;
        }
        {
            let mut s = Settings::current();
            match which {
                TG_CLOSE => {
                    s.close_when_done = self.v.tg_close;
                    if let Some(j) = &self.job
                        && j.is_finished()
                    {
                        let clean = j.phase() == JobPhase::Done && j.failed_files.load(SeqCst) == 0 && j.skipped_files.load(SeqCst) == 0;
                        self.close_at = if s.close_when_done && clean && !self.manager.has_waiting() && auto_close_allowed() {
                            Some(Instant::now() + Duration::from_secs(s.close_after_seconds.max(0) as u64))
                        } else {
                            None
                        };
                    }
                }
                TG_VERIFY => {
                    let on = self.v.tg_verify;
                    s.verify_copies = on;
                    if let Some(j) = &self.job
                        && !j.is_finished()
                    {
                        j.set_verify(on);
                        j.set_spec_verify(on);
                    }
                }
                TG_LIMIT => {
                    let on = self.v.tg_limit;
                    s.speed_limit_mbps = if on { s.limit_value_mbps.max(0.1) } else { 0.0 };
                    self.v.limit_enabled = on;
                }
                _ => {}
            }
            s.save();
        }
        if which == TG_LIMIT && self.v.tg_limit {
            // LimitBox.Focus(); SelectAll()
            h.set_focus(self, LIMIT_BOX);
            self.limit_focused = true;
            self.limit.select_all();
        }
        self.manager.apply_live_settings();
        crate::app::settings_reload();
        self.tick(h);
    }

    fn limit_changed(&mut self) {
        if self.loading_quick || !self.limit_focused {
            return;
        }
        let v = format::parse_limit(&self.limit.text);
        if v <= 0 {
            return;
        }
        let mbps = Self::use_mbps(self.job.as_ref());
        {
            let mut s = Settings::current();
            s.limit_value_mbps = format::limit_to_mbps(v as f64, mbps);
            if s.speed_limit_mbps > 0.0 {
                s.speed_limit_mbps = s.limit_value_mbps;
            }
            s.save();
        }
        self.manager.apply_live_settings();
        crate::app::settings_reload();
    }

    // ================= queue =================

    fn refresh_queue(&mut self) {
        self.v.queue_count = self.manager.waiting_count().to_string();
        if let Some(p) = &self.queue_popup
            && let Ok(mut p) = p.try_borrow_mut()
            && p.host.win.is_visible()
        {
            let Window { host, c } = &mut *p;
            c.fill(&self.manager);
            host.invalidate();
        }
    }

    fn queue_open(&self) -> bool {
        self.queue_popup.as_ref().is_some_and(|p| p.try_borrow().map(|p| p.host.win.is_visible()).unwrap_or(true))
    }

    fn close_queue(&mut self) {
        if let Some(p) = &self.queue_popup
            && let Ok(p) = p.try_borrow()
            && p.host.win.is_visible()
        {
            p.host.win.hide();
        }
    }

    fn toggle_queue(&mut self, h: &mut Host) {
        if self.queue_open() {
            self.close_queue();
        } else {
            self.open_queue(h);
        }
        self.refresh_queue();
    }

    fn open_queue(&mut self, h: &mut Host) {
        let Some(q) = h.rect_of(QUEUE) else { return };
        if self.queue_popup.is_none() {
            let spec = Spec { owner: Some(h.win), popup: true, ..Default::default() };
            self.queue_popup = Window::create(&h.g, &spec, Sizing::ContentHeight(400.0), 0.0, QueueContent::default());
        }
        let Some(p) = &self.queue_popup else { return };
        let Ok(mut p) = p.try_borrow_mut() else { return };
        let Window { host, c } = &mut *p;
        c.fill(&self.manager);
        host.dpi = h.dpi;
        host.fixed_dpi = h.fixed_dpi;
        let s = h.scale();
        let (_, ph) = host.measure(c);
        let (l, t, _, _) = h.win.rect();
        // Placement="Top", VerticalOffset="-6": its bottom 6 DIPs above the button, left edges lined up
        let x = l + (q.x * s).round() as i32;
        let y = t + (q.y * s).round() as i32 - (6.0 * s).round() as i32 - ph;
        if let Some(lay) = host.layered.as_mut() {
            lay.pos = (x, y);
            lay.alpha = 255;
        }
        host.paint(c);
        host.win.show(false);
    }

    // ================= buttons =================

    fn pause_click(&mut self, h: &mut Host) {
        let Some(j) = self.job.clone() else { return };
        if j.is_finished() {
            return;
        }
        if j.is_paused() {
            j.resume();
        } else {
            j.pause();
        }
        self.tick(h);
    }

    fn cancel_click(&mut self, h: &mut Host) {
        let Some(j) = self.job.clone() else { return self.leave_report(h) };
        if j.is_finished() {
            return self.leave_report(h);
        }
        crate::ui::dialogs::ask(
            Some(h.win),
            "Cancel this copy?",
            "Files that are already copied stay where they are. Nothing half-written is left behind.",
            "Cancel copy",
            "Keep going",
            move |yes| {
                if yes && !j.is_finished() {
                    j.cancel();
                }
            },
        );
    }

    fn unit_click(&mut self, h: &mut Host) {
        let now_mbps = Self::use_mbps(self.job.as_ref());
        {
            let mut s = Settings::current();
            s.speed_unit = Some(if now_mbps { "mbs".into() } else { "mbps".into() });
            s.save();
        }
        if self.job.as_ref().is_some_and(|j| j.is_finished()) {
            self.report_shown = false;
        }
        crate::app::settings_reload();
        self.tick(h);
    }

    fn view_click(&mut self, h: &mut Host) {
        self.close_queue();
        {
            let mut s = Settings::current();
            s.extended = !s.extended;
            s.save();
        }
        self.last_samples = -1;
        self.tick(h);
        // ApplyView(true): the window grows or shrinks, then stays on the screen (KeepOnScreen)
        h.fit(self);
        self.keep_on_screen(h);
    }

    /// After growing (extended view), keep the whole window on the screen: SystemParameters.WorkArea (the primary
    /// screen's work area), as the C# does.
    fn keep_on_screen(&self, h: &mut Host) {
        let (l, t, r, b) = h.win.rect();
        let wa = win::primary_work_area();
        let height = b - t;
        if b > wa.3 {
            let top = wa.1.max(wa.3 - height - (8.0 * h.scale()).round() as i32);
            h.win.move_to(l, top);
        }
        let _ = r;
    }

    fn open_click(&self) {
        let Some(j) = &self.job else { return };
        let target = paths::to_short(&j.spec.dest).to_string_lossy();
        if j.spec.sources.len() == 1 && j.spec.explicit_items.is_none() {
            let mut top = paths::combine(&j.spec.dest, &paths::file_name(&j.spec.sources[0]));
            if let Some(scan) = j.scan()
                && scan.files.len() == 1
                && let Some(fp) = scan.files[0].final_path()
            {
                top = fp;
            }
            if paths::exists(&top) {
                win::shell_open("explorer.exe", Some(&format!("/select,\"{}\"", paths::to_short(&top))));
                return;
            }
        }
        win::shell_open("explorer.exe", Some(&format!("\"{target}\"")));
    }

    fn close_click(&mut self, h: &mut Host) {
        if let Some(j) = &self.job
            && !j.is_finished()
        {
            if crate::app::elevated() {
                h.win.minimize(); // no tray for the administrator copy
                return;
            }
            // the copy keeps going; the tray icon brings the window back
            crate::app::ensure_tray();
            h.win.hide();
            crate::app::tray_balloon("Choppy keeps copying here. Click the icon to see it.");
            return;
        }
        h.close();
    }

    // ================= test hook =================

    pub fn press_for_test(&mut self, h: &mut Host, name: &str) {
        match name {
            "pause" => self.pause_click(h),
            "skip" => {
                if let Some(j) = &self.job
                    && !j.is_finished()
                {
                    j.skip_current();
                }
            }
            "cancel" => self.cancel_click(h),
            "cancel!" => {
                // as if "Cancel copy" was confirmed
                if let Some(j) = &self.job
                    && !j.is_finished()
                {
                    j.cancel();
                }
            }
            "verify" => {
                self.v.tg_verify = !self.v.tg_verify;
                self.quick_changed(h, TG_VERIFY);
            }
            "unit" => self.unit_click(h),
            "queue" => self.toggle_queue(h),
            "retry" => self.retry_click(),
            "view" => self.view_click(h),
            "autoclose" => {
                self.v.tg_close = !self.v.tg_close;
                self.quick_changed(h, TG_CLOSE);
            }
            "limit" => {
                self.v.tg_limit = !self.v.tg_limit;
                self.quick_changed(h, TG_LIMIT);
            }
            n if n.starts_with("limit ") => {
                self.limit_focused = true;
                self.limit.set(&n[6..]);
                self.limit_changed();
                self.limit_focused = false;
                if h.ui.focus == LIMIT_BOX {
                    h.ui.focus = 0;
                }
            }
            _ => {}
        }
        self.tick(h);
    }

    fn retry_click(&self) {
        if let Some(j) = &self.job
            && !crate::engine::lock(&j.failed_items).is_empty()
        {
            self.manager.retry_failed(j);
        }
    }

    pub fn state_for_test(&self, h: &Host) -> String {
        let v = &self.v;
        let s = Settings::current();
        format!(
            "{{\"phase\":\"{}\",\"percent\":\"{}\",\"badge\":\"{}\",\"from\":\"{}\",\"to\":\"{}\",\"line1\":\"{}\",\"line2\":\"{}\",\"stats\":\"{}\",\"elapsed\":\"{}\",\"remaining\":\"{}\",\"speed\":\"{}\",\"avg\":\"{}\",\"current\":\"{}\",\"report\":\"{}\",\"queue\":\"{}\",\"extended\":{},\"autoclose\":{},\"verify\":{},\"limit\":\"{}\",\"visible\":{},\"graphUpdates\":{},\"samples\":{}}}",
            self.job.as_ref().map(|j| j.phase().name()).unwrap_or("none"),
            v.percent,
            esc(&format!("{} {}", v.badge_mode, v.badge_detail)),
            esc(&format!("{}{} {}", v.from_name, v.from_warn.as_ref().map(|w| format!("{WARN_RUN}{w}")).unwrap_or_default(), v.from_path)),
            esc(&format!("{}{} {}", v.to_name, v.to_warn.as_ref().map(|w| format!("{WARN_RUN}{w}")).unwrap_or_default(), v.to_path)),
            esc(&format!("{}{}{}", v.c_now, v.c_avg_label, v.c_avg)),
            esc(&format!("{}{}", v.c2_label, v.c2_name)),
            esc(&format!("{}{}{} | {}", v.size_done, v.size_of, v.size_total, v.files_text)),
            esc(&v.elapsed_val),
            esc(&v.remain_val),
            esc(&format!("{}{}", v.sp_num, v.sp_unit)),
            esc(&format!("{}{}", v.sp_avg_num, v.sp_avg_unit)),
            esc(&format!("{} {}{}{}", v.cur_name, v.cur_done, v.cur_of, v.cur_total)),
            esc(&v.rep_title),
            esc(&v.queue_count),
            s.extended,
            v.tg_close,
            v.tg_verify,
            if v.tg_limit { esc(&format!("{} {}", self.limit.text, v.limit_unit)) } else { "off".into() },
            h.win.is_visible(),
            self.graph.updates,
            self.job.as_ref().map(|j| crate::engine::lock(&j.speed).samples).unwrap_or(0)
        )
    }
}

/// The warning run before a held-back USB link: "    ⚠ " in the icon font (as the C# Run).
const WARN_RUN: &str = "   \u{E7BA} ";

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Tests keep windows open unless they ask for the real auto close.
fn auto_close_allowed() -> bool {
    !app_paths::test_mode() || std::env::var("CHOPPY_TEST_AUTOCLOSE").as_deref() == Ok("1")
}

impl MainContent {
    /// Head style: 15.5 SemiBold, the drive's name (+ the USB warning runs).
    fn drive_runs(name: &str, warn: &Option<String>) -> El {
        let mut r = vec![(name.to_string(), sf(15.5, SEMI), TEXT)];
        if let Some(w) = warn {
            r.push((WARN_RUN.to_string(), icons(12.0), WARN));
            r.push((w.clone(), sf(13.0, NORMAL), WARN));
        }
        runs(r).vcenter()
    }
}

impl Content for MainContent {
    fn build(&mut self, ui: &UiState) -> El {
        let ext = self.extended();
        let v = &self.v;
        let dim_path = PathText { parent_c: DIM, leaf_c: DIM, size: 14.5, weight: NORMAL };

        // ===== title bar =====
        let title = grid(&[], &[])
            .child(
                hstack()
                    .vcenter()
                    .m(18.0, 0.0, 0.0, 0.0)
                    .child(image(self.icon.as_ref().map(|i| i.1.clone())).size(20.0, 20.0))
                    .child(text("Choppy", sf(15.0, SEMI), 0xFFE9ECF0).m(10.0, 0.0, 0.0, 1.0).vcenter())
                    .child(text(&v.title_extra, sf(13.0, NORMAL), FAINT).m(8.0, 1.0, 0.0, 0.0).vcenter()),
            )
            .child(
                hstack()
                    .right()
                    .child(title_button(ui, SETTINGS, false, G_SETTINGS, 16.0, 46.0).tip("Settings"))
                    .child(title_button(ui, CLOSE, true, G_CLOSE, 12.0, 46.0).tip("Close (a running copy keeps going in the tray)")),
            );

        // ===== route, badge, bar, numbers =====
        let mut rg = grid(
            &[Len::Px(48.0), Len::Px(30.0), Len::Auto, Len::Star(1.0), Len::Auto, Len::Auto],
            &[Len::Px(27.0), Len::Px(27.0), Len::Auto, Len::Auto, Len::Auto, Len::Auto],
        )
        .named("RouteGrid");
        for (row, label, name, warn, tip, path, server, ptip) in [
            (0, "FROM", &v.from_name, &v.from_warn, &v.from_tip, &v.from_path, v.from_server, "FromPath"),
            (1, "TO", &v.to_name, &v.to_warn, &v.to_tip, &v.to_path, v.to_server, "ToPath"),
        ] {
            rg.push(text(label, T_CAP, LABEL).vcenter().row(row));
            rg.push(drive_icon(server).vcenter().left().cell(row, 1));
            rg.push(Self::drive_runs(if name.is_empty() { "-" } else { name }, warn).cell(row, 2).tip_opt(tip.clone()).named(if row == 0 { "FromName" } else { "ToName" }));
            rg.push(dim_path.el(path).h(20.0).m(10.0, 1.0, 0.0, 0.0).vcenter().cell(row, 3).col_span(3).named(ptip));
        }
        // compact: speeds on their own row (right, where the bar ends), so the destination keeps the whole width
        rg.push(
            runs(vec![run(&v.c_now, 14.0, SEMI, NOW_SPEED), run(&v.c_avg_label, 12.5, NORMAL, DIM), run(&v.c_avg, 14.0, SEMI, AVG_SPEED)])
                .right()
                .vcenter()
                .m(0.0, 4.0, 0.0, -6.0)
                .row(2)
                .col_span(5)
                .shown(!ext)
                .named("CompactSpeeds"),
        );
        // extended: badge + percent
        let mut badge_sp = hstack()
            .child(ellipse(7.0, 7.0, if v.badge_warn { WARN } else { NOW_SPEED }).vcenter().m(0.0, 1.0, 0.0, 0.0))
            .child(text(&v.badge_mode, sf(13.5, SEMI), ACCENT_TEXT).m(8.0, 0.0, 0.0, 0.0));
        if !v.badge_detail.is_empty() {
            badge_sp.push(text(&v.badge_detail, sf(13.5, NORMAL), ACCENT_TEXT).m(12.0, 0.0, 0.0, 0.0));
        }
        rg.push(
            border(Some(Fill::Solid(0xFF132B44)), Some(0xFF245E9C), 1.0, 15.0, Th::new(12.0, 4.0, 14.0, 5.0))
                .child(badge_sp)
                .left()
                .vcenter()
                .m(0.0, 12.0, 0.0, 0.0)
                .row(3)
                .col_span(4)
                .shown(ext)
                .tip_opt(v.badge_tip.clone())
                .named("Badge"),
        );
        rg.push(text(&v.percent, display(30.0, BOLD), 0xFFF5F7FA).right().bottom().m(0.0, 8.0, 0.0, -4.0).cell(3, 4).col_span(2).shown(ext).named("PercentExt"));
        let frac = v.frac;
        rg.push(
            panel()
                .h(7.0)
                .vcenter()
                .m(0.0, 14.0, 0.0, 0.0)
                .row(4)
                .col_span(if ext { 6 } else { 5 })
                .child(fill(TRACK, 3.5))
                .child(draw((0.0, 0.0), move |cv, r| {
                    // Bar.Width = max(7, track x fraction), rounded like a layout width
                    if frac <= 0.0 {
                        return;
                    }
                    let w = ((7f64.max(r.w * frac)) * cv.scale).round_ties_even() / cv.scale;
                    let b = crate::sys::gfx::Rect::new(r.x, r.y, w, r.h);
                    cv.fill_gradient(b, 3.5, &[(0.0, 0xFF1A7FE6), (1.0, 0xFF5AC8FA)], (0.0, 0.0), (1.0, 0.0));
                }))
                .named("BarTrack"),
        );
        rg.push(
            text(&v.percent, display(25.0, BOLD), 0xFFF5F7FA)
                .min_w(70.0)
                .align(TextAlign::Right)
                .vcenter()
                .m(12.0, 12.0, 0.0, 0.0)
                .cell(4, 5)
                .shown(!ext)
                .named("PercentCompact"),
        );
        // 24.8 GB of 35.8 GB · 2 of 6 files · Elapsed 37 min · Remaining 16 min
        rg.push(
            grid(&[Len::Auto, Len::Star(1.0), Len::Auto], &[])
                .m(0.0, 9.0, 0.0, 0.0)
                .row(5)
                .col_span(if ext { 6 } else { 5 })
                .named("Stats")
                .child(runs(vec![run(&v.size_done, 14.0, SEMI, SIZE_DONE), run(&v.size_of, 14.0, NORMAL, DIM), run(&v.size_total, 14.0, SEMI, SIZE_TOTAL)]).vcenter())
                .child(text(&v.files_text, sf(14.0, NORMAL), DIM).ha(Al::Center).vcenter().m(10.0, 0.0, 10.0, 0.0).col(1).shown(ext).named("FilesText"))
                .child(
                    runs(vec![
                        run(&v.elapsed_label, 14.0, NORMAL, DIM),
                        run(&v.elapsed_val, 14.0, SEMI, TIME),
                        run(&v.remain_label, 14.0, NORMAL, DIM),
                        run(&v.remain_val, 14.0, SEMI, REMAIN),
                    ])
                    .vcenter()
                    .col(2),
                ),
        );

        // compact: what is happening right now
        let compact_line = runs(vec![run(&v.c2_label, 14.0, NORMAL, v.c2_label_c), run(&v.c2_name, 14.0, NORMAL, v.c2_name_c)])
            .trimmed()
            .m(0.0, 12.0, 0.0, 0.0)
            .shown(!ext)
            .named("CompactLine");

        // ===== extended =====
        let speed_card = border(Some(Fill::Solid(CARD)), Some(CARD_BORDER), 1.0, 9.0, Th::new(16.0, 10.0, 16.0, 10.0)).m(0.0, 14.0, 0.0, 0.0).child(
            vstack()
                .child(
                    grid(&[], &[]).child(text("SPEED", T_CAP, LABEL).vcenter()).child(
                        link_button(
                            ui,
                            UNIT,
                            hstack()
                                .child(
                                    runs(vec![run("AVG  ", 12.0, SEMI, DIM), run(&v.sp_avg_num, 15.0, SEMI, AVG_SPEED), run(&v.sp_avg_unit, 15.0, SEMI, AVG_SPEED)])
                                        .vcenter(),
                                )
                                .child(
                                    runs(vec![(v.sp_num.clone(), display(24.0, SEMI), NOW_SPEED), (v.sp_unit.clone(), display(24.0, SEMI), NOW_SPEED)])
                                        .vcenter()
                                        .m(20.0, 0.0, 0.0, 0.0),
                                ),
                        )
                        .right()
                        .m(0.0, -4.0, -6.0, -4.0)
                        .tip("Blue = speed right now, lavender = average of this copy (the time left uses the average). Click to switch Mbps / MB/s.")
                        .named("BtnUnit"),
                    ),
                )
                .child(self.graph.el().h(70.0).m(0.0, 6.0, 0.0, 0.0).named("Graph")),
        );
        let cur_frac = v.cur_frac;
        let current = vstack()
            .m(0.0, 16.0, 0.0, 0.0)
            .shown(v.current_visible)
            .named("CurrentSection")
            .child(
                grid(&[], &[])
                    .child(text(&v.cur_label, T_CAP, LABEL).vcenter())
                    .child(runs(vec![run(&v.cur_done, 14.0, SEMI, SIZE_DONE), run(&v.cur_of, 14.0, NORMAL, DIM), run(&v.cur_total, 14.0, SEMI, SIZE_TOTAL)]).right().vcenter()),
            )
            .child(text(if v.cur_name.is_empty() { "-" } else { &v.cur_name }, sf(16.0, SEMI), TEXT).m(0.0, 4.0, 0.0, 0.0).trimmed().tip_opt(v.cur_name_tip.clone()).named("CurName"))
            .child(panel().h(4.0).m(0.0, 9.0, 0.0, 0.0).child(fill(TRACK, 2.0)).child(draw((0.0, 0.0), move |cv, r| {
                // CurBar.Width = the track's width x the share (a layout width: whole pixels)
                let w = (r.w * cur_frac * cv.scale).round_ties_even() / cv.scale;
                if w > 0.0 {
                    cv.fill_round(crate::sys::gfx::Rect::new(r.x, r.y, w, r.h), 2.0, NOW_SPEED);
                }
            })));
        let mut chips = wrap().m(0.0, 8.0, 0.0, 0.0).shown(!v.chips.is_empty());
        for (t, c) in v.chips.iter() {
            chips.push(
                border(Some(Fill::Solid(0xFF22262C)), Some(0xFF333942), 1.0, 11.0, Th::new(10.0, 3.0, 10.0, 4.0))
                    .m(0.0, 0.0, 8.0, 4.0)
                    .child(text(t, sf(12.5, NORMAL), *c)),
            );
        }
        let report = vstack()
            .m(0.0, 16.0, 0.0, 0.0)
            .shown(v.report_visible)
            .named("ReportSection")
            .child(text(&v.rep_label, T_CAP, v.rep_label_c))
            .child(text(&v.rep_title, sf(15.5, SEMI), TEXT).m(0.0, 4.0, 0.0, 0.0).wrapped())
            .child(chips)
            .child(text(&v.rep_note, sf(13.0, NORMAL), v.rep_note_c).m(0.0, 4.0, 0.0, 0.0).wrapped().shown(v.rep_note_visible));
        let mut legend = vstack().col(1).m(18.0, 0.0, 0.0, 0.0).vcenter().named("Legend");
        for (i, (c, name, size)) in v.legend.iter().enumerate() {
            let mut g = grid(&[Len::Auto, Len::Star(1.0), Len::Auto], &[])
                .m(0.0, if i == 0 { 0.0 } else { 6.0 }, 0.0, 0.0)
                .child(fill(*c, 2.0).size(10.0, 10.0).vcenter().m(0.0, 1.0, 10.0, 0.0))
                .child(text(name, sf(14.0, NORMAL), TEXT2).trimmed().tip(name.clone()).col(1));
            if *size >= 0 {
                g.push(text(format::size(*size), sf(14.0, NORMAL), DIM).m(14.0, 0.0, 0.0, 0.0).col(2));
            }
            legend.push(g);
        }
        let largest = vstack().m(0.0, 18.0, 0.0, 0.0).named("LargestSection").child(text("LARGEST FILES", T_CAP, LABEL)).child(
            grid(&[Len::Px(80.0), Len::Star(1.0)], &[])
                .m(0.0, 10.0, 0.0, 0.0)
                .child(donut(v.donut.clone()).size(76.0, 76.0).left().vcenter().named("DonutChart"))
                .child(legend),
        );
        let quick = vstack().m(0.0, 18.0, 0.0, 0.0).child(text("QUICK TOGGLES", T_CAP, LABEL)).child(
            wrap()
                .m(0.0, 9.0, 0.0, -6.0)
                .child(crate::ui::kit::quick(ui, TG_CLOSE, v.tg_close, "Close at completion", "Close this window a few seconds after a copy finishes with no problems").named("TgClose"))
                .child(crate::ui::kit::quick(ui, TG_VERIFY, v.tg_verify, "Verify files", "Check every copied file against the original (xxHash64). A move then removes the original only after its copy matches.").named("TgVerify"))
                .child(quick_pill(
                    hstack()
                        .child(quick_dot(TG_LIMIT, v.tg_limit, "Speed limit", "Leave room on the line for everything else").vcenter().named("TgLimit"))
                        .child(rect_el(1.0, 18.0, 0xFF3A4048).m(10.0, 0.0, 8.0, 0.0).vcenter())
                        .child(self.limit.el(ui, LIMIT_BOX, &BOX_SMALL, v.limit_enabled).size(56.0, 26.0).vcenter().named("LimitBox"))
                        .child(text(&v.limit_unit, sf(13.5, NORMAL), DIM).vcenter().m(7.0, 0.0, 8.0, 0.0).named("LimitUnit")),
                )),
        );
        let ext_panel = vstack().shown(ext).named("ExtPanel").child(speed_card).child(current).child(report).child(largest).child(quick);

        // ===== actions =====
        let queue_open = self.queue_open();
        let mut left = hstack();
        left.push(
            hstack()
                .shown(v.run_buttons)
                .named("RunButtons")
                .child({
                    let l = v.pause_text.clone();
                    button(ui, PAUSE, &BTN_PRIMARY, Th::hv(18.0, 0.0), true, move |fg| text(l, sf(14.0, SEMI), fg)).h(36.0).min_w(84.0).named("BtnPause")
                })
                .child(
                    button(ui, SKIP, &BTN, Th::hv(18.0, 0.0), v.skip_enabled, |fg| text("Skip", sf(14.0, SEMI), fg))
                        .h(36.0)
                        .m(8.0, 0.0, 0.0, 0.0)
                        .shown(ext)
                        .tip("Skip the current file")
                        .named("BtnSkip"),
                ),
        );
        left.push(
            hstack()
                .shown(v.done_buttons)
                .named("DoneButtons")
                .child(
                    button(ui, OPEN, &BTN_PRIMARY, Th::hv(16.0, 0.0), true, |fg| text("Open folder", sf(14.0, SEMI), fg))
                        .h(36.0)
                        .min_w(74.0)
                        .shown(v.open_visible)
                        .named("BtnOpen"),
                )
                .child(
                    button(ui, RETRY, &BTN, Th::hv(14.0, 0.0), true, |fg| text("Retry failed", sf(14.0, SEMI), fg))
                        .h(36.0)
                        .min_w(74.0)
                        .m(8.0, 0.0, 0.0, 0.0)
                        .shown(v.retry_visible),
                )
                .child(
                    button(ui, LOG, &BTN, Th::hv(14.0, 0.0), true, |fg| text("Log", sf(14.0, SEMI), fg))
                        .h(36.0)
                        .m(8.0, 0.0, 0.0, 0.0)
                        .shown(ext && v.log_visible && self.job.as_ref().is_some_and(|j| j.is_finished()))
                        .tip("Open the log of this copy"),
                ),
        );
        left.push({
            let l = v.cancel_text.clone();
            button(ui, CANCEL, &BTN_CANCEL, Th::hv(18.0, 0.0), true, move |fg| text(l, sf(14.0, SEMI), fg)).h(36.0).m(8.0, 0.0, 0.0, 0.0).named("BtnCancel")
        });
        let qc = v.queue_count.clone();
        left.push(
            button(ui, QUEUE, &BTN, Th::hv(14.0, 0.0), true, move |_| {
                hstack()
                    .child(runs(vec![run("Queue   ", 14.0, NORMAL, DIM), run(if qc.is_empty() { "0" } else { &qc }, 14.0, SEMI, TEXT)]).vcenter())
                    .child(text(if queue_open { G_CHEVRON_UP } else { G_CHEVRON_DOWN }, icons(9.0), DIM).vcenter().m(10.0, 2.0, 0.0, 0.0))
            })
            .h(36.0)
            .m(8.0, 0.0, 0.0, 0.0)
            .shown(ext)
            .named("BtnQueue"),
        );
        let view = link_button(
            ui,
            VIEW,
            hstack()
                .child(text(if ext { "Compact" } else { "Extended" }, sf(14.5, NORMAL), TEXT2).vcenter())
                .child(text(if ext { G_CHEVRON_UP } else { G_CHEVRON_DOWN }, icons(10.0), DIM).vcenter().m(9.0, 2.0, 0.0, 0.0)),
        )
        .right()
        .vcenter()
        .named("BtnView");
        let actions = grid(&[], &[]).m(0.0, 16.0, 0.0, 0.0).child(left).child(view);

        let content = vstack().m(20.0, 0.0, 20.0, 18.0).row(1).child(rg).child(compact_line).child(ext_panel).child(actions);
        let root = grid(&[], &[Len::Px(46.0), Len::Auto]).named("Root").child(title).child(content);
        border(Some(window_bg()), Some(WIN_BORDER), 1.0, 0.0, Th::default()).child(root)
    }

    fn click(&mut self, h: &mut Host, id: u32) {
        match id {
            SETTINGS => crate::app::show_settings(Some(h.win)),
            CLOSE => self.close_click(h),
            UNIT => self.unit_click(h),
            TG_CLOSE => {
                self.v.tg_close = !self.v.tg_close;
                self.quick_changed(h, TG_CLOSE);
            }
            TG_VERIFY => {
                self.v.tg_verify = !self.v.tg_verify;
                self.quick_changed(h, TG_VERIFY);
            }
            TG_LIMIT => {
                self.v.tg_limit = !self.v.tg_limit;
                self.quick_changed(h, TG_LIMIT);
            }
            PAUSE => self.pause_click(h),
            SKIP => {
                if let Some(j) = &self.job
                    && !j.is_finished()
                {
                    j.skip_current();
                }
            }
            CANCEL => self.cancel_click(h),
            QUEUE => self.toggle_queue(h),
            VIEW => self.view_click(h),
            OPEN => self.open_click(),
            RETRY => self.retry_click(),
            LOG => {
                if let Some(p) = self.job.as_ref().and_then(|j| j.log_path()) {
                    win::shell_open("notepad.exe", Some(&format!("\"{}\"", p.display())));
                }
            }
            _ => {}
        }
        h.invalidate();
    }

    fn key(&mut self, h: &mut Host, vk: u16) -> bool {
        if h.ui.focus == LIMIT_BOX {
            if vk == 0x0D || vk == 0x1B {
                // Keyboard.ClearFocus(): the box shows the setting again
                h.set_focus(self, 0);
                return true;
            }
            return false;
        }
        if vk == 0x1B && self.job.as_ref().is_some_and(|j| j.is_finished()) {
            self.leave_report(h);
            return true;
        }
        if vk == 0x20 && self.job.as_ref().is_some_and(|j| !j.is_finished()) && !h.focusable(h.ui.focus) {
            self.pause_click(h);
            return true;
        }
        false
    }

    fn text_box(&mut self, id: u32) -> Option<(&mut TextBox, BoxStyle)> {
        (id == LIMIT_BOX).then_some((&mut self.limit, BOX_SMALL))
    }

    fn text_changed(&mut self, _h: &mut Host, id: u32) {
        if id == LIMIT_BOX {
            self.limit_focused = true;
            self.limit_changed();
        }
    }

    fn focus_lost(&mut self, h: &mut Host, id: u32) {
        if id == LIMIT_BOX {
            self.limit_focused = false;
            self.sync_quick();
            h.invalidate();
        }
    }

    fn timer(&mut self, h: &mut Host, id: usize) {
        match id {
            T_TICK => self.tick(h),
            T_LIFT => {
                h.win.kill_timer(T_LIFT);
                h.win.set_topmost(false);
            }
            _ => {}
        }
    }

    fn activated(&mut self, _h: &mut Host, on: bool) {
        if !on {
            self.close_queue();
        }
    }

    fn moved(&mut self, _h: &mut Host) {
        self.close_queue();
    }

    fn closing(&mut self, h: &mut Host) -> bool {
        // Alt+F4 / the system menu close like the close button
        self.close_click(h);
        true
    }

    fn closed(&mut self, h: &mut Host) {
        h.win.kill_timer(T_TICK);
        if let Some(p) = self.queue_popup.take()
            && let Ok(p) = p.try_borrow()
        {
            p.host.win.destroy();
        }
        if !self.warm_up {
            crate::app::main_closed();
        }
    }
}

impl MainContent {
    pub fn set_icon(&mut self, g: &crate::sys::gfx::Gfx, scale: f64) {
        let px = (20.0 * scale).round() as u32;
        if self.icon.as_ref().is_none_or(|i| i.0 != px) {
            self.icon = g.icon_image(crate::ui::ICO, px).map(|i| (px, i));
        }
    }

    /// The window was just made: it shows the manager's job and starts its 100 ms refresh.
    pub fn loaded(&mut self, h: &mut Host) {
        self.set_icon(&h.g, crate::ui::icon_scale());
        self.on_manager_changed(h);
        h.win.set_timer(T_TICK, 100);
    }

    pub fn mouse(&mut self, inside: bool) {
        self.mouse_over = inside;
    }

    pub fn lift(&self, h: &mut Host) {
        h.win.set_topmost(true);
        h.win.set_timer(T_LIFT, 1500);
    }

    /// A finished job from the queue's list: its report on screen.
    pub fn show_finished(&mut self, h: &mut Host, f: Arc<CopyJob>) {
        self.close_queue();
        self.dismissed.remove(&key(&f));
        self.show_job(Some(f));
        self.tick(h);
        self.update_close_button();
    }

    pub fn current_job(&self) -> Option<Arc<CopyJob>> {
        self.job.clone()
    }
}

// ================= the queue pop-up =================

#[derive(Default)]
pub struct QueueContent {
    waiting: Vec<Arc<crate::engine::model::JobSpec>>,
    finished: Vec<Arc<CopyJob>>,
    scroll: f64,
}

pub type QueueWindow = Window<QueueContent>;

impl QueueContent {
    fn fill(&mut self, m: &JobManager) {
        self.waiting = m.waiting_snapshot();
        self.finished = m.finished_snapshot();
    }
}

const Q_REMOVE: u32 = 1000;
const Q_ROW: u32 = 2000;

impl Content for QueueContent {
    fn build(&mut self, ui: &UiState) -> El {
        let mut list = vstack().named("QueueList");
        for (i, s) in self.waiting.iter().enumerate() {
            let id = Q_REMOVE + i as u32;
            list.push(
                grid(&[Len::Star(1.0), Len::Auto], &[])
                    .m(8.0, 4.0, 4.0, 4.0)
                    .child(
                        vstack()
                            .child(text(s.title(), sf(13.5, NORMAL), TEXT2).trimmed())
                            .child(text(format!("to {}", paths::to_short(&s.dest)), sf(12.0, NORMAL), DIM).trimmed()),
                    )
                    .child(link_button(ui, id, text(G_CANCEL, icons(11.0), DIM)).vcenter().col(1).tip("Remove from the queue")),
            );
        }
        for (i, f) in self.finished.iter().enumerate() {
            let bad = had_problems(f);
            let st = match f.phase() {
                JobPhase::Canceled => "canceled".to_string(),
                JobPhase::Failed => "stopped".to_string(),
                _ if f.failed_files.load(SeqCst) > 0 => format!("{} failed", f.failed_files.load(SeqCst)),
                _ if f.skipped_files.load(SeqCst) > 0 => format!("{} skipped", f.skipped_files.load(SeqCst)),
                _ => "done".to_string(),
            };
            let t = format!("{}{} - {st}", if bad { "! " } else { "✓ " }, f.spec.title());
            list.push(
                link_button(ui, Q_ROW + i as u32, text(t, sf(12.5, NORMAL), if bad { WARN } else { FAINT }).trimmed())
                    .m(2.0, 1.0, 0.0, 1.0)
                    .tip("Show this copy's report"),
            );
        }
        if list.children.is_empty() {
            list.push(text("Nothing waiting. A new copy waits here while one is running.", sf(12.5, NORMAL), FAINT).wrapped().m(8.0, 4.0, 4.0, 4.0));
        }
        border(Some(Fill::Solid(0xFF22262C)), Some(0xFF3A4048), 1.0, 8.0, Th::all(6.0)).w(400.0).child(scroll(self.scroll, list).max_h(260.0))
    }

    fn click(&mut self, h: &mut Host, id: u32) {
        if (Q_REMOVE..Q_ROW).contains(&id) {
            if let Some(s) = self.waiting.get((id - Q_REMOVE) as usize).cloned() {
                crate::app::manager().remove(&s);
            }
        } else if id >= Q_ROW
            && let Some(f) = self.finished.get((id - Q_ROW) as usize).cloned()
        {
            h.win.hide();
            crate::app::with_main(move |m| {
                let Window { host, c } = m;
                c.show_finished(host, f.clone());
            });
        }
    }

    fn wheel(&mut self, h: &mut Host, delta: i32, _x: f64, _y: f64) {
        self.scroll = (self.scroll - delta as f64 / 120.0 * 48.0).max(0.0);
        h.invalidate();
    }
}

/// Makes the copy window (hidden) for `manager`.
pub fn create(manager: Arc<JobManager>) -> Option<Rc<RefCell<MainWindow>>> {
    let g = crate::app::gfx();
    let spec = Spec { title: "Choppy".into(), framed: true, minimize: true, taskbar: !crate::testhook::park(), ..Default::default() };
    let w = Window::create(&g, &spec, Sizing::ContentHeight(WIDTH), 46.0, MainContent::new(manager))?;
    {
        let mut m = w.borrow_mut();
        let Window { host, c } = &mut *m;
        let at = if crate::testhook::park() { Place::At(-32000, 0) } else { Place::Screen };
        c.set_icon(&host.g.clone(), crate::ui::icon_scale());
        host.place(c, at);
    }
    Some(w)
}

/// A copy window that is only drawn once, off-screen and never focused, then closed (App.WarmUp): the first real one
/// finds Direct2D, DirectWrite and the window's fonts ready.
pub fn warm_up(manager: Arc<JobManager>) {
    let g = crate::app::gfx();
    let spec = Spec { title: "Choppy".into(), framed: true, minimize: true, taskbar: false, ..Default::default() };
    let Some(w) = Window::create(&g, &spec, Sizing::ContentHeight(WIDTH), 46.0, MainContent::new(manager)) else { return };
    let Ok(mut m) = w.try_borrow_mut() else { return };
    let Window { host, c } = &mut *m;
    c.warm_up = true;
    c.set_icon(&host.g.clone(), crate::ui::icon_scale());
    host.place(c, Place::At(-32000, -32000));
    host.paint(c);
    host.win.show(false);
    host.close();
}
