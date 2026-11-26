//! SettingsWindow.xaml / .cs: one line per setting (name | what it does / its choices | switch; the choices and
//! switches line up), the "?" card that sets Mbps and MB/s side by side, Benchmark, Logs folder, About.

use std::cell::RefCell;
use std::rc::Rc;

use crate::engine::model::ConflictPolicy;
use crate::engine::settings::{Settings, app_paths};
use crate::sys::gfx::{Canvas, Cap, Image, Rect};
use crate::sys::win::{self, Spec, Win};
use crate::ui::format;
use crate::ui::host::{Content, Host, Place, Sizing, Window};
use crate::ui::kit::*;
use crate::ui::layout::*;
use crate::ui::theme::*;

const CLOSE: u32 = 1;
const TAKEOVER: u32 = 2;
const P_ASK: u32 = 3;
const P_REPLACE: u32 = 4;
const P_NEWER: u32 = 5;
const P_SKIP: u32 = 6;
const P_BOTH: u32 = 7;
const U_AUTO: u32 = 8;
const U_MBPS: u32 = 9;
const U_MBS: u32 = 10;
const UNIT_HELP: u32 = 11;
const VERIFY: u32 = 12;
const CLOSE_VALUE: u32 = 13;
const AUTO_CLOSE: u32 = 14;
const LIMIT_VALUE: u32 = 15;
const LIMIT: u32 = 16;
const BENCH: u32 = 17;
const LOGS: u32 = 18;
const ABOUT: u32 = 19;
const DONE: u32 = 20;

/// The name column (NameCol).
const NAME_COL: f64 = 200.0;
/// Between the numbers and the bars of the "?" card (and the arrow, which spans the bars exactly).
const BAR_GAP: f64 = 16.0;

pub type SettingsWindow = Window<SettingsContent>;

pub struct SettingsContent {
    loading: bool,
    verify: bool,
    policy: ConflictPolicy,
    limit_on: bool,
    limit_value: TextBox,
    limit_unit: String,
    auto_close: bool,
    close_value: TextBox,
    unit: String,
    takeover: bool,
    uc_note: bool,
    icon: Option<Image>,
    unit_help: Option<Rc<RefCell<UnitHelpWindow>>>,
}

impl SettingsContent {
    fn new() -> SettingsContent {
        let mut c = SettingsContent {
            loading: true,
            verify: false,
            policy: ConflictPolicy::Ask,
            limit_on: false,
            limit_value: TextBox::new(6, true),
            limit_unit: "MB/s".into(),
            auto_close: false,
            close_value: TextBox::new(3, true),
            unit: "auto".into(),
            takeover: false,
            uc_note: false,
            icon: None,
            unit_help: None,
        };
        c.reload_with(0);
        c
    }

    /// Shows the settings as they are now (the copy window's quick toggles change them too); the text box with
    /// the keyboard focus keeps what is being typed.
    pub fn reload(&mut self, focused: u32) {
        self.reload_with(focused);
    }

    fn reload_with(&mut self, focused: u32) {
        self.loading = true;
        let s = Settings::current();
        self.verify = s.verify_copies;
        self.policy = match s.policy {
            ConflictPolicy::Replace | ConflictPolicy::ReplaceOlder | ConflictPolicy::Skip | ConflictPolicy::KeepBoth => s.policy,
            _ => ConflictPolicy::Ask,
        };
        self.limit_on = s.speed_limit_mbps > 0.0;
        let mbps = s.speed_unit.as_deref() == Some("mbps");
        if focused != LIMIT_VALUE {
            self.limit_value.set(&format::limit(if s.speed_limit_mbps > 0.0 { s.speed_limit_mbps } else { s.limit_value_mbps }, mbps));
        }
        self.limit_unit = if mbps { "Mbps".into() } else { "MB/s".into() };
        self.auto_close = s.close_when_done;
        if focused != CLOSE_VALUE {
            self.close_value.set(&s.close_after_seconds.to_string());
        }
        self.unit = match s.speed_unit.as_deref() {
            Some("mbps") => "mbps".into(),
            Some("mbs") => "mbs".into(),
            _ => "auto".into(),
        };
        self.takeover = s.explorer_takeover;
        drop(s);
        self.uc_note = crate::explorer::ultracopier_installed();
        self.loading = false;
    }

