//! App.cs, the parts the windows need (step 4): the UI thread, the job manager's questions from the copy threads
//! (conflicts, low space, administrator rights) as windows, showing the copy / settings / Benchmark windows, the
//! dialogs list, exiting when nothing is left, and the tray (EnsureTray, TrayOpen, UpdateTrayMenu, SetTrayText,
//! TrayBalloon, QuitFromTray). Step 5 adds the rest of App.cs: the single instance and its pipe, the Explorer
//! takeover, warm-up, resume, the administrator copy.
#![deny(unsafe_code)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};

use crate::engine::copy_job::CopyJob;
use crate::engine::model::{ConflictAnswer, ConflictInfo, ConflictPolicy, JobSpec, OpKind};
use crate::engine::paths;
use crate::engine::settings::Settings;
use crate::job_manager::JobManager;
use crate::sys::gfx::Gfx;
use crate::sys::win::{self, Win};
use crate::ui::bench_window::{self, BenchWindow};
use crate::ui::conflict_window::{self, ConflictWindow, Incoming};
use crate::ui::dialogs::{self, DialogWindow};
use crate::ui::format;
use crate::ui::host::Window;
use crate::ui::main_window::{self, MainWindow};
use crate::ui::settings_window::{self, SettingsWindow};
use crate::ui::tray_menu;
use crate::wstr::WString;

pub struct App {
    pub gfx: Rc<Gfx>,
    pub manager: Arc<JobManager>,
    pub main: Option<Rc<RefCell<MainWindow>>>,
    pub settings: Option<Rc<RefCell<SettingsWindow>>>,
    pub bench: Option<Rc<RefCell<BenchWindow>>>,
    pub conflict: Option<Rc<RefCell<ConflictWindow>>>,
    pub dialogs: Vec<Rc<RefCell<DialogWindow>>>,
    pub tray: bool,
    pub resident: bool,
    pub elevated: bool,
    pub hook: Option<crate::testhook::TestHook>,
    pub quit_requested: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

static MANAGER: OnceLock<Arc<JobManager>> = OnceLock::new();

/// Runs `f` with the app (UI thread). Keep it short: never call into a window while holding it.
pub fn with<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut a| a.as_mut().map(f)))
}

pub fn gfx() -> Rc<Gfx> {
    with(|a| a.gfx.clone()).expect("the UI thread's app")
}

pub fn manager() -> Arc<JobManager> {
    MANAGER.get().cloned().expect("the job manager")
}

pub fn elevated() -> bool {
    with(|a| a.elevated).unwrap_or(false)
}

pub fn main_window() -> Option<Rc<RefCell<MainWindow>>> {
    with(|a| a.main.clone()).flatten()
}

/// Runs `f` on the copy window when there is one; when it is busy (its own click started this), right after.
pub fn with_main(f: impl Fn(&mut MainWindow) + 'static) {
    let Some(w) = main_window() else { return };
    match w.try_borrow_mut() {
        Ok(mut m) => f(&mut m),
        Err(_) => win::post_local(Box::new(move || {
            if let Some(w) = main_window()
                && let Ok(mut m) = w.try_borrow_mut()
            {
                f(&mut m);
            }
        })),
    }
}

/// App.Start: the UI thread, the job manager wired to the windows, the test hook; then `args`; the message loop.
pub fn run(args: &[WString]) -> i32 {
    crate::testhook::dpi96_if_asked();
    crate::sys::com_init_sta();
    win::init_ui_thread();
    let Some(gfx) = Gfx::new() else { return 1 };
    let manager = make_manager();
    let _ = MANAGER.set(manager.clone());
    APP.with(|a| {
        *a.borrow_mut() = Some(App {
            gfx,
            manager,
            main: None,
            settings: None,
            bench: None,
            conflict: None,
            dialogs: Vec::new(),
            tray: false,
            resident: false,
            elevated: false,
            hook: None,
            quit_requested: false,
        })
    });
    if let Ok(dir) = std::env::var("CHOPPY_TESTHOOK")
        && !dir.is_empty()
    {
        let h = crate::testhook::TestHook::start(&dir);
        with(|a| a.hook = Some(h));
    }
    let a: Vec<String> = args.iter().map(|a| a.to_string_lossy()).collect();
    handle_args(&a);
    // test files a benchmark left behind (Choppy closed or the PC went off while it ran)
    std::thread::spawn(|| {
        let _ = std::panic::catch_unwind(crate::bench::files::clean_leftovers);
    });
    let code = win::run_loop();
    win::tray_remove();
    code
}

