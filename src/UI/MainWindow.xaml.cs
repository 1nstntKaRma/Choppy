using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.Linq;
using System.Threading;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Shell;
using System.Windows.Threading;
using Choppy.Engine;

namespace Choppy.UI
{
    /// <summary>
    /// The copy window: compact view (from / to, bar, sizes, times, current file) or extended view (badge, speed graph,
    /// current file, largest files, quick toggles, queue). Refreshed every 100 ms straight from the engine.
    /// Color code: blue = speed right now, lavender = average of this copy, green = done, peach = total, amber = time.
    /// </summary>
    public partial class MainWindow : Window
    {
        internal JobManager Manager;
        CopyJob job;                 // the job on screen
        readonly DispatcherTimer timer;
        long lastSamples = -1;       // the engine's sample counter when the graph was last drawn
        DateTime? closeAt;
        string shownCur;
        DateTime shownCurAt;
        bool loadingQuick;
        bool reportShown;
        bool routeLabeled;           // drive names filled in once the job has looked at the drives
        DateTime lastTray;

        static readonly Color[] SplitColors =
        {
            Color.FromRgb(0x4F, 0xC3, 0xF7), Color.FromRgb(0x7F, 0xE3, 0xD4), Color.FromRgb(0xB7, 0x9C, 0xFF), Color.FromRgb(0xF5, 0xC5, 0x42)
        };
        static readonly Color OtherColor = Color.FromRgb(0x5F, 0x67, 0x72);

        internal MainWindow(JobManager mgr)
        {
            InitializeComponent();
            Manager = mgr;
            try { AppIcon.Source = BitmapFrame.Create(new Uri("pack://application:,,,/Choppy;component/Choppy.ico")); } catch { }
            try { Icon = AppIcon.Source; } catch { }
            var dim = (Brush)FindResource("B.Dim");
            FromPath.ParentBrush = dim; FromPath.LeafBrush = dim; FromPath.FontSize = 14.5;
            ToPath.ParentBrush = dim; ToPath.LeafBrush = dim; ToPath.FontSize = 14.5;
            if (App.Current2 != null && App.Current2.Elevated) TitleExtra.Text = "· Administrator";
            ApplyView(false);
            SyncQuick();
            timer = new DispatcherTimer(DispatcherPriority.Render) { Interval = TimeSpan.FromMilliseconds(100) };
            timer.Tick += (s, e) => Tick();
            Manager.Changed += OnManagerChanged;
            Loaded += (s, e) => { Native2.Rounded(this); OnManagerChanged(); timer.Start(); };
            Closed += (s, e) => { timer.Stop(); Manager.Changed -= OnManagerChanged; };
            MouseLeftButtonDown += (s, e) => { if (e.ButtonState == MouseButtonState.Pressed && e.GetPosition(this).Y < 46) { try { DragMove(); } catch { } } };
            KeyDown += OnKey;
        }

        void OnKey(object sender, KeyEventArgs e)
        {
            if (LimitBox.IsKeyboardFocusWithin) { if (e.Key == Key.Enter || e.Key == Key.Escape) { Keyboard.ClearFocus(); FocusManager.SetFocusedElement(this, this); } return; }
            if (e.Key == Key.Escape && job != null && job.IsFinished) LeaveReport();
            if (e.Key == Key.Space && job != null && !job.IsFinished) { Pause_Click(null, null); e.Handled = true; }
        }

        // ================= compact / extended =================

        bool Extended { get { return Settings.Current.Extended; } }

        void ApplyView(bool keepOnScreen)
        {
            bool ext = Extended;
            var on = Visibility.Visible; var off = Visibility.Collapsed;
            Badge.Visibility = ext ? on : off;
            PercentExt.Visibility = ext ? on : off;
            PercentCompact.Visibility = ext ? off : on;
            CompactSpeeds.Visibility = ext ? off : on;
            CompactLine.Visibility = ext ? off : on;
            FilesText.Visibility = ext ? on : off;
            ExtPanel.Visibility = ext ? on : off;
            Grid.SetColumnSpan(BarTrack, ext ? 6 : 5);
            Grid.SetColumnSpan(Stats, ext ? 6 : 5);
            BtnSkip.Visibility = ext ? on : off;
            BtnQueue.Visibility = ext ? on : off;
            BtnLog.Visibility = ext && job != null && job.IsFinished && job.LogPath != null ? on : off;
            ViewText.Text = ext ? "Compact" : "Extended";
            ViewChevron.Text = ext ? "\uE70E" : "\uE70D";
            if (keepOnScreen) Dispatcher.BeginInvoke(new Action(KeepOnScreen), DispatcherPriority.Loaded);
        }

        /// <summary>After growing (extended view), keep the whole window on the screen it's on.</summary>
        void KeepOnScreen()
        {
            var wa = SystemParameters.WorkArea;
            if (Top + ActualHeight > wa.Bottom) Top = Math.Max(wa.Top, wa.Bottom - ActualHeight - 8);
        }

        void View_Click(object sender, RoutedEventArgs e)
        {
            var s = Settings.Current;
            s.Extended = !s.Extended;
            s.Save();
            ApplyView(true);
            lastSamples = -1;
            Tick();
        }

        // ================= job binding =================

        // a finished job with problems stays on screen until it is dismissed, even when the next copy starts
        readonly HashSet<CopyJob> dismissed = new HashSet<CopyJob>();

        static bool HadProblems(CopyJob j)
        {
            return j.IsFinished && (j.FailedFiles > 0 || j.SkippedFiles > 0 || j.Phase == JobPhase.Failed);
        }

        bool Pinned { get { return job != null && HadProblems(job) && !dismissed.Contains(job); } }

