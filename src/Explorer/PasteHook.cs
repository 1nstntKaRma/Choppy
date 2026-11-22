using System;
using System.Collections.Specialized;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using System.Windows;
using System.Windows.Threading;

namespace Choppy.Explorer
{
    /// <summary>
    /// Ctrl+V in Explorer -> Choppy. A low-level keyboard hook (the same idea Ultracopier uses) that takes the key
    /// ONLY when: an Explorer window or the desktop is in front, its file list has focus (not a rename box, the
    /// address bar or search), and the clipboard holds files. Everything else passes straight through.
    /// </summary>
    internal static class PasteHook
    {
        const int WH_KEYBOARD_LL = 13, WM_KEYDOWN = 0x100, WM_KEYUP = 0x101, WM_SYSKEYDOWN = 0x104;
        const int VK_V = 0x56, VK_CONTROL = 0x11, VK_SHIFT = 0x10, VK_MENU = 0x12, VK_LWIN = 0x5B, VK_RWIN = 0x5C;
        const uint LLKHF_INJECTED = 0x10;
        const uint CF_HDROP = 15;

        delegate IntPtr LowLevelKeyboardProc(int nCode, IntPtr wParam, IntPtr lParam);

        [StructLayout(LayoutKind.Sequential)]
        struct KBDLLHOOKSTRUCT { public uint vkCode, scanCode, flags, time; public IntPtr dwExtraInfo; }

        [StructLayout(LayoutKind.Sequential)]
        struct RECT { public int l, t, r, b; }

        [StructLayout(LayoutKind.Sequential)]
        struct GUITHREADINFO
        {
            public int cbSize, flags;
            public IntPtr hwndActive, hwndFocus, hwndCapture, hwndMenuOwner, hwndMoveSize, hwndCaret;
            public RECT rcCaret;
        }

        [DllImport("user32.dll", SetLastError = true)] static extern IntPtr SetWindowsHookEx(int idHook, LowLevelKeyboardProc fn, IntPtr hMod, uint threadId);
        [DllImport("user32.dll", SetLastError = true)] static extern bool UnhookWindowsHookEx(IntPtr hhk);
        [DllImport("user32.dll")] static extern IntPtr CallNextHookEx(IntPtr hhk, int nCode, IntPtr wParam, IntPtr lParam);
        [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern IntPtr GetModuleHandle(string name);
        [DllImport("user32.dll")] static extern short GetAsyncKeyState(int vKey);
        [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern int GetClassName(IntPtr hWnd, StringBuilder name, int max);
        [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
        [DllImport("user32.dll")] static extern bool GetGUIThreadInfo(uint threadId, ref GUITHREADINFO info);
        [DllImport("user32.dll")] static extern IntPtr GetParent(IntPtr hWnd);
        [DllImport("user32.dll")] static extern bool IsClipboardFormatAvailable(uint format);

        static IntPtr hook = IntPtr.Zero;
        static LowLevelKeyboardProc proc;
        static bool swallowUp;
        static Dispatcher ui;
        public static Action<IntPtr> OnPaste;

        public static bool Running { get { return hook != IntPtr.Zero; } }

        public static bool Start(Dispatcher dispatcher, Action<IntPtr> onPaste)
        {
            if (hook != IntPtr.Zero) return true;
            ui = dispatcher;
            OnPaste = onPaste;
            proc = Callback;
            hook = SetWindowsHookEx(WH_KEYBOARD_LL, proc, GetModuleHandle(null), 0);
            return hook != IntPtr.Zero;
        }

        public static void Stop()
        {
            if (hook != IntPtr.Zero) UnhookWindowsHookEx(hook);
            hook = IntPtr.Zero;
        }

        static string ClassOf(IntPtr h)
        {
            var sb = new StringBuilder(64);
            GetClassName(h, sb, sb.Capacity);
            return sb.ToString();
        }

        static IntPtr Callback(int nCode, IntPtr wParam, IntPtr lParam)
        {
            try
            {
                if (nCode >= 0)
                {
                    var k = (KBDLLHOOKSTRUCT)Marshal.PtrToStructure(lParam, typeof(KBDLLHOOKSTRUCT));
                    int msg = wParam.ToInt32();
                    if (k.vkCode == VK_V && (k.flags & LLKHF_INJECTED) == 0)
                    {
                        if ((msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN) && Wanted())
                        {
                            swallowUp = true;
                            IntPtr fg = GetForegroundWindow();
                            ui.BeginInvoke(new Action(() => { var h = OnPaste; if (h != null) h(fg); }));
                            return (IntPtr)1;
                        }
                        if (msg == WM_KEYUP && swallowUp) { swallowUp = false; return (IntPtr)1; }
                    }
                }
            }
            catch { }
            return CallNextHookEx(hook, nCode, wParam, lParam);
        }

        static bool Down(int vk) { return (GetAsyncKeyState(vk) & 0x8000) != 0; }

        /// <summary>Plain Ctrl+V, in an Explorer file list or on the desktop, with files on the clipboard.</summary>
        static bool Wanted()
        {
            if (!Down(VK_CONTROL) || Down(VK_SHIFT) || Down(VK_MENU) || Down(VK_LWIN) || Down(VK_RWIN)) return false;
            IntPtr fg = GetForegroundWindow();
            if (fg == IntPtr.Zero) return false;
            string cls = ClassOf(fg);
            if (cls != "CabinetWClass" && cls != "ExploreWClass" && cls != "Progman" && cls != "WorkerW") return false;
            uint pid;
            uint tid = GetWindowThreadProcessId(fg, out pid);
            var gti = new GUITHREADINFO { cbSize = Marshal.SizeOf(typeof(GUITHREADINFO)) };
            if (!GetGUIThreadInfo(tid, ref gti) || gti.hwndFocus == IntPtr.Zero) return false;
            // the file view itself: DirectUIHWND / SysListView32 inside SHELLDLL_DefView
            string fcls = ClassOf(gti.hwndFocus);
            bool fileView = fcls == "SHELLDLL_DefView" || ((fcls == "DirectUIHWND" || fcls == "SysListView32") && ClassOf(GetParent(gti.hwndFocus)) == "SHELLDLL_DefView");
            if (!fileView) return false;
            return IsClipboardFormatAvailable(CF_HDROP);
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
            Action<ushort, bool> key = (vk, up) => list.Add(new INPUT { type = INPUT_KEYBOARD, ki = new KEYBDINPUT { wVk = vk, dwFlags = up ? KEYEVENTF_KEYUP : 0 } });
            if (!ctrlHeld) key(VK_CONTROL, false);
            key(VK_V, false); key(VK_V, true);
            if (!ctrlHeld) key(VK_CONTROL, true);
            SendInput((uint)list.Count, list.ToArray(), Marshal.SizeOf(typeof(INPUT)));
        }
    }

    /// <summary>The files on the clipboard and whether they were cut (move) or copied.</summary>
    internal static class ClipboardFiles
    {
        public static bool Read(out string[] files, out bool move)
        {
            files = null; move = false;
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
            catch { return false; }
        }
    }
}
