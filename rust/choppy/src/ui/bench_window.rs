//! BenchmarkWindow.xaml / .cs: Choppy VS Windows (its own copy engine) on every drive of this PC. Lists the drives (and
//! why one is left out, looked at again when one is plugged in or removed), runs on its own thread with progress and an
//! estimate, shows each case as it finishes, and saves the report. It can be resized and maximized at any time,
//! running or not.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use crate::bench::benchmark::{Benchmark, TIE_MARGIN, default_rounds};
use crate::bench::report::{self, BenchDrive, BenchReport, BenchResult, BenchTotals, TIE_WORD, text as bt};
use crate::engine::settings::{MAX_BENCH_RUNS, Settings, app_paths};
use crate::engine::usb_link;
use crate::sys::gfx::{Argb, Image};
use crate::sys::win::{self, Ev, Spec};
use crate::ui::format;
use crate::ui::host::{Content, Host, Place, Sizing, T_USER, Window};
use crate::ui::kit::*;
use crate::ui::layout::*;
use crate::ui::theme::*;

const MIN: u32 = 1;
const MAX: u32 = 2;
const CLOSE: u32 = 3;
const QUICK: u32 = 4;
const THOROUGH: u32 = 5;
const RUNS: u32 = 6;
const NETWORK: u32 = 7;
const RESCAN: u32 = 8;
const FOLD: u32 = 9;
const FOLDER: u32 = 10;
const REPORT: u32 = 11;
const AGAIN: u32 = 12;
const START: u32 = 13;
const DRIVE0: u32 = 100;

const T_TICK: usize = T_USER + 1;
const T_RESCAN: usize = T_USER + 2;

const KEEP_OFF: &str = "Keep other work off these drives while it runs: it measures them.";
/// The results' columns (0 = the rest).
const COLS: [f64; 6] = [150.0, -1.0, 108.0, 108.0, 100.0, 146.0];

pub type BenchWindow = Window<BenchContent>;

struct Row {
    r: BenchResult,
    new_route: bool,
}

pub struct BenchContent {
    bench: Option<Arc<Benchmark>>,
    running: bool,
    drives: Option<Vec<BenchDrive>>,
    report: Option<BenchReport>,
    html_path: Option<PathBuf>,
    json_path: Option<PathBuf>,
    shown: usize,
    scan_no: u32,
    close_when_done: bool,
    last_route: Option<(usize, usize)>,
    /// The drives shown (for the folded line).
    listed: Option<Vec<BenchDrive>>,
    /// The height follows the content, until the top or bottom edge is dragged.
    auto_height: bool,
    sizing_height: bool,
    /// The drives folded by themselves: no room on this screen (not kept for the next time).
    folded_for_room: bool,
    /// ... and opened again with the +: they stay open.
    keep_open: bool,
    bar_fraction: f64,
    scanning: bool,
    usable: i32,
    chosen: i32,
    /// Drives unchecked: left out of the run (while the window is open).
    off: HashSet<String>,
    /// The last run's drives, with their read and write speeds.
    measured: Option<HashMap<String, BenchDrive>>,
    /// The − pressed (while the window is open: it always opens with every drive showing).
    folded: bool,
    loading_runs: bool,
    thorough: bool,
    network: bool,
    runs_box: TextBox,
    estimate: String,
    estimate_visible: bool,
    intro_visible: bool,
    hint: String,
    progress_visible: bool,
    results_visible: bool,
    step_text: String,
    elapsed_val: String,
    remain_label: String,
    remain_val: String,
    summary: Vec<(String, Argb)>,
    summary_c: Argb,
    summary_note: String,
    totals: Option<BenchTotals>,
    rows: Vec<Row>,
    scroll: f64,
    /// ResultsScroll.MaxHeight (FitToScreen).
    list_max: f64,
    icon: Option<Image>,
    is_max: bool,
}

fn res(cls: &str) -> Argb {
    match cls {
        "good" => GOOD,
        "bad" => BAD,
        _ => DIM,
    }
}

impl BenchContent {
    fn new() -> BenchContent {
        let mut c = BenchContent {
            bench: None,
            running: false,
            drives: None,
            report: None,
            html_path: None,
            json_path: None,
            shown: 0,
            scan_no: 0,
            close_when_done: false,
            last_route: None,
            listed: None,
            auto_height: true,
            sizing_height: false,
            folded_for_room: false,
            keep_open: false,
            bar_fraction: 0.0,
            scanning: false,
            usable: -1,
            chosen: -1,
            off: HashSet::new(),
            measured: None,
            folded: false,
            loading_runs: false,
            thorough: false,
            network: false,
            runs_box: TextBox::new(2, true),
            estimate: String::new(),
            estimate_visible: true,
            intro_visible: true,
            hint: KEEP_OFF.into(),
            progress_visible: false,
            results_visible: false,
            step_text: String::new(),
            elapsed_val: String::new(),
            remain_label: String::new(),
            remain_val: String::new(),
            summary: Vec::new(),
            summary_c: TEXT,
            summary_note: String::new(),
            totals: None,
            rows: Vec::new(),
            scroll: 0.0,
            list_max: 420.0,
            icon: None,
            is_max: false,
        };
        c.show_runs();
        c
    }

    // ================= drives =================

    /// Looks at the drives on a worker thread (a network drive can take a moment to answer): when the window opens, when
    /// Network drives changes, and - until a run starts - with the arrows beside DRIVES and by itself when a drive is
    /// plugged in or removed.
    fn scan(&mut self) {
        self.scan_no += 1;
        let no = self.scan_no;
        let net = self.network;
        self.scanning = true;
        self.estimate_visible = true;
        self.estimate = "Looking at the drives...".into();
        std::thread::spawn(move || {
            let found = std::panic::catch_unwind(|| Benchmark::find_drives(net, None)).unwrap_or_default();
            win::post(Box::new(move || {
                crate::app::with_bench(move |c, h| {
                    if no != c.scan_no {
                        return;
                    }
                    c.scanning = false;
                    if c.running {
                        return;
                    }
                    c.drives = Some(found.clone());
                    c.preview();
                    h.invalidate();
                })
            }));
        });
    }

    /// The drives that would be tested and how long it would take, for the chosen mode and runs.
    fn preview(&mut self) {
        // the Benchmark first: it reads which drives are checked from the list taken out next (fix list 6.17)
        let b = self.new_bench();
        let Some(mut drives) = self.drives.take() else { return };
        let est = b.preview(&mut drives);
        self.usable = drives.iter().filter(|d| d.skip.is_none()).count() as i32;
        self.chosen = drives.iter().filter(|d| d.skip.is_none() && !self.off.contains(&d.letter)).count() as i32;
        self.drives = Some(drives);
        self.estimate_visible = true;
        self.estimate = if self.chosen == 0 { String::new() } else { format!("about {}", format::short(Duration::from_secs_f64(est.max(60.0)))) };
        self.hint = if self.usable == 0 {
            "No drive here can be tested - each one's reason is listed above.".into()
        } else if self.chosen == 0 {
            "Check at least one drive to test.".into()
        } else {
            KEEP_OFF.into()
        };
    }

    /// A Benchmark as chosen: Quick or Thorough, network drives or not, the runs, and the drives checked.
    fn new_bench(&self) -> Benchmark {
        let runs = self.runs();
        let mut b = Benchmark::new();
        b.thorough = self.thorough;
        b.network = self.network;
        b.rounds_override = if runs != default_rounds(self.thorough) { runs } else { 0 };
        if !self.off.is_empty()
            && let Some(d) = &self.drives
        {
            b.only = Some(d.iter().map(|d| d.letter.clone()).filter(|l| !self.off.contains(l)).collect());
        }
        b
    }

    // ================= runs =================

    /// Runs each for the mode chosen: the number typed in (kept in the settings), else the default (Quick 2, Thorough 3).
    fn runs(&self) -> i32 {
        let s = Settings::current();
        let v = if self.thorough { s.bench_runs_thorough } else { s.bench_runs_quick };
        if v > 0 { v } else { default_rounds(self.thorough) }
    }