        void OnManagerChanged()
        {
            if (!Dispatcher.CheckAccess()) { Dispatcher.BeginInvoke(new Action(OnManagerChanged)); return; }
            var cur = Manager.Current ?? Manager.LastFinished;
            if (cur != job && !Pinned) ShowJob(cur);
            RefreshQueue();
            Tick();
            UpdateCloseButton();
        }

        void UpdateCloseButton()
        {
            if (job == null || !job.IsFinished) return;
            var next = Manager.Current;
            BtnCancel.Content = next != null && next != job ? "Next copy ›" : "Close";
        }

        /// <summary>Report "Close": go on to the copy that is running now, or close the window.</summary>
        void LeaveReport()
        {
            if (job != null) dismissed.Add(job);
            var next = Manager.Current;
            if (next != null && next != job) { ShowJob(next); Tick(); }
            else Close();
        }

        // ---------- from / to ----------

        /// <summary>The folder the files come from (their common folder when there are several).</summary>
        static string SourceFolder(CopyJob j)
        {
            var parents = j.Spec.Sources.Select(s => Paths.Parent(Paths.ToShort(s)) ?? Paths.ToShort(s)).Distinct(StringComparer.OrdinalIgnoreCase).ToList();
            if (parents.Count == 0) return "";
            string common = parents[0];
            foreach (var p in parents.Skip(1))
                while (common != null && !Paths.IsSameOrInside(p, common)) common = Paths.Parent(common);
            return common ?? parents[0];
        }

        /// <summary>"D: Media", "N: NAS", or the server of a \\server\share path.</summary>
        static string DriveName(string shortPath, VolumeInfo v)
        {
            if (string.IsNullOrEmpty(shortPath)) return "-";
            if (shortPath.StartsWith(@"\\"))
            {
                var parts = shortPath.TrimStart('\\').Split('\\');
                return parts.Length > 0 ? parts[0] : shortPath;
            }
            string letter = shortPath.Length >= 2 && shortPath[1] == ':' ? shortPath.Substring(0, 2).ToUpperInvariant() : shortPath;
            string label = v != null ? v.Label : null;
            return string.IsNullOrEmpty(label) ? letter : letter + " " + label;
        }

        void ShowRoute(CopyJob j)
        {
            string from = SourceFolder(j), to = Paths.ToShort(j.Spec.Dest);
            FromName.Text = DriveName(from, j.SrcVol);
            ToName.Text = DriveName(to, j.DstVol);
            FromPath.SetPath(from);
            ToPath.SetPath(to);
            bool fromNet = from.StartsWith(@"\\") || (j.SrcVol != null && j.SrcVol.IsNetwork);
            bool toNet = to.StartsWith(@"\\") || (j.DstVol != null && j.DstVol.IsNetwork);
            FromIcon.Data = (Geometry)FindResource(fromNet ? "G.Server" : "G.Drive");
            ToIcon.Data = (Geometry)FindResource(toNet ? "G.Server" : "G.Drive");
            FromName.ToolTip = string.Join("\n", j.Spec.Sources.Take(12).Select(Paths.ToShort)) + (j.Spec.Sources.Count > 12 ? "\n…" : "");
            routeLabeled = j.SrcVol != null && j.DstVol != null;
        }

        void ShowJob(CopyJob j)
        {
            job = j;
            closeAt = null;
            reportShown = false;
            lastSamples = -1;
            Graph.Clear();
            shownCur = null;
            if (j == null) return;
            ShowRoute(j);
            var extra = new List<string>();
            if (j.Spec.Op == OpKind.Move) extra.Add("Move");
            if (j.Spec.Elevated) extra.Add("Administrator");
            TitleExtra.Text = extra.Count > 0 ? "· " + string.Join(" · ", extra) : "";
            Legend.Children.Clear();
            DonutChart.SetParts(new KeyValuePair<double, Color>[0]);
            splitShown = false;
            CurrentSection.Visibility = Visibility.Visible;
            ReportSection.Visibility = Visibility.Collapsed;
            RunButtons.Visibility = Visibility.Visible;
            DoneButtons.Visibility = Visibility.Collapsed;
            BtnCancel.Content = "Cancel";
            BtnPause.Content = "Pause";
            C2Label.Foreground = (Brush)FindResource("B.Dim");
            C2Name.Foreground = (Brush)FindResource("B.Text2");
            ApplyView(false);
            SyncQuick();
        }

        // ---------- largest files ----------

        bool splitShown;

        void FillSplit()
        {
            if (splitShown || job == null || job.Scan == null) return;
            if (job.Phase == JobPhase.Analyzing || job.Phase == JobPhase.Queued) return;
            splitShown = true;
            Legend.Children.Clear();
            var parts = new List<KeyValuePair<double, Color>>();
            long top = 0;
            int i = 0;
            foreach (var kv in job.Scan.Largest)
            {
                if (i >= 4) break;
                var c = SplitColors[i];
                parts.Add(new KeyValuePair<double, Color>(kv.Value, c));
                AddLegend(c, kv.Key, kv.Value);
                top += kv.Value;
                i++;
            }
            int others = job.Scan.Files.Count - i;
            long otherBytes = job.Scan.TotalBytes - top;
            if (others > 0)
            {
                parts.Add(new KeyValuePair<double, Color>(Math.Max(otherBytes, 1), OtherColor));
                AddLegend(OtherColor, "Other " + Format.Count(others) + (others == 1 ? " file" : " files"), otherBytes);
            }
            if (job.Scan.Files.Count == 0 && job.Plan != null && job.Plan.InstantMove)
                AddLegend(OtherColor, "Renamed in place - nothing to copy", -1);
            DonutChart.SetParts(parts);
        }

