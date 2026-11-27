## Choppy v2.02

⚡ Windows 10 / 11 - fast, clear file copier ⚡ Everything that matters in one plain, dark window.

### What's in v2.02
- **Updates in one click:** when a new Choppy is out, the tray icon gets a small green dot and its menu a new **Update to vX**, and Settings shows a softly glowing **Update to vX** at the top. One click is all it takes: Choppy downloads the release from GitHub, checks it against the release's SHA-256 sums and installs it - no questions, no windows. Your settings and the Default file manager stay as they are. The new version starts by itself and says so (in Settings, or over the tray icon).
  - A copy or a Benchmark that is running is never cut off: the update waits for it to finish (*Update to vX after the copy*).
  - If something goes wrong, Choppy stays on the version you have and says why; the details are in `update.log` in the Logs folder.
- **Check for updates** (new in Settings): **At startup** (the default), **X seconds after startup** or **Every X days** - or turn it off. It only asks GitHub which version is the latest.
- **Everything carries over:** your settings and unfinished copies (*Resume*) stay as they are.

### Install / Update
From v2.02 on, Choppy updates itself (see above). To get v2.02:
1. Download **Choppy-v2.02.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.