    /// The box shows the runs of the mode chosen; the intro and its tooltip say them.
    fn show_runs(&mut self) {
        self.loading_runs = true;
        self.runs_box.set(&self.runs().to_string());
        self.loading_runs = false;
    }

    fn runs_tip(&self) -> String {
        let (n, d) = (self.runs(), default_rounds(self.thorough));
        format!(
            "How many times each one copies every case, taking turns. {}: {d}{}. Kept for the next time",
            if self.thorough { "Thorough" } else { "Quick" },
            if n == d { "" } else { " if not changed" }
        )
    }

    fn intro(&self) -> String {
        let n = self.runs();
        format!(
            "Both copy the same test files on each drive checked below - one big file and many small files, from the system drive to each other drive and back, and within each drive. They take turns, each one {}, and the source is never in memory. The test files go into a temporary folder on each drive and are removed at the end.",
            match n {
                1 => "once".to_string(),
                2 => "twice".to_string(),
                n => format!("{n} times"),
            }
        )
    }

    /// A number from 1 typed in: kept for this mode (the default kept as no change), and the estimate follows.
    fn runs_changed(&mut self) {
        if self.loading_runs {
            return;
        }
        let t = &self.runs_box.text;
        // empty or 0 while typing: nothing changes (the box shows the runs again when it is left)
        let Some(v) = t.parse::<i32>().ok().filter(|v| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) && *v >= 1 && *v <= MAX_BENCH_RUNS) else { return };
        let keep = if v == default_rounds(self.thorough) { 0 } else { v };
        {
            let mut st = Settings::current();
            if self.thorough {
                st.bench_runs_thorough = keep;
            } else {
                st.bench_runs_quick = keep;
            }
            st.save();
        }
        if !self.running {
            self.preview();
        }
    }

    fn measured_of(&self, d: &BenchDrive) -> Option<&BenchDrive> {
        self.measured.as_ref().and_then(|m| m.get(&d.letter))
    }

    fn shut(&self) -> bool {
        self.folded || self.folded_for_room
    }

    fn toggle_fold(&mut self, h: &mut Host) {
        let open = self.shut();
        self.folded = !open;
        self.folded_for_room = false;
        self.keep_open = open;
        self.fit_to_screen(h);
    }

    /// Folded: "C: NVMe · D: HDD · F: USB SSD · 1 unchecked · 2 left out" beside DRIVES (a held-back USB drive in
    /// amber), the details as its tooltip.
    fn drives_line(&self) -> (Vec<(String, crate::sys::gfx::Font, Argb)>, Option<String>) {
        let mut runs_ = Vec::new();
        let Some(listed) = &self.listed else { return (runs_, None) };
        if !self.shut() {
            return (runs_, None);
        }
        let f = sf(12.5, NORMAL);
        let mut on: Vec<&BenchDrive> = listed.iter().filter(|d| d.skip.is_none() && !self.off.contains(&d.letter)).collect();
        on.sort_by(|a, b| (!a.system, &a.letter).cmp(&(!b.system, &b.letter)));
        let mut unticked: Vec<&BenchDrive> = listed.iter().filter(|d| d.skip.is_none() && self.off.contains(&d.letter)).collect();
        unticked.sort_by(|a, b| a.letter.cmp(&b.letter));
        let mut left: Vec<&BenchDrive> = listed.iter().filter(|d| d.skip.is_some()).collect();
        left.sort_by(|a, b| a.letter.cmp(&b.letter));
        let mut tip = Vec::new();
        for d in on {
            if !runs_.is_empty() {
                runs_.push(("  ·  ".to_string(), f, FAINT));
            }
            let held = d.usb().is_some_and(|u| u.held_back());
            runs_.push((d.letter.clone(), sf(12.5, SEMI), TEXT2));
            runs_.push((format!(" {}", d.short()), f, if held { WARN } else { DIM }));
            let mut parts = vec![format!("{}{}", d.letter, d.label.as_ref().filter(|l| !l.is_empty()).map(|l| format!(" {l}")).unwrap_or_default()), d.kind()];
            if let Some(m) = d.model.as_ref().filter(|m| !m.is_empty()) {
                parts.push(m.clone());
            }
            if let Some(m) = self.measured_of(d) {
                parts.push(format!("read {}, write {}", bt::speed(m.read_mbps), bt::speed(m.write_mbps)));
            }
            tip.push(format!("{}{}", parts.join(" · "), if held { format!("\n    {}", d.usb().and_then(|u| u.advice()).unwrap_or_default()) } else { String::new() }));
        }
        if !unticked.is_empty() {
            runs_.push((format!("{}{} unchecked", if runs_.is_empty() { "" } else { "  ·  " }, unticked.len()), f, DIM));
            tip.push(format!("Unchecked: {}", unticked.iter().map(|d| d.letter.as_str()).collect::<Vec<_>>().join(", ")));
        }
        if !left.is_empty() {
            runs_.push((format!("{}{} left out", if runs_.is_empty() { "" } else { "  ·  " }, left.len()), f, FAINT));
            tip.push(format!("Left out: {}", left.iter().map(|d| format!("{} {}", d.letter, d.skip.clone().unwrap_or_default())).collect::<Vec<_>>().join(", ")));
        }
        (runs_, Some(tip.join("\n")))
    }

    // ================= results =================

    fn add_new(&mut self, all: Vec<BenchResult>, final_: bool) {
        for r in all.iter().skip(self.shown) {
            let route = (r.from_i, r.to_i);
            let new_route = self.last_route != Some(route);
            self.last_route = Some(route);
            self.rows.push(Row { r: r.clone(), new_route });
            // ResultsScroll.ScrollToEnd()
            self.scroll = f64::MAX / 4.0;
        }
        self.shown = all.len();
        self.show_summary(&all, final_, false);
    }

    /// "Choppy is Faster 10 of 19 · Marginal Tie 9 of 19 · Never Slower", each part in its color.
    fn show_summary(&mut self, results: &[BenchResult], final_: bool, ended: bool) {
        let f = results.iter().filter(|r| r.scored && r.verdict > 0).count();
        let t = results.iter().filter(|r| r.scored && r.verdict == 0).count();
        let s = results.iter().filter(|r| r.scored && r.verdict < 0).count();
        let n = f + t + s;
        self.summary.clear();
        if n == 0 {
            self.summary.push((if final_ { "No case was measured" } else { "Each case shows here as it finishes" }.into(), if final_ { TEXT } else { DIM }));
            self.summary_c = if final_ { TEXT } else { DIM };
            return;
        }
        self.summary_c = TEXT;
        if !final_ {
            self.summary.push(("So far: ".into(), DIM));
        }
        self.summary.push(("Choppy is ".into(), TEXT));
        self.summary.push((format!("Faster {}", report::of(f, n)), GOOD));
        if t > 0 {
            self.summary.push((" · ".into(), TEXT));
            self.summary.push((format!("{TIE_WORD} {}", report::of(t, n)), DIM));
        }
        if s > 0 {
            self.summary.push((" · ".into(), TEXT));
            self.summary.push((format!("Slower {}", report::of(s, n)), BAD));
        } else if final_ && ended && t > 0 {
            self.summary.push((" · ".into(), TEXT));
            self.summary.push(("Never Slower".into(), GOOD));
        }
    }

    // ================= size =================

    /// Sized to its content, the results list grows until the window reaches the bottom of the screen, then scrolls.
    /// Sized by hand or maximized, the list fills the window.
    fn fit_to_screen(&mut self, h: &mut Host) {
        self.make_room(h);
        if !self.auto_height || h.win.is_maximized() || h.win.is_minimized() {
            self.list_max = INF;
            h.invalidate();
            return;
        }
        h.paint(self);
        let s = h.scale();
        let wa = h.win.work_area();
        let wa_h = (wa.3 - wa.1) as f64 / s;
        // from its parts: the window itself takes its new height a moment later
        let other = self.need(h, 0.0);
        self.list_max = (wa_h - 24.0 - other).max(60.0);
        let list_h = h.last.as_ref().and_then(|t| t.find("ResultsList")).map(|e| e.rect.h).unwrap_or(0.0);
        let height = other + list_h.min(self.list_max);
        let (l, t, _, _) = h.win.rect();
        if (t as f64 / s) + height > wa.3 as f64 / s && !crate::testhook::park() {
            let top = (wa.1 as f64 / s).max(wa.3 as f64 / s - height - 12.0);
            h.win.move_to(l, (top * s).round() as i32);
        }
        h.invalidate();
    }

    /// The drives stay open, and the results list gives up room first. Only when this screen (or the maximized window)
    /// is too short for the drives, a row of results and the buttons do they fold to one line - for now (the + opens
    /// them, and then they stay open).
    fn make_room(&mut self, h: &mut Host) {
        if self.folded_for_room || self.keep_open || self.folded || !self.results_visible {
            return;
        }
        h.paint(self);
        let s = h.scale();
        let wa = h.win.work_area();
        let room = if h.win.is_maximized() { h.size.1 } else { (wa.3 - wa.1) as f64 / s - if self.auto_height { 24.0 } else { 0.0 } };
        if self.need(h, 60.0) <= room {
            return;
        }
        self.folded_for_room = true;
    }

    /// The height everything takes but the results list, with this much of the list.
    fn need(&self, h: &Host, list: f64) -> f64 {
        let Some(t) = &h.last else { return 0.0 };
        let hh = |n: &str| t.find(n).map(|e| if e.vis == Vis::Collapsed { 0.0 } else { e.rect.h }).unwrap_or(0.0);
        // 46 + the border + Body's margins + the rows but the results
        let mut min = 46.0 + 2.0 + 2.0 + 20.0 + hh("Row0") + hh("Row1") + hh("Row2") + hh("Row4");
        if self.results_visible {
            min += 18.0 + hh("SummaryText") + hh("SummaryNote") + 12.0 + 2.0 + 8.0 + hh("ResultsHead") + 14.0 + list;
        }
        min
    }

    /// The window can be made as small as everything but the results list, and a few of its rows: the buttons never go
    /// out of sight.
    fn update_min_height(&self, h: &Host) {
        let s = h.scale();
        let list_h = h.last.as_ref().and_then(|t| t.find("ResultsList")).map(|e| e.rect.h).unwrap_or(0.0);
        let wa = h.win.work_area();
        let min = self.need(h, list_h.min(120.0)).min((wa.3 - wa.1) as f64 / s).ceil();
        h.win.set_min_track((800.0 * s).round() as i32, (min * s).round() as i32);
    }

    // ================= running =================

    /// Start; while it runs Abort, then Close (once aborted); after an abort Close.
    fn start_click(&mut self, h: &mut Host) {
        if self.running {
            if !self.bench.as_ref().is_some_and(|b| b.is_canceled()) {
                self.ask_abort(h, false);
            } else {
                self.close_when_done = true; // the window closes as soon as the test files are removed
            }
            return;
        }
        if self.report.as_ref().is_some_and(|r| r.canceled) {
            h.close();
            return;
        }
        let b = self.new_bench();
        self.begin(h, b);
    }

    /// The buttons for where it is: Start; Abort while it runs, then Close (the window closes once the test files are
    /// removed); at the end Run again, or after an abort Run again and Close. Both Abort and Close are red.
    fn buttons(&self) -> (bool, bool, bool, bool, String, &'static BtnStyle, bool, Option<String>) {
        let running = self.running;
        let aborting = running && self.bench.as_ref().is_some_and(|b| b.is_canceled());
        let done = !running && self.report.is_some();
        let aborted = done && self.report.as_ref().is_some_and(|r| r.canceled);
        let folder = done && self.html_path.is_some();
        let again_on = !self.scanning && self.chosen != 0;
        let (text, style, on, tip): (String, &'static BtnStyle, bool, Option<String>) = if aborting {
            (
                if self.close_when_done { "Closing...".into() } else { "Close".into() },
                &BTN_STOP,
                !self.close_when_done,
                Some("Closes the window as soon as the test files are removed".into()),
            )
        } else if running {
            ("Abort".into(), &BTN_STOP, true, Some("Stops the Benchmark and removes its test files. The cases that finished stay in the report.".into()))
        } else if aborted {
            ("Close".into(), &BTN_STOP, true, None)
        } else if done {
            ("Run again".into(), &BTN, !self.scanning && self.chosen != 0, None)
        } else {
            ("Start".into(), &BTN_PRIMARY, !self.scanning && self.chosen > 0, None)
        };
        (folder, aborted, again_on, aborted, text, style, on, tip)
    }

    fn begin(&mut self, h: &mut Host, b: Benchmark) {
        let b = Arc::new(b);
        self.bench = Some(b.clone());
        self.report = None;
        self.html_path = None;
        self.json_path = None;
        self.shown = 0;
        self.last_route = None;
        self.close_when_done = false;
        h.win.kill_timer(T_RESCAN);
        self.rows.clear();
        self.progress_visible = true;
        self.results_visible = true;
        self.intro_visible = false;
        self.totals = None;
        self.estimate_visible = false; // the progress line has the times now
        self.measured = None;
        self.summary_note.clear();
        self.show_summary(&[], false, false);
        self.hint = KEEP_OFF.into();
        self.bar_fraction = 0.0;
        self.step_text = "Starting".into();
        self.elapsed_val = "0 s".into();
        self.remain_label.clear();
        self.remain_val.clear();
        self.running = true;
        self.fit_to_screen(h);
        std::thread::spawn(move || {
            let rep = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| b.run())).unwrap_or_else(|e| {
                let mut r = b.report();
                r.error = Some(crate::engine::panic_message(e.as_ref()));
                r
            });
            win::post(Box::new(move || crate::app::with_bench(move |c, h| c.finished(h, rep.clone()))));
        });
        h.win.set_timer(T_TICK, 250);
    }

    fn tick(&mut self, h: &mut Host) {
        let Some(b) = self.bench.clone() else { return };
        if !self.running {
            return;
        }
        let s = b.status();
        self.step_text = if b.is_canceled() { "Aborting - removing the test files...".into() } else { s.text.clone() };
        self.bar_fraction = s.fraction;
        // the copy window's wording and colors: Elapsed 2 min    Remaining 6 min
        self.elapsed_val = format::short(s.elapsed);
        self.remain_label = if b.is_canceled() { String::new() } else { "    Remaining ".into() };
        self.remain_val = if b.is_canceled() {
            String::new()
        } else if s.elapsed.as_secs_f64() > 8.0 {
            format::short(s.remaining)
        } else {
            "…".into()
        };
        if s.results > self.shown {
            self.add_new(b.results_so_far(), false);
            self.fit_to_screen(h);
        }
        h.invalidate();
    }

    /// A run is going (an update waits for it to end).
    pub fn running(&self) -> bool {
        self.running
    }

    fn finished(&mut self, h: &mut Host, rep: BenchReport) {
        h.win.kill_timer(T_TICK);
        self.running = false;
        win::post_local(Box::new(crate::update::copies_done)); // an update that waited for the run
        self.add_new(rep.results.clone(), true);
        self.show_summary(&rep.results.clone(), true, rep.ended());
        self.totals = rep.totals();
        // the progress line stays, finished: the time it took, nothing remaining
        self.step_text = if rep.canceled {
            "Aborted"
        } else if rep.error.is_some() {
            "Couldn't finish"
        } else {
            "Finished"
        }
        .into();
        if rep.ended() {
            self.bar_fraction = 1.0;
        }
        self.elapsed_val = bt::duration(rep.seconds);
        self.remain_label.clear();
        self.remain_val.clear();
        let mut rep = rep;
        let saved = match rep.save(None) {
            Ok((html, json)) => {
                let name = html.file_name().map(|n| n.to_string_lossy().to_string());
                self.html_path = Some(html);
                self.json_path = Some(json);
                name
            }
            Err(e) => {
                rep.notes.push(format!("The report couldn't be saved: {}", crate::bench::files::io_message(&e)));
                None
            }
        };
        let mut notes = Vec::new();
        if rep.canceled {
            notes.push("Aborted before the end - the cases that finished are shown.".to_string());
        }
        if let Some(e) = &rep.error {
            notes.push(e.clone());
        }
        if rep.scored() > 0 {
            notes.push(format!(
                "The time decides: the average of each one's runs until the copies are on the disk. Less than {}% apart is a {TIE_WORD}.",
                (TIE_MARGIN * 100.0) as i32
            ));
        }
        self.summary_note = notes.join(" ");
        self.measured = rep.drives.as_ref().map(|ds| {
            let mut m = HashMap::new();
            for d in ds.iter().filter(|d| d.measured) {
                m.entry(d.letter.clone()).or_insert_with(|| d.clone());
            }
            m
        });
        self.scroll = 0.0;
        self.estimate.clear();
        self.hint = match &saved {
            Some(s) => format!("Saved in Choppy's Logs folder: {s} and .json{}", rep.notes.first().map(|n| format!(" · {n}")).unwrap_or_default()),
            None => rep.notes.join(" "),
        };
        self.report = Some(rep);
        self.fit_to_screen(h);
        if self.close_when_done {
            h.close();
        }
    }

    /// Asks before aborting (a question of its own); then the run stops (and the window closes when `close` says so).
    fn ask_abort(&mut self, h: &mut Host, close: bool) {
        if !self.running || self.bench.as_ref().is_some_and(|b| b.is_canceled()) {
            return;
        }
        crate::ui::dialogs::ask(Some(h.win), "Abort the Benchmark?", "Its test files are removed. The cases that finished stay in the report.", "Abort", "Keep going", move |yes| {
            if !yes {
                return;
            }
            crate::app::with_bench(move |c, h| {
                if !c.running {
                    return; // it ended while the question was up
                }
                if let Some(b) = &c.bench {
                    b.cancel();
                }
                if close {
                    c.close_when_done = true;
                }
                h.invalidate();
            });
        });
    }

    // ================= test hook =================

    pub fn close_for_test(&mut self, h: &mut Host) {
        if self.closing(h) {
            return;
        }
        h.close();
    }

    /// "start tiny C,D" / "start quick" / "stop" (= "abort") / "state" / "fold" / "unfold" / "max" / "maxsys" /
    /// "normal" / "size w [h]" / "move x y" / "rect" / "mode quick|thorough" / "runs n" / "press main|again|rescan" /
    /// "drive D off|on" / "ui" (the test hook drives the window without the mouse).
    pub fn for_test(&mut self, h: &mut Host, cmd: &str) -> String {
        let p: Vec<&str> = cmd.split(' ').filter(|s| !s.is_empty()).collect();
        if p.is_empty() {
            return "no command".into();
        }
        let r = match p[0] {
            "start" => {
                if self.running {
                    return "running".into();
                }
                if p.len() > 1 && p[1] != "tiny" {
                    self.thorough = p[1] == "thorough";
                    self.show_runs();
                }
                let mut b = self.new_bench();
                b.tiny = p.len() > 1 && p[1] == "tiny";
                if p.len() > 2 {
                    b.only = Some(p[2].split(',').map(|x| format!("{}:", x.trim().trim_end_matches(':').to_uppercase())).collect());
                }
                self.begin(h, b);
                "ok".into()
            }
            "stop" | "abort" => {
                // as Abort answered with Abort
                if !self.running {
                    return "not running".into();
                }
                if let Some(b) = &self.bench {
                    b.cancel();
                }
                "ok".into()
            }
            "drive" => {
                // "drive D: off": as its checkbox clicked
                let letter = p.get(1).map(|l| format!("{}:", l.trim_end_matches(':').to_uppercase())).unwrap_or_default();
                let Some(i) = self.drive_index(&letter) else { return format!("no checkbox for {letter}") };
                if !self.drive_enabled(i) {
                    return "can't be changed now".into();
                }
                let on = p.len() < 3 || p[2] != "off";
                if on {
                    self.off.remove(&letter);
                } else {
                    self.off.insert(letter);
                }
                if !self.running {
                    self.preview();
                }
                "ok".into()
            }
            "mode" => {
                self.mode_changed(p.get(1) == Some(&"thorough"));
                "ok".into()
            }
            "runs" => {
                // as typed in the box, then the box left
                self.runs_box.set(p.get(1).copied().unwrap_or(""));
                self.runs_changed();
                self.show_runs();
                format!("runs {}", self.runs())
            }
            "press" => match p.get(1).copied().unwrap_or("") {
                "main" => {
                    if self.running && !self.bench.as_ref().is_some_and(|b| b.is_canceled()) {
                        return "would ask to abort (use abort)".into();
                    }
                    self.start_click(h);
                    "ok".into()
                }
                "again" => {
                    if !self.running {
                        let b = self.new_bench();
                        self.begin(h, b);
                    }
                    "ok".into()
                }
                "rescan" => {
                    if self.bench.is_none() {
                        self.scan();
                    }
                    "ok".into()
                }
                _ => "unknown button".into(),
            },
            "ui" => {
                let (folder, report_btn, again_on, again_vis, text, style, on, _) = self.buttons();
                let mut shown = Vec::new();
                let tag = |st: &BtnStyle| {
                    if st.bg == BTN_STOP.bg {
                        " [red]"
                    } else if st.bg == BTN_PRIMARY.bg {
                        " [blue]"
                    } else {
                        ""
                    }
                };
                if folder {
                    shown.push("Open folder".to_string());
                    shown.push(format!("Open report{}", if report_btn { "" } else { " [blue]" }));
                }
                if again_vis {
                    shown.push(format!("Run again{}", if again_on { "" } else { " (off)" }));
                }
                shown.push(format!("{text}{}{}", if on { "" } else { " (off)" }, tag(style)));
                let boxes: Vec<String> = self.drive_order().iter().map(|&i| {
                    let d = &self.listed.as_ref().unwrap()[i];
                    let can = d.skip.is_none();
                    let checked = can && !self.off.contains(&d.letter);
                    format!("{}{}{}", d.letter, if checked { "[x]" } else { "[ ]" }, if self.drive_enabled(i) { "" } else { "(locked)" })
                }).collect();
                format!(
                    "buttons: {} | runs {}{} | estimate {} | rescan {}{} | drives {}{} | hint {} | step {}",
                    shown.join(", "),
                    self.runs_box.text,
                    if self.running { " (off)" } else { "" },
                    self.estimate,
                    if self.bench.is_some() {
                        "hidden"
                    } else if self.scanning {
                        "off"
                    } else {
                        "on"
                    },
                    if self.shut() { " | folded" } else { "" },
                    boxes.join(" "),
                    self.listed.as_ref().map(|l| format!(" out:{}", l.iter().filter(|d| d.skip.is_some()).map(|d| d.letter.as_str()).collect::<Vec<_>>().join(","))).unwrap_or_default(),
                    self.hint,
                    self.step_text
                )
            }
            "state" => {
                if self.running {
                    format!("running {}%", (self.bench.as_ref().map(|b| b.status().fraction).unwrap_or(0.0) * 100.0) as i32)
                } else if let Some(r) = &self.report {
                    format!(
                        "done: {}{}{} | {}",
                        r.summary(),
                        if r.canceled { " (stopped)" } else { "" },
                        r.totals().map(|t| format!(" | {}", t.line())).unwrap_or_default(),
                        self.html_path.as_ref().map(|p| p.display().to_string()).unwrap_or_default()
                    )
                } else {
                    "idle".into()
                }
            }
            "fold" | "unfold" => {
                if self.shut() != (p[0] == "fold") {
                    self.toggle_fold(h);
                }
                "ok".into()
            }
            "max" => {
                self.max_click(h);
                "ok".into()
            }
            "maxsys" => {
                // as the title bar double-clicked does it
                h.win.sys_maximize();
                "ok".into()
            }
            "normal" => {
                h.win.restore();
                "ok".into()
            }
            "size" => {
                // "size 900 600": as if both were dragged by hand; "size 900": only the width (the height follows the content)
                let s = h.scale();
                let (l, t, r_, b_) = h.win.rect();
                let mut nw = r_ - l;
                let mut nh = b_ - t;
                if p.len() > 2 {
                    self.auto_height = false;
                    h.sizing = Sizing::Free(false);
                    nh = (crate::engine::netfmt::parse_double(p[2]).unwrap_or(0.0) * s).round() as i32;
                }
                if p.len() > 1 {
                    nw = (crate::engine::netfmt::parse_double(p[1]).unwrap_or(0.0).max(800.0) * s).round() as i32;
                }
                h.win.resize(nw, nh);
                self.fit_to_screen(h);
                "ok".into()
            }
            "move" => {
                let s = h.scale();
                let x = crate::engine::netfmt::parse_double(p.get(1).copied().unwrap_or("0")).unwrap_or(0.0);
                let y = crate::engine::netfmt::parse_double(p.get(2).copied().unwrap_or("0")).unwrap_or(0.0);
                h.win.move_to((x * s).round() as i32, (y * s).round() as i32);
                "ok".into()
            }
            "rect" => {
                let s = h.scale();
                let (l, t, r_, b_) = h.win.rect();
                let wa = h.win.work_area();
                let list = h.last.as_ref().and_then(|t| t.find("ResultsList")).map(|e| e.rect.h).unwrap_or(0.0);
                let scroll = h.last.as_ref().and_then(|t| t.find("ResultsScroll")).map(|e| e.rect.h).unwrap_or(0.0);
                format!(
                    "{} {},{} {}x{} min 800x{} sizeToContent {} list {}/{} workArea {},{} {}x{} dpi {}",
                    if h.win.is_maximized() {
                        "Maximized"
                    } else if h.win.is_minimized() {
                        "Minimized"
                    } else {
                        "Normal"
                    },
                    (l as f64 / s) as i32,
                    (t as f64 / s) as i32,
                    ((r_ - l) as f64 / s) as i32,
                    ((b_ - t) as f64 / s) as i32,
                    self.need(h, list.min(120.0)).ceil() as i32,
                    if self.auto_height && !h.win.is_maximized() { "Height" } else { "Manual" },
                    scroll as i32,
                    list as i32,
                    (wa.0 as f64 / s) as i32,
                    (wa.1 as f64 / s) as i32,
                    ((wa.2 - wa.0) as f64 / s) as i32,
                    ((wa.3 - wa.1) as f64 / s) as i32,
                    s
                )
            }
            _ => "unknown".into(),
        };
        h.invalidate();
        r
    }

    fn mode_changed(&mut self, thorough: bool) {
        if self.thorough == thorough {
            return;
        }
        self.thorough = thorough;
        self.show_runs();
        if !self.running {
            self.preview();
        }
    }

    fn max_click(&mut self, h: &mut Host) {
        if h.win.is_maximized() {
            h.win.restore();
            return;
        }
        // else the content keeps the maximized window at its height
        h.sizing = Sizing::Free(false);
        h.win.maximize();
    }

    /// The drives as listed: every one that can be tested first, the system drive first, then by letter; once a run
    /// has started only those (the others share one line).
    fn drive_order(&self) -> Vec<usize> {
        let Some(list) = &self.listed else { return Vec::new() };
        let compact = self.bench.is_some();
        let mut order: Vec<usize> = (0..list.len()).filter(|&i| !compact || list[i].skip.is_none()).collect();
        order.sort_by(|&a, &b| (list[a].skip.is_some(), !list[a].system, &list[a].letter).cmp(&(list[b].skip.is_some(), !list[b].system, &list[b].letter)));
        order
    }

    fn drive_index(&self, letter: &str) -> Option<usize> {
        self.drive_order().into_iter().find(|&i| self.listed.as_ref().unwrap()[i].letter == letter)
    }

    fn drive_enabled(&self, i: usize) -> bool {
        let running = self.bench.is_some() && self.report.is_none();
        self.listed.as_ref().is_some_and(|l| l[i].skip.is_none()) && !running
    }

    fn drive_rows(&self, ui: &UiState) -> El {
        let mut list = vstack().named("DriveList");
        let Some(all) = &self.listed else { return list };
        let compact = self.bench.is_some();
        let running = self.bench.is_some() && self.report.is_none();
        let order = self.drive_order();
        let mut left: Vec<&BenchDrive> = all.iter().filter(|d| d.skip.is_some()).collect();
        left.sort_by(|a, b| a.letter.cmp(&b.letter));
        for (n, &i) in order.iter().enumerate() {
            let d = &all[i];
            let can = d.skip.is_none();
            let on = can && !self.off.contains(&d.letter);
            let name = if on {
                TEXT
            } else if can {
                DIM
            } else {
                FAINT
            };
            let label = if on {
                TEXT2
            } else if can {
                DIM
            } else {
                FAINT
            };
            let id = DRIVE0 + i as u32;
            let mut g = grid(&[Len::Px(34.0), Len::Px(34.0), Len::Px(190.0), Len::Star(1.0), Len::Auto], &[Len::Auto, Len::Auto]).m(0.0, 7.0, 0.0, 7.0);
            g.push(tick(ui, id, on, can && !running).tip_opt(if !can {
                None
            } else if on {
                Some(format!("Tested - uncheck to leave {} out", d.letter))
            } else {
                Some(format!("Left out - check to test {}", d.letter))
            }));
            g.push(text(&d.letter, sf(14.5, SEMI), name).vcenter().col(1));
            g.push(text(d.label.clone().or(d.remote.clone()).unwrap_or_default(), sf(14.0, NORMAL), label).trimmed().m(0.0, 0.0, 12.0, 0.0).vcenter().col(2));
            let detail = if can {
                let mut parts = vec![d.kind()];
                if let Some(u) = d.usb() {
                    parts.push(usb_link::short(u.speed).into());
                }
                if let Some(m) = d.model.as_ref().filter(|m| !m.is_empty()) {
                    parts.push(m.clone());
                }
                if let Some(f) = d.file_system.as_ref().filter(|f| !f.is_empty()) {
                    parts.push(f.clone());
                }
                parts.push(format!("{} free", format::size(d.free)));
                parts.join(" · ")
            } else {
                d.skip.clone().unwrap_or_default()
            };
            g.push(text(&detail, sf(13.0, NORMAL), if on { DIM } else { FAINT }).trimmed().vcenter().col(3).tip(detail.clone()));
            let m = self.measured_of(d);
            let state = if !can {
                "left out".to_string()
            } else if !on {
                String::new()
            } else if let Some(m) = m {
                format!("read {} · write {}", bt::speed(m.read_mbps), bt::speed(m.write_mbps))
            } else if d.system {
                "system drive".into()
            } else {
                String::new()
            };
            g.push(text(state, sf(13.0, NORMAL), if on { if m.is_some() { SIZE_DONE } else { LABEL } } else { FAINT }).m(12.0, 0.0, 0.0, 0.0).vcenter().col(4));
            // known before it runs: a USB drive that its port (or cable, or a hub) holds back
            if can && let Some(u) = d.usb().filter(|u| u.held_back()) {
                g.push(
                    runs(vec![("\u{E7BA}  ".into(), icons(12.0), WARN), (u.advice().unwrap_or_default(), sf(13.0, NORMAL), WARN)])
                        .wrapped()
                        .m(0.0, 4.0, 0.0, 0.0)
                        .tip(usb_link::aka(u.speed))
                        .cell(1, 2)
                        .col_span(3),
                );
            }
            let line = n < order.len() - 1 || (compact && !left.is_empty());
            list.push(border(None, Some(SEP), 0.0, 0.0, Th::default()).sides(Th::new(0.0, 0.0, 0.0, if line { 1.0 } else { 0.0 })).child(g));
        }
        if compact && !left.is_empty() {
            let t = format!("Left out: {}", left.iter().map(|d| format!("{} {}", d.letter, d.skip.clone().unwrap_or_default())).collect::<Vec<_>>().join(", "));
            list.push(text(&t, sf(12.5, NORMAL), FAINT).trimmed().m(0.0, 7.0, 0.0, 7.0).tip(t.clone()));
        }
        list
    }

    fn result_row(&self, row: &Row, first: bool) -> El {
        let r = &row.r;
        let mut g = grid(&cols(), &[]).m(0.0, 8.0, 0.0, 8.0);
        if row.new_route {
            g.push(
                vstack()
                    .child(text(r.route(), sf(14.0, SEMI), TEXT))
                    .child(text(r.route_kinds(), sf(12.0, NORMAL), FAINT).trimmed()),
            );
        }
        g.push(vstack().col(1).child(text(r.case.title(), sf(14.0, NORMAL), TEXT2)).child(text(r.case.detail(), sf(12.0, NORMAL), FAINT).trimmed()));
        if !r.scored {
            g.push(text(r.error.clone().unwrap_or_else(|| "not measured".into()), sf(13.0, NORMAL), WARN).wrapped().vcenter().col(2).col_span(3));
            g.push(badge("Error", 0, true).col(5));
        } else {
            g.push(speed_el(r.choppy_mbps, r.choppy_secs, NOW_SPEED).col(2));
            g.push(speed_el(r.windows_mbps, r.windows_secs, TEXT).col(3));
            g.push(text(bt::saved(r.saved), sf(14.0, SEMI), res(bt::saved_class(r.saved))).right().col(4));
            g.push(badge(report::word(r.verdict), r.verdict, false).col(5));
            let c: Vec<String> = r.runs.iter().filter(|x| x.choppy).map(|x| bt::secs(x.on_disk)).collect();
            let w: Vec<String> = r.runs.iter().filter(|x| !x.choppy).map(|x| bt::secs(x.on_disk)).collect();
            g = g.tip(format!(
                "Choppy: {}\nWindows: {}\nLess than {}% apart is a {TIE_WORD}{}",
                c.join(", "),
                w.join(", "),
                (TIE_MARGIN * 100.0) as i32,
                r.method.as_ref().map(|m| format!("\nChoppy's way: {m}")).unwrap_or_default()
            ));
        }
        border(None, Some(SEP), 0.0, 0.0, Th::default()).sides(Th::new(0.0, if row.new_route && !first { 1.0 } else { 0.0 }, 0.0, 0.0)).child(g)
    }

    fn totals_el(&self) -> El {
        let mut g = grid(&[Len::Star(1.0); 4], &[]).named("TotalsGrid");
        let Some(t) = &self.totals else { return g };
        let cases = format!("{}{}", t.cases, if t.cases == 1 { " case" } else { " cases" });
        let pct = vec![(bt::saved(t.avg_saved), display(24.0, SEMI), res(bt::saved_class(t.avg_saved))), (format!(" {}", t.word()), display(15.0, SEMI), res(bt::saved_class(t.avg_saved)))];
        let cells: [(&str, Vec<(String, crate::sys::gfx::Font, Argb)>, String, &str); 4] = [
            (
                "ON AVERAGE",
                pct,
                format!("average of {cases}"),
                "The average of every case's Time saved: how much less time Choppy took than Windows (green), or more (red)",
            ),
            (
                "CHOPPY AVERAGE",
                vec![(bt::speed(t.choppy_mbps), display(24.0, SEMI), NOW_SPEED)],
                format!("{} in {}", bt::size(t.bytes), bt::span(t.choppy_secs)),
                "All the bytes over all the time until the copies were on the disk, every case copied once",
            ),
            (
                "WINDOWS AVERAGE",
                vec![(bt::speed(t.windows_mbps), display(24.0, SEMI), TEXT)],
                format!("{} in {}", bt::size(t.bytes), bt::span(t.windows_secs)),
                "All the bytes over all the time until the copies were on the disk, every case copied once",
            ),
            (
                "TIME SAVED",
                vec![(bt::signed_span(t.secs_saved()), display(24.0, SEMI), res(t.secs_class()))],
                "every case copied once".into(),
                "How much less time Choppy took than Windows (green), or more (red), every case copied once",
            ),
        ];
        for (col, (cap, value, sub, tip)) in cells.into_iter().enumerate() {
            let sp = vstack()
                .m(18.0, 0.0, 12.0, 0.0)
                .child(text(cap, T_CAP, LABEL))
                .child(runs(value).m(0.0, 3.0, 0.0, 1.0).trimmed())
                .child(text(sub, sf(12.5, NORMAL), FAINT).trimmed());
            g.push(
                border(Some(Fill::Solid(0)), Some(SEP), 0.0, 0.0, Th::default())
                    .sides(Th::new(if col > 0 { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0))
                    .child(sp)
                    .col(col)
                    .tip(tip),
            );
        }
        g
    }
}

