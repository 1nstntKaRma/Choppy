## Choppy v1.29

📋⚡ Windows 10 / 11 - fast, clear file copier ⚡ everything that matters in one plain, dark window.

### What's in v1.29
- **Many small files within one hard disk, faster:** Choppy reads a batch of small files (up to 256 MB) in the order they sit on the disk, then writes them all - instead of read, write, read, write per file (1,000 files: about 68 MB/s, up from about 59).
- **Steadier self-tuning on hard disks:** longer trial stretches, and another way has to be clearly faster before Choppy switches (a hard disk's speed swings over short stretches).
- **External USB SSDs** are copied like SSDs (USB hard disks and sticks stay one steady stream; nothing waits in memory for anything removable).
- **Network shares:** the small-file tuning can go up to 64 files at once.

### Install / Update
1. Download **Choppy-v1.29.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.