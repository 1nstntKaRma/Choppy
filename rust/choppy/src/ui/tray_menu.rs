//! The tray icon's menu (App.cs UpdateTrayMenu + DarkMenuRenderer): "Update to vX" with a green dot when an update is
//! out (2.02), Show Choppy, Settings, Default file manager (checked when on), Quit Choppy, drawn like the C#'s dark WinForms ContextMenuStrip (#23282F, the selected item
//! #2F353D, separators and border #3A4048, text #E6E9EE, the check on #0E72E6) in the menu font (Segoe UI 9 pt).

use std::cell::RefCell;
use std::rc::Rc;

use crate::engine::settings::Settings;
use crate::sys::gfx::{Fam, Font, Rect};
use crate::sys::win::{self, Spec};
use crate::ui::host::{Content, Host, Sizing, Window};
use crate::ui::kit::UiState;
use crate::ui::layout::*;

const SHOW: u32 = 1;
const SETTINGS: u32 = 2;
const TAKEOVER: u32 = 3;
const QUIT: u32 = 4;
const UPDATE: u32 = 5;

const BG: u32 = 0xFF23282F;
const HI: u32 = 0xFF2F353D;
const LINE: u32 = 0xFF3A4048;
const FG: u32 = 0xFFE6E9EE;
const CHECK_BG: u32 = 0xFF0E72E6;
const CHECK_HI: u32 = 0xFF1C80F2;

pub struct TrayMenu {
    takeover: bool,
    /// the update item's text, and whether a click starts it (not while it waits or runs)
    update: Option<(String, bool)>,
}

pub type TrayMenuWindow = Window<TrayMenu>;

thread_local! {
    static MENU: RefCell<Option<Rc<RefCell<TrayMenuWindow>>>> = const { RefCell::new(None) };
}

const MENU_FONT: Font = Font { fam: Fam::Menu, size: 12.0, weight: 400 };

impl TrayMenu {
    fn item(ui: &UiState, id: u32, label: &str, checked: Option<bool>) -> El {
        let hot = ui.is_hot(id);
        let check = match checked {
            Some(true) => {
                // the check margin's box, filled with CheckBackground (CheckSelectedBackground under the mouse)
                fill(if hot { CHECK_HI } else { CHECK_BG }, 0.0).size(18.0, 18.0).m(3.0, 0.0, 0.0, 0.0).vcenter().child(text("\u{E73E}", crate::ui::theme::icons(11.0), FG).center())
            }
            _ => panel().size(21.0, 18.0),
        };
        border(hot.then_some(Fill::Solid(HI)), hot.then_some(HI), 1.0, 0.0, Th::default())
            .h(22.0)
            .m(2.0, 0.0, 1.0, 0.0)
            .id(id)
            .child(hstack().vcenter().child(check).child(text(label, MENU_FONT, FG).vcenter().m(11.0, 0.0, 22.0, 0.0)))
    }

    /// "Update to vX": the tray icon's green dot in the check margin; dimmed while it waits for a copy or runs.
    fn update_item(ui: &UiState, label: &str, active: bool) -> El {
        let hot = active && ui.is_hot(UPDATE);
        let dot = panel().size(21.0, 18.0).child(ellipse(8.0, 8.0, crate::ui::theme::UPDATE).center().m(3.0, 0.0, 0.0, 0.0));
        border(hot.then_some(Fill::Solid(HI)), hot.then_some(HI), 1.0, 0.0, Th::default())
            .h(22.0)
            .m(2.0, 0.0, 1.0, 0.0)
            .id(UPDATE)
            .child(hstack().vcenter().child(dot).child(text(label, MENU_FONT, if active { FG } else { 0xFF8A929E }).vcenter().m(11.0, 0.0, 22.0, 0.0)))
            .tip(if active { "Downloads it from GitHub and installs it - your settings stay as they are" } else { "" })
    }

    fn sep() -> El {
        draw((0.0, 0.0), |cv, r| cv.fill(Rect::new(r.x + 25.0, r.y + 3.0, r.w - 25.0, 1.0), LINE)).h(7.0).m(2.0, 0.0, 1.0, 0.0)
    }
}

impl Content for TrayMenu {
    fn build(&mut self, ui: &UiState) -> El {
        let mut list = vstack().m(0.0, 2.0, 0.0, 2.0);
        if let Some((label, active)) = &self.update {
            list.push(Self::update_item(ui, label, *active));
            list.push(Self::sep());
        }
        let list = list
            .child(Self::item(ui, SHOW, "Show Choppy", None))
            .child(Self::item(ui, SETTINGS, "Settings", None))
            .child(Self::sep())
            .child(Self::item(ui, TAKEOVER, "Default file manager", Some(self.takeover)).tip("On: Ctrl+V of files in Explorer uses Choppy. Off: only the right-click \"Paste with Choppy\"."))
            .child(Self::sep())
            .child(Self::item(ui, QUIT, "Quit Choppy", None));
        border(Some(Fill::Solid(BG)), Some(LINE), 1.0, 0.0, Th::default()).min_w(150.0).child(list)
    }

