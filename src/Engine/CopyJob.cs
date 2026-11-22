using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

namespace Choppy.Engine
{
    /// <summary>One file in flight (for the "current file" card, Skip and progress).</summary>
    internal sealed class Transfer
    {
        public FileItem File;
        public long Done;            // bytes copied of this file
        public volatile bool Skip;
        public volatile bool Verifying;
        public long VerifyDone;      // bytes hashed (source + copy)
        public long Started;         // Stopwatch ticks
        public bool TimesSet;        // creation date already set through the copy's own handle
        public string RenameTo;      // plain final name: rename through the copy's handle once every byte is written
        public bool Renamed;
        public bool HasSrcHash;      // the pipeline hashed the source as it read it: verify reads only the copy
        public ulong SrcHash;
    }

    /// <summary>
    /// A copy or move job. Never loses a file:
    ///  - every file is written to "&lt;name&gt;.chpart" and renamed into place only when complete (and verified, if on),
    ///  - a move deletes a source file only after its copy is in place and verified,
    ///  - errors are retried with backoff; a network drop waits for the network to come back.
    /// </summary>
    internal sealed class CopyJob
    {
        public const string TempSuffix = ".chpart";

        public readonly JobSpec Spec;
        public Plan Plan;
        public VolumeInfo SrcVol, DstVol;
        public ScanResult Scan;
        public Scanner Scanner;

        // ---- live state (read by the UI) ----
        volatile JobPhase phase = JobPhase.Queued;
        public JobPhase Phase { get { return phase; } private set { phase = value; } }
        public volatile string Status;          // extra line: "Waiting for the network...", "The destination is full..."
        public volatile string RetryFile;       // a file waiting to be tried again (in use, network hiccup)
        public volatile string RetryWhy;
        public long TotalBytes, DoneBytes, SkippedBytes, FailedBytes;
        public int TotalFiles, DoneFiles, SkippedFiles, FailedFiles, VerifiedFiles, AlreadyThereFiles, ConflictsLeft;
        public long CopiedBytes;                // bytes of files that finished
        public readonly Stopwatch Clock = new Stopwatch();     // running time (stops while paused)
        public readonly Stopwatch CopyClock = new Stopwatch(); // copying time only (no analysis, no pauses): the average speed

        /// <summary>Average speed of this copy so far (bytes/s). Steadier than the live speed: the time left uses it.</summary>
        public double AverageSpeed
        {
            get
            {
                double s = CopyClock.Elapsed.TotalSeconds;
                if (s < 0.5) return 0;
                return Math.Max(0, Interlocked.Read(ref DoneBytes)) / s;
            }
        }
        public DateTime StartedAt, FinishedAt;
        public readonly List<FileItem> FailedItems = new List<FileItem>();
        public readonly List<FileItem> SkippedItems = new List<FileItem>();
        public readonly List<string> Notes = new List<string>();
        public string LogPath;
        public volatile bool Verify;            // effective for this job (copies: setting, can be switched live; moves: always)

        // speed history (bytes/s), one sample per 250 ms
        public readonly object SpeedLock = new object();
        public readonly List<double> SpeedHistory = new List<double>();   // last 600 samples (2.5 min)
        public long SpeedSamples;               // samples taken so far - keeps counting when the history is full
        public double SpeedNow, SpeedSmooth, SpeedPeak;
        public double SpeedSecond;              // average of the last second (4 samples): the "now" number, less jumpy

        // ---- callbacks from the UI ----
        public Func<ConflictInfo, ConflictAnswer> AskConflict;
        public Func<long, long, bool> AskLowSpace;          // (need, free) -> continue?
        public Func<bool> AskElevate;                       // the destination needs administrator rights: copy the rest as administrator?
        public volatile bool AsAdmin;                       // an administrator copy is doing the rest (in its own window)
        public int AdminFiles;                              // files the administrator copy did
        public event Action<CopyJob> Finished;
        public event Action<CopyJob> PhaseChanged;

        // ---- control ----
        volatile bool canceled;
        volatile bool paused;
        readonly ManualResetEventSlim runGate = new ManualResetEventSlim(true);
        readonly object conflictLock = new object();
        readonly object activeLock = new object();
        readonly List<Transfer> active = new List<Transfer>();
        readonly Dictionary<string, bool> knownDirs = new Dictionary<string, bool>(StringComparer.OrdinalIgnoreCase);
        readonly object dirLock = new object();
        readonly HashSet<string> createdDirs = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        volatile bool waitingNet;
        volatile bool fastPathOff;
        int fastChecks;
        List<FileItem> files;
        int nextIndex;
        int activeWorkers;
        int bigInFlight;
        readonly object bigLock = new object();
        int pendingBig, pendingSmall;
        Thread thread;
        double limitBps;
        double tokens; double tokenTime;
        readonly object tokenLock = new object();
        readonly Stopwatch tokenClock = Stopwatch.StartNew();
        int elevState;                                      // 0 = not asked yet, 1 = yes (hand over), 2 = no
        readonly object elevLock = new object();
        volatile bool handOff;                              // the rest goes to the administrator copy: no new files here

        public bool IsCanceled { get { return canceled; } }
        public bool IsPaused { get { return paused; } }
        public bool IsRunning { get { return thread != null && (Phase == JobPhase.Analyzing || Phase == JobPhase.Copying || Phase == JobPhase.Waiting || Phase == JobPhase.Finishing); } }
        public bool IsFinished { get { return Phase == JobPhase.Done || Phase == JobPhase.Canceled || Phase == JobPhase.Failed; } }

        public CopyJob(JobSpec spec)
        {
            Spec = spec;
            limitBps = spec.SpeedLimitMbps > 0 ? spec.SpeedLimitMbps * 1000000.0 / 8.0 : 0;
        }

        // ================= control =================

        public void Start()
        {
            if (thread != null) return;
            thread = new Thread(Run) { IsBackground = true, Name = "CopyJob " + Spec.Id };
            thread.Start();
        }

        public void Pause()
        {
            if (paused || IsFinished) return;
            paused = true;
            runGate.Reset();
            Clock.Stop();
            CopyClock.Stop();
            RaisePhase();
        }

        public void Resume()
        {
            if (!paused) return;
            paused = false;
            if (Status != null && Status.StartsWith("The destination is full")) Status = null;
            if (!IsFinished) Clock.Start();
            if (Phase == JobPhase.Copying || Phase == JobPhase.Waiting) CopyClock.Start();
            runGate.Set();
            RaisePhase();
        }

        public void Cancel()
        {
            canceled = true;
            runGate.Set();
            if (thread == null) { Phase = JobPhase.Canceled; RaisePhase(); }
        }

        public void SetSpeedLimit(double mbps)
        {
            limitBps = mbps > 0 ? mbps * 1000000.0 / 8.0 : 0;
        }

        /// <summary>Skips the file shown as "current file".</summary>
        public void SkipCurrent()
        {
            var t = CurrentTransfer();
            if (t != null) t.Skip = true;
        }

        public Transfer CurrentTransfer()
        {
            lock (activeLock)
            {
                Transfer best = null;
                foreach (var t in active) if (best == null || t.File.Size > best.File.Size) best = t;
                return best;
            }
        }

        public int ActiveCount { get { lock (activeLock) return active.Count; } }

        void RaisePhase()
        {
            var h = PhaseChanged;
            if (h != null) try { h(this); } catch { }
        }

        void SetPhase(JobPhase p)
        {
            Phase = p;
            RaisePhase();
        }

        // ================= main =================