    fn changed(&mut self) {
        if self.loading {
            return;
        }
        let mut s = Settings::current();
        s.verify_copies = self.verify;
        s.policy = self.policy;
        s.close_when_done = self.auto_close;
        // whole seconds only; 0 closes the window right away
        let t = &self.close_value.text;
        if !t.is_empty()
            && t.bytes().all(|b| b.is_ascii_digit())
            && let Ok(secs) = t.parse::<i32>()
            && secs <= 999
        {
            s.close_after_seconds = secs;
        }
        let unit = self.unit.clone();
        if s.speed_unit.as_deref().unwrap_or("auto") != unit {
            s.speed_unit = Some(unit.clone());
            let mbps = unit == "mbps";
            self.loading = true;
            self.limit_value.set(&format::limit(if s.speed_limit_mbps > 0.0 { s.speed_limit_mbps } else { s.limit_value_mbps }, mbps));
            self.limit_unit = if mbps { "Mbps".into() } else { "MB/s".into() };
            self.loading = false;
        }
        let mbps = s.speed_unit.as_deref() == Some("mbps");
        // the typed limit in Mbps, or 0 when it isn't a number
        let v = format::parse_limit(&self.limit_value.text);
        let v = if v > 0 { format::limit_to_mbps(v as f64, mbps).min(100000.0) } else { 0.0 };
        if v > 0.0 {
            s.limit_value_mbps = v;
        }
        s.speed_limit_mbps = if self.limit_on { s.limit_value_mbps } else { 0.0 };
        s.save();
        drop(s);
        crate::app::manager().apply_live_settings();
    }

    fn takeover_changed(&mut self, h: &mut Host) {
        if self.loading {
            return;
        }
        if let Some(err) = crate::explorer::takeover_set(self.takeover) {
            crate::ui::dialogs::show(Some(h.win), "Couldn't change the Explorer setting", &err, "OK", None, |_| {});
            self.loading = true;
            self.takeover = Settings::current().explorer_takeover;
            self.loading = false;
        }
    }

    /// Row: a Border with BorderThickness 0,0,0,1 (#2E343C, the separator under it), Padding 0,13,0,13 unless given.
    fn row(child: El, pad: Th) -> El {
        border(None, Some(SEP), 0.0, 0.0, pad).sides(Th::new(0.0, 0.0, 0.0, 1.0)).child(child).named("Row")
    }

    pub fn set_for_test(&mut self, h: &mut Host, key: &str, value: &str) {
        match key {
            "verify" => {
                self.verify = value == "1";
                self.changed();
            }
            "policy" => {
                self.policy = match value {
                    "replace" => ConflictPolicy::Replace,
                    "skip" => ConflictPolicy::Skip,
                    "keepboth" => ConflictPolicy::KeepBoth,
                    "newer" => ConflictPolicy::ReplaceOlder,
                    _ => ConflictPolicy::Ask,
                };
                self.changed();
            }
            "limit" => {
                if value == "0" {
                    self.limit_on = false;
                } else {
                    self.limit_value.set(value);
                    self.changed();
                    self.limit_on = true;
                }
                self.changed();
            }
            "autoclose" => {
                self.auto_close = value == "1";
                self.changed();
            }
            "closeafter" => {
                self.close_value.set(value);
                self.changed();
            }
            "unithelp" => {
                if (value == "1") != self.unit_help_open() {
                    self.toggle_unit_help(h);
                }
            }
            "unit" => {
                self.unit = match value {
                    "mbps" => "mbps",
                    "mbs" => "mbs",
                    _ => "auto",
                }
                .into();
                self.changed();
            }
            _ => {}
        }
        h.invalidate();
    }

    // ---------- the "?" card ----------

