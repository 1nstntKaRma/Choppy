using System;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

namespace Choppy.Bench
{
    /// <summary>
    /// The move to Rust, step 3 (rust branch only, not ported): the Rust engine as Choppy's side of the Benchmark, in a
    /// child process - choppy.exe --benchcopy job.txt, as Windows' side runs (--shellcopy). This C# Choppy takes the same
    /// switch, so pointing the Benchmark's --rust at this exe checks the method itself (the C# engine in and out of process).
    ///
    /// How the time is counted, as ChoppyCopy counts the copy in the Benchmark's own process: the child copies the tiny
    /// warm-up file first, untimed (both drives looked at once); then its clock runs from right before the job starts
    /// until the job's Finished event. Starting the process is not counted (C# doesn't start one). The child also tells
    /// the moment of that event ("ended=", a FILETIME to the microsecond): the time from then until the parent has it
    /// back (the child ending) is added to the time to the disk, and then the copies are flushed (256 threads) as for
    /// every side. So the Rust side's time to the disk = its copy + its process ending + the flush.
    /// </summary>
    internal static class EngineChild
    {
        [DllImport("kernel32.dll")]
        static extern void GetSystemTimePreciseAsFileTime(out long fileTime);

        /// <summary>The time now as a FILETIME, to the microsecond: a clock the parent and the child both read.</summary>
        public static long PreciseNow() { long t; GetSystemTimePreciseAsFileTime(out t); return t; }

        /// <summary>
        /// The child. Its lines: the warm-up file, a folder for it, the source, the destination folder, the engine's test
        /// switches (the Benchmark's --choppy, or empty). Writes "seconds=...", "ended=...", "method=..." or "error=..."
        /// to job.txt.result.
        /// </summary>
        public static int Child(string jobFile)
        {
            string result;
            try
            {
                var lines = File.ReadAllLines(jobFile, Encoding.UTF8);
                var opts = (lines.Length > 4 ? lines[4] : "").Split(new[] { ' ' }, StringSplitOptions.RemoveEmptyEntries);
                Action<Engine.JobSpec> tweak = opts.Length == 0 ? null : (Action<Engine.JobSpec>)(sp => { for (int k = 0; k < opts.Length; k++) Cli.EngineOption(opts, ref k, sp); });
                string m;
                long ended;
                if (lines[0].Length > 0) Benchmark.EngineCopy(lines[0], lines[1], tweak, () => false, out m);
                double secs = Benchmark.EngineCopy(lines[2], lines[3], tweak, () => false, out m, out ended);
                result = "seconds=" + secs.ToString("R", CultureInfo.InvariantCulture) + "\r\nended=" + ended.ToString(CultureInfo.InvariantCulture) + "\r\nmethod=" + (m ?? "");
            }
            catch (Exception ex) { result = "error=" + ex.Message.Replace("\r", " ").Replace("\n", " "); }
            try { File.WriteAllText(jobFile + ".result", result, Encoding.UTF8); } catch { return 1; }
            return 0;
        }

        /// <summary>One timed copy by a Choppy in a child process: its seconds and the moment it said done (0 = not told). Throws on an error or a cancel.</summary>
        public static double Run(string exe, string warmSrc, string warmDir, string src, string destDir, string engineArgs, Func<bool> canceled, out string method, out long ended)
        {
            method = null;
            ended = 0;
            string job = Path.Combine(Path.GetTempPath(), "choppy-benchcopy-" + Guid.NewGuid().ToString("N").Substring(0, 8) + ".txt");
            File.WriteAllLines(job, new[] { warmSrc ?? "", warmDir ?? "", src, destDir, engineArgs ?? "" }, Encoding.UTF8);
            try
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
                string res = File.Exists(job + ".result") ? File.ReadAllText(job + ".result", Encoding.UTF8) : "error=the copy ended without a result";
                double secs = -1;
                foreach (var line in res.Split(new[] { '\r', '\n' }, StringSplitOptions.RemoveEmptyEntries))
                {
                    if (line.StartsWith("error=")) throw new IOException(line.Substring(6));
                    if (line.StartsWith("seconds=")) secs = double.Parse(line.Substring(8), CultureInfo.InvariantCulture);
                    if (line.StartsWith("ended=")) ended = long.Parse(line.Substring(6), CultureInfo.InvariantCulture);
                    if (line.StartsWith("method=") && line.Length > 7) method = line.Substring(7);
                }
                if (secs < 0) throw new IOException("the copy gave no time");
                return secs;
            }
            finally
            {
                try { File.Delete(job); } catch { }
                try { File.Delete(job + ".result"); } catch { }
            }
        }
    }
}
