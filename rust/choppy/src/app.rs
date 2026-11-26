//! App.cs: one Choppy per user. A second start (Explorer paste, right-click, command line) hands its job to the first
//! through a pipe and exits at once, so the window opens instantly in the running copy (instance.rs).
//!   Choppy.exe copy|move <src...> <destDir>   Choppy.exe paste <destDir>   Choppy.exe --tray   Choppy.exe (settings)
//! The UI thread, the job manager's questions from the copy threads (conflicts, low space, administrator rights) as
//! windows, the copy / settings / Benchmark windows, the dialogs list, exiting when nothing is left, the tray (step 4);
//! the commands, the Explorer paste, the takeover, the quick first window, the resume offer, the administrator copy
//! and a crash's revive (step 5).
#![deny(unsafe_code)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::{Arc, OnceLock};

use crate::engine::copy_job::CopyJob;
use crate::engine::handoff;
use crate::engine::model::{ConflictAnswer, ConflictInfo, ConflictPolicy, JobSpec, OpKind};
use crate::engine::paths;
use crate::engine::settings::{Settings, app_paths};
use crate::explorer::{clipboard_files, icon_refresh, paste_hook, paste_log, shell_location, takeover};
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
    /// a copy window was built in this process: the next ones come up at once (see schedule_warm_up)
    pub window_built: bool,
    pub resume_offered: bool,
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

/// App.Start: runs this Choppy (instance::claim got the mutex first): the UI thread, the job manager wired to the
/// windows, the pipe, the test hook; then `args`; the message loop. `elevated` = this is the short-lived administrator
/// copy (started by the user's own Choppy for a folder that needs administrator rights): its own mutex and pipe, no
/// keyboard hook, no tray, never touches the Run key, the right-click entries or Ultracopier. It runs its copy in its
/// own window and exits.
pub fn run(args: &[WString], mutex: crate::sys::NamedMutex, pipe_name: &str, elevated: bool) -> i32 {
    crate::testhook::dpi96_if_asked();
    crate::sys::com_init_sta();
    win::init_ui_thread();
    // an error on the UI thread is noted and Choppy goes on; one that ends the process: noted, and a Choppy that
    // watches Ctrl+V starts again in the tray
    win::on_ui_error(|e| note_error(&e, false));
    // signing out or shutting down ends Choppy on purpose: its guard starts nothing
    win::on_session_end(crate::guard::quitting);
    ELEVATED.store(elevated, std::sync::atomic::Ordering::SeqCst);
    crate::sys::on_crash(on_crash);
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
            elevated,
            hook: None,
            quit_requested: false,
            window_built: false,
            resume_offered: false,
        })
    });
    crate::instance::listen(pipe_name, |a| win::post(Box::new(move || handle_args(&a))));
    if let Ok(dir) = std::env::var("CHOPPY_TESTHOOK")
        && !dir.is_empty()
    {
        let h = crate::testhook::TestHook::start(&dir);
        with(|a| a.hook = Some(h));
    }

    // the installer's own steps (--menu, --uninstall...) find the registry entries checked before they run
    let step = args.first().is_some_and(|a| a.starts_with("--"));
    if !elevated && step {
        takeover::refresh();
    }
    // "Default file manager" on: any start of Choppy for a copy, a paste or the settings stays in the tray watching
    // Ctrl+V - a right-click paste brings the takeover back even if the tray copy was closed (not for the installer's
    // own steps like --menu, which wait for it to exit). The copy's window comes first: the tray, the hook and the
    // registry check follow right after it
    let takeover = !elevated && !app_paths::test_mode() && !step && Settings::current().explorer_takeover;
    if takeover {
        with(|a| a.resident = true);
    }
    handle_args(args);
    if !elevated && !step {
        win::idle(Box::new(move || {
            takeover::refresh();
            if takeover {
                apply_takeover();
            }
        }));
    }
    if !elevated {
        win::idle(Box::new(offer_resume));
    }
    // a new icon in this exe: Explorer's icon cache still has the old one (taskbar pins, Start menu)
    if !elevated && !step {
        win::idle(Box::new(|| {
            let _ = std::thread::Builder::new().name("icon refresh".into()).spawn(|| {
                let _ = std::panic::catch_unwind(icon_refresh::if_changed);
            });
        }));
    }
    // test files a benchmark left behind (Choppy closed or the PC went off while it ran)
    if !elevated {
        win::idle(Box::new(|| {
            let _ = std::thread::Builder::new().name("bench leftovers".into()).spawn(|| {
                let _ = std::panic::catch_unwind(crate::bench::files::clean_leftovers);
            });
        }));
    }
    let code = win::run_loop();
    // OnExit: a quit on purpose - the guard (if any) leaves without starting anything
    crate::guard::quitting();
    paste_hook::stop();
    // the app (its windows, the test hook's timer) goes now, while the UI thread's other thread-locals are still there:
    // left to the thread's end, it would reach for ones already gone (Rust aborts then: exit code 0xC0000409)
    let app = APP.with(|a| a.borrow_mut().take());
    drop(app);
    win::tray_remove();
    mutex.release(); // one Choppy per user until here
    drop(mutex);
    code
}

