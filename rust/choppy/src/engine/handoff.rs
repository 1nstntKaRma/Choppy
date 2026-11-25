//! The rest of a copy into a folder only administrators may write to (C:\, Program Files...) goes to a short-lived
//! Choppy started as administrator (Handoff.cs). The user's own Choppy writes a job record "<id>.admin"; next to it the
//! administrator copy keeps "<record>.progress" up to date, reads "<record>.control" (pause / run / cancel) and leaves
//! "<record>.result" when it ends. Plain text, like the other job records.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::copy_job::CopyJob;
use super::model::{FileItem, FileState, JobSpec};
use super::settings::{app_paths, journal, read_all_lines};
use super::{netfmt, paths};
use crate::sys;
use crate::wstr::WString;

pub const ARG: &str = "--elevated-job";

/// This Choppy runs as administrator (an administrator's own Choppy without "Run as administrator" doesn't).
pub fn is_admin() -> bool {
    sys::is_admin()
}

/// Administrator copy, very first thing: an elevated start gets a fresh environment, so test mode (scratch data
/// folder, test hook, windows off-screen) comes along in the record instead.
pub fn prepare(args: &[WString]) {
    if args.len() < 2 || !args[0].eq_str(ARG) {
        return;
    }
    let record = PathBuf::from(args[1].to_os());
    if !genuine(&record, args.get(2).map(|a| a.to_string_lossy())) {
        return;
    }
    let Ok(lines) = read_all_lines(&record) else { return };
    for line in lines {
        let Some(i) = line.find('=') else { continue };
        if i == 0 {
            continue;
        }
        let (k, v) = (&line[..i], &line[i + 1..]);
        match k {
            "testdata" => sys::set_env_var("CHOPPY_DATA", v),
            "testhook" => sys::set_env_var("CHOPPY_TESTHOOK", v),
            "testpark" => sys::set_env_var("CHOPPY_PARK", v),
            _ => {}
        }
    }
}

// ================= the user's side =================

/// Writes the administrator copy's record. Err = why it couldn't.
pub fn save(s: &JobSpec) -> Result<PathBuf, String> {
    let dir = app_paths::job_dir();
    std::fs::create_dir_all(&dir).map_err(|e| io_text(&e))?;
    let path = dir.join(format!("{}.admin", s.id));
    let mut extra = vec![format!("parent={}", sys::current_pid())];
    if app_paths::test_mode() {
        extra.push(format!("testdata={}", app_paths::data_dir().to_string_lossy()));
        if let Some(hook) = std::env::var_os("CHOPPY_TESTHOOK").filter(|h| !h.is_empty()) {
            extra.push(format!("testhook={}", PathBuf::from(hook).join("admin").to_string_lossy()));
        }
        if let Some(park) = std::env::var_os("CHOPPY_PARK").filter(|p| !p.is_empty()) {
            extra.push(format!("testpark={}", park.to_string_lossy()));
        }
    }
    journal::write(s, &path, &extra).map_err(|e| io_text(&e))?;
    clean(&path, false);
    Ok(path)
}

/// An IOException's message for a failed file call: the Win32 text, as C# showed it.
fn io_text(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(code) => sys::win32_message(code as u32),
        None => e.to_string(),
    }
}

/// The record's SHA-256. It goes on the administrator copy's command line: the record sits in the user's own folder,
/// so anything running as the user could change it - the administrator copy runs only the record the user's Choppy
/// wrote.
fn fingerprint(record: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(record)?;
    Ok(sha256(&bytes).iter().map(|b| format!("{b:02x}")).collect())
}

fn genuine(record: &Path, fingerprint_arg: Option<String>) -> bool {
    let Some(fp) = fingerprint_arg.filter(|f| !f.is_empty()) else { return false };
    record.is_file() && fingerprint(record).is_ok_and(|mine| mine == fp.trim().to_lowercase())
}

/// Starts Choppy as administrator on the record. Err = the Win32 error (1223 when the user says no).
pub fn launch(record: &Path) -> Result<sys::Process, u32> {
    let exe = std::env::current_exe().map_err(|e| e.raw_os_error().unwrap_or(5) as u32)?;
    let fp = fingerprint(record).map_err(|e| e.raw_os_error().unwrap_or(5) as u32)?;
    let args = WString::from(format!("{ARG} \"{}\" {fp}", record.to_string_lossy()));
    let dir = exe.parent().map(|d| WString::from_os(d.as_os_str())).unwrap_or_default();
    sys::shell_execute_runas(&WString::from_os(exe.as_os_str()), &args, &dir)
}

