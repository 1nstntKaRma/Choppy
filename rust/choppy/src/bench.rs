//! The Benchmark (step 4): Benchmark, BenchFiles, BenchReport, ShellCopy - Choppy against Windows' own copy on every
//! drive of this PC, as 1.48 runs it (its HTML / JSON report formats unchanged).
#![deny(unsafe_code)]

pub mod benchmark;
pub mod files;
pub mod report;
pub mod shell_copy;

/// What stops a step: a cancel (C#'s OperationCanceledException) or an error with its message.
#[derive(Clone, Debug)]
pub enum BenchErr {
    Canceled,
    Error(String),
}

pub type BenchResult<T> = Result<T, BenchErr>;
