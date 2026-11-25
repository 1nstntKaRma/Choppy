## Choppy v2.00

📋⚡ Windows 10 / 11 - fast, clear file copier ⚡ everything that matters in one plain, dark window.

### What's in v2.00
- **Rebuilt in Rust:** Choppy is now one native exe with everything it needs inside - no .NET. The copy engine, every window, the Benchmark, the tray and Explorer's Ctrl+V and right-click paste work as in 1.48, and the windows look the same.
- **Opens quicker:** on the test PC the copy window came up in about **0.02 s** after a paste with Choppy in the tray (1.48: about 0.12 s), and in about **0.2 s** when Choppy wasn't running (1.48: about 1.1 s).
- **Lighter:** in the tray it used about **38 MB** of memory on the test PC (1.48: about 100 MB).
- **Same copy speed:** the same way of copying as 1.48. On the test PC the Benchmark put it level with 1.48 route by route.
- **Everything carries over:** your settings, the right-click entry, *Default file manager*, start with Windows and unfinished copies (*Resume*) stay as they are. Nothing to set up again.
- **The exe is bigger:** about 3.6 MB instead of under 1 MB, since it now carries what .NET used to provide.

### Install / Update
1. Download **Choppy-v2.00.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.