        void Run()
        {
            StartedAt = DateTime.Now;
            Clock.Start();
            var monitor = new Thread(MonitorLoop) { IsBackground = true, Name = "CopyJob monitor" };
            try
            {
                Verify = Spec.Op == OpKind.Move || Spec.Verify;
                SetPhase(JobPhase.Analyzing);
                monitor.Start();

                SrcVol = Spec.Sources.Count > 0 ? VolumeProbe.Probe(Spec.Sources[0]) : null;
                bool allAsAdmin = false;
                if (!Paths.DirExists(Spec.Dest))
                {
                    int e = CreateDirTree(Spec.Dest);
                    // the destination folder can't even be made without administrator rights: the whole copy goes there
                    if (e == Native.ERROR_ACCESS_DENIED && AskToElevate()) allAsAdmin = true;
                    else if (e != 0) throw new JobError("Can't open the destination folder " + Paths.ToShort(Spec.Dest) + ": " + Native.ErrorText(e));
                }
                DstVol = VolumeProbe.Probe(Spec.Dest);

                Plan = Plan.Choose(Spec, SrcVol, DstVol, null);
                if (allAsAdmin && !canceled)
                {
                    TotalFiles = Spec.Sources.Count;
                    SetPhase(JobPhase.Copying);
                    if (!paused) CopyClock.Start();
                    RunElevated(Spec.ExplicitItems, Spec.ExplicitDirs, Spec.Sources);
                    CopyClock.Stop();
                }
                else if (Plan.InstantMove && !canceled)
                {
                    RunInstantMove();
                }
                else if (!canceled)
                {
                    RunCopy(Spec.Sources, Spec.ExplicitItems);
                }
                if (canceled) { SetPhase(JobPhase.Canceled); }
                else { SetPhase(JobPhase.Done); }
            }
            catch (JobError je)
            {
                Notes.Add(je.Message);
                Status = je.Message;
                SetPhase(canceled ? JobPhase.Canceled : JobPhase.Failed);
            }
            catch (Exception ex)
            {
                Notes.Add("Unexpected error: " + ex.Message);
                Status = "Unexpected error: " + ex.Message;
                SetPhase(JobPhase.Failed);
            }
            finally
            {
                Clock.Stop();
                CopyClock.Stop();
                FinishedAt = DateTime.Now;
                WriteLog();
                var h = Finished;
                if (h != null) try { h(this); } catch { }
            }
        }

        sealed class JobError : Exception { public JobError(string m) : base(m) { } }

        // ================= instant move (same drive) =================

        void RunInstantMove()
        {
            SetPhase(JobPhase.Copying);
            TotalFiles = Spec.Sources.Count;
            var fallback = new List<string>();
            var asAdmin = new List<string>();
            foreach (string src in Spec.Sources)
            {
                if (canceled) return;
                runGate.Wait();
                if (handOff) { asAdmin.Add(src); continue; }
                string name = Paths.FileName(src);
                string target = Paths.Combine(Spec.Dest, name);
                int a = Native.GetFileAttributesW(src);
                if (a == Native.INVALID_FILE_ATTRIBUTES)
                {
                    FailNote(src, Native.ErrorText(Marshal.GetLastWin32Error()));
                    continue;
                }
                bool isDir = (a & (int)Native.FILE_ATTRIBUTE_DIRECTORY) != 0;
                if (Paths.SamePath(src, target)) { Notes.Add("Already in this folder: " + Paths.ToShort(src)); Interlocked.Increment(ref SkippedFiles); continue; }
                if (isDir && Paths.IsSameOrInside(Spec.Dest, src)) { FailNote(src, "A folder can't be moved into itself."); continue; }
                if (MoveItem(src, target, isDir, fallback)) Interlocked.Increment(ref DoneFiles);
                else asAdmin.Add(src);
            }
            if (asAdmin.Count > 0 && !canceled)
            {
                // the rest is renamed in place by an administrator copy
                RunElevated(null, null, asAdmin);
            }
            if (fallback.Count > 0 && !canceled)
            {
                // something on this "same drive" refused a rename (another volume mounted inside): copy + delete instead
                Plan = Plan.Choose(new JobSpec { Op = OpKind.Copy }, SrcVol, DstVol, null);
                RunCopy(fallback, null);
            }
        }

        /// <summary>False = the destination needs administrator rights and the user said yes: an administrator copy moves it.</summary>
        bool MoveItem(string src, string target, bool isDir, List<string> fallback)
        {
            if (canceled) return true;
            int ta = Native.GetFileAttributesW(target);
            if (ta == Native.INVALID_FILE_ATTRIBUTES)
            {
                if (Native.MoveFileExW(src, target, 0)) return true;
                int e = Marshal.GetLastWin32Error();
                if (e == 17 /*ERROR_NOT_SAME_DEVICE*/) { fallback.Add(src); return true; }
                if (e == Native.ERROR_ACCESS_DENIED && DestRefuses(Paths.Parent(target)) && HandOver(null)) return false;
                FailNote(src, Native.ErrorText(e));
                return true;
            }
            bool targetIsDir = (ta & (int)Native.FILE_ATTRIBUTE_DIRECTORY) != 0;
            if (isDir && targetIsDir)
            {
                // merge folders: move the children one by one. One the destination refuses: the administrator copy
                // merges what is left of this whole item
                foreach (var child in ListNames(src))
                {
                    if (canceled) return true;
                    if (!MoveItem(Paths.Combine(src, child.Key), Paths.Combine(target, child.Key), child.Value, fallback)) return false;
                }
                Native.RemoveDirectoryW(src);
                return true;
            }
            // name conflict
            var fi = new FileItem { Src = src, Rel = Paths.FileName(target) };
            Native.WIN32_FILE_ATTRIBUTE_DATA sd, dd;
            if (Native.GetFileAttributesExW(src, 0, out sd)) { fi.Size = ((long)sd.nFileSizeHigh << 32) | sd.nFileSizeLow; fi.MTime = sd.ftLastWriteTime; }
            if (Native.GetFileAttributesExW(target, 0, out dd)) { fi.DestExists = true; fi.DestIsDir = targetIsDir; fi.DestSize = ((long)dd.nFileSizeHigh << 32) | dd.nFileSizeLow; fi.DestMTime = dd.ftLastWriteTime; }
            Interlocked.Increment(ref ConflictsLeft);
            var choice = ResolveConflict(fi, target);
            switch (choice)
            {
                case ConflictPolicy.Skip:
                    Interlocked.Increment(ref SkippedFiles); lock (SkippedItems) SkippedItems.Add(fi);
                    return true;
                case ConflictPolicy.KeepBoth:
                    string free = FreeName(target, isDir);
                    if (!Native.MoveFileExW(src, free, 0)) FailNote(src, Native.ErrorText(Marshal.GetLastWin32Error()));
                    return true;
                default: // Replace
                    if (isDir || targetIsDir) { FailNote(src, "A file and a folder have the same name - kept both untouched."); return true; }
                    ClearReadOnly(target);
                    if (!Native.MoveFileExW(src, target, Native.MOVEFILE_REPLACE_EXISTING)) FailNote(src, Native.ErrorText(Marshal.GetLastWin32Error()));
                    return true;
            }
        }

        static List<KeyValuePair<string, bool>> ListNames(string dir)
        {
            var r = new List<KeyValuePair<string, bool>>();
            Native.WIN32_FIND_DATAW fd;
            IntPtr h = Native.FindFirstFileExW(Paths.Combine(dir, "*"), Native.FindExInfoBasic, out fd, 0, IntPtr.Zero, Native.FIND_FIRST_EX_LARGE_FETCH);
            if (h == Native.INVALID_HANDLE_VALUE) return r;
            try
            {
                do
                {
                    if (fd.cFileName == "." || fd.cFileName == "..") continue;
                    r.Add(new KeyValuePair<string, bool>(fd.cFileName, (fd.dwFileAttributes & Native.FILE_ATTRIBUTE_DIRECTORY) != 0));
                } while (Native.FindNextFileW(h, out fd));
            }
            finally { Native.FindClose(h); }
            return r;
        }

        void FailNote(string src, string why)
        {
            var fi = new FileItem { Src = src, Rel = Paths.FileName(src), Error = why, State = (int)FileState.Failed };
            Interlocked.Increment(ref FailedFiles);
            lock (FailedItems) FailedItems.Add(fi);
        }

        // ================= copy (and cross-drive move) =================

