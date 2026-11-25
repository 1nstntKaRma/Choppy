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

        // the same questions asked through the shell's own interfaces (each one call into Explorer), not by name
        // through IDispatch (two calls each, and the first use of `dynamic` loads a compiler)
        static readonly Guid CLSID_ShellWindows = new Guid("9BA05972-F6A8-11CF-A442-00A0C90A8F39");
        static readonly Guid IID_IPersistFolder2 = new Guid("1AC3D9F0-175C-11D1-95BE-00609797EA4F");

        [ComImport, Guid("85CB6900-4D95-11CF-960C-0080C7F4EE85"), InterfaceType(ComInterfaceType.InterfaceIsDual)]
        interface IShellWindows
        {
            int Count { get; }
            [return: MarshalAs(UnmanagedType.IDispatch)] object Item([MarshalAs(UnmanagedType.Struct)] object index);
        }

        // only the slots up to the one called are declared: the others just keep the order
        [ComImport, Guid("000214E2-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        interface IShellBrowser
        {
            [PreserveSig] int GetWindow(out IntPtr hwnd);
            void ContextSensitiveHelp(); void InsertMenusSB(); void SetMenuSB(); void RemoveMenusSB(); void SetStatusTextSB(); void EnableModelessSB();
            void TranslateAcceleratorSB(); void BrowseObject(); void GetViewStateStream(); void GetControlWindow(); void SendControlMsg();
            [PreserveSig] int QueryActiveShellView([MarshalAs(UnmanagedType.IUnknown)] out object view);
        }

        [ComImport, Guid("CDE725B0-CCC9-4519-917E-325D72FAB4CE"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        interface IFolderView
        {
            void GetCurrentViewMode(); void SetCurrentViewMode();
            [PreserveSig] int GetFolder(ref Guid riid, [MarshalAs(UnmanagedType.IUnknown)] out object folder);
        }

        [ComImport, Guid("1AC3D9F0-175C-11D1-95BE-00609797EA4F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        interface IPersistFolder2
        {
            void GetClassID(); void Initialize();
            [PreserveSig] int GetCurFolder(out IntPtr pidl);
        }

        [DllImport("user32.dll")] static extern IntPtr GetAncestor(IntPtr hwnd, uint flags);
        [DllImport("shell32.dll")] static extern int SHGetNameFromIDList(IntPtr pidl, uint sigdn, out IntPtr name);
        const uint GA_ROOT = 2, SIGDN_FILESYSPATH = 0x80058000;

        public static string FolderOf(IntPtr hwnd)
        {
            bool isVirtual;
            return FolderOf(hwnd, out isVirtual);
        }

        /// <summary>
        /// A real folder path, or null. isVirtual = the window shows a place that isn't a folder (This PC, a library, a
        /// zip: Explorer keeps those pastes); null without it = Explorer didn't answer (ask again).
        /// </summary>
        public static string FolderOf(IntPtr hwnd, out bool isVirtual)
        {
            isVirtual = false;
            var sb = new StringBuilder(64);
            GetClassName(hwnd, sb, sb.Capacity);
            string cls = sb.ToString();
            if (cls == "Progman" || cls == "WorkerW") return Environment.GetFolderPath(Environment.SpecialFolder.DesktopDirectory);

            IntPtr activeTab = FindWindowEx(hwnd, IntPtr.Zero, "ShellTabWindowClass", null);
            bool answered;
            string p = Ask(hwnd, activeTab, out answered);
            if (!answered) p = AskByName(hwnd, activeTab, out answered);   // the old way, in case this Windows answers only that one
            if (!answered) return null;
            isVirtual = !IsFolder(p);
            return isVirtual ? null : p;
        }

        /// <summary>The folder the window's active tab shows (null: no file system path). answered = the window was found and said.</summary>
        static string Ask(IntPtr hwnd, IntPtr activeTab, out bool answered)
        {
            answered = false;
            IShellWindows windows = null;
            try
            {
                windows = (IShellWindows)Activator.CreateInstance(Type.GetTypeFromCLSID(CLSID_ShellWindows));
                int count = windows.Count;
                for (int i = 0; i < count; i++)
                {
                    object w = null, sb = null, view = null, folder = null;
                    try
                    {
                        w = windows.Item(i);
                        var sp = w as IServiceProvider;
                        if (sp == null) continue;
                        Guid sid = SID_STopLevelBrowser, iid = IID_IShellBrowser;
                        IntPtr p;
                        if (sp.QueryService(ref sid, ref iid, out p) != 0 || p == IntPtr.Zero) continue;
                        try { sb = Marshal.GetObjectForIUnknown(p); } finally { Marshal.Release(p); }
                        var browser = (IShellBrowser)sb;
                        IntPtr h;
                        if (browser.GetWindow(out h) != 0) continue;
                        // the browser's window is the tab (Windows 11 has several per window; the active one comes first)
                        if (h != hwnd && GetAncestor(h, GA_ROOT) != hwnd) continue;
                        if (activeTab != IntPtr.Zero && h != hwnd && h != activeTab) continue;
                        if (browser.QueryActiveShellView(out view) != 0 || view == null) return null;
                        Guid pf = IID_IPersistFolder2;
                        if (((IFolderView)view).GetFolder(ref pf, out folder) != 0 || folder == null) return null;
                        IntPtr pidl;
                        if (((IPersistFolder2)folder).GetCurFolder(out pidl) != 0 || pidl == IntPtr.Zero) return null;
                        try
                        {
                            answered = true;
                            IntPtr name;
                            if (SHGetNameFromIDList(pidl, SIGDN_FILESYSPATH, out name) != 0 || name == IntPtr.Zero) return null;   // This PC, a library...
                            try { return Marshal.PtrToStringUni(name); } finally { Marshal.FreeCoTaskMem(name); }
                        }
                        finally { Marshal.FreeCoTaskMem(pidl); }
                    }
                    catch { }
                    finally
                    {
                        foreach (var o in new[] { folder, view, sb, w }) if (o != null) try { Marshal.ReleaseComObject(o); } catch { }
                    }
                }
            }
            catch { }
            finally { if (windows != null) try { Marshal.ReleaseComObject(windows); } catch { } }
            return null;
        }

        /// <summary>The same through Shell.Application, by name (the way Choppy asked before 1.36).</summary>
        static string AskByName(IntPtr hwnd, IntPtr activeTab, out bool answered)
        {
            answered = false;
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
                            if (tab == activeTab) { answered = p != null; return p; }
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
            answered = first != null;
            return first;
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

        /// <summary>A folder Choppy can paste into ("::{...}" is a virtual place, a zip or an FTP place isn't a folder on a disk).</summary>
        static bool IsFolder(string p)
        {
            if (string.IsNullOrEmpty(p) || p.StartsWith("::")) return false;
            try { return Directory.Exists(p); } catch { return false; }
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

        /// <summary>
        /// Test switch (rust branch only, in both programs, for comparing what they write): with test mode (CHOPPY_DATA),
        /// CHOPPY_REGTEST=&lt;scratch key&gt; runs the registry steps as an installed Choppy does, but every key lands under
        /// HKCU\&lt;scratch key&gt;\... (never the real Run key, right-click entries or settings), "installed" means the exe
        /// sits in &lt;data folder&gt;\Programs\Choppy, and Explorer's icon cache is left alone.
        /// </summary>
        internal static string TestRoot
        {
            get
            {
                if (!AppPaths.TestMode) return null;
                string r = Environment.GetEnvironmentVariable("CHOPPY_REGTEST");
                return string.IsNullOrEmpty(r) ? null : r;
            }
        }

        /// <summary>An HKCU key path, under the test root when there is one.</summary>
        internal static string K(string path) { return TestRoot == null ? path : TestRoot + "\\" + path; }

        /// <summary>Tests never touch the real Explorer, startup or Ultracopier (unless CHOPPY_REGTEST points them at a scratch key).</summary>
        internal static bool RegistryOff { get { return AppPaths.TestMode && TestRoot == null; } }

        static string exePath;
        public static string ExePath { get { return exePath ?? (exePath = Process.GetCurrentProcess().MainModule.FileName); } }

        public static bool UltracopierInstalled()
        {
            try
            {
                if (Process.GetProcessesByName("ultracopier").Length > 0) return true;
                using (var k = Registry.CurrentUser.OpenSubKey(K(RunKey)))
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
                if (RegistryOff)
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
                    using (var k = Registry.CurrentUser.CreateSubKey(K(RunKey))) k.SetValue("Choppy", "\"" + ExePath + "\" --tray");
                }
                else
                {
                    using (var k = Registry.CurrentUser.CreateSubKey(K(RunKey))) if (k.GetValue("Choppy") != null) k.DeleteValue("Choppy");
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
                    string dir = Path.Combine(TestRoot != null ? AppPaths.DataDir : Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Programs", "Choppy");
                    return ExePath.StartsWith(dir + "\\", StringComparison.OrdinalIgnoreCase);
                }
                catch { return false; }
            }
        }

        /// <summary>Right-click "Paste with Choppy" (installed copy only).</summary>
        public static void EnsureMenu()
        {
            if (RegistryOff || !IsInstalledCopy) return;
            try { WriteVerbs(); } catch { }
        }

        /// <summary>Keeps the registered paths pointing at this exe (if Choppy was moved/updated).</summary>
        public static void Refresh()
        {
            EnsureMenu();
            if (RegistryOff || !Settings.Current.ExplorerTakeover) return;
            try
            {
                using (var k = Registry.CurrentUser.CreateSubKey(K(RunKey))) k.SetValue("Choppy", "\"" + ExePath + "\" --tray");
            }
            catch { }
        }

        /// <summary>Uninstall: default off (Ultracopier's entry back) and the right-click menu removed.</summary>
        public static string RemoveAll()
        {
            string err = Set(false);
            if (!RegistryOff) RemoveVerbs();
            return err;
        }

        static void WriteVerbs()
        {
            string exe = ExePath;
            foreach (var parent in VerbParents)
            {
                using (var k = Registry.CurrentUser.CreateSubKey(K(parent + "\\" + VerbName)))
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
                try { Registry.CurrentUser.DeleteSubKeyTree(K(parent + "\\" + VerbName), false); } catch { }
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
                using (var k = Registry.CurrentUser.CreateSubKey(K(RunKey)))
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
                using (var k = Registry.CurrentUser.CreateSubKey(K(RunKey))) k.SetValue("ultracopier", s.UltracopierRunBackup);
                s.UltracopierRunBackup = null;
            }
            catch { }
        }
    }
}
