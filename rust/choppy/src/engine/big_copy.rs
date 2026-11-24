//! Big files between local drives (BigCopy.cs): RingReader, PipeTuning, PipeTuner, BigResult and BigCopy (the
//! pipeline copy, and the small-file batch's OpenSmall / ReadSmall / WriteSmall).

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::netfmt;
use super::xxhash64::XxHash64;
use super::{lock, panic_message, wait};
use crate::sys::{self, Handle, PendingRead, Slot};
use crate::wstr::WString;

/// What C# threw when VirtualAlloc gave no memory (new OutOfMemoryException()): caught where a copy catches errors.
pub const OUT_OF_MEMORY: &str = "Insufficient memory to continue the execution of the program.";

// ================= RingReader =================

/// Reads a file front to back on its own thread into a ring of large aligned buffers, so the next piece is already
/// coming off the disk while the caller writes or hashes the last one. On an overlapped handle every free buffer has a
/// read waiting at the disk: the disk always has the next piece of this file queued, so other programs reading the
/// same disk (a torrent client, an indexer) can't slip in between and send the heads away.
///
/// The buffers are `sys::Slot`s: the reader thread holds the ones it reads into, the caller the ones it took with
/// `next` (+ `gather`) until `release` hands them back. The two queues below are C#'s two semaphores: `free` holds
/// the buffers the reader may fill (in ring order), `full` the pieces read and not taken yet (in file order).
pub struct RingReader {
    shared: Arc<Shared>,
    th: Option<JoinHandle<()>>,
    chunk: i32,
    slots: i32,
    pieces: i64,
    phased: bool,
    taken: i64,
}

struct Shared {
    st: Mutex<RingState>,
    free_cv: Condvar,
    full_cv: Condvar,
    stop: AtomicBool,
    error: AtomicU32,     // read error (0 = none)
    all_read: AtomicBool, // the last piece is in memory: nothing is left to read
}

struct RingState {
    free: VecDeque<Slot>,
    full: VecDeque<Posted>,
    reader_ended: bool,
    reader_panic: Option<String>,
}

/// A piece the reader is done with: its bytes (n of them; 0 = a read error, the slot may be still out at the disk).
struct Posted {
    slot: Option<Slot>,
    n: i32,
}

/// The pieces taken with `next` (+ `gather`): one after the other in the file and in memory.
pub struct Piece {
    slots: Vec<Slot>,
    pub n: i32,
}

impl Piece {
    /// Every byte of the pieces' buffers (n of them hold the file; the rest is room for a sector's padding).
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        sys::joined_mut(&mut self.slots).expect("joined pieces follow each other in the ring")
    }
    pub fn bytes(&self) -> &[u8] {
        sys::joined(&self.slots).expect("joined pieces follow each other in the ring")
    }
}

/// What the reader thread works with.
struct Reader {
    shared: Arc<Shared>,
    h: Arc<Handle>,
    chunk: i32,
    slots: i32,
    depth: i32,
    size: i64,
    pieces: i64,
    start: i64,
    overlapped: bool,
    phased: bool, // one disk at both ends: read a batch, then write it (see read_phased)
    hash: Option<Arc<Mutex<XxHash64>>>,
}