/// The user said no to the administrator prompt (ERROR_CANCELLED).
pub fn declined(code: u32) -> bool {
    code == sys::ERROR_CANCELLED
}

pub fn control(record: &Path, seq: i32, what: &str) {
    if let Err(e) = write_atomic(&with_ext(record, ".control"), &format!("{seq} {what}")) {
        panic!("{}", io_text(&e)); // C# let this IOException go up to the copy's catch
    }
}

pub fn read_progress(record: &Path) -> Option<HashMap<String, String>> {
    read_map(&with_ext(record, ".progress"))
}

/// What the administrator copy left in "<record>.result".
#[derive(Default)]
pub struct HandoffResult {
    pub phase: Option<String>,
    pub status: Option<String>,
    pub log: Option<String>,
    pub total_bytes: i64,
    pub done_bytes: i64,
    pub copied_bytes: i64,
    pub skipped_bytes: i64,
    pub failed_bytes: i64,
    pub total_files: i32,
    pub done_files: i32,
    pub skipped: i32,
    pub failed: i32,
    pub verified: i32,
    pub already: i32,
    pub failed_items: Vec<FileItem>,
    pub skipped_items: Vec<FileItem>,
    pub notes: Vec<String>,
}

pub fn read_result(record: &Path) -> Option<HandoffResult> {
    let f = with_ext(record, ".result");
    if !f.is_file() {
        return None;
    }
    let lines = read_all_lines(&f).ok()?;
    if lines.last().is_none_or(|l| l != "end") {
        return None; // still being written
    }
    let mut r = HandoffResult::default();
    for line in &lines {
        let Some(i) = line.find('=') else { continue };
        if i == 0 {
            continue;
        }
        let (k, v) = (&line[..i], &line[i + 1..]);
        match k {
            "phase" => r.phase = Some(v.to_string()),
            "status" => r.status = Some(v.to_string()),
            "log" => r.log = Some(v.to_string()),
            "totalBytes" => r.total_bytes = l(v),
            "doneBytes" => r.done_bytes = l(v),
            "copiedBytes" => r.copied_bytes = l(v),
            "skippedBytes" => r.skipped_bytes = l(v),
            "failedBytes" => r.failed_bytes = l(v),
            "totalFiles" => r.total_files = l(v) as i32,
            "doneFiles" => r.done_files = l(v) as i32,
            "skipped" => r.skipped = l(v) as i32,
            "failed" => r.failed = l(v) as i32,
            "verified" => r.verified = l(v) as i32,
            "already" => r.already = l(v) as i32,
            "note" => r.notes.push(v.to_string()),
            "failedItem" | "skippedItem" => {
                let p: Vec<&str> = v.split('\t').collect();
                if p.len() < 3 {
                    continue;
                }
                let fi = FileItem::new(WString::from(p[0]), WString::from(p[1]), l(p[2]));
                fi.set_error(p.get(3).map(|e| e.to_string()));
                if k == "failedItem" {
                    fi.set_st(FileState::Failed);
                    r.failed_items.push(fi);
                } else {
                    fi.set_st(FileState::Skipped);
                    r.skipped_items.push(fi);
                }
            }
            _ => {}
        }
    }
    Some(r)
}

pub fn clean(record: &Path, with_record: bool) {
    for ext in [".progress", ".result", ".control", ".progress.tmp", ".result.tmp", ".control.tmp"] {
        let _ = std::fs::remove_file(with_ext(record, ext));
    }
    if with_record {
        let _ = std::fs::remove_file(record);
    }
}

/// Administrator copies whose Choppy is gone: their files go (Journal.LoadUnfinished ends with this).
pub fn clean_stale() {
    let dir = app_paths::job_dir();
    let Ok(list) = sys::find_first(&WString::from_os(dir.join("*.admin").as_os_str())) else { return };
    for fd in list {
        if fd.attributes & sys::FILE_ATTRIBUTE_DIRECTORY != 0 {
            continue;
        }
        let f = dir.join(fd.name.to_os());
        let mut parent = 0;
        if let Ok(lines) = read_all_lines(&f) {
            for line in lines {
                if let Some(v) = line.strip_prefix("parent=") {
                    parent = netfmt::parse_int(v).unwrap_or(0); // int.TryParse: 0 when it isn't a number
                }
            }
        }
        if !alive(parent) {
            clean(&f, true);
        }
    }
}

/// The process still runs (one this Choppy can't look at counts as gone, as in C#).
pub fn alive(pid: i32) -> bool {
    sys::process_alive(pid)
}

// ================= the administrator's side =================

/// The record, if it is the one the user's Choppy wrote (see fingerprint); else None.
pub fn load(record: &Path, fingerprint_arg: Option<&str>) -> Option<JobSpec> {
    if !genuine(record, fingerprint_arg.map(|s| s.to_string())) {
        return None;
    }
    let mut s = journal::read(record).ok()??;
    s.elevated = true;
    s.handoff_path = Some(WString::from_os(record.as_os_str()));
    Some(s)
}

/// Keeps the progress file current and follows the control file until the copy ends.
pub fn report(job: &Arc<CopyJob>) {
    let Some(record) = job.spec.handoff_path.clone() else { return };
    job.on_finished(Arc::new(write_result));
    let job = job.clone();
    let _ = std::thread::Builder::new().name("handoff report".into()).spawn(move || {
        let record = PathBuf::from(record.to_os());
        let mut seen = 0;
        while !job.is_finished() {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _ = write_progress(&job);
                if let Some(c) = read_small(&with_ext(&record, ".control")) {
                    let p: Vec<&str> = c.splitn(2, ' ').collect();
                    if p.len() == 2
                        && let Some(seq) = netfmt::parse_int(p[0])
                        && seq > seen
                    {
                        seen = seq;
                        match p[1] {
                            "cancel" => job.cancel(),
                            "pause" => job.pause(),
                            "run" => job.resume(),
                            _ => {}
                        }
                    }
                }
            }));
            sys::sleep_ms(250);
        }
    });
}

fn line(sb: &mut String, l: &str) {
    sb.push_str(l);
    sb.push_str("\r\n");
}

fn write_progress(j: &CopyJob) -> std::io::Result<()> {
    let mut sb = String::new();
    line(&mut sb, &format!("pid={}", sys::current_pid()));
    line(&mut sb, &format!("phase={}", j.phase().name()));
    line(&mut sb, &format!("paused={}", if j.is_paused() { "1" } else { "0" }));
    line(&mut sb, &format!("doneBytes={}", j.done_bytes.load(Ordering::SeqCst)));
    line(&mut sb, &format!("totalBytes={}", j.total_bytes.load(Ordering::SeqCst)));
    line(&mut sb, &format!("doneFiles={}", j.done_files.load(Ordering::SeqCst)));
    line(&mut sb, &format!("totalFiles={}", j.total_files.load(Ordering::SeqCst)));
    if let Some(t) = j.current_transfer() {
        let f = t.file();
        let verifying = t.verifying.load(Ordering::SeqCst);
        line(&mut sb, &format!("cur={}", f.rel));
        line(&mut sb, &format!("curSrc={}", f.src));
        let done = if verifying { t.verify_done.load(Ordering::SeqCst) } else { t.done.load(Ordering::SeqCst) };
        line(&mut sb, &format!("curDone={done}"));
        line(&mut sb, &format!("curSize={}", if verifying { f.size() * 2 } else { f.size() }));
        line(&mut sb, &format!("verifying={}", if verifying { "1" } else { "0" }));
    }
    if let Some(s) = j.status() {
        line(&mut sb, &format!("status={}", one_line(&s)));
    }
    let record = PathBuf::from(j.spec.handoff_path.as_ref().map(|p| p.to_os()).unwrap_or_default());
    write_atomic(&with_ext(&record, ".progress"), &sb)
}

/// The job ended (its Finished event): the last progress, then the result. Errors are swallowed, as C#'s catch did.
fn write_result(j: &Arc<CopyJob>) {
    // the report thread may be writing the progress at this very moment: its failure never costs the result (C# lost
    // the result with it, and the user's Choppy then reported the copied files as failed)
    let _ = write_progress(j);
    let _ = write_result_file(j);
}

