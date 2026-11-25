//! ConflictWindow.xaml / .cs: a file already exists. Two cards (incoming / already there) with Explorer's
//! thumbnails; a click on a card picks the file that stays (incoming = Replace, already there = Skip) and the main
//! button says what that does; Compare checks the content (xxHash64); "Apply to all remaining conflicts".

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::mpsc::Sender;
use std::time::Instant;

use crate::engine::model::{ConflictAnswer, ConflictPolicy, OpKind};
use crate::engine::paths;
use crate::sys::LocalTime;
use crate::sys::gfx::Image;
use crate::sys::win::{self, Spec, Win};
use crate::ui::format::{self, When};
use crate::ui::host::{Content, Host, Place, Sizing, Window};
use crate::ui::kit::*;
use crate::ui::layout::*;
use crate::ui::theme::*;
use crate::wstr::WString;

const CLOSE: u32 = 1;
const IN_CARD: u32 = 2;
const EX_CARD: u32 = 3;
const APPLY_ALL: u32 = 4;
const COMPARE: u32 = 5;
const SKIP: u32 = 6;
const BOTH: u32 = 7;
const REPLACE: u32 = 8;

const PICK_BORDER: u32 = 0xFF2D6FB4;
const PICK_BACK: u32 = 0xFF172230;
const PICK_HEAD: u32 = 0xFF4FA8F0;
const PICK_BOX: u32 = 0xFF1E2833;
const PICK_EXT: u32 = 0xFF4C5A6B;
const OFF_BORDER: u32 = 0xFF2A2F37;
const OFF_BACK: u32 = 0xFF191C21;
const OFF_BOX: u32 = 0xFF22272E;
const OFF_EXT: u32 = 0xFF4C535C;
const HOVER_BORDER: u32 = 0xFF3E4650;

/// What the window shows of the engine's ConflictInfo.
pub struct Incoming {
    pub incoming_path: String,
    pub existing_path: String,
    pub incoming_size: i64,
    pub existing_size: i64,
    pub incoming_time: Option<LocalTime>,
    pub existing_time: Option<LocalTime>,
    pub remaining: i32,
    pub op: OpKind,
}

pub type ConflictWindow = Window<ConflictContent>;

pub struct ConflictContent {
    info: Incoming,
    keep_existing: bool,
    apply_all: bool,
    in_tag: Option<String>,
    ex_tag: Option<String>,
    in_img: Option<Image>,
    ex_img: Option<Image>,
    icon: Option<Image>,
    compare: Option<(String, Option<bool>, String)>,
    compare_enabled: bool,
    answer: Option<ConflictAnswer>,
    tx: Option<Sender<ConflictAnswer>>,
    closed: Arc<AtomicBool>,
    disabled: Vec<Win>,
    last_report: Option<Instant>,
}

fn when(t: &Option<LocalTime>) -> Option<When> {
    t.map(|t| When { year: t.year as i32, month: t.month as u32, day: t.day as u32, hour: t.hour as u32, minute: t.minute as u32 })
}

/// Seconds for comparing two times (DateTime.MinValue = the earliest).
fn secs(t: &Option<LocalTime>) -> f64 {
    match t {
        None => f64::MIN,
        Some(t) => {
            let days = (t.year as f64) * 372.0 + (t.month as f64) * 31.0 + t.day as f64;
            days * 86400.0 + t.hour as f64 * 3600.0 + t.minute as f64 * 60.0 + t.second as f64 + t.millisecond as f64 / 1000.0
        }
    }
}

impl ConflictContent {
    fn new(info: Incoming, tx: Sender<ConflictAnswer>) -> ConflictContent {
        let mut c = ConflictContent {
            in_tag: None,
            ex_tag: None,
            keep_existing: false,
            apply_all: false,
            in_img: None,
            ex_img: None,
            icon: None,
            compare: None,
            compare_enabled: true,
            answer: None,
            tx: Some(tx),
            closed: Arc::new(AtomicBool::new(false)),
            disabled: Vec::new(),
            last_report: None,
            info,
        };
        // little hints: which one is newer / bigger
        let (a, b) = (secs(&c.info.incoming_time), secs(&c.info.existing_time));
        if a > b + 2.0 {
            c.in_tag = Some("Newer".into());
        } else if b > a + 2.0 {
            c.ex_tag = Some("Newer".into());
        } else if c.info.incoming_size == c.info.existing_size {
            c.in_tag = Some("Same size + date".into());
        }
        c
    }

    /// Picks the file that stays: its card lights up and the main button says what that does - Replace (incoming)
    /// or Skip (already there). The other of the two moves to the button beside Compare.
    fn pick(&mut self, h: &mut Host, existing: bool) {
        self.keep_existing = existing;
        let f = h.ui.focus;
        if f != SKIP && f != BOTH && f != COMPARE {
            h.ui.focus = REPLACE;
        }
        h.invalidate();
    }