        void RunCopy(List<string> sources, List<FileItem> explicitItems)
        {
            SetPhase(JobPhase.Analyzing);
            Scanner = new Scanner(() => canceled);
            int srcThreads = SrcVol != null && SrcVol.IsNetwork ? 8 : 1;
            int dstThreads = DstVol != null && DstVol.IsNetwork ? 8 : 2;
            if (explicitItems != null)
            {
                Scan = new ScanResult();
                foreach (var f in explicitItems)
                {
                    var c = new FileItem { Src = f.Src, Rel = f.Rel, Size = f.Size, Attr = f.Attr, CTime = f.CTime, ATime = f.ATime, MTime = f.MTime };
                    Native.WIN32_FILE_ATTRIBUTE_DATA d;
                    if (Native.GetFileAttributesExW(c.Src, 0, out d)) { c.Size = ((long)d.nFileSizeHigh << 32) | d.nFileSizeLow; c.MTime = d.ftLastWriteTime; c.CTime = d.ftCreationTime; c.Attr = d.dwFileAttributes; }
                    Native.WIN32_FILE_ATTRIBUTE_DATA dd;
                    if (Native.GetFileAttributesExW(Paths.Combine(Spec.Dest, c.Rel), 0, out dd)) { c.DestExists = true; c.DestSize = ((long)dd.nFileSizeHigh << 32) | dd.nFileSizeLow; c.DestMTime = dd.ftLastWriteTime; }
                    Scan.Files.Add(c);
                    Scan.TotalBytes += c.Size;
                    if (c.DestExists) Scan.Conflicts++;
                }
                if (Spec.ExplicitDirs != null)
                    foreach (var d in Spec.ExplicitDirs)
                    {
                        var c = new DirItem { Src = d.Src, Rel = d.Rel, Depth = d.Depth };
                        Native.WIN32_FILE_ATTRIBUTE_DATA a;
                        if (Native.GetFileAttributesExW(c.Src, 0, out a)) { c.Attr = a.dwFileAttributes; c.CTime = a.ftCreationTime; c.MTime = a.ftLastWriteTime; }
                        int t = Native.GetFileAttributesW(Paths.Combine(Spec.Dest, c.Rel));
                        if (t != Native.INVALID_FILE_ATTRIBUTES)
                        {
                            if ((t & (int)Native.FILE_ATTRIBUTE_DIRECTORY) != 0) c.DestExisted = true; else c.DestIsFile = true;
                        }
                        Scan.Dirs.Add(c);
                    }
                var big = Scan.Files.OrderByDescending(x => x.Size).Take(4);
                foreach (var x in big) Scan.Largest.Add(new KeyValuePair<string, long>(x.Name, x.Size));
            }
            else
            {
                var sub = new JobSpec { Op = Spec.Op, Sources = sources, Dest = Spec.Dest };
                Scan = Scanner.Run(sub, srcThreads, dstThreads);
            }
            if (canceled) return;
            foreach (var p in Scan.Problems) Notes.Add(p);

            files = Scan.Files;
            TotalFiles = files.Count;
            TotalBytes = Scan.TotalBytes;
            ConflictsLeft = Scan.Conflicts;
            Plan = Plan.Choose(new JobSpec { Op = OpKind.Copy }, SrcVol, DstVol, Scan);
            if (Spec.ForceWorkers > 0) { Plan.Workers = Spec.ForceWorkers; Plan.CanTune = false; }
            int da = Native.GetFileAttributesW(Spec.Dest);
            pipeOffAtDest = da != Native.INVALID_FILE_ATTRIBUTES && ((uint)da & (Native.FILE_ATTRIBUTE_COMPRESSED | Native.FILE_ATTRIBUTE_ENCRYPTED)) != 0;

            // known destination folders
            lock (dirLock)
            {
                knownDirs[Spec.Dest] = true;
                foreach (var d in Scan.Dirs) if (d.DestExisted) knownDirs[Paths.Combine(Spec.Dest, d.Rel)] = true;
            }

            // space check (bytes that will land there; conflicts may overwrite, count them anyway)
            ulong free, total, totalFree;
            if (Native.GetDiskFreeSpaceExW(Paths.ToShort(Spec.Dest).TrimEnd('\\') + "\\", out free, out total, out totalFree))
            {
                long need = TotalBytes;
                if (need > 0 && (ulong)need > free && AskLowSpace != null)
                {
                    if (!AskLowSpace(need, (long)free)) { canceled = true; return; }
                }
            }

            foreach (var f in files) { if (f.Size >= Plan.BigThreshold) pendingBig++; else pendingSmall++; }

            SetPhase(JobPhase.Copying);
            if (!paused) CopyClock.Start();
            activeWorkers = Plan.Workers;
            int threads = Plan.CanTune ? 32 : Plan.Workers;
            var workers = new Thread[Math.Max(1, threads)];
            for (int i = 0; i < workers.Length; i++)
            {
                int idx = i;
                workers[i] = new Thread(() => WorkerLoop(idx)) { IsBackground = true, Name = "copy " + i };
                workers[i].Start();
            }
            foreach (var w in workers) w.Join();

            if (handOff && !canceled)
            {
                // the rest (files never started + those the destination refused) goes to the administrator copy
                var rest = files.Where(x => x.St == FileState.Pending).ToList();
                if (rest.Count > 0) RunElevated(rest, Scan.Dirs, null);
            }
            CopyClock.Stop();

            if (canceled) return;
            SetPhase(JobPhase.Finishing);
            FinishFolders();
        }

        void WorkerLoop(int idx)
        {
            while (!canceled)
            {
                if (idx >= Volatile.Read(ref activeWorkers))
                {
                    if (Volatile.Read(ref nextIndex) >= files.Count) return;
                    Thread.Sleep(100);
                    continue;
                }
                WaitToRun();
                if (canceled || handOff) return;
                int i = Interlocked.Increment(ref nextIndex) - 1;
                if (i >= files.Count) return;
                var f = files[i];
                bool big = f.Size >= Plan.BigThreshold;
                if (big) Interlocked.Decrement(ref pendingBig); else Interlocked.Decrement(ref pendingSmall);
                if (big)
                {
                    lock (bigLock)
                    {
                        while (bigInFlight >= Plan.BigSlots && !canceled) Monitor.Wait(bigLock, 200);
                        bigInFlight++;
                    }
                }
                try { ProcessFile(f); }
                catch (Exception ex) { MarkFailed(f, "Unexpected error: " + ex.Message); }
                finally
                {
                    if (big) lock (bigLock) { bigInFlight--; Monitor.PulseAll(bigLock); }
                }
            }
        }

        void WaitToRun()
        {
            while (!canceled && (paused || waitingNet))
            {
                runGate.Wait(250);
                if (waitingNet) Thread.Sleep(250);
            }
        }

        // ---------- one file ----------

