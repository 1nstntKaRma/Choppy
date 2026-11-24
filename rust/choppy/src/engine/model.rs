//! What a job is made of (Model.cs): the job, its files and folders, the scan's result, conflicts.

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use super::paths;
use crate::sys;
use crate::wstr::WString;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum OpKind {
    #[default]
    Copy,
    Move,
}

impl OpKind {
    /// OpKind.ToString()
    pub fn name(self) -> &'static str {
        match self {
            OpKind::Copy => "Copy",
            OpKind::Move => "Move",
        }
    }
}

/// What to do when a file with the same name is already at the destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ConflictPolicy {
    #[default]
    Ask,
    Replace,
    Skip,
    KeepBoth,
    ReplaceOlder,
}

impl ConflictPolicy {
    const ALL: [ConflictPolicy; 5] =
        [ConflictPolicy::Ask, ConflictPolicy::Replace, ConflictPolicy::Skip, ConflictPolicy::KeepBoth, ConflictPolicy::ReplaceOlder];

    /// ConflictPolicy.ToString()
    pub fn name(self) -> &'static str {
        match self {
            ConflictPolicy::Ask => "Ask",
            ConflictPolicy::Replace => "Replace",
            ConflictPolicy::Skip => "Skip",
            ConflictPolicy::KeepBoth => "KeepBoth",
            ConflictPolicy::ReplaceOlder => "ReplaceOlder",
        }
    }

    /// Enum.TryParse(v, out p): a name (case as written) or its number, white space around; names joined by commas
    /// add up as flags. A number no policy has is refused (C# would keep it as an unnamed value).
    pub fn parse(v: &str) -> Option<ConflictPolicy> {
        let t = v.trim();
        if t.is_empty() {
            return None;
        }
        let first = t.as_bytes()[0];
        let value: i32 = if first.is_ascii_digit() || first == b'-' || first == b'+' {
            super::netfmt::parse_int(t)?
        } else {
            let mut sum = 0;
            for part in t.split(',') {
                let p = part.trim();
                sum |= Self::ALL.iter().position(|c| c.name() == p)? as i32;
            }
            sum
        };
        Self::ALL.get(usize::try_from(value).ok()?).copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum FileState {
    Pending = 0,
    Active = 1,
    Done = 2,
    Skipped = 3,
    Failed = 4,
}

impl FileState {
    pub fn from_i32(v: i32) -> FileState {
        match v {
            1 => FileState::Active,
            2 => FileState::Done,
            3 => FileState::Skipped,
            4 => FileState::Failed,
            _ => FileState::Pending,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobPhase {
    Queued,
    Analyzing,
    Copying,
    Waiting,
    Finishing,
    Done,
    Canceled,
    Failed,
}

impl JobPhase {
    /// JobPhase.ToString()
    pub fn name(self) -> &'static str {
        match self {
            JobPhase::Queued => "Queued",
            JobPhase::Analyzing => "Analyzing",
            JobPhase::Copying => "Copying",
            JobPhase::Waiting => "Waiting",
            JobPhase::Finishing => "Finishing",
            JobPhase::Done => "Done",
            JobPhase::Canceled => "Canceled",
            JobPhase::Failed => "Failed",
        }
    }
}

/// A job record being written in the background (Journal.SaveSoon); the copy waits for it before it touches a file.
#[derive(Default)]
pub struct Recorded {
    done: Mutex<bool>,
    cv: Condvar,
}

impl Recorded {
    pub fn finish(&self) {
        *self.done.lock().unwrap() = true;
        self.cv.notify_all();
    }
    pub fn is_completed(&self) -> bool {
        *self.done.lock().unwrap()
    }
    /// Waits up to `timeout`; true when it is written.
    pub fn wait(&self, timeout: Duration) -> bool {
        let g = self.done.lock().unwrap();
        let (g, _) = self.cv.wait_timeout_while(g, timeout, |d| !*d).unwrap();
        *g
    }
}

pub struct JobSpec {
    pub id: String,
    pub op: OpKind,
    pub sources: Vec<WString>,           // long-form paths
    pub dest: WString,                   // long-form destination folder
    pub verify: bool,                    // check each copy (a move then removes the original only after its copy matches)
    pub policy: ConflictPolicy,
    pub resume_mode: bool,               // a resumed job skips files already there with the same size and date
    pub speed_limit_mbps: f64,           // 0 = no limit
    pub origin: String,                  // "Explorer", "Right-click", "CLI", ...
    pub explicit_items: Option<Vec<FileItem>>, // "Retry failed" jobs carry their files directly
    pub explicit_dirs: Option<Vec<DirItem>>,   // folders that come with them (an administrator copy finishes them)
    pub elevated: bool,                  // runs as administrator for the user's own Choppy
    pub handoff_path: Option<WString>,   // its record: progress, result and control files sit next to it
    pub recorded: Option<Arc<Recorded>>, // the job record being written (see Journal.SaveSoon)
    pub force_workers: i32,              // tests: fixed number of files in flight (0 = Adaptive)
    pub no_fast_path: bool,              // tests: always rename with a separate call
    pub engine: Option<String>,          // tests: None = Adaptive, "copyfile" = CopyFileEx only, "nobuf" = the old unbuffered CopyFileEx
    // tests: pipeline piece size / pieces in flight (0 = Adaptive)
    pub pipe_chunk_mb: i32,
    pub pipe_slots: i32,
    pub pipe_batch: i32,
    pub pipe_depth: i32,
    pub pipe_write_step_mb: i32,
    pub cached_write_mb: i32,
    pub flush_files: i32,
    pub flush_threads: i32,
    pub no_prealloc: bool,               // tests: no size reserved up front
    pub small_mode: Option<String>,      // tests: small files - "unbuf", "cached", "tiny" (cache under 64 KB), "off" (no batches), "batch" (batches on any hard disk)
    pub force_prealloc: bool,            // tests: reserve the size up front even on a hard disk
    pub no_tune: bool,                   // tests: keep the first guess, measure nothing
    pub pipe_phased: i32,                // tests: 1 = read a batch, then write it; 0 = both at once (-1 = Adaptive)
    pub no_cached_tail: bool,            // tests: the last file's end goes straight to the disk too
    pub tail_mb: i32,                    // tests: how much of each big file's end goes through the cache (-1 = Adaptive)
    pub max_file_test: i64,              // tests: the destination takes no file bigger than this (as FAT32 does at 4 GB)
}

/// Guid.NewGuid().ToString("N").Substring(0, 12): the first 12 of its 32 lowercase hex digits.
pub fn new_id() -> String {
    let g = sys::new_guid();
    format!("{:08x}{:04x}", g.data1, g.data2)
}

impl Default for JobSpec {
    fn default() -> Self {
        JobSpec {
            id: new_id(),
            op: OpKind::Copy,
            sources: Vec::new(),
            dest: WString::new(),
            verify: false,
            policy: ConflictPolicy::Ask,
            resume_mode: false,
            speed_limit_mbps: 0.0,
            origin: String::new(),
            explicit_items: None,
            explicit_dirs: None,
            elevated: false,
            handoff_path: None,
            recorded: None,
            force_workers: 0,
            no_fast_path: false,
            engine: None,
            pipe_chunk_mb: 0,
            pipe_slots: 0,
            pipe_batch: 0,
            pipe_depth: 0,
            pipe_write_step_mb: 0,
            cached_write_mb: 0,
            flush_files: 0,
            flush_threads: 0,
            no_prealloc: false,
            small_mode: None,
            force_prealloc: false,
            no_tune: false,
            pipe_phased: -1,
            no_cached_tail: false,
            tail_mb: -1,
            max_file_test: 0,
        }
    }
}

impl JobSpec {
    pub fn new(op: OpKind) -> JobSpec {
        JobSpec { op, ..Default::default() }
    }

    pub fn title(&self) -> String {
        let verb = if self.op == OpKind::Move { "Move" } else { "Copy" };
        if let Some(items) = &self.explicit_items {
            let n = items.len();
            return format!("{verb} {n}{}{}", if n == 1 { " file" } else { " files" }, if self.elevated { "" } else { " again" });
        }
        if self.sources.len() == 1 {
            return format!("{verb} {}", paths::file_name(&paths::to_short(&self.sources[0])));
        }
        format!("{verb} {} items", self.sources.len())
    }
}

/// One file of a job. The scan fills it in; the copy changes what C# changed from any thread (its state, its size when
/// the file grew or shrank since the scan, the error, where it ended up...), so those are atomics or behind a lock.
pub struct FileItem {
    pub src: WString,         // long-form source path
    pub rel: WString,         // path under the destination folder, top-level name included
    pub size: AtomicI64,      // C#'s Size: the copy puts it right when the file changed since the scan
    pub attr: u32,
    pub ctime: i64,
    pub atime: i64,
    pub mtime: i64,
    pub dest_exists: bool,    // a file with this name already sits at the destination
    pub dest_is_dir: bool,
    pub dest_size: i64,
    pub dest_mtime: i64,
    pub temp_exists: bool,    // a leftover "<name>.chpart" from an interrupted run
    pub state: AtomicI32,     // FileState
    pub error: Mutex<Option<String>>,
    pub final_path: Mutex<Option<WString>>, // where it ended up (Keep both can change the name)
    pub verified: AtomicBool,
    pub already_there: AtomicBool, // resume: same size + date already at the destination
    pub source_kept: AtomicBool,   // move: copied, but the source couldn't be deleted
}

impl Default for FileItem {
    fn default() -> Self {
        FileItem {
            src: WString::new(),
            rel: WString::new(),
            size: AtomicI64::new(0),
            attr: 0,
            ctime: 0,
            atime: 0,
            mtime: 0,
            dest_exists: false,
            dest_is_dir: false,
            dest_size: 0,
            dest_mtime: 0,
            temp_exists: false,
            state: AtomicI32::new(FileState::Pending as i32),
            error: Mutex::new(None),
            final_path: Mutex::new(None),
            verified: AtomicBool::new(false),
            already_there: AtomicBool::new(false),
            source_kept: AtomicBool::new(false),
        }
    }
}

impl FileItem {
    /// `new FileItem { Src = src, Rel = rel, Size = size }`
    pub fn new(src: WString, rel: WString, size: i64) -> FileItem {
        FileItem { src, rel, size: AtomicI64::new(size), ..Default::default() }
    }
    pub fn size(&self) -> i64 {
        self.size.load(Ordering::SeqCst)
    }
    pub fn set_size(&self, v: i64) {
        self.size.store(v, Ordering::SeqCst);
    }
    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub fn set_error(&self, why: Option<String>) {
        *self.error.lock().unwrap_or_else(|e| e.into_inner()) = why;
    }
    pub fn final_path(&self) -> Option<WString> {
        self.final_path.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub fn set_final_path(&self, p: WString) {
        *self.final_path.lock().unwrap_or_else(|e| e.into_inner()) = Some(p);
    }
    pub fn st(&self) -> FileState {
        FileState::from_i32(self.state.load(Ordering::SeqCst))
    }
    pub fn set_st(&self, s: FileState) {
        self.state.store(s as i32, Ordering::SeqCst);
    }
    pub fn name(&self) -> WString {
        paths::file_name(&self.rel)
    }
}

#[derive(Clone, Debug, Default)]
pub struct DirItem {
    pub src: WString,
    pub rel: WString,
    pub attr: u32,
    pub ctime: i64,
    pub atime: i64,
    pub mtime: i64,
    pub dest_existed: bool,
    pub dest_is_file: bool, // a FILE with this folder's name blocks it
    pub depth: i32,
}

#[derive(Default)]
pub struct ScanResult {
    pub files: Vec<Arc<FileItem>>, // shared with the copy (its lists of failed and skipped files hold them too)
    pub dirs: Vec<DirItem>,
    pub total_bytes: i64,
    pub conflicts: i32,
    pub largest: Vec<(WString, i64)>,
    pub problems: Vec<String>, // unreadable folders, links not followed
    pub median_size: i64,
}

pub struct ConflictInfo {
    pub file: Arc<FileItem>,
    pub incoming_path: WString,  // short form
    pub existing_path: WString,  // short form
    pub incoming_size: i64,
    pub existing_size: i64,
    pub incoming_time: Option<sys::LocalTime>, // None = DateTime.MinValue
    pub existing_time: Option<sys::LocalTime>,
    pub remaining: i32,          // conflicts not answered yet, this one included
    pub op: OpKind,
}

#[derive(Clone, Copy, Debug)]
pub struct ConflictAnswer {
    pub choice: ConflictPolicy,
    pub apply_to_all: bool,
    pub cancel_job: bool, // a dialog can stop the whole copy
}

pub mod file_times {
    use crate::sys;

    /// DateTime.FromFileTimeUtc(ft).ToLocalTime(); None where C# gives DateTime.MinValue (not a valid time).
    pub fn to_local(ft: i64) -> Option<sys::LocalTime> {
        sys::file_time_to_local(ft)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_id_and_defaults() {
        let id = new_id();
        assert_eq!(id.len(), 12);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(new_id(), id);
        let s = JobSpec::default();
        assert_eq!((s.pipe_phased, s.tail_mb, s.policy, s.op), (-1, -1, ConflictPolicy::Ask, OpKind::Copy));
        assert_eq!(FileItem::default().st(), FileState::Pending);
    }

    #[test]
    fn policy_like_enum_try_parse() {
        assert_eq!(ConflictPolicy::parse("KeepBoth"), Some(ConflictPolicy::KeepBoth));
        assert_eq!(ConflictPolicy::parse(" Skip "), Some(ConflictPolicy::Skip));
        assert_eq!(ConflictPolicy::parse("skip"), None);
        assert_eq!(ConflictPolicy::parse("4"), Some(ConflictPolicy::ReplaceOlder));
        assert_eq!(ConflictPolicy::parse("Replace, Skip"), Some(ConflictPolicy::KeepBoth));
        assert_eq!(ConflictPolicy::parse("7"), None);
    }
}
