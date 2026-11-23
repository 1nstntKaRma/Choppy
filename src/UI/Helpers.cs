using System;
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using Choppy.Engine;

namespace Choppy.UI
{
    /// <summary>Window frame touches: dark frame + rounded corners (Windows 11), the same window on Windows 10.</summary>
    internal static class Native2
    {
        [DllImport("dwmapi.dll")]
        static extern int DwmSetWindowAttribute(IntPtr hwnd, int attr, ref int value, int size);

        public static void Rounded(Window w)
        {
            try
            {
                IntPtr h = new WindowInteropHelper(w).Handle;
                int on = 1;
                DwmSetWindowAttribute(h, 20, ref on, 4);          // DWMWA_USE_IMMERSIVE_DARK_MODE
                int round = 2;
                DwmSetWindowAttribute(h, 33, ref round, 4);       // DWMWA_WINDOW_CORNER_PREFERENCE = round
                int border = 0x00433A33;                          // DWMWA_BORDER_COLOR (0x00BBGGRR) = #333A43
                DwmSetWindowAttribute(h, 34, ref border, 4);
            }
            catch { }
        }
    }

    /// <summary>Explorer's own thumbnails (photos, videos, RAW with codecs) for the conflict dialog.</summary>
    internal static class ShellThumbs
    {
        [ComImport, Guid("bcc18b79-ba16-442f-80c4-8a59c30c463b"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
        interface IShellItemImageFactory
        {
            [PreserveSig] int GetImage(SIZE size, int flags, out IntPtr phbm);
        }

        [StructLayout(LayoutKind.Sequential)]
        struct SIZE { public int cx, cy; }

        [DllImport("shell32.dll", CharSet = CharSet.Unicode, PreserveSig = false)]
        static extern void SHCreateItemFromParsingName(string path, IntPtr pbc, [MarshalAs(UnmanagedType.LPStruct)] Guid riid, [MarshalAs(UnmanagedType.Interface)] out IShellItemImageFactory factory);

        [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr h);
        [DllImport("gdi32.dll")] static extern int GetObject(IntPtr h, int size, ref DIBSECTION ds);

        [StructLayout(LayoutKind.Sequential)]
        struct BITMAP { public int bmType, bmWidth, bmHeight, bmWidthBytes; public ushort bmPlanes, bmBitsPixel; public IntPtr bmBits; }
        [StructLayout(LayoutKind.Sequential)]
        struct BITMAPINFOHEADER { public int biSize, biWidth, biHeight; public short biPlanes, biBitCount; public int biCompression, biSizeImage, biXPelsPerMeter, biYPelsPerMeter, biClrUsed, biClrImportant; }
        [StructLayout(LayoutKind.Sequential)]
        struct DIBSECTION { public BITMAP dsBm; public BITMAPINFOHEADER dsBmih; public int b0, b1, b2; public IntPtr dshSection; public int dsOffset; }

        const int SIIGBF_BIGGERSIZEOK = 0x1, SIIGBF_THUMBNAILONLY = 0x8;

        /// <summary>A thumbnail, or null when the file type has none (then the card shows the extension).</summary>
        public static BitmapSource Get(string path, int size)
        {
            IntPtr hbm = IntPtr.Zero;
            try
            {
                IShellItemImageFactory f;
                SHCreateItemFromParsingName(Paths.ToShort(path), IntPtr.Zero, typeof(IShellItemImageFactory).GUID, out f);
                int hr = f.GetImage(new SIZE { cx = size, cy = size }, SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK, out hbm);
                Marshal.ReleaseComObject(f);
                if (hr != 0 || hbm == IntPtr.Zero) return null;
                var ds = new DIBSECTION();
                if (GetObject(hbm, Marshal.SizeOf(typeof(DIBSECTION)), ref ds) == 0 || ds.dsBm.bmBits == IntPtr.Zero || ds.dsBm.bmBitsPixel != 32)
                {
                    var b = Imaging.CreateBitmapSourceFromHBitmap(hbm, IntPtr.Zero, Int32Rect.Empty, BitmapSizeOptions.FromEmptyOptions());
                    b.Freeze();
                    return b;
                }
                int w = ds.dsBm.bmWidth, h = ds.dsBm.bmHeight, stride = ds.dsBm.bmWidthBytes;
                var bytes = new byte[stride * h];
                Marshal.Copy(ds.dsBm.bmBits, bytes, 0, bytes.Length);
                if (ds.dsBmih.biHeight > 0)
                {
                    // bottom-up DIB: flip rows
                    var flipped = new byte[bytes.Length];
                    for (int y = 0; y < h; y++) Buffer.BlockCopy(bytes, y * stride, flipped, (h - 1 - y) * stride, stride);
                    bytes = flipped;
                }
                // thumbnails without real alpha come back with alpha 0: treat as opaque
                bool anyAlpha = false;
                for (int i = 3; i < bytes.Length; i += 4) if (bytes[i] != 0) { anyAlpha = true; break; }
                if (!anyAlpha) for (int i = 3; i < bytes.Length; i += 4) bytes[i] = 255;
                var bs = BitmapSource.Create(w, h, 96, 96, anyAlpha ? PixelFormats.Pbgra32 : PixelFormats.Bgra32, null, bytes, stride);
                bs.Freeze();
                return bs;
            }
            catch { return null; }
            finally { if (hbm != IntPtr.Zero) DeleteObject(hbm); }
        }
    }

    /// <summary>The app icon at a given size: the frame of Choppy.ico that fits the screen's scaling (not the 16 px one stretched).</summary>
    internal static class ChoppyIcon
    {
        public const string Uri = "pack://application:,,,/Choppy;component/Choppy.ico";
        static BitmapDecoder decoder;
        static double scale;

        public static ImageSource Frame(double size)
        {
            if (decoder == null)
            {
                decoder = BitmapDecoder.Create(new System.Uri(Uri), BitmapCreateOptions.None, BitmapCacheOption.OnLoad);
                try { using (var g = System.Drawing.Graphics.FromHwnd(IntPtr.Zero)) scale = g.DpiX / 96.0; } catch { scale = 1; }
            }
            double px = size * Math.Max(1, scale);
            BitmapFrame best = null, largest = null;
            foreach (var f in decoder.Frames)
            {
                if (largest == null || f.PixelWidth > largest.PixelWidth) largest = f;
                if (f.PixelWidth >= px && (best == null || f.PixelWidth < best.PixelWidth)) best = f;
            }
            return best ?? largest;
        }

        /// <summary>The icon for the tray (the small icon size, its own frame).</summary>
        public static System.Drawing.Icon Small()
        {
            var r = Application.GetResourceStream(new System.Uri(Uri));
            using (var st = r.Stream) return new System.Drawing.Icon(st, System.Windows.Forms.SystemInformation.SmallIconSize);
        }
    }

    /// <summary>A box that takes whole numbers only: other keys and pastes are ignored.</summary>
    internal static class WholeNumberBox
    {
        public static void Attach(TextBox box)
        {
            box.PreviewTextInput += (s, e) => { foreach (char c in e.Text) if (c < '0' || c > '9') { e.Handled = true; return; } };
            DataObject.AddPastingHandler(box, (s, e) =>
            {
                var text = e.DataObject.GetDataPresent(typeof(string)) ? (string)e.DataObject.GetData(typeof(string)) : null;
                if (Format.ParseLimit(text) == 0) e.CancelCommand();
            });
            box.PreviewKeyDown += (s, e) => { if (e.Key == System.Windows.Input.Key.Space) e.Handled = true; };
        }
    }

    /// <summary>Dark question / message box in the Choppy style.</summary>
    internal static class Dialogs
    {
        public const string Repo = "https://github.com/1nstntKaRma/Choppy";

        /// <summary>About: name, version, author, the project page and the MIT license.</summary>
        public static void About(Window owner)
        {
            var app = Application.Current;
            var w = new Window
            {
                Title = "About Choppy",
                Width = 520,
                SizeToContent = SizeToContent.Height,
                WindowStyle = WindowStyle.None,
                ResizeMode = ResizeMode.NoResize,
                ShowInTaskbar = owner == null || !owner.IsVisible,
                Background = (Brush)app.FindResource("B.Window"),
                BorderBrush = (Brush)app.FindResource("B.WinBorder"),
                BorderThickness = new Thickness(1),
                FontFamily = (FontFamily)app.FindResource("F.Ui"),
                Foreground = (Brush)app.FindResource("B.Text"),
                WindowStartupLocation = owner != null && owner.IsVisible ? WindowStartupLocation.CenterOwner : WindowStartupLocation.CenterScreen
            };
            System.Windows.Shell.WindowChrome.SetWindowChrome(w, new System.Windows.Shell.WindowChrome { CaptionHeight = 30, ResizeBorderThickness = new Thickness(0), GlassFrameThickness = new Thickness(0, 0, 0, 1), UseAeroCaptionButtons = false });
            if (owner != null && owner.IsVisible) w.Owner = owner;
            var sp = new StackPanel { Margin = new Thickness(24, 22, 24, 20) };

            var head = new StackPanel { Orientation = Orientation.Horizontal };
            try { head.Children.Add(new Image { Source = ChoppyIcon.Frame(72), Width = 72, Height = 72, Margin = new Thickness(0, 0, 18, 0) }); } catch { }
            var names = new StackPanel { VerticalAlignment = VerticalAlignment.Center };
            names.Children.Add(new TextBlock { Text = "Choppy", FontSize = 22, FontWeight = FontWeights.SemiBold });
            names.Children.Add(new TextBlock { Text = "v" + typeof(Dialogs).Assembly.GetName().Version.ToString(3) + " · fast, clear file copier for Windows", FontSize = 13.5, Foreground = (Brush)app.FindResource("B.Dim") });
            head.Children.Add(names);
            sp.Children.Add(head);

            var by = new TextBlock { FontSize = 14, Margin = new Thickness(0, 18, 0, 0), Foreground = (Brush)app.FindResource("B.Text2") };
            by.Inlines.Add(new System.Windows.Documents.Run("by "));
            by.Inlines.Add(new System.Windows.Documents.Run("1nstntKaRma") { FontWeight = FontWeights.SemiBold, Foreground = (Brush)app.FindResource("B.Text") });
            sp.Children.Add(by);
            var link = new Button { Style = (Style)app.FindResource("Btn.Link"), Margin = new Thickness(-6, 4, 0, 0), HorizontalAlignment = HorizontalAlignment.Left, ToolTip = Repo };
            link.Content = new TextBlock { Text = Repo.Replace("https://", ""), FontSize = 14, Foreground = (Brush)app.FindResource("B.Link") };
            link.Click += (s, e) => { try { System.Diagnostics.Process.Start(Repo); } catch { } };
            sp.Children.Add(link);

            sp.Children.Add(new TextBlock { Text = "MIT LICENSE", Style = (Style)app.FindResource("T.Cap"), Margin = new Thickness(0, 18, 0, 6) });
            var lic = new TextBox
            {
                Text = License(), IsReadOnly = true, TextWrapping = TextWrapping.Wrap, FontSize = 12, Height = 170,
                VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Background = (Brush)app.FindResource("B.Card"),
                Foreground = (Brush)app.FindResource("B.Dim"), BorderBrush = (Brush)app.FindResource("B.CardBorder"), BorderThickness = new Thickness(1), Padding = new Thickness(10, 8, 10, 8)
            };
            sp.Children.Add(lic);

            var ok = new Button { Content = "Close", Style = (Style)app.FindResource("Btn.Primary"), HorizontalAlignment = HorizontalAlignment.Right, Margin = new Thickness(0, 18, 0, 0), IsDefault = true, IsCancel = true };
            ok.Click += (s, e) => w.Close();
            sp.Children.Add(ok);
            w.Content = sp;
            w.Tag = new Action<int>(r => w.Close());   // test hook: "dialog 1"
            w.Loaded += (s, e) => { Native2.Rounded(w); ok.Focus(); };
            w.MouseLeftButtonDown += (s, e) => { try { w.DragMove(); } catch { } };
            App.Current2.RegisterDialog(w);
            w.ShowDialog();
        }

        static string License()
        {
            try
            {
                using (var st = typeof(Dialogs).Assembly.GetManifestResourceStream("LICENSE"))
                using (var r = new System.IO.StreamReader(st)) return r.ReadToEnd().Trim();
            }
            catch { return "MIT License - Copyright (c) 2026 1nstntKaRma"; }
        }

        public static bool Ask(Window owner, string title, string text, string yes, string no)
        {
            return Show(owner, title, text, yes, no) == 1;
        }

        /// <summary>Returns 1 for the first (primary) button, 2 for the second, 0 for closed.</summary>
        public static int Show(Window owner, string title, string text, string primary, string secondary)
        {
            var w = new Window
            {
                Title = "Choppy",
                Width = 440,
                SizeToContent = SizeToContent.Height,
                WindowStyle = WindowStyle.None,
                ResizeMode = ResizeMode.NoResize,
                ShowInTaskbar = owner == null || !owner.IsVisible,
                Background = (Brush)Application.Current.FindResource("B.Window"),
                BorderBrush = (Brush)Application.Current.FindResource("B.WinBorder"),
                BorderThickness = new Thickness(1),
                FontFamily = (FontFamily)Application.Current.FindResource("F.Ui"),
                Foreground = (Brush)Application.Current.FindResource("B.Text"),
                WindowStartupLocation = owner != null && owner.IsVisible ? WindowStartupLocation.CenterOwner : WindowStartupLocation.CenterScreen,
                Topmost = owner == null || !owner.IsVisible
            };
            System.Windows.Shell.WindowChrome.SetWindowChrome(w, new System.Windows.Shell.WindowChrome { CaptionHeight = 30, ResizeBorderThickness = new Thickness(0), GlassFrameThickness = new Thickness(0, 0, 0, 1), UseAeroCaptionButtons = false });
            if (owner != null && owner.IsVisible) w.Owner = owner;
            int result = 0;
            var sp = new StackPanel { Margin = new Thickness(22, 20, 22, 20) };
            sp.Children.Add(new TextBlock { Text = title, FontSize = 18, FontWeight = FontWeights.SemiBold, TextWrapping = TextWrapping.Wrap });
            if (!string.IsNullOrEmpty(text))
                sp.Children.Add(new TextBlock { Text = text, FontSize = 14, Foreground = (Brush)Application.Current.FindResource("B.Dim"), TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 8, 0, 0) });
            var row = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Right, Margin = new Thickness(0, 20, 0, 0) };
            if (secondary != null)
            {
                var b2 = new Button { Content = secondary, Style = (Style)Application.Current.FindResource("Btn"), Margin = new Thickness(0, 0, 10, 0), IsCancel = true };
                b2.Click += (s, e) => { result = 2; w.Close(); };
                row.Children.Add(b2);
            }
            var b1 = new Button { Content = primary, Style = (Style)Application.Current.FindResource("Btn.Primary"), IsDefault = true };
            b1.Click += (s, e) => { result = 1; w.Close(); };
            row.Children.Add(b1);
            sp.Children.Add(row);
            w.Content = sp;
            w.Tag = new Action<int>(r => { result = r; w.Close(); });   // test hook presses buttons through this
            w.Loaded += (s, e) => { Native2.Rounded(w); b1.Focus(); };
            w.MouseLeftButtonDown += (s, e) => { try { w.DragMove(); } catch { } };
            App.Current2.RegisterDialog(w);
            w.ShowDialog();
            return result;
        }
    }
}