    fn unit_help_open(&self) -> bool {
        self.unit_help.as_ref().is_some_and(|p| p.try_borrow().map(|p| p.host.win.is_visible()).unwrap_or(true))
    }

    fn close_unit_help(&self) {
        if let Some(p) = &self.unit_help
            && let Ok(p) = p.try_borrow()
        {
            p.host.win.hide();
        }
    }

    fn toggle_unit_help(&mut self, h: &mut Host) {
        if self.unit_help_open() {
            self.close_unit_help();
            return;
        }
        let Some(b) = h.rect_of(UNIT_HELP) else { return };
        if self.unit_help.is_none() {
            let spec = Spec { owner: Some(h.win), popup: true, ..Default::default() };
            self.unit_help = Window::create(&h.g, &spec, Sizing::ContentHeight(412.0), 0.0, UnitHelpContent);
        }
        let Some(p) = &self.unit_help else { return };
        let Ok(mut p) = p.try_borrow_mut() else { return };
        let Window { host, c } = &mut *p;
        host.dpi = h.dpi;
        host.fixed_dpi = h.fixed_dpi;
        let s = h.scale();
        let (pw, _) = host.measure(c);
        let (l, t, _, _) = h.win.rect();
        // Placement="Left" HorizontalOffset="-10" VerticalOffset="-14": the card's right edge (its room for the
        // shadow included) at the button's left, 10 further left, 14 higher
        let x = l + (b.x * s).round() as i32 - pw - (10.0 * s).round() as i32;
        let y = t + (b.y * s).round() as i32 - (14.0 * s).round() as i32;
        if let Some(lay) = host.layered.as_mut() {
            lay.pos = (x, y);
            lay.alpha = 255;
        }
        host.paint(c);
        host.win.show(false);
    }

    /// The test hook's "shot unithelp": the card alone (no shadow) at 1.5x on #1F242B with 14 DIPs around it.
    pub fn shot_unit_help(&self, h: &Host, path: &std::path::Path) -> bool {
        let g = h.g.clone();
        let s = h.scale();
        let lx = Lx { g: &g, scale: s, round: true, ideal: false };
        let mut card = UnitHelpContent::card();
        card.measure(&lx, (400.0, INF));
        let (cw, ch) = card.desired();
        let pad = 14.0;
        card.arrange(&lx, Rect::new(pad, pad, cw, ch), (pad, pad));
        let k = 1.5;
        let (w, hh) = (cw + 2.0 * pad, ch + 2.0 * pad);
        let Some(mut t) = g.picture_target((w * k).ceil() as u32, (hh * k).ceil() as u32, 96.0 * k) else { return false };
        t.begin();
        {
            let mut cv = Canvas::new(&g, &mut t, k);
            cv.fill(Rect::new(0.0, 0.0, w, hh), 0xFF1F242B);
            card.draw(&mut cv);
        }
        t.end();
        if let Some(d) = path.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        g.save_png(&t, path)
    }
}

