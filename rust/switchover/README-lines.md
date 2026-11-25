# README lines that no longer fit the Rust Choppy

A list only - README.md is not changed on the `rust` branch (the user edits the README, also on GitHub). Line numbers
are README.md as of 1.48 (main, 367241b).

| Line | Today | Why it no longer fits | A possible replacement (soft wording) |
|---:|---|---|---|
| 9 | `![.NET Framework](https://img.shields.io/badge/.NET%20Framework-4.8%20(built--in)-512BD4)` | Choppy no longer uses .NET | drop the badge, or `![Rust](https://img.shields.io/badge/built%20with-Rust-B7410E)` |
| 32 | **Zero dependencies:** one `Choppy.exe` (under 1 MB) on **.NET Framework 4.8**, part of every Windows 10/11. Installs per user - no administrator rights. | the exe is 3.6 MB (3,684 KB: 3,772,416 bytes for 1.48) and needs no .NET | **Zero dependencies:** one `Choppy.exe` (about 3.6 MB) with everything inside - nothing else to install. Installs per user - no administrator rights. |

Still true, no change needed:

- 29 **Quick** ("opens in a fraction of a second" with Choppy in the tray): still true, and quicker (the final exe: 0.019 s
  vs 1.48's 0.120 s in the tray). Could also mention a cold start (0.192 s vs 1.147 s), if the user wants.
- 40-43 the PowerShell one-liner ("any stock Windows 10/11 PC, no `git` needed"): it downloads the source zip of
  `main` and runs build.ps1; without Rust on the PC, build.ps1 downloads the release's ready-made Choppy.exe and checks
  its SHA-256, as it did without the .NET SDK. The source zip grows from about 0.8 MB to about 4.6 MB (the Rust code
  and the step tools' pictures in `rust/`).
- 53 `-Shell Apply / RightClick / Revert / Cancel`: unchanged.
- 57-58 the release zip and the two `.cmd` files: unchanged.
- 60 the SmartScreen note: still true. The Rust exe is a new file with no download history yet, so SmartScreen's
  "Windows protected your PC" is a bit more likely for the first downloads (build.ps1 unblocks the files it installs).
- 66 Choppy VS Windows: the Benchmark is the same.
- 98-102 Careful With Your Files (`.chpart`, xxHash64, Resume): unchanged.
- 104-113 What It Changes On The PC: the same paths and keys (step 5 compared the registry writes with 1.48's).
- 117 License / no telemetry: unchanged.
- The screenshots (`screenshots/*.webp`) show the 1.48 windows; the Rust windows are drawn to the same pixels (step 4),
  so they still match. Retaking them is optional.