        void ProcessFile(FileItem f)
        {
            f.St = FileState.Active;
            string final = Paths.Combine(Spec.Dest, f.Rel);
            string parent = Paths.Parent(final);

            // the destination folder for this file
            int de = EnsureDir(parent);
            if (de != 0)
            {
                if (IsNetError(de) && WaitForNetwork()) de = EnsureDir(parent);
                if (de == Native.ERROR_ACCESS_DENIED && HandOver(f)) return;
                if (de != 0) { MarkFailed(f, "Can't create the folder " + Paths.ToShort(parent) + ": " + Native.ErrorText(de)); return; }
            }

            bool replace = false;
            if (f.DestExists)
            {
                if (Spec.ResumeMode && !f.DestIsDir && f.DestSize == f.Size && f.DestMTime == f.MTime)
                {
                    // already copied by the interrupted run
                    f.AlreadyThere = true;
                    f.FinalPath = final;
                    Interlocked.Decrement(ref ConflictsLeft);
                    Interlocked.Add(ref DoneBytes, f.Size);
                    Interlocked.Add(ref CopiedBytes, f.Size);
                    Interlocked.Increment(ref AlreadyThereFiles);
                    Interlocked.Increment(ref DoneFiles);
                    f.St = FileState.Done;
                    if (Spec.Op == OpKind.Move) DeleteSource(f);
                    return;
                }
                var choice = ResolveConflict(f, final);
                if (canceled) return;
                if (choice == ConflictPolicy.Skip) { MarkSkipped(f); return; }
                if (choice == ConflictPolicy.KeepBoth) final = FreeName(final, false);
                else
                {
                    if (f.DestIsDir) { MarkFailed(f, "A folder with this name is already there."); return; }
                    replace = true;
                }
            }

            string temp = TempName(final);
            if (f.TempExists) DeleteQuiet(temp);

            int attempt = 0;
            bool verifyRetried = false;
            while (true)
            {
                if (canceled) return;
                WaitToRun();
                if (canceled) return;

                var t = new Transfer { File = f, Started = Stopwatch.GetTimestamp() };
                // fast path for a brand-new file without verify: one rename through the copy's own handle
                // instead of open + rename + close afterwards (saves round trips per file on a NAS)
                if (!replace && !Verify && !Spec.NoFastPath && !fastPathOff) t.RenameTo = final;
                lock (activeLock) active.Add(t);
                int err = 0;
                bool ok;
                try
                {
                    ok = CopyFile(f, temp, t, out err);
                }
                finally { lock (activeLock) active.Remove(t); }

                if (!ok)
                {
                    Interlocked.Add(ref DoneBytes, -t.Done);
                    // it had already taken its real name (a brand-new name, so the file there is ours): remove it
                    if (t.Renamed) DeleteQuiet(final);
                    if (err == Native.ERROR_REQUEST_ABORTED)
                    {
                        DeleteQuiet(temp);
                        if (canceled) return;
                        if (t.Skip) { MarkSkipped(f); return; }
                    }
                    if (err == Native.ERROR_FILE_EXISTS || err == Native.ERROR_ALREADY_EXISTS)
                    {
                        // a leftover temp from an earlier run: remove it and go again
                        DeleteQuiet(temp);
                        if (attempt++ < 3) continue;
                    }
                    if (err == Native.ERROR_DISK_FULL || err == Native.ERROR_HANDLE_DISK_FULL)
                    {
                        DeleteQuiet(temp);
                        Status = "The destination is full - free some space, then press Resume.";
                        Pause();
                        continue;
                    }
                    if (IsNetError(err) && attempt < 6)
                    {
                        attempt++;
                        if (!WaitForNetwork()) return;
                        Backoff(attempt, f, "The network hiccuped");
                        continue;
                    }
                    if (IsBusy(err) && attempt < 3)
                    {
                        attempt++;
                        Backoff(attempt, f, "In use by another program");
                        continue;
                    }
                    DeleteQuiet(temp);
                    // the destination folder refuses (only administrators may add files there), not the source
                    if (err == Native.ERROR_ACCESS_DENIED && DestRefuses(parent) && HandOver(f)) return;
                    MarkFailed(f, Native.ErrorText(err));
                    return;
                }

                // verify before the file gets its real name
                if (Verify)
                {
                    t.Verifying = true;
                    lock (activeLock) active.Add(t);
                    int ve;
                    bool same;
                    bool vok;
                    try { vok = VerifyOne(f, temp, t, out same, out ve); }
                    finally { lock (activeLock) active.Remove(t); t.Verifying = false; }
                    if (!vok)
                    {
                        if (ve == Native.ERROR_REQUEST_ABORTED)
                        {
                            DeleteQuiet(temp);
                            if (canceled) return;
                            if (t.Skip) { Interlocked.Add(ref DoneBytes, -t.Done); MarkSkipped(f); return; }
                        }
                        if (IsNetError(ve) && attempt < 6) { attempt++; if (!WaitForNetwork()) return; Backoff(attempt); Interlocked.Add(ref DoneBytes, -t.Done); DeleteQuiet(temp); continue; }
                        Interlocked.Add(ref DoneBytes, -t.Done);
                        DeleteQuiet(temp);
                        MarkFailed(f, "Couldn't check the copy: " + Native.ErrorText(ve));
                        return;
                    }
                    if (!same)
                    {
                        Interlocked.Add(ref DoneBytes, -t.Done);
                        DeleteQuiet(temp);
                        if (!verifyRetried) { verifyRetried = true; Notes.Add("Check failed once, copied again: " + Paths.ToShort(f.Src)); continue; }
                        MarkFailed(f, "The copy didn't match the original twice (xxHash64). Not kept.");
                        return;
                    }
                    f.Verified = true;
                    Interlocked.Increment(ref VerifiedFiles);
                }

                if (t.Renamed)
                {
                    // already under its real name (renamed through the copy's handle after the last byte).
                    // Guard: the first few of every job are checked where they must be; if one isn't, the fast path is off.
                    if (Interlocked.Increment(ref fastChecks) <= 3 && !Paths.Exists(final))
                    {
                        fastPathOff = true;
                        lock (Notes) Notes.Add("Fast rename switched off: a file wasn't where expected (" + Paths.ToShort(final) + ").");
                        MarkFailed(f, "The copy wasn't found under its final name - not counted as done.");
                        return;
                    }
                    if (!t.TimesSet) SetTimes(final, f.CTime, f.MTime, false);
                    f.FinalPath = final;
                    f.St = FileState.Done;
                    Interlocked.Add(ref CopiedBytes, f.Size);
                    Interlocked.Increment(ref DoneFiles);
                    if (Spec.Op == OpKind.Move) DeleteSource(f);
                    return;
                }

                // keep the original creation date too (Windows' copy keeps only the modified date)
                if (!t.TimesSet) SetTimes(temp, f.CTime, f.MTime, false);

                // into place
                int re = RenameIntoPlace(temp, ref final, replace);
                if (re != 0)
                {
                    if (IsNetError(re) && attempt < 6)
                    {
                        attempt++;
                        if (!WaitForNetwork()) return;
                        Backoff(attempt);
                        re = RenameIntoPlace(temp, ref final, replace);
                    }
                    if (re != 0)
                    {
                        Interlocked.Add(ref DoneBytes, -t.Done);
                        DeleteQuiet(temp);
                        MarkFailed(f, "Couldn't put the file in place: " + Native.ErrorText(re));
                        return;
                    }
                }
                // done
                f.FinalPath = final;
                f.St = FileState.Done;
                Interlocked.Add(ref CopiedBytes, f.Size);
                Interlocked.Increment(ref DoneFiles);
                if (replace) Interlocked.Decrement(ref ConflictsLeft);
                if (Spec.Op == OpKind.Move) DeleteSource(f);
                return;
            }
        }

        /// <summary>Big local files take the pipeline; everything else (and whatever the pipeline can't take) CopyFileEx.</summary>
        bool CopyFile(FileItem f, string temp, Transfer t, out int err)
        {
            if (UsePipe(f))
            {
                if (CopyBig(f, temp, t, out err)) return true;
                if (err != BigCopy.NotSupported) return false;
                // this file can't go that way (a file system or driver that refuses unbuffered I/O): CopyFileEx copies it
                Interlocked.Add(ref DoneBytes, -t.Done);
                t.Done = 0;
                DeleteQuiet(temp);
            }
            return CopyOne(f, temp, t, out err);
        }

        bool pipeOffAtDest;   // the destination folder is compressed or encrypted: the pipeline would fight it

        bool UsePipe(FileItem f)
        {
            return Plan.Pipeline && Spec.Engine == null && !pipeOffAtDest && f.Size >= Plan.PipeThreshold && (f.Attr & BigCopy.SkipAttrs) == 0;
        }

        bool CopyBig(FileItem f, string temp, Transfer t, out int err)
        {
            var tune = Plan.Pipe;
            if (Spec.PipeChunkMB > 0 || Spec.PipeSlots > 0)
                tune = new PipeTuning { Align = tune.Align, Chunk = Spec.PipeChunkMB > 0 ? Spec.PipeChunkMB << 20 : tune.Chunk, Slots = Spec.PipeSlots > 0 ? Spec.PipeSlots : tune.Slots };
            Func<long, bool> progress = done =>
            {
                long delta = done - t.Done;
                if (delta != 0)
                {
                    t.Done = done;
                    Interlocked.Add(ref DoneBytes, delta);
                    if (delta > 0) Throttle(delta);
                }
                if (canceled || t.Skip) return false;
                if (paused)
                {
                    while (paused && !canceled && !t.Skip) runGate.Wait(200);
                    if (canceled || t.Skip) return false;
                }
                return true;
            };
            // the last big file of a copy: its end goes through the Windows cache, so the copy is done when the
            // last bytes are in memory - like Explorer's copy (a move and a checked copy read the disk, they don't)
            long tail = 0;
            if (Spec.TailMB >= 0) tail = (long)Spec.TailMB << 20;
            // never for a USB stick: it may be pulled out right after "done"
            else if (!Verify && Spec.Op == OpKind.Copy && !Spec.NoCachedTail && (DstVol == null || DstVol.Kind != DeviceKind.Usb))
            {
                long after = Interlocked.Read(ref TotalBytes) - Interlocked.Read(ref DoneBytes) - (f.Size - t.Done);
                if (after < (64L << 20)) tail = CacheBudget();
            }
            // a move flushes the copy to the disk before the original can go
            var r = BigCopy.Copy(f.Src, temp, f.CTime, f.MTime, f.Attr, tune, Verify, Spec.Op == OpKind.Move, t.RenameTo, tail, progress);
            err = r.Error;
            if (err != 0) return false;
            t.TimesSet = true;
            t.Renamed = r.Renamed;
            if (Verify) { t.HasSrcHash = true; t.SrcHash = r.Hash; }
            if (r.Bytes != f.Size)
            {
                Interlocked.Add(ref TotalBytes, r.Bytes - f.Size);
                f.Size = r.Bytes;
            }
            return true;
        }

