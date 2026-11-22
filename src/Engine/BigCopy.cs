using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;
using Microsoft.Win32.SafeHandles;

namespace Choppy.Engine
{
    /// <summary>
    /// Reads a file front to back on its own thread into a ring of large aligned buffers, so the next piece is
    /// already coming off the disk while the caller writes or hashes the last one. On an overlapped handle every free
    /// buffer has a read waiting at the disk: the disk always has the next piece of this file queued, so other
    /// programs reading the same disk (a torrent client, an indexer) can't slip in between and send the heads away.
    /// </summary>
    internal sealed unsafe class RingReader : IDisposable
    {
        readonly SafeFileHandle h;
        readonly int chunk, slots;
        readonly long size, pieces, start;   // reads [start, size)
        readonly bool overlapped;
        readonly bool phased;           // one disk at both ends: read a batch, then write it (see ReadPhased)
        readonly IntPtr mem;
        readonly int[] lens;
        readonly IntPtr[] ovs;          // one OVERLAPPED per buffer (unmanaged: it must not move while a read is out)
        readonly SemaphoreSlim free, full;
        readonly Thread th;
        readonly XxHash64 hash;
        volatile bool stop;
        long taken;
        public volatile int Error;      // read error (0 = none)

        /// <summary>
        /// size = where to stop reading (the file's size, or the end of a stretch); start = where to begin (overlapped
        /// handles only, a sector multiple). overlapped = the handle was opened with FILE_FLAG_OVERLAPPED.
        /// </summary>
        public RingReader(SafeFileHandle h, long size, int chunk, int slots, bool overlapped, XxHash64 hash, bool phased = false, long start = 0)
        {
            this.h = h; this.size = size; this.chunk = chunk; this.slots = slots; this.overlapped = overlapped; this.hash = hash;
            this.phased = phased && overlapped;
            this.start = overlapped ? start : 0;
            pieces = size - this.start <= 0 ? 0 : (size - this.start + chunk - 1) / chunk;
            mem = Native.VirtualAlloc(IntPtr.Zero, (UIntPtr)((long)chunk * slots), Native.MEM_COMMIT | Native.MEM_RESERVE, Native.PAGE_READWRITE);
            if (mem == IntPtr.Zero) throw new OutOfMemoryException();
            lens = new int[slots];
            if (overlapped)
            {
                ovs = new IntPtr[slots];
                for (int i = 0; i < slots; i++)
                {
                    ovs[i] = Marshal.AllocHGlobal(sizeof(NativeOverlapped));
                    var o = (NativeOverlapped*)ovs[i];
                    *o = new NativeOverlapped();
                    o->EventHandle = Native.CreateEventW(IntPtr.Zero, true, false, IntPtr.Zero);
                }
            }
            free = new SemaphoreSlim(slots);
            full = new SemaphoreSlim(0);
            th = new Thread(overlapped ? (ThreadStart)ReadQueued : ReadInOrder) { IsBackground = true, Name = "read ahead", Priority = ThreadPriority.AboveNormal };
            th.Start();
        }

        public int Slots { get { return slots; } }

        byte* Buf(long piece) { return (byte*)mem + (piece % slots) * chunk; }

        void Post(long piece, int n)
        {
            if (n > 0 && hash != null) hash.Update(Buf(piece), n);
            lens[piece % slots] = n;
            full.Release();
        }

        // plain handle: one read after the other on this thread
        void ReadInOrder()
        {
            for (long k = 0; k < pieces && !stop; k++)
            {
                free.Wait();
                if (stop) return;
                int want = (int)Math.Min(chunk, size - start - k * chunk), n = 0;
                while (n < want)
                {
                    int m;
                    if (!Native.ReadFile(h, Buf(k) + n, want - n, out m, IntPtr.Zero)) { Error = Marshal.GetLastWin32Error(); break; }
                    if (m == 0) { Error = Native.ERROR_HANDLE_EOF; break; }   // shorter than it said
                    n += m;
                }
                if (Error != 0) n = 0;
                Post(k, n);
                if (n <= 0) return;
            }
        }

