//! Where Choppy keeps its things, the user's settings, and the crash-safe job records (Settings.cs).

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use super::model::{ConflictPolicy, DirItem, FileItem, JobSpec, OpKind, Recorded};
use super::{handoff, netfmt, paths};
use crate::sys;
use crate::wstr::WString;

pub mod app_paths {
    use super::*;

    fn choppy_data() -> Option<std::ffi::OsString> {
        std::env::var_os("CHOPPY_DATA").filter(|v| !v.is_empty())
    }

    /// Test mode points everything at a scratch folder (CHOPPY_DATA).
    pub fn data_dir() -> PathBuf {
        if let Some(over) = choppy_data() {
            return PathBuf::from(over);
        }
        let local = sys::local_app_data().map(|w| PathBuf::from(w.to_os())).unwrap_or_default();
        local.join("Choppy")
    }
    pub fn job_dir() -> PathBuf {
        data_dir().join("Jobs")
    }
    pub fn log_dir() -> PathBuf {
        data_dir().join("Logs")
    }
    pub fn test_mode() -> bool {
        choppy_data().is_some()
    }
}

/// User settings in HKCU\Software\Choppy (or a settings.ini in the data folder in test mode). They stay as the user
/// set them through updates; settings saved before v1.48 (no SettingsVersion) are put back to the defaults once.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub verify_copies: bool,                // check every copy with xxHash64 (copies and moves)
    pub policy: ConflictPolicy,
    pub speed_limit_mbps: f64,              // 0 = off
    pub limit_value_mbps: f64,              // the limit's value, kept while it is off (12 MB/s)
    pub close_when_done: bool,              // close the window when a copy finishes with no problems
    pub close_after_seconds: i32,           // ... this many seconds after it (0 = right away)
    pub explorer_takeover: bool,            // Ctrl+V in Explorer + right-click "Paste with Choppy"
    pub speed_unit: Option<String>,         // auto | mbps | mbs
    pub ultracopier_run_backup: Option<String>, // Ultracopier's startup entry, put back when takeover is turned off
    pub extended: bool,                     // copy window: extended view (else compact)
    pub bench_runs_quick: i32,              // Benchmark: runs each of a Quick / Thorough run (0 = the default, 2 / 3)
    pub bench_runs_thorough: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            verify_copies: false,
            policy: ConflictPolicy::Ask,
            speed_limit_mbps: 0.0,
            limit_value_mbps: 12.0 * 8.0 * 1.048576,
            close_when_done: true,
            close_after_seconds: 0,
            explorer_takeover: false,
            speed_unit: Some("auto".into()),
            ultracopier_run_backup: None,
            extended: false,
            bench_runs_quick: 0,
            bench_runs_thorough: 0,
        }
    }
}

/// The settings' layout. Settings saved with an older one (before v1.48: none) are put back to the defaults once -
/// all but the Default file manager (chosen at install, and it changes Explorer) and what Choppy keeps to undo it.
/// NEVER raise it: that would reset the user's settings again.
const LAYOUT: i32 = 2;

/// no longer kept
const GONE: [&str; 1] = ["BenchDrives"];

const KEY: &str = r"Software\Choppy";

pub const MAX_BENCH_RUNS: i32 = 99;

static CURRENT: OnceLock<Mutex<Settings>> = OnceLock::new();

/// A key -> value map with OrdinalIgnoreCase keys (Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)).
#[derive(Default)]
struct Values(HashMap<WString, String>);

impl Values {
    fn set(&mut self, k: &WString, v: String) {
        self.0.insert(k.to_upper_invariant(), v);
    }
    fn get(&self, k: &str) -> Option<&str> {
        self.0.get(&WString::from(k).to_upper_invariant()).map(|s| s.as_str())
    }
}

