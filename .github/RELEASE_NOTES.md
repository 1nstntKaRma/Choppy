## Choppy v1.35

📋⚡ Windows 10 / 11 - fast, clear file copier ⚡ everything that matters in one plain, dark window.

### What's in v1.35
- **Moves never lose a file:** an original is deleted only once its copy is on the disk itself (flushed, not just in memory). Small files are flushed in batches, many at once: 2,000 files (1 GB) onto a hard disk move in about 11 s, all of it on the disk.
- **Resuming a move** checks each copy that is already there (content, not just size and date) before its original goes; a bad one is copied again.
- **An original that changed while it was moved is kept**, with a note.
- **A file that only looks like Choppy's** (`<name>.chpart`) is never deleted or replaced: another temporary name is used.
- Fixes:
  - switching Verify on while a file copies no longer marks a good copy as failed;
  - dates and attributes are set again if the first try didn't take;
  - an original that couldn't be deleted keeps its read-only flag;
  - an empty folder whose place is taken by a file is no longer removed in a move;
  - in a move on one drive, an item on a drive mounted inside a folder stays put instead of landing in the wrong place;
  - big-file buffers also respect free commit memory.

### Install / Update
1. Download **Choppy-v1.35.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.