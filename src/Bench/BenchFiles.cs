using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using Choppy.Engine;

namespace Choppy.Bench
{
    /// <summary>
    /// The benchmark's test files: its temporary folders (each with a marker file, listed in the data folder until
    /// they are gone), the test data, flushing and purging, and a delete that never leaves those folders.
    /// </summary>
    internal static class BenchFiles
    {
        public const string FolderName = "Choppy benchmark (temporary)";
        public const string MarkerName = "Choppy benchmark - safe to delete.txt";
        const string MarkerHead = "Choppy benchmark test data";
        static readonly object listLock = new object();

        /// <summary>Every temporary folder the benchmark made and hasn't removed yet (cleaned at the next start).</summary>
        static string ListFile { get { return Path.Combine(AppPaths.DataDir, "benchmark-folders.txt"); } }

        // ---------- temporary folders ----------

        /// <summary>
        /// Makes the drive's temporary folder: "X:\Choppy benchmark (temporary)" (a numbered name when a folder of that
        /// name is someone else's), for the system drive also under %TEMP% when its root takes no new folder.
        /// Returns its path, or null with the reason.
        /// </summary>
        public static string MakeFolder(string root, bool system, out string why)
        {
            why = null;
            var bases = new List<string> { root };
            if (system) bases.Add(Path.GetTempPath());
            foreach (string b in bases)
            {
                for (int n = 1; n <= 9; n++)
                {
                    string dir = Path.Combine(b, n == 1 ? FolderName : FolderName + " " + n);
                    if (Directory.Exists(dir) || File.Exists(dir))
                    {
                        // ours from an earlier run that ended without cleaning up: removed, then made again
                        if (!IsOurs(dir) || !Remove(dir)) continue;
                    }
                    // listed before it exists: whatever happens next, the next start finds it
                    Register(dir);
                    try
                    {
                        Directory.CreateDirectory(dir);
                        File.WriteAllText(Path.Combine(dir, MarkerName), MarkerHead + " - Choppy's benchmark writes its test files here and removes this folder"
                            + " when it ends (or at Choppy's next start). Safe to delete." + Environment.NewLine, Encoding.UTF8);
                        return dir;
                    }
                    catch (Exception ex)
                    {
                        why = ex is UnauthorizedAccessException ? "it doesn't let this user make a folder" : ex.Message;
                        try { if (Directory.Exists(dir)) Directory.Delete(dir); } catch { }
                        Unregister(dir);
                        break;
                    }
                }
            }
            if (why == null) why = "no free name for a temporary folder";
            return null;
        }

        /// <summary>True when the folder carries the benchmark's marker file.</summary>
        static bool IsOurs(string dir)
        {
            try
            {
                string m = Path.Combine(dir, MarkerName);
                if (!File.Exists(m)) return false;
                using (var r = new StreamReader(m, Encoding.UTF8)) { string first = r.ReadLine(); return first != null && first.StartsWith(MarkerHead, StringComparison.Ordinal); }
            }
            catch { return false; }
        }

        static void Register(string dir)
        {
            lock (listLock)
            {
                try { Directory.CreateDirectory(AppPaths.DataDir); File.AppendAllText(ListFile, dir + Environment.NewLine, Encoding.UTF8); } catch { }
            }
        }

        static void Unregister(string dir)
        {
            lock (listLock)
            {
                try
                {
                    if (!File.Exists(ListFile)) return;
                    var keep = new List<string>();
                    foreach (var l in File.ReadAllLines(ListFile, Encoding.UTF8))
                        if (l.Trim().Length > 0 && !string.Equals(l.Trim(), dir, StringComparison.OrdinalIgnoreCase)) keep.Add(l.Trim());
                    if (keep.Count == 0) File.Delete(ListFile); else File.WriteAllLines(ListFile, keep, Encoding.UTF8);
                }
                catch { }
            }
        }

