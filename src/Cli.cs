using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text;
using System.Threading;
using Choppy.Engine;

namespace Choppy
{
    /// <summary>
    /// Headless runner for tests:
    ///   Choppy.exe --cli copy|move &lt;src...&gt; &lt;destDir&gt; [--verify] [--policy ask|replace|skip|keepboth|newer]
    ///                 [--limit Mbps] [--resume] [--json out.json] [--log progress.txt] [--cancel-after sec] [--pause-at sec,sec]
    ///   Choppy.exe --selftest out.txt
    ///   Choppy.exe --usb [out.txt]       how each USB drive is connected (runs at, can do, port, UASP)
    ///   Choppy.exe --benchmark [out.json] [--thorough] [--network] [--drives C,D] [--list] [--log progress.txt] [--cancel-after sec] [--tiny]
    ///                 tests: [--routes C>D,D>D] [--cases big|small] [--big MB] [--rounds N] [--choppy "--chunk 1 --slots 512 ..."]
    ///                 the move to Rust (rust branch only): [--rust choppy.exe] = C#, Rust and Windows in turns [--rust-fresh] [--size-down 0|1]
    ///   Choppy.exe --benchcopy job.txt   (the Benchmark's child: one timed copy by this engine; Bench.EngineChild)
    /// Exit code: 0 = all done, 2 = some files failed/skipped, 3 = canceled, 1 = error.
    /// </summary>
    internal static class Cli
    {
        public static int Run(string[] args)
        {
            try
            {
                if (args.Length >= 1 && args[0] == "--selftest") return SelfTest(args.Length > 1 ? args[1] : null);
                if (args.Length >= 1 && args[0] == "--where") return Where(args.Length > 1 ? args[1] : null);
                if (args.Length >= 1 && args[0] == "--usb") return Usb(args.Length > 1 ? args[1] : null);
                if (args.Length >= 2 && args[0] == "--plan-dump") return PlanDump(args);
                if (args.Length >= 1 && args[0] == "--benchmark") return BenchCli(args);
                if (args.Length >= 2 && args[0] == "--shellcopy") return Bench.ShellCopy.Child(args[1]);
                if (args.Length >= 2 && args[0] == "--benchcopy") return Bench.EngineChild.Child(args[1]);   // step 3 (rust branch only)
                var rest = new List<string>();
                string json = null, log = null;
                var spec = new JobSpec { Origin = "CLI" };
                double cancelAfter = 0, pauseAt = 0, pauseFor = 0;
                string skipAt = null;
                for (int i = 1; i < args.Length; i++)
                {
                    string a = args[i];
                    switch (a)
                    {
                        case "--verify": spec.Verify = true; break;
                        case "--resume": spec.ResumeMode = true; break;
                        case "--policy": spec.Policy = ParsePolicy(args[++i]); break;
                        case "--limit": spec.SpeedLimitMbps = double.Parse(args[++i], CultureInfo.InvariantCulture); break;
                        case "--json": json = args[++i]; break;
                        case "--log": log = args[++i]; break;
                        case "--cancel-after": cancelAfter = double.Parse(args[++i], CultureInfo.InvariantCulture); break;
                        case "--pause": { var p = args[++i].Split(','); pauseAt = double.Parse(p[0], CultureInfo.InvariantCulture); pauseFor = double.Parse(p[1], CultureInfo.InvariantCulture); } break;
                        case "--skip-at": skipAt = args[++i]; break;
                        default: if (!EngineOption(args, ref i, spec)) rest.Add(a); break;
                    }
                }
                if (rest.Count < 3) { Console.Error.WriteLine("usage: --cli copy|move <src...> <dest>"); return 1; }
                spec.Op = rest[0].Equals("move", StringComparison.OrdinalIgnoreCase) ? OpKind.Move : OpKind.Copy;
                for (int i = 1; i < rest.Count - 1; i++) spec.Sources.Add(Paths.ToLong(rest[i]));
                spec.Dest = Paths.ToLong(rest[rest.Count - 1]);

                var job = new CopyJob(spec);
                job.AskConflict = info => new ConflictAnswer { Choice = ConflictPolicy.Skip };
                job.AskLowSpace = (need, free) => false;
                Journal.Save(spec);
                var sw = Stopwatch.StartNew();
                job.Start();
                StreamWriter lw = log != null ? new StreamWriter(log, false, Encoding.UTF8) { AutoFlush = true } : null;
                bool pausedOnce = false, skipped = false;
                while (!job.IsFinished)
                {
                    Thread.Sleep(50);
                    double t = sw.Elapsed.TotalSeconds;
                    if (cancelAfter > 0 && t >= cancelAfter && !job.IsCanceled) job.Cancel();
                    if (pauseAt > 0 && !pausedOnce && t >= pauseAt) { job.Pause(); pausedOnce = true; }
                    if (pausedOnce && job.IsPaused && t >= pauseAt + pauseFor) job.Resume();
                    if (skipAt != null && !skipped)
                    {
                        var cur = job.CurrentTransfer();
                        if (cur != null && cur.File.Name.Equals(skipAt, StringComparison.OrdinalIgnoreCase) && cur.Done > 0) { job.SkipCurrent(); skipped = true; }
                    }
                    if (lw != null)
                    {
                        var cur = job.CurrentTransfer();
                        lw.WriteLine(string.Format(CultureInfo.InvariantCulture, "{0:F1}s {1} {2}/{3} files {4}/{5} bytes {6:F1} Mbps workers={7} badge='{8}' cur={9} status={10}",
                            t, job.Phase, job.DoneFiles, job.TotalFiles, Interlocked.Read(ref job.DoneBytes), job.TotalBytes, job.SpeedNow * 8 / 1e6,
                            job.ActiveCount, job.Plan != null ? job.Plan.Badge : "", cur != null ? cur.File.Name : "-", job.Status));
                    }
                }
                sw.Stop();
                if (lw != null) lw.Dispose();
                if (job.Phase != JobPhase.Canceled) Journal.Remove(spec);
                if (json != null) File.WriteAllText(json, Report(job, sw.Elapsed.TotalSeconds), Encoding.UTF8);
                if (job.Phase == JobPhase.Canceled) return 3;
                if (job.Phase == JobPhase.Failed) return 1;
                return (job.FailedFiles > 0 || job.SkippedFiles > 0) ? 2 : 0;
            }
            catch (Exception ex)
            {
                try { File.WriteAllText(Path.Combine(Path.GetTempPath(), "choppy-cli-error.txt"), ex.ToString()); } catch { }
                return 1;
            }
        }

        /// <summary>
        /// Choppy against Windows' own copy on every drive, headless: writes out.json and out.html (default: the Logs
        /// folder). --list only lists the drives and the time it would take, into out.json as text. Exit code 0 = done,
        /// 3 = stopped, 1 = error.
        /// </summary>
        static int BenchCli(string[] args)
        {
            var b = new Bench.Benchmark();
            string outPath = null, log = null;
            double cancelAfter = 0;
            bool list = false;
            for (int i = 1; i < args.Length; i++)
            {
                switch (args[i])
                {
                    case "--quick": b.Thorough = false; break;
                    case "--thorough": b.Thorough = true; break;
                    case "--network": b.Network = true; break;
                    case "--tiny": b.Tiny = true; break;
                    case "--list": list = true; break;
                    case "--log": log = Full(args[++i]); break;
                    case "--cancel-after": cancelAfter = double.Parse(args[++i], CultureInfo.InvariantCulture); break;
                    // tests: Choppy's side with fixed engine switches (as --cli takes them), only some cases, a fixed big size, more rounds
                    case "--choppy":
                        {
                            b.TweakArgs = args[++i];
                            var opts = b.TweakArgs.Split(new[] { ' ' }, StringSplitOptions.RemoveEmptyEntries);
                            b.Tweak = sp => { for (int k = 0; k < opts.Length; k++) EngineOption(opts, ref k, sp); };
                        }
                        break;
                    // the move to Rust, step 3 (rust branch only): C#, the Rust choppy.exe and Windows in turns (Benchmark.RustExe)
                    case "--rust": b.RustExe = Full(args[++i]); break;
                    case "--rust-fresh": b.RustFresh = true; break;
                    case "--size-down": b.SizeDown = int.Parse(args[++i]); break;
                    case "--cases": b.Cases = new HashSet<string>(args[++i].Split(',')); break;
                    case "--big": b.BigMB = int.Parse(args[++i]); break;
                    case "--rounds": b.RoundsOverride = int.Parse(args[++i]); break;
                    case "--routes": b.RoutesOnly = new HashSet<string>(args[++i].ToUpperInvariant().Split(',')); break;
                    case "--drives":
                        b.Only = new HashSet<string>();
                        foreach (var s in args[++i].Split(',')) if (s.Trim().Length > 0) b.Only.Add(s.Trim().TrimEnd('\\', ':').ToUpperInvariant() + ":");
                        break;
                    default: if (outPath == null && !args[i].StartsWith("--")) outPath = Full(args[i]); break;
                }
            }
            if (list)
            {
                var drives = Bench.Benchmark.FindDrives(b.Network, b.Only);
                double est = b.Preview(drives);
                var sb = new StringBuilder();
                foreach (var d in drives)
                    sb.AppendLine(d.Letter + " " + d.Label + " | " + (d.Vol != null || d.Network ? d.Kind : "") + " | " + d.Model + " | " + d.Bus + " | " + d.FileSystem
                        + " | disk " + (d.Vol != null ? d.Vol.Disk : -1) + " | " + (d.Skip == null ? "tested" : "skipped: " + d.Skip));
                sb.AppendLine("about " + Bench.BenchText.Duration(est) + (b.Thorough ? " (thorough)" : " (quick)"));
                if (outPath != null) File.WriteAllText(outPath, sb.ToString(), Encoding.UTF8);
                return 0;
            }
            if (log != null) Directory.CreateDirectory(Path.GetDirectoryName(log));
            StreamWriter lw = log != null ? new StreamWriter(log, false, Encoding.UTF8) { AutoFlush = true } : null;
            if (lw != null) b.Log = line => { lock (lw) lw.WriteLine(line); };
            Timer stop = cancelAfter > 0 ? new Timer(_ => b.Cancel(), null, (int)(cancelAfter * 1000), Timeout.Infinite) : null;
            var rep = b.Run();
            if (stop != null) stop.Dispose();
            string html, json;
            rep.Save(outPath, out html, out json);
            if (lw != null) { lw.WriteLine(rep.Summary); if (rep.Totals != null) lw.WriteLine(rep.Totals.Line); lw.WriteLine(html); lw.Dispose(); }
            return rep.Canceled ? 3 : rep.Error != null ? 1 : 0;
        }

