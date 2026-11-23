using System;
using System.Collections.Specialized;
using System.IO;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows;
using System.Windows.Threading;
using Choppy.Engine;

namespace Choppy.Explorer
{
    /// <summary>
    /// Ctrl+V in Explorer -> Choppy. A low-level keyboard hook that takes the key ONLY when: an Explorer window or the
    /// desktop is in front, its file list has focus (not a rename box, the address bar or search), and the clipboard
    /// holds files. Everything else passes straight through.
    ///
    /// It must never miss: Windows gives a hook a moment to answer (LowLevelHooksTimeout, at most 1 s) and when it
    /// answers late, passes the key on (Explorer's own paste) and may drop the hook for good without a word. So the hook
    /// lives on its own thread that does nothing else (never behind the window's work), answers without waiting on
    /// anything, and is put in again every few seconds (a dropped hook is back at once).
    /// </summary>
    internal static class PasteHook
    {
        const int WH_KEYBOARD_LL = 13, WM_KEYDOWN = 0x100, WM_KEYUP = 0x101, WM_SYSKEYDOWN = 0x104, WM_TIMER = 0x113, WM_QUIT = 0x12;
        const int VK_V = 0x56, VK_INSERT = 0x2D, VK_CONTROL = 0x11, VK_SHIFT = 0x10, VK_MENU = 0x12, VK_LWIN = 0x5B, VK_RWIN = 0x5C;
        static readonly IntPtr OwnKeys = (IntPtr)0x43685079;   // marks the paste Choppy hands back to Explorer (dwExtraInfo)
        const uint CF_HDROP = 15;
        const uint RefreshMs = 3000;

        delegate IntPtr LowLevelKeyboardProc(int nCode, IntPtr wParam, IntPtr lParam);

        [StructLayout(LayoutKind.Sequential)]
        struct RECT { public int l, t, r, b; }

        [StructLayout(LayoutKind.Sequential)]
        struct GUITHREADINFO
        {
            public int cbSize, flags;
            public IntPtr hwndActive, hwndFocus, hwndCapture, hwndMenuOwner, hwndMoveSize, hwndCaret;
            public RECT rcCaret;
        }

        [StructLayout(LayoutKind.Sequential)]
        struct MSG { public IntPtr hwnd; public uint message; public IntPtr wParam, lParam; public uint time; public int x, y; }

