using System;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

namespace Choppy.Engine
{
    /// <summary>
    /// How a USB drive is connected, asked of the USB hub it is plugged into (as Windows' own USB tools do, no
    /// administrator rights needed): the speed it runs at, the most it can do, and whether the port does USB 3. A drive
    /// that can do more than it runs at is held back by the port, the cable or a hub on the way.
    /// </summary>
    internal sealed class UsbLink
    {
        public const int Low = 0, Full = 1, High = 2, Super = 3, SuperPlus = 4;

        public int Speed;              // what it runs at now (Low .. SuperPlus)
        public int Capable;            // the most the drive can do (as Speed); Speed when unknown
        public bool CapableKnown;      // Windows 8+ says what the drive can do
        public bool PortUsb3;          // the port takes USB 3 (5 Gbps or more)
        public bool PortKnown;
        public bool Uasp;              // UASP: commands queued like a SATA disk's (many small files go faster than Bulk-Only)
        public int BcdUsb;             // the USB version the drive reports right now (0x0210, 0x0320...)

        /// <summary>The drive could run faster in a faster port (or with another cable).</summary>
        public bool HeldBack { get { return CapableKnown && Capable > Speed && Speed >= Full; } }

        /// <summary>"USB 10 Gbps", "USB 5 Gbps", "USB 2.0 (480 Mbps)"</summary>
        public string SpeedText { get { return Name(Speed); } }

        public static string Name(int speed)
        {
            switch (speed)
            {
                case SuperPlus: return "USB 10 Gbps";
                case Super: return "USB 5 Gbps";
                case High: return "USB 2.0 (480 Mbps)";
                case Full: return "USB 1.1 (12 Mbps)";
                default: return "USB 1.0 (1.5 Mbps)";
            }
        }

        /// <summary>Short: "10 Gbps", "5 Gbps", "USB 2.0"</summary>
        public static string Short(int speed)
        {
            switch (speed)
            {
                case SuperPlus: return "10 Gbps";
                case Super: return "5 Gbps";
                case High: return "USB 2.0";
                case Full: return "USB 1.1";
                default: return "USB 1.0";
            }
        }

        /// <summary>
        /// About the most a drive moves over this link, in MB/s (1,048,576 bytes): what is left of the line rate after
        /// USB's own encoding and protocol. USB 2.0 drives measure 35-42, 5 Gbps ones 400-450, 10 Gbps ones 900-1,050.
        /// </summary>
        public static double CeilingMBps(int speed)
        {
            switch (speed)
            {
                case SuperPlus: return 1000;
                case Super: return 430;
                case High: return 40;
                case Full: return 1;
                default: return 0.15;
            }
        }

        /// <summary>The names people know the speeds by (a tooltip).</summary>
        public static string Aka(int speed)
        {
            switch (speed)
            {
                case SuperPlus: return "USB 3.2 Gen 2 (also called USB 3.1 Gen 2), SuperSpeed+; 20 Gbps (Gen 2x2) reports the same";
                case Super: return "USB 3.2 Gen 1 (also called USB 3.0 or USB 3.1 Gen 1), SuperSpeed";
                case High: return "USB 2.0 High Speed";
                default: return "USB 1.x";
            }
        }

        /// <summary>Why it is held back and what helps, in one line; null when it isn't.</summary>
        public string Advice
        {
            get
            {
                if (!HeldBack) return null;
                string s = "Runs at " + Short(Speed) + " - the drive can do " + Short(Capable) + ". ";
                if (Speed <= High)
                    return s + (PortKnown && PortUsb3
                        ? "The port does USB 3: the cable, or a hub or extension on the way, is USB 2."
                        : "The port is USB 2: a USB 3 port (often blue, or marked SS) makes it up to 10 times faster.");
                return s + "A 10 Gbps port (USB 3.2 Gen 2, often red or marked SS10) and cable make big files up to twice as fast.";
            }
        }