fn make_manager() -> Arc<JobManager> {
    let mut m = JobManager::default();
    m.dispatch = Some(Arc::new(win::post));
    m.on_ui_thread = Some(Arc::new(win::on_ui_thread));
    m.ask = Some(Arc::new(ask_conflict));
    m.ask_low_space = Some(Arc::new(ask_low_space));
    m.ask_elevate = Some(Arc::new(ask_elevate));
    m.job_started = Some(Arc::new(|j: &Arc<CopyJob>| {
        if j.spec.handoff_path.is_some() {
            crate::engine::handoff::report(j); // keeps the user's Choppy up to date
        }
        // a new copy: the window comes up on top
        win::post(Box::new(|| show_main(true)));
    }));
    m.all_done = Some(Arc::new(|_| win::post(Box::new(maybe_exit))));
    let m = Arc::new(m);
    m.on_changed(Arc::new(|| {
        with_main(|w| {
            let Window { host, c } = w;
            c.on_manager_changed(host);
        })
    }));
    m
}

// ================= commands =================

/// HandleArgs (step 4: copy / move, the settings, --tray; step 5 adds paste, --resume, --takeover, --menu...).
pub fn handle_args(args: &[String]) {
    let cmd = args.first().map(|a| a.to_lowercase()).unwrap_or_default();
    match cmd.as_str() {
        "--tray" => {
            with(|a| a.resident = true);
            ensure_tray();
        }
        "copy" | "move" => {
            if args.len() < 3 {
                show_settings(None);
            } else if let Some(spec) =
                new_spec(if cmd == "move" { OpKind::Move } else { OpKind::Copy }, &args[1..args.len() - 1], &args[args.len() - 1], "Command line")
            {
                manager().enqueue(spec);
                show_main(true);
            }
        }
        _ => show_settings(None),
    }
    maybe_exit();
}

fn new_spec(op: OpKind, sources: &[String], dest: &str, origin: &str) -> Option<JobSpec> {
    let s = Settings::current();
    let mut spec = JobSpec {
        op,
        dest: paths::to_long(&WString::from(dest)),
        verify: s.verify_copies,
        policy: s.policy,
        origin: origin.into(),
        ..JobSpec::new(op)
    };
    drop(s);
    for src in sources {
        if !src.trim().is_empty() {
            spec.sources.push(paths::to_long(&WString::from(src.as_str())));
        }
    }
    (!spec.sources.is_empty() && !spec.dest.is_empty()).then_some(spec)
}

// ================= windows =================

/// Shows the copy window. lift = a new copy was started: the window comes up above every other window, then after
/// a moment goes back to normal stacking (it stays in front until the user clicks something else).
pub fn show_main(lift: bool) {
    let park = crate::testhook::park();
    let w = match main_window() {
        Some(w) => w,
        None => {
            let Some(w) = main_window::create(manager()) else { return };
            with(|a| a.main = Some(w.clone()));
            if let Ok(mut m) = w.try_borrow_mut() {
                let Window { host, c } = &mut *m;
                c.loaded(host);
            }
            w
        }
    };
    let Ok(mut m) = w.try_borrow_mut() else { return };
    let Window { host, c } = &mut *m;
    if lift && !park {
        host.win.set_topmost(true); // before Show: it appears on top, not behind Explorer
    }
    if !host.win.is_visible() {
        host.paint(c);
        host.win.show_drawn(!park);
    }
    if host.win.is_minimized() {
        host.win.restore();
    }
    if park {
        return;
    }
    // Windows may refuse the focus to a program in the background; on top it is seen at once all the same
    let active = host.win.activate();
    if lift || !active {
        c.lift(host);
    }
}

pub fn show_settings(owner: Option<Win>) {
    if let Some(s) = with(|a| a.settings.clone()).flatten() {
        if let Ok(s) = s.try_borrow() {
            s.host.win.activate();
        }
        return;
    }
    if let Some(w) = settings_window::create(owner) {
        with(|a| a.settings = Some(w));
    }
}

