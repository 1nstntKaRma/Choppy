//! Processes and threads (Native.cs and what C# took from System.Diagnostics / System.Security, step 2): this
//! process's id, whether another one still runs, Choppy started as administrator (the hand-off), whether this one is,
//! the read-ahead thread's priority, keeping the PC awake, free memory, the clock and how the PC writes a date.

use core::ffi::c_void;

use windows::Win32::Foundation::{CloseHandle, HANDLE, SYSTEMTIME, WAIT_OBJECT_0};
use windows::Win32::Globalization::{
    DATE_SHORTDATE, GetDateFormatEx, GetLocaleInfoEx, GetTimeFormatEx, LOCALE_SDECIMAL, LOCALE_SSHORTTIME, LOCALE_STHOUSAND,
    TIME_FORMAT_FLAGS,
};
use windows::Win32::Security::{CheckTokenMembership, CreateWellKnownSid, PSID, WinBuiltinAdministratorsSid};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Power::{EXECUTION_STATE, SetThreadExecutionState};
use windows::Win32::Storage::FileSystem::FileTimeToLocalFileTime;
use windows::Win32::System::SystemInformation::{
    GetSystemTimeAsFileTime, GetSystemTimePreciseAsFileTime, GlobalMemoryStatusEx, MEMORYSTATUSEX,
};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, GetCurrentThread, GetExitCodeProcess, OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_SYNCHRONIZE,
    SetThreadPriority, Sleep, THREAD_PRIORITY_ABOVE_NORMAL, WaitForSingleObject,
};
use windows::Win32::UI::Shell::{SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows::core::{BOOL, PCWSTR};

use super::{LocalTime, win32_code};
use crate::wstr::WString;

/// This process's id (Process.GetCurrentProcess().Id).
pub fn current_pid() -> u32 {
    unsafe { GetCurrentProcessId() }
}

const STILL_ACTIVE: u32 = 259;

/// Process.GetProcessById(pid) + !HasExited, as .NET Framework does it: the process is opened for
/// PROCESS_QUERY_INFORMATION | SYNCHRONIZE, and one it can't open (gone, or running at a higher level) counts as not
/// running (C# caught the exception).
pub fn process_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    let Ok(h) = (unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_SYNCHRONIZE, false, pid as u32) }) else {
        return false;
    };
    let p = Process(h);
    !p.has_exited()
}

/// A process started from here (closed when dropped).
pub struct Process(HANDLE);

unsafe impl Send for Process {}