        // overlapped handle: as many reads out at once as there are free buffers
        void ReadQueued()
        {
            if (phased) { ReadPhased(); return; }
            long issued = 0, done = 0;
            while (done < pieces && !stop)
            {
                while (issued < pieces && issued - done < slots)
                {
                    if (issued == done) free.Wait();          // nothing out: wait for a buffer
                    else if (!free.Wait(0)) break;              // reads are out: go collect one
                    if (stop) break;
                    if (!Issue(issued, 0, chunk)) { Post(done, 0); Drain(done, issued); return; }
                    issued++;
                }
                if (stop) break;
                int n = CollectWhole(done);
                Post(done, n);
                done++;
                if (n <= 0) { Drain(done, issued); return; }
            }
            Drain(done, issued);
        }

        // One disk at both ends: a whole batch (every buffer) is read with no write in between, then written with no
        // read in between - the heads go between the two places once per batch instead of once per piece.
        void ReadPhased()
        {
            long done = 0;
            while (done < pieces && !stop)
            {
                for (int i = 0; i < slots; i++) { free.Wait(); if (stop) return; }   // the writer gave every buffer back
                long end = Math.Min(pieces, done + slots);
                for (long k = done; k < end; k++)
                    if (!Issue(k, 0, chunk)) { Drain(done, k); Post(done, 0); return; }
                for (; done < end; done++)
                {
                    int n = CollectWhole(done);
                    Post(done, n);
                    if (n <= 0) { Drain(done + 1, end); return; }
                }
            }
        }

        /// <summary>A piece read in full (a read can come back short over a network: the rest is read too). 0 = error.</summary>
        int CollectWhole(long piece)
        {
            int n = Collect(piece);
            long want = Math.Min(chunk, size - start - piece * chunk);
            while (n > 0 && n < want && Error == 0)
            {
                if (!Issue(piece, n, (int)(chunk - n))) break;
                int m = Collect(piece);
                if (m <= 0) { if (Error == 0) Error = Native.ERROR_HANDLE_EOF; break; }
                n += m;
            }
            if (n <= 0 && Error == 0) Error = Native.ERROR_HANDLE_EOF;
            return Error != 0 ? 0 : n;
        }

        bool Issue(long piece, int offsetInPiece, int bytes)
        {
            var o = (NativeOverlapped*)ovs[piece % slots];
            long off = start + piece * chunk + offsetInPiece;
            o->OffsetLow = (int)(off & 0xFFFFFFFF);
            o->OffsetHigh = (int)(off >> 32);
            o->InternalLow = IntPtr.Zero; o->InternalHigh = IntPtr.Zero;
            Native.ResetEvent(o->EventHandle);
            if (Native.ReadFileAt(h, Buf(piece) + offsetInPiece, bytes, IntPtr.Zero, o)) return true;
            int e = Marshal.GetLastWin32Error();
            if (e == Native.ERROR_IO_PENDING) return true;
            Error = e;
            return false;
        }

        int Collect(long piece)
        {
            int n;
            if (Native.GetOverlappedResult(h, (NativeOverlapped*)ovs[piece % slots], out n, true)) return n;
            int e = Marshal.GetLastWin32Error();
            if (e != Native.ERROR_HANDLE_EOF && Error == 0) Error = e;
            return 0;
        }

        // wait for reads still out before their buffers can go away
        void Drain(long from, long to)
        {
            for (long k = from; k < to; k++) { int n; Native.GetOverlappedResult(h, (NativeOverlapped*)ovs[k % slots], out n, true); }
        }

        /// <summary>The next piece in file order. False = the end (or a read error: see Error).</summary>
        public bool Next(out byte* p, out int n)
        {
            p = Buf(taken);
            n = 0;
            if (taken >= pieces) return false;
            full.Wait();
            n = lens[taken % slots];
            return n > 0;
        }

