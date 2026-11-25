using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Net;
using System.Text;
using Choppy.Engine;
using Microsoft.Win32;

namespace Choppy.Bench
{
    internal sealed class BenchDrive
    {
        public string Letter;          // "C:"
        public string Root;            // "C:\"
        public string Label, Model, Bus, FileSystem, Remote;
        public long Size, Free;
        public bool System, Network;
        public VolumeInfo Vol;
        public string Skip;            // why it isn't tested (null = tested)
        public string FoundSkip;       // the reason found when the drives were listed (a preview adds free space to it)
        public string TestDir;         // its temporary folder while the benchmark runs
        public double ReadMBps, WriteMBps;
        public bool Measured;
        public bool RealDisk;          // a disk device that describes itself (not a cloud or virtual file system)
        public bool PurgeOk = true;    // opening a file past the cache drops it from the cache here
        public string PurgeNote;

        public string Name { get { return Letter + (string.IsNullOrEmpty(Label) ? "" : " " + Label); } }

        /// <summary>A USB drive: how it is connected (null = not USB, or Windows doesn't say).</summary>
        public UsbLink Usb { get { return Vol != null ? Vol.Usb : null; } }

        /// <summary>"NVMe SSD", "Hard disk", "USB SSD", "Network share"...</summary>
        public string Kind
        {
            get
            {
                if (Network) return "Network share";
                if (Vol == null) return "Drive";
                switch (Vol.Kind)
                {
                    case DeviceKind.Ssd: return Bus == "NVMe" ? "NVMe SSD" : Bus == "USB" ? "USB SSD" : Bus == "SATA" ? "SATA SSD" : "SSD";
                    case DeviceKind.Hdd: return Bus == "USB" ? "USB hard disk" : "Hard disk";
                    case DeviceKind.Usb: return "USB drive";
                    case DeviceKind.Ram: return "RAM disk";
                    case DeviceKind.Optical: return "Disc";
                }
                return Bus != null ? Bus + " disk" : "Disk";
            }
        }

        /// <summary>Short kind for the route lines: "NVMe", "HDD", "USB SSD".</summary>
        public string Short
        {
            get
            {
                string k = Kind;
                switch (k)
                {
                    case "NVMe SSD": return "NVMe";
                    case "SATA SSD": case "SSD": return "SSD";
                    case "Hard disk": return "HDD";
                    case "USB hard disk": return "USB HDD";
                    case "USB drive": return "USB";
                    case "Network share": return "Network";
                    case "RAM disk": return "RAM";
                }
                return k;
            }
        }
    }

    internal sealed class BenchCase
    {
        public string Key;                // "big", "small", "mixed"
        public long[] Big = new long[0];  // big files
        public long[] Smalls = new long[0];
        public int Files { get { return Big.Length + Smalls.Length; } }
        public long Bytes { get { return Big.Sum() + Smalls.Sum(); } }

        public string Title
        {
            get
            {
                if (Key == "big") return "One big file";
                if (Key == "small") return Smalls.Length.ToString("N0", CultureInfo.InvariantCulture) + " small files";
                return "Mixed folder";
            }
        }

        public string Detail
        {
            get
            {
                if (Key == "big") return BenchText.Size(Bytes);
                if (Key == "small") return BenchText.Size(Bytes) + " · 4 KB - 1 MB each";
                return BenchText.Size(Bytes) + " · " + Big.Length + " big + " + Smalls.Length.ToString("N0", CultureInfo.InvariantCulture) + " small";
            }
        }
    }

    internal sealed class BenchRun
    {
        public bool Choppy;
        public double Done;      // seconds until it said it was done
        public double OnDisk;    // ... and until its copies were on the disk
        public double After;     // step 3: of that, the Rust child's process ending (counted against it)
    }

    internal sealed class BenchResult
    {
        public BenchDrive From, To;
        public BenchCase Case;
        public readonly List<BenchRun> Runs = new List<BenchRun>();
        public string Method;    // the way Choppy's Adaptive mode picked
        public string Error;
        public double ChoppySecs, WindowsSecs;         // average seconds until the copies are on the disk: these decide
        public double ChoppyMBps, WindowsMBps;         // the bytes over those average times
        public double ChoppyDoneMBps, WindowsDoneMBps; // until each said it was done (kept in the raw data only)
        public double Saved, DoneSaved;   // how much less time Choppy took than Windows (0.12 = 12% less; below 0 = more)
        public int Verdict, DoneVerdict;  // +1 Faster, 0 Tie (a Marginal Tie: less than 4% apart), -1 Slower
        public bool Scored;

        public bool Within { get { return From == To; } }
        public string Route { get { return From.Letter + " → " + To.Letter; } }
        public string RouteKinds { get { return Within ? "within one " + From.Kind : From.Short + " → " + To.Short; } }

        List<double> Times(bool choppy, bool disk) { return Runs.Where(r => r.Choppy == choppy).Select(r => disk ? r.OnDisk : r.Done).ToList(); }

        public void Score()
        {
            Verdict = Benchmark.Judge(Times(true, true), Times(false, true), out Saved);
            DoneVerdict = Benchmark.Judge(Times(true, false), Times(false, false), out DoneSaved);
            ChoppySecs = Benchmark.Mean(Times(true, true));
            WindowsSecs = Benchmark.Mean(Times(false, true));
            ChoppyMBps = Speed(Times(true, true));
            WindowsMBps = Speed(Times(false, true));
            ChoppyDoneMBps = Speed(Times(true, false));
            WindowsDoneMBps = Speed(Times(false, false));
            Scored = Error == null && ChoppyMBps > 0 && WindowsMBps > 0;
        }

