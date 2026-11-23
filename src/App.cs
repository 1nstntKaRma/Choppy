using System;
using System.Collections.Generic;
using System.IO;
using System.IO.Pipes;
using System.Linq;
using System.Security.AccessControl;
using System.Security.Principal;
using System.Text;
using System.Threading;
using System.Windows;
using System.Windows.Threading;
using Choppy.Engine;
using Choppy.Explorer;
using Choppy.UI;
using WinForms = System.Windows.Forms;

namespace Choppy
{
    /// <summary>
    /// One Choppy per user. A second start (Explorer paste, right-click, command line) hands its job to the first
    /// through a pipe and exits at once, so the window opens instantly in the running copy.
    ///   Choppy.exe copy|move &lt;src...&gt; &lt;destDir&gt;   Choppy.exe paste &lt;destDir&gt;   Choppy.exe --tray   Choppy.exe (settings)
    /// </summary>
    internal sealed class App : Application
    {
        public static App Current2;
        public JobManager Manager;
        MainWindow main;
        SettingsWindow settingsWin;
        WinForms.NotifyIcon tray;
        bool resident;
        Mutex instanceMutex;
        string pipeName;
        TestHook testHook;
        readonly List<Window> dialogs = new List<Window>();

        /// <summary>
        /// This is the short-lived administrator copy (started by the user's own Choppy for a folder that needs
        /// administrator rights): its own mutex and pipe, no keyboard hook, no tray, never touches the Run key,
        /// the right-click entries or Ultracopier. It runs its copy in its own window and exits.
        /// </summary>
        public bool Elevated;

        /// <summary>Runs this Choppy (Instance.Claim got the mutex first).</summary>
        public static int Start(string[] args, Mutex mutex, string name, bool elevated)
        {
            var app = new App { instanceMutex = mutex, pipeName = name, Elevated = elevated };
            Current2 = app;
            app.Startup += (s, e) => app.Begin(args);
            app.DispatcherUnhandledException += (s, e) =>
            {
                try { File.AppendAllText(Path.Combine(AppPaths.DataDir, "errors.txt"), DateTime.Now + " " + e.Exception + Environment.NewLine); } catch { }
                e.Handled = true;
            };
            // an error on another thread ends the process: noted, and a Choppy that watches Ctrl+V starts again in the tray
            AppDomain.CurrentDomain.UnhandledException += (s, e) => app.OnCrash(e.ExceptionObject as Exception);
            int code = app.Run();
            try { mutex.ReleaseMutex(); } catch { }
            return code;
        }

        void Begin(string[] args)
        {
            ShutdownMode = ShutdownMode.OnExplicitShutdown;
            Resources.MergedDictionaries.Add(new ResourceDictionary { Source = new Uri("/Choppy;component/UI/Theme.xaml", UriKind.Relative) });

            Manager = new JobManager();
            Manager.Ask = AskConflict;
            Manager.AskLowSpace = AskLowSpace;
            Manager.AskElevate = AskElevate;
            Manager.JobStarted = j =>
            {
                if (j.Spec.HandoffPath != null) Handoff.Report(j);   // keeps the user's Choppy up to date
                Dispatcher.BeginInvoke(new Action(() => ShowMain(true)));   // a new copy: the window comes up on top
            };
            Manager.AllDone = j => Dispatcher.BeginInvoke(new Action(MaybeExit));

            Ipc.Listen(pipeName, a => Dispatcher.BeginInvoke(new Action(() => HandleArgs(a))));

            string hookDir = Environment.GetEnvironmentVariable("CHOPPY_TESTHOOK");
            if (!string.IsNullOrEmpty(hookDir)) testHook = new TestHook(this, hookDir);

            // the installer's own steps (--menu, --uninstall...) find the registry entries checked before they run
            bool step = args.Length > 0 && args[0].StartsWith("--");
            if (!Elevated && step) Takeover.Refresh();
            // "Default file manager" on: any start of Choppy for a copy, a paste or the settings stays in the tray watching
            // Ctrl+V - a right-click paste brings the takeover back even if the tray copy was closed (not for the
            // installer's own steps like --menu, which wait for it to exit). The copy's window comes first: the tray,
            // the hook and the registry check follow right after it
            bool takeover = !Elevated && !AppPaths.TestMode && !step && Settings.Current.ExplorerTakeover;
            if (takeover) resident = true;
            HandleArgs(args);
            if (!Elevated && !step)
                Dispatcher.BeginInvoke(new Action(() => { Takeover.Refresh(); if (takeover) ApplyTakeover(); }), DispatcherPriority.Background);
            if (!Elevated) Dispatcher.BeginInvoke(new Action(OfferResume), DispatcherPriority.ApplicationIdle);
        }

