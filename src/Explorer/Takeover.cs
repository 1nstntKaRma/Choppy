using System;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32;
using Choppy.Engine;

namespace Choppy.Explorer
{
    /// <summary>Which folder an Explorer window (or the desktop) is showing - the active tab on Windows 11.</summary>
    internal static class ShellLocation
    {
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string cls, string title);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr hWnd, StringBuilder name, int max);

        [ComImport, Guid("6D5140C1-7436-11CE-8034-00AA006009FA"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        interface IServiceProvider { [PreserveSig] int QueryService(ref Guid service, ref Guid riid, out IntPtr obj); }

        [ComImport, Guid("000214E2-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        interface IShellBrowserWindow { [PreserveSig] int GetWindow(out IntPtr hwnd); }   // IOleWindow::GetWindow comes first

        static readonly Guid SID_STopLevelBrowser = new Guid("4C96BE40-915C-11CF-99D3-00AA004AE837");
        static readonly Guid IID_IShellBrowser = new Guid("000214E2-0000-0000-C000-000000000046");

        /// <summary>A real folder path, or null (virtual places like This PC - Explorer keeps those pastes).</summary>
        public static string FolderOf(IntPtr hwnd)
        {
            var sb = new StringBuilder(64);
            GetClassName(hwnd, sb, sb.Capacity);
            string cls = sb.ToString();
            if (cls == "Progman" || cls == "WorkerW") return Environment.GetFolderPath(Environment.SpecialFolder.DesktopDirectory);

            IntPtr activeTab = FindWindowEx(hwnd, IntPtr.Zero, "ShellTabWindowClass", null);
            string first = null;
            object shell = null, windows = null;
            try
            {
                shell = Activator.CreateInstance(Type.GetTypeFromProgID("Shell.Application"));
                dynamic dyn = shell;
                windows = dyn.Windows();
                dynamic list = windows;
                int count = list.Count;
                for (int i = 0; i < count; i++)
                {
                    dynamic w = null;
                    try
                    {
                        w = list.Item(i);
                        if (w == null) continue;
                        long h = (long)w.HWND;
                        if (h != hwnd.ToInt64()) continue;
                        string p = PathOf(w);
                        if (activeTab != IntPtr.Zero)
                        {
                            IntPtr tab = TabOf((object)w);
                            if (tab == activeTab) return Usable(p);
                        }
                        if (first == null) first = p;
                    }
                    catch { }
                    finally { if (w != null) try { Marshal.ReleaseComObject((object)w); } catch { } }
                }
            }
            catch { }
            finally
            {
                if (windows != null) try { Marshal.ReleaseComObject(windows); } catch { }
                if (shell != null) try { Marshal.ReleaseComObject(shell); } catch { }
            }
            return Usable(first);
        }

        static string PathOf(dynamic w)
        {
            try { return (string)w.Document.Folder.Self.Path; } catch { return null; }
        }

        static IntPtr TabOf(object w)
        {
            try
            {
                var sp = (IServiceProvider)w;
                Guid sid = SID_STopLevelBrowser, iid = IID_IShellBrowser;
                IntPtr p;
                if (sp.QueryService(ref sid, ref iid, out p) != 0 || p == IntPtr.Zero) return IntPtr.Zero;
                try
                {
                    var b = (IShellBrowserWindow)Marshal.GetObjectForIUnknown(p);
                    IntPtr h;
                    return b.GetWindow(out h) == 0 ? h : IntPtr.Zero;
                }
                finally { Marshal.Release(p); }
            }
            catch { return IntPtr.Zero; }
        }

        static string Usable(string p)
        {
            if (string.IsNullOrEmpty(p) || p.StartsWith("::")) return null;
            try { return Directory.Exists(p) ? p : null; } catch { return null; }
        }
    }

    /// <summary>
    /// Windows integration, per user (HKCU) - no admin rights, nothing machine-wide.
    ///  - Right-click "Paste with Choppy": always there while Choppy is installed (the installed copy registers
    ///    it; uninstall removes it).
    ///  - "Default file manager" (Set): ON = Ctrl+V of files in Explorer goes to Choppy (tray copy with the
    ///    keyboard hook, started with Windows). Only one copier may take Ctrl+V: Ultracopier is closed and its startup
    ///    entry is parked in Choppy's settings. OFF = Windows is left alone (Ultracopier's entry is put back);
    ///    Choppy then works only through the right-click menu.
    /// </summary>
    internal static class Takeover
    {
        const string RunKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
        const string VerbName = "ChoppyPaste";
        static readonly string[] VerbParents = { @"Software\Classes\Directory\Background\shell", @"Software\Classes\Directory\shell", @"Software\Classes\Drive\shell" };

        public static string ExePath { get { return Process.GetCurrentProcess().MainModule.FileName; } }

        public static bool UltracopierInstalled()
        {
            try
            {
                if (Process.GetProcessesByName("ultracopier").Length > 0) return true;
                using (var k = Registry.CurrentUser.OpenSubKey(RunKey))
                    if (k != null && k.GetValue("ultracopier") != null) return true;
                string pf = Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles);
                return File.Exists(Path.Combine(pf, "Ultracopier", "ultracopier.exe"));
            }
            catch { return false; }
        }

        /// <summary>Turns the takeover on/off. Returns null when it worked, else what went wrong.</summary>
        public static string Set(bool on)
        {
            var s = Settings.Current;
            try
            {
                if (AppPaths.TestMode)
                {
                    // tests never touch the real Explorer, startup or Ultracopier
                    s.ExplorerTakeover = on;
                    s.Save();
                    if (App.Current2 != null) App.Current2.ApplyTakeover();
                    return null;
                }
                if (on)
                {
                    StopUltracopier(s);
                    EnsureMenu();
                    using (var k = Registry.CurrentUser.CreateSubKey(RunKey)) k.SetValue("Choppy", "\"" + ExePath + "\" --tray");
                }
                else
                {
                    using (var k = Registry.CurrentUser.CreateSubKey(RunKey)) if (k.GetValue("Choppy") != null) k.DeleteValue("Choppy");
                    RestoreUltracopier(s);
                }
                s.ExplorerTakeover = on;
                s.Save();
                if (App.Current2 != null) App.Current2.ApplyTakeover();
                return null;
            }
            catch (Exception ex) { return ex.Message; }
        }

        /// <summary>The installed copy (not a build or test exe) is the one that owns the right-click menu.</summary>
        public static bool IsInstalledCopy
        {
            get
            {
                try
                {
                    string dir = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Programs", "Choppy");
                    return ExePath.StartsWith(dir + "\\", StringComparison.OrdinalIgnoreCase);
                }
                catch { return false; }
            }
        }

        /// <summary>Right-click "Paste with Choppy" (installed copy only).</summary>
        public static void EnsureMenu()
        {
            if (AppPaths.TestMode || !IsInstalledCopy) return;
            try { WriteVerbs(); } catch { }
        }

        /// <summary>Keeps the registered paths pointing at this exe (if Choppy was moved/updated).</summary>
        public static void Refresh()
        {
            EnsureMenu();
            if (AppPaths.TestMode || !Settings.Current.ExplorerTakeover) return;
            try
            {
                using (var k = Registry.CurrentUser.CreateSubKey(RunKey)) k.SetValue("Choppy", "\"" + ExePath + "\" --tray");
            }
            catch { }
        }

        /// <summary>Uninstall: default off (Ultracopier's entry back) and the right-click menu removed.</summary>
        public static string RemoveAll()
        {
            string err = Set(false);
            if (!AppPaths.TestMode) RemoveVerbs();
            return err;
        }

        static void WriteVerbs()
        {
            string exe = ExePath;
            foreach (var parent in VerbParents)
            {
                using (var k = Registry.CurrentUser.CreateSubKey(parent + "\\" + VerbName))
                {
                    k.SetValue("MUIVerb", "Paste with Choppy");
                    k.SetValue("Icon", "\"" + exe + "\",0");
                    k.SetValue("Position", "Top");
                    using (var c = k.CreateSubKey("command"))
                        c.SetValue("", "\"" + exe + "\" paste \"" + (parent.Contains("Background") ? "%V" : "%1") + "\"");
                }
            }
        }

        static void RemoveVerbs()
        {
            foreach (var parent in VerbParents)
                try { Registry.CurrentUser.DeleteSubKeyTree(parent + "\\" + VerbName, false); } catch { }
        }

        static void StopUltracopier(Settings s)
        {
            try
            {
                // quit it FIRST: "ultracopier.exe quit" starts a second Ultracopier that writes its startup entry again
                foreach (var p in Process.GetProcessesByName("ultracopier"))
                {
                    string exe = null;
                    try { exe = p.MainModule.FileName; } catch { }
                    if (exe != null)
                    {
                        var q = Process.Start(new ProcessStartInfo(exe, "quit") { UseShellExecute = false, WorkingDirectory = Path.GetDirectoryName(exe) });
                        if (q != null) q.WaitForExit(5000);
                    }
                    if (!p.WaitForExit(5000)) try { p.CloseMainWindow(); } catch { }
                }
                for (int i = 0; i < 20 && Process.GetProcessesByName("ultracopier").Length > 0; i++) System.Threading.Thread.Sleep(250);
                // then park its startup entry
                using (var k = Registry.CurrentUser.CreateSubKey(RunKey))
                {
                    var v = k.GetValue("ultracopier") as string;
                    if (v != null) { s.UltracopierRunBackup = v; k.DeleteValue("ultracopier"); }
                }
            }
            catch { }
        }

        static void RestoreUltracopier(Settings s)
        {
            if (string.IsNullOrEmpty(s.UltracopierRunBackup)) return;
            try
            {
                using (var k = Registry.CurrentUser.CreateSubKey(RunKey)) k.SetValue("ultracopier", s.UltracopierRunBackup);
                s.UltracopierRunBackup = null;
            }
            catch { }
        }
    }
}
