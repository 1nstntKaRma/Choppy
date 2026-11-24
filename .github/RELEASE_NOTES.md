## Choppy v1.47

📋⚡ Windows 10 / 11 - fast, clear file copier ⚡ everything that matters in one plain, dark window.

### What's in v1.47
- **Close at completion after 0 seconds:** set it to **0** in Settings and the copy window closes as soon as a copy finishes with no problems.
- **Benchmark - choose the runs:** a **Runs** box beside *Quick* / *Thorough* sets how many times each one copies every case (Quick 2, Thorough 3 unless changed). Each mode keeps its own number in Choppy's settings, so it stays through updates. The estimate follows, the test files stay the same size, and the report says when the runs were changed (*Quick, 5 runs each*).
- **Benchmark - Abort, then Close:** while it runs, the red button is **Abort**; once aborted it turns into a red **Close**, which closes the window as soon as the test files are removed. After an abort: *Open folder*, *Open report*, *Run again* and *Close*. A run that ends normally still offers *Run again*.
- **Benchmark - look at the drives again:** the arrows beside *Drives* list them again, and a drive plugged in or removed shows up by itself while nothing runs - no need to close the window for a USB drive.
- **Benchmark - tidier:** a larger **−** / **+** that reacts to the button only, not the whole *Drives* line; *Keep other work off these drives* moved out of the buttons row to under the drives (under the progress bar while it runs, where the report was saved at the end); *Copy results* removed.

### Install / Update
1. Download **Choppy-v1.47.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.