        // ================= commands =================

        internal void HandleArgs(string[] args)
        {
            string cmd = args.Length > 0 ? args[0].ToLowerInvariant() : "";
            if (Elevated)
            {
                // the administrator copy does its one job and nothing else
                if (cmd == Handoff.Arg && args.Length > 1)
                {
                    var hs = Handoff.Load(args[1], args.Length > 2 ? args[2] : null);
                    if (hs != null) Manager.Enqueue(hs);
                }
                else if (cmd == "--quit") { QuitFromTray(); return; }
                MaybeExit();
                return;
            }
            switch (cmd)
            {
                case "--tray":
                    resident = true;
                    ApplyTakeover();
                    break;
                case "copy":
                case "move":
                    if (args.Length < 3) { ShowSettings(null); break; }
                    var spec = NewSpec(cmd == "move" ? OpKind.Move : OpKind.Copy, args.Skip(1).Take(args.Length - 2), args[args.Length - 1], "Command line");
                    if (spec != null) { Manager.Enqueue(spec); ShowMain(true); }
                    break;
                case "paste":
                    PasteInto(args.Length > 1 ? args[1] : null, "Right-click");
                    break;
                case "--resume":
                    OfferResume();
                    break;
                case "--takeover":
                    {
                        string err = Takeover.Set(args.Length > 1 && args[1].Equals("on", StringComparison.OrdinalIgnoreCase));
                        if (err != null) Dialogs.Show(null, "Couldn't change the Explorer setting", err, "OK", null);
                    }
                    break;
                case "--menu":
                    Takeover.EnsureMenu();
                    break;
                case "--uninstall":
                    Takeover.RemoveAll();
                    QuitFromTray();
                    return;
                case "--quit":
                    QuitFromTray();
                    return;
                default:
                    ShowSettings(null);
                    break;
            }
            MaybeExit();
        }

        JobSpec NewSpec(OpKind op, IEnumerable<string> sources, string dest, string origin)
        {
            var s = Settings.Current;
            var spec = new JobSpec { Op = op, Dest = Paths.ToLong(dest), Verify = s.VerifyCopies, Policy = s.Policy, Origin = origin };
            foreach (var src in sources) if (!string.IsNullOrWhiteSpace(src)) spec.Sources.Add(Paths.ToLong(src));
            return spec.Sources.Count > 0 && !string.IsNullOrEmpty(spec.Dest) ? spec : null;
        }

        /// <summary>Pastes the files on the clipboard into a folder (Ctrl+V hook and right-click).</summary>
        void PasteInto(string dest, string origin)
        {
            string[] files; bool move;
            if (string.IsNullOrEmpty(dest) || !ClipboardFiles.Read(out files, out move))
            {
                Dialogs.Show(main, "Nothing to paste", "Copy or cut some files first, then paste them here.", "OK", null);
                return;
            }
            var spec = NewSpec(move ? OpKind.Move : OpKind.Copy, files, dest, origin);
            if (spec == null) return;
            Manager.Enqueue(spec);
            ShowMain(true);   // a new copy (running or queued): the window comes up on top
            // like Explorer: files that were cut are pasted once
            if (move && !AppPaths.TestMode) try { Clipboard.Clear(); } catch { }
        }

