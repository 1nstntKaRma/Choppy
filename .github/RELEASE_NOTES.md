## Choppy v1.41

📋⚡ Windows 10 / 11 - fast, clear file copier ⚡ everything that matters in one plain, dark window.

### What's in v1.41
- **Big files to a hard disk are done as soon as Windows' own copy is:** a last file that fits in memory goes to a hard disk the way Explorer sends it - through the Windows cache, in 4 MB writes, its size reserved first - and Windows writes it out right after (within one hard disk: once the whole batch is read, so the writing never competes with the reads). Measured, 1 GB, three runs each:
  - NVMe to hard disk: "done" after 2.5 s (before: 5.7 s; Windows 2.1 s); on the disk as soon as Windows' copy (before: 18-30% later);
  - within one hard disk: done 8.1 s (before: 10.5 s; Windows 8.6 s), on the disk 7% sooner than Windows.
- **From a hard disk into a fast drive, no more trials of slower ways:** 1 MB reads, 32 at the disk, won every trial (201-207 MB/s, the others 171-184), and each trial of a slower way cost 4-5% of a 1 GB copy (Windows' copy: from 5% ahead to 1-3%, a Marginal Tie).
- **Benchmark, clearer:**
  - "Choppy VS Windows"; the summary reads *Choppy is Faster [10 of 19] · Marginal Tie [9 of 19] · Never Slower* ("Never Slower" only when a whole run ends with none slower);
  - the time decides: each one's average time until the copies are on the disk; **Time saved** shows how much less time Choppy took (green) or more (red); less than 4% apart is a *Marginal Tie*, anything more is *Faster* or *Slower* - no more "tie" for a run that was slower;
  - each case shows the average speed and, under it, the average time;
  - bigger files that show sustained speed: 2 GB where a route does 350 MB/s or more, 1 GB below that (Thorough: 4 GB / 2 GB);
  - while it runs: *Elapsed* and *Remaining* in the copy window's colors, no estimate at the top any more; the HTML report and *Copy results* follow all of this.
- **Close at completion after 2 seconds** (was 4), and the seconds can be set in Settings, next to the switch.
- **Speed limit** starts at 101 Mbps.
- **Show speed in:** each choice explains itself (Auto: Mbps for a network share, MB/s for disks), and a **?** button opens a small chart of 10 / 100 / 1,000 / 10,000 Mbps in MB/s.

### Install / Update
1. Download **Choppy-v1.41.zip** and extract all files (or use the PowerShell one-liner from the README).
2. Double-click **Activate Choppy.cmd** (or run `build.ps1` and choose **[1]**).
3. Remove at any time with **Revert to Windows Copy.cmd**.