impl RingReader {
    /// size = where to stop reading (the file's size, or the end of a stretch); start = where to begin (overlapped
    /// handles only, a sector multiple). overlapped = the handle was opened with FILE_FLAG_OVERLAPPED.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        h: Arc<Handle>,
        size: i64,
        chunk: i32,
        slots: i32,
        overlapped: bool,
        hash: Option<Arc<Mutex<XxHash64>>>,
        phased: bool,
        start: i64,
        depth: i32,
    ) -> RingReader {
        let depth = if depth > 0 { depth.min(slots) } else { slots };
        let phased = phased && overlapped;
        let start = if overlapped { start } else { 0 };
        let pieces = if size - start <= 0 { 0 } else { (size - start + chunk as i64 - 1) / chunk as i64 };
        let Some(ring) = sys::ring(chunk as usize, slots as usize, overlapped) else { panic!("{OUT_OF_MEMORY}") };
        let shared = Arc::new(Shared {
            st: Mutex::new(RingState {
                free: ring.into_iter().collect(),
                full: VecDeque::new(),
                reader_ended: false,
                reader_panic: None,
            }),
            free_cv: Condvar::new(),
            full_cv: Condvar::new(),
            stop: AtomicBool::new(false),
            error: AtomicU32::new(0),
            all_read: AtomicBool::new(false),
        });
        let rd = Reader { shared: shared.clone(), h, chunk, slots, depth, size, pieces, start, overlapped, phased, hash };
        let th = std::thread::Builder::new()
            .name("read ahead".into())
            .spawn(move || {
                sys::set_thread_priority_above_normal();
                let r = catch_unwind(AssertUnwindSafe(|| if rd.overlapped { rd.read_queued() } else { rd.read_in_order() }));
                // whatever happened, the caller never waits for a piece that won't come
                let mut st = lock(&rd.shared.st);
                st.reader_ended = true;
                if let Err(e) = r {
                    st.reader_panic = Some(panic_message(&*e));
                }
                rd.shared.full_cv.notify_all();
            })
            .unwrap_or_else(|_| panic!("{OUT_OF_MEMORY}"));
        RingReader { shared, th: Some(th), chunk, slots, pieces, phased, taken: 0 }
    }

    pub fn slots(&self) -> i32 {
        self.slots
    }

    /// The read error (0 = none).
    pub fn error(&self) -> u32 {
        self.shared.error.load(Ordering::SeqCst)
    }

    /// The last piece is in memory: nothing is left to read.
    pub fn all_read(&self) -> bool {
        self.shared.all_read.load(Ordering::SeqCst)
    }

    /// Waits for the next piece the reader posted (`block`), or takes it only if it is there. None = none (a reader
    /// that ended without posting it raises its panic here, or counts as a read error).
    fn take_posted(&self, block: bool, peek_error_stays: bool) -> Option<Posted> {
        let mut st = lock(&self.shared.st);
        loop {
            if let Some(front) = st.full.front() {
                if peek_error_stays && front.n <= 0 {
                    return None; // a read error: left for next to report
                }
                return st.full.pop_front();
            }
            if st.reader_ended {
                if let Some(msg) = st.reader_panic.take() {
                    drop(st);
                    panic!("{msg}");
                }
                if self.shared.error.load(Ordering::SeqCst) == 0 {
                    self.shared.error.store(sys::ERROR_HANDLE_EOF, Ordering::SeqCst);
                }
                return None;
            }
            if !block {
                return None;
            }
            st = wait(&self.shared.full_cv, st);
        }
    }

    /// The next piece in file order. None = the end (or a read error: see `error`).
    pub fn next(&mut self) -> Option<Piece> {
        if self.taken >= self.pieces {
            return None;
        }
        let p = self.take_posted(true, false)?;
        if p.n <= 0 {
            return None;
        }
        Some(Piece { slots: vec![p.slot.expect("a piece read without its buffer")], n: p.n })
    }

    /// The pieces right after the one from `next` that are already read (and sit right after it in memory) join it,
    /// up to `more`: one bigger write keeps a hard disk streaming. n grows by their bytes; returns how many joined.
    pub fn gather(&mut self, p: &mut Piece, more: i32) -> i32 {
        let mut got = 0;
        let mut k = self.taken;
        let chunk = self.chunk as i64;
        // phased: wait for the rest of the batch (it is being read right now); else take only what is there
        while got < more && p.n as i64 == (got as i64 + 1) * chunk && k + 1 < self.pieces && (k + 1) % self.slots as i64 != 0 {
            let Some(next) = self.take_posted(self.phased, true) else { break };
            k += 1;
            got += 1;
            p.n += next.n;
            p.slots.push(next.slot.expect("a piece read without its buffer"));
        }
        got
    }

    /// Hands the pieces from `next` (+ `gather`) back to the reader.
    pub fn release(&mut self, p: Piece) {
        self.taken += p.slots.len() as i64;
        let mut st = lock(&self.shared.st);
        st.free.extend(p.slots);
        self.shared.free_cv.notify_all();
    }
}

impl Drop for RingReader {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        {
            let _st = lock(&self.shared.st);
            self.shared.free_cv.notify_all();
        }
        if let Some(th) = self.th.take() {
            let _ = th.join();
        }
        // the buffers and their events go with the last Slot (a read still out was waited for by its PendingRead)
    }
}

impl Reader {
    fn stopped(&self) -> bool {
        self.shared.stop.load(Ordering::SeqCst)
    }
    fn error(&self) -> u32 {
        self.shared.error.load(Ordering::SeqCst)
    }
    fn set_error(&self, e: u32) {
        self.shared.error.store(e, Ordering::SeqCst);
    }

    /// free.Wait(): the next buffer the writer gave back. None = stopped.
    fn wait_free(&self) -> Option<Slot> {
        let mut st = lock(&self.shared.st);
        loop {
            if self.stopped() {
                return None;
            }
            if let Some(s) = st.free.pop_front() {
                return Some(s);
            }
            st = wait(&self.shared.free_cv, st);
        }
    }

    /// free.Wait(0): a buffer if one is free now.
    fn try_free(&self) -> Option<Slot> {
        lock(&self.shared.st).free.pop_front()
    }

    fn post(&self, slot: Option<Slot>, n: i32) {
        if n > 0
            && let (Some(h), Some(s)) = (&self.hash, &slot)
        {
            lock(h).update(&s.bytes()[..n as usize]);
        }
        let mut st = lock(&self.shared.st);
        st.full.push_back(Posted { slot, n });
        self.shared.full_cv.notify_all();
    }

