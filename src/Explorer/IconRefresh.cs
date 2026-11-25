using System;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using Choppy.Engine;
using Microsoft.Win32;

namespace Choppy.Explorer
{
    /// <summary>
    /// Explorer keeps every program's icon in its icon cache, by the exe's path. An update replaces Choppy.exe in the same
    /// place, so the taskbar (a pinned Choppy), the Start menu and the right-click entry went on showing the old icon (seen
    /// on Windows 11 after v1.37's new icon). Once per icon - the first start after it changes - Choppy tells Explorer the
    /// picture behind its exe and its shortcuts changed, and has Windows rebuild the icon cache.
    /// </summary>
    internal static class IconRefresh
    {
        const string Key = @"Software\Choppy";
        const string Value = "IconStamp";

        /// <summary>Refreshes the icons when Choppy's icon differs from the one last shown (cheap when it doesn't).</summary>
        public static void IfChanged()
        {
            if (Takeover.RegistryOff) return;
            try
            {
                string stamp = Stamp();
                if (stamp == null) return;
                using (var k = Registry.CurrentUser.OpenSubKey(Takeover.K(Key)))
                    if (k != null && stamp.Equals(k.GetValue(Value) as string, StringComparison.Ordinal)) return;
                // rust branch: the test switch only compares what is written - Explorer's own cache stays as it is
                if (Takeover.TestRoot == null) Refresh();
                using (var k = Registry.CurrentUser.CreateSubKey(Takeover.K(Key))) k.SetValue(Value, stamp);
            }
            catch { }
        }

        /// <summary>The icon's own fingerprint (not the exe's: every update changes the exe, few change the icon).</summary>
        static string Stamp()
        {
            // read from the exe's resources directly: this runs off the window's thread
            var rm = new System.Resources.ResourceManager("Choppy.g", typeof(IconRefresh).Assembly);
            using (var st = rm.GetStream("choppy.ico"))
            using (var ms = new MemoryStream())
            {
                if (st == null) return null;
                st.CopyTo(ms);
                return XxHash64.OfBytes(ms.ToArray()).ToString("x16");
            }
        }

        static void Refresh()
        {
            string exe = Takeover.ExePath;
            // the cached picture of the exe's icon, then the exe and every shortcut to it (Start menu, taskbar pins)
            try
            {
                int index = Shell_GetCachedImageIndexW(exe, 0, 0);
                if (index >= 0) SHUpdateImageW(exe, 0, 0, index);
            }
            catch { }
            Notify(SHCNE_UPDATEITEM, exe);
            foreach (string dir in new[]
            {
                Environment.GetFolderPath(Environment.SpecialFolder.Programs),
                Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), @"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar"),
                Environment.GetFolderPath(Environment.SpecialFolder.DesktopDirectory),
            })
            {
                try { foreach (var lnk in Directory.GetFiles(dir, "Choppy*.lnk")) Notify(SHCNE_UPDATEITEM, lnk); } catch { }
            }
            SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST | SHCNF_FLUSHNOWAIT, IntPtr.Zero, IntPtr.Zero);
            // Windows 10 / 11 keep the cache in a file as well: ie4uinit -show rebuilds it from the programs themselves
            string ie4 = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System), "ie4uinit.exe");
            if (File.Exists(ie4))
                try { using (Process.Start(new ProcessStartInfo(ie4, "-show") { UseShellExecute = false, CreateNoWindow = true, WindowStyle = ProcessWindowStyle.Hidden })) { } } catch { }
        }

        static void Notify(int ev, string path)
        {
            IntPtr p = Marshal.StringToHGlobalUni(path);
            try { SHChangeNotify(ev, SHCNF_PATHW | SHCNF_FLUSHNOWAIT, p, IntPtr.Zero); }
            finally { Marshal.FreeHGlobal(p); }
        }

        const int SHCNE_UPDATEITEM = 0x00002000, SHCNE_ASSOCCHANGED = 0x08000000;
        const int SHCNF_IDLIST = 0x0000, SHCNF_PATHW = 0x0005, SHCNF_FLUSHNOWAIT = 0x3000;

        [DllImport("shell32.dll")]
        static extern void SHChangeNotify(int eventId, int flags, IntPtr item1, IntPtr item2);

        [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
        static extern int Shell_GetCachedImageIndexW(string iconPath, int iconIndex, int flags);

        [DllImport("shell32.dll", CharSet = CharSet.Unicode)]
        static extern void SHUpdateImageW(string hashItem, int index, int flags, int imageIndex);
    }
}
