using System;
using System.IO;
using System.Linq;
using System.Text;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using Choppy.UI;

namespace Choppy
{
    /// <summary>
    /// Opt-in test driver (CHOPPY_TESTHOOK=&lt;dir&gt;): reads &lt;dir&gt;\cmd.txt, runs the lines, appends results to out.txt.
    /// Never moves the real mouse or keyboard: buttons are pressed in code, screenshots are rendered off-screen.
    /// CHOPPY_PARK=1 keeps every window off-screen with no taskbar button.
    ///   shot main|conflict|settings|dialog file.png     press pause|skip|cancel|verify|unit|queue|retry
    ///   conflict replace|skip|keepboth|compare [all]   dialog 1|2     settings open|close     set key value
    ///   state file.json     enqueue copy|move|paste ...args separated by |     quit
    /// </summary>
    internal sealed class TestHook
    {
        public static bool Park { get { return Environment.GetEnvironmentVariable("CHOPPY_PARK") == "1"; } }
        public bool QuitRequested;
        readonly App app;
        readonly string dir;
        readonly DispatcherTimer timer;

        public TestHook(App app, string dir)
        {
            this.app = app;
            this.dir = dir;
            Directory.CreateDirectory(dir);
            timer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(150) };
            timer.Tick += (s, e) => Poll();
            timer.Start();
            File.AppendAllText(Path.Combine(dir, "out.txt"), "ready " + DateTime.Now.ToString("HH:mm:ss.fff") + Environment.NewLine);
        }

        void Poll()
        {
            string f = Path.Combine(dir, "cmd.txt");
            if (!File.Exists(f)) return;
            string[] lines;
            try { lines = File.ReadAllLines(f, Encoding.UTF8); File.Delete(f); } catch { return; }
            foreach (var raw in lines)
            {
                string line = raw.Trim();
                if (line.Length == 0) continue;
                string result;
                try { result = Run(line); } catch (Exception ex) { result = "error " + ex.Message; }
                File.AppendAllText(Path.Combine(dir, "out.txt"), line + " => " + result + Environment.NewLine, Encoding.UTF8);
            }
        }

        string Run(string line)
        {
            var parts = line.Split(new[] { ' ' }, 2);
            string cmd = parts[0], arg = parts.Length > 1 ? parts[1] : "";
            switch (cmd)
            {
                case "shot":
                    {
                        var p = arg.Split(new[] { ' ' }, 2);
                        Window w = p[0] == "conflict" ? (Window)app.OpenConflict : p[0] == "settings" ? (Window)app.SettingsWin : p[0] == "dialog" ? app.TopDialog
                            : p[0] == "benchmark" ? (Window)app.BenchWin : app.Main;
                        if (w == null) return "no window";
                        Shot(w, p[1]);
                        return "ok";
                    }
                case "press":
                    if (app.Main == null) return "no main window";
                    app.Main.PressForTest(arg.Trim());
                    return "ok";
                case "conflict":
                    {
                        if (app.OpenConflict == null) return "no conflict window";
                        var p = arg.Split(' ');
                        app.OpenConflict.PressForTest(p[0], p.Length > 1 && p[1] == "all");
                        return "ok";
                    }
                case "dialog":
                    {
                        var d = app.TopDialog;
                        if (d == null || !(d.Tag is Action<int>)) return "no dialog";
                        ((Action<int>)d.Tag)(int.Parse(arg.Trim()));
                        return "ok";
                    }
                case "about":
                    // modal: opened after this command returns, so the hook keeps answering ("dialog 1" closes it)
                    app.Dispatcher.BeginInvoke(new Action(() => Dialogs.About((Window)app.SettingsWin ?? app.Main)));
                    return "ok";
                case "settings":
                    if (arg.Trim() == "open") app.ShowSettings(app.Main);
                    else if (app.SettingsWin != null) app.SettingsWin.Close();
                    return "ok";
                case "benchmark":
                    // benchmark open | close | start tiny|quick|thorough [C,D] | stop | state
                    if (arg.Trim() == "open") { app.ShowBenchmark(); return "ok"; }
                    if (app.BenchWin == null) return "benchmark not open";
                    if (arg.Trim() == "close") { app.BenchWin.Close(); return "ok"; }
                    return app.BenchWin.ForTest(arg);
                case "set":
                    {
                        if (app.SettingsWin == null) return "settings not open";
                        var p = arg.Split(new[] { ' ' }, 2);
                        app.SettingsWin.SetForTest(p[0], p.Length > 1 ? p[1] : "");
                        return "ok";
                    }
                case "state":
                    File.WriteAllText(arg.Trim(), app.Main != null ? app.Main.StateForTest() : "{\"phase\":\"no window\"}", Encoding.UTF8);
                    return "ok";
                case "enqueue":
                    app.HandleArgs(arg.Split('|').Select(s => s.Trim()).ToArray());
                    return "ok";
                case "quit":
                    QuitRequested = true;
                    app.QuitForTest();
                    return "ok";
            }
            return "unknown command";
        }

        static void Shot(Window w, string file)
        {
            w.UpdateLayout();
            double scale = 1.5;
            int pw = (int)Math.Ceiling(w.ActualWidth * scale), ph = (int)Math.Ceiling(w.ActualHeight * scale);
            var rtb = new RenderTargetBitmap(pw, ph, 96 * scale, 96 * scale, PixelFormats.Pbgra32);
            var dv = new DrawingVisual();
            using (var dc = dv.RenderOpen())
            {
                // the window background + content, as a picture
                var content = (FrameworkElement)w.Content;
                // absolute viewbox = the whole client area in the content's own coordinates (its margin included;
                // a default VisualBrush trims empty edges)
                double cw = w.ActualWidth - w.BorderThickness.Left - w.BorderThickness.Right;
                double ch = w.ActualHeight - w.BorderThickness.Top - w.BorderThickness.Bottom;
                var m = content.Margin;
                var box = new Rect(-m.Left, -m.Top, cw, ch);
                var vb = new VisualBrush(content) { Stretch = Stretch.None, ViewboxUnits = BrushMappingMode.Absolute, Viewbox = box, AlignmentX = AlignmentX.Left, AlignmentY = AlignmentY.Top };
                var r = new Rect(0, 0, w.ActualWidth, w.ActualHeight);
                dc.DrawRectangle(w.Background, w.BorderBrush != null ? new Pen(w.BorderBrush, 1) : null, r);
                dc.DrawRectangle(vb, null, new Rect(w.BorderThickness.Left, w.BorderThickness.Top, cw, ch));
            }
            rtb.Render(dv);
            var enc = new PngBitmapEncoder();
            enc.Frames.Add(BitmapFrame.Create(rtb));
            Directory.CreateDirectory(Path.GetDirectoryName(file));
            using (var fs = File.Create(file)) enc.Save(fs);
        }
    }
}