fn cols() -> Vec<Len> {
    COLS.iter().map(|&w| if w < 0.0 { Len::Star(1.0) } else { Len::Px(w) }).collect()
}

/// The average speed, and under it the average time.
fn speed_el(mbps: f64, secs: f64, c: Argb) -> El {
    vstack()
        .right()
        .m(0.0, 0.0, 4.0, 0.0)
        .child(text(bt::speed(mbps), sf(14.0, SEMI), c).right())
        .child(text(bt::secs(secs), sf(12.0, NORMAL), FAINT).right())
}

fn badge(t: &str, verdict: i32, error: bool) -> El {
    let c: u32 = if error || verdict == 0 {
        0x9AA2AD
    } else if verdict > 0 {
        0x4CC38A
    } else {
        0xF07A72
    };
    border(Some(Fill::Solid(0x24000000 | c)), Some(0x70000000 | c), 1.0, 12.0, Th::new(11.0, 3.0, 11.0, 4.0))
        .m(22.0, 0.0, 0.0, 0.0)
        .left()
        .vcenter()
        .child(text(t, sf(12.5, SEMI), 0xFF000000 | c))
}

/// The small buttons beside DRIVES (Mini): 32 x 28, a glyph.
fn mini(ui: &UiState, id: u32, glyph: &str, enabled: bool) -> El {
    let g = glyph.to_string();
    button(ui, id, &BTN_MINI, Th::default(), enabled, move |fg| text(g, icons(12.0), fg)).size(32.0, 28.0)
}

