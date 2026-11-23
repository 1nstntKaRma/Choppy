using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Threading;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Documents;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using Choppy.Bench;
using Choppy.Engine;

namespace Choppy.UI
{
    /// <summary>
    /// Benchmark: Choppy against Windows' own copy on every drive of this PC. Lists the drives (and why one is left
    /// out), runs on its own thread with progress and an estimate, shows each case as it finishes, and saves the report.
    /// </summary>
    public partial class BenchmarkWindow : Window
    {
        Benchmark bench;
        Thread worker;
        readonly DispatcherTimer timer;
        List<BenchDrive> drives;
        BenchReport report;
        string htmlPath, jsonPath;
        int shown;                  // cases already in the list
        int scanNo;
        bool closeWhenDone;
        BenchDrive lastFrom, lastTo;

        static Brush Res(string key) { return (Brush)Application.Current.FindResource(key); }

        public BenchmarkWindow()
        {
            InitializeComponent();
            try { AppIcon.Source = ChoppyIcon.Frame(20); Icon = BitmapFrame.Create(new Uri(ChoppyIcon.Uri)); } catch { }
            timer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(250) };
            timer.Tick += (s, e) => Tick();
            BuildHead();
            Loaded += (o, e) => { Native2.Rounded(this); Scan(); };
            MouseLeftButtonDown += (o, e) => { if (e.GetPosition(this).Y < 46) try { DragMove(); } catch { } };
            KeyDown += (o, e) => { if (e.Key == System.Windows.Input.Key.Escape) Close(); };
            Closing += OnClosing;
        }

        // ================= drives =================

        /// <summary>Looks at the drives on a worker thread (a network drive can take a moment to answer).</summary>
        void Scan()
        {
            int no = ++scanNo;
            bool net = NetworkToggle.IsChecked == true;
            StartBtn.IsEnabled = false;
            Estimate.Text = "Looking at the drives...";
            ThreadPool.QueueUserWorkItem(_ =>
            {
                List<BenchDrive> found;
                try { found = Benchmark.FindDrives(net, null); } catch { found = new List<BenchDrive>(); }
                Dispatcher.BeginInvoke(new Action(() =>
                {
                    if (no != scanNo || worker != null || report != null) return;
                    drives = found;
                    Preview();
                }));
            });
        }

        /// <summary>The drives that would be tested and how long it would take, for the chosen mode.</summary>
        void Preview()
        {
            if (drives == null) return;
            var b = new Benchmark { Thorough = ModeThorough.IsChecked == true, Network = NetworkToggle.IsChecked == true };
            double est = b.Preview(drives);
            int used = drives.Count(d => d.Skip == null);
            ShowDrives(drives, false);
            Estimate.Text = used == 0 ? "" : "about " + Format.Short(TimeSpan.FromSeconds(Math.Max(60, est)));
            StartBtn.IsEnabled = used > 0;
            FootNote.Text = used == 0 ? "No drive here can be tested - each one's reason is listed above."
                : "Keep other work off these drives while it runs: it measures them.";
        }