        /// <summary>One legend row: the whole name (cut only when it really doesn't fit) and its size.</summary>
        void AddLegend(Color c, string name, long size)
        {
            var g = new Grid { Margin = new Thickness(0, Legend.Children.Count == 0 ? 0 : 6, 0, 0) };
            g.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            g.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            g.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            var sq = new Border { Width = 10, Height = 10, CornerRadius = new CornerRadius(2), Background = new SolidColorBrush(c), VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(0, 1, 10, 0) };
            var nm = new TextBlock { Text = name, FontSize = 14, Foreground = (Brush)FindResource("B.Text2"), TextTrimming = TextTrimming.CharacterEllipsis, ToolTip = name };
            Grid.SetColumn(nm, 1);
            g.Children.Add(sq); g.Children.Add(nm);
            if (size >= 0)
            {
                var sz = new TextBlock { Text = Format.Size(size), FontSize = 14, Foreground = (Brush)FindResource("B.Dim"), Margin = new Thickness(14, 0, 0, 0) };
                Grid.SetColumn(sz, 2);
                g.Children.Add(sz);
            }
            Legend.Children.Add(g);
        }

        // ================= live refresh =================

        bool UseMbps(CopyJob j)
        {
            string u = Settings.Current.SpeedUnit;
            if (u == "mbps") return true;
            if (u == "mbs") return false;
            return j != null && ((j.SrcVol != null && j.SrcVol.IsNetwork) || (j.DstVol != null && j.DstVol.IsNetwork));
        }

        string Speed(double bps, bool mbps)
        {
            string n, u;
            Format.Speed(bps, mbps, out n, out u);
            return n + " " + u;
        }

