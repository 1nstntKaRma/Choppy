//! BenchFiles.cs: the Benchmark's test files - its temporary folders (each with a marker file, listed in the data
//! folder until they are gone), the test data, flushing and purging, and a delete that never leaves those folders.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, Mutex};

use crate::engine::paths;
use crate::engine::settings::app_paths;
use crate::sys;
use crate::wstr::WString;

use super::{BenchErr, BenchResult};

/// Windows names don't care about case: folders and markers of earlier versions ("Choppy benchmark ...") are these too.
pub const FOLDER_NAME: &str = "Choppy Benchmark (temporary)";
pub const MARKER_NAME: &str = "Choppy Benchmark - safe to delete.txt";
const MARKER_HEAD: &str = "Choppy Benchmark test data";

static LIST_LOCK: Mutex<()> = Mutex::new(());

/// Every temporary folder the benchmark made and hasn't removed yet (cleaned at the next start).
fn list_file() -> PathBuf {
    app_paths::data_dir().join("benchmark-folders.txt")
}

fn utf8_bom(text: &str) -> Vec<u8> {
    let mut b = vec![0xEF, 0xBB, 0xBF];
    b.extend_from_slice(text.as_bytes());
    b
}

/// File.ReadAllLines with Encoding.UTF8 (a byte order mark skipped; CR LF, LF or CR).
fn read_lines(p: &Path) -> std::io::Result<Vec<String>> {
    let b = std::fs::read(p)?;
    let t = String::from_utf8_lossy(b.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&b)).to_string();
    let mut out: Vec<String> = t.split("\r\n").flat_map(|l| l.split(['\n', '\r'])).map(|s| s.to_string()).collect();
    if out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    Ok(out)
}

// ---------- temporary folders ----------

/// Makes the drive's temporary folder: "X:\Choppy Benchmark (temporary)" (a numbered name when a folder of that name
/// is someone else's), for the system drive also under %TEMP% when its root takes no new folder. Returns its path,
/// or the reason.
pub fn make_folder(root: &Path, system: bool) -> Result<PathBuf, String> {
    let mut why: Option<String> = None;
    let mut bases = vec![root.to_path_buf()];
    if system {
        bases.push(std::env::temp_dir());
    }
    for b in bases {
        for n in 1..=9 {
            let dir = b.join(if n == 1 { FOLDER_NAME.to_string() } else { format!("{FOLDER_NAME} {n}") });
            if dir.exists() {
                // ours from an earlier run that ended without cleaning up: removed, then made again
                if !is_ours(&dir) || !remove(&dir) {
                    continue;
                }
            }
            // listed before it exists: whatever happens next, the next start finds it
            register(&dir);
            let made = std::fs::create_dir_all(&dir).and_then(|_| {
                std::fs::write(
                    dir.join(MARKER_NAME),
                    utf8_bom(&format!(
                        "{MARKER_HEAD} - Choppy's Benchmark writes its test files here and removes this folder when it ends (or at Choppy's next start). Safe to delete.\r\n"
                    )),
                )
            });
            match made {
                Ok(()) => return Ok(dir),
                Err(e) => {
                    why = Some(if e.kind() == std::io::ErrorKind::PermissionDenied {
                        "it doesn't let this user make a folder".into()
                    } else {
                        io_message(&e)
                    });
                    if dir.is_dir() {
                        let _ = std::fs::remove_dir(&dir);
                    }
                    unregister(&dir);
                    break;
                }
            }
        }
    }
    Err(why.unwrap_or_else(|| "no free name for a temporary folder".into()))
}

/// An I/O error as .NET words it (the Win32 message).
pub fn io_message(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(c) => sys::win32_message(c as u32) + ".",
        None => e.to_string(),
    }
}

/// True when the folder carries the benchmark's marker file.
fn is_ours(dir: &Path) -> bool {
    let m = dir.join(MARKER_NAME);
    match read_lines(&m) {
        Ok(l) => l.first().is_some_and(|f| f.to_lowercase().starts_with(&MARKER_HEAD.to_lowercase())),
        Err(_) => false,
    }
}

