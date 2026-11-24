## Choppy v1.48

📋⚡ Windows 10 / 11 - fast, clear file copier ⚡ everything that matters in one plain, dark window.

### What's in v1.48
- **Benchmark - choose the drives:** each drive has a checkbox. Every drive that can be tested starts checked; uncheck one to leave it out of the run. The estimate follows, and *Start* waits for at least one checked drive. While it runs the checkboxes are locked; afterwards *Run again* uses them.
- **Benchmark - drives always open expanded:** the list opens with every drive showing each time. The **−** still folds it while the window is open. It folds by itself only when the screen is too short to show the drives, a row of results and the buttons.
- **Benchmark - looking at the drives again only before a run:** the arrows beside *Drives* (and the automatic look when a drive is plugged in or removed) are there until a run starts, then hidden.
- **Settings start fresh once:** updating from an earlier version puts the settings back to their defaults one time - *Ask me*, speed in *Auto*, *Check every copy* off, *Close at completion* on at **0 s**, *Speed limit* off at **12 MB/s**. *Default file manager* stays as chosen at install. From now on, settings stay as you set them through updates.

### Install / Update
1. Download **Choppy-v1.48.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.