        void Tick()
        {
            var j = job;
            if (j == null) return;
            var phase = j.Phase;
            bool finished = j.IsFinished;
            if (!routeLabeled && j.SrcVol != null && j.DstVol != null) ShowRoute(j);
            SyncQuick();

            // badge: "Adaptive  1 stream"
            if (j.AsAdmin) { BadgeMode.Text = "Administrator"; BadgeDetail.Text = "own window"; }
            else if (j.IsPaused) { BadgeMode.Text = "Paused"; BadgeDetail.Text = ""; }
            else if (phase == JobPhase.Waiting) { BadgeMode.Text = "Waiting"; BadgeDetail.Text = "for the network"; }
            else if (j.Plan != null) { BadgeMode.Text = j.Plan.Mode; BadgeDetail.Text = j.Plan.Detail; Badge.ToolTip = j.Plan.Why; }
            else { BadgeMode.Text = "Adaptive"; BadgeDetail.Text = phase == JobPhase.Queued ? "waiting in the queue" : "checking the drives"; }
            BadgeDetail.Visibility = BadgeDetail.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
            BadgeDot.Fill = j.IsPaused || phase == JobPhase.Waiting ? (Brush)FindResource("B.Warn") : (Brush)FindResource("B.NowSpeed");

            // counters
            long total = Math.Max(0, j.TotalBytes - j.SkippedBytes - j.FailedBytes);
            long done = Math.Max(0, Math.Min(total, Interlocked.Read(ref j.DoneBytes)));
            int filesTotal = Math.Max(0, j.TotalFiles - j.SkippedFiles - j.FailedFiles);
            int filesDone = Math.Min(j.DoneFiles, Math.Max(filesTotal, j.DoneFiles));
            double frac;
            if (phase == JobPhase.Analyzing) frac = 0;
            else if (total > 0) frac = (double)done / total;
            else frac = filesTotal > 0 ? (double)filesDone / filesTotal : (finished ? 1 : 0);
            if (phase == JobPhase.Done) frac = 1;
            frac = Math.Max(0, Math.Min(1, frac));
            string pct = phase == JobPhase.Analyzing ? "…" : ((int)Math.Floor(frac * 100)) + "%";
            PercentExt.Text = pct; PercentCompact.Text = pct;
            Bar.Width = Math.Max(frac > 0 ? 7 : 0, BarTrack.ActualWidth * frac);

            // speeds: now (blue) + average (lavender); the time left uses the average
            bool mbps = UseMbps(j);
            double now;
            List<double> hist = null;
            lock (j.SpeedLock)
            {
                now = j.SpeedSecond;
                // redraw on every new sample: once the history is full (600) its size stops changing, the counter doesn't
                if (j.SpeedSamples != lastSamples) { hist = new List<double>(j.SpeedHistory); lastSamples = j.SpeedSamples; }
            }
            double avg = j.AverageSpeed;
            bool running = !finished && phase == JobPhase.Copying && !j.IsPaused;
            TimeSpan? left = null;
            if (running && avg > 1 && total > done && j.CopyClock.Elapsed.TotalSeconds >= 2)
            {
                var l = TimeSpan.FromSeconds((total - done) / avg);
                if (l.TotalDays < 7) left = l;
            }

            string sn, su, an, au;
            Format.Speed(running ? now : 0, mbps, out sn, out su);
            Format.Speed(avg, mbps, out an, out au);
            SpNum.Text = finished || !running ? "–" : sn; SpUnit.Text = finished || !running ? "" : " " + su;
            SpAvgNum.Text = avg > 0 ? an : "–"; SpAvgUnit.Text = avg > 0 ? " " + au : "";
            CNow.Text = running ? sn + " " + su : "";
            CAvgLabel.Text = avg > 0 ? (running ? "    AVG  " : "AVG  ") : "";
            CAvg.Text = avg > 0 ? an + " " + au : "";
            if (hist != null && !finished) Graph.SetSamples(hist);
            if (avg > 0) Graph.SetAverage(avg);

            // sizes (green done, peach total), files, times (amber)
            if (phase == JobPhase.Analyzing && j.Scanner != null)
            {
                SizeDone.Text = Format.Size(Interlocked.Read(ref j.Scanner.BytesSeen)); SizeOf.Text = " found"; SizeTotal.Text = "";
                FilesText.Text = Format.Count(j.Scanner.FilesSeen) + " files";
            }
            else if (phase == JobPhase.Queued)
            {
                SizeDone.Text = ""; SizeOf.Text = "Waiting in the queue"; SizeTotal.Text = ""; FilesText.Text = "";
            }
            else
            {
                string a, b;
                Format.Pair(finished ? Math.Min(done, total) : done, total, out a, out b);
                SizeDone.Text = a; SizeOf.Text = " of "; SizeTotal.Text = b;
                FilesText.Text = Format.Count(filesDone) + " of " + Format.Count(filesTotal) + (filesTotal == 1 ? " file" : " files");
            }
            ElapsedLabel.Text = "Elapsed ";
            ElapsedVal.Text = Format.Short(j.Clock.Elapsed);
            if (finished) { RemainLabel.Text = ""; RemainVal.Text = ""; }
            else
            {
                RemainLabel.Text = "    Remaining ";
                RemainVal.Text = j.IsPaused ? "paused" : left.HasValue ? Format.Short(left.Value) : "…";
            }

            // taskbar + title + tray
            Taskbar.ProgressState = finished ? TaskbarItemProgressState.None
                : j.IsPaused || phase == JobPhase.Waiting ? TaskbarItemProgressState.Paused
                : phase == JobPhase.Analyzing ? TaskbarItemProgressState.Indeterminate : TaskbarItemProgressState.Normal;
            Taskbar.ProgressValue = frac;
            string speeds = running ? Speed(now, mbps) + " (avg " + Speed(avg, mbps) + ")" : null;
            Title = finished ? "Choppy" : ((int)(frac * 100)) + "%" + (speeds != null ? " · " + speeds : "") + " - Choppy";
            if ((DateTime.Now - lastTray).TotalMilliseconds > 1000)
            {
                lastTray = DateTime.Now;
                App.Current2.SetTrayText(finished ? "Choppy" : "Choppy " + ((int)(frac * 100)) + "%" + (speeds != null ? " · " + speeds : "") + (left.HasValue ? " · " + Format.Short(left.Value) + " left" : ""));
            }

            FillSplit();
            if (finished) { ShowReport(j); return; }

            BtnPause.Content = j.IsPaused ? "Resume" : "Pause";
            BtnSkip.IsEnabled = j.ActiveCount > 0 && !j.IsPaused && !j.AsAdmin;

            // compact line + current file card
            var dimB = (Brush)FindResource("B.Dim");
            C2Label.Foreground = dimB;
            var t = j.CurrentTransfer();
            string retryFile = j.RetryFile;
            double fw = ((FrameworkElement)CurBar.Parent).ActualWidth;
            if (phase == JobPhase.Analyzing || phase == JobPhase.Queued)
            {
                CurLabel.Text = "ANALYZING";
                CurName.Text = phase == JobPhase.Queued ? "Waiting for the copy before it" : j.Scanner != null ? j.Scanner.Stage + "…" : "Checking the drives…";
                CurDone.Text = ""; CurOf.Text = ""; CurTotal.Text = ""; CurBar.Width = 0;
                C2Label.Text = phase == JobPhase.Queued ? "Queued  " : "Analyzing  ";
                C2Name.Text = CurName.Text;
                return;
            }
            if (phase == JobPhase.Finishing)
            {
                CurLabel.Text = "FINISHING"; CurName.Text = "Folder dates and attributes"; CurDone.Text = ""; CurOf.Text = ""; CurTotal.Text = ""; CurBar.Width = fw;
                C2Label.Text = "Finishing  "; C2Name.Text = "folder dates and attributes";
                return;
            }
            if (phase == JobPhase.Waiting)
            {
                C2Label.Foreground = (Brush)FindResource("B.Warn");
                C2Label.Text = "Waiting for the network  ";
                C2Name.Text = "it goes on by itself";
            }
            CurLabel.Text = j.AsAdmin ? "CURRENT FILE · AS ADMINISTRATOR" : "CURRENT FILE";
            if (t == null && retryFile != null)
            {
                CurName.Text = retryFile;
                CurDone.Text = ""; CurOf.Text = j.RetryWhy ?? "Trying again"; CurTotal.Text = "";
                CurBar.Width = 0;
                if (phase != JobPhase.Waiting) { C2Label.Text = "Waiting  "; C2Name.Text = retryFile + " - " + (j.RetryWhy ?? "trying again"); }
            }
            else if (t == null && j.AsAdmin)
            {
                CurName.Text = j.Status ?? "Copying as administrator";
                CurDone.Text = ""; CurOf.Text = ""; CurTotal.Text = ""; CurBar.Width = 0;
                C2Label.Text = "As administrator  "; C2Name.Text = "in its own window";
            }
            else if (t != null)
            {
                // with many small files in flight, hold each name a moment so it can be read
                if (shownCur == null || t.File.Name == shownCur || (DateTime.Now - shownCurAt).TotalMilliseconds > 350 || t.File.Size > (32 << 20))
                {
                    if (t.File.Name != shownCur) { shownCur = t.File.Name; shownCurAt = DateTime.Now; }
                    CurName.Text = t.File.Name;
                    CurName.ToolTip = Paths.ToShort(t.File.Src);
                    string verb;
                    if (t.Verifying)
                    {
                        long vtotal = Math.Max(1, t.File.Size * (j.AsAdmin ? 1 : 2));
                        double vf = Math.Min(1, (double)t.VerifyDone / vtotal);
                        if (j.AsAdmin) vf = Math.Min(1, (double)t.Done / Math.Max(1, t.File.Size));
                        CurDone.Text = ""; CurOf.Text = "checking the copy " + (int)(vf * 100) + "%"; CurTotal.Text = "";
                        CurBar.Width = fw * vf;
                        verb = "Checking  ";
                    }
                    else
                    {
                        if (t.File.Size > 0)
                        {
                            string a, b;
                            Format.Pair(t.Done, t.File.Size, out a, out b);
                            CurDone.Text = a; CurOf.Text = " of "; CurTotal.Text = b;
                        }
                        else { CurDone.Text = ""; CurOf.Text = "empty file"; CurTotal.Text = ""; }
                        CurBar.Width = t.File.Size > 0 ? fw * Math.Min(1, (double)t.Done / t.File.Size) : fw;
                        verb = j.Spec.Op == OpKind.Move ? "Moving  " : "Copying  ";
                    }
                    if (phase != JobPhase.Waiting)
                    {
                        C2Label.Text = j.IsPaused ? "Paused  " : verb;
                        C2Name.Text = t.File.Name;
                    }
                }
            }
            if (j.IsPaused)
            {
                C2Label.Foreground = (Brush)FindResource("B.Warn");
                if (t == null) { C2Label.Text = "Paused  "; C2Name.Text = j.Status ?? "press Resume to go on"; }
                else if (j.Status != null && j.Status.StartsWith("The destination is full")) C2Name.Text = j.Status;
            }
        }

