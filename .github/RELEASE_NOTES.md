## Choppy v1.42

📋⚡ Windows 10 / 11 - fast, clear file copier ⚡ everything that matters in one plain, dark window.

### What's in v1.42
- **USB ports, known before a copy or a Benchmark:** Choppy asks the USB hub how each USB drive is connected (as Windows' own USB tools do, no administrator rights) - the speed it runs at (USB 2.0, 5 Gbps, 10 Gbps), the most the drive can do, and whether the port takes USB 3. A drive that can do more than it gets - a USB 3 drive in a USB 2 port, a 10 Gbps drive in a 5 Gbps port, a USB 2 cable or hub on the way:
  - the copy window says so beside the drive's name - *⚠ USB 2.0, can do 5 Gbps* - and its tooltip says what helps;
  - the Benchmark shows it under the drive before it starts, and the report and *Copy results* note that the copies there were held back;
  - every USB drive in the Benchmark shows the speed it is connected at; the report adds the most it can do, the port and UASP.
- **File already there:** click *Incoming* or *Already there* (or press ← / →) to pick the file that stays - the main button becomes **Replace** for *Incoming*, **Skip** for *Already there*, and the other one moves beside *Compare*.
- **Taskbar icon on Windows 11:** after an update, a pinned Choppy, the Start menu and the right-click entry could go on showing the old icon from Windows' icon cache. The first start of a Choppy with a new icon now has Windows refresh it - once per icon, not on every update.
- **Benchmark:**
  - the summary reads *Choppy is Faster 12 of 20 · Marginal Tie 6 of 20 · Slower 2 of 20* - no brackets, in the window, the HTML report and *Copy results*;
  - each case's result says **Tie**; *less than 4% apart is a Marginal Tie* stays in the notes;
  - **Stop** is a solid red button.
- **Times in two colors:** *Elapsed* amber, *Remaining* pink - in the copy window and the Benchmark.
- **Mbps ↔ MB/s chart:** longer bars centered between two equal columns, the arrow exactly as long as the bars, each unit right above the units under it.
- Settings: *Close at completion* has no "Whole seconds" tooltip any more.

### Install / Update
1. Download **Choppy-v1.42.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.