        void OnExplorerPaste(IntPtr explorerWindow)
        {
            // the folder the window shows (asked of Explorer itself; it can be slow or busy for a moment: asked again)
            string dest = null;
            bool isVirtual = false;
            for (int i = 0; i < 3 && dest == null; i++)
            {
                if (i > 0) System.Threading.Thread.Sleep(120);
                dest = ShellLocation.FolderOf(explorerWindow, out isVirtual);
                if (isVirtual) break;
            }
            if (dest == null)
            {
                // This PC, Libraries, a zip folder...: Explorer's own paste
                PasteLog.Write(isVirtual ? "Pasted by Windows: the window shows a place that isn't a folder (This PC, a library, a zip...)" : "Pasted by Windows: Explorer didn't say which folder the window shows");
                PasteHook.ReplayPaste();
                return;
            }
            string[] files; bool move, busy;
            if (!ClipboardFiles.Read(out files, out move, out busy))
            {
                PasteLog.Write(busy ? "Pasted by Windows: another program kept the clipboard open" : "Pasted by Windows: no files on the clipboard any more");
                PasteHook.ReplayPaste();
                return;
            }
            string what = "Choppy: " + (move ? "move " : "copy ") + files.Length + (files.Length == 1 ? " item" : " items") + " to " + dest;
            var spec = NewSpec(move ? OpKind.Move : OpKind.Copy, files, dest, "Explorer");
            if (spec == null) { PasteLog.Write(what); return; }
            Manager.Enqueue(spec);
            ShowMain(true);
            PasteLog.Write(what + " (window up " + PasteHook.MsSincePress() + " ms after the key)");
            if (move && !AppPaths.TestMode) try { Clipboard.Clear(); } catch { }
        }

        // ================= windows =================

        public void ShowMain() { ShowMain(false); }

        /// <summary>
        /// Shows the copy window. lift = a new copy was started: the window comes up above every other window, then
        /// after a moment goes back to normal stacking (it stays in front until the user clicks something else).
        /// </summary>
        public void ShowMain(bool lift)
        {
            if (main == null)
            {
                windowBuilt = true;
                main = new MainWindow(Manager);
                main.Closed += (s, e) => { main = null; MaybeExit(); };
                if (TestHook.Park) { main.WindowStartupLocation = WindowStartupLocation.Manual; main.Left = -32000; main.Top = 0; main.ShowInTaskbar = false; main.ShowActivated = false; }
            }
            if (lift && !TestHook.Park) main.Topmost = true;   // before Show: it appears on top, not behind Explorer
            if (!main.IsVisible) main.Show();
            if (main.WindowState == WindowState.Minimized) main.WindowState = WindowState.Normal;
            if (TestHook.Park) return;
            // Windows may refuse the focus to a program in the background; on top it is seen at once all the same
            bool active = main.Activate();
            if (lift || !active)
            {
                var w = main;
                w.Topmost = true;
                var t = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(1500) };
                t.Tick += (s, e) => { t.Stop(); if (w == main) w.Topmost = false; };
                t.Start();
            }
        }

        // ---------- a quick first window ----------

        bool windowBuilt;   // a copy window was built in this process: the next ones come up in a few hundredths

        /// <summary>
        /// The first window of a process is slow to build: WPF sets up its controls, fonts and renderer then (measured:
        /// about 1.2 s from a paste to the window, every later window 0.1 s). A Choppy that waits in the tray builds
        /// one off-screen a few seconds after it starts and closes it again, so the first copy's window is as quick as
        /// the rest. The Explorer folder lookup is run once too.
        /// </summary>
        void ScheduleWarmUp()
        {
            if (windowBuilt || Elevated) return;
            var t = new DispatcherTimer(DispatcherPriority.ApplicationIdle) { Interval = TimeSpan.FromSeconds(3) };
            t.Tick += (s, e) => { t.Stop(); WarmUp(); };
            t.Start();
        }

        void WarmUp()
        {
            if (windowBuilt || main != null) return;
            windowBuilt = true;
            try
            {
                var w = UI.MainWindow.ForWarmUp(Manager);
                w.ContentRendered += (s, e) => w.Close();
                w.Show();
            }
            catch { }
            if (!AppPaths.TestMode) try { bool v; ShellLocation.FolderOf(IntPtr.Zero, out v); } catch { }
        }

