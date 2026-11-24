# Choppy in Rust - progress

Read [PLAN.md](PLAN.md) first (steps, gates, rules, file map). One step per session; update this file at the end.

## Steps

| # | Step | Status |
|---|------|--------|
| 0 | Tools + UI look test | **done** (2026-11-24): tools installed, both prototypes compared, **the user picked A** |
| 1 | Basics | not started |
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

## Decisions

- 2026-11-24: Build Tools 2026 (current winget package) rather than 2022.
- 2026-11-24: look test B uses Slint's femtovg renderer (see above).
- 2026-11-24: **the user picked A** (own UI on Direct2D + DirectWrite): "open in is BLAZING FAST", lowest CPU,
  exe about 15 times smaller than B, smallest memory, and the window looks impeccable. B deleted; A stays in
  `look-test/` as step 4's starting point.

## Next session

**Step 1 (basics).** Read PLAN.md (rules, step 1 gate, file map, self-test names). Port Native (into `sys`), Paths,
Settings, XxHash64, Model, VolumeProbe, UsbLink, Scanner, Plan into `choppy/`, reading each C# file fully once and
filling the step-1 fix list as you go. `build.rs` embeds app.manifest, Choppy.ico and the version info (FileVersion
from VERSION). Add `--plan-dump out.txt` to both the Rust app and the C# app (C# change on the `rust` branch only,
never main), dump every drive pair on the user's PC with both and compare line for line; `--usb out.txt` too. The
step-1 self-test checks (27, listed in PLAN.md) must print the same names and pass.