    fn want(&self, piece: i64) -> i64 {
        (self.chunk as i64).min(self.size - self.start - piece * self.chunk as i64)
    }

    // plain handle: one read after the other on this thread
    fn read_in_order(&self) {
        let mut k = 0i64;
        while k < self.pieces && !self.stopped() {
            let Some(mut slot) = self.wait_free() else { return };
            debug_assert_eq!(slot.index() as i64, k % self.slots as i64);
            let want = self.want(k) as usize;
            let mut n = 0usize;
            while n < want {
                match sys::read(&self.h, &mut slot.bytes_mut()[n..want]) {
                    Err(e) => {
                        self.set_error(e);
                        break;
                    }
                    Ok(0) => {
                        self.set_error(sys::ERROR_HANDLE_EOF); // shorter than it said
                        break;
                    }
                    Ok(m) => n += m as usize,
                }
            }
            if self.error() != 0 {
                n = 0;
            }
            if n > 0 && k + 1 == self.pieces {
                self.shared.all_read.store(true, Ordering::SeqCst);
            }
            self.post(Some(slot), n as i32);
            if n == 0 {
                return;
            }
            k += 1;
        }
    }

    // overlapped handle: as many reads out at once as there are free buffers
    fn read_queued(&self) {
        if self.phased {
            self.read_phased();
            return;
        }
        let (mut issued, mut done) = (0i64, 0i64);
        let mut out: VecDeque<PendingRead> = VecDeque::new(); // the reads for pieces done..issued
        while done < self.pieces && !self.stopped() {
            while issued < self.pieces && issued - done < self.depth as i64 {
                let slot = if issued == done {
                    self.wait_free() // nothing out: wait for a buffer
                } else {
                    self.try_free() // reads are out: go collect one
                };
                let Some(slot) = slot else { break };
                if self.stopped() {
                    break;
                }
                match self.issue(issued, slot, 0, self.chunk) {
                    Ok(p) => out.push_back(p),
                    Err(_) => {
                        self.post(None, 0);
                        return; // `out` waits for the reads still out as it goes (C#'s Drain)
                    }
                }
                issued += 1;
            }
            if self.stopped() {
                break;
            }
            let Some(p) = out.pop_front() else { break };
            let (slot, n) = self.collect_whole(done, p);
            if n > 0 && done + 1 == self.pieces {
                self.shared.all_read.store(true, Ordering::SeqCst);
            }
            self.post(Some(slot), n);
            done += 1;
            if n <= 0 {
                return;
            }
        }
    }

    // One disk at both ends: a whole batch (every buffer) is read with no write in between, then written with no
    // read in between - the heads go between the two places once per batch instead of once per piece. At most
    // depth reads are out at the disk at once (many small reads queued stream best: 1 MB x 32 = 185 MB/s on a
    // hard disk here, 32 MB x 8 = 174).
    fn read_phased(&self) {
        let mut done = 0i64;
        while done < self.pieces && !self.stopped() {
            let mut batch: VecDeque<Slot> = VecDeque::with_capacity(self.slots as usize);
            for _ in 0..self.slots {
                match self.wait_free() {
                    Some(s) => batch.push_back(s), // the writer gave every buffer back
                    None => return,
                }
            }
            let end = self.pieces.min(done + self.slots as i64);
            let mut issued = done;
            let mut out: VecDeque<PendingRead> = VecDeque::new();
            while done < end {
                while issued < end && issued - done < self.depth as i64 {
                    let slot = batch.pop_front().expect("a buffer for every piece of the batch");
                    match self.issue(issued, slot, 0, self.chunk) {
                        Ok(p) => out.push_back(p),
                        Err(_) => {
                            drop(out); // waits for the reads still out (C#: Drain, then Post)
                            self.post(None, 0);
                            return;
                        }
                    }
                    issued += 1;
                }
                let p = out.pop_front().expect("a read out for the next piece");
                let (slot, n) = self.collect_whole(done, p);
                if n > 0 && done + 1 == self.pieces {
                    self.shared.all_read.store(true, Ordering::SeqCst);
                }
                self.post(Some(slot), n);
                if n <= 0 {
                    return;
                }
                done += 1;
            }
        }
    }

    /// A piece read in full (a read can come back short over a network: the rest is read too). 0 = error.
    fn collect_whole(&self, piece: i64, p: PendingRead) -> (Slot, i32) {
        let (mut slot, r) = p.wait();
        let mut n = self.collect(r);
        let want = self.want(piece);
        while n > 0 && (n as i64) < want && self.error() == 0 {
            match self.issue(piece, slot, n, self.chunk - n) {
                Err(s) => {
                    slot = s;
                    break;
                }
                Ok(p) => {
                    let (s, r) = p.wait();
                    slot = s;
                    let m = self.collect(r);
                    if m <= 0 {
                        if self.error() == 0 {
                            self.set_error(sys::ERROR_HANDLE_EOF);
                        }
                        break;
                    }
                    n += m;
                }
            }
        }
        if n <= 0 && self.error() == 0 {
            self.set_error(sys::ERROR_HANDLE_EOF);
        }
        (slot, if self.error() != 0 { 0 } else { n })
    }