        public void ShowSettings(Window owner)
        {
            if (settingsWin != null) { settingsWin.Activate(); return; }
            settingsWin = new SettingsWindow();
            if (owner != null && owner.IsVisible) { settingsWin.Owner = owner; settingsWin.WindowStartupLocation = WindowStartupLocation.CenterOwner; }
            if (TestHook.Park) { settingsWin.WindowStartupLocation = WindowStartupLocation.Manual; settingsWin.Left = -32000; settingsWin.Top = 0; settingsWin.ShowInTaskbar = false; settingsWin.ShowActivated = false; }
            settingsWin.Closed += (s, e) => { settingsWin = null; MaybeExit(); };
            settingsWin.Show();
        }

        internal MainWindow Main { get { return main; } }
        internal SettingsWindow SettingsWin { get { return settingsWin; } }
        internal ConflictWindow OpenConflict;

        public void RegisterDialog(Window w)
        {
            dialogs.Add(w);
            w.Closed += (s, e) => dialogs.Remove(w);
            if (TestHook.Park) { w.WindowStartupLocation = WindowStartupLocation.Manual; w.Left = -32000; w.Top = 0; w.ShowInTaskbar = false; w.ShowActivated = false; }
        }

        internal Window TopDialog { get { return dialogs.LastOrDefault(); } }

        /// <summary>Exits when nothing is left to do (not in tray mode).</summary>
        void MaybeExit()
        {
            if (resident) return;
            if (Manager.Busy) return;
            if (main != null || settingsWin != null || dialogs.Count > 0) return;
            if (testHook != null && !testHook.QuitRequested) return;
            Shutdown();
        }

        // ================= engine questions (from copy threads) =================

        ConflictAnswer AskConflict(CopyJob job, ConflictInfo info)
        {
            ConflictAnswer ans = null;
            Dispatcher.Invoke(new Action(() =>
            {
                var w = new ConflictWindow(info);
                if (main != null && main.IsVisible) w.Owner = main;
                else { w.WindowStartupLocation = WindowStartupLocation.CenterScreen; w.Topmost = true; w.ShowInTaskbar = true; }
                if (TestHook.Park) { w.WindowStartupLocation = WindowStartupLocation.Manual; w.Left = -32000; w.Top = 0; w.ShowInTaskbar = false; w.ShowActivated = false; }
                OpenConflict = w;
                try { w.ShowDialog(); } finally { OpenConflict = null; }
                ans = w.Answer;
            }));
            return ans ?? new ConflictAnswer { Choice = ConflictPolicy.Skip };
        }

        bool AskLowSpace(CopyJob job, long need, long free)
        {
            bool go = false;
            Dispatcher.Invoke(new Action(() =>
            {
                ShowMain();
                go = Dialogs.Ask(main, "Not enough free space",
                    "This copy needs " + Format.Size(need) + " and " + Paths.ToShort(job.Spec.Dest) + " has " + Format.Size(free) + " free. "
                    + "Files that replace existing ones need less. Copy anyway?", "Copy anyway", "Cancel");
            }));
            return go;
        }

        /// <summary>The destination only lets administrators add files: copy the rest as administrator? (once per copy)</summary>
        bool AskElevate(CopyJob job)
        {
            bool go = false;
            Dispatcher.Invoke(new Action(() =>
            {
                ShowMain();
                string verb = job.Spec.Op == OpKind.Move ? "Move" : "Copy";
                go = Dialogs.Ask(main, "This folder needs administrator rights",
                    Paths.ToShort(job.Spec.Dest) + "\n" + verb + " as administrator? The rest of this " + verb.ToLowerInvariant() + " goes on in its own window.",
                    verb + " as administrator", "No");
            }));
            return go;
        }

        // ================= resume after a crash =================

        bool resumeOffered;
        void OfferResume()
        {
            if (resumeOffered) return;
            resumeOffered = true;
            var known = new HashSet<string>(Manager.WaitingSnapshot().Select(s => s.Id));
            if (Manager.Current != null) known.Add(Manager.Current.Spec.Id);
            foreach (var spec in Journal.LoadUnfinished())
            {
                if (known.Contains(spec.Id)) continue;
                string what = spec.Title + " to " + Paths.ToShort(spec.Dest);
                int r = Dialogs.Show(main, "Resume the unfinished copy?",
                    what + " stopped before it finished (the PC restarted or Choppy was closed). Resuming keeps every file already copied and copies only the rest.",
                    "Resume", "Forget it");
                if (r == 1) { spec.ResumeMode = true; Manager.Enqueue(spec); }
                else Journal.Remove(spec);
            }
            MaybeExit();
        }

