using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using Choppy.Engine;
using Microsoft.Win32;
using Microsoft.Win32.SafeHandles;

namespace Choppy.Bench
{
    /// <summary>
    /// Choppy against Windows' own copy (the shell copy engine Explorer uses) on every drive of this PC: from the
    /// system drive to each other drive and back, and within each drive; one big file and many small files. Both copy
    /// the same source, which is not in memory, in turns (A-B-B-A), and are timed until they say "done" and until the
    /// copies are on the disk. The test files live only in a temporary folder per drive, removed at the end.
    /// </summary>
    internal sealed class Benchmark
    {
        public const long MB = 1L << 20;
        public const double TieMargin = 0.04;    // less than 4% of the time apart is a Marginal Tie

        public bool Thorough;                     // bigger files, a mixed folder, three rounds
        public bool Network;                      // mapped network drives too
        public HashSet<string> Only;              // tests: only these drive letters ("C:", "D:")
        public bool Tiny;                         // tests: very small sizes
        public Action<string> Log;                // a line per step (the command line's --log)
        public Action<JobSpec> Tweak;             // tests: engine switches for Choppy's side
        public HashSet<string> Cases;             // tests: only these cases ("big", "small", "mixed")
        public int BigMB;                         // tests: this big file size on every route
        public int RoundsOverride;                // tests: more rounds
        public HashSet<string> RoutesOnly;        // tests: only these routes ("C>D", "D>D")

        public readonly BenchReport Report = new BenchReport();
        volatile bool cancel;
        public void Cancel() { cancel = true; }
        public bool IsCanceled { get { return cancel; } }
        bool Canceled() { return cancel; }

        int Rounds { get { return RoundsOverride > 0 ? RoundsOverride : Thorough ? 3 : 2; } }

        // ================= drives =================

        /// <summary>Every drive letter with what it is, and why it isn't tested when it isn't.</summary>
        public static List<BenchDrive> FindDrives(bool network, HashSet<string> only)
        {
            var list = new List<BenchDrive>();
            string sys = (Path.GetPathRoot(Environment.SystemDirectory) ?? "C:\\").Substring(0, 2).ToUpperInvariant();
            var seen = new Dictionary<string, string>();
            foreach (var di in DriveInfo.GetDrives())
            {
                var d = new BenchDrive { Root = di.Name, Letter = di.Name.Substring(0, 2).ToUpperInvariant() };
                d.System = d.Letter == sys;
                list.Add(d);
                try
                {
                    if (di.DriveType == DriveType.Network)
                    {
                        d.Network = true;
                        d.Remote = RemotePath(d.Letter);
                        d.Model = d.Remote;
                        if (!network) { d.Skip = "a network drive - turn on Network drives to include it"; continue; }
                    }
                    if (di.DriveType == DriveType.CDRom) { d.Skip = "a disc drive"; continue; }
                    if (!di.IsReady) { d.Skip = "not ready (no disk in it, or locked)"; continue; }
                    d.Label = di.VolumeLabel;
                    d.FileSystem = di.DriveFormat;
                    d.Size = di.TotalSize;
                    d.Free = di.AvailableFreeSpace;
                    if (only != null && !only.Contains(d.Letter)) { d.Skip = "not chosen for this run"; continue; }
                    d.Vol = VolumeProbe.Probe(Paths.ToLong(d.Root));
                    if (!d.Network) ReadModel(d);
                    else d.Model = d.Remote;
                    if ((d.Vol.FsFlags & FILE_READ_ONLY_VOLUME) != 0) { d.Skip = "read-only"; continue; }
                    // a cloud drive (Google Drive, for one) shows up as a fixed drive: it is no disk, and what is written there is uploaded
                    if (!d.Network && !d.RealDisk && d.Vol.Kind != DeviceKind.Ram) { d.Skip = "not a disk of this PC (a virtual or cloud drive)"; continue; }
                    string other;
                    if (d.Vol.Id != null && seen.TryGetValue(d.Vol.Id, out other)) { d.Skip = "the same drive as " + other; continue; }
                    if (d.Vol.Id != null) seen[d.Vol.Id] = d.Letter;
                }
                catch (Exception ex) { d.Skip = "couldn't be read (" + ex.Message + ")"; }
            }
            foreach (var d in list) d.FoundSkip = d.Skip;
            return list;
        }

