using System;
using System.Collections.Generic;
using System.IO;
using System.Text;
using Microsoft.Win32;

namespace Choppy.Engine
{
    internal static class AppPaths
    {
        /// <summary>Test mode points everything at a scratch folder (CHOPPY_DATA).</summary>
        public static string DataDir
        {
            get
            {
                string over = Environment.GetEnvironmentVariable("CHOPPY_DATA");
                if (!string.IsNullOrEmpty(over)) return over;
                return Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Choppy");
            }
        }
        public static string JobDir { get { return Path.Combine(DataDir, "Jobs"); } }
        public static string LogDir { get { return Path.Combine(DataDir, "Logs"); } }
        public static bool TestMode { get { return !string.IsNullOrEmpty(Environment.GetEnvironmentVariable("CHOPPY_DATA")); } }
    }

    /// <summary>User settings in HKCU\Software\Choppy (or a settings.ini in the data folder in test mode).</summary>
    internal sealed class Settings
    {
        public bool VerifyCopies;                       // copies: check with xxHash64 (moves always check)
        public ConflictPolicy Policy = ConflictPolicy.Ask;
        public double SpeedLimitMbps;                   // 0 = off
        public double LimitValueMbps = 100;             // the limit's value, kept while it is off
        public bool CloseWhenDone = true;               // close the window a few seconds after a clean finish
        public bool ExplorerTakeover;                   // Ctrl+V in Explorer + right-click "Paste with Choppy"
        public string SpeedUnit = "auto";               // auto | mbps | mbs
        public string UltracopierRunBackup;             // Ultracopier's startup entry, put back when takeover is turned off
        public bool Extended;                           // copy window: extended view (else compact)

        static Settings current;
        public static Settings Current { get { if (current == null) current = Load(); return current; } }

        const string Key = @"Software\Choppy";

        public static Settings Load()
        {
            var s = new Settings();
            try
            {
                var d = Read();
                string v;
                if (d.TryGetValue("VerifyCopies", out v)) s.VerifyCopies = v == "1";
                if (d.TryGetValue("Policy", out v)) { ConflictPolicy p; if (Enum.TryParse(v, out p)) s.Policy = p; }
                if (d.TryGetValue("SpeedLimitMbps", out v)) { double x; if (double.TryParse(v, System.Globalization.NumberStyles.Float, System.Globalization.CultureInfo.InvariantCulture, out x)) s.SpeedLimitMbps = x; }
                if (d.TryGetValue("LimitValueMbps", out v)) { double x; if (double.TryParse(v, System.Globalization.NumberStyles.Float, System.Globalization.CultureInfo.InvariantCulture, out x) && x > 0) s.LimitValueMbps = x; }
                if (s.SpeedLimitMbps > 0) s.LimitValueMbps = s.SpeedLimitMbps;
                if (d.TryGetValue("CloseWhenDone", out v)) s.CloseWhenDone = v != "0";
                if (d.TryGetValue("ExplorerTakeover", out v)) s.ExplorerTakeover = v == "1";
                if (d.TryGetValue("SpeedUnit", out v)) s.SpeedUnit = v;
                if (d.TryGetValue("UltracopierRunBackup", out v)) s.UltracopierRunBackup = v;
                if (d.TryGetValue("View", out v)) s.Extended = v == "extended";
            }
            catch { }
            return s;
        }

        public void Save()
        {
            var d = new Dictionary<string, string>
            {
                { "VerifyCopies", VerifyCopies ? "1" : "0" },
                { "Policy", Policy.ToString() },
                { "SpeedLimitMbps", SpeedLimitMbps.ToString(System.Globalization.CultureInfo.InvariantCulture) },
                { "LimitValueMbps", LimitValueMbps.ToString(System.Globalization.CultureInfo.InvariantCulture) },
                { "CloseWhenDone", CloseWhenDone ? "1" : "0" },
                { "ExplorerTakeover", ExplorerTakeover ? "1" : "0" },
                { "SpeedUnit", SpeedUnit ?? "auto" },
                { "UltracopierRunBackup", UltracopierRunBackup ?? "" },
                { "View", Extended ? "extended" : "compact" },
            };
            try { Write(d); } catch { }
        }

        static Dictionary<string, string> Read()
        {
            var d = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
            if (AppPaths.TestMode)
            {
                string f = Path.Combine(AppPaths.DataDir, "settings.ini");
                if (File.Exists(f))
                    foreach (var line in File.ReadAllLines(f))
                    {
                        int i = line.IndexOf('=');
                        if (i > 0) d[line.Substring(0, i)] = line.Substring(i + 1);
                    }
                return d;
            }
            using (var k = Registry.CurrentUser.OpenSubKey(Key))
            {
                if (k == null) return d;
                foreach (var n in k.GetValueNames()) d[n] = Convert.ToString(k.GetValue(n));
            }
            return d;
        }

        static void Write(Dictionary<string, string> d)
        {
            if (AppPaths.TestMode)
            {
                Directory.CreateDirectory(AppPaths.DataDir);
                var sb = new StringBuilder();
                foreach (var kv in d) sb.AppendLine(kv.Key + "=" + kv.Value);
                File.WriteAllText(Path.Combine(AppPaths.DataDir, "settings.ini"), sb.ToString());
                return;
            }
            using (var k = Registry.CurrentUser.CreateSubKey(Key))
                foreach (var kv in d) k.SetValue(kv.Key, kv.Value);
        }
    }

