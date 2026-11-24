//! Fast analysis (Scanner.cs): lists the sources (several folders at once on a network), then lists only the
//! destination folders that already exist to find name conflicts - one listing per folder instead of one check per file.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::copy_job::TEMP_SUFFIX;
use super::model::{DirItem, FileItem, JobSpec, OpKind, ScanResult};
use super::paths;
use crate::sys;
use crate::wstr::{WString, w};

pub struct Scanner {
    pub files_seen: AtomicI32,
    pub bytes_seen: AtomicI64,
    stage: AtomicU8, // 0 = "Analyzing", 1 = "Checking the destination"
    canceled: Box<dyn Fn() -> bool + Send + Sync>,
}

#[derive(Default)]
struct Node {
    dir: DirItem,
    files: Vec<FileItem>,
    subs: Vec<NodeRef>,
    notes: Vec<String>,
    error: Option<String>,
}

type NodeRef = Arc<Mutex<Node>>;

struct Entry {
    is_dir: bool,
    size: i64,
    mtime: i64,
}

/// A name -> entry map with OrdinalIgnoreCase keys.
type DestMap = HashMap<WString, Entry>;

enum TopItem {
    File(FileItem),
    Node(NodeRef),
}

impl Scanner {
    pub fn new(canceled: impl Fn() -> bool + Send + Sync + 'static) -> Scanner {
        Scanner {
            files_seen: AtomicI32::new(0),
            bytes_seen: AtomicI64::new(0),
            stage: AtomicU8::new(0),
            canceled: Box::new(canceled),
        }
    }

