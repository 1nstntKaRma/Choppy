//! A fake copy job: the README demo, moving like the real engine (a sample every 250 ms, the speed held by the
//! 57 MB/s limit, or ~180 MB/s with the limit off), and `view()` = what MainWindow.Tick() would put on screen.
//! Deterministic: the same time gives the same numbers in A and B (`--at 29 --freeze` for the pictures).

use crate::format;
use crate::graph::Graph;

const MB: f64 = 1024.0 * 1024.0;

pub struct File {
    pub name: &'static str,
    pub size: i64,
}

/// In the order they are copied (by name).
pub const FILES: [File; 6] = [
    File { name: "Birthday party.mp4", size: 600 * 1024 * 1024 },
    File { name: "Cover.jpg", size: 6 * 1024 * 1024 },
    File { name: "Drone - coastline 4K.mp4", size: 1126 * 1024 * 1024 },
    File { name: "Road trip north - day 2.mov", size: 250 * 1024 * 1024 },
    File { name: "Thumbnails.zip", size: 12 * 1024 * 1024 },
    File { name: "Wedding 2025 - full ceremony.mp4", size: 1536 * 1024 * 1024 },
];

pub const FROM_NAME: &str = "D: 4TB - Shadow";
pub const FROM_PATH: &str = r"D:\Home Videos";
pub const TO_NAME: &str = "X: 4TB - uTorrent";
pub const TO_PATH: &str = r"X:\Backup\Home Videos";
/// Without the limit: what two hard disks do.
const DRIVE_SPEED: f64 = 182.0 * MB;

pub struct Demo {
    /// Copy clock (paused time not counted).
    pub t: f64,
    pub done: f64,
    pub paused: bool,
    pub frozen: bool,
    rate: f64,
    history: Vec<f64>,
    pub samples: u64,
    sample_acc: f64,
    sample_bytes: f64,
    last_second: Vec<f64>,
    end_hold: f64,
    pub graph: Graph,
    graph_seen: u64,
    // settings shown in the window (quick toggles, unit, view)
    pub extended: bool,
    pub close_when_done: bool,
    pub verify: bool,
    /// Speed limit in Mbps, 0 = off; `limit_value` is the number kept for when it is turned back on.
    pub limit_mbps: f64,
    pub limit_value_mbps: f64,
    pub mbps_unit: bool,
    pub queue: usize,
}

pub fn total() -> i64 {
    FILES.iter().map(|f| f.size).sum()
}

/// A repeatable wobble in -1..1 for sample i.
fn noise(i: u64) -> f64 {
    let mut x = i.wrapping_mul(0x9E3779B97F4A7C15) ^ 0xD1B54A32D192ED03;
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58476D1CE4E5B9);
    x ^= x >> 29;
    (x % 20001) as f64 / 10000.0 - 1.0
}

impl Demo {
    pub fn new() -> Self {
        let limit = format::limit_to_mbps(57.0, false);
        Demo {
            t: 0.0,
            done: 0.0,
            paused: false,
            frozen: false,
            rate: 0.0,
            history: Vec::new(),
            samples: 0,
            sample_acc: 0.0,
            sample_bytes: 0.0,
            last_second: Vec::new(),
            end_hold: 0.0,
            graph: Graph::new(),
            graph_seen: 0,
            extended: true,
            close_when_done: true,
            verify: false,
            limit_mbps: limit,
            limit_value_mbps: limit,
            mbps_unit: false,
            queue: 0,
        }
    }

    /// Jump to `secs` into the copy (the history fills as if it had run).
    pub fn start_at(&mut self, secs: f64) {
        let steps = (secs / 0.05).round() as usize;
        for _ in 0..steps {
            self.advance(0.05);
        }
    }

    fn target(&self) -> f64 {
        if self.limit_mbps > 0.0 { self.limit_mbps * 1e6 / 8.0 } else { DRIVE_SPEED }
    }

    /// Moves the copy on by `dt` seconds (called every 100 ms by the windows).
    pub fn advance(&mut self, dt: f64) {
        if self.frozen {
            return;
        }
        if self.done >= total() as f64 {
            self.end_hold += dt;
            if self.end_hold > 3.0 {
                self.restart();
            }
            return;
        }
        if self.paused {
            return;
        }
        self.t += dt;
        // the speed follows the limit within a second or so, with the wobble of a real copy (the limiter lets a
        // little through, then holds back, every couple of seconds); the first second it climbs from almost nothing
        let i = self.samples;
        let ramp = if self.t < 1.2 { 0.06 } else if self.t < 2.2 { 1.1 } else { 1.0 };
        let k = i / 7;
        let f = (i % 7) as f64 / 7.0;
        let slow = noise(k + 911) * (1.0 - f) + noise(k + 912) * f;
        let want = self.target() * ramp * (1.0 + 0.055 * slow + 0.025 * noise(i));
        self.rate += (want - self.rate) * (dt / 0.2).min(1.0);
        let bytes = (self.rate * dt).min(total() as f64 - self.done);
        self.done += bytes;
        self.sample_acc += dt;
        self.sample_bytes += bytes;
        while self.sample_acc >= 0.25 - 1e-9 {
            self.sample_acc -= 0.25;
            let s = self.sample_bytes / 0.25;
            self.sample_bytes = 0.0;
            self.history.push(s);
            if self.history.len() > 600 {
                self.history.remove(0);
            }
            self.last_second.push(s);
            if self.last_second.len() > 4 {
                self.last_second.remove(0);
            }
            self.samples += 1;
        }
    }

