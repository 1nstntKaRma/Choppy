using System;
using System.Collections.Generic;
using System.Globalization;
using System.Windows;
using System.Windows.Media;

namespace Choppy.UI
{
    /// <summary>Live speed graph: smooth line with a soft gradient fill and an end dot (last 30 s).</summary>
    internal sealed class SpeedGraph : FrameworkElement
    {
        double[] samples = new double[0];
        double scaleMax = 1;
        static readonly Pen LinePen;
        static readonly Pen GridPen;
        static readonly Brush DotBrush = Frozen(new SolidColorBrush(Color.FromRgb(0x5A, 0xC8, 0xFA)));
        static readonly Brush DotHalo = Frozen(new SolidColorBrush(Color.FromArgb(0x40, 0x5A, 0xC8, 0xFA)));
        static readonly Brush Fill;

        static SpeedGraph()
        {
            LinePen = new Pen(new SolidColorBrush(Color.FromRgb(0x5A, 0xC8, 0xFA)), 2) { LineJoin = PenLineJoin.Round, StartLineCap = PenLineCap.Round, EndLineCap = PenLineCap.Round };
            LinePen.Freeze();
            GridPen = new Pen(new SolidColorBrush(Color.FromArgb(0x30, 0xFF, 0xFF, 0xFF)), 1);
            GridPen.Freeze();
            AvgPen = new Pen(new SolidColorBrush(Color.FromRgb(0xC4, 0xA7, 0xFF)), 1.4) { DashStyle = new DashStyle(new double[] { 4, 3 }, 0) };
            AvgPen.Freeze();
            var g = new LinearGradientBrush { StartPoint = new Point(0, 0), EndPoint = new Point(0, 1) };
            g.GradientStops.Add(new GradientStop(Color.FromArgb(0x70, 0x2E, 0x8F, 0xD8), 0));
            g.GradientStops.Add(new GradientStop(Color.FromArgb(0x28, 0x1E, 0x5A, 0x8A), 0.6));
            g.GradientStops.Add(new GradientStop(Color.FromArgb(0x00, 0x16, 0x1A, 0x20), 1));
            g.Freeze();
            Fill = g;
        }

        static Brush Frozen(Brush b) { b.Freeze(); return b; }

        public int Window = 120;  // samples shown (120 x 250 ms = 30 s)

        public long Updates;       // how many times new samples arrived (tests check that it keeps moving)

        public void SetSamples(IList<double> all)
        {
            Updates++;
            int n = Math.Min(Window, all.Count);
            var s = new double[n];
            for (int i = 0; i < n; i++) s[i] = all[all.Count - n + i];
            // light smoothing so the line reads like a trend, not noise
            var sm = new double[n];
            for (int i = 0; i < n; i++)
            {
                double acc = 0, w = 0;
                for (int k = -2; k <= 2; k++)
                {
                    int j = i + k; if (j < 0 || j >= n) continue;
                    double wt = 3 - Math.Abs(k);
                    acc += s[j] * wt; w += wt;
                }
                sm[i] = w > 0 ? acc / w : 0;
            }
            samples = sm;
            double max = 0;
            foreach (var v in sm) if (v > max) max = v;
            // scale moves smoothly so the line doesn't jump
            double target = Math.Max(1, max * 1.25);
            scaleMax = scaleMax <= 1 ? target : (target > scaleMax ? target : scaleMax * 0.97 + target * 0.03);
            InvalidateVisual();
        }

        public void Clear() { samples = new double[0]; scaleMax = 1; average = 0; InvalidateVisual(); }

        double average;
        static readonly Pen AvgPen;

        /// <summary>The job's average speed: a dashed lavender line across the graph.</summary>
        public void SetAverage(double bytesPerSec)
        {
            average = bytesPerSec;
            if (average * 1.15 > scaleMax) scaleMax = average * 1.25;
            InvalidateVisual();
        }