        /// <summary>How much of a file's end may wait in memory: a quarter of the free memory, at most 2 GB.</summary>
        static long CacheBudget()
        {
            var m = new Native.MEMORYSTATUSEX { dwLength = (uint)Marshal.SizeOf(typeof(Native.MEMORYSTATUSEX)) };
            if (!Native.GlobalMemoryStatusEx(ref m)) return 0;
            long b = (long)(m.ullAvailPhys / 4);
            return b < (64L << 20) ? 0 : Math.Min(2L << 30, b);
        }

        bool CopyOne(FileItem f, string temp, Transfer t, out int err)
        {
            err = 0;
            uint flags = Native.COPY_FILE_FAIL_IF_EXISTS | Native.COPY_FILE_ALLOW_DECRYPTED_DESTINATION;
            if (Spec.Engine == "nobuf" && f.Size >= (256L << 20)) flags |= Native.COPY_FILE_NO_BUFFERING;   // tests: the old way, to compare
            int cancelFlag = 0;
            Native.CopyProgressRoutine cb = (total, transferred, ss, sbt, sn, reason, hs, hd, data) =>
            {
                long delta = transferred - t.Done;
                if (delta != 0)
                {
                    t.Done = transferred;
                    Interlocked.Add(ref DoneBytes, delta);
                    if (delta > 0) Throttle(delta);
                }
                if (canceled || t.Skip) return Native.PROGRESS_CANCEL;
                if (paused)
                {
                    while (paused && !canceled && !t.Skip) runGate.Wait(200);
                    if (canceled || t.Skip) return Native.PROGRESS_CANCEL;
                }
                if (hd != IntPtr.Zero)
                {
                    // original creation date, set on the copy's own open handle (no extra round trip later)
                    if (!t.TimesSet && reason == Native.CALLBACK_STREAM_SWITCH && f.CTime != 0)
                    {
                        long c = f.CTime;
                        if (Native.SetFileTime(hd, ref c, IntPtr.Zero, IntPtr.Zero)) t.TimesSet = true;
                    }
                    // every byte is written: give it its real name now (never replaces anything)
                    if (t.RenameTo != null && !t.Renamed && transferred >= total)
                    {
                        if (Native.RenameByHandle(hd, t.RenameTo)) t.Renamed = true;
                        else t.RenameTo = null;
                    }
                }
                return Native.PROGRESS_CONTINUE;
            };
            bool ok = Native.CopyFileExW(f.Src, temp, cb, IntPtr.Zero, ref cancelFlag, flags);
            if (!ok) err = Marshal.GetLastWin32Error();
            GC.KeepAlive(cb);
            if (ok && t.Done < f.Size)
            {
                // the file grew/shrank between analysis and copy: count what was really copied
                Interlocked.Add(ref TotalBytes, t.Done - f.Size);
                f.Size = t.Done;
            }
            return ok;
        }

        bool VerifyOne(FileItem f, string temp, Transfer t, out bool same, out int err)
        {
            same = false; err = 0;
            ulong h1, h2;
            long half;
            if (t.HasSrcHash)
            {
                // hashed on the way in: only the copy is read back (from the disk, not the cache)
                h1 = t.SrcHash;
                half = f.Size;
                t.VerifyDone = half;
            }
            else
            {
                Func<long, bool> prog = n => { t.VerifyDone = n; WaitToRunInCallback(t); return !(canceled || t.Skip); };
                if (!XxHash64.HashFile(f.Src, false, prog, out h1, out err)) return false;
                half = t.VerifyDone;
            }
            Func<long, bool> prog2 = n => { t.VerifyDone = half + n; WaitToRunInCallback(t); return !(canceled || t.Skip); };
            if (!XxHash64.HashFile(temp, true, prog2, out h2, out err)) return false;
            same = h1 == h2;
            return true;
        }

        void WaitToRunInCallback(Transfer t)
        {
            while (paused && !canceled && !t.Skip) runGate.Wait(200);
        }

        int RenameIntoPlace(string temp, ref string final, bool replace)
        {
            for (int i = 0; i < 4; i++)
            {
                if (Native.MoveFileExW(temp, final, replace ? Native.MOVEFILE_REPLACE_EXISTING : 0)) return 0;
                int e = Marshal.GetLastWin32Error();
                if (!replace && (e == Native.ERROR_ALREADY_EXISTS || e == Native.ERROR_FILE_EXISTS))
                {
                    // something new appeared under that name while we copied: keep both, never overwrite silently
                    final = FreeName(final, false);
                    continue;
                }
                if (replace && e == Native.ERROR_ACCESS_DENIED && ClearReadOnly(final)) continue;
                if (IsBusy(e)) { Thread.Sleep(500 * (i + 1)); continue; }
                return e;
            }
            return Marshal.GetLastWin32Error() == 0 ? Native.ERROR_ACCESS_DENIED : Marshal.GetLastWin32Error();
        }

        void DeleteSource(FileItem f)
        {
            if (Native.DeleteFileW(f.Src)) return;
            int e = Marshal.GetLastWin32Error();
            if (e == Native.ERROR_ACCESS_DENIED && ClearReadOnly(f.Src) && Native.DeleteFileW(f.Src)) return;
            if (e == Native.ERROR_FILE_NOT_FOUND) return;
            f.SourceKept = true;
            lock (Notes) Notes.Add("Moved, but the original couldn't be removed: " + Paths.ToShort(f.Src) + " - " + Native.ErrorText(e));
        }

        // ---------- administrator rights ----------

        /// <summary>Asks once per copy (the first file the destination refuses). True = hand the rest over.</summary>
        bool AskToElevate()
        {
            if (AskElevate == null || Spec.Elevated || Handoff.IsAdmin || canceled) return false;
            lock (elevLock)
            {
                if (elevState == 0) elevState = AskElevate() ? 1 : 2;
                return elevState == 1;
            }
        }

        /// <summary>The destination refused this file for lack of rights: true = it goes to the administrator copy.</summary>
        bool HandOver(FileItem f)
        {
            if (!AskToElevate()) return false;
            if (f != null) f.St = FileState.Pending;
            handOff = true;
            return true;
        }

        /// <summary>True when a new file can't be made in this folder because of rights (as opposed to an unreadable source).</summary>
        static bool DestRefuses(string dir)
        {
            if (string.IsNullOrEmpty(dir)) return false;
            string probe = Paths.Combine(dir, "~ch-" + Guid.NewGuid().ToString("N").Substring(0, 8) + ".tmp");
            using (var h = Native.CreateFileW(probe, Native.GENERIC_WRITE, 0, IntPtr.Zero, Native.CREATE_NEW, 0x100 /*TEMPORARY*/ | 0x04000000 /*DELETE_ON_CLOSE*/, IntPtr.Zero))
            {
                if (!h.IsInvalid) return false;
                return Marshal.GetLastWin32Error() == Native.ERROR_ACCESS_DENIED;
            }
        }