impl Content for BenchContent {
    fn build(&mut self, ui: &UiState) -> El {
        self.listed = self.drives.clone().or_else(|| self.report.as_ref().and_then(|r| r.drives.clone()));
        let maxed = false;
        let title = grid(&[], &[])
            .child(
                hstack()
                    .vcenter()
                    .m(18.0, 0.0, 0.0, 0.0)
                    .child(image(self.icon.clone()).size(20.0, 20.0))
                    .child(text("Benchmark", sf(15.0, SEMI), 0xFFE9ECF0).m(10.0, 0.0, 0.0, 1.0).vcenter()),
            )
            .child(
                hstack()
                    .right()
                    .child(title_button(ui, MIN, false, G_MINIMIZE, 10.0, 46.0).tip("Minimize"))
                    .child(title_button(ui, MAX, false, if self.is_max { G_RESTORE } else { G_MAXIMIZE }, 10.0, 46.0).tip(if self.is_max { "Restore" } else { "Maximize" }))
                    .child(title_button(ui, CLOSE, true, G_CLOSE, 12.0, 46.0).tip("Close")),
            );
        let _ = maxed;
        let running = self.running;

        // row 0: the title, the intro, the totals, the options
        let mut row0 = vstack().named("Row0").child(text("Choppy VS Windows", display(20.0, SEMI), TEXT));
        row0.push(text(self.intro(), sf(13.5, NORMAL), DIM).wrapped().line_height(20.0).m(0.0, 6.0, 0.0, 0.0).shown(self.intro_visible).named("Intro"));
        row0.push(
            border(Some(Fill::Solid(CARD)), Some(CARD_BORDER), 1.0, 9.0, Th::new(0.0, 12.0, 0.0, 13.0))
                .m(0.0, 14.0, 0.0, 0.0)
                .shown(self.totals.is_some())
                .named("TotalsCard")
                .child(self.totals_el()),
        );
        let options = grid(&[Len::Auto, Len::Auto, Len::Auto, Len::Star(1.0)], &[])
            .m(0.0, 16.0, 0.0, 0.0)
            .named("Options")
            .child(
                hstack()
                    .vcenter()
                    .child(seg(ui, QUICK, !self.thorough, "Quick", "Big files of 1-2 GB and up to 1,000 small files").m(0.0, 0.0, 6.0, 0.0).enabled(!running).opacity(if running { 0.45 } else { 1.0 }))
                    .child(seg(ui, THOROUGH, self.thorough, "Thorough", "Big files of 2-4 GB, up to 2,000 small files and a mixed folder").m(0.0, 0.0, 0.0, 0.0).enabled(!running).opacity(if running { 0.45 } else { 1.0 })),
            )
            .child(
                hstack()
                    .col(1)
                    .m(20.0, 0.0, 0.0, 0.0)
                    .vcenter()
                    .tip(self.runs_tip())
                    .named("RunsPanel")
                    .child(text("Runs", sf(14.0, NORMAL), TEXT2).vcenter().m(0.0, 0.0, 9.0, 0.0))
                    .child(self.runs_box.el(ui, RUNS, &BOX_SMALL, !running).size(44.0, 30.0)),
            )
            .child(
                toggle(ui, NETWORK, self.network, Some(text("Network drives", sf(14.0, NORMAL), TEXT2)))
                    .col(2)
                    .m(22.0, 0.0, 0.0, 0.0)
                    .vcenter()
                    .tip("Mapped network drives too: to and from the system drive, and within the share")
                    .enabled(!running)
                    .opacity(if running { 0.45 } else { 1.0 }),
            )
            .child(text(&self.estimate, sf(14.0, NORMAL), TIME).right().vcenter().col(3).shown(self.estimate_visible).named("Estimate"));
        row0.push(options);

        // row 1: the drives
        let (mut line, line_tip) = self.drives_line();
        if line.is_empty() {
            // an empty TextBlock still has its line's height
            line.push((String::new(), sf(12.5, NORMAL), DIM));
        }
        let shut = self.shut();
        let head = grid(&[Len::Auto, Len::Star(1.0), Len::Auto, Len::Auto], &[])
            .m(0.0, 13.0, 0.0, 6.0)
            .named("DrivesHead")
            .child(text("DRIVES", T_CAP, LABEL).vcenter())
            .child(runs(line).m(12.0, 0.0, 12.0, 0.0).vcenter().trimmed().col(1).tip_opt(line_tip).named("DrivesLine"))
            .child(
                mini(ui, RESCAN, G_REFRESH, !self.scanning)
                    .m(0.0, 0.0, 6.0, 0.0)
                    .vcenter()
                    .col(2)
                    .shown(self.bench.is_none())
                    .tip("Look at the drives again - a drive just plugged in, removed or mapped"),
            )
            .child(mini(ui, FOLD, if shut { G_ADD } else { G_REMOVE }, true).vcenter().col(3).tip(if shut { "Show every drive" } else { "Fold the drives to one line" }));
        let row1 = vstack().row(1).named("Row1").child(head).child(
            border(Some(Fill::Solid(CARD)), Some(CARD_BORDER), 1.0, 9.0, Th::new(14.0, 6.0, 14.0, 6.0)).shown(!shut).named("DrivesCard").child(self.drive_rows(ui)),
        );

        // row 2: progress, and the line under it
        let frac = self.bar_fraction;
        let progress = vstack()
            .m(0.0, 18.0, 0.0, 0.0)
            .shown(self.progress_visible)
            .named("ProgressPanel")
            .child(
                grid(&[], &[])
                    .child(text(&self.step_text, sf(14.0, NORMAL), TEXT2).trimmed().m(0.0, 0.0, 280.0, 0.0))
                    .child(
                        runs(vec![
                            run("Elapsed ", 14.0, NORMAL, DIM),
                            run(&self.elapsed_val, 14.0, SEMI, TIME),
                            run(&self.remain_label, 14.0, NORMAL, DIM),
                            run(&self.remain_val, 14.0, SEMI, REMAIN),
                        ])
                        .right(),
                    ),
            )
            .child(panel().h(7.0).m(0.0, 10.0, 0.0, 0.0).child(fill(TRACK, 3.5)).child(draw((0.0, 0.0), move |cv, r| {
                let w = (r.w * frac * cv.scale).round_ties_even() / cv.scale;
                if w > 0.0 {
                    cv.fill_gradient(crate::sys::gfx::Rect::new(r.x, r.y, w, r.h), 3.5, &[(0.0, 0xFF1A7FE6), (1.0, 0xFF5AC8FA)], (0.0, 0.0), (1.0, 0.0));
                }
            })));
        let row2 = vstack()
            .row(2)
            .named("Row2")
            .child(progress)
            .child(text(&self.hint, sf(12.5, NORMAL), FAINT).wrapped().m(0.0, 10.0, 0.0, 0.0).shown(!self.hint.is_empty()).named("Hint"));

        // row 3: the results
        let mut head_g = grid(&cols(), &[]).m(0.0, 8.0, 0.0, 6.0).named("ResultsHead").tip(
            "The average speed, and under it the average time, until the copies are on the disk. Time saved: how much less time Choppy took than Windows (green), or more (red)",
        );
        for (i, n) in ["ROUTE", "WHAT", "CHOPPY", "WINDOWS", "TIME SAVED", "RESULT"].iter().enumerate() {
            let mut t = text(*n, T_CAP, LABEL).col(i);
            if (2..=4).contains(&i) {
                t = t.right().m(0.0, 0.0, if i == 4 { 0.0 } else { 4.0 }, 0.0);
            }
            if i == 5 {
                t = t.m(22.0, 0.0, 0.0, 0.0);
            }
            head_g.push(t);
        }
        let mut list = vstack().named("ResultsList");
        for (i, row) in self.rows.iter().enumerate() {
            list.push(self.result_row(row, i == 0));
        }
        let results = grid(&[], &[Len::Auto, Len::Auto, Len::Star(1.0)])
            .row(3)
            .m(0.0, 18.0, 0.0, 0.0)
            .top()
            .shown(self.results_visible)
            .named("ResultsPanel")
            .child(runs(self.summary.iter().map(|(t, c)| (t.clone(), display(21.0, SEMI), *c)).collect()).wrapped().named("SummaryText"))
            .child(text(&self.summary_note, sf(13.0, NORMAL), DIM).wrapped().m(0.0, 4.0, 0.0, 0.0).row(1).named("SummaryNote"))
            .child(
                border(Some(Fill::Solid(CARD)), Some(CARD_BORDER), 1.0, 9.0, Th::new(14.0, 4.0, 14.0, 4.0))
                    .row(2)
                    .m(0.0, 12.0, 0.0, 0.0)
                    .named("ResultsCard")
                    .child(grid(&[], &[Len::Auto, Len::Star(1.0)]).child(head_g).child(scroll(self.scroll, list).row(1).max_h(self.list_max).named("ResultsScroll"))),
            );

        // row 4: the buttons
        let (folder, aborted, again_on, again_vis, start_text, start_style, start_on, start_tip) = self.buttons();
        let report_style: &BtnStyle = if aborted { &BTN } else { &BTN_PRIMARY };
        let st = start_text.clone();
        let row4 = hstack()
            .row(4)
            .m(0.0, 20.0, 0.0, 0.0)
            .right()
            .named("Row4")
            .child(
                text_button(ui, FOLDER, &BTN, "Open folder", true).h(38.0).m(0.0, 0.0, 10.0, 0.0).shown(folder).tip("Choppy's Logs folder: every Benchmark's page and .json"),
            )
            .child(text_button(ui, REPORT, report_style, "Open report", true).h(38.0).m(0.0, 0.0, 10.0, 0.0).shown(folder))
            .child(text_button(ui, AGAIN, &BTN, "Run again", again_on).h(38.0).m(0.0, 0.0, 10.0, 0.0).shown(again_vis))
            .child(
                button(ui, START, start_style, Th::hv(26.0, 0.0), start_on, move |fg| text(st, sf(14.0, SEMI), fg))
                    .h(38.0)
                    .min_w(104.0)
                    .tip_opt(start_tip),
            );

        let body = grid(&[], &[Len::Auto, Len::Auto, Len::Auto, Len::Star(1.0), Len::Auto])
            .row(1)
            .m(24.0, 2.0, 24.0, 20.0)
            .named("Body")
            .child(row0)
            .child(row1)
            .child(row2)
            .child(results)
            .child(row4);
        let root = grid(&[], &[Len::Px(46.0), Len::Star(1.0)]).child(title).child(body);
        border(Some(window_bg()), Some(WIN_BORDER), 1.0, 0.0, Th::default()).child(root)
    }