        /// <summary>
        /// Removes one of the benchmark's folders: only one with its marker file (or an empty one), and inside it links
        /// are removed, never followed. True when it is gone; a folder that stays is kept on the list for the next start.
        /// </summary>
        public static bool Remove(string dir)
        {
            bool gone;
            if (!Directory.Exists(dir) && !File.Exists(dir)) gone = true;
            else if (IsOurs(dir))
            {
                gone = false;
                // a virus scanner may still look at a file just written: a few more tries
                for (int i = 0; i < 20 && !gone; i++)
                {
                    if (i > 0) Thread.Sleep(250);
                    DeleteTree(Paths.ToLong(dir));
                    gone = !Paths.Exists(Paths.ToLong(dir));
                }
            }
            else
            {
                // not ours (or the marker never got written): only an empty folder goes
                try { Directory.Delete(dir); } catch { }
                gone = true;   // whatever is in it isn't the benchmark's: off the list, untouched
            }
            if (gone) Unregister(dir);
            return gone;
        }

        /// <summary>Folders an earlier benchmark left behind (Choppy closed or the PC went off mid-run): removed.
        /// Never while a benchmark runs (in this Choppy or another one): its folders are in use.</summary>
        public static int CleanLeftovers()
        {
            try { if (!File.Exists(ListFile)) return 0; } catch { return 0; }
            using (var m = Lock())
            {
                if (m == null) return 0;
                string[] dirs;
                lock (listLock)
                {
                    try { if (!File.Exists(ListFile)) return 0; dirs = File.ReadAllLines(ListFile, Encoding.UTF8); } catch { return 0; }
                }
                int n = 0;
                foreach (var d in dirs)
                {
                    string dir = d.Trim();
                    if (dir.Length == 0) continue;
                    bool existed = Directory.Exists(dir);
                    if (Remove(dir) && existed) n++;
                }
                return n;
            }
        }

        /// <summary>
        /// Held while a benchmark runs: one at a time, and no clean-up of leftovers meanwhile. Null when another one has it.
        /// The same thread may take it again (a run cleans up before it starts).
        /// </summary>
        public static Held Lock()
        {
            var m = new Mutex(false, @"Local\Choppy-benchmark");
            try
            {
                if (m.WaitOne(0)) return new Held(m);
            }
            catch (AbandonedMutexException) { return new Held(m); }   // a benchmark that was killed: free now
            m.Dispose();
            return null;
        }

        internal sealed class Held : IDisposable
        {
            Mutex m;
            public Held(Mutex m) { this.m = m; }
            public void Dispose()
            {
                if (m == null) return;
                try { m.ReleaseMutex(); } catch { }
                m.Dispose();
                m = null;
            }
        }

        /// <summary>A folder and all in it. Links inside (junctions, symbolic links) are removed themselves, never followed.</summary>
        public static void DeleteTree(string dir)
        {
            int a = Native.GetFileAttributesW(dir);
            if (a == Native.INVALID_FILE_ATTRIBUTES) return;
            if ((a & (int)Native.FILE_ATTRIBUTE_REPARSE_POINT) == 0 && (a & (int)Native.FILE_ATTRIBUTE_DIRECTORY) != 0)
            {
                Native.WIN32_FIND_DATAW fd;
                IntPtr h = Native.FindFirstFileExW(Paths.Combine(dir, "*"), Native.FindExInfoBasic, out fd, 0, IntPtr.Zero, Native.FIND_FIRST_EX_LARGE_FETCH);
                if (h != Native.INVALID_HANDLE_VALUE)
                {
                    try
                    {
                        do
                        {
                            if (fd.cFileName == "." || fd.cFileName == "..") continue;
                            string p = Paths.Combine(dir, fd.cFileName);
                            if ((fd.dwFileAttributes & Native.FILE_ATTRIBUTE_DIRECTORY) != 0)
                            {
                                if ((fd.dwFileAttributes & Native.FILE_ATTRIBUTE_REPARSE_POINT) == 0) DeleteTree(p);
                                else Native.RemoveDirectoryW(p);
                            }
                            else
                            {
                                if ((fd.dwFileAttributes & Native.FILE_ATTRIBUTE_READONLY) != 0) Native.SetFileAttributesW(p, Native.FILE_ATTRIBUTE_NORMAL);
                                Native.DeleteFileW(p);
                            }
                        } while (Native.FindNextFileW(h, out fd));
                    }
                    finally { Native.FindClose(h); }
                }
                Native.RemoveDirectoryW(dir);
            }
            else if ((a & (int)Native.FILE_ATTRIBUTE_DIRECTORY) != 0) Native.RemoveDirectoryW(dir);   // a link: only the link
            else Native.DeleteFileW(dir);
        }

