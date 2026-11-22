## Choppy v1.26

📋⚡ Windows 10 / 11 - fast, clear file copier ⚡ everything that matters in one plain, dark window.

### What's in v1.26
- **Copies within one hard disk, much faster:** Choppy reads 256 MB, then writes it, instead of both at once - the heads stop running back and forth (about 90 MB/s on a disk that streams 180, up from about 60).
- **Big files to a hard disk** go out in larger writes, and hard disks get a deeper read queue.
- **Moves follow *Verify files*:** on = each copy is checked before its original is removed; off = no extra read (a big file is still flushed to the disk before its original goes).
- **Compact view:** the speeds have their own row, so a long destination path keeps its room.
- Every push to main now brings a new version and its release.

### Install / Update
1. Download **Choppy-v1.26.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.