    pub fn stage(&self) -> &'static str {
        if self.stage.load(Ordering::SeqCst) == 0 { "Analyzing" } else { "Checking the destination" }
    }

    fn canceled(&self) -> bool {
        (self.canceled)()
    }

    pub fn run(&self, spec: &JobSpec, source_threads: i32, dest_threads: i32) -> ScanResult {
        let mut res = ScanResult::default();
        let mut tops: Vec<TopItem> = Vec::new();
        let mut dir_roots: Vec<NodeRef> = Vec::new();

        for src in &spec.sources {
            let d = match sys::get_file_attributes_ex(src) {
                Ok(d) => d,
                Err(e) => {
                    res.problems.push(format!("Not found: {} - {}", paths::to_short(src), sys::error_text(e)));
                    continue;
                }
            };
            let is_dir = (d.attributes & sys::FILE_ATTRIBUTE_DIRECTORY) != 0;
            let mut name = paths::file_name(src);
            if let Some(parent) = paths::parent(src)
                && paths::same_path(&parent, &spec.dest) {
                    if spec.op == OpKind::Move {
                        res.problems.push(format!("Already in this folder: {}", paths::to_short(src)));
                        continue;
                    }
                    name = copy_name(&spec.dest, &name, is_dir);
                }
            if is_dir {
                if paths::is_same_or_inside(&spec.dest, src) {
                    res.problems.push(format!("A folder can't be copied into itself: {}", paths::to_short(src)));
                    continue;
                }
                let node = Arc::new(Mutex::new(Node {
                    dir: DirItem {
                        src: src.clone(),
                        rel: name,
                        attr: d.attributes,
                        ctime: d.creation,
                        atime: d.access,
                        mtime: d.write,
                        depth: 0,
                        ..Default::default()
                    },
                    ..Default::default()
                }));
                tops.push(TopItem::Node(node.clone()));
                dir_roots.push(node);
            } else {
                let size = d.size();
                tops.push(TopItem::File(FileItem {
                    src: src.clone(),
                    rel: name,
                    size,
                    attr: d.attributes,
                    ctime: d.creation,
                    atime: d.access,
                    mtime: d.write,
                    ..Default::default()
                }));
                self.files_seen.fetch_add(1, Ordering::SeqCst);
                self.bytes_seen.fetch_add(size, Ordering::SeqCst);
            }
        }

        // 1) list the source folders
        self.run_pool(dir_roots, source_threads.max(1), &|n: &NodeRef| self.list_source(n));
        if self.canceled() {
            return res;
        }

        // 2) conflicts: list destination folders that exist
        self.stage.store(1, Ordering::SeqCst);
        self.scan_destination(&spec.dest, &mut tops, dest_threads.max(1));
        if self.canceled() {
            return res;
        }

        // 3) flatten in source order: files of a folder, then its subfolders (depth first)
        for t in tops {
            match t {
                TopItem::File(f) => res.files.push(f),
                TopItem::Node(n) => flatten(&n, &mut res),
            }
        }

        let mut total = 0i64;
        let mut conflicts = 0;
        let mut sizes: Vec<i64> = Vec::with_capacity(res.files.len());
        for f in &res.files {
            total += f.size;
            sizes.push(f.size);
            if f.dest_exists {
                conflicts += 1;
            }
        }
        res.total_bytes = total;
        res.conflicts = conflicts;
        if !sizes.is_empty() {
            sizes.sort_unstable();
            res.median_size = sizes[sizes.len() / 2];
        }
        res.largest = top_largest(&res.files, 4);
        res
    }

    // ---- worker pool over a growing set of folders ----
    fn run_pool(&self, roots: Vec<NodeRef>, threads: i32, work: &(dyn Fn(&NodeRef) -> Vec<NodeRef> + Sync)) {
        if roots.is_empty() {
            return;
        }
        if threads <= 1 {
            // depth-first on this thread
            let mut stack: Vec<NodeRef> = roots.into_iter().rev().collect();
            while !stack.is_empty() && !self.canceled() {
                let n = stack.pop().unwrap();
                let subs = work(&n);
                stack.extend(subs.into_iter().rev());
            }
            return;
        }
        let pending = AtomicUsize::new(roots.len());
        let q = Mutex::new(VecDeque::from(roots));
        let done = AtomicBool::new(false);
        std::thread::scope(|sc| {
            for _ in 0..threads {
                let (q, pending, done) = (&q, &pending, &done);
                std::thread::Builder::new()
                    .name("scan".into())
                    .spawn_scoped(sc, move || {
                        let mut spin = 0u32;
                        while !done.load(Ordering::SeqCst) {
                            let next = q.lock().unwrap().pop_front();
                            if let Some(n) = next {
                                spin = 0;
                                let subs = if self.canceled() { Vec::new() } else { work(&n) };
                                for s in subs {
                                    pending.fetch_add(1, Ordering::SeqCst);
                                    q.lock().unwrap().push_back(s);
                                }
                                if pending.fetch_sub(1, Ordering::SeqCst) == 1 {
                                    done.store(true, Ordering::SeqCst);
                                }
                            } else if pending.load(Ordering::SeqCst) == 0 {
                                done.store(true, Ordering::SeqCst);
                            } else if spin >= 10 {
                                // SpinWait: after 10 spins it yields; here a 1 ms sleep as the C# does then
                                std::thread::sleep(Duration::from_millis(1));
                            } else {
                                for _ in 0..(4u32 << spin) {
                                    std::hint::spin_loop();
                                }
                                spin += 1;
                            }
                        }
                    })
                    .expect("scan thread");
            }
        });
    }

    fn list_source(&self, n: &NodeRef) -> Vec<NodeRef> {
        let mut subs = Vec::new();
        let mut node = n.lock().unwrap();
        let list = match sys::find_first(&paths::combine(&node.dir.src, &w("*"))) {
            Ok(l) => l,
            Err(e) => {
                if e != sys::ERROR_FILE_NOT_FOUND {
                    node.error = Some(sys::error_text(e));
                }
                return subs;
            }
        };
        for fd in list {
            if fd.name.eq_str(".") || fd.name.eq_str("..") {
                continue;
            }
            let full = paths::combine(&node.dir.src, &fd.name);
            let rel = node.dir.rel.clone() + "\\" + &fd.name;
            if (fd.attributes & sys::FILE_ATTRIBUTE_DIRECTORY) != 0 {
                if (fd.attributes & sys::FILE_ATTRIBUTE_REPARSE_POINT) != 0 {
                    // junctions / folder links are not followed (they can loop or point anywhere)
                    node.notes.push(format!("Folder link not followed: {}", paths::to_short(&full)));
                    continue;
                }
                let dir = DirItem {
                    src: full,
                    rel,
                    attr: fd.attributes,
                    ctime: fd.creation,
                    atime: fd.access,
                    mtime: fd.write,
                    depth: node.dir.depth + 1,
                    ..Default::default()
                };
                let sub = Arc::new(Mutex::new(Node { dir, ..Default::default() }));
                node.subs.push(sub.clone());
                subs.push(sub);
            } else {
                let size = fd.size();
                node.files.push(FileItem {
                    src: full,
                    rel,
                    size,
                    attr: fd.attributes,
                    ctime: fd.creation,
                    atime: fd.access,
                    mtime: fd.write,
                    ..Default::default()
                });
                self.files_seen.fetch_add(1, Ordering::SeqCst);
                self.bytes_seen.fetch_add(size, Ordering::SeqCst);
            }
        }
        subs
    }

    // ---- destination ----
    fn list_dest(dir_long: &WString) -> DestMap {
        let mut map = DestMap::new();
        let Ok(list) = sys::find_first(&paths::combine(dir_long, &w("*"))) else { return map };
        for fd in list {
            if fd.name.eq_str(".") || fd.name.eq_str("..") {
                continue;
            }
            map.insert(
                fd.name.to_upper_invariant(),
                Entry { is_dir: (fd.attributes & sys::FILE_ATTRIBUTE_DIRECTORY) != 0, size: fd.size(), mtime: fd.write },
            );
        }
        map
    }

    fn mark(f: &mut FileItem, map: &DestMap) {
        let name = f.name();
        if let Some(e) = map.get(&name.to_upper_invariant()) {
            f.dest_exists = true;
            f.dest_is_dir = e.is_dir;
            f.dest_size = e.size;
            f.dest_mtime = e.mtime;
        }
        if map.contains_key(&(name + TEMP_SUFFIX).to_upper_invariant()) {
            f.temp_exists = true;
        }
    }

    fn scan_destination(&self, dest: &WString, tops: &mut [TopItem], threads: i32) {
        if !paths::dir_exists(dest) {
            return;
        }
        let root_map = Self::list_dest(dest);
        let mut roots = Vec::new();
        for t in tops.iter_mut() {
            match t {
                TopItem::File(f) => Self::mark(f, &root_map),
                TopItem::Node(n) => {
                    let mut node = n.lock().unwrap();
                    if let Some(e) = root_map.get(&node.dir.rel.to_upper_invariant()) {
                        if e.is_dir {
                            node.dir.dest_existed = true;
                            roots.push(n.clone());
                        } else {
                            node.dir.dest_is_file = true;
                        }
                    }
                }
            }
        }
        self.run_pool(roots, threads, &|n: &NodeRef| {
            let mut node = n.lock().unwrap();
            let map = Self::list_dest(&paths::combine(dest, &node.dir.rel));
            for f in node.files.iter_mut() {
                Self::mark(f, &map);
            }
            let mut next = Vec::new();
            for s in &node.subs {
                let mut sn = s.lock().unwrap();
                let nm = paths::file_name(&sn.dir.rel);
                if let Some(e) = map.get(&nm.to_upper_invariant()) {
                    if e.is_dir {
                        sn.dir.dest_existed = true;
                        next.push(s.clone());
                    } else {
                        sn.dir.dest_is_file = true;
                    }
                }
            }
            next
        });
    }
}

