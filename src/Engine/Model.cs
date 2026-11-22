using System;
using System.Collections.Generic;

namespace Choppy.Engine
{
    internal enum OpKind { Copy, Move }

    /// <summary>What to do when a file with the same name is already at the destination.</summary>
    internal enum ConflictPolicy { Ask, Replace, Skip, KeepBoth, ReplaceOlder }

    internal enum FileState { Pending = 0, Active = 1, Done = 2, Skipped = 3, Failed = 4 }

    internal enum JobPhase { Queued, Analyzing, Copying, Waiting, Finishing, Done, Canceled, Failed }

    internal sealed class JobSpec
    {
        public string Id = Guid.NewGuid().ToString("N").Substring(0, 12);
        public OpKind Op;
        public List<string> Sources = new List<string>();   // long-form paths
        public string Dest;                                  // long-form destination folder
        public bool Verify;                                  // check each copy (a move then removes the original only after its copy matches)
        public ConflictPolicy Policy = ConflictPolicy.Ask;
        public bool ResumeMode;                              // a resumed job skips files already there with the same size and date
        public double SpeedLimitMbps;                        // 0 = no limit
        public string Origin = "";                           // "Explorer", "Right-click", "CLI", ...
        public List<FileItem> ExplicitItems;                 // "Retry failed" jobs carry their files directly
        public List<DirItem> ExplicitDirs;                   // folders that come with them (an administrator copy finishes them)
        public bool Elevated;                                // runs as administrator for the user's own Choppy
        public string HandoffPath;                           // its record: progress, result and control files sit next to it
        public int ForceWorkers;                             // tests: fixed number of files in flight (0 = Adaptive)
        public bool NoFastPath;                              // tests: always rename with a separate call
        public string Engine;                                // tests: null = Adaptive, "copyfile" = CopyFileEx only, "nobuf" = the old unbuffered CopyFileEx
        public int PipeChunkMB, PipeSlots, PipeBatch, CachedWriteMB;                   // tests: pipeline piece size / pieces in flight (0 = Adaptive)
        public bool NoPrealloc;                              // tests: no size reserved up front
        public string SmallMode;                             // tests: small files - "unbuf", "cached", "tiny" (cache under 64 KB), "off" (no batches), "batch" (batches on any hard disk)
        public bool ForcePrealloc;                           // tests: reserve the size up front even on a hard disk
        public bool NoTune;                                  // tests: keep the first guess, measure nothing
        public bool NoCachedTail;                            // tests: the last file's end goes straight to the disk too
        public int TailMB = -1;                              // tests: how much of each big file's end goes through the cache (-1 = Adaptive)

        public string Title
        {
            get
            {
                string verb = Op == OpKind.Move ? "Move" : "Copy";
                if (ExplicitItems != null) return verb + " " + ExplicitItems.Count + (ExplicitItems.Count == 1 ? " file" : " files") + (Elevated ? "" : " again");
                if (Sources.Count == 1) return verb + " " + Paths.FileName(Paths.ToShort(Sources[0]));
                return verb + " " + Sources.Count + " items";
            }
        }
    }

    internal sealed class FileItem
    {
        public string Src;          // long-form source path
        public string Rel;          // path under the destination folder, top-level name included
        public long Size;
        public uint Attr;
        public long CTime, ATime, MTime;
        public bool DestExists;     // a file with this name already sits at the destination
        public bool DestIsDir;
        public long DestSize, DestMTime;
        public bool TempExists;     // a leftover "<name>.chpart" from an interrupted run
        public volatile int State;  // FileState
        public string Error;
        public string FinalPath;    // where it ended up (Keep both can change the name)
        public bool Verified;
        public bool AlreadyThere;   // resume: same size + date already at the destination
        public bool SourceKept;     // move: copied, but the source couldn't be deleted

        public FileState St { get { return (FileState)State; } set { State = (int)value; } }
        public string Name { get { return Paths.FileName(Rel); } }
    }

    internal sealed class DirItem
    {
        public string Src;
        public string Rel;
        public uint Attr;
        public long CTime, ATime, MTime;
        public bool DestExisted;
        public bool DestIsFile;     // a FILE with this folder's name blocks it
        public int Depth;
    }

    internal sealed class ScanResult
    {
        public List<FileItem> Files = new List<FileItem>();
        public List<DirItem> Dirs = new List<DirItem>();
        public long TotalBytes;
        public int Conflicts;
        public List<KeyValuePair<string, long>> Largest = new List<KeyValuePair<string, long>>();
        public List<string> Problems = new List<string>();   // unreadable folders, links not followed
        public long MedianSize;
    }

    internal sealed class ConflictInfo
    {
        public FileItem File;
        public string IncomingPath;     // short form
        public string ExistingPath;     // short form
        public long IncomingSize, ExistingSize;
        public DateTime IncomingTime, ExistingTime;
        public int Remaining;           // conflicts not answered yet, this one included
        public OpKind Op;
    }

    internal sealed class ConflictAnswer
    {
        public ConflictPolicy Choice;
        public bool ApplyToAll;
        public bool CancelJob = false;   // a dialog can stop the whole copy
    }

    internal static class FileTimes
    {
        public static DateTime ToLocal(long ft)
        {
            try { return DateTime.FromFileTimeUtc(ft).ToLocalTime(); } catch { return DateTime.MinValue; }
        }
    }
}
