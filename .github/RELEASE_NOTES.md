## Choppy v1.32

📋⚡ Windows 10 / 11 - fast, clear file copier ⚡ everything that matters in one plain, dark window.

### What's in v1.32
- **Hard disk to hard disk:** big files are read 512 MB ahead in 2 MB reads and written in 64 MB pieces; once the source is fully read, what's left goes through the Windows cache. 4 GB on disk: 166-179 MB/s (was ~150).
- **Settings on one line each:** a shorter window, **Logs folder**, About next to the version.
- **Speed limit in whole numbers**, in Settings and in the quick toggles.
- **A speed limit holds steady:** small writes while it's on, so the speed sits at the limit instead of jumping around it.
- Starts a little faster.

### Install / Update
1. Download **Choppy-v1.32.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.