fn top_largest(files: &[FileItem], n: usize) -> Vec<(WString, i64)> {
    let mut top: Vec<&FileItem> = Vec::with_capacity(n + 1);
    for f in files {
        if top.len() < n || f.size > top[top.len() - 1].size {
            let mut i = top.len();
            while i > 0 && top[i - 1].size < f.size {
                i -= 1;
            }
            top.insert(i, f);
            if top.len() > n {
                top.remove(n);
            }
        }
    }
    top.iter().map(|f| (f.name(), f.size)).collect()
}

fn flatten(n: &NodeRef, res: &mut ScanResult) {
    let node = std::mem::take(&mut *n.lock().unwrap());
    if let Some(e) = &node.error {
        res.problems.push(format!("Couldn't read folder {} - {}", paths::to_short(&node.dir.src), e));
    }
    res.dirs.push(node.dir);
    res.problems.extend(node.notes);
    res.files.extend(node.files);
    for s in &node.subs {
        flatten(s, res);
    }
}

/// Explorer-style name for copying into the same folder: "name - Copy", "name - Copy (2)".
pub fn copy_name(dest_dir: &WString, name: &WString, is_dir: bool) -> WString {
    let (mut stem, mut ext) = (name.clone(), WString::new());
    if !is_dir
        && let Some(dot) = name.last_index_of('.')
            && dot > 0 {
                stem = name.slice(0, dot);
                ext = name.substring(dot);
            }
    let mut cand = stem.clone() + " - Copy" + &ext;
    let mut n = 2;
    while paths::exists(&paths::combine(dest_dir, &cand)) {
        cand = stem.clone() + &format!(" - Copy ({n})") + &ext;
        n += 1;
    }
    cand
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stopped before it starts: only what the sources themselves said, no listing.
    #[test]
    fn canceled_scan_lists_nothing() {
        let tmp = WString::from_os(std::env::temp_dir().as_os_str());
        let mut spec = JobSpec::new(OpKind::Copy);
        spec.sources.push(paths::to_long(&tmp));
        spec.sources.push(paths::to_long(&(tmp.clone() + r"\choppy-no-such-file-here")));
        spec.dest = paths::to_long(&w(r"C:\choppy-no-such-folder"));
        let res = Scanner::new(|| true).run(&spec, 4, 4);
        assert!(res.files.is_empty() && res.dirs.is_empty());
        assert_eq!(res.problems.len(), 1);
        assert!(res.problems[0].starts_with("Not found: "));
    }
}
