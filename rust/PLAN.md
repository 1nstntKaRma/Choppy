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
  `look-test/` = the step-0 prototype A (`a-d2d` + `common`), kept as the starting point of step 4's windows.
- **UI: our own small UI layer on Direct2D + DirectWrite through the `windows` crate** (look test A, picked by the
  user 2026-11-24). No UI framework. Step 4 moves look-test A's layers (`gfx`, `ui`, `textbox`, `popup`, `controls`)
  into `choppy/src/ui` (the Direct2D / DirectWrite calls into `sys`) and builds every window on them, laid out with
  WPF's rounding (see PROGRESS.md, step 0) and checked against `tools/wpflayout.ps1`.

## Steps and gates

A step is done only when its gate is met **and** every fix-list entry of that step is ported and checked.

| # | Step | Gate |
|---|------|------|
| 0 | Tools + UI look test | Rust + MSVC build a hello-world. The copy window built twice (A = Direct2D/DirectWrite via `windows`, B = Slint), compared with today's WPF window (pictures at 125 % and 100 %, open time, memory, CPU, exe size). **The user picks A or B by looking.** Done: **A** (2026-11-24). |
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
| src/Engine/XxHash64.cs | 134 | 1 | the hash; `HashFile` moved to step 2 (it reads through BigCopy's RingReader; its checks are step 2's) |
| src/Engine/Model.cs | 125 | 1 | JobSpec, VolumeInfo (MaxFileSize), enums |
| src/Engine/VolumeProbe.cs | 214 | 1 | |
| src/Engine/UsbLink.cs | 298 | 1 | + `CHOPPY_FAKE_USB` |
| src/Engine/Scanner.cs | 329 | 1 | |
| src/Engine/Plan.cs | 246 | 1 | |
| src/app.manifest, src/Choppy.ico | - | 1 | embedded by build.rs (with the version info) |
| src/Cli.cs: `--usb` (Usb), the new `--plan-dump` | - | 1 | |
| src/Engine/BigCopy.cs | 633 | 2 | `PipeTuning` + `PipeTuner.Describe` came in step 1 (Plan picks a PipeTuning) |
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
| 1.1 | Windows' struct layouts: WIN32_FIND_DATAW 592 bytes, WIN32_FILE_ATTRIBUTE_DATA 36 (FILETIME at 4-byte packing) | Native.cs:106 | sys/file.rs `struct_sizes` | self-test `struct size` | checked |
| 1.2 | ErrorText's own words for 17 errors (gone, folder gone, access denied, open elsewhere, locked part, full, too big for the file system, network dropped x9, name not allowed, path too long, bad sector x2, Stopped) | Native.cs:408 | sys.rs `error_text` | plan dump `errors`: codes 0-1999 + 11 more identical | checked |
| 1.3 | Any other error: Win32Exception's message + " (code)", the code as C#'s int | Native.cs:435 | sys.rs `error_text` | same | checked |
| 1.4 | Win32Exception's message (.NET 4.8): FormatMessage into 256 characters, 4 times more while it doesn't fit (up to 64 K), trailing white space / control characters / periods cut, "Unknown error (0x..)" | System.dll Win32Exception | sys/text.rs `win32_message` | same (the dump found the retry: codes 357, 384, 560... are longer than 256) | checked |
| 1.5 | FILE_SUPPORTS_BLOCK_REFCOUNTING = a ReFS volume can clone blocks | Native.cs:405 | sys.rs | self-test `same ReFS drive...` x3 | checked |
| 1.6 | FixArg: Explorer's drive root `C:\"` arrives as `C:"` and becomes `C:\`; stray quotes and white space dropped | Paths.cs:12 | engine/paths.rs `fix_arg` | self-test `drive root from Explorer (C:")`, `folder arg with quote`; dump `paths` | checked |
| 1.7 | ToLong: relative paths against the start folder, Path.GetFullPath, trailing `\` trimmed but roots kept, `\\?\` / `\\?\UNC\` prefix; `\\?\` paths only trimmed, `\\.\` untouched | Paths.cs:25,52 | paths.rs `to_long`, `trim_end`; main.rs `set_start_dir` | self-test `long local`, `long unc`; dump `paths` (51 inputs) | checked |
| 1.8 | What .NET's GetFullPath adds to Win32's: invalid characters (`"`, `<`, `>`, `\|`, control), white space only, wildcards, stray colons -> the C# keeps the path as given; white space at the end trimmed beyond Win32's spaces | mscorlib Path | paths.rs `net_full_path` | dump `paths` (`a*b`, `a\|b`, `a:b`, `C:\x\f:s`, `:x`, `1:\x`, NBSP, U+2028) | checked |
| 1.9 | 8.3 short names (`PROGRA~1`) expanded as .NET 4.8 does, its buffer quirk included: where only part of the path exists, a path of 15 characters or less stays short, a longer one expands one level up | mscorlib PathHelper | paths.rs `expand_short_names` | dump (8 `PROGRA~` inputs); .NET measured in PowerShell 5.1 for lengths 13-36 | checked |
| 1.10 | ToShort: `\\?\UNC\` (any case) -> `\\`, `\\?\` removed | Paths.cs:44 | paths.rs `to_short` | self-test `short unc`; dump | checked |
| 1.11 | Parent: a drive root has none; a trailing `\` is skipped; `C:` gets its `\` back | Paths.cs:77 | paths.rs `parent` | self-test `parent`, `parent of root`; dump | checked |
| 1.12 | IsSameOrInside / SamePath: trailing `\` ignored, OrdinalIgnoreCase through Windows' upper-case table | Paths.cs:89,97 | paths.rs; sys/text.rs `equal_ignore_case` | self-test `inside`, `not inside`; dump pairs (Ä/ä, ß/SS, ı/I, Σ/σ) | checked |
| 1.13 | NumberedName "name (2).ext"; folders, names without an extension or starting with a dot get it at the end | Paths.cs:107 | paths.rs `numbered_name` | self-test `numbered`, `numbered noext`; dump | checked |
| 1.14 | Combine, FileName, IsUnc, Exists, DirExists as in C# | Paths.cs:64-125 | paths.rs | dump (`name`, `unc`, `exists` columns) | checked |
| 1.15 | Paths stay UTF-16 like C# strings: a lone surrogate survives, lengths count UTF-16 units (NTFS's 255) | C# string | wstr.rs `WString` | dump (emoji / ü / é names in paths and scans) | checked |
| 1.16 | AppPaths: CHOPPY_DATA = test mode (data, Jobs, Logs, settings.ini there), else %LOCALAPPDATA%\Choppy | Settings.cs:12 | engine/settings.rs `app_paths` | every dump runs in test mode | checked |
| 1.17 | Before any command: the start folder kept, the data folder made and used as the current folder (relative output lands there) | Program.cs:17-24 | main.rs | dump `scan-relative` | checked |
| 1.18 | HKCU\Software\Choppy, the same 13 value names, all REG_SZ text | Settings.cs:55,89 | settings.rs `save_to`, `write` | cargo test `settings_in_the_registry` (a scratch key, never the real one) | checked |
| 1.19 | Layout 2: no or an older SettingsVersion -> the defaults once (ExplorerTakeover and UltracopierRunBackup kept), saved; **never raise it** | Settings.cs:44,77 | settings.rs `LAYOUT`, `load_from` | dump set-old, set-bad (layout 1), set-num (layout 3 kept), set-v2 | checked |
| 1.20 | Defaults: Ask me, close at completion after 0 s, Check every copy off, speed limit off at 12 MB/s (100.663296 Mbps), Auto, compact | Settings.cs:32-42 | `Settings::default` | dump (no settings.ini); cargo test | checked |
| 1.21 | Reading rules: VerifyCopies "1", CloseWhenDone anything but "0", CloseAfterSeconds 0-999, LimitValueMbps > 0, a speed limit that is on sets the value, runs 1-99 else 0, View "extended", names in any case | Settings.cs:64-76,111 | `load_from`, `runs` | dump set-edge, set-bad, set-num | checked |
| 1.22 | Policy read as Enum.TryParse: names as written, numbers, names joined by commas add up ("Replace, Skip" = KeepBoth) | Settings.cs:65 | engine/model.rs `ConflictPolicy::parse` | dump set-edge; cargo test | checked (on purpose: a number no policy has, like 7, is refused; C# kept an unnamed value) |
| 1.23 | BenchDrives is no longer kept: deleted on save | Settings.cs:50,150 | settings.rs `GONE` | cargo test | checked |
| 1.24 | Registry values as .NET reads them: one closing NUL dropped, REG_EXPAND_SZ expanded, DWORD as int, QWORD as long, lists and bytes as their type's name | Settings.cs:132 | settings.rs `registry_text` | cargo test | checked |
| 1.25 | Numbers as .NET Framework writes and reads them: "G" = 15 digits rounded half up, `1E-05` / `1E+15`; "F<n>" (half away from zero); NumberStyles.Float parsing | Settings.cs:66,95 | engine/netfmt.rs | cargo tests with values measured on .NET 4.8; dump set-g1, set-g2, journal `limit` | checked |
| 1.26 | settings.ini: ReadAllLines (BOM, CR LF / LF / CR), key = text before the first `=`, never empty; written `key=value` CR LF, UTF-8 without BOM | Settings.cs:118,139 | settings.rs `read`, `write`, `read_all_lines` | dump set-crlf, set-edge; cargo test | checked |
| 1.27 | Job record: written when queued, UTF-8, flushed to the disk, then renamed into place; keys op, dest, verify, policy, limit, origin, src, file, dir + extra lines | Settings.cs:183 | settings.rs `journal::write` | dump `journal` (the file's text identical) | checked |
| 1.28 | An administrator copy has no .job record (it runs from its .admin) | Settings.cs:163,179 | `journal::save`, `save_soon` | dump `records admin 0` | checked |
| 1.29 | SaveSoon writes in the background; Remove of a record still being written waits for it (else it would come back as an unfinished copy) | Settings.cs:172,253 | `journal::save_soon`, `remove` (model.rs `Recorded`) | dump `records soon 0` | checked (the record's text is taken when SaveSoon is called; C#'s task read the job a moment later) |
| 1.30 | Reading a record: dest and src through ToLong (repairs C:" from older records), Depth from the backslashes, nothing to do = no job | Settings.cs:210 | `journal::read` | dump `journal` read-back | checked |
| 1.31 | LoadUnfinished: `*.job` by Windows' own pattern (it also matches `.jobx`, as C# does), resume mode on, a record with nothing to do deleted | Settings.cs:259 | `journal::load_unfinished` | dump `unfinished` lines, `after load empty 0` | checked (its Handoff.CleanStale call is 2.3) |
| 1.32 | xxHash64, seed 0, streamed in any pieces (a 32-byte tail between them) | XxHash64.cs:10 | engine/xxhash64.rs | self-test `xxh64 ...` x5 | checked |
| 1.33 | JobSpec.Id = the first 12 hex digits of a new GUID | Model.cs:17 | model.rs `new_id` | cargo test | checked |
| 1.34 | Title: "Copy 3 files again" ("again" not for an administrator copy), "Move photo.jpg", "Copy 2 items" | Model.cs:44 | model.rs `title` | dump titles | checked |
| 1.35 | Test switches default to Adaptive (PipePhased -1, TailMB -1) | Model.cs:39,41 | `JobSpec::default` | cargo test | checked |
| 1.36 | FileTimes.ToLocal: the date's daylight saving; the standard offset where Windows has no rule (1601); clamped at .NET's last date; outside .NET's range = MinValue | Model.cs:120 | model.rs `file_times`; sys/text.rs `file_time_to_local` | dump `time` lines (8 values; the dump found 1601 and 9999) | checked |
| 1.37 | GetVolumePathName on a path that may not exist yet: walk up until it works; else the UNC root or the first 3 characters | VolumeProbe.cs:65 | engine/volume_probe.rs `probe` | dump `scan-no-dest` | checked |
| 1.38 | UNC or DRIVE_REMOTE = Network: id `net:` + root (UNC lower case, a letter upper case); round trip = best of 3 metadata calls on a name that never exists (no cached answer) | VolumeProbe.cs:78,196 | `probe`, `measure_rtt` | dump `UNC` drive, scan-unc, scan-unc-src (\\localhost\C$) | checked |
| 1.39 | A local volume's id = `vol:` + serial (X8), `root:` when unknown; the same id = the same volume | VolumeProbe.cs:90 | `probe` | dump `drives` | checked |
| 1.40 | The volume opened with no access rights; USB / SD / MMC bus or a removable drive = pluggable (removable: nothing waits in memory for it); a USB SSD with no seek penalty = SSD, else USB; seek penalty -> HDD / SSD; NVMe -> SSD; a descriptor that didn't fit still counts (32 bytes or more) | VolumeProbe.cs:137 | `probe_disk` | dump `drives` (C: NVMe, D:/X: HDD, F: USB SSD, G: cloud drive) | checked |
| 1.41 | A disk number only from a disk device: Google Drive's G: answers as type 8 "number 0", which looked like X:'s disk 0 | VolumeProbe.cs:185 | `disk_number` | self-test `disk number...`; dump G: disk -1 | checked |
| 1.42 | Bytes per sector from GetDiskFreeSpace when 512 or more (unbuffered I/O alignment) | VolumeProbe.cs:106 | `probe` | dump (`sector`, plan `align`) | checked |
| 1.43 | MaxFileSize (FAT* = 4 GB less a byte), CanClone, IsSlowSeek (HDD, USB, disc), KindText | VolumeProbe.cs:28-54 | `VolumeInfo` | self-test `FAT32...`; dump | checked |
| 1.44 | SameDisk: the same volume or the same disk number, never on a network | VolumeProbe.cs:132 | `same_disk` | self-test; dump plans | checked |
| 1.45 | USB: disk -> its device node (the first 64 disk interfaces) -> up to 8 levels to the hub port; UASPStor on the way = UASP | UsbLink.cs:100,177 | engine/usb_link.rs `for_disk`, `disk_dev_inst` | `--usb` identical (F: 10 Gbps, UASP) | checked |
| 1.46 | The hub is asked (GENERIC_WRITE): CONNECTION_INFORMATION_EX (bcdUSB, speed capped at 5 Gbps, connected?), then EX_V2 (port does USB 3; runs at / can do 5 or 10 Gbps) | UsbLink.cs:140 | `ask` | `--usb`; dump `drives` | checked |
| 1.47 | HeldBack, Advice, Name, Short, CeilingMBps, Aka | UsbLink.cs:26-97 | usb_link.rs | dump (fake link advice; `usb 0-4` table) | checked |
| 1.48 | CHOPPY_FAKE_USB=speed,capable[,port3] fakes every USB drive's link | UsbLink.cs:103 | `for_disk` | dump with 2,3,1 and 4,4; `--usb` with 2,3,0 | checked (on purpose: a malformed value fakes nothing; C# threw) |
| 1.49 | SP_DEVICE_INTERFACE_DETAIL_DATA_W's cbSize is 8 in a 64-bit process | UsbLink.cs:195 | sys/device.rs `device_interfaces` | `--usb` | checked |
| 1.50 | Scan: the sources first (8 listings at once on a network, else 1), then only the destination folders that exist (8 / 2) | Scanner.cs:9; CopyJob.cs:403 | engine/scanner.rs `run` | dump scans with 1/2 and 4/4 threads, 8 on \\localhost | checked |
| 1.51 | Copy into its own folder = "name - Copy", "name - Copy (2)" (as Explorer); a move there = "Already in this folder" | Scanner.cs:62,315 | `copy_name` | dump scan-same-folder, scan-move-same | checked |
| 1.52 | A folder can't be copied into itself | Scanner.cs:69 | `run` | dump scan-into-itself | checked |
| 1.53 | Junctions and folder links inside a folder are not followed (they can loop or point anywhere), noted | Scanner.cs:221 | `list_source` | dump scan-move-all | checked |
| 1.54 | An unreadable folder = "Couldn't read folder ... - <error>"; an empty one is no error; a missing source = "Not found: ..." | Scanner.cs:56,207 | `list_source`, `run` | dump scan-unreadable (C:\System Volume Information), scan-main | checked |
| 1.55 | Conflicts by name in any case, one listing per folder: there, a folder, size, date; "<name>.chpart" = a leftover | Scanner.cs:269 | `mark` | dump scan-main (Same.TXT / same.txt, LAST.txt.CHPART) | checked |
| 1.56 | A file where a folder goes = DestIsFile (not listed); existing folders = DestExisted and listed | Scanner.cs:280 | `scan_destination` | dump scan-main | checked |
| 1.57 | Order: a folder's files, then its subfolders, depth first; median of the sorted sizes; the 4 largest (equal ones keep their order) | Scanner.cs:100,123 | `flatten`, `top_largest` | dump scans | checked |
| 1.58 | A pool of threads over a growing set of folders, done when nothing is pending; cancel stops the listing | Scanner.cs:151 | `run_pool` | dump 4/4 threads; cargo test `canceled_scan_lists_nothing` | checked |
| 1.59 | Same volume + a move (not a retry) = instant move (rename only) | Plan.cs:58 | engine/plan.rs `choose` | dump: every pair x copy / move / retry move | checked |
| 1.60 | Network: 1 / 4 / 8 (more than 16 files, median under 8 MB) / 16 (more than 64) in flight, tuning up to 64, no pipeline | Plan.cs:68 | `choose` | dump `UNC` pairs x 12 shapes | checked |
| 1.61 | The same ReFS volume = Windows' own copy (block clone), 1 / 4 / 8 in flight | Plan.cs:84 | `choose` | self-test ReFS x3 | checked |
| 1.62 | One SSD in a USB case at both ends: 8 MB x 4, one read out, small batches, no trials (more in flight hung a USB case: event 129 resets) | Plan.cs:99 | `choose` | dump F:>F: | checked |
| 1.63 | A hard disk, USB stick or disc at either end: one file at a time, 32 MB x 8 | Plan.cs:116 | `choose` | dump C/D/X/F pairs | checked |
| 1.64 | A hard disk into a fast drive: 1 MB x 32, 4 reads out, no trials | Plan.cs:127,188 | `choose` | self-test `hard disk into an SSD...`; dump D>C, X>C | checked |
| 1.65 | One disk at both ends: phased (read a batch, then write it) | Plan.cs:135 | `choose` | dump D>D, X>X | checked |
| 1.66 | Into a hard disk: 4 pieces per write (not one disk), cached writes in 4 MB, no size reserved | Plan.cs:137-148 | `choose` | dump C>D, C>X | checked |
| 1.67 | Two hard disks: 2 MB x 256, 4 reads out, 32 pieces per write, the cached end once read, no trials | Plan.cs:149 | `choose` | self-test `two hard disks unchanged`; dump D>X | checked |
| 1.68 | One hard disk: 1 MB x 512, 32 reads out, 64 MB writes, size reserved, the cached end once read, no trials | Plan.cs:165 | `choose` | self-test `one hard disk...`; dump D>D | checked |
| 1.69 | Trials: one disk 32x16 / 16x8 phased; fast -> hard disk 1x32 join32 / 8x8 join4; hard disk -> fast none; 2 s trials, 8% margin | Plan.cs:182-204 | `choose` | self-test `one USB drive...`, `SSD into a hard disk unchanged`; dump | checked |
| 1.70 | SSD or other: 1 / 4 / 8 (more than 32 files, median under 1 MB) in flight, 1 for up to 3 files with one of 1 GB or more; 8 MB x 4 with trials 4x8, 16x4, 32x4; tuning with more than one file | Plan.cs:207 | `choose` | dump C>C, C>F, C>G x 12 shapes | checked |
| 1.71 | Small files in batches when a hard disk is at either end: one worker, no tuning, at most 4 | Plan.cs:227 | `with_small_batch` | dump | checked |
| 1.72 | Trials take the plan's alignment, reservation and cached write size; alignment = 4096 or the larger sector | Plan.cs:97,240 | `aligned` | dump (`ways` columns) | checked |
| 1.73 | Badge, Detail, Why texts | Plan.cs:26-52 | `badge`, `why` | dump (every plan line) | checked |
| 1.74 | `--usb [out.txt]`: every drive but network and CD, "X:\ disk n KIND: link  [ms]", UTF-8 with BOM, the console without a file | Cli.cs:258 | cli/usb.rs | `--usb` identical (ms masked), with a faked link too | checked |
| 1.75 | `--selftest out.txt`: "PASS/FAIL name", then "ALL PASS" or "n FAILED", no BOM, exit 0 / 2; step 1's 27 checks | Cli.cs:552 | cli/selftest.rs | the Rust lines are the C# 1.48 ones of step 1 (names, order, results) | checked |
| 1.76 | A command line error -> %TEMP%\choppy-cli-error.txt, exit 1 | Cli.cs:97 | cli.rs `run` (catch_unwind) | both: `--usb` to a folder that doesn't exist | checked |
| 1.77 | `--plan-dump out.txt [--scan copy\|move dest src...]` (new, in both) | Cli.cs (rust branch) | cli/plan_dump.rs | rust/tools/plandump/run-all.sh | checked |
| 1.78 | The exe carries app.manifest, Choppy.ico and the version info (FileVersion = VERSION + .0, the C# strings) | Choppy.csproj | build.rs | manifest extracted with mt.exe = src/app.manifest; icon pixels identical; VersionInfo fields identical (C#'s ProductVersion adds the SDK's +commit) | checked |
| 1.79 | No VC++ runtime needed (+crt-static) | - | .cargo/config.toml | dumpbin: system DLLs only | checked |
| 2.1 | RenameByHandle: the new name must be the full `\\?\` path (Windows resolves a plain name against the current folder, not the file's); FILE_RENAME_INFO laid out as C# did (name at 20, size = 20 + name + 2); never replaces | Native.cs:280 | sys/io.rs `rename_by_handle` | gate: every new file without verify takes it (copy-new, copy-ssd-hdd, ...), copy-no-fast = the by-path way, both trees identical | checked |
| 2.2 | HashFile: unbuffered (+ overlapped) = straight from the disk / server, not what Windows still holds; buffered = in order on a plain handle; 8 MB x 4 read ahead while hashing; progress can stop it (ERROR_REQUEST_ABORTED) | XxHash64.cs:99 | engine/xxhash64.rs `hash_file` | self-test `hash unbuffered / buffered <n> B` (5 sizes); gate copy-verify, copy-net-verify, move-verify, move-resume | checked |
| 2.3 | LoadUnfinished ends with CleanStale: `*.admin` records whose parent Choppy is gone (or has no `parent=`) go with their .progress / .result / .control (+ .tmp) files; a live one stays | Settings.cs:276, Handoff.cs:184 | engine/handoff.rs `clean_stale` | cargo test `admin_side_queue_and_stale_records` | checked |
| 2.4 | FSCTL_GET_RETRIEVAL_POINTERS from cluster 0: the first extent's LCN (ERROR_MORE_DATA still gives it), no admin needed; small files read in that order; none = long.MaxValue | Native.cs:241, BigCopy.cs:532 | sys/io.rs `first_lcn`; big_copy.rs `open_small` | gate copy-one-hdd, copy-small-batch (batches read in disk order, trees identical) | checked |
| 2.5 | SetThreadExecutionState(ES_CONTINUOUS \| ES_SYSTEM_REQUIRED) on the monitor thread while a copy runs, ES_CONTINUOUS while paused and at the end | Native.cs:400, CopyJob.cs:1815 | sys/proc.rs; copy_job.rs `monitor_loop` | review (C#'s flags and moments); the system-wide state can't show it here (another program already holds it, `powercfg /requests` needs administrator rights: on the user's list) | checked (review) |
| 2.6 | RingReader on an overlapped handle: every free buffer has a read out at the disk (other programs can't slip in), at most ReadDepth out; pieces handed over in file order; the reader thread AboveNormal | BigCopy.cs:99 | big_copy.rs `Reader::read_queued`; sys/ring.rs (buffers as owned slots, reads as PendingRead) | self-test pipeline block (34 lines); gate (every route) | checked |
| 2.7 | Phased (one disk at both ends): wait for every buffer, read the batch (ReadDepth out), then the writer takes the whole batch before writing (Gather waits for it), written a piece / WriteStep at a time | BigCopy.cs:123,215,441-444 | `read_phased`, `gather`; `copy` | self-test `pipeline phased`, `phased, read depth + write step` x4; gate copy-one-hdd, copy-one-usb | checked |
| 2.8 | CollectWhole: a short read (a network) reads the rest of the piece; a read of 0 / EOF = error 38; the first error is kept (EOF from GetOverlappedResult isn't one) | BigCopy.cs:150,180 | `collect_whole`, `collect` | gate copy-net-verify, copy-from-net (overlapped hashing over SMB) | checked |
| 2.9 | Gather: pieces already read that follow in memory (no ring wrap) join one write while n is still whole pieces; a read error is left for Next | BigCopy.cs:210 | `gather`, sys/ring.rs `joined_mut` | self-test `pipeline joined writes` x4; gate copy-hdd-hdd (join 32) | checked |
| 2.10 | Dispose: the reader stops, waits for reads still out (Drain), only then the buffers go | BigCopy.cs:190,233 | `Drop for RingReader`; `Drop for PendingRead` | self-test `pipeline stop`; gate cancel, copy-skip-big; review (a) | checked |
| 2.11 | PipeTuner: 64 MB warm-up sizes the trials (trialSeconds at that speed, whole 32 MB, 64-512 MB); each way once; under 0.1 s = tried again; another way must beat the start by the margin; no room in this file (the cached end kept out) = the next big file tries; Summary / Describe texts | BigCopy.cs:265-343 | big_copy.rs `PipeTuner` | cargo test `tuner_warms_up_tries_each_way_and_keeps_a_clear_winner`; gate (SSD routes run it) | checked |
| 2.12 | BigCopy opens: source unbuffered + sequential + overlapped, sharing read + delete; the copy CREATE_NEW (never replaces), unbuffered, sharing read + write, with DELETE access for the rename | BigCopy.cs:392,401 | `copy` | self-test `pipeline never replaces`; gate | checked |
| 2.13 | The whole size reserved up front (sector-rounded): FileAllocationInfo (a full disk = that error at once, others ignored), then EndOfFile (an error = Unsupported) | BigCopy.cs:411-417 | `copy` | self-test (with and without reservation); gate | checked |
| 2.14 | Buffers are committed memory: at most an eighth of the free RAM and of the free commit; no more buffers than the stretch has pieces, at least 2 | BigCopy.cs:422,431 | `copy` | review (a); gate | checked |
| 2.15 | The last unbuffered write is padded to a whole sector with zeros (inside the piece's buffer), the size put right at the end | BigCopy.cs:466,496 | `copy` | self-test odd sizes (1, 3158073, 5243657, 123457 B); gate odd sizes | checked |
| 2.16 | Cached tail: the file's end through the Windows cache (its own handle at `done`, the unbuffered one closed first, renamed by path afterwards), TailAfterRead only once the source is all read, CachedWriteChunk per write; a failed open just goes on unbuffered | BigCopy.cs:449-460 | `copy` | self-test `pipeline cached tail`, `cached end after read`; gate copy-ssd-hdd, copy-new | checked |
| 2.17 | Speed limit: one write at most LimitStep (an eighth of a second, 256 KB - 64 MB, sector-aligned); time held in progress (pause, limit) over 50 ms isn't the way's speed | BigCopy.cs:445,478-482; CopyJob.cs:1764 | `copy`; copy_job.rs `limit_step` | gate copy-limit, copy-pause | checked |
| 2.18 | The source's size changed during the copy (done != size) = NotSupported: CopyFileEx copies it again; Unsupported = errors 87 / 50 / 1 (a driver that refuses unbuffered I/O) | BigCopy.cs:494,578 | `copy`, `unsupported`; copy_job.rs `copy_file` | review (b) | checked (review) |
| 2.19 | After the data: EndOfFile = size, the named `:$DATA` streams (not `::$DATA`; FAT's ERROR_HANDLE_EOF = none; can't tell = NotSupported) through their own handles sharing delete, then dates + attributes (R/H/S/A/not indexed; none = NORMAL) on the open handle, flush (a move), the hash, rename by handle | BigCopy.cs:496-508,590,612 | `copy`, `named_streams`, `copy_stream` | self-test `pipeline stream`, `pipeline dates`; gate (Zone.Identifier on big1.bin, attributes, dates) | checked |
| 2.20 | SkipAttrs: encrypted, compressed, sparse, offline, recall-on-open / data access, reparse points never take the pipeline (CopyFileEx does) | BigCopy.cs:369 | `SKIP_ATTRS`; copy_job.rs `use_pipe` | gate: packed.bin (NTFS-compressed, 34 MB) on every route | checked |
| 2.21 | Small-file batch: opened unbuffered (extra streams = the usual way), read whole and sector-padded in disk order, written in one go through the cache (test modes: unbuffered + trimmed), dates + attributes + rename by handle | BigCopy.cs:520-575; CopyJob.cs:814-897 | `open_small`, `read_small`, `write_small`; copy_job.rs `fill_batch`, `copy_from_batch` | gate copy-one-hdd, copy-small-batch | checked |
| 2.22 | Every file is written as `<name>.chpart` and renamed into place only when complete (and verified); a canceled or failed try removes it | CopyJob.cs:28-34,620-701 | copy_job.rs `process_file` | gate cancel (no .chpart left, every file there whole); cargo test (a canceled administrator copy) | checked |
| 2.23 | TempName: `<final>[.~n].chpart` while it fits 255 units, else `~ch<xxh64 of the UTF-8 name>[.~n].chpart`; FreeTemp = the next free `.~n` | CopyJob.cs:1527,1537 | `temp_name_n`, `free_temp` | self-test `temp`, `temp long` | checked |
| 2.24 | A `<name>.chpart` already there: in resume mode a leftover of this job goes (unless it is one of the job's own destination names), otherwise another temp name is used and the file there is never touched | CopyJob.cs:618-626,1552 | `process_file`, `is_job_dest` | gate copy-resume, copy-skip/replace (big1.bin.chpart, LAST.txt.CHPART kept or replaced exactly as C#) | checked |
| 2.25 | Fast path: a new file without verify is renamed through its own handle; the first 3 of a job are checked where they must be, else the fast path goes off for the job | CopyJob.cs:639,741-749 | `process_file`; `copy_one` callback | gate copy-new vs copy-no-fast | checked |
| 2.26 | Resume: a file there with the same size and date counts as done; for a move only if it reads back the same (xxh64 of both, the copy unbuffered), else it is replaced | CopyJob.cs:586-603,1566 | `process_file`, `same_content` | gate copy-resume, move-resume (zone.txt: same size + date, other bytes, replaced; mid.bin kept) | checked |
| 2.27 | Conflicts: Ask (no UI = Skip; the dialog's cancel = the job), Apply to all changes the policy, Replace if newer by date, Keep both = "name (2).ext" with no `.chpart` of it either, Replace refuses a folder; ConflictsLeft counted | CopyJob.cs:604-615,1474,1510 | `resolve_conflict`, `free_name` | gate copy-skip, copy-replace, copy-keepboth, copy-newer, copy-ask, move-same, move-cross | checked |
| 2.28 | Errors per file: temp name taken = another (3 tries); disk full = pause with a message unless there is room (then the drive refused it: fails); network errors (14 codes) wait up to 15 min for both sides, then back off 1-16 s (6 tries); in use 3 tries; access denied where the destination refuses = the admin hand-off | CopyJob.cs:651-701,1691-1759 | `process_file`, `wait_for_network`, `backoff`, `is_net_error`, `is_busy` | review (b) | checked (review) |
| 2.29 | Verify before the rename: the pipeline's source hash reused (only the copy read back, unbuffered); one mismatch = copied again with a note, two = failed and not kept | CopyJob.cs:703-737,1084 | `process_file`, `verify_one` | gate copy-verify, copy-net-verify, move-verify | checked |
| 2.30 | RenameIntoPlace: a new file under that name meanwhile = keep both; replacing a read-only file clears it; busy = wait 0.5 / 1 / 1.5 s; 4 tries | CopyJob.cs:1113 | `rename_into_place` | gate copy-replace (read-only big2.bin replaced); review | checked |
| 2.31 | The original creation date kept too: on the copy's own handle in CopyFileEx's callback (first stream switch) or by path afterwards; folders get theirs after the files | CopyJob.cs:1058,750,760,1671 | `copy_one`, `set_times`, `finish_folders` | gate (creation dates of files and new folders on every route) | checked |
| 2.32 | Moves: an original goes only after its copy is in place and on the disk (big files flushed before their rename; the rest flushed in batches of 500 / 256 MB, 512 at once, 32 on a network, at most two batches waiting, at most 10 s), and only if it didn't change (size + date); read-only originals cleared to delete and put back if that fails | CopyJob.cs:1132-1292 | `moved`, `flush_loop`, `flush_then_delete`, `flush_path`, `delete_source`, `finish_durable` | gate move-cross, move-resume, move-verify (what the source keeps identical); review (e) | checked |
| 2.33 | Same-drive move: a rename; folders merge child by child; ERROR_NOT_SAME_DEVICE (a drive mounted inside) = copy + delete for top items only; "Already in this folder", "A folder can't be moved into itself"; a refused destination = the admin hand-off | CopyJob.cs:276-388 | `run_instant_move`, `move_item`, `list_names` | gate move-same, move-same-new | checked |
| 2.34 | The last big file of a copy: its end through the cache (not for USB / removable, not when the source is the slow end, not with verify or a move); to a hard disk from a fast source the whole file through the cache with its size reserved; while the tuner still measures at most a quarter of the file | CopyJob.cs:948-984,1009,1026 | `copy_big`, `source_is_slow_end`, `cache_budget` | gate copy-ssd-hdd, copy-hdd-ssd, copy-ssd-usb | checked |
| 2.35 | FAT32: a file over the drive's largest fails at once with the reason (the file system's name in it); the rest go on | CopyJob.cs:464-469 | `run_copy` | gate copy-fat32 (`--max-file`; no FAT32 drive here) | checked |
| 2.36 | Low space: more bytes than free asks (the CLI says no = canceled); compressed / encrypted destination folder = no pipeline; Plan forced by test switches; the tuner only without a speed limit or fixed switches | CopyJob.cs:447-480 | `run_copy` | gate (test switches); review | checked |
| 2.37 | Administrator hand-off, the user's side: asked once per copy; the rest (not started + refused) or whole items go to a record; launched with runas; progress mirrored as the current file; Pause / Resume / Cancel passed on with a growing number; ended without a result (2 s quiet) or declined = those files failed with the reason; its counts, lists and notes join | CopyJob.cs:1297-1470 | `ask_to_elevate`, `hand_over`, `dest_refuses`, `run_elevated`, `hand_back` | up to the prompt (record, fingerprint, command line); the elevated run itself is on the user's list | checked (up to the prompt) |
| 2.38 | Administrator hand-off files: the record (+ parent / testdata / testhook / testpark), its SHA-256 on the command line (lower-case hex, trimmed), `.progress` / `.result` (`end` last) / `.control` ("n what", newest wins), UTF-8 with BOM, written next to it and swapped in (ReplaceFile write-through / MoveFile); Prepare takes test mode from a genuine record | Handoff.cs:36-351 | engine/handoff.rs | cargo test `admin_side_queue_and_stale_records` (a real copy reporting, paused / run / canceled through the control file, results read back); `sha256` known values | checked |
| 2.39 | Alive(pid): a process that can't be opened (gone, or a higher integrity level) counts as gone, as .NET's exception did | Handoff.cs:203 | sys/proc.rs `process_alive` | cargo test (live / dead parent); review | checked |
| 2.40 | Folders: made as needed (the whole path), empty ones too, existing ones remembered; dates and hidden / system / archive set after the files (8 at once on a network); a move removes empty source folders deepest first, not one a file blocked | CopyJob.cs:1620-1687 | `ensure_dir`, `create_dir_tree`, `finish_folders` | gate (folder dates and attributes, hid / sysdir, empty folders, what a move leaves) | checked |
| 2.41 | Speed: 600 samples of 250 ms, smoothed, peak, last second; Adaptive: more files at once (x2 with 150+ pending, kept only if 15% faster), a second big file over a network (+15%); a changed situation undoes the try, 4 times = stop | CopyJob.cs:1805-1952 | `monitor_loop`, `tune_small`, `tune_big`, `interrupt` | review | checked (review) |
| 2.42 | Speed limit: a token bucket (a quarter second of burst), a wait of at most 2 s per call | CopyJob.cs:1770 | `throttle` | gate copy-limit, copy-pause, copy-skip-big | checked |
| 2.43 | Log (only with problems, notes, or as administrator): title, To, Started / finished in the user's date format, counts, FAILED / SKIPPED / NOTES; UTF-8 with BOM; "yyyy-MM-dd HH-mm-ss <id>.txt" | CopyJob.cs:1956 | `write_log`; sys/proc.rs `general_date_time` | gate: logs compared (dates masked); en-US format checked against .NET (`11/25/2026 5:08:09 PM`) | checked |
| 2.44 | Panics (C#'s exceptions) are caught at every thread: one in a file's copy fails that file ("Unexpected error: ..."), elsewhere the job fails after what was done is wrapped up; the CopyFileEx callback never unwinds into Windows (it cancels the copy); no `.chpart` is renamed after one | (C# crashed or caught per thread) | copy_job.rs threads; sys/io.rs `copy_file_ex` | review (a), (b) | checked (review) |
| 2.45 | JobManager: one job at a time in order; the settings' speed limit when the job has none; the record saved in the background, removed when the job ends unless it failed; Retry failed = a new job with those files; live speed limit; changes reported on the UI thread | JobManager.cs:36-131 | job_manager.rs | cargo test `admin_side_queue_and_stale_records` (two jobs in order, records gone) | checked |
| 2.46 | `--cli copy\|move`: switches and engine test switches, sources and destination through ToLong, conflicts skipped, low space cancels, the record written and removed, the JSON report and progress log (UTF-8 with BOM), exit codes 0 / 1 / 2 / 3 | Cli.cs:25-253 | cli/job.rs | gate: 31 scenarios through it, the JSON reports compared | checked |
| 2.47 | Self-test: "temp", "temp long" and the pipeline block between the paths and the plan checks; a crash in the block = "pipeline: <message>" | Cli.cs:533-616,827-830 | cli/selftest.rs | the 70 lines identical to C# 1.48's | checked |
| 3.x | (filled in step 3) | | | | |
| 4.x | (filled in step 4) | | | | |
| 5.x | (filled in step 5) | | | | |
| 6.x | (filled in step 6) | | | | |

## UI look test (step 0)

Today's copy window (MainWindow.xaml + Theme.xaml + Controls.cs + Helpers.cs) built twice with equal care:

- **A** = our own UI on Direct2D + DirectWrite through the `windows` crate (`rust/look-test/a-d2d`).
- **B** = Slint (removed after the pick; it was `rust/look-test/b-slint`, commit 1aa7bbf).

Both show the README demo (D: 4TB - Shadow `D:\Home Videos` -> X: 4TB - uTorrent `X:\Backup\Home Videos`,
Adaptive · 1 stream, 57 MB/s limit, the four demo videos + 2 small files) animated every 100 ms, both views, the
custom title bar, hover/pressed states, the quick toggles, the speed-limit box, the Queue pop-up, keyboard focus,
dragging and closing. Measured against WPF: open time, memory, CPU while animating, exe size (PROGRESS.md).

**The user picked A** (2026-11-24): it opens fastest (0.035 s vs B 0.24 s, today 1.9 s), uses the least CPU and
memory, the exe is about 15 times smaller than B's (0.66 MB vs 10.1 MB), and its window looks the same as today's.
