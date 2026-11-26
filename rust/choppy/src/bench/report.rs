//! BenchReport.cs: the drives, cases, runs and results, every measured case together (BenchTotals), the whole run
//! (BenchReport) saved as an HTML page and as JSON in the 1.48 formats, and BenchText (numbers as the report shows
//! them, the same on every PC).

use std::path::{Path, PathBuf};

use crate::engine::settings::app_paths;
use crate::engine::usb_link::{self, UsbLink};
use crate::engine::volume_probe::{DeviceKind, VolumeInfo};
use crate::engine::netfmt;

use super::benchmark::{MB, TIE_MARGIN, default_rounds, judge, mean};

#[derive(Clone, Debug, Default)]
pub struct BenchDrive {
    /// "C:"
    pub letter: String,
    /// "C:\"
    pub root: String,
    pub label: Option<String>,
    pub model: Option<String>,
    pub bus: Option<String>,
    pub file_system: Option<String>,
    pub remote: Option<String>,
    pub size: i64,
    pub free: i64,
    pub system: bool,
    pub network: bool,
    pub vol: Option<VolumeInfo>,
    /// Why it isn't tested (None = tested).
    pub skip: Option<String>,
    /// The reason found when the drives were listed (a preview adds free space to it).
    pub found_skip: Option<String>,
    /// Its temporary folder while the benchmark runs.
    pub test_dir: Option<PathBuf>,
    pub read_mbps: f64,
    pub write_mbps: f64,
    pub measured: bool,
    /// A disk device that describes itself (not a cloud or virtual file system).
    pub real_disk: bool,
    /// Opening a file past the cache drops it from the cache here.
    pub purge_ok: bool,
    pub purge_note: Option<String>,
}

impl BenchDrive {
    pub fn new(letter: &str, root: &str) -> BenchDrive {
        BenchDrive { letter: letter.into(), root: root.into(), purge_ok: true, ..Default::default() }
    }

    /// A USB drive: how it is connected (None = not USB, or Windows doesn't say).
    pub fn usb(&self) -> Option<&UsbLink> {
        self.vol.as_ref().and_then(|v| v.usb.as_ref())
    }

    /// "NVMe SSD", "Hard disk", "USB SSD", "Network share"...
    pub fn kind(&self) -> String {
        if self.network {
            return "Network share".into();
        }
        let Some(v) = &self.vol else { return "Drive".into() };
        let bus = self.bus.as_deref();
        match v.kind {
            DeviceKind::Ssd => {
                return match bus {
                    Some("NVMe") => "NVMe SSD",
                    Some("USB") => "USB SSD",
                    Some("SATA") => "SATA SSD",
                    _ => "SSD",
                }
                .into();
            }
            DeviceKind::Hdd => return if bus == Some("USB") { "USB hard disk".into() } else { "Hard disk".into() },
            DeviceKind::Usb => return "USB drive".into(),
            DeviceKind::Ram => return "RAM disk".into(),
            DeviceKind::Optical => return "Disc".into(),
            _ => {}
        }
        match bus {
            Some(b) => format!("{b} disk"),
            None => "Disk".into(),
        }
    }

