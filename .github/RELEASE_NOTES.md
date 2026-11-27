## Choppy v2.01

⚡ Windows 10 / 11 - fast, clear file copier ⚡ Everything that matters in one plain, dark window.

### What's in v2.01
- **Many small files copy about twice as fast:** on the test PC, 20,000 small files from the NVMe to itself took about **3.1 s** instead of 6.4 s; 2,000 small files to a hard disk about 15-20% less time.
  - More files are in flight at once between SSDs inside the PC.
  - A new small file goes straight to its own name instead of through `<name>.chpart` and a rename - the rename was about a third of a small file's time.
- **Just as careful:** a small file written straight to its name is made *marked as unfinished* (and hidden) and loses the mark with its last byte. If a crash or a pulled drive cuts a copy off, **Resume** finds the unfinished files and copies them again on its own - no questions about files Choppy left half-written. A canceled copy removes them at once. Big files, network shares, FAT / exFAT drives, files that replace another and *Check every copy* keep the `.chpart` way, where it costs nothing.
- **Big folders start sooner:** a folder tree is listed with several threads on SSDs and hard disks. On the test PC a folder with about 160,000 files was ready in about **0.3 s** instead of 1.4 s.
- **No more short waits:** checking the destination ended up to 15 ms late on every copy into an existing folder; now it ends at once.
- **Adaptive mode reacts sooner:** on drives inside the PC it tries more files at once after about 3 s instead of 12 s.
- **Finishing is quicker:** folders get their dates with several threads at once.
- **Safer in rare cases:**
  - A very deep folder tree, or an error while listing folders, can no longer end Choppy or leave a copy waiting forever.
  - Small files copied to a hard disk keep their Linux permissions from WSL (extended attributes) - before, only their extra data streams were looked at.
  - A small file that couldn't be written the quick way on a hard disk no longer leaves a `.chpart` file behind.
- **Small fixes:** Settings, About and the Benchmark report show the version as released (v2.01, not 2.1.0); the README has a new header.
- **Everything carries over:** your settings and unfinished copies (*Resume*) stay as they are.

### Install / Update
1. Download **Choppy-v2.01.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.
