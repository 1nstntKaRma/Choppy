## Choppy v1.46

📋⚡ Windows 10 / 11 - fast, clear file copier ⚡ everything that matters in one plain, dark window.

### What's in v1.46
- **Benchmark window, sized as you like:** drag any edge, maximize it (button, double-click the title bar, Windows+Up) or minimize it - before, during or after a run. Maximized, it fills the screen it is on up to the taskbar, on any monitor; the results list fills whatever height it has. It never gets smaller than its buttons and a couple of result rows.
- **Drives fold to one line:** the **−** beside *Drives* folds the list to `C: NVMe · D: HDD · F: USB SSD · 1 left out` (details in its tooltip, a held-back USB drive in amber), the **+** opens it again; the choice is kept for next time. On a screen too short for the drives and the results, they fold by themselves once results show.
- **The whole run in four numbers**, at the top of the window and of the report once it ends:
  - **On average** - the average *Time saved* of all cases, green when Choppy is faster, red when slower
  - **Choppy average** and **Windows average** - each one's speed over the whole run: all the bytes over all the time, every case copied once
  - **Time saved** - how much less time Choppy took in all (red when more)
  - also in *Copy results* and in the `.json` (`totals`).

### Install / Update
1. Download **Choppy-v1.46.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.