        /// <summary>
        /// Runs the rest of this copy in Choppy started as administrator (its own window) and mirrors its progress
        /// here until it ends; then its counts, problems and notes join this copy's report.
        /// items + dirs: the files left (and the folders around them); sources: whole items (same-drive move, or nothing started).
        /// </summary>
        void RunElevated(List<FileItem> items, List<DirItem> dirs, List<string> sources)
        {
            var spec = new JobSpec
            {
                Id = Spec.Id + "a",
                Op = Spec.Op,
                Dest = Spec.Dest,
                Verify = Verify && Spec.Op == OpKind.Copy,
                Policy = Spec.Policy,
                SpeedLimitMbps = limitBps > 0 ? limitBps * 8 / 1e6 : 0,
                Origin = "Administrator",
                Engine = Spec.Engine,
                Sources = sources != null ? sources.ToList() : Spec.Sources.ToList()
            };
            long handedBytes = 0;
            if (items != null)
            {
                spec.ExplicitItems = items.Select(x => new FileItem { Src = x.Src, Rel = x.Rel, Size = x.Size }).ToList();
                spec.ExplicitDirs = dirs != null ? dirs.Select(d => new DirItem { Src = d.Src, Rel = d.Rel, Depth = d.Depth }).ToList() : null;
                foreach (var x in items) handedBytes += x.Size;
            }
            string record;
            try { record = Handoff.Save(spec); }
            catch (Exception ex) { HandBack(items, sources, "Couldn't write the administrator copy's record: " + ex.Message); return; }

            Process p;
            try { p = Handoff.Launch(record); }
            catch (Exception ex)
            {
                Handoff.Clean(record, true);
                HandBack(items, sources, Handoff.Declined(ex) ? "Administrator rights weren't given." : "Couldn't start Choppy as administrator: " + ex.Message);
                return;
            }

            AsAdmin = true;
            Status = "Copying as administrator in its own window";
            RaisePhase();
            long baseDone = Interlocked.Read(ref DoneBytes);
            int baseFiles = DoneFiles;
            var mirror = new Transfer { File = new FileItem { Rel = "", Src = "" }, Started = Stopwatch.GetTimestamp() };
            bool mirrored = false;
            int seq = 0, ownerPid = 0;
            bool sentPause = false, sentCancel = false;
            Handoff.Result res = null;
            var quiet = Stopwatch.StartNew();
            try
            {
                while (true)
                {
                    res = Handoff.ReadResult(record);
                    if (res != null) break;
                    var pr = Handoff.ReadProgress(record);
                    if (pr != null)
                    {
                        string v;
                        long n;
                        if (pr.TryGetValue("pid", out v)) int.TryParse(v, out ownerPid);
                        if (pr.TryGetValue("doneBytes", out v) && long.TryParse(v, out n)) Interlocked.Exchange(ref DoneBytes, baseDone + n);
                        if (pr.TryGetValue("doneFiles", out v) && long.TryParse(v, out n)) DoneFiles = baseFiles + (int)n;
                        string cur;
                        if (pr.TryGetValue("cur", out cur) && cur.Length > 0)
                        {
                            long cs = 0, cd = 0;
                            if (pr.TryGetValue("curSize", out v)) long.TryParse(v, out cs);
                            if (pr.TryGetValue("curDone", out v)) long.TryParse(v, out cd);
                            if (mirror.File.Rel != cur) mirror.File = new FileItem { Rel = cur, Src = pr.TryGetValue("curSrc", out v) ? v : cur, Size = cs };
                            mirror.File.Size = cs;
                            mirror.Done = cd;
                            mirror.Verifying = pr.TryGetValue("verifying", out v) && v == "1";
                            if (!mirrored) { lock (activeLock) active.Add(mirror); mirrored = true; }
                        }
                        else if (mirrored) { lock (activeLock) active.Remove(mirror); mirrored = false; }
                        Status = pr.TryGetValue("paused", out v) && v == "1" ? "Paused in the administrator window" : "Copying as administrator in its own window";
                        quiet.Restart();
                    }
                    // Pause / Resume / Cancel here reach it too
                    if (canceled && !sentCancel) { Handoff.Control(record, ++seq, "cancel"); sentCancel = true; }
                    else if (!canceled && paused != sentPause) { Handoff.Control(record, ++seq, paused ? "pause" : "run"); sentPause = paused; }
                    // it ended (or was closed) without leaving a result
                    bool alive = !p.HasExited || Handoff.Alive(ownerPid);
                    if (!alive && quiet.Elapsed.TotalSeconds > 2)
                    {
                        res = Handoff.ReadResult(record);
                        break;
                    }
                    Thread.Sleep(200);
                }
            }
            finally
            {
                if (mirrored) lock (activeLock) active.Remove(mirror);
                AsAdmin = false;
                p.Dispose();
            }

            if (res == null)
            {
                Handoff.Clean(record, true);
                Status = null;
                HandBack(items, sources, "The administrator copy stopped before it finished.");
                return;
            }
            // its numbers join this copy's
            Interlocked.Exchange(ref DoneBytes, baseDone + res.DoneBytes);
            DoneFiles = baseFiles + res.DoneFiles;
            if (items != null)
            {
                Interlocked.Add(ref TotalBytes, res.TotalBytes - handedBytes);
                TotalFiles += res.TotalFiles - items.Count;
                foreach (var x in items) x.St = FileState.Done;   // the ones that weren't are in its failed / skipped lists
            }
            Interlocked.Add(ref CopiedBytes, res.CopiedBytes);
            AdminFiles += res.DoneFiles;
            VerifiedFiles += res.Verified;
            AlreadyThereFiles += res.Already;
            SkippedFiles += res.Skipped;
            Interlocked.Add(ref SkippedBytes, res.SkippedBytes);
            FailedFiles += res.Failed;
            Interlocked.Add(ref FailedBytes, res.FailedBytes);
            lock (FailedItems) FailedItems.AddRange(res.FailedItems);
            lock (SkippedItems) SkippedItems.AddRange(res.SkippedItems);
            lock (Notes)
            {
                Notes.Add((Spec.Op == OpKind.Move ? "Moved" : "Copied") + " as administrator: " + res.DoneFiles + (res.DoneFiles == 1 ? " file" : " files") + (res.Log != null ? " (its log: " + res.Log + ")" : ""));
                foreach (var n in res.Notes) Notes.Add("As administrator: " + n);
            }
            Status = res.Phase == "Failed" ? res.Status : null;
            if (res.Phase == "Canceled") canceled = true;
            Handoff.Clean(record, true);
        }

        /// <summary>No administrator copy after all: what was meant for it is reported as failed, like before.</summary>
        void HandBack(List<FileItem> items, List<string> sources, string why)
        {
            lock (Notes) Notes.Add(why);
            if (items != null) foreach (var f in items) MarkFailed(f, "Access denied. " + why);
            else if (sources != null) foreach (var s in sources) FailNote(s, "Access denied. " + why);
        }

        // ---------- conflicts ----------

        ConflictPolicy ResolveConflict(FileItem f, string final)
        {
            lock (conflictLock)
            {
                ConflictPolicy p = Spec.Policy;
                if (p == ConflictPolicy.Ask)
                {
                    if (AskConflict == null) p = ConflictPolicy.Skip;
                    else
                    {
                        var info = new ConflictInfo
                        {
                            File = f,
                            IncomingPath = Paths.ToShort(f.Src),
                            ExistingPath = Paths.ToShort(final),
                            IncomingSize = f.Size,
                            ExistingSize = f.DestSize,
                            IncomingTime = FileTimes.ToLocal(f.MTime),
                            ExistingTime = FileTimes.ToLocal(f.DestMTime),
                            Remaining = Math.Max(1, Volatile.Read(ref ConflictsLeft)),
                            Op = Spec.Op
                        };
                        bool wasRunning = Clock.IsRunning;
                        var a = AskConflict(info);
                        if (a == null || a.CancelJob) { Cancel(); return ConflictPolicy.Skip; }
                        p = a.Choice;
                        if (a.ApplyToAll) Spec.Policy = p;
                    }
                }
                if (p == ConflictPolicy.ReplaceOlder)
                    p = f.MTime > f.DestMTime ? ConflictPolicy.Replace : ConflictPolicy.Skip;
                if (p != ConflictPolicy.Replace) Interlocked.Decrement(ref ConflictsLeft);
                return p;
            }
        }

        static string FreeName(string path, bool isDir)
        {
            string dir = Paths.Parent(path), name = Paths.FileName(path);
            for (int n = 2; n < 10000; n++)
            {
                string cand = Paths.Combine(dir, Paths.NumberedName(name, n, isDir));
                if (!Paths.Exists(cand) && !Paths.Exists(cand + TempSuffix)) return cand;
            }
            return Paths.Combine(dir, Paths.NumberedName(name, (int)(DateTime.Now.Ticks % 100000), isDir));
        }

        public static string TempName(string final)
        {
            string name = Paths.FileName(final);
            if (name.Length + TempSuffix.Length <= 255) return final + TempSuffix;
            ulong h = XxHash64.OfBytes(Encoding.UTF8.GetBytes(name));
            return Paths.Combine(Paths.Parent(final), "~ch" + h.ToString("x16") + TempSuffix);
        }

        // ---------- bookkeeping ----------

