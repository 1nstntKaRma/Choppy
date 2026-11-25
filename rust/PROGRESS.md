# Choppy in Rust - progress

Read [PLAN.md](PLAN.md) first (steps, gates, rules, file map). One step per session; update this file at the end.

## Steps

| # | Step | Status |
|---|------|--------|
| 0 | Tools + UI look test | **done** (2026-11-24): tools installed, both prototypes compared, **the user picked A** |
| 1 | Basics | **done** (2026-11-24): gate met, plan dump identical to C# 1.48, fix list 1.1-1.79 checked |
| 2 | Copy engine | **done** (2026-11-25): gate met (33 copy / move scenarios identical to C# 1.48), self-test lines identical, review pass done, fix list 2.1-2.64 checked |
| 3 | Speed proof | **benchmark runs finished** (2026-11-25): gate met (48 cases, level with 1.48, nothing newly Slower), one porting fix (3.1); **waits for the user's sign-off** |
| 4 | All windows | not started |
| 5 | Explorer / system | not started |
| 6 | Switch-over | not started |

## Step 0 (2026-11-24)

**Tools.** winget: Visual Studio Build Tools 2026 (18.12, VC tools + Windows SDK 10.0.26100) and rustup 1.29.3;
`rustup default stable-x86_64-pc-windows-msvc` = rustc 1.100.0. A hello-world builds and runs with MSVC.
(The Build Tools installer asked for a restart "to finish"; building works without it.)

**Workspace** `rust/`: `choppy/` (the app, empty modules), `look-test/common` (the README demo job, Format, the
speed graph's math - shared so A and B showed identical numbers), `look-test/a-d2d` (B, `look-test/b-slint`, was
removed after the pick; see commit 1aa7bbf),
`tools/` (scripts for the pictures and numbers below). `rust/target/` is ignored.

**Look test.** Both prototypes show the README demo (`--at 29 --freeze` / `--compact --at 17` for pictures,
`--x/--y`, `--dpi 96` (A) or `SLINT_SCALE_FACTOR=1` (B) for 100 %, `--exit-after s`). Pictures in
`look-test/results/`: 1 extended at 125 %, 2 compact at 125 %, 3 text zoomed, 4 states (Queue pop-up, typing in the
speed-limit box), 5 A and B at 100 %. Today's window was captured from a test-mode copy of 1.48 (scratch exe,
CHOPPY_DATA) copying the demo files D:\Home Videos -> X:\Backup\Home Videos at 57 MB/s (removed afterwards).

| | Today (WPF) | A: Direct2D | B: Slint |
|---|---|---|---|
| Opens in (start -> visible window, median of 5) | 1.93 s | 0.035 s | 0.24 s |
| Memory after 10 s (working set / private) | 137 / 126 MB | 40 / 63 MB | 76 / 110 MB |
| CPU while animating (one core) | 3.4 % | 1.2 % | 1.9 % |
| Exe size | 0.95 MB (+ .NET in Windows) | 0.66 MB | 10.1 MB |

(WPF's open time is a cold start of a scratch copy without its start-up profile; the testing notes measured ~1.1 s
for the installed one. A's 40 MB is mostly the graphics driver Direct2D loads.)

**A (Direct2D + DirectWrite, `windows` 0.62):** a small immediate-mode UI layer (`ui.rs` buttons / pills /
toggles in Theme.xaml's styles with hover, pressed, keyboard focus; `textbox.rs` caret, selection, clipboard,
digits only; `popup.rs` a real layered pop-up window with fade; `controls.rs` graph / donut / drive icon).
Layout is done in pixels with WPF's rounding (`Canvas::rp/at/center`): the top half lands on WPF's exact pixels,
the rest within 0-2 px. Exact parity needs WPF's own arithmetic (DIP doubles, Math.Round to even, Grid's
rounding-error distribution); `tools/wpflayout.ps1` prints WPF's real layout of MainWindow to check against (step 4).
WPF draws this window's text with grayscale smoothing (no ClearType colors), so A does too.