    fn click(&mut self, h: &mut Host, id: u32) {
        match id {
            MIN => h.win.minimize(),
            MAX => self.max_click(h),
            CLOSE => {
                if !self.closing(h) {
                    h.close();
                }
            }
            QUICK | THOROUGH => self.mode_changed(id == THOROUGH),
            NETWORK => {
                self.network = !self.network;
                if !self.running {
                    self.scan();
                }
            }
            RESCAN => {
                if self.bench.is_none() {
                    self.scan();
                }
            }
            FOLD => self.toggle_fold(h),
            FOLDER => match &self.html_path {
                Some(p) if p.exists() => {
                    win::shell_open("explorer.exe", Some(&format!("/select,\"{}\"", p.display())));
                }
                _ => {
                    let d = app_paths::log_dir();
                    let _ = std::fs::create_dir_all(&d);
                    win::shell_open("explorer.exe", Some(&format!("\"{}\"", d.display())));
                }
            },
            REPORT => {
                if let Some(p) = &self.html_path {
                    win::shell_open(&p.display().to_string(), None);
                }
            }
            AGAIN => {
                if !self.running {
                    let b = self.new_bench();
                    self.begin(h, b);
                }
            }
            START => self.start_click(h),
            id if id >= DRIVE0 => {
                let i = (id - DRIVE0) as usize;
                let Some(letter) = self.listed.as_ref().and_then(|l| l.get(i)).map(|d| d.letter.clone()) else { return };
                if !self.off.remove(&letter) {
                    self.off.insert(letter);
                }
                if !self.running {
                    self.preview();
                }
            }
            _ => {}
        }
        h.invalidate();
    }