fn register(dir: &Path) {
    let _g = crate::engine::lock(&LIST_LOCK);
    let _ = std::fs::create_dir_all(app_paths::data_dir());
    let f = list_file();
    let new = !f.exists();
    let mut text = Vec::new();
    if new {
        text.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    text.extend_from_slice(format!("{}\r\n", dir.display()).as_bytes());
    use std::io::Write;
    if let Ok(mut fh) = std::fs::OpenOptions::new().create(true).append(true).open(&f) {
        let _ = fh.write_all(&text);
    }
}

fn unregister(dir: &Path) {
    let _g = crate::engine::lock(&LIST_LOCK);
    let f = list_file();
    if !f.exists() {
        return;
    }
    let Ok(lines) = read_lines(&f) else { return };
    let d = dir.display().to_string().to_lowercase();
    let keep: Vec<String> = lines.iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && l.to_lowercase() != d).collect();
    if keep.is_empty() {
        let _ = std::fs::remove_file(&f);
    } else {
        let _ = std::fs::write(&f, utf8_bom(&keep.iter().map(|l| format!("{l}\r\n")).collect::<String>()));
    }
}

/// Removes one of the benchmark's folders: only one with its marker file (or an empty one), and inside it links are
/// removed, never followed. True when it is gone; a folder that stays is kept on the list for the next start.
pub fn remove(dir: &Path) -> bool {
    let gone = if !dir.exists() {
        true
    } else if is_ours(dir) {
        let long = paths::to_long(&WString::from(dir.display().to_string()));
        let mut g = false;
        // a virus scanner may still look at a file just written: a few more tries
        for i in 0..20 {
            if g {
                break;
            }
            if i > 0 {
                sys::sleep_ms(250);
            }
            delete_tree(&long);
            g = !paths::exists(&long);
        }
        g
    } else {
        // not ours (or the marker never got written): only an empty folder goes
        let _ = std::fs::remove_dir(dir);
        true // whatever is in it isn't the benchmark's: off the list, untouched
    };
    if gone {
        unregister(dir);
    }
    gone
}

/// Folders an earlier benchmark left behind (Choppy closed or the PC went off mid-run): removed. Never while a
/// benchmark runs (in this Choppy or another one): its folders are in use.
pub fn clean_leftovers() -> i32 {
    if !list_file().exists() {
        return 0;
    }
    let Some(_m) = lock() else { return 0 };
    let dirs = {
        let _g = crate::engine::lock(&LIST_LOCK);
        let f = list_file();
        if !f.exists() {
            return 0;
        }
        match read_lines(&f) {
            Ok(d) => d,
            Err(_) => return 0,
        }
    };
    let mut n = 0;
    for d in dirs {
        let dir = d.trim();
        if dir.is_empty() {
            continue;
        }
        let p = PathBuf::from(dir);
        let existed = p.is_dir();
        if remove(&p) && existed {
            n += 1;
        }
    }
    n
}

/// Held while a benchmark runs: one at a time, and no clean-up of leftovers meanwhile. None when another one has it.
/// The same thread may take it again (a run cleans up before it starts).
pub fn lock() -> Option<sys::Held> {
    sys::named_mutex_try(r"Local\Choppy-benchmark")
}

/// A folder and all in it. Links inside (junctions, symbolic links) are removed themselves, never followed.
pub fn delete_tree(dir: &WString) {
    let a = sys::get_file_attributes(dir);
    if a == sys::INVALID_FILE_ATTRIBUTES {
        return;
    }
    if a & sys::FILE_ATTRIBUTE_REPARSE_POINT == 0 && a & sys::FILE_ATTRIBUTE_DIRECTORY != 0 {
        if let Ok(find) = sys::find_first(&paths::combine(dir, &WString::from("*"))) {
            for fd in find {
                let n = fd.name.to_string_lossy();
                if n == "." || n == ".." {
                    continue;
                }
                let p = paths::combine(dir, &fd.name);
                if fd.attributes & sys::FILE_ATTRIBUTE_DIRECTORY != 0 {
                    if fd.attributes & sys::FILE_ATTRIBUTE_REPARSE_POINT == 0 {
                        delete_tree(&p);
                    } else {
                        sys::remove_directory(&p);
                    }
                } else {
                    if fd.attributes & sys::FILE_ATTRIBUTE_READONLY != 0 {
                        sys::set_file_attributes(&p, sys::FILE_ATTRIBUTE_NORMAL);
                    }
                    let _ = sys::delete_file(&p);
                }
            }
        }
        sys::remove_directory(dir);
    } else if a & sys::FILE_ATTRIBUTE_DIRECTORY != 0 {
        sys::remove_directory(dir); // a link: only the link
    } else {
        let _ = sys::delete_file(dir);
    }
}