    /// ReadFile at the piece's place (+ offset_in_piece) into its slot. Err = it failed at once (Error set).
    fn issue(&self, piece: i64, slot: Slot, offset_in_piece: i32, bytes: i32) -> Result<PendingRead, Slot> {
        debug_assert_eq!(slot.index() as i64, piece % self.slots as i64);
        let off = self.start + piece * self.chunk as i64 + offset_in_piece as i64;
        match sys::read_at(&self.h, slot, offset_in_piece as usize, bytes as usize, off) {
            Ok(p) => Ok(p),
            Err((slot, e)) => {
                self.set_error(e);
                Err(slot)
            }
        }
    }

    /// GetOverlappedResult's answer: the bytes, or 0 with the first error kept (the end of the file isn't one).
    fn collect(&self, r: Result<u32, u32>) -> i32 {
        match r {
            Ok(n) => n as i32,
            Err(e) => {
                if e != sys::ERROR_HANDLE_EOF && self.error() == 0 {
                    self.set_error(e);
                }
                0
            }
        }
    }
}

// ================= PipeTuning, PipeTuner =================

/// How the pipeline runs for one job (picked by Plan from the drives on both ends).
#[derive(Clone, Debug, PartialEq)]
pub struct PipeTuning {
    pub chunk: i32,              // bytes per read / write
    pub slots: i32,              // pieces in flight between the reader and the writer
    pub align: i32,              // sector multiple for unbuffered I/O
    pub write_batch: i32,        // pieces already read that may go out as one write (a hard disk streams without gaps)
    pub phased: bool,            // one hard disk at both ends: read all buffers, then write them all
    pub prealloc: bool,          // reserve the whole size up front
    pub cached_write_chunk: i32, // bytes per write through the Windows cache (0 = a whole piece)
    pub tail_after_read: bool,   // the cached tail starts once the source is fully read: what the buffers still hold goes through the cache
    pub read_depth: i32,         // reads out at the disk at once (0 = one per free buffer): the buffers can hold far more than that
    pub write_step: i32,         // phased: bytes per write of a batch (0 = a piece at a time)
}

impl Default for PipeTuning {
    fn default() -> Self {
        PipeTuning {
            chunk: 8 << 20,
            slots: 4,
            align: 4096,
            write_batch: 1,
            phased: false,
            prealloc: true,
            cached_write_chunk: 0,
            tail_after_read: false,
            read_depth: 0,
            write_step: 0,
        }
    }
}

const WARM_BYTES: i64 = 64 << 20;
const UNIT: i64 = 32 << 20; // stretches end on whole pieces of every way (pieces are at most 32 MB)

/// Finds the fastest way for this copy while it runs. No fixed rule fits every PC, disk and cable, so the first big
/// file is copied in a few short stretches (under a second each), each one another way - piece size, pieces in flight,
/// reading and writing at once or one after the other - and the fastest way copies the rest, and the next big files.
pub struct PipeTuner {
    ways: Vec<PipeTuning>,
    rate: Vec<f64>, // bytes/s per way (0 = not measured)
    state: i32,     // -1 = warming up, 0..n-1 = trying way i, n = settled
    trial_bytes: i64,
    best: usize,      // which way is Best (C# compared the objects themselves)
    pub changed: bool, // settled on another way than the starting one
    trial_seconds: f64,
    margin: f64, // a hard disk measures steadily only over longer stretches
}

impl PipeTuner {
    pub fn new(start: PipeTuning, others: Vec<PipeTuning>, trial_seconds: f64, margin: f64) -> PipeTuner {
        let mut ways = vec![start];
        ways.extend(others);
        let n = ways.len();
        PipeTuner { ways, rate: vec![0.0; n], state: -1, trial_bytes: 256 << 20, best: 0, changed: false, trial_seconds, margin }
    }

    pub fn best(&self) -> &PipeTuning {
        &self.ways[self.best]
    }

    pub fn settled(&self) -> bool {
        self.state >= self.ways.len() as i32
    }

    /// The way for the stretch from done on, and where that stretch ends. keep_free = the file's end that must stay out
    /// of trials. Returns (way, end, measured).
    pub fn next(&self, done: i64, size: i64, keep_free: i64) -> (PipeTuning, i64, bool) {
        if self.settled() || self.ways.len() < 2 {
            return (self.best().clone(), size, false);
        }
        let len = if self.state < 0 { WARM_BYTES } else { self.trial_bytes };
        if done + len + keep_free + UNIT > size {
            return (self.best().clone(), size, false); // no room in this file: the next big file tries
        }
        let way = if self.state < 0 { self.best().clone() } else { self.ways[self.state as usize].clone() };
        (way, done + len, self.state >= 0)
    }

