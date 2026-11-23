using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace Choppy.Engine
{
    internal enum DeviceKind { Unknown, Ssd, Hdd, Usb, Network, Optical, Ram }

    /// <summary>What sits behind a path: which volume, what kind of device, and (for network) how far away.</summary>
    internal sealed class VolumeInfo
    {
        public string Root;          // "C:\", "\\server\share\"
        public string Id;            // used for "same volume" checks
        public DeviceKind Kind;
        public string FileSystem;
        public string Label;         // the drive's name ("Media"); empty when it has none
        public double RttMs = -1;    // network round trip of a metadata call (min of 3)
        public int Disk = -1;        // physical disk number (-1 = unknown / network)
        public int Sector = 512;     // bytes per sector: unbuffered reads and writes go in multiples of it
        public bool Removable;       // can be pulled out (USB, cards): nothing may wait in memory for it
        public uint FsFlags;         // the file system's FILE_* capability flags
        public UsbLink Usb;          // a USB drive: the speed it runs at and the most it can do (null = not USB, or unknown)

        public bool IsNetwork { get { return Kind == DeviceKind.Network; } }
        public bool IsSlowSeek { get { return Kind == DeviceKind.Hdd || Kind == DeviceKind.Usb || Kind == DeviceKind.Optical; } }

        /// <summary>The largest file this file system holds (FAT32: 4 GB less a byte); 0 = no limit that matters.</summary>
        public long MaxFileSize
        {
            get { return FileSystem != null && FileSystem.StartsWith("FAT", StringComparison.OrdinalIgnoreCase) ? uint.MaxValue : 0; }
        }

        /// <summary>ReFS (a Dev Drive, for one): a copy on the same volume can share the original's blocks.</summary>
        public bool CanClone { get { return (FsFlags & Native.FILE_SUPPORTS_BLOCK_REFCOUNTING) != 0; } }

        public string KindText
        {
            get
            {
                switch (Kind)
                {
                    case DeviceKind.Ssd: return "SSD";
                    case DeviceKind.Hdd: return "HDD";
                    case DeviceKind.Usb: return "USB";
                    case DeviceKind.Network: return "Network";
                    case DeviceKind.Optical: return "Disc";
                    case DeviceKind.Ram: return "RAM disk";
                }
                return "Disk";
            }
        }
    }

    internal static class VolumeProbe
    {
        public static VolumeInfo Probe(string longPath)
        {
            var v = new VolumeInfo();
            string shortPath = Paths.ToShort(longPath);
            var sb = new StringBuilder(1024);
            string root = null;
            // GetVolumePathName wants an existing-ish path; walk up until it works.
            string probe = shortPath;
            while (probe != null)
            {
                if (Native.GetVolumePathNameW(probe, sb, sb.Capacity)) { root = sb.ToString(); break; }
                probe = Paths.Parent(probe);
            }
            if (root == null) root = shortPath.StartsWith(@"\\") ? UncRoot(shortPath) : (shortPath.Length >= 3 ? shortPath.Substring(0, 3) : shortPath);
            if (!root.EndsWith("\\")) root += "\\";
            v.Root = root;

            uint type = Native.GetDriveTypeW(root);
            bool unc = root.StartsWith(@"\\");
            if (unc || type == Native.DRIVE_REMOTE)
            {
                v.Kind = DeviceKind.Network;
                v.Id = "net:" + (unc ? root.ToLowerInvariant() : root.ToUpperInvariant());
                v.RttMs = MeasureRtt(longPath);
            }
            else
            {
                uint serial, maxComp, fsFlags;
                var fs = new StringBuilder(64);
                if (Native.GetVolumeInformationW(root, null, 0, out serial, out maxComp, out fsFlags, fs, fs.Capacity))
                {
                    v.Id = "vol:" + serial.ToString("X8");
                    v.FileSystem = fs.ToString();
                    v.FsFlags = fsFlags;
                }
                else v.Id = "root:" + root.ToUpperInvariant();

                if (type == Native.DRIVE_CDROM) v.Kind = DeviceKind.Optical;
                else if (type == Native.DRIVE_RAMDISK) v.Kind = DeviceKind.Ram;
                else
                {
                    bool usb;
                    v.Kind = ProbeDisk(root, type, out v.Disk, out v.Removable, out usb);
                    if (usb) v.Usb = UsbLink.ForDisk(v.Disk);
                }
                uint spc, bps, fc, tc;
                if (Native.GetDiskFreeSpaceW(root, out spc, out bps, out fc, out tc) && bps >= 512) v.Sector = (int)bps;
            }
            {
                // the name the drive shows in Explorer ("Media", "NAS")
                uint serial, maxComp, fsFlags;
                var fs = new StringBuilder(64);
                var label = new StringBuilder(261);
                if (Native.GetVolumeInformationW(root, label, label.Capacity, out serial, out maxComp, out fsFlags, fs, fs.Capacity))
                {
                    if (v.FileSystem == null) v.FileSystem = fs.ToString();
                    v.Label = label.ToString();
                }
            }
            return v;
        }

        static string UncRoot(string unc)
        {
            // \\server\share\...
            int a = unc.IndexOf('\\', 2);
            if (a < 0) return unc + "\\";
            int b = unc.IndexOf('\\', a + 1);
            return b < 0 ? unc + "\\" : unc.Substring(0, b + 1);
        }

        /// <summary>True when both sit on the same physical disk (reads and writes then share one set of heads).</summary>
        public static bool SameDisk(VolumeInfo a, VolumeInfo b)
        {
            return a != null && b != null && !a.IsNetwork && !b.IsNetwork && (a.Id == b.Id || (a.Disk >= 0 && a.Disk == b.Disk));
        }

        static DeviceKind ProbeDisk(string root, uint driveType, out int disk, out bool removable, out bool usb)
        {
            disk = -1;
            usb = false;
            removable = driveType == Native.DRIVE_REMOVABLE;
            // Open the volume device itself (no access rights needed for these queries).
            string dev = null;
            if (root.Length == 3 && root[1] == ':') dev = @"\\.\" + root.Substring(0, 2);
            else
            {
                var vn = new StringBuilder(100);
                if (Native.GetVolumeNameForVolumeMountPointW(root, vn, vn.Capacity)) dev = vn.ToString().TrimEnd('\\');
            }
            if (dev == null) return driveType == Native.DRIVE_REMOVABLE ? DeviceKind.Usb : DeviceKind.Unknown;

            using (SafeFileHandle h = Native.CreateFileW(dev, 0, Native.FILE_SHARE_READ | Native.FILE_SHARE_WRITE, IntPtr.Zero, Native.OPEN_EXISTING, 0, IntPtr.Zero))
            {
                if (h.IsInvalid) return driveType == Native.DRIVE_REMOVABLE ? DeviceKind.Usb : DeviceKind.Unknown;

                Native.STORAGE_DEVICE_NUMBER dn;
                int dret;
                if (Native.DeviceIoControl(h, Native.IOCTL_STORAGE_GET_DEVICE_NUMBER, IntPtr.Zero, 0, out dn, Marshal.SizeOf(typeof(Native.STORAGE_DEVICE_NUMBER)), out dret, IntPtr.Zero))
                    disk = DiskNumber(dn);

                var q =new Native.STORAGE_PROPERTY_QUERY { PropertyId = 0, QueryType = 0 };
                Native.STORAGE_DEVICE_DESCRIPTOR_HEADER dd;
                int ret;
                bool ok = Native.DeviceIoControl(h, Native.IOCTL_STORAGE_QUERY_PROPERTY, ref q, Marshal.SizeOf(q), out dd, Marshal.SizeOf(typeof(Native.STORAGE_DEVICE_DESCRIPTOR_HEADER)), out ret, IntPtr.Zero);
                var sq = new Native.STORAGE_PROPERTY_QUERY { PropertyId = 7 /*StorageDeviceSeekPenaltyProperty*/, QueryType = 0 };
                Native.DEVICE_SEEK_PENALTY_DESCRIPTOR sp;
                int sret;
                bool knowsSeek = Native.DeviceIoControl(h, Native.IOCTL_STORAGE_QUERY_PROPERTY, ref sq, Marshal.SizeOf(sq), out sp, Marshal.SizeOf(typeof(Native.DEVICE_SEEK_PENALTY_DESCRIPTOR)), out sret, IntPtr.Zero);
                usb = (ok || ret >= 32) && dd.BusType == 7;
                if (((ok || ret >= 32) && (dd.BusType == 7 /*USB*/ || dd.BusType == 12 /*SD*/ || dd.BusType == 13 /*MMC*/)) || driveType == Native.DRIVE_REMOVABLE)
                {
                    // pluggable: a USB SSD that says it has no seek delay is copied like an SSD (still removable: nothing
                    // waits in memory for it); USB hard disks, sticks and cards stay one steady stream
                    removable = true;
                    return knowsSeek && !sp.IncursSeekPenalty && dd.BusType == 7 ? DeviceKind.Ssd : DeviceKind.Usb;
                }
                if (knowsSeek) return sp.IncursSeekPenalty ? DeviceKind.Hdd : DeviceKind.Ssd;
                if (ok && dd.BusType == 17 /*NVMe*/) return DeviceKind.Ssd;
                return DeviceKind.Unknown;
            }
        }

        public const int FILE_DEVICE_DISK = 7;

        /// <summary>
        /// The physical disk a volume sits on, or -1. Only a disk device's number counts: numbers of other device types
        /// are counted separately - Google Drive's G: answers as a file system device "number 0", the same number as a
        /// real hard disk, which made a copy between the two look like one disk.
        /// </summary>
        public static int DiskNumber(Native.STORAGE_DEVICE_NUMBER dn)
        {
            return dn.DeviceType == FILE_DEVICE_DISK && dn.DeviceNumber >= 0 ? dn.DeviceNumber : -1;
        }

        /// <summary>Round trip of one metadata call to the share (best of 3). Tailscale to the NAS was ~9 ms.</summary>
        public static double MeasureRtt(string longPath)
        {
            string p = longPath;
            while (p != null && !Paths.Exists(p)) p = Paths.Parent(p);
            if (p == null) return -1;
            double best = double.MaxValue;
            for (int i = 0; i < 3; i++)
            {
                var sw = Stopwatch.StartNew();
                Native.WIN32_FILE_ATTRIBUTE_DATA d;
                // a name that never exists forces a real server round trip (no cached attributes)
                Native.GetFileAttributesExW(Paths.Combine(p, "~choppy-rtt-" + Guid.NewGuid().ToString("N").Substring(0, 8)), 0, out d);
                sw.Stop();
                best = Math.Min(best, sw.Elapsed.TotalMilliseconds);
            }
            return best;
        }
    }
}