// ---------- test data ----------

/// The small files: sizes spread evenly on a log scale from 4 KB to 1 MB (most are small, most bytes are in the
/// larger ones), the same set on every PC and every run (System.Random(20261123)).
pub fn small_sizes(count: usize) -> Vec<i64> {
    let mut r = crate::cli::selftest::NetRandom::new(20261123);
    let (lo, hi) = (4096f64.ln(), ((1 << 20) as f64).ln());
    (0..count).map(|_| (lo + r.next_double() * (hi - lo)).exp().round_ties_even() as i64).collect()
}

/// One big file written past the Windows cache (unbuffered, then flushed): nothing of it is in memory.
pub fn write_big(path: &Path, size: i64, canceled: &dyn Fn() -> bool, progress: Option<&dyn Fn(i64)>) -> BenchResult<()> {
    const CHUNK: usize = 8 << 20;
    let Some(mut mem) = sys::Memory::commit(CHUNK) else { return Err(BenchErr::Error("Insufficient memory to continue the execution of the program.".into())) };
    fill(mem.bytes_mut(), size as u64);
    let long = paths::to_long(&WString::from(path.display().to_string()));
    let h = sys::create_file(&long, sys::GENERIC_WRITE, 0, sys::CREATE_NEW, sys::FILE_FLAG_NO_BUFFERING)
        .map_err(|e| BenchErr::Error(format!("Can't write {}: {}", path.display(), sys::error_text(e))))?;
    let mut done = 0i64;
    while done < size {
        if canceled() {
            return Err(BenchErr::Canceled);
        }
        let n = (CHUNK as i64).min(size - done) as usize; // sizes are whole MB: always a multiple of the sector
        mem.bytes_mut()[..8].copy_from_slice(&done.to_le_bytes()); // every piece differs
        match sys::write(&h, &mem.bytes()[..n]) {
            Ok(w) if w as usize == n => {}
            Ok(_) => return Err(BenchErr::Error(format!("Can't write {}: {}", path.display(), sys::error_text(0)))),
            Err(e) => return Err(BenchErr::Error(format!("Can't write {}: {}", path.display(), sys::error_text(e)))),
        }
        done += n as i64;
        if let Some(p) = progress {
            p(n as i64);
        }
    }
    let _ = sys::flush(&h);
    Ok(())
}

fn fill(p: &mut [u8], seed: u64) {
    let mut x = 0x9E3779B97F4A7C15u64 ^ seed;
    for c in p.as_chunks_mut::<8>().0 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        c.copy_from_slice(&x.to_le_bytes());
    }
}

/// Many small files in a few folders. Written through the cache, then flushed and dropped from it (Purge).
pub fn write_small(dir: &Path, sizes: &[i64], canceled: &dyn Fn() -> bool, progress: Option<&dyn Fn(i64)>) -> BenchResult<Vec<PathBuf>> {
    let mut data = vec![0u8; 2 << 20];
    crate::cli::selftest::NetRandom::new(7).next_bytes(&mut data);
    let mut files = Vec::with_capacity(sizes.len());
    let folders = (sizes.len() / 100).max(1);
    for (i, &size) in sizes.iter().enumerate() {
        if canceled() {
            return Err(BenchErr::Canceled);
        }
        let sub = dir.join(format!("Folder {:02}", i % folders + 1));
        if i < folders {
            std::fs::create_dir_all(&sub).map_err(|e| BenchErr::Error(io_message(&e)))?;
        }
        let f = sub.join(format!("File {:05}.bin", i + 1));
        let off = ((i as i64 * 7919) % (data.len() as i64 - (1 << 20))) as usize;
        use std::io::Write;
        let mut fh = std::fs::OpenOptions::new().write(true).create_new(true).open(&f).map_err(|e| BenchErr::Error(io_message(&e)))?;
        fh.write_all(&data[off..off + size as usize]).map_err(|e| BenchErr::Error(io_message(&e)))?;
        files.push(f);
        if let Some(p) = progress {
            p(size);
        }
    }
    Ok(files)
}

