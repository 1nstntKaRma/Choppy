using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

namespace Choppy.Bench
{
    /// <summary>
    /// The move to Rust, step 3 (rust branch only, not ported): the Rust engine as Choppy's side of the Benchmark, in a
    /// child process - choppy.exe --benchcopy, as Windows' side runs (--shellcopy). This C# Choppy takes the same switch,
    /// so pointing the Benchmark's --rust at this exe checks the method itself (the C# engine in and out of process).
    ///
    /// How the time is counted, as ChoppyCopy counts the copy in the Benchmark's own process: the child copies the tiny
    /// warm-up file first, untimed (both drives looked at once); then its clock runs from right before the job starts
    /// until the job's Finished event. Starting a process is not counted (C# doesn't start one). The child also tells
    /// the moment of that event ("ended=", a FILETIME to the microsecond): the time from then until the parent has the
    /// answer is added to the time to the disk ("after"), and then the copies are flushed (256 threads) as for every side.
    ///
    /// Two ways: warm (--benchcopy -: one child for the whole run, a job file's path per line on its stdin, the result
    /// back in one line on its stdout; warm like the C# engine in this process, which is what the app is: one process
    /// that copies again and again) or fresh (--benchcopy job.txt: a new process per copy, the result in job.txt.result;
    /// its ending and the result file then count against it as well).
    /// </summary>
    internal static class EngineChild
    {
        [DllImport("kernel32.dll")]
        static extern void GetSystemTimePreciseAsFileTime(out long fileTime);

        /// <summary>The time now as a FILETIME, to the microsecond: a clock the parent and the child both read.</summary>
        public static long PreciseNow() { long t; GetSystemTimePreciseAsFileTime(out t); return t; }

        /// <summary>
        /// The child: --benchcopy job.txt (one copy, its result in job.txt.result) or --benchcopy - (job files' paths on
        /// stdin, each result in one line on stdout, its fields apart by tabs). A job file's lines: the warm-up file, a
        /// folder for it, the source, the destination folder, the engine's test switches (the Benchmark's --choppy, or
        /// empty). The result: "seconds=...", "ended=...", "method=..." or "error=...".
        /// </summary>
        public static int Child(string arg)
        {
            if (arg != "-")
            {
                string r = ResultOf(arg);
                try { File.WriteAllText(arg + ".result", r, Encoding.UTF8); } catch { return 1; }
                return 0;
            }
            var stdin = new StreamReader(Console.OpenStandardInput(), new UTF8Encoding(false));
            var stdout = new StreamWriter(Console.OpenStandardOutput(), new UTF8Encoding(false)) { AutoFlush = true };
            string line;
            while ((line = stdin.ReadLine()) != null)
            {
                if (line.Trim().Length == 0) continue;
                stdout.WriteLine(ResultOf(line.Trim()).Replace("\r\n", "\t"));
            }
            return 0;
        }

        static string ResultOf(string jobFile)
        {
            try
            {
                var lines = File.ReadAllLines(jobFile, Encoding.UTF8);
                var opts = (lines.Length > 4 ? lines[4] : "").Split(new[] { ' ' }, StringSplitOptions.RemoveEmptyEntries);
                Action<Engine.JobSpec> tweak = opts.Length == 0 ? null : (Action<Engine.JobSpec>)(sp => { for (int k = 0; k < opts.Length; k++) Cli.EngineOption(opts, ref k, sp); });
                string m;
                long ended;
                if (lines[0].Length > 0) Benchmark.EngineCopy(lines[0], lines[1], tweak, () => false, out m);
                double secs = Benchmark.EngineCopy(lines[2], lines[3], tweak, () => false, out m, out ended);
                return "seconds=" + secs.ToString("R", CultureInfo.InvariantCulture) + "\r\nended=" + ended.ToString(CultureInfo.InvariantCulture) + "\r\nmethod=" + (m ?? "");
            }
            catch (Exception ex) { return "error=" + ex.Message.Replace("\r", " ").Replace("\n", " "); }
        }

        sealed class Server { public Process P; public StreamWriter In; }
        static readonly Dictionary<string, Server> servers = new Dictionary<string, Server>(StringComparer.OrdinalIgnoreCase);

