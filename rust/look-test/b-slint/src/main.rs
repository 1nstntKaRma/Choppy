//! Look test B: Choppy's copy window in Slint (ui/main.slint), fed every 100 ms from the same demo job as A.
//! See ../../../PLAN.md (UI look test).
#![windows_subsystem = "windows"]

slint::include_modules!();

use std::cell::RefCell;
use std::rc::Rc;

use look_common::demo::{self, Args, Demo};
use look_common::format;
use slint::winit_030::{WinitWindowAccessor, winit};
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};

fn color(c: u32) -> slint::Color {
    slint::Color::from_argb_u8((c >> 24) as u8, (c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// The speed graph's box: the window (576) minus the margins (20 + 20), the card's padding (16 + 16) and borders.
fn graph_width(scale: f32) -> f64 {
    (576.0 - 40.0 - 32.0 - 4.0 / scale) as f64
}

/// Puts the demo's state on screen (MainWindow.Tick).
fn refresh(ui: &MainWindow, d: &mut Demo, editing_limit: bool) {
    let v = d.view();
    ui.set_ext(d.extended);
    ui.set_percent(v.percent.into());
    ui.set_frac(v.frac as f32);
    ui.set_badge_mode(v.badge_mode.into());
    ui.set_badge_detail(v.badge_detail.into());
    ui.set_badge_warn(v.badge_warn);
    ui.set_speed_num(v.speed_num.into());
    ui.set_speed_unit(v.speed_unit.into());
    ui.set_avg_num(v.avg_num.into());
    ui.set_avg_unit(v.avg_unit.into());
    ui.set_c_now(v.c_now.into());
    ui.set_c_avg_label(v.c_avg_label.into());
    ui.set_c_avg(v.c_avg.into());
    ui.set_size_done(v.size_done.into());
    ui.set_size_of(v.size_of.into());
    ui.set_size_total(v.size_total.into());
    ui.set_files(v.files.into());
    ui.set_elapsed(v.elapsed.into());
    ui.set_remain_label(v.remain_label.into());
    ui.set_remain(v.remain.into());
    ui.set_cur_label(v.cur_label.into());
    ui.set_cur_name(v.cur_name.into());
    ui.set_cur_done(v.cur_done.into());
    ui.set_cur_of(v.cur_of.into());
    ui.set_cur_total(v.cur_total.into());
    ui.set_cur_frac(v.cur_frac as f32);
    ui.set_c2_label(v.c2_label.into());
    ui.set_c2_warn(v.c2_warn);
    ui.set_c2_name(v.c2_name.into());
    ui.set_pause_text(v.pause_text.into());
    ui.set_limit_unit(v.limit_unit.into());
    ui.set_close_on(d.close_when_done);
    ui.set_verify_on(d.verify);
    ui.set_limit_on(d.limit_mbps > 0.0);
    // SyncQuick: the box shows the setting unless it is being typed in
    if !editing_limit && ui.get_limit_text().as_str() != v.limit_text {
        ui.set_limit_text(v.limit_text.into());
    }
    ui.set_win_title(v.title.into());

    // the graph, as SVG path commands in the graph's own box
    let w = graph_width(ui.window().scale_factor());
    match d.graph.shape(w, 70.0) {
        Some(sh) => {
            let mut line = format!("M{:.2},{:.2}", sh.start.0, sh.start.1);
            for s in sh.segs.iter() {
                line += &format!(" C{:.2},{:.2} {:.2},{:.2} {:.2},{:.2}", s.0.0, s.0.1, s.1.0, s.1.1, s.2.0, s.2.1);
            }
            let last = sh.segs.last().map(|s| s.2).unwrap_or(sh.start);
            // the area, moved up by its own top (its Path starts there, see main.slint)
            let t = sh.area_top;
            let mut area = format!("M{:.2},{:.2} L{:.2},{:.2}", sh.start.0, 70.0 - t, sh.start.0, sh.start.1 - t);
            for s in sh.segs.iter() {
                area += &format!(" C{:.2},{:.2} {:.2},{:.2} {:.2},{:.2}", s.0.0, s.0.1 - t, s.1.0, s.1.1 - t, s.2.0, s.2.1 - t);
            }
            area += &format!(" L{:.2},{:.2} Z", last.0, 70.0 - t);
            ui.set_graph_line(line.into());
            ui.set_graph_area(area.into());
            ui.set_graph_area_top(sh.area_top as f32);
            ui.set_graph_has(true);
            ui.set_dot_x(last.0 as f32);
            ui.set_dot_y(last.1 as f32);
            ui.set_has_avg(sh.avg_y.is_some());
            ui.set_avg_y(sh.avg_y.unwrap_or(0.0) as f32);
        }
        None => ui.set_graph_has(false),
    }
}

/// Donut.OnRender's arcs in a 76 x 76 box: 2.2 degree gaps, from the top, radius 76 / 2 - 13 / 2.
fn donut() -> Vec<Seg> {
    let parts = Demo::largest();
    let total: f64 = parts.iter().map(|p| p.1.max(1) as f64).sum();
    let (c, r) = (38.0f64, 31.5f64);
    let gap = 2.2;
    let mut a = -90.0f64;
    let mut out = Vec::new();
    for (_, size, col) in parts.iter() {
        let sweep = *size.max(&1) as f64 / total * 360.0;
        let mut s = sweep - gap;
        if s < 0.8 {
            s = sweep.min(0.8);
        }
        let (a0, a1) = ((a + gap / 2.0).to_radians(), (a + gap / 2.0 + s).to_radians());
        let cmd = format!(
            "M{:.3},{:.3} A{r},{r} 0 {} 1 {:.3},{:.3}",
            c + r * a0.cos(),
            c + r * a0.sin(),
            if s > 180.0 { 1 } else { 0 },
            c + r * a1.cos(),
            c + r * a1.sin()
        );
        out.push(Seg { commands: cmd.into(), color: color(*col) });
        a += sweep;
    }
    out
}

/// The DWM touches of Native2.Rounded (dark frame, round corners and border color on Windows 11) + the shadow.
fn frame(ui: &MainWindow) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::*;
    ui.window().with_winit_window(|w: &winit::window::Window| {
        use winit::platform::windows::WindowExtWindows;
        w.set_undecorated_shadow(true);
        if let Ok(h) = w.window_handle()
            && let RawWindowHandle::Win32(h) = h.as_raw()
        {
            let hwnd = HWND(h.hwnd.get() as *mut _);
            unsafe {
                let on: i32 = 1;
                let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4);
                let round: i32 = 2;
                let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(33), &round as *const _ as _, 4);
                let border: u32 = 0x00433A33;
                let _ = DwmSetWindowAttribute(hwnd, DWMWINDOWATTRIBUTE(34), &border as *const _ as _, 4);
            }
        }
    });
}

