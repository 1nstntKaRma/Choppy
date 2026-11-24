using System;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Windows;
using System.Windows.Media.Imaging;
using Choppy.Engine;

namespace Choppy.UI
{
    public partial class SettingsWindow : Window
    {
        bool loading = true;

        public SettingsWindow()
        {
            InitializeComponent();
            try { AppIcon.Source = ChoppyIcon.Frame(20); Icon = BitmapFrame.Create(new Uri(ChoppyIcon.Uri)); } catch { }
            // the administrator copy never touches Explorer, the Run key or Ultracopier
            if (App.Current2 != null && App.Current2.Elevated) TakeoverRow.Visibility = Visibility.Collapsed;
            Version.Text = "v" + typeof(SettingsWindow).Assembly.GetName().Version.ToString(3);
            WholeNumberBox.Attach(LimitValue);
            WholeNumberBox.Attach(CloseValue);
            BuildUnitChart();
            Reload();
            Loaded += (o, e) => Native2.Rounded(this);
            MouseLeftButtonDown += (o, e) => { if (e.GetPosition(this).Y < 46) try { DragMove(); } catch { } };
            KeyDown += (o, e) => { if (e.Key == System.Windows.Input.Key.Escape) Close(); };
        }

        /// <summary>Unit for the speed limit: what "Show speed in" says (Auto = MB/s here; a network copy shows Mbps).</summary>
        static bool LimitInMbps { get { return Settings.Current.SpeedUnit == "mbps"; } }

        /// <summary>Shows the settings as they are now (the copy window's quick toggles change them too).</summary>
        internal void Reload()
        {
            loading = true;
            var s = Settings.Current;
            Verify.IsChecked = s.VerifyCopies;
            switch (s.Policy)
            {
                case ConflictPolicy.Replace: PReplace.IsChecked = true; break;
                case ConflictPolicy.ReplaceOlder: PNewer.IsChecked = true; break;
                case ConflictPolicy.Skip: PSkip.IsChecked = true; break;
                case ConflictPolicy.KeepBoth: PBoth.IsChecked = true; break;
                default: PAsk.IsChecked = true; break;
            }
            Limit.IsChecked = s.SpeedLimitMbps > 0;
            if (!LimitValue.IsKeyboardFocusWithin) LimitValue.Text = Format.Limit(s.SpeedLimitMbps > 0 ? s.SpeedLimitMbps : s.LimitValueMbps, LimitInMbps);
            LimitUnit.Text = LimitInMbps ? "Mbps" : "MB/s";
            LimitValue.IsEnabled = Limit.IsChecked == true;
            AutoClose.IsChecked = s.CloseWhenDone;
            if (!CloseValue.IsKeyboardFocusWithin) CloseValue.Text = s.CloseAfterSeconds.ToString(System.Globalization.CultureInfo.InvariantCulture);
            CloseValue.IsEnabled = s.CloseWhenDone;
            if (s.SpeedUnit == "mbps") UMbps.IsChecked = true; else if (s.SpeedUnit == "mbs") UMbs.IsChecked = true; else UAuto.IsChecked = true;
            Takeover.IsChecked = s.ExplorerTakeover;
            UcNote.Visibility = Explorer.Takeover.UltracopierInstalled() ? Visibility.Visible : Visibility.Collapsed;
            loading = false;
        }

        void Changed(object sender, RoutedEventArgs e)
        {
            if (loading) return;
            var s = Settings.Current;
            s.VerifyCopies = Verify.IsChecked == true;
            s.Policy = PReplace.IsChecked == true ? ConflictPolicy.Replace : PNewer.IsChecked == true ? ConflictPolicy.ReplaceOlder
                : PSkip.IsChecked == true ? ConflictPolicy.Skip : PBoth.IsChecked == true ? ConflictPolicy.KeepBoth : ConflictPolicy.Ask;
            s.CloseWhenDone = AutoClose.IsChecked == true;
            CloseValue.IsEnabled = s.CloseWhenDone;
            int secs;   // whole seconds only; 0 closes the window right away
            if (int.TryParse(CloseValue.Text, NumberStyles.None, CultureInfo.InvariantCulture, out secs) && secs <= 999) s.CloseAfterSeconds = secs;
            string unit = UMbps.IsChecked == true ? "mbps" : UMbs.IsChecked == true ? "mbs" : "auto";
            if (unit != s.SpeedUnit)
            {
                s.SpeedUnit = unit;
                loading = true;
                LimitValue.Text = Format.Limit(s.SpeedLimitMbps > 0 ? s.SpeedLimitMbps : s.LimitValueMbps, LimitInMbps);
                LimitUnit.Text = LimitInMbps ? "Mbps" : "MB/s";
                loading = false;
            }
            LimitValue.IsEnabled = Limit.IsChecked == true;
            double v = ParseLimit();
            if (v > 0) s.LimitValueMbps = v;
            s.SpeedLimitMbps = Limit.IsChecked == true ? s.LimitValueMbps : 0;
            s.Save();
            App.Current2.Manager.ApplyLiveSettings();
        }

