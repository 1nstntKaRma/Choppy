//! Updates (2.02). Choppy looks for a newer release on GitHub as the settings say (Check for updates: at startup, some
//! seconds after it, or every few days; the tray Choppy, or a Choppy whose settings are open) and shows one: a green
//! dot on the tray icon, "Update to vX" in its menu, a glowing label at the top of the settings. A click on either
//! updates with no more questions: res/update.ps1 (built into the exe) downloads the release's zip, checks it against
//! the release's SHA-256 sums and runs its build.ps1 -Shell Update - the install ChocoPlus runs, keeping every
//! setting. That quits this Choppy, replaces the exe and starts the new one, which then says it is updated
//! (`--updated`), or this one again with what went wrong (`--update-failed`).
//!
//! A copy or a Benchmark that runs is never stopped for it: the update starts once they are done. The administrator copy never
//! looks. Test mode never goes online: CHOPPY_FAKE_UPDATE=v9.9 stands in for GitHub's answer, and a click there only
//! notes that it would update.
#![deny(unsafe_code)]

use std::cell::RefCell;
use std::os::windows::process::CommandExt;
use std::time::{Duration, Instant};

use crate::engine::settings::{Settings, UpdateWhen, app_paths};
use crate::sys::{self, win};

pub const REPO: &str = "1nstntKaRma/Choppy";
const SCRIPT: &str = include_str!("../res/update.ps1");
/// A look that got no answer (offline) is tried again this much later.
const RETRY: Duration = Duration::from_secs(30 * 60);
/// "Updated to vX" stays this long in the settings that open after an update.
pub const UPDATED_SHOWN: Duration = Duration::from_secs(10);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Idle,
    /// clicked while a copy runs: it starts when the copies are done
    Waiting,
    /// the script runs (this Choppy is about to be quit by it)
    Updating,
}

struct State {
    started: bool,
    timer: usize,
    checking: bool,
    /// At startup / After: this start's look got its answer
    checked_this_run: bool,
    failed_at: Option<Instant>,
    last_error: Option<String>,
    phase: Phase,
    from_settings: bool,
    just_updated: Option<(String, Instant)>,
    /// test mode: what a click would have started
    test_launch: Option<String>,
}

thread_local! {
    static STATE: RefCell<State> = const {
        RefCell::new(State {
            started: false,
            timer: 0,
            checking: false,
            checked_this_run: false,
            failed_at: None,
            last_error: None,
            phase: Phase::Idle,
            from_settings: false,
            just_updated: None,
            test_launch: None,
        })
    };
}

fn st<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(&mut s.borrow_mut()))
}

fn parts(v: &str) -> Option<Vec<u64>> {
    let v = v.trim();
    let v = v.strip_prefix(['v', 'V']).unwrap_or(v);
    if v.is_empty() {
        return None;
    }
    v.split('.').map(|p| if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) && p.len() <= 9 { p.parse().ok() } else { None }).collect()
}

/// `tag` is a later version than `than`, part by part as whole numbers (as ChocoPlus compares: 2.10 > 2.9 > 2.02).
pub fn newer(tag: &str, than: &str) -> bool {
    match (parts(tag), parts(than)) {
        (Some(mut a), Some(mut b)) => {
            let n = a.len().max(b.len());
            a.resize(n, 0);
            b.resize(n, 0);
            a > b
        }
        _ => false,
    }
}

/// A release tag as Choppy's are named ("v2.02"): the only text that goes to the script's command line.
fn valid_tag(t: &str) -> bool {
    t.len() <= 24 && t.starts_with('v') && parts(t).is_some()
}

/// The newer release to update to, when Check for updates is on and one was seen.
pub fn available() -> Option<String> {
    if crate::app::elevated() {
        return None;
    }
    let s = Settings::current();
    if !s.update_check {
        return None;
    }
    s.update_latest.clone().filter(|t| valid_tag(t) && newer(t, env!("CHOPPY_VERSION")))
}

pub fn phase() -> Phase {
    st(|s| s.phase)
}

