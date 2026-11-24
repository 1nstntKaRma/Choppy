//! `--selftest out.txt` (Cli.SelfTest): the same check names in the same order as the C# self-test. Step 1 has its
//! checks in: xxHash64, the paths, the plan per drive. Step 2 adds "temp" + the pipeline block (after the paths),
//! step 5 the Ctrl+V hook and step 4 the bench checks (after the plan checks).

use super::{append_line, write_all_text};
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
    // (step 2: "temp", "temp long" and the pipeline checks go here)

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