static ELEVATED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// errors.txt in the data folder: DateTime.Now + " " + the error ("(closed) " first when it ended Choppy).
pub fn note_error(text: &str, closed: bool) {
    let now = crate::sys::file_time_to_local(crate::sys::now_file_time()).map(|t| crate::sys::general_date_time(&t)).unwrap_or_default();
    let line = format!("{now} {}{text}\r\n", if closed { "(closed) " } else { "" });
    let path = app_paths::data_dir().join("errors.txt");
    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open(path) {
        use std::io::Write;
        let _ = f.write_all(line.as_bytes());
    }
}

/// OnCrash: an error ended the process. Noted; a Choppy that watched Ctrl+V starts again in the tray (not when it
/// fails right at the start: it would only fail again).
fn on_crash() {
    note_error("the process ended on an error Windows reported", true);
    // the guard starts it again once this process is completely gone
    if crate::guard::running() {
        return;
    }
    let elevated = ELEVATED.load(std::sync::atomic::Ordering::SeqCst);
    if elevated || app_paths::test_mode() || crate::started().elapsed().as_secs() < 30 {
        return;
    }
    // the settings may be held by the thread that failed: not waited for
    let on = match Settings::try_current() {
        Some(s) => s.explorer_takeover,
        None => return,
    };
    if !on {
        return;
    }
    paste_log::write("Choppy closed on an error; started again in the tray.");
    let _ = std::process::Command::new(takeover::exe_path().to_os()).arg("--revive").arg(crate::sys::current_pid().to_string()).spawn();
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

pub fn handle_args(args: &[WString]) {
    let cmd = args.first().map(|a| a.to_lower_invariant().to_string_lossy()).unwrap_or_default();
    if elevated() {
        // the administrator copy does its one job and nothing else
        if cmd == handoff::ARG && args.len() > 1 {
            let record = std::path::PathBuf::from(args[1].to_os());
            let fp = args.get(2).map(|a| a.to_string_lossy());
            if let Some(hs) = handoff::load(&record, fp.as_deref()) {
                manager().enqueue(hs);
            }
        } else if cmd == "--quit" {
            quit_from_tray();
            return;
        }
        maybe_exit();
        return;
    }
    match cmd.as_str() {
        "--tray" => {
            with(|a| a.resident = true);
            apply_takeover();
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
        "paste" => paste_into(args.get(1), "Right-click"),
        "--resume" => offer_resume(),
        "--takeover" => {
            let on = args.get(1).is_some_and(|a| a.eq_ignore_case_str("on"));
            if let Some(err) = takeover::set(on) {
                dialogs::show(None, "Couldn't change the Explorer setting", &err, "OK", None, |_| {});
            }
        }
        "--menu" => {
            takeover::ensure_menu();
            icon_refresh::if_changed(); // the installer runs this right after it replaced the exe
        }
        "--uninstall" => {
            takeover::remove_all();
            quit_from_tray();
            return;
        }
        "--quit" => {
            quit_from_tray();
            return;
        }
        _ => show_settings(None),
    }
    maybe_exit();
}

fn new_spec(op: OpKind, sources: &[WString], dest: &WString, origin: &str) -> Option<JobSpec> {
    let s = Settings::current();
    let mut spec = JobSpec {
        op,
        dest: paths::to_long(dest),
        verify: s.verify_copies,
        policy: s.policy,
        origin: origin.into(),
        ..JobSpec::new(op)
    };
    drop(s);
    for src in sources {
        if !src.is_white_space() {
            spec.sources.push(paths::to_long(src));
        }
    }
    (!spec.sources.is_empty() && !spec.dest.is_empty()).then_some(spec)
}

/// Pastes the files on the clipboard into a folder (right-click "Paste with Choppy").
fn paste_into(dest: Option<&WString>, origin: &str) {
    let dest = dest.filter(|d| !d.is_empty());
    let files = if dest.is_some() { clipboard_files::read() } else { None };
    let (Some(dest), Some(f)) = (dest, files) else {
        dialogs::show(main_win(), "Nothing to paste", "Copy or cut some files first, then paste them here.", "OK", None, |_| {});
        return;
    };
    let Some(spec) = new_spec(if f.moved { OpKind::Move } else { OpKind::Copy }, &f.files, dest, origin) else { return };
    manager().enqueue(spec);
    show_main(true); // a new copy (running or queued): the window comes up on top
    // like Explorer: files that were cut are pasted once
    if f.moved && !app_paths::test_mode() {
        crate::sys::clipboard_clear();
    }
}

/// The Ctrl+V hook took a paste in this Explorer window (on the UI thread).
fn on_explorer_paste(explorer_window: isize) {
    // the folder the window shows (asked of Explorer itself; it can be slow or busy for a moment: asked again)
    let mut dest = None;
    let mut is_virtual = false;
    for i in 0..3 {
        if dest.is_some() {
            break;
        }
        if i > 0 {
            crate::sys::sleep_ms(120);
        }
        (dest, is_virtual) = shell_location::folder_of(explorer_window);
        if is_virtual {
            break;
        }
    }
    let Some(dest) = dest else {
        // This PC, Libraries, a zip folder...: Explorer's own paste
        paste_log::write(if is_virtual {
            "Pasted by Windows: the window shows a place that isn't a folder (This PC, a library, a zip...)"
        } else {
            "Pasted by Windows: Explorer didn't say which folder the window shows"
        });
        paste_hook::replay_paste();
        return;
    };
    let (files, busy) = clipboard_files::read_busy();
    let Some(f) = files else {
        paste_log::write(if busy {
            "Pasted by Windows: another program kept the clipboard open"
        } else {
            "Pasted by Windows: no files on the clipboard any more"
        });
        paste_hook::replay_paste();
        return;
    };
    let n = f.files.len();
    let what = format!("Choppy: {}{n}{} to {dest}", if f.moved { "move " } else { "copy " }, if n == 1 { " item" } else { " items" });
    let Some(spec) = new_spec(if f.moved { OpKind::Move } else { OpKind::Copy }, &f.files, &dest, "Explorer") else {
        paste_log::write(&what);
        return;
    };
    manager().enqueue(spec);
    show_main(true);
    paste_log::write(&format!("{what} (window up {} ms after the key)", paste_hook::ms_since_press()));
    if f.moved && !app_paths::test_mode() {
        crate::sys::clipboard_clear();
    }
}

// ================= windows =================

/// Shows the copy window. lift = a new copy was started: the window comes up above every other window, then after
/// a moment goes back to normal stacking (it stays in front until the user clicks something else).
pub fn show_main(lift: bool) {
    let park = crate::testhook::park();
    let w = match main_window() {
        Some(w) => w,
        None => {
            with(|a| a.window_built = true);
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

// ---------- a quick first window ----------

/// The first window of a process is slower to build: Direct2D and DirectWrite set up the window's target, its fonts
/// and text layouts then. A Choppy that waits in the tray builds one off-screen a few seconds after it starts and
/// closes it again, so the first copy's window is as quick as the rest. The Explorer folder lookup is run once too.
fn schedule_warm_up() {
    if with(|a| a.window_built || a.elevated).unwrap_or(true) {
        return;
    }
    win::timer(3000, true, Rc::new(|| win::idle(Box::new(warm_up))));
}

fn warm_up() {
    if with(|a| a.window_built || a.main.is_some()).unwrap_or(true) {
        return;
    }
    with(|a| a.window_built = true);
    main_window::warm_up(manager());
    if !app_paths::test_mode() {
        let _ = shell_location::folder_of(0);
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

// ================= resume after a crash =================

/// The unfinished copies (the PC restarted or Choppy was closed) are offered one after another.
fn offer_resume() {
    if with(|a| std::mem::replace(&mut a.resume_offered, true)).unwrap_or(true) {
        return;
    }
    let m = manager();
    let mut known: std::collections::HashSet<String> = m.waiting_snapshot().iter().map(|s| s.id.clone()).collect();
    if let Some(c) = m.current() {
        known.insert(c.spec.id.clone());
    }
    let list: Vec<JobSpec> =
        crate::engine::settings::journal::load_unfinished().into_iter().filter(|s| !known.contains(&s.id)).collect();
    offer_next(list.into_iter());
}

fn offer_next(mut rest: std::vec::IntoIter<JobSpec>) {
    let Some(mut spec) = rest.next() else {
        maybe_exit();
        return;
    };
    let what = format!("{} to {}", spec.title(), paths::to_short(&spec.dest));
    let text = format!(
        "{what} stopped before it finished (the PC restarted or Choppy was closed). Resuming keeps every file already copied and copies only the rest."
    );
    dialogs::show(main_win(), "Resume the unfinished copy?", &text, "Resume", Some("Forget it"), move |r| {
        if r == 1 {
            spec.resume_mode = true;
            manager().enqueue(spec);
        } else {
            crate::engine::settings::journal::remove(&spec);
        }
        offer_next(rest);
    });
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
        paste_hook::stop();
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

/// The takeover as the settings say now: on = in the tray watching Ctrl+V (the hook, a quick first window); off = the
/// hook stops, and an idle tray Choppy goes.
pub fn apply_takeover() {
    if elevated() || with(|_| ()).is_none() {
        return;
    }
    let on = Settings::current().explorer_takeover;
    if on {
        with(|a| a.resident = true);
        // a tray Choppy that ends without quitting is started again, once it is completely gone (guard.rs)
        crate::guard::start();
        ensure_tray();
        if !app_paths::test_mode() {
            paste_hook::start(Some(on_explorer_paste), false);
        }
        schedule_warm_up();
    } else {
        paste_hook::stop();
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