        /// <summary>The average speed: the bytes over the average time of the runs.</summary>
        double Speed(List<double> secs)
        {
            double m = Benchmark.Mean(secs);
            return m > 0 ? Case.Bytes / (double)Benchmark.MB / m : 0;
        }
    }

    /// <summary>
    /// Every measured case together: the average of their time saved, each one's average speed (all the bytes over all
    /// the time, every case copied once) and the time Choppy saved, added up.
    /// </summary>
    internal sealed class BenchTotals
    {
        public int Cases;
        public double AvgSaved;                  // the average of the cases' time saved (0.12 = 12% less time; below 0 = more)
        public long Bytes;                       // every case once
        public double ChoppySecs, WindowsSecs;   // the cases' average times, added up
        public double ChoppyMBps, WindowsMBps;   // the bytes over those times

        /// <summary>How much less time Choppy took, every case copied once (below 0 = more).</summary>
        public double SecsSaved { get { return WindowsSecs - ChoppySecs; } }

        /// <summary>null when no case was measured.</summary>
        public static BenchTotals Of(IEnumerable<BenchResult> results)
        {
            var rs = results.Where(r => r.Scored).ToList();
            if (rs.Count == 0) return null;
            var t = new BenchTotals
            {
                Cases = rs.Count,
                AvgSaved = rs.Average(r => r.Saved),
                Bytes = rs.Sum(r => r.Case.Bytes),
                ChoppySecs = rs.Sum(r => r.ChoppySecs),
                WindowsSecs = rs.Sum(r => r.WindowsSecs),
            };
            t.ChoppyMBps = t.ChoppySecs > 0 ? t.Bytes / (double)Benchmark.MB / t.ChoppySecs : 0;
            t.WindowsMBps = t.WindowsSecs > 0 ? t.Bytes / (double)Benchmark.MB / t.WindowsSecs : 0;
            return t;
        }

        /// <summary>"faster" / "slower" / "even", as the average rounds to a whole percent.</summary>
        public string Word { get { string c = BenchText.SavedClass(AvgSaved); return c == "good" ? "faster" : c == "bad" ? "slower" : "even"; } }

        /// <summary>"good" / "bad" / "tie" for the seconds saved (to a hundredth of a second).</summary>
        public string SecsClass { get { double s = Math.Round(SecsSaved, 2); return s > 0 ? "good" : s < 0 ? "bad" : "tie"; } }

        /// <summary>"+23% faster on average · Choppy 1.21 GB/s · Windows 980 MB/s · Time saved 1 min 2 s"</summary>
        public string Line
        {
            get
            {
                return BenchText.Saved(AvgSaved) + " " + Word + " on average · Choppy " + BenchText.Speed(ChoppyMBps) + " · Windows " + BenchText.Speed(WindowsMBps)
                    + " · Time saved " + BenchText.SignedSpan(SecsSaved);
            }
        }
    }

    /// <summary>The whole run: this PC, its drives, every case's numbers. Saved as an HTML page and as JSON.</summary>
    internal sealed class BenchReport
    {
        public string Version = typeof(BenchReport).Assembly.GetName().Version.ToString(3);
        public DateTime Started;
        public double Seconds;
        public string Mode;
        public int Rounds, Planned;
        public string Cpu, Windows;
        public int Threads;
        public double RamGB;
        public bool Admin;
        public List<BenchDrive> Drives;
        public readonly List<BenchResult> Results = new List<BenchResult>();
        public readonly List<BenchResult> RustResults = new List<BenchResult>();   // step 3 (rust branch only): the Rust engine's, beside Results
        public readonly List<string> Notes = new List<string>();
        public bool Canceled;
        public string Error;

        public int Faster { get { return Results.Count(r => r.Scored && r.Verdict > 0); } }
        public int Ties { get { return Results.Count(r => r.Scored && r.Verdict == 0); } }
        public int Slower { get { return Results.Count(r => r.Scored && r.Verdict < 0); } }
        public int Scored { get { return Results.Count(r => r.Scored); } }

        /// <summary>Every measured case together (null when none was).</summary>
        public BenchTotals Totals { get { return BenchTotals.Of(Results); } }

        /// <summary>The whole run ended (not stopped, no error): only then can it say "Never Slower".</summary>
        public bool Ended { get { return !Canceled && Error == null; } }

        /// <summary>"Choppy is Faster 9 of 12 · Marginal Tie 2 of 12 · Slower 1 of 12"</summary>
        public string Summary { get { return SummaryOf(Faster, Ties, Slower, Ended); } }

        /// <summary>The same by the time each one said it was done (kept in the raw data).</summary>
        public string DoneSummary
        {
            get
            {
                int f = Results.Count(r => r.Scored && r.DoneVerdict > 0), t = Results.Count(r => r.Scored && r.DoneVerdict == 0), s = Results.Count(r => r.Scored && r.DoneVerdict < 0);
                return SummaryOf(f, t, s, false);
            }
        }

        /// <summary>The summary and the notes say "Marginal Tie"; each case's result says "Tie" (the notes explain it once).</summary>
        public const string TieWord = "Marginal Tie";

        /// <summary>The verdict as each case's result says it.</summary>
        public static string Word(int verdict) { return verdict > 0 ? "Faster" : verdict < 0 ? "Slower" : "Tie"; }

        /// <summary>"10 of 19"</summary>
        public static string Of(int count, int n) { return count + " of " + n; }

        /// <summary>
        /// "Choppy is Faster 10 of 19 · Marginal Tie 9 of 19 · Never Slower": a part for each verdict there is; "Never
        /// Slower" only when a whole run ended with ties and no case slower (all faster says it already).
        /// </summary>
        public static string SummaryOf(int faster, int ties, int slower, bool ended)
        {
            int n = faster + ties + slower;
            if (n == 0) return "No case was measured";
            var s = "Choppy is Faster " + Of(faster, n);
            if (ties > 0) s += " · " + TieWord + " " + Of(ties, n);
            if (slower > 0) s += " · Slower " + Of(slower, n);
            else if (ended && ties > 0) s += " · Never Slower";
            return s;
        }