        /// <summary>
        /// The pieces right after the one from Next that are already read (and sit right after it in memory) join it,
        /// up to more: one bigger write keeps a hard disk streaming. n grows by their bytes; returns how many joined.
        /// </summary>
        public int Gather(int more, ref int n)
        {
            int got = 0;
            long k = taken;
            // phased: wait for the rest of the batch (it is being read right now); else take only what is there
            while (got < more && n == (got + 1) * chunk && k + 1 < pieces && (k + 1) % slots != 0 && (phased ? full.Wait(Timeout.Infinite) : full.Wait(0)))
            {
                int m = lens[(k + 1) % slots];
                if (m <= 0) { full.Release(); break; }   // a read error: left for Next to report
                k++;
                got++;
                n += m;
            }
            return got;
        }

        /// <summary>Hands the pieces from Next (+ Gather) back to the reader.</summary>
        public void Release(int count = 1)
        {
            taken += count;
            free.Release(count);
        }

        public void Dispose()
        {
            stop = true;
            free.Release(slots + 1);
            th.Join();
            if (ovs != null)
                foreach (var o in ovs) { Native.CloseHandle(((NativeOverlapped*)o)->EventHandle); Marshal.FreeHGlobal(o); }
            Native.VirtualFree(mem, UIntPtr.Zero, Native.MEM_RELEASE);
            free.Dispose(); full.Dispose();
        }
    }

    /// <summary>How the pipeline runs for one job (picked by Plan from the drives on both ends).</summary>
    internal sealed class PipeTuning
    {
        public int Chunk = 8 << 20;     // bytes per read / write
        public int Slots = 4;           // pieces in flight between the reader and the writer
        public int Align = 4096;        // sector multiple for unbuffered I/O
        public int WriteBatch = 1;      // pieces already read that may go out as one write (a hard disk streams without gaps)
        public bool Phased;             // one hard disk at both ends: read all buffers, then write them all
        public bool Prealloc = true;    // reserve the whole size up front
        public int CachedWriteChunk;    // bytes per write through the Windows cache (0 = a whole piece)
    }

    /// <summary>
    /// Finds the fastest way for this copy while it runs. No fixed rule fits every PC, disk and cable, so the first big
    /// file is copied in a few short stretches (about 1.5 s each), each one another way - piece size, pieces in flight,
    /// reading and writing at once or one after the other - and the fastest way copies the rest, and the next big files.
    /// </summary>
    internal sealed class PipeTuner
    {
        readonly List<PipeTuning> ways;
        readonly double[] rate;             // bytes/s per way (0 = not measured)
        int state = -1;                     // -1 = warming up, 0..n-1 = trying way i, n = settled
        long trialBytes = 256L << 20;
        public PipeTuning Best;
        public bool Changed;                // settled on another way than the starting one

        const long WarmBytes = 128L << 20;
        const long Unit = 64L << 20;        // stretches end on whole pieces of every way (pieces are at most 64 MB)

        public PipeTuner(PipeTuning start, IEnumerable<PipeTuning> others)
        {
            ways = new List<PipeTuning> { start };
            if (others != null) ways.AddRange(others);
            rate = new double[ways.Count];
            Best = start;
        }

        public bool Settled { get { return state >= ways.Count; } }

        /// <summary>The way for the stretch from done on, and where that stretch ends. keepFree = the file's end that must stay out of trials.</summary>
        public PipeTuning Next(long done, long size, long keepFree, out long end, out bool measured)
        {
            measured = false;
            end = size;
            if (Settled || ways.Count < 2) return Best;
            long len = state < 0 ? WarmBytes : trialBytes;
            if (done + len + keepFree + Unit > size) return Best;   // no room in this file: the next big file tries
            end = done + len;
            measured = state >= 0;
            return state < 0 ? Best : ways[state];
        }

