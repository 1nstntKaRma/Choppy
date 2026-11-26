# Switching Choppy over to Rust - the checklist

Nothing here runs by itself. The switch-over starts only when the user says go, after trying the Rust Choppy and
deciding the version number and whether there is a beta. Until then `main` stays on C# 1.48 (full freeze).

Checked in step 6 (2026-11-25/26): `rust` merges into `main` without a conflict (main's tip, 367241b, is where `rust`
started, so it is a fast-forward), and the tree after steps 2-4 below (no `src/`, VERSION 2.00) builds with
`build.ps1 -Shell Cancel`, passes the self-test (97 lines, ALL PASS) and is stamped 2.0.0.0. release.yml's packaging,
run locally on that tree, gave `Choppy-v2.00.zip` with `bin/Choppy.exe`, `build.ps1`, `VERSION`, `README.md`, `LICENSE`
and the two `.cmd` files (1.57 MB).

## Before the go (the user)

1. Try the Rust Choppy. The user installs it themselves, for example from a checkout of the `rust` branch: double-click
   `Activate Choppy.cmd` (it builds with cargo, then installs over 1.48). The settings, the right-click entry, the
   Default file manager and unfinished copies carry over: both versions use the same names everywhere (mutex, pipe,
   `HKCU\Software\Choppy`, the verbs, the Run entry), and `Layout` stays 2, so nothing is reset. Going back to 1.48
   is the same with the v1.48 release zip.
2. The live checks of step 5 on the installed Rust Choppy (PROGRESS.md, step 5, "Only the user can do"): real Ctrl+V in
   Explorer, the right-click "Paste with Choppy", start with Windows, the administrator prompt (Yes and No),
   `wherecheck.py` with Explorer windows open. And step 2's administrator copy end to end.
3. Approve step 4's look (`rust/tools/step4/results/`, the tray menu, a tooltip) and sign off step 3's table.
4. Decide:
   - **The version number: `2.0`** (the user's choice, 2026-11-26). It works because ChocoPlus compares the parts
     as numbers: 2.0 (2 > 1) is newer than 1.48; a `1.5` would read as 1.5 < 1.48 and never be offered. About and
     Settings show `v2.0.0`, as 1.48 shows `v1.48.0`; the exe is stamped 2.0.0.0; the next version is 2.01.
   - **A beta or not.**
     - No public beta: the user's own try (step 1) is the beta; then release the version directly.
     - A public beta as a GitHub pre-release: ChocoPlus (`/releases/latest`, API and page) and build.ps1's download
       (`/releases/latest/download`) skip pre-releases, so only people who pick it by hand get it. It needs small
       changes first, not made yet: release.yml marking a version like `2.00-beta1` as a pre-release
       (`gh release create --prerelease`), and Get-Stamp (build.ps1), build.rs and the stamp check reading the number
       before the `-`.

## The switch-over (on the go)

Run from a clean checkout of the repository, as 1nstntKaRma. Don't push `main` until step 7: every push to `main` runs
release.yml, and a new VERSION publishes a public release.

1. **Bring both branches up to date and check main hasn't moved.**

   ```
   git fetch origin
   git log -1 --format=%H origin/main        # 367241ba8467502db7e21df524543cc1036a1ecb = 1.48, as checked
   git checkout main
   git pull --ff-only origin main
   ```

   If main has moved (for example a README edit on GitHub), the merge below is still expected to be clean (`rust`
   changes README.md nowhere); use the `--no-ff` form.

2. **Merge `rust` into `main`.**

   ```
   git merge --ff-only origin/rust           # main's tip is where rust started: a fast-forward
   # or, if main moved:  git merge --no-ff origin/rust -m "Choppy in Rust: the rust branch"
   ```

   After this, `main` has everything the `rust` branch has: the Rust code in `rust/` (the workspace `rust/Cargo.toml`,
   the app `rust/choppy`, the plan, the progress notes and the tools), the cargo build step in build.ps1, the Rust steps
   in release.yml and build.yml, and still the C# in `src/`.

3. **Remove the C# version.** It stays in git history and in the `v1.48` tag (`git checkout v1.48` brings it back).

   ```
   git rm -r -q src nuget.config
   git commit -m "Remove the C# version (kept in the v1.48 tag)"
   ```

   - The Rust build needs nothing from `src/`: build.rs embeds `rust/choppy/res/app.manifest` and
     `rust/choppy/res/Choppy.ico` (byte for byte the C# project's), and the About box reads `LICENSE` at the top.
   - The Rust code stays where it is, in `rust/`. build.ps1 builds `rust\Cargo.toml` (from `rust\`, so
     `rust\.cargo\config.toml` applies: the C runtime inside the exe, the same exe from the same source) into
     `bin\Choppy.exe`; release.yml and build.yml only call build.ps1. Nothing needs to move.
   - Also gone with `src/`: the C#-only test switches of the `rust` branch (the plan dump, `CHOPPY_REGTEST`,
     `CHOPPY_DUMP`, `CHOPPY_DPI96` in the C# test hook, the three-way Benchmark with `--rust`). The tools in
     `rust/tools` that compare C# with Rust then need a C# exe built from the last `rust` commit, e.g.
     `git worktree add ../choppy-cs origin/rust` and `dotnet build ../choppy-cs/src/Choppy.csproj -c Release`.
   - Leave `rust/choppy/Cargo.toml`'s `version = "1.48.0"` as it is: the exe's version comes from VERSION
     (build.rs), and changing it would also change Cargo.lock, so the `--locked` build fails until Cargo.lock is
     committed again.

4. **Bump VERSION and add the release notes.**

   ```
   printf '2.0' > VERSION                    # the chosen number; no newline, like today's file
   ```

   Copy `rust/switchover/RELEASE_NOTES-draft.md` into `.github/RELEASE_NOTES.md` (fill in the chosen number and
   anything the user wants changed). README.md: the lines in `rust/switchover/README-lines.md`, as the user wants them
   (README changes don't affect the release).

5. **Build and self-test locally** (build only - never Apply / Install here):

   ```
   powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File .\build.ps1 -Shell Cancel
   (Get-Item bin\Choppy.exe).VersionInfo.FileVersion        # 2.0.0.0 for 2.0
   ```

   It prints `[Built] bin\Choppy.exe (... KB), self-test passed`. Build from a short folder path: deep in a long path
   the linker hits Windows' 260-character limit (LNK1104).

6. **Commit.**

   ```
   git add VERSION .github/RELEASE_NOTES.md README.md
   git commit -m "Version 2.0: Choppy in Rust - ..."
   ```

7. **Push - this publishes the release.**

   ```
   git push origin main
   ```

   release.yml then: "Releasing v2.0." -> Rust 1.100.0 -> build.ps1 build + self-test -> the version stamp check ->
   only Windows' own DLLs -> `Choppy-v2.0.zip` + `Choppy.exe` + `SHA256SUMS.txt` -> build provenance -> the release
   and its tag `v2.0`, marked latest.

8. **Check the release.**

   ```
   gh run list --workflow release.yml --limit 1
   gh release view v2.0                       # the three assets
   gh attestation verify Choppy-v2.0.zip --repo 1nstntKaRma/Choppy   # after downloading it
   ```

   ChocoPlus then sees v2.0 as newer than an installed 1.48.0.0 (its update dot on the Choppy button), and the
   README's one-liner / build.ps1's download fallback get the Rust exe. The exe GitHub builds is byte for byte the
   one a local `build.ps1` build of the same commit gives (checked for 1.48: SHA-256 F37EFF8A...).

9. **Afterwards.** The `rust` branch can stay as the record of the port (or be deleted: its commits are in main).
   PLAN.md's "For after the switch-over" lists (PROGRESS.md, steps 2, 3, 5) are the first improvements; the crate's
   `#![allow(dead_code)]` can go then too.

## If the release goes wrong

- A broken build or a failing self-test on GitHub stops before anything is published: fix, push again.
- A problem found after publishing: the quickest fix is the next version (2.01). To take 2.0 back, the user deletes
  the release and its tag on GitHub (`gh release delete v2.0 --cleanup-tag`); v1.48 is the latest release again, and
  ChocoPlus offers it only where an installed version is older than it.
- Going back to C# in main: `git revert` the switch-over commits (or reset main to 367241b with the user's okay), then
  a new VERSION above the last released one.