        static double FinalAverage(CopyJob j)
        {
            double s = j.CopyClock.Elapsed.TotalSeconds;
            if (s <= 0) s = j.Clock.Elapsed.TotalSeconds;
            return s > 0 ? j.CopiedBytes / s : 0;
        }

        // ================= end report =================

        string problemNote;

        void ShowReport(CopyJob j)
        {
            bool canceled = j.Phase == JobPhase.Canceled, failed = j.Phase == JobPhase.Failed;
            bool clean = !canceled && !failed && j.FailedFiles == 0 && j.SkippedFiles == 0;
            bool mbps = UseMbps(j);
            double avg = FinalAverage(j);
            if (!reportShown)
            {
                reportShown = true;
                CurrentSection.Visibility = Visibility.Collapsed;
                ReportSection.Visibility = Visibility.Visible;
                RunButtons.Visibility = Visibility.Collapsed;
                DoneButtons.Visibility = Visibility.Visible;
                BtnCancel.Content = "Close";
                Taskbar.ProgressState = TaskbarItemProgressState.None;
                if (avg > 0) { string an, au; Format.Speed(avg, mbps, out an, out au); SpAvgNum.Text = an; SpAvgUnit.Text = " " + au; CAvgLabel.Text = "AVG  "; CAvg.Text = an + " " + au; }

                RepLabel.Text = canceled ? "CANCELED" : failed ? "STOPPED" : clean ? "DONE" : "DONE WITH PROBLEMS";
                RepLabel.Foreground = (Brush)FindResource(canceled || failed ? "B.Warn" : clean ? "B.Good" : "B.Warn");
                var dur = j.Clock.Elapsed;
                string verb = j.Spec.Op == OpKind.Move ? "Moved" : "Copied";
                string summary;
                if (j.Plan != null && j.Plan.InstantMove && j.FailedFiles == 0)
                    summary = verb + " " + Format.Count(j.DoneFiles) + (j.DoneFiles == 1 ? " item" : " items") + " instantly (same drive)";
                else if (failed)
                    summary = j.Status ?? "The copy stopped.";
                else if (canceled)
                    summary = verb + " " + Format.Count(j.DoneFiles) + (j.DoneFiles == 1 ? " file" : " files") + " · " + Format.Size(j.CopiedBytes) + " before canceling";
                else
                    summary = verb + " " + Format.Count(j.DoneFiles) + (j.DoneFiles == 1 ? " file" : " files") + " · " + Format.Size(j.CopiedBytes) + " in " + Format.Duration(dur);
                RepTitle.Text = summary;

                RepRows.Children.Clear();
                if (avg > 0 && !canceled && !failed) AddChip("avg " + Speed(avg, mbps), "B.AvgSpeed");
                if (j.AdminFiles > 0) AddChip("As administrator " + Format.Count(j.AdminFiles), "B.AccentText");
                if (j.VerifiedFiles > 0) AddChip("Verified " + Format.Count(j.VerifiedFiles), "B.Good");
                if (j.AlreadyThereFiles > 0) AddChip("Already there " + Format.Count(j.AlreadyThereFiles), "B.Dim");
                if (j.SkippedFiles > 0) AddChip("Skipped " + Format.Count(j.SkippedFiles), "B.Warn");
                if (j.FailedFiles > 0) AddChip("Failed " + Format.Count(j.FailedFiles), "B.Bad");
                if (canceled) AddChip("Not copied " + Format.Count(Math.Max(0, j.TotalFiles - j.DoneFiles - j.SkippedFiles - j.FailedFiles)), "B.Dim");
                RepRows.Visibility = RepRows.Children.Count > 0 ? Visibility.Visible : Visibility.Collapsed;

                problemNote = null;
                if (j.FailedItems.Count > 0)
                {
                    var f = j.FailedItems[0];
                    problemNote = f.Name + ": " + f.Error + (j.FailedItems.Count > 1 ? "  (+" + (j.FailedItems.Count - 1) + " more in the log)" : "");
                }
                else if (j.SkippedFiles > 0) problemNote = Format.Count(j.SkippedFiles) + (j.SkippedFiles == 1 ? " file was" : " files were") + " skipped (already there).";
                else
                {
                    // "Copied as administrator: ..." is in the chips already, not a problem
                    var notes = j.Notes.Where(n => !n.StartsWith("Copied as administrator") && !n.StartsWith("Moved as administrator")).ToList();
                    if (notes.Count > 0) problemNote = notes[0] + (notes.Count > 1 ? "  (+" + (notes.Count - 1) + " more in the log)" : "");
                }

                // compact: the outcome in the line under the numbers
                C2Label.Foreground = (Brush)FindResource(canceled || failed ? "B.Warn" : clean ? "B.Good" : "B.Warn");
                C2Label.Text = (canceled ? "Canceled" : failed ? "Stopped" : clean ? "Done" : "Done with problems") + "  ";

                BtnRetry.Visibility = j.FailedItems.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
                BtnLog.Visibility = Extended && j.LogPath != null ? Visibility.Visible : Visibility.Collapsed;
                BtnOpen.Visibility = canceled && j.DoneFiles == 0 ? Visibility.Collapsed : Visibility.Visible;

                if (clean && Settings.Current.CloseWhenDone && !Manager.HasWaiting && AutoCloseAllowed) closeAt = DateTime.Now.AddSeconds(4);
                UpdateCloseButton();
            }
            if (closeAt.HasValue)
            {
                if (IsMouseOver) closeAt = DateTime.Now.AddSeconds(4);
                else if (DateTime.Now >= closeAt.Value) { closeAt = null; Close(); return; }
            }
            string countdown = closeAt.HasValue ? "closing in " + Math.Max(1, (int)Math.Ceiling((closeAt.Value - DateTime.Now).TotalSeconds)) + " s" : null;
            if (problemNote != null)
            {
                C2Name.Text = problemNote;
                C2Name.Foreground = (Brush)FindResource(j.FailedFiles > 0 ? "B.Bad" : "B.Warn");
            }
            else
            {
                C2Name.Text = RepTitle.Text.Replace((j.Spec.Op == OpKind.Move ? "Moved" : "Copied") + " ", "") + (countdown != null ? " · " + countdown : "");
                C2Name.Foreground = (Brush)FindResource("B.Text2");
            }
            string note = problemNote ?? (countdown != null ? "Closing in " + countdown.Substring(11) + " (Close at completion)" : null);
            RepNote.Text = note ?? "";
            RepNote.Visibility = note != null ? Visibility.Visible : Visibility.Collapsed;
            RepNote.Foreground = (Brush)FindResource(problemNote != null ? "B.Dim" : "B.Faint");
        }