        public void ReadMachine()
        {
            try
            {
                using (var k = Registry.LocalMachine.OpenSubKey(@"HARDWARE\DESCRIPTION\System\CentralProcessor\0"))
                    Cpu = k != null ? ((k.GetValue("ProcessorNameString") as string) ?? "").Trim() : null;
            }
            catch { }
            Threads = Environment.ProcessorCount;
            try
            {
                var m = new Native.MEMORYSTATUSEX { dwLength = (uint)System.Runtime.InteropServices.Marshal.SizeOf(typeof(Native.MEMORYSTATUSEX)) };
                if (Native.GlobalMemoryStatusEx(ref m)) RamGB = Math.Round(m.ullTotalPhys / (double)(1L << 30), 1);
            }
            catch { }
            try
            {
                using (var k = Registry.LocalMachine.OpenSubKey(@"SOFTWARE\Microsoft\Windows NT\CurrentVersion"))
                {
                    string name = (k.GetValue("ProductName") as string) ?? "Windows";
                    string build = (k.GetValue("CurrentBuild") as string) ?? "";
                    string disp = (k.GetValue("DisplayVersion") as string) ?? (k.GetValue("ReleaseId") as string);
                    object ubr = k.GetValue("UBR");
                    int b;
                    // Windows 11 still calls itself "Windows 10" here
                    if (int.TryParse(build, out b) && b >= 22000) name = name.Replace("Windows 10", "Windows 11");
                    Windows = name + (disp != null ? " " + disp : "") + " (build " + build + (ubr != null ? "." + ubr : "") + ")";
                }
            }
            catch { Windows = Environment.OSVersion.VersionString; }
            try { Admin = new System.Security.Principal.WindowsPrincipal(System.Security.Principal.WindowsIdentity.GetCurrent()).IsInRole(System.Security.Principal.WindowsBuiltInRole.Administrator); } catch { }
        }

        public string Machine
        {
            get
            {
                return (Cpu ?? "CPU unknown") + " (" + Threads + " threads), " + RamGB.ToString("0.#", CultureInfo.InvariantCulture) + " GB RAM, " + Windows;
            }
        }

        // ================= saving =================

        /// <summary>Saves benchmark-&lt;date&gt;.html and .json in Choppy's Logs folder (or next to a given .json path).</summary>
        public void Save(string jsonPath, out string html, out string json)
        {
            if (jsonPath == null)
            {
                string dir = AppPaths.LogDir;
                Directory.CreateDirectory(dir);
                string b = Path.Combine(dir, "benchmark-" + Started.ToString("yyyy-MM-dd HH-mm-ss", CultureInfo.InvariantCulture));
                json = b + ".json";
                html = b + ".html";
            }
            else
            {
                json = jsonPath;
                html = Path.ChangeExtension(jsonPath, ".html");
                string d = Path.GetDirectoryName(Path.GetFullPath(jsonPath));
                if (!string.IsNullOrEmpty(d)) Directory.CreateDirectory(d);
            }
            File.WriteAllText(json, ToJson(), new UTF8Encoding(false));
            File.WriteAllText(html, ToHtml(Path.GetFileName(json)), new UTF8Encoding(false));
        }

        // ================= JSON =================

        static string N(double v, string fmt = "0.###") { return double.IsNaN(v) || double.IsInfinity(v) ? "null" : v.ToString(fmt, CultureInfo.InvariantCulture); }

        static string S(string s)
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

        static string Verdict(int v) { return v > 0 ? "faster" : v < 0 ? "slower" : "marginal tie"; }

