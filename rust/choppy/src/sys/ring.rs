//! Big aligned buffers and the reads the disk fills in the background (BigCopy.cs's RingReader memory, its OVERLAPPED
//! reads, the small-file batch buffer). RingReader's logic (which piece is read when, the semaphores, phased batches)
//! stays in engine/big_copy.rs; what it can do to memory is only what this module hands out.
//!
//! A ring is one block of committed memory (VirtualAlloc: page aligned, so every buffer starts on a sector boundary for
//! unbuffered I/O) cut into `slots` buffers of `chunk` bytes. Each buffer is a [`Slot`], made once and never copied:
//! whoever holds a Slot is the only one who can touch its bytes, and the block is freed when the last Slot is gone.
//! A read the disk is still filling holds its Slot inside a [`PendingRead`], so nobody can look at those bytes before
//! the read is collected; dropping a PendingRead waits for the disk first (C#'s Drain), so a buffer is never freed or
//! handed on while Windows still writes into it.

use core::ffi::c_void;
use std::sync::Arc;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::ReadFile;
use windows::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Memory::{MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc, VirtualFree};
use windows::Win32::System::Threading::{CreateEventW, ResetEvent};
use windows::core::PCWSTR;

use super::{ERROR_IO_PENDING, Handle, win32_code};

/// Committed memory from VirtualAlloc, released (VirtualFree) when dropped.
struct Block {
    ptr: *mut u8,
    len: usize,
}

// The block is only reached through Slots / Memory, each of which owns a disjoint part of it.
unsafe impl Send for Block {}
unsafe impl Sync for Block {}

impl Block {
    fn commit(len: usize) -> Option<Block> {
        let p = unsafe { VirtualAlloc(None, len, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) };
        if p.is_null() { None } else { Some(Block { ptr: p as *mut u8, len }) }
    }
}

impl Drop for Block {
    fn drop(&mut self) {
        unsafe {
            let _ = VirtualFree(self.ptr as *mut c_void, 0, MEM_RELEASE);
        }
    }
}

/// A buffer of committed memory owned by one holder (the small-file batch: CopyJob.FillBatch).
pub struct Memory(Block);

impl Memory {
    /// VirtualAlloc(MEM_COMMIT | MEM_RESERVE): None = no memory for it.
    pub fn commit(len: usize) -> Option<Memory> {
        Block::commit(len).map(Memory)
    }
    pub fn len(&self) -> usize {
        self.0.len
    }
    pub fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.0.ptr, self.0.len) }
    }
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.0.ptr, self.0.len) }
    }
}

/// One buffer of a ring. Exactly one Slot exists for each buffer, so `bytes_mut` hands out the only reference to it.
pub struct Slot {
    block: Arc<Block>,
    index: usize,
    off: usize,
    len: usize,
    /// one OVERLAPPED per buffer, with its own manual-reset event; boxed: it must not move while a read is out
    ov: Option<Box<OVERLAPPED>>,
}

// A Slot owns its bytes and its OVERLAPPED; it moves between the reader thread and the writer, never shared.
unsafe impl Send for Slot {}

impl Slot {
    /// Its place in the ring (piece k sits in slot k % slots).
    pub fn index(&self) -> usize {
        self.index
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.block.ptr.add(self.off), self.len) }
    }
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.block.ptr.add(self.off), self.len) }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(ov) = &self.ov {
            unsafe {
                let _ = CloseHandle(ov.hEvent);
            }
        }
    }
}

/// `slots` buffers of `chunk` bytes in one block (VirtualAlloc), in ring order; with `overlapped` each one gets its
/// OVERLAPPED and event for reads at an offset. None = not enough memory (C# threw OutOfMemoryException).
pub fn ring(chunk: usize, slots: usize, overlapped: bool) -> Option<Vec<Slot>> {
    let block = Arc::new(Block::commit(chunk.checked_mul(slots)?)?);
    let mut r = Vec::with_capacity(slots);
    for i in 0..slots {
        let ov = if overlapped {
            let ev = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.ok()?;
            Some(Box::new(OVERLAPPED { hEvent: ev, ..Default::default() }))
        } else {
            None
        };
        r.push(Slot { block: block.clone(), index: i, off: i * chunk, len: chunk, ov });
    }
    Some(r)
}