impl Content for SettingsContent {
    fn build(&mut self, ui: &UiState) -> El {
        let head = |t: &str| text(t, sf(15.0, SEMI), TEXT);
        let sub = |t: &str| text(t, sf(13.0, NORMAL), FAINT).m(0.0, 2.0, 0.0, 0.0);
        let mut sp = vstack().m(22.0, 0.0, 22.0, 16.0);
        // default file manager (the administrator copy never touches Explorer, the Run key or Ultracopier)
        if !crate::app::elevated() {
            sp.push(
                Self::row(
                    vstack()
                        .child(
                            toggle(
                                ui,
                                TAKEOVER,
                                self.takeover,
                                Some(
                                    grid(&[Len::Px(NAME_COL), Len::Star(1.0)], &[])
                                        .child(head("Default file manager").vcenter())
                                        .child(sub("Ctrl+V in Explorer opens Choppy").vcenter().m(0.0, 0.0, 0.0, 0.0).col(1)),
                                ),
                            )
                            .tip("Off: Windows' own copy stays the default; use right-click \"Paste with Choppy\". Drag and drop always stays Windows' own copy."),
                        )
                        .child(
                            text("Another copier that takes Ctrl+V is closed while this is on (off brings it back).", sf(13.0, NORMAL), WARN)
                                .m(0.0, 6.0, 0.0, 0.0)
                                .shown(self.uc_note),
                        ),
                    Th::new(0.0, 4.0, 0.0, 12.0),
                )
                .named("TakeoverRow"),
            );
        }
        // conflicts
        let p = self.policy;
        sp.push(Self::row(
            vstack().child(head("When a file already exists")).child(
                wrap()
                    .m(0.0, 10.0, 0.0, -6.0)
                    .child(seg(ui, P_ASK, p == ConflictPolicy::Ask, "Ask me", ""))
                    .child(seg(ui, P_REPLACE, p == ConflictPolicy::Replace, "Replace", ""))
                    .child(seg(ui, P_NEWER, p == ConflictPolicy::ReplaceOlder, "Replace if newer", ""))
                    .child(seg(ui, P_SKIP, p == ConflictPolicy::Skip, "Skip", ""))
                    .child(seg(ui, P_BOTH, p == ConflictPolicy::KeepBoth, "Keep both", "")),
            ),
            Th::new(0.0, 13.0, 0.0, 13.0),
        ));
        // unit
        let help_hot = ui.is_hot(UNIT_HELP);
        let help = border(
            Some(Fill::Solid(if help_hot { 0xFF15324F } else { 0xFF262B32 })),
            Some(if help_hot { 0xFF2A79C8 } else { 0xFF4A515C }),
            1.0,
            14.0,
            Th::default(),
        )
        .size(28.0, 28.0)
        .ha(Al::Center)
        .child(text("?", sf(14.0, SEMI), ACCENT_TEXT).center().m(0.0, 0.0, 0.0, 1.0))
        .id(UNIT_HELP)
        .hand()
        .focusable(true)
        .tip("Mbps and MB/s side by side");
        let u = self.unit.as_str();
        sp.push(Self::row(
            grid(&[Len::Px(NAME_COL), Len::Star(1.0), Len::Auto], &[])
                .child(head("Show speed in").vcenter())
                .child(
                    hstack()
                        .col(1)
                        .child(seg(ui, U_AUTO, u == "auto", "Auto", "Picks by the copy: Mbps for a network share, MB/s for hard disks, SSDs and USB drives").m(0.0, 0.0, 6.0, 0.0))
                        .child(seg(ui, U_MBPS, u == "mbps", "Mbps", "Megabits per second - the unit internet and network speeds are given in").m(0.0, 0.0, 6.0, 0.0))
                        .child(seg(ui, U_MBS, u == "mbs", "MB/s", "Megabytes per second - the unit file sizes and disk speeds are given in").m(0.0, 0.0, 0.0, 0.0)),
                )
                .child(grid(&[], &[]).col(2).w(44.0).vcenter().child(help)),
            Th::new(0.0, 10.0, 0.0, 10.0),
        ));
        // verify
        sp.push(Self::row(
            toggle(
                ui,
                VERIFY,
                self.verify,
                Some(grid(&[Len::Px(NAME_COL), Len::Star(1.0)], &[]).child(head("Check every copy").vcenter()).child(sub("xxHash64").vcenter().m(0.0, 0.0, 0.0, 0.0).col(1))),
            )
            .tip("Reads each copy back and compares it with the original. A move then removes the original only after its copy matches."),
            Th::new(0.0, 13.0, 0.0, 13.0),
        ));
        // close at completion
        sp.push(Self::row(
            grid(&[Len::Px(NAME_COL), Len::Auto, Len::Auto, Len::Star(1.0), Len::Auto], &[])
                .child(head("Close at completion").vcenter().tip("Closes the copy window this many seconds after a copy finishes with no problems (0: right away). A copy with problems stays open."))
                .child(self.close_value.el(ui, CLOSE_VALUE, &BOX_SMALL, self.auto_close).size(60.0, 30.0).col(1))
                .child(text("s", sf(14.0, NORMAL), DIM).vcenter().m(10.0, 0.0, 0.0, 0.0).col(2))
                .child(toggle(ui, AUTO_CLOSE, self.auto_close, None).vcenter().col(4)),
            Th::new(0.0, 10.0, 0.0, 10.0),
        ));
        // speed limit
        sp.push(Self::row(
            grid(&[Len::Px(NAME_COL), Len::Auto, Len::Auto, Len::Star(1.0), Len::Auto], &[])
                .child(head("Speed limit").vcenter().tip("Leaves room on the line for everything else"))
                .child(self.limit_value.el(ui, LIMIT_VALUE, &BOX_SMALL, self.limit_on).size(60.0, 30.0).col(1))
                .child(text(&self.limit_unit, sf(14.0, NORMAL), DIM).vcenter().m(10.0, 0.0, 0.0, 0.0).col(2))
                .child(toggle(ui, LIMIT, self.limit_on, None).vcenter().col(4)),
            Th::new(0.0, 10.0, 0.0, 10.0),
        ));
        // benchmark
        sp.push(Self::row(
            grid(&[Len::Px(NAME_COL), Len::Star(1.0), Len::Auto], &[])
                .child(head("Benchmark").vcenter())
                .child(sub("Choppy VS Windows").vcenter().m(0.0, 0.0, 0.0, 0.0).trimmed().col(1))
                .child(
                    button(ui, BENCH, &BTN, Th::hv(14.0, 0.0), true, |fg| text("Run...", sf(14.0, SEMI), fg))
                        .h(30.0)
                        .col(2)
                        .tip("Choppy and Windows copy the same test files on every drive of this PC, then a report shows which one takes less time where"),
                ),
            Th::new(0.0, 10.0, 0.0, 10.0),
        ));
        let link = |id: u32, t: &str| link_button(ui, id, text(t, sf(14.0, NORMAL), LINK).tpad(Th::new(0.0, 0.0, 0.0, 2.0)));
        sp.push(
            grid(&[Len::Px(NAME_COL), Len::Star(1.0), Len::Auto], &[])
                .m(0.0, 14.0, 0.0, 0.0)
                .child(link(LOGS, "Logs folder").m(-6.0, 0.0, 0.0, 0.0).left().vcenter())
                .child(
                    hstack()
                        .col(1)
                        .vcenter()
                        .m(-6.0, 0.0, 0.0, 0.0)
                        .child(link(ABOUT, "About"))
                        .child(text(format!("v{}", env!("CHOPPY_VERSION")), sf(14.0, NORMAL), FAINT).vcenter().m(8.0, 0.0, 0.0, 0.0)),
                )
                .child(button(ui, DONE, &BTN_PRIMARY, Th::hv(22.0, 0.0), true, |fg| text("Done", sf(14.0, SEMI), fg)).h(38.0).min_w(74.0).col(2)),
        );

        let title = grid(&[], &[])
            .child(
                hstack()
                    .vcenter()
                    .m(18.0, 0.0, 0.0, 0.0)
                    .child(image(self.icon.clone()).size(20.0, 20.0))
                    .child(text("Settings", sf(15.0, SEMI), 0xFFE9ECF0).m(10.0, 0.0, 0.0, 1.0).vcenter()),
            )
            .child(title_button(ui, CLOSE, true, G_CLOSE, 12.0, 46.0).right());
        let root = grid(&[], &[Len::Px(46.0), Len::Auto]).child(title).child(sp.row(1));
        border(Some(window_bg()), Some(WIN_BORDER), 1.0, 0.0, Th::default()).child(root)
    }