    pub fn report(&mut self, bytes: i64, seconds: f64, measured: bool) {
        if self.settled() || bytes <= 0 {
            return;
        }
        let r = bytes as f64 / seconds.max(0.001);
        if self.state < 0 {
            // the warm-up only sizes the trials: any time is good enough (a fast drive does 64 MB in a few hundredths)
            // a trial of trialSeconds at the speed seen so far, whole 32 MB units, 64 MB .. 512 MB
            let b = (r * self.trial_seconds) as i64 / UNIT * UNIT;
            self.trial_bytes = (2 * UNIT).max((16 * UNIT).min(b));
            self.state = 0;
            return;
        }
        if !measured || seconds < 0.1 {
            return; // too short to tell: this way is tried again on the next stretch
        }
        self.rate[self.state as usize] = r;
        self.state += 1;
        if (self.state as usize) < self.ways.len() {
            return;
        }
        // another way must win clearly (3% on SSDs, 8% on hard disks), or the starting one stays (no flapping on noise)
        let mut best = 0;
        for i in 1..self.ways.len() {
            if self.rate[i] > self.rate[best] {
                best = i;
            }
        }
        if best != 0 && self.rate[best] < self.rate[0] * self.margin {
            best = 0;
        }
        self.best = best;
        self.changed = best != 0;
    }

    /// What was measured, for the log and tests: "32x8 phased 150 MB/s | 16x8 phased 161 MB/s <-".
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        for i in 0..self.ways.len() {
            if self.rate[i] > 0.0 {
                parts.push(format!(
                    "{} {} MB/s{}",
                    Self::describe(&self.ways[i]),
                    netfmt::f(self.rate[i] / 1048576.0, 0),
                    if i == self.best && self.settled() { " <-" } else { "" }
                ));
            }
        }
        parts.join(" | ")
    }

    /// "32x8 phased join4 w64": piece MB x pieces, phased, joined writes, write step MB.
    pub fn describe(t: &PipeTuning) -> String {
        format!(
            "{}x{}{}{}{}",
            t.chunk >> 20,
            t.slots,
            if t.phased { " phased" } else { "" },
            if t.write_batch > 1 { format!(" join{}", t.write_batch) } else { String::new() },
            if t.write_step > 0 { format!(" w{}", t.write_step >> 20) } else { String::new() }
        )
    }
}

// ================= BigCopy =================

#[derive(Default, Debug)]
pub struct BigResult {
    pub error: u32,      // 0 = done; NOT_SUPPORTED = use CopyFileEx for this file
    pub bytes: i64,      // bytes copied
    pub hash: u64,       // xxHash64 of the source as it was read (when asked)
    pub renamed: bool,   // already under its final name
    pub times_set: bool, // dates + attributes are on the copy (else they are set by name afterwards)
}

/// BigCopy.NotSupported (C#'s -1): this file can't go through the pipeline - CopyFileEx takes it.
pub const NOT_SUPPORTED: u32 = u32::MAX;

const KEEP_ATTRS: u32 = sys::FILE_ATTRIBUTE_READONLY
    | sys::FILE_ATTRIBUTE_HIDDEN
    | sys::FILE_ATTRIBUTE_SYSTEM
    | sys::FILE_ATTRIBUTE_ARCHIVE
    | sys::FILE_ATTRIBUTE_NOT_CONTENT_INDEXED;

/// Files this path never takes: encrypted, compressed, sparse, cloud placeholders, links.
pub const SKIP_ATTRS: u32 = sys::FILE_ATTRIBUTE_ENCRYPTED
    | sys::FILE_ATTRIBUTE_COMPRESSED
    | sys::FILE_ATTRIBUTE_SPARSE_FILE
    | sys::FILE_ATTRIBUTE_OFFLINE
    | sys::FILE_ATTRIBUTE_RECALL_ON_OPEN
    | sys::FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS
    | sys::FILE_ATTRIBUTE_REPARSE_POINT;