/// Drops files from the Windows cache: an open past the cache with no other handle open makes the file system write
/// out and drop what it holds of the file (measured: a re-read comes from the disk). Flushed first, many at once, so
/// the drop has nothing left to write.
pub fn purge(files: &[PathBuf], canceled: &(dyn Fn() -> bool + Sync)) -> BenchResult<()> {
    flush(files, canceled)?;
    for_all(files, 64, canceled, &|f| {
        let _ = sys::create_file(
            &paths::to_long(&WString::from(f.display().to_string())),
            sys::GENERIC_READ,
            sys::FILE_SHARE_READ | sys::FILE_SHARE_WRITE | sys::FILE_SHARE_DELETE,
            sys::OPEN_EXISTING,
            sys::FILE_FLAG_NO_BUFFERING,
        );
        Ok(())
    })
}

/// Puts each file's data on the disk itself (FlushFileBuffers), up to 256 at once: the disk then orders the writes
/// itself (1,000 small files on a hard disk: 256 at once 2.1 s, 32 at once 7.7 s, one by one 22 s).
pub fn flush(files: &[PathBuf], canceled: &(dyn Fn() -> bool + Sync)) -> BenchResult<()> {
    for_all(files, 256, canceled, &|f| {
        if let Ok(h) = sys::create_file(
            &paths::to_long(&WString::from(f.display().to_string())),
            sys::GENERIC_WRITE,
            sys::FILE_SHARE_READ | sys::FILE_SHARE_WRITE | sys::FILE_SHARE_DELETE,
            sys::OPEN_EXISTING,
            0,
        ) {
            let _ = sys::flush(&h);
        }
        Ok(())
    })
}

/// Every file under a folder with its size (links are not followed).
pub fn list(dir: &Path) -> Vec<(PathBuf, i64)> {
    let mut r = Vec::new();
    list_into(&paths::to_long(&WString::from(dir.display().to_string())), &mut r);
    r
}

fn list_into(dir: &WString, r: &mut Vec<(PathBuf, i64)>) {
    let Ok(find) = sys::find_first(&paths::combine(dir, &WString::from("*"))) else { return };
    let mut subs = Vec::new();
    for fd in find {
        let n = fd.name.to_string_lossy();
        if n == "." || n == ".." {
            continue;
        }
        let p = paths::combine(dir, &fd.name);
        if fd.attributes & sys::FILE_ATTRIBUTE_DIRECTORY != 0 {
            if fd.attributes & sys::FILE_ATTRIBUTE_REPARSE_POINT == 0 {
                subs.push(p);
            }
        } else {
            r.push((PathBuf::from(paths::to_short(&p).to_os()), fd.size()));
        }
    }
    for s in subs {
        list_into(&s, r);
    }
}

/// Runs an action on every item with a few threads; the first error is returned after all have stopped.
pub fn for_all(items: &[PathBuf], threads: usize, canceled: &(dyn Fn() -> bool + Sync), act: &(dyn Fn(&Path) -> BenchResult<()> + Sync)) -> BenchResult<()> {
    if items.is_empty() {
        return Ok(());
    }
    let next = AtomicUsize::new(0);
    let first: Arc<Mutex<Option<BenchErr>>> = Arc::new(Mutex::new(None));
    std::thread::scope(|s| {
        for _ in 0..threads.min(items.len()) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, SeqCst);
                    if i >= items.len() || crate::engine::lock(&first).is_some() || canceled() {
                        break;
                    }
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| act(&items[i])))
                        .unwrap_or_else(|e| Err(BenchErr::Error(crate::engine::panic_message(e.as_ref()))));
                    if let Err(e) = r {
                        let mut f = crate::engine::lock(&first);
                        if f.is_none() {
                            *f = Some(e);
                        }
                    }
                }
            });
        }
    });
    if let Some(e) = crate::engine::lock(&first).take() {
        return Err(e);
    }
    if canceled() {
        return Err(BenchErr::Canceled);
    }
    Ok(())
}
