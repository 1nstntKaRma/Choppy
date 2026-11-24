//! Streaming xxHash64 (seed 0) plus a file hasher that reads straight from the disk/server (no cache) (XxHash64.cs).

use std::sync::Arc;

use super::big_copy::RingReader;
use crate::sys;
use crate::wstr::WString;

const P1: u64 = 11400714785074694791;
const P2: u64 = 14029467366897019727;
const P3: u64 = 1609587929392839161;
const P4: u64 = 9650029242287828579;
const P5: u64 = 2870177450012600261;

#[derive(Clone)]
pub struct XxHash64 {
    v1: u64,
    v2: u64,
    v3: u64,
    v4: u64,
    total: u64,
    tail: [u8; 32],
    tail_len: usize,
}

impl Default for XxHash64 {
    fn default() -> Self {
        Self::new()
    }
}

fn read64(b: &[u8]) -> u64 {
    u64::from_le_bytes(b[..8].try_into().unwrap())
}

fn round(acc: u64, input: u64) -> u64 {
    acc.wrapping_add(input.wrapping_mul(P2)).rotate_left(31).wrapping_mul(P1)
}

fn merge(acc: u64, val: u64) -> u64 {
    (acc ^ round(0, val)).wrapping_mul(P1).wrapping_add(P4)
}

impl XxHash64 {
    pub fn new() -> XxHash64 {
        XxHash64 { v1: P1.wrapping_add(P2), v2: P2, v3: 0, v4: 0u64.wrapping_sub(P1), total: 0, tail: [0; 32], tail_len: 0 }
    }

    pub fn update(&mut self, data: &[u8]) {
        let len = data.len();
        self.total = self.total.wrapping_add(len as u64);
        if self.tail_len + len < 32 {
            self.tail[self.tail_len..self.tail_len + len].copy_from_slice(data);
            self.tail_len += len;
            return;
        }
        let mut p = data;
        if self.tail_len > 0 {
            let fill = 32 - self.tail_len;
            self.tail[self.tail_len..].copy_from_slice(&p[..fill]);
            let t = self.tail;
            self.v1 = round(self.v1, read64(&t[0..]));
            self.v2 = round(self.v2, read64(&t[8..]));
            self.v3 = round(self.v3, read64(&t[16..]));
            self.v4 = round(self.v4, read64(&t[24..]));
            p = &p[fill..];
            self.tail_len = 0;
        }
        let (mut a, mut b, mut c, mut d) = (self.v1, self.v2, self.v3, self.v4);
        while p.len() >= 32 {
            a = round(a, read64(&p[0..]));
            b = round(b, read64(&p[8..]));
            c = round(c, read64(&p[16..]));
            d = round(d, read64(&p[24..]));
            p = &p[32..];
        }
        self.v1 = a;
        self.v2 = b;
        self.v3 = c;
        self.v4 = d;
        self.tail[..p.len()].copy_from_slice(p);
        self.tail_len = p.len();
    }

    pub fn digest(&self) -> u64 {
        let mut h = if self.total >= 32 {
            let mut h = self
                .v1
                .rotate_left(1)
                .wrapping_add(self.v2.rotate_left(7))
                .wrapping_add(self.v3.rotate_left(12))
                .wrapping_add(self.v4.rotate_left(18));
            h = merge(h, self.v1);
            h = merge(h, self.v2);
            h = merge(h, self.v3);
            merge(h, self.v4)
        } else {
            self.v3.wrapping_add(P5)
        };
        h = h.wrapping_add(self.total);
        let mut p = &self.tail[..self.tail_len];
        while p.len() >= 8 {
            h ^= round(0, read64(p));
            h = h.rotate_left(27).wrapping_mul(P1).wrapping_add(P4);
            p = &p[8..];
        }
        if p.len() >= 4 {
            h ^= (u32::from_le_bytes(p[..4].try_into().unwrap()) as u64).wrapping_mul(P1);
            h = h.rotate_left(23).wrapping_mul(P2).wrapping_add(P3);
            p = &p[4..];
        }
        for &b in p {
            h ^= (b as u64).wrapping_mul(P5);
            h = h.rotate_left(11).wrapping_mul(P1);
        }
        h ^= h >> 33;
        h = h.wrapping_mul(P2);
        h ^= h >> 29;
        h = h.wrapping_mul(P3);
        h ^= h >> 32;
        h
    }

    pub fn of_bytes(data: &[u8]) -> u64 {
        let mut x = XxHash64::new();
        x.update(data);
        x.digest()
    }
}

/// Hashes a file. no_cache = opened unbuffered so the bytes really come from the disk/NAS, not from what Windows still
/// holds in memory after writing it. progress gets the bytes read so far (false = stop). Err = the error when it can't
/// read (ERROR_REQUEST_ABORTED once stopped).
pub fn hash_file(long_path: &WString, no_cache: bool, mut progress: Option<&mut dyn FnMut(i64) -> bool>) -> Result<u64, u32> {
    let flags = sys::FILE_FLAG_SEQUENTIAL_SCAN | if no_cache { sys::FILE_FLAG_NO_BUFFERING | sys::FILE_FLAG_OVERLAPPED } else { 0 };
    let h = Arc::new(sys::create_file(
        long_path,
        sys::GENERIC_READ,
        sys::FILE_SHARE_READ | sys::FILE_SHARE_WRITE | sys::FILE_SHARE_DELETE,
        sys::OPEN_EXISTING,
        flags,
    )?);
    let size = sys::file_size(&h)?;
    let mut x = XxHash64::new();
    let mut done = 0i64;
    // the next pieces are read while this one is hashed
    let mut rr = RingReader::new(h, size, 8 << 20, 4, no_cache, None, false, 0, 0);
    while let Some(p) = rr.next() {
        let read = p.n;
        x.update(&p.bytes()[..read as usize]);
        rr.release(p);
        done += read as i64;
        if let Some(pr) = progress.as_deref_mut()
            && !pr(done)
        {
            return Err(sys::ERROR_REQUEST_ABORTED);
        }
    }
    if rr.error() != 0 {
        return Err(rr.error());
    }
    Ok(x.digest())
}