impl Settings {
    /// Settings.Current: loaded on first use.
    pub fn current() -> MutexGuard<'static, Settings> {
        CURRENT.get_or_init(|| Mutex::new(Settings::load())).lock().unwrap()
    }

    /// Settings.Current when no other thread holds it right now (a crash handler can't wait).
    pub fn try_current() -> Option<MutexGuard<'static, Settings>> {
        CURRENT.get()?.try_lock().ok()
    }

    pub fn load() -> Settings {
        Self::load_from(KEY)
    }

    pub(crate) fn load_from(key: &str) -> Settings {
        let mut s = Settings::default();
        // as the C# try { } catch { }: a failure keeps what was read so far
        let Ok(d) = read(key) else { return s };
        if let Some(v) = d.get("VerifyCopies") {
            s.verify_copies = v == "1";
        }
        if let Some(v) = d.get("Policy")
            && let Some(p) = ConflictPolicy::parse(v) {
                s.policy = p;
            }
        if let Some(v) = d.get("SpeedLimitMbps")
            && let Some(x) = netfmt::parse_double(v) {
                s.speed_limit_mbps = x;
            }
        if let Some(v) = d.get("LimitValueMbps")
            && let Some(x) = netfmt::parse_double(v)
                && x > 0.0 {
                    s.limit_value_mbps = x;
                }
        if s.speed_limit_mbps > 0.0 {
            s.limit_value_mbps = s.speed_limit_mbps;
        }
        if let Some(v) = d.get("CloseWhenDone") {
            s.close_when_done = v != "0";
        }
        if let Some(v) = d.get("CloseAfterSeconds")
            && let Some(x) = netfmt::parse_int(v)
                && (0..=999).contains(&x) {
                    s.close_after_seconds = x;
                }
        if let Some(v) = d.get("ExplorerTakeover") {
            s.explorer_takeover = v == "1";
        }
        if let Some(v) = d.get("SpeedUnit") {
            s.speed_unit = Some(v.to_string());
        }
        if let Some(v) = d.get("UltracopierRunBackup") {
            s.ultracopier_run_backup = Some(v.to_string());
        }
        if let Some(v) = d.get("View") {
            s.extended = v == "extended";
        }
        if let Some(v) = d.get("BenchRunsQuick") {
            s.bench_runs_quick = runs(v);
        }
        if let Some(v) = d.get("BenchRunsThorough") {
            s.bench_runs_thorough = runs(v);
        }
        let layout = d.get("SettingsVersion").and_then(netfmt::parse_int);
        if layout.is_none_or(|l| l < LAYOUT) {
            // from an older version (or none yet): the defaults, once; from now on they stay as they are set
            s = Settings {
                explorer_takeover: s.explorer_takeover,
                ultracopier_run_backup: s.ultracopier_run_backup.clone(),
                ..Settings::default()
            };
            s.save_to(key);
        }
        s
    }

    pub fn save(&self) {
        self.save_to(KEY);
    }

    pub(crate) fn save_to(&self, key: &str) {
        let d: Vec<(&str, String)> = vec![
            ("VerifyCopies", if self.verify_copies { "1" } else { "0" }.into()),
            ("Policy", self.policy.name().into()),
            ("SpeedLimitMbps", netfmt::g(self.speed_limit_mbps)),
            ("LimitValueMbps", netfmt::g(self.limit_value_mbps)),
            ("CloseWhenDone", if self.close_when_done { "1" } else { "0" }.into()),
            ("CloseAfterSeconds", self.close_after_seconds.to_string()),
            ("ExplorerTakeover", if self.explorer_takeover { "1" } else { "0" }.into()),
            ("SpeedUnit", self.speed_unit.clone().unwrap_or_else(|| "auto".into())),
            ("UltracopierRunBackup", self.ultracopier_run_backup.clone().unwrap_or_default()),
            ("View", if self.extended { "extended" } else { "compact" }.into()),
            ("BenchRunsQuick", self.bench_runs_quick.to_string()),
            ("BenchRunsThorough", self.bench_runs_thorough.to_string()),
            ("SettingsVersion", LAYOUT.to_string()),
        ];
        let _ = write(key, &d);
    }
}

/// A saved number of Benchmark runs: 1 to 99, else 0 (the default).
fn runs(v: &str) -> i32 {
    match netfmt::parse_int(v) {
        Some(x) if (1..=MAX_BENCH_RUNS).contains(&x) => x,
        _ => 0,
    }
}

fn settings_ini() -> PathBuf {
    app_paths::data_dir().join("settings.ini")
}

fn read(key: &str) -> Result<Values, ()> {
    let mut d = Values::default();
    if app_paths::test_mode() {
        let f = settings_ini();
        if f.is_file() {
            for line in read_all_lines(&f).map_err(|_| ())? {
                let w = WString::from(line.as_str());
                if let Some(i) = w.index_of('=', 0)
                    && i > 0 {
                        d.set(&w.slice(0, i), w.substring(i + 1).to_string_lossy());
                    }
            }
        }
        return Ok(d);
    }
    let Some(values) = sys::reg_read_values(key).map_err(|_| ())? else { return Ok(d) };
    for (name, v) in values {
        d.set(&name, registry_text(&v));
    }
    Ok(d)
}

