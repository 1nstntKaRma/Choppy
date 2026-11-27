//! The method Adaptive mode picked for one job. Shown in the badge (Plan.cs).

use super::big_copy::PipeTuning;
use super::model::{JobSpec, OpKind, ScanResult};
use super::volume_probe::{self, DeviceKind, VolumeInfo};

#[derive(Clone, Debug)]
pub struct Plan {
    pub workers: i32,                // files in flight at once
    pub big_slots: i32,              // big files in flight at once
    pub big_threshold: i64,
    pub pipeline: bool,              // big local files: read and write at the same time, no Windows cache
    pub pipe_threshold: i64,
    pub pipe: PipeTuning,
    pub pipe_ways: Vec<PipeTuning>,  // other ways the tuner measures against Pipe on this copy
    pub instant_move: bool,          // same drive + move = rename only
    pub network: bool,
    pub can_tune: bool,              // measure once and switch to more files in flight when that is faster
    pub clone: bool,                 // the same ReFS volume: Windows' own copy shares the blocks instead of writing them
    pub one_usb: bool,               // one SSD in a USB case at both ends: it stalls when asked to read and write much at once
    pub kind: String,
    pub tuned: bool,
    pub tune_note: Option<String>,
    pub max_workers: i32,            // the most files in flight tuning may go to
    pub small_batch: bool,           // a hard disk at either end: small files in read-then-write batches
    pub tune_seconds: f64,           // length of one trial stretch
    pub tune_margin: f64,            // how much faster another way must be to take over
}

impl Default for Plan {
    fn default() -> Self {
        Plan {
            workers: 1,
            big_slots: 1,
            big_threshold: 32 << 20,
            pipeline: false,
            pipe_threshold: 32 << 20,
            pipe: PipeTuning::default(),
            pipe_ways: Vec::new(),
            instant_move: false,
            network: false,
            can_tune: false,
            clone: false,
            one_usb: false,
            kind: "Disk".into(),
            tuned: false,
            tune_note: None,
            max_workers: 32,
            small_batch: false,
            tune_seconds: 0.75,
            tune_margin: 1.03,
        }
    }
}

