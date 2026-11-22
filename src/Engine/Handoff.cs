using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Security.Principal;
using System.Text;
using System.Threading;

namespace Choppy.Engine
{
    /// <summary>
    /// The rest of a copy into a folder only administrators may write to (C:\, Program Files...) goes to a short-lived
    /// Choppy started as administrator. The user's own Choppy writes a job record "&lt;id&gt;.admin"; next to it the
    /// administrator copy keeps "&lt;record&gt;.progress" up to date, reads "&lt;record&gt;.control" (pause / run / cancel)
    /// and leaves "&lt;record&gt;.result" when it ends. Plain text, like the other job records.
    /// </summary>
    internal static class Handoff
    {
        public const string Arg = "--elevated-job";

        public static bool IsAdmin
        {
            get
            {
                try { return new WindowsPrincipal(WindowsIdentity.GetCurrent()).IsInRole(WindowsBuiltInRole.Administrator); }
                catch { return false; }
            }
        }

        /// <summary>
        /// Administrator copy, very first thing: an elevated start gets a fresh environment, so test mode
        /// (scratch data folder, test hook, windows off-screen) comes along in the record instead.
        /// </summary>
        public static void Prepare(string[] args)
        {
            if (args.Length < 2 || args[0] != Arg) return;
            if (!Genuine(args[1], args.Length > 2 ? args[2] : null)) return;
            try
            {
                foreach (var line in File.ReadAllLines(args[1], Encoding.UTF8))
                {
                    int i = line.IndexOf('=');
                    if (i <= 0) continue;
                    string k = line.Substring(0, i), v = line.Substring(i + 1);
                    if (k == "testdata") Environment.SetEnvironmentVariable("CHOPPY_DATA", v);
                    else if (k == "testhook") Environment.SetEnvironmentVariable("CHOPPY_TESTHOOK", v);
                    else if (k == "testpark") Environment.SetEnvironmentVariable("CHOPPY_PARK", v);
                }
            }
            catch { }
        }

        // ================= the user's side =================

        public static string Save(JobSpec s)
        {
            Directory.CreateDirectory(AppPaths.JobDir);
            string path = Path.Combine(AppPaths.JobDir, s.Id + ".admin");
            var extra = new List<string> { "parent=" + Process.GetCurrentProcess().Id };
            if (AppPaths.TestMode)
            {
                extra.Add("testdata=" + AppPaths.DataDir);
                string hook = Environment.GetEnvironmentVariable("CHOPPY_TESTHOOK");
                if (!string.IsNullOrEmpty(hook)) extra.Add("testhook=" + Path.Combine(hook, "admin"));
                string park = Environment.GetEnvironmentVariable("CHOPPY_PARK");
                if (!string.IsNullOrEmpty(park)) extra.Add("testpark=" + park);
            }
            Journal.Write(s, path, extra);
            Clean(path, false);
            return path;
        }

        /// <summary>
        /// The record's SHA-256. It goes on the administrator copy's command line: the record sits in the user's own
        /// folder, so anything running as the user could change it - the administrator copy runs only the record
        /// the user's Choppy wrote.
        /// </summary>
        static string Fingerprint(string record)
        {
            using (var sha = System.Security.Cryptography.SHA256.Create())
            using (var f = File.OpenRead(record))
                return BitConverter.ToString(sha.ComputeHash(f)).Replace("-", "").ToLowerInvariant();
        }

        static bool Genuine(string record, string fingerprint)
        {
            try { return !string.IsNullOrEmpty(fingerprint) && File.Exists(record) && Fingerprint(record) == fingerprint.Trim().ToLowerInvariant(); }
            catch { return false; }
        }

        /// <summary>Starts Choppy as administrator on the record. Throws Win32Exception 1223 when the user says no.</summary>
        public static Process Launch(string record)
        {
            string exe = Process.GetCurrentProcess().MainModule.FileName;
            var psi = new ProcessStartInfo(exe, Arg + " \"" + record + "\" " + Fingerprint(record))
            {
                UseShellExecute = true,
                Verb = "runas",
                WorkingDirectory = Path.GetDirectoryName(exe)
            };
            return Process.Start(psi);
        }

        public static bool Declined(Exception ex)
        {
            var w = ex as Win32Exception;
            return w != null && w.NativeErrorCode == 1223;   // ERROR_CANCELLED
        }

        public static void Control(string record, int seq, string what)
        {
            WriteAtomic(record + ".control", seq + " " + what);
        }

        public static Dictionary<string, string> ReadProgress(string record)
        {
            return ReadMap(record + ".progress");
        }