        void ShowDrives(List<BenchDrive> list, bool measured)
        {
            DriveList.Children.Clear();
            // once it runs, the drives left out share one line (the window stays within the screen)
            bool compact = bench != null;   // started (running or done)
            var order = list.Where(d => !compact || d.Skip == null).OrderBy(d => d.Skip != null).ThenBy(d => !d.System).ThenBy(d => d.Letter).ToList();
            var left = list.Where(d => d.Skip != null).OrderBy(d => d.Letter).ToList();
            for (int i = 0; i < order.Count; i++)
            {
                var d = order[i];
                bool on = d.Skip == null;
                var g = new Grid { Margin = new Thickness(0, 7, 0, 7) };
                g.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(34) });
                g.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(190) });
                g.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
                g.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
                g.Children.Add(new TextBlock { Text = d.Letter, FontSize = 14.5, FontWeight = FontWeights.SemiBold, Foreground = on ? Res("B.Text") : Res("B.Faint") });
                Cell(g, 1, new TextBlock { Text = d.Label ?? d.Remote ?? "", FontSize = 14, Foreground = on ? Res("B.Text2") : Res("B.Faint"), TextTrimming = TextTrimming.CharacterEllipsis, Margin = new Thickness(0, 0, 12, 0) });
                string detail;
                if (on)
                {
                    var parts = new List<string> { d.Kind };
                    if (!string.IsNullOrEmpty(d.Model)) parts.Add(d.Model);
                    if (!string.IsNullOrEmpty(d.FileSystem)) parts.Add(d.FileSystem);
                    parts.Add(Format.Size(d.Free) + " free");
                    detail = string.Join(" · ", parts);
                }
                else detail = d.Skip;
                Cell(g, 2, new TextBlock { Text = detail, FontSize = 13, Foreground = on ? Res("B.Dim") : Res("B.Faint"), TextTrimming = TextTrimming.CharacterEllipsis, VerticalAlignment = VerticalAlignment.Center, ToolTip = detail });
                string state = !on ? "left out" : measured && d.Measured ? "read " + BenchText.Speed(d.ReadMBps) + " · write " + BenchText.Speed(d.WriteMBps) : d.System ? "system drive" : "";
                Cell(g, 3, new TextBlock { Text = state, FontSize = 13, Foreground = on ? (measured ? Res("B.SizeDone") : Res("B.Label")) : Res("B.Faint"), Margin = new Thickness(12, 0, 0, 0), VerticalAlignment = VerticalAlignment.Center });
                var row = new Border { Child = g, BorderBrush = Res("B.Sep"), BorderThickness = new Thickness(0, 0, 0, i < order.Count - 1 || (compact && left.Count > 0) ? 1 : 0) };
                DriveList.Children.Add(row);
            }
            if (compact && left.Count > 0)
            {
                string text = "Left out: " + string.Join(", ", left.Select(d => d.Letter + " " + d.Skip));
                DriveList.Children.Add(new TextBlock { Text = text, FontSize = 12.5, Foreground = Res("B.Faint"), TextTrimming = TextTrimming.CharacterEllipsis, Margin = new Thickness(0, 7, 0, 7), ToolTip = text });
            }
        }

        static void Cell(Grid g, int col, UIElement e) { Grid.SetColumn(e, col); g.Children.Add(e); }

        void Mode_Changed(object sender, RoutedEventArgs e) { if (IsLoaded && worker == null && report == null) Preview(); }

        void Network_Changed(object sender, RoutedEventArgs e) { if (IsLoaded && worker == null && report == null) Scan(); }

        // ================= results =================

        static readonly double[] Cols = { 150, -1, 112, 112, 70, 152 };

        Grid RowGrid()
        {
            var g = new Grid();
            foreach (var w in Cols) g.ColumnDefinitions.Add(new ColumnDefinition { Width = w < 0 ? new GridLength(1, GridUnitType.Star) : new GridLength(w) });
            return g;
        }

        void BuildHead()
        {
            var g = ResultsHead;
            foreach (var w in Cols) g.ColumnDefinitions.Add(new ColumnDefinition { Width = w < 0 ? new GridLength(1, GridUnitType.Star) : new GridLength(w) });
            string[] names = { "ROUTE", "WHAT", "CHOPPY", "WINDOWS", "RATIO", "RESULT" };
            for (int i = 0; i < names.Length; i++)
            {
                var t = new TextBlock { Text = names[i], Style = (Style)Application.Current.FindResource("T.Cap") };
                if (i >= 2 && i <= 4) { t.HorizontalAlignment = HorizontalAlignment.Right; t.Margin = new Thickness(0, 0, i == 4 ? 0 : 4, 0); }
                if (i == 5) t.Margin = new Thickness(22, 0, 0, 0);
                Cell(g, i, t);
            }
            ResultsHead.ToolTip = "MB/s until the copies are on the disk; under it, until each one said it was done";
        }

        void AddRow(BenchResult r)
        {
            bool newRoute = r.From != lastFrom || r.To != lastTo;
            lastFrom = r.From; lastTo = r.To;
            var g = RowGrid();
            g.Margin = new Thickness(0, 8, 0, 8);
            if (newRoute)
            {
                var route = new StackPanel();
                route.Children.Add(new TextBlock { Text = r.Route, FontSize = 14, FontWeight = FontWeights.SemiBold });
                route.Children.Add(new TextBlock { Text = r.RouteKinds, FontSize = 12, Foreground = Res("B.Faint"), TextTrimming = TextTrimming.CharacterEllipsis });
                Cell(g, 0, route);
            }
            var what = new StackPanel();
            what.Children.Add(new TextBlock { Text = r.Case.Title, FontSize = 14, Foreground = Res("B.Text2") });
            what.Children.Add(new TextBlock { Text = r.Case.Detail, FontSize = 12, Foreground = Res("B.Faint"), TextTrimming = TextTrimming.CharacterEllipsis });
            Cell(g, 1, what);
            if (!r.Scored)
            {
                var err = new TextBlock { Text = r.Error ?? "not measured", FontSize = 13, Foreground = Res("B.Warn"), TextWrapping = TextWrapping.Wrap, VerticalAlignment = VerticalAlignment.Center };
                Grid.SetColumnSpan(err, 3);
                Cell(g, 2, err);
                Cell(g, 5, Badge("Error", 0, true));
            }
            else
            {
                Cell(g, 2, Speed(r.ChoppyMBps, r.ChoppyDoneMBps, Res("B.NowSpeed")));
                Cell(g, 3, Speed(r.WindowsMBps, r.WindowsDoneMBps, Res("B.Text")));
                Cell(g, 4, new TextBlock { Text = BenchText.Ratio(r.Ratio), FontSize = 14, FontWeight = FontWeights.SemiBold, HorizontalAlignment = HorizontalAlignment.Right, Foreground = Res("B.Text2") });
                Cell(g, 5, Badge(r.Verdict > 0 ? "Choppy faster" : r.Verdict < 0 ? "Choppy slower" : "Tie", r.Verdict, false));
                g.ToolTip = "Choppy: " + string.Join(", ", r.Runs.Where(x => x.Choppy).Select(x => x.OnDisk.ToString("0.00") + " s"))
                    + "\nWindows: " + string.Join(", ", r.Runs.Where(x => !x.Choppy).Select(x => x.OnDisk.ToString("0.00") + " s"))
                    + "\nA tie within ±" + (r.Noise * 100).ToString("0.#") + "%" + (r.Method != null ? "\nChoppy's way: " + r.Method : "");
            }
            var row = new Border { Child = g, BorderBrush = Res("B.Sep"), BorderThickness = new Thickness(0, newRoute && ResultsList.Children.Count > 0 ? 1 : 0, 0, 0) };
            ResultsList.Children.Add(row);
            ResultsScroll.ScrollToEnd();
            FitToScreen();
        }

        /// <summary>The results list grows until the window reaches the bottom of the screen, then scrolls.</summary>
        void FitToScreen()
        {
            UpdateLayout();
            double room = SystemParameters.WorkArea.Height - 24;
            double other = ActualHeight - ResultsScroll.ActualHeight;
            ResultsScroll.MaxHeight = Math.Max(150, room - other);
            if (Top + ActualHeight > SystemParameters.WorkArea.Bottom && !TestHook.Park) Top = Math.Max(SystemParameters.WorkArea.Top, SystemParameters.WorkArea.Bottom - ActualHeight - 12);
        }

        static FrameworkElement Speed(double disk, double done, Brush color)
        {
            var sp = new StackPanel { HorizontalAlignment = HorizontalAlignment.Right, Margin = new Thickness(0, 0, 4, 0) };
            sp.Children.Add(new TextBlock { Text = BenchText.Speed(disk), FontSize = 14, FontWeight = FontWeights.SemiBold, Foreground = color, HorizontalAlignment = HorizontalAlignment.Right });
            sp.Children.Add(new TextBlock { Text = "done " + BenchText.Speed(done), FontSize = 12, Foreground = Res("B.Faint"), HorizontalAlignment = HorizontalAlignment.Right });
            return sp;
        }

        static FrameworkElement Badge(string text, int verdict, bool error)
        {
            Color c = error || verdict == 0 ? Color.FromRgb(0x9A, 0xA2, 0xAD) : verdict > 0 ? Color.FromRgb(0x4C, 0xC3, 0x8A) : Color.FromRgb(0xF0, 0x7A, 0x72);
            return new Border
            {
                CornerRadius = new CornerRadius(12),
                Padding = new Thickness(11, 3, 11, 4),
                Margin = new Thickness(22, 0, 0, 0),
                HorizontalAlignment = HorizontalAlignment.Left,
                VerticalAlignment = VerticalAlignment.Center,
                Background = new SolidColorBrush(Color.FromArgb(0x24, c.R, c.G, c.B)),
                BorderBrush = new SolidColorBrush(Color.FromArgb(0x70, c.R, c.G, c.B)),
                BorderThickness = new Thickness(1),
                Child = new TextBlock { Text = text, FontSize = 12.5, FontWeight = FontWeights.SemiBold, Foreground = new SolidColorBrush(c) }
            };
        }

        void ShowSummary(List<BenchResult> results, bool final)
        {
            int f = results.Count(r => r.Scored && r.Verdict > 0), t = results.Count(r => r.Scored && r.Verdict == 0), s = results.Count(r => r.Scored && r.Verdict < 0), n = f + t + s;
            SummaryText.Inlines.Clear();
            if (n == 0) { SummaryText.Inlines.Add(new Run(final ? "No case was measured" : "Each case shows here as it finishes")); SummaryText.Foreground = final ? Res("B.Text") : Res("B.Dim"); return; }
            SummaryText.Foreground = Res("B.Text");
            if (!final) SummaryText.Inlines.Add(new Run("So far: ") { Foreground = Res("B.Dim") });
            SummaryText.Inlines.Add(new Run("Choppy "));
            if (f == n) SummaryText.Inlines.Add(new Run("faster on all " + n) { Foreground = Res("B.Good") });
            else
            {
                SummaryText.Inlines.Add(new Run("faster on " + f) { Foreground = Res("B.Good") });
                SummaryText.Inlines.Add(new Run(" of " + n));
                if (t > 0) { SummaryText.Inlines.Add(new Run(" · ")); SummaryText.Inlines.Add(new Run("tie on " + t) { Foreground = Res("B.Dim") }); }
                if (s > 0) { SummaryText.Inlines.Add(new Run(" · ")); SummaryText.Inlines.Add(new Run("slower on " + s) { Foreground = Res("B.Bad") }); }
            }
        }

        // ================= running =================

        void Start_Click(object sender, RoutedEventArgs e)
        {
            if (worker != null) { AskStop(); return; }
            Begin(new Benchmark { Thorough = ModeThorough.IsChecked == true, Network = NetworkToggle.IsChecked == true });
        }

        void Begin(Benchmark b)
        {
            bench = b;
            report = null; htmlPath = jsonPath = null; shown = 0; lastFrom = lastTo = null;
            ResultsList.Children.Clear();
            ModeQuick.IsEnabled = ModeThorough.IsEnabled = NetworkToggle.IsEnabled = false;
            CopyBtn.Visibility = FolderBtn.Visibility = ReportBtn.Visibility = Visibility.Collapsed;
            StartBtn.Content = "Stop";
            StartBtn.Style = (Style)Application.Current.FindResource("Btn.Cancel");
            StartBtn.IsEnabled = true;
            ProgressPanel.Visibility = Visibility.Visible;
            ResultsPanel.Visibility = Visibility.Visible;
            Intro.Visibility = Visibility.Collapsed;
            if (drives != null) ShowDrives(drives, false);
            SummaryNote.Text = "";
            ShowSummary(new List<BenchResult>(), false);
            FootNote.Text = "Keep other work off these drives while it runs: it measures them.";
            Bar.Width = 0;
            StepText.Text = "Starting";
            TimeText.Text = "";
            worker = new Thread(() =>
            {
                BenchReport rep;
                try { rep = b.Run(); }
                catch (Exception ex) { rep = b.Report; rep.Error = ex.Message; }
                Dispatcher.BeginInvoke(new Action(() => Finished(rep)));
            }) { IsBackground = true, Name = "benchmark" };
            worker.Start();
            timer.Start();
        }

        void Tick()
        {
            if (bench == null) return;
            var s = bench.Status();
            if (bench.IsCanceled) StepText.Text = "Stopping - removing the test files...";
            else StepText.Text = s.Text;
            double track = ((FrameworkElement)Bar.Parent).ActualWidth;
            Bar.Width = Math.Max(0, track * s.Fraction);
            TimeText.Text = Format.Short(s.Elapsed) + (s.Elapsed.TotalSeconds > 8 && !bench.IsCanceled ? " · about " + Format.Short(s.Remaining) + " left" : "");
            if (s.Results > shown) AddNew(bench.ResultsSoFar(), false);
        }

        void AddNew(List<BenchResult> all, bool final)
        {
            for (int i = shown; i < all.Count; i++) AddRow(all[i]);
            shown = all.Count;
            ShowSummary(all, final);
        }

        void Finished(BenchReport rep)
        {
            timer.Stop();
            worker = null;
            report = rep;
            AddNew(rep.Results.ToList(), true);
            ProgressPanel.Visibility = Visibility.Collapsed;
            string saved = null;
            try { rep.Save(null, out htmlPath, out jsonPath); saved = Path.GetFileName(htmlPath); }
            catch (Exception ex) { saved = null; rep.Notes.Add("The report couldn't be saved: " + ex.Message); }
            var notes = new List<string>();
            if (rep.Canceled) notes.Add("Stopped before the end - the cases that finished are shown.");
            if (rep.Error != null) notes.Add(rep.Error);
            if (rep.Scored > 0)
                notes.Add("Counted by the time until the copies are on the disk; within " + (int)(Benchmark.TieMargin * 100) + "% (or within each one's own spread) is a tie. "
                    + "By the time each one says it's done: " + rep.DoneSummary.Replace("Choppy faster", "faster") + ".");
            SummaryNote.Text = string.Join(" ", notes);
            if (rep.Drives != null) ShowDrives(rep.Drives, true);
            ResultsScroll.ScrollToTop();
            FitToScreen();
            Estimate.Text = "took " + BenchText.Duration(rep.Seconds);
            FootNote.Text = saved != null ? "Saved in Choppy's Logs folder: " + saved + " and .json" + (rep.Notes.Count > 0 ? " · " + rep.Notes[0] : "")
                : string.Join(" ", rep.Notes);
            CopyBtn.Visibility = rep.Results.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
            FolderBtn.Visibility = ReportBtn.Visibility = htmlPath != null ? Visibility.Visible : Visibility.Collapsed;
            ReportBtn.Style = (Style)Application.Current.FindResource("Btn.Primary");
            StartBtn.Style = (Style)Application.Current.FindResource("Btn");
            StartBtn.Content = "Run again";
            StartBtn.IsEnabled = true;
            ModeQuick.IsEnabled = ModeThorough.IsEnabled = NetworkToggle.IsEnabled = true;
            if (closeWhenDone) Close();
        }

        bool AskStop()
        {
            if (worker == null || bench.IsCanceled) return worker != null;
            if (!Dialogs.Ask(this, "Stop the benchmark?", "Its test files are removed. The cases that finished stay in the report.", "Stop", "Keep going")) return false;
            if (worker == null) return false;   // it ended while the question was up
            bench.Cancel();
            StartBtn.IsEnabled = false;
            StartBtn.Content = "Stopping...";
            return true;
        }

        void OnClosing(object sender, System.ComponentModel.CancelEventArgs e)
        {
            if (worker == null) { timer.Stop(); return; }
            e.Cancel = true;
            // running: stop first (its test files are removed), then the window closes by itself
            if (bench.IsCanceled || AskStop()) closeWhenDone = true;
        }

        // ================= after =================

        void Copy_Click(object sender, RoutedEventArgs e)
        {
            if (report == null) return;
            for (int i = 0; i < 5; i++)
            {
                try { Clipboard.SetText(report.ToText()); FootNote.Text = "Copied - a short table to paste in a chat or an issue."; return; }
                catch { Thread.Sleep(60); }
            }
            FootNote.Text = "The clipboard is busy - try again.";
        }

        void Report_Click(object sender, RoutedEventArgs e)
        {
            if (htmlPath != null) try { Process.Start(new ProcessStartInfo(htmlPath) { UseShellExecute = true }); } catch { }
        }

        void Folder_Click(object sender, RoutedEventArgs e)
        {
            try
            {
                if (htmlPath != null && File.Exists(htmlPath)) Process.Start("explorer.exe", "/select,\"" + htmlPath + "\"");
                else { Directory.CreateDirectory(AppPaths.LogDir); Process.Start("explorer.exe", "\"" + AppPaths.LogDir + "\""); }
            }
            catch { }
        }

        void Close_Click(object sender, RoutedEventArgs e) { Close(); }

        // ================= test hook =================

        /// <summary>"start tiny C,D" / "start quick" / "stop" (the test hook drives the window without the mouse).</summary>
        internal string ForTest(string cmd)
        {
            var p = cmd.Split(new[] { ' ' }, StringSplitOptions.RemoveEmptyEntries);
            if (p.Length == 0) return "no command";
            switch (p[0])
            {
                case "start":
                    {
                        if (worker != null) return "running";
                        var b = new Benchmark { Thorough = p.Length > 1 && p[1] == "thorough", Tiny = p.Length > 1 && p[1] == "tiny", Network = NetworkToggle.IsChecked == true };
                        if (p.Length > 2) b.Only = new HashSet<string>(p[2].Split(',').Select(x => x.Trim().TrimEnd(':').ToUpperInvariant() + ":"));
                        Begin(b);
                        return "ok";
                    }
                case "stop":
                    if (worker == null) return "not running";
                    bench.Cancel();
                    StartBtn.IsEnabled = false;
                    StartBtn.Content = "Stopping...";
                    return "ok";
                case "state":
                    return worker != null ? "running " + (int)(bench.Status().Fraction * 100) + "%" : report != null ? "done: " + report.Summary + (report.Canceled ? " (stopped)" : "") + " | " + htmlPath : "idle";
            }
            return "unknown";
        }
    }
}