/// Where slots that follow each other in the ring start and end in their block. None = they don't follow each other.
fn span(slots: &[Slot]) -> Option<(*mut u8, usize, usize)> {
    let first = slots.first()?;
    let mut end = first.off;
    for s in slots.iter() {
        if !Arc::ptr_eq(&s.block, &first.block) || s.off != end {
            return None;
        }
        end += s.len;
    }
    Some((first.block.ptr, first.off, end))
}

/// The bytes of slots that follow each other in the ring, as one buffer (pieces joined into one write). None = they
/// don't follow each other in memory.
pub fn joined_mut(slots: &mut [Slot]) -> Option<&mut [u8]> {
    let (ptr, off, end) = span(slots)?;
    // every byte of [off, end) belongs to one of these slots, all held here mutably
    Some(unsafe { std::slice::from_raw_parts_mut(ptr.add(off), end - off) })
}

/// `joined_mut` for reading.
pub fn joined(slots: &[Slot]) -> Option<&[u8]> {
    let (ptr, off, end) = span(slots)?;
    Some(unsafe { std::slice::from_raw_parts(ptr.add(off), end - off) })
}

/// A read out at the disk into a slot (a handle opened with FILE_FLAG_OVERLAPPED). `wait` gives the slot back with
/// the bytes read; dropping it waits for the read first.
pub struct PendingRead {
    slot: Option<Slot>,
    h: Arc<Handle>,
}

/// ReadFile at `offset` of the file into the slot's bytes from `at` on (`bytes` of them), with the slot's OVERLAPPED
/// (its event reset, its status cleared). Err = it failed at once (nothing is out); the slot comes back.
pub fn read_at(h: &Arc<Handle>, mut slot: Slot, at: usize, bytes: usize, offset: i64) -> Result<PendingRead, (Slot, u32)> {
    assert!(at + bytes <= slot.len, "read past the buffer");
    let ptr = unsafe { slot.block.ptr.add(slot.off + at) };
    let Some(ov) = slot.ov.as_deref_mut() else { panic!("a ring without OVERLAPPED") };
    ov.Internal = 0;
    ov.InternalHigh = 0;
    ov.Anonymous.Anonymous.Offset = offset as u32;
    ov.Anonymous.Anonymous.OffsetHigh = (offset as u64 >> 32) as u32;
    unsafe {
        let _ = ResetEvent(ov.hEvent);
    }
    let ovp: *mut OVERLAPPED = ov;
    // The slot (its bytes and OVERLAPPED) moves into the PendingRead below and stays there until the read is done.
    let r = unsafe { ReadFile(h.0, Some(std::slice::from_raw_parts_mut(ptr, bytes)), None, Some(ovp)) };
    match r {
        Ok(()) => Ok(PendingRead { slot: Some(slot), h: h.clone() }),
        Err(e) => {
            let code = win32_code(&e);
            if code == ERROR_IO_PENDING { Ok(PendingRead { slot: Some(slot), h: h.clone() }) } else { Err((slot, code)) }
        }
    }
}

fn overlapped_result(h: HANDLE, ov: &OVERLAPPED) -> Result<u32, u32> {
    let mut n = 0u32;
    unsafe { GetOverlappedResult(h, ov, &mut n, true) }.map(|_| n).map_err(|e| win32_code(&e))
}

impl PendingRead {
    /// GetOverlappedResult(wait = TRUE): the slot back, with Ok(bytes read) or Err(the Win32 error).
    pub fn wait(mut self) -> (Slot, Result<u32, u32>) {
        let slot = self.slot.take().expect("a pending read without its slot");
        let r = overlapped_result(self.h.0, slot.ov.as_deref().expect("a ring without OVERLAPPED"));
        (slot, r)
    }
}

impl Drop for PendingRead {
    fn drop(&mut self) {
        // the read is still out: its buffer can go only once the disk is done with it
        if let Some(slot) = self.slot.take() {
            let _ = overlapped_result(self.h.0, slot.ov.as_deref().expect("a ring without OVERLAPPED"));
        }
    }
}
