using System;
using System.Collections.Generic;

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
        public List<PipeTuning> PipeWays = new List<PipeTuning>();   // other ways the tuner measures against Pipe on this copy
        public bool InstantMove;             // same drive + move = rename only
        public bool Network;
        public bool CanTune;                 // measure once and switch to more files in flight when that is faster
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
                if (Pipeline) s += " Big files are read and written past the Windows cache" + (PipeWays.Count > 0 ? "; the first big file tries a few ways and the fastest copies the rest." : ".");
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
                p.MaxWorkers = 64;   // a far NAS spends a fixed time per file: tuning may go on to 64 in flight
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
                bool srcSlow = src != null && src.IsSlowSeek, dstSlow = dst != null && dst.IsSlowSeek;
                // big pieces, several reads queued at the disk: other programs on it can't send the heads away between
                // them (measured with a torrent-like load: 32 MB x 4 = 157 MB/s, 16 MB x 4 = 152, Explorer 146).
                p.Pipe.Chunk = 32 << 20;
                p.Pipe.Slots = 8;
                // a hard disk read into a fast drive: many small reads queued keep it streaming (1 MB x 32 measured
                // ahead of 32 MB x 8 and of Explorer, two rounds out of two)
                if (srcSlow && !dstSlow) { p.Pipe.Chunk = 1 << 20; p.Pipe.Slots = 32; }
                // one disk at both ends: read 256 MB, then write it - not both at once (the heads would go back and forth every piece)
                p.Pipe.Phased = oneDisk;
                // writing to a hard disk: pieces already read go out together (up to 128 MB), no gap for the platter to pass
                if (dstSlow && !oneDisk) p.Pipe.WriteBatch = 4;
                // no size reserved up front on a hard disk: the reservation can land the file on a slower part of the
                // platter (NVMe to hard disk: 199-206 MB/s without it, 169-170 with it)
                if (dstSlow) p.Pipe.Prealloc = false;
                // two hard disks: 512 MB read ahead in 2 MB reads, joined into 64 MB writes, so a write doesn't stop the
                // reads and the source streams from the first second (4 GB on disk: 32 MB x 8 = 163 MB/s, 32 MB x 16 =
                // 164-171, 2 MB x 256 = 175-179, Explorer 155-159). Only 4 reads out at the disk at once: a deep queue
                // starts slower (first 1.5 s: 4 out = 191-217 MB/s, 256 out = 119-160) and gains nothing after (shown
                // 173-183 vs 167-178; Explorer 176-178). Once the source is fully read, what the buffers still hold goes
                // through the cache: the copy is done then, and the disk never waits on the cache while it reads.
                // No trials here: two disks swing more from one second to the next than the ways differ.
                if (srcSlow && dstSlow && !oneDisk)
                {
                    p.Pipe.Chunk = 2 << 20;
                    p.Pipe.Slots = 256;
                    p.Pipe.ReadDepth = 4;
                    p.Pipe.WriteBatch = 32;
                    p.Pipe.TailAfterRead = true;
                    return Aligned(p.WithSmallBatch());
                }
                // all of the above is a first guess: the first big file measures these ways against it and keeps the fastest
                if (oneDisk)
                {
                    p.PipeWays.Add(Way(32, 16, true, 1));
                    p.PipeWays.Add(Way(16, 8, true, 1));
                }
                else if (srcSlow && !dstSlow)
                {
                    p.PipeWays.Add(Way(4, 16, false, 1));
                    p.PipeWays.Add(Way(8, 8, false, 1));
                }
                else
                {
                    p.PipeWays.Add(Way(1, 32, false, 32));    // small reads, joined into 32 MB writes
                    p.PipeWays.Add(Way(8, 8, false, 4));
                }
                // a hard disk's speed swings over short stretches: longer trials, and another way must win by 8%
                p.TuneSeconds = 2.0;
                p.TuneMargin = 1.08;
                return Aligned(p.WithSmallBatch());
            }
            p.Kind = (dst != null && dst.Kind == DeviceKind.Ssd) || (src != null && src.Kind == DeviceKind.Ssd) ? "SSD" : "Disk";
            p.Workers = files <= 1 ? 1 : (files > 32 && median < (1L << 20)) ? 8 : 4;
            if (files <= 3 && largest >= (1L << 30)) p.Workers = 1; // one huge file: the pipeline has the drive to itself
            p.BigSlots = 1;
            p.Pipe.Chunk = 8 << 20;
            p.Pipe.Slots = 4;
            p.PipeWays.Add(Way(4, 8, false, 1));
            p.PipeWays.Add(Way(16, 4, false, 1));
            p.PipeWays.Add(Way(32, 4, false, 1));
            p.CanTune = files > 1;
            return Aligned(p);
        }

        public int MaxWorkers = 32;          // the most files in flight tuning may go to
        public bool SmallBatch;              // a hard disk at either end: small files in read-then-write batches
        public double TuneSeconds = 0.75;    // length of one trial stretch
        public double TuneMargin = 1.03;     // how much faster another way must be to take over

        // many small files: read in batches in disk order, then written through the cache - one worker
        // (1,000 files: two hard disks 90-93 MB/s, one disk 67-69; one file at a time 73-76 / 58-60; Explorer 64-67 / 52-55)
        Plan WithSmallBatch()
        {
            SmallBatch = true;
            CanTune = false;
            MaxWorkers = 4;
            return this;
        }

        static PipeTuning Way(int mb, int slots, bool phased, int batch)
        {
            return new PipeTuning { Chunk = mb << 20, Slots = slots, Phased = phased, WriteBatch = batch };
        }

        static Plan Aligned(Plan p)
        {
            foreach (var w in p.PipeWays) { w.Align = p.Pipe.Align; w.Prealloc = p.Pipe.Prealloc; }
            return p;
        }
    }
}