    fn key(&mut self, h: &mut Host, vk: u16) -> bool {
        if vk == 0x1B && h.ui.focus != RUNS {
            if !self.closing(h) {
                h.close();
            }
            return true;
        }
        false
    }

    fn text_box(&mut self, id: u32) -> Option<(&mut TextBox, BoxStyle)> {
        (id == RUNS).then_some((&mut self.runs_box, BOX_SMALL))
    }

    fn text_changed(&mut self, _h: &mut Host, id: u32) {
        if id == RUNS {
            self.runs_changed();
        }
    }

    fn focus_lost(&mut self, h: &mut Host, id: u32) {
        if id == RUNS {
            self.show_runs();
            h.invalidate();
        }
    }

    fn timer(&mut self, h: &mut Host, id: usize) {
        match id {
            T_TICK => self.tick(h),
            T_RESCAN => {
                h.win.kill_timer(T_RESCAN);
                if self.bench.is_none() {
                    self.scan();
                }
            }
            _ => {}
        }
    }

    fn wheel(&mut self, h: &mut Host, delta: i32, x: f64, y: f64) {
        let over = h.last.as_ref().and_then(|t| t.find("ResultsScroll")).is_some_and(|e| e.rect.contains(x, y));
        if over {
            let (content, view) = h.last.as_ref().and_then(|t| t.find("ResultsScroll")).map(|e| e.extent).unwrap_or((0.0, 0.0));
            let cur = h.last.as_ref().and_then(|t| t.find("ResultsScroll")).map(|e| if let Kind::Scroll { offset } = e.kind { offset } else { 0.0 }).unwrap_or(0.0);
            self.scroll = (cur - delta as f64 / 120.0 * 48.0).clamp(0.0, (content - view).max(0.0));
            h.invalidate();
        }
    }

