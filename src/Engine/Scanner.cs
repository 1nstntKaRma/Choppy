using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Threading;

namespace Choppy.Engine
{
    /// <summary>
    /// Fast analysis: lists the sources (several folders at once on a network), then lists only the destination
    /// folders that already exist to find name conflicts - one listing per folder instead of one check per file.
    /// </summary>
    internal sealed class Scanner
    {
        public int FilesSeen;
        public long BytesSeen;
        public volatile string Stage = "Analyzing";

        readonly Func<bool> canceled;

        public Scanner(Func<bool> canceled) { this.canceled = canceled; }

        sealed class Node
        {
            public DirItem Dir;
            public List<FileItem> Files = new List<FileItem>();
            public List<Node> Subs = new List<Node>();
            public List<string> Notes = new List<string>();
            public string Error;
        }

        struct Entry
        {
            public bool IsDir;
            public long Size;
            public long MTime;
        }

        sealed class TopItem
        {
            public FileItem File;
            public Node Node;
        }

        public ScanResult Run(JobSpec spec, int sourceThreads, int destThreads)
        {
            var res = new ScanResult();
            var tops = new List<TopItem>();
            var dirRoots = new List<Node>();

            foreach (string src in spec.Sources)
            {
                Native.WIN32_FILE_ATTRIBUTE_DATA d;
                if (!Native.GetFileAttributesExW(src, 0, out d))
                {
                    res.Problems.Add("Not found: " + Paths.ToShort(src) + " - " + Native.ErrorText(Marshal.GetLastWin32Error()));
                    continue;
                }
                bool isDir = (d.dwFileAttributes & Native.FILE_ATTRIBUTE_DIRECTORY) != 0;
                string name = Paths.FileName(src);
                string parent = Paths.Parent(src);
                if (parent != null && Paths.SamePath(parent, spec.Dest))
                {
                    if (spec.Op == OpKind.Move) { res.Problems.Add("Already in this folder: " + Paths.ToShort(src)); continue; }
                    name = CopyName(spec.Dest, name, isDir);
                }
                if (isDir)
                {
                    if (Paths.IsSameOrInside(spec.Dest, src))
                    {
                        res.Problems.Add("A folder can't be copied into itself: " + Paths.ToShort(src));
                        continue;
                    }
                    var node = new Node
                    {
                        Dir = new DirItem { Src = src, Rel = name, Attr = d.dwFileAttributes, CTime = d.ftCreationTime, ATime = d.ftLastAccessTime, MTime = d.ftLastWriteTime, Depth = 0 }
                    };
                    tops.Add(new TopItem { Node = node });
                    dirRoots.Add(node);
                }
                else
                {
                    long size = ((long)d.nFileSizeHigh << 32) | d.nFileSizeLow;
                    var f = new FileItem { Src = src, Rel = name, Size = size, Attr = d.dwFileAttributes, CTime = d.ftCreationTime, ATime = d.ftLastAccessTime, MTime = d.ftLastWriteTime };
                    tops.Add(new TopItem { File = f });
                    Interlocked.Increment(ref FilesSeen);
                    Interlocked.Add(ref BytesSeen, size);
                }
            }

            // 1) list the source folders
            RunPool(dirRoots, Math.Max(1, sourceThreads), ListSource);
            if (canceled()) return res;

            // 2) conflicts: list destination folders that exist
            Stage = "Checking the destination";
            ScanDestination(spec.Dest, tops, Math.Max(1, destThreads));
            if (canceled()) return res;

            // 3) flatten in source order: files of a folder, then its subfolders (depth first)
            foreach (var t in tops)
            {
                if (t.File != null) res.Files.Add(t.File);
                else Flatten(t.Node, res);
            }

            long total = 0; int conflicts = 0;
            var sizes = new long[res.Files.Count];
            for (int i = 0; i < res.Files.Count; i++)
            {
                var f = res.Files[i];
                total += f.Size;
                sizes[i] = f.Size;
                if (f.DestExists) conflicts++;
            }
            res.TotalBytes = total;
            res.Conflicts = conflicts;
            if (sizes.Length > 0) { Array.Sort(sizes); res.MedianSize = sizes[sizes.Length / 2]; }
            res.Largest = TopLargest(res.Files, 4);
            return res;
        }