/// The Benchmark window (one at a time; it isn't owned by the settings, so closing those leaves it open).
pub fn show_benchmark() {
    if let Some(b) = with(|a| a.bench.clone()).flatten() {
        if let Ok(b) = b.try_borrow() {
            if b.host.win.is_minimized() {
                b.host.win.restore();
            }
            b.host.win.activate();
        }
        return;
    }
    if let Some(w) = bench_window::create() {
        with(|a| a.bench = Some(w));
    }
}

/// The settings window shows the settings as they are now (the copy window's quick toggles changed them).
pub fn settings_reload() {
    let Some(s) = with(|a| a.settings.clone()).flatten() else { return };
    let r = s.try_borrow_mut();
    if let Ok(mut s) = r {
        let Window { host, c } = &mut *s;
        let f = host.ui.focus;
        c.reload(f);
        host.invalidate();
    }
}

pub fn register_dialog(w: Rc<RefCell<DialogWindow>>) {
    with(|a| a.dialogs.push(w));
}

pub fn dialog_closed() {
    win::post_local(Box::new(|| {
        with(|a| a.dialogs.retain(|d| d.try_borrow().map(|d| !d.host.closed).unwrap_or(true)));
        maybe_exit();
    }));
}

pub fn top_dialog() -> Option<Rc<RefCell<DialogWindow>>> {
    with(|a| a.dialogs.last().cloned()).flatten()
}

pub fn main_closed() {
    win::post_local(Box::new(|| {
        with(|a| a.main = None);
        maybe_exit();
    }));
}
pub fn settings_closed() {
    win::post_local(Box::new(|| {
        with(|a| a.settings = None);
        maybe_exit();
    }));
}
pub fn bench_closed() {
    win::post_local(Box::new(|| {
        with(|a| a.bench = None);
        maybe_exit();
    }));
}

/// Exits when nothing is left to do (not in tray mode).
pub fn maybe_exit() {
    let quit = with(|a| {
        if a.resident || a.manager.busy() {
            return false;
        }
        if a.main.is_some() || a.settings.is_some() || a.bench.is_some() || !a.dialogs.is_empty() || a.conflict.is_some() {
            return false;
        }
        if a.hook.is_some() && !a.quit_requested {
            return false;
        }
        true
    })
    .unwrap_or(false);
    if quit {
        win::quit(0);
    }
}

// ================= engine questions (from copy threads) =================

fn ask_conflict(job: &Arc<CopyJob>, info: &ConflictInfo) -> Option<ConflictAnswer> {
    let (tx, rx) = mpsc::channel::<ConflictAnswer>();
    let inc = Incoming {
        incoming_path: info.incoming_path.to_string_lossy(),
        existing_path: info.existing_path.to_string_lossy(),
        incoming_size: info.incoming_size,
        existing_size: info.existing_size,
        incoming_time: info.incoming_time,
        existing_time: info.existing_time,
        remaining: info.remaining,
        op: info.op,
    };
    let _ = job;
    win::post(Box::new(move || open_conflict(inc, tx)));
    Some(rx.recv().unwrap_or(ConflictAnswer { choice: ConflictPolicy::Skip, apply_to_all: false, cancel_job: false }))
}

fn open_conflict(inc: Incoming, tx: mpsc::Sender<ConflictAnswer>) {
    let owner = main_window().and_then(|m| m.try_borrow().ok().map(|m| m.host.win)).filter(|w| w.is_visible());
    if let Some(w) = conflict_window::create(inc, owner, tx) {
        with(|a| a.conflict = Some(w));
    }
}

/// Runs `f` on the open conflict window (its compare and thumbnail threads report through here).
pub fn with_conflict(f: impl FnOnce(&mut crate::ui::conflict_window::ConflictContent, &mut crate::ui::host::Host)) {
    let Some(w) = with(|a| a.conflict.clone()).flatten() else { return };
    if let Ok(mut m) = w.try_borrow_mut() {
        let Window { host, c } = &mut *m;
        if !host.closed {
            f(c, host);
        }
    }
}

/// Runs `f` on the Benchmark window (its worker and scans report through here).
pub fn with_bench(f: impl FnOnce(&mut crate::ui::bench_window::BenchContent, &mut crate::ui::host::Host)) {
    let Some(w) = with(|a| a.bench.clone()).flatten() else { return };
    if let Ok(mut m) = w.try_borrow_mut() {
        let Window { host, c } = &mut *m;
        if !host.closed {
            f(c, host);
        }
    }
}

