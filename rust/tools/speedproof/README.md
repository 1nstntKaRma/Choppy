# Step 3: the speed proof

The proven C# Benchmark (src/Bench, `rust` branch only) runs three sides in turns on every case: C# 1.48 (in its own
process, as always), the Rust engine (a child) and Windows' copy (a child, `--shellcopy`). All three copy the same
source, which is dropped from the cache before every run; the order goes A-B-C-C-B-A and each case starts one further
along.

    Choppy.exe --benchmark out.json --thorough --network --rust <rust choppy.exe> [--log f] [--drives C,D] [--routes C>D]
               [--cases big,small,mixed] [--rounds 5] [--size-down 0|1] [--rust-fresh]
    python table.py out.json [rerun.json ...]     (a later report replaces a route's case from an earlier one)

Run it in test mode (`CHOPPY_DATA=<scratch folder>`), quote `--routes "C>D"` in a shell, and only while nothing else
runs (no build, no other test copy or benchmark): the disks are timed.

## How the time is counted (the same for C# and Rust)

- C#: `Benchmark.ChoppyCopy` - a tiny warm-up file is copied first (untimed); the clock starts right before the job
  starts and stops at its Finished event.
- Rust: `choppy.exe --benchcopy -` (`cli/bench_copy.rs`), one child for the whole run, warm like the C# engine in its
  process (the app is one process that copies again and again). Per copy it gets a job file's path on stdin, copies
  the warm-up file (untimed), then times the copy exactly as ChoppyCopy does, and answers in one line on stdout:
  `seconds`, `ended` (the moment of the Finished event, a FILETIME to the microsecond) and the way it picked. The time
  from `ended` until the parent has the answer (~0.1 ms) is added to Rust's time: nothing after its start goes
  uncounted except starting the process.
- All sides: then the parent lists the copies and flushes them (256 threads); the time to the disk = done + flush.
  The verdicts use the time to the disk, 4 % = Marginal Tie, as the Benchmark always did.
- `--rust-fresh` starts a new Rust process per copy (its end and the result file then count against it: ~9 ms).
- A/A check: `--rust <the C# Choppy.exe>` puts the C# engine in the child: the method against itself.
- `CHOPPY_BENCH_TRACE=<file>` writes each phase's start (ms) and the files done then, for both engines.
- `--size-down 1` keeps a rerun of some routes on the files a run over all drives copies (the time budget would
  otherwise pick bigger ones).

## results/

Masked copies (drive labels blanked, the NAS path made generic) of the reports (`name.json`) and their logs (`name-log.txt`: `run:` lines = every
copy with its way, `result:` lines = each case):

| file | what |
|---|---|
| full | first Thorough + Network run (C, D, F, N, X), Rust before the sleep fix, a new Rust process per copy |
| cc-warm, cc-fresh | C>C small + mixed, 5 rounds, warm vs fresh child: the same 0.509 s for Rust every run, the clue to 3.1 |
| cx | C>X 5 rounds (2 GB, before --size-down existed): the tuner trials happen in both engines, timing decides |
| full2 | the final Thorough + Network run, fixed engine, warm child |
| e, e2 | E: (10 Gbps USB drive, plugged in during the step): Thorough, then 5 rounds on two cases |
| r5 | 5 rounds on C>D, C>X, N>C (the cases full2 left in doubt) |
| xs, xsaa | 5 rounds of C>D + C>X small: Rust, then the A/A check (C# in the child): +/-10 % noise on hard-disk small files |
| final-table.md | full2 + e + r5 + e2 by `table.py` |