        // tests keep windows open unless they ask for the real auto close
        static bool AutoCloseAllowed { get { return !AppPaths.TestMode || Environment.GetEnvironmentVariable("CHOPPY_TEST_AUTOCLOSE") == "1"; } }

        void AddChip(string text, string brushKey)
        {
            var b = new Border { CornerRadius = new CornerRadius(11), Padding = new Thickness(10, 3, 10, 4), Margin = new Thickness(0, 0, 8, 4), Background = new SolidColorBrush(Color.FromRgb(0x22, 0x26, 0x2C)), BorderBrush = new SolidColorBrush(Color.FromRgb(0x33, 0x39, 0x42)), BorderThickness = new Thickness(1) };
            b.Child = new TextBlock { Text = text, FontSize = 12.5, Foreground = (Brush)FindResource(brushKey) };
            RepRows.Children.Add(b);
        }

        // ================= quick toggles (the same settings as the Settings window) =================

        /// <summary>Shows the settings as they are now (they can also change in the Settings window).</summary>
        void SyncQuick()
        {
            loadingQuick = true;
            try
            {
                var s = Settings.Current;
                if (TgClose.IsChecked != s.CloseWhenDone) TgClose.IsChecked = s.CloseWhenDone;
                bool move = job != null && job.Spec.Op == OpKind.Move;
                bool verify = move || (job != null && !job.IsFinished ? job.Verify || (job.Phase == JobPhase.Queued && job.Spec.Verify) : s.VerifyCopies);
                if (TgVerify.IsChecked != verify) TgVerify.IsChecked = verify;
                TgVerify.IsEnabled = !move;
                TgVerify.ToolTip = move ? "A move always checks each copy (xxHash64) before the original is removed."
                    : "Check every copied file against the original (xxHash64)";
                bool limitOn = s.SpeedLimitMbps > 0;
                if (TgLimit.IsChecked != limitOn) TgLimit.IsChecked = limitOn;
                bool mbps = UseMbps(job);
                LimitUnit.Text = mbps ? "Mbps" : "MB/s";
                if (!LimitBox.IsKeyboardFocusWithin)
                {
                    string v = Format.Limit(limitOn ? s.SpeedLimitMbps : s.LimitValueMbps, mbps);
                    if (LimitBox.Text != v) LimitBox.Text = v;
                }
                LimitBox.IsEnabled = limitOn;
            }
            finally { loadingQuick = false; }
        }

