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
    ///   Choppy.exe --benchmark [out.json] [--thorough] [--network] [--drives C,D] [--list] [--log progress.txt] [--cancel-after sec] [--tiny]
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
                if (args.Length >= 1 && args[0] == "--benchmark") return BenchCli(args);
                if (args.Length >= 2 && args[0] == "--shellcopy") return Bench.ShellCopy.Child(args[1]);
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
                        case "--workers": spec.ForceWorkers = int.Parse(args[++i]); break;
                        case "--no-fast": spec.NoFastPath = true; break;
                        case "--engine": { string e = args[++i]; spec.Engine = e == "auto" ? null : e; } break;
                        case "--chunk": spec.PipeChunkMB = int.Parse(args[++i]); break;
                        case "--slots": spec.PipeSlots = int.Parse(args[++i]); break;
                        case "--batch": spec.PipeBatch = int.Parse(args[++i]); break;
                        case "--depth": spec.PipeDepth = int.Parse(args[++i]); break;
                        case "--flush-files": spec.FlushFiles = int.Parse(args[++i]); break;
                        case "--flush-threads": spec.FlushThreads = int.Parse(args[++i]); break;
                        case "--no-prealloc": spec.NoPrealloc = true; break;
                        case "--no-tune": spec.NoTune = true; break;
                        case "--prealloc": spec.ForcePrealloc = true; break;
                        case "--small": spec.SmallMode = args[++i]; break;
                        case "--wchunk": spec.CachedWriteMB = int.Parse(args[++i]); break;
                        case "--no-tail": spec.NoCachedTail = true; break;
                        case "--tail": spec.TailMB = int.Parse(args[++i]); break;
                        case "--max-file": spec.MaxFileTest = long.Parse(args[++i]); break;
                        default: rest.Add(a); break;
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
            if (lw != null) { lw.WriteLine(rep.Summary); lw.WriteLine(html); lw.Dispose(); }
            return rep.Canceled ? 3 : rep.Error != null ? 1 : 0;
        }

        /// <summary>A path from the command line, against the folder Choppy was started from.</summary>
        static string Full(string p) { return Path.IsPathRooted(p) ? p : Path.Combine(Program.StartDir ?? "", p); }

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
            double ratio, noise;
            check("bench: same times = tie", Bench.Benchmark.Judge(new[] { 2.0, 2.0 }, new[] { 2.0, 2.0 }, out ratio, out noise) == 0 && Math.Abs(ratio - 1) < 1e-9);
            check("bench: 20% quicker = faster", Bench.Benchmark.Judge(new[] { 1.0, 1.02 }, new[] { 1.2, 1.22 }, out ratio, out noise) == 1 && ratio > 1.15 && ratio < 1.25);
            check("bench: 20% slower = slower", Bench.Benchmark.Judge(new[] { 1.2, 1.22 }, new[] { 1.0, 1.02 }, out ratio, out noise) == -1 && ratio < 0.9);
            check("bench: 3% apart = tie", Bench.Benchmark.Judge(new[] { 1.0, 1.0 }, new[] { 1.03, 1.03 }, out ratio, out noise) == 0);
            check("bench: within the runs' own scatter = tie", Bench.Benchmark.Judge(new[] { 1.0, 1.4 }, new[] { 1.3, 1.35 }, out ratio, out noise) == 0 && noise > 0.15);
            check("bench: median of three", Bench.Benchmark.Median(new[] { 3.0, 1.0, 2.0 }) == 2.0 && Bench.Benchmark.Median(new[] { 1.0, 2.0 }) == 1.5);

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
            check("bench: sizes by speed (1 GB + 1,000 small files at 350 MB/s and more, 512 MB + 500 below)",
                rt(c, c).Big == 1024 * Bench.Benchmark.MB && rt(c, c).Small == 1000 && rt(c, d).Big == 512 * Bench.Benchmark.MB && rt(c, d).Small == 500
                && rt(d, d).Big == 512 * Bench.Benchmark.MB && rt(c, e).Big == 1024 * Bench.Benchmark.MB && rt(e, c).Big == 1024 * Bench.Benchmark.MB && rt(e, e).Big == 512 * Bench.Benchmark.MB);
            check("bench: two cases per route", routes.TrueForAll(r => r.Cases.Count == 2 && r.Cases[0].Files == 1 && r.Cases[1].Files == r.Small && r.Est > 0));
            // many hard disks: one step smaller everywhere, to keep to the time budget
            var many = new List<Bench.BenchDrive> { c };
            for (int i = 0; i < 9; i++) many.Add(drive(((char)('F' + i)).ToString() + ":", false, DeviceKind.Hdd, 3 + i, 150, 150));
            var mr = Bench.Benchmark.PlanRoutes(many);
            bq.SizeRoutes(mr);
            check("bench: a time budget (many hard disks: a step smaller)", mr.Count == 28 && mr[0].Big == 512 * Bench.Benchmark.MB && mr.TrueForAll(r => r.Big <= 512 * Bench.Benchmark.MB) && mr.Exists(r => r.Big == 256 * Bench.Benchmark.MB && r.Small == 250));
            var thorough = new Bench.Benchmark { Thorough = true };
            var tr = Bench.Benchmark.PlanRoutes(new List<Bench.BenchDrive> { c, d });
            thorough.SizeRoutes(tr);
            check("bench: thorough = 2 GB / 1 GB, 2,000 / 1,000 small files, a mixed folder",
                tr.TrueForAll(r => r.Cases.Count == 3) && tr[0].Big == 2048 * Bench.Benchmark.MB && tr[0].Small == 2000 && tr[1].Big == 1024 * Bench.Benchmark.MB && tr[1].Small == 1000);
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
            check("bench: scored (on the disk faster, done slower)", r1.Scored && r1.Verdict == 1 && r1.DoneVerdict == -1 && !r2.Scored && rep.Summary == "Choppy faster on all 1");
            check("bench: summary line", Bench.BenchReport.SummaryOf(9, 2, 1) == "Choppy faster on 9 of 12 · tie on 2 · slower on 1");
            string json = rep.ToJson(), html = rep.ToHtml("x.json"), text = rep.ToText();
            int depth = 0; bool inStr = false, esc = false, balanced = true;
            foreach (char ch in json)
            {
                if (inStr) { if (esc) esc = false; else if (ch == '\\') esc = true; else if (ch == '"') inStr = false; continue; }
                if (ch == '"') inStr = true; else if (ch == '{' || ch == '[') depth++; else if (ch == '}' || ch == ']') { depth--; if (depth < 0) balanced = false; }
            }
            check("bench: JSON well formed", balanced && depth == 0 && !inStr && json.Contains("\"verdict\": \"faster\"") && json.Contains("\\\"NVMe\\\"") && json.Contains("\"error\": \"test error\""));
            check("bench: HTML escaped", html.Contains("&lt;b&gt;&amp;amp;") && !html.Contains("<b>&amp;</b>") && html.Contains("faster on all 1") && html.Contains("not a disk of this PC"));
            check("bench: text table", text.Contains("| C: → D: (NVMe → HDD) | One big file, 1 GB |") && text.Contains("error: test error"));

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
