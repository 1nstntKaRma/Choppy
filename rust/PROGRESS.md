# Choppy in Rust - progress

Read [PLAN.md](PLAN.md) first (steps, gates, rules, file map). One step per session; update this file at the end.

## Steps

| # | Step | Status |
|---|------|--------|
| 0 | Tools + UI look test | **done** (2026-11-24): tools installed, both prototypes compared, **the user picked A** |
| 1 | Basics | **done** (2026-11-24): gate met, plan dump identical to C# 1.48, fix list 1.1-1.79 checked |
| 2 | Copy engine | not started |
| 3 | Speed proof | not started |
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

## Next session

**Step 2 (copy engine), on Max effort without Ultracode; then a review pass on Extra + Ultracode.** Read PLAN.md
(step 2's gate, file map, self-test names) and "For the next steps" above. Start with BigCopy.cs: RingReader,
PipeTuner, BigCopy, and `HashFile` from XxHash64.cs (fix list 2.2); then CopyJob.cs (`TempName` first), Handoff.cs
(2.3: `clean_stale`), JobManager.cs, and Cli's `--cli copy|move` (EngineOption, ParsePolicy, Report, Esc) with the
self-test's `temp` + pipeline checks (they go between the paths checks and the Plan checks; `NetRandom` in
cli/selftest.rs gives System.Random's bytes). The `sys` calls step 2 adds: CopyFileExW, MoveFileExW, overlapped
ReadFile/WriteFile, SetFileInformationByHandle (RenameByHandle 2.1), streams, VirtualAlloc, retrieval pointers (2.4),
SetThreadExecutionState (2.5). The gate compares destination trees made by C# and Rust on a fixture set
(`rust/tools/plandump/fixture.py` is a start).
