## Choppy v1.43

📋⚡ Windows 10 / 11 - fast, clear file copier ⚡ everything that matters in one plain, dark window.

### What's in v1.43
- **Big files from an SSD or NVMe to a hard disk, any size:** the last file of a copy goes through the Windows cache in 4 MB writes with its size reserved first, the way Explorer's copy sends it - now whatever its size. Before, a file bigger than a quarter of the free memory (2 GB on a 16 GB PC) went straight to the disk and first tried a few ways of writing, which took longer on a nearly full disk. Measured from the NVMe to two hard disks about 80% full, five runs each, time until the copy is on the disk against Windows' copy:
  - 2 GB: **+5% / −1%** (before −8% / −22%)
  - 4 GB: **0% / −2%** (before +10% / −20%)
  - Faster or a Marginal Tie on both disks; small files and mixed folders are copied as before.
- As before: never for a USB drive, a move or a copy with Verify; from a hard disk only a file that fits in memory.

### Install / Update
1. Download **Choppy-v1.43.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.