        public string ToJson()
        {
            var j = new StringBuilder();
            j.Append("{\n");
            j.Append("  \"tool\": \"Choppy Benchmark\",\n");
            j.Append("  \"version\": " + S(Version) + ",\n");
            j.Append("  \"date\": " + S(Started.ToString("yyyy-MM-ddTHH:mm:ss", CultureInfo.InvariantCulture)) + ",\n");
            j.Append("  \"mode\": " + S(Mode) + ", \"rounds\": " + Rounds + ", \"seconds\": " + N(Seconds, "0.0") + ",\n");
            j.Append("  \"canceled\": " + (Canceled ? "true" : "false") + ", \"error\": " + S(Error) + ",\n");
            j.Append("  \"units\": \"seconds = the average time until the copies are on the disk; MB/s = 1,048,576 bytes per second over that time; timeSaved = how much less time Choppy took than Windows (below 0 = more)\",\n");
            j.Append("  \"machine\": { \"cpu\": " + S(Cpu) + ", \"threads\": " + Threads + ", \"ramGB\": " + N(RamGB) + ", \"windows\": " + S(Windows) + ", \"admin\": " + (Admin ? "true" : "false") + " },\n");
            j.Append("  \"summary\": { \"text\": " + S(Summary) + ", \"cases\": " + Scored + ", \"faster\": " + Faster + ", \"marginalTie\": " + Ties + ", \"slower\": " + Slower
                + ", \"byDone\": " + S(DoneSummary) + " },\n");
            var tot = Totals;
            if (tot != null)
                j.Append("  \"totals\": { \"cases\": " + tot.Cases + ", \"averageTimeSaved\": " + N(tot.AvgSaved, "0.0000") + ", \"bytes\": " + tot.Bytes
                    + ", \"choppySeconds\": " + N(tot.ChoppySecs, "0.000") + ", \"windowsSeconds\": " + N(tot.WindowsSecs, "0.000") + ", \"secondsSaved\": " + N(tot.SecsSaved, "0.000")
                    + ", \"choppyMBps\": " + N(tot.ChoppyMBps, "0.0") + ", \"windowsMBps\": " + N(tot.WindowsMBps, "0.0")
                    + ", \"about\": \"averageTimeSaved = the average of the cases' timeSaved; seconds = the cases' average times added up (every case copied once); MBps = all the bytes over those seconds\" },\n");
            j.Append("  \"drives\": [");
            var ds = Drives ?? new List<BenchDrive>();
            for (int i = 0; i < ds.Count; i++)
            {
                var d = ds[i];
                j.Append(i == 0 ? "\n" : ",\n");
                j.Append("    { \"letter\": " + S(d.Letter) + ", \"label\": " + S(d.Label) + ", \"kind\": " + S(d.Skip == null || d.Vol != null ? d.Kind : null)
                    + ", \"model\": " + S(d.Model) + ", \"bus\": " + S(d.Bus) + ", \"fileSystem\": " + S(d.FileSystem)
                    + ", \"sizeGB\": " + N(d.Size / (double)(1L << 30), "0.0") + ", \"freeGB\": " + N(d.Free / (double)(1L << 30), "0.0")
                    + ", \"system\": " + (d.System ? "true" : "false") + ", \"network\": " + (d.Network ? "true" : "false")
                    + ", \"physicalDisk\": " + (d.Vol != null ? d.Vol.Disk : -1)
                    + ", \"tested\": " + (d.Skip == null && d.Measured ? "true" : "false") + ", \"skipped\": " + S(d.Skip)
                    + ", \"readMBps\": " + (d.Measured ? N(d.ReadMBps, "0") : "null") + ", \"writeMBps\": " + (d.Measured ? N(d.WriteMBps, "0") : "null")
                    + ", \"cacheDrop\": " + S(d.PurgeNote) + ", \"usb\": " + UsbJson(d.Usb) + " }");
            }
            j.Append("\n  ],\n  \"results\": [");
            ResultsJson(j, Results);
            if (RustResults.Count > 0)
            {
                // step 3 (rust branch only): the Rust engine's results, against the same Windows runs
                var rt = BenchTotals.Of(RustResults);
                j.Append("\n  ],\n  \"rustSummary\": { \"text\": " + S(SummaryOf(RustResults.Count(r => r.Scored && r.Verdict > 0), RustResults.Count(r => r.Scored && r.Verdict == 0), RustResults.Count(r => r.Scored && r.Verdict < 0), Ended))
                    + ", \"averageTimeSaved\": " + (rt != null ? N(rt.AvgSaved, "0.0000") : "null") + ", \"choppySeconds\": " + (rt != null ? N(rt.ChoppySecs, "0.000") : "null")
                    + ", \"windowsSeconds\": " + (rt != null ? N(rt.WindowsSecs, "0.000") : "null") + " },\n  \"rustResults\": [");
                ResultsJson(j, RustResults);
            }
            j.Append("\n  ],\n  \"notes\": [");
            for (int i = 0; i < Notes.Count; i++) j.Append((i == 0 ? "" : ", ") + S(Notes[i]));
            j.Append("]\n}\n");
            return j.ToString();
        }

        static void ResultsJson(StringBuilder j, List<BenchResult> results)
        {
            for (int i = 0; i < results.Count; i++)
            {
                var r = results[i];
                j.Append(i == 0 ? "\n" : ",\n");
                j.Append("    { \"from\": " + S(r.From.Letter) + ", \"to\": " + S(r.To.Letter) + ", \"route\": " + S(r.From.Letter + " -> " + r.To.Letter)
                    + ", \"kinds\": " + S(r.RouteKinds.Replace("→", "->")) + ", \"case\": " + S(r.Case.Key) + ", \"title\": " + S(r.Case.Title)
                    + ", \"files\": " + r.Case.Files + ", \"bytes\": " + r.Case.Bytes + ",\n");
                j.Append("      \"verdict\": " + (r.Scored ? S(Verdict(r.Verdict)) : "null") + ", \"timeSaved\": " + N(r.Saved, "0.0000")
                    + ", \"doneVerdict\": " + (r.Scored ? S(Verdict(r.DoneVerdict)) : "null") + ", \"doneTimeSaved\": " + N(r.DoneSaved, "0.0000") + ",\n");
                j.Append("      \"choppy\": { \"seconds\": " + N(r.ChoppySecs, "0.000") + ", \"MBps\": " + N(r.ChoppyMBps, "0.0") + ", \"doneMBps\": " + N(r.ChoppyDoneMBps, "0.0") + ", \"method\": " + S(r.Method) + ", \"runs\": " + Runs(r, true) + " },\n");
                j.Append("      \"windows\": { \"seconds\": " + N(r.WindowsSecs, "0.000") + ", \"MBps\": " + N(r.WindowsMBps, "0.0") + ", \"doneMBps\": " + N(r.WindowsDoneMBps, "0.0") + ", \"runs\": " + Runs(r, false) + " },\n");
                j.Append("      \"error\": " + S(r.Error) + " }");
            }
        }

        /// <summary>{ "runsAt": "USB 2.0", "canDo": "5 Gbps", "port": "USB 3", "uasp": false, "heldBack": true } or null</summary>
        static string UsbJson(UsbLink u)
        {
            if (u == null) return "null";
            return "{ \"runsAt\": " + S(UsbLink.Short(u.Speed)) + ", \"canDo\": " + (u.CapableKnown ? S(UsbLink.Short(u.Capable)) : "null")
                + ", \"port\": " + (u.PortKnown ? S(u.PortUsb3 ? "USB 3" : "USB 2") : "null") + ", \"uasp\": " + (u.Uasp ? "true" : "false")
                + ", \"heldBack\": " + (u.HeldBack ? "true" : "false") + ", \"advice\": " + S(u.Advice) + " }";
        }

