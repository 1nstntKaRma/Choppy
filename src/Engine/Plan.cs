using System;

namespace Choppy.Engine
{
    /// <summary>The method Adaptive mode picked for one job. Shown in the badge.</summary>
    internal sealed class Plan
    {
        public int Workers = 1;              // files in flight at once
        public int BigSlots = 1;             // big files in flight at once
        public long BigThreshold = 32L << 20;
        public bool Pipeline;                // big local files: read and write at the same time, no Windows cache
        public long PipeThreshold = 32L << 20;
        public PipeTuning Pipe = new PipeTuning();
        public bool InstantMove;             // same drive + move = rename only
        public bool Network;
        public bool CanTune;                 // network jobs measure once and may switch to more streams
        public string Kind = "Disk";
        public bool Tuned;
        public string TuneNote;

        /// <summary>Short badge: "Adaptive · 1 stream", "Adaptive · 8 streams", "Adaptive · instant move".</summary>
        public string Badge { get { return Mode + " · " + Detail; } }
        public string Mode { get { return "Adaptive"; } }
        public string Detail
        {
            get
            {
                if (InstantMove) return "instant move";
                return Workers + (Workers == 1 ? " stream" : " streams") + (Tuned && TuneNote != null ? " · " + TuneNote : "");
            }
        }

        /// <summary>What the badge stands for (its tooltip).</summary>
        public string Why
        {
            get
            {
                if (InstantMove) return "Same drive: the files are renamed in place, nothing is copied.";
                string s = Kind + ": ";
                if (Network) s += Workers + " files in flight hide the network's round trips" + (CanTune ? "; it measures and adds more when that is faster." : ".");
                else if (Workers <= 1) s += "one file at a time, in order - no seeking back and forth.";
                else s += Workers + " small files at once.";
                if (Pipeline) s += " Big files are read and written at the same time, past the Windows cache.";
                return s;
            }
        }

        public static Plan Choose(JobSpec spec, VolumeInfo src, VolumeInfo dst, ScanResult scan)
        {
            var p = new Plan();
            bool sameVolume = src != null && dst != null && src.Id == dst.Id;
            if (spec.Op == OpKind.Move && sameVolume && spec.ExplicitItems == null)
            {
                p.InstantMove = true;
                p.Kind = "Same drive";
                return p;
            }
            int files = scan != null ? scan.Files.Count : 0;
            long median = scan != null ? scan.MedianSize : 0;
            long largest = (scan != null && scan.Largest.Count > 0) ? scan.Largest[0].Value : 0;

            if ((src != null && src.IsNetwork) || (dst != null && dst.IsNetwork))
            {
                // Latency is the enemy: several files in flight hide the round trips,
                // and Windows' own copy pipelines the writes of each big file.
                p.Network = true;
                p.Kind = "Network";
                // measured on the NAS over Tailscale (600 small files): 8 in flight = 30 files/s, 16 = 42, 32 = 52.
                // The NAS spends a fixed time per file, so many in flight is what helps; tuning can go on to 32.
                p.Workers = files <= 1 ? 1 : (files > 64 && median < (8L << 20)) ? 16 : (files > 16 && median < (8L << 20)) ? 8 : 4;
                p.BigSlots = 1;
                p.CanTune = files > 1;
                return p;
            }
            DeviceKind slow = DeviceKind.Unknown;
            if (dst != null && dst.IsSlowSeek) slow = dst.Kind;
            else if (src != null && src.IsSlowSeek) slow = src.Kind;
            // local drives: big files go through the pipeline (reader + writer at once, big aligned pieces)
            p.Pipeline = true;
            p.Pipe.Align = Math.Max(4096, Math.Max(src != null ? src.Sector : 512, dst != null ? dst.Sector : 512));
            bool oneDisk = VolumeProbe.SameDisk(src, dst);
            if (slow != DeviceKind.Unknown)
            {
                // a spinning disk or USB stick hates seeking: one file at a time, in order
                p.Kind = slow == DeviceKind.Usb ? "USB" : slow == DeviceKind.Optical ? "Disc" : "HDD";
                p.Workers = 1;
                p.BigSlots = 1;
                // big pieces, several reads queued at the disk: other programs on it can't send the heads away between
                // them (measured with a torrent-like load: 32 MB x 4 = 157 MB/s, 16 MB x 4 = 152, Explorer 146).
                // One disk for both ends: bigger pieces, fewer trips of the heads between reading and writing
                p.Pipe.Chunk = oneDisk ? 64 << 20 : 32 << 20;
                p.Pipe.Slots = oneDisk ? 3 : 4;
                return p;
            }
            p.Kind = (dst != null && dst.Kind == DeviceKind.Ssd) || (src != null && src.Kind == DeviceKind.Ssd) ? "SSD" : "Disk";
            p.Workers = files <= 1 ? 1 : (files > 32 && median < (1L << 20)) ? 8 : 4;
            if (files <= 3 && largest >= (1L << 30)) p.Workers = 1; // one huge file: the pipeline has the drive to itself
            p.BigSlots = 1;
            p.Pipe.Chunk = 8 << 20;
            p.Pipe.Slots = 4;
            return p;
        }
    }
}
