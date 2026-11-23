using System;
using System.IO;
using System.Threading;
using System.Windows;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using Choppy.Engine;

namespace Choppy.UI
{
    public partial class ConflictWindow : Window
    {
        readonly ConflictInfo info;
        internal ConflictAnswer Answer;
        volatile bool closed;

        internal ConflictWindow(ConflictInfo info)
        {
            InitializeComponent();
            this.info = info;
            try { AppIcon.Source = ChoppyIcon.Frame(18); Icon = BitmapFrame.Create(new Uri(ChoppyIcon.Uri)); } catch { }
            FileName.Text = Path.GetFileName(info.ExistingPath);
            FileName.ToolTip = info.ExistingPath;
            InSize.Text = Format.Size(info.IncomingSize);
            ExSize.Text = Format.Size(info.ExistingSize);
            InDate.Text = "Modified " + DateText(info.IncomingTime);
            ExDate.Text = "Modified " + DateText(info.ExistingTime);
            InCard.ToolTip = info.IncomingPath;
            string ext = Path.GetExtension(info.ExistingPath).TrimStart('.').ToUpperInvariant();
            InExt.Text = ExExt.Text = ext.Length > 0 && ext.Length <= 5 ? ext : "FILE";

            // little hints: which one is newer / bigger
            if (info.IncomingTime > info.ExistingTime.AddSeconds(2)) ShowTag(InTag, InTagText, "Newer");
            else if (info.ExistingTime > info.IncomingTime.AddSeconds(2)) ShowTag(ExTag, ExTagText, "Newer");
            else if (info.IncomingSize == info.ExistingSize) { ShowTag(InTag, InTagText, "Same size + date"); }

            ExCard.ToolTip = info.ExistingPath;
            if (info.Remaining > 1) ApplyAllText.Text = "Apply to all " + info.Remaining + " remaining conflicts";
            else ApplyAll.Visibility = Visibility.Collapsed;
            Pick(false);

            Loaded += (s, e) =>
            {
                Native2.Rounded(this);
                LoadThumbs();
                BtnReplace.Focus();
            };
            Closed += (s, e) => { closed = true; if (Answer == null) Answer = new ConflictAnswer { Choice = ConflictPolicy.Skip }; };
            MouseLeftButtonDown += (s, e) => { if (e.GetPosition(this).Y < 40) try { DragMove(); } catch { } };
            KeyDown += (s, e) => { if (e.Key == Key.Escape) Skip_Click(null, null); };
            // left / right: the other card
            PreviewKeyDown += (s, e) =>
            {
                if (e.Key == Key.Left && keepExisting) { Pick(false); e.Handled = true; }
                else if (e.Key == Key.Right && !keepExisting) { Pick(true); e.Handled = true; }
            };
        }

        bool keepExisting;   // the card picked: the file already there stays (Skip) instead of the incoming one (Replace)

        /// <summary>
        /// Picks the file that stays: its card lights up and the main button says what that does - Replace (incoming) or
        /// Skip (already there). The other of the two moves to the button beside Compare.
        /// </summary>
        void Pick(bool existing)
        {
            keepExisting = existing;
            Look(InCard, InHead, InCheck, InBox, InImg, InExt, !existing);
            Look(ExCard, ExHead, ExCheck, ExBox, ExImg, ExExt, existing);
            BtnReplace.Content = existing ? "Skip" : "Replace";
            BtnSkip.Content = existing ? "Replace" : "Skip";
            string skipTip = info.Op == OpKind.Move ? "Leave both where they are (the original is not moved)" : "Keep the file that is already there";
            const string replaceTip = "Put the incoming file in place of the one already there";
            BtnReplace.ToolTip = existing ? skipTip : replaceTip;
            BtnSkip.ToolTip = existing ? replaceTip : skipTip;
            if (IsLoaded && !BtnSkip.IsKeyboardFocused && !BtnBoth.IsKeyboardFocused && !BtnCompare.IsKeyboardFocused) BtnReplace.Focus();
        }

        static readonly Brush PickBorder = Frozen(0xFF2D6FB4), PickBack = Frozen(0xFF172230), PickHead = Frozen(0xFF4FA8F0), PickBox = Frozen(0xFF1E2833), PickExt = Frozen(0xFF4C5A6B);
        static readonly Brush OffBorder = Frozen(0xFF2A2F37), OffBack = Frozen(0xFF191C21), OffBox = Frozen(0xFF22272E), OffExt = Frozen(0xFF4C535C), HoverBorder = Frozen(0xFF3E4650);

        static Brush Frozen(uint argb)
        {
            var b = new SolidColorBrush(Color.FromArgb((byte)(argb >> 24), (byte)(argb >> 16), (byte)(argb >> 8), (byte)argb));
            b.Freeze();
            return b;
        }

        /// <summary>A card picked (blue, its picture bright, a check mark) or not (grey, its picture dimmed).</summary>
        void Look(System.Windows.Controls.Border card, System.Windows.Controls.TextBlock head, UIElement check, System.Windows.Controls.Border box,
            UIElement img, System.Windows.Controls.TextBlock ext, bool picked)
        {
            card.BorderBrush = picked ? PickBorder : card.IsMouseOver ? HoverBorder : OffBorder;
            card.Background = picked ? PickBack : OffBack;
            head.Foreground = picked ? PickHead : (Brush)FindResource("B.Label");
            check.Visibility = picked ? Visibility.Visible : Visibility.Hidden;
            box.Background = picked ? PickBox : OffBox;
            ext.Foreground = picked ? PickExt : OffExt;
            img.Opacity = picked ? 1 : 0.55;
        }