        protected override void OnRender(DrawingContext dc)
        {
            double w = ActualWidth, h = ActualHeight;
            if (w <= 0 || h <= 0) return;
            dc.DrawRectangle(Brushes.Transparent, null, new Rect(0, 0, w, h));
            double top = 6, bottom = h - 2;
            dc.DrawLine(GridPen, new Point(0, top + 0.5), new Point(w, top + 0.5));
            int n = samples.Length;
            if (n < 2) return;
            var pts = new Point[n];
            double step = w / (Window - 1);
            double x0 = w - (n - 1) * step;
            for (int i = 0; i < n; i++)
            {
                double y = bottom - (samples[i] / scaleMax) * (bottom - top);
                pts[i] = new Point(x0 + i * step, Math.Max(top, Math.Min(bottom, y)));
            }
            var line = new StreamGeometry();
            var area = new StreamGeometry();
            using (var c = line.Open()) { c.BeginFigure(pts[0], false, false); Curve(c, pts); }
            using (var c = area.Open())
            {
                c.BeginFigure(new Point(pts[0].X, h), true, true);
                c.LineTo(pts[0], false, false);
                Curve(c, pts);
                c.LineTo(new Point(pts[n - 1].X, h), false, false);
            }
            line.Freeze(); area.Freeze();
            dc.DrawGeometry(Fill, null, area);
            if (average > 0)
            {
                double ay = Math.Max(top, Math.Min(bottom, bottom - (average / scaleMax) * (bottom - top)));
                dc.DrawLine(AvgPen, new Point(0, ay), new Point(w, ay));
            }
            dc.DrawGeometry(null, LinePen, line);
            var last = pts[n - 1];
            dc.DrawEllipse(DotHalo, null, last, 6, 6);
            dc.DrawEllipse(DotBrush, null, last, 3.2, 3.2);
        }

        static void Curve(StreamGeometryContext c, Point[] p)
        {
            // Catmull-Rom -> Bezier
            for (int i = 0; i < p.Length - 1; i++)
            {
                Point p0 = i > 0 ? p[i - 1] : p[i], p1 = p[i], p2 = p[i + 1], p3 = i + 2 < p.Length ? p[i + 2] : p[i + 1];
                var c1 = new Point(p1.X + (p2.X - p0.X) / 6, p1.Y + (p2.Y - p0.Y) / 6);
                var c2 = new Point(p2.X - (p3.X - p1.X) / 6, p2.Y - (p3.Y - p1.Y) / 6);
                c.BezierTo(c1, c2, p2, true, true);
            }
        }
    }

    /// <summary>Size split donut.</summary>
    internal sealed class Donut : FrameworkElement
    {
        readonly List<KeyValuePair<double, Color>> parts = new List<KeyValuePair<double, Color>>();
        public double Thickness = 13;
        static readonly Brush Track = Fz(new SolidColorBrush(Color.FromRgb(0x2A, 0x2F, 0x37)));
        static Brush Fz(Brush b) { b.Freeze(); return b; }

        public void SetParts(IEnumerable<KeyValuePair<double, Color>> p)
        {
            parts.Clear();
            parts.AddRange(p);
            InvalidateVisual();
        }

        protected override void OnRender(DrawingContext dc)
        {
            double size = Math.Min(ActualWidth, ActualHeight);
            if (size <= 0) return;
            var center = new Point(ActualWidth / 2, ActualHeight / 2);
            double r = size / 2 - Thickness / 2;
            var trackPen = new Pen(Track, Thickness);
            dc.DrawEllipse(null, trackPen, center, r, r);
            double total = 0;
            foreach (var p in parts) total += p.Key;
            if (total <= 0) return;
            double gap = parts.Count > 1 ? 2.2 : 0;   // degrees between parts
            double a = -90;
            foreach (var p in parts)
            {
                double sweep = p.Key / total * 360;
                if (sweep <= 0.01) continue;
                double s = sweep - gap;
                if (s < 0.8) s = Math.Min(sweep, 0.8);
                var pen = new Pen(new SolidColorBrush(p.Value), Thickness) { StartLineCap = PenLineCap.Flat, EndLineCap = PenLineCap.Flat };
                DrawArc(dc, pen, center, r, a + gap / 2, s);
                a += sweep;
            }
        }