impl Process {
    /// Process.HasExited: the exit code says so, or the process handle is signaled.
    pub fn has_exited(&self) -> bool {
        if self.0.is_invalid() || self.0.0.is_null() {
            return true;
        }
        let mut code = 0u32;
        if unsafe { GetExitCodeProcess(self.0, &mut code) }.is_ok() && code != STILL_ACTIVE {
            return true;
        }
        (unsafe { WaitForSingleObject(self.0, 0) }) == WAIT_OBJECT_0
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// .NET's GetShellError: what ShellExecuteEx's hInstApp says when it set no error of its own.
fn shell_error(h_inst_app: isize) -> u32 {
    match h_inst_app {
        2 => 2,            // SE_ERR_FNF -> ERROR_FILE_NOT_FOUND
        3 => 3,            // SE_ERR_PNF -> ERROR_PATH_NOT_FOUND
        5 => 5,            // SE_ERR_ACCESSDENIED -> ERROR_ACCESS_DENIED
        8 => 8,            // SE_ERR_OOM -> ERROR_NOT_ENOUGH_MEMORY
        28..=30 => 1156,   // SE_ERR_DDETIMEOUT / DDEFAIL / DDEBUSY -> ERROR_DDE_FAIL
        26 => 32,          // SE_ERR_SHARE -> ERROR_SHARING_VIOLATION
        31 => 1155,        // SE_ERR_NOASSOC -> ERROR_NO_ASSOCIATION
        32 => 1157,        // SE_ERR_DLLNOTFOUND -> ERROR_DLL_NOT_FOUND
        other => other as u32,
    }
}

/// Starts `exe args` as administrator (ShellExecuteEx, verb "runas", in `dir`), as Process.Start with UseShellExecute
/// and Verb = "runas" does: on its own thread in a single-threaded COM apartment. Err = the Win32 error
/// (ERROR_CANCELLED = 1223 when the user says no).
pub fn shell_execute_runas(exe: &WString, args: &WString, dir: &WString) -> Result<Process, u32> {
    let (exe, args, dir) = (exe.to_wide_z(), args.to_wide_z(), dir.to_wide_z());
    let verb: Vec<u16> = "runas\0".encode_utf16().collect();
    let r = std::thread::spawn(move || -> Result<usize, u32> {
        unsafe {
            let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let mut info = SHELLEXECUTEINFOW {
                cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC,
                lpVerb: PCWSTR(verb.as_ptr()),
                lpFile: PCWSTR(exe.as_ptr()),
                lpParameters: PCWSTR(args.as_ptr()),
                lpDirectory: PCWSTR(dir.as_ptr()),
                nShow: 1, // SW_SHOWNORMAL
                ..Default::default()
            };
            let r = ShellExecuteExW(&mut info).map(|_| info.hProcess.0 as usize).map_err(|e| match win32_code(&e) {
                0 => shell_error(info.hInstApp.0 as isize),
                code => code,
            });
            if hr.is_ok() {
                CoUninitialize();
            }
            r
        }
    })
    .join()
    .unwrap_or(Err(super::ERROR_ACCESS_DENIED))?;
    Ok(Process(HANDLE(r as *mut c_void)))
}

/// WindowsPrincipal.IsInRole(Administrator): this process's token counts as a member of Administrators (an
/// administrator's own Choppy without "Run as administrator" doesn't).
pub fn is_admin() -> bool {
    unsafe {
        let mut sid = [0u32; 17]; // SECURITY_MAX_SID_SIZE (68 bytes), DWORD-aligned
        let mut size = size_of_val(&sid) as u32;
        let psid = PSID(sid.as_mut_ptr() as *mut c_void);
        if CreateWellKnownSid(WinBuiltinAdministratorsSid, None, Some(psid), &mut size).is_err() {
            return false;
        }
        let mut member = BOOL(0);
        CheckTokenMembership(None, psid, &mut member).is_ok() && member.as_bool()
    }
}

/// ThreadPriority.AboveNormal for the calling thread (the read-ahead thread).
pub fn set_thread_priority_above_normal() {
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
    }
}

/// Thread.Sleep(ms) as .NET Framework does it: Win32 Sleep, which wakes on the system timer's tick (15.6 ms unless this
/// process asked for finer), so 100 ms is about 109 here. Rust's std::thread::sleep waits on a high-resolution timer
/// (exactly 100 ms): the engine's naps then end at other moments than the C#'s - the idle workers' naps decide when a
/// copy ends, so its "done" moved by up to a nap (found by step 3's speed proof). Every sleep the C# makes goes here.
pub fn sleep_ms(ms: u32) {
    unsafe { Sleep(ms) }
}

/// SetThreadExecutionState for the calling thread (ES_CONTINUOUS | ES_SYSTEM_REQUIRED: the PC doesn't sleep).
pub fn set_thread_execution_state(flags: u32) {
    unsafe {
        SetThreadExecutionState(EXECUTION_STATE(flags));
    }
}

/// GlobalMemoryStatusEx: (free RAM, free commit) in bytes. None = it couldn't tell.
pub fn memory_status() -> Option<(u64, u64)> {
    let mut m = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    unsafe { GlobalMemoryStatusEx(&mut m) }.ok()?;
    Some((m.ullAvailPhys, m.ullAvailPageFile))
}

/// The time now as a FILETIME (UTC, 100 ns since 1601).
pub fn now_file_time() -> i64 {
    let f = unsafe { GetSystemTimeAsFileTime() };
    (((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64) as i64
}

/// The time now as a FILETIME, to the microsecond (GetSystemTimePreciseAsFileTime): the Benchmark's child tells its
/// parent when its copy ended (step 3), in a clock both processes read.
pub fn now_precise_file_time() -> i64 {
    let f = unsafe { GetSystemTimePreciseAsFileTime() };
    (((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64) as i64
}

/// The local time now as a FILETIME (DateTime.Now's ticks less 1600 years' worth).
pub fn now_local_file_time() -> i64 {
    let f = unsafe { GetSystemTimeAsFileTime() };
    let mut l = f;
    if unsafe { FileTimeToLocalFileTime(&f, &mut l) }.is_err() {
        l = f;
    }
    (((l.dwHighDateTime as u64) << 32) | l.dwLowDateTime as u64) as i64
}

/// DateTime.ToString() in the user's own format: the short date, a space, the long time ("11/25/2026 5:08:09 PM"),
/// as Windows writes them for the user's locale (the patterns .NET's CurrentCulture takes from it).
pub fn general_date_time(t: &LocalTime) -> String {
    let st = SYSTEMTIME {
        wYear: t.year,
        wMonth: t.month,
        wDayOfWeek: 0,
        wDay: t.day,
        wHour: t.hour,
        wMinute: t.minute,
        wSecond: t.second,
        wMilliseconds: t.millisecond,
    };
    let mut d = [0u16; 128];
    let mut h = [0u16; 128];
    unsafe {
        let dn = GetDateFormatEx(PCWSTR::null(), DATE_SHORTDATE, Some(&st), PCWSTR::null(), Some(&mut d), PCWSTR::null());
        let tn = GetTimeFormatEx(PCWSTR::null(), TIME_FORMAT_FLAGS(0), Some(&st), PCWSTR::null(), Some(&mut h));
        let date = WString::from_nul_terminated(&d[..dn.max(0) as usize]);
        let time = WString::from_nul_terminated(&h[..tn.max(0) as usize]);
        format!("{date} {time}")
    }
}

/// What .NET's CurrentCulture takes from the user's locale for numbers and the clock: the decimal point, the
/// thousands separator, and the short time pattern ("h:mm tt", "HH:mm").
pub struct LocaleInfo {
    pub decimal: String,
    pub group: String,
    pub short_time: String,
}

pub fn locale_info() -> LocaleInfo {
    let get = |what: u32, default: &str| -> String {
        let mut b = [0u16; 64];
        let n = unsafe { GetLocaleInfoEx(PCWSTR::null(), what, Some(&mut b)) };
        if n <= 1 { default.to_string() } else { String::from_utf16_lossy(&b[..(n - 1) as usize]) }
    };
    LocaleInfo {
        decimal: get(LOCALE_SDECIMAL, "."),
        group: get(LOCALE_STHOUSAND, ","),
        short_time: get(LOCALE_SSHORTTIME, "h:mm tt"),
    }
}

/// The UI thread is a COM single-threaded apartment (WPF's STA): the shell's thumbnails, the taskbar button, WIC.
pub fn com_init_sta() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

/// A process with this exe name runs (Process.GetProcessesByName: any session of this user's view).
pub fn process_running(exe: &str) -> bool {
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return false };
        let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut found = false;
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let n = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            if String::from_utf16_lossy(&e.szExeFile[..n]).eq_ignore_ascii_case(exe) {
                found = true;
                break;
            }
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
        found
    }
}

// ================= step 5: the other copier, the revive, a crash =================

/// The ids of the running processes with this exe name (Process.GetProcessesByName, which matches without ".exe").
pub fn process_ids(exe: &str) -> Vec<u32> {
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS};
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let n = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            if String::from_utf16_lossy(&e.szExeFile[..n]).eq_ignore_ascii_case(exe) {
                out.push(e.th32ProcessID);
            }
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// Process.MainModule.FileName: None where .NET threw (a process this user can't read, e.g. one running as
/// administrator: it is opened for PROCESS_QUERY_INFORMATION | PROCESS_VM_READ as .NET does).
pub fn process_exe(pid: u32) -> Option<WString> {
    use windows::Win32::System::Threading::{PROCESS_NAME_WIN32, PROCESS_VM_READ, QueryFullProcessImageNameW};
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid).ok()?;
        let p = Process(h);
        let mut buf = vec![0u16; 32768];
        let mut n = buf.len() as u32;
        QueryFullProcessImageNameW(p.0, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut n).ok()?;
        Some(WString::from_units(buf[..n as usize].to_vec()))
    }
}

/// Process.WaitForExit(ms) on a process by id: true once it has ended (or is gone, or can't be waited on).
pub fn wait_process(pid: u32, ms: u32) -> bool {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) else { return true };
        let p = Process(h);
        WaitForSingleObject(p.0, ms) != windows::Win32::Foundation::WAIT_TIMEOUT
    }
}