    /// A drive plugged in or removed: the drives are looked at again (a moment later Windows has it ready).
    fn device_change(&mut self, h: &mut Host, w: usize, volume: bool) {
        const DBT_DEVICEARRIVAL: usize = 0x8000;
        const DBT_DEVICEREMOVECOMPLETE: usize = 0x8004;
        if (w == DBT_DEVICEARRIVAL || w == DBT_DEVICEREMOVECOMPLETE) && volume && self.bench.is_none() {
            h.win.kill_timer(T_RESCAN);
            h.win.set_timer(T_RESCAN, 800);
        }
    }

    /// Dragging the top or bottom edge makes the height the user's; dragging only a side keeps it following the content.
    fn size_move(&mut self, h: &mut Host, ev: Ev) {
        match ev {
            Ev::EnterSizeMove => self.sizing_height = false,
            // 1 left, 2 right, 3 top, 4 top left, 5 top right, 6 bottom, 7 bottom left, 8 bottom right
            Ev::Sizing(edge) if edge >= 3 && !self.sizing_height => {
                self.sizing_height = true;
                self.auto_height = false;
                h.sizing = Sizing::Free(false);
                self.fit_to_screen(h);
            }
            Ev::ExitSizeMove
                if !h.win.is_maximized() && self.auto_height && h.sizing != Sizing::Free(true) => {
                    h.sizing = Sizing::Free(true);
                    self.fit_to_screen(h);
                }
            _ => {}
        }
    }