    fn click(&mut self, h: &mut Host, id: u32) {
        match id {
            CLOSE | DONE => h.close(),
            TAKEOVER => {
                self.takeover = !self.takeover;
                self.takeover_changed(h);
            }
            P_ASK | P_REPLACE | P_NEWER | P_SKIP | P_BOTH => {
                self.policy = match id {
                    P_REPLACE => ConflictPolicy::Replace,
                    P_NEWER => ConflictPolicy::ReplaceOlder,
                    P_SKIP => ConflictPolicy::Skip,
                    P_BOTH => ConflictPolicy::KeepBoth,
                    _ => ConflictPolicy::Ask,
                };
                self.changed();
            }
            U_AUTO | U_MBPS | U_MBS => {
                self.unit = match id {
                    U_MBPS => "mbps",
                    U_MBS => "mbs",
                    _ => "auto",
                }
                .into();
                self.changed();
            }
            UNIT_HELP => self.toggle_unit_help(h),
            VERIFY => {
                self.verify = !self.verify;
                self.changed();
            }
            AUTO_CLOSE => {
                self.auto_close = !self.auto_close;
                self.changed();
            }
            LIMIT => {
                self.limit_on = !self.limit_on;
                self.changed();
            }
            BENCH => crate::app::show_benchmark(),
            LOGS => {
                let d = app_paths::log_dir();
                let _ = std::fs::create_dir_all(&d);
                win::shell_open("explorer.exe", Some(&format!("\"{}\"", d.display())));
            }
            ABOUT => crate::ui::dialogs::about(Some(h.win)),
            _ => {}
        }
        h.invalidate();
    }