        [DllImport("user32.dll", SetLastError = true)] static extern IntPtr SetWindowsHookEx(int idHook, LowLevelKeyboardProc fn, IntPtr hMod, uint threadId);
        [DllImport("user32.dll", SetLastError = true)] static extern bool UnhookWindowsHookEx(IntPtr hhk);
        [DllImport("user32.dll")] static extern IntPtr CallNextHookEx(IntPtr hhk, int nCode, IntPtr wParam, IntPtr lParam);
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern IntPtr GetModuleHandle(string name);
        [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
        [DllImport("user32.dll")] static extern short GetAsyncKeyState(int vKey);
        [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr hWnd, StringBuilder name, int max);
        [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
        [DllImport("user32.dll")] static extern bool GetGUIThreadInfo(uint threadId, ref GUITHREADINFO info);
        [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr hWnd);
        [DllImport("user32.dll")] static extern bool IsClipboardFormatAvailable(uint format);
        [DllImport("user32.dll")] static extern int GetMessage(out MSG msg, IntPtr hwnd, uint min, uint max);
        [DllImport("user32.dll")] static extern bool PostThreadMessage(uint threadId, uint msg, IntPtr wParam, IntPtr lParam);
        [DllImport("user32.dll")] static extern UIntPtr SetTimer(IntPtr hwnd, UIntPtr id, uint ms, IntPtr fn);
        [DllImport("user32.dll")] static extern bool KillTimer(IntPtr hwnd, UIntPtr id);

        static Thread thread;
        static uint threadId;
        static IntPtr hook = IntPtr.Zero;
        static LowLevelKeyboardProc proc;
        static int swallowUp;   // the key whose release is taken too (the press was)
        static long pressedAt;  // when the last paste taken was pressed (Stopwatch ticks)
        static bool observeOnly;
        static Dispatcher ui;
        static readonly StringBuilder cls = new StringBuilder(64);   // the hook thread's own (no allocation per key)
        public static Action<IntPtr> OnPaste;

        /// <summary>Times the hook was put in again; and how many of those found the old one already dropped by Windows.</summary>
        public static int Refreshes, Dropped;

        public static bool Running { get { return thread != null && hook != IntPtr.Zero; } }

        /// <summary>observe = the self-test: the hook runs but never takes a key.</summary>
        public static bool Start(Dispatcher dispatcher, Action<IntPtr> onPaste, bool observe = false)
        {
            ui = dispatcher;
            OnPaste = onPaste;
            if (thread != null) return hook != IntPtr.Zero;
            observeOnly = observe;
            var ready = new ManualResetEventSlim();
            thread = new Thread(() => Loop(ready)) { IsBackground = true, Name = "Ctrl+V hook", Priority = ThreadPriority.Highest };
            thread.SetApartmentState(ApartmentState.STA);
            thread.Start();
            ready.Wait(5000);
            return hook != IntPtr.Zero;
        }

        public static void Stop()
        {
            var t = thread;
            if (t == null) return;
            PostThreadMessage(threadId, WM_QUIT, IntPtr.Zero, IntPtr.Zero);
            t.Join(3000);
            thread = null;
        }

        static void Loop(ManualResetEventSlim ready)
        {
            threadId = GetCurrentThreadId();
            proc = Callback;
            // compiled now, not on the first Ctrl+V (that one must answer in time too)
            try
            {
                RuntimeHelpers.PrepareMethod(typeof(PasteHook).GetMethod("Callback", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static).MethodHandle);
                RuntimeHelpers.PrepareMethod(typeof(PasteHook).GetMethod("Check", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static).MethodHandle);
                RuntimeHelpers.PrepareMethod(typeof(PasteHook).GetMethod("ClassOf", System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Static).MethodHandle);
            }
            catch { }
            hook = SetWindowsHookEx(WH_KEYBOARD_LL, proc, GetModuleHandle(null), 0);
            if (hook == IntPtr.Zero) PasteLog.Write("The Ctrl+V hook couldn't start (error " + Marshal.GetLastWin32Error() + "); it tries again in a moment.");
            UIntPtr timer = SetTimer(IntPtr.Zero, UIntPtr.Zero, RefreshMs, IntPtr.Zero);
            ready.Set();
            MSG m;
            while (GetMessage(out m, IntPtr.Zero, 0, 0) > 0)
                if (m.message == WM_TIMER) Refresh();
            KillTimer(IntPtr.Zero, timer);
            if (hook != IntPtr.Zero) UnhookWindowsHookEx(hook);
            hook = IntPtr.Zero;
        }

        /// <summary>A fresh hook in front, then the old one out (on this thread, so no key is seen twice or missed).</summary>
        static void Refresh()
        {
            IntPtr old = hook;
            IntPtr h = SetWindowsHookEx(WH_KEYBOARD_LL, proc, GetModuleHandle(null), 0);
            if (h == IntPtr.Zero) return;   // keep the old one
            hook = h;
            Refreshes++;
            if (old != IntPtr.Zero && !UnhookWindowsHookEx(old))
            {
                Dropped++;
                PasteLog.Write("Windows had dropped the Ctrl+V hook (it answered late once); it is back.");
            }
            else if (old == IntPtr.Zero) PasteLog.Write("The Ctrl+V hook is running.");
        }

        static string ClassOf(IntPtr h)
        {
            cls.Length = 0;
            GetClassName(h, cls, cls.Capacity);
            return cls.ToString();
        }

        static IntPtr Callback(int nCode, IntPtr wParam, IntPtr lParam)
        {
            try
            {
                if (nCode >= 0)
                {
                    // KBDLLHOOKSTRUCT: vkCode at 0, flags at 8, dwExtraInfo at 16. Keys typed by other programs count too
                    // (a clipboard manager pastes by sending Shift+Insert or Ctrl+V); only Choppy's own hand-back doesn't.
                    int vk = Marshal.ReadInt32(lParam, 0);
                    if ((vk == VK_V || vk == VK_INSERT) && Marshal.ReadIntPtr(lParam, 16) != OwnKeys)
                    {
                        int msg = wParam.ToInt32();
                        if (msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN)
                        {
                            IntPtr fg;
                            string why = Check(vk, out fg);
                            if (why == null && !observeOnly)
                            {
                                pressedAt = System.Diagnostics.Stopwatch.GetTimestamp();
                                swallowUp = vk;
                                var d = ui;
                                if (d != null) d.BeginInvoke(new Action(() => { var h = OnPaste; if (h != null) h(fg); }));
                                return (IntPtr)1;
                            }
                            // files on the clipboard and Explorer in front, but left to Windows: say why
                            if (why != null && why.Length > 0) PasteLog.Write((vk == VK_V ? "Ctrl+V" : "Shift+Insert") + " left to Windows: " + why);
                        }
                        else if (msg == WM_KEYUP && swallowUp == vk) { swallowUp = 0; return (IntPtr)1; }
                    }
                }
            }
            catch { }
            return CallNextHookEx(hook, nCode, wParam, lParam);
        }

        /// <summary>Milliseconds since the last paste Choppy took was pressed (the paste log notes how quick the window was).</summary>
        public static long MsSincePress()
        {
            return (System.Diagnostics.Stopwatch.GetTimestamp() - pressedAt) * 1000 / System.Diagnostics.Stopwatch.Frequency;
        }

        static bool Down(int vk) { return (GetAsyncKeyState(vk) & 0x8000) != 0; }

        /// <summary>
        /// null = take it (plain Ctrl+V or Shift+Insert in an Explorer file list or on the desktop, files on the
        /// clipboard). "" = not ours, nothing to say (another program, other keys). Else why Explorer keeps it.
        /// </summary>
        static string Check(int vk, out IntPtr fg)
        {
            fg = IntPtr.Zero;
            if (Down(VK_MENU) || Down(VK_LWIN) || Down(VK_RWIN)) return "";
            if (vk == VK_V ? !Down(VK_CONTROL) || Down(VK_SHIFT) : !Down(VK_SHIFT) || Down(VK_CONTROL)) return "";
            fg = GetForegroundWindow();
            if (fg == IntPtr.Zero) return "";
            string c = ClassOf(fg);
            if (c != "CabinetWClass" && c != "ExploreWClass" && c != "Progman" && c != "WorkerW") return "";
            if (!IsClipboardFormatAvailable(CF_HDROP)) return "";   // text or nothing: Explorer's business
            uint pid;
            uint tid = GetWindowThreadProcessId(fg, out pid);
            var gti = new GUITHREADINFO { cbSize = Marshal.SizeOf(typeof(GUITHREADINFO)) };
            if (!GetGUIThreadInfo(tid, ref gti) || gti.hwndFocus == IntPtr.Zero) return "no focus in the Explorer window";
            // the file view itself: DirectUIHWND / SysListView32 inside SHELLDLL_DefView
            string f = ClassOf(gti.hwndFocus);
            if (f == "SHELLDLL_DefView") return null;
            if ((f == "DirectUIHWND" || f == "SysListView32") && ClassOf(GetParent(gti.hwndFocus)) == "SHELLDLL_DefView") return null;
            if (f == "SysTreeView32") return "the folder tree on the left has focus (the paste goes to the folder picked there)";
            if (f == "Edit") return "a text box has focus (address bar, search or a rename)";
            return "focus is on " + f;
        }

        // ---- fallback: give Explorer its own paste back (virtual folders like This PC, Libraries) ----

        [StructLayout(LayoutKind.Sequential)]
        struct INPUT { public uint type; public KEYBDINPUT ki; public long pad; }
        [StructLayout(LayoutKind.Sequential)]
        struct KEYBDINPUT { public ushort wVk, wScan; public uint dwFlags, time; public IntPtr dwExtraInfo; }
        [DllImport("user32.dll", SetLastError = true)] static extern uint SendInput(uint n, INPUT[] inputs, int size);
        const uint INPUT_KEYBOARD = 1, KEYEVENTF_KEYUP = 2;

        public static void ReplayPaste()
        {
            bool ctrlHeld = Down(VK_CONTROL);
            var list = new System.Collections.Generic.List<INPUT>();
            Action<ushort, bool> key = (vk, up) => list.Add(new INPUT { type = INPUT_KEYBOARD, ki = new KEYBDINPUT { wVk = vk, dwFlags = up ? KEYEVENTF_KEYUP : 0, dwExtraInfo = OwnKeys } });
            if (!ctrlHeld) key(VK_CONTROL, false);
            key(VK_V, false); key(VK_V, true);
            if (!ctrlHeld) key(VK_CONTROL, true);
            SendInput((uint)list.Count, list.ToArray(), Marshal.SizeOf(typeof(INPUT)));
        }
    }

    /// <summary>
    /// What happened to each Ctrl+V of files in Explorer, and the hook's own state: %LOCALAPPDATA%\Choppy\Logs\ctrl-v.txt
    /// (the newest lines; kept small). Written in the background, never from inside the hook's answer.
    /// </summary>
    internal static class PasteLog
    {
        static readonly object gate = new object();
        static readonly System.Collections.Generic.Queue<string> pending = new System.Collections.Generic.Queue<string>();
        static bool writing;

        public static void Write(string line)
        {
            lock (gate)
            {
                if (pending.Count < 200) pending.Enqueue(DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss.fff") + "  " + line);
                if (writing) return;
                writing = true;
            }
            ThreadPool.QueueUserWorkItem(_ => Flush());
        }

        static void Flush()
        {
            while (true)
            {
                string[] lines;
                lock (gate)
                {
                    if (pending.Count == 0) { writing = false; return; }
                    lines = pending.ToArray();
                    pending.Clear();
                }
                try
                {
                    Directory.CreateDirectory(AppPaths.LogDir);
                    string file = Path.Combine(AppPaths.LogDir, "ctrl-v.txt");
                    var fi = new FileInfo(file);
                    if (fi.Exists && fi.Length > 256 * 1024)
                    {
                        // keep the newest half
                        var all = File.ReadAllLines(file);
                        File.WriteAllLines(file, new System.Collections.Generic.List<string>(all).GetRange(all.Length / 2, all.Length - all.Length / 2), Encoding.UTF8);
                    }
                    File.AppendAllLines(file, lines, Encoding.UTF8);
                }
                catch { }
            }
        }
    }

    /// <summary>The files on the clipboard and whether they were cut (move) or copied.</summary>
    internal static class ClipboardFiles
    {
        /// <summary>busy = another program held the clipboard the whole time (tried for about half a second).</summary>
        public static bool Read(out string[] files, out bool move, out bool busy)
        {
            files = null; move = false; busy = false;
            for (int attempt = 0; ; attempt++)
            {
                try
                {
                    if (!Clipboard.ContainsFileDropList()) return false;
                    StringCollection c = Clipboard.GetFileDropList();
                    files = new string[c.Count];
                    c.CopyTo(files, 0);
                    var effect = Clipboard.GetData("Preferred DropEffect") as MemoryStream;
                    if (effect != null && effect.Length >= 4)
                    {
                        var b = new byte[4];
                        effect.Position = 0;
                        effect.Read(b, 0, 4);
                        int de = BitConverter.ToInt32(b, 0);
                        move = (de & 2) != 0 && (de & 1) == 0;   // DROPEFFECT_MOVE only = cut
                    }
                    return files.Length > 0;
                }
                catch (COMException)
                {
                    // another program has the clipboard open right now (CLIPBRD_E_CANT_OPEN): wait a moment
                    if (attempt >= 10) { busy = true; return false; }
                    Thread.Sleep(50);
                }
                catch { return false; }
            }
        }

        public static bool Read(out string[] files, out bool move)
        {
            bool busy;
            return Read(out files, out move, out busy);
        }
    }
}