/// Big files between local drives: the source is read and the copy is written at the same time (reader thread + writer
/// thread, large aligned buffers, no Windows cache on either side). CopyFileEx with no buffering reads a piece, writes
/// it, then reads the next: two disks take turns and each one idles half the time. Keeps what CopyFileEx keeps:
/// created / modified dates, attributes, extra data streams (Zone.Identifier).
///
/// Copies src to dst (a new name: never replaces anything). progress(bytes written so far) returns false to stop.
/// rename_to: give the copy this final name through its own handle once every byte is written.
/// cached_tail: the last bytes of the file go through the Windows cache (like Explorer's copy), so the copy is done as
/// soon as they are in memory and Windows writes them out right after. 0 = all straight to the disk.
/// max_write: the most bytes one write may take right now (a speed limit writes in small steps, so it holds steady
/// instead of in bursts); None = no cap.
#[allow(clippy::too_many_arguments)]
pub fn copy(
    src: &WString,
    dst: &WString,
    ctime: i64,
    mtime: i64,
    attrs: u32,
    tune: &PipeTuning,
    hash: bool,
    flush: bool,
    rename_to: Option<&WString>,
    cached_tail: i64,
    progress: &mut dyn FnMut(i64) -> bool,
    mut tuner: Option<&mut PipeTuner>,
    max_write: Option<&dyn Fn() -> i32>,
) -> BigResult {
    let mut r = BigResult::default();
    let mut tune = tune.clone();
    let mut cached_tail = cached_tail;
    let mut rename_to = rename_to;
    let hs = match sys::create_file(
        src,
        sys::GENERIC_READ,
        sys::FILE_SHARE_READ | sys::FILE_SHARE_DELETE,
        sys::OPEN_EXISTING,
        sys::FILE_FLAG_NO_BUFFERING | sys::FILE_FLAG_SEQUENTIAL_SCAN | sys::FILE_FLAG_OVERLAPPED,
    ) {
        Ok(h) => Arc::new(h),
        Err(e) => {
            r.error = unsupported(e);
            return r;
        }
    };
    let size = match sys::file_size(&hs) {
        Ok(s) => s,
        Err(e) => {
            r.error = e;
            return r;
        }
    };
    let Some(streams) = named_streams(src) else {
        r.error = NOT_SUPPORTED;
        return r;
    };

    let mut hd = match sys::create_file(
        dst,
        sys::GENERIC_READ | sys::GENERIC_WRITE | sys::DELETE,
        sys::FILE_SHARE_READ | sys::FILE_SHARE_WRITE,
        sys::CREATE_NEW,
        sys::FILE_FLAG_NO_BUFFERING | sys::ATTRIBUTE_NORMAL,
    ) {
        Ok(h) => Some(h),
        Err(e) => {
            r.error = unsupported(e);
            return r;
        }
    };
    let mut hc: Option<sys::Handle> = None; // the cached tail goes through this one (and the copy is finished on it)
    let align = 512.max(tune.align);
    let room = (size + align as i64 - 1) / align as i64 * align as i64;
    // the whole size up front: the disk can hand out one piece, and a full disk shows at once
    if tune.prealloc
        && room > 0
        && let Err(e) = sys::set_allocation(hd.as_ref().unwrap(), room)
        && (e == sys::ERROR_DISK_FULL || e == sys::ERROR_HANDLE_DISK_FULL)
    {
        r.error = e;
        return r;
    }
    if tune.prealloc
        && room > 0
        && let Err(e) = sys::set_end_of_file(hd.as_ref().unwrap(), room)
    {
        r.error = unsupported(e);
        return r;
    }

    let x = if hash { Some(Arc::new(Mutex::new(XxHash64::new()))) } else { None };
    let mut done = 0i64;
    // the buffers are committed memory: an eighth of the free RAM and of the free commit (a PC with a small page file
    // runs out of commit before RAM)
    let mem_cap = sys::memory_status().map_or(i64::MAX, |(phys, page)| (phys.min(page) / 8) as i64);
    // stretch by stretch: with a tuner the first stretches try other ways, then the fastest copies the rest
    while done < size && r.error == 0 {
        let mut end = size;
        let mut measured = false;
        if let Some(t) = tuner.as_deref() {
            (tune, end, measured) = t.next(done, size, cached_tail);
        }
        let chunk = tune.chunk as i64;
        // no more buffers than the stretch has pieces (a 40 MB file needs two, not four), nor than memory allows
        let mut slots = 2.max((tune.slots as i64).min((end - done + chunk - 1) / chunk));
        slots = 2.max(slots.min(mem_cap / chunk));
        let from = done;
        let mut waited = Duration::ZERO;
        let t0 = Instant::now();
        {
            let mut rr = RingReader::new(hs.clone(), end, tune.chunk, slots as i32, true, x.clone(), tune.phased, done, tune.read_depth);
            while let Some(mut p) = rr.next() {
                // phased (one disk): the whole batch is read before any of it is written
                if tune.phased {
                    rr.gather(&mut p, rr.slots() - 1);
                } else if tune.write_batch > 1 {
                    rr.gather(&mut p, tune.write_batch - 1);
                }
                let n = p.n;
                // phased: written a piece at a time, so the progress moves smoothly (the writes follow each other on
                // the disk)
                let mut step = if tune.phased {
                    if tune.write_step > 0 { tune.chunk.max(tune.write_step) } else { tune.chunk }
                } else {
                    n
                };
                let cap = max_write.map_or(0, |f| f());
                if cap > 0 && cap < step {
                    step = align.max(cap / align * align);
                }
                let mut off = 0i32;
                while off < n && r.error == 0 {
                    if hc.is_none()
                        && cached_tail > 0
                        && size - done <= cached_tail
                        && (!tune.tail_after_read || (end == size && rr.all_read()))
                    {
                        // the last part: through the Windows cache from here on (done is a whole number of pieces).
                        // The unbuffered handle closes first: Windows writes cached data out when such a handle
                        // sets the size or closes, and the tail must stay in memory until Windows writes it on its own
                        match sys::create_file(
                            dst,
                            sys::GENERIC_WRITE,
                            sys::FILE_SHARE_READ | sys::FILE_SHARE_WRITE | sys::FILE_SHARE_DELETE,
                            sys::OPEN_EXISTING,
                            sys::FILE_FLAG_SEQUENTIAL_SCAN,
                        ) {
                            Ok(c) if sys::set_file_pointer(&c, done) => {
                                hd = None;
                                hc = Some(c);
                                rename_to = None; // renamed by path once it is closed
                            }
                            _ => cached_tail = 0,
                        }
                    }
                    let len = if hc.is_some() && tune.cached_write_chunk > 0 { step.min(tune.cached_write_chunk) } else { step }
                        .min(n - off);
                    let buf = &mut p.bytes_mut()[off as usize..];
                    let ok = if let Some(c) = &hc {
                        sys::write(c, &buf[..len as usize])
                    } else {
                        let mut w = len;
                        if w % align != 0 {
                            // the tail: pad to a whole sector, the size is put right below
                            let padded = (len + align - 1) / align * align;
                            buf[len as usize..padded as usize].fill(0);
                            w = padded;
                        }
                        sys::write(hd.as_ref().unwrap(), &buf[..w as usize])
                    };
                    if let Err(e) = ok {
                        r.error = unsupported(e);
                        break;
                    }
                    off += len;
                    done += len as i64;
                    // a pause or the speed limit holds the copy here: that time isn't the way's speed
                    let c0 = Instant::now();
                    if !progress(done) {
                        r.error = sys::ERROR_REQUEST_ABORTED;
                    }
                    let held = c0.elapsed();
                    if held > Duration::from_millis(50) {
                        waited += held;
                    }
                }
                rr.release(p);
                if r.error != 0 {
                    break;
                }
            }
            if r.error == 0 && rr.error() != 0 {
                r.error = unsupported(rr.error());
            }
        }
        if let Some(t) = tuner.as_deref_mut()
            && r.error == 0
        {
            t.report(done - from, t0.elapsed().saturating_sub(waited).as_secs_f64(), measured);
        }
    }
    r.bytes = done;
    if r.error != 0 {
        return r;
    }
    if done != size {
        r.error = NOT_SUPPORTED; // the size changed under us: let CopyFileEx do it
        return r;
    }

    let fin = hc.as_ref().or(hd.as_ref()).expect("the copy's handle"); // the handle the copy is finished on
    if let Err(e) = sys::set_end_of_file(fin, size) {
        r.error = e;
        return r;
    }
    for s in &streams {
        if copy_stream(&(src.clone() + s), &(dst.clone() + s)) != 0 {
            r.error = NOT_SUPPORTED;
            return r;
        }
    }
    // dates + attributes, set on the open handle (nothing changes them after this)
    let mut a = attrs & KEEP_ATTRS;
    if a == 0 {
        a = sys::FILE_ATTRIBUTE_NORMAL;
    }
    r.times_set = sys::set_basic_info(fin, ctime, mtime, a);
    if flush && let Err(e) = sys::flush(fin) {
        r.error = e;
        return r;
    }
    if let Some(x) = &x {
        r.hash = lock(x).digest();
    }
    if let Some(to) = rename_to {
        r.renamed = sys::rename_by_handle(fin, to);
    }
    r
}

