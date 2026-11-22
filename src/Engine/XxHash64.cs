using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Threading;
using Microsoft.Win32.SafeHandles;

namespace Choppy.Engine
{
    /// <summary>Streaming xxHash64 (seed 0) plus a file hasher that reads straight from the disk/server (no cache).</summary>
    internal sealed unsafe class XxHash64
    {
        const ulong P1 = 11400714785074694791UL, P2 = 14029467366897019727UL, P3 = 1609587929392839161UL,
                    P4 = 9650029242287828579UL, P5 = 2870177450012600261UL;

        ulong v1, v2, v3, v4;
        ulong total;
        readonly byte[] tail = new byte[32];
        int tailLen;

        public XxHash64()
        {
            v1 = unchecked(P1 + P2); v2 = P2; v3 = 0; v4 = unchecked(0 - P1);
        }

        static ulong Rotl(ulong x, int r) { return (x << r) | (x >> (64 - r)); }
        static ulong Round(ulong acc, ulong input) { unchecked { acc += input * P2; acc = Rotl(acc, 31); acc *= P1; return acc; } }
        static ulong Merge(ulong acc, ulong val) { unchecked { val = Round(0, val); acc ^= val; acc = acc * P1 + P4; return acc; } }

        public void Update(byte* data, int len)
        {
            unchecked
            {
                total += (ulong)len;
                if (tailLen + len < 32)
                {
                    for (int i = 0; i < len; i++) tail[tailLen + i] = data[i];
                    tailLen += len;
                    return;
                }
                byte* p = data;
                byte* end = data + len;
                if (tailLen > 0)
                {
                    int fill = 32 - tailLen;
                    for (int i = 0; i < fill; i++) tail[tailLen + i] = p[i];
                    fixed (byte* t = tail)
                    {
                        v1 = Round(v1, *(ulong*)t); v2 = Round(v2, *(ulong*)(t + 8));
                        v3 = Round(v3, *(ulong*)(t + 16)); v4 = Round(v4, *(ulong*)(t + 24));
                    }
                    p += fill; tailLen = 0;
                }
                ulong a = v1, b = v2, c = v3, d = v4;
                byte* limit = end - 32;
                while (p <= limit)
                {
                    a = Round(a, *(ulong*)p); b = Round(b, *(ulong*)(p + 8));
                    c = Round(c, *(ulong*)(p + 16)); d = Round(d, *(ulong*)(p + 24));
                    p += 32;
                }
                v1 = a; v2 = b; v3 = c; v4 = d;
                int rest = (int)(end - p);
                for (int i = 0; i < rest; i++) tail[i] = p[i];
                tailLen = rest;
            }
        }

        public ulong Digest()
        {
            unchecked
            {
                ulong h;
                if (total >= 32)
                {
                    h = Rotl(v1, 1) + Rotl(v2, 7) + Rotl(v3, 12) + Rotl(v4, 18);
                    h = Merge(h, v1); h = Merge(h, v2); h = Merge(h, v3); h = Merge(h, v4);
                }
                else h = v3 + P5;
                h += total;
                fixed (byte* t = tail)
                {
                    byte* p = t; byte* end = t + tailLen;
                    while (p + 8 <= end) { h ^= Round(0, *(ulong*)p); h = Rotl(h, 27) * P1 + P4; p += 8; }
                    if (p + 4 <= end) { h ^= (ulong)(*(uint*)p) * P1; h = Rotl(h, 23) * P2 + P3; p += 4; }
                    while (p < end) { h ^= (*p) * P5; h = Rotl(h, 11) * P1; p++; }
                }
                h ^= h >> 33; h *= P2; h ^= h >> 29; h *= P3; h ^= h >> 32;
                return h;
            }
        }

        public static ulong OfBytes(byte[] data)
        {
            var x = new XxHash64();
            fixed (byte* p = data) x.Update(p, data.Length);
            return x.Digest();
        }

        /// <summary>
        /// Hashes a file. noCache=true opens it unbuffered so the bytes really come from the disk/NAS,
        /// not from what Windows still holds in memory after writing it.
        /// progress gets the bytes read so far; returns false + error when it can't read.
        /// </summary>
        public static bool HashFile(string longPath, bool noCache, Func<long, bool> progress, out ulong hash, out int error)
        {
            hash = 0; error = 0;
            uint flags = Native.FILE_FLAG_SEQUENTIAL_SCAN | (noCache ? Native.FILE_FLAG_NO_BUFFERING | Native.FILE_FLAG_OVERLAPPED : 0);
            using (SafeFileHandle h = Native.CreateFileW(longPath, Native.GENERIC_READ,
                Native.FILE_SHARE_READ | Native.FILE_SHARE_WRITE | Native.FILE_SHARE_DELETE, IntPtr.Zero, Native.OPEN_EXISTING, flags, IntPtr.Zero))
            {
                if (h.IsInvalid) { error = Marshal.GetLastWin32Error(); return false; }
                long size;
                if (!Native.GetFileSizeEx(h, out size)) { error = Marshal.GetLastWin32Error(); return false; }
                var x = new XxHash64();
                long done = 0;
                // the next pieces are read while this one is hashed
                using (var rr = new RingReader(h, size, 8 << 20, 4, noCache, null))
                {
                    byte* buf; int read;
                    while (rr.Next(out buf, out read))
                    {
                        x.Update(buf, read);
                        rr.Release();
                        done += read;
                        if (progress != null && !progress(done)) { error = Native.ERROR_REQUEST_ABORTED; return false; }
                    }
                    if (rr.Error != 0) { error = rr.Error; return false; }
                }
                hash = x.Digest();
                return true;
            }
        }
    }
}