        public void Report(long bytes, double seconds, bool measured)
        {
            if (Settled || seconds < 0.2 || bytes <= 0) return;
            double r = bytes / seconds;
            if (state < 0)
            {
                // a trial of about 1.5 s at the speed seen so far, whole 64 MB units, 128 MB .. 1 GB
                long b = (long)(r * 1.5) / Unit * Unit;
                trialBytes = Math.Max(2 * Unit, Math.Min(16 * Unit, b));
                state = 0;
                return;
            }
            if (!measured) return;
            rate[state] = r;
            if (++state < ways.Count) return;
            // another way must win by 3%, or the starting one stays (no flapping on noise)
            int best = 0;
            for (int i = 1; i < ways.Count; i++) if (rate[i] > rate[best]) best = i;
            if (best != 0 && rate[best] < rate[0] * 1.03) best = 0;
            Best = ways[best];
            Changed = best != 0;
        }

        /// <summary>What was measured, for the log and tests: "32x8 phased 150 MB/s | 16x8 phased 161 MB/s <-".</summary>
        public string Summary
        {
            get
            {
                var parts = new List<string>();
                for (int i = 0; i < ways.Count; i++)
                    if (rate[i] > 0) parts.Add(Describe(ways[i]) + " " + (rate[i] / 1048576).ToString("0", System.Globalization.CultureInfo.InvariantCulture) + " MB/s" + (ways[i] == Best && Settled ? " <-" : ""));
                return string.Join(" | ", parts);
            }
        }

        public static string Describe(PipeTuning t)
        {
            return (t.Chunk >> 20) + "x" + t.Slots + (t.Phased ? " phased" : "") + (t.WriteBatch > 1 ? " join" + t.WriteBatch : "");
        }
    }

    internal sealed class BigResult
    {
        public int Error;               // 0 = done; BigCopy.NotSupported = use CopyFileEx for this file
        public long Bytes;              // bytes copied
        public ulong Hash;              // xxHash64 of the source as it was read (when asked)
        public bool Renamed;            // already under its final name
    }

    /// <summary>
    /// Big files between local drives: the source is read and the copy is written at the same time (reader thread +
    /// writer thread, large aligned buffers, no Windows cache on either side). CopyFileEx with no buffering reads a
    /// piece, writes it, then reads the next: two disks take turns and each one idles half the time.
    /// Keeps what CopyFileEx keeps: created / modified dates, attributes, extra data streams (Zone.Identifier).
    /// </summary>
    internal static unsafe class BigCopy
    {
        public const int NotSupported = -1;

        const uint KeepAttrs = Native.FILE_ATTRIBUTE_READONLY | Native.FILE_ATTRIBUTE_HIDDEN | Native.FILE_ATTRIBUTE_SYSTEM
            | Native.FILE_ATTRIBUTE_ARCHIVE | Native.FILE_ATTRIBUTE_NOT_CONTENT_INDEXED;

        /// <summary>Files this path never takes: encrypted, compressed, sparse, cloud placeholders, links.</summary>
        public const uint SkipAttrs = Native.FILE_ATTRIBUTE_ENCRYPTED | Native.FILE_ATTRIBUTE_COMPRESSED | Native.FILE_ATTRIBUTE_SPARSE_FILE
            | Native.FILE_ATTRIBUTE_OFFLINE | Native.FILE_ATTRIBUTE_RECALL_ON_OPEN | Native.FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS | Native.FILE_ATTRIBUTE_REPARSE_POINT;

        /// <summary>
        /// Copies src to dst (a new name: never replaces anything). progress(bytes written so far) returns false to stop.
        /// renameTo: give the copy this final name through its own handle once every byte is written.
        /// </summary>
        public static BigResult Copy(string src, string dst, long ctime, long mtime, uint attrs, PipeTuning tune,
            bool hash, bool flush, string renameTo, Func<long, bool> progress)
        {
            return Copy(src, dst, ctime, mtime, attrs, tune, hash, flush, renameTo, 0, progress, null);
        }

