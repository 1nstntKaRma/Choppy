using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
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
                        case "--no-prealloc": spec.NoPrealloc = true; break;
                        case "--no-tune": spec.NoTune = true; break;
                        case "--prealloc": spec.ForcePrealloc = true; break;
                        case "--small": spec.SmallMode = args[++i]; break;
                        case "--wchunk": spec.CachedWriteMB = int.Parse(args[++i]); break;
                        case "--no-tail": spec.NoCachedTail = true; break;
                        case "--tail": spec.TailMB = int.Parse(args[++i]); break;
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
                sb.AppendLine("window " + h.ToInt64().ToString("X") + " -> " + (Explorer.ShellLocation.FolderOf(h) ?? "(not a folder - Explorer keeps that paste)"));
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
            sb.AppendLine(fails == 0 ? "ALL PASS" : fails + " FAILED");
            if (outFile != null) File.WriteAllText(outFile, sb.ToString());
            return fails == 0 ? 0 : 2;
        }
    }
}