        static void DrawArc(DrawingContext dc, Pen pen, Point c, double r, double startDeg, double sweepDeg)
        {
            if (sweepDeg >= 359.9) { dc.DrawEllipse(null, pen, c, r, r); return; }
            double a0 = startDeg * Math.PI / 180, a1 = (startDeg + sweepDeg) * Math.PI / 180;
            var p0 = new Point(c.X + r * Math.Cos(a0), c.Y + r * Math.Sin(a0));
            var p1 = new Point(c.X + r * Math.Cos(a1), c.Y + r * Math.Sin(a1));
            var g = new StreamGeometry();
            using (var ctx = g.Open())
            {
                ctx.BeginFigure(p0, false, false);
                ctx.ArcTo(p1, new Size(r, r), 0, sweepDeg > 180, SweepDirection.Clockwise, true, false);
            }
            g.Freeze();
            dc.DrawGeometry(null, pen, g);
        }
    }

    /// <summary>A path shown as dim parent + bright last part; a long parent loses its start ("…\x\y\Leaf").</summary>
    internal sealed class PathText : FrameworkElement
    {
        string parent = "", leaf = "";
        public Brush ParentBrush = Fz(new SolidColorBrush(Color.FromRgb(0xAE, 0xB6, 0xC2)));
        public Brush LeafBrush = Fz(new SolidColorBrush(Color.FromRgb(0xF3, 0xF5, 0xF7)));
        public double FontSize = 15;
        Typeface Face = new Typeface(new FontFamily("Segoe UI Variable Text, Segoe UI"), FontStyles.Normal, FontWeights.Normal, FontStretches.Normal);

        /// <summary>One bright color for the whole path (the route line), semibold.</summary>
        public void Bright(Brush b, double size)
        {
            ParentBrush = b; LeafBrush = b; FontSize = size;
            Face = new Typeface(new FontFamily("Segoe UI Variable Text, Segoe UI"), FontStyles.Normal, FontWeights.SemiBold, FontStretches.Normal);
            InvalidateMeasure(); InvalidateVisual();
        }
        static Brush Fz(Brush b) { b.Freeze(); return b; }

        public void SetPath(string path)
        {
            path = path ?? "";
            string p = path.TrimEnd('\\');
            int i = p.LastIndexOf('\\');
            if (i < 0 || p.EndsWith(":")) { parent = ""; leaf = path; }
            else { parent = p.Substring(0, i + 1); leaf = p.Substring(i + 1); }
            ToolTip = path;
            InvalidateMeasure();
            InvalidateVisual();
        }

        FormattedText Ft(string s, Brush b)
        {
            double dpi = 1;
            try { dpi = VisualTreeHelper.GetDpi(this).PixelsPerDip; } catch { }
            return new FormattedText(s, CultureInfo.CurrentUICulture, FlowDirection.LeftToRight, Face, FontSize, b, dpi);
        }

        protected override Size MeasureOverride(Size available)
        {
            var a = Ft(parent + leaf, LeafBrush);
            double w = double.IsInfinity(available.Width) ? a.WidthIncludingTrailingWhitespace : Math.Min(available.Width, a.WidthIncludingTrailingWhitespace);
            return new Size(w, a.Height);
        }

        protected override void OnRender(DrawingContext dc)
        {
            double w = ActualWidth;
            var lf = Ft(leaf, LeafBrush);
            string par = parent;
            var pf = Ft(par, ParentBrush);
            if (pf.WidthIncludingTrailingWhitespace + lf.WidthIncludingTrailingWhitespace > w && par.Length > 0)
            {
                // drop characters from the start of the parent until it fits
                int cut = 1;
                while (cut < par.Length)
                {
                    pf = Ft("…" + par.Substring(cut), ParentBrush);
                    if (pf.WidthIncludingTrailingWhitespace + lf.WidthIncludingTrailingWhitespace <= w) break;
                    cut++;
                }
                if (cut >= par.Length) pf = Ft("…\\", ParentBrush);
                // very little room: the folder's own name matters more than "…\"
                if (pf.WidthIncludingTrailingWhitespace + lf.WidthIncludingTrailingWhitespace > w) par = "";
            }
            double x = 0;
            if (par.Length > 0) { dc.DrawText(pf, new Point(0, 0)); x = pf.WidthIncludingTrailingWhitespace; }
            lf.MaxTextWidth = Math.Max(1, w - x);
            lf.Trimming = TextTrimming.CharacterEllipsis;
            lf.MaxLineCount = 1;
            dc.DrawText(lf, new Point(x, 0));
        }
    }
}