    /// Short kind for the route lines: "NVMe", "HDD", "USB SSD".
    pub fn short(&self) -> String {
        let k = self.kind();
        match k.as_str() {
            "NVMe SSD" => "NVMe".into(),
            "SATA SSD" | "SSD" => "SSD".into(),
            "Hard disk" => "HDD".into(),
            "USB hard disk" => "USB HDD".into(),
            "USB drive" => "USB".into(),
            "Network share" => "Network".into(),
            "RAM disk" => "RAM".into(),
            _ => k,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct BenchCase {
    /// "big", "small", "mixed"
    pub key: String,
    pub big: Vec<i64>,
    pub smalls: Vec<i64>,
}

impl BenchCase {
    pub fn files(&self) -> usize {
        self.big.len() + self.smalls.len()
    }
    pub fn bytes(&self) -> i64 {
        self.big.iter().sum::<i64>() + self.smalls.iter().sum::<i64>()
    }
    pub fn title(&self) -> String {
        match self.key.as_str() {
            "big" => "One big file".into(),
            "small" => format!("{} small files", n0(self.smalls.len() as i64)),
            _ => "Mixed folder".into(),
        }
    }
    pub fn detail(&self) -> String {
        match self.key.as_str() {
            "big" => text::size(self.bytes()),
            "small" => format!("{} · 4 KB - 1 MB each", text::size(self.bytes())),
            _ => format!("{} · {} big + {} small", text::size(self.bytes()), self.big.len(), n0(self.smalls.len() as i64)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BenchRun {
    pub choppy: bool,
    /// Seconds until it said it was done ...
    pub done: f64,
    /// ... and until its copies were on the disk.
    pub on_disk: f64,
}

#[derive(Clone, Debug, Default)]
pub struct BenchResult {
    /// The drives of the route (as they were when it ran) and which of the run's drives they are.
    pub from: BenchDrive,
    pub to: BenchDrive,
    pub from_i: usize,
    pub to_i: usize,
    pub case: BenchCase,
    pub runs: Vec<BenchRun>,
    /// The way Choppy's Adaptive mode picked.
    pub method: Option<String>,
    pub error: Option<String>,
    /// Average seconds until the copies are on the disk: these decide.
    pub choppy_secs: f64,
    pub windows_secs: f64,
    /// The bytes over those average times.
    pub choppy_mbps: f64,
    pub windows_mbps: f64,
    /// Until each said it was done (kept in the raw data only).
    pub choppy_done_mbps: f64,
    pub windows_done_mbps: f64,
    /// How much less time Choppy took than Windows (0.12 = 12% less; below 0 = more).
    pub saved: f64,
    pub done_saved: f64,
    /// +1 Faster, 0 Tie (a Marginal Tie: less than 4% apart), -1 Slower.
    pub verdict: i32,
    pub done_verdict: i32,
    pub scored: bool,
}

impl BenchResult {
    pub fn within(&self) -> bool {
        self.from_i == self.to_i
    }
    pub fn route(&self) -> String {
        format!("{} → {}", self.from.letter, self.to.letter)
    }
    pub fn route_kinds(&self) -> String {
        if self.within() { format!("within one {}", self.from.kind()) } else { format!("{} → {}", self.from.short(), self.to.short()) }
    }

    fn times(&self, choppy: bool, disk: bool) -> Vec<f64> {
        self.runs.iter().filter(|r| r.choppy == choppy).map(|r| if disk { r.on_disk } else { r.done }).collect()
    }

    pub fn score(&mut self) {
        let (v, s) = judge(&self.times(true, true), &self.times(false, true));
        self.verdict = v;
        self.saved = s;
        let (v, s) = judge(&self.times(true, false), &self.times(false, false));
        self.done_verdict = v;
        self.done_saved = s;
        self.choppy_secs = mean(&self.times(true, true));
        self.windows_secs = mean(&self.times(false, true));
        self.choppy_mbps = self.speed(&self.times(true, true));
        self.windows_mbps = self.speed(&self.times(false, true));
        self.choppy_done_mbps = self.speed(&self.times(true, false));
        self.windows_done_mbps = self.speed(&self.times(false, false));
        self.scored = self.error.is_none() && self.choppy_mbps > 0.0 && self.windows_mbps > 0.0;
    }

    /// The average speed: the bytes over the average time of the runs.
    fn speed(&self, secs: &[f64]) -> f64 {
        let m = mean(secs);
        if m > 0.0 { self.case.bytes() as f64 / MB as f64 / m } else { 0.0 }
    }
}

/// Every measured case together: the average of their time saved, each one's average speed (all the bytes over all
/// the time, every case copied once) and the time Choppy saved, added up.
#[derive(Clone, Debug, Default)]
pub struct BenchTotals {
    pub cases: usize,
    /// The average of the cases' time saved (0.12 = 12% less time; below 0 = more).
    pub avg_saved: f64,
    /// Every case once.
    pub bytes: i64,
    /// The cases' average times, added up.
    pub choppy_secs: f64,
    pub windows_secs: f64,
    /// The bytes over those times.
    pub choppy_mbps: f64,
    pub windows_mbps: f64,
}

impl BenchTotals {
    /// How much less time Choppy took, every case copied once (below 0 = more).
    pub fn secs_saved(&self) -> f64 {
        self.windows_secs - self.choppy_secs
    }

    /// None when no case was measured.
    pub fn of<'a>(results: impl IntoIterator<Item = &'a BenchResult>) -> Option<BenchTotals> {
        let rs: Vec<&BenchResult> = results.into_iter().filter(|r| r.scored).collect();
        if rs.is_empty() {
            return None;
        }
        let mut t = BenchTotals {
            cases: rs.len(),
            avg_saved: rs.iter().map(|r| r.saved).sum::<f64>() / rs.len() as f64,
            bytes: rs.iter().map(|r| r.case.bytes()).sum(),
            choppy_secs: rs.iter().map(|r| r.choppy_secs).sum(),
            windows_secs: rs.iter().map(|r| r.windows_secs).sum(),
            ..Default::default()
        };
        t.choppy_mbps = if t.choppy_secs > 0.0 { t.bytes as f64 / MB as f64 / t.choppy_secs } else { 0.0 };
        t.windows_mbps = if t.windows_secs > 0.0 { t.bytes as f64 / MB as f64 / t.windows_secs } else { 0.0 };
        Some(t)
    }

    /// "faster" / "slower" / "even", as the average rounds to a whole percent.
    pub fn word(&self) -> &'static str {
        match text::saved_class(self.avg_saved) {
            "good" => "faster",
            "bad" => "slower",
            _ => "even",
        }
    }

    /// "good" / "bad" / "tie" for the seconds saved (to a hundredth of a second).
    pub fn secs_class(&self) -> &'static str {
        let s = round_dec(self.secs_saved(), 2);
        if s > 0.0 {
            "good"
        } else if s < 0.0 {
            "bad"
        } else {
            "tie"
        }
    }

    /// "+23% faster on average · Choppy 1.21 GB/s · Windows 980 MB/s · Time saved 1 min 2 s"
    pub fn line(&self) -> String {
        format!(
            "{} {} on average · Choppy {} · Windows {} · Time saved {}",
            text::saved(self.avg_saved),
            self.word(),
            text::speed(self.choppy_mbps),
            text::speed(self.windows_mbps),
            text::signed_span(self.secs_saved())
        )
    }
}

/// Math.Round(v, digits) (to even, as .NET).
fn round_dec(v: f64, digits: i32) -> f64 {
    let p = 10f64.powi(digits);
    (v * p).round_ties_even() / p
}

/// A local date and time (DateTime) for the report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stamp {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl Stamp {
    pub fn now() -> Stamp {
        let ft = crate::sys::now_local_file_time();
        let w = crate::ui::format::When::from_local_file_time(ft);
        Stamp { year: w.year, month: w.month, day: w.day, hour: w.hour, minute: w.minute, second: ((ft / 10_000_000) % 60) as u32 }
    }
    const MONTHS: [&'static str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    /// "d MMM yyyy, HH:mm"
    pub fn long(&self) -> String {
        format!("{} {} {}, {:02}:{:02}", self.day, Self::MONTHS[(self.month as usize).clamp(1, 12) - 1], self.year, self.hour, self.minute)
    }
    /// "yyyy-MM-ddTHH:mm:ss"
    pub fn iso(&self) -> String {
        format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", self.year, self.month, self.day, self.hour, self.minute, self.second)
    }
    /// "yyyy-MM-dd HH-mm-ss"
    pub fn file(&self) -> String {
        format!("{:04}-{:02}-{:02} {:02}-{:02}-{:02}", self.year, self.month, self.day, self.hour, self.minute, self.second)
    }
}

/// The whole run: this PC, its drives, every case's numbers. Saved as an HTML page and as JSON.
#[derive(Clone, Debug, Default)]
pub struct BenchReport {
    pub version: String,
    pub started: Stamp,
    pub seconds: f64,
    pub mode: String,
    pub rounds: i32,
    pub planned: usize,
    pub cpu: Option<String>,
    pub windows: Option<String>,
    pub threads: i32,
    pub ram_gb: f64,
    pub admin: bool,
    pub drives: Option<Vec<BenchDrive>>,
    pub results: Vec<BenchResult>,
    pub notes: Vec<String>,
    pub canceled: bool,
    pub error: Option<String>,
}

/// The summary and the notes say "Marginal Tie"; each case's result says "Tie" (the notes explain it once).
pub const TIE_WORD: &str = "Marginal Tie";

/// The verdict as each case's result says it.
pub fn word(verdict: i32) -> &'static str {
    if verdict > 0 {
        "Faster"
    } else if verdict < 0 {
        "Slower"
    } else {
        "Tie"
    }
}

/// "10 of 19"
pub fn of(count: usize, n: usize) -> String {
    format!("{count} of {n}")
}

/// "Choppy is Faster 10 of 19 · Marginal Tie 9 of 19 · Never Slower": a part for each verdict there is; "Never
/// Slower" only when a whole run ended with ties and no case slower (all faster says it already).
pub fn summary_of(faster: usize, ties: usize, slower: usize, ended: bool) -> String {
    let n = faster + ties + slower;
    if n == 0 {
        return "No case was measured".into();
    }
    let mut s = format!("Choppy is Faster {}", of(faster, n));
    if ties > 0 {
        s += &format!(" · {TIE_WORD} {}", of(ties, n));
    }
    if slower > 0 {
        s += &format!(" · Slower {}", of(slower, n));
    } else if ended && ties > 0 {
        s += " · Never Slower";
    }
    s
}

impl BenchReport {
    pub fn new() -> BenchReport {
        BenchReport { version: env!("CHOPPY_VERSION").into(), ..Default::default() }
    }

    pub fn faster(&self) -> usize {
        self.results.iter().filter(|r| r.scored && r.verdict > 0).count()
    }
    pub fn ties(&self) -> usize {
        self.results.iter().filter(|r| r.scored && r.verdict == 0).count()
    }
    pub fn slower(&self) -> usize {
        self.results.iter().filter(|r| r.scored && r.verdict < 0).count()
    }
    pub fn scored(&self) -> usize {
        self.results.iter().filter(|r| r.scored).count()
    }
    /// Every measured case together (None when none was).
    pub fn totals(&self) -> Option<BenchTotals> {
        BenchTotals::of(self.results.iter())
    }
    /// The whole run ended (not stopped, no error): only then can it say "Never Slower".
    pub fn ended(&self) -> bool {
        !self.canceled && self.error.is_none()
    }
    /// "Choppy is Faster 9 of 12 · Marginal Tie 2 of 12 · Slower 1 of 12"
    pub fn summary(&self) -> String {
        summary_of(self.faster(), self.ties(), self.slower(), self.ended())
    }
    /// The same by the time each one said it was done (kept in the raw data).
    pub fn done_summary(&self) -> String {
        let f = self.results.iter().filter(|r| r.scored && r.done_verdict > 0).count();
        let t = self.results.iter().filter(|r| r.scored && r.done_verdict == 0).count();
        let s = self.results.iter().filter(|r| r.scored && r.done_verdict < 0).count();
        summary_of(f, t, s, false)
    }

    pub fn read_machine(&mut self) {
        self.cpu = crate::sys::hklm_string(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0", "ProcessorNameString").map(|s| s.trim().to_string());
        self.threads = std::thread::available_parallelism().map(|n| n.get() as i32).unwrap_or(1);
        if let Some(total) = crate::sys::total_memory() {
            self.ram_gb = round_dec(total as f64 / (1u64 << 30) as f64, 1);
        }
        let k = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
        let mut name = crate::sys::hklm_string(k, "ProductName").unwrap_or_else(|| "Windows".into());
        let build = crate::sys::hklm_string(k, "CurrentBuild").unwrap_or_default();
        let disp = crate::sys::hklm_string(k, "DisplayVersion").or_else(|| crate::sys::hklm_string(k, "ReleaseId"));
        let ubr = crate::sys::hklm_dword(k, "UBR");
        // Windows 11 still calls itself "Windows 10" here
        if build.parse::<i32>().is_ok_and(|b| b >= 22000) {
            name = name.replace("Windows 10", "Windows 11");
        }
        self.windows = Some(format!(
            "{name}{} (build {build}{})",
            disp.map(|d| format!(" {d}")).unwrap_or_default(),
            ubr.map(|u| format!(".{u}")).unwrap_or_default()
        ));
        self.admin = crate::sys::is_admin();
    }

    // ================= saving =================

    /// Saves benchmark-<date>.html and .json in Choppy's Logs folder (or next to a given .json path). Returns (html, json).
    pub fn save(&self, json_path: Option<&Path>) -> std::io::Result<(PathBuf, PathBuf)> {
        let (json, html) = match json_path {
            None => {
                let dir = app_paths::log_dir();
                std::fs::create_dir_all(&dir)?;
                let b = dir.join(format!("benchmark-{}", self.started.file()));
                (b.with_extension("json").with_file_name(format!("benchmark-{}.json", self.started.file())), dir.join(format!("benchmark-{}.html", self.started.file())))
            }
            Some(p) => {
                let html = p.with_extension("html");
                if let Some(d) = p.parent()
                    && !d.as_os_str().is_empty()
                {
                    std::fs::create_dir_all(d)?;
                }
                (p.to_path_buf(), html)
            }
        };
        std::fs::write(&json, self.to_json())?;
        let name = json.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        std::fs::write(&html, self.to_html(&name))?;
        Ok((html, json))
    }

    // ================= JSON =================

    pub fn to_json(&self) -> String {
        let mut j = String::new();
        j += "{\n";
        j += "  \"tool\": \"Choppy Benchmark\",\n";
        j += &format!("  \"version\": {},\n", s(Some(&self.version)));
        j += &format!("  \"date\": {},\n", s(Some(&self.started.iso())));
        j += &format!("  \"mode\": {}, \"rounds\": {}, \"seconds\": {},\n", s(Some(&self.mode)), self.rounds, n(self.seconds, 1, 1));
        j += &format!("  \"canceled\": {}, \"error\": {},\n", self.canceled, s(self.error.as_deref()));
        j += "  \"units\": \"seconds = the average time until the copies are on the disk; MB/s = 1,048,576 bytes per second over that time; timeSaved = how much less time Choppy took than Windows (below 0 = more)\",\n";
        j += &format!(
            "  \"machine\": {{ \"cpu\": {}, \"threads\": {}, \"ramGB\": {}, \"windows\": {}, \"admin\": {} }},\n",
            s(self.cpu.as_deref()),
            self.threads,
            n(self.ram_gb, 3, 0),
            s(self.windows.as_deref()),
            self.admin
        );
        j += &format!(
            "  \"summary\": {{ \"text\": {}, \"cases\": {}, \"faster\": {}, \"marginalTie\": {}, \"slower\": {}, \"byDone\": {} }},\n",
            s(Some(&self.summary())),
            self.scored(),
            self.faster(),
            self.ties(),
            self.slower(),
            s(Some(&self.done_summary()))
        );
        if let Some(t) = self.totals() {
            j += &format!(
                "  \"totals\": {{ \"cases\": {}, \"averageTimeSaved\": {}, \"bytes\": {}, \"choppySeconds\": {}, \"windowsSeconds\": {}, \"secondsSaved\": {}, \"choppyMBps\": {}, \"windowsMBps\": {}, \"about\": \"averageTimeSaved = the average of the cases' timeSaved; seconds = the cases' average times added up (every case copied once); MBps = all the bytes over those seconds\" }},\n",
                t.cases,
                n(t.avg_saved, 4, 4),
                t.bytes,
                n(t.choppy_secs, 3, 3),
                n(t.windows_secs, 3, 3),
                n(t.secs_saved(), 3, 3),
                n(t.choppy_mbps, 1, 1),
                n(t.windows_mbps, 1, 1)
            );
        }
        j += "  \"drives\": [";
        let empty = Vec::new();
        let ds = self.drives.as_ref().unwrap_or(&empty);
        for (i, d) in ds.iter().enumerate() {
            j += if i == 0 { "\n" } else { ",\n" };
            j += &format!(
                "    {{ \"letter\": {}, \"label\": {}, \"kind\": {}, \"model\": {}, \"bus\": {}, \"fileSystem\": {}, \"sizeGB\": {}, \"freeGB\": {}, \"system\": {}, \"network\": {}, \"physicalDisk\": {}, \"tested\": {}, \"skipped\": {}, \"readMBps\": {}, \"writeMBps\": {}, \"cacheDrop\": {}, \"usb\": {} }}",
                s(Some(&d.letter)),
                s(d.label.as_deref()),
                s(if d.skip.is_none() || d.vol.is_some() { Some(d.kind()) } else { None }.as_deref()),
                s(d.model.as_deref()),
                s(d.bus.as_deref()),
                s(d.file_system.as_deref()),
                n(d.size as f64 / (1u64 << 30) as f64, 1, 1),
                n(d.free as f64 / (1u64 << 30) as f64, 1, 1),
                d.system,
                d.network,
                d.vol.as_ref().map(|v| v.disk).unwrap_or(-1),
                d.skip.is_none() && d.measured,
                s(d.skip.as_deref()),
                if d.measured { n(d.read_mbps, 0, 0) } else { "null".into() },
                if d.measured { n(d.write_mbps, 0, 0) } else { "null".into() },
                s(d.purge_note.as_deref()),
                usb_json(d.usb())
            );
        }
        j += "\n  ],\n  \"results\": [";
        for (i, r) in self.results.iter().enumerate() {
            j += if i == 0 { "\n" } else { ",\n" };
            j += &format!(
                "    {{ \"from\": {}, \"to\": {}, \"route\": {}, \"kinds\": {}, \"case\": {}, \"title\": {}, \"files\": {}, \"bytes\": {},\n",
                s(Some(&r.from.letter)),
                s(Some(&r.to.letter)),
                s(Some(&format!("{} -> {}", r.from.letter, r.to.letter))),
                s(Some(&r.route_kinds().replace('→', "->"))),
                s(Some(&r.case.key)),
                s(Some(&r.case.title())),
                r.case.files(),
                r.case.bytes()
            );
            j += &format!(
                "      \"verdict\": {}, \"timeSaved\": {}, \"doneVerdict\": {}, \"doneTimeSaved\": {},\n",
                if r.scored { s(Some(verdict(r.verdict))) } else { "null".into() },
                n(r.saved, 4, 4),
                if r.scored { s(Some(verdict(r.done_verdict))) } else { "null".into() },
                n(r.done_saved, 4, 4)
            );
            j += &format!(
                "      \"choppy\": {{ \"seconds\": {}, \"MBps\": {}, \"doneMBps\": {}, \"method\": {}, \"runs\": {} }},\n",
                n(r.choppy_secs, 3, 3),
                n(r.choppy_mbps, 1, 1),
                n(r.choppy_done_mbps, 1, 1),
                s(r.method.as_deref()),
                runs_json(r, true)
            );
            j += &format!(
                "      \"windows\": {{ \"seconds\": {}, \"MBps\": {}, \"doneMBps\": {}, \"runs\": {} }},\n",
                n(r.windows_secs, 3, 3),
                n(r.windows_mbps, 1, 1),
                n(r.windows_done_mbps, 1, 1),
                runs_json(r, false)
            );
            j += &format!("      \"error\": {} }}", s(r.error.as_deref()));
        }
        j += "\n  ],\n  \"notes\": [";
        for (i, note) in self.notes.iter().enumerate() {
            j += &format!("{}{}", if i == 0 { "" } else { ", " }, s(Some(note)));
        }
        j += "]\n}\n";
        j
    }

    /// The tested drives that run slower than they can, because of the port, cable or hub: "F: runs at USB 2.0 ..."
    pub fn held_back(&self) -> Vec<String> {
        self.drives
            .iter()
            .flatten()
            .filter(|d| d.usb().is_some_and(|u| u.held_back()) && (d.skip.is_none() || d.measured))
            .map(|d| format!("{} {}", d.letter, d.usb().and_then(|u| u.advice()).unwrap_or_default()))
            .collect()
    }

    /// "Quick", or "Quick, 5 runs each" when the runs were changed from the default.
    fn mode_text(&self) -> String {
        let mut m = cap(&self.mode);
        if self.rounds > 0 && self.rounds != default_rounds(self.mode == "thorough") {
            m += &format!(", {}{}", self.rounds, if self.rounds == 1 { " run each" } else { " runs each" });
        }
        m
    }

    // ================= HTML =================

    /// The summary line with each verdict in its color.
    fn summary_html(&self) -> String {
        let n = self.scored();
        let mut h = format!("Choppy is <span class=\"good\">Faster {}</span>", of(self.faster(), n));
        if self.ties() > 0 {
            h += &format!(" · <span class=\"tie\">{TIE_WORD} {}</span>", of(self.ties(), n));
        }
        if self.slower() > 0 {
            h += &format!(" · <span class=\"bad\">Slower {}</span>", of(self.slower(), n));
        } else if self.ended() && self.ties() > 0 {
            h += " · <span class=\"good\">Never Slower</span>";
        }
        h
    }

    /// Average time saved · Choppy's average speed · Windows' · the time saved, each in its color (none measured: nothing).
    fn totals_html(&self) -> String {
        let Some(t) = self.totals() else { return String::new() };
        let cases = format!("{}{}", t.cases, if t.cases == 1 { " case" } else { " cases" });
        format!(
            "<section class=\"totals\">{}{}{}{}</section>\n",
            stat(
                "On average",
                &format!("<span class=\"{}\">{}<small> {}</small></span>", text::saved_class(t.avg_saved), h(&text::saved(t.avg_saved)), t.word()),
                &format!("average of {cases}"),
                "The average of every case's Time saved"
            ),
            stat(
                "Choppy average",
                &format!("<span class=\"c\">{}</span>", h(&text::speed(t.choppy_mbps))),
                &format!("{} in {}", text::size(t.bytes), text::span(t.choppy_secs)),
                "All the bytes over all the time, every case copied once"
            ),
            stat(
                "Windows average",
                &format!("<span class=\"w\">{}</span>", h(&text::speed(t.windows_mbps))),
                &format!("{} in {}", text::size(t.bytes), text::span(t.windows_secs)),
                "All the bytes over all the time, every case copied once"
            ),
            stat(
                "Time saved",
                &format!("<span class=\"{}\">{}</span>", t.secs_class(), h(&text::signed_span(t.secs_saved()))),
                "every case copied once",
                "How much less time Choppy took than Windows (green), or more (red)"
            )
        )
    }

    pub fn to_html(&self, json_name: &str) -> String {
        let mut o = String::new();
        o += "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n";
        o += &format!("<title>Choppy Benchmark · {}</title>\n<style>\n{CSS}</style></head><body><div class=\"wrap\">\n", h(&self.started.long()));

        // header
        let icon = text::icon_data_uri();
        o += &format!(
            "<header>{}<div><h1>Choppy VS Windows</h1><div class=\"dim\">Choppy v{} Benchmark · {} · {} · took {}</div></div></header>\n",
            icon.map(|i| format!("<img src=\"{i}\" width=\"52\" height=\"52\" alt=\"\">")).unwrap_or_default(),
            h(&self.version),
            h(&self.started.long()),
            h(&self.mode_text()),
            h(&text::duration(self.seconds))
        );

        // the whole run in four numbers
        o += &self.totals_html();

        // summary
        let n = self.scored();
        o += "<section class=\"card\">";
        if self.canceled {
            o += "<div class=\"warn\">Stopped before the end: these are the cases that finished.</div>";
        }
        if let Some(e) = &self.error {
            o += &format!("<div class=\"warn\">{}</div>", h(e));
        }
        if n > 0 {
            o += &format!("<div class=\"summary\">{}</div>", self.summary_html());
            o += &format!(
                "<p class=\"dim\">The time decides: the average of each one's runs until the copies are on the disk. Less than {}% apart is a {TIE_WORD}.</p>",
                (TIE_MARGIN * 100.0) as i32
            );
        } else if !self.canceled && self.error.is_none() {
            o += "<div class=\"summary\">No case was measured</div>";
        }
        for hb in self.held_back() {
            o += &format!("<p class=\"warn\">Held back: {} Both copies ran at that speed.</p>", h(&hb));
        }
        o += "</section>\n";

        // results
        if !self.results.is_empty() {
            o += "<section class=\"card\"><h2>Results</h2><div class=\"scroll\"><table class=\"res\"><thead><tr><th>Route</th><th>What</th><th class=\"r\">Choppy</th><th class=\"r\">Windows</th><th class=\"r\">Time saved</th><th>Result</th></tr></thead><tbody>\n";
            // GroupBy keeps the order each group first appears in
            let mut groups: Vec<(String, Vec<&BenchResult>)> = Vec::new();
            for r in self.results.iter() {
                let k = format!("{}|{}", r.route(), r.route_kinds());
                match groups.iter_mut().find(|g| g.0 == k) {
                    Some(g) => g.1.push(r),
                    None => groups.push((k, vec![r])),
                }
            }
            for (_, rows) in groups {
                for (i, r) in rows.iter().enumerate() {
                    o += &format!("<tr{}>", if i == 0 { " class=\"first\"" } else { "" });
                    if i == 0 {
                        o += &format!("<td rowspan=\"{}\" class=\"route\"><b>{}</b><div class=\"faint\">{}</div></td>", rows.len(), h(&r.route()), h(&r.route_kinds()));
                    }
                    o += &format!("<td>{}<div class=\"faint\">{}</div></td>", h(&r.case.title()), h(&r.case.detail()));
                    if !r.scored {
                        o += &format!(
                            "<td colspan=\"3\" class=\"faint\">{}</td><td><span class=\"badge b-err\">Error</span></td></tr>\n",
                            h(r.error.as_deref().unwrap_or("not measured"))
                        );
                        continue;
                    }
                    let max = r.choppy_mbps.max(r.windows_mbps);
                    o += &format!("<td class=\"r\"><span class=\"c\">{}</span><div class=\"faint\">{}</div></td>", h(&text::speed(r.choppy_mbps)), h(&text::secs(r.choppy_secs)));
                    o += &format!("<td class=\"r\"><span class=\"w\">{}</span><div class=\"faint\">{}</div></td>", h(&text::speed(r.windows_mbps)), h(&text::secs(r.windows_secs)));
                    o += &format!(
                        "<td class=\"r\"><b class=\"{}\">{}</b><div class=\"bars\"><i class=\"bc\" style=\"width:{}%\"></i><i class=\"bw\" style=\"width:{}%\"></i></div></td>",
                        text::saved_class(r.saved),
                        h(&text::saved(r.saved)),
                        pct(r.choppy_mbps, max),
                        pct(r.windows_mbps, max)
                    );
                    o += &format!("<td><span class=\"badge b-{}\">{}</span></td></tr>\n", cls(r.verdict), word(r.verdict));
                }
            }
            o += "</tbody></table></div><p class=\"faint\">The average speed, and under it the average time, until the copies are on the disk. Time saved: how much less time Choppy took than Windows (green), or more (red). Bars: <span class=\"c\">Choppy</span> and <span class=\"w\">Windows</span>.</p></section>\n";
        }

        // this PC
        o += "<section class=\"card\"><h2>This PC</h2><dl>";
        o += &format!("<dt>CPU</dt><dd>{} · {} threads</dd>", h(self.cpu.as_deref().unwrap_or("unknown")), self.threads);
        o += &format!("<dt>Memory</dt><dd>{} GB</dd>", h(&inv(self.ram_gb, 1, 0)));
        o += &format!("<dt>Windows</dt><dd>{}</dd>", h(self.windows.as_deref().unwrap_or("")));
        o += &format!("<dt>Choppy</dt><dd>v{}{}</dd>", h(&self.version), if self.admin { " · ran as administrator" } else { "" });
        o += "</dl>";
        o += "<div class=\"scroll\"><table class=\"drv\"><thead><tr><th>Drive</th><th>Kind</th><th>Model</th><th>File system</th><th class=\"r\">Size</th><th class=\"r\">Free</th><th class=\"r\">Measured</th><th>Tested</th></tr></thead><tbody>\n";
        for d in self.drives.iter().flatten() {
            o += &format!(
                "<tr><td><b>{}</b> {}{}</td>",
                h(&d.letter),
                h(d.label.as_deref().unwrap_or("")),
                if d.system { " <span class=\"faint\">(system)</span>" } else { "" }
            );
            let usb = d.usb().map(|u| {
                format!(
                    "<div class=\"{}\" title=\"{}\">{}{}{}</div>",
                    if u.held_back() { "warn" } else { "faint" },
                    h(usb_link::aka(u.speed)),
                    h(usb_link::name(u.speed)),
                    if u.held_back() { format!(" - can do {}", h(usb_link::short(u.capable))) } else { String::new() },
                    if u.uasp { " · UASP" } else { "" }
                )
            });
            o += &format!("<td>{}{}</td>", h(&if d.vol.is_some() || d.network { d.kind() } else { String::new() }), usb.unwrap_or_default());
            o += &format!(
                "<td>{}{}</td>",
                h(d.model.as_deref().unwrap_or("")),
                d.bus
                    .as_ref()
                    .map(|b| format!(
                        "<div class=\"faint\">{}{}</div>",
                        h(b),
                        match &d.vol {
                            Some(v) if v.disk >= 0 => format!(" · disk {}", v.disk),
                            _ => String::new(),
                        }
                    ))
                    .unwrap_or_default()
            );
            o += &format!("<td>{}</td>", h(d.file_system.as_deref().unwrap_or("")));
            o += &format!(
                "<td class=\"r\">{}</td><td class=\"r\">{}</td>",
                if d.size > 0 { h(&text::size(d.size)) } else { String::new() },
                if d.size > 0 { h(&text::size(d.free)) } else { String::new() }
            );
            o += &format!(
                "<td class=\"r\">{}</td>",
                if d.measured { format!("read {}<div class=\"faint\">write {}</div>", h(&text::speed(d.read_mbps)), h(&text::speed(d.write_mbps))) } else { String::new() }
            );
            o += &format!(
                "<td>{}</td></tr>\n",
                if d.skip.is_none() && d.measured {
                    "<span class=\"good\">yes</span>".to_string()
                } else {
                    format!("<span class=\"faint\">{}</span>", h(d.skip.as_deref().unwrap_or(if self.canceled { "stopped before it" } else { "no" })))
                }
            );
        }
        o += "</tbody></table></div></section>\n";

        // every run
        if !self.results.is_empty() {
            o += "<section class=\"card\"><details><summary><h2>Every run</h2></summary><div class=\"scroll\"><table class=\"runs\"><thead><tr><th>Route</th><th>What</th><th>Choppy (s)</th><th>Windows (s)</th><th>Choppy's way</th></tr></thead><tbody>\n";
            for r in self.results.iter() {
                o += &format!(
                    "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"faint\">{}</td></tr>\n",
                    h(&r.route()),
                    h(&r.case.title()),
                    run_list(r, true),
                    run_list(r, false),
                    h(r.method.as_deref().unwrap_or(""))
                );
            }
            o += "</tbody></table></div><p class=\"faint\">Seconds until the copies were on the disk, in the order they ran: each case alternates (Choppy, Windows, Windows, Choppy - or the other way round).</p></details></section>\n";
        }

        // how
        o += "<section class=\"card\"><h2>How it's measured</h2><ul class=\"dim\">";
        o += "<li>Routes: from the system drive to each other drive and back, and within each drive.</li>";
        o += &format!("<li>Each case is copied {} times by each, in turns, into a new folder. The source is never in memory: it is read once untimed (a new file's first open is slower), then dropped from the Windows cache before every copy.</li>", self.rounds);
        o += "<li>Sizes go by each route's speed: 2 GB and 1,000 small files where it does 350 MB/s or more, 1 GB and 500 files below that, 512 MB and 250 under 40 MB/s (Thorough: twice the big file and the small files, and a mixed folder).</li>";
        o += "<li>Choppy: its own engine, as every copy runs it (Adaptive picks the way per drive). Windows: the shell copy engine Explorer uses for a paste (IFileOperation), without its window.</li>";
        o += "<li>Each time runs until the copies are on the disk: what is still in memory when a program says it's done is written out (every copy is flushed). The numbers are the average of each one's runs.</li>";
        o += &format!("<li>Test files go into a temporary folder on each drive (\"{}\"), removed at the end.</li>", h(super::files::FOLDER_NAME));
        o += "</ul>";
        for note in self.notes.iter() {
            o += &format!("<p class=\"warn\">{}</p>", h(note));
        }
        o += "</section>\n";
        o += &format!("<footer class=\"faint\">The raw numbers: {} (next to this page).</footer>\n", h(json_name));
        o += "</div></body></html>\n";
        o
    }
}

/// `v.ToString(fmt, InvariantCulture)` for "0.###" (max 3, min 0), "0.0" (1, 1), "0" (0, 0)...: rounded half away
/// from zero like .NET Framework.
pub fn inv(v: f64, max: usize, min: usize) -> String {
    let s = netfmt::f(v, max);
    let s = if max > min && s.contains('.') {
        let (a, b) = s.split_once('.').unwrap_or((&s, ""));
        let mut b = b.to_string();
        while b.len() > min && b.ends_with('0') {
            b.pop();
        }
        if b.is_empty() { a.to_string() } else { format!("{a}.{b}") }
    } else {
        s
    };
    if s.starts_with('-') && s[1..].chars().all(|c| c == '0' || c == '.') { s[1..].to_string() } else { s }
}

/// N(v, fmt): a number in the JSON, null for NaN / infinity.
fn n(v: f64, max: usize, min: usize) -> String {
    if v.is_nan() || v.is_infinite() { "null".into() } else { inv(v, max, min) }
}

/// "N0" with the invariant culture.
fn n0(v: i64) -> String {
    crate::ui::format::count_inv(v)
}

/// A JSON string ("\u00xx" for control characters, lower-case hex), or null.
fn s(v: Option<&str>) -> String {
    let Some(v) = v else { return "null".into() };
    let mut o = String::from("\"");
    for c in v.chars() {
        if c == '"' || c == '\\' {
            o.push('\\');
            o.push(c);
        } else if (c as u32) < 32 {
            o += &format!("\\u{:04x}", c as u32);
        } else {
            o.push(c);
        }
    }
    o.push('"');
    o
}

fn verdict(v: i32) -> &'static str {
    if v > 0 {
        "faster"
    } else if v < 0 {
        "slower"
    } else {
        "marginal tie"
    }
}

/// { "runsAt": "USB 2.0", "canDo": "5 Gbps", "port": "USB 3", "uasp": false, "heldBack": true } or null
fn usb_json(u: Option<&UsbLink>) -> String {
    let Some(u) = u else { return "null".into() };
    format!(
        "{{ \"runsAt\": {}, \"canDo\": {}, \"port\": {}, \"uasp\": {}, \"heldBack\": {}, \"advice\": {} }}",
        s(Some(usb_link::short(u.speed))),
        if u.capable_known { s(Some(usb_link::short(u.capable))) } else { "null".into() },
        if u.port_known { s(Some(if u.port_usb3 { "USB 3" } else { "USB 2" })) } else { "null".into() },
        u.uasp,
        u.held_back(),
        s(u.advice().as_deref())
    )
}

fn runs_json(r: &BenchResult, choppy: bool) -> String {
    let parts: Vec<String> =
        r.runs.iter().filter(|x| x.choppy == choppy).map(|x| format!("{{ \"done\": {}, \"onDisk\": {} }}", n(x.done, 3, 3), n(x.on_disk, 3, 3))).collect();
    format!("[{}]", parts.join(", "))
}

fn cap(v: &str) -> String {
    let mut c = v.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// WebUtility.HtmlEncode on .NET Framework: < > " ' & as entities, characters 160-255 and those past the first plane
/// as numeric entities.
pub fn h(v: &str) -> String {
    let mut o = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '<' => o += "&lt;",
            '>' => o += "&gt;",
            '"' => o += "&quot;",
            '\'' => o += "&#39;",
            '&' => o += "&amp;",
            c if (160..256).contains(&(c as u32)) || c as u32 >= 0x10000 => o += &format!("&#{};", c as u32),
            c => o.push(c),
        }
    }
    o
}

fn cls(verdict: i32) -> &'static str {
    if verdict > 0 {
        "good"
    } else if verdict < 0 {
        "bad"
    } else {
        "tie"
    }
}

fn stat(cap: &str, value_html: &str, sub: &str, title: &str) -> String {
    format!(
        "<div class=\"stat\" title=\"{}\"><div class=\"cap\">{}</div><div class=\"big\">{value_html}</div><div class=\"faint\">{}</div></div>",
        h(title),
        h(cap),
        h(sub)
    )
}

fn pct(v: f64, max: f64) -> String {
    n(if max > 0.0 { (100.0 * v / max).max(2.0) } else { 0.0 }, 1, 0)
}

fn run_list(r: &BenchResult, choppy: bool) -> String {
    r.runs.iter().filter(|x| x.choppy == choppy).map(|x| h(&n(x.on_disk, 2, 2))).collect::<Vec<_>>().join("<br>")
}

const CSS: &str = concat!(
    ":root{--card:#171A1F;--border:#262B33;--sep:#2E343C;--text:#F1F3F5;--dim:#9AA2AD;--label:#8A929E;--faint:#6B737E;--good:#4CC38A;--bad:#F07A72;--warn:#E8B04B;--c:#5AC8FA;--w:#DDE1E6}\n",
    "*{box-sizing:border-box}html{background:#1B1F25}\n",
    "body{margin:0;min-height:100vh;background:linear-gradient(#232931,#1B1F25 420px);color:var(--text);font:14px/1.5 \"Segoe UI Variable Text\",\"Segoe UI\",system-ui,sans-serif}\n",
    ".wrap{max-width:1060px;margin:0 auto;padding:28px 16px 40px}\n",
    "header{display:flex;gap:16px;align-items:center;margin-bottom:6px}h1{font:600 26px/1.2 \"Segoe UI Variable Display\",\"Segoe UI\",sans-serif;margin:0 0 3px}\n",
    "h2{font-size:12px;letter-spacing:.06em;text-transform:uppercase;color:var(--label);margin:0 0 10px;font-weight:600;display:inline-block}\n",
    ".card{background:var(--card);border:1px solid var(--border);border-radius:10px;padding:16px 18px;margin-top:14px}\n",
    ".summary{font-size:23px;font-weight:600;line-height:1.3}.summary+p{margin:8px 0 0}\n",
    ".totals{display:grid;grid-template-columns:repeat(4,1fr);background:var(--card);border:1px solid var(--border);border-radius:10px;margin-top:14px}\n",
    ".stat{padding:13px 18px 14px;border-left:1px solid var(--sep);min-width:0}.stat:first-child{border-left:0}\n",
    ".cap{font-size:12px;letter-spacing:.06em;text-transform:uppercase;color:var(--label);font-weight:600}\n",
    ".big{font:600 28px/1.25 \"Segoe UI Variable Display\",\"Segoe UI\",sans-serif;margin:5px 0 2px;font-variant-numeric:tabular-nums;white-space:nowrap}.big small{font-size:17px}\n",
    ".dim{color:var(--dim)}.faint{color:var(--faint);font-size:12.5px}.good{color:var(--good)}.bad{color:var(--bad)}.tie{color:var(--dim)}.warn{color:var(--warn);margin:0 0 8px}\n",
    ".c{color:var(--c);font-weight:600}.w{color:var(--w);font-weight:600}\n",
    ".scroll{overflow-x:auto}table{width:100%;border-collapse:collapse}\n",
    "th{font-size:11.5px;letter-spacing:.04em;text-transform:uppercase;color:var(--label);text-align:left;font-weight:600;padding:6px 10px;border-bottom:1px solid var(--sep);white-space:nowrap}\n",
    "td{padding:9px 10px;border-bottom:1px solid var(--sep);vertical-align:top}tr.first td{border-top:1px solid #3A414B}tbody tr:first-child td{border-top:0}\n",
    ".r{text-align:right;font-variant-numeric:tabular-nums;white-space:nowrap}td.route{white-space:nowrap}\n",
    ".bars{width:120px;margin:5px 0 0 auto}.bars i{display:block;height:5px;border-radius:3px;margin-top:3px}.bc{background:var(--c)}.bw{background:#7A828E}\n",
    ".badge{display:inline-block;padding:3px 11px;border-radius:999px;font-size:12.5px;font-weight:600;white-space:nowrap}\n",
    ".b-good{background:rgba(76,195,138,.14);color:var(--good);border:1px solid rgba(76,195,138,.45)}\n",
    ".b-bad{background:rgba(240,122,114,.13);color:var(--bad);border:1px solid rgba(240,122,114,.45)}\n",
    ".b-tie,.b-err{background:rgba(154,162,173,.1);color:var(--dim);border:1px solid rgba(154,162,173,.35)}\n",
    "dl{display:grid;grid-template-columns:max-content 1fr;gap:4px 18px;margin:0 0 14px}dt{color:var(--label)}dd{margin:0}\n",
    "details summary{cursor:pointer;list-style:none}details summary::-webkit-details-marker{display:none}details summary h2::after{content:\"  ▸ show\";color:var(--faint);text-transform:none;letter-spacing:0}details[open] summary h2::after{content:\"  ▾\"}\n",
    "ul{margin:0;padding-left:20px}li{margin:3px 0}footer{margin-top:16px;text-align:center}\n",
    "@media (max-width:640px){.summary{font-size:19px}td,th{padding:7px 6px}.bars{width:70px}.totals{grid-template-columns:1fr 1fr}.stat:nth-child(3){border-left:0}.stat:nth-child(n+3){border-top:1px solid var(--sep)}.big{font-size:23px}}\n",
);

/// BenchText: numbers as the report shows them (the same on every PC, whatever its language settings).
pub mod text {
    use super::{inv, round_dec};