        /// <summary>A path from the command line, against the folder Choppy was started from.</summary>
        static string Full(string p) { return Path.IsPathRooted(p) ? p : Path.Combine(Program.StartDir ?? "", p); }

        /// <summary>Test switches for the engine (fixed ways instead of Adaptive's choice). False = not one of them.</summary>
        internal static bool EngineOption(string[] args, ref int i, JobSpec spec)
        {
            switch (args[i])
            {
                case "--workers": spec.ForceWorkers = int.Parse(args[++i]); return true;
                case "--no-fast": spec.NoFastPath = true; return true;
                case "--engine": { string e = args[++i]; spec.Engine = e == "auto" ? null : e; } return true;
                case "--chunk": spec.PipeChunkMB = int.Parse(args[++i]); return true;
                case "--slots": spec.PipeSlots = int.Parse(args[++i]); return true;
                case "--batch": spec.PipeBatch = int.Parse(args[++i]); return true;
                case "--depth": spec.PipeDepth = int.Parse(args[++i]); return true;
                case "--wstep": spec.PipeWriteStepMB = int.Parse(args[++i]); return true;
                case "--phased": spec.PipePhased = int.Parse(args[++i]); return true;
                case "--flush-files": spec.FlushFiles = int.Parse(args[++i]); return true;
                case "--flush-threads": spec.FlushThreads = int.Parse(args[++i]); return true;
                case "--no-prealloc": spec.NoPrealloc = true; return true;
                case "--no-tune": spec.NoTune = true; return true;
                case "--prealloc": spec.ForcePrealloc = true; return true;
                case "--small": spec.SmallMode = args[++i]; return true;
                case "--wchunk": spec.CachedWriteMB = int.Parse(args[++i]); return true;
                case "--no-tail": spec.NoCachedTail = true; return true;
                case "--tail": spec.TailMB = int.Parse(args[++i]); return true;
                case "--max-file": spec.MaxFileTest = long.Parse(args[++i]); return true;
            }
            return false;
        }

        static ConflictPolicy ParsePolicy(string s)
        {
            switch (s.ToLowerInvariant())
            {
                case "replace": return ConflictPolicy.Replace;
                case "skip": return ConflictPolicy.Skip;
                case "keepboth": return ConflictPolicy.KeepBoth;
                case "newer": return ConflictPolicy.ReplaceOlder;
            }
            return ConflictPolicy.Ask;
        }

        static string Esc(string s)
        {
            if (s == null) return "null";
            var sb = new StringBuilder("\"");
            foreach (char c in s)
            {
                if (c == '"' || c == '\\') sb.Append('\\').Append(c);
                else if (c < 32) sb.Append("\\u").Append(((int)c).ToString("x4"));
                else sb.Append(c);
            }
            return sb.Append('"').ToString();
        }

        static string Report(CopyJob j, double secs)
        {
            var sb = new StringBuilder();
            sb.Append("{");
            sb.Append("\"phase\":" + Esc(j.Phase.ToString()));
            sb.Append(",\"seconds\":" + secs.ToString("F2", CultureInfo.InvariantCulture));
            sb.Append(",\"badge\":" + Esc(j.Plan != null ? j.Plan.Badge : null));
            sb.Append(",\"tune\":" + Esc(j.TuneSummary));
            sb.Append(",\"engine\":" + Esc(j.Spec.Engine ?? (j.Plan != null && j.Plan.Pipeline ? "pipeline " + PipeTuner.Describe(j.Plan.Pipe) : "copyfile")));
            sb.Append(",\"srcKind\":" + Esc(j.SrcVol != null ? j.SrcVol.KindText : null));
            sb.Append(",\"dstKind\":" + Esc(j.DstVol != null ? j.DstVol.KindText : null));
            sb.Append(",\"rttMs\":" + (j.DstVol != null ? j.DstVol.RttMs : -1).ToString("F1", CultureInfo.InvariantCulture));
            sb.Append(",\"totalFiles\":" + j.TotalFiles + ",\"doneFiles\":" + j.DoneFiles + ",\"skipped\":" + j.SkippedFiles + ",\"failed\":" + j.FailedFiles);
            sb.Append(",\"verified\":" + j.VerifiedFiles + ",\"alreadyThere\":" + j.AlreadyThereFiles);
            sb.Append(",\"totalBytes\":" + j.TotalBytes + ",\"copiedBytes\":" + j.CopiedBytes);
            double run = j.Clock.Elapsed.TotalSeconds;
            sb.Append(",\"runSeconds\":" + run.ToString("F2", CultureInfo.InvariantCulture));
            sb.Append(",\"avgMbps\":" + (run > 0 ? j.CopiedBytes * 8 / 1e6 / run : 0).ToString("F1", CultureInfo.InvariantCulture));
            sb.Append(",\"peakMbps\":" + (j.SpeedPeak * 8 / 1e6).ToString("F1", CultureInfo.InvariantCulture));
            sb.Append(",\"largest\":[");
            if (j.Scan != null) for (int i = 0; i < j.Scan.Largest.Count; i++) { if (i > 0) sb.Append(","); sb.Append(Esc(j.Scan.Largest[i].Key + "=" + j.Scan.Largest[i].Value)); }
            sb.Append("],\"failedItems\":[");
            for (int i = 0; i < j.FailedItems.Count; i++) { if (i > 0) sb.Append(","); sb.Append(Esc(Paths.ToShort(j.FailedItems[i].Src) + " :: " + j.FailedItems[i].Error)); }
            sb.Append("],\"notes\":[");
            for (int i = 0; i < j.Notes.Count; i++) { if (i > 0) sb.Append(","); sb.Append(Esc(j.Notes[i])); }
            sb.Append("],\"log\":" + Esc(j.LogPath));
            sb.Append("}");
            return sb.ToString();
        }

        [System.Runtime.InteropServices.DllImport("user32.dll", CharSet = System.Runtime.InteropServices.CharSet.Unicode)]
        static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string cls, string title);

        /// <summary>Read-only check: how each USB drive is connected (the speed it runs at, the most it can do, the port).</summary>
        static int Usb(string outFile)
        {
            var sb = new StringBuilder();
            foreach (var di in DriveInfo.GetDrives())
            {
                if (di.DriveType == DriveType.Network || di.DriveType == DriveType.CDRom) continue;
                var sw = Stopwatch.StartNew();
                var v = VolumeProbe.Probe(Paths.ToLong(di.Name));
                var link = UsbLink.ForDisk(v.Disk);
                sb.AppendLine(di.Name + " disk " + v.Disk + " " + v.KindText + ": " + (link == null ? "not USB"
                    : link.SpeedText + ", can do " + UsbLink.Short(link.Capable) + (link.CapableKnown ? "" : " (unknown)") + ", port " + (link.PortKnown ? (link.PortUsb3 ? "USB 3" : "USB 2") : "?")
                      + ", bcdUSB " + link.BcdUsb.ToString("X4") + (link.Uasp ? ", UASP" : ", Bulk-Only") + (link.HeldBack ? " - HELD BACK: " + link.Advice : ""))
                    + "  [" + sw.ElapsedMilliseconds + " ms]");
            }
            if (outFile != null) File.WriteAllText(outFile, sb.ToString(), Encoding.UTF8); else Console.Write(sb.ToString());
            return 0;
        }

        // ---------------- --plan-dump (rust branch only: the move to Rust) ----------------
        // A read-only check that the Rust Choppy sees the drives and picks the plan exactly as this one does. Both print
        // the same lines (rust/choppy/src/cli/plan_dump.rs): what VolumeProbe sees on every drive; the plan Plan.Choose
        // picks for every pair of drives, each kind of job and each shape of scan; Paths on tricky input; ErrorText; in
        // test mode (CHOPPY_DATA) the settings as loaded and a job record written and read back.
        // --plan-dump out.txt --scan copy|move <dest> <src>...: the Scanner's result instead (one thread, then four).