        /// <summary>The link of the USB drive that holds this physical disk; null when the disk isn't on USB (or Windows doesn't say).</summary>
        public static UsbLink ForDisk(int diskNumber)
        {
            if (diskNumber < 0) return null;
            // tests: CHOPPY_FAKE_USB=2,3,1 = every USB drive runs at USB 2.0, can do 5 Gbps, its port does USB 3
            string fake = Environment.GetEnvironmentVariable("CHOPPY_FAKE_USB");
            if (!string.IsNullOrEmpty(fake))
            {
                var f = fake.Split(',');
                return new UsbLink { Speed = int.Parse(f[0]), Capable = int.Parse(f[1]), CapableKnown = true, PortKnown = f.Length > 2, PortUsb3 = f.Length > 2 && f[2] == "1" };
            }
            try
            {
                int devInst = DiskDevInst(diskNumber);
                if (devInst == 0) return null;
                // up from the disk to the device that sits on a hub port: disk -> (USBSTOR / UASPStor) -> USB device -> hub
                bool uasp = false;
                int cur = devInst;
                for (int level = 0; level < 8; level++)
                {
                    string service = Prop(cur, CM_DRP_SERVICE);
                    if (service != null && service.Equals("UASPStor", StringComparison.OrdinalIgnoreCase)) uasp = true;
                    int parent;
                    if (CM_Get_Parent(out parent, cur, 0) != 0 || parent == 0) return null;
                    string hub = HubPath(parent);
                    if (hub != null)
                    {
                        int port = PropInt(cur, CM_DRP_ADDRESS);
                        if (port <= 0) return null;
                        var link = Ask(hub, port);
                        if (link != null) link.Uasp = uasp;
                        return link;
                    }
                    cur = parent;
                }
            }
            catch { }
            return null;
        }

        /// <summary>The hub's answer about one of its ports.</summary>
        static UsbLink Ask(string hubPath, int port)
        {
            using (var h = Native.CreateFileW(hubPath, Native.GENERIC_WRITE, Native.FILE_SHARE_WRITE, IntPtr.Zero, Native.OPEN_EXISTING, 0, IntPtr.Zero))
            {
                if (h.IsInvalid) return null;
                // USB_NODE_CONNECTION_INFORMATION_EX: ConnectionIndex, the device descriptor (bcdUSB at +2), ..., Speed at 23
                var inEx = new byte[35];
                BitConverter.GetBytes(port).CopyTo(inEx, 0);
                var outEx = new byte[1024];
                int ret;
                if (!DeviceIoControl(h, IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX, inEx, inEx.Length, outEx, outEx.Length, out ret, IntPtr.Zero) || ret < 35) return null;
                if (BitConverter.ToInt32(outEx, 31) != 1 /*DeviceConnected*/) return null;
                var link = new UsbLink { BcdUsb = BitConverter.ToUInt16(outEx, 6), Speed = Math.Min((int)outEx[23], Super) };
                link.Capable = link.Speed;
                // USB_NODE_CONNECTION_INFORMATION_EX_V2 (Windows 8+): which USB versions the port takes, and what the drive
                // can do and runs at (SuperSpeed or more, SuperSpeedPlus or more)
                var inV2 = new byte[16];
                BitConverter.GetBytes(port).CopyTo(inV2, 0);
                BitConverter.GetBytes(16).CopyTo(inV2, 4);
                BitConverter.GetBytes(7).CopyTo(inV2, 8);   // the protocols we understand: USB 1.1, 2.0, 3.x
                var outV2 = new byte[16];
                if (DeviceIoControl(h, IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX_V2, inV2, inV2.Length, outV2, outV2.Length, out ret, IntPtr.Zero) && ret >= 16)
                {
                    uint protocols = BitConverter.ToUInt32(outV2, 8), flags = BitConverter.ToUInt32(outV2, 12);
                    link.PortKnown = protocols != 0;
                    link.PortUsb3 = (protocols & 4) != 0;
                    if ((flags & 4) != 0) link.Speed = SuperPlus;
                    else if ((flags & 1) != 0) link.Speed = Math.Max(link.Speed, Super);
                    link.Capable = (flags & 8) != 0 ? SuperPlus : (flags & 2) != 0 ? Super : link.Speed;
                    link.Capable = Math.Max(link.Capable, link.Speed);
                    link.CapableKnown = true;
                }
                return link;
            }
        }