        /// <summary>The tested drives that run slower than they can, because of the port, cable or hub: "F: runs at USB 2.0 ..."</summary>
        public List<string> HeldBack()
        {
            return (Drives ?? new List<BenchDrive>()).Where(d => d.Usb != null && d.Usb.HeldBack && (d.Skip == null || d.Measured))
                .Select(d => d.Letter + " " + d.Usb.Advice).ToList();
        }

        static string Runs(BenchResult r, bool choppy)
        {
            return "[" + string.Join(", ", r.Runs.Where(x => x.Choppy == choppy).Select(x => "{ \"done\": " + N(x.Done, "0.000") + ", \"onDisk\": " + N(x.OnDisk, "0.000") + " }")) + "]";
        }

        static string Cap(string s) { return string.IsNullOrEmpty(s) ? s : char.ToUpperInvariant(s[0]) + s.Substring(1); }

        /// <summary>"Quick", or "Quick, 5 runs each" when the runs were changed from the default.</summary>
        string ModeText
        {
            get { return Cap(Mode) + (Rounds > 0 && Rounds != Benchmark.DefaultRounds(Mode == "thorough") ? ", " + Rounds + (Rounds == 1 ? " run each" : " runs each") : ""); }
        }

        // ================= HTML =================

        static string H(string s) { return WebUtility.HtmlEncode(s ?? ""); }

        static string Cls(int verdict) { return verdict > 0 ? "good" : verdict < 0 ? "bad" : "tie"; }

        /// <summary>The summary line with each verdict in its color.</summary>
        string SummaryHtml()
        {
            int n = Scored;
            var h = new StringBuilder("Choppy is <span class=\"good\">Faster " + Of(Faster, n) + "</span>");
            if (Ties > 0) h.Append(" · <span class=\"tie\">" + TieWord + " " + Of(Ties, n) + "</span>");
            if (Slower > 0) h.Append(" · <span class=\"bad\">Slower " + Of(Slower, n) + "</span>");
            else if (Ended && Ties > 0) h.Append(" · <span class=\"good\">Never Slower</span>");
            return h.ToString();
        }

        /// <summary>Average time saved · Choppy's average speed · Windows' · the time saved, each in its color (none measured: nothing).</summary>
        string TotalsHtml()
        {
            var t = Totals;
            if (t == null) return "";
            string cases = t.Cases + (t.Cases == 1 ? " case" : " cases");
            return "<section class=\"totals\">"
                + Stat("On average", "<span class=\"" + BenchText.SavedClass(t.AvgSaved) + "\">" + H(BenchText.Saved(t.AvgSaved)) + "<small> " + t.Word + "</small></span>",
                    "average of " + cases, "The average of every case's Time saved")
                + Stat("Choppy average", "<span class=\"c\">" + H(BenchText.Speed(t.ChoppyMBps)) + "</span>",
                    BenchText.Size(t.Bytes) + " in " + BenchText.Span(t.ChoppySecs), "All the bytes over all the time, every case copied once")
                + Stat("Windows average", "<span class=\"w\">" + H(BenchText.Speed(t.WindowsMBps)) + "</span>",
                    BenchText.Size(t.Bytes) + " in " + BenchText.Span(t.WindowsSecs), "All the bytes over all the time, every case copied once")
                + Stat("Time saved", "<span class=\"" + t.SecsClass + "\">" + H(BenchText.SignedSpan(t.SecsSaved)) + "</span>",
                    "every case copied once", "How much less time Choppy took than Windows (green), or more (red)")
                + "</section>\n";
        }

        static string Stat(string cap, string valueHtml, string sub, string title)
        {
            return "<div class=\"stat\" title=\"" + H(title) + "\"><div class=\"cap\">" + H(cap) + "</div><div class=\"big\">" + valueHtml + "</div><div class=\"faint\">" + H(sub) + "</div></div>";
        }

        public string ToHtml(string jsonName)
        {
            var h = new StringBuilder();
            h.Append("<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
            h.Append("<title>Choppy Benchmark · " + H(Started.ToString("d MMM yyyy, HH:mm", CultureInfo.InvariantCulture)) + "</title>\n<style>\n" + Css + "</style></head><body><div class=\"wrap\">\n");

            // header
            string icon = BenchText.IconDataUri();
            h.Append("<header>" + (icon != null ? "<img src=\"" + icon + "\" width=\"52\" height=\"52\" alt=\"\">" : "")
                + "<div><h1>Choppy VS Windows</h1><div class=\"dim\">Choppy v" + H(Version) + " Benchmark · "
                + H(Started.ToString("d MMM yyyy, HH:mm", CultureInfo.InvariantCulture)) + " · " + H(ModeText) + " · took " + H(BenchText.Duration(Seconds)) + "</div></div></header>\n");

            // the whole run in four numbers
            h.Append(TotalsHtml());

            // summary
            int n = Scored;
            h.Append("<section class=\"card\">");
            if (Canceled) h.Append("<div class=\"warn\">Stopped before the end: these are the cases that finished.</div>");
            if (Error != null) h.Append("<div class=\"warn\">" + H(Error) + "</div>");
            if (n > 0)
            {
                h.Append("<div class=\"summary\">" + SummaryHtml() + "</div>");
                h.Append("<p class=\"dim\">The time decides: the average of each one's runs until the copies are on the disk. Less than " + (int)(Benchmark.TieMargin * 100)
                    + "% apart is a " + TieWord + ".</p>");
            }
            else if (!Canceled && Error == null) h.Append("<div class=\"summary\">No case was measured</div>");
            foreach (var hb in HeldBack()) h.Append("<p class=\"warn\">Held back: " + H(hb) + " Both copies ran at that speed.</p>");
            h.Append("</section>\n");

            // results
            if (Results.Count > 0)
            {
                h.Append("<section class=\"card\"><h2>Results</h2><div class=\"scroll\"><table class=\"res\"><thead><tr><th>Route</th><th>What</th><th class=\"r\">Choppy</th><th class=\"r\">Windows</th><th class=\"r\">Time saved</th><th>Result</th></tr></thead><tbody>\n");
                foreach (var g in Results.GroupBy(r => r.Route + "|" + r.RouteKinds))
                {
                    var rows = g.ToList();
                    for (int i = 0; i < rows.Count; i++)
                    {
                        var r = rows[i];
                        h.Append("<tr" + (i == 0 ? " class=\"first\"" : "") + ">");
                        if (i == 0) h.Append("<td rowspan=\"" + rows.Count + "\" class=\"route\"><b>" + H(r.Route) + "</b><div class=\"faint\">" + H(r.RouteKinds) + "</div></td>");
                        h.Append("<td>" + H(r.Case.Title) + "<div class=\"faint\">" + H(r.Case.Detail) + "</div></td>");
                        if (!r.Scored)
                        {
                            h.Append("<td colspan=\"3\" class=\"faint\">" + H(r.Error ?? "not measured") + "</td><td><span class=\"badge b-err\">Error</span></td></tr>\n");
                            continue;
                        }
                        double max = Math.Max(r.ChoppyMBps, r.WindowsMBps);
                        h.Append("<td class=\"r\"><span class=\"c\">" + H(BenchText.Speed(r.ChoppyMBps)) + "</span><div class=\"faint\">" + H(BenchText.Secs(r.ChoppySecs)) + "</div></td>");
                        h.Append("<td class=\"r\"><span class=\"w\">" + H(BenchText.Speed(r.WindowsMBps)) + "</span><div class=\"faint\">" + H(BenchText.Secs(r.WindowsSecs)) + "</div></td>");
                        h.Append("<td class=\"r\"><b class=\"" + BenchText.SavedClass(r.Saved) + "\">" + H(BenchText.Saved(r.Saved)) + "</b><div class=\"bars\"><i class=\"bc\" style=\"width:" + Pct(r.ChoppyMBps, max) + "%\"></i><i class=\"bw\" style=\"width:" + Pct(r.WindowsMBps, max) + "%\"></i></div></td>");
                        h.Append("<td><span class=\"badge b-" + Cls(r.Verdict) + "\">" + Word(r.Verdict) + "</span></td></tr>\n");
                    }
                }
                h.Append("</tbody></table></div><p class=\"faint\">The average speed, and under it the average time, until the copies are on the disk. Time saved: how much less time "
                    + "Choppy took than Windows (green), or more (red). Bars: <span class=\"c\">Choppy</span> and <span class=\"w\">Windows</span>.</p></section>\n");
            }

            // this PC
            h.Append("<section class=\"card\"><h2>This PC</h2><dl>");
            h.Append("<dt>CPU</dt><dd>" + H(Cpu ?? "unknown") + " · " + Threads + " threads</dd>");
            h.Append("<dt>Memory</dt><dd>" + H(RamGB.ToString("0.#", CultureInfo.InvariantCulture)) + " GB</dd>");
            h.Append("<dt>Windows</dt><dd>" + H(Windows) + "</dd>");
            h.Append("<dt>Choppy</dt><dd>v" + H(Version) + (Admin ? " · ran as administrator" : "") + "</dd>");
            h.Append("</dl>");
            h.Append("<div class=\"scroll\"><table class=\"drv\"><thead><tr><th>Drive</th><th>Kind</th><th>Model</th><th>File system</th><th class=\"r\">Size</th><th class=\"r\">Free</th><th class=\"r\">Measured</th><th>Tested</th></tr></thead><tbody>\n");
            foreach (var d in Drives ?? new List<BenchDrive>())
            {
                h.Append("<tr><td><b>" + H(d.Letter) + "</b> " + H(d.Label) + (d.System ? " <span class=\"faint\">(system)</span>" : "") + "</td>");
                h.Append("<td>" + H(d.Vol != null || d.Network ? d.Kind : "") + (d.Usb != null ? "<div class=\"" + (d.Usb.HeldBack ? "warn" : "faint") + "\" title=\"" + H(UsbLink.Aka(d.Usb.Speed)) + "\">"
                    + H(UsbLink.Name(d.Usb.Speed)) + (d.Usb.HeldBack ? " - can do " + H(UsbLink.Short(d.Usb.Capable)) : "") + (d.Usb.Uasp ? " · UASP" : "") + "</div>" : "") + "</td>");
                h.Append("<td>" + H(d.Model ?? "") + (d.Bus != null ? "<div class=\"faint\">" + H(d.Bus) + (d.Vol != null && d.Vol.Disk >= 0 ? " · disk " + d.Vol.Disk : "") + "</div>" : "") + "</td>");
                h.Append("<td>" + H(d.FileSystem ?? "") + "</td>");
                h.Append("<td class=\"r\">" + (d.Size > 0 ? H(BenchText.Size(d.Size)) : "") + "</td><td class=\"r\">" + (d.Size > 0 ? H(BenchText.Size(d.Free)) : "") + "</td>");
                h.Append("<td class=\"r\">" + (d.Measured ? "read " + H(BenchText.Speed(d.ReadMBps)) + "<div class=\"faint\">write " + H(BenchText.Speed(d.WriteMBps)) + "</div>" : "") + "</td>");
                h.Append("<td>" + (d.Skip == null && d.Measured ? "<span class=\"good\">yes</span>" : "<span class=\"faint\">" + H(d.Skip ?? (Canceled ? "stopped before it" : "no")) + "</span>") + "</td></tr>\n");
            }
            h.Append("</tbody></table></div></section>\n");

            // every run
            if (Results.Count > 0)
            {
                h.Append("<section class=\"card\"><details><summary><h2>Every run</h2></summary><div class=\"scroll\"><table class=\"runs\"><thead><tr><th>Route</th><th>What</th><th>Choppy (s)</th><th>Windows (s)</th><th>Choppy's way</th></tr></thead><tbody>\n");
                foreach (var r in Results)
                {
                    h.Append("<tr><td>" + H(r.Route) + "</td><td>" + H(r.Case.Title) + "</td><td>" + RunList(r, true) + "</td><td>" + RunList(r, false) + "</td><td class=\"faint\">" + H(r.Method ?? "") + "</td></tr>\n");
                }
                h.Append("</tbody></table></div><p class=\"faint\">Seconds until the copies were on the disk, in the order they ran: each case alternates (Choppy, Windows, Windows, Choppy - or the other way round).</p></details></section>\n");
            }

            // how
            h.Append("<section class=\"card\"><h2>How it's measured</h2><ul class=\"dim\">");
            h.Append("<li>Routes: from the system drive to each other drive and back, and within each drive.</li>");
            h.Append("<li>Each case is copied " + Rounds + " times by each, in turns, into a new folder. The source is never in memory: it is read once untimed (a new file's first open is slower), then dropped from the Windows cache before every copy.</li>");
            h.Append("<li>Sizes go by each route's speed: 2 GB and 1,000 small files where it does 350 MB/s or more, 1 GB and 500 files below that, 512 MB and 250 under 40 MB/s (Thorough: twice the big file and the small files, and a mixed folder).</li>");
            h.Append("<li>Choppy: its own engine, as every copy runs it (Adaptive picks the way per drive). Windows: the shell copy engine Explorer uses for a paste (IFileOperation), without its window.</li>");
            h.Append("<li>Each time runs until the copies are on the disk: what is still in memory when a program says it's done is written out (every copy is flushed). The numbers are the average of each one's runs.</li>");
            h.Append("<li>Test files go into a temporary folder on each drive (\"" + H(BenchFiles.FolderName) + "\"), removed at the end.</li>");
            h.Append("</ul>");
            foreach (var note in Notes) h.Append("<p class=\"warn\">" + H(note) + "</p>");
            h.Append("</section>\n");
            h.Append("<footer class=\"faint\">The raw numbers: " + H(jsonName) + " (next to this page).</footer>\n");
            h.Append("</div></body></html>\n");
            return h.ToString();
        }

        static string Pct(double v, double max) { return N(max > 0 ? Math.Max(2, 100 * v / max) : 0, "0.#"); }

        static string RunList(BenchResult r, bool choppy)
        {
            return string.Join("<br>", r.Runs.Where(x => x.Choppy == choppy).Select(x => H(N(x.OnDisk, "0.00"))));
        }

        const string Css =
            ":root{--card:#171A1F;--border:#262B33;--sep:#2E343C;--text:#F1F3F5;--dim:#9AA2AD;--label:#8A929E;--faint:#6B737E;--good:#4CC38A;--bad:#F07A72;--warn:#E8B04B;--c:#5AC8FA;--w:#DDE1E6}\n"
            + "*{box-sizing:border-box}html{background:#1B1F25}\n"
            + "body{margin:0;min-height:100vh;background:linear-gradient(#232931,#1B1F25 420px);color:var(--text);font:14px/1.5 \"Segoe UI Variable Text\",\"Segoe UI\",system-ui,sans-serif}\n"
            + ".wrap{max-width:1060px;margin:0 auto;padding:28px 16px 40px}\n"
            + "header{display:flex;gap:16px;align-items:center;margin-bottom:6px}h1{font:600 26px/1.2 \"Segoe UI Variable Display\",\"Segoe UI\",sans-serif;margin:0 0 3px}\n"
            + "h2{font-size:12px;letter-spacing:.06em;text-transform:uppercase;color:var(--label);margin:0 0 10px;font-weight:600;display:inline-block}\n"
            + ".card{background:var(--card);border:1px solid var(--border);border-radius:10px;padding:16px 18px;margin-top:14px}\n"
            + ".summary{font-size:23px;font-weight:600;line-height:1.3}.summary+p{margin:8px 0 0}\n"
            + ".totals{display:grid;grid-template-columns:repeat(4,1fr);background:var(--card);border:1px solid var(--border);border-radius:10px;margin-top:14px}\n"
            + ".stat{padding:13px 18px 14px;border-left:1px solid var(--sep);min-width:0}.stat:first-child{border-left:0}\n"
            + ".cap{font-size:12px;letter-spacing:.06em;text-transform:uppercase;color:var(--label);font-weight:600}\n"
            + ".big{font:600 28px/1.25 \"Segoe UI Variable Display\",\"Segoe UI\",sans-serif;margin:5px 0 2px;font-variant-numeric:tabular-nums;white-space:nowrap}.big small{font-size:17px}\n"
            + ".dim{color:var(--dim)}.faint{color:var(--faint);font-size:12.5px}.good{color:var(--good)}.bad{color:var(--bad)}.tie{color:var(--dim)}.warn{color:var(--warn);margin:0 0 8px}\n"
            + ".c{color:var(--c);font-weight:600}.w{color:var(--w);font-weight:600}\n"
            + ".scroll{overflow-x:auto}table{width:100%;border-collapse:collapse}\n"
            + "th{font-size:11.5px;letter-spacing:.04em;text-transform:uppercase;color:var(--label);text-align:left;font-weight:600;padding:6px 10px;border-bottom:1px solid var(--sep);white-space:nowrap}\n"
            + "td{padding:9px 10px;border-bottom:1px solid var(--sep);vertical-align:top}tr.first td{border-top:1px solid #3A414B}tbody tr:first-child td{border-top:0}\n"
            + ".r{text-align:right;font-variant-numeric:tabular-nums;white-space:nowrap}td.route{white-space:nowrap}\n"
            + ".bars{width:120px;margin:5px 0 0 auto}.bars i{display:block;height:5px;border-radius:3px;margin-top:3px}.bc{background:var(--c)}.bw{background:#7A828E}\n"
            + ".badge{display:inline-block;padding:3px 11px;border-radius:999px;font-size:12.5px;font-weight:600;white-space:nowrap}\n"
            + ".b-good{background:rgba(76,195,138,.14);color:var(--good);border:1px solid rgba(76,195,138,.45)}\n"
            + ".b-bad{background:rgba(240,122,114,.13);color:var(--bad);border:1px solid rgba(240,122,114,.45)}\n"
            + ".b-tie,.b-err{background:rgba(154,162,173,.1);color:var(--dim);border:1px solid rgba(154,162,173,.35)}\n"
            + "dl{display:grid;grid-template-columns:max-content 1fr;gap:4px 18px;margin:0 0 14px}dt{color:var(--label)}dd{margin:0}\n"
            + "details summary{cursor:pointer;list-style:none}details summary::-webkit-details-marker{display:none}details summary h2::after{content:\"  ▸ show\";color:var(--faint);text-transform:none;letter-spacing:0}details[open] summary h2::after{content:\"  ▾\"}\n"
            + "ul{margin:0;padding-left:20px}li{margin:3px 0}footer{margin-top:16px;text-align:center}\n"
            + "@media (max-width:640px){.summary{font-size:19px}td,th{padding:7px 6px}.bars{width:70px}.totals{grid-template-columns:1fr 1fr}.stat:nth-child(3){border-left:0}.stat:nth-child(n+3){border-top:1px solid var(--sep)}.big{font-size:23px}}\n";
    }

    /// <summary>Numbers as the report shows them (the same on every PC, whatever its language settings).</summary>
    internal static class BenchText
    {
        static readonly CultureInfo C = CultureInfo.InvariantCulture;

        public static string Size(long bytes)
        {
            double v = bytes;
            if (v >= 1000L << 20) { v /= 1L << 30; return (v >= 10 ? v.ToString("0", C) : v.ToString("0.#", C)) + " GB"; }
            if (v >= 1000 << 10) { v /= 1L << 20; return v.ToString("0", C) + " MB"; }
            return (v / 1024).ToString("0", C) + " KB";
        }

        public static string Speed(double mbps)
        {
            if (mbps <= 0) return "-";
            if (mbps >= 1000) return (mbps / 1024).ToString("0.00", C) + " GB/s";
            return mbps.ToString(mbps >= 100 ? "0" : "0.0", C) + " MB/s";
        }

        /// <summary>"+17%" (Choppy took less time) / "−11%" (more) / "0%".</summary>
        public static string Saved(double saved)
        {
            double p = Math.Round(saved * 100);
            return p > 0 ? "+" + p.ToString("0", C) + "%" : p < 0 ? "−" + (-p).ToString("0", C) + "%" : "0%";
        }

        /// <summary>"good" when Choppy took less time, "bad" when more, "tie" when that rounds to 0%.</summary>
        public static string SavedClass(double saved)
        {
            double p = Math.Round(saved * 100);
            return p > 0 ? "good" : p < 0 ? "bad" : "tie";
        }