    fn click(&mut self, h: &mut Host, id: u32) {
        h.win.hide();
        let job: Box<dyn FnOnce()> = match id {
            SHOW => Box::new(crate::app::tray_open),
            SETTINGS => Box::new(|| crate::app::show_settings(None)),
            TAKEOVER => Box::new(|| {
                let on = !Settings::current().explorer_takeover;
                let _ = crate::explorer::takeover_set(on);
            }),
            QUIT => Box::new(crate::app::quit_from_tray),
            UPDATE => Box::new(|| crate::update::click(false)),
            _ => return,
        };
        win::post_local(job);
    }

    fn activated(&mut self, h: &mut Host, on: bool) {
        if !on {
            h.win.hide();
        }
    }

    fn key(&mut self, h: &mut Host, vk: u16) -> bool {
        if vk == 0x1B {
            h.win.hide();
            return true;
        }
        false
    }
}

/// The test hook's "shot traymenu": the menu as it would open now, drawn off-screen at 1.5x.
pub fn shot(path: &std::path::Path) -> bool {
    let g = crate::app::gfx();
    let spec = Spec { popup: true, ..Default::default() };
    // the settings' lock is let go before the label takes it again
    let takeover = Settings::current().explorer_takeover;
    let take = TrayMenu { takeover, update: crate::update::label().map(|l| (l, crate::update::phase() == crate::update::Phase::Idle)) };
    let Some(w) = Window::create(&g, &spec, Sizing::ContentHeight(f64::INFINITY), 0.0, take) else { return false };
    let Ok(mut m) = w.try_borrow_mut() else { return false };
    let Window { host, c } = &mut *m;
    let ok = host.shot(c, path, 144.0);
    host.win.destroy();
    ok
}

/// The test hook's "shot trayicon": the tray icon as it shows now (with its dot when an update is out), at `px`
/// (0: the tray's own size), as a PNG.
pub fn shot_icon(path: &std::path::Path, px: i32) -> bool {
    let n = if px > 0 { px } else { win::small_icon_px() };
    let Some(mut p) = win::icon_pixels(n) else { return false };
    if crate::update::available().is_some() {
        win::add_dot(&mut p, n, crate::ui::theme::UPDATE);
    }
    let g = crate::app::gfx();
    let Some(img) = g.bgra_image(&p, n as u32, n as u32, false, n as u32, n as u32) else { return false };
    let Some(mut t) = g.picture_target(n as u32, n as u32, 96.0) else { return false };
    t.begin();
    t.clear(0);
    {
        let mut cv = crate::sys::gfx::Canvas::new(&g, &mut t, 1.0);
        cv.image(&img, Rect::new(0.0, 0.0, n as f64, n as f64), 1.0);
    }
    t.end();
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    g.save_png(&t, path)
}

/// Shows the menu at the mouse (above the taskbar when it would go past the screen), in front so a click elsewhere
/// closes it.
pub fn show() {
    let w = MENU.with(|m| m.borrow().clone());
    let w = match w {
        Some(w) => w,
        None => {
            let g = crate::app::gfx();
            let spec = Spec { popup: true, topmost: true, activate: true, ..Default::default() };
            let Some(w) = Window::create(&g, &spec, Sizing::ContentHeight(f64::INFINITY), 0.0, TrayMenu { takeover: false, update: None }) else { return };
            MENU.with(|m| *m.borrow_mut() = Some(w.clone()));
            w
        }
    };
    let Ok(mut m) = w.try_borrow_mut() else { return };
    let Window { host, c } = &mut *m;
    c.takeover = Settings::current().explorer_takeover;
    c.update = crate::update::label().map(|l| (l, crate::update::phase() == crate::update::Phase::Idle));
    let (cx, cy) = win::cursor_screen();
    let (wa, dpi) = win::work_area_at(cx, cy);
    host.dpi = dpi;
    host.fixed_dpi = Some(dpi);
    let s = dpi as f64 / 96.0;
    let lx = Lx { g: &host.g, scale: s, round: true, ideal: false };
    let mut root = c.build(&host.ui);
    root.measure(&lx, (INF, INF));
    let (pw, ph) = ((root.desired().0 * s).ceil() as i32, (root.desired().1 * s).ceil() as i32);
    let x = if cx + pw > wa.2 { cx - pw } else { cx };
    let y = if cy + ph > wa.3 { cy - ph } else { cy };
    if let Some(l) = host.layered.as_mut() {
        l.pos = (x.max(wa.0), y.max(wa.1));
        l.alpha = 255;
    }
    host.ui.hot = 0;
    host.paint(c);
    host.win.show(true);
    win::foreground(host.win);
}
