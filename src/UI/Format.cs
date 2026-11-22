using System;
using System.Globalization;

namespace Choppy.UI
{
    internal static class Format
    {
        static readonly CultureInfo C = CultureInfo.CurrentCulture;

        public static string Size(long bytes)
        {
            if (bytes < 0) bytes = 0;
            if (bytes < 1000) return bytes + " B";
            double v = bytes;
            string[] u = { "B", "KB", "MB", "GB", "TB" };
            int i = 0;
            while (v >= 1000 && i < u.Length - 1) { v /= 1024; i++; }
            if (v >= 1000) { v /= 1024; i++; }
            return (v >= 100 ? v.ToString("0", C) : v >= 10 ? v.ToString("0.#", C) : v.ToString("0.##", C)) + " " + u[i];
        }

        /// <summary>Same unit for both numbers: "11.8 GB" / "19.0 GB".</summary>
        public static void Pair(long a, long total, out string sa, out string stotal)
        {
            string[] u = { "B", "KB", "MB", "GB", "TB" };
            double div = 1; int i = 0;
            while (total / div >= 1000 && i < u.Length - 1) { div *= 1024; i++; }
            string fmt = i == 0 ? "0" : "0.0";
            sa = (a / div).ToString(fmt, C) + " " + u[i];
            stotal = (total / div).ToString(fmt, C) + " " + u[i];
        }

        public static string Count(long n) { return n.ToString("N0", C); }

        /// <summary>Speed number + unit. Network jobs default to Mbps (what the ISP talks in), local disks to MB/s.</summary>
        public static void Speed(double bytesPerSec, bool mbps, out string number, out string unit)
        {
            if (mbps)
            {
                double v = bytesPerSec * 8 / 1e6;
                if (v >= 1000) { number = (v / 1000).ToString(v >= 10000 ? "0" : "0.0", C); unit = "Gbps"; }
                else { number = v.ToString(v >= 100 ? "0" : v >= 10 ? "0" : "0.0", C); unit = "Mbps"; }
            }
            else
            {
                double v = bytesPerSec / (1024 * 1024);
                if (v >= 1000) { number = (v / 1024).ToString("0.0", C); unit = "GB/s"; }
                else { number = v.ToString(v >= 100 ? "0" : "0.0", C); unit = "MB/s"; }
            }
        }

        public static string Duration(TimeSpan t)
        {
            if (t.TotalSeconds < 1) return "under a second";
            if (t.TotalSeconds < 60) return (int)t.TotalSeconds + " s";
            if (t.TotalMinutes < 60) return (int)t.TotalMinutes + " min " + t.Seconds + " s";
            return (int)t.TotalHours + " h " + t.Minutes + " min";
        }

        /// <summary>"12 s", "37 min", "1 h 5 min" (elapsed / remaining in the copy window).</summary>
        public static string Short(TimeSpan t)
        {
            if (t.TotalSeconds < 60) return Math.Max(0, (int)Math.Round(t.TotalSeconds)) + " s";
            if (t.TotalMinutes < 60) return (int)Math.Round(t.TotalMinutes) + " min";
            int h = (int)t.TotalHours, m = t.Minutes;
            return h + " h" + (m > 0 ? " " + m + " min" : "");
        }

        /// <summary>A speed limit (kept in Mbps) in the unit on screen.</summary>
        /// <summary>The speed limit in the unit shown, as a whole number (at least 1).</summary>
        public static string Limit(double mbps, bool asMbps)
        {
            double v = asMbps ? mbps : mbps * 1e6 / 8 / (1024 * 1024);
            return Math.Max(1, Math.Round(v)).ToString("0", C);
        }

        /// <summary>A typed limit: whole numbers only (0 = not one).</summary>
        public static int ParseLimit(string text)
        {
            int v;
            return int.TryParse(text, NumberStyles.None, C, out v) && v > 0 ? v : 0;
        }

        public static double LimitToMbps(double value, bool asMbps)
        {
            return asMbps ? value : value * (1024 * 1024) * 8 / 1e6;
        }

        public static string Remaining(TimeSpan t)
        {
            if (t.TotalSeconds < 5) return "a few seconds remaining";
            if (t.TotalSeconds < 60) return (int)Math.Ceiling(t.TotalSeconds / 5) * 5 + " s remaining";
            if (t.TotalMinutes < 60) return (int)Math.Ceiling(t.TotalMinutes) + " min remaining";
            int h = (int)t.TotalHours, m = t.Minutes;
            return h + " h " + (m > 0 ? m + " min " : "") + "remaining";
        }

        static readonly CultureInfo En = CultureInfo.InvariantCulture;

        /// <summary>The user's clock style (24 h or AM/PM), with English day/month names to match the English UI.</summary>
        static string TimePattern
        {
            get { return C.DateTimeFormat.ShortTimePattern.Contains("H") ? "HH:mm" : "h:mm tt"; }
        }

        public static string Clock(DateTime d)
        {
            string s = d.ToString(TimePattern, En);
            if (d.Date != DateTime.Today) s = d.ToString("ddd", En) + " " + s;
            return s;
        }

        /// <summary>"21 Nov 2026, 14:14"</summary>
        public static string Date(DateTime d)
        {
            if (d == DateTime.MinValue) return "unknown";
            return d.ToString("d MMM yyyy", En) + ", " + d.ToString(TimePattern, En);
        }
    }
}