    fn done(&mut self, h: &mut Host, p: ConflictPolicy) {
        self.answer = Some(ConflictAnswer { choice: p, apply_to_all: self.apply_all, cancel_job: false });
        h.close();
    }

    fn compare_click(&mut self, h: &mut Host) {
        self.compare_enabled = false;
        if self.info.incoming_size != self.info.existing_size {
            self.show_compare(false, "Different sizes - these are not the same file.".into());
            return;
        }
        self.compare = Some(("\u{E895}".into(), None, "Comparing the content…".into()));
        let a = paths::to_long(&WString::from(self.info.incoming_path.as_str()));
        let b = paths::to_long(&WString::from(self.info.existing_path.as_str()));
        let size = self.info.incoming_size;
        let total = (size * 2).max(1);
        let closed = self.closed.clone();
        let w = h.win;
        std::thread::spawn(move || {
            let report = |n: i64| {
                let pct = (100.0 * n as f64 / total as f64) as i64;
                win::post(Box::new(move || {
                    crate::app::with_conflict(move |c, h| {
                        if c.last_report.is_none_or(|t| t.elapsed().as_millis() >= 150) {
                            c.last_report = Some(Instant::now());
                            c.compare = Some(("\u{E895}".into(), None, format!("Comparing the content… {pct}%")));
                            h.invalidate();
                        }
                    })
                }));
            };
            let c1 = closed.clone();
            let mut p1 = |n: i64| {
                report(n);
                !c1.load(SeqCst)
            };
            let ha = crate::engine::xxhash64::hash_file(&a, false, Some(&mut p1));
            let c2 = closed.clone();
            let mut p2 = |n: i64| {
                report(size + n);
                !c2.load(SeqCst)
            };
            let hb = ha.and_then(|_| crate::engine::xxhash64::hash_file(&b, false, Some(&mut p2)));
            if closed.load(SeqCst) {
                return;
            }
            let (same, text) = match (ha, hb) {
                (Err(e), _) | (_, Err(e)) => (false, format!("Couldn't read one of the files: {}", crate::sys::error_text(e))),
                (Ok(x), Ok(y)) if x == y => (true, "Same content - Skip is safe.".to_string()),
                _ => (false, "Different content, same size.".to_string()),
            };
            let _ = w;
            win::post(Box::new(move || {
                crate::app::with_conflict(move |c, h| {
                    c.show_compare(same, text.clone());
                    h.invalidate();
                })
            }));
        });
        h.invalidate();
    }

    fn show_compare(&mut self, same: bool, text: String) {
        self.compare = Some((if same { G_CHECK.into() } else { G_WARNING.into() }, Some(same), text));
    }

    fn load_thumbs(&self) {
        let (a, b) = (self.info.incoming_path.clone(), self.info.existing_path.clone());
        let closed = self.closed.clone();
        std::thread::spawn(move || {
            crate::sys::com_init_sta();
            for (which, p) in [(0, a), (1, b)] {
                let t = win::shell_thumbnail(&p, 320);
                if closed.load(SeqCst) {
                    return;
                }
                if let Some((px, w, h, premul)) = t {
                    win::post(Box::new(move || {
                        crate::app::with_conflict(move |c, host| {
                            if let Some(img) = host.g.bgra_image(&px, w, h, premul, w, h) {
                                if which == 0 {
                                    c.in_img = Some(img);
                                } else {
                                    c.ex_img = Some(img);
                                }
                                host.invalidate();
                            }
                        })
                    }));
                }
            }
        });
    }

    pub fn press_for_test(&mut self, h: &mut Host, name: &str, all: bool) {
        self.apply_all = all;
        match name {
            "replace" => self.done(h, ConflictPolicy::Replace),
            "skip" => self.done(h, ConflictPolicy::Skip),
            "keepboth" => self.done(h, ConflictPolicy::KeepBoth),
            "compare" => self.compare_click(h),
            "pickexisting" => self.pick(h, true),
            "pickincoming" => self.pick(h, false),
            "primary" => self.done(h, if self.keep_existing { ConflictPolicy::Skip } else { ConflictPolicy::Replace }),
            _ => {}
        }
        h.invalidate();
    }

