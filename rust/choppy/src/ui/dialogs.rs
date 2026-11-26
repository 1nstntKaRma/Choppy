//! Helpers.cs Dialogs: the dark question / message box in the Choppy style, and About. Like the C#, these windows
//! are built in code: WPF's defaults apply there (no layout rounding, Ideal text), so `plain` is on. ShowDialog's
//! behavior: every other window of the app waits (disabled) until it closes; the answer goes to a callback (the UI
//! thread never runs a second message loop).

use std::cell::RefCell;
use std::rc::Rc;

use crate::sys::gfx::Image;
use crate::sys::win::{self, Spec, Win};
use crate::ui::host::{Content, Host, Place, Sizing, Window};
use crate::ui::kit::*;
use crate::ui::layout::*;
use crate::ui::theme::*;

pub const REPO: &str = "https://github.com/1nstntKaRma/Choppy";

const B1: u32 = 1;
const B2: u32 = 2;
const LINK: u32 = 3;
const LICENSE: u32 = 4;

pub type Done = Box<dyn FnOnce(i32)>;

pub struct DialogContent {
    title: String,
    text: Option<String>,
    primary: String,
    secondary: Option<String>,
    /// About instead of a question.
    about: Option<About>,
    pub result: i32,
    done: Option<Done>,
    /// The windows this one disabled (ShowDialog), enabled again when it closes.
    disabled: Vec<Win>,
}

struct About {
    icon: Option<Image>,
    license_scroll: f64,
}

pub type DialogWindow = Window<DialogContent>;

impl DialogContent {
    /// The answer (1 primary, 2 secondary, 0 closed) and close; the test hook's "dialog n" too.
    pub fn answer(&mut self, h: &mut Host, r: i32) {
        self.result = r;
        h.close();
    }
}

impl Content for DialogContent {
    fn build(&mut self, ui: &UiState) -> El {
        let body = if self.about.is_some() { self.build_about(ui) } else { self.build_question(ui) };
        border(Some(window_bg()), Some(WIN_BORDER), 1.0, 0.0, Th::default()).child(body)
    }

    fn click(&mut self, h: &mut Host, id: u32) {
        match id {
            B1 => self.answer(h, 1),
            B2 => self.answer(h, 2),
            LINK => {
                win::shell_open(REPO, None);
            }
            _ => {}
        }
    }

    fn key(&mut self, h: &mut Host, vk: u16) -> bool {
        match vk {
            // IsCancel: Esc presses the second button (About: Close)
            0x1B => {
                if self.about.is_some() {
                    self.answer(h, 1);
                } else if self.secondary.is_some() {
                    self.answer(h, 2);
                }
                true
            }
            // IsDefault: Enter presses the primary button unless another button has the focus
            0x0D if h.ui.focus == 0 || h.ui.focus == LICENSE => {
                self.answer(h, 1);
                true
            }
            _ => false,
        }
    }

    fn wheel(&mut self, h: &mut Host, delta: i32, _x: f64, _y: f64) {
        if let Some(a) = self.about.as_mut() {
            // three lines a notch, as WPF's ScrollViewer
            a.license_scroll = (a.license_scroll - delta as f64 / 120.0 * 48.0).max(0.0);
            h.invalidate();
        }
    }

    fn closed(&mut self, _h: &mut Host) {
        for w in self.disabled.drain(..) {
            if w.exists() {
                w.enable(true);
            }
        }
        if let Some(d) = self.done.take() {
            let r = self.result;
            // after the window is gone (the owner gets the focus back first)
            win::post_local(Box::new(move || d(r)));
        }
        crate::app::dialog_closed();
    }
}

impl DialogContent {
    fn build_question(&self, ui: &UiState) -> El {
        let mut sp = vstack().m(22.0, 20.0, 22.0, 20.0);
        sp.push(text(&self.title, sf(18.0, SEMI), TEXT).wrapped());
        if let Some(t) = self.text.as_ref().filter(|t| !t.is_empty()) {
            sp.push(text(t, sf(14.0, NORMAL), DIM).wrapped().m(0.0, 8.0, 0.0, 0.0));
        }
        let mut row = hstack().right().m(0.0, 20.0, 0.0, 0.0);
        if let Some(s) = &self.secondary {
            row.push(text_button(ui, B2, &BTN, s, true).m(0.0, 0.0, 10.0, 0.0));
        }
        row.push(text_button(ui, B1, &BTN_PRIMARY, &self.primary, true));
        sp.child(row)
    }