        void MarkSkipped(FileItem f)
        {
            f.St = FileState.Skipped;
            Interlocked.Increment(ref SkippedFiles);
            Interlocked.Add(ref SkippedBytes, f.Size);
            lock (SkippedItems) SkippedItems.Add(f);
        }

        void MarkFailed(FileItem f, string why)
        {
            f.St = FileState.Failed;
            f.Error = why;
            Interlocked.Increment(ref FailedFiles);
            Interlocked.Add(ref FailedBytes, f.Size);
            lock (FailedItems) FailedItems.Add(f);
        }

        static void DeleteQuiet(string path)
        {
            if (!Native.DeleteFileW(path))
            {
                if (Marshal.GetLastWin32Error() == Native.ERROR_ACCESS_DENIED && ClearReadOnly(path)) Native.DeleteFileW(path);
            }
        }

        static bool ClearReadOnly(string path)
        {
            int a = Native.GetFileAttributesW(path);
            if (a == Native.INVALID_FILE_ATTRIBUTES || (a & (int)Native.FILE_ATTRIBUTE_READONLY) == 0) return false;
            return Native.SetFileAttributesW(path, (uint)a & ~Native.FILE_ATTRIBUTE_READONLY);
        }

        static void SetTimes(string path, long ctime, long mtime, bool dir)
        {
            uint flags = dir ? Native.FILE_FLAG_BACKUP_SEMANTICS : 0;
            using (var h = Native.CreateFileW(path, Native.FILE_WRITE_ATTRIBUTES, Native.FILE_SHARE_READ | Native.FILE_SHARE_WRITE | Native.FILE_SHARE_DELETE, IntPtr.Zero, Native.OPEN_EXISTING, flags, IntPtr.Zero))
            {
                if (h.IsInvalid) return;
                long c = ctime, m = mtime;
                Native.SetFileTime(h, ref c, IntPtr.Zero, ref m);
            }
        }

        // ---------- folders ----------

        int EnsureDir(string dir)
        {
            lock (dirLock) { if (knownDirs.ContainsKey(dir)) return 0; }
            int e = CreateDirTree(dir);
            if (e == 0) lock (dirLock) knownDirs[dir] = true;
            return e;
        }

        int CreateDirTree(string dir)
        {
            if (Native.CreateDirectoryW(dir, IntPtr.Zero)) { lock (dirLock) createdDirs.Add(dir); return 0; }
            int e = Marshal.GetLastWin32Error();
            if (e == Native.ERROR_ALREADY_EXISTS) return Paths.DirExists(dir) ? 0 : Native.ERROR_ALREADY_EXISTS;
            if (e == Native.ERROR_PATH_NOT_FOUND)
            {
                string p = Paths.Parent(dir);
                if (p == null || p.Length <= Paths.LongPrefix.Length + 3) return e;
                int pe = CreateDirTree(p);
                if (pe != 0) return pe;
                if (Native.CreateDirectoryW(dir, IntPtr.Zero)) { lock (dirLock) createdDirs.Add(dir); return 0; }
                e = Marshal.GetLastWin32Error();
                if (e == Native.ERROR_ALREADY_EXISTS) return 0;
            }
            return e;
        }

        void FinishFolders()
        {
            if (Scan == null) return;
            // empty folders exist too, folders get their original dates and attributes
            var dirs = Scan.Dirs;
            var work = new List<DirItem>();
            foreach (var d in dirs)
            {
                if (canceled) return;
                if (d.DestIsFile) { Notes.Add("A file with this folder's name is in the way: " + Paths.ToShort(Paths.Combine(Spec.Dest, d.Rel))); continue; }
                string target = Paths.Combine(Spec.Dest, d.Rel);
                if (!d.DestExisted) { EnsureDir(target); work.Add(d); }
            }
            int threads = DstVol != null && DstVol.IsNetwork ? 8 : 1;
            int next = -1;
            var ts = new List<Thread>();
            for (int i = 0; i < Math.Min(threads, Math.Max(1, work.Count)); i++)
            {
                var th = new Thread(() =>
                {
                    int k;
                    while ((k = Interlocked.Increment(ref next)) < work.Count && !canceled)
                    {
                        var d = work[k];
                        string target = Paths.Combine(Spec.Dest, d.Rel);
                        SetTimes(target, d.CTime, d.MTime, true);
                        uint keep = d.Attr & (0x2 /*hidden*/ | 0x4 /*system*/ | 0x20 /*archive*/);
                        if ((keep & 0x6) != 0) Native.SetFileAttributesW(target, keep | Native.FILE_ATTRIBUTE_DIRECTORY);
                    }
                }) { IsBackground = true };
                th.Start(); ts.Add(th);
            }
            foreach (var th in ts) th.Join();

            if (Spec.Op == OpKind.Move)
            {
                // remove the now-empty source folders, deepest first (a folder that still holds anything stays)
                var order = dirs.OrderByDescending(d => d.Depth).ToList();
                foreach (var d in order) Native.RemoveDirectoryW(d.Src);
            }
        }

        // ---------- errors / network ----------

        static bool IsNetError(int e)
        {
            switch (e)
            {
                case Native.ERROR_BAD_NETPATH:
                case Native.ERROR_NETWORK_BUSY:
                case Native.ERROR_DEV_NOT_EXIST:
                case Native.ERROR_UNEXP_NET_ERR:
                case Native.ERROR_NETNAME_DELETED:
                case Native.ERROR_BAD_NET_NAME:
                case Native.ERROR_SEM_TIMEOUT:
                case Native.ERROR_NETWORK_UNREACHABLE:
                case Native.ERROR_HOST_UNREACHABLE:
                case Native.ERROR_CONNECTION_ABORTED:
                case Native.ERROR_CONNECTION_UNAVAIL:
                case Native.ERROR_NOT_CONNECTED:
                case Native.ERROR_NOT_READY:
                case Native.ERROR_INVALID_HANDLE:
                    return true;
            }
            return false;
        }

        static bool IsBusy(int e)
        {
            return e == Native.ERROR_SHARING_VIOLATION || e == Native.ERROR_LOCK_VIOLATION;
        }

        void Backoff(int attempt, FileItem f = null, string why = null)
        {
            int ms = Math.Min(16000, 1000 << Math.Min(4, attempt - 1));
            var sw = Stopwatch.StartNew();
            if (f != null) { RetryFile = f.Name; RetryWhy = why; }
            while (sw.ElapsedMilliseconds < ms && !canceled)
            {
                if (f != null) RetryWhy = why + " - trying again in " + Math.Max(1, (ms - sw.ElapsedMilliseconds + 999) / 1000) + " s";
                Thread.Sleep(100);
            }
            if (f != null && RetryFile == f.Name) { RetryFile = null; RetryWhy = null; }
        }

        /// <summary>Waits (up to 15 min) until both sides answer again. False = canceled or gave up.</summary>
        bool WaitForNetwork()
        {
            string a = Spec.Dest;
            string b = (SrcVol != null && SrcVol.IsNetwork && Spec.Sources.Count > 0) ? Paths.Parent(Spec.Sources[0]) : null;
            Func<bool> reachable = () => Paths.DirExists(a) && (b == null || Paths.Exists(b));
            if (reachable()) return true;
            bool owner = false;
            lock (activeLock)
            {
                if (!waitingNet) { waitingNet = true; owner = true; }
            }
            if (owner) { Status = "Waiting for the network to come back..."; SetPhase(JobPhase.Waiting); }
            var sw = Stopwatch.StartNew();
            bool ok = false;
            while (!canceled && sw.Elapsed < TimeSpan.FromMinutes(15))
            {
                if (reachable()) { ok = true; break; }
                Thread.Sleep(2000);
            }
            if (owner)
            {
                waitingNet = false;
                Status = ok ? null : "The network didn't come back in 15 minutes.";
                if (!canceled) SetPhase(JobPhase.Copying);
            }
            return ok && !canceled;
        }

        // ---------- speed limit ----------

        void Throttle(long bytes)
        {
            double rate = limitBps;
            if (rate <= 0) return;
            double wait = 0;
            lock (tokenLock)
            {
                double now = tokenClock.Elapsed.TotalSeconds;
                tokens += (now - tokenTime) * rate;
                tokenTime = now;
                if (tokens > rate * 0.25) tokens = rate * 0.25;
                tokens -= bytes;
                if (tokens < 0) wait = -tokens / rate;
            }
            if (wait > 0)
            {
                int ms = (int)Math.Min(2000, wait * 1000);
                var sw = Stopwatch.StartNew();
                while (sw.ElapsedMilliseconds < ms && !canceled) Thread.Sleep(Math.Min(50, ms));
            }
        }

