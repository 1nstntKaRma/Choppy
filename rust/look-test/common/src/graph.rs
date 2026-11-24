//! SpeedGraph's math (Controls.cs): last 30 s (120 samples x 250 ms), light smoothing, a scale that moves
//! smoothly so the line doesn't jump, the average as a dashed line, Catmull-Rom -> Bezier.

pub const WINDOW: usize = 120;

#[derive(Default, Clone)]
pub struct Graph {
    samples: Vec<f64>,
    scale_max: f64,
    average: f64,
    pub updates: u64,
}

/// One cubic segment: control 1, control 2, end.
pub type Seg = ((f64, f64), (f64, f64), (f64, f64));

pub struct Shape {
    pub start: (f64, f64),
    pub segs: Vec<Seg>,
    /// The dashed average line's y, if there is an average.
    pub avg_y: Option<f64>,
    pub top: f64,
    /// The top of the filled area's bounds: WPF's gradient fill runs from here to the bottom
    /// (LinearGradientBrush maps to the geometry's bounding box).
    pub area_top: f64,
}

/// The smallest y a cubic Bezier reaches (its ends, and where dy/dt = 0).
fn bezier_min_y(p0: f64, p1: f64, p2: f64, p3: f64) -> f64 {
    let mut m = p0.min(p3);
    let (a, b, c) = (-p0 + 3.0 * p1 - 3.0 * p2 + p3, 2.0 * (p0 - 2.0 * p1 + p2), p1 - p0);
    let y = |t: f64| {
        let u = 1.0 - t;
        u * u * u * p0 + 3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t * p3
    };
    let mut roots = Vec::new();
    if a.abs() < 1e-12 {
        if b.abs() > 1e-12 {
            roots.push(-c / b);
        }
    } else {
        let d = b * b - 4.0 * a * c;
        if d >= 0.0 {
            roots.push((-b + d.sqrt()) / (2.0 * a));
            roots.push((-b - d.sqrt()) / (2.0 * a));
        }
    }
    for t in roots {
        if t > 0.0 && t < 1.0 {
            m = m.min(y(t));
        }
    }
    m
}

impl Graph {
    pub fn new() -> Self {
        Graph { samples: Vec::new(), scale_max: 1.0, average: 0.0, updates: 0 }
    }

    pub fn clear(&mut self) {
        self.samples.clear();
        self.scale_max = 1.0;
        self.average = 0.0;
    }

    pub fn set_samples(&mut self, all: &[f64]) {
        self.updates += 1;
        let n = WINDOW.min(all.len());
        let s = &all[all.len() - n..];
        // light smoothing so the line reads like a trend, not noise
        let mut sm = vec![0.0; n];
        for i in 0..n {
            let (mut acc, mut w) = (0.0, 0.0);
            for k in -2i64..=2 {
                let j = i as i64 + k;
                if j < 0 || j >= n as i64 {
                    continue;
                }
                let wt = 3.0 - (k.abs() as f64);
                acc += s[j as usize] * wt;
                w += wt;
            }
            sm[i] = if w > 0.0 { acc / w } else { 0.0 };
        }
        let max = sm.iter().cloned().fold(0.0, f64::max);
        // scale moves smoothly so the line doesn't jump
        let target = (max * 1.25).max(1.0);
        self.scale_max = if self.scale_max <= 1.0 {
            target
        } else if target > self.scale_max {
            target
        } else {
            self.scale_max * 0.97 + target * 0.03
        };
        self.samples = sm;
    }

    /// The job's average speed: a dashed lavender line across the graph.
    pub fn set_average(&mut self, bytes_per_sec: f64) {
        self.average = bytes_per_sec;
        if self.average * 1.15 > self.scale_max {
            self.scale_max = self.average * 1.25;
        }
    }

    /// The curve in a w x h box (OnRender), or None with fewer than 2 samples.
    pub fn shape(&self, w: f64, h: f64) -> Option<Shape> {
        let (top, bottom) = (6.0, h - 2.0);
        let n = self.samples.len();
        let avg_y = (self.average > 0.0)
            .then(|| (bottom - (self.average / self.scale_max) * (bottom - top)).clamp(top, bottom));
        if n < 2 {
            return None; // only the top grid line (drawn by the caller at top + 0.5)
        }
        let step = w / (WINDOW as f64 - 1.0);
        let x0 = w - (n as f64 - 1.0) * step;
        let p: Vec<(f64, f64)> = (0..n)
            .map(|i| {
                let y = bottom - (self.samples[i] / self.scale_max) * (bottom - top);
                (x0 + i as f64 * step, y.clamp(top, bottom))
            })
            .collect();
        let mut segs = Vec::with_capacity(n - 1);
        for i in 0..n - 1 {
            let p0 = if i > 0 { p[i - 1] } else { p[i] };
            let (p1, p2) = (p[i], p[i + 1]);
            let p3 = if i + 2 < n { p[i + 2] } else { p[i + 1] };
            let c1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
            let c2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
            segs.push((c1, c2, p2));
        }
        let mut area_top = p[0].1;
        let mut prev = p[0].1;
        for sg in segs.iter() {
            area_top = area_top.min(bezier_min_y(prev, sg.0.1, sg.1.1, sg.2.1));
            prev = sg.2.1;
        }
        Some(Shape { start: p[0], segs, avg_y, top, area_top })
    }
}