fn main() {
    let args = Args::parse();
    let ui = MainWindow::new().expect("window");
    let d = Rc::new(RefCell::new(args.demo()));
    ui.set_from_name(demo::FROM_NAME.into());
    ui.set_from_path(demo::FROM_PATH.into());
    ui.set_to_name(demo::TO_NAME.into());
    ui.set_to_path(demo::TO_PATH.into());
    ui.set_donut(ModelRc::new(VecModel::from(donut())));
    let legend: Vec<LegendRow> = Demo::largest()
        .into_iter()
        .map(|(n, s, c)| LegendRow { name: n.into(), size: format::size(s).into(), color: color(c) })
        .collect();
    ui.set_legend(ModelRc::new(VecModel::from(legend)));
    let editing = Rc::new(RefCell::new(false));
    refresh(&ui, &mut d.borrow_mut(), false);

    let w = ui.as_weak();
    let dd = d.clone();
    let ed = editing.clone();
    // one handler per action; each changes the demo and redraws
    let act = move |f: &dyn Fn(&mut Demo, &MainWindow)| {
        if let Some(ui) = w.upgrade() {
            let mut d = dd.borrow_mut();
            f(&mut d, &ui);
            refresh(&ui, &mut d, *ed.borrow());
        }
    };
    let act = Rc::new(act);
    macro_rules! on {
        ($set:ident, $body:expr) => {{
            let a = act.clone();
            ui.$set(move || a(&$body));
        }};
    }
    on!(on_unit_clicked, |d: &mut Demo, _: &MainWindow| d.mbps_unit = !d.mbps_unit);
    on!(on_toggle_close, |d: &mut Demo, _: &MainWindow| d.close_when_done = !d.close_when_done);
    on!(on_toggle_verify, |d: &mut Demo, _: &MainWindow| d.verify = !d.verify);
    on!(on_pause_clicked, |d: &mut Demo, _: &MainWindow| d.paused = !d.paused);
    on!(on_skip_clicked, |d: &mut Demo, _: &MainWindow| d.skip());
    on!(on_cancel_clicked, |d: &mut Demo, _: &MainWindow| d.restart());
    on!(on_settings_clicked, |_: &mut Demo, _: &MainWindow| {});
    {
        let a = act.clone();
        let ed = editing.clone();
        ui.on_toggle_limit(move || {
            a(&|d: &mut Demo, ui: &MainWindow| {
                let on = d.limit_mbps <= 0.0;
                d.set_limit_on(on);
                if on {
                    // LimitBox.Focus(); SelectAll()
                    ui.set_limit_text(d.view().limit_text.into());
                    ui.set_focus_limit_req(ui.get_focus_limit_req() + 1);
                }
            });
            *ed.borrow_mut() = false;
        });
    }
    {
        let a = act.clone();
        let w2 = ui.as_weak();
        ui.on_view_clicked(move || {
            a(&|d: &mut Demo, _: &MainWindow| d.extended = !d.extended);
            // SizeToContent="Height" + KeepOnScreen: the window follows the content's height
            if let Some(ui) = w2.upgrade() {
                let h = ui.get_content_height();
                ui.window().set_size(slint::LogicalSize::new(576.0, h));
            }
        });
    }
    {
        let dd = d.clone();
        let ed = editing.clone();
        let w2 = ui.as_weak();
        ui.on_limit_edited(move |text: SharedString| {
            // WholeNumberBox + MaxLength 6: digits only
            let clean: String = text.chars().filter(|c| c.is_ascii_digit()).take(6).collect();
            if let Some(ui) = w2.upgrade()
                && clean != text.as_str()
            {
                ui.set_limit_text(clean.clone().into());
            }
            *ed.borrow_mut() = true;
            dd.borrow_mut().set_limit_text(&clean);
        });
    }
    {
        let w2 = ui.as_weak();
        ui.on_start_drag(move || {
            if let Some(ui) = w2.upgrade() {
                ui.window().with_winit_window(|w: &winit::window::Window| {
                    let _ = w.drag_window();
                });
            }
        });
    }
    {
        let w2 = ui.as_weak();
        ui.on_close_clicked(move || {
            if let Some(ui) = w2.upgrade() {
                let _ = ui.hide();
            }
            let _ = slint::quit_event_loop();
        });
    }

    // every 100 ms, like the real window
    let timer = Timer::default();
    {
        let w2 = ui.as_weak();
        let dd = d.clone();
        let ed = editing.clone();
        let mut last = std::time::Instant::now();
        timer.start(TimerMode::Repeated, std::time::Duration::from_millis(100), move || {
            let now = std::time::Instant::now();
            let dt = (now - last).as_secs_f64().min(0.5);
            last = now;
            if let Some(ui) = w2.upgrade() {
                let mut d = dd.borrow_mut();
                d.advance(dt);
                refresh(&ui, &mut d, *ed.borrow() && ui.get_limit_on());
            }
        });
    }
    let exit_timer = Timer::default();
    if let Some(secs) = args.exit_after {
        exit_timer.start(TimerMode::SingleShot, std::time::Duration::from_secs_f64(secs), || {
            let _ = slint::quit_event_loop();
        });
    }

    // where it opens: given, or the middle of the primary screen's work area (CenterScreen)
    let pos = args.pos;
    let setup = Timer::default();
    {
        let w2 = ui.as_weak();
        setup.start(TimerMode::SingleShot, std::time::Duration::from_millis(0), move || {
            if let Some(ui) = w2.upgrade() {
                frame(&ui);
                let _ = &pos;
            }
        });
    }
    if let Some((x, y)) = pos {
        ui.window().set_position(slint::PhysicalPosition::new(x, y));
    } else {
        use windows::Win32::Foundation::RECT;
        use windows::Win32::UI::WindowsAndMessaging::*;
        let mut wa = RECT::default();
        unsafe {
            let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut wa as *mut _ as _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
        }
        let scale = unsafe { windows::Win32::UI::HiDpi::GetDpiForSystem() } as f32 / 96.0;
        let (pw, ph) = ((576.0 * scale) as i32, (ui.get_content_height() * scale) as i32);
        ui.window().set_position(slint::PhysicalPosition::new(wa.left + (wa.right - wa.left - pw) / 2, wa.top + (wa.bottom - wa.top - ph) / 2));
    }
    let _ = ui.get_legend().row_count();
    ui.run().expect("run");
}
