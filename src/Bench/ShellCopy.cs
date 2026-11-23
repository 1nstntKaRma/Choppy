using System;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

namespace Choppy.Bench
{
    /// <summary>
    /// Windows' own copy for the benchmark: IFileOperation, the shell copy engine Explorer itself uses for a paste,
    /// run with no UI. Each timed copy runs in a child process (Choppy.exe --shellcopy job.txt), so Cancel stops it
    /// at once, even in the middle of a big file.
    /// </summary>
    internal static class ShellCopy
    {
        [ComImport, Guid("43826d1e-e718-42ee-bc55-a1e261c37bfe"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        interface IShellItem
        {
            void BindToHandler(IntPtr pbc, [MarshalAs(UnmanagedType.LPStruct)] Guid bhid, [MarshalAs(UnmanagedType.LPStruct)] Guid riid, out IntPtr ppv);
            void GetParent(out IShellItem ppsi);
            void GetDisplayName(uint sigdnName, out IntPtr ppszName);
            void GetAttributes(uint sfgaoMask, out uint psfgaoAttribs);
            void Compare(IShellItem psi, uint hint, out int piOrder);
        }

        [ComImport, Guid("947aab5f-0a5c-4c13-b4d6-4bf7836fc9f8"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        interface IFileOperation
        {
            uint Advise(IntPtr pfops);
            void Unadvise(uint dwCookie);
            void SetOperationFlags(uint dwOperationFlags);
            void SetProgressMessage([MarshalAs(UnmanagedType.LPWStr)] string pszMessage);
            void SetProgressDialog(IntPtr popd);
            void SetProperties(IntPtr pproparray);
            void SetOwnerWindow(IntPtr hwndOwner);
            void ApplyPropertiesToItem(IShellItem psiItem);
            void ApplyPropertiesToItems(IntPtr punkItems);
            void RenameItem(IShellItem psiItem, [MarshalAs(UnmanagedType.LPWStr)] string pszNewName, IntPtr pfopsItem);
            void RenameItems(IntPtr pUnkItems, [MarshalAs(UnmanagedType.LPWStr)] string pszNewName);
            void MoveItem(IShellItem psiItem, IShellItem psiDestinationFolder, [MarshalAs(UnmanagedType.LPWStr)] string pszNewName, IntPtr pfopsItem);
            void MoveItems(IntPtr punkItems, IShellItem psiDestinationFolder);
            void CopyItem(IShellItem psiItem, IShellItem psiDestinationFolder, [MarshalAs(UnmanagedType.LPWStr)] string pszCopyName, IntPtr pfopsItem);
            void CopyItems(IntPtr punkItems, IShellItem psiDestinationFolder);
            void DeleteItem(IShellItem psiItem, IntPtr pfopsItem);
            void DeleteItems(IntPtr punkItems);
            uint NewItem(IShellItem psiDestinationFolder, uint dwFileAttributes, [MarshalAs(UnmanagedType.LPWStr)] string pszName, [MarshalAs(UnmanagedType.LPWStr)] string pszTemplateName, IntPtr pfopsItem);
            void PerformOperations();
            [return: MarshalAs(UnmanagedType.Bool)] bool GetAnyOperationsAborted();
        }

        [DllImport("shell32.dll", CharSet = CharSet.Unicode, PreserveSig = false)]
        static extern void SHCreateItemFromParsingName(string path, IntPtr pbc, [MarshalAs(UnmanagedType.LPStruct)] Guid riid, out IShellItem item);

        static readonly Guid ClsidFileOperation = new Guid("3ad05575-8857-4850-9277-11b85bdb8e09");
        // FOF_NO_UI (silent, no confirmations, no error UI, folders made without asking) + FOF_ALLOWUNDO, as a paste in Explorer
        const uint FOF_NO_UI = 0x0004 | 0x0010 | 0x0200 | 0x0400, FOF_ALLOWUNDO = 0x0040;

        /// <summary>Copies one file or folder into a folder with the shell's copy engine. True = the engine aborted something.</summary>
        static bool Copy(string src, string destDir)
        {
            var op = (IFileOperation)Activator.CreateInstance(Type.GetTypeFromCLSID(ClsidFileOperation, true));
            IShellItem s = null, d = null;
            try
            {
                op.SetOperationFlags(FOF_NO_UI | FOF_ALLOWUNDO);
                SHCreateItemFromParsingName(src, IntPtr.Zero, typeof(IShellItem).GUID, out s);
                SHCreateItemFromParsingName(destDir, IntPtr.Zero, typeof(IShellItem).GUID, out d);
                op.CopyItem(s, d, null, IntPtr.Zero);
                op.PerformOperations();
                return op.GetAnyOperationsAborted();
            }
            finally
            {
                if (s != null) Marshal.ReleaseComObject(s);
                if (d != null) Marshal.ReleaseComObject(d);
                Marshal.ReleaseComObject(op);
            }
        }

        /// <summary>
        /// The child: Choppy.exe --shellcopy job.txt. Its lines: a tiny warm-up file, a folder for it, the source, the
        /// destination folder. The first copy in a process loads the shell's copy engine (Explorer has it loaded
        /// already): that happens on the warm-up file, untimed. Writes "seconds=..." or "error=..." to job.txt.result.
        /// </summary>
        public static int Child(string jobFile)
        {
            string result;
            try
            {
                var lines = File.ReadAllLines(jobFile, Encoding.UTF8);
                if (lines[0].Length > 0) Copy(lines[0], lines[1]);
                var sw = Stopwatch.StartNew();
                bool aborted = Copy(lines[2], lines[3]);
                sw.Stop();
                result = "seconds=" + sw.Elapsed.TotalSeconds.ToString("R", CultureInfo.InvariantCulture) + "\r\naborted=" + (aborted ? "1" : "0");
            }
            catch (Exception ex) { result = "error=" + ex.Message.Replace("\r", " ").Replace("\n", " "); }
            try { File.WriteAllText(jobFile + ".result", result, Encoding.UTF8); } catch { return 1; }
            return 0;
        }

        /// <summary>One timed copy by Windows, in a child process. Returns its seconds; throws on an error or a cancel.</summary>
        public static double Run(string warmSrc, string warmDir, string src, string destDir, Func<bool> canceled)
        {
            string job = Path.Combine(Path.GetTempPath(), "choppy-shellcopy-" + Guid.NewGuid().ToString("N").Substring(0, 8) + ".txt");
            File.WriteAllLines(job, new[] { warmSrc ?? "", warmDir ?? "", src, destDir }, Encoding.UTF8);
            try
            {
                var psi = new ProcessStartInfo(Exe, "--shellcopy \"" + job + "\"") { UseShellExecute = false, CreateNoWindow = true };
                using (var p = Process.Start(psi))
                {
                    while (!p.WaitForExit(50))
                    {
                        if (canceled())
                        {
                            try { p.Kill(); } catch { }
                            p.WaitForExit(10000);
                            throw new OperationCanceledException();
                        }
                    }
                }
                string res = File.Exists(job + ".result") ? File.ReadAllText(job + ".result", Encoding.UTF8) : "error=Windows' copy ended without a result";
                double secs = -1;
                foreach (var line in res.Split(new[] { '\r', '\n' }, StringSplitOptions.RemoveEmptyEntries))
                {
                    if (line.StartsWith("error=")) throw new IOException("Windows' copy failed: " + line.Substring(6));
                    if (line == "aborted=1") throw new IOException("Windows' copy stopped part way");
                    if (line.StartsWith("seconds=")) secs = double.Parse(line.Substring(8), CultureInfo.InvariantCulture);
                }
                if (secs < 0) throw new IOException("Windows' copy gave no time");
                return secs;
            }
            finally
            {
                try { File.Delete(job); } catch { }
                try { File.Delete(job + ".result"); } catch { }
            }
        }

        static string exe;
        static string Exe
        {
            get
            {
                if (exe == null) using (var me = Process.GetCurrentProcess()) exe = me.MainModule.FileName;
                return exe;
            }
        }
    }
}