        public sealed class Result
        {
            public string Phase, Status, Log;
            public long TotalBytes, DoneBytes, CopiedBytes, SkippedBytes, FailedBytes;
            public int TotalFiles, DoneFiles, Skipped, Failed, Verified, Already;
            public readonly List<FileItem> FailedItems = new List<FileItem>();
            public readonly List<FileItem> SkippedItems = new List<FileItem>();
            public readonly List<string> Notes = new List<string>();
        }

        public static Result ReadResult(string record)
        {
            string f = record + ".result";
            if (!File.Exists(f)) return null;
            string[] lines;
            try { lines = File.ReadAllLines(f, Encoding.UTF8); } catch { return null; }
            if (lines.Length == 0 || lines[lines.Length - 1] != "end") return null;   // still being written
            var r = new Result();
            foreach (var line in lines)
            {
                int i = line.IndexOf('=');
                if (i <= 0) continue;
                string k = line.Substring(0, i), v = line.Substring(i + 1);
                switch (k)
                {
                    case "phase": r.Phase = v; break;
                    case "status": r.Status = v; break;
                    case "log": r.Log = v; break;
                    case "totalBytes": r.TotalBytes = L(v); break;
                    case "doneBytes": r.DoneBytes = L(v); break;
                    case "copiedBytes": r.CopiedBytes = L(v); break;
                    case "skippedBytes": r.SkippedBytes = L(v); break;
                    case "failedBytes": r.FailedBytes = L(v); break;
                    case "totalFiles": r.TotalFiles = (int)L(v); break;
                    case "doneFiles": r.DoneFiles = (int)L(v); break;
                    case "skipped": r.Skipped = (int)L(v); break;
                    case "failed": r.Failed = (int)L(v); break;
                    case "verified": r.Verified = (int)L(v); break;
                    case "already": r.Already = (int)L(v); break;
                    case "note": r.Notes.Add(v); break;
                    case "failedItem":
                    case "skippedItem":
                        {
                            var p = v.Split('\t');
                            if (p.Length < 3) break;
                            var fi = new FileItem { Src = p[0], Rel = p[1], Size = L(p[2]), Error = p.Length > 3 ? p[3] : null };
                            if (k == "failedItem") { fi.St = FileState.Failed; r.FailedItems.Add(fi); }
                            else { fi.St = FileState.Skipped; r.SkippedItems.Add(fi); }
                        }
                        break;
                }
            }
            return r;
        }

        public static void Clean(string record, bool withRecord)
        {
            foreach (var ext in new[] { ".progress", ".result", ".control", ".progress.tmp", ".result.tmp", ".control.tmp" })
                try { File.Delete(record + ext); } catch { }
            if (withRecord) try { File.Delete(record); } catch { }
        }

        public static void CleanStale()
        {
            try
            {
                foreach (var f in Directory.GetFiles(AppPaths.JobDir, "*.admin"))
                {
                    int parent = 0;
                    try
                    {
                        foreach (var line in File.ReadAllLines(f, Encoding.UTF8))
                            if (line.StartsWith("parent=")) int.TryParse(line.Substring(7), out parent);
                    }
                    catch { }
                    if (!Alive(parent)) Clean(f, true);
                }
            }
            catch { }
        }

        public static bool Alive(int pid)
        {
            if (pid <= 0) return false;
            try { using (var p = Process.GetProcessById(pid)) return !p.HasExited; }
            catch { return false; }
        }

        // ================= the administrator's side =================

        /// <summary>The record, if it is the one the user's Choppy wrote (see Fingerprint); else null.</summary>
        public static JobSpec Load(string record, string fingerprint)
        {
            try
            {
                if (!Genuine(record, fingerprint)) return null;
                var s = Journal.Read(record);
                if (s == null) return null;
                s.Elevated = true;
                s.HandoffPath = record;
                return s;
            }
            catch { return null; }
        }

        /// <summary>Keeps the progress file current and follows the control file until the copy ends.</summary>
        public static void Report(CopyJob job)
        {
            string record = job.Spec.HandoffPath;
            if (record == null) return;
            job.Finished += WriteResult;
            var t = new Thread(() =>
            {
                int seen = 0;
                while (!job.IsFinished)
                {
                    try
                    {
                        WriteProgress(job);
                        string c = ReadSmall(record + ".control");
                        if (c != null)
                        {
                            var p = c.Split(new[] { ' ' }, 2);
                            int seq;
                            if (p.Length == 2 && int.TryParse(p[0], out seq) && seq > seen)
                            {
                                seen = seq;
                                if (p[1] == "cancel") job.Cancel();
                                else if (p[1] == "pause") job.Pause();
                                else if (p[1] == "run") job.Resume();
                            }
                        }
                    }
                    catch { }
                    Thread.Sleep(250);
                }
            }) { IsBackground = true, Name = "handoff report" };
            t.Start();
        }