        const uint FILE_READ_ONLY_VOLUME = 0x00080000;

        [DllImport("mpr.dll", CharSet = CharSet.Unicode)]
        static extern int WNetGetConnectionW(string localName, StringBuilder remoteName, ref int length);

        static string RemotePath(string letter)
        {
            var sb = new StringBuilder(512);
            int n = sb.Capacity;
            return WNetGetConnectionW(letter, sb, ref n) == 0 ? sb.ToString() : null;
        }

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        static extern bool DeviceIoControl(SafeFileHandle device, uint code, ref Native.STORAGE_PROPERTY_QUERY input, int inputSize,
            byte[] output, int outputSize, out int returned, IntPtr overlapped);

        static readonly string[] BusNames = { null, "SCSI", "ATAPI", "ATA", "FireWire", "SSA", "Fibre Channel", "USB", "RAID", "iSCSI", "SAS",
            "SATA", "SD card", "MMC", "Virtual", "Virtual (file)", "Storage Spaces", "NVMe", "SCM", "UFS" };

        /// <summary>The disk's model and bus from the drive itself (no administrator rights needed).</summary>
        static void ReadModel(BenchDrive d)
        {
            try
            {
                using (var h = Native.CreateFileW(@"\\.\" + d.Letter, 0, Native.FILE_SHARE_READ | Native.FILE_SHARE_WRITE, IntPtr.Zero, Native.OPEN_EXISTING, 0, IntPtr.Zero))
                {
                    if (h.IsInvalid) return;
                    // a disk answers as a disk device (a cloud drive's volume answers as a file system) and describes itself
                    Native.STORAGE_DEVICE_NUMBER dn;
                    int dret;
                    bool disk = Native.DeviceIoControl(h, Native.IOCTL_STORAGE_GET_DEVICE_NUMBER, IntPtr.Zero, 0, out dn, Marshal.SizeOf(typeof(Native.STORAGE_DEVICE_NUMBER)), out dret, IntPtr.Zero)
                        && VolumeProbe.DiskNumber(dn) >= 0;
                    var q = new Native.STORAGE_PROPERTY_QUERY { PropertyId = 0, QueryType = 0 };
                    var buf = new byte[1024];
                    int ret;
                    if (!DeviceIoControl(h, Native.IOCTL_STORAGE_QUERY_PROPERTY, ref q, Marshal.SizeOf(q), buf, buf.Length, out ret, IntPtr.Zero) || ret < 32) return;
                    int vendor = BitConverter.ToInt32(buf, 12), product = BitConverter.ToInt32(buf, 16), bus = BitConverter.ToInt32(buf, 28);
                    string model = (Ascii(buf, vendor, ret) + " " + Ascii(buf, product, ret)).Trim();
                    d.Model = model.Length > 0 ? model : null;
                    d.Bus = bus > 0 && bus < BusNames.Length ? BusNames[bus] : null;
                    d.RealDisk = disk;
                }
            }
            catch { }
        }

        static string Ascii(byte[] b, int at, int len)
        {
            if (at <= 0 || at >= len) return "";
            int end = at;
            while (end < len && b[end] != 0) end++;
            return Encoding.ASCII.GetString(b, at, end - at).Trim();
        }

        // ================= the plan =================

        internal sealed class Route
        {
            public BenchDrive From, To;
            public long Big;                 // the big file
            public int Small;                // how many small files
            public List<BenchCase> Cases = new List<BenchCase>();
            public double Est;               // seconds, with what is known of the drives
            public bool Within { get { return From == To; } }
        }

        /// <summary>System drive to each other drive and back, and within each drive (the system drive first).</summary>
        internal static List<Route> PlanRoutes(IList<BenchDrive> usable)
        {
            var r = new List<Route>();
            var hub = usable.FirstOrDefault(d => d.System) ?? usable.FirstOrDefault();
            if (hub == null) return r;
            r.Add(new Route { From = hub, To = hub });
            foreach (var o in usable.Where(d => d != hub).OrderBy(d => d.Network).ThenBy(d => d.Letter))
            {
                r.Add(new Route { From = hub, To = o });
                r.Add(new Route { From = o, To = hub });
                r.Add(new Route { From = o, To = o });
            }
            return r;
        }

        // Sizes go by the route's speed in three steps (not a continuous scale): the same PC gets the same sizes every
        // time, so runs of different versions compare. Quick: 2 GB + 1,000 small files where it goes at 350 MB/s or
        // more, 1 GB + 500 below that, 512 MB + 250 under 40 MB/s. Thorough: 4 GB / 2 GB / 1 GB and 2,000 / 1,000 / 500
        // small files, a mixed folder, three rounds. A big file is big: a copy of a few seconds is mostly its start and
        // where on the platter it lands (512 MB within a hard disk moved by up to 20% from one run to the next).
        const double FastMBps = 350, SlowMBps = 40;
        static readonly long[] MixedBig = { 96 * MB, 160 * MB, 256 * MB };
        const int MixedSmall = 400;
        double BudgetSeconds { get { return Thorough ? 45 * 60 : 15 * 60; } }

        /// <summary>Speed class of a route: 0 = fast, 1 = medium, 2 = slow.</summary>
        static int SpeedClass(Route r)
        {
            double sp = BigMBps(r);
            return sp >= FastMBps ? 0 : sp >= SlowMBps ? 1 : 2;
        }

        long BigFor(int step) { return Tiny ? 64 * MB : (Thorough ? 4096 * MB : 2048 * MB) >> Math.Min(step, 3); }
        int SmallFor(int step) { return Tiny ? 100 : Math.Max(125, (Thorough ? 2000 : 1000) >> Math.Min(step, 3)); }

        /// <summary>Space a drive needs at most: one source and one copy of everything, and room to spare.</summary>
        long NeedBytes { get { return 2 * (BigFor(0) + BenchFiles.SmallSizes(SmallFor(0)).Sum() + (Thorough ? MixedBig.Sum() + BenchFiles.SmallSizes(MixedSmall).Sum() : 0)) + 1024 * MB; } }

        /// <summary>Sizes per route from its speed class (one step smaller for every route when that is over the time budget); the estimate of each.</summary>
        internal void SizeRoutes(List<Route> routes)
        {
            for (int down = 0; down <= 1; down++)
            {
                foreach (var r in routes)
                {
                    int step = SpeedClass(r) + down;
                    r.Big = BigFor(step);
                    r.Small = SmallFor(step);
                }
                if (Tiny || routes.Sum(x => Estimate(x)) <= BudgetSeconds) break;
            }
            foreach (var r in routes)
            {
                r.Cases.Clear();
                r.Cases.Add(new BenchCase { Key = "big", Big = new[] { r.Big }, Smalls = new long[0] });
                r.Cases.Add(new BenchCase { Key = "small", Big = new long[0], Smalls = BenchFiles.SmallSizes(r.Small) });
                if (Thorough) r.Cases.Add(new BenchCase { Key = "mixed", Big = MixedBig, Smalls = BenchFiles.SmallSizes(MixedSmall) });
                if (BigMB > 0) r.Cases[0].Big = new[] { BigMB * MB };
                if (Cases != null) r.Cases.RemoveAll(c => !Cases.Contains(c.Key));
                r.Est = Estimate(r);
            }
        }

        /// <summary>Both ends on one disk with heads: every byte is read and written by them.</summary>
        static bool OneDisk(Route r)
        {
            return (r.Within || VolumeProbe.SameDisk(r.From.Vol, r.To.Vol)) && (Slow(r.From) || Slow(r.To));
        }

        /// <summary>MB/s a big file can go at on this route: the slower end, both shared on one hard disk, or an SSD that reads and writes at once.</summary>
        static double BigMBps(Route r)
        {
            double rd = r.From.ReadMBps, wr = r.To.WriteMBps;
            if (OneDisk(r)) return 1 / (1 / rd + 1 / wr);
            if (r.Within) return Math.Min(rd, wr) * 0.7;
            return Math.Min(rd, wr);
        }

        static bool Slow(BenchDrive d) { return d.Network || (d.Vol != null && d.Vol.IsSlowSeek); }

        /// <summary>Seconds a route takes: its sources, then every run of every case with the flush, the delete and a pause.</summary>
        double Estimate(Route r)
        {
            double sp = BigMBps(r);
            bool slow = Slow(r.From) || Slow(r.To), one = OneDisk(r);
            double rtt = Math.Max(r.From.Vol != null ? r.From.Vol.RttMs : 0, r.To.Vol != null ? r.To.Vol.RttMs : 0);
            double perFile = r.From.Network || r.To.Network ? 0.004 + 3 * Math.Max(0, rtt) / 1000 : slow ? (one ? 0.007 : 0.004) : 0.0004;
            double tidy = (slow ? 0.75 : 0.25) + 0.4;               // pause between runs + the other program's start, the warm-up
            int runs = 2 * Rounds;
            double t = 0;
            var cases = new List<KeyValuePair<long, int>> { new KeyValuePair<long, int>(r.Big, 0), new KeyValuePair<long, int>(BenchFiles.SmallSizes(r.Small).Sum(), r.Small) };
            if (Thorough) cases.Add(new KeyValuePair<long, int>(MixedBig.Sum() + BenchFiles.SmallSizes(MixedSmall).Sum(), MixedSmall + MixedBig.Length));
            foreach (var c in cases)
            {
                double bytes = c.Key / (double)MB;
                int files = c.Value;
                double make = bytes / r.From.WriteMBps + bytes / r.From.ReadMBps + files * (Slow(r.From) ? 0.005 : 0.0005);
                double copy = bytes / sp + files * perFile;
                double after = files * (slow ? 0.0015 : 0.0002) + files * 0.0002;   // flush + delete; purge of the source
                t += make + runs * (copy + after + tidy);
            }
            return t;
        }

        /// <summary>Speeds assumed before a drive is measured (only for the first estimate).</summary>
        static void Assume(BenchDrive d)
        {
            if (d.Measured) return;
            string k = d.Kind;
            if (d.Network) { d.ReadMBps = 60; d.WriteMBps = 40; }
            else if (k == "NVMe SSD") { d.ReadMBps = 2000; d.WriteMBps = 1500; }
            else if (k == "USB SSD") { d.ReadMBps = 350; d.WriteMBps = 300; }
            else if (k.EndsWith("SSD")) { d.ReadMBps = 450; d.WriteMBps = 400; }
            else if (k.Contains("hard disk") || k == "Hard disk") { d.ReadMBps = 170; d.WriteMBps = 170; }
            else if (k == "USB drive") { d.ReadMBps = 100; d.WriteMBps = 60; }
            else if (k == "RAM disk") { d.ReadMBps = 4000; d.WriteMBps = 4000; }
            else { d.ReadMBps = 300; d.WriteMBps = 250; }
        }

        double ProbeSeconds(BenchDrive d)
        {
            long size = ProbeSize(d);
            return size / (double)MB / d.WriteMBps + 2 * size / (double)MB / d.ReadMBps + 1;
        }

        long ProbeSize(BenchDrive d) { return Tiny ? 32 * MB : d.Network ? 64 * MB : (d.Vol != null && d.Vol.IsSlowSeek) ? 128 * MB : 256 * MB; }

        /// <summary>The drives that would be tested and the whole run's seconds, before anything is written.</summary>
        public double Preview(List<BenchDrive> drives)
        {
            foreach (var d in drives) d.Skip = d.FoundSkip;
            var usable = drives.Where(d => d.Skip == null).ToList();
            foreach (var d in usable)
            {
                Assume(d);
                if (d.Free < NeedBytes) d.Skip = "only " + Gb(d.Free) + " free; the test needs " + Gb(NeedBytes);
            }
            usable = usable.Where(d => d.Skip == null).ToList();
            var routes = PlanRoutes(usable);
            SizeRoutes(routes);
            return routes.Sum(r => r.Est) + usable.Sum(d => ProbeSeconds(d));
        }

        static string Gb(long b) { return (b / (double)(1L << 30)).ToString(b < (10L << 30) ? "0.0" : "0", System.Globalization.CultureInfo.InvariantCulture) + " GB"; }

        // ================= progress =================

        readonly object progLock = new object();
        readonly Stopwatch clock = new Stopwatch();
        double estTotal = 1, estDone, stepEst;
        readonly Stopwatch stepClock = new Stopwatch();
        string stepText = "Starting";

        void Step(string text, double est)
        {
            lock (progLock)
            {
                estDone += stepEst;
                stepEst = Math.Max(0, est);
                stepText = text;
                stepClock.Restart();
            }
            var l = Log;
            if (l != null) try { l(clock.Elapsed.TotalSeconds.ToString("0.0", System.Globalization.CultureInfo.InvariantCulture) + "s " + text); } catch { }
        }

        /// <summary>Where the run is: the fraction done, what runs now, time so far and an estimate of the rest.</summary>
        public BenchStatus Status()
        {
            lock (progLock)
            {
                double inStep = Math.Min(stepClock.Elapsed.TotalSeconds, stepEst * 0.95);
                double now = estDone + inStep;
                double frac = Math.Max(0, Math.Min(1, now / Math.Max(1, estTotal)));
                double elapsed = clock.Elapsed.TotalSeconds;
                // the estimate learns from how long the steps so far took against their guesses
                double pace = now > 20 ? Math.Max(0.5, Math.Min(3, elapsed / now)) : 1;
                var s = new BenchStatus
                {
                    Fraction = frac,
                    Text = stepText,
                    Elapsed = clock.Elapsed,
                    Remaining = TimeSpan.FromSeconds(Math.Max(0, (estTotal - now) * pace)),
                    Results = Report.Results.Count,
                };
                return s;
            }
        }

        /// <summary>The cases finished so far (a copy of the list: the run adds to it from its own thread).</summary>
        public List<BenchResult> ResultsSoFar()
        {
            lock (progLock) return new List<BenchResult>(Report.Results);
        }

        // ================= the run =================

        BenchDrive[] used;
        int copyNo;

        /// <summary>Runs everything (on the calling thread) and returns the report; the test files are gone after it.</summary>
        public BenchReport Run()
        {
            clock.Start();
            Report.Started = DateTime.Now;
            Report.Mode = Tiny ? "tiny" : Thorough ? "thorough" : "quick";
            Report.Rounds = Rounds;
            Report.ReadMachine();
            Native.SetThreadExecutionState(Native.ES_CONTINUOUS | Native.ES_SYSTEM_REQUIRED);   // no sleep while it runs
            var held = BenchFiles.Lock();
            try
            {
                if (held == null) throw new InvalidOperationException("Another Benchmark is running (in another Choppy window or on the command line).");
                Step("Removing what an earlier run left behind", 1);
                BenchFiles.CleanLeftovers();
                Step("Looking at the drives", 2);
                var drives = FindDrives(Network, Only);
                Report.Drives = drives;
                foreach (var d in drives.Where(x => x.Skip == null)) Assume(d);
                lock (progLock) estTotal = Preview(drives) + 3;
                var usable = drives.Where(d => d.Skip == null).ToList();

                // a temporary folder per drive, then each drive's own speed (and whether a file can be dropped from the cache there)
                foreach (var d in usable)
                {
                    if (cancel) throw new OperationCanceledException();
                    string why;
                    d.TestDir = BenchFiles.MakeFolder(d.Root, d.System, out why);
                    if (d.TestDir == null) d.Skip = "Choppy can't make a folder there (" + why + ")";
                }
                usable = usable.Where(d => d.Skip == null).ToList();
                used = usable.ToArray();
                foreach (var d in usable)
                {
                    Step("Measuring " + d.Letter + " (" + d.Kind + ")", ProbeSeconds(d));
                    MeasureDrive(d);
                }
                if (usable.Count == 0) throw new InvalidOperationException("No drive of this PC can be tested (each one's reason is listed).");

                var routes = PlanRoutes(usable);
                if (RoutesOnly != null) routes.RemoveAll(r => !RoutesOnly.Contains(r.From.Letter.Substring(0, 1) + ">" + r.To.Letter.Substring(0, 1)));
                SizeRoutes(routes);
                lock (progLock) estTotal = estDone + stepEst + routes.Sum(r => r.Est) + usable.Count * 1.5;
                Report.Planned = routes.Sum(r => r.Cases.Count);
                if (Log != null) Log("plan: " + string.Join(", ", routes.Select(r => r.From.Letter + ">" + r.To.Letter + " " + (r.Big / MB) + " MB + " + r.Small + " files")) + "; about " + BenchText.Duration(routes.Sum(r => r.Est)));
                for (int i = 0; i < routes.Count; i++) RunRoute(routes[i], i + 1);
            }
            catch (OperationCanceledException) { Report.Canceled = true; }
            catch (Exception ex) { Report.Error = ex.Message; }
            finally
            {
                Step("Removing the test files", 3);
                if (Report.Drives != null)
                    foreach (var d in Report.Drives)
                        if (d.TestDir != null && !BenchFiles.Remove(d.TestDir))
                            Report.Notes.Add("Some test files on " + d.Letter + " couldn't be removed yet (" + d.TestDir + "); Choppy removes them at its next start.");
                if (held != null) held.Dispose();
                Native.SetThreadExecutionState(Native.ES_CONTINUOUS);
                clock.Stop();
                Report.Seconds = clock.Elapsed.TotalSeconds;
            }
            return Report;
        }

        /// <summary>Writes a file past the cache (its write speed), reads it twice (the disk, then the cache), drops it from the cache and reads it again.</summary>
        void MeasureDrive(BenchDrive d)
        {
            string f = Path.Combine(d.TestDir, "Drive speed.bin");
            long size = ProbeSize(d);
            try
            {
                // a drive that sleeps (a USB drive, a hard disk that spun down) wakes first: measured here, E: answered at
                // 41 MB/s for its first seconds and copied at 440-900 MB/s after
                string wake = Path.Combine(d.TestDir, "Wake up.bin");
                BenchFiles.WriteBig(wake, 8 * MB, Canceled, null);
                BenchFiles.Purge(new[] { wake }, Canceled);
                ReadSpeed(wake, 8 * MB);
                File.Delete(wake);
                var sw = Stopwatch.StartNew();
                BenchFiles.WriteBig(f, size, Canceled, null);
                d.WriteMBps = size / (double)MB / Math.Max(0.001, sw.Elapsed.TotalSeconds);
                double disk = ReadSpeed(f, size), cached = ReadSpeed(f, size);
                BenchFiles.Purge(new[] { f }, Canceled);
                double again = ReadSpeed(f, size);
                d.ReadMBps = disk;
                // dropped from the cache: the third read is back near the disk's speed. When the cache was no faster than the
                // drive there is nothing to tell: the copies then start from a freshly written source each time anyway
                d.PurgeOk = cached > disk * 1.5 && again < (disk + cached) / 2;
                d.PurgeNote = cached <= disk * 1.5 ? "unclear" : d.PurgeOk ? "yes" : "no";
                if (d.PurgeOk) d.ReadMBps = Math.Max(disk, again);   // both came from the disk: the better one
                d.Measured = true;
            }
            finally { try { File.Delete(f); } catch { } }
        }

        double ReadSpeed(string f, long size)
        {
            var buf = new byte[1 << 20];
            var sw = Stopwatch.StartNew();
            using (var fs = new FileStream(f, FileMode.Open, FileAccess.Read, FileShare.Read, 1 << 20, FileOptions.SequentialScan))
                while (fs.Read(buf, 0, buf.Length) > 0) if (cancel) throw new OperationCanceledException();
            return size / (double)MB / Math.Max(0.001, sw.Elapsed.TotalSeconds);
        }

        void RunRoute(Route r, int no)
        {
            string srcDir = Path.Combine(r.From.TestDir, "Source " + no);
            Directory.CreateDirectory(srcDir);
            // a tiny file each program copies first, untimed (the first copy of a process loads what it needs)
            string warm = Path.Combine(srcDir, "Warm-up.bin");
            File.WriteAllBytes(warm, new byte[64 << 10]);
            int caseNo = 0;
            foreach (var c in r.Cases)
            {
                if (cancel) throw new OperationCanceledException();
                var res = new BenchResult { From = r.From, To = r.To, Case = c };
                string label = r.From.Letter + " → " + r.To.Letter + " · " + c.Title;
                try
                {
                    Step(label + " · writing the test files", MakeSeconds(r, c));
                    string src;
                    List<string> files = MakeSource(r, c, srcDir, out src);
                    Prime(files);
                    // A-B-B-A (-A-B): each one goes first and last as often, and the next case starts with the other one
                    string order = string.Concat(Enumerable.Repeat(caseNo % 2 == 0 ? "CWWC" : "WCCW", Rounds)).Substring(0, 2 * Rounds);
                    double perRun = (r.Est - Enumerable.Range(0, r.Cases.Count).Sum(i => MakeSeconds(r, r.Cases[i]))) / (2.0 * Rounds * r.Cases.Count);
                    int[] seen = new int[2];
                    for (int k = 0; k < order.Length; k++)
                    {
                        bool choppy = order[k] == 'C';
                        int nth = ++seen[choppy ? 0 : 1];
                        Step(label + " · " + (choppy ? "Choppy" : "Windows") + ", run " + nth + " of " + Rounds, Math.Max(0.5, perRun));
                        // the source is read from the disk every time: dropped from the cache, or written again where that can't be done
                        if (k > 0)
                        {
                            if (r.From.PurgeOk) BenchFiles.Purge(files, Canceled);
                            else { BenchFiles.DeleteTree(Paths.ToLong(src)); files = MakeSource(r, c, srcDir, out src); Prime(files); }
                        }
                        Thread.Sleep(Slow(r.From) || Slow(r.To) ? 750 : 250);    // the disks settle (what the last delete left to write)
                        res.Runs.Add(CopyOnce(r, c, choppy, src, warm, res));
                    }
                    res.Score();
                }
                catch (OperationCanceledException) { throw; }
                catch (Exception ex) { res.Error = ex.Message; }
                finally
                {
                    foreach (var dir in Directory.GetFileSystemEntries(srcDir))
                        if (!string.Equals(dir, warm, StringComparison.OrdinalIgnoreCase)) BenchFiles.DeleteTree(Paths.ToLong(dir));
                }
                lock (progLock) Report.Results.Add(res);
                caseNo++;
            }
            BenchFiles.DeleteTree(Paths.ToLong(srcDir));
        }

        double MakeSeconds(Route r, BenchCase c)
        {
            return c.Bytes / (double)MB / r.From.WriteMBps + c.Bytes / (double)MB / r.From.ReadMBps + c.Files * (Slow(r.From) ? 0.006 : 0.0006) + 0.2;
        }

        /// <summary>The case's source in the route's source folder: a file (one big file) or a folder. Returns its files.</summary>
        List<string> MakeSource(Route r, BenchCase c, string srcDir, out string src)
        {
            var files = new List<string>();
            if (c.Key == "big")
            {
                src = Path.Combine(srcDir, "Big file.bin");
                BenchFiles.WriteBig(src, c.Big[0], Canceled, null);
                files.Add(src);
                return files;
            }
            src = Path.Combine(srcDir, c.Key == "small" ? "Small files" : "Mixed folder");
            Directory.CreateDirectory(src);
            for (int i = 0; i < c.Big.Length; i++)
            {
                string f = Path.Combine(src, "Video " + (i + 1) + ".bin");
                BenchFiles.WriteBig(f, c.Big[i], Canceled, null);
                files.Add(f);
            }
            files.AddRange(BenchFiles.WriteSmall(src, c.Smalls, Canceled, null));
            return files;
        }

        /// <summary>
        /// A new file's first open is slower (a virus scanner reads it then): every source file is read once, untimed,
        /// and then dropped from the cache, so every run - the first one too - starts from the same state. Measured
        /// before: the first of four runs was up to 4x slower (1 GB within the NVMe: 2.22 s, then 0.52 s).
        /// </summary>
        void Prime(List<string> files)
        {
            var buf = new byte[1 << 20];
            foreach (var f in files)
            {
                if (cancel) throw new OperationCanceledException();
                using (var fs = new FileStream(f, FileMode.Open, FileAccess.Read, FileShare.Read, 1 << 16, FileOptions.SequentialScan))
                    while (fs.Read(buf, 0, buf.Length) > 0) { }
            }
            BenchFiles.Purge(files, Canceled);
        }

        /// <summary>One timed copy into a fresh folder on the destination, then flushed (the time to the disk), checked and deleted.</summary>
        BenchRun CopyOnce(Route r, BenchCase c, bool choppy, string src, string warm, BenchResult res)
        {
            int n = ++copyNo;
            string dest = Path.Combine(r.To.TestDir, "Copy " + n);
            string warmDest = Path.Combine(r.To.TestDir, "Warm-up " + n);
            Directory.CreateDirectory(dest);
            Directory.CreateDirectory(warmDest);
            try
            {
                double done;
                if (choppy)
                {
                    string m;
                    ChoppyCopy(warm, warmDest, out m);
                    done = ChoppyCopy(src, dest, out m);
                    if (m != null) res.Method = m;
                }
                else done = ShellCopy.Run(warm, warmDest, src, dest, Canceled);
                var copies = BenchFiles.List(dest);
                var sw = Stopwatch.StartNew();
                BenchFiles.Flush(copies.Select(x => x.Key).ToList(), Canceled);
                double flush = sw.Elapsed.TotalSeconds;
                long bytes = copies.Sum(x => x.Value);
                if (copies.Count != c.Files || bytes != c.Bytes)
                    throw new IOException((choppy ? "Choppy's" : "Windows'") + " copy has " + copies.Count + " of " + c.Files + " files, " + bytes + " of " + c.Bytes + " bytes");
                return new BenchRun { Choppy = choppy, Done = done, OnDisk = done + flush };
            }
            finally
            {
                BenchFiles.DeleteTree(Paths.ToLong(dest));
                BenchFiles.DeleteTree(Paths.ToLong(warmDest));
            }
        }

        /// <summary>Choppy's engine as any copy runs it (Adaptive picks the way; the pipeline and its tuner as usual). Seconds until it says done.</summary>
        double ChoppyCopy(string src, string dest, out string method)
        {
            var spec = new JobSpec { Op = OpKind.Copy, Origin = "Benchmark", Policy = ConflictPolicy.Skip, Dest = Paths.ToLong(dest) };
            spec.Sources.Add(Paths.ToLong(src));
            if (Tweak != null) Tweak(spec);
            var job = new CopyJob(spec);
            job.AskConflict = i => new ConflictAnswer { Choice = ConflictPolicy.Skip };
            job.AskLowSpace = (need, free) => false;
            job.AskElevate = () => false;
            double secs = 0;
            var sw = new Stopwatch();
            using (var finished = new ManualResetEventSlim())
            {
                job.Finished += j => { secs = sw.Elapsed.TotalSeconds; finished.Set(); };
                sw.Start();
                job.Start();
                while (!finished.Wait(50)) if (cancel) job.Cancel();
            }
            if (cancel) throw new OperationCanceledException();
            method = job.Plan != null ? job.Plan.Badge + (!string.IsNullOrEmpty(job.TuneSummary) ? " (" + job.TuneSummary + ")" : "") : null;
            if (job.Phase != JobPhase.Done || job.FailedFiles > 0 || job.SkippedFiles > 0)
            {
                string why = job.FailedItems.Count > 0 ? job.FailedItems[0].Error : job.Status ?? job.Phase.ToString();
                throw new IOException("Choppy's copy didn't finish: " + why);
            }
            return secs;
        }

        // ================= the verdict =================

        /// <summary>
        /// The time decides: saved = how much less time Choppy took than Windows, as a share of Windows' time (their
        /// average times; below 0 = Choppy took longer). +1 = Faster, -1 = Slower, 0 = a Marginal Tie (less than 4% apart).
        /// </summary>
        public static int Judge(IList<double> choppySecs, IList<double> windowsSecs, out double saved)
        {
            saved = 0;
            if (choppySecs.Count == 0 || windowsSecs.Count == 0) return 0;
            double c = Mean(choppySecs), w = Mean(windowsSecs);
            if (c <= 0 || w <= 0) return 0;
            saved = (w - c) / w;
            if (Math.Abs(saved) < TieMargin) return 0;
            return saved > 0 ? 1 : -1;
        }

        public static double Mean(IList<double> x) { return x.Count == 0 ? 0 : x.Average(); }
    }

    internal sealed class BenchStatus
    {
        public double Fraction;
        public string Text;
        public TimeSpan Elapsed, Remaining;
        public int Results;
    }
}