/// What the tray's menu item and the settings' label say (None: nothing to show).
pub fn label() -> Option<String> {
    let tag = available()?;
    Some(match phase() {
        Phase::Idle => format!("Update to {tag}"),
        Phase::Waiting if bench_running() => format!("Update to {tag} after the Benchmark"),
        Phase::Waiting => format!("Update to {tag} after the copy"),
        Phase::Updating => format!("Updating to {tag}..."),
    })
}

/// "Updated to vX" for a moment after an update (the tag and how long ago).
pub fn just_updated() -> Option<(String, Duration)> {
    st(|s| s.just_updated.clone()).map(|(t, at)| (t, at.elapsed())).filter(|(_, e)| *e < UPDATED_SHOWN)
}

/// The tray tooltip when nothing copies.
pub fn idle_tip() -> String {
    match available() {
        Some(t) => format!("Choppy · Update to {t}"),
        None => "Choppy".into(),
    }
}

/// The tray icon's dot, the tooltip, the settings' label.
fn refresh_ui() {
    win::tray_dot(available().map(|_| crate::ui::theme::UPDATE));
    if crate::app::main_window().is_none() {
        crate::app::set_tray_text(&idle_tip());
    }
    crate::app::settings_invalidate();
}

// ================= when to look =================

/// Starts looking in this process as the settings say (the tray Choppy; a Choppy whose settings opened). Once.
pub fn start() {
    if crate::app::elevated() || st(|s| std::mem::replace(&mut s.started, true)) {
        return;
    }
    refresh_ui(); // what an earlier look found shows at once
    win::idle(Box::new(tick));
}

/// The update settings changed: the next look is planned again, the dot and label follow.
pub fn settings_changed() {
    refresh_ui();
    if st(|s| s.started) {
        tick();
    }
}

/// Looks now when it is time, else plans the next look (at most an hour ahead: the clock is read again then, so a
/// PC that slept catches up).
fn tick() {
    let old = st(|s| std::mem::take(&mut s.timer));
    if old != 0 {
        win::kill(old);
    }
    let (on, when, secs, days, checked) = {
        let s = Settings::current();
        (s.update_check, s.update_when, s.update_after_seconds, s.update_every_days, s.update_checked)
    };
    if !on || st(|s| s.checking || s.phase == Phase::Updating) {
        return;
    }
    let done = st(|s| s.checked_this_run);
    let due = match when {
        UpdateWhen::Startup => (!done).then_some(Duration::ZERO),
        UpdateWhen::After => (!done).then(|| Duration::from_secs(secs.max(1) as u64).saturating_sub(crate::started().elapsed())),
        UpdateWhen::Every => {
            let need = days.max(1) as i64 * 86_400 * 10_000_000;
            let age = sys::now_file_time() - checked;
            // never looked, or the clock went back: now
            Some(if checked <= 0 || age < 0 || age >= need { Duration::ZERO } else { Duration::from_nanos(((need - age) * 100) as u64) })
        }
    };
    let failed = st(|s| s.failed_at);
    let due = due.map(|d| match failed {
        Some(f) => d.max(RETRY.saturating_sub(f.elapsed())),
        None => d,
    });
    match due {
        Some(d) if d.is_zero() => look(),
        Some(d) => {
            let ms = d.min(Duration::from_secs(3600)).as_millis() as u32 + 500;
            let id = win::timer(ms, true, std::rc::Rc::new(tick));
            st(|s| s.timer = id);
        }
        None => {}
    }
}

/// Asks GitHub which release is the latest (on a thread of its own; the answer comes back to the UI thread).
fn look() {
    let fake = std::env::var("CHOPPY_FAKE_UPDATE").ok().filter(|v| !v.is_empty());
    if app_paths::test_mode() && fake.is_none() {
        return;
    }
    st(|s| s.checking = true);
    let spawned = std::thread::Builder::new().name("update check".into()).spawn(move || {
        let r = std::panic::catch_unwind(|| match fake {
            Some(t) => Ok(t),
            None => sys::latest_release_tag(REPO),
        })
        .unwrap_or_else(|_| Err("the check failed".into()));
        win::post(Box::new(move || looked(r)));
    });
    if spawned.is_err() {
        st(|s| s.checking = false);
    }
}

