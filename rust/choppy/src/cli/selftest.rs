//! `--selftest out.txt` (Cli.SelfTest): the same check names in the same order as the C# self-test: xxHash64, the
//! paths, the temp names and the pipeline block (PipelineChecks), the plan per drive. Step 5 adds the Ctrl+V hook and
//! step 4 the bench checks (after the plan checks).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use super::{append_line, write_all_text};
use crate::engine::big_copy::{self, PipeTuning};
use crate::engine::copy_job;
use crate::engine::xxhash64;
use crate::engine::model::{JobSpec, OpKind};
use crate::engine::paths;
use crate::engine::plan::Plan;
use crate::engine::volume_probe::{self, DeviceKind, FILE_DEVICE_DISK, StorageDeviceNumber, VolumeInfo};
use crate::engine::xxhash64::XxHash64;
use crate::sys;
use crate::wstr::{WString, w};

/// System.Random(seed).NextBytes: the same pseudo-random bytes the C# self-test hashes (Knuth's subtractive generator
/// as .NET Framework implements it).
pub struct NetRandom {
    seed_array: [i32; 56],
    inext: usize,
    inextp: usize,
}

impl NetRandom {
    pub fn new(seed: i32) -> NetRandom {
        const MSEED: i32 = 161803398;
        let mut sa = [0i32; 56];
        let subtraction = if seed == i32::MIN { i32::MAX } else { seed.abs() };
        let mut mj = MSEED - subtraction;
        sa[55] = mj;
        let mut mk = 1i32;
        for i in 1..55 {
            let ii = (21 * i) % 55;
            sa[ii] = mk;
            mk = mj.wrapping_sub(mk);
            if mk < 0 {
                mk += i32::MAX;
            }
            mj = sa[ii];
        }
        for _ in 1..5 {
            for i in 1..56 {
                sa[i] = sa[i].wrapping_sub(sa[1 + (i + 30) % 55]);
                if sa[i] < 0 {
                    sa[i] += i32::MAX;
                }
            }
        }
        NetRandom { seed_array: sa, inext: 0, inextp: 21 }
    }

    fn internal_sample(&mut self) -> i32 {
        let mut loc_inext = self.inext + 1;
        if loc_inext >= 56 {
            loc_inext = 1;
        }
        let mut loc_inextp = self.inextp + 1;
        if loc_inextp >= 56 {
            loc_inextp = 1;
        }
        let mut ret = self.seed_array[loc_inext].wrapping_sub(self.seed_array[loc_inextp]);
        if ret == i32::MAX {
            ret -= 1;
        }
        if ret < 0 {
            ret += i32::MAX;
        }
        self.seed_array[loc_inext] = ret;
        self.inext = loc_inext;
        self.inextp = loc_inextp;
        ret
    }

    pub fn next_bytes(&mut self, buf: &mut [u8]) {
        for b in buf.iter_mut() {
            *b = (self.internal_sample() % 256) as u8;
        }
    }
}