        void InCard_Click(object sender, MouseButtonEventArgs e) { Pick(false); }
        void ExCard_Click(object sender, MouseButtonEventArgs e) { Pick(true); }

        void Card_Hover(object sender, MouseEventArgs e)
        {
            var card = (System.Windows.Controls.Border)sender;
            bool picked = card == ExCard ? keepExisting : !keepExisting;
            if (!picked) card.BorderBrush = card.IsMouseOver ? HoverBorder : OffBorder;
        }

        static void ShowTag(FrameworkElement b, System.Windows.Controls.TextBlock t, string text)
        {
            t.Text = text;
            b.Visibility = Visibility.Visible;
        }

        static string DateText(DateTime d) { return Format.Date(d); }

        void LoadThumbs()
        {
            string a = info.IncomingPath, b = info.ExistingPath;
            var t = new Thread(() =>
            {
                var ia = ShellThumbs.Get(a, 320);
                if (!closed) Dispatcher.BeginInvoke(new Action(() => { if (ia != null) { InImg.Source = ia; InExt.Visibility = Visibility.Collapsed; } }));
                var ib = ShellThumbs.Get(b, 320);
                if (!closed) Dispatcher.BeginInvoke(new Action(() => { if (ib != null) { ExImg.Source = ib; ExExt.Visibility = Visibility.Collapsed; } }));
            }) { IsBackground = true, Name = "thumbs" };
            t.SetApartmentState(ApartmentState.STA);
            t.Start();
        }

        void Done(ConflictPolicy p)
        {
            Answer = new ConflictAnswer { Choice = p, ApplyToAll = ApplyAll.IsChecked == true };
            try { DialogResult = true; } catch { }
            Close();
        }

        void Skip_Click(object sender, RoutedEventArgs e) { Done(ConflictPolicy.Skip); }
        void Primary_Click(object sender, RoutedEventArgs e) { Done(keepExisting ? ConflictPolicy.Skip : ConflictPolicy.Replace); }
        void Second_Click(object sender, RoutedEventArgs e) { Done(keepExisting ? ConflictPolicy.Replace : ConflictPolicy.Skip); }
        void Both_Click(object sender, RoutedEventArgs e) { Done(ConflictPolicy.KeepBoth); }
        void Replace_Click(object sender, RoutedEventArgs e) { Done(ConflictPolicy.Replace); }

        void Compare_Click(object sender, RoutedEventArgs e)
        {
            CompareRow.Visibility = Visibility.Visible;
            BtnCompare.IsEnabled = false;
            if (info.IncomingSize != info.ExistingSize)
            {
                ShowCompare(false, "Different sizes - these are not the same file.");
                return;
            }
            CompareIcon.Text = ""; CompareIcon.Foreground = (Brush)FindResource("B.Dim");
            CompareText.Text = "Comparing the content…";
            string a = Paths.ToLong(info.IncomingPath), b = Paths.ToLong(info.ExistingPath);
            long total = Math.Max(1, info.IncomingSize * 2);
            var th = new Thread(() =>
            {
                ulong ha, hb = 0; int err;
                long half = 0;
                bool ok = XxHash64.HashFile(a, false, n => { Report(n, total); return !closed; }, out ha, out err);
                half = info.IncomingSize;
                ok = ok && XxHash64.HashFile(b, false, n => { Report(half + n, total); return !closed; }, out hb, out err);
                if (closed) return;
                Dispatcher.BeginInvoke(new Action(() =>
                {
                    if (!ok) ShowCompare(false, "Couldn't read one of the files: " + Native.ErrorText(err));
                    else if (ha == hb) ShowCompare(true, "Same content - Skip is safe.");
                    else ShowCompare(false, "Different content, same size.");
                }));
            }) { IsBackground = true };
            th.Start();
        }

        DateTime lastReport;
        void Report(long n, long total)
        {
            if ((DateTime.Now - lastReport).TotalMilliseconds < 150) return;
            lastReport = DateTime.Now;
            Dispatcher.BeginInvoke(new Action(() => { if (!closed) CompareText.Text = "Comparing the content… " + (int)(100.0 * n / total) + "%"; }));
        }

        void ShowCompare(bool same, string text)
        {
            CompareIcon.Text = same ? "" : "";
            CompareIcon.Foreground = (Brush)FindResource(same ? "B.Good" : "B.Warn");
            CompareText.Text = text;
            CompareText.Foreground = (Brush)FindResource(same ? "B.Good" : "B.Text2");
        }

        internal void PressForTest(string name, bool all)
        {
            ApplyAll.IsChecked = all;
            switch (name)
            {
                case "replace": Replace_Click(null, null); break;
                case "skip": Skip_Click(null, null); break;
                case "keepboth": Both_Click(null, null); break;
                case "compare": Compare_Click(null, null); break;
                case "pickexisting": Pick(true); break;
                case "pickincoming": Pick(false); break;
                case "primary": Primary_Click(null, null); break;
            }
        }
    }
}