        /// <summary>Ends the warm children (their stdin closes; killed if they don't go within 10 s).</summary>
        public static void StopAll()
        {
            lock (servers)
            {
                foreach (var s in servers.Values) Stop(s);
                servers.Clear();
            }
        }

        static void Stop(Server s)
        {
            try { s.In.Dispose(); } catch { }
            try { if (!s.P.WaitForExit(10000)) s.P.Kill(); } catch { }
            try { s.P.Dispose(); } catch { }
        }

        /// <summary>
        /// One timed copy by a Choppy in a child process (warm: the one kept for this exe; else a new one): its seconds,
        /// and "after" = the seconds from the moment it said done until the answer was here (0 = not told). Throws on an
        /// error or a cancel.
        /// </summary>
        public static double Run(string exe, bool warm, string warmSrc, string warmDir, string src, string destDir, string engineArgs, Func<bool> canceled, out string method, out double after)
        {
            method = null;
            after = 0;
            string job = Path.Combine(Path.GetTempPath(), "choppy-benchcopy-" + Guid.NewGuid().ToString("N").Substring(0, 8) + ".txt");
            File.WriteAllLines(job, new[] { warmSrc ?? "", warmDir ?? "", src, destDir, engineArgs ?? "" }, Encoding.UTF8);
            try
            {
                string res;
                long here;
                if (warm)
                {
                    res = Ask(exe, job, canceled).Replace("\t", "\r\n");
                    here = PreciseNow();
                }
                else
                {
                    var psi = new ProcessStartInfo(exe, "--benchcopy \"" + job + "\"") { UseShellExecute = false, CreateNoWindow = true };
                    using (var p = Process.Start(psi))
                    {
                        while (!p.WaitForExit(50))
                        {
                            if (canceled())
                            {
                                try { p.Kill(); } catch { }
                                p.WaitForExit(10000);
                                throw new OperationCanceledException();
                            }
                        }
                    }
                    res = File.Exists(job + ".result") ? File.ReadAllText(job + ".result", Encoding.UTF8) : "error=the copy ended without a result";
                    here = PreciseNow();
                }
                double secs = -1;
                long ended = 0;
                foreach (var line in res.Split(new[] { '\r', '\n' }, StringSplitOptions.RemoveEmptyEntries))
                {
                    if (line.StartsWith("error=")) throw new IOException(line.Substring(6));
                    if (line.StartsWith("seconds=")) secs = double.Parse(line.Substring(8), CultureInfo.InvariantCulture);
                    if (line.StartsWith("ended=")) ended = long.Parse(line.Substring(6), CultureInfo.InvariantCulture);
                    if (line.StartsWith("method=") && line.Length > 7) method = line.Substring(7);
                }
                if (secs < 0) throw new IOException("the copy gave no time");
                if (ended > 0) after = Math.Max(0, (here - ended) / 1e7);
                return secs;
            }
            finally
            {
                try { File.Delete(job); } catch { }
                try { File.Delete(job + ".result"); } catch { }
            }
        }

        /// <summary>The warm child for this exe (started once) copies the job; its answer line.</summary>
        static string Ask(string exe, string job, Func<bool> canceled)
        {
            Server s;
            lock (servers)
            {
                if (!servers.TryGetValue(exe, out s) || s.P.HasExited)
                {
                    if (s != null) Stop(s);
                    var psi = new ProcessStartInfo(exe, "--benchcopy -")
                    {
                        UseShellExecute = false, CreateNoWindow = true, RedirectStandardInput = true, RedirectStandardOutput = true,
                        StandardOutputEncoding = new UTF8Encoding(false),
                    };
                    var p = Process.Start(psi);
                    s = new Server { P = p, In = new StreamWriter(p.StandardInput.BaseStream, new UTF8Encoding(false)) { AutoFlush = true } };
                    servers[exe] = s;
                }
            }
            s.In.WriteLine(job);
            var answer = s.P.StandardOutput.ReadLineAsync();
            while (!answer.Wait(50))
            {
                if (canceled())
                {
                    try { s.P.Kill(); } catch { }
                    s.P.WaitForExit(10000);
                    throw new OperationCanceledException();
                }
            }
            if (answer.Result == null) throw new IOException("the copy ended without a result (the child is gone)");
            return answer.Result;
        }
    }
}