    fn state(&mut self, h: &mut Host, st: u32) {
        const SIZE_MAXIMIZED: u32 = 2;
        const SIZE_MINIMIZED: u32 = 1;
        let max = st == SIZE_MAXIMIZED;
        self.is_max = max;
        if max {
            // maximized some other way (the title bar double-clicked, Windows+Up, dragged to the top) while the height
            // still followed the content: it lets go of the height
            h.sizing = Sizing::Free(false);
        } else if st != SIZE_MINIMIZED {
            h.sizing = Sizing::Free(self.auto_height);
        }
        if st != SIZE_MINIMIZED {
            self.fit_to_screen(h);
        }
    }

    fn closing(&mut self, h: &mut Host) -> bool {
        if !self.running {
            h.win.kill_timer(T_TICK);
            h.win.kill_timer(T_RESCAN);
            return false;
        }
        // running: abort first (its test files are removed), then the window closes by itself
        if self.bench.as_ref().is_some_and(|b| b.is_canceled()) {
            self.close_when_done = true;
        } else {
            self.ask_abort(h, true);
        }
        h.invalidate();
        true
    }

    fn closed(&mut self, _h: &mut Host) {
        crate::app::bench_closed();
    }

    /// LayoutUpdated: the smallest height it can be dragged to.
    fn after_paint(&mut self, h: &Host) {
        if !h.win.is_maximized() && !h.win.is_minimized() {
            self.update_min_height(h);
        }
    }
}

/// Makes the Benchmark window and shows it.
pub fn create() -> Option<Rc<RefCell<BenchWindow>>> {
    let g = crate::app::gfx();
    let park = crate::testhook::park();
    let spec = Spec { title: "Choppy Benchmark".into(), resizable: true, minimize: true, maximize: true, taskbar: !park, ..Default::default() };
    let w = Window::create(&g, &spec, Sizing::ContentHeight(860.0), 46.0, BenchContent::new())?;
    {
        let mut m = w.borrow_mut();
        let Window { host, c } = &mut *m;
        host.resize = 6.0;
        c.icon = host.g.icon_image(crate::ui::ICO, (20.0 * crate::ui::icon_scale()).round() as u32);
        let at = if park { Place::At(-32000, 0) } else { Place::Screen };
        host.place(c, at);
        // from here the user may size it: the height follows the content until the top or bottom edge is dragged
        host.sizing = Sizing::Free(true);
        // Loaded: the drives are looked at
        c.scan();
        host.paint(c);
        c.update_min_height(host);
        host.win.show_drawn(!park);
    }
    Some(w)
}