        static string B(bool b) { return b ? "1" : "0"; }
        static string G(double d) { return d.ToString(CultureInfo.InvariantCulture); }

        static string VolText(VolumeInfo v)
        {
            var u = v.Usb;
            return "root " + v.Root + " | id " + v.Id + " | kind " + v.Kind + " " + v.KindText + " | fs " + (v.FileSystem ?? "(null)") + " | label " + (v.Label ?? "(null)")
                + " | disk " + v.Disk + " | sector " + v.Sector + " | removable " + B(v.Removable) + " | fsflags " + v.FsFlags.ToString("X8") + " | onusb " + B(v.OnUsb)
                + " | rtt " + (v.RttMs >= 0 ? "measured" : G(v.RttMs)) + " | maxfile " + v.MaxFileSize + " | clone " + B(v.CanClone) + " | slowseek " + B(v.IsSlowSeek)
                + " | usb " + (u == null ? "-" : u.Speed + " " + u.SpeedText + ", can " + u.Capable + (u.CapableKnown ? "" : "?") + ", port " + (u.PortKnown ? (u.PortUsb3 ? "3" : "2") : "?")
                    + ", bcd " + u.BcdUsb.ToString("X4") + ", uasp " + B(u.Uasp) + ", heldback " + B(u.HeldBack) + ", advice " + (u.Advice ?? "-") + ", ceiling " + G(UsbLink.CeilingMBps(u.Speed)));
        }

        static string PipeText(PipeTuning t)
        {
            return PipeTuner.Describe(t) + " (chunk " + t.Chunk + " slots " + t.Slots + " align " + t.Align + " batch " + t.WriteBatch + " phased " + B(t.Phased)
                + " prealloc " + B(t.Prealloc) + " cached " + t.CachedWriteChunk + " tail " + B(t.TailAfterRead) + " depth " + t.ReadDepth + " step " + t.WriteStep + ")";
        }

        static string PlanText(Plan p)
        {
            var s = new StringBuilder();
            s.Append(p.Kind + " | workers " + p.Workers + " max " + p.MaxWorkers + " | big " + p.BigSlots + " from " + p.BigThreshold
                + " | pipeline " + B(p.Pipeline) + " from " + p.PipeThreshold + " " + PipeText(p.Pipe) + " | ways " + p.PipeWays.Count);
            foreach (var w in p.PipeWays) s.Append(" [" + PipeText(w) + "]");
            s.Append(" | instant " + B(p.InstantMove) + " network " + B(p.Network) + " clone " + B(p.Clone) + " oneusb " + B(p.OneUsb) + " smallbatch " + B(p.SmallBatch)
                + " | tune " + B(p.CanTune) + " " + G(p.TuneSeconds) + " s, x" + G(p.TuneMargin) + " tuned " + B(p.Tuned) + " " + (p.TuneNote ?? "-") + " | " + p.Badge + " | " + p.Why);
            return s.ToString();
        }

        const long GB = 1L << 30, MB = 1L << 20, KB = 1L << 10;
        static readonly string[] ShapeNames = { "none", "empty", "1 x 2 GB", "3 x 1 GB", "4 x 1 GB", "20 x 4 MB", "20 x 16 MB", "40 x 512 KB", "40 x 2 MB", "100 x 64 KB", "100 x 16 MB", "1001 mixed" };
        static readonly long[,] Shapes =
        {
            { -1, 0, 0 }, { 0, 0, 0 }, { 1, 2 * GB, 2 * GB }, { 3, GB, GB }, { 4, GB, GB }, { 20, 4 * MB, 4 * MB }, { 20, 16 * MB, 16 * MB },
            { 40, 512 * KB, 512 * KB }, { 40, 2 * MB, 2 * MB }, { 100, 64 * KB, MB }, { 100, 16 * MB, 16 * MB }, { 1001, 100 * KB, 2 * GB },
        };

        static readonly string[] PathInputs =
        {
            @"C:\a\b\", @"C:\", "C:\"", "C:", @"C:\\x\\\\y", "D:\\Games\"", "\"D:\\Games\"", "  C:\\x  ", @"\\srv\share\x", @"\\srv\share\", @"\\srv\share",
            @"\\?\C:\x\\", @"\\?\C:""", @"\\?\UNC\srv\share\x", @"\\.\COM1", @"relative\x", @".\x\..\y", "..", @"C:\a\..\b\.\c", @"C:\a/b//c", @"C:\a\b. ",
            @"C:\a\b...", @"C:\a*b", @"C:\a?b", @"C:\a|b", @"C:\a<b", "a:b", @"C:\x\f:s", ":x", @"1:\x", "C:x", @"\x", @"C:\PROGRA~1\x",
            @"C:\PROGRA~1\Common Files", @"C:\PROGRA~1", @"C:\PROGRA~1\Common Files\x", @"C:\PROGRA~2\..\PROGRA~1", @"C:\PROGRA~1\aaa", @"C:\PROGRA~1\aaaa", @"C:\PROGRA~1\COMMON~1\x", @"\\localhost\C$\PROGRA~1\x", @"C:\Users\Public\..\Public", @"C:\con", @"C:\a\b\NUL", "", "   ", "\t C:\\tab", "\u00e9\\\u00fc",
            "C:\\x\\\U0001F600.txt", "C:\\x\u00A0", "C:\\x\\\u2028",
        };

