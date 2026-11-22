using System;
using System.IO;

namespace Choppy.Engine
{
    /// <summary>Long-path (\\?\) helpers. The engine works only with long-form paths; the UI shows the short form.</summary>
    internal static class Paths
    {
        public const string LongPrefix = @"\\?\";
        public const string LongUncPrefix = @"\\?\UNC\";

        /// <summary>
        /// Repairs a path that came through a command line: Explorer passes a drive root as "C:\" and the backslash
        /// before the closing quote turns into a quote character (C:"). Also drops stray quotes around it.
        /// </summary>
        public static string FixArg(string path)
        {
            if (string.IsNullOrEmpty(path)) return path;
            string p = path.Trim();
            if (p.EndsWith("\"")) p = p.TrimEnd('"') + "\\";
            if (p.StartsWith("\"")) p = p.TrimStart('"');
            return p;
        }

        public static string ToLong(string path)
        {
            path = FixArg(path);
            if (string.IsNullOrEmpty(path)) return path;
            if (path.StartsWith(LongPrefix, StringComparison.Ordinal)) return TrimEnd(path);
            if (path.StartsWith(@"\\.\", StringComparison.Ordinal)) return path;
            string full = path;
            try
            {
                if (!Path.IsPathRooted(path) && Program.StartDir != null) path = Path.Combine(Program.StartDir, path);
                full = Path.GetFullPath(path);
            }
            catch { }
            full = TrimEnd(full);
            if (full.StartsWith(@"\\", StringComparison.Ordinal)) return LongUncPrefix + full.Substring(2);
            return LongPrefix + full;
        }

        /// <summary>Removes the \\?\ prefix for display and for APIs that don't need it.</summary>
        public static string ToShort(string path)
        {
            if (string.IsNullOrEmpty(path)) return path;
            if (path.StartsWith(LongUncPrefix, StringComparison.OrdinalIgnoreCase)) return @"\\" + path.Substring(LongUncPrefix.Length);
            if (path.StartsWith(LongPrefix, StringComparison.Ordinal)) return path.Substring(LongPrefix.Length);
            return path;
        }

        static string TrimEnd(string p)
        {
            // keep "C:\" and "\\?\C:\" roots intact
            while (p.Length > 3 && (p.EndsWith("\\") || p.EndsWith("/")))
            {
                string t = p.Substring(0, p.Length - 1);
                if (t.EndsWith(":")) break;
                p = t;
            }
            return p;
        }

        public static string Combine(string dir, string rel)
        {
            if (string.IsNullOrEmpty(rel)) return dir;
            if (dir.EndsWith("\\")) return dir + rel;
            return dir + "\\" + rel;
        }

        public static string FileName(string path)
        {
            int i = path.LastIndexOf('\\');
            return i < 0 ? path : path.Substring(i + 1);
        }

        public static string Parent(string path)
        {
            if (path == null) return null;
            if (path.EndsWith(":\\") || path.EndsWith(":")) return null;    // a drive root has no parent
            int i = path.LastIndexOf('\\');
            if (i <= 0) return null;
            if (i == path.Length - 1) return Parent(path.Substring(0, i));
            string p = path.Substring(0, i);
            if (p.EndsWith(":")) p += "\\";
            return p;
        }

        /// <summary>True when 'inner' is 'outer' or sits inside it.</summary>
        public static bool IsSameOrInside(string inner, string outer)
        {
            string a = ToShort(inner).TrimEnd('\\') + "\\";
            string b = ToShort(outer).TrimEnd('\\') + "\\";
            return a.StartsWith(b, StringComparison.OrdinalIgnoreCase);
        }

        public static bool SamePath(string a, string b)
        {
            return string.Equals(ToShort(a).TrimEnd('\\'), ToShort(b).TrimEnd('\\'), StringComparison.OrdinalIgnoreCase);
        }

        public static bool IsUnc(string longPath)
        {
            return longPath.StartsWith(LongUncPrefix, StringComparison.OrdinalIgnoreCase) || (longPath.StartsWith(@"\\") && !longPath.StartsWith(LongPrefix));
        }

        /// <summary>"name (2).ext" style names for Keep both.</summary>
        public static string NumberedName(string fileName, int n, bool isDir)
        {
            if (isDir) return fileName + " (" + n + ")";
            int dot = fileName.LastIndexOf('.');
            if (dot <= 0) return fileName + " (" + n + ")";
            return fileName.Substring(0, dot) + " (" + n + ")" + fileName.Substring(dot);
        }

        public static bool Exists(string longPath)
        {
            return Native.GetFileAttributesW(longPath) != Native.INVALID_FILE_ATTRIBUTES;
        }

        public static bool DirExists(string longPath)
        {
            int a = Native.GetFileAttributesW(longPath);
            return a != Native.INVALID_FILE_ATTRIBUTES && (a & (int)Native.FILE_ATTRIBUTE_DIRECTORY) != 0;
        }
    }
}
