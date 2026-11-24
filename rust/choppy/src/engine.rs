//! The copy engine: Paths, Settings, XxHash64, Model, VolumeProbe, UsbLink, Scanner, Plan (step 1);
//! BigCopy, CopyJob, Handoff, JobManager (step 2).
#![deny(unsafe_code)]

use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub mod big_copy;
pub mod copy_job;
pub mod handoff;
pub mod model;
pub mod netfmt;
pub mod paths;
pub mod plan;
pub mod scanner;
pub mod settings;
pub mod usb_link;
pub mod volume_probe;
pub mod xxhash64;

/// Locks a mutex; one a panicking thread left behind is used as it is (the panic was caught and reported where it
/// happened, as C#'s locks don't know about exceptions either).
pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Condvar::wait_timeout with `lock`'s rule.
pub fn wait_timeout<'a, T>(cv: &Condvar, g: MutexGuard<'a, T>, d: Duration) -> MutexGuard<'a, T> {
    cv.wait_timeout(g, d).map(|r| r.0).unwrap_or_else(|e| e.into_inner().0)
}

/// Condvar::wait with `lock`'s rule.
pub fn wait<'a, T>(cv: &Condvar, g: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    cv.wait(g).unwrap_or_else(|e| e.into_inner())
}

/// What a caught panic said (C#'s exception Message).
pub fn panic_message(e: &(dyn std::any::Any + Send)) -> String {
    e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| "panic".into())
}

/// System.Diagnostics.Stopwatch: running time that can stop and go on (Start, Stop, Restart, Elapsed), from any thread.
#[derive(Default)]
pub struct Stopwatch {
    st: Mutex<(Option<Instant>, Duration)>, // (running since, time before that)
}

impl Stopwatch {
    pub fn start_new() -> Stopwatch {
        let s = Stopwatch::default();
        s.start();
        s
    }
    pub fn start(&self) {
        let mut g = lock(&self.st);
        if g.0.is_none() {
            g.0 = Some(Instant::now());
        }
    }
    pub fn stop(&self) {
        let mut g = lock(&self.st);
        if let Some(t) = g.0.take() {
            g.1 += t.elapsed();
        }
    }
    pub fn restart(&self) {
        *lock(&self.st) = (Some(Instant::now()), Duration::ZERO);
    }
    pub fn is_running(&self) -> bool {
        lock(&self.st).0.is_some()
    }
    pub fn elapsed(&self) -> Duration {
        let g = lock(&self.st);
        g.1 + g.0.map_or(Duration::ZERO, |t| t.elapsed())
    }
}