/// Process.CloseMainWindow: WM_CLOSE to the process's main window (its first visible top-level window without an
/// owner). False = it has none.
pub fn close_main_window(pid: u32) -> bool {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GW_OWNER, GetWindow, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE};
    struct Find {
        pid: u32,
        found: isize,
    }
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        unsafe {
            let f = &mut *(l.0 as *mut Find);
            let mut p = 0u32;
            GetWindowThreadProcessId(h, Some(&mut p));
            if p == f.pid && IsWindowVisible(h).as_bool() && GetWindow(h, GW_OWNER).is_err() {
                f.found = h.0 as isize;
                return BOOL(0);
            }
            BOOL(1)
        }
    }
    let mut f = Find { pid, found: 0 };
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut f as *mut Find as isize));
        if f.found == 0 {
            return false;
        }
        PostMessageW(Some(HWND(f.found as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0)).is_ok()
    }
}

/// The path of this exe (Process.GetCurrentProcess().MainModule.FileName).
pub fn current_exe() -> WString {
    std::env::current_exe().map(|p| WString::from_os(p.as_os_str())).unwrap_or_default()
}

static CRASH: std::sync::OnceLock<fn()> = std::sync::OnceLock::new();

/// AppDomain.UnhandledException for what Rust can't catch (an access violation...): `f` runs once, then Windows ends
/// the process as it would have.
pub fn on_crash(f: fn()) {
    use windows::Win32::System::Diagnostics::Debug::{EXCEPTION_POINTERS, SetUnhandledExceptionFilter};
    unsafe extern "system" fn filter(_e: *const EXCEPTION_POINTERS) -> i32 {
        if let Some(f) = CRASH.get() {
            let _ = std::panic::catch_unwind(f);
        }
        0 // EXCEPTION_CONTINUE_SEARCH
    }
    let _ = CRASH.set(f);
    unsafe {
        SetUnhandledExceptionFilter(Some(filter));
    }
}