        void Quick_Changed(object sender, RoutedEventArgs e)
        {
            if (loadingQuick) return;
            var s = Settings.Current;
            if (sender == TgClose)
            {
                s.CloseWhenDone = TgClose.IsChecked == true;
                if (job != null && job.IsFinished)
                {
                    bool clean = job.Phase == JobPhase.Done && job.FailedFiles == 0 && job.SkippedFiles == 0;
                    closeAt = s.CloseWhenDone && clean && !Manager.HasWaiting && AutoCloseAllowed ? DateTime.Now.AddSeconds(4) : (DateTime?)null;
                }
            }
            else if (sender == TgVerify)
            {
                bool on = TgVerify.IsChecked == true;
                s.VerifyCopies = on;
                if (job != null && !job.IsFinished && job.Spec.Op == OpKind.Copy) { job.Verify = on; job.Spec.Verify = on; }
            }
            else if (sender == TgLimit)
            {
                bool on = TgLimit.IsChecked == true;
                s.SpeedLimitMbps = on ? Math.Max(0.1, s.LimitValueMbps) : 0;
                LimitBox.IsEnabled = on;
                if (on) { LimitBox.Focus(); LimitBox.SelectAll(); }
            }
            s.Save();
            Manager.ApplyLiveSettings();
            var sw = App.Current2.SettingsWin;
            if (sw != null) sw.Reload();
            Tick();
        }

        void LimitBox_Changed(object sender, TextChangedEventArgs e)
        {
            if (loadingQuick || !LimitBox.IsKeyboardFocusWithin) return;
            double v;
            if (!double.TryParse(LimitBox.Text, NumberStyles.Float, CultureInfo.CurrentCulture, out v) || v <= 0) return;
            var s = Settings.Current;
            s.LimitValueMbps = Format.LimitToMbps(v, UseMbps(job));
            if (s.SpeedLimitMbps > 0) s.SpeedLimitMbps = s.LimitValueMbps;
            s.Save();
            Manager.ApplyLiveSettings();
            var sw = App.Current2.SettingsWin;
            if (sw != null) sw.Reload();
        }

        // ================= queue =================

        void RefreshQueue()
        {
            int n = Manager.WaitingCount;
            QueueCount.Text = n.ToString();
            if (!QueuePopup.IsOpen) return;
            QueueList.Children.Clear();
            foreach (var spec in Manager.WaitingSnapshot())
            {
                var s = spec;
                var g = new Grid { Margin = new Thickness(8, 4, 4, 4) };
                g.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
                g.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
                var sp = new StackPanel();
                sp.Children.Add(new TextBlock { Text = s.Title, FontSize = 13.5, Foreground = (Brush)FindResource("B.Text2"), TextTrimming = TextTrimming.CharacterEllipsis });
                sp.Children.Add(new TextBlock { Text = "to " + Paths.ToShort(s.Dest), FontSize = 12, Foreground = (Brush)FindResource("B.Dim"), TextTrimming = TextTrimming.CharacterEllipsis });
                var x = new Button { Style = (Style)FindResource("Btn.Link"), Content = new TextBlock { Text = "\uE711", FontFamily = (FontFamily)FindResource("F.Icons"), FontSize = 11, Foreground = (Brush)FindResource("B.Dim") }, ToolTip = "Remove from the queue", VerticalAlignment = VerticalAlignment.Center };
                x.Click += (o, e) => { Manager.Remove(s); };
                Grid.SetColumn(x, 1);
                g.Children.Add(sp); g.Children.Add(x);
                QueueList.Children.Add(g);
            }
            foreach (var fj in Manager.FinishedSnapshot())
            {
                var f = fj;
                bool bad = HadProblems(f);
                string st = f.Phase == JobPhase.Canceled ? "canceled" : f.Phase == JobPhase.Failed ? "stopped"
                    : f.FailedFiles > 0 ? f.FailedFiles + " failed" : f.SkippedFiles > 0 ? f.SkippedFiles + " skipped" : "done";
                var row = new Button { Style = (Style)FindResource("Btn.Link"), HorizontalAlignment = HorizontalAlignment.Stretch, HorizontalContentAlignment = HorizontalAlignment.Left, Margin = new Thickness(2, 1, 0, 1), ToolTip = "Show this copy's report" };
                row.Content = new TextBlock
                {
                    FontSize = 12.5, TextTrimming = TextTrimming.CharacterEllipsis,
                    Foreground = (Brush)FindResource(bad ? "B.Warn" : "B.Faint"),
                    Text = (bad ? "! " : "✓ ") + f.Spec.Title + " - " + st
                };
                row.Click += (o, e) => { QueuePopup.IsOpen = false; dismissed.Remove(f); ShowJob(f); Tick(); UpdateCloseButton(); };
                QueueList.Children.Add(row);
            }
            if (QueueList.Children.Count == 0)
                QueueList.Children.Add(new TextBlock { Text = "Nothing waiting. A new copy waits here while one is running.", FontSize = 12.5, Margin = new Thickness(8, 4, 4, 4), Foreground = (Brush)FindResource("B.Faint"), TextWrapping = TextWrapping.Wrap });
        }

        void Queue_Click(object sender, RoutedEventArgs e)
        {
            QueuePopup.IsOpen = !QueuePopup.IsOpen;
            QueueChevron.Text = QueuePopup.IsOpen ? "\uE70E" : "\uE70D";
            RefreshQueue();
        }

        // ================= buttons =================

        void Pause_Click(object sender, RoutedEventArgs e)
        {
            if (job == null || job.IsFinished) return;
            if (job.IsPaused) job.Resume(); else job.Pause();
            Tick();
        }

        void Skip_Click(object sender, RoutedEventArgs e)
        {
            if (job != null && !job.IsFinished) job.SkipCurrent();
        }

        void Cancel_Click(object sender, RoutedEventArgs e)
        {
            if (job == null || job.IsFinished) { LeaveReport(); return; }
            if (!Dialogs.Ask(this, "Cancel this copy?", "Files that are already copied stay where they are. Nothing half-written is left behind.", "Cancel copy", "Keep going")) return;
            if (job != null && !job.IsFinished) job.Cancel();
        }

