//! Engine measurements for tuning (stdout; nothing here is used by a copy):
//! `--diag probe <path>...`                          how long a drive probe takes
//! `--diag scan <src> <dest> <threads,...> [rounds]` the folder listing with that many threads
//! `--diag smallcopy <src> <dest> <threads> <way>`   small files copied one per call, that many at once, each way:
//!   direct = CopyFileEx straight to the name, temp = to "<name>.chpart" + the full-path rename through its handle,
//!   plain = the same with the plain-name rename in its folder (sys::rename_in_folder), own = read + write + dates

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use crate::engine::model::{JobSpec, OpKind};
use crate::engine::paths;
use crate::engine::scanner::Scanner;
use crate::engine::volume_probe;
use crate::sys;
use crate::wstr::WString;

pub fn diag(args: &[WString]) -> i32 {
    let arg = |i: usize| args.get(i).map(|a| a.to_string_lossy()).unwrap_or_default();
    let path = |i: usize| paths::to_long(&paths::fix_arg(&WString::from(arg(i).as_str())));
    match arg(1).as_str() {
        "probe" => {
            for p in &args[2..] {
                let long = paths::to_long(&paths::fix_arg(p));
                let mut ms = Vec::new();
                let mut v = volume_probe::VolumeInfo::default();
                for _ in 0..3 {
                    let sw = Instant::now();
                    v = volume_probe::probe(&long);
                    ms.push(sw.elapsed().as_secs_f64() * 1000.0);
                }
                println!("{p} kind={} disk={} usb={} probe ms {:.1} / {:.1} / {:.1}", v.kind_text(), v.disk, v.on_usb, ms[0], ms[1], ms[2]);
            }
            0
        }
        "scan" if args.len() >= 5 => {
            let mut spec = JobSpec::new(OpKind::Copy);
            spec.sources.push(path(2));
            spec.dest = path(3);
            let threads: Vec<i32> = arg(4).split(',').filter_map(|t| t.trim().parse().ok()).collect();
            let rounds: usize = arg(5).parse().unwrap_or(3);
            for r in 0..rounds {
                for &t in &threads {
                    let sw = Instant::now();
                    let res = Scanner::new(|| false).run(&spec, t, 2);
                    println!(
                        "round {} threads {t:>2}: {:>8.1} ms  files {} dirs {} problems {}",
                        r + 1,
                        sw.elapsed().as_secs_f64() * 1000.0,
                        res.files.len(),
                        res.dirs.len(),
                        res.problems.len()
                    );
                }
            }
            0
        }
        "smallcopy" if args.len() >= 6 => {
            let dst = path(3);
            let threads: usize = arg(4).parse().unwrap_or(8);
            let way = arg(5);
            let mut spec = JobSpec::new(OpKind::Copy);
            spec.sources.push(path(2));
            spec.dest = dst.clone();
            let res = Scanner::new(|| false).run(&spec, 8, 2);
            let _ = sys::create_directory(&dst);
            for d in &res.dirs {
                let _ = sys::create_directory(&paths::combine(&dst, &d.rel));
            }
            let (next, failed) = (AtomicUsize::new(0), AtomicUsize::new(0));
            let sw = Instant::now();
            std::thread::scope(|sc| {
                for _ in 0..threads {
                    sc.spawn(|| {
                        let mut buf = vec![0u8; 1 << 20];
                        while let Some(f) = res.files.get(next.fetch_add(1, Ordering::Relaxed)) {
                            let to = paths::combine(&dst, &f.rel);
                            let ok = match way.as_str() {
                                "own" => own_copy(&f.src, &to, f.ctime, f.mtime, f.attr, &mut buf),
                                "temp" | "plain" => {
                                    let temp = to.clone() + ".chpart";
                                    let mut renamed = false;
                                    let r = copy_file(&f.src, &temp, &mut |p| {
                                        if let Some(hd) = p.dest
                                            && !renamed
                                            && p.transferred >= p.total
                                        {
                                            renamed = if way == "plain" { sys::rename_local(hd, &temp, &to) } else { sys::rename_by_handle(hd, &to) };
                                        }
                                    });
                                    r && renamed
                                }
                                _ => copy_file(&f.src, &to, &mut |_| {}),
                            };
                            if !ok {
                                failed.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    });
                }
            });
            println!(
                "{way} threads {threads}: {} files in {:.0} ms, failed {}",
                res.files.len(),
                sw.elapsed().as_secs_f64() * 1000.0,
                failed.load(Ordering::Relaxed)
            );
            0
        }
        _ => {
            eprintln!("--diag probe <path>... | scan <src> <dest> <threads,...> [rounds] | smallcopy <src> <dest> <threads> direct|temp|plain|own");
            1
        }
    }
}

fn copy_file(src: &WString, dst: &WString, each: &mut dyn FnMut(&sys::CopyProgress)) -> bool {
    let flags = sys::COPY_FILE_FAIL_IF_EXISTS | sys::COPY_FILE_ALLOW_DECRYPTED_DESTINATION;
    sys::copy_file_ex(src, dst, flags, &mut |p| {
        each(p);
        sys::PROGRESS_CONTINUE
    })
    .is_ok()
}

/// One small file read whole, written to a new name, its dates and attributes set on the open copy.
fn own_copy(src: &WString, dst: &WString, ctime: i64, mtime: i64, attr: u32, buf: &mut Vec<u8>) -> bool {
    let share = sys::FILE_SHARE_READ | sys::FILE_SHARE_DELETE;
    let Ok(hs) = sys::create_file(src, sys::GENERIC_READ, share, sys::OPEN_EXISTING, sys::FILE_FLAG_SEQUENTIAL_SCAN) else {
        return false;
    };
    let Ok(size) = sys::file_size(&hs) else { return false };
    let size = size as usize;
    if size > buf.len() {
        buf.resize(size, 0);
    }
    let mut got = 0;
    while got < size {
        match sys::read(&hs, &mut buf[got..size]) {
            Ok(0) => break,
            Ok(n) => got += n as usize,
            Err(_) => return false,
        }
    }
    let Ok(hd) = sys::create_file(dst, sys::GENERIC_WRITE, sys::FILE_SHARE_READ, sys::CREATE_NEW, sys::ATTRIBUTE_NORMAL) else {
        return false;
    };
    if got > 0 && sys::write(&hd, &buf[..got]).is_err() {
        return false;
    }
    let keep = attr & (sys::FILE_ATTRIBUTE_READONLY | sys::FILE_ATTRIBUTE_HIDDEN | sys::FILE_ATTRIBUTE_SYSTEM | sys::FILE_ATTRIBUTE_ARCHIVE);
    sys::set_basic_info(&hd, ctime, mtime, if keep == 0 { sys::FILE_ATTRIBUTE_NORMAL } else { keep })
}