    fn key(&mut self, h: &mut Host, vk: u16) -> bool {
        if vk == 0x1B {
            h.close();
            return true;
        }
        false
    }

    fn text_box(&mut self, id: u32) -> Option<(&mut TextBox, BoxStyle)> {
        match id {
            LIMIT_VALUE => Some((&mut self.limit_value, BOX_SMALL)),
            CLOSE_VALUE => Some((&mut self.close_value, BOX_SMALL)),
            _ => None,
        }
    }

    fn text_changed(&mut self, _h: &mut Host, _id: u32) {
        if !self.loading {
            self.changed();
        }
    }

    fn activated(&mut self, _h: &mut Host, on: bool) {
        if !on {
            self.close_unit_help();
        }
    }

    fn moved(&mut self, _h: &mut Host) {
        self.close_unit_help();
    }

    fn closed(&mut self, _h: &mut Host) {
        if let Some(p) = self.unit_help.take()
            && let Ok(p) = p.try_borrow()
        {
            p.host.win.destroy();
        }
        crate::app::settings_closed();
    }
}


// ---------- the "?" card ----------

pub struct UnitHelpContent;
pub type UnitHelpWindow = Window<UnitHelpContent>;

impl UnitHelpContent {
    /// The card: header (Mbps above its column, an arrow as long as the bars, MB/s above its column), a line, the
    /// rows: 10 / 100 / 1,000 / 10,000 Mbps in MB/s, bars on a log scale (each step ten times the last). The two
    /// number columns share one width (SharedSizeGroup "Side"), so the bars sit in the middle and both sides mirror.
    fn card() -> El {
        let mbps = [10, 100, 1000, 10000];
        let what = ["Slow Upload", "Home Internet", "Gigabit Network", "10-Gigabit Network"];
        let mut g = grid(&[Len::Shared(0), Len::Star(1.0), Len::Shared(0)], &[Len::Auto; 6]);
        g.push(text("Mbps", sf(15.0, SEMI), ACCENT_TEXT).right().m(0.0, 0.0, 0.0, 2.0));
        let arrow = grid(&[], &[])
            .m(BAR_GAP, 1.0, BAR_GAP, 2.0)
            .vcenter()
            .col(1)
            .child(fill(FAINT, 1.0).h(2.0).m(0.0, 0.0, 6.0, 0.0).vcenter())
            .child(
                draw((9.0, 10.0), |cv, r| {
                    if let Some(p) = cv.path(|pb| {
                        pb.begin(r.x, r.y, true);
                        pb.line(r.x + 9.0, r.y + 5.0);
                        pb.line(r.x, r.y + 10.0);
                        pb.end(true);
                    }) {
                        cv.fill_path(&p, FAINT);
                    }
                })
                .right()
                .vcenter(),
            );
        g.push(arrow);
        g.push(text("MB/s", sf(15.0, SEMI), AVG_SPEED).right().col(2).m(0.0, 0.0, 0.0, 2.0));
        g.push(text("8 bits make a byte: divide Mbps by about 8.4 for MB/s", sf(12.5, NORMAL), DIM).ha(Al::Center).m(0.0, 4.0, 0.0, 14.0).row(1).col_span(3));
        for i in 0..mbps.len() {
            let mbs = mbps[i] as f64 * 1e6 / 8.0 / (1024.0 * 1024.0);
            let bottom = if i < mbps.len() - 1 { 10.0 } else { 0.0 };
            let left = runs(vec![run(format::count_inv(mbps[i]), 13.5, SEMI, ACCENT_TEXT), run(" Mbps", 13.5, NORMAL, DIM)]).top().right().m(0.0, 0.0, 0.0, bottom);
            g.push(left.row(i + 2));
            let k = i;
            let n = mbps.len();
            let track = draw((0.0, 0.0), move |cv, r| {
                cv.fill_round(r, 4.0, TRACK);
                let w = (r.w * (k + 1) as f64 / n as f64 * cv.scale).round_ties_even() / cv.scale;
                cv.fill_gradient(Rect::new(r.x, r.y, w, r.h), 4.0, &[(0.0, 0xFF1A7FE6), (1.0, 0xFF5AC8FA)], (0.0, 0.0), (1.0, 0.0));
            })
            .h(8.0)
            .vcenter();
            let mid = vstack()
                .m(BAR_GAP, 0.0, BAR_GAP, bottom)
                .cell(i + 2, 1)
                .child(grid(&[], &[]).h(18.0).child(track))
                .child(text(what[i], sf(11.0, NORMAL), FAINT).m(0.0, 1.0, 0.0, 0.0));
            g.push(mid);
            let val = if mbs >= 100.0 {
                format::count_inv(mbs.round() as i64)
            } else if mbs >= 10.0 {
                crate::engine::netfmt::f(mbs, 1)
            } else {
                crate::engine::netfmt::f(mbs, 2)
            };
            let right = runs(vec![run(val, 13.5, SEMI, AVG_SPEED), run(" MB/s", 13.5, NORMAL, DIM)]).top().right().m(0.0, 0.0, 0.0, bottom);
            g.push(right.cell(i + 2, 2));
        }
        border(Some(Fill::Solid(0xFF1E2228)), Some(0xFF3A414B), 1.0, 10.0, Th::new(20.0, 14.0, 20.0, 14.0)).w(400.0).child(g).named("UnitHelpCard")
    }
}

