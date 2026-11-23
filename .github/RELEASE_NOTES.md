## Choppy v1.37

📋⚡ Windows 10 / 11 - fast, clear file copier ⚡ everything that matters in one plain, dark window.

### What's in v1.37
- **New icon** - the About window shows it twice as big.
- **Installing or updating from an elevated tool or prompt is reliable:**
  - the person signed in is found by the session the run belongs to (several Explorer processes, Microsoft and work accounts), and the install task is bound to their account itself;
  - the task now starts on battery too (Windows' defaults wait for mains power, and stop it when the laptop is unplugged);
  - the Choppy it starts in the tray runs at normal priority (it ran below normal until the next sign-in);
  - the elevated side waits for the install's own "done" and its exit code, and a step that fails says why ("couldn't start", "stopped before it finished") instead of a 3-minute timeout.
- **An update never gets stuck on the old Choppy:** one that can't start any more is replaced anyway, a running copy's "Quit Choppy?" gets up to a minute for its answer, and the file is tried again for a few seconds while a virus scanner looks at it.

### Install / Update
1. Download **Choppy-v1.37.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.