/// Convert.ToString(key.GetValue(name)) as .NET Framework reads each kind of value: text without its closing NUL
/// (REG_EXPAND_SZ expanded), a DWORD as int and a QWORD as long in decimal, a list or bytes as their type's name.
fn registry_text(v: &sys::RegValue) -> String {
    const REG_SZ: u32 = 1;
    const REG_EXPAND_SZ: u32 = 2;
    const REG_DWORD: u32 = 4;
    const REG_MULTI_SZ: u32 = 7;
    const REG_QWORD: u32 = 11;
    let text = |data: &[u8]| {
        let mut units: Vec<u16> = data.chunks(2).map(|c| u16::from_le_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
        if units.last() == Some(&0) {
            units.pop();
        }
        WString::from_units(units)
    };
    let int = |data: &[u8], n: usize| {
        let mut b = [0u8; 8];
        b[..data.len().min(n)].copy_from_slice(&data[..data.len().min(n)]);
        i64::from_le_bytes(b)
    };
    match v.kind {
        REG_SZ => text(&v.data).to_string_lossy(),
        REG_EXPAND_SZ => sys::expand_environment_strings(&text(&v.data)).to_string_lossy(),
        REG_DWORD if v.data.len() <= 4 => (int(&v.data, 4) as i32).to_string(),
        REG_DWORD | REG_QWORD if v.data.len() <= 8 => int(&v.data, 8).to_string(),
        REG_MULTI_SZ => "System.String[]".into(),
        _ => "System.Byte[]".into(),
    }
}

fn write(key: &str, d: &[(&str, String)]) -> Result<(), ()> {
    if app_paths::test_mode() {
        let dir = app_paths::data_dir();
        fs::create_dir_all(&dir).map_err(|_| ())?;
        let mut sb = String::new();
        for (k, v) in d {
            sb.push_str(k);
            sb.push('=');
            sb.push_str(v);
            sb.push_str("\r\n");
        }
        return fs::write(dir.join("settings.ini"), sb).map_err(|_| ());
    }
    sys::reg_write_values(key, d, &GONE).map_err(|_| ())
}

/// File.ReadAllLines: UTF-8 (a byte order mark skipped), lines split at \r\n, \n or \r.
pub fn read_all_lines(path: &Path) -> std::io::Result<Vec<String>> {
    let bytes = fs::read(path)?;
    let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(&bytes);
    let text = String::from_utf8_lossy(b);
    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\r' => {
                if it.peek() == Some(&'\n') {
                    it.next();
                }
                lines.push(std::mem::take(&mut cur));
            }
            '\n' => lines.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    Ok(lines)
}

/// Crash-safe job record: written when a job is queued, removed when it ends (done or canceled).
/// A record still there at the next start = the PC crashed or lost power mid-copy -> offer to resume.
pub mod journal {
    use super::*;

    pub fn save(s: &JobSpec) {
        if s.elevated {
            return; // an administrator copy runs from its own record ("<id>.admin"), see Handoff
        }
        let _ = (|| -> std::io::Result<()> {
            fs::create_dir_all(app_paths::job_dir())?;
            write(s, &app_paths::job_dir().join(format!("{}.job", s.id)), &[])
        })();
    }

    /// Saves the record in the background: it is flushed to the disk, which takes from a few ms to a lot more on a
    /// busy drive, and the copy's window doesn't wait for that. The copy itself does (CopyJob.Run), before it
    /// touches a file. (The record's text is taken now; C#'s task read the job a moment later.)
    pub fn save_soon(s: &mut JobSpec) {
        if s.elevated {
            return;
        }
        let rec = Arc::new(Recorded::default());
        s.recorded = Some(rec.clone());
        let text = record_text(s, &[]);
        let path = app_paths::job_dir().join(format!("{}.job", s.id));
        std::thread::spawn(move || {
            let _ = (|| -> std::io::Result<()> {
                fs::create_dir_all(app_paths::job_dir())?;
                write_text(&text, &path)
            })();
            rec.finish();
        });
    }

    fn record_text(s: &JobSpec, extra: &[String]) -> String {
        let mut sb = String::new();
        let mut line = |l: String| {
            sb.push_str(&l);
            sb.push_str("\r\n");
        };
        line(format!("op={}", s.op.name()));
        line(format!("dest={}", s.dest));
        line(format!("verify={}", if s.verify { "1" } else { "0" }));
        line(format!("policy={}", s.policy.name()));
        line(format!("limit={}", netfmt::g(s.speed_limit_mbps)));
        line(format!("origin={}", s.origin));
        if s.resume_mode {
            line("resume=1".into()); // an administrator copy of a resumed job resumes too (fix list 6.24)
        }
        for src in &s.sources {
            line(format!("src={src}"));
        }
        if let Some(items) = &s.explicit_items {
            for f in items {
                line(format!("file={}\t{}", f.src, f.rel));
            }
        }
        if let Some(dirs) = &s.explicit_dirs {
            for d in dirs {
                line(format!("dir={}\t{}", d.src, d.rel));
            }
        }
        for e in extra {
            line(e.clone());
        }
        sb
    }

    /// Writes a job record (flushed to the disk, then renamed into place).
    pub fn write(s: &JobSpec, path: &Path, extra: &[String]) -> std::io::Result<()> {
        write_text(&record_text(s, extra), path)
    }

    fn write_text(text: &str, path: &Path) -> std::io::Result<()> {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        {
            // FileMode.Create, FileShare.None; Flush(true) = FlushFileBuffers
            let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).share_mode(0).open(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
        }
        // into place with the rename itself on the disk (MOVEFILE_WRITE_THROUGH): before, a crash in a copy's first
        // seconds could lose the record and no resume was offered (fix list 6.27)
        let (t, p) = (WString::from_os(tmp.as_os_str()), WString::from_os(path.as_os_str()));
        sys::move_file_ex(&t, &p, sys::MOVEFILE_REPLACE_EXISTING | sys::MOVEFILE_WRITE_THROUGH)
            .map_err(|e| std::io::Error::from_raw_os_error(e as i32))
    }

    /// Reads a job record; None when it holds nothing to do.
    pub fn read(f: &Path) -> std::io::Result<Option<JobSpec>> {
        let mut s = JobSpec {
            id: f.file_stem().map(|x| x.to_string_lossy().to_string()).unwrap_or_default(),
            ..Default::default()
        };
        for line in read_all_lines(f)? {
            let Some(i) = line.find('=') else { continue };
            if i == 0 {
                continue;
            }
            let (k, v) = (&line[..i], &line[i + 1..]);
            match k {
                "op" => s.op = if v == "Move" { OpKind::Move } else { OpKind::Copy },
                "dest" => s.dest = paths::to_long(&WString::from(v)), // repairs C:" from older records
                "verify" => s.verify = v == "1",
                "policy" => {
                    if let Some(p) = ConflictPolicy::parse(v) {
                        s.policy = p;
                    }
                }
                "limit" => {
                    if let Some(x) = netfmt::parse_double(v) {
                        s.speed_limit_mbps = x;
                    }
                }
                "origin" => s.origin = v.to_string(),
                "resume" => s.resume_mode = v == "1",
                "src" => s.sources.push(paths::to_long(&WString::from(v))),
                "file" | "dir" => {
                    let Some(t) = v.find('\t') else { continue };
                    if t == 0 {
                        continue;
                    }
                    let (src, rel) = (WString::from(&v[..t]), WString::from(&v[t + 1..]));
                    if k == "file" {
                        s.explicit_items.get_or_insert_with(Vec::new).push(FileItem { src, rel, ..Default::default() });
                    } else {
                        let depth = rel.units().iter().filter(|&&c| c == b'\\' as u16).count() as i32;
                        s.explicit_dirs.get_or_insert_with(Vec::new).push(DirItem { src, rel, depth, ..Default::default() });
                    }
                }
                _ => {}
            }
        }
        Ok(if !s.dest.is_empty() && (!s.sources.is_empty() || s.explicit_items.is_some()) { Some(s) } else { None })
    }

    pub fn remove(s: &JobSpec) {
        // still being written: removed right after (else it would come back as an unfinished copy)
        let path = app_paths::job_dir().join(format!("{}.job", s.id));
        if let Some(r) = &s.recorded
            && !r.is_completed() {
                let r = r.clone();
                std::thread::spawn(move || {
                    while !r.wait(Duration::from_secs(3600)) {}
                    let _ = fs::remove_file(&path);
                });
                return;
            }
        let _ = fs::remove_file(&path);
    }