        // ---------- test data ----------

        /// <summary>
        /// The small files: sizes spread evenly on a log scale from 4 KB to 1 MB (most are small, most bytes are in
        /// the larger ones), the same set on every PC and every run.
        /// </summary>
        public static long[] SmallSizes(int count)
        {
            var r = new Random(20261123);
            var s = new long[count];
            double lo = Math.Log(4096), hi = Math.Log(1 << 20);
            for (int i = 0; i < count; i++) s[i] = (long)Math.Round(Math.Exp(lo + r.NextDouble() * (hi - lo)));
            return s;
        }

        /// <summary>One big file written past the Windows cache (unbuffered, then flushed): nothing of it is in memory.</summary>
        public static unsafe void WriteBig(string path, long size, Func<bool> canceled, Action<long> progress)
        {
            const int chunk = 8 << 20;
            IntPtr mem = Native.VirtualAlloc(IntPtr.Zero, (UIntPtr)chunk, Native.MEM_COMMIT | Native.MEM_RESERVE, Native.PAGE_READWRITE);
            if (mem == IntPtr.Zero) throw new OutOfMemoryException();
            try
            {
                Fill((byte*)mem, chunk, (ulong)size);
                using (var h = Native.CreateFileW(Paths.ToLong(path), Native.GENERIC_WRITE, 0, IntPtr.Zero, Native.CREATE_NEW,
                    Native.FILE_FLAG_NO_BUFFERING, IntPtr.Zero))
                {
                    if (h.IsInvalid) throw new IOException("Can't write " + path + ": " + Native.ErrorText(Marshal.GetLastWin32Error()));
                    long done = 0;
                    while (done < size)
                    {
                        if (canceled()) throw new OperationCanceledException();
                        int n = (int)Math.Min(chunk, size - done);   // sizes are whole MB: always a multiple of the sector
                        *(long*)mem = done;                           // every piece differs
                        int w;
                        if (!Native.WriteFile(h, (byte*)mem, n, out w, IntPtr.Zero) || w != n)
                            throw new IOException("Can't write " + path + ": " + Native.ErrorText(Marshal.GetLastWin32Error()));
                        done += n;
                        if (progress != null) progress(n);
                    }
                    Native.FlushFileBuffers(h);
                }
            }
            finally { Native.VirtualFree(mem, UIntPtr.Zero, Native.MEM_RELEASE); }
        }

        static unsafe void Fill(byte* p, int n, ulong seed)
        {
            ulong x = 0x9E3779B97F4A7C15UL ^ seed;
            ulong* q = (ulong*)p;
            for (int i = 0; i < n / 8; i++) { x ^= x << 13; x ^= x >> 7; x ^= x << 17; q[i] = x; }
        }

        /// <summary>Many small files in a few folders. Written through the cache, then flushed and dropped from it (Purge).</summary>
        public static List<string> WriteSmall(string dir, long[] sizes, Func<bool> canceled, Action<long> progress)
        {
            var data = new byte[2 << 20];
            new Random(7).NextBytes(data);
            var files = new List<string>(sizes.Length);
            int folders = Math.Max(1, sizes.Length / 100);
            for (int i = 0; i < sizes.Length; i++)
            {
                if (canceled()) throw new OperationCanceledException();
                string sub = Path.Combine(dir, "Folder " + (i % folders + 1).ToString("00"));
                if (i < folders) Directory.CreateDirectory(sub);
                string f = Path.Combine(sub, "File " + (i + 1).ToString("00000") + ".bin");
                int off = (int)((i * 7919L) % (data.Length - (1 << 20)));
                using (var fs = new FileStream(f, FileMode.CreateNew, FileAccess.Write, FileShare.None, 4096)) fs.Write(data, off, (int)sizes[i]);
                files.Add(f);
                if (progress != null) progress(sizes[i]);
            }
            return files;
        }