        /// <summary>The device node of the disk that answers with this number.</summary>
        static int DiskDevInst(int diskNumber)
        {
            var guid = GUID_DEVINTERFACE_DISK;
            IntPtr set = SetupDiGetClassDevsW(ref guid, null, IntPtr.Zero, DIGCF_PRESENT | DIGCF_DEVICEINTERFACE);
            if (set == new IntPtr(-1)) return 0;
            try
            {
                for (int i = 0; i < 64; i++)
                {
                    var ifd = new SP_DEVICE_INTERFACE_DATA { cbSize = Marshal.SizeOf(typeof(SP_DEVICE_INTERFACE_DATA)) };
                    if (!SetupDiEnumDeviceInterfaces(set, IntPtr.Zero, ref guid, i, ref ifd)) break;
                    var info = new SP_DEVINFO_DATA { cbSize = Marshal.SizeOf(typeof(SP_DEVINFO_DATA)) };
                    int need;
                    SetupDiGetDeviceInterfaceDetailW(set, ref ifd, IntPtr.Zero, 0, out need, IntPtr.Zero);
                    if (need <= 0) continue;
                    IntPtr detail = Marshal.AllocHGlobal(need);
                    try
                    {
                        // SP_DEVICE_INTERFACE_DETAIL_DATA_W: its cbSize is the fixed part (8 in a 64-bit process, 6 in a 32-bit one)
                        Marshal.WriteInt32(detail, IntPtr.Size == 8 ? 8 : 6);
                        if (!SetupDiGetDeviceInterfaceDetailW(set, ref ifd, detail, need, out need, ref info)) continue;
                        string path = Marshal.PtrToStringUni(detail + 4);
                        using (var h = Native.CreateFileW(path, 0, Native.FILE_SHARE_READ | Native.FILE_SHARE_WRITE, IntPtr.Zero, Native.OPEN_EXISTING, 0, IntPtr.Zero))
                        {
                            if (h.IsInvalid) continue;
                            Native.STORAGE_DEVICE_NUMBER dn;
                            int r;
                            if (Native.DeviceIoControl(h, Native.IOCTL_STORAGE_GET_DEVICE_NUMBER, IntPtr.Zero, 0, out dn, Marshal.SizeOf(typeof(Native.STORAGE_DEVICE_NUMBER)), out r, IntPtr.Zero)
                                && VolumeProbe.DiskNumber(dn) == diskNumber)
                                return info.DevInst;
                        }
                    }
                    finally { Marshal.FreeHGlobal(detail); }
                }
            }
            finally { SetupDiDestroyDeviceInfoList(set); }
            return 0;
        }

        /// <summary>The path to open a hub by, when this device node is a USB hub; else null.</summary>
        static string HubPath(int devInst)
        {
            var id = new StringBuilder(512);
            if (CM_Get_Device_IDW(devInst, id, id.Capacity, 0) != 0) return null;
            var guid = GUID_DEVINTERFACE_USB_HUB;
            int len;
            if (CM_Get_Device_Interface_List_SizeW(out len, ref guid, id.ToString(), 0) != 0 || len <= 1) return null;
            var buf = new char[len];
            if (CM_Get_Device_Interface_ListW(ref guid, id.ToString(), buf, len, 0) != 0) return null;
            string first = new string(buf).Split('\0')[0];
            return first.Length > 0 ? first : null;
        }