        void Unit_Click(object sender, RoutedEventArgs e)
        {
            var s = Settings.Current;
            bool nowMbps = UseMbps(job);
            s.SpeedUnit = nowMbps ? "mbs" : "mbps";
            s.Save();
            if (job != null && job.IsFinished) reportShown = false;
            var sw = App.Current2.SettingsWin;
            if (sw != null) sw.Reload();
            Tick();
        }

        void Open_Click(object sender, RoutedEventArgs e)
        {
            if (job == null) return;
            try
            {
                string target = Paths.ToShort(job.Spec.Dest);
                if (job.Spec.Sources.Count == 1 && job.Spec.ExplicitItems == null)
                {
                    string top = Paths.Combine(job.Spec.Dest, Paths.FileName(job.Spec.Sources[0]));
                    var only = job.Scan != null && job.Scan.Files.Count == 1 ? job.Scan.Files[0] : null;
                    if (only != null && only.FinalPath != null) top = only.FinalPath;
                    if (Paths.Exists(top)) { Process.Start("explorer.exe", "/select,\"" + Paths.ToShort(top) + "\""); return; }
                }
                Process.Start("explorer.exe", "\"" + target + "\"");
            }
            catch { }
        }

        void Retry_Click(object sender, RoutedEventArgs e)
        {
            if (job != null && job.FailedItems.Count > 0) Manager.RetryFailed(job);
        }

        void Log_Click(object sender, RoutedEventArgs e)
        {
            if (job != null && job.LogPath != null) try { Process.Start("notepad.exe", "\"" + job.LogPath + "\""); } catch { }
        }

        void Settings_Click(object sender, RoutedEventArgs e) { App.Current2.ShowSettings(this); }

        void Close_Click(object sender, RoutedEventArgs e)
        {
            if (job != null && !job.IsFinished)
            {
                if (App.Current2.Elevated) { WindowState = WindowState.Minimized; return; }   // no tray for the administrator copy
                // the copy keeps going; the tray icon brings the window back
                App.Current2.EnsureTray();
                Hide();
                App.Current2.TrayBalloon("Choppy keeps copying here. Click the icon to see it.");
                return;
            }
            Close();
        }

        // ================= test hook =================

        internal void PressForTest(string name)
        {
            switch (name)
            {
                case "pause": Pause_Click(null, null); break;
                case "skip": Skip_Click(null, null); break;
                case "cancel": Cancel_Click(null, null); break;
                case "cancel!": if (job != null && !job.IsFinished) job.Cancel(); break;   // as if "Cancel copy" was confirmed
                case "verify": TgVerify.IsChecked = TgVerify.IsChecked != true; break;
                case "unit": Unit_Click(null, null); break;
                case "queue": Queue_Click(null, null); break;
                case "retry": Retry_Click(null, null); break;
                case "view": View_Click(null, null); break;
                case "autoclose": TgClose.IsChecked = TgClose.IsChecked != true; break;
                case "limit": TgLimit.IsChecked = TgLimit.IsChecked != true; break;
                default:
                    if (name.StartsWith("limit "))
                    {
                        LimitBox.Focus();
                        LimitBox.Text = name.Substring(6);
                        Keyboard.ClearFocus();
                    }
                    break;
            }
            Tick();
        }

        internal string StateForTest()
        {
            return "{\"phase\":\"" + (job != null ? job.Phase.ToString() : "none") + "\",\"percent\":\"" + PercentExt.Text + "\",\"badge\":\"" + Esc(BadgeMode.Text + " " + BadgeDetail.Text)
                + "\",\"from\":\"" + Esc(FromName.Text + " " + FromPath.ToolTip) + "\",\"to\":\"" + Esc(ToName.Text + " " + ToPath.ToolTip)
                + "\",\"line1\":\"" + Esc(CNow.Text + CAvgLabel.Text + CAvg.Text) + "\",\"line2\":\"" + Esc(C2Label.Text + C2Name.Text)
                + "\",\"stats\":\"" + Esc(SizeDone.Text + SizeOf.Text + SizeTotal.Text + " | " + FilesText.Text) + "\",\"elapsed\":\"" + Esc(ElapsedVal.Text) + "\",\"remaining\":\"" + Esc(RemainVal.Text)
                + "\",\"speed\":\"" + Esc(SpNum.Text + SpUnit.Text) + "\",\"avg\":\"" + Esc(SpAvgNum.Text + SpAvgUnit.Text)
                + "\",\"current\":\"" + Esc(CurName.Text + " " + CurDone.Text + CurOf.Text + CurTotal.Text)
                + "\",\"report\":\"" + Esc(RepTitle.Text) + "\",\"queue\":\"" + Esc(QueueCount.Text) + "\",\"extended\":" + (Extended ? "true" : "false")
                + ",\"autoclose\":" + (TgClose.IsChecked == true ? "true" : "false") + ",\"verify\":" + (TgVerify.IsChecked == true ? "true" : "false")
                + ",\"limit\":\"" + (TgLimit.IsChecked == true ? Esc(LimitBox.Text + " " + LimitUnit.Text) : "off") + "\""
                + ",\"visible\":" + (IsVisible ? "true" : "false")
                + ",\"graphUpdates\":" + Graph.Updates + ",\"samples\":" + (job != null ? job.SpeedSamples : 0) + "}";
        }

        static string Esc(object o) { return (o as string ?? (o != null ? o.ToString() : "")).Replace("\\", "\\\\").Replace("\"", "\\\""); }
    }
}