    fn build_about(&self, ui: &UiState) -> El {
        let a = self.about.as_ref().unwrap();
        let mut sp = vstack().m(24.0, 22.0, 24.0, 20.0);
        let names = vstack()
            .vcenter()
            .child(text("Choppy", sf(22.0, SEMI), TEXT))
            .child(text(format!("v{} · fast, clear file copier for Windows", env!("CHOPPY_VERSION")), sf(13.5, NORMAL), DIM));
        let mut head = hstack();
        if a.icon.is_some() {
            head.push(image(a.icon.clone()).size(72.0, 72.0).m(0.0, 0.0, 18.0, 0.0));
        }
        sp.push(head.child(names));
        sp.push(runs(vec![run("by ", 14.0, NORMAL, TEXT2), run("1nstntKaRma", 14.0, SEMI, TEXT)]).m(0.0, 18.0, 0.0, 0.0));
        sp.push(link_button(ui, LINK, text(REPO.replace("https://", ""), sf(14.0, NORMAL), LINK_C)).m(-6.0, 4.0, 0.0, 0.0).left().tip(REPO));
        sp.push(text("MIT LICENSE", T_CAP, LABEL).m(0.0, 18.0, 0.0, 6.0));
        // the license: a read-only TextBox (WPF's own template: hover and focus color its border), 170 high
        let bd = if ui.is_focused(LICENSE) {
            0xFF569DE5
        } else if ui.is_hot(LICENSE) {
            0xFF7EB4EA
        } else {
            CARD_BORDER
        };
        let lic = text(license(), sf(12.0, NORMAL), DIM).wrapped().m(12.0, 8.0, 12.0, 8.0);
        sp.push(
            border(Some(Fill::Solid(CARD)), Some(bd), 1.0, 0.0, Th::default())
                .h(170.0)
                .child(scroll(a.license_scroll, lic))
                .id(LICENSE)
                .cursor(win::Cursor::IBeam)
                .focusable(true),
        );
        sp.push(text_button(ui, B1, &BTN_PRIMARY, "Close", true).right().m(0.0, 18.0, 0.0, 0.0));
        sp
    }
}

const LINK_C: u32 = crate::ui::theme::LINK;

/// The LICENSE file (embedded like the C#'s manifest resource), trimmed.
fn license() -> String {
    include_str!("../../../../LICENSE").trim().to_string()
}

/// Dialogs.Show: returns through `done` 1 for the first (primary) button, 2 for the second, 0 for closed.
pub fn show(owner: Option<Win>, title: &str, text: &str, primary: &str, secondary: Option<&str>, done: impl FnOnce(i32) + 'static) {
    let c = DialogContent {
        title: title.into(),
        text: Some(text.into()),
        primary: primary.into(),
        secondary: secondary.map(|s| s.into()),
        about: None,
        result: 0,
        done: Some(Box::new(done)),
        disabled: Vec::new(),
    };
    open(owner, c, 440.0, true);
}

/// Dialogs.Ask: true for the first button.
pub fn ask(owner: Option<Win>, title: &str, text: &str, yes: &str, no: &str, done: impl FnOnce(bool) + 'static) {
    show(owner, title, text, yes, Some(no), move |r| done(r == 1));
}

/// About: name, version, author, the project page and the MIT license.
pub fn about(owner: Option<Win>) {
    let g = crate::app::gfx();
    // ChoppyIcon.Frame(72): the smallest frame of Choppy.ico at least 72 x the screen's scale
    let scale = win::dpi_at_cursor() as f64 / 96.0;
    let icon = g.icon_image(crate::ui::ICO, (72.0 * scale.max(1.0)).round() as u32);
    let c = DialogContent {
        title: "About Choppy".into(),
        text: None,
        primary: "Close".into(),
        secondary: None,
        about: Some(About { icon, license_scroll: 0.0 }),
        result: 0,
        done: None,
        disabled: Vec::new(),
    };
    open(owner, c, 520.0, false);
}

fn open(owner: Option<Win>, mut c: DialogContent, width: f64, topmost_alone: bool) {
    let g = crate::app::gfx();
    let owner = owner.filter(|o| o.exists() && o.is_visible());
    let park = crate::testhook::park();
    let spec = Spec {
        title: if c.about.is_some() { "About Choppy".into() } else { "Choppy".into() },
        owner,
        taskbar: owner.is_none() && !park,
        topmost: owner.is_none() && topmost_alone && !park,
        ..Default::default()
    };
    // ShowDialog: the app's other windows wait
    c.disabled = win::thread_windows();
    let Some(w) = Window::create(&g, &spec, Sizing::ContentHeight(width), f64::INFINITY, c) else { return };
    let rc: Rc<RefCell<DialogWindow>> = w;
    {
        let mut m = rc.borrow_mut();
        let Window { host, c } = &mut *m;
        host.plain = true;
        for d in c.disabled.iter() {
            d.enable(false);
        }
        let at = if park {
            Place::At(-32000, 0)
        } else {
            match owner {
                Some(o) => Place::Owner(o),
                None => Place::Screen,
            }
        };
        host.place(c, at);
        // Loaded: the primary button (About: Close) has the focus
        host.ui.focus = B1;
        host.win.show_drawn(!park);
    }
    crate::app::register_dialog(rc);
}
