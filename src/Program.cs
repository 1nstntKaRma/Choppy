using System;
using System.IO;
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
            if (args.Length > 0 && (args[0] == "--cli" || args[0] == "--selftest" || args[0] == "--where"))
                return Cli.Run(args);
            return App.Start(args);
        }
    }
}