        static readonly string[,] PathPairs =
        {
            { @"\\?\C:\a\b\c", @"\\?\C:\a\b" }, { @"\\?\C:\a\bc", @"\\?\C:\a\b" }, { @"\\?\C:\A\B", @"\\?\c:\a\b\" }, { @"C:\x", @"\\?\C:\X" },
            { @"\\?\UNC\srv\share\x", @"\\srv\SHARE" }, { "\\\\?\\C:\\\u00c4\\x", "\\\\?\\C:\\\u00e4" }, { "\\\\?\\C:\\\u00df", "\\\\?\\C:\\SS" },
            { "\\\\?\\C:\\\u0131", "\\\\?\\C:\\I" }, { "\\\\?\\C:\\\u03a3", "\\\\?\\C:\\\u03c3" }, { @"\\?\C:\a", @"\\?\C:\a" },
        };

        static readonly string[] NumberedNames = { "photo.jpg", "README", ".hidden", "dir.name", "a.b.c", "x." };
        static readonly int[] NumberedN = { 2, 3, 2, 2, 10, 4 };
        static readonly bool[] NumberedDir = { false, false, false, true, false, false };

        static readonly long[] MoreErrors = { 2250, 4390, 4392, 6800, 8202, 10054, 10060, 12002, 0x7FFFFFFF, -1, -2147024891 };

        static int PlanDump(string[] args)
        {
            var sb = new StringBuilder();
            if (args.Length > 2 && args[2] == "--scan") DumpScan(args, sb);
            else
            {
                sb.AppendLine("drives");
                var vols = new List<KeyValuePair<string, VolumeInfo>>();
                foreach (var di in DriveInfo.GetDrives())
                {
                    var v = VolumeProbe.Probe(Paths.ToLong(di.Name));
                    string letter = di.Name.Substring(0, 2);
                    sb.AppendLine(letter + " " + VolText(v));
                    vols.Add(new KeyValuePair<string, VolumeInfo>(letter, v));
                }
                // a network share too (this PC's own, through the network): network plans for every shape
                var net = VolumeProbe.Probe(Paths.ToLong(@"\\localhost\C$\"));
                sb.AppendLine("UNC " + VolText(net).Replace(" | rtt measured", " | rtt -"));
                vols.Add(new KeyValuePair<string, VolumeInfo>("UNC", net));
                sb.AppendLine("plans");
                var opNames = new[] { "copy", "move", "retry move" };
                var ops = new[] { OpKind.Copy, OpKind.Move, OpKind.Move };
                foreach (var a in vols)
                    foreach (var b in vols)
                        for (int o = 0; o < ops.Length; o++)
                        {
                            var spec = new JobSpec { Op = ops[o] };
                            if (o == 2) spec.ExplicitItems = new List<FileItem>();
                            for (int k = 0; k < ShapeNames.Length; k++)
                            {
                                ScanResult scan = null;
                                if (Shapes[k, 0] >= 0)
                                {
                                    scan = new ScanResult();
                                    for (long i = 0; i < Shapes[k, 0]; i++) scan.Files.Add(new FileItem());
                                    scan.MedianSize = Shapes[k, 1];
                                    if (Shapes[k, 0] > 0) scan.Largest.Add(new KeyValuePair<string, long>("big.bin", Shapes[k, 2]));
                                }
                                var p = Plan.Choose(spec, a.Value, b.Value, scan);
                                sb.AppendLine(a.Key + ">" + b.Key + " " + opNames[o] + " " + ShapeNames[k] + " | " + PlanText(p));
                            }
                        }
                // no drive at one end (a job with no sources)
                sb.AppendLine("(none)>first copy none | " + PlanText(Plan.Choose(new JobSpec { Op = OpKind.Copy }, null, vols.Count > 0 ? vols[0].Value : null, null)));

                sb.AppendLine("paths");
                foreach (var i in PathInputs)
                {
                    string l = Paths.ToLong(i);
                    sb.AppendLine("[" + i + "] long [" + l + "] short [" + Paths.ToShort(l) + "] fix [" + Paths.FixArg(i) + "] parent [" + (Paths.Parent(l) ?? "(null)")
                        + "] name [" + Paths.FileName(l) + "] unc " + B(Paths.IsUnc(l)) + " exists " + B(Paths.Exists(l)));
                }
                for (int k = 0; k < PathPairs.GetLength(0); k++)
                    sb.AppendLine("[" + PathPairs[k, 0] + "] [" + PathPairs[k, 1] + "] inside " + B(Paths.IsSameOrInside(PathPairs[k, 0], PathPairs[k, 1])) + " same " + B(Paths.SamePath(PathPairs[k, 0], PathPairs[k, 1])));
                for (int k = 0; k < NumberedNames.Length; k++)
                    sb.AppendLine("[" + NumberedNames[k] + "] " + NumberedN[k] + " " + B(NumberedDir[k]) + " => [" + Paths.NumberedName(NumberedNames[k], NumberedN[k], NumberedDir[k]) + "]");
                sb.AppendLine("errors");
                for (long code = 0; code < 2000; code++) sb.AppendLine(code + " " + Native.ErrorText((int)code));
                foreach (long code in MoreErrors) sb.AppendLine(code + " " + Native.ErrorText(unchecked((int)code)));
                if (AppPaths.TestMode) DumpSettings(sb);
            }
            File.WriteAllText(args[1], sb.ToString());
            return 0;
        }

        static void DumpSettings(StringBuilder sb)
        {
            sb.AppendLine("settings");
            var s = Settings.Load();
            sb.AppendLine("verify " + B(s.VerifyCopies) + " | policy " + s.Policy + " | limit " + G(s.SpeedLimitMbps) + " | value " + G(s.LimitValueMbps) + " | close " + B(s.CloseWhenDone)
                + " after " + s.CloseAfterSeconds + " | takeover " + B(s.ExplorerTakeover) + " | unit " + (s.SpeedUnit ?? "(null)") + " | backup " + (s.UltracopierRunBackup ?? "(null)")
                + " | extended " + B(s.Extended) + " | runs " + s.BenchRunsQuick + " " + s.BenchRunsThorough);
            sb.AppendLine("settings.ini");
            string ini = Path.Combine(AppPaths.DataDir, "settings.ini");
            if (File.Exists(ini)) sb.Append(File.ReadAllText(ini));
            sb.AppendLine("journal");
            var spec = new JobSpec { Id = "plandump", Op = OpKind.Move, Dest = Paths.ToLong(@"X:\Dest folder\"), Verify = true, Policy = ConflictPolicy.KeepBoth, SpeedLimitMbps = 57 * 8 * 1.048576, Origin = "Right-click" };
            spec.Sources.Add(Paths.ToLong(@"C:\a\b.txt"));
            spec.Sources.Add(Paths.ToLong("\\\\srv\\share\\\u00e9 \u00fc"));
            spec.ExplicitItems = new List<FileItem> { new FileItem { Src = @"\\?\C:\a\c.bin", Rel = @"sub\c.bin" } };
            spec.ExplicitDirs = new List<DirItem> { new DirItem { Src = @"\\?\C:\a\sub", Rel = @"x\sub" } };
            Directory.CreateDirectory(AppPaths.JobDir);
            string f = Path.Combine(AppPaths.JobDir, "plandump.job");
            Journal.Write(spec, f, new[] { "extra=1" });
            sb.Append(File.ReadAllText(f, Encoding.UTF8));
            var r = Journal.Read(f);
            File.Delete(f);
            sb.AppendLine("read " + r.Id + " | " + r.Op + " | " + r.Dest + " | verify " + B(r.Verify) + " | " + r.Policy + " | limit " + G(r.SpeedLimitMbps) + " | " + r.Origin
                + " | " + string.Join(" ; ", r.Sources) + " | " + string.Join(" ; ", (r.ExplicitItems ?? new List<FileItem>()).Select(i => i.Src + " > " + i.Rel))
                + " | " + string.Join(" ; ", (r.ExplicitDirs ?? new List<DirItem>()).Select(d => d.Src + " > " + d.Rel + " @" + d.Depth)) + " | " + r.Title);
            // titles
            var t = new JobSpec { Op = OpKind.Copy };
            t.Sources.Add(Paths.ToLong(@"C:\x\photo.jpg"));
            sb.AppendLine(t.Title);
            t.Sources.Add(Paths.ToLong(@"C:\x\other"));
            sb.AppendLine(t.Title);
            t.Op = OpKind.Move;
            t.ExplicitItems = new List<FileItem> { new FileItem() };
            sb.AppendLine(t.Title);
            t.ExplicitItems = new List<FileItem> { new FileItem(), new FileItem(), new FileItem() };
            t.Elevated = true;
            sb.AppendLine(t.Title);
            // records: an administrator copy has none; one still being written is removed once it is written;
            // unfinished ones come back to resume, one with nothing to do is deleted
            Func<string, string> has = id => File.Exists(Path.Combine(AppPaths.JobDir, id + ".job")) ? "1" : "0";
            var el = new JobSpec { Id = "plandump-admin", Op = OpKind.Copy, Dest = Paths.ToLong(@"C:\x"), Elevated = true };
            el.Sources.Add(Paths.ToLong(@"C:\y"));
            Journal.Save(el);
            Journal.SaveSoon(el);
            var soon = new JobSpec { Id = "plandump-soon", Op = OpKind.Copy, Dest = Paths.ToLong(@"C:\x") };
            soon.Sources.Add(Paths.ToLong(@"C:\y"));
            Journal.SaveSoon(soon);
            Journal.Remove(soon);
            System.Threading.Thread.Sleep(1500);
            var kept = new JobSpec { Id = "plandump-kept", Op = OpKind.Move, Dest = Paths.ToLong(@"C:\x"), Policy = ConflictPolicy.Replace };
            kept.Sources.Add(Paths.ToLong(@"C:\y"));
            Journal.Save(kept);
            File.WriteAllText(Path.Combine(AppPaths.JobDir, "plandump-empty.job"), "op=Copy\r\n");
            File.WriteAllText(Path.Combine(AppPaths.JobDir, "plandump-x.jobx"), "op=Copy\r\ndest=C:\\x\r\nsrc=C:\\y\r\n");
            sb.AppendLine("records admin " + has("plandump-admin") + " soon " + has("plandump-soon") + " kept " + has("plandump-kept"));
            foreach (var u in Journal.LoadUnfinished())
                sb.AppendLine("unfinished " + u.Id + " | " + u.Op + " | " + u.Policy + " | resume " + B(u.ResumeMode) + " | " + u.Dest + " | " + string.Join(" ; ", u.Sources));
            sb.AppendLine("after load empty " + has("plandump-empty"));
            foreach (var j in Directory.GetFiles(AppPaths.JobDir)) File.Delete(j);
            // local times of file times (the conflict window shows them)
            foreach (long ft in new[] { 0L, 132000000000000000L, 132700000000000000L, 133500000000000000L, 134400243957103472L, -1L, 2650467743999999999L, 2650467744000000000L })
            {
                var lt = FileTimes.ToLocal(ft);
                sb.AppendLine("time " + ft + " " + (lt == DateTime.MinValue ? "min" : lt.ToString("yyyy-MM-dd HH:mm:ss.fff", CultureInfo.InvariantCulture)));
            }
            for (int sp = 0; sp <= 4; sp++) sb.AppendLine("usb " + sp + " " + UsbLink.Name(sp) + " | " + UsbLink.Short(sp) + " | " + G(UsbLink.CeilingMBps(sp)) + " | " + UsbLink.Aka(sp));
        }

        static void DumpScan(string[] args, StringBuilder sb)
        {
            var op = args.Length > 3 && args[3].Equals("move", StringComparison.OrdinalIgnoreCase) ? OpKind.Move : OpKind.Copy;
            var spec = new JobSpec { Op = op, Dest = Paths.ToLong(args[4]) };
            for (int i = 5; i < args.Length; i++) spec.Sources.Add(Paths.ToLong(args[i]));
            var srcVol = spec.Sources.Count > 0 ? VolumeProbe.Probe(spec.Sources[0]) : null;
            var dstVol = VolumeProbe.Probe(spec.Dest);
            sb.AppendLine("scan " + op.ToString().ToLowerInvariant());
            sb.AppendLine("src " + (srcVol != null ? VolText(srcVol) : "(null)"));
            sb.AppendLine("dst " + VolText(dstVol));
            // as CopyJob.RunCopy: 8 listings at once on a network, else 1 (source) and 2 (destination); then 4 and 4
            int st = srcVol != null && srcVol.IsNetwork ? 8 : 1, dt = dstVol.IsNetwork ? 8 : 2;
            foreach (var th in new[] { new[] { st, dt }, new[] { 4, 4 } })
            {
                var scanner = new Scanner(() => false);
                var res = scanner.Run(spec, th[0], th[1]);
                sb.AppendLine("threads " + th[0] + " " + th[1]);
                sb.AppendLine("problems " + res.Problems.Count);
                foreach (var p in res.Problems) sb.AppendLine(p);
                sb.AppendLine("files " + res.Files.Count);
                foreach (var f in res.Files)
                    sb.AppendLine(f.Src + " | " + f.Rel + " | " + f.Size + " | " + f.Attr.ToString("X") + " | c " + f.CTime + " a " + f.ATime + " m " + f.MTime + " | dest " + B(f.DestExists)
                        + " dir " + B(f.DestIsDir) + " " + f.DestSize + " " + f.DestMTime + " | temp " + B(f.TempExists));
                sb.AppendLine("dirs " + res.Dirs.Count);
                foreach (var d in res.Dirs)
                    sb.AppendLine(d.Src + " | " + d.Rel + " | " + d.Attr.ToString("X") + " | c " + d.CTime + " a " + d.ATime + " m " + d.MTime + " | existed " + B(d.DestExisted)
                        + " isfile " + B(d.DestIsFile) + " | depth " + d.Depth);
                sb.AppendLine("total " + res.TotalBytes + " conflicts " + res.Conflicts + " median " + res.MedianSize);
                sb.AppendLine("largest " + string.Join(", ", res.Largest.Select(l => l.Key + "=" + l.Value)));
                sb.AppendLine("seen " + scanner.FilesSeen + " " + scanner.BytesSeen + " | " + scanner.Stage);
                sb.AppendLine("plan " + PlanText(Plan.Choose(new JobSpec { Op = OpKind.Copy }, srcVol, dstVol, res)));
            }
        }

        /// <summary>Read-only check: every open Explorer window and the folder a paste would go to.</summary>
        static int Where(string outFile)
        {
            var sb = new StringBuilder();
            IntPtr h = IntPtr.Zero;
            int n = 0;
            while ((h = FindWindowEx(IntPtr.Zero, h, "CabinetWClass", null)) != IntPtr.Zero)
            {
                n++;
                bool isVirtual;
                var sw = Stopwatch.StartNew();
                string dest = Explorer.ShellLocation.FolderOf(h, out isVirtual);
                sb.AppendLine("window " + h.ToInt64().ToString("X") + " -> " + (dest ?? (isVirtual ? "(not a folder - Explorer keeps that paste)" : "(Explorer didn't answer)")) + "  [" + sw.ElapsedMilliseconds + " ms]");
            }
            IntPtr desk = FindWindowEx(IntPtr.Zero, IntPtr.Zero, "Progman", null);
            sb.AppendLine("desktop -> " + Explorer.ShellLocation.FolderOf(desk));
            sb.AppendLine(n + " Explorer windows");
            if (outFile != null) File.WriteAllText(outFile, sb.ToString(), Encoding.UTF8);
            return 0;
        }

        /// <summary>The pipeline against real files in %TEMP%: odd sizes, an extra stream, dates, and both ways of hashing.</summary>
        static void PipelineChecks(Action<string, bool> check)
        {
            string dir = Path.Combine(Path.GetTempPath(), "choppy-selftest-" + Guid.NewGuid().ToString("N").Substring(0, 8));
            Directory.CreateDirectory(dir);
            try
            {
                var tune = new PipeTuning { Chunk = 1 << 20, Slots = 3, Align = 4096 };
                foreach (int size in new[] { 0, 1, 4096, 2 << 20, (3 << 20) + 12345 })
                {
                    string a = Path.Combine(dir, "a" + size + ".bin"), b = Path.Combine(dir, "b" + size + ".bin");
                    var data = new byte[size];
                    new Random(size).NextBytes(data);
                    File.WriteAllBytes(a, data);
                    File.SetCreationTimeUtc(a, new DateTime(2020, 1, 2, 3, 4, 5, DateTimeKind.Utc));
                    File.SetLastWriteTimeUtc(a, new DateTime(2021, 6, 7, 8, 9, 10, DateTimeKind.Utc));
                    var fa = new FileInfo(a);
                    var r = BigCopy.Copy(Paths.ToLong(a), Paths.ToLong(b), fa.CreationTimeUtc.ToFileTimeUtc(), fa.LastWriteTimeUtc.ToFileTimeUtc(),
                        (uint)fa.Attributes, tune, true, true, null, n => true);
                    bool same = r.Error == 0 && File.Exists(b) && System.Linq.Enumerable.SequenceEqual(File.ReadAllBytes(b), data);
                    check("pipeline copy " + size + " B", same && r.Hash == XxHash64.OfBytes(data));
                    var fb = new FileInfo(b);
                    check("pipeline dates " + size + " B", fb.Exists && fb.CreationTimeUtc == fa.CreationTimeUtc && fb.LastWriteTimeUtc == fa.LastWriteTimeUtc);
                    ulong h; int e;
                    check("hash unbuffered " + size + " B", XxHash64.HashFile(Paths.ToLong(b), true, null, out h, out e) && h == XxHash64.OfBytes(data));
                    check("hash buffered " + size + " B", XxHash64.HashFile(Paths.ToLong(a), false, null, out h, out e) && h == XxHash64.OfBytes(data));
                }
                // pieces joined into bigger writes: same bytes, at every odd size
                foreach (int size in new[] { (1 << 20) * 3, (5 << 20) + 777, (7 << 20) + 4096, 123457 })
                {
                    string a = Path.Combine(dir, "join-a" + size + ".bin"), b = Path.Combine(dir, "join-b" + size + ".bin");
                    var data = new byte[size];
                    new Random(size + 1).NextBytes(data);
                    File.WriteAllBytes(a, data);
                    var jt = new PipeTuning { Chunk = 1 << 20, Slots = 4, Align = 4096, WriteBatch = 3 };
                    var rj = BigCopy.Copy(Paths.ToLong(a), Paths.ToLong(b), 0, File.GetLastWriteTimeUtc(a).ToFileTimeUtc(), 0x20, jt, true, false, null, n => true);
                    check("pipeline joined writes " + size + " B", rj.Error == 0 && rj.Hash == XxHash64.OfBytes(data) && System.Linq.Enumerable.SequenceEqual(File.ReadAllBytes(b), data));
                    // one disk: read a batch, then write it
                    string c = Path.Combine(dir, "phase-" + size + ".bin");
                    var pt = new PipeTuning { Chunk = 1 << 20, Slots = 3, Align = 4096, Phased = true };
                    var rp = BigCopy.Copy(Paths.ToLong(a), Paths.ToLong(c), 0, File.GetLastWriteTimeUtc(a).ToFileTimeUtc(), 0x20, pt, true, false, null, 2 << 20, n => true);
                    check("pipeline phased " + size + " B", rp.Error == 0 && rp.Hash == XxHash64.OfBytes(data) && System.Linq.Enumerable.SequenceEqual(File.ReadAllBytes(c), data));
                    // one hard disk: small reads, a few at the disk at once, the batch written in bigger steps, room reserved
                    string ph = Path.Combine(dir, "phase2-" + size + ".bin");
                    var pt2 = new PipeTuning { Chunk = 1 << 20, Slots = 6, Align = 4096, Phased = true, ReadDepth = 2, WriteStep = 3 << 20, Prealloc = true };
                    var rp2 = BigCopy.Copy(Paths.ToLong(a), Paths.ToLong(ph), 0, File.GetLastWriteTimeUtc(a).ToFileTimeUtc(), 0x20, pt2, true, false, null, n => true);
                    check("pipeline phased, read depth + write step " + size + " B", rp2.Error == 0 && rp2.Hash == XxHash64.OfBytes(data) && System.Linq.Enumerable.SequenceEqual(File.ReadAllBytes(ph), data));
                    // nothing reserved up front (hard disk destinations): the file grows as it is written
                    string g = Path.Combine(dir, "grow-" + size + ".bin");
                    var gt = new PipeTuning { Chunk = 1 << 20, Slots = 4, Align = 4096, WriteBatch = 3, Prealloc = false };
                    var rg = BigCopy.Copy(Paths.ToLong(a), Paths.ToLong(g), 0, File.GetLastWriteTimeUtc(a).ToFileTimeUtc(), 0x20, gt, true, false, null, n => true);
                    check("pipeline no reservation " + size + " B", rg.Error == 0 && rg.Hash == XxHash64.OfBytes(data) && System.Linq.Enumerable.SequenceEqual(File.ReadAllBytes(g), data));
                }
                // the end through the Windows cache: same bytes, same size
                {
                    string a = Path.Combine(dir, "tail-a.bin"), b = Path.Combine(dir, "tail-b.bin");
                    var data = new byte[(5 << 20) + 777];
                    new Random(9).NextBytes(data);
                    File.WriteAllBytes(a, data);
                    var rt = BigCopy.Copy(Paths.ToLong(a), Paths.ToLong(b), 0, File.GetLastWriteTimeUtc(a).ToFileTimeUtc(), 0x20, tune, false, false, null, 2 << 20, n => true);
                    check("pipeline cached tail", rt.Error == 0 && System.Linq.Enumerable.SequenceEqual(File.ReadAllBytes(b), data));
                    // two hard disks: the cached end starts once the source is fully read (whatever the buffers still hold)
                    string c = Path.Combine(dir, "tail-c.bin");
                    var at = new PipeTuning { Chunk = 1 << 20, Slots = 8, Align = 4096, WriteBatch = 2, Prealloc = false, TailAfterRead = true };
                    var ra = BigCopy.Copy(Paths.ToLong(a), Paths.ToLong(c), 0, File.GetLastWriteTimeUtc(a).ToFileTimeUtc(), 0x20, at, true, false, null, 64 << 20, n => true);
                    check("pipeline cached end after read", ra.Error == 0 && ra.Hash == XxHash64.OfBytes(data) && System.Linq.Enumerable.SequenceEqual(File.ReadAllBytes(c), data));
                }
                // an extra data stream (downloads carry Zone.Identifier) comes along
                string s = Path.Combine(dir, "s.bin"), t = Path.Combine(dir, "t.bin");
                File.WriteAllBytes(s, new byte[5000]);
                const string zone = "[ZoneTransfer]\r\nZoneId=3\r\n";
                WriteStream(Paths.ToLong(s) + ":Zone.Identifier", zone);
                var rs = BigCopy.Copy(Paths.ToLong(s), Paths.ToLong(t), 0, File.GetLastWriteTimeUtc(s).ToFileTimeUtc(), 0x20, tune, false, false, null, n => true);
                check("pipeline stream" + (rs.Error != 0 ? " (error " + rs.Error + ")" : ""), rs.Error == 0 && ReadStream(Paths.ToLong(t) + ":Zone.Identifier") == zone);
                // never replaces anything
                var rx = BigCopy.Copy(Paths.ToLong(s), Paths.ToLong(t), 0, 0, 0x20, tune, false, false, null, n => true);
                check("pipeline never replaces", rx.Error == Native.ERROR_FILE_EXISTS);
                // stopped half way: says so
                File.WriteAllBytes(s, new byte[4 << 20]);
                var rc = BigCopy.Copy(Paths.ToLong(s), Paths.ToLong(Path.Combine(dir, "c.bin")), 0, 0, 0x20, tune, false, false, null, n => n < (2 << 20));
                check("pipeline stop", rc.Error == Native.ERROR_REQUEST_ABORTED);
            }
            catch (Exception ex) { check("pipeline: " + ex.Message, false); }
            finally { try { Directory.Delete(dir, true); } catch { } }
        }

        /// <summary>The benchmark's parts that need no drives: verdicts, the plan, sizes, the report, clean-up, Windows' copy on a small file.</summary>
        static void BenchChecks(Action<string, bool> check, StringBuilder sb, bool desktop)
        {
            // the time decides: the share of Windows' time Choppy saved; less than 4% apart is a Marginal Tie, nothing else is
            double saved;
            check("bench: same times = Marginal Tie", Bench.Benchmark.Judge(new[] { 2.0, 2.0 }, new[] { 2.0, 2.0 }, out saved) == 0 && Math.Abs(saved) < 1e-9);
            check("bench: less time = Faster (+17%)", Bench.Benchmark.Judge(new[] { 1.0, 1.02 }, new[] { 1.2, 1.22 }, out saved) == 1 && Math.Abs(saved - 0.2 / 1.21) < 1e-9 && Bench.BenchText.Saved(saved) == "+17%");
            check("bench: more time = Slower (-20%)", Bench.Benchmark.Judge(new[] { 1.2, 1.22 }, new[] { 1.0, 1.02 }, out saved) == -1 && Bench.BenchText.Saved(saved) == "−20%");
            check("bench: 3% apart = Marginal Tie", Bench.Benchmark.Judge(new[] { 1.0, 1.0 }, new[] { 1.03, 1.03 }, out saved) == 0 && saved > 0);
            check("bench: 9% behind with scattered runs is still Slower", Bench.Benchmark.Judge(new[] { 1.0, 1.4 }, new[] { 1.1, 1.1 }, out saved) == -1);
            check("bench: averages", Bench.Benchmark.Mean(new[] { 3.0, 1.0, 2.0 }) == 2.0 && Bench.Benchmark.Mean(new[] { 1.0, 2.0 }) == 1.5);
            check("bench: seconds", Bench.BenchText.Secs(0.4) == "0.40 s" && Bench.BenchText.Secs(12.34) == "12.3 s" && Bench.BenchText.Saved(0.001) == "0%");

            // the plan: system drive to each drive and back, and within each
            Func<string, bool, DeviceKind, int, double, double, Bench.BenchDrive> drive = (l, sys, k, disk, rd, wr) => new Bench.BenchDrive
            {
                Letter = l, Root = l + "\\", System = sys, Measured = true, ReadMBps = rd, WriteMBps = wr, FileSystem = "NTFS",
                Vol = new VolumeInfo { Id = "vol:" + l, Kind = k, Disk = disk, FileSystem = "NTFS" }, Bus = k == DeviceKind.Ssd ? "NVMe" : "SATA"
            };
            var c = drive("C:", true, DeviceKind.Ssd, 0, 2000, 1500);
            var d = drive("D:", false, DeviceKind.Hdd, 1, 180, 180);
            var e = drive("E:", false, DeviceKind.Ssd, 2, 400, 350);
            var routes = Bench.Benchmark.PlanRoutes(new List<Bench.BenchDrive> { d, c, e });
            Func<Bench.BenchDrive, Bench.BenchDrive, bool> has = (a, b2) => routes.Exists(r => r.From == a && r.To == b2);
            check("bench: routes (7 for 3 drives, the system drive first, no route between two others)",
                routes.Count == 7 && routes[0].From == c && routes[0].To == c && has(c, d) && has(d, c) && has(d, d) && has(c, e) && has(e, c) && has(e, e) && !has(d, e));
            var bq = new Bench.Benchmark();
            bq.SizeRoutes(routes);
            Func<Bench.BenchDrive, Bench.BenchDrive, Bench.Benchmark.Route> rt = (a, b2) => routes.Find(r => r.From == a && r.To == b2);
            check("bench: sizes by speed (2 GB + 1,000 small files at 350 MB/s and more, 1 GB + 500 below)",
                rt(c, c).Big == 2048 * Bench.Benchmark.MB && rt(c, c).Small == 1000 && rt(c, d).Big == 1024 * Bench.Benchmark.MB && rt(c, d).Small == 500
                && rt(d, d).Big == 1024 * Bench.Benchmark.MB && rt(c, e).Big == 2048 * Bench.Benchmark.MB && rt(e, c).Big == 2048 * Bench.Benchmark.MB && rt(e, e).Big == 1024 * Bench.Benchmark.MB);
            check("bench: two cases per route", routes.TrueForAll(r => r.Cases.Count == 2 && r.Cases[0].Files == 1 && r.Cases[1].Files == r.Small && r.Est > 0));
            // many hard disks: one step smaller everywhere, to keep to the time budget
            var many = new List<Bench.BenchDrive> { c };
            for (int i = 0; i < 9; i++) many.Add(drive(((char)('F' + i)).ToString() + ":", false, DeviceKind.Hdd, 3 + i, 150, 150));
            var mr = Bench.Benchmark.PlanRoutes(many);
            bq.SizeRoutes(mr);
            check("bench: a time budget (many hard disks: a step smaller)", mr.Count == 28 && mr[0].Big == 1024 * Bench.Benchmark.MB && mr.TrueForAll(r => r.Big <= 1024 * Bench.Benchmark.MB) && mr.Exists(r => r.Big == 512 * Bench.Benchmark.MB && r.Small == 250));
            var thorough = new Bench.Benchmark { Thorough = true };
            var tr = Bench.Benchmark.PlanRoutes(new List<Bench.BenchDrive> { c, d });
            thorough.SizeRoutes(tr);
            check("bench: thorough = 4 GB / 2 GB, 2,000 / 1,000 small files, a mixed folder",
                tr.TrueForAll(r => r.Cases.Count == 3) && tr[0].Big == 4096 * Bench.Benchmark.MB && tr[0].Small == 2000 && tr[1].Big == 2048 * Bench.Benchmark.MB && tr[1].Small == 1000);
            var s1 = Bench.BenchFiles.SmallSizes(1000);
            long sum = 0; foreach (var x in s1) sum += x;
            check("bench: small files 4 KB - 1 MB, the same set every time", Array.TrueForAll(s1, x => x >= 4096 && x <= 1 << 20) && sum == Bench.BenchFiles.SmallSizes(1000).Sum()
                && sum > 150 * Bench.Benchmark.MB && sum < 250 * Bench.Benchmark.MB);

            // the report: summary, JSON, HTML (escaped), text
            var rep = new Bench.BenchReport { Started = new DateTime(2026, 11, 23, 14, 5, 0), Mode = "quick", Rounds = 2, Cpu = "Test CPU", Windows = "Windows test", Threads = 4, RamGB = 8 };
            c.Label = "<b>&amp;"; c.Model = "Test \"NVMe\"";
            rep.Drives = new List<Bench.BenchDrive> { c, d, new Bench.BenchDrive { Letter = "G:", Skip = "not a disk of this PC (a virtual or cloud drive)" } };
            var big = new Bench.BenchCase { Key = "big", Big = new[] { 1024 * Bench.Benchmark.MB } };
            var r1 = new Bench.BenchResult { From = c, To = d, Case = big };
            r1.Runs.Add(new Bench.BenchRun { Choppy = true, Done = 5.0, OnDisk = 5.1 }); r1.Runs.Add(new Bench.BenchRun { Choppy = false, Done = 4.0, OnDisk = 6.0 });
            r1.Runs.Add(new Bench.BenchRun { Choppy = false, Done = 4.1, OnDisk = 6.1 }); r1.Runs.Add(new Bench.BenchRun { Choppy = true, Done = 5.0, OnDisk = 5.2 });
            r1.Score();
            var r2 = new Bench.BenchResult { From = d, To = c, Case = new Bench.BenchCase { Key = "small", Smalls = s1 }, Error = "test error" };
            r2.Score();
            rep.Results.Add(r1); rep.Results.Add(r2);
            check("bench: scored (on the disk faster, done slower; averages)", r1.Scored && r1.Verdict == 1 && r1.DoneVerdict == -1 && !r2.Scored && rep.Summary == "Choppy is Faster 1 of 1"
                && Math.Abs(r1.ChoppySecs - 5.15) < 1e-9 && Math.Abs(r1.WindowsSecs - 6.05) < 1e-9);
            check("bench: summary line", Bench.BenchReport.SummaryOf(9, 2, 1, true) == "Choppy is Faster 9 of 12 · Marginal Tie 2 of 12 · Slower 1 of 12");
            check("bench: each case says Tie, the summary Marginal Tie", Bench.BenchReport.Word(0) == "Tie" && Bench.BenchReport.Word(1) == "Faster" && Bench.BenchReport.Word(-1) == "Slower");
            check("bench: Never Slower only when a whole run ended without one",
                Bench.BenchReport.SummaryOf(10, 9, 0, true) == "Choppy is Faster 10 of 19 · Marginal Tie 9 of 19 · Never Slower"
                && Bench.BenchReport.SummaryOf(10, 9, 0, false) == "Choppy is Faster 10 of 19 · Marginal Tie 9 of 19"
                && Bench.BenchReport.SummaryOf(5, 0, 0, true) == "Choppy is Faster 5 of 5");
            var tot = rep.Totals;
            check("bench: totals (the measured cases only; each one's speed over the times added up)", tot != null && tot.Cases == 1 && Math.Abs(tot.AvgSaved - r1.Saved) < 1e-9
                && Math.Abs(tot.ChoppySecs - 5.15) < 1e-9 && Math.Abs(tot.SecsSaved - 0.9) < 1e-9 && Math.Abs(tot.ChoppyMBps - 1024 / 5.15) < 1e-6
                && tot.Line == "+15% faster on average · Choppy 199 MB/s · Windows 169 MB/s · Time saved 0.90 s" && Bench.BenchTotals.Of(new[] { r2 }) == null);
            check("bench: time saved as a span", Bench.BenchText.SignedSpan(-12.34) == "−12.3 s" && Bench.BenchText.SignedSpan(0.001) == "0 s"
                && Bench.BenchText.SignedSpan(75) == "1 min 15 s" && Bench.BenchText.Span(0.41) == "0.41 s");
            string json = rep.ToJson(), html = rep.ToHtml("x.json");
            int depth = 0; bool inStr = false, esc = false, balanced = true;
            foreach (char ch in json)
            {
                if (inStr) { if (esc) esc = false; else if (ch == '\\') esc = true; else if (ch == '"') inStr = false; continue; }
                if (ch == '"') inStr = true; else if (ch == '{' || ch == '[') depth++; else if (ch == '}' || ch == ']') { depth--; if (depth < 0) balanced = false; }
            }
            check("bench: JSON well formed", balanced && depth == 0 && !inStr && json.Contains("\"verdict\": \"faster\"") && json.Contains("\\\"NVMe\\\"") && json.Contains("\"error\": \"test error\"") && json.Contains("\"totals\": { \"cases\": 1,"));
            check("bench: HTML escaped", html.Contains("&lt;b&gt;&amp;amp;") && !html.Contains("<b>&amp;</b>") && html.Contains("Faster 1 of 1") && html.Contains("Choppy VS Windows")
                && html.Contains("not a disk of this PC") && !html.Contains(">done ") && html.Contains("<section class=\"totals\">"));

            // test folders: removed with what's in them; a link inside is removed, never followed; a folder that isn't ours stays
            string root = Path.Combine(Path.GetTempPath(), "choppy-selftest-bench-" + Guid.NewGuid().ToString("N").Substring(0, 8));
            Directory.CreateDirectory(root);
            try
            {
                string outside = Path.Combine(root, "outside");
                Directory.CreateDirectory(outside);
                File.WriteAllText(Path.Combine(outside, "keep.txt"), "keep");
                string why;
                string t1 = Bench.BenchFiles.MakeFolder(root, false, out why);
                Directory.CreateDirectory(Path.Combine(t1, "a", "b"));
                File.WriteAllText(Path.Combine(t1, "a", "b", "x.bin"), "x");
                File.SetAttributes(Path.Combine(t1, "a", "b", "x.bin"), FileAttributes.ReadOnly);
                bool link = MakeJunction(Path.Combine(t1, "link"), outside);
                // a folder of that name that isn't the benchmark's: never used, a numbered one instead
                string foreign = Path.Combine(root, "x");
                Directory.CreateDirectory(foreign);
                string t2 = Bench.BenchFiles.MakeFolder(foreign, false, out why);
                Directory.Delete(t2, true);
                Directory.CreateDirectory(t2);
                File.WriteAllText(Path.Combine(t2, "theirs.txt"), "theirs");
                string t3 = Bench.BenchFiles.MakeFolder(foreign, false, out why);
                check("bench: a folder that isn't ours gets a numbered name", t1 != null && t3 != null && t3.EndsWith(Bench.BenchFiles.FolderName + " 2"));
                Bench.BenchFiles.CleanLeftovers();
                check("bench: leftovers removed, links not followed" + (link ? "" : " (no link made)"),
                    !Directory.Exists(t1) && !Directory.Exists(t3) && File.Exists(Path.Combine(outside, "keep.txt")));
                check("bench: a folder without the marker untouched", File.Exists(Path.Combine(t2, "theirs.txt")));
            }
            catch (Exception ex) { check("bench: test folders: " + ex.Message, false); }
            finally { try { Directory.Delete(root, true); } catch { } }

            // Windows' own copy (the shell's engine, in a child process) of a small file and a folder
            string sdir = Path.Combine(Path.GetTempPath(), "choppy-selftest-shell-" + Guid.NewGuid().ToString("N").Substring(0, 8));
            try
            {
                Directory.CreateDirectory(Path.Combine(sdir, "src", "sub"));
                Directory.CreateDirectory(Path.Combine(sdir, "dst"));
                var data = new byte[300000];
                new Random(3).NextBytes(data);
                File.WriteAllBytes(Path.Combine(sdir, "src", "sub", "f.bin"), data);
                File.WriteAllBytes(Path.Combine(sdir, "w.bin"), new byte[100]);
                Directory.CreateDirectory(Path.Combine(sdir, "warm"));
                double secs = Bench.ShellCopy.Run(Path.Combine(sdir, "w.bin"), Path.Combine(sdir, "warm"), Path.Combine(sdir, "src"), Path.Combine(sdir, "dst"), () => false);
                string got = Path.Combine(sdir, "dst", "src", "sub", "f.bin");
                check("bench: Windows' copy (IFileOperation, child process)", secs >= 0 && File.Exists(got) && System.Linq.Enumerable.SequenceEqual(File.ReadAllBytes(got), data)
                    && File.Exists(Path.Combine(sdir, "warm", "w.bin")));
            }
            catch (Exception ex)
            {
                // a build server without a desktop may have no shell to copy with (like the Ctrl+V hook above)
                if (!desktop || ex.Message.Contains("80040154") || ex.Message.IndexOf("not registered", StringComparison.OrdinalIgnoreCase) >= 0) sb.AppendLine("SKIP Windows' copy (no shell here: " + ex.Message + ")");
                else check("bench: Windows' copy: " + ex.Message, false);
            }
            finally { try { Directory.Delete(sdir, true); } catch { } }
        }

        /// <summary>A junction (no administrator rights needed), for the clean-up check.</summary>
        static bool MakeJunction(string link, string target)
        {
            try
            {
                var psi = new ProcessStartInfo("cmd.exe", "/c mklink /J \"" + link + "\" \"" + target + "\"") { UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true };
                using (var p = Process.Start(psi)) { p.StandardOutput.ReadToEnd(); p.WaitForExit(10000); }
                return Directory.Exists(link) && (File.GetAttributes(link) & FileAttributes.ReparsePoint) != 0;
            }
            catch { return false; }
        }

        static void WriteStream(string path, string text)
        {
            using (var h = Native.CreateFileW(path, Native.GENERIC_WRITE, 0, IntPtr.Zero, 2, 0, IntPtr.Zero))
            using (var fs = new FileStream(h, FileAccess.Write)) { var b = Encoding.ASCII.GetBytes(text); fs.Write(b, 0, b.Length); }
        }

        static string ReadStream(string path)
        {
            using (var h = Native.CreateFileW(path, Native.GENERIC_READ, Native.FILE_SHARE_READ, IntPtr.Zero, Native.OPEN_EXISTING, 0, IntPtr.Zero))
            {
                if (h.IsInvalid) return null;
                using (var fs = new FileStream(h, FileAccess.Read)) using (var r = new StreamReader(fs, Encoding.ASCII)) return r.ReadToEnd();
            }
        }

        static int SelfTest(string outFile)
        {
            var sb = new StringBuilder();
            int fails = 0;
            Action<string, bool> check = (name, ok) => { sb.AppendLine((ok ? "PASS " : "FAIL ") + name); if (!ok) fails++; };
            // xxHash64 reference values (seed 0)
            check("xxh64 empty", XxHash64.OfBytes(new byte[0]) == 0xEF46DB3751D8E999UL);
            check("xxh64 'a'", XxHash64.OfBytes(Encoding.ASCII.GetBytes("a")) == 0xD24EC4F1A98C6E5BUL);
            check("xxh64 'abc'", XxHash64.OfBytes(Encoding.ASCII.GetBytes("abc")) == 0x44BC2CF5AD770999UL);
            string fox = "The quick brown fox jumps over the lazy dog";
            check("xxh64 fox", XxHash64.OfBytes(Encoding.ASCII.GetBytes(fox)) == 0x0B242D361FDA71BCUL);
            // streaming in odd pieces gives the same digest
            var big = new byte[100003];
            new Random(7).NextBytes(big);
            ulong whole = XxHash64.OfBytes(big);
            var x = new XxHash64();
            unsafe
            {
                fixed (byte* p = big)
                {
                    int pos = 0; int[] steps = { 1, 3, 31, 32, 33, 64, 999, 4096, 7 };
                    int k = 0;
                    while (pos < big.Length) { int n = Math.Min(steps[k++ % steps.Length], big.Length - pos); x.Update(p + pos, n); pos += n; }
                }
            }
            check("xxh64 streaming", x.Digest() == whole);
            // paths
            check("long local", Paths.ToLong(@"C:\a\b\") == @"\\?\C:\a\b");
            check("long unc", Paths.ToLong(@"\\srv\share\x") == @"\\?\UNC\srv\share\x");
            check("short unc", Paths.ToShort(@"\\?\UNC\srv\share\x") == @"\\srv\share\x");
            check("inside", Paths.IsSameOrInside(@"\\?\C:\a\b\c", @"\\?\C:\a\b"));
            check("not inside", !Paths.IsSameOrInside(@"\\?\C:\a\bc", @"\\?\C:\a\b"));
            check("parent of root", Paths.Parent(@"C:\") == null && Paths.Parent(@"\\?\C:\") == null);
            check("drive root from Explorer (C:\")", Paths.ToLong("C:\"") == @"\\?\C:\" && Paths.ToLong(@"\\?\C:""") == @"\\?\C:\");
            check("folder arg with quote", Paths.ToLong("D:\\Games\"") == @"\\?\D:\Games");
            check("parent", Paths.Parent(@"\\?\C:\x") == @"\\?\C:\" && Paths.Parent(@"\\?\C:\x\y\") == @"\\?\C:\x");
            check("struct size", System.Runtime.InteropServices.Marshal.SizeOf(typeof(Native.WIN32_FIND_DATAW)) == 592
                && System.Runtime.InteropServices.Marshal.SizeOf(typeof(Native.WIN32_FILE_ATTRIBUTE_DATA)) == 36);
            check("numbered", Paths.NumberedName("photo.jpg", 2, false) == "photo (2).jpg");
            check("numbered noext", Paths.NumberedName("README", 3, false) == "README (3)");
            check("temp", CopyJob.TempName(@"\\?\C:\x\a.txt") == @"\\?\C:\x\a.txt.chpart");
            string longName = new string('n', 252) + ".bin";
            check("temp long", CopyJob.TempName(@"\\?\C:\x\" + longName).EndsWith(".chpart") && Paths.FileName(CopyJob.TempName(@"\\?\C:\x\" + longName)).Length < 255);
            PipelineChecks(check);
            // the way per drive: the same ReFS volume goes to Windows' own copy (it clones the blocks); FAT32 caps a file at 4 GB
            var refs = new VolumeInfo { Id = "vol:1", Kind = DeviceKind.Ssd, FileSystem = "ReFS", FsFlags = Native.FILE_SUPPORTS_BLOCK_REFCOUNTING };
            var ntfs = new VolumeInfo { Id = "vol:2", Kind = DeviceKind.Ssd, FileSystem = "NTFS" };
            var pr = Plan.Choose(new JobSpec { Op = OpKind.Copy }, refs, refs, null);
            check("same ReFS drive: Windows' own copy (block clone)", pr.Clone && !pr.Pipeline && !pr.InstantMove);
            check("ReFS to another drive: the pipeline", Plan.Choose(new JobSpec { Op = OpKind.Copy }, refs, ntfs, null).Pipeline);
            check("same ReFS drive, a move: renamed in place", Plan.Choose(new JobSpec { Op = OpKind.Move }, refs, refs, null).InstantMove);
            // only a disk device's number says which disk a volume is on (a cloud drive answers as a file system, "number 0")
            check("disk number: a disk counts, a file system device doesn't",
                VolumeProbe.DiskNumber(new Native.STORAGE_DEVICE_NUMBER { DeviceType = VolumeProbe.FILE_DEVICE_DISK, DeviceNumber = 2 }) == 2
                && VolumeProbe.DiskNumber(new Native.STORAGE_DEVICE_NUMBER { DeviceType = 8, DeviceNumber = 0 }) == -1
                && !VolumeProbe.SameDisk(new VolumeInfo { Id = "vol:g", Disk = -1 }, new VolumeInfo { Id = "vol:x", Disk = 0, Kind = DeviceKind.Hdd }));
            // big files within one hard disk: the whole file (up to 512 MB) read in 1 MB reads, then written in 64 MB writes, room
            // reserved, nothing through the cache, no trials; one USB drive keeps 32 MB batches with trials; two hard disks unchanged
            var hdd1 = new VolumeInfo { Id = "vol:h1", Kind = DeviceKind.Hdd, Disk = 1, FileSystem = "NTFS" };
            var hdd1b = new VolumeInfo { Id = "vol:h1b", Kind = DeviceKind.Hdd, Disk = 1, FileSystem = "NTFS" };
            var hdd2 = new VolumeInfo { Id = "vol:h2", Kind = DeviceKind.Hdd, Disk = 2, FileSystem = "NTFS" };
            var usb = new VolumeInfo { Id = "vol:u", Kind = DeviceKind.Usb, Disk = 3, FileSystem = "NTFS" };
            var one = Plan.Choose(new JobSpec { Op = OpKind.Copy }, hdd1, hdd1b, null);
            check("one hard disk: whole-file batches of 1 MB reads, 64 MB writes, room reserved, cached end only once read, no trials",
                one.Pipe.Phased && one.Pipe.Chunk == 1 << 20 && one.Pipe.Slots == 512 && one.Pipe.ReadDepth == 32 && one.Pipe.WriteStep == 64 << 20
                && one.Pipe.Prealloc && one.Pipe.TailAfterRead && one.PipeWays.Count == 0);
            var oneUsb = Plan.Choose(new JobSpec { Op = OpKind.Copy }, usb, usb, null);
            check("one USB drive: 32 MB batches with trials", oneUsb.Pipe.Phased && oneUsb.Pipe.Chunk == 32 << 20 && oneUsb.PipeWays.Count > 0);
            var two = Plan.Choose(new JobSpec { Op = OpKind.Copy }, hdd1, hdd2, null);
            check("two hard disks unchanged", !two.Pipe.Phased && two.Pipe.Chunk == 2 << 20 && two.Pipe.Slots == 256 && two.Pipe.ReadDepth == 4 && two.Pipe.TailAfterRead);
            // a hard disk into a fast drive: 1 MB reads, only 4 out at the disk at once, no trials; the other way round unchanged
            var hddToSsd = Plan.Choose(new JobSpec { Op = OpKind.Copy }, hdd1, ntfs, null);
            var ssdToHdd = Plan.Choose(new JobSpec { Op = OpKind.Copy }, ntfs, hdd1, null);
            check("hard disk into an SSD: 1 MB reads, 4 out at once, no trials", !hddToSsd.Pipe.Phased && hddToSsd.Pipe.Chunk == 1 << 20 && hddToSsd.Pipe.Slots == 32
                && hddToSsd.Pipe.ReadDepth == 4 && hddToSsd.PipeWays.Count == 0);
            check("SSD into a hard disk unchanged", ssdToHdd.Pipe.Chunk == 32 << 20 && ssdToHdd.Pipe.ReadDepth == 0 && ssdToHdd.PipeWays.Count == 2);
            check("FAT32 holds files up to 4 GB", new VolumeInfo { FileSystem = "FAT32" }.MaxFileSize == uint.MaxValue
                && new VolumeInfo { FileSystem = "exFAT" }.MaxFileSize == 0 && ntfs.MaxFileSize == 0 && new VolumeInfo().MaxFileSize == 0);
            // the Ctrl+V hook: its own thread, put in again on time (observe only: it never takes a key here)
            bool desktop = Choppy.Explorer.PasteHook.Start(null, null, true);
            if (desktop)
            {
                System.Threading.Thread.Sleep(3500);
                bool ok = Choppy.Explorer.PasteHook.Running && Choppy.Explorer.PasteHook.Refreshes >= 1 && Choppy.Explorer.PasteHook.Dropped == 0;
                Choppy.Explorer.PasteHook.Stop();
                check("Ctrl+V hook on its own thread, refreshed (" + Choppy.Explorer.PasteHook.Refreshes + ")", ok && !Choppy.Explorer.PasteHook.Running);
            }
            else sb.AppendLine("SKIP Ctrl+V hook (no desktop to hook here)");
            BenchChecks(check, sb, desktop);
            sb.AppendLine(fails == 0 ? "ALL PASS" : fails + " FAILED");
            if (outFile != null) File.WriteAllText(outFile, sb.ToString());
            return fails == 0 ? 0 : 2;
        }
    }
}