pub fn conflict_closed() {
    win::post_local(Box::new(|| {
        with(|a| a.conflict = None);
        maybe_exit();
    }));
}

fn ask_low_space(job: &Arc<CopyJob>, need: i64, free: i64) -> bool {
    let (tx, rx) = mpsc::channel::<bool>();
    let dest = paths::to_short(&job.spec.dest).to_string_lossy();
    win::post(Box::new(move || {
        show_main(false);
        let text = format!(
            "This copy needs {} and {dest} has {} free. Files that replace existing ones need less. Copy anyway?",
            format::size(need),
            format::size(free)
        );
        dialogs::ask(main_win(), "Not enough free space", &text, "Copy anyway", "Cancel", move |go| {
            let _ = tx.send(go);
        });
    }));
    rx.recv().unwrap_or(false)
}

/// The destination only lets administrators add files: copy the rest as administrator? (once per copy)
fn ask_elevate(job: &Arc<CopyJob>) -> bool {
    let (tx, rx) = mpsc::channel::<bool>();
    let dest = paths::to_short(&job.spec.dest).to_string_lossy();
    let verb = if job.spec.op == OpKind::Move { "Move" } else { "Copy" };
    win::post(Box::new(move || {
        show_main(false);
        let text = format!("{dest}\n{verb} as administrator? The rest of this {} goes on in its own window.", verb.to_lowercase());
        dialogs::ask(main_win(), "This folder needs administrator rights", &text, &format!("{verb} as administrator"), "No", move |go| {
            let _ = tx.send(go);
        });
    }));
    rx.recv().unwrap_or(false)
}

fn main_win() -> Option<Win> {
    main_window().and_then(|m| m.try_borrow().ok().map(|m| m.host.win))
}

// ================= tray =================

pub fn ensure_tray() {
    if crate::testhook::park() || elevated() || with(|a| a.tray).unwrap_or(true) {
        return;
    }
    with(|a| a.tray = true);
    win::tray_add(
        "Choppy",
        Rc::new(|m| match m {
            // NotifyIcon.MouseClick (left) opens; the right button shows the menu
            0x0202 => win::post_local(Box::new(tray_open)),
            0x0205 => tray_menu::show(),
            _ => {}
        }),
    );
}

pub fn tray_open() {
    if manager().current().is_some() || main_window().is_some() {
        show_main(false);
    } else {
        show_settings(None);
    }
}

/// Tray tooltip: progress + speed now (avg) + time left while copying (NotifyIcon.Text: up to 63 characters).
pub fn set_tray_text(text: &str) {
    if !with(|a| a.tray).unwrap_or(false) {
        return;
    }
    let t: String = text.chars().take(63).collect();
    win::tray_tip(&t);
}

pub fn tray_balloon(text: &str) {
    if with(|a| a.tray).unwrap_or(false) {
        win::tray_balloon("Choppy", text, 3000);
    }
}

pub fn quit_from_tray() {
    let go = || {
        let m = manager();
        m.cancel_current();
        for s in m.waiting_snapshot() {
            m.remove(&s);
        }
        win::tray_remove();
        with(|a| a.tray = false);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while m.current().is_some_and(|c| !c.is_finished()) && std::time::Instant::now() < deadline {
            crate::sys::sleep_ms(50);
        }
        win::quit(0);
    };
    if manager().busy() {
        dialogs::ask(main_win(), "Quit Choppy?", "A copy is still running. Quitting cancels it (files already copied stay).", "Quit", "Keep copying", move |yes| {
            if yes {
                go();
            }
        });
    } else {
        go();
    }
}

/// The test hook's "quit".
pub fn quit_for_test() {
    with(|a| a.quit_requested = true);
    manager().cancel_current();
    win::quit(0);
}

/// ApplyTakeover (step 4: in the tray while it is on; step 5 adds the Ctrl+V hook and the warm-up).
pub fn apply_takeover() {
    if elevated() {
        return;
    }
    let on = Settings::current().explorer_takeover;
    if on {
        with(|a| a.resident = true);
        ensure_tray();
    } else {
        let busy = manager().busy();
        let resident = with(|a| a.resident).unwrap_or(false);
        if resident && !busy && with(|a| a.tray).unwrap_or(false) {
            win::tray_remove();
            with(|a| a.tray = false);
        }
        with(|a| a.resident = false);
    }
    maybe_exit();
}