        /// <summary>
        /// cachedTail: the last bytes of the file go through the Windows cache (like Explorer's copy), so the copy is
        /// done as soon as they are in memory and Windows writes them out right after. 0 = all straight to the disk.
        /// </summary>
        public static BigResult Copy(string src, string dst, long ctime, long mtime, uint attrs, PipeTuning tune,
            bool hash, bool flush, string renameTo, long cachedTail, Func<long, bool> progress, PipeTuner tuner = null)
        {
            var r = new BigResult();
            using (var hs = Native.CreateFileW(src, Native.GENERIC_READ, Native.FILE_SHARE_READ | Native.FILE_SHARE_DELETE, IntPtr.Zero,
                Native.OPEN_EXISTING, Native.FILE_FLAG_NO_BUFFERING | Native.FILE_FLAG_SEQUENTIAL_SCAN | Native.FILE_FLAG_OVERLAPPED, IntPtr.Zero))
            {
                if (hs.IsInvalid) { r.Error = Unsupported(Marshal.GetLastWin32Error()); return r; }
                long size;
                if (!Native.GetFileSizeEx(hs, out size)) { r.Error = Marshal.GetLastWin32Error(); return r; }
                var streams = NamedStreams(src);
                if (streams == null) { r.Error = NotSupported; return r; }

                using (var hd = Native.CreateFileW(dst, Native.GENERIC_READ | Native.GENERIC_WRITE | Native.DELETE, Native.FILE_SHARE_READ | Native.FILE_SHARE_WRITE, IntPtr.Zero,
                    Native.CREATE_NEW, Native.FILE_FLAG_NO_BUFFERING | Native.FILE_ATTRIBUTE_NORMAL, IntPtr.Zero))
                {
                    SafeFileHandle hc = null;   // the cached tail goes through this one
                    SafeFileHandle fin = hd;    // the handle the copy is finished on (hc once the tail started)
                    try
                    {
                        if (hd.IsInvalid) { r.Error = Unsupported(Marshal.GetLastWin32Error()); return r; }
                        int align = Math.Max(512, tune.Align);
                        long room = (size + align - 1) / align * align;
                        // the whole size up front: the disk can hand out one piece, and a full disk shows at once
                        if (tune.Prealloc && room > 0 && !SetLength(hd, Native.FileAllocationInfo, room))
                        {
                            int e = Marshal.GetLastWin32Error();
                            if (e == Native.ERROR_DISK_FULL || e == Native.ERROR_HANDLE_DISK_FULL) { r.Error = e; return r; }
                        }
                        if (tune.Prealloc && room > 0 && !SetLength(hd, Native.FileEndOfFileInfo, room)) { r.Error = Unsupported(Marshal.GetLastWin32Error()); return r; }

                        var x = hash ? new XxHash64() : null;
                        long done = 0;
                        var mem = new Native.MEMORYSTATUSEX { dwLength = (uint)Marshal.SizeOf(typeof(Native.MEMORYSTATUSEX)) };
                        long memCap = Native.GlobalMemoryStatusEx(ref mem) ? (long)(mem.ullAvailPhys / 8) : long.MaxValue;
                        // stretch by stretch: with a tuner the first stretches try other ways, then the fastest copies the rest
                        while (done < size && r.Error == 0)
                        {
                            long end = size;
                            bool measured = false;
                            if (tuner != null) tune = tuner.Next(done, size, cachedTail, out end, out measured);
                            // no more buffers than the stretch has pieces (a 40 MB file needs two, not four), nor than memory allows
                            int slots = (int)Math.Max(2, Math.Min(tune.Slots, (end - done + tune.Chunk - 1) / tune.Chunk));
                            slots = (int)Math.Max(2, Math.Min(slots, memCap / tune.Chunk));
                            long from = done, waited = 0;
                            long t0 = System.Diagnostics.Stopwatch.GetTimestamp();
                            using (var rr = new RingReader(hs, end, tune.Chunk, slots, true, x, tune.Phased, done))
                            {
                                byte* p; int n;
                                while (rr.Next(out p, out n))
                                {
                                    // phased (one disk): the whole batch is read before any of it is written
                                    int joined = tune.Phased ? rr.Gather(rr.Slots - 1, ref n) : tune.WriteBatch > 1 ? rr.Gather(tune.WriteBatch - 1, ref n) : 0;
                                    // phased: written a piece at a time, so the progress moves smoothly (the writes follow each other on the disk)
                                    int step = tune.Phased ? tune.Chunk : n;
                                    for (int off = 0; off < n && r.Error == 0; )
                                    {
                                        if (hc == null && cachedTail > 0 && size - done <= cachedTail)
                                        {
                                            // the last part: through the Windows cache from here on (done is a whole number of pieces).
                                            // The unbuffered handle closes first: Windows writes cached data out when such a handle
                                            // sets the size or closes, and the tail must stay in memory until Windows writes it on its own
                                            hc = Native.CreateFileW(dst, Native.GENERIC_WRITE, Native.FILE_SHARE_READ | Native.FILE_SHARE_WRITE | Native.FILE_SHARE_DELETE,
                                                IntPtr.Zero, Native.OPEN_EXISTING, Native.FILE_FLAG_SEQUENTIAL_SCAN, IntPtr.Zero);
                                            long at;
                                            if (hc.IsInvalid || !Native.SetFilePointerEx(hc, done, out at, 0)) { hc.Dispose(); hc = null; cachedTail = 0; }
                                            else { hd.Dispose(); fin = hc; renameTo = null; }   // renamed by path once it is closed
                                        }
                                        int len = Math.Min(hc != null && tune.CachedWriteChunk > 0 ? Math.Min(step, tune.CachedWriteChunk) : step, n - off), w = len, wrote;
                                        byte* q = p + off;
                                        bool ok;
                                        if (hc != null) ok = Native.WriteFile(hc, q, len, out wrote, IntPtr.Zero);
                                        else
                                        {
                                            if (w % align != 0)
                                            {
                                                // the tail: pad to a whole sector, the size is put right below
                                                int padded = (len + align - 1) / align * align;
                                                for (int k = len; k < padded; k++) q[k] = 0;
                                                w = padded;
                                            }
                                            ok = Native.WriteFile(hd, q, w, out wrote, IntPtr.Zero);
                                        }
                                        if (!ok) { r.Error = Unsupported(Marshal.GetLastWin32Error()); break; }
                                        off += len;
                                        done += len;
                                        // a pause or the speed limit holds the copy here: that time isn't the way's speed
                                        long c0 = System.Diagnostics.Stopwatch.GetTimestamp();
                                        if (!progress(done)) r.Error = Native.ERROR_REQUEST_ABORTED;
                                        long held = System.Diagnostics.Stopwatch.GetTimestamp() - c0;
                                        if (held > System.Diagnostics.Stopwatch.Frequency / 20) waited += held;
                                    }
                                    rr.Release(1 + joined);
                                    if (r.Error != 0) break;
                                }
                                if (r.Error == 0 && rr.Error != 0) r.Error = Unsupported(rr.Error);
                            }
                            if (tuner != null && r.Error == 0)
                                tuner.Report(done - from, (double)(System.Diagnostics.Stopwatch.GetTimestamp() - t0 - waited) / System.Diagnostics.Stopwatch.Frequency, measured);
                        }
                        r.Bytes = done;
                        if (r.Error != 0) return r;
                        if (done != size) { r.Error = NotSupported; return r; }   // the size changed under us: let CopyFileEx do it

                        if (!SetLength(fin, Native.FileEndOfFileInfo, size)) { r.Error = Marshal.GetLastWin32Error(); return r; }
                        foreach (var s in streams)
                        {
                            int se = CopyStream(src + s, dst + s);
                            if (se != 0) { r.Error = NotSupported; return r; }
                        }
                        // dates + attributes, set on the open handle (nothing changes them after this)
                        var bi = new Native.FILE_BASIC_INFO { CreationTime = ctime, LastWriteTime = mtime, FileAttributes = attrs & KeepAttrs };
                        if (bi.FileAttributes == 0) bi.FileAttributes = Native.FILE_ATTRIBUTE_NORMAL;
                        Native.SetFileInformationByHandle(fin, Native.FileBasicInfo, &bi, Marshal.SizeOf(typeof(Native.FILE_BASIC_INFO)));
                        if (flush && !Native.FlushFileBuffers(fin)) { r.Error = Marshal.GetLastWin32Error(); return r; }
                        if (x != null) r.Hash = x.Digest();
                        if (renameTo != null) r.Renamed = Native.RenameByHandle(fin.DangerousGetHandle(), renameTo);
                    }
                    finally { if (hc != null) hc.Dispose(); }
                }
            }
            return r;
        }

        // the way in or out refuses unbuffered I/O (some file systems and drivers): CopyFileEx takes this file
        static int Unsupported(int e)
        {
            return e == Native.ERROR_INVALID_PARAMETER || e == 50 /*ERROR_NOT_SUPPORTED*/ || e == 1 /*ERROR_INVALID_FUNCTION*/ ? NotSupported : e;
        }

        static bool SetLength(SafeFileHandle h, int infoClass, long bytes)
        {
            long v = bytes;
            return Native.SetFileInformationByHandle(h, infoClass, &v, 8);
        }

        /// <summary>Names of the extra data streams (":Zone.Identifier:$DATA"); null = couldn't tell.</summary>
        static List<string> NamedStreams(string path)
        {
            var r = new List<string>();
            Native.WIN32_FIND_STREAM_DATA d;
            IntPtr h = Native.FindFirstStreamW(path, 0 /*FindStreamInfoStandard*/, out d, 0);
            if (h == Native.INVALID_HANDLE_VALUE)
            {
                int e = Marshal.GetLastWin32Error();
                return e == Native.ERROR_HANDLE_EOF || e == 38 ? r : null;   // no streams at all (FAT, exFAT)
            }
            try
            {
                do
                {
                    string n = d.cStreamName;
                    if (!string.IsNullOrEmpty(n) && n != "::$DATA" && n.EndsWith(":$DATA", StringComparison.OrdinalIgnoreCase)) r.Add(n);
                } while (Native.FindNextStreamW(h, out d));
            }
            finally { Native.FindClose(h); }
            return r;
        }

        static int CopyStream(string from, string to)
        {
            using (var a = Native.CreateFileW(from, Native.GENERIC_READ, Native.FILE_SHARE_READ | Native.FILE_SHARE_DELETE, IntPtr.Zero, Native.OPEN_EXISTING, 0, IntPtr.Zero))
            {
                if (a.IsInvalid) return Marshal.GetLastWin32Error();
                // the copy itself is open with DELETE access (for the rename): a stream of it opens only when it shares delete
                using (var b = Native.CreateFileW(to, Native.GENERIC_WRITE, Native.FILE_SHARE_READ | Native.FILE_SHARE_WRITE | Native.FILE_SHARE_DELETE, IntPtr.Zero, 2 /*CREATE_ALWAYS*/, 0, IntPtr.Zero))
                {
                    if (b.IsInvalid) return Marshal.GetLastWin32Error();
                    try
                    {
                        using (var ra = new FileStream(a, FileAccess.Read, 1))
                        using (var wb = new FileStream(b, FileAccess.Write, 1))
                            ra.CopyTo(wb, 1 << 16);
                    }
                    catch (IOException) { return Native.ERROR_IO_DEVICE; }
                }
            }
            return 0;
        }
    }
}