impl Plan {
    /// Short badge: "Adaptive · 1 stream", "Adaptive · 8 streams", "Adaptive · instant move".
    pub fn badge(&self) -> String {
        format!("{} · {}", self.mode(), self.detail())
    }
    pub fn mode(&self) -> &'static str {
        "Adaptive"
    }
    pub fn detail(&self) -> String {
        if self.instant_move {
            return "instant move".into();
        }
        let note = match (&self.tune_note, self.tuned) {
            (Some(n), true) => format!(" · {n}"),
            _ => String::new(),
        };
        format!("{}{}{}", self.workers, if self.workers == 1 { " stream" } else { " streams" }, note)
    }

    /// What the badge stands for (its tooltip).
    pub fn why(&self) -> String {
        if self.instant_move {
            return "Same drive: the files are renamed in place, nothing is copied.".into();
        }
        let mut s = format!("{}: ", self.kind);
        if self.clone {
            return s + "Windows' own copy, which shares the original's blocks on ReFS (Windows 11 24H2 and later): nothing is written twice.";
        }
        if self.network {
            s += &format!(
                "{} files in flight hide the network's round trips{}",
                self.workers,
                if self.can_tune { "; it measures and adds more when that is faster." } else { "." }
            );
        } else if self.one_usb {
            s += "one file at a time, read and written at once in 8 MB pieces - a USB case stalls when much more is in flight.";
        } else if self.workers <= 1 {
            s += "one file at a time, in order - no seeking back and forth.";
        } else {
            s += &format!("{} small files at once.", self.workers);
        }
        if self.pipeline {
            s += " Big files are read and written past the Windows cache";
            s += if !self.pipe_ways.is_empty() { "; the first big file tries a few ways and the fastest copies the rest." } else { "." };
        }
        s
    }

    pub fn choose(spec: &JobSpec, src: Option<&VolumeInfo>, dst: Option<&VolumeInfo>, scan: Option<&ScanResult>) -> Plan {
        let mut p = Plan::default();
        let same_volume = matches!((src, dst), (Some(s), Some(d)) if s.id == d.id);
        if spec.op == OpKind::Move && same_volume && spec.explicit_items.is_none() {
            p.instant_move = true;
            p.kind = "Same drive".into();
            return p;
        }
        let files = scan.map_or(0, |s| s.files.len() as i64);
        let median = scan.map_or(0, |s| s.median_size);
        let largest = scan.and_then(|s| s.largest.first()).map_or(0, |l| l.1);

        if src.is_some_and(|s| s.is_network()) || dst.is_some_and(|d| d.is_network()) {
            // Latency is the enemy: several files in flight hide the round trips,
            // and Windows' own copy pipelines the writes of each big file.
            p.network = true;
            p.kind = "Network".into();
            // measured on the NAS over Tailscale (600 small files): 8 in flight = 30 files/s, 16 = 42, 32 = 52.
            // The NAS spends a fixed time per file, so many in flight is what helps; tuning can go on to 32.
            p.workers = if files <= 1 {
                1
            } else if files > 64 && median < (8 << 20) {
                16
            } else if files > 16 && median < (8 << 20) {
                8
            } else {
                4
            };
            p.big_slots = 1;
            p.can_tune = files > 1;
            p.max_workers = 64; // a far NAS spends a fixed time per file: tuning may go on to 64 in flight
            return p;
        }
        // the same ReFS volume (a Dev Drive): Windows' own copy clones the blocks, so a file of any size takes a moment
        // (Windows 11 24H2 on; before that it is an ordinary copy). Nothing is faster than not writing at all.
        if same_volume && src.unwrap().can_clone() {
            p.clone = true;
            p.kind = "Same ReFS drive".into();
            p.workers = if files <= 1 {
                1
            } else if files > 32 && median < (1 << 20) {
                8
            } else {
                4
            };
            p.can_tune = files > 1;
            return p;
        }
        let mut slow = DeviceKind::Unknown;
        if let Some(d) = dst.filter(|d| d.is_slow_seek()) {
            slow = d.kind;
        } else if let Some(s) = src.filter(|s| s.is_slow_seek()) {
            slow = s.kind;
        }
        // local drives: big files go through the pipeline (reader + writer at once, big aligned pieces)
        p.pipeline = true;
        p.pipe.align = 4096.max(src.map_or(512, |s| s.sector).max(dst.map_or(512, |d| d.sector)));
        let one_disk = volume_probe::same_disk(src, dst);
        // one SSD in a USB case at both ends: it reads and writes at once, but never much in flight. With more out (8 small
        // files at once, 4 reads of 8 MB beside a write, or one read and one write of 16 MB) a 10 Gbps Netac NVMe case
        // stopped answering until Windows reset it (event 129 - 19 times in six minutes): a 2 GB copy within it took
        // 6-25 s, 1,000 small files 36-121 s, Windows' copy 4.2 s / 4 s. Into it or out of it alone it keeps up with 8 in
        // flight. 2 GB within it, three runs each, time against Windows': 8 MB pieces with one read out +34% (723 MB/s),
        // 4 MB +30%, 2 MB +27%, 1 MB with two out +3%; each batch read, then written: -9% (456 MB/s). No trials: a
        // bigger piece is what stalls it. Small files: batches read, then written through the cache, one at a time (five
        // runs each: 2 GB +32%, 2,000 small files +65%, a mixed folder +48%, no reset).
        if slow == DeviceKind::Unknown && one_disk && src.unwrap().on_usb {
            p.one_usb = true;
            p.kind = "USB".into();
            p.pipe.chunk = 8 << 20;
            p.pipe.slots = 4;
            p.pipe.read_depth = 1;
            return aligned(p.with_small_batch());
        }
        if slow != DeviceKind::Unknown {
            // a spinning disk or USB stick hates seeking: one file at a time, in order
            p.kind = match slow {
                DeviceKind::Usb => "USB",
                DeviceKind::Optical => "Disc",
                _ => "HDD",
            }
            .into();
            p.workers = 1;
            p.big_slots = 1;
            let src_slow = src.is_some_and(|s| s.is_slow_seek());
            let dst_slow = dst.is_some_and(|d| d.is_slow_seek());
            // big pieces, several reads queued at the disk: other programs on it can't send the heads away between
            // them (measured with a torrent-like load: 32 MB x 4 = 157 MB/s, 16 MB x 4 = 152, Explorer 146).
            p.pipe.chunk = 32 << 20;
            p.pipe.slots = 8;
            // a hard disk read into a fast drive: small reads queued keep it streaming (1 MB x 32 measured ahead of
            // 32 MB x 8 and of Explorer, two rounds out of two), but only 4 out at the disk at once: with all 32 out,
            // some copies took 0.3-0.4 s longer (the disk likely serves them out of order and loses a turn). 1 GB from
            // two hard disks about 80% full to the NVMe, five runs each, time to the disk against Windows' copy:
            // 32 out -1% to -4%; 8 out -1% / -1%; 4 out +1% / -1% and -1% / -1%, every run within 0.16 s; 2 out -1% /
            // -1%; 2 MB x 4 -2% / -2%; 4 MB x 4 -1% / -2%; read through the Windows cache -1% / -1%; Windows' own
            // engine run by Choppy -1% / +1%. 2 GB: 4 out 0% / -1%, 32 out -2% / -1%.
            if src_slow && !dst_slow {
                p.pipe.chunk = 1 << 20;
                p.pipe.slots = 32;
                p.pipe.read_depth = 4;
            }
            // one disk at both ends: read a batch, then write it - not both at once (the heads would go back and forth every piece)
            p.pipe.phased = one_disk;
            // writing to a hard disk: pieces already read go out together (up to 128 MB), no gap for the platter to pass
            if dst_slow && !one_disk {
                p.pipe.write_batch = 4;
            }
            // what goes to a hard disk through the Windows cache goes in 4 MB writes: Windows holds a big write back
            // sooner while much is waiting in memory (1 GB NVMe to hard disk, done after 4.2 s with up to 128 MB per
            // write, 2.5 s with 4 MB - Explorer 2.1 s; to the disk the same; within one hard disk done 10.1 s -> 8.1 s)
            if dst_slow {
                p.pipe.cached_write_chunk = 4 << 20;
            }
            // no size reserved up front on a hard disk: the reservation can land the file on a slower part of the
            // platter (NVMe to hard disk: 199-206 MB/s without it, 169-170 with it; again in v1.40, 4 GB: 179 / 170).
            // A 512 MB file lands on a fast part more often with it (as Explorer's copy, which reserves) - both ways
            // tie with Explorer over ten runs, so it stays off in the pipeline (the last file, through the cache, has its
            // size reserved as Explorer's copy does - CopyJob.CopyBig)
            if dst_slow {
                p.pipe.prealloc = false;
            }
            // two hard disks: 512 MB read ahead in 2 MB reads, joined into 64 MB writes, so a write doesn't stop the
            // reads and the source streams from the first second (4 GB on disk: 32 MB x 8 = 163 MB/s, 32 MB x 16 =
            // 164-171, 2 MB x 256 = 175-179, Explorer 155-159). Only 4 reads out at the disk at once: a deep queue
            // starts slower (first 1.5 s: 4 out = 191-217 MB/s, 256 out = 119-160) and gains nothing after (shown
            // 173-183 vs 167-178; Explorer 176-178). Once the source is fully read, what the buffers still hold goes
            // through the cache: the copy is done then, and the disk never waits on the cache while it reads.
            // No trials here: two disks swing more from one second to the next than the ways differ.
            if src_slow && dst_slow && !one_disk {
                p.pipe.chunk = 2 << 20;
                p.pipe.slots = 256;
                p.pipe.read_depth = 4;
                p.pipe.write_batch = 32;
                p.pipe.tail_after_read = true;
                return aligned(p.with_small_batch());
            }
            // one hard disk at both ends: the whole file (up to 512 MB at a time) is read before any of it is written,
            // in 1 MB reads with 32 at the disk at once, then written in 64 MB writes into room reserved up front (a
            // reservation finds one stretch of free space; growing piece by piece fills the holes on a full disk).
            // Measured, 512 MB within one hard disk, ten runs each: 256 MB batches of 32 MB 84-86 MB/s, Explorer
            // 90-92, this 87-89 - and up to 98 when the reservation lands on a fast part of the platter; 2 GB: 89 / 91.
            // The end goes through the cache only once its batch is fully read: during the reads it would compete
            // with them for the heads (512 MB: 6.3 s to the disk then, 5.9 without). No trials: one batch is longer than a trial.
            if one_disk && slow == DeviceKind::Hdd {
                p.pipe.chunk = 1 << 20;
                p.pipe.slots = 512;
                p.pipe.read_depth = 32;
                p.pipe.write_step = 64 << 20;
                p.pipe.prealloc = true;
                p.pipe.tail_after_read = true;
                return aligned(p.with_small_batch());
            }
            // all of the above is a first guess: the first big file measures these ways against it and keeps the fastest
            if one_disk {
                p.pipe_ways.push(way(32, 16, true, 1));
                p.pipe_ways.push(way(16, 8, true, 1));
            } else if src_slow && !dst_slow {
                // a hard disk into a fast drive: 1 MB x 32 won every trial here (1 GB, eight copies: 201-207 MB/s, 4 MB x 16
                // 171-179, 8 MB x 8 184), and a trial of a slower way only costs time (4-5% of a 1 GB copy) - none
            } else {
                // a fast drive into a hard disk: the last file skips the pipeline and its trials, whatever its size - it goes
                // through the Windows cache as Explorer's copy sends it (see CopyJob.CopyBig: 2 GB, five runs each to two hard
                // disks, the pipeline with trials took 8% / 22% longer than Windows' copy, the cache 5% less / 1% more;
                // 4 GB: the pipeline 10% less / 20% more, the cache 0% / 2% more)
                p.pipe_ways.push(way(1, 32, false, 32)); // small reads, joined into 32 MB writes
                p.pipe_ways.push(way(8, 8, false, 4));
            }
            // a hard disk's speed swings over short stretches: longer trials, and another way must win by 8%
            p.tune_seconds = 2.0;
            p.tune_margin = 1.08;
            return aligned(p.with_small_batch());
        }
        p.kind = if dst.is_some_and(|d| d.kind == DeviceKind::Ssd) || src.is_some_and(|s| s.kind == DeviceKind::Ssd) {
            "SSD"
        } else {
            "Disk"
        }
        .into();
        // many small files between two SSDs inside the PC: most of each file's time is Windows' own work around it
        // (making it, the antivirus' look at it), not the drive, so many at once pay - 20,000 files of 1-64 KB, the NVMe
        // to itself: 8 at once 6.3 s, 16 5.0 s, 32 4.5 s, 64 4.4 s. A drive in a USB case stays at 8 (it stalls with
        // much more in flight - see one_usb above); tuning may go on to 64.
        let internal_ssd = |v: Option<&VolumeInfo>| v.is_some_and(|v| matches!(v.kind, DeviceKind::Ssd | DeviceKind::Ram) && !v.on_usb);
        p.workers = if files <= 1 {
            1
        } else if files > 32 && median < (1 << 20) {
            if internal_ssd(src) && internal_ssd(dst) {
                p.max_workers = 64;
                32
            } else {
                8
            }
        } else {
            4
        };
        if files <= 3 && largest >= (1 << 30) {
            p.workers = 1; // one huge file: the pipeline has the drive to itself
        }
        p.big_slots = 1;
        p.pipe.chunk = 8 << 20;
        p.pipe.slots = 4;
        p.pipe_ways.push(way(4, 8, false, 1));
        p.pipe_ways.push(way(16, 4, false, 1));
        p.pipe_ways.push(way(32, 4, false, 1));
        p.can_tune = files > 1;
        aligned(p)
    }

    // many small files: read in batches in disk order, then written through the cache - one worker
    // (1,000 files: two hard disks 90-93 MB/s, one disk 67-69; one file at a time 73-76 / 58-60; Explorer 64-67 / 52-55)
    fn with_small_batch(mut self) -> Plan {
        self.small_batch = true;
        self.can_tune = false;
        self.max_workers = 4;
        self
    }
}

fn way(mb: i32, slots: i32, phased: bool, batch: i32) -> PipeTuning {
    PipeTuning { chunk: mb << 20, slots, phased, write_batch: batch, ..Default::default() }
}

fn aligned(mut p: Plan) -> Plan {
    let (align, prealloc, cached) = (p.pipe.align, p.pipe.prealloc, p.pipe.cached_write_chunk);
    for w in p.pipe_ways.iter_mut() {
        w.align = align;
        w.prealloc = prealloc;
        w.cached_write_chunk = cached;
    }
    p
}