fn looked(r: Result<String, String>) {
    st(|s| s.checking = false);
    match r {
        Ok(tag) if valid_tag(&tag) => {
            st(|s| {
                s.checked_this_run = true;
                s.failed_at = None;
                s.last_error = None;
            });
            let mut s = Settings::current();
            s.update_checked = sys::now_file_time();
            s.update_latest = Some(tag);
            s.save();
        }
        Ok(tag) => st(|s| {
            s.failed_at = Some(Instant::now());
            s.last_error = Some(format!("GitHub named the latest release \"{tag}\""));
        }),
        Err(e) => st(|s| {
            s.failed_at = Some(Instant::now());
            s.last_error = Some(e);
        }),
    }
    refresh_ui();
    tick();
}

// ================= updating =================

fn bench_running() -> bool {
    crate::app::with(|a| a.bench.clone()).flatten().is_some_and(|b| b.try_borrow().map(|b| b.c.running()).unwrap_or(true))
}

/// A copy or a Benchmark runs: quitting now would stop it.
fn busy() -> bool {
    crate::app::manager().busy() || bench_running()
}

/// The tray's "Update to vX" or the settings' label was clicked.
pub fn click(from_settings: bool) {
    if available().is_none() || phase() != Phase::Idle {
        return;
    }
    st(|s| s.from_settings = from_settings);
    if busy() {
        st(|s| s.phase = Phase::Waiting);
        refresh_ui();
        return;
    }
    launch();
}

/// The copies (or the Benchmark) are done: an update that waited for them starts now.
pub fn copies_done() {
    if phase() == Phase::Waiting && !busy() {
        launch();
    }
}

fn launch() {
    let Some(tag) = available() else {
        st(|s| s.phase = Phase::Idle);
        refresh_ui();
        return;
    };
    st(|s| s.phase = Phase::Updating);
    refresh_ui();
    let then = if st(|s| s.from_settings) { "settings" } else { "tray" };
    // test mode: only noted, or (CHOPPY_UPDATE_DRYRUN=1) the script downloads and checks, and installs nothing
    let dry = app_paths::test_mode();
    if dry {
        st(|s| s.test_launch = Some(format!("{tag} {then}")));
        if std::env::var("CHOPPY_UPDATE_DRYRUN").as_deref() != Ok("1") {
            return;
        }
    }
    match run_script(&tag, then, dry) {
        Ok(child) => {
            // the script ends after this Choppy has gone; one that ends sooner failed (it says why itself)
            let _ = std::thread::Builder::new().name("update script".into()).spawn(move || {
                let mut child = child;
                let _ = child.wait();
                win::post(Box::new(|| {
                    if phase() == Phase::Updating {
                        st(|s| s.phase = Phase::Idle);
                        refresh_ui();
                    }
                }));
            });
        }
        Err(e) => {
            st(|s| s.phase = Phase::Idle);
            refresh_ui();
            show_failed(&format!("The update couldn't start: {e}"));
        }
    }
}

/// Writes the script to %TEMP%\Choppy-update and starts it with Windows PowerShell, no window.
fn run_script(tag: &str, then: &str, dry: bool) -> Result<std::process::Child, String> {
    let dir = std::env::temp_dir().join("Choppy-update");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let script = dir.join("update.ps1");
    std::fs::write(&script, SCRIPT).map_err(|e| e.to_string())?;
    let logs = app_paths::log_dir();
    let _ = std::fs::create_dir_all(&logs);
    let root = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).unwrap_or_else(|| r"C:\Windows".into());
    let ps = root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let mut cmd = std::process::Command::new(ps);
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .args(["-Tag", tag, "-Then", then, "-Log"])
        .arg(logs.join("update.log"));
    if dry {
        cmd.arg("-DryRun");
    }
    cmd.current_dir(&dir)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| e.to_string())
}