/// One small file read ahead into memory (one disk at both ends): opened unbuffered, where it starts on the disk noted
/// (so a batch is read in disk order), then read whole. None = this file goes the usual way. Some((handle, size, lcn)).
pub fn open_small(src: &WString) -> Option<(Handle, i64, i64)> {
    let h = sys::create_file(
        src,
        sys::GENERIC_READ,
        sys::FILE_SHARE_READ | sys::FILE_SHARE_DELETE,
        sys::OPEN_EXISTING,
        sys::FILE_FLAG_NO_BUFFERING | sys::FILE_FLAG_SEQUENTIAL_SCAN,
    )
    .ok()?;
    let size = sys::file_size(&h).ok()?;
    let streams = named_streams(src)?;
    if !streams.is_empty() {
        return None; // extra streams: the usual way keeps them
    }
    let lcn = sys::first_lcn(&h).unwrap_or(i64::MAX);
    Some((h, size, lcn))
}

/// Reads a small file whole into `buf` (its room in the batch: the size rounded up to a sector). False = it didn't
/// read as it should (or no longer fits its room).
pub fn read_small(h: &Handle, buf: &mut [u8], size: i64, align: i32) -> bool {
    let want = ((size + align as i64 - 1) / align as i64 * align as i64) as usize;
    if want > buf.len() {
        return false;
    }
    let mut done = 0usize;
    while (done as i64) < size {
        let n = match sys::read(h, &mut buf[done..want]) {
            Ok(n) if n > 0 => n as usize,
            _ => return false,
        };
        done += n;
        if n % align as usize != 0 {
            break; // the end of the file
        }
    }
    done as i64 == size
}

