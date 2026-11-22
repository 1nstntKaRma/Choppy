## Choppy v1.34

📋⚡ Windows 10 / 11 - fast, clear file copier ⚡ everything that matters in one plain, dark window.

### What's in v1.34
- **Ctrl+V goes to Choppy every time** (Default file manager on):
  - the keyboard hook runs on a thread of its own, so a busy window never makes Windows take the paste - and it is put in again every few seconds, so Windows can't drop it for good;
  - Shift+Insert too - the paste a clipboard manager sends;
  - any start of Choppy brings the hook back to the tray, and a Choppy that closes on an error starts again;
  - the folder and the clipboard are asked again when Explorer or another program is busy for a moment;
  - every paste is noted in `%LOCALAPPDATA%\Choppy\Logs\ctrl-v.txt`, with the reason whenever Windows did it instead.

### Install / Update
1. Download **Choppy-v1.34.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.