        /// <summary>"0.41 s", "5.90 s", "23.4 s"</summary>
        public static string Secs(double s)
        {
            return (s < 10 ? s.ToString("0.00", C) : s < 100 ? s.ToString("0.0", C) : s.ToString("0", C)) + " s";
        }

        /// <summary>"0.41 s", "12.3 s", "1 min 12 s", "1 h 5 min"</summary>
        public static string Span(double secs) { return secs < 59.95 ? Secs(secs) : Duration(secs); }

        /// <summary>Span with "−" before a time lost; "0 s" when it rounds to nothing.</summary>
        public static string SignedSpan(double secs)
        {
            if (Math.Round(secs, 2) == 0) return "0 s";
            return secs < 0 ? "−" + Span(-secs) : Span(secs);
        }

        public static string Duration(double secs)
        {
            var t = TimeSpan.FromSeconds(secs);
            if (t.TotalMinutes < 1) return (int)Math.Round(secs) + " s";
            if (t.TotalHours < 1) return (int)t.TotalMinutes + " min " + t.Seconds + " s";
            return (int)t.TotalHours + " h " + t.Minutes + " min";
        }

        /// <summary>Choppy's icon as a PNG data URI for the report (null when it can't be read).</summary>
        public static string IconDataUri()
        {
            try
            {
                var rm = new System.Resources.ResourceManager("Choppy.g", typeof(BenchText).Assembly);
                using (var st = rm.GetStream("choppy.ico"))
                {
                    if (st == null) return null;
                    var dec = new System.Windows.Media.Imaging.IconBitmapDecoder(st, System.Windows.Media.Imaging.BitmapCreateOptions.None, System.Windows.Media.Imaging.BitmapCacheOption.OnLoad);
                    System.Windows.Media.Imaging.BitmapFrame best = null;
                    foreach (var f in dec.Frames) if (f.PixelWidth >= 104 && (best == null || f.PixelWidth < best.PixelWidth)) best = f;
                    if (best == null) best = dec.Frames.OrderByDescending(f => f.PixelWidth).First();
                    var enc = new System.Windows.Media.Imaging.PngBitmapEncoder();
                    enc.Frames.Add(best);
                    using (var ms = new MemoryStream()) { enc.Save(ms); return "data:image/png;base64," + Convert.ToBase64String(ms.ToArray()); }
                }
            }
            catch { return null; }
        }
    }
}
