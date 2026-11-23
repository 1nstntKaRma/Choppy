## Choppy v1.45

📋⚡ Windows 10 / 11 - fast, clear file copier ⚡ everything that matters in one plain, dark window.

### What's in v1.45
- **Copies within one SSD in a USB case, fixed:** a 10 Gbps NVMe case stopped answering when Choppy had much in flight on it at once (8 small files, or several big reads beside a write) - Windows reset it 19 times in six minutes, and a 2 GB copy within it took 6-25 s instead of 4. Now Choppy copies one file at a time there, big files read and written at once in 8 MB pieces with one read out, small files read in batches, then written. No resets since. Five runs each, time until the copy is on the disk against Windows' copy:
  - 2 GB: **+32%** (2.86 s vs 4.20 s; was −297%)
  - 2,000 small files, 366 MB: **+65%** (3.2 s vs 9.2 s; 1,000 files were −1701%)
  - mixed folder (3 big + 400 small): **+48%** (was −838%)
- **Big files from a hard disk or USB drive into an internal SSD, done on the disk:** the reads set the pace there, so the end of the file no longer waits in memory to be written after *done* - every byte goes straight to the disk. Into the NVMe, time to the disk against Windows' copy: 2 GB from a 10 Gbps USB drive **+8.6%** (was −5%), from a 10 Gbps USB SSD **+6.6%** (was −4%), from a hard disk **0% / 0%** (was −5%) - both read the disk as fast as it goes.
- **Benchmark on that PC (Thorough, with network drives, v1.44):** 6 of 48 cases were Slower - these two fixes are for all six.

### Install / Update
1. Download **Choppy-v1.45.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.
