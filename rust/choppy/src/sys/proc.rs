//! Processes and threads (Native.cs and what C# took from System.Diagnostics / System.Security, step 2): this
//! process's id, whether another one still runs, Choppy started as administrator (the hand-off), whether this one is,
//! the read-ahead thread's priority, keeping the PC awake, free memory, the clock and how the PC writes a date.

use core::ffi::c_void;

use windows::Win32::Foundation::{CloseHandle, HANDLE, SYSTEMTIME, WAIT_OBJECT_0};
use windows::Win32::Globalization::{DATE_SHORTDATE, GetDateFormatEx, GetTimeFormatEx, TIME_FORMAT_FLAGS};
use windows::Win32::Security::{CheckTokenMembership, CreateWellKnownSid, PSID, WinBuiltinAdministratorsSid};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Power::{EXECUTION_STATE, SetThreadExecutionState};
use windows::Win32::Storage::FileSystem::FileTimeToLocalFileTime;
use windows::Win32::System::SystemInformation::{
    GetSystemTimeAsFileTime, GetSystemTimePreciseAsFileTime, GlobalMemoryStatusEx, MEMORYSTATUSEX,
};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, GetCurrentThread, GetExitCodeProcess, OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_SYNCHRONIZE,
    SetThreadPriority, THREAD_PRIORITY_ABOVE_NORMAL, WaitForSingleObject,
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