        static string Prop(int devInst, int prop)
        {
            var buf = new byte[1024];
            int type, len = buf.Length;
            if (CM_Get_DevNode_Registry_PropertyW(devInst, prop, out type, buf, ref len, 0) != 0 || len < 2) return null;
            return Encoding.Unicode.GetString(buf, 0, len).TrimEnd('\0');
        }

        static int PropInt(int devInst, int prop)
        {
            var buf = new byte[4];
            int type, len = 4;
            return CM_Get_DevNode_Registry_PropertyW(devInst, prop, out type, buf, ref len, 0) == 0 && len == 4 ? BitConverter.ToInt32(buf, 0) : -1;
        }

        // ---------- Windows ----------

        static readonly Guid GUID_DEVINTERFACE_DISK = new Guid("53f56307-b6bf-11d0-94f2-00a0c91efb8b");
        static readonly Guid GUID_DEVINTERFACE_USB_HUB = new Guid("f18a0e88-c30c-11d0-8815-00a0c906bed8");
        const int DIGCF_PRESENT = 2, DIGCF_DEVICEINTERFACE = 0x10;
        const int CM_DRP_SERVICE = 0x05, CM_DRP_ADDRESS = 0x1D;   // SPDRP_* + 1
        const uint IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX = 0x220448;      // FILE_DEVICE_USB, function 274, buffered
        const uint IOCTL_USB_GET_NODE_CONNECTION_INFORMATION_EX_V2 = 0x22045C;   // function 279

        [StructLayout(LayoutKind.Sequential)]
        struct SP_DEVICE_INTERFACE_DATA { public int cbSize; public Guid InterfaceClassGuid; public int Flags; public IntPtr Reserved; }

        [StructLayout(LayoutKind.Sequential)]
        struct SP_DEVINFO_DATA { public int cbSize; public Guid ClassGuid; public int DevInst; public IntPtr Reserved; }

        [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        static extern IntPtr SetupDiGetClassDevsW(ref Guid classGuid, string enumerator, IntPtr parent, int flags);

        [DllImport("setupapi.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        static extern bool SetupDiEnumDeviceInterfaces(IntPtr set, IntPtr devInfo, ref Guid guid, int index, ref SP_DEVICE_INTERFACE_DATA data);

        [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        static extern bool SetupDiGetDeviceInterfaceDetailW(IntPtr set, ref SP_DEVICE_INTERFACE_DATA data, IntPtr detail, int size, out int required, IntPtr devInfo);

        [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        static extern bool SetupDiGetDeviceInterfaceDetailW(IntPtr set, ref SP_DEVICE_INTERFACE_DATA data, IntPtr detail, int size, out int required, ref SP_DEVINFO_DATA devInfo);

        [DllImport("setupapi.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        static extern bool SetupDiDestroyDeviceInfoList(IntPtr set);

        [DllImport("cfgmgr32.dll")]
        static extern int CM_Get_Parent(out int parent, int devInst, int flags);

        [DllImport("cfgmgr32.dll", CharSet = CharSet.Unicode)]
        static extern int CM_Get_Device_IDW(int devInst, StringBuilder buffer, int length, int flags);

        [DllImport("cfgmgr32.dll", CharSet = CharSet.Unicode)]
        static extern int CM_Get_Device_Interface_List_SizeW(out int length, ref Guid guid, string deviceId, int flags);

        [DllImport("cfgmgr32.dll", CharSet = CharSet.Unicode)]
        static extern int CM_Get_Device_Interface_ListW(ref Guid guid, string deviceId, char[] buffer, int length, int flags);

        [DllImport("cfgmgr32.dll", CharSet = CharSet.Unicode)]
        static extern int CM_Get_DevNode_Registry_PropertyW(int devInst, int property, out int type, byte[] buffer, ref int length, int flags);

        [DllImport("kernel32.dll", SetLastError = true)]
        [return: MarshalAs(UnmanagedType.Bool)]
        static extern bool DeviceIoControl(SafeFileHandle device, uint code, byte[] input, int inputSize, byte[] output, int outputSize, out int returned, IntPtr overlapped);
    }
}