**B (Slint 1.19.1):** femtovg (OpenGL) renderer. Skia was the first choice but (1) its prebuilt library exists only
without `+crt-static` (the static key 404s; with it the exe would need the VC++ runtime), and (2) its build
script's own download failed here (curl error 3 inside the build script; the same curl works by hand); a manual
download wasn't approved, so femtovg. Slint draws pop-ups inside the window (winit backend: "we don't render
popups as separate windows yet"), so the Queue pop-up can't reach past the window's edge like WPF's. Its
royalty-free license needs a "Made with Slint" badge on the README / download page or an AboutSlint box.
`Cargo.lock` pins prettyplease 0.2.37 (bindgen 0.73 + syn 2; only Skia's build needs it).

**Gotchas found:** Slint centers elements that have no `x`; a Slint `Path` shrinks to fit its stroke unless
`fit: preserve`; a WPF gradient fill maps to the filled shape's bounds (RelativeToBoundingBox), not the control's.
Capturing: the screen shows a small "restore"-like icon over every captioned window's top bar (another program's
overlay, not Choppy). `tools/drive.py` drives a window with posted messages (never the real mouse) and blanks
everything outside the app's own windows in its pictures.

## Step 1 (2026-11-24)

**Ported** into `choppy/src`: Native -> `sys` (`sys.rs` + `sys/file, volume, device, reg, text`: the only `unsafe`,
each call behind a safe function), Paths, Settings (AppPaths, Settings, Journal), XxHash64 (the hash), Model,
VolumeProbe, UsbLink, Scanner, Plan, and from BigCopy.cs the `PipeTuning` + `PipeTuner.Describe` that Plan needs.
Commands: `--usb`, the new `--plan-dump`, `--selftest` with step 1's 27 checks; Program.Main's start (the start
folder kept, the data folder made and used as the current folder). New helpers: `wstr.rs` (`WString`: paths as UTF-16
like C# strings), `engine/netfmt.rs` (numbers as .NET Framework writes and reads them). Step-2 stubs:
`engine/copy_job.rs` (`TEMP_SUFFIX`), `engine/handoff.rs` (`clean_stale`). `build.rs` embeds app.manifest,
Choppy.ico and the version info through the Windows SDK's rc.exe (no crate). One crate: `windows` 0.62.

**Gate: met.**
- Self-test: the 27 step-1 checks pass; their lines (names, order, results) are identical to C# 1.48's (whose own
  self-test passes all 96 lines here).
- Plan dump, C# (this branch) vs Rust: identical, 4,008 lines. Five drives (C: NVMe, D:/X: hard disks, F: SSD in a
  10 Gbps USB case, G: Google Drive) plus `\\localhost\C$` as a network share; every ordered pair (36) x copy / move /
  retried move x 12 scan shapes = 1,296 plans; Paths on 51 inputs, 10 pairs, 6 numbered names; ErrorText for 2,011
  codes; 8 local times; the USB names; in test mode the settings, a job record written + read back, resume.
- Also identical: 8 kinds of settings.ini, 2 faked USB links, the Scanner on a test tree (11 cases: copy, move, into
  its own folder, into itself, relative paths, a missing destination, network paths, an unreadable folder, a hard
  disk to another; 1 and 4 threads), `--usb` (and with a fake link; only the ms differ), the error file.
- `--usb out.txt` matches C# (F: "USB 10 Gbps, can do 10 Gbps, port USB 3, bcdUSB 0320, UASP").
- `cargo test`: 8 tests (Settings on a scratch registry key - never HKCU\Software\Choppy itself -, number text
  measured on .NET 4.8, Enum.TryParse, the job id, a canceled scan). `cargo clippy`: clean.
- Fix list (PLAN.md): 1.1-1.79 all checked; 2.1-2.5 (from Native.cs, XxHash64.cs, Settings.cs) wait for step 2.
- Release exe 0.87 MB, no VC++ runtime (dumpbin: system DLLs only); FileVersion 1.48.0.0; manifest and icon identical
  to the C# exe's.

**Rerun it** (Git Bash, from the repo): `bash rust/tools/plandump/run-all.sh` builds both and prints SAME/DIFF per
check; `HDD_SRC=D: HDD_DST=X:` adds the disk-to-disk scan. It works in %TEMP%\choppy-plandump and leaves nothing
behind on the drives.

