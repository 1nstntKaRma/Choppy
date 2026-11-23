## Choppy v1.40

📋⚡ Windows 10 / 11 - fast, clear file copier ⚡ everything that matters in one plain, dark window.

### What's in v1.40
- **Big files within one hard disk keep up with Windows' own copy:** the whole file (up to 512 MB at a time) is read before any of it is written - in 1 MB reads, 32 at the disk at once - then written in 64 MB writes into room reserved up front, and none of it waits in memory at the end (there it would compete with the reads for the heads). Measured with the benchmark, 512 MB within one hard disk, ten runs each: 87-89 MB/s (before: 84-86; Windows 90-92), up to 98 when the reserved room lands on a fast part of the disk; 2 GB: 91 MB/s (before: 89). A full benchmark run now shows a tie there on both hard disks of the PC it was measured on (before: slower).
- **Benchmark, for comparing engine settings** (command line, for tests): `--routes C>D,D>D`, `--cases big`, `--big <MB>`, `--rounds <n>` and `--choppy "<engine switches>"` measure one route as often as needed against Windows.

### Install / Update
1. Download **Choppy-v1.40.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.
