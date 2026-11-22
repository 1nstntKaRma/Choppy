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
            try { AppIcon.Source = BitmapFrame.Create(new Uri("pack://application:,,,/Choppy;component/Choppy.ico")); Icon = AppIcon.Source; } catch { }
            // the administrator copy never touches Explorer, the Run key or Ultracopier
            if (App.Current2 != null && App.Current2.Elevated) TakeoverRow.Visibility = Visibility.Collapsed;
            Version.Text = "v" + typeof(SettingsWindow).Assembly.GetName().Version.ToString(3);
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
            double v;
            if (double.TryParse(LimitValue.Text, NumberStyles.Float, CultureInfo.CurrentCulture, out v) && v > 0) return Math.Min(100000, Format.LimitToMbps(v, LimitInMbps));
            return 0;
        }

        void LimitValue_Changed(object sender, System.Windows.Controls.TextChangedEventArgs e)
        {
            if (!loading) Changed(sender, null);
        }

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
                case "unit": if (value == "mbps") UMbps.IsChecked = true; else if (value == "mbs") UMbs.IsChecked = true; else UAuto.IsChecked = true; break;
            }
        }
    }
}
