//! Step 5's tests: the instance mutex, the pipe hand-off (a second start's command line, relative paths made
//! absolute), the icon stamp, what the takeover reads from the registry, Explorer asked both ways. Never the real
//! instance names, the real Run key or the user's Explorer windows (only counted).

use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Duration;

use crate::engine::paths;
use crate::sys::{self, NamedMutex, Wait};
use crate::wstr::WString;

fn unique(tag: &str) -> String {
    format!("Choppy-rust-test-{tag}-{}", crate::engine::model::new_id())
}

#[test]
fn instance_mutex_first_owns_then_others_wait() {
    let name = format!(r"Local\{}", unique("mutex"));
    let (m, created) = NamedMutex::create(&name, true).expect("mutex");
    assert!(created, "the first start creates it (and owns it)");
    let n2 = name.clone();
    // another start (another thread here: ownership is per thread) finds it taken
    let other = std::thread::spawn(move || {
        let (m2, created2) = NamedMutex::create(&n2, true).expect("mutex");
        (created2, m2.wait0())
    })
    .join()
    .unwrap();
    assert_eq!(other, (false, Wait::Busy));
    // the first one quits: the next start takes over
    assert!(m.release());
    let n3 = name.clone();
    let got = std::thread::spawn(move || {
        let (m3, _) = NamedMutex::create(&n3, true).expect("mutex");
        let w = m3.wait0();
        if w == Wait::Owned {
            m3.release();
        }
        w
    })
    .join()
    .unwrap();
    assert_eq!(got, Wait::Owned);
    // one left by a thread that ended without releasing it counts as free (AbandonedMutexException in C#)
    let n4 = name.clone();
    std::thread::spawn(move || {
        let (m4, _) = NamedMutex::create(&n4, false).expect("mutex");
        assert_eq!(m4.wait0(), Wait::Owned);
        std::mem::forget(m4); // the thread ends still owning it
    })
    .join()
    .unwrap();
    let (m5, _) = NamedMutex::create(&name, false).expect("mutex");
    assert_eq!(m5.wait0(), Wait::Abandoned);
    m5.release();
}

static GOT: Mutex<Option<mpsc::Sender<Vec<WString>>>> = Mutex::new(None);

fn got_args(a: Vec<WString>) {
    if let Some(tx) = GOT.lock().unwrap().as_ref() {
        let _ = tx.send(a);
    }
}

#[test]
fn pipe_hands_a_command_line_over_once() {
    let name = unique("pipe");
    // nothing listening yet: nothing is sent (the caller may try again)
    let t = std::time::Instant::now();
    assert!(!crate::instance::send(&name, &[WString::from("--tray")]));
    assert!(t.elapsed() >= Duration::from_millis(1400), "C#'s Connect(1500) waits for a server to appear");
    let (tx, rx) = mpsc::channel();
    *GOT.lock().unwrap() = Some(tx);
    crate::instance::listen(&name, got_args);
    let start = paths::start_dir().cloned().unwrap_or_else(|| WString::from(std::env::current_dir().unwrap().as_os_str().to_string_lossy().as_ref()));
    let args: Vec<WString> = ["copy", "rel\\a b.txt", r"C:\abs\ü 😀", "-x", "dest"].iter().map(|s| WString::from(*s)).collect();
    let mut sent = false;
    for _ in 0..40 {
        if crate::instance::send(&name, &args) {
            sent = true;
            break;
        }
        sys::sleep_ms(100);
    }
    assert!(sent);
    let a: Vec<String> = rx.recv_timeout(Duration::from_secs(5)).expect("handed over").iter().map(|w| w.to_string_lossy()).collect();
    let abs = |p: &str| paths::make_abs(&WString::from(p)).to_string_lossy();
    assert_eq!(a, vec!["copy".to_string(), abs("rel\\a b.txt"), r"C:\abs\ü 😀".into(), "-x".into(), abs("dest")]);
    assert!(a[1].starts_with(&start.to_string_lossy()) || paths::start_dir().is_none());
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "sent once");
    // an empty command line arrives as no arguments (a plain start: the settings)
    assert!(crate::instance::send(&name, &[]));
    assert!(rx.recv_timeout(Duration::from_secs(5)).expect("handed over").is_empty());
}

#[test]
fn make_abs_as_ipc_did() {
    let p = |s: &str| paths::make_abs(&WString::from(s)).to_string_lossy();
    assert_eq!(p(r"C:\x\y"), r"C:\x\y"); // rooted: as it is
    assert_eq!(p(r"\\server\share\f"), r"\\server\share\f");
    assert_eq!(p(r"\x"), r"\x"); // rooted on the current drive: as it is (IsPathRooted)
    assert_eq!(p("a<b"), "a<b"); // .NET threw: as it is
    assert_eq!(p("D:rel"), "D:rel"); // IsPathRooted counts a drive letter
    assert!(p("sub\\..\\f.txt").ends_with("\\f.txt") && !p("sub\\..\\f.txt").contains(".."));
}

#[test]
fn icon_stamp_is_the_one_1_48_writes() {
    // HKCU\Software\Choppy\IconStamp as C# 1.48 wrote it here for the same icon (xxHash64 of choppy.ico)
    assert_eq!(crate::explorer::icon_refresh::stamp(), "22d2b958e52870c7");
}

#[test]
fn takeover_reads_registry_text_as_net() {
    let key = format!(r"Software\{}", unique("reg"));
    sys::reg_write_values(&key, &[("ultracopier", r#""C:\Program Files\Ultracopier\ultracopier.exe""#.into()), ("", "default".into())], &[]).unwrap();
    let r = crate::explorer::takeover::reg_string(&key, "ULTRACOPIER");
    assert_eq!(r.as_deref(), Some(r#""C:\Program Files\Ultracopier\ultracopier.exe""#)); // names in any case
    assert_eq!(crate::explorer::takeover::reg_string(&key, "").as_deref(), Some("default"));
    assert_eq!(crate::explorer::takeover::reg_string(&key, "none"), None);
    assert!(sys::reg_value_exists(&key, "ultracopier"));
    sys::reg_write_values(&key, &[], &["ultracopier", "missing"]).unwrap(); // a missing one is fine
    assert!(!sys::reg_value_exists(&key, "ultracopier"));
    sys::reg_delete_tree(&key).unwrap();
    assert_eq!(crate::explorer::takeover::reg_string(&key, ""), None);
}

#[test]
fn explorer_answers_both_ways() {
    sys::com_init_sta();
    let (a, b) = sys::shell_window_counts();
    assert!(a.is_some(), "ShellWindows through its own interface");
    assert_eq!(a, b, "Shell.Application by name (IDispatch) lists the same windows");
    // no window 0: neither way finds it (the warm-up's call)
    assert_eq!(crate::explorer::shell_location::folder_of(0), (None, false));
    // the desktop: its folder, not asked of Explorer
    let desk = sys::find_window(0, 0, "Progman");
    if desk != 0 {
        let (d, v) = crate::explorer::shell_location::folder_of(desk);
        assert!(!v && d.is_some_and(|d| !d.is_empty()));
    }
}