    /// One card: picked (blue, its picture bright, a check mark) or not (grey, its picture dimmed).
    #[allow(clippy::too_many_arguments)]
    fn card(&self, ui: &UiState, id: u32, head: &str, picked: bool, img: &Option<Image>, ext: &str, size: i64, tag: &Option<String>, tag_bg: u32, tag_fg: u32, date: &Option<LocalTime>, tip: &str) -> El {
        let bd = if picked {
            PICK_BORDER
        } else if ui.is_hot(id) {
            HOVER_BORDER
        } else {
            OFF_BORDER
        };
        let mut box_g = grid(&[], &[]);
        box_g.push(text(ext, sf(22.0, SEMI), if picked { PICK_EXT } else { OFF_EXT }).center().shown(img.is_none()));
        if img.is_some() {
            box_g.push(image(img.clone()).uniform_to_fill(if picked { 1.0 } else { 0.55 }));
        }
        let mut size_row = hstack().m(0.0, 12.0, 0.0, 0.0).child(text(format::size(size), sf(15.0, SEMI), TEXT));
        size_row.push(
            border(Some(Fill::Solid(tag_bg)), None, 0.0, 8.0, Th::new(7.0, 1.0, 7.0, 2.0))
                .m(8.0, 0.0, 0.0, 0.0)
                .vcenter()
                .shown(tag.is_some())
                .child(text(tag.clone().unwrap_or_default(), sf(11.5, NORMAL), tag_fg)),
        );
        border(Some(Fill::Solid(if picked { PICK_BACK } else { OFF_BACK })), Some(bd), 1.0, 9.0, Th::hv(16.0, 14.0))
            .id(id)
            .hand()
            .tip(tip)
            .child(
                vstack()
                    .child(
                        grid(&[], &[])
                            .child(text(head, sf(12.0, BOLD), if picked { PICK_HEAD } else { LABEL }).vcenter())
                            .child(text(G_CHECK, icons(13.0), 0xFF4FA8F0).right().vcenter().vis(if picked { Vis::Visible } else { Vis::Hidden })),
                    )
                    .child(fill(if picked { PICK_BOX } else { OFF_BOX }, 6.0).h(124.0).m(0.0, 10.0, 0.0, 0.0).clip().child(box_g))
                    .child(size_row)
                    .child(text(format!("Modified {}", format::date(when(date).as_ref())), sf(13.0, NORMAL), DIM).m(0.0, 3.0, 0.0, 0.0)),
            )
    }
}

impl Content for ConflictContent {
    fn build(&mut self, ui: &UiState) -> El {
        let i = &self.info;
        let name = std::path::Path::new(&i.existing_path).file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let ext = std::path::Path::new(&i.existing_path).extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
        let ext = if !ext.is_empty() && ext.chars().count() <= 5 { ext } else { "FILE".into() };
        let title = grid(&[], &[])
            .child(
                hstack()
                    .vcenter()
                    .m(16.0, 0.0, 0.0, 0.0)
                    .child(image(self.icon.clone()).size(18.0, 18.0))
                    .child(text("Choppy", sf(13.5, NORMAL), 0xFFE3E7EC).m(10.0, 0.0, 0.0, 0.0).vcenter()),
            )
            .child(title_button(ui, CLOSE, true, G_CLOSE, 10.0, 40.0).right().tip("Skip this file"));
        let mut sp = vstack().m(22.0, 4.0, 22.0, 22.0);
        sp.push(text("NAME CONFLICT", sf(12.0, BOLD), 0xFF4FA8F0));
        sp.push(text("File already exists", display(24.0, SEMI), TEXT).m(0.0, 6.0, 0.0, 0.0));
        sp.push(text(name, sf(15.0, NORMAL), 0xFFC9CED6).m(0.0, 4.0, 0.0, 0.0).trimmed().tip(i.existing_path.clone()).named("FileName"));
        sp.push(
            grid(&[Len::Star(1.0), Len::Px(14.0), Len::Star(1.0)], &[])
                .m(0.0, 18.0, 0.0, 0.0)
                .child(self.card(ui, IN_CARD, "INCOMING", !self.keep_existing, &self.in_img, &ext, i.incoming_size, &self.in_tag, 0xFF15324F, 0xFF8ACDFF, &i.incoming_time, &i.incoming_path).named("InCard"))
                .child(
                    self.card(ui, EX_CARD, "ALREADY THERE", self.keep_existing, &self.ex_img, &ext, i.existing_size, &self.ex_tag, 0xFF2A2F37, TEXT2, &i.existing_time, &i.existing_path)
                        .col(2)
                        .named("ExCard"),
                ),
        );
        if let Some((icon, same, t)) = &self.compare {
            let (ic, tc) = match same {
                None => (DIM, TEXT),
                Some(true) => (GOOD, GOOD),
                Some(false) => (WARN, TEXT2),
            };
            sp.push(
                border(Some(Fill::Solid(0xFF1B1F25)), Some(0xFF2C3138), 1.0, 7.0, Th::hv(12.0, 8.0))
                    .m(0.0, 14.0, 0.0, 0.0)
                    .child(hstack().child(text(icon, icons(14.0), ic).vcenter()).child(text(t, sf(13.5, NORMAL), tc).m(9.0, 0.0, 0.0, 0.0).vcenter().wrapped())),
            );
        }
        if i.remaining > 1 {
            sp.push(check(ui, APPLY_ALL, self.apply_all, text(format!("Apply to all {} remaining conflicts", i.remaining), sf(14.0, NORMAL), TEXT2)).m(0.0, 18.0, 0.0, 0.0).left());
        }
        let skip_tip = if i.op == OpKind::Move { "Leave both where they are (the original is not moved)" } else { "Keep the file that is already there" };
        let replace_tip = "Put the incoming file in place of the one already there";
        let (primary, second) = if self.keep_existing { ("Skip", "Replace") } else { ("Replace", "Skip") };
        let row = grid(&[], &[])
            .m(0.0, 18.0, 0.0, 0.0)
            .child(
                hstack()
                    .child(text_button(ui, COMPARE, &BTN, "Compare", self.compare_enabled).tip("Check whether both files have the same content (xxHash64)"))
                    .child(text_button(ui, SKIP, &BTN, second, true).m(10.0, 0.0, 0.0, 0.0).tip(if self.keep_existing { replace_tip } else { skip_tip }))
                    .child(text_button(ui, BOTH, &BTN, "Keep both", true).m(10.0, 0.0, 0.0, 0.0)),
            )
            .child(text_button(ui, REPLACE, &BTN_PRIMARY, primary, true).min_w(110.0).right().tip(if self.keep_existing { skip_tip } else { replace_tip }));
        sp.push(row);
        let root = grid(&[], &[Len::Px(40.0), Len::Auto]).child(title).child(sp.row(1));
        border(Some(window_bg()), Some(WIN_BORDER), 1.0, 0.0, Th::default()).child(root)
    }