impl Content for UnitHelpContent {
    fn build(&mut self, _ui: &UiState) -> El {
        // Margin 0,0,12,12 leaves room for the DropShadowEffect (BlurRadius 16, ShadowDepth 3, Opacity 0.5)
        let card = Self::card();
        panel().child(draw((0.0, 0.0), |cv, r| cv.shadow(Rect::new(r.x, r.y, r.w - 12.0, r.h - 12.0), 10.0, 16.0, 3.0, 0.5))).child(card.m(0.0, 0.0, 12.0, 12.0))
    }
}

/// Makes the settings window and shows it.
pub fn create(owner: Option<Win>) -> Option<Rc<RefCell<SettingsWindow>>> {
    let g = crate::app::gfx();
    let owner = owner.filter(|o| o.is_visible());
    let park = crate::testhook::park();
    let spec = Spec { title: "Choppy settings".into(), owner, taskbar: !park, ..Default::default() };
    let w = Window::create(&g, &spec, Sizing::ContentHeight(540.0), 46.0, SettingsContent::new())?;
    {
        let mut m = w.borrow_mut();
        let Window { host, c } = &mut *m;
        c.icon = host.g.icon_image(crate::ui::ICO, (20.0 * crate::ui::icon_scale()).round() as u32);
        let at = if park {
            Place::At(-32000, 0)
        } else {
            match owner {
                Some(o) => Place::Owner(o),
                None => Place::Screen,
            }
        };
        host.place(c, at);
        host.paint(c);
        host.win.show_drawn(!park);
    }
    Some(w)
}

#[allow(dead_code)]
fn _unused(_: Cap) {}