// ================= the guard (2.0) =================

/// A named manual-reset event (closed when dropped): a tray Choppy's "I quit on purpose" for its guard.
pub struct NamedEvent(HANDLE);

unsafe impl Send for NamedEvent {}
unsafe impl Sync for NamedEvent {}

impl NamedEvent {
    /// Made (not set yet), or opened when it is already there.
    pub fn create(name: &str) -> Option<NamedEvent> {
        use windows::Win32::System::Threading::CreateEventW;
        let n = windows::core::HSTRING::from(name);
        unsafe { CreateEventW(None, true, false, &n) }.ok().map(NamedEvent)
    }

    /// Opened for waiting; None when no process has it any more.
    pub fn open(name: &str) -> Option<NamedEvent> {
        use windows::Win32::System::Threading::{OpenEventW, SYNCHRONIZATION_SYNCHRONIZE};
        let n = windows::core::HSTRING::from(name);
        unsafe { OpenEventW(SYNCHRONIZATION_SYNCHRONIZE, false, &n) }.ok().map(NamedEvent)
    }

    pub fn set(&self) {
        unsafe {
            let _ = windows::Win32::System::Threading::SetEvent(self.0);
        }
    }
}

impl Drop for NamedEvent {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// How the process a guard waits for ended.
pub enum GuardEnd {
    /// it said it quits on purpose (its event), or it can't be waited for
    Quit,
    /// it is completely gone without saying so: its exit code and how long it ran (seconds)
    Gone { code: u32, ran: f64 },
}

/// Waits - without a time limit, using no CPU - until `quit` is set or process `pid` has completely ended. Nothing is
/// asked of that process and nothing is done to it.
pub fn wait_guarded(pid: u32, quit: &NamedEvent) -> GuardEnd {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetProcessTimes, INFINITE, PROCESS_QUERY_LIMITED_INFORMATION, WaitForMultipleObjects};
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return GuardEnd::Quit };
        let p = Process(h);
        // the event first: when both are signaled (it quit, then ended), the quit counts
        let r = WaitForMultipleObjects(&[quit.0, p.0], false, INFINITE);
        if r == WAIT_OBJECT_0 || (WaitForSingleObject(quit.0, 0)) == WAIT_OBJECT_0 {
            return GuardEnd::Quit;
        }
        if r.0 != WAIT_OBJECT_0.0 + 1 {
            return GuardEnd::Quit; // a failed wait: never a reason to start anything
        }
        let mut code = 0u32;
        let _ = GetExitCodeProcess(p.0, &mut code);
        let (mut created, mut exited, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let ran = if GetProcessTimes(p.0, &mut created, &mut exited, &mut k, &mut u).is_ok() {
            let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
            t(exited).saturating_sub(t(created)) as f64 / 1e7
        } else {
            0.0
        };
        GuardEnd::Gone { code, ran }
    }
}

/// Windows is signing out or shutting down (SM_SHUTTINGDOWN).
pub fn shutting_down() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_SHUTTINGDOWN};
    unsafe { GetSystemMetrics(SM_SHUTTINGDOWN) != 0 }
}

/// This process waits in the background: lowest priority, its memory given back.
pub fn background_quiet() {
    use windows::Win32::System::Threading::{GetCurrentProcess, IDLE_PRIORITY_CLASS, SetPriorityClass, SetProcessWorkingSetSize};
    unsafe {
        let _ = SetPriorityClass(GetCurrentProcess(), IDLE_PRIORITY_CLASS);
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}
