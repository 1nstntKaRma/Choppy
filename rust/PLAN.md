# Choppy in Rust - the plan

Decided 2026-11-24: Choppy moves from C# (.NET Framework 4.8 + WPF, ~14k lines in `src/`) to Rust, for faster
opening and less memory. Copy speed stays the same (drive-bound; the same Win32 calls). The Rust exe will be bigger
than today's < 1 MB exe (the .NET runtime ships with Windows, Rust carries what it uses).

- All Rust work lives on the **`rust`** branch (pushed to GitHub as a backup). No releases until switch-over:
  `release.yml` only runs on pushes to `main`.
- **C# full freeze:** never touch `main`, never bump `VERSION`. C# experiments (the plan dump, the Benchmark running
  the Rust engine) happen on the `rust` branch only.
- **One step per session.** Each session starts from [PROGRESS.md](PROGRESS.md) and updates it at the end.

## Rules

1. **Port, don't redesign.** The C# code is the spec. Keep names and structure close (`CopyJob` -> `copy_job.rs`,
   `Plan.Choose` -> `plan::choose`), and carry the "why" comments over: they are the fix history. Improvements only
   after switch-over.
2. **Read each C# file fully once, while porting it**, and fill that step's fix list from it (below).
3. **The look can't get worse.** Today's WPF design (Theme.xaml) is kept exactly: colors, fonts, sizes, margins.
4. **Keep everything the outside world sees:**
   - exe name and install path `%LOCALAPPDATA%\Programs\Choppy\Choppy.exe`; the command line and every switch
     (`copy|move|paste`, `--tray`, `--resume`, `--takeover`, `--menu`, `--uninstall`, `--quit`, `--revive`, `--cli`,
     `--selftest`, `--where`, `--usb`, `--benchmark ...`, `--shellcopy`, the engine test switches);
   - the HKCU settings key and value names. **Never raise `Layout` / `SettingsVersion` 2** (that would reset the
     user's settings again);
   - `%LOCALAPPDATA%\Choppy` Logs and Jobs formats: a job queued by 1.48 resumes in Rust;
   - the admin hand-off files (`.admin` / `.progress` / `.control` / `.result`, SHA-256 on the command line);
   - test mode: `CHOPPY_DATA`, `CHOPPY_PARK`, `CHOPPY_TESTHOOK`, `CHOPPY_TEST_AUTOCLOSE`, `CHOPPY_FAKE_USB`;
   - the test-hook protocol (`shot`, `press`, `conflict`, `dialog`, `about`, `settings`, `benchmark ...`, `set`,
     `state`, `enqueue`, `quit`) and its answers;
   - the self-test check names (the list below), so build.ps1 and release.yml keep working unchanged.
5. Never install or update the installed Choppy. Test-mode instances from a scratch copy only.
6. Never write Choppy's former name and never name other copy tools in public text (the Ultracopier interop code
   stays, generic in the UI).
7. Commits as 1nstntKaRma, plain messages.

## Rust setup

- Target `x86_64-pc-windows-msvc`; `rust/.cargo/config.toml` sets `+crt-static`, so no VC++ runtime is needed.
- The official `windows` crate (+ `windows-sys` only if a call needs it) and as few other crates as possible; each
  one is listed with its reason in `rust/choppy/Cargo.toml`.
- `unsafe` only in the `sys` module, behind safe wrappers. Everything else is `#![deny(unsafe_code)]`.
- Dedicated threads like the C# code (workers, the pipeline, the pipe server, the hook thread). No async runtime.
- Panics are caught at every thread boundary (`catch_unwind`), so a job fails safely: a `.chpart` file is never
  renamed into place after a panic.
- `build.rs` embeds `src/app.manifest` (PerMonitorV2, longPathAware, supportedOS), `Choppy.ico` and the version
  info. FileVersion must equal `VERSION` (`1.48` -> `1.48.0.0`) - release.yml checks it.
- Release profile: `lto = true`, `codegen-units = 1`, `panic = "unwind"` (needed for catch_unwind), `opt-level = 3`,
  `strip = "symbols"`.
