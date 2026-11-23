## Choppy v1.38

📋⚡ Windows 10 / 11 - fast, clear file copier ⚡ everything that matters in one plain, dark window.

### What's in v1.38
- **Benchmark** (*Settings → Benchmark → Run...*): Choppy against Windows' own copy - the shell copy engine Explorer uses for a paste - on every drive of this PC, with a report that shows where each one is faster:
  - finds the drives itself (NVMe, SSD, hard disk, USB; mapped network drives on request) and lists any it leaves out with the reason - read-only, a cloud drive such as Google Drive, not enough free space;
  - from the system drive to each other drive and back, and within each drive; one big file and many small files (4 KB-1 MB), sized by each route's speed - the same sizes every time on one PC, so versions compare;
  - fair: both copy the same source, never from memory (read once beforehand, then dropped from the Windows cache before every copy), in turns (Choppy, Windows, Windows, Choppy), timed until each says it's done and until the copies are on the disk; closer than 4%, or within the runs' own spread, is a tie;
  - a summary (*Choppy faster on 17 of 26 · tie on 7 · slower on 2* on the PC it was built on), every case with both speeds, the ratio and a badge, and the PC: CPU, memory, Windows build, each drive's model, bus and file system;
  - saved as `benchmark-<date>.html` and `.json` (every run's raw times) in Choppy's Logs folder; *Copy results* puts a short table on the clipboard;
  - *Quick* (about 5-10 minutes) or *Thorough* (bigger files, a mixed folder, three rounds); progress and the time left while it runs, *Stop* at any moment;
  - its test files go only into a `Choppy benchmark (temporary)` folder on each drive and are removed at the end, after *Stop*, or at Choppy's next start if it was closed in the middle.
- **Command line:** `Choppy.exe --benchmark [out.json] [--thorough] [--network] [--drives C,D]` runs it headless and writes `out.json` + `out.html`; `--list` shows the drives and the time it would take.

### Install / Update
1. Download **Choppy-v1.38.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.
