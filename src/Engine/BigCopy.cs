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
        readonly long size, pieces;
        readonly bool overlapped;
        readonly IntPtr mem;
        readonly int[] lens;
        readonly IntPtr[] ovs;          // one OVERLAPPED per buffer (unmanaged: it must not move while a read is out)
        readonly SemaphoreSlim free, full;
        readonly Thread th;
        readonly XxHash64 hash;
        volatile bool stop;
        long taken;
        public volatile int Error;      // read error (0 = none)

        /// <summary>size = bytes to read (the file's size). overlapped = the handle was opened with FILE_FLAG_OVERLAPPED.</summary>
        public RingReader(SafeFileHandle h, long size, int chunk, int slots, bool overlapped, XxHash64 hash)
        {
            this.h = h; this.size = size; this.chunk = chunk; this.slots = slots; this.overlapped = overlapped; this.hash = hash;
            pieces = size <= 0 ? 0 : (size + chunk - 1) / chunk;
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
                int want = (int)Math.Min(chunk, size - k * chunk), n = 0;
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
                int n = Collect(done);
                // a read can come back short before the end (over a network): read the rest of the piece
                long want = Math.Min(chunk, size - done * chunk);
                while (n > 0 && n < want && Error == 0)
                {
                    if (!Issue(done, n, (int)(chunk - n))) break;
                    int m = Collect(done);
                    if (m <= 0) { if (Error == 0) Error = Native.ERROR_HANDLE_EOF; break; }
                    n += m;
                }
                if (n <= 0 && Error == 0) Error = Native.ERROR_HANDLE_EOF;
                if (Error != 0) n = 0;
                Post(done, n);
                done++;
                if (n <= 0) { Drain(done, issued); return; }
            }
            Drain(done, issued);
        }

        bool Issue(long piece, int offsetInPiece, int bytes)
        {
            var o = (NativeOverlapped*)ovs[piece % slots];
            long off = piece * chunk + offsetInPiece;
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
            while (got < more && n == (got + 1) * chunk && k + 1 < pieces && (k + 1) % slots != 0 && full.Wait(0))
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
            return Copy(src, dst, ctime, mtime, attrs, tune, hash, flush, renameTo, 0, progress);
        }

        /// <summary>
        /// cachedTail: the last bytes of the file go through the Windows cache (like Explorer's copy), so the copy is
        /// done as soon as they are in memory and Windows writes them out right after. 0 = all straight to the disk.
        /// </summary>
        public static BigResult Copy(string src, string dst, long ctime, long mtime, uint attrs, PipeTuning tune,
            bool hash, bool flush, string renameTo, long cachedTail, Func<long, bool> progress)
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
                        if (room > 0 && !SetLength(hd, Native.FileAllocationInfo, room))
                        {
                            int e = Marshal.GetLastWin32Error();
                            if (e == Native.ERROR_DISK_FULL || e == Native.ERROR_HANDLE_DISK_FULL) { r.Error = e; return r; }
                        }
                        if (room > 0 && !SetLength(hd, Native.FileEndOfFileInfo, room)) { r.Error = Unsupported(Marshal.GetLastWin32Error()); return r; }

                        var x = hash ? new XxHash64() : null;
                        long done = 0;
                        // no more buffers than the file has pieces (a 40 MB file needs two, not four), nor than memory allows
                        int slots = (int)Math.Max(2, Math.Min(tune.Slots, (size + tune.Chunk - 1) / tune.Chunk));
                        var mem = new Native.MEMORYSTATUSEX { dwLength = (uint)Marshal.SizeOf(typeof(Native.MEMORYSTATUSEX)) };
                        if (Native.GlobalMemoryStatusEx(ref mem)) slots = (int)Math.Max(2, Math.Min(slots, (long)(mem.ullAvailPhys / 8) / tune.Chunk));
                        using (var rr = new RingReader(hs, size, tune.Chunk, slots, true, x))
                        {
                            byte* p; int n;
                            while (rr.Next(out p, out n))
                            {
                                int joined = tune.WriteBatch > 1 ? rr.Gather(tune.WriteBatch - 1, ref n) : 0;
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
                                int w = n, wrote;
                                bool ok;
                                if (hc != null) ok = Native.WriteFile(hc, p, n, out wrote, IntPtr.Zero);
                                else
                                {
                                    if (w % align != 0)
                                    {
                                        // the tail: pad to a whole sector, the size is put right below
                                        int padded = (n + align - 1) / align * align;
                                        for (int k = n; k < padded; k++) p[k] = 0;
                                        w = padded;
                                    }
                                    ok = Native.WriteFile(hd, p, w, out wrote, IntPtr.Zero);
                                }
                                if (!ok) { r.Error = Unsupported(Marshal.GetLastWin32Error()); break; }
                                rr.Release(1 + joined);
                                done += n;
                                if (!progress(done)) { r.Error = Native.ERROR_REQUEST_ABORTED; break; }
                            }
                            if (r.Error == 0 && rr.Error != 0) r.Error = Unsupported(rr.Error);
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