        // ================= tray + Explorer takeover =================

        void OnCrash(Exception ex)
        {
            try { File.AppendAllText(Path.Combine(AppPaths.DataDir, "errors.txt"), DateTime.Now + " (closed) " + ex + Environment.NewLine); } catch { }
            try
            {
                var me = System.Diagnostics.Process.GetCurrentProcess();
                // not when it fails right at the start (it would only fail again)
                if (Elevated || AppPaths.TestMode || !Settings.Current.ExplorerTakeover || DateTime.Now - me.StartTime < TimeSpan.FromSeconds(30)) return;
                PasteLog.Write("Choppy closed on an error; started again in the tray.");
                System.Diagnostics.Process.Start(Takeover.ExePath, "--revive " + me.Id);
            }
            catch { }
        }

        public void ApplyTakeover()
        {
            if (Elevated) return;
            bool on = Settings.Current.ExplorerTakeover;
            if (on)
            {
                resident = true;
                EnsureTray();
                if (!AppPaths.TestMode) PasteHook.Start(Dispatcher, OnExplorerPaste);
                ScheduleWarmUp();
            }
            else
            {
                PasteHook.Stop();
                if (resident && !Manager.Busy)
                {
                    resident = false;
                    if (tray != null) { tray.Visible = false; tray.Dispose(); tray = null; }
                }
                resident = false;
            }
            UpdateTrayMenu();
            MaybeExit();
        }

        public void EnsureTray()
        {
            if (tray != null || TestHook.Park || Elevated) return;
            tray = new WinForms.NotifyIcon();
            try { tray.Icon = ChoppyIcon.Small(); } catch { try { tray.Icon = System.Drawing.Icon.ExtractAssociatedIcon(Takeover.ExePath); } catch { } }
            tray.Text = "Choppy";
            tray.MouseClick += (s, e) => { if (e.Button == WinForms.MouseButtons.Left) Dispatcher.BeginInvoke(new Action(TrayOpen)); };
            tray.Visible = true;
            UpdateTrayMenu();
        }

        void TrayOpen()
        {
            if (Manager.Current != null || main != null) ShowMain();
            else ShowSettings(null);
        }

        void UpdateTrayMenu()
        {
            if (tray == null) return;
            var m = new WinForms.ContextMenuStrip { Renderer = new DarkMenuRenderer(), ShowImageMargin = false, ShowCheckMargin = true };
            m.Items.Add("Show Choppy", null, (s, e) => Dispatcher.BeginInvoke(new Action(TrayOpen)));
            m.Items.Add("Settings", null, (s, e) => Dispatcher.BeginInvoke(new Action(() => ShowSettings(null))));
            m.Items.Add(new WinForms.ToolStripSeparator());
            var take = new WinForms.ToolStripMenuItem("Default file manager") { Checked = Settings.Current.ExplorerTakeover, ToolTipText = "On: Ctrl+V of files in Explorer uses Choppy. Off: only the right-click \"Paste with Choppy\"." };
            take.Click += (s, e) => Dispatcher.BeginInvoke(new Action(() => Takeover.Set(!Settings.Current.ExplorerTakeover)));
            m.Items.Add(take);
            m.Items.Add(new WinForms.ToolStripSeparator());
            m.Items.Add("Quit Choppy", null, (s, e) => Dispatcher.BeginInvoke(new Action(QuitFromTray)));
            foreach (WinForms.ToolStripItem it in m.Items) it.ForeColor = System.Drawing.Color.FromArgb(0xE6, 0xE9, 0xEE);
            var old = tray.ContextMenuStrip;
            tray.ContextMenuStrip = m;
            if (old != null) old.Dispose();
        }

