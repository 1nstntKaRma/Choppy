//! Controls.cs: the live speed graph (smooth line, gradient fill, dashed lavender average, end dot), the size-split
//! donut, PathText (a dim parent + bright last part; a long parent loses its start), and the drive / server icons
//! (MainWindow.xaml G.Drive / G.Server).

use std::cell::RefCell;
use std::rc::Rc;

use crate::sys::gfx::{Argb, Canvas, Cap, Font, PathBuilder, Rect, TextOpts};
use crate::ui::layout::{El, draw};
use crate::ui::theme::*;

/// SpeedGraph: the last 30 s (120 samples x 250 ms), lightly smoothed, a scale that moves smoothly so the line
/// doesn't jump, the job's average as a dashed line.
#[derive(Clone)]
pub struct SpeedGraph {
    samples: Vec<f64>,
    scale_max: f64,
    average: f64,
    pub window: usize,
    /// How many times new samples arrived (tests check that it keeps moving).
    pub updates: i64,
}

impl Default for SpeedGraph {
    fn default() -> Self {
        SpeedGraph { samples: Vec::new(), scale_max: 1.0, average: 0.0, window: 120, updates: 0 }
    }
}

impl SpeedGraph {
    pub fn set_samples(&mut self, all: &[f64]) {
        self.updates += 1;
        let n = self.window.min(all.len());
        let s = &all[all.len() - n..];
        // light smoothing so the line reads like a trend, not noise
        let mut sm = vec![0.0; n];
        for (i, smi) in sm.iter_mut().enumerate() {
            let (mut acc, mut w) = (0.0, 0.0);
            for k in -2i64..=2 {
                let j = i as i64 + k;
                if j < 0 || j >= n as i64 {
                    continue;
                }
                let wt = 3.0 - k.abs() as f64;
                acc += s[j as usize] * wt;
                w += wt;
            }
            *smi = if w > 0.0 { acc / w } else { 0.0 };
        }
        let max = sm.iter().cloned().fold(0.0, f64::max);
        // scale moves smoothly so the line doesn't jump
        let target = (max * 1.25).max(1.0);
        self.scale_max = if self.scale_max <= 1.0 || target > self.scale_max {
            target
        } else {
            self.scale_max * 0.97 + target * 0.03
        };
        self.samples = sm;
    }

    pub fn clear(&mut self) {
        self.samples.clear();
        self.scale_max = 1.0;
        self.average = 0.0;
    }

    /// The job's average speed: a dashed lavender line across the graph.
    pub fn set_average(&mut self, bytes_per_sec: f64) {
        self.average = bytes_per_sec;
        if self.average * 1.15 > self.scale_max {
            self.scale_max = self.average * 1.25;
        }
    }

    /// OnRender into `r` (DIPs).
    pub fn render(&self, cv: &mut Canvas, r: Rect) {
        let (w, h) = (r.w, r.h);
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let (top, bottom) = (6.0, h - 2.0);
        cv.line(r.x, r.y + top + 0.5, r.x + w, r.y + top + 0.5, 0x30FFFFFF, 1.0, &[]);
        let n = self.samples.len();
        if n < 2 {
            return;
        }
        let step = w / (self.window as f64 - 1.0);
        let x0 = w - (n as f64 - 1.0) * step;
        let pts: Vec<(f64, f64)> = (0..n)
            .map(|i| {
                let y = bottom - (self.samples[i] / self.scale_max) * (bottom - top);
                (r.x + x0 + i as f64 * step, r.y + y.clamp(top, bottom))
            })
            .collect();
        let curve = |pb: &mut PathBuilder| {
            // Catmull-Rom -> Bezier
            for i in 0..n - 1 {
                let p0 = if i > 0 { pts[i - 1] } else { pts[i] };
                let (p1, p2) = (pts[i], pts[i + 1]);
                let p3 = if i + 2 < n { pts[i + 2] } else { pts[i + 1] };
                let c1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
                let c2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
                pb.bezier(c1, c2, p2);
            }
        };
        let area = cv.path(|pb| {
            pb.begin(pts[0].0, r.y + h, true);
            pb.line(pts[0].0, pts[0].1);
            curve(pb);
            pb.line(pts[n - 1].0, r.y + h);
            pb.end(true);
        });
        let line = cv.path(|pb| {
            pb.begin(pts[0].0, pts[0].1, false);
            curve(pb);
            pb.end(false);
        });
        // the fill maps to the filled area's own bounds (a WPF brush on a geometry)
        if let Some(a) = &area {
            cv.fill_path_gradient(a, &[(0.0, 0x702E8FD8), (0.6, 0x281E5A8A), (1.0, 0x00161A20)]);
        }
        if self.average > 0.0 {
            let ay = (bottom - (self.average / self.scale_max) * (bottom - top)).clamp(top, bottom);
            cv.line(r.x, r.y + ay, r.x + w, r.y + ay, AVG_SPEED, 1.4, &[4.0, 3.0]);
        }
        if let Some(l) = &line {
            cv.stroke_path(l, NOW_SPEED, 2.0, Cap::Round);
        }
        let last = pts[n - 1];
        cv.ellipse(last.0, last.1, 6.0, 6.0, 0x405AC8FA);
        cv.ellipse(last.0, last.1, 3.2, 3.2, NOW_SPEED);
    }