- Workspace `rust/`: `choppy/` = the app (modules `sys`, `engine`, `bench`, `explorer`, `ui`, `cli`, `testhook`),
  `tools/` = scripts for screenshot parity and numbers (WPF layout dump, test-mode demo copy, window driver, measure),
  `look-test/` = the step-0 prototypes (removed once the pick is recorded).

## Steps and gates

A step is done only when its gate is met **and** every fix-list entry of that step is ported and checked.

| # | Step | Gate |
|---|------|------|
| 0 | Tools + UI look test | Rust + MSVC build a hello-world. The copy window built twice (A = Direct2D/DirectWrite via `windows`, B = Slint), compared with today's WPF window (pictures at 125 % and 100 %, open time, memory, CPU, exe size). **The user picks A or B by looking.** |
| 1 | Basics | Native, Paths, Settings, XxHash64, Model, VolumeProbe, UsbLink, Scanner, Plan ported. Their self-tests pass. A **plan dump** for every drive pair on the user's PC matches C# 1.48 line for line: the same dump command (`--plan-dump out.txt`) is added to both; the C# side on the `rust` branch only. `--usb out.txt` matches too. |
| 2 | Copy engine | BigCopy, CopyJob, Handoff, JobManager, the headless copy/move parts of Cli. On a fixture set, the destination trees are identical to C#'s: contents, sizes, dates, attributes, alternate data streams (Zone.Identifier), long and unicode names, "name - Copy (2)" conflicts, the FAT32 4 GB limit, ReFS same-volume cases. The engine self-tests pass. |
| 3 | Speed proof | The proven C# Benchmark (on the `rust` branch only) learns to run the Rust engine as the Choppy side. A Thorough + Network run matches 1.48 on every route with no new Slower. HDD placement noise reaches ~20 %, so disputed routes get 5 rounds. **The user signs off.** |
| 4 | All windows | Theme, Controls, Format, Helpers, Main (compact/extended), Settings, Conflict, the Benchmark window + Bench/* logic + HTML/JSON report + the ShellCopy child, TestHook, tray. Screenshot parity at 125 % and 100 % on both monitors. **The user approves.** |
| 5 | Explorer / system | App, Program, PasteHook, Takeover, IconRefresh, single-instance mutex + pipe hand-off, Run key, right-click verb, Default file manager, Ultracopier parking, admin hand-off, keep-awake, `--where`. Opens faster than 1.48 (already running 0.14-0.20 s, not running ~1.1 s), measured with the window-latency method (memory: choppy-testing-notes). |
| 6 | Switch-over | Only build.ps1's BUILD step changes (cargo); its install part stays. The zip layout `Choppy-vX.zip` with `bin\Choppy.exe`, the ChocoPlus contract (`^Choppy-v.*\.zip$`, `build.ps1 -NoBuild -Shell Apply\|Revert\|Install` with -NonInteractive), release.yml's version-stamp check, the self-test, a full Thorough benchmark, a Windows Defender / SmartScreen check of the new exe on the user's PC, and **the user using it themselves before release**. The version number (2.00?) and a beta are decided then. |

## File -> step map

Every C# file belongs to exactly one step. Cli.cs and App.cs are split by command / part, as listed.

| File | Lines | Step | Notes |
|------|------:|:----:|-------|
| src/Engine/Native.cs | 439 | 1 | becomes `sys`; later steps add the calls they need there |
| src/Engine/Paths.cs | 127 | 1 | |
| src/Engine/Settings.cs | 282 | 1 | HKCU key + names, test-mode ini, `Layout` 2 |
| src/Engine/XxHash64.cs | 134 | 1 | incl. `HashFile` (buffered / unbuffered) |
| src/Engine/Model.cs | 125 | 1 | JobSpec, VolumeInfo (MaxFileSize), enums |
| src/Engine/VolumeProbe.cs | 214 | 1 | |
| src/Engine/UsbLink.cs | 298 | 1 | + `CHOPPY_FAKE_USB` |
| src/Engine/Scanner.cs | 329 | 1 | |
| src/Engine/Plan.cs | 246 | 1 | |
| src/app.manifest, src/Choppy.ico | - | 1 | embedded by build.rs (with the version info) |
| src/Cli.cs: `--usb` (Usb), the new `--plan-dump` | - | 1 | |
| src/Engine/BigCopy.cs | 633 | 2 | |
| src/Engine/CopyJob.cs | 1989 | 2 | incl. `TempName` (.chpart) |
| src/Engine/Handoff.cs | 353 | 2 | the admin hand-off files |
| src/JobManager.cs | 133 | 2 | |
| src/Cli.cs: `--cli copy\|move` (Run's job part, EngineOption, ParsePolicy, Report, Esc), `--selftest` (SelfTest, PipelineChecks, WriteStream, ReadStream) | - | 2 | the self-test harness lands here; each step adds its checks |
| (C# on the `rust` branch: Bench/Benchmark.cs runs the Rust engine as the Choppy side) | - | 3 | not a port |
| src/UI/Theme.xaml | 512 | 4 | |
| src/UI/Controls.cs | 265 | 4 | SpeedGraph, Donut, PathText |
| src/UI/Format.cs | 120 | 4 | |
| src/UI/Helpers.cs | 346 | 4 | DWM frame, work area, ShellThumbs, ChoppyIcon, WholeNumberBox, Dialogs |
| src/UI/MainWindow.xaml + .cs | 243 + 883 | 4 | |
| src/UI/SettingsWindow.xaml + .cs | 210 + 223 | 4 | |
| src/UI/ConflictWindow.xaml + .cs | 106 + 210 | 4 | |
| src/UI/BenchmarkWindow.xaml + .cs | 230 + 943 | 4 | |
| src/Bench/Benchmark.cs, BenchFiles.cs, BenchReport.cs, ShellCopy.cs | 659 + 384 + 698 + 151 | 4 | HTML/JSON report formats unchanged |
| src/TestHook.cs | 178 | 4 | same commands and answers |
| src/App.cs: the tray (EnsureTray, TrayOpen, UpdateTrayMenu, SetTrayText, TrayBalloon, QuitFromTray) | - | 4 | |
| src/Cli.cs: `--benchmark` (BenchCli, Full), `--shellcopy`, BenchChecks, MakeJunction | - | 4 | |
| src/App.cs: everything else (start-up, warm-up, windows, Claim mutex, pipe hand-off, admin relaunch, `--takeover/--menu/--uninstall/--quit/--revive`) | 661 | 5 | |
| src/Program.cs | 66 | 5 | |
| src/Explorer/PasteHook.cs | 335 | 5 | |
| src/Explorer/Takeover.cs | 387 | 5 | Run key, right-click verb, Default file manager, Ultracopier parking |
| src/Explorer/IconRefresh.cs | 97 | 5 | |
| src/Cli.cs: `--where` (Where) | - | 5 | |
| src/Choppy.csproj, nuget.config | - | 6 | replaced by cargo |
| build.ps1 | - | 6 | only the BUILD step changes |
| .github/workflows/release.yml, build.yml | - | 6 | |
| Activate Choppy.cmd, Revert to Windows Copy.cmd | - | 6 | check they still work |

## Self-test checks -> step

`Choppy.exe --selftest out.txt` prints these names (71 in the source; 96 lines at run time, because the pipeline
checks run for several sizes). The Rust self-test prints the same names in the same order.

**Step 1 (27):** `xxh64 empty`, `xxh64 'a'`, `xxh64 'abc'`, `xxh64 fox`, `xxh64 streaming`, `long local`,
`long unc`, `short unc`, `inside`, `not inside`, `parent of root`, `drive root from Explorer (C:")`,
`folder arg with quote`, `parent`, `struct size`, `numbered`, `numbered noext`,
`same ReFS drive: Windows' own copy (block clone)`, `ReFS to another drive: the pipeline`,
`same ReFS drive, a move: renamed in place`, `disk number: a disk counts, a file system device doesn't`,
`one hard disk: whole-file batches of 1 MB reads, 64 MB writes, room reserved, cached end only once read, no trials`,
`one USB drive: 32 MB batches with trials`, `two hard disks unchanged`,
`hard disk into an SSD: 1 MB reads, 4 out at once, no trials`, `SSD into a hard disk unchanged`,
`FAT32 holds files up to 4 GB`.

**Step 2 (16):** `temp`, `temp long`, `pipeline copy <n> B`, `pipeline dates <n> B`, `hash unbuffered <n> B`,
`hash buffered <n> B` (n = 0, 1, 4096, 2097152, 3158073), `pipeline joined writes <n> B`, `pipeline phased <n> B`,
`pipeline phased, read depth + write step <n> B`, `pipeline no reservation <n> B` (n = 3145728, 5243657, 7344128,
123457), `pipeline cached tail`, `pipeline cached end after read`, `pipeline stream`, `pipeline never replaces`,
`pipeline stop`, `pipeline: <error>` (the catch-all).

**Step 4 (27):** `bench: same times = Marginal Tie`, `bench: less time = Faster (+17%)`,
`bench: more time = Slower (-20%)`, `bench: 3% apart = Marginal Tie`,
`bench: 9% behind with scattered runs is still Slower`, `bench: averages`, `bench: seconds`,
`bench: routes (7 for 3 drives, the system drive first, no route between two others)`,
`bench: sizes by speed (2 GB + 1,000 small files at 350 MB/s and more, 1 GB + 500 below)`,
`bench: two cases per route`, `bench: a time budget (many hard disks: a step smaller)`,
`bench: thorough = 4 GB / 2 GB, 2,000 / 1,000 small files, a mixed folder`,
`bench: small files 4 KB - 1 MB, the same set every time`, `bench: scored (on the disk faster, done slower; averages)`,
`bench: summary line`, `bench: each case says Tie, the summary Marginal Tie`,
`bench: Never Slower only when a whole run ended without one`,
`bench: totals (the measured cases only; each one's speed over the times added up)`, `bench: time saved as a span`,
`bench: JSON well formed`, `bench: HTML escaped`, `bench: a folder that isn't ours gets a numbered name`,
`bench: leftovers removed, links not followed`, `bench: a folder without the marker untouched`,
`bench: Windows' copy (IFileOperation, child process)`, `bench: test folders: <error>`,
`bench: Windows' copy: <error>` (the two catch-alls).

**Step 5 (1):** `Ctrl+V hook on its own thread, refreshed (<n>)` (or `SKIP Ctrl+V hook (no desktop to hook here)`).

Order in the 1.48 output (keep it): xxh64 + paths + temp, the pipeline block, the Plan checks, the Ctrl+V hook, the
bench checks, then `ALL PASS` or `<n> FAILED` (exit code 0 / 2).

## Fix list (ledger)

Built per step from the files that step ports, while reading each C# file fully once. One row per rule or fix the
code carries (usually a "why" comment, a special case, a magic number with a reason). A step is done only when all
its rows are `checked`.

| id | rule / fix (one line) | C# file:line | Rust location | how it's tested | status |
|----|-----------------------|--------------|---------------|-----------------|--------|
| 1.x | (filled in step 1) | | | self-test name or side-by-side check | ported / checked |
| 2.x | (filled in step 2) | | | | |
| 3.x | (filled in step 3) | | | | |
| 4.x | (filled in step 4) | | | | |
| 5.x | (filled in step 5) | | | | |
| 6.x | (filled in step 6) | | | | |

## UI look test (step 0)

Today's copy window (MainWindow.xaml + Theme.xaml + Controls.cs + Helpers.cs) built twice with equal care:

- **A** = our own UI on Direct2D + DirectWrite through the `windows` crate (`rust/look-test/a-d2d`).
- **B** = Slint (`rust/look-test/b-slint`). Its free (royalty-free) license needs a "Made with Slint" badge on the
  README / download page, or an AboutSlint box in the app.

Both show the README demo (D: 4TB - Shadow `D:\Home Videos` -> X: 4TB - uTorrent `X:\Backup\Home Videos`,
Adaptive · 1 stream, 57 MB/s limit, the four demo videos + 2 small files) animated every 100 ms, both views, the
custom title bar, hover/pressed states, the quick toggles, the speed-limit box, the Queue pop-up, keyboard focus,
dragging and closing. Measured against WPF: open time, memory, CPU while animating, exe size. The result and the
user's pick are in PROGRESS.md.