        /// <summary>Tray tooltip: progress + speed now (avg) + time left while copying.</summary>
        public void SetTrayText(string text)
        {
            if (tray == null || text == null) return;
            try { tray.Text = text.Length > 63 ? text.Substring(0, 63) : text; } catch { }
        }

        public void TrayBalloon(string text)
        {
            if (tray == null) return;
            try { tray.ShowBalloonTip(3000, "Choppy", text, WinForms.ToolTipIcon.None); } catch { }
        }

        void QuitFromTray()
        {
            if (Manager.Busy && !Dialogs.Ask(main, "Quit Choppy?", "A copy is still running. Quitting cancels it (files already copied stay).", "Quit", "Keep copying")) return;
            Manager.CancelCurrent();
            foreach (var s in Manager.WaitingSnapshot()) Manager.Remove(s);
            PasteHook.Stop();
            if (tray != null) { tray.Visible = false; tray.Dispose(); tray = null; }
            var deadline = DateTime.Now.AddSeconds(5);
            while (Manager.Current != null && !Manager.Current.IsFinished && DateTime.Now < deadline) { Thread.Sleep(50); }
            Shutdown();
        }

        protected override void OnExit(ExitEventArgs e)
        {
            PasteHook.Stop();
            if (tray != null) { tray.Visible = false; tray.Dispose(); }
            GC.KeepAlive(instanceMutex);   // one Choppy per user until here
            base.OnExit(e);
        }