/// `--updated <tag> settings|tray`: the new Choppy, right after the update. The settings it was started from open
/// again (they say "Updated to vX" for a moment); from the tray, a note over the tray icon.
pub fn updated(tag: &str, settings: bool) {
    let tag = if valid_tag(tag) { tag.to_string() } else { format!("v{}", env!("CHOPPY_VERSION")) };
    st(|s| s.just_updated = Some((tag.clone(), Instant::now())));
    refresh_ui();
    if settings || !crate::app::with(|a| a.tray).unwrap_or(false) {
        crate::app::show_settings(None);
    } else {
        crate::app::tray_balloon(&format!("Choppy is updated to {tag}."));
    }
}

/// `--update-failed <why>`: the script says what went wrong; this Choppy is the one that runs now.
pub fn failed(why: &str) {
    st(|s| s.phase = Phase::Idle);
    refresh_ui();
    show_failed(why);
}

fn show_failed(why: &str) {
    let text = format!("{}\n\nChoppy stays on v{} and works as before. The details are in update.log in the Logs folder.", why.trim(), env!("CHOPPY_VERSION"));
    let owner = crate::app::settings_win();
    crate::ui::dialogs::show(owner, "Couldn't update Choppy", &text, "OK", Some("Open the log"), |r| {
        if r == 2 {
            let log = app_paths::log_dir().join("update.log");
            win::shell_open(&log.display().to_string(), None);
        }
    });
}

/// The test hook's `update ...`: state | fake vX | click tray|settings | done (the copies ended) | updated vX
/// settings|tray | failed text.
pub fn for_test(arg: &str) -> String {
    let (cmd, rest) = arg.trim().split_once(' ').unwrap_or((arg.trim(), ""));
    match cmd {
        "fake" => {
            if rest.is_empty() {
                return "error fake vX".into();
            }
            looked(Ok(rest.trim().to_string()));
        }
        "click" => click(rest.trim() == "settings"),
        "done" => copies_done(),
        "updated" => {
            let (t, w) = rest.split_once(' ').unwrap_or((rest, ""));
            updated(t.trim(), w.trim() == "settings");
        }
        "failed" => failed(rest),
        "state" => {}
        _ => return "unknown update command".into(),
    }
    let s = Settings::current();
    let (on, when, secs, days, latest) = (s.update_check, s.update_when.name(), s.update_after_seconds, s.update_every_days, s.update_latest.clone());
    drop(s);
    let (avail, lab) = (available(), label());
    let (phase, launched, error, timer) = st(|x| (x.phase, x.test_launch.clone(), x.last_error.clone(), x.timer != 0));
    format!(
        "check {} {when} {secs}s {days}d | latest {} | available {} | phase {phase:?} | label {} | launched {} | planned {timer} | error {}",
        if on { "on" } else { "off" },
        latest.unwrap_or_else(|| "-".into()),
        avail.unwrap_or_else(|| "-".into()),
        lab.unwrap_or_else(|| "-".into()),
        launched.unwrap_or_else(|| "-".into()),
        error.unwrap_or_else(|| "-".into()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(newer("v2.02", "2.01"));
        assert!(newer("v2.10", "2.9"));
        assert!(newer("v3", "2.99"));
        assert!(newer("v2.01.1", "2.01"));
        assert!(!newer("v2.01", "2.01"));
        assert!(!newer("v2.1", "2.01")); // 2.1 and 2.01 are the same version, part by part
        assert!(!newer("v2.0", "2.01"));
        assert!(!newer("vx", "2.01"));
        assert!(!newer("v2..1", "2.01"));
        assert!(!newer("", "2.01"));
    }

    #[test]
    fn tags() {
        assert!(valid_tag("v2.02"));
        assert!(valid_tag("v10"));
        assert!(!valid_tag("2.02"));
        assert!(!valid_tag("v2.02;calc"));
        assert!(!valid_tag("v2.02 -DryRun"));
        assert!(!valid_tag("v-1"));
        assert!(!valid_tag("v"));
    }
}
