## Choppy v1.25

📋⚡ Windows 10 / 11 - fast, clear file copier ⚡ everything that matters in one plain, dark window.

### What's in v1.25
- **Faster big files:** a new engine for big files between local drives - reading and writing at the same time, big pieces, several reads queued at the disk, past the Windows cache. It holds its speed when another program uses the same disk, and NVMe to NVMe goes past 2 GB/s.
- **Verify reads less:** the source is hashed on the way in, so a check (and every move) reads only the copy back.
- **Folders that need administrator rights** (`C:\`, `Program Files`): one question per copy, then the rest goes on in an administrator window. Your window follows it; Pause and Cancel reach it.
- **New look:** From / To with drive names, a short badge (*Adaptive · 1 stream*), sizes in green / peach, elapsed and remaining time in amber, full names under *Largest files*, **Extended** view.
- **Quick toggles** in the extended view: Close at completion, Verify files, Speed limit (with its value) - green = on, red = off.
- **Shorter settings**, Default file manager first, Speed limit last; **About** with the MIT license.

### Install / Update
1. Download **Choppy-v1.25.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.
