## Choppy v1.28

📋⚡ Windows 10 / 11 - fast, clear file copier ⚡ everything that matters in one plain, dark window.

### What's in v1.28
- **Faster onto hard disks:** no space reserved up front any more - the reservation could put the file on a slower part of the disk (NVMe to hard disk: about 200 MB/s, up from 170).
- **Faster off hard disks:** many small reads queued at the disk keep it streaming when the other side is fast.
- **Adaptive tuning works on more files:** the trials are shorter (under a second each), so files from a few hundred MB on get measured; on a fast drive the whole tuning takes a fraction of a second. The badge shows *tuned* when Choppy switched.

### Install / Update
1. Download **Choppy-v1.28.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.