        static List<KeyValuePair<string, long>> TopLargest(List<FileItem> files, int n)
        {
            var top = new List<FileItem>(n + 1);
            foreach (var f in files)
            {
                if (top.Count < n || f.Size > top[top.Count - 1].Size)
                {
                    int i = top.Count;
                    while (i > 0 && top[i - 1].Size < f.Size) i--;
                    top.Insert(i, f);
                    if (top.Count > n) top.RemoveAt(n);
                }
            }
            var r = new List<KeyValuePair<string, long>>();
            foreach (var f in top) r.Add(new KeyValuePair<string, long>(f.Name, f.Size));
            return r;
        }

        void Flatten(Node n, ScanResult res)
        {
            res.Dirs.Add(n.Dir);
            if (n.Error != null) res.Problems.Add("Couldn't read folder " + Paths.ToShort(n.Dir.Src) + " - " + n.Error);
            res.Problems.AddRange(n.Notes);
            res.Files.AddRange(n.Files);
            foreach (var s in n.Subs) Flatten(s, res);
        }

        // ---- worker pool over a growing set of folders ----
        void RunPool(List<Node> roots, int threads, Func<Node, List<Node>> work)
        {
            if (roots.Count == 0) return;
            var q = new ConcurrentQueue<Node>(roots);
            int pending = roots.Count;
            if (threads <= 1)
            {
                // depth-first on this thread
                var stack = new Stack<Node>();
                for (int i = roots.Count - 1; i >= 0; i--) stack.Push(roots[i]);
                while (stack.Count > 0 && !canceled())
                {
                    var n = stack.Pop();
                    var subs = work(n);
                    for (int i = subs.Count - 1; i >= 0; i--) stack.Push(subs[i]);
                }
                return;
            }
            var done = new ManualResetEventSlim(false);
            var ts = new Thread[threads];
            for (int t = 0; t < threads; t++)
            {
                ts[t] = new Thread(() =>
                {
                    var spin = new SpinWait();
                    while (!done.IsSet)
                    {
                        Node n;
                        if (q.TryDequeue(out n))
                        {
                            spin.Reset();
                            List<Node> subs = canceled() ? new List<Node>() : work(n);
                            foreach (var s in subs) { Interlocked.Increment(ref pending); q.Enqueue(s); }
                            if (Interlocked.Decrement(ref pending) == 0) done.Set();
                        }
                        else if (Volatile.Read(ref pending) == 0) { done.Set(); }
                        else
                        {
                            if (spin.NextSpinWillYield) Thread.Sleep(1); else spin.SpinOnce();
                        }
                    }
                }) { IsBackground = true, Name = "scan" };
                ts[t].Start();
            }
            done.Wait();
            foreach (var t in ts) t.Join();
        }

        List<Node> ListSource(Node n)
        {
            var subs = new List<Node>();
            Native.WIN32_FIND_DATAW fd;
            IntPtr h = Native.FindFirstFileExW(Paths.Combine(n.Dir.Src, "*"), Native.FindExInfoBasic, out fd, Native.FindExSearchNameMatch, IntPtr.Zero, Native.FIND_FIRST_EX_LARGE_FETCH);
            if (h == Native.INVALID_HANDLE_VALUE)
            {
                int e = Marshal.GetLastWin32Error();
                if (e != Native.ERROR_FILE_NOT_FOUND) n.Error = Native.ErrorText(e);
                return subs;
            }
            try
            {
                do
                {
                    string name = fd.cFileName;
                    if (name == "." || name == "..") continue;
                    string full = Paths.Combine(n.Dir.Src, name);
                    string rel = n.Dir.Rel + "\\" + name;
                    if ((fd.dwFileAttributes & Native.FILE_ATTRIBUTE_DIRECTORY) != 0)
                    {
                        var dir = new DirItem { Src = full, Rel = rel, Attr = fd.dwFileAttributes, CTime = fd.ftCreationTime, ATime = fd.ftLastAccessTime, MTime = fd.ftLastWriteTime, Depth = n.Dir.Depth + 1 };
                        if ((fd.dwFileAttributes & Native.FILE_ATTRIBUTE_REPARSE_POINT) != 0)
                        {
                            // junctions / folder links are not followed (they can loop or point anywhere)
                            n.Notes.Add("Folder link not followed: " + Paths.ToShort(full));
                            continue;
                        }
                        var sub = new Node { Dir = dir };
                        n.Subs.Add(sub);
                        subs.Add(sub);
                    }
                    else
                    {
                        long size = ((long)fd.nFileSizeHigh << 32) | fd.nFileSizeLow;
                        n.Files.Add(new FileItem { Src = full, Rel = rel, Size = size, Attr = fd.dwFileAttributes, CTime = fd.ftCreationTime, ATime = fd.ftLastAccessTime, MTime = fd.ftLastWriteTime });
                        Interlocked.Increment(ref FilesSeen);
                        Interlocked.Add(ref BytesSeen, size);
                    }
                } while (Native.FindNextFileW(h, out fd));
            }
            finally { Native.FindClose(h); }
            return subs;
        }

        // ---- destination ----
        Dictionary<string, Entry> ListDest(string dirLong)
        {
            var map = new Dictionary<string, Entry>(StringComparer.OrdinalIgnoreCase);
            Native.WIN32_FIND_DATAW fd;
            IntPtr h = Native.FindFirstFileExW(Paths.Combine(dirLong, "*"), Native.FindExInfoBasic, out fd, Native.FindExSearchNameMatch, IntPtr.Zero, Native.FIND_FIRST_EX_LARGE_FETCH);
            if (h == Native.INVALID_HANDLE_VALUE) return map;
            try
            {
                do
                {
                    string name = fd.cFileName;
                    if (name == "." || name == "..") continue;
                    map[name] = new Entry
                    {
                        IsDir = (fd.dwFileAttributes & Native.FILE_ATTRIBUTE_DIRECTORY) != 0,
                        Size = ((long)fd.nFileSizeHigh << 32) | fd.nFileSizeLow,
                        MTime = fd.ftLastWriteTime
                    };
                } while (Native.FindNextFileW(h, out fd));
            }
            finally { Native.FindClose(h); }
            return map;
        }

        void Mark(FileItem f, Dictionary<string, Entry> map)
        {
            string name = f.Name;
            Entry e;
            if (map.TryGetValue(name, out e))
            {
                f.DestExists = true; f.DestIsDir = e.IsDir; f.DestSize = e.Size; f.DestMTime = e.MTime;
            }
            if (map.ContainsKey(name + Engine.CopyJob.TempSuffix)) f.TempExists = true;
        }

        void ScanDestination(string dest, List<TopItem> tops, int threads)
        {
            if (!Paths.DirExists(dest)) return;
            var rootMap = ListDest(dest);
            var roots = new List<Node>();
            foreach (var t in tops)
            {
                if (t.File != null) { Mark(t.File, rootMap); continue; }
                Entry e;
                if (rootMap.TryGetValue(t.Node.Dir.Rel, out e))
                {
                    if (e.IsDir) { t.Node.Dir.DestExisted = true; roots.Add(t.Node); }
                    else t.Node.Dir.DestIsFile = true;
                }
            }
            RunPool(roots, threads, n =>
            {
                var map = ListDest(Paths.Combine(dest, n.Dir.Rel));
                foreach (var f in n.Files) Mark(f, map);
                var next = new List<Node>();
                foreach (var s in n.Subs)
                {
                    Entry e;
                    string nm = Paths.FileName(s.Dir.Rel);
                    if (map.TryGetValue(nm, out e))
                    {
                        if (e.IsDir) { s.Dir.DestExisted = true; next.Add(s); }
                        else s.Dir.DestIsFile = true;
                    }
                }
                return next;
            });
        }

        /// <summary>Explorer-style name for copying into the same folder: "name - Copy", "name - Copy (2)".</summary>
        public static string CopyName(string destDir, string name, bool isDir)
        {
            string stem = name, ext = "";
            if (!isDir)
            {
                int dot = name.LastIndexOf('.');
                if (dot > 0) { stem = name.Substring(0, dot); ext = name.Substring(dot); }
            }
            string cand = stem + " - Copy" + ext;
            int n = 2;
            while (Paths.Exists(Paths.Combine(destDir, cand))) cand = stem + " - Copy (" + (n++) + ")" + ext;
            return cand;
        }
    }
}