    pub fn size(bytes: i64) -> String {
        let v = bytes as f64;
        if v >= (1000i64 << 20) as f64 {
            let v = v / (1i64 << 30) as f64;
            return format!("{} GB", if v >= 10.0 { inv(v, 0, 0) } else { inv(v, 1, 0) });
        }
        if v >= (1000i64 << 10) as f64 {
            return format!("{} MB", inv(v / (1i64 << 20) as f64, 0, 0));
        }
        format!("{} KB", inv(v / 1024.0, 0, 0))
    }

    pub fn speed(mbps: f64) -> String {
        if mbps <= 0.0 {
            return "-".into();
        }
        if mbps >= 1000.0 {
            return format!("{} GB/s", inv(mbps / 1024.0, 2, 2));
        }
        format!("{} MB/s", if mbps >= 100.0 { inv(mbps, 0, 0) } else { inv(mbps, 1, 1) })
    }

    /// "+17%" (Choppy took less time) / "−11%" (more) / "0%".
    pub fn saved(saved: f64) -> String {
        let p = (saved * 100.0).round_ties_even();
        if p > 0.0 {
            format!("+{}%", inv(p, 0, 0))
        } else if p < 0.0 {
            format!("−{}%", inv(-p, 0, 0))
        } else {
            "0%".into()
        }
    }