        /// <summary>
        /// Drops files from the Windows cache: an open past the cache with no other handle open makes the file system
        /// write out and drop what it holds of the file (measured here: a re-read comes from the disk). Flushed first,
        /// many at once, so the drop has nothing left to write.
        /// </summary>
        public static void Purge(IList<string> files, Func<bool> canceled)
        {
            Flush(files, canceled);
            ForAll(files, 64, canceled, f =>
            {
                using (var h = Native.CreateFileW(Paths.ToLong(f), Native.GENERIC_READ, Native.FILE_SHARE_READ | Native.FILE_SHARE_WRITE | Native.FILE_SHARE_DELETE,
                    IntPtr.Zero, Native.OPEN_EXISTING, Native.FILE_FLAG_NO_BUFFERING, IntPtr.Zero)) { }
            });
        }

        /// <summary>
        /// Puts each file's data on the disk itself (FlushFileBuffers), up to 256 at once: the disk then orders the writes
        /// itself (1,000 small files on a hard disk: 256 at once 2.1 s, 32 at once 7.7 s, one by one 22 s; already on the disk 0.03 s).
        /// </summary>
        public static void Flush(IList<string> files, Func<bool> canceled)
        {
            ForAll(files, 256, canceled, f =>
            {
                using (var h = Native.CreateFileW(Paths.ToLong(f), Native.GENERIC_WRITE, Native.FILE_SHARE_READ | Native.FILE_SHARE_WRITE | Native.FILE_SHARE_DELETE,
                    IntPtr.Zero, Native.OPEN_EXISTING, 0, IntPtr.Zero))
                {
                    if (!h.IsInvalid) Native.FlushFileBuffers(h);
                }
            });
        }

        /// <summary>Every file under a folder with its size (links are not followed).</summary>
        public static List<KeyValuePair<string, long>> List(string dir)
        {
            var r = new List<KeyValuePair<string, long>>();
            ListInto(Paths.ToLong(dir), r);
            return r;
        }

        static void ListInto(string dir, List<KeyValuePair<string, long>> r)
        {
            Native.WIN32_FIND_DATAW fd;
            IntPtr h = Native.FindFirstFileExW(Paths.Combine(dir, "*"), Native.FindExInfoBasic, out fd, 0, IntPtr.Zero, Native.FIND_FIRST_EX_LARGE_FETCH);
            if (h == Native.INVALID_HANDLE_VALUE) return;
            var subs = new List<string>();
            try
            {
                do
                {
                    if (fd.cFileName == "." || fd.cFileName == "..") continue;
                    string p = Paths.Combine(dir, fd.cFileName);
                    if ((fd.dwFileAttributes & Native.FILE_ATTRIBUTE_DIRECTORY) != 0) { if ((fd.dwFileAttributes & Native.FILE_ATTRIBUTE_REPARSE_POINT) == 0) subs.Add(p); }
                    else r.Add(new KeyValuePair<string, long>(Paths.ToShort(p), ((long)fd.nFileSizeHigh << 32) | fd.nFileSizeLow));
                } while (Native.FindNextFileW(h, out fd));
            }
            finally { Native.FindClose(h); }
            foreach (var s in subs) ListInto(s, r);
        }

        /// <summary>Runs an action on every item with a few threads; the first error is thrown after all have stopped.</summary>
        public static void ForAll(IList<string> items, int threads, Func<bool> canceled, Action<string> act)
        {
            if (items.Count == 0) return;
            int next = -1;
            Exception first = null;
            var ts = new Thread[Math.Min(threads, items.Count)];
            for (int t = 0; t < ts.Length; t++)
            {
                ts[t] = new Thread(() =>
                {
                    int i;
                    while ((i = Interlocked.Increment(ref next)) < items.Count && first == null && !canceled())
                    {
                        try { act(items[i]); }
                        catch (Exception ex) { Interlocked.CompareExchange(ref first, ex, null); }
                    }
                }, 256 << 10) { IsBackground = true, Name = "bench files" };
                ts[t].Start();
            }
            foreach (var t in ts) t.Join();
            if (first != null) throw first;
            if (canceled()) throw new OperationCanceledException();
        }
    }
}
