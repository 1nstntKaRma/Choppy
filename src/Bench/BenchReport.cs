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
    }

    internal sealed class BenchResult
    {
        public BenchDrive From, To;
        public BenchCase Case;
        public readonly List<BenchRun> Runs = new List<BenchRun>();
        public string Method;    // the way Choppy's Adaptive mode picked
        public string Error;
        public double ChoppyMBps, WindowsMBps, ChoppyDoneMBps, WindowsDoneMBps;
        public double Ratio, DoneRatio, Noise, DoneNoise;
        public int Verdict, DoneVerdict;  // +1 Choppy faster, 0 tie, -1 Choppy slower
        public bool Scored;

        public bool Within { get { return From == To; } }
        public string Route { get { return From.Letter + " → " + To.Letter; } }
        public string RouteKinds { get { return Within ? "within one " + From.Kind : From.Short + " → " + To.Short; } }

        List<double> Times(bool choppy, bool disk) { return Runs.Where(r => r.Choppy == choppy).Select(r => disk ? r.OnDisk : r.Done).ToList(); }

        public void Score()
        {
            Verdict = Benchmark.Judge(Times(true, true), Times(false, true), out Ratio, out Noise);
            DoneVerdict = Benchmark.Judge(Times(true, false), Times(false, false), out DoneRatio, out DoneNoise);
            ChoppyMBps = Speed(Times(true, true));
            WindowsMBps = Speed(Times(false, true));
            ChoppyDoneMBps = Speed(Times(true, false));
            WindowsDoneMBps = Speed(Times(false, false));
            Scored = Error == null && ChoppyMBps > 0 && WindowsMBps > 0;
        }

        double Speed(List<double> secs)
        {
            double m = Benchmark.Median(secs);
            return m > 0 ? Case.Bytes / (double)Benchmark.MB / m : 0;
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
        public readonly List<string> Notes = new List<string>();
        public bool Canceled;
        public string Error;

        public int Faster { get { return Results.Count(r => r.Scored && r.Verdict > 0); } }
        public int Ties { get { return Results.Count(r => r.Scored && r.Verdict == 0); } }
        public int Slower { get { return Results.Count(r => r.Scored && r.Verdict < 0); } }
        public int Scored { get { return Results.Count(r => r.Scored); } }

        /// <summary>"Choppy faster on 9 of 12 · tie on 2 · slower on 1"</summary>
        public string Summary { get { return SummaryOf(Faster, Ties, Slower); } }

        public string DoneSummary
        {
            get
            {
                int f = Results.Count(r => r.Scored && r.DoneVerdict > 0), t = Results.Count(r => r.Scored && r.DoneVerdict == 0), s = Results.Count(r => r.Scored && r.DoneVerdict < 0);
                return SummaryOf(f, t, s);
            }
        }

        public static string SummaryOf(int faster, int ties, int slower)
        {
            int n = faster + ties + slower;
            if (n == 0) return "No case was measured";
            if (faster == n) return "Choppy faster on all " + n;
            var s = "Choppy faster on " + faster + " of " + n;
            if (ties > 0) s += " · tie on " + ties;
            if (slower > 0) s += " · slower on " + slower;
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

        static string Verdict(int v) { return v > 0 ? "faster" : v < 0 ? "slower" : "tie"; }

        public string ToJson()
        {
            var j = new StringBuilder();
            j.Append("{\n");
            j.Append("  \"tool\": \"Choppy benchmark\",\n");
            j.Append("  \"version\": " + S(Version) + ",\n");
            j.Append("  \"date\": " + S(Started.ToString("yyyy-MM-ddTHH:mm:ss", CultureInfo.InvariantCulture)) + ",\n");
            j.Append("  \"mode\": " + S(Mode) + ", \"rounds\": " + Rounds + ", \"seconds\": " + N(Seconds, "0.0") + ",\n");
            j.Append("  \"canceled\": " + (Canceled ? "true" : "false") + ", \"error\": " + S(Error) + ",\n");
            j.Append("  \"units\": \"MB/s = 1,048,576 bytes per second; seconds\",\n");
            j.Append("  \"machine\": { \"cpu\": " + S(Cpu) + ", \"threads\": " + Threads + ", \"ramGB\": " + N(RamGB) + ", \"windows\": " + S(Windows) + ", \"admin\": " + (Admin ? "true" : "false") + " },\n");
            j.Append("  \"summary\": { \"text\": " + S(Summary) + ", \"cases\": " + Scored + ", \"faster\": " + Faster + ", \"tie\": " + Ties + ", \"slower\": " + Slower
                + ", \"byDone\": " + S(DoneSummary) + " },\n");
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
                    + ", \"cacheDrop\": " + S(d.PurgeNote) + " }");
            }
            j.Append("\n  ],\n  \"results\": [");
            for (int i = 0; i < Results.Count; i++)
            {
                var r = Results[i];
                j.Append(i == 0 ? "\n" : ",\n");
                j.Append("    { \"from\": " + S(r.From.Letter) + ", \"to\": " + S(r.To.Letter) + ", \"route\": " + S(r.From.Letter + " -> " + r.To.Letter)
                    + ", \"kinds\": " + S(r.RouteKinds.Replace("→", "->")) + ", \"case\": " + S(r.Case.Key) + ", \"title\": " + S(r.Case.Title)
                    + ", \"files\": " + r.Case.Files + ", \"bytes\": " + r.Case.Bytes + ",\n");
                j.Append("      \"verdict\": " + (r.Scored ? S(Verdict(r.Verdict)) : "null") + ", \"ratio\": " + N(r.Ratio) + ", \"tieWithin\": " + N(r.Noise)
                    + ", \"doneVerdict\": " + (r.Scored ? S(Verdict(r.DoneVerdict)) : "null") + ", \"doneRatio\": " + N(r.DoneRatio) + ",\n");
                j.Append("      \"choppy\": { \"MBps\": " + N(r.ChoppyMBps, "0.0") + ", \"doneMBps\": " + N(r.ChoppyDoneMBps, "0.0") + ", \"method\": " + S(r.Method) + ", \"runs\": " + Runs(r, true) + " },\n");
                j.Append("      \"windows\": { \"MBps\": " + N(r.WindowsMBps, "0.0") + ", \"doneMBps\": " + N(r.WindowsDoneMBps, "0.0") + ", \"runs\": " + Runs(r, false) + " },\n");
                j.Append("      \"error\": " + S(r.Error) + " }");
            }
            j.Append("\n  ],\n  \"notes\": [");
            for (int i = 0; i < Notes.Count; i++) j.Append((i == 0 ? "" : ", ") + S(Notes[i]));
            j.Append("]\n}\n");
            return j.ToString();
        }

        static string Runs(BenchResult r, bool choppy)
        {
            return "[" + string.Join(", ", r.Runs.Where(x => x.Choppy == choppy).Select(x => "{ \"done\": " + N(x.Done, "0.000") + ", \"onDisk\": " + N(x.OnDisk, "0.000") + " }")) + "]";
        }

        // ================= text (clipboard) =================

        /// <summary>A short Markdown table: reads as plain text and shows as a table on GitHub and in chats.</summary>
        public string ToText()
        {
            var t = new StringBuilder();
            t.AppendLine("**Choppy v" + Version + " benchmark** - " + Started.ToString("d MMM yyyy, HH:mm", CultureInfo.InvariantCulture) + " - " + Cap(Mode) + (Canceled ? " (stopped early)" : ""));
            t.AppendLine(Machine);
            t.AppendLine();
            t.AppendLine("**" + Summary + "** (time until the copies are on the disk; by the time each says it's done: " + Lower(DoneSummary) + ")");
            t.AppendLine();
            t.AppendLine("| Route | What | Choppy | Windows | Choppy / Windows | Result |");
            t.AppendLine("|---|---|---:|---:|---:|---|");
            foreach (var r in Results)
            {
                string route = r.Route + " (" + r.RouteKinds + ")";
                if (!r.Scored) { t.AppendLine("| " + route + " | " + r.Case.Title + ", " + r.Case.Detail + " | | | | error: " + (r.Error ?? "not measured") + " |"); continue; }
                t.AppendLine("| " + route + " | " + r.Case.Title + ", " + BenchText.Size(r.Case.Bytes) + " | " + BenchText.Speed(r.ChoppyMBps) + " | " + BenchText.Speed(r.WindowsMBps)
                    + " | " + BenchText.Ratio(r.Ratio) + " | " + Verdict(r.Verdict) + " |");
            }
            t.AppendLine();
            t.Append("Drives: ");
            t.AppendLine(string.Join("; ", (Drives ?? new List<BenchDrive>()).Where(d => d.Skip == null).Select(d => d.Letter + " " + d.Kind + (d.Model != null ? " " + d.Model : "") + " (" + d.FileSystem + ")")));
            return t.ToString();
        }

        static string Cap(string s) { return string.IsNullOrEmpty(s) ? s : char.ToUpperInvariant(s[0]) + s.Substring(1); }
        static string Lower(string s) { return s.Replace("Choppy faster", "faster"); }

        // ================= HTML =================

        static string H(string s) { return WebUtility.HtmlEncode(s ?? ""); }

        public string ToHtml(string jsonName)
        {
            var h = new StringBuilder();
            h.Append("<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
            h.Append("<title>Choppy benchmark · " + H(Started.ToString("d MMM yyyy, HH:mm", CultureInfo.InvariantCulture)) + "</title>\n<style>\n" + Css + "</style></head><body><div class=\"wrap\">\n");

            // header
            string icon = BenchText.IconDataUri();
            h.Append("<header>" + (icon != null ? "<img src=\"" + icon + "\" width=\"52\" height=\"52\" alt=\"\">" : "")
                + "<div><h1>Choppy benchmark</h1><div class=\"dim\">Choppy v" + H(Version) + " against Windows' own copy (the engine Explorer uses) · "
                + H(Started.ToString("d MMM yyyy, HH:mm", CultureInfo.InvariantCulture)) + " · " + H(Cap(Mode)) + " · took " + H(BenchText.Duration(Seconds)) + "</div></div></header>\n");

            // summary
            int n = Scored;
            h.Append("<section class=\"card\">");
            if (Canceled) h.Append("<div class=\"warn\">Stopped before the end: these are the cases that finished.</div>");
            if (Error != null) h.Append("<div class=\"warn\">" + H(Error) + "</div>");
            if (n > 0)
            {
                h.Append("<div class=\"summary\">");
                if (Faster == n) h.Append("Choppy <span class=\"good\">faster on all " + n + "</span>");
                else
                {
                    h.Append("Choppy <span class=\"good\">faster on " + Faster + "</span> of " + n);
                    if (Ties > 0) h.Append(" · <span class=\"tie\">tie on " + Ties + "</span>");
                    if (Slower > 0) h.Append(" · <span class=\"bad\">slower on " + Slower + "</span>");
                }
                h.Append("</div>");
                h.Append("<p class=\"dim\">Counted by the time until the copies are on the disk. A difference within " + (int)(Benchmark.TieMargin * 100)
                    + "%, or within how far each one's own runs differ, counts as a tie. By the time each one says it's done: " + H(Lower(DoneSummary)) + ".</p>");
            }
            else if (!Canceled && Error == null) h.Append("<div class=\"summary\">No case was measured</div>");
            h.Append("</section>\n");

            // results
            if (Results.Count > 0)
            {
                h.Append("<section class=\"card\"><h2>Results</h2><div class=\"scroll\"><table class=\"res\"><thead><tr><th>Route</th><th>What</th><th class=\"r\">Choppy</th><th class=\"r\">Windows</th><th class=\"r\">Choppy / Windows</th><th>Result</th></tr></thead><tbody>\n");
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
                        h.Append("<td class=\"r\"><span class=\"c\">" + H(BenchText.Speed(r.ChoppyMBps)) + "</span><div class=\"faint\">done " + H(BenchText.Speed(r.ChoppyDoneMBps)) + "</div></td>");
                        h.Append("<td class=\"r\"><span class=\"w\">" + H(BenchText.Speed(r.WindowsMBps)) + "</span><div class=\"faint\">done " + H(BenchText.Speed(r.WindowsDoneMBps)) + "</div></td>");
                        h.Append("<td class=\"r\"><b>" + H(BenchText.Ratio(r.Ratio)) + "</b><div class=\"bars\"><i class=\"bc\" style=\"width:" + Pct(r.ChoppyMBps, max) + "%\"></i><i class=\"bw\" style=\"width:" + Pct(r.WindowsMBps, max) + "%\"></i></div></td>");
                        string cls = r.Verdict > 0 ? "b-good" : r.Verdict < 0 ? "b-bad" : "b-tie";
                        string word = r.Verdict > 0 ? "Choppy faster" : r.Verdict < 0 ? "Choppy slower" : "Tie";
                        h.Append("<td><span class=\"badge " + cls + "\" title=\"tie within ±" + N(r.Noise * 100, "0.#") + "%\">" + word + "</span></td></tr>\n");
                    }
                }
                h.Append("</tbody></table></div><p class=\"faint\">MB/s of the time until the copies are on the disk (under it: until each one said it was done). "
                    + "1 MB = 1,048,576 bytes. Bars: <span class=\"c\">Choppy</span> and <span class=\"w\">Windows</span>.</p></section>\n");
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
                h.Append("<td>" + H(d.Vol != null || d.Network ? d.Kind : "") + "</td>");
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
                h.Append("<section class=\"card\"><details><summary><h2>Every run</h2></summary><div class=\"scroll\"><table class=\"runs\"><thead><tr><th>Route</th><th>What</th><th>Choppy: done / on the disk (s)</th><th>Windows: done / on the disk (s)</th><th>Choppy's way</th></tr></thead><tbody>\n");
                foreach (var r in Results)
                {
                    h.Append("<tr><td>" + H(r.Route) + "</td><td>" + H(r.Case.Title) + "</td><td>" + RunList(r, true) + "</td><td>" + RunList(r, false) + "</td><td class=\"faint\">" + H(r.Method ?? "") + "</td></tr>\n");
                }
                h.Append("</tbody></table></div><p class=\"faint\">In the order they ran: each case alternates (Choppy, Windows, Windows, Choppy - or the other way round).</p></details></section>\n");
            }

            // how
            h.Append("<section class=\"card\"><h2>How it's measured</h2><ul class=\"dim\">");
            h.Append("<li>Routes: from the system drive to each other drive and back, and within each drive.</li>");
            h.Append("<li>Each case is copied " + Rounds + " times by each, in turns, into a new folder. The source is never in memory: it is read once untimed (a new file's first open is slower), then dropped from the Windows cache before every copy.</li>");
            h.Append("<li>Sizes go by each route's speed: 1 GB and 1,000 small files where it does 350 MB/s or more, 512 MB and 500 files below that, 256 MB and 250 under 40 MB/s (Thorough: twice that, and a mixed folder).</li>");
            h.Append("<li>Choppy: its own engine, as every copy runs it (Adaptive picks the way per drive). Windows: the shell copy engine Explorer uses for a paste (IFileOperation), without its window.</li>");
            h.Append("<li>\"Done\" is when each one says so; \"on the disk\" adds the time to write out what was still in memory (every copy is flushed).</li>");
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
            return string.Join("<br>", r.Runs.Where(x => x.Choppy == choppy).Select(x => H(N(x.Done, "0.00")) + " / " + H(N(x.OnDisk, "0.00"))));
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
            + "@media (max-width:640px){.summary{font-size:19px}td,th{padding:7px 6px}.bars{width:70px}}\n";
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

        public static string Ratio(double r) { return r > 0 ? r.ToString("0.00", C) + "×" : "-"; }

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
