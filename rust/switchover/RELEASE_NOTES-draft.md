## Choppy v2.0

📋⚡ Windows 10 / 11 - fast, clear file copier ⚡ everything that matters in one plain, dark window.

### What's in v2.0
- **Rebuilt in Rust:** Choppy is now one native exe with everything it needs inside - no .NET. The copy engine, every window, the Benchmark, the tray and Explorer's Ctrl+V and right-click paste work as in 1.48, and the windows look the same.
- **Opens quicker:** on the test PC the copy window came up in about **0.02 s** after a paste with Choppy in the tray (1.48: about 0.12 s), and in about **0.2 s** when Choppy wasn't running (1.48: about 1.1 s).
- **Lighter:** in the tray it used about **38 MB** of memory on the test PC (1.48: about 100 MB).
- **Same copy speed:** the same way of copying as 1.48. On the test PC the Benchmark put it level with 1.48 route by route.
- **Starts again by itself:** with *Default file manager* on, if the tray Choppy ever ends without being quit (ended from Task Manager, or an error), it starts again in the tray once it is completely gone - a Choppy that is only busy is never closed. It tries this a few times at most and notes each time in Choppy's folder.
- **Copies finish a little sooner:** a copy ends as soon as its last file is done, not up to a tenth of a second later.
- **Safer in rare cases:**
  - A moved big file's original goes only once the copy's name is on the disk too, so a power cut right then can't leave the copy only as a `.chpart` file.
  - When the network doesn't come back within 15 minutes, the file is reported as failed instead of staying active.
  - A file that another program (an antivirus) holds for a moment is checked again instead of failing the check.
  - A file that got shorter while it was copied is copied again as it is now.
  - A resumed copy never removes a `.chpart` file that something is still writing, and an administrator copy of a resumed job resumes too.
  - A folder that couldn't be created is reported, and a move keeps its original folder.
  - Retrying a failed move puts each item back into its own folder.
- **Small fixes:** the Benchmark's time estimate follows the drives checked; holding the mouse on a finished copy's window keeps it open; `Choppy.exe --takeover on` sent to a running Choppy turns it on.
- **Everything carries over:** your settings, the right-click entry, *Default file manager*, start with Windows and unfinished copies (*Resume*) stay as they are. Nothing to set up again.
- **The exe is bigger:** about 3.7 MB instead of under 1 MB, since it now carries what .NET used to provide.

### Install / Update
1. Download **Choppy-v2.0.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.