**Found by comparing** (all kept as the C# does them):
- Win32Exception's message: .NET retries FormatMessage with a 4x bigger buffer (messages over 256 characters).
- Path.GetFullPath expands 8.3 names (`PROGRA~1`) with a buffer quirk: where only part of the path exists, 15
  characters or less stay short (measured in PowerShell 5.1, which runs .NET 4.8).
- double.ToString(): 15 digits rounded half up (100000000000000.5 -> 100000000000001), E notation below 1E-04.
- DateTime.ToLocalTime: the standard offset for 1601 (no Windows rule), clamped at 9999-12-31.
- NTFS updates the times a folder listing shows lazily: a folder's listed times change once it has been listed.
  Both programs see the same; the tool lists twice and masks folder access times.

**On purpose, different** (only reachable with broken input): a policy number no policy has (7) is refused (C#
kept an unnamed value); a malformed CHOPPY_FAKE_USB fakes nothing (C# threw); Journal.SaveSoon takes the record's
text at once (C#'s task read the job a moment later); ProductVersion has no "+commit" suffix (C#'s SDK adds it;
release.yml checks FileVersion only).

**For the next steps:**
- Paths and file names are `WString` (UTF-16); text for the screen and logs via `Display` / `to_string_lossy()`.
  OrdinalIgnoreCase = `eq_ignore_case` / `starts_with_ignore_case`; map keys = `to_upper_invariant()`.
- Any number C# writes with InvariantCulture (logs, JSON, job records, settings): `netfmt::g` / `netfmt::f`;
  reading: `netfmt::parse_double` / `parse_int`.
- `FileItem.state` is an `AtomicI32`; its other fields that the copy changes across threads (Error, FinalPath,
  Verified...) need a shared form in step 2 (C# changed them from any thread).
- `Settings::current()` is a `MutexGuard` (C#'s static Settings.Current).
- The crate root has `#![allow(dead_code)]` while parts wait for later steps (remove at switch-over).
- The exe is a GUI-subsystem program like the C# one: console output only shows when redirected.
- No rustfmt config: the code follows the look test's style (lines up to ~120).

## Step 2 (2026-11-25)

**Ported** into `choppy/src`: BigCopy (`engine/big_copy.rs`: RingReader, PipeTuning, PipeTuner, `copy`, the small-file
batch's `open_small` / `read_small` / `write_small`), HashFile (`engine/xxhash64.rs` `hash_file`), CopyJob
(`engine/copy_job.rs`), Handoff (`engine/handoff.rs`, SHA-256 in plain Rust), JobManager (`job_manager.rs`),
`--cli copy|move` (`cli/job.rs`: EngineOption, ParsePolicy, Esc, Report) and the self-test's `temp` + pipeline block.
New `sys` modules: `io.rs` (reads / writes, file information through a handle incl. the rename, CopyFileExW and its
callback, moves, deletes, folders, streams, free space, retrieval pointers), `ring.rs` (the read-ahead memory as owned
`Slot`s; a read the disk is filling is a `PendingRead` that waits for the disk when dropped; the batch memory),
`proc.rs` (processes, runas, the administrator check, thread priority, keep-awake, free memory, the clock, the user's
date format). Model: FileItem's copy-time fields are atomics / locks; ScanResult.files are `Arc<FileItem>`;
ConflictInfo carries the file. Four more `windows` features (Threading, Memory, SystemInformation, Power); no new crate.

**Gate: met.**
- Destination trees: `python rust/tools/enginecheck/check.py <C# Choppy.exe> <Rust choppy.exe> [scenario...]` builds
  the same fixture for each program - odd sizes up to 96 MB (one scenario: 3 GB), attributes, Zone.Identifier on small
  and big files, unicode / emoji names, a path past 260 characters, an NTFS-compressed file, a junction, 200 small
  files, empty / hidden / system folders - and a destination with every kind of conflict and leftover; runs both with
  `--cli`; compares the trees (names, sizes, contents, created / modified dates, attributes, streams), what a move
  leaves at the source, the JSON reports and the logs (times, speeds and run folders masked). **33 scenarios + the
  usage error, all identical:** copy-new, -skip, -replace, -keepboth, -newer, -ask, -resume, -verify, -own-folder
  ("name - Copy (2)"), -fat32, move-same, move-same-new, copy-net, copy-net-verify, copy-from-net, copy-copyfile,
  -limit, -small-batch, -no-fast, -pause, -skip-big, -tuner, -log, cancel (by rule: exit 3, every file there whole, no
  `.chpart`), copy-ssd-hdd, -hdd-ssd, -one-hdd, move-cross, move-resume, move-verify, copy-hdd-hdd, copy-ssd-usb,
  copy-one-usb. Routes: C: NVMe, D: / X: hard disks, F: USB SSD (10 Gbps), `\\localhost\C$`. A full run takes ~20 min.
- FAT32's 4 GB limit: through `--max-file` (no FAT32 drive here). ReFS on one drive: no ReFS volume on this PC (Windows
  10 Home can't make one); the plan's self-tests cover the choice, copy-copyfile the CopyFileEx path such a copy takes,
  move-same the rename.
- Self-test: the 70 lines up to the plan checks are identical to C# 1.48's (C# then adds the Ctrl+V hook and the bench
  checks: steps 5 and 4).
- `cargo test`: 13 (step 2: the administrator side through its files incl. pause / run / cancel and the result, the
  queue, stale administrator records, conflict answers, Retry failed, the tuner's choices, SHA-256). `cargo clippy`:
  clean.
- Fix list 2.1-2.64 checked: 46 by tests, 17 by review, the administrator prompt up to the prompt.
- Copy times stay level with 1.48 in every scenario (both print theirs; step 3 measures properly).

**Review pass** (5 reviewer agents in parallel on Extra: threads and buffers, failures, every Win32 call, the fix
list + self-test, moves and data safety; Ultracode couldn't be switched on while the user was away). No data loss, no
incomplete rename, no memory-unsafety found. Fixed:
- A move's flusher that died (a thread it couldn't make) could leave copies unflushed and workers waiting forever; now
  the work is done where it is and FinishDurable flushes what was left. Thread-making failures elsewhere (workers,
  folders) no longer leave threads running after the job ended.
- The administrator copy's mirror ("as administrator", the current file) is cleared however its loop ends (C#'s
  `finally`); its result is written even if the progress write beside it fails (C# lost the result then).
- The OVERLAPPED of each buffer is reached only through raw pointers (Windows writes it while a read is out).
- Errors read right after the call; RenameIntoPlace's error after four rounds as C#'s; ShellExecuteEx's hInstApp as
  .NET reads it; the SID buffer DWORD-aligned; a small-file batch reads a file only into its own room; the self-test's
  failures end the way C#'s did; the fix list completed (16 rows added, test claims corrected).

**Only the user can do** (nothing blocks step 3):
1. The administrator hand-off end to end: it needs the Windows administrator prompt, and the Rust side that runs as
   administrator (`--elevated-job`) comes with step 5's App. Everything up to the prompt is tested (the record, its
   SHA-256, the command line, the administrator side's progress / control / result files with a real copy). Once step 5
   is in: copy a file into a folder only administrators may write to (e.g. `C:\Program Files\x`), answer Yes, check it
   lands and the copy window mirrors it; then the same with No (the files are reported as failed).
2. Optional: while a Rust copy runs, `powercfg /requests` in an administrator prompt should list choppy.exe under
   SYSTEM (keep-awake; it can't be read without administrator rights, and another program already holds it here).
3. Optional: a FAT32 USB stick for the real 4 GB limit (the gate fakes it with `--max-file`).

**For after the switch-over** (C# 1.48 behaviour the port keeps on purpose; found by the review):
- A move's big file is flushed before its rename, not after; then its original goes. A power cut right then can leave
  the complete copy only as `<name>.chpart` with the original gone (the flusher's path flushes after the rename).
- When the network doesn't come back in 15 minutes, the file stays "active": not failed, its `.chpart` left, the job
  "Done" with 0 failed.
- A try retried after a network or busy error leaves its `.chpart`; the next try takes another temp name.
- A source that shrinks during a pipeline copy fails with "Reached the end of the file" instead of going to CopyFileEx.
- Verify has no retry for a file briefly in use (antivirus): a good copy is deleted and the file fails.
- `--cli` pauses forever on a full destination (no one to press Resume).
- Resume mode deletes any `<name>.chpart` that isn't one of the job's own names, even one an administrator copy from
  the crashed run is still writing; the administrator copy of a resumed job doesn't get resume mode.
- FinishFolders ignores a folder it couldn't make: a move still removes the (empty) source folder.
- Retrying a failed same-drive move puts an item from inside a merged folder into the destination's top folder.
- The job record's final rename isn't flushed (a crash in its first seconds = no resume offered; nothing is lost).
- Copying both `x` and `x.chpart` from one folder at once: `x.chpart` is also `x`'s temp name; with several workers
  Windows' name tunneling can give one copy the other's creation date (found by the gate's fixture, both programs).

**For the next steps:**
- The Rust engine runs headless as `choppy.exe --cli copy|move <src...> <dest> [--json out.json] [--log f]` plus the
  engine's test switches, with the same JSON as 1.48 - step 3's Benchmark can run it as the Choppy side.
- `CopyJob` is used as `Arc<CopyJob>`: the UI reads its atomics and `phase()`, `plan()`, `scan()`, `scanner()`,
  `current_transfer()`, `speed`, `status()`, `retry_file()` / `retry_why()`; callbacks get the `&Arc<CopyJob>`;
  events through `on_finished` / `on_phase_changed`. JobManager's `dispatch` and `on_ui_thread` are for step 4/5's UI
  thread. The administrator side (`--elevated-job`): step 5's App calls `handoff::load`, enqueues it and calls
  `handoff::report` (as App.cs does); `main.rs` already calls `handoff::prepare`.
- Rerun the gate after any engine change: `check.py` (all scenarios, ~20 min; or by name).

## Step 3 (2026-11-25)

**Benchmark runs: finished** (13:15 JST). Nothing more is timed for step 3; the drives are free for step 4.

**How it was measured.** The proven C# Benchmark (`src/Bench`, `rust` branch only) runs three sides in turns on every
case, on the same source (dropped from the cache before each run, order A-B-C-C-B-A): C# 1.48 in its own process as
always, the Rust engine in a child (`choppy.exe --benchcopy -`, one warm process for the whole run, like the app) and
Windows' copy (`--shellcopy`). Rust is timed exactly as `Benchmark.ChoppyCopy` times C#: an untimed warm-up file, then
from right before the job starts to its Finished event; the moment it ended comes back with the answer and the time
until the parent has it (~0.1 ms) counts against Rust; then the copies are flushed for every side (time to the disk).
Starting the Rust process isn't counted (C# starts none). Details, switches and the masked reports:
`rust/tools/speedproof/` (README, `table.py`, `results/`). Rerun: `Choppy.exe --benchmark out.json --thorough --network
--rust <rust choppy.exe>` in test mode, with nothing else running.

**Found and fixed (fix list 3.1):** every `Thread.Sleep` of the C# is Win32 Sleep, which wakes on the system timer's
tick (15.6 ms here: Sleep(100) is ~109 ms); Rust's `std::thread::sleep` is high-resolution (exactly 100 ms). A copy
ends only when its idle workers wake from their 100 ms naps, so Rust's "done" fell on 100 ms steps and C#'s on ~109 ms
steps: 1,000 small files within the NVMe were done at 0.509 s in every Rust run against 0.448 or 0.557 s for C#, and
small-file cases looked 5-20 % faster or slower by luck. Now every sleep goes through `sys::sleep_ms` (Win32 Sleep),
as .NET does; a phase trace of both engines shows the same steps since. The step 2 gate (`check.py`) passes again (33
scenarios + usage, all identical).

**Checked, not a porting difference:** the method (a C# A/A run - the C# engine in the child against itself - shows
the same spread); the tuner's trials on NVMe -> hard disk mixed (both engines try them, timing decides); the small-file
batch, CopyFileEx callback, size reservation and cached-tail code (line by line the same). Hard-disk small files:
the flush of 500 files onto D: / X: (both ~80 % full) swings +/-25 % from run to run; three 5-round runs of C>X small
gave Rust -5.5 %, +4.2 % and (pre-fix) +5.8 % against C#, and the A/A run showed C# against itself +6.6 % / +10.3 %.

**Gate: met.** One Thorough + Network run with the fixed engine (C: NVMe, D: / X: hard disks, F: Netac SSD in a 10 Gbps
USB case, N: the NAS), the E: run (the 10 Gbps MS YSUSP20 USB drive, plugged in during the step, woken first), and 5
rounds for every case that run left in doubt (C>D, C>X, N>C; C>E small / mixed, E>C small / mixed). Times = average
seconds to the disk; "Rust vs C#" = how much less time Rust took (below 0 = more); verdicts as the Benchmark judges
(4 % = Marginal Tie).

| Route | Case | Size | Rust | C# 1.48 | Windows | Rust vs C# | Rust vs Windows | C# vs Windows | Runs |
|---|---|---|---|---|---|---|---|---|---|
| C>C | big | 2 GB | 0.814 s | 0.813 s | 0.961 s | -0.1% | Faster +15.4% | Faster +15.5% | 3 |
| C>C | small | 190 MB, 1000 files | 0.645 s | 0.616 s | 3.482 s | -4.7% | Faster +81.5% | Faster +82.3% | 3 |
| C>C | mixed | 588 MB, 403 files | 0.395 s | 0.401 s | 1.597 s | +1.5% | Faster +75.3% | Faster +74.9% | 3 |
| C>D | big | 1 GB | 4.780 s | 4.822 s | 4.825 s | +0.9% | Tie +0.9% | Tie +0.0% | 5 |
| C>D | small | 94 MB, 500 files | 1.978 s | 2.051 s | 3.299 s | +3.6% | Faster +40.0% | Faster +37.8% | 5 |
| C>D | mixed | 588 MB, 403 files | 4.806 s | 4.905 s | 5.487 s | +2.0% | Faster +12.4% | Faster +10.6% | 5 |
| D>C | big | 1 GB | 5.634 s | 5.718 s | 5.669 s | +1.5% | Tie +0.6% | Tie -0.9% | 3 |
| D>C | small | 94 MB, 500 files | 2.037 s | 2.064 s | 5.606 s | +1.3% | Faster +63.7% | Faster +63.2% | 3 |
| D>C | mixed | 588 MB, 403 files | 4.155 s | 4.196 s | 6.894 s | +1.0% | Faster +39.7% | Faster +39.1% | 3 |
| D>D | big | 1 GB | 10.778 s | 10.735 s | 11.483 s | -0.4% | Faster +6.1% | Faster +6.5% | 3 |
| D>D | small | 94 MB, 500 files | 3.210 s | 3.257 s | 7.513 s | +1.4% | Faster +57.3% | Faster +56.6% | 3 |
| D>D | mixed | 588 MB, 403 files | 9.165 s | 9.551 s | 14.001 s | +4.0% | Faster +34.5% | Faster +31.8% | 3 |
| C>F | big | 2 GB | 2.399 s | 2.395 s | 2.999 s | -0.2% | Faster +20.0% | Faster +20.2% | 3 |
| C>F | small | 190 MB, 1000 files | 0.540 s | 0.592 s | 4.233 s | +8.8% | Faster +87.2% | Faster +86.0% | 3 |
| C>F | mixed | 588 MB, 403 files | 0.864 s | 0.843 s | 2.501 s | -2.5% | Faster +65.5% | Faster +66.3% | 3 |
| F>C | big | 2 GB | 2.176 s | 2.208 s | 2.227 s | +1.4% | Tie +2.3% | Tie +0.9% | 3 |
| F>C | small | 190 MB, 1000 files | 0.643 s | 0.627 s | 3.687 s | -2.6% | Faster +82.5% | Faster +83.0% | 3 |
| F>C | mixed | 588 MB, 403 files | 0.827 s | 0.813 s | 2.063 s | -1.7% | Faster +59.9% | Faster +60.6% | 3 |
| F>F | big | 2 GB | 2.740 s | 2.828 s | 4.186 s | +3.1% | Faster +34.5% | Faster +32.5% | 3 |
| F>F | small | 190 MB, 1000 files | 1.653 s | 1.587 s | 4.462 s | -4.2% | Faster +62.9% | Faster +64.4% | 3 |
| F>F | mixed | 588 MB, 403 files | 1.459 s | 1.454 s | 2.933 s | -0.3% | Faster +50.2% | Faster +50.4% | 3 |
| C>X | big | 1 GB | 5.222 s | 5.195 s | 5.116 s | -0.5% | Tie -2.1% | Tie -1.6% | 5 |
| C>X | small | 94 MB, 500 files | 2.901 s | 2.750 s | 3.626 s | -5.5% | Faster +20.0% | Faster +24.1% | 5 |
| C>X | mixed | 588 MB, 403 files | 5.866 s | 5.678 s | 6.471 s | -3.3% | Faster +9.3% | Faster +12.3% | 5 |
| X>C | big | 1 GB | 5.946 s | 5.829 s | 5.979 s | -2.0% | Tie +0.6% | Tie +2.5% | 3 |
| X>C | small | 94 MB, 500 files | 2.472 s | 2.545 s | 8.891 s | +2.9% | Faster +72.2% | Faster +71.4% | 3 |
| X>C | mixed | 588 MB, 403 files | 5.153 s | 5.144 s | 9.383 s | -0.2% | Faster +45.1% | Faster +45.2% | 3 |
| X>X | big | 1 GB | 11.612 s | 11.540 s | 12.093 s | -0.6% | Tie +4.0% | Faster +4.6% | 3 |
| X>X | small | 94 MB, 500 files | 4.409 s | 4.245 s | 11.287 s | -3.9% | Faster +60.9% | Faster +62.4% | 3 |
| X>X | mixed | 588 MB, 403 files | 10.497 s | 10.260 s | 16.536 s | -2.3% | Faster +36.5% | Faster +38.0% | 3 |
| C>N | big | 512 MB | 46.175 s | 46.449 s | 46.341 s | +0.6% | Tie +0.4% | Tie -0.2% | 3 |
| C>N | small | 50 MB, 250 files | 12.622 s | 12.542 s | 23.460 s | -0.6% | Faster +46.2% | Faster +46.5% | 3 |
| C>N | mixed | 588 MB, 403 files | 61.314 s | 62.254 s | 82.201 s | +1.5% | Faster +25.4% | Faster +24.3% | 3 |
| N>C | big | 512 MB | 10.160 s | 10.189 s | 10.348 s | +0.3% | Tie +1.8% | Tie +1.5% | 5 |
| N>C | small | 50 MB, 250 files | 2.408 s | 2.449 s | 27.130 s | +1.7% | Faster +91.1% | Faster +91.0% | 5 |
| N>C | mixed | 588 MB, 403 files | 17.459 s | 16.983 s | 54.414 s | -2.8% | Faster +67.9% | Faster +68.8% | 5 |
| N>N | big | 512 MB | 0.725 s | 0.732 s | 1.248 s | +1.0% | Faster +41.9% | Faster +41.3% | 3 |
| N>N | small | 50 MB, 250 files | 6.630 s | 6.443 s | 83.385 s | -2.9% | Faster +92.0% | Faster +92.3% | 3 |
| N>N | mixed | 588 MB, 403 files | 9.662 s | 9.923 s | 133.334 s | +2.6% | Faster +92.8% | Faster +92.6% | 3 |
| C>E | big | 2 GB | 2.513 s | 2.536 s | 3.009 s | +0.9% | Faster +16.5% | Faster +15.7% | 3 |
| C>E | small | 190 MB, 1000 files | 1.419 s | 1.478 s | 4.177 s | +4.0% | Faster +66.0% | Faster +64.6% | 5 |
| C>E | mixed | 588 MB, 403 files | 1.340 s | 1.320 s | 2.576 s | -1.5% | Faster +48.0% | Faster +48.8% | 5 |
| E>C | big | 2 GB | 2.100 s | 2.115 s | 2.182 s | +0.7% | Tie +3.8% | Tie +3.1% | 3 |
| E>C | small | 190 MB, 1000 files | 1.274 s | 1.296 s | 3.637 s | +1.7% | Faster +65.0% | Faster +64.4% | 5 |
| E>C | mixed | 588 MB, 403 files | 1.097 s | 1.123 s | 2.037 s | +2.3% | Faster +46.1% | Faster +44.9% | 5 |
| E>E | big | 2 GB | 4.606 s | 4.627 s | 5.410 s | +0.5% | Faster +14.9% | Faster +14.5% | 3 |
| E>E | small | 190 MB, 1000 files | 1.638 s | 1.617 s | 4.482 s | -1.3% | Faster +63.4% | Faster +63.9% | 3 |
| E>E | mixed | 588 MB, 403 files | 1.928 s | 1.887 s | 3.455 s | -2.2% | Faster +44.2% | Faster +45.4% | 3 |

- Rust: Faster 39 / Marginal Tie 9 / **Slower 0** of 48. C# 1.48 in the same runs: Faster 40 / Marginal Tie 8 / Slower 0.
  The one verdict that differs is X>X big (C# +4.6 % Faster, Rust +4.0 % Tie: 0.6 % apart, on the line).
- Every case added up: Rust 300.8 s, C# 1.48 301.6 s, Windows 672.3 s (Rust 0.3 % less time than C#).
- No case is apart beyond its runs' spread; nothing is newly Slower.
- First run (before the fix, a new Rust process per copy, `results/full`): also never Slower against Windows except
  C>X big 1 GB (-6.6 %), which 5 rounds after the fix put at a Tie (Rust -2.1 %, C# -1.6 %).

**Only the user can do:** sign off on the table above (step 4 doesn't wait for it).

**For after the switch-over** (1.48 behaviour kept on purpose):
- A copy ends only when its idle workers wake from their 100 ms naps (`WorkerLoop`): every copy's "done" comes up to
  ~110 ms late. Waking them when the last file is taken would end it at once.

**For the next steps:**
- `src/Bench/EngineChild.cs` and the three-way parts of Benchmark.cs / BenchReport.cs / Cli.cs (`--rust`,
  `--rust-fresh`, `--size-down`, `RustResults`, `rustSummary` / `rustResults` in the JSON, `run:` / `result:` log lines,
  `BenchRun.After`, `CHOPPY_BENCH_TRACE`) are step 3's harness: **not ported**. Step 4 ports the Benchmark as 1.48 has it
  (in-process ChoppyCopy, the 1.48 JSON / HTML). The Rust `--benchcopy` switch stays as a test switch, so the C#
  Benchmark can compare any later engine change.
- Any C# `Thread.Sleep` that later steps port goes through `sys::sleep_ms` (fix list 3.1), never `std::thread::sleep`.

## Decisions

- 2026-11-24: Build Tools 2026 (current winget package) rather than 2022.
- 2026-11-24: look test B uses Slint's femtovg renderer (see above).
- 2026-11-24: **the user picked A** (own UI on Direct2D + DirectWrite): "open in is BLAZING FAST", lowest CPU,
  exe about 15 times smaller than B, smallest memory, and the window looks impeccable. B deleted; A stays in
  `look-test/` as step 4's starting point.
- 2026-11-24 (step 1): `XxHash64.HashFile` moves to step 2: it reads through BigCopy's RingReader (threads,
  overlapped reads), which step 2 writes on Max effort, and its self-test checks are step 2's anyway.
- 2026-11-24 (step 1): paths and names are UTF-16 (`WString`), as in C#: nothing is lost on odd names, and lengths
  count as NTFS and the C# code count them.
- 2026-11-25 (step 2): RingReader's buffers are owned `Slot`s handed between the reader thread and the writer through
  two queues (C#'s two semaphores); the unsafe part stays in `sys/ring.rs`.
- 2026-11-25 (step 2): in paths that only run when something fails, the port is safer than the C# on purpose (fix list
  2.63, 2.64); nothing changes when all goes well. C#'s doubtful behaviours are kept and listed under "For after the
  switch-over" (step 2).
- 2026-11-25 (step 2): the review pass ran as 5 reviewer agents on Extra in parallel (Ultracode can't be switched on
  while the user is away).
- 2026-11-25 (step 3): C# stays in the Benchmark's own process (the proven way); the Rust engine runs in one warm child
  for the whole run. A new process per copy would count a process start for neither but a fresh .NET would pay its
  JIT inside the timed copy (measured: a C# child is slower than C# in-process), so a warm child is the fair match.
- 2026-11-25 (step 3): reruns keep the full run's file sizes (`--size-down 1`): fewer routes would otherwise get bigger
  files from the time budget.

## Next session

**Step 4 (all windows), on Extra without Ultracode.** Read PLAN.md (step 4's gate, file map, the UI layer from look test
A) and "For the next steps" of steps 2 and 3. The Benchmark window and `src/Bench` are ported as 1.48 has them - not
step 3's three-way harness. Sleeps: `sys::sleep_ms`. The user still signs off step 3's table; that doesn't hold step 4.
