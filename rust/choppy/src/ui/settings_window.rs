//! SettingsWindow.xaml / .cs: one line per setting (name | what it does / its choices | switch; the choices and
//! switches line up), the "?" card that sets Mbps and MB/s side by side, Check for updates and the update label in
//! the title row (2.02), Benchmark, Logs folder, About.

use std::cell::RefCell;
use std::rc::Rc;

use crate::engine::model::ConflictPolicy;
use crate::engine::settings::{MAX_UPDATE_DAYS, MAX_UPDATE_SECONDS, Settings, UpdateWhen, app_paths};
use crate::sys::gfx::{Canvas, Cap, Image, Rect};
use crate::sys::win::{self, Spec, Win};
use crate::ui::format;
use crate::ui::host::{Content, Host, Place, Sizing, T_USER, Window};
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
const UPD_CHECK: u32 = 21;
const UPD_START: u32 = 22;
const UPD_AFTER: u32 = 23;
const UPD_EVERY: u32 = 24;
const UPD_AFTER_VALUE: u32 = 25;
const UPD_EVERY_VALUE: u32 = 26;
const UPD_LABEL: u32 = 27;

/// The update label's glow: a frame about every 33 ms (WM_TIMER rounds it to the 15.6 ms tick: ~32 a second).
const T_GLOW: usize = T_USER + 1;
const GLOW_MS: u32 = 33;
/// One full glow every 2 s, between 30 % and 100 %.
const GLOW_PERIOD: f64 = 2.0;

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
    update_check: bool,
    update_when: UpdateWhen,
    after_value: TextBox,
    every_value: TextBox,
    glowing: bool,
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
            update_check: true,
            update_when: UpdateWhen::Startup,
            after_value: TextBox::new(4, true),
            every_value: TextBox::new(3, true),
            glowing: false,
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
        self.update_check = s.update_check;
        self.update_when = s.update_when;
        if focused != UPD_AFTER_VALUE {
            self.after_value.set(&s.update_after_seconds.to_string());
        }
        if focused != UPD_EVERY_VALUE {
            self.every_value.set(&s.update_every_days.to_string());
        }
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
        // Check for updates: whole seconds / days in their range; anything else keeps what was saved
        let updates = (s.update_check, s.update_when, s.update_after_seconds, s.update_every_days);
        s.update_check = self.update_check;
        s.update_when = self.update_when;
        if let Some(v) = whole(&self.after_value.text, MAX_UPDATE_SECONDS) {
            s.update_after_seconds = v;
        }
        if let Some(v) = whole(&self.every_value.text, MAX_UPDATE_DAYS) {
            s.update_every_days = v;
        }
        let updates_changed = updates != (s.update_check, s.update_when, s.update_after_seconds, s.update_every_days);
        s.save();
        drop(s);
        crate::app::manager().apply_live_settings();
        if updates_changed {
            crate::update::settings_changed();
        }
    }

    /// The update label in the title row, centered between "Settings" and the close button: "Update to vX" (a click
    /// updates), "... after the copy", "Updating to vX...", or "Updated to vX" for a moment after an update. Its
    /// green glow breathes from 30 % to 100 % and back every 2 s while there is something to click.
    fn update_label(&self, ui: &UiState) -> Option<El> {
        let (label, active, k, fade) = if let Some(l) = crate::update::label() {
            let t = crate::started().elapsed().as_secs_f64() % GLOW_PERIOD / GLOW_PERIOD;
            let k = 0.3 + 0.7 * (0.5 - 0.5 * (2.0 * std::f64::consts::PI * t).cos());
            (l, crate::update::phase() == crate::update::Phase::Idle, k, 1.0)
        } else if let Some((tag, age)) = crate::update::just_updated() {
            // steady, and gone in its last second
            let left = (crate::update::UPDATED_SHOWN.as_secs_f64() - age.as_secs_f64()).clamp(0.0, 1.0);
            (format!("Updated to {tag}"), false, 0.55, left)
        } else {
            return None;
        };
        let hot = active && ui.is_hot(UPD_LABEL);
        let pressed = active && ui.is_pressed(UPD_LABEL);
        let bg = if pressed { 0xFF142B20 } else if hot { 0xFF1D4030 } else { 0xFF183426 };
        let edge = ((0.35 + 0.45 * k) * 255.0) as u32;
        let bd = if hot { UPDATE } else { edge << 24 | (UPDATE & 0x00FF_FFFF) };
        let mark = if active || crate::update::phase() != crate::update::Phase::Idle {
            ellipse(7.0, 7.0, UPDATE).vcenter().m(0.0, 1.0, 8.0, 0.0)
        } else {
            text(G_CHECK, icons(11.0), UPDATE).vcenter().m(0.0, 1.0, 7.0, 0.0)
        };
        let body = border(Some(Fill::Solid(bg)), Some(bd), 1.0, 13.0, Th::new(12.0, 0.0, 14.0, 0.0))
            .h(26.0)
            .child(hstack().vcenter().child(mark).child(text(label, sf(13.0, SEMI), UPDATE_TEXT).vcenter().m(0.0, 0.0, 0.0, 1.0)));
        let glow = draw((0.0, 0.0), move |cv, r| cv.glow(r, 13.0, 18.0, UPDATE_GLOW, k));
        let mut el = panel().center().child(glow).child(body).opacity(fade);
        if active {
            el = el.id(UPD_LABEL).hand().tip("Downloads it from GitHub and installs it - your settings stay as they are");
        }
        Some(el)
    }

    /// Seg with a number box inside: "[30] seconds after startup", "Every [7] days".
    fn seg_box(&self, ui: &UiState, id: u32, on: bool, enabled: bool, before: &str, value: (u32, &TextBox), after: &str, tip: &str) -> El {
        let hot = enabled && ui.is_hot(id);
        let (bg, mut bd, fg, w) = if on { (ACCENT, 0xFF2A86EE, WHITE, SEMI) } else { (0xFF262B32, 0xFF383E47, TEXT2, NORMAL) };
        if hot && !on {
            bd = 0xFF59616C;
        }
        let mut row = hstack().vcenter();
        if !before.is_empty() {
            row.push(text(before, sf(13.5, w), fg).vcenter().m(4.0, 0.0, 8.0, 0.0));
        }
        row.push(value.1.el(ui, value.0, &BOX_SMALL, enabled).size(44.0, 24.0));
        row.push(text(after, sf(13.5, w), fg).vcenter().m(8.0, 0.0, 0.0, 0.0));
        border(Some(Fill::Solid(bg)), Some(bd), 1.0, 6.0, Th::new(if before.is_empty() { 4.0 } else { 8.0 }, 3.0, 12.0, 3.0))
            .child(row)
            .id(id)
            .hand()
            .focusable(true)
            .tip(tip)
            .m(0.0, 0.0, 6.0, 6.0)
    }

    fn update_choice(&mut self, w: UpdateWhen) {
        if self.update_check {
            self.update_when = w;
            self.changed();
        }
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
            "updcheck" => {
                self.update_check = value == "1";
                self.changed();
            }
            "updwhen" => self.update_choice(match value {
                "after" => UpdateWhen::After,
                "every" => UpdateWhen::Every,
                _ => UpdateWhen::Startup,
            }),
            "updafter" | "upddays" => {
                let (id, tb) = if key == "updafter" { (UPD_AFTER_VALUE, &mut self.after_value) } else { (UPD_EVERY_VALUE, &mut self.every_value) };
                tb.set(value);
                self.text_changed(h, id);
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
        // check for updates (the administrator copy never looks)
        if !crate::app::elevated() {
            let (on, w) = (self.update_check, self.update_when);
            sp.push(Self::row(
                vstack()
                    .child(
                        toggle(ui, UPD_CHECK, on, Some(head("Check for updates").vcenter()))
                            .tip("Looks on GitHub for a newer Choppy - it only asks which version is the latest. Updating is always your click: the tray icon's green dot, or the label at the top here."),
                    )
                    .child(
                        wrap()
                            .m(0.0, 10.0, 0.0, -6.0)
                            .opacity(if on { 1.0 } else { 0.45 })
                            .child(seg(ui, UPD_START, w == UpdateWhen::Startup, "At startup", "When Choppy starts (with Windows, when Default file manager is on)"))
                            .child(self.seg_box(ui, UPD_AFTER, w == UpdateWhen::After, on, "", (UPD_AFTER_VALUE, &self.after_value), "seconds after startup", "Waits this long after Choppy starts (1 to 3600 seconds)"))
                            .child(self.seg_box(ui, UPD_EVERY, w == UpdateWhen::Every, on, "Every", (UPD_EVERY_VALUE, &self.every_value), "days", "Once in this many days (1 to 365), whenever Choppy runs")),
                    ),
                Th::new(0.0, 13.0, 0.0, 13.0),
            ));
        }
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

        let mut title = grid(&[Len::Auto, Len::Star(1.0), Len::Auto], &[])
            .child(
                hstack()
                    .vcenter()
                    .m(18.0, 0.0, 0.0, 0.0)
                    .child(image(self.icon.clone()).size(20.0, 20.0))
                    .child(text("Settings", sf(15.0, SEMI), 0xFFE9ECF0).m(10.0, 0.0, 0.0, 1.0).vcenter()),
            )
            .child(title_button(ui, CLOSE, true, G_CLOSE, 12.0, 46.0).col(2));
        if let Some(l) = self.update_label(ui) {
            title.push(l.col(1));
        }
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
            UPD_CHECK => {
                self.update_check = !self.update_check;
                self.changed();
            }
            UPD_START => self.update_choice(UpdateWhen::Startup),
            UPD_AFTER => self.update_choice(UpdateWhen::After),
            UPD_EVERY => self.update_choice(UpdateWhen::Every),
            UPD_LABEL => crate::update::click(true),
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
            UPD_AFTER_VALUE if self.update_check => Some((&mut self.after_value, BOX_SMALL)),
            UPD_EVERY_VALUE if self.update_check => Some((&mut self.every_value, BOX_SMALL)),
            _ => None,
        }
    }

    fn text_changed(&mut self, _h: &mut Host, id: u32) {
        if self.loading {
            return;
        }
        // typing a number picks its choice
        match id {
            UPD_AFTER_VALUE => self.update_when = UpdateWhen::After,
            UPD_EVERY_VALUE => self.update_when = UpdateWhen::Every,
            _ => {}
        }
        self.changed();
    }

    fn focus_lost(&mut self, h: &mut Host, id: u32) {
        // a box left empty or out of range shows what is saved again
        if matches!(id, UPD_AFTER_VALUE | UPD_EVERY_VALUE) {
            self.reload_with(0);
            h.invalidate();
        }
    }

    fn timer(&mut self, h: &mut Host, id: usize) {
        if id == T_GLOW {
            h.invalidate();
        }
    }

    /// The glow's frames run while the update label shows (and only then).
    fn after_paint(&mut self, h: &Host) {
        let want = crate::update::label().is_some() || crate::update::just_updated().is_some();
        if want != self.glowing {
            self.glowing = want;
            if want {
                h.win.set_timer(T_GLOW, GLOW_MS);
            } else {
                h.win.kill_timer(T_GLOW);
            }
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

/// A whole number from 1 to `max`, as typed.
fn whole(t: &str, max: i32) -> Option<i32> {
    if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    t.parse::<i32>().ok().filter(|v| (1..=max).contains(v))
}

#[allow(dead_code)]
fn _unused(_: Cap) {}