        /// <summary>The typed limit in Mbps, or 0 when it isn't a number.</summary>
        double ParseLimit()
        {
            int v = Format.ParseLimit(LimitValue.Text);
            return v > 0 ? Math.Min(100000, Format.LimitToMbps(v, LimitInMbps)) : 0;
        }

        void LimitValue_Changed(object sender, System.Windows.Controls.TextChangedEventArgs e)
        {
            if (!loading) Changed(sender, null);
        }

        void CloseValue_Changed(object sender, System.Windows.Controls.TextChangedEventArgs e)
        {
            if (!loading) Changed(sender, null);
        }

        void UnitHelp_Click(object sender, RoutedEventArgs e) { UnitHelp.IsOpen = !UnitHelp.IsOpen; }

        /// <summary>
        /// A line of the chart: Mbps | bar | MB/s. The two number columns share one width (the widest number of either),
        /// so the bars sit in the middle of the card and both sides mirror each other.
        /// </summary>
        static System.Windows.Controls.Grid ChartRow(double bottom)
        {
            var g = new System.Windows.Controls.Grid { Margin = new Thickness(0, 0, 0, bottom) };
            g.ColumnDefinitions.Add(new System.Windows.Controls.ColumnDefinition { Width = GridLength.Auto, SharedSizeGroup = "Side" });
            g.ColumnDefinitions.Add(new System.Windows.Controls.ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            g.ColumnDefinitions.Add(new System.Windows.Controls.ColumnDefinition { Width = GridLength.Auto, SharedSizeGroup = "Side" });
            return g;
        }

        const double BarGap = 16;   // between the numbers and the bars (and the arrow, which spans the bars exactly)

        /// <summary>
        /// The "?" card: 10 / 100 / 1,000 / 10,000 Mbps in MB/s, bars on a log scale (each step ten times the last). The
        /// header sits on the same columns: "Mbps" above the Mbps values, an arrow as long as the bars, "MB/s" above the
        /// MB/s values - each unit right above the units under it.
        /// </summary>
        void BuildUnitChart()
        {
            int[] mbps = { 10, 100, 1000, 10000 };
            string[] what = { "Slow Upload", "Home Internet", "Gigabit Network", "10-Gigabit Network" };
            var app = Application.Current;
            System.Func<string, System.Windows.Media.Brush> res = k => (System.Windows.Media.Brush)app.FindResource(k);
            var inv = System.Globalization.CultureInfo.InvariantCulture;
            System.Windows.Controls.Grid.SetIsSharedSizeScope(UnitRows, true);
            var head = ChartRow(2);
            head.Children.Add(new System.Windows.Controls.TextBlock { Text = "Mbps", FontSize = 15, FontWeight = FontWeights.SemiBold, Foreground = res("B.AccentText"), HorizontalAlignment = HorizontalAlignment.Right });
            var arrow = new System.Windows.Controls.Grid { Margin = new Thickness(BarGap, 1, BarGap, 0), VerticalAlignment = VerticalAlignment.Center };
            arrow.Children.Add(new System.Windows.Controls.Border { Height = 2, CornerRadius = new CornerRadius(1), Background = res("B.Faint"), Margin = new Thickness(0, 0, 6, 0), VerticalAlignment = VerticalAlignment.Center });
            arrow.Children.Add(new System.Windows.Shapes.Path { Data = System.Windows.Media.Geometry.Parse("M0,0 L9,5 L0,10 Z"), Fill = res("B.Faint"), HorizontalAlignment = HorizontalAlignment.Right, VerticalAlignment = VerticalAlignment.Center });
            System.Windows.Controls.Grid.SetColumn(arrow, 1);
            head.Children.Add(arrow);
            var mbsHead = new System.Windows.Controls.TextBlock { Text = "MB/s", FontSize = 15, FontWeight = FontWeights.SemiBold, Foreground = res("B.AvgSpeed"), HorizontalAlignment = HorizontalAlignment.Right };
            System.Windows.Controls.Grid.SetColumn(mbsHead, 2);
            head.Children.Add(mbsHead);
            UnitRows.Children.Add(head);
            UnitRows.Children.Add(new System.Windows.Controls.TextBlock { Text = "8 bits make a byte: divide Mbps by about 8.4 for MB/s", FontSize = 12.5, Foreground = res("B.Dim"), HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(0, 4, 0, 14) });
            for (int i = 0; i < mbps.Length; i++)
            {
                double mbs = mbps[i] * 1e6 / 8 / (1024 * 1024);
                var g = ChartRow(i < mbps.Length - 1 ? 10 : 0);
                // right-aligned on both sides: every "Mbps" under the header's "Mbps", every "MB/s" under "MB/s"
                var left = new System.Windows.Controls.TextBlock { FontSize = 13.5, VerticalAlignment = VerticalAlignment.Top, HorizontalAlignment = HorizontalAlignment.Right };
                left.Inlines.Add(new System.Windows.Documents.Run(mbps[i].ToString("N0", inv)) { FontWeight = FontWeights.SemiBold, Foreground = res("B.AccentText") });
                left.Inlines.Add(new System.Windows.Documents.Run(" Mbps") { Foreground = res("B.Dim") });
                g.Children.Add(left);
                // the bar on the numbers' line, what it stands for under it
                var track = new System.Windows.Controls.Grid { Height = 8, VerticalAlignment = VerticalAlignment.Center };
                track.Children.Add(new System.Windows.Controls.Border { CornerRadius = new CornerRadius(4), Background = res("B.Track") });
                var bar = new System.Windows.Controls.Border { CornerRadius = new CornerRadius(4), Background = res("B.Progress"), HorizontalAlignment = HorizontalAlignment.Left };
                int k = i;
                track.SizeChanged += (o, e) => bar.Width = e.NewSize.Width * (k + 1) / mbps.Length;
                track.Children.Add(bar);
                var line = new System.Windows.Controls.Grid { Height = 18 };   // the numbers' line height: the bar is centered on it
                line.Children.Add(track);
                var mid = new System.Windows.Controls.StackPanel { Margin = new Thickness(BarGap, 0, BarGap, 0) };
                mid.Children.Add(line);
                mid.Children.Add(new System.Windows.Controls.TextBlock { Text = what[i], FontSize = 11, Foreground = res("B.Faint"), Margin = new Thickness(0, 1, 0, 0) });
                System.Windows.Controls.Grid.SetColumn(mid, 1);
                g.Children.Add(mid);
                var right = new System.Windows.Controls.TextBlock { FontSize = 13.5, VerticalAlignment = VerticalAlignment.Top, HorizontalAlignment = HorizontalAlignment.Right };
                right.Inlines.Add(new System.Windows.Documents.Run(mbs >= 100 ? mbs.ToString("N0", inv) : mbs.ToString(mbs >= 10 ? "0.0" : "0.00", inv)) { FontWeight = FontWeights.SemiBold, Foreground = res("B.AvgSpeed") });
                right.Inlines.Add(new System.Windows.Documents.Run(" MB/s") { Foreground = res("B.Dim") });
                System.Windows.Controls.Grid.SetColumn(right, 2);
                g.Children.Add(right);
                UnitRows.Children.Add(g);
            }
        }

        /// <summary>The "?" card itself (tests render it).</summary>
        internal FrameworkElement UnitChartForTest { get { return UnitHelpCard; } }

        void Takeover_Changed(object sender, RoutedEventArgs e)
        {
            if (loading) return;
            bool on = Takeover.IsChecked == true;
            string err = Explorer.Takeover.Set(on);
            if (err != null)
            {
                Dialogs.Show(this, "Couldn't change the Explorer setting", err, "OK", null);
                loading = true; Takeover.IsChecked = Settings.Current.ExplorerTakeover; loading = false;
            }
        }

        void Logs_Click(object sender, RoutedEventArgs e)
        {
            try { Directory.CreateDirectory(AppPaths.LogDir); Process.Start("explorer.exe", "\"" + AppPaths.LogDir + "\""); } catch { }
        }

        void Close_Click(object sender, RoutedEventArgs e) { Close(); }

        void Benchmark_Click(object sender, RoutedEventArgs e) { App.Current2.ShowBenchmark(); }

        void About_Click(object sender, RoutedEventArgs e) { Dialogs.About(this); }

        internal void SetForTest(string key, string value)
        {
            switch (key)
            {
                case "verify": Verify.IsChecked = value == "1"; break;
                case "policy":
                    if (value == "replace") PReplace.IsChecked = true; else if (value == "skip") PSkip.IsChecked = true;
                    else if (value == "keepboth") PBoth.IsChecked = true; else if (value == "newer") PNewer.IsChecked = true; else PAsk.IsChecked = true;
                    break;
                case "limit": if (value == "0") Limit.IsChecked = false; else { LimitValue.Text = value; Limit.IsChecked = true; } break;
                case "autoclose": AutoClose.IsChecked = value == "1"; break;
                case "closeafter": CloseValue.Text = value; break;
                case "unithelp": UnitHelp.IsOpen = value == "1"; break;
                case "unit": if (value == "mbps") UMbps.IsChecked = true; else if (value == "mbs") UMbs.IsChecked = true; else UAuto.IsChecked = true; break;
            }
        }
    }
}