        // ---------- monitor: speed history + one-time tuning ----------

        /// <summary>One measured experiment: baseline window, then a try window; keep the winner, no flapping.</summary>
        sealed class Probe
        {
            public int Stage;          // 0 = waiting, 1 = baseline, 2 = trying, 3 = finished
            public double Start;
            public double StartValue;
            public double BaseRate;
            public int Saved;
            public int Interrupts;
        }

        void MonitorLoop()
        {
            long lastBytes = 0;
            var sw = Stopwatch.StartNew();
            double lastT = 0;
            var small = new Probe();
            var bigP = new Probe();
            while (!IsFinished)
            {
                Thread.Sleep(250);
                double t = sw.Elapsed.TotalSeconds;
                long b = Interlocked.Read(ref DoneBytes);
                double dt = t - lastT;
                if (dt <= 0) continue;
                double s = Math.Max(0, (b - lastBytes) / dt);
                lastBytes = b; lastT = t;
                bool running = Phase == JobPhase.Copying && !paused && !waitingNet;
                lock (SpeedLock)
                {
                    SpeedNow = running ? s : 0;
                    if (running)
                    {
                        SpeedSmooth = SpeedSmooth <= 0 ? s : SpeedSmooth * 0.92 + s * 0.08;
                        if (s > SpeedPeak) SpeedPeak = s;
                    }
                    SpeedHistory.Add(SpeedNow);
                    if (SpeedHistory.Count > 600) SpeedHistory.RemoveRange(0, SpeedHistory.Count - 600);
                    SpeedSamples++;
                    int k = Math.Min(4, SpeedHistory.Count);
                    double sum = 0;
                    for (int i = SpeedHistory.Count - k; i < SpeedHistory.Count; i++) sum += SpeedHistory[i];
                    SpeedSecond = running && k > 0 ? sum / k : 0;
                }

                // Adaptive: measure the real path, try more in flight, keep only what is faster
                if (Plan == null || !Plan.CanTune) continue;
                bool bigActive = Volatile.Read(ref bigInFlight) > 0;
                if (!running) { Interrupt(small, false); Interrupt(bigP, true); continue; }
                if (!bigActive)
                {
                    Interrupt(bigP, true);
                    TuneSmall(small, t, Volatile.Read(ref DoneFiles));
                }
                else
                {
                    Interrupt(small, false);
                    TuneBig(bigP, t, b);
                }
            }
        }

        // small files are limited by round trips, not bandwidth: measure files per second
        void TuneSmall(Probe p, double t, int doneFiles)
        {
            if (p.Stage == 3) return;
            int pend = Volatile.Read(ref pendingSmall);
            switch (p.Stage)
            {
                case 0:
                    if (pend < 150 || activeWorkers >= 32) return;
                    if (p.Start == 0) { p.Start = t; return; }
                    if (t - p.Start >= 2) { p.Stage = 1; p.Start = t; p.StartValue = doneFiles; }
                    return;
                case 1:
                    if (t - p.Start < 5) return;
                    p.BaseRate = (doneFiles - p.StartValue) / (t - p.Start);
                    if (pend < 100) { p.Stage = 3; return; }
                    p.Saved = activeWorkers;
                    Volatile.Write(ref activeWorkers, Math.Min(32, activeWorkers * 2));
                    p.Stage = 2; p.Start = t; p.StartValue = doneFiles;
                    return;
                case 2:
                    if (t - p.Start < 5) return;
                    double rate = (doneFiles - p.StartValue) / (t - p.Start);
                    if (rate > p.BaseRate * 1.15)
                    {
                        Plan.Workers = activeWorkers;
                        Plan.Tuned = true;
                        RaisePhase();
                        if (pend >= 150 && activeWorkers < 32)
                        {
                            // it helped: try one more step up
                            p.BaseRate = rate; p.Saved = activeWorkers;
                            Volatile.Write(ref activeWorkers, Math.Min(32, activeWorkers * 2));
                            p.Start = t; p.StartValue = doneFiles;
                            return;
                        }
                    }
                    else Volatile.Write(ref activeWorkers, p.Saved);
                    p.Stage = 3;
                    return;
            }
        }

        // big files are limited by bandwidth: measure bytes per second with a second big file in flight
        void TuneBig(Probe p, double t, long bytes)
        {
            if (p.Stage == 3) return;
            switch (p.Stage)
            {
                case 0:
                    if (Volatile.Read(ref pendingBig) < 1 || Plan.BigSlots != 1) return;
                    if (p.Start == 0) { p.Start = t; return; }
                    if (t - p.Start >= 3) { p.Stage = 1; p.Start = t; p.StartValue = bytes; }
                    return;
                case 1:
                    if (t - p.Start < 8) return;
                    p.BaseRate = (bytes - p.StartValue) / (t - p.Start);
                    if (Volatile.Read(ref pendingBig) < 1) { p.Stage = 3; return; }
                    lock (bigLock) { Plan.BigSlots = 2; Monitor.PulseAll(bigLock); }
                    p.Stage = 2; p.Start = t; p.StartValue = bytes;
                    return;
                case 2:
                    if (t - p.Start < 8) return;
                    double rate = (bytes - p.StartValue) / (t - p.Start);
                    if (rate > p.BaseRate * 1.15)
                    {
                        Plan.Tuned = true;
                        Plan.TuneNote = "2 big files at once";
                        RaisePhase();
                    }
                    else lock (bigLock) { Plan.BigSlots = 1; Monitor.PulseAll(bigLock); }
                    p.Stage = 3;
                    return;
            }
        }

        // the situation changed mid-measurement (big file started/ended, pause): undo the try, measure again later
        void Interrupt(Probe p, bool big)
        {
            if (p.Stage == 0 || p.Stage == 3) { if (p.Stage == 0) p.Start = 0; return; }
            if (p.Stage == 2)
            {
                if (big) lock (bigLock) { Plan.BigSlots = 1; Monitor.PulseAll(bigLock); }
                else Volatile.Write(ref activeWorkers, p.Saved);
            }
            p.Interrupts++;
            p.Stage = p.Interrupts >= 4 ? 3 : 0;
            p.Start = 0;
        }

        // ---------- log ----------

        void WriteLog()
        {
            try
            {
                if (FailedItems.Count == 0 && SkippedItems.Count == 0 && Notes.Count == 0 && !Spec.Elevated) return;   // an administrator copy always leaves its log
                string dir = AppPaths.LogDir;
                System.IO.Directory.CreateDirectory(dir);
                LogPath = System.IO.Path.Combine(dir, DateTime.Now.ToString("yyyy-MM-dd HH-mm-ss") + " " + Spec.Id + ".txt");
                var sb = new StringBuilder();
                sb.AppendLine("Choppy - " + Spec.Title);
                sb.AppendLine("To: " + Paths.ToShort(Spec.Dest));
                sb.AppendLine("Started " + StartedAt + ", finished " + FinishedAt + " (" + Phase + ")");
                sb.AppendLine("Done " + DoneFiles + ", skipped " + SkippedFiles + ", failed " + FailedFiles + ", verified " + VerifiedFiles);
                if (FailedItems.Count > 0)
                {
                    sb.AppendLine(); sb.AppendLine("FAILED:");
                    lock (FailedItems) foreach (var f in FailedItems) sb.AppendLine("  " + Paths.ToShort(f.Src) + "  ->  " + f.Error);
                }
                if (SkippedItems.Count > 0)
                {
                    sb.AppendLine(); sb.AppendLine("SKIPPED:");
                    lock (SkippedItems) foreach (var f in SkippedItems) sb.AppendLine("  " + Paths.ToShort(f.Src));
                }
                if (Notes.Count > 0)
                {
                    sb.AppendLine(); sb.AppendLine("NOTES:");
                    lock (Notes) foreach (var n in Notes) sb.AppendLine("  " + n);
                }
                System.IO.File.WriteAllText(LogPath, sb.ToString(), Encoding.UTF8);
            }
            catch { LogPath = null; }
        }
    }
}
