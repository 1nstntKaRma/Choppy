//! Controls.cs: the speed graph (smooth line, gradient fill, dashed lavender average, end dot), the size donut,
//! and the drive / server icons (MainWindow.xaml G.Drive / G.Server). Boxes come in pixels; the shapes inside are
//! drawn in DIPs from the box's corner, like WPF's OnRender.

use look_common::graph::Graph;

use crate::gfx::*;

/// SpeedGraph.OnRender.
pub fn speed_graph(cv: &mut Canvas, g: &Graph, r: Rect) {
    let s = cv.scale;
    let (ox, oy, w, h) = (r.x / s, r.y / s, r.w / s, r.h / s);
    // the grid line at the top (6 + 0.5 DIPs down)
    cv.line(r.x, r.y + 6.5 * s, r.r(), r.y + 6.5 * s, 0x30FFFFFF, 1.0, None);
    let Some(shape) = g.shape(w as f64, h as f64) else { return };
    let p = |q: (f64, f64)| (ox + q.0 as f32, oy + q.1 as f32);
    let start = p(shape.start);
    let last = shape.segs.last().map(|sg| p(sg.2)).unwrap_or(start);
    let bottom = oy + h;
    let area = cv.path(|sk| {
        pth::begin(sk, start.0, bottom, true);
        pth::line(sk, start.0, start.1);
        for seg in shape.segs.iter() {
            pth::bezier(sk, p(seg.0), p(seg.1), p(seg.2));
        }
        pth::line(sk, last.0, bottom);
        pth::end(sk, true);
    });
    let line = cv.path(|sk| {
        pth::begin(sk, start.0, start.1, false);
        for seg in shape.segs.iter() {
            pth::bezier(sk, p(seg.0), p(seg.1), p(seg.2));
        }
        pth::end(sk, false);
    });
    // the fill: #702E8FD8 -> #281E5A8A at 0.6 -> #00161A20, over the filled area's own bounds (WPF maps a
    // gradient to the bounding box of what it fills)
    let ftop = r.y + shape.area_top as f32 * s;
    if let (Some(area), Some(fill)) = (area, cv.gradient(&[(0.0, 0x702E8FD8), (0.6, 0x281E5A8A), (1.0, 0x00161A20)], (r.x, ftop), (r.x, r.b()))) {
        cv.fill_geometry_brush(&area, &fill);
    }
    if let Some(ay) = shape.avg_y {
        let y = r.y + ay as f32 * s;
        cv.line(r.x, y, r.r(), y, 0xFFC4A7FF, 1.4, Some(&[4.0, 3.0]));
    }
    if let Some(line) = line {
        let rs = cv.round_stroke();
        cv.stroke_geometry(&line, 0xFF5AC8FA, 2.0, rs.as_ref());
    }
    let (lx, ly) = (last.0 * s, last.1 * s);
    cv.ellipse(lx, ly, 6.0 * s, 6.0 * s, 0x405AC8FA);
    cv.ellipse(lx, ly, 3.2 * s, 3.2 * s, 0xFF5AC8FA);
}

/// Donut.OnRender: the parts with 2.2 degree gaps over a track, 13 thick, starting at the top.
pub fn donut(cv: &mut Canvas, parts: &[(f64, u32)], r: Rect) {
    let s = cv.scale;
    let thick = 13.0f32;
    let (w, h) = (r.w / s, r.h / s);
    let size = w.min(h);
    let (cx, cy) = (r.x / s + w / 2.0, r.y / s + h / 2.0);
    let rad = size / 2.0 - thick / 2.0;
    if let Some(t) = cv.path(|sk| {
        pth::begin(sk, cx, cy - rad, false);
        pth::arc(sk, cx, cy + rad, rad, false);
        pth::arc(sk, cx, cy - rad, rad, false);
        pth::end(sk, true);
    }) {
        cv.stroke_geometry(&t, 0xFF2A2F37, thick, None);
    }
    let total: f64 = parts.iter().map(|p| p.0).sum();
    if total <= 0.0 {
        return;
    }
    let gap = if parts.len() > 1 { 2.2 } else { 0.0 };
    let mut a = -90.0f64;
    for &(v, c) in parts {
        let sweep = v / total * 360.0;
        if sweep <= 0.01 {
            continue;
        }
        let mut sw = sweep - gap;
        if sw < 0.8 {
            sw = sweep.min(0.8);
        }
        let a0 = (a + gap / 2.0).to_radians();
        let a1 = (a + gap / 2.0 + sw).to_radians();
        let (x0, y0) = (cx + rad * a0.cos() as f32, cy + rad * a0.sin() as f32);
        let (x1, y1) = (cx + rad * a1.cos() as f32, cy + rad * a1.sin() as f32);
        if let Some(g) = cv.path(|sk| {
            pth::begin(sk, x0, y0, false);
            pth::arc(sk, x1, y1, rad, sw > 180.0);
            pth::end(sk, false);
        }) {
            cv.stroke_geometry(&g, c, thick, None);
        }
        a += sweep;
    }
}

/// G.Drive: a rounded box with a line across, stroke #AEB6C2 1.3 (or G.Server for network places); (x, y) pixels.
pub fn drive_icon(cv: &mut Canvas, x: f32, y: f32, server: bool) {
    let (x, y) = (x / cv.scale, y / cv.scale);
    let g = if server {
        cv.path(|s| {
            rounded(s, x, y, 1.0, 1.0, 15.0, 17.0);
            for (yy, x0, x1) in [(6.3, 1.0, 15.0), (11.7, 1.0, 15.0), (3.7, 3.6, 5.6), (9.0, 3.6, 5.6), (14.3, 3.6, 5.6)] {
                pth::begin(s, x + x0, y + yy, false);
                pth::line(s, x + x1, y + yy);
                pth::end(s, false);
            }
        })
    } else {
        cv.path(|s| {
            rounded(s, x, y, 1.0, 1.0, 17.0, 13.0);
            pth::begin(s, x + 1.0, y + 5.0, false);
            pth::line(s, x + 17.0, y + 5.0);
            pth::end(s, false);
        })
    };
    if let Some(g) = g {
        cv.stroke_geometry(&g, 0xFFAEB6C2, 1.3, None);
    }
}

/// M l+1.5,t H r-1.5 A1.5 ... (corner radius 1.5).
fn rounded(s: &windows::Win32::Graphics::Direct2D::ID2D1GeometrySink, x: f32, y: f32, l: f32, t: f32, r: f32, b: f32) {
    pth::begin(s, x + l + 1.5, y + t, false);
    pth::line(s, x + r - 1.5, y + t);
    pth::arc(s, x + r, y + t + 1.5, 1.5, false);
    pth::line(s, x + r, y + b - 1.5);
    pth::arc(s, x + r - 1.5, y + b, 1.5, false);
    pth::line(s, x + l + 1.5, y + b);
    pth::arc(s, x + l, y + b - 1.5, 1.5, false);
    pth::line(s, x + l, y + t + 1.5);
    pth::arc(s, x + l + 1.5, y + t, 1.5, false);
    pth::end(s, true);
}