    pub fn restart(&mut self) {
        let keep = (self.extended, self.close_when_done, self.verify, self.limit_mbps, self.limit_value_mbps, self.mbps_unit);
        *self = Demo::new();
        (self.extended, self.close_when_done, self.verify, self.limit_mbps, self.limit_value_mbps, self.mbps_unit) = keep;
    }

    /// Skip: the rest of the current file is left out (here: jumps to the next file).
    pub fn skip(&mut self) {
        let mut start = 0.0;
        for f in FILES.iter() {
            let end = start + f.size as f64;
            if self.done < end {
                self.done = end;
                return;
            }
            start = end;
        }
    }

    pub fn finished(&self) -> bool {
        self.done >= total() as f64
    }

    pub fn average(&self) -> f64 {
        if self.t > 0.0 { self.done / self.t } else { 0.0 }
    }

    fn speed_second(&self) -> f64 {
        if self.last_second.is_empty() { 0.0 } else { self.last_second.iter().sum::<f64>() / self.last_second.len() as f64 }
    }

    /// The window's text and numbers right now (MainWindow.Tick).
    pub fn view(&mut self) -> View {
        let total = total();
        let done = (self.done as i64).min(total);
        let finished = self.finished();
        let running = !finished && !self.paused;
        let frac = if finished { 1.0 } else { (done as f64 / total as f64).clamp(0.0, 1.0) };
        let mbps = self.mbps_unit;
        let avg = self.average();
        let now = self.speed_second();
        let (sn, su) = format::speed(if running { now } else { 0.0 }, mbps);
        let (an, au) = format::speed(avg, mbps);
        if self.samples != self.graph_seen && !finished {
            self.graph_seen = self.samples;
            self.graph.set_samples(&self.history);
        }
        if avg > 0.0 {
            self.graph.set_average(avg);
        }
        let (size_done, size_total) = format::pair(done, total);
        let mut start = 0i64;
        let mut files_done = 0;
        let mut cur = &FILES[FILES.len() - 1];
        let mut cur_done = cur.size;
        for f in FILES.iter() {
            if done >= start + f.size {
                files_done += 1;
            } else {
                cur = f;
                cur_done = done - start;
                break;
            }
            start += f.size;
        }
        let left = (running && avg > 1.0 && (total - done) > 0 && self.t >= 2.0).then(|| (total - done) as f64 / avg);
        let (cd, ct) = format::pair(cur_done, cur.size);
        View {
            percent: format!("{}%", (frac * 100.0).floor() as i64),
            frac,
            badge_mode: if self.paused { "Paused".into() } else { "Adaptive".into() },
            badge_detail: if self.paused { String::new() } else { "1 stream".into() },
            badge_warn: self.paused,
            speed_num: if running { sn.clone() } else { "–".into() },
            speed_unit: if running { format!(" {su}") } else { String::new() },
            avg_num: if avg > 0.0 { an.clone() } else { "–".into() },
            avg_unit: if avg > 0.0 { format!(" {au}") } else { String::new() },
            c_now: if running { format!("{sn} {su}") } else { String::new() },
            c_avg_label: if avg > 0.0 { if running { "    AVG  ".into() } else { "AVG  ".into() } } else { String::new() },
            c_avg: if avg > 0.0 { format!("{an} {au}") } else { String::new() },
            size_done,
            size_of: " of ".into(),
            size_total,
            files: format!("{} of {} files", format::count(files_done), format::count(FILES.len() as i64)),
            elapsed: format::short(self.t),
            remain_label: if finished { String::new() } else { "    Remaining ".into() },
            remain: if finished {
                String::new()
            } else if self.paused {
                "paused".into()
            } else {
                left.map(format::short).unwrap_or_else(|| "…".into())
            },
            cur_label: "CURRENT FILE".into(),
            cur_name: cur.name.into(),
            cur_done: cd,
            cur_of: " of ".into(),
            cur_total: ct,
            cur_frac: (cur_done as f64 / cur.size as f64).min(1.0),
            c2_label: if self.paused { "Paused  ".into() } else { "Copying  ".into() },
            c2_warn: self.paused,
            c2_name: cur.name.into(),
            pause_text: if self.paused { "Resume".into() } else { "Pause".into() },
            limit_unit: if mbps { "Mbps" } else { "MB/s" },
            limit_text: format::limit(if self.limit_mbps > 0.0 { self.limit_mbps } else { self.limit_value_mbps }, mbps),
            title: format!(
                "{}%{} - Choppy",
                (frac * 100.0) as i64,
                if running {
                    format!(" · {} {} (avg {} {})", format::speed(now, mbps).0, su, an, au)
                } else {
                    String::new()
                }
            ),
        }
    }