        internal void QuitForTest()
        {
            Manager.CancelCurrent();
            Shutdown();
        }
    }

    /// <summary>Dark tray menu.</summary>
    internal sealed class DarkMenuRenderer : WinForms.ToolStripProfessionalRenderer
    {
        public DarkMenuRenderer() : base(new DarkColors()) { RoundedEdges = false; }
        protected override void OnRenderItemText(WinForms.ToolStripItemTextRenderEventArgs e) { e.TextColor = System.Drawing.Color.FromArgb(0xE6, 0xE9, 0xEE); base.OnRenderItemText(e); }

        sealed class DarkColors : WinForms.ProfessionalColorTable
        {
            static readonly System.Drawing.Color Bg = System.Drawing.Color.FromArgb(0x23, 0x28, 0x2F);
            static readonly System.Drawing.Color Hi = System.Drawing.Color.FromArgb(0x2F, 0x35, 0x3D);
            public override System.Drawing.Color ToolStripDropDownBackground { get { return Bg; } }
            public override System.Drawing.Color MenuItemSelected { get { return Hi; } }
            public override System.Drawing.Color MenuItemBorder { get { return Hi; } }
            public override System.Drawing.Color MenuBorder { get { return System.Drawing.Color.FromArgb(0x3A, 0x40, 0x48); } }
            public override System.Drawing.Color ImageMarginGradientBegin { get { return Bg; } }
            public override System.Drawing.Color ImageMarginGradientMiddle { get { return Bg; } }
            public override System.Drawing.Color ImageMarginGradientEnd { get { return Bg; } }
            public override System.Drawing.Color SeparatorDark { get { return System.Drawing.Color.FromArgb(0x3A, 0x40, 0x48); } }
            public override System.Drawing.Color SeparatorLight { get { return Bg; } }
            public override System.Drawing.Color CheckBackground { get { return System.Drawing.Color.FromArgb(0x0E, 0x72, 0xE6); } }
            public override System.Drawing.Color CheckSelectedBackground { get { return System.Drawing.Color.FromArgb(0x1C, 0x80, 0xF2); } }
            public override System.Drawing.Color CheckPressedBackground { get { return System.Drawing.Color.FromArgb(0x0E, 0x72, 0xE6); } }
        }
    }

    /// <summary>
    /// One Choppy per user: its mutex, or handing a job to the one already running. Kept apart from App, so a second
    /// start hands its job over without loading WPF.
    /// </summary>
    internal static class Instance
    {
        public static string Name(bool elevated)
        {
            string sid = "user";
            try { sid = WindowsIdentity.GetCurrent().User.Value; } catch { }
            return "Choppy-" + sid + (AppPaths.TestMode ? "-test" : "") + (elevated ? "-admin" : "");
        }

        /// <summary>This process's instance mutex, or null when there is nothing more to do here (code = exit code).</summary>
        public static Mutex Claim(string[] args, string name, out int code)
        {
            code = 0;
            bool created;
            var mutex = new Mutex(true, @"Local\" + name, out created);
            if (created && args.Length > 0 && args[0] == "--quit") { mutex.ReleaseMutex(); return null; }   // nothing running
            if (created) return mutex;
            // hand the job to the running Choppy
            for (int i = 0; i < 40; i++)
            {
                if (Ipc.Send(name, args)) return null;
                Thread.Sleep(100);
                // it just exited: take over (left behind by a Choppy that ended on an error counts too)
                try { if (mutex.WaitOne(0)) return mutex; }
                catch (AbandonedMutexException) { return mutex; }
            }
            code = 1;
            return null;
        }
    }

    /// <summary>Hands command lines from a second Choppy to the running one (per-user pipe).</summary>
    internal static class Ipc
    {
        const char Sep = '\u001f';

        [System.Runtime.InteropServices.DllImport("user32.dll")]
        static extern bool AllowSetForegroundWindow(int processId);

        /// <summary>
        /// True once the job is written to the running Choppy. After that it never sends again:
        /// a lost "ok" is harmless, a job sent twice is a duplicate copy.
        /// </summary>
        public static bool Send(string name, string[] args)
        {
            NamedPipeClientStream c = null;
            try
            {
                // relative paths are made absolute here, in the folder the second copy was started in
                var abs = args.Select((a, i) => i > 0 && !a.StartsWith("-") ? MakeAbs(a) : a).ToArray();
                c = new NamedPipeClientStream(".", name, PipeDirection.InOut);
                c.Connect(1500);
            }
            catch { if (c != null) c.Dispose(); return false; }   // nothing was sent: safe to try again
            using (c)
            {
                try
                {
                    // we were started by the user's click: let the running Choppy bring its window to the front
                    AllowSetForegroundWindow(-1);
                    var abs = args.Select((a, i) => i > 0 && !a.StartsWith("-") ? MakeAbs(a) : a).ToArray();
                    var data = Encoding.UTF8.GetBytes(string.Join(Sep.ToString(), abs) + "\n");
                    c.Write(data, 0, data.Length);
                    c.Flush();
                }
                catch { return false; }
                try
                {
                    var buf = new byte[8];
                    var read = c.ReadAsync(buf, 0, buf.Length);
                    read.Wait(3000);
                }
                catch { }
                return true;
            }
        }

        static string MakeAbs(string p)
        {
            try { return Path.IsPathRooted(p) ? p : Path.GetFullPath(Path.Combine(Program.StartDir ?? "", p)); } catch { return p; }
        }

        public static void Listen(string name, Action<string[]> onArgs)
        {
            var t = new Thread(() =>
            {
                while (true)
                {
                    try
                    {
                        var sec = new PipeSecurity();
                        sec.AddAccessRule(new PipeAccessRule(WindowsIdentity.GetCurrent().User, PipeAccessRights.FullControl, AccessControlType.Allow));
                        using (var s = new NamedPipeServerStream(name, PipeDirection.InOut, NamedPipeServerStream.MaxAllowedServerInstances, PipeTransmissionMode.Byte, PipeOptions.None, 4096, 4096, sec))
                        {
                            s.WaitForConnection();
                            var ms = new MemoryStream();
                            var buf = new byte[4096];
                            while (true)
                            {
                                int n = s.Read(buf, 0, buf.Length);
                                if (n <= 0) break;
                                ms.Write(buf, 0, n);
                                if (buf[n - 1] == (byte)'\n') break;
                            }
                            string text = Encoding.UTF8.GetString(ms.ToArray()).TrimEnd('\n');
                            var reply = Encoding.ASCII.GetBytes("ok");
                            s.Write(reply, 0, reply.Length);
                            s.Flush();
                            try { s.WaitForPipeDrain(); } catch { }
                            onArgs(text.Length == 0 ? new string[0] : text.Split(Sep));
                        }
                    }
                    catch { Thread.Sleep(200); }
                }
            }) { IsBackground = true, Name = "ipc" };
            t.Start();
        }
    }
}