pub fn self_test(out_file: Option<&WString>) -> i32 {
    let mut sb = String::new();
    let mut fails = 0;
    let mut check = |name: &str, ok: bool| {
        append_line(&mut sb, &format!("{}{}", if ok { "PASS " } else { "FAIL " }, name));
        if !ok {
            fails += 1;
        }
    };
    // xxHash64 reference values (seed 0)
    check("xxh64 empty", XxHash64::of_bytes(&[]) == 0xEF46DB3751D8E999);
    check("xxh64 'a'", XxHash64::of_bytes(b"a") == 0xD24EC4F1A98C6E5B);
    check("xxh64 'abc'", XxHash64::of_bytes(b"abc") == 0x44BC2CF5AD770999);
    let fox = "The quick brown fox jumps over the lazy dog";
    check("xxh64 fox", XxHash64::of_bytes(fox.as_bytes()) == 0x0B242D361FDA71BC);
    // streaming in odd pieces gives the same digest
    let mut big = vec![0u8; 100003];
    NetRandom::new(7).next_bytes(&mut big);
    let whole = XxHash64::of_bytes(&big);
    let mut x = XxHash64::new();
    let steps = [1usize, 3, 31, 32, 33, 64, 999, 4096, 7];
    let (mut pos, mut k) = (0usize, 0usize);
    while pos < big.len() {
        let n = steps[k % steps.len()].min(big.len() - pos);
        k += 1;
        x.update(&big[pos..pos + n]);
        pos += n;
    }
    check("xxh64 streaming", x.digest() == whole);
    // paths
    let long = |s: &str| paths::to_long(&w(s));
    check("long local", long(r"C:\a\b\") == w(r"\\?\C:\a\b"));
    check("long unc", long(r"\\srv\share\x") == w(r"\\?\UNC\srv\share\x"));
    check("short unc", paths::to_short(&w(r"\\?\UNC\srv\share\x")) == w(r"\\srv\share\x"));
    check("inside", paths::is_same_or_inside(&w(r"\\?\C:\a\b\c"), &w(r"\\?\C:\a\b")));
    check("not inside", !paths::is_same_or_inside(&w(r"\\?\C:\a\bc"), &w(r"\\?\C:\a\b")));
    check("parent of root", paths::parent(&w(r"C:\")).is_none() && paths::parent(&w(r"\\?\C:\")).is_none());
    check("drive root from Explorer (C:\")", long("C:\"") == w(r"\\?\C:\") && long(r#"\\?\C:""#) == w(r"\\?\C:\"));
    check("folder arg with quote", long("D:\\Games\"") == w(r"\\?\D:\Games"));
    check(
        "parent",
        paths::parent(&w(r"\\?\C:\x")) == Some(w(r"\\?\C:\")) && paths::parent(&w(r"\\?\C:\x\y\")) == Some(w(r"\\?\C:\x")),
    );
    check("struct size", sys::struct_sizes() == (592, 36));
    check("numbered", paths::numbered_name(&w("photo.jpg"), 2, false) == w("photo (2).jpg"));
    check("numbered noext", paths::numbered_name(&w("README"), 3, false) == w("README (3)"));
    check("temp", copy_job::temp_name(&w(r"\\?\C:\x\a.txt")) == w(r"\\?\C:\x\a.txt.chpart"));
    let long_name = "n".repeat(252) + ".bin";
    let tl = copy_job::temp_name(&(w(r"\\?\C:\x\") + long_name.as_str()));
    check("temp long", tl.ends_with(".chpart") && paths::file_name(&tl).len() < 255);
    pipeline_checks(&mut check);

    // the way per drive: the same ReFS volume goes to Windows' own copy (it clones the blocks); FAT32 caps a file at 4 GB
    let vol = |id: &str, kind: DeviceKind, fs: &str, disk: i32| VolumeInfo {
        id: w(id),
        kind,
        file_system: Some(fs.into()),
        disk,
        ..Default::default()
    };
    let refs = VolumeInfo { fs_flags: sys::FILE_SUPPORTS_BLOCK_REFCOUNTING, ..vol("vol:1", DeviceKind::Ssd, "ReFS", -1) };
    let ntfs = vol("vol:2", DeviceKind::Ssd, "NTFS", -1);
    let copy = JobSpec::new(OpKind::Copy);
    let mv = JobSpec::new(OpKind::Move);
    let pr = Plan::choose(&copy, Some(&refs), Some(&refs), None);
    check("same ReFS drive: Windows' own copy (block clone)", pr.clone && !pr.pipeline && !pr.instant_move);
    check("ReFS to another drive: the pipeline", Plan::choose(&copy, Some(&refs), Some(&ntfs), None).pipeline);
    check("same ReFS drive, a move: renamed in place", Plan::choose(&mv, Some(&refs), Some(&refs), None).instant_move);
    // only a disk device's number says which disk a volume is on (a cloud drive answers as a file system, "number 0")
    let dn = |t: i32, n: i32| StorageDeviceNumber { device_type: t, device_number: n, partition_number: 0 };
    check(
        "disk number: a disk counts, a file system device doesn't",
        volume_probe::disk_number(dn(FILE_DEVICE_DISK, 2)) == 2
            && volume_probe::disk_number(dn(8, 0)) == -1
            && !volume_probe::same_disk(
                Some(&VolumeInfo { id: w("vol:g"), disk: -1, ..Default::default() }),
                Some(&VolumeInfo { id: w("vol:x"), disk: 0, kind: DeviceKind::Hdd, ..Default::default() }),
            ),
    );
    // big files within one hard disk: the whole file (up to 512 MB) read in 1 MB reads, then written in 64 MB writes, room
    // reserved, nothing through the cache, no trials; one USB drive keeps 32 MB batches with trials; two hard disks unchanged
    let hdd1 = vol("vol:h1", DeviceKind::Hdd, "NTFS", 1);
    let hdd1b = vol("vol:h1b", DeviceKind::Hdd, "NTFS", 1);
    let hdd2 = vol("vol:h2", DeviceKind::Hdd, "NTFS", 2);
    let usb = vol("vol:u", DeviceKind::Usb, "NTFS", 3);
    let one = Plan::choose(&copy, Some(&hdd1), Some(&hdd1b), None);
    check(
        "one hard disk: whole-file batches of 1 MB reads, 64 MB writes, room reserved, cached end only once read, no trials",
        one.pipe.phased
            && one.pipe.chunk == 1 << 20
            && one.pipe.slots == 512
            && one.pipe.read_depth == 32
            && one.pipe.write_step == 64 << 20
            && one.pipe.prealloc
            && one.pipe.tail_after_read
            && one.pipe_ways.is_empty(),
    );
    let one_usb = Plan::choose(&copy, Some(&usb), Some(&usb), None);
    check("one USB drive: 32 MB batches with trials", one_usb.pipe.phased && one_usb.pipe.chunk == 32 << 20 && !one_usb.pipe_ways.is_empty());
    let two = Plan::choose(&copy, Some(&hdd1), Some(&hdd2), None);
    check(
        "two hard disks unchanged",
        !two.pipe.phased && two.pipe.chunk == 2 << 20 && two.pipe.slots == 256 && two.pipe.read_depth == 4 && two.pipe.tail_after_read,
    );
    // a hard disk into a fast drive: 1 MB reads, only 4 out at the disk at once, no trials; the other way round unchanged
    let hdd_to_ssd = Plan::choose(&copy, Some(&hdd1), Some(&ntfs), None);
    let ssd_to_hdd = Plan::choose(&copy, Some(&ntfs), Some(&hdd1), None);
    check(
        "hard disk into an SSD: 1 MB reads, 4 out at once, no trials",
        !hdd_to_ssd.pipe.phased
            && hdd_to_ssd.pipe.chunk == 1 << 20
            && hdd_to_ssd.pipe.slots == 32
            && hdd_to_ssd.pipe.read_depth == 4
            && hdd_to_ssd.pipe_ways.is_empty(),
    );
    check(
        "SSD into a hard disk unchanged",
        ssd_to_hdd.pipe.chunk == 32 << 20 && ssd_to_hdd.pipe.read_depth == 0 && ssd_to_hdd.pipe_ways.len() == 2,
    );
    let fs = |f: Option<&str>| VolumeInfo { file_system: f.map(|s| s.to_string()), ..Default::default() };
    check(
        "FAT32 holds files up to 4 GB",
        fs(Some("FAT32")).max_file_size() == u32::MAX as i64
            && fs(Some("exFAT")).max_file_size() == 0
            && ntfs.max_file_size() == 0
            && fs(None).max_file_size() == 0,
    );
    // (step 5: the Ctrl+V hook check; step 4: the bench checks)
    append_line(&mut sb, &if fails == 0 { "ALL PASS".to_string() } else { format!("{fails} FAILED") });
    if let Some(f) = out_file {
        write_all_text(f, &sb, false).expect("--selftest: the output file");
    }
    if fails == 0 { 0 } else { 2 }
}

/// A UTC date as a FILETIME (DateTime(y, m, d, h, mi, s, Utc).ToFileTimeUtc()).
fn utc_file_time(y: i64, m: i64, d: i64, h: i64, mi: i64, s: i64) -> i64 {
    // days since 0000-03-01 (years starting in March), then from 1601-01-01
    let (yy, mm) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let doy = (153 * mm + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 584694; // 1601-01-01 is day 584,694 after 0000-03-01
    (days * 86400 + h * 3600 + mi * 60 + s) * 10_000_000
}

fn long(p: &Path) -> WString {
    paths::to_long(&WString::from_os(p.as_os_str()))
}

/// File.GetLastWriteTimeUtc(p).ToFileTimeUtc()
fn mtime(p: &Path) -> i64 {
    sys::get_file_attributes_ex(&long(p)).map_or(0, |d| d.write)
}

/// new Random(seed).NextBytes(new byte[size])
fn random(size: usize, seed: i32) -> Vec<u8> {
    let mut data = vec![0u8; size];
    NetRandom::new(seed).next_bytes(&mut data);
    data
}

fn same_bytes(p: &Path, data: &[u8]) -> bool {
    std::fs::read(p).is_ok_and(|b| b == data)
}

/// The writer a check gets: (name, passed).
type Check<'a> = &'a mut dyn FnMut(&str, bool);

/// BigCopy.Copy with no tuner and no cap; progress always goes on.
#[allow(clippy::too_many_arguments)]
fn pipe(a: &Path, b: &Path, ctime: i64, mtime: i64, attrs: u32, t: &PipeTuning, hash: bool, flush: bool, tail: i64) -> big_copy::BigResult {
    big_copy::copy(&long(a), &long(b), ctime, mtime, attrs, t, hash, flush, None, tail, &mut |_| true, None, None)
}

/// The pipeline against real files in %TEMP%: odd sizes, an extra stream, dates, and both ways of hashing.
fn pipeline_checks(check: Check) {
    let dir: PathBuf = std::env::temp_dir().join(format!("choppy-selftest-{:08x}", sys::new_guid().data1));
    // outside the catch-all, as in C#: no folder = the self-test itself fails (the CLI's error file, exit 1)
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let r = catch_unwind(AssertUnwindSafe(|| pipeline_body(&dir, &mut *check)));
    if let Err(e) = r {
        check(&format!("pipeline: {}", crate::engine::panic_message(&*e)), false);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn pipeline_body(dir: &Path, check: Check) {
    let tune = PipeTuning { chunk: 1 << 20, slots: 3, align: 4096, ..Default::default() };
    for size in [0usize, 1, 4096, 2 << 20, (3 << 20) + 12345] {
        let (a, b) = (dir.join(format!("a{size}.bin")), dir.join(format!("b{size}.bin")));
        let data = random(size, size as i32);
        std::fs::write(&a, &data).expect("the test file");
        {
            let h = sys::create_file(&long(&a), sys::FILE_WRITE_ATTRIBUTES, sys::FILE_SHARE_READ, sys::OPEN_EXISTING, 0)
                .expect("the test file's dates");
            sys::set_file_time(&h, Some(utc_file_time(2020, 1, 2, 3, 4, 5)), Some(utc_file_time(2021, 6, 7, 8, 9, 10)));
        }
        let fa = sys::get_file_attributes_ex(&long(&a)).expect("the test file");
        let r = pipe(&a, &b, fa.creation, fa.write, fa.attributes, &tune, true, true, 0);
        let same = r.error == 0 && b.is_file() && same_bytes(&b, &data);
        check(&format!("pipeline copy {size} B"), same && r.hash == XxHash64::of_bytes(&data));
        let fb = sys::get_file_attributes_ex(&long(&b));
        check(&format!("pipeline dates {size} B"), fb.is_ok_and(|fb| fb.creation == fa.creation && fb.write == fa.write));
        let want = XxHash64::of_bytes(&data);
        check(&format!("hash unbuffered {size} B"), xxhash64::hash_file(&long(&b), true, None) == Ok(want));
        check(&format!("hash buffered {size} B"), xxhash64::hash_file(&long(&a), false, None) == Ok(want));
    }
    // pieces joined into bigger writes: same bytes, at every odd size
    for size in [(1usize << 20) * 3, (5 << 20) + 777, (7 << 20) + 4096, 123457] {
        let (a, b) = (dir.join(format!("join-a{size}.bin")), dir.join(format!("join-b{size}.bin")));
        let data = random(size, size as i32 + 1);
        std::fs::write(&a, &data).expect("the test file");
        let want = XxHash64::of_bytes(&data);
        let jt = PipeTuning { chunk: 1 << 20, slots: 4, align: 4096, write_batch: 3, ..Default::default() };
        let rj = pipe(&a, &b, 0, mtime(&a), 0x20, &jt, true, false, 0);
        check(&format!("pipeline joined writes {size} B"), rj.error == 0 && rj.hash == want && same_bytes(&b, &data));
        // one disk: read a batch, then write it
        let c = dir.join(format!("phase-{size}.bin"));
        let pt = PipeTuning { chunk: 1 << 20, slots: 3, align: 4096, phased: true, ..Default::default() };
        let rp = pipe(&a, &c, 0, mtime(&a), 0x20, &pt, true, false, 2 << 20);
        check(&format!("pipeline phased {size} B"), rp.error == 0 && rp.hash == want && same_bytes(&c, &data));
        // one hard disk: small reads, a few at the disk at once, the batch written in bigger steps, room reserved
        let ph = dir.join(format!("phase2-{size}.bin"));
        let pt2 = PipeTuning {
            chunk: 1 << 20,
            slots: 6,
            align: 4096,
            phased: true,
            read_depth: 2,
            write_step: 3 << 20,
            prealloc: true,
            ..Default::default()
        };
        let rp2 = pipe(&a, &ph, 0, mtime(&a), 0x20, &pt2, true, false, 0);
        check(
            &format!("pipeline phased, read depth + write step {size} B"),
            rp2.error == 0 && rp2.hash == want && same_bytes(&ph, &data),
        );
        // nothing reserved up front (hard disk destinations): the file grows as it is written
        let g = dir.join(format!("grow-{size}.bin"));
        let gt = PipeTuning { chunk: 1 << 20, slots: 4, align: 4096, write_batch: 3, prealloc: false, ..Default::default() };
        let rg = pipe(&a, &g, 0, mtime(&a), 0x20, &gt, true, false, 0);
        check(&format!("pipeline no reservation {size} B"), rg.error == 0 && rg.hash == want && same_bytes(&g, &data));
    }
    // the end through the Windows cache: same bytes, same size
    {
        let (a, b) = (dir.join("tail-a.bin"), dir.join("tail-b.bin"));
        let data = random((5 << 20) + 777, 9);
        std::fs::write(&a, &data).expect("the test file");
        let rt = pipe(&a, &b, 0, mtime(&a), 0x20, &tune, false, false, 2 << 20);
        check("pipeline cached tail", rt.error == 0 && same_bytes(&b, &data));
        // two hard disks: the cached end starts once the source is fully read (whatever the buffers still hold)
        let c = dir.join("tail-c.bin");
        let at = PipeTuning {
            chunk: 1 << 20,
            slots: 8,
            align: 4096,
            write_batch: 2,
            prealloc: false,
            tail_after_read: true,
            ..Default::default()
        };
        let ra = pipe(&a, &c, 0, mtime(&a), 0x20, &at, true, false, 64 << 20);
        check("pipeline cached end after read", ra.error == 0 && ra.hash == XxHash64::of_bytes(&data) && same_bytes(&c, &data));
    }
    // an extra data stream (downloads carry Zone.Identifier) comes along
    let (s, t) = (dir.join("s.bin"), dir.join("t.bin"));
    std::fs::write(&s, vec![0u8; 5000]).expect("the test file");
    const ZONE: &str = "[ZoneTransfer]\r\nZoneId=3\r\n";
    write_stream(&(long(&s) + ":Zone.Identifier"), ZONE);
    let rs = pipe(&s, &t, 0, mtime(&s), 0x20, &tune, false, false, 0);
    let named = if rs.error != 0 { format!("pipeline stream (error {})", rs.error as i32) } else { "pipeline stream".to_string() };
    check(&named, rs.error == 0 && read_stream(&(long(&t) + ":Zone.Identifier")).as_deref() == Some(ZONE));
    // never replaces anything
    let rx = pipe(&s, &t, 0, 0, 0x20, &tune, false, false, 0);
    check("pipeline never replaces", rx.error == sys::ERROR_FILE_EXISTS);
    // stopped half way: says so
    std::fs::write(&s, vec![0u8; 4 << 20]).expect("the test file");
    let mut stop = |n: i64| n < (2 << 20);
    let rc = big_copy::copy(&long(&s), &long(&dir.join("c.bin")), 0, 0, 0x20, &tune, false, false, None, 0, &mut stop, None, None);
    check("pipeline stop", rc.error == sys::ERROR_REQUEST_ABORTED);
}

/// An extra data stream written as ASCII (CREATE_ALWAYS). A stream that can't be made throws as C#'s
/// `new FileStream(<invalid handle>)` did (the catch-all then ends the block).
fn write_stream(path: &WString, text: &str) {
    match sys::create_file(path, sys::GENERIC_WRITE, 0, sys::CREATE_ALWAYS, 0) {
        Ok(h) => {
            let _ = sys::write(&h, text.as_bytes());
        }
        Err(_) => panic!("Invalid handle.\r\nParameter name: handle"),
    }
}

/// An extra data stream read as ASCII; None = it isn't there.
fn read_stream(path: &WString) -> Option<String> {
    let h = sys::create_file(path, sys::GENERIC_READ, sys::FILE_SHARE_READ, sys::OPEN_EXISTING, 0).ok()?;
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    while let Ok(n) = sys::read(&h, &mut buf) {
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n as usize]);
    }
    Some(out.iter().map(|&b| if b < 128 { b as char } else { '?' }).collect())
}