        static void WriteProgress(CopyJob j)
        {
            var sb = new StringBuilder();
            sb.AppendLine("pid=" + Process.GetCurrentProcess().Id);
            sb.AppendLine("phase=" + j.Phase);
            sb.AppendLine("paused=" + (j.IsPaused ? "1" : "0"));
            sb.AppendLine("doneBytes=" + Interlocked.Read(ref j.DoneBytes));
            sb.AppendLine("totalBytes=" + j.TotalBytes);
            sb.AppendLine("doneFiles=" + j.DoneFiles);
            sb.AppendLine("totalFiles=" + j.TotalFiles);
            var t = j.CurrentTransfer();
            if (t != null)
            {
                sb.AppendLine("cur=" + t.File.Rel);
                sb.AppendLine("curSrc=" + t.File.Src);
                sb.AppendLine("curDone=" + (t.Verifying ? t.VerifyDone : t.Done));
                sb.AppendLine("curSize=" + (t.Verifying ? t.File.Size * 2 : t.File.Size));
                sb.AppendLine("verifying=" + (t.Verifying ? "1" : "0"));
            }
            if (j.Status != null) sb.AppendLine("status=" + OneLine(j.Status));
            WriteAtomic(j.Spec.HandoffPath + ".progress", sb.ToString());
        }

        static void WriteResult(CopyJob j)
        {
            try
            {
                WriteProgress(j);
                var sb = new StringBuilder();
                sb.AppendLine("phase=" + j.Phase);
                if (j.Status != null) sb.AppendLine("status=" + OneLine(j.Status));
                if (j.LogPath != null) sb.AppendLine("log=" + j.LogPath);
                sb.AppendLine("totalBytes=" + j.TotalBytes);
                sb.AppendLine("doneBytes=" + Interlocked.Read(ref j.DoneBytes));
                sb.AppendLine("copiedBytes=" + j.CopiedBytes);
                sb.AppendLine("skippedBytes=" + j.SkippedBytes);
                sb.AppendLine("failedBytes=" + j.FailedBytes);
                sb.AppendLine("totalFiles=" + j.TotalFiles);
                sb.AppendLine("doneFiles=" + j.DoneFiles);
                sb.AppendLine("skipped=" + j.SkippedFiles);
                sb.AppendLine("failed=" + j.FailedFiles);
                sb.AppendLine("verified=" + j.VerifiedFiles);
                sb.AppendLine("already=" + j.AlreadyThereFiles);
                lock (j.Notes) foreach (var n in j.Notes) sb.AppendLine("note=" + OneLine(n));
                lock (j.FailedItems) foreach (var f in j.FailedItems) sb.AppendLine("failedItem=" + f.Src + "\t" + (f.Rel ?? Paths.FileName(f.Src)) + "\t" + f.Size + "\t" + OneLine(f.Error));
                lock (j.SkippedItems) foreach (var f in j.SkippedItems) sb.AppendLine("skippedItem=" + f.Src + "\t" + (f.Rel ?? Paths.FileName(f.Src)) + "\t" + f.Size);
                sb.AppendLine("end");
                WriteAtomic(j.Spec.HandoffPath + ".result", sb.ToString());
            }
            catch { }
        }

        // ================= files =================

        static long L(string v)
        {
            long x;
            return long.TryParse(v, NumberStyles.Integer, CultureInfo.InvariantCulture, out x) ? x : 0;
        }

        static string OneLine(string s)
        {
            return (s ?? "").Replace("\r", " ").Replace("\n", " ").Replace("\t", " ");
        }

        static void WriteAtomic(string path, string text)
        {
            string tmp = path + ".tmp";
            File.WriteAllText(tmp, text, Encoding.UTF8);
            if (File.Exists(path)) File.Replace(tmp, path, null);
            else File.Move(tmp, path);
        }

        static string ReadSmall(string path)
        {
            try { return File.Exists(path) ? File.ReadAllText(path, Encoding.UTF8).Trim() : null; } catch { return null; }
        }

        static Dictionary<string, string> ReadMap(string path)
        {
            string text = ReadSmall(path);
            if (text == null) return null;
            var d = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
            foreach (var line in text.Split('\n'))
            {
                int i = line.IndexOf('=');
                if (i > 0) d[line.Substring(0, i)] = line.Substring(i + 1).TrimEnd('\r');
            }
            return d;
        }
    }
}