fn write_result_file(j: &CopyJob) -> std::io::Result<()> {
    {
        let n = |a: &std::sync::atomic::AtomicI32| a.load(Ordering::SeqCst);
        let b = |a: &std::sync::atomic::AtomicI64| a.load(Ordering::SeqCst);
        let mut sb = String::new();
        line(&mut sb, &format!("phase={}", j.phase().name()));
        if let Some(s) = j.status() {
            line(&mut sb, &format!("status={}", one_line(&s)));
        }
        if let Some(l) = j.log_path() {
            line(&mut sb, &format!("log={}", l.to_string_lossy()));
        }
        line(&mut sb, &format!("totalBytes={}", b(&j.total_bytes)));
        line(&mut sb, &format!("doneBytes={}", b(&j.done_bytes)));
        line(&mut sb, &format!("copiedBytes={}", b(&j.copied_bytes)));
        line(&mut sb, &format!("skippedBytes={}", b(&j.skipped_bytes)));
        line(&mut sb, &format!("failedBytes={}", b(&j.failed_bytes)));
        line(&mut sb, &format!("totalFiles={}", n(&j.total_files)));
        line(&mut sb, &format!("doneFiles={}", n(&j.done_files)));
        line(&mut sb, &format!("skipped={}", n(&j.skipped_files)));
        line(&mut sb, &format!("failed={}", n(&j.failed_files)));
        line(&mut sb, &format!("verified={}", n(&j.verified_files)));
        line(&mut sb, &format!("already={}", n(&j.already_there_files)));
        for note in super::lock(&j.notes).iter() {
            line(&mut sb, &format!("note={}", one_line(note)));
        }
        for f in super::lock(&j.failed_items).iter() {
            let rel = if f.rel.is_empty() { paths::file_name(&f.src) } else { f.rel.clone() };
            line(&mut sb, &format!("failedItem={}\t{}\t{}\t{}", f.src, rel, f.size(), one_line(&f.error().unwrap_or_default())));
        }
        for f in super::lock(&j.skipped_items).iter() {
            let rel = if f.rel.is_empty() { paths::file_name(&f.src) } else { f.rel.clone() };
            line(&mut sb, &format!("skippedItem={}\t{}\t{}", f.src, rel, f.size()));
        }
        line(&mut sb, "end");
        let record = PathBuf::from(j.spec.handoff_path.as_ref().map(|p| p.to_os()).unwrap_or_default());
        write_atomic(&with_ext(&record, ".result"), &sb)
    }
}

// ================= files =================

/// "<record>" + ext (the extension is added to the whole name).
fn with_ext(record: &Path, ext: &str) -> PathBuf {
    let mut s = record.as_os_str().to_owned();
    s.push(ext);
    PathBuf::from(s)
}

/// long.TryParse(v, NumberStyles.Integer, InvariantCulture): 0 when it isn't one.
fn l(v: &str) -> i64 {
    netfmt::parse_long(v).unwrap_or(0)
}

fn one_line(s: &str) -> String {
    s.replace(['\r', '\n', '\t'], " ")
}

/// Written next to it, then put in its place (File.Replace, or File.Move the first time). UTF-8 with a byte order
/// mark (Encoding.UTF8).
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = with_ext(path, ".tmp");
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(text.as_bytes());
    std::fs::write(&tmp, bytes)?;
    let (t, p) = (WString::from_os(tmp.as_os_str()), WString::from_os(path.as_os_str()));
    let r = if path.is_file() { sys::replace_file(&p, &t) } else { sys::move_file(&t, &p) };
    r.map_err(|e| std::io::Error::from_raw_os_error(e as i32))
}

/// File.ReadAllText(UTF-8).Trim(); None = not there or unreadable.
fn read_small(path: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(&bytes);
    Some(String::from_utf8_lossy(b).trim_matches(|c: char| (c as u32) < 0x10000 && crate::wstr::is_white_space(c as u32 as u16)).to_string())
}

/// "key=value" lines (keys in any case: stored lower case).
fn read_map(path: &Path) -> Option<HashMap<String, String>> {
    let text = read_small(path)?;
    let mut d = HashMap::new();
    for line in text.split('\n') {
        if let Some(i) = line.find('=')
            && i > 0
        {
            d.insert(line[..i].to_ascii_lowercase(), line[i + 1..].trim_end_matches('\r').to_string());
        }
    }
    Some(d)
}

// ================= SHA-256 (the record's fingerprint) =================

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01,
    0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc,
    0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147,
    0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08,
    0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208,
    0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// SHA-256 (FIPS 180-4) of the bytes, as .NET's SHA256 computes it.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(block[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = h;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for i in 0..8 {
            h[i] = h[i].wrapping_add(v[i]);
        }
    }
    let mut out = [0u8; 32];
    for i in 0..8 {
        out[i * 4..i * 4 + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn sha256_known_values() {
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let long = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        assert_eq!(hex(&sha256(long)), "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1");
        let million = vec![b'a'; 1_000_000];
        assert_eq!(hex(&sha256(&million)), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }

    #[test]
    fn one_line_and_map() {
        assert_eq!(one_line("a\r\nb\tc"), "a  b c");
    }
}