    pub fn el(&self) -> El {
        let me = self.clone();
        draw((0.0, 0.0), move |cv, r| me.render(cv, r))
    }
}

/// Donut: the parts with 2.2 degree gaps over a track, 13 thick, starting at the top.
pub fn donut(parts: Vec<(f64, Argb)>) -> El {
    draw((0.0, 0.0), move |cv, r| {
        let thick = 13.0;
        let size = r.w.min(r.h);
        if size <= 0.0 {
            return;
        }
        let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let rad = size / 2.0 - thick / 2.0;
        cv.stroke_ellipse(cx, cy, rad, TRACK, thick);
        let total: f64 = parts.iter().map(|p| p.0).sum();
        if total <= 0.0 {
            return;
        }
        let gap = if parts.len() > 1 { 2.2 } else { 0.0 };
        let mut a = -90.0f64;
        for &(v, c) in parts.iter() {
            let sweep = v / total * 360.0;
            if sweep <= 0.01 {
                continue;
            }
            let mut sw = sweep - gap;
            if sw < 0.8 {
                sw = sweep.min(0.8);
            }
            if sw >= 359.9 {
                cv.stroke_ellipse(cx, cy, rad, c, thick);
            } else {
                let a0 = (a + gap / 2.0).to_radians();
                let a1 = (a + gap / 2.0 + sw).to_radians();
                if let Some(g) = cv.path(|pb| {
                    pb.begin(cx + rad * a0.cos(), cy + rad * a0.sin(), false);
                    pb.arc(cx + rad * a1.cos(), cy + rad * a1.sin(), rad, sw > 180.0);
                    pb.end(false);
                }) {
                    cv.stroke_path(&g, c, thick, Cap::Flat);
                }
            }
            a += sweep;
        }
    })
}

/// PathText: a path shown as dim parent + bright last part; a long parent loses its start ("…\x\y\Leaf"). Its
/// text is FormattedText (WPF's Ideal text formatting, not the window's Display mode).
pub struct PathText {
    pub parent_c: Argb,
    pub leaf_c: Argb,
    pub size: f64,
    pub weight: u16,
}

impl PathText {
    /// SetPath: the parent (with its trailing "\") and the leaf.
    pub fn split(path: &str) -> (String, String) {
        let p = path.trim_end_matches('\\');
        match p.rfind('\\') {
            Some(i) if !p.ends_with(':') => (p[..=i].to_string(), p[i + 1..].to_string()),
            _ => (String::new(), path.to_string()),
        }
    }

