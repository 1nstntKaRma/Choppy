//! Big files between local drives (BigCopy.cs): step 2 ports RingReader, PipeTuner and BigCopy. Plan (step 1) picks
//! a PipeTuning, so that is here already, with PipeTuner.Describe (the plan dump and the logs print it).

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

/// Finds the fastest way for this copy while it runs (step 2). Here: only its Describe.
pub struct PipeTuner;

impl PipeTuner {
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
