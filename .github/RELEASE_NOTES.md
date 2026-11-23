## Choppy v1.36

📋⚡ Windows 10 / 11 - fast, clear file copier ⚡ everything that matters in one plain, dark window.

### What's in v1.36
- **New icon.**
- **The window comes up at once:** with Choppy in the tray, the first copy after it starts used to take about 1.4 s to show its window (a program's first window is slow to build). The tray copy now builds one off-screen a few seconds after it starts, so every copy's window is up in 0.1-0.2 s. Nothing else waits before the window either: the job record is written in the background, and the Explorer folder lookup is quicker.
- **Ctrl+V in a zip folder or another place that isn't a folder** goes to Explorer's own paste at once (it waited about a quarter of a second before).
- **ctrl-v.txt** notes how quick the window was for each paste.
- **FAT32 drives** (USB sticks, cards) hold no file of 4 GB or more: such a file is listed as failed right away, with the reason, instead of after 4 GB were written; the rest are copied.
- **A drive that refuses a file although it has room** (a size limit) fails that file and goes on, instead of pausing as if it were full.
- **The PC doesn't go to sleep in the middle of a copy** (the screen may still turn off); a paused copy lets it.
- **A copy within one ReFS drive (a Dev Drive)** goes to Windows' own copy, which shares the original's blocks instead of writing them again (Windows 11 24H2 and later).

### Install / Update
1. Download **Choppy-v1.36.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.