    pub fn el(&self, path: &str) -> El {
        let (parent, leaf) = Self::split(path);
        let font = Font { fam: crate::sys::gfx::Fam::Ui, size: self.size, weight: self.weight };
        let (pc, lc) = (self.parent_c, self.leaf_c);
        // measured width: the whole path (MeasureOverride); its height: FormattedText's (LineSpacing x size)
        let w_full = Rc::new(RefCell::new(None::<f64>));
        let wf = w_full.clone();
        let full = format!("{parent}{leaf}");
        let mut e = draw((0.0, 0.0), move |cv, r| {
            let ideal = TextOpts { ideal: true, ..Default::default() };
            let lf = cv.g.text(&[(&leaf, font, lc)], &ideal, cv.scale);
            let mut par = parent.clone();
            let mut pf = cv.g.text(&[(&par, font, pc)], &ideal, cv.scale);
            if pf.w + lf.w > r.w && !par.is_empty() {
                // drop characters from the start of the parent until it fits
                let chars: Vec<char> = parent.chars().collect();
                let mut cut = 1;
                while cut < chars.len() {
                    let s: String = std::iter::once('…').chain(chars[cut..].iter().copied()).collect();
                    pf = cv.g.text(&[(&s, font, pc)], &ideal, cv.scale);
                    if pf.w + lf.w <= r.w {
                        par = s;
                        break;
                    }
                    cut += 1;
                }
                if cut >= chars.len() {
                    par = "…\\".into();
                    pf = cv.g.text(&[(&par, font, pc)], &ideal, cv.scale);
                }
                // very little room: the folder's own name matters more than "…\"
                if pf.w + lf.w > r.w {
                    par.clear();
                }
            }
            let mut x = 0.0;
            if !par.is_empty() {
                cv.text(&pf, r.x, r.y);
                x = pf.w;
            }
            let o = TextOpts { ideal: true, trim: true, max_w: (r.w - x).max(1.0), ..Default::default() };
            let lt = cv.g.text(&[(&leaf, font, lc)], &o, cv.scale);
            cv.text(&lt, r.x + x, r.y);
            let _ = &wf;
        });
        e.tip = Some(path.to_string());
        let _ = (full, w_full);
        e
    }
}

/// The size PathText asks for: the whole path's width and FormattedText's height.
pub fn path_text_size(g: &crate::sys::gfx::Gfx, path: &str, size: f64, weight: u16, scale: f64) -> (f64, f64) {
    let font = Font { fam: crate::sys::gfx::Fam::Ui, size, weight };
    let t = g.text(&[(path, font, 0)], &TextOpts { ideal: true, ..Default::default() }, scale);
    (t.w, t.h)
}

/// G.Drive: a rounded box with a line across, or G.Server for network places; stroke #AEB6C2 1.3. The Path asks
/// for its geometry's bounds with the pen (17.65 x 13.65 / 15.65 x 17.65).
pub fn drive_icon(server: bool) -> El {
    let size = if server { (15.65, 17.65) } else { (17.65, 13.65) };
    draw(size, move |cv, r| {
        let (x, y) = (r.x, r.y);
        let g = cv.path(|pb| {
            if server {
                rounded(pb, x, y, 1.0, 1.0, 15.0, 17.0);
                for (yy, x0, x1) in [(6.3, 1.0, 15.0), (11.7, 1.0, 15.0), (3.7, 3.6, 5.6), (9.0, 3.6, 5.6), (14.3, 3.6, 5.6)] {
                    pb.begin(x + x0, y + yy, false);
                    pb.line(x + x1, y + yy);
                    pb.end(false);
                }
            } else {
                rounded(pb, x, y, 1.0, 1.0, 17.0, 13.0);
                pb.begin(x + 1.0, y + 5.0, false);
                pb.line(x + 17.0, y + 5.0);
                pb.end(false);
            }
        });
        if let Some(g) = g {
            cv.stroke_path(&g, 0xFFAEB6C2, 1.3, Cap::Flat);
        }
    })
}

/// M l+1.5,t H r-1.5 A1.5 ... (corner radius 1.5).
fn rounded(pb: &mut PathBuilder, x: f64, y: f64, l: f64, t: f64, r: f64, b: f64) {
    pb.begin(x + l + 1.5, y + t, false);
    pb.line(x + r - 1.5, y + t);
    pb.arc(x + r, y + t + 1.5, 1.5, false);
    pb.line(x + r, y + b - 1.5);
    pb.arc(x + r - 1.5, y + b, 1.5, false);
    pb.line(x + l + 1.5, y + b);
    pb.arc(x + l, y + b - 1.5, 1.5, false);
    pb.line(x + l, y + t + 1.5);
    pb.arc(x + l + 1.5, y + t, 1.5, false);
    pb.end(true);
}
