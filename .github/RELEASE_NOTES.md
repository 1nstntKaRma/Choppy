## Choppy v1.44

📋⚡ Windows 10 / 11 - fast, clear file copier ⚡ everything that matters in one plain, dark window.

### What's in v1.44
- **Big files from a hard disk to an SSD or NVMe, steady:** still read in 1 MB pieces, but only 4 of them are out at the disk at once instead of 32. With all 32 queued, some copies took 0.3-0.4 s longer for 1 GB - in the v1.43 Benchmark one such run made X: → C: *Slower* (−5%). Measured from two hard disks about 80% full to the NVMe, five runs each, time until the copy is on the disk against Windows' copy:
  - 1 GB: **+1% / −1%**, again **−1% / −1%** (before between −1% and −4%)
  - 2 GB: **0% / −1%** (before −2% / −1%)
  - every run within 0.16 s of the others (before up to 0.47 s apart); bigger pieces, 2 or 8 at once, or reading through the Windows cache did no better.
- **Benchmark on that PC (Quick):** *Choppy is Faster 9 of 14 · Marginal Tie 5 of 14 · Never Slower* - small files 47-80% less time on every route.

### Install / Update
1. Download **Choppy-v1.44.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.