    /// "good" when Choppy took less time, "bad" when more, "tie" when that rounds to 0%.
    pub fn saved_class(saved: f64) -> &'static str {
        let p = (saved * 100.0).round_ties_even();
        if p > 0.0 {
            "good"
        } else if p < 0.0 {
            "bad"
        } else {
            "tie"
        }
    }

    /// "0.41 s", "5.90 s", "23.4 s"
    pub fn secs(s: f64) -> String {
        format!("{} s", if s < 10.0 { inv(s, 2, 2) } else if s < 100.0 { inv(s, 1, 1) } else { inv(s, 0, 0) })
    }

    /// "0.41 s", "12.3 s", "1 min 12 s", "1 h 5 min"
    pub fn span(secs_: f64) -> String {
        if secs_ < 59.95 { secs(secs_) } else { duration(secs_) }
    }

    /// Span with "−" before a time lost; "0 s" when it rounds to nothing.
    pub fn signed_span(s: f64) -> String {
        if round_dec(s, 2) == 0.0 {
            return "0 s".into();
        }
        if s < 0.0 { format!("−{}", span(-s)) } else { span(s) }
    }

    pub fn duration(secs: f64) -> String {
        // TimeSpan.FromSeconds rounds to whole milliseconds
        let ms = (secs * 1000.0).round();
        let total_min = ms / 60000.0;
        if total_min < 1.0 {
            return format!("{} s", secs.round_ties_even() as i64);
        }
        let s = ((ms / 1000.0) as i64) % 60;
        if total_min / 60.0 < 1.0 {
            return format!("{} min {} s", total_min as i64, s);
        }
        format!("{} h {} min", (total_min / 60.0) as i64, (total_min as i64) % 60)
    }

    /// Choppy's icon as a PNG data URI for the report (None when it can't be read): the smallest frame at least 104
    /// pixels wide, as WPF's PngBitmapEncoder writes it.
    pub fn icon_data_uri() -> Option<String> {
        let png = crate::sys::gfx::ico_frame_png(crate::ui::ICO, 104)?;
        Some(format!("data:image/png;base64,{}", base64(&png)))
    }

    fn base64(b: &[u8]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut o = String::with_capacity(b.len().div_ceil(3) * 4);
        for c in b.chunks(3) {
            let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
            o.push(T[(n >> 18) as usize & 63] as char);
            o.push(T[(n >> 12) as usize & 63] as char);
            o.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
            o.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
        }
        o
    }
}
