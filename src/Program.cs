using System;
using System.IO;
using System.Runtime.CompilerServices;
using System.Threading;
using Choppy.Engine;

namespace Choppy
{
    internal static class Program
    {
        /// <summary>The folder Choppy was started from (relative paths on the command line are resolved against it).</summary>
        public static string StartDir;

        [STAThread]
        static int Main(string[] args)
        {
            StartDir = Environment.CurrentDirectory;
            Handoff.Prepare(args);   // an administrator copy takes test mode from its record (it starts with a fresh environment)
            // Never work "in" a random folder: anything relative can only ever land in Choppy's own data folder.
            try
            {
                Directory.CreateDirectory(AppPaths.DataDir);
                Environment.CurrentDirectory = AppPaths.DataDir;
            }
            catch { }
            if (args.Length > 0 && (args[0] == "--cli" || args[0] == "--selftest" || args[0] == "--where" || args[0] == "--usb" || args[0] == "--benchmark" || args[0] == "--shellcopy"))
            {
                Profile(args[0] == "--shellcopy" ? "shellcopy.jit" : "cli.jit");   // the benchmark's child runs beside it
                return RunCli(args);
            }
            // a Choppy that ended on an error started this one: wait until it is gone, then watch Ctrl+V from the tray
            int dead;
            if (args.Length == 2 && args[0] == "--revive" && int.TryParse(args[1], out dead))
            {
                try { using (var p = System.Diagnostics.Process.GetProcessById(dead)) p.WaitForExit(20000); } catch { }
                args = new[] { "--tray" };
            }
            // a Choppy is already running: hand it the job and exit, before anything of WPF is loaded
            bool elevated = args.Length >= 2 && args[0] == Handoff.Arg;
            string name = Instance.Name(elevated);
            int code;
            var mutex = Instance.Claim(args, name, out code);
            if (mutex == null) return code;
            Profile("app.jit");   // only a start that runs the app records what it needed (a hand-over would overwrite it)
            return RunApp(args, mutex, name, elevated);
        }

        /// <summary>Compiles on spare cores what the last start needed (a small profile file in the data folder).</summary>
        static void Profile(string file)
        {
            try
            {
                System.Runtime.ProfileOptimization.SetProfileRoot(AppPaths.DataDir);
                System.Runtime.ProfileOptimization.StartProfile(file);
            }
            catch { }
        }

        // separate methods: WPF (App's base class) loads only when App is first used, not when Main is compiled
        [MethodImpl(MethodImplOptions.NoInlining)]
        static int RunApp(string[] args, Mutex mutex, string name, bool elevated) { return App.Start(args, mutex, name, elevated); }

        [MethodImpl(MethodImplOptions.NoInlining)]
        static int RunCli(string[] args) { return Cli.Run(args); }
    }
}