/// Writes a file from memory to a new name in one go, past the cache, with its dates and attributes; renamed through
/// its own handle to rename_to when given. Returns (0 or the error, renamed, times set). buf must have room for the
/// sector padding.
#[allow(clippy::too_many_arguments)]
pub fn write_small(
    dst: &WString,
    buf: &mut [u8],
    size: i64,
    align: i32,
    ctime: i64,
    mtime: i64,
    attrs: u32,
    rename_to: Option<&WString>,
    cached: bool,
) -> (u32, bool, bool) {
    let hd = match sys::create_file(
        dst,
        sys::GENERIC_READ | sys::GENERIC_WRITE | sys::DELETE,
        sys::FILE_SHARE_READ,
        sys::CREATE_NEW,
        (if cached { 0 } else { sys::FILE_FLAG_NO_BUFFERING }) | sys::ATTRIBUTE_NORMAL,
    ) {
        Ok(h) => h,
        Err(e) => return (unsupported(e), false, false),
    };
    // through the cache: the exact size; Windows writes it out in its own time, many files in one sweep
    let padded = if cached { size } else { (size + align as i64 - 1) / align as i64 * align as i64 } as usize;
    buf[size as usize..padded].fill(0);
    if padded > 0
        && let Err(e) = sys::write(&hd, &buf[..padded])
    {
        return (unsupported(e), false, false);
    }
    if !cached && let Err(e) = sys::set_end_of_file(&hd, size) {
        return (e, false, false); // only the padded unbuffered write needs trimming
    }
    let mut a = attrs & KEEP_ATTRS;
    if a == 0 {
        a = sys::FILE_ATTRIBUTE_NORMAL;
    }
    let times_set = sys::set_basic_info(&hd, ctime, mtime, a);
    let renamed = rename_to.is_some_and(|to| sys::rename_by_handle(&hd, to));
    (0, renamed, times_set)
}

/// The way in or out refuses unbuffered I/O (some file systems and drivers): CopyFileEx takes this file.
pub fn unsupported(e: u32) -> u32 {
    if e == sys::ERROR_INVALID_PARAMETER || e == sys::ERROR_NOT_SUPPORTED || e == sys::ERROR_INVALID_FUNCTION { NOT_SUPPORTED } else { e }
}

/// Names of the extra data streams (":Zone.Identifier:$DATA"); None = couldn't tell.
fn named_streams(path: &WString) -> Option<Vec<WString>> {
    match sys::stream_names(path) {
        // no streams at all (FAT, exFAT)
        Err(e) => if e == sys::ERROR_HANDLE_EOF { Some(Vec::new()) } else { None },
        Ok(names) => {
            let suffix = WString::from(":$DATA");
            Some(
                names
                    .into_iter()
                    .filter(|n| {
                        !n.is_empty()
                            && !n.eq_str("::$DATA")
                            && n.len() >= suffix.len()
                            && n.substring(n.len() - suffix.len()).eq_ignore_case(&suffix)
                    })
                    .collect(),
            )
        }
    }
}

/// Copies one extra data stream. 0 or the error.
fn copy_stream(from: &WString, to: &WString) -> u32 {
    let a = match sys::create_file(from, sys::GENERIC_READ, sys::FILE_SHARE_READ | sys::FILE_SHARE_DELETE, sys::OPEN_EXISTING, 0) {
        Ok(h) => h,
        Err(e) => return e,
    };
    // the copy itself is open with DELETE access (for the rename): a stream of it opens only when it shares delete
    let b = match sys::create_file(
        to,
        sys::GENERIC_WRITE,
        sys::FILE_SHARE_READ | sys::FILE_SHARE_WRITE | sys::FILE_SHARE_DELETE,
        sys::CREATE_ALWAYS,
        0,
    ) {
        Ok(h) => h,
        Err(e) => return e,
    };
    // FileStream.CopyTo in 64 KB pieces; an IOException was ERROR_IO_DEVICE
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = match sys::read(&a, &mut buf) {
            Ok(0) => return 0,
            Ok(n) => n as usize,
            Err(_) => return sys::ERROR_IO_DEVICE,
        };
        let mut w = 0;
        while w < n {
            match sys::write(&b, &buf[w..n]) {
                Ok(m) if m > 0 => w += m as usize,
                _ => return sys::ERROR_IO_DEVICE,
            }
        }
    }
}