    fn click(&mut self, h: &mut Host, id: u32) {
        match id {
            CLOSE => self.done(h, ConflictPolicy::Skip),
            IN_CARD => self.pick(h, false),
            EX_CARD => self.pick(h, true),
            APPLY_ALL => self.apply_all = !self.apply_all,
            COMPARE => self.compare_click(h),
            SKIP => self.done(h, if self.keep_existing { ConflictPolicy::Replace } else { ConflictPolicy::Skip }),
            BOTH => self.done(h, ConflictPolicy::KeepBoth),
            REPLACE => self.done(h, if self.keep_existing { ConflictPolicy::Skip } else { ConflictPolicy::Replace }),
            _ => {}
        }
        h.invalidate();
    }

    fn key(&mut self, h: &mut Host, vk: u16) -> bool {
        match vk {
            0x1B => {
                self.done(h, ConflictPolicy::Skip);
                true
            }
            // left / right: the other card
            0x25 if self.keep_existing => {
                self.pick(h, false);
                true
            }
            0x27 if !self.keep_existing => {
                self.pick(h, true);
                true
            }
            _ => false,
        }
    }

    fn closing(&mut self, h: &mut Host) -> bool {
        self.done(h, ConflictPolicy::Skip);
        true
    }

    fn closed(&mut self, _h: &mut Host) {
        self.closed.store(true, SeqCst);
        for w in self.disabled.drain(..) {
            if w.exists() {
                w.enable(true);
            }
        }
        let a = self.answer.unwrap_or(ConflictAnswer { choice: ConflictPolicy::Skip, apply_to_all: false, cancel_job: false });
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(a);
        }
        crate::app::conflict_closed();
    }
}

/// Shows the question (ShowDialog: the app's other windows wait). The answer goes to `tx` when it closes.
pub fn create(info: Incoming, owner: Option<Win>, tx: Sender<ConflictAnswer>) -> Option<Rc<RefCell<ConflictWindow>>> {
    let g = crate::app::gfx();
    let park = crate::testhook::park();
    let spec = Spec {
        title: "File already exists - Choppy".into(),
        owner,
        framed: true,
        taskbar: owner.is_none() && !park,
        topmost: owner.is_none() && !park,
        ..Default::default()
    };
    let mut c = ConflictContent::new(info, tx);
    c.disabled = win::thread_windows();
    let w = Window::create(&g, &spec, Sizing::ContentHeight(660.0), 40.0, c)?;
    {
        let mut m = w.borrow_mut();
        let Window { host, c } = &mut *m;
        for d in c.disabled.iter() {
            d.enable(false);
        }
        c.icon = host.g.icon_image(crate::ui::ICO, (18.0 * crate::ui::icon_scale()).round() as u32);
        let at = if park {
            Place::At(-32000, 0)
        } else {
            match owner {
                Some(o) => Place::Owner(o),
                None => Place::Screen,
            }
        };
        host.place(c, at);
        // Loaded: the thumbnails, the focus on the main button
        c.load_thumbs();
        host.ui.focus = REPLACE;
        host.paint(c);
        host.win.show_drawn(!park);
    }
    Some(w)
}