    /// <summary>
    /// Crash-safe job record: written when a job is queued, removed when it ends (done or canceled).
    /// A record still there at the next start = the PC crashed or lost power mid-copy -> offer to resume.
    /// </summary>
    internal static class Journal
    {
        public static void Save(JobSpec s)
        {
            if (s.Elevated) return;   // an administrator copy runs from its own record ("<id>.admin"), see Handoff
            try
            {
                Directory.CreateDirectory(AppPaths.JobDir);
                Write(s, Path.Combine(AppPaths.JobDir, s.Id + ".job"), null);
            }
            catch { }
        }

        /// <summary>Writes a job record (flushed to the disk, then renamed into place).</summary>
        public static void Write(JobSpec s, string path, IEnumerable<string> extra)
        {
            var sb = new StringBuilder();
            sb.AppendLine("op=" + s.Op);
            sb.AppendLine("dest=" + s.Dest);
            sb.AppendLine("verify=" + (s.Verify ? "1" : "0"));
            sb.AppendLine("policy=" + s.Policy);
            sb.AppendLine("limit=" + s.SpeedLimitMbps.ToString(System.Globalization.CultureInfo.InvariantCulture));
            sb.AppendLine("origin=" + s.Origin);
            foreach (var src in s.Sources) sb.AppendLine("src=" + src);
            if (s.ExplicitItems != null)
                foreach (var f in s.ExplicitItems) sb.AppendLine("file=" + f.Src + "\t" + f.Rel);
            if (s.ExplicitDirs != null)
                foreach (var d in s.ExplicitDirs) sb.AppendLine("dir=" + d.Src + "\t" + d.Rel);
            if (extra != null) foreach (var e in extra) sb.AppendLine(e);
            string tmp = path + ".tmp";
            using (var fs = new FileStream(tmp, FileMode.Create, FileAccess.Write, FileShare.None))
            {
                var b = Encoding.UTF8.GetBytes(sb.ToString());
                fs.Write(b, 0, b.Length);
                fs.Flush(true);
            }
            if (File.Exists(path)) File.Delete(path);
            File.Move(tmp, path);
        }

        /// <summary>Reads a job record; null when it holds nothing to do.</summary>
        public static JobSpec Read(string f)
        {
            var s = new JobSpec { Id = Path.GetFileNameWithoutExtension(f) };
            foreach (var line in File.ReadAllLines(f, Encoding.UTF8))
            {
                int i = line.IndexOf('=');
                if (i <= 0) continue;
                string k = line.Substring(0, i), v = line.Substring(i + 1);
                switch (k)
                {
                    case "op": s.Op = v == "Move" ? OpKind.Move : OpKind.Copy; break;
                    case "dest": s.Dest = Paths.ToLong(v); break;   // repairs C:" from older records
                    case "verify": s.Verify = v == "1"; break;
                    case "policy": { ConflictPolicy p; if (Enum.TryParse(v, out p)) s.Policy = p; } break;
                    case "limit": { double x; if (double.TryParse(v, System.Globalization.NumberStyles.Float, System.Globalization.CultureInfo.InvariantCulture, out x)) s.SpeedLimitMbps = x; } break;
                    case "origin": s.Origin = v; break;
                    case "src": s.Sources.Add(Paths.ToLong(v)); break;
                    case "file":
                    case "dir":
                        {
                            int t = v.IndexOf('\t');
                            if (t <= 0) break;
                            if (k == "file")
                            {
                                if (s.ExplicitItems == null) s.ExplicitItems = new List<FileItem>();
                                s.ExplicitItems.Add(new FileItem { Src = v.Substring(0, t), Rel = v.Substring(t + 1) });
                            }
                            else
                            {
                                if (s.ExplicitDirs == null) s.ExplicitDirs = new List<DirItem>();
                                string rel = v.Substring(t + 1);
                                s.ExplicitDirs.Add(new DirItem { Src = v.Substring(0, t), Rel = rel, Depth = rel.Split('\\').Length - 1 });
                            }
                        }
                        break;
                }
            }
            return !string.IsNullOrEmpty(s.Dest) && (s.Sources.Count > 0 || s.ExplicitItems != null) ? s : null;
        }

        public static void Remove(JobSpec s)
        {
            try { File.Delete(Path.Combine(AppPaths.JobDir, s.Id + ".job")); } catch { }
        }

        public static List<JobSpec> LoadUnfinished()
        {
            var r = new List<JobSpec>();
            try
            {
                if (!Directory.Exists(AppPaths.JobDir)) return r;
                foreach (var f in Directory.GetFiles(AppPaths.JobDir, "*.job"))
                {
                    try
                    {
                        var s = Read(f);
                        if (s != null) { s.ResumeMode = true; r.Add(s); }
                        else File.Delete(f);
                    }
                    catch { }
                }
                // administrator copies whose Choppy is gone: the user's own record (above) resumes them
                Handoff.CleanStale();
            }
            catch { }
            return r;
        }
    }
}