    /// The donut + legend: the four largest files, then "Other n files" (MainWindow.FillSplit).
    pub fn largest() -> Vec<(String, i64, u32)> {
        let mut by_size: Vec<&File> = FILES.iter().collect();
        by_size.sort_by(|a, b| b.size.cmp(&a.size));
        let mut out: Vec<(String, i64, u32)> =
            by_size.iter().take(4).enumerate().map(|(i, f)| (f.name.to_string(), f.size, crate::color::SPLIT[i])).collect();
        let top: i64 = out.iter().map(|p| p.1).sum();
        let others = FILES.len() - 4;
        out.push((format!("Other {} files", others), total() - top, crate::color::OTHER));
        out
    }

    /// The limit typed in the box (whole number in the unit shown).
    pub fn set_limit_text(&mut self, text: &str) {
        let v = format::parse_limit(text);
        if v <= 0 {
            return;
        }
        self.limit_value_mbps = format::limit_to_mbps(v as f64, self.mbps_unit);
        if self.limit_mbps > 0.0 {
            self.limit_mbps = self.limit_value_mbps;
        }
    }

    pub fn set_limit_on(&mut self, on: bool) {
        self.limit_mbps = if on { self.limit_value_mbps.max(0.1) } else { 0.0 };
    }
}

impl Default for Demo {
    fn default() -> Self {
        Self::new()
    }
}

/// Everything the copy window shows that changes while copying.
pub struct View {
    pub percent: String,
    pub frac: f64,
    pub badge_mode: String,
    pub badge_detail: String,
    pub badge_warn: bool,
    pub speed_num: String,
    pub speed_unit: String,
    pub avg_num: String,
    pub avg_unit: String,
    pub c_now: String,
    pub c_avg_label: String,
    pub c_avg: String,
    pub size_done: String,
    pub size_of: String,
    pub size_total: String,
    pub files: String,
    pub elapsed: String,
    pub remain_label: String,
    pub remain: String,
    pub cur_label: String,
    pub cur_name: String,
    pub cur_done: String,
    pub cur_of: String,
    pub cur_total: String,
    pub cur_frac: f64,
    pub c2_label: String,
    pub c2_warn: bool,
    pub c2_name: String,
    pub pause_text: String,
    pub limit_unit: &'static str,
    pub limit_text: String,
    pub title: String,
}

/// Command line shared by A and B: `--at <secs>` start into the copy, `--freeze` hold still (pictures),
/// `--compact` start in the compact view, `--x/--y <px>` place the window (screen pixels).
pub struct Args {
    pub at: f64,
    pub freeze: bool,
    pub compact: bool,
    pub pos: Option<(i32, i32)>,
    pub exit_after: Option<f64>,
    /// `--dpi 96`: draw at this DPI whatever the screen's (pictures at 100 %).
    pub dpi: Option<u32>,
}

impl Args {
    pub fn parse() -> Args {
        let a: Vec<String> = std::env::args().collect();
        let mut r = Args { at: 0.0, freeze: false, compact: false, pos: None, exit_after: None, dpi: None };
        let mut x = None;
        let mut y = None;
        let mut i = 1;
        while i < a.len() {
            match a[i].as_str() {
                "--at" => { i += 1; r.at = a.get(i).and_then(|s| s.parse().ok()).unwrap_or(0.0); }
                "--freeze" => r.freeze = true,
                "--compact" => r.compact = true,
                "--x" => { i += 1; x = a.get(i).and_then(|s| s.parse().ok()); }
                "--y" => { i += 1; y = a.get(i).and_then(|s| s.parse().ok()); }
                "--exit-after" => { i += 1; r.exit_after = a.get(i).and_then(|s| s.parse().ok()); }
                "--dpi" => { i += 1; r.dpi = a.get(i).and_then(|s| s.parse().ok()); }
                _ => {}
            }
            i += 1;
        }
        if let (Some(x), Some(y)) = (x, y) {
            r.pos = Some((x, y));
        }
        r
    }

    /// A demo set up as the arguments say.
    pub fn demo(&self) -> Demo {
        let mut d = Demo::new();
        d.extended = !self.compact;
        if self.at > 0.0 {
            d.start_at(self.at);
        }
        d.frozen = self.freeze;
        d
    }
}