    pub fn load_unfinished() -> Vec<JobSpec> {
        let mut r = Vec::new();
        let dir = app_paths::job_dir();
        if !dir.is_dir() {
            return r;
        }
        // Directory.GetFiles(dir, "*.job"): Windows' own pattern match (a three-letter extension also matches longer ones)
        let pattern = WString::from_os(dir.join("*.job").as_os_str());
        if let Ok(list) = sys::find_first(&pattern) {
            for fd in list {
                if fd.attributes & sys::FILE_ATTRIBUTE_DIRECTORY != 0 {
                    continue;
                }
                let f = dir.join(fd.name.to_os());
                match read(&f) {
                    Ok(Some(mut s)) => {
                        s.resume_mode = true;
                        r.push(s);
                    }
                    Ok(None) => {
                        let _ = fs::remove_file(&f);
                    }
                    Err(_) => {}
                }
            }
        }
        // administrator copies whose Choppy is gone: the user's own record (above) resumes them
        handoff::clean_stale();
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn reg_add(key: &str, name: &str, kind: &str, data: &str) {
        let ok = Command::new("reg")
            .args(["add", &format!(r"HKCU\{key}"), "/v", name, "/t", kind, "/d", data, "/f"])
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "reg add {name}");
    }

    /// The registry side of Settings, on a scratch key (never HKCU\Software\Choppy itself).
    #[test]
    fn settings_in_the_registry() {
        let _g = super::super::step2_tests::ENV.lock().unwrap_or_else(|e| e.into_inner()); // no test mode meanwhile
        if app_paths::test_mode() {
            return; // CHOPPY_DATA set: Settings use settings.ini
        }
        let key = format!(r"Software\Choppy-rust-test-{}", super::super::model::new_id());
        // none yet: the defaults, saved once with the layout
        let s = Settings::load_from(&key);
        assert_eq!(s, Settings::default());
        let saved: HashMap<String, String> = sys::reg_read_values(&key)
            .unwrap()
            .unwrap()
            .into_iter()
            .map(|(n, v)| (n.to_string_lossy(), registry_text(&v)))
            .collect();
        assert_eq!(saved["SettingsVersion"], "2");
        assert_eq!(saved["LimitValueMbps"], "100.663296");
        assert_eq!(saved["SpeedLimitMbps"], "0");
        assert_eq!(saved["Policy"], "Ask");
        assert_eq!(saved["View"], "compact");
        assert_eq!(saved.len(), 13);
        // values of other kinds, read as .NET reads them; a value no longer kept is removed on save
        reg_add(&key, "CloseAfterSeconds", "REG_DWORD", "7");
        reg_add(&key, "SpeedUnit", "REG_EXPAND_SZ", "%SystemRoot%");
        reg_add(&key, "UltracopierRunBackup", "REG_MULTI_SZ", r"a\0b");
        reg_add(&key, "Policy", "REG_SZ", "Skip");
        reg_add(&key, "verifycopies", "REG_SZ", "1");
        reg_add(&key, "BenchDrives", "REG_SZ", "collapsed");
        let s = Settings::load_from(&key);
        assert_eq!(s.close_after_seconds, 7);
        assert_eq!(s.speed_unit.as_deref(), Some(std::env::var("SystemRoot").unwrap().as_str()));
        assert_eq!(s.ultracopier_run_backup.as_deref(), Some("System.String[]"));
        assert_eq!(s.policy, ConflictPolicy::Skip);
        assert!(s.verify_copies);
        s.save_to(&key);
        let names: Vec<String> = sys::reg_read_values(&key).unwrap().unwrap().into_iter().map(|(n, _)| n.to_string_lossy()).collect();
        assert!(!names.iter().any(|n| n == "BenchDrives"));
        assert_eq!(Settings::load_from(&key), s);
        sys::reg_delete_tree(&key).unwrap();
        assert!(sys::reg_read_values(&key).unwrap().is_none());
    }

    #[test]
    fn read_all_lines_like_net() {
        let dir = std::env::temp_dir().join(format!("choppy-rust-test-{}", super::super::model::new_id()));
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("x.txt");
        fs::write(&f, b"\xEF\xBB\xBFa=1\r\nb=2\nc=3\rd=4\r\n\r\ne").unwrap();
        assert_eq!(read_all_lines(&f).unwrap(), vec!["a=1", "b=2", "c=3", "d=4", "", "e"]);
        fs::remove_dir_all(&dir).unwrap();
    }
}
