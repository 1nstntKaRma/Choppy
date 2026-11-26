//! One Choppy per user (App.cs Instance + Ipc, step 5): the user's SID for the names, the instance mutex, and the
//! named pipe a second start hands its command line through - created, connected, read and written as .NET's
//! NamedPipeServerStream / NamedPipeClientStream do it.

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
    GetLastError, HANDLE, HLOCAL, LocalFree, WAIT_ABANDONED, WAIT_OBJECT_0,
};
use windows::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW};
use windows::Win32::Security::{GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_MODE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
};
use windows::Win32::System::Pipes::{ConnectNamedPipe, CreateNamedPipeW, PIPE_TYPE_BYTE, WaitNamedPipeW};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::System::Threading::{
    CreateMutexW, GetCurrentProcess, OpenMutexW, OpenProcessToken, ReleaseMutex, SYNCHRONIZATION_SYNCHRONIZE,
    WaitForSingleObject,
};
use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
use windows::core::{HSTRING, PCWSTR, PWSTR};

use super::Handle;

/// WindowsIdentity.GetCurrent().User.Value: this process's user as "S-1-5-21-...".
pub fn user_sid() -> Option<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).ok()?;
        let token = Owned(token);
        let mut len = 0u32;
        let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut len);
        if len == 0 {
            return None;
        }
        // TOKEN_USER holds a pointer: the buffer aligned for it
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        GetTokenInformation(token.0, TokenUser, Some(buf.as_mut_ptr() as *mut _), len, &mut len).ok()?;
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut s).ok()?;
        let text = s.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(s.0 as *mut _)));
        text
    }
}

struct Owned(HANDLE);
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// The instance mutex (System.Threading.Mutex): closed when dropped (Windows releases it, as abandoned, if this
/// thread still owns it then).
pub struct NamedMutex(HANDLE);

// the handle may travel; ownership stays with the thread that took it, as in .NET
unsafe impl Send for NamedMutex {}

/// What a wait of 0 ms found (Mutex.WaitOne(0)).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wait {
    Owned,
    /// left behind by a thread that ended without releasing it (.NET's AbandonedMutexException): owned now as well
    Abandoned,
    Busy,
}

impl NamedMutex {
    /// new Mutex(initiallyOwned, name, out createdNew): CreateMutex; a mutex this user may not create is opened
    /// instead (as .NET does on ERROR_ACCESS_DENIED). created = it didn't exist (and, initially owned, is ours).
    pub fn create(name: &str, initially_owned: bool) -> Option<(NamedMutex, bool)> {
        let n = HSTRING::from(name);
        unsafe {
            match CreateMutexW(None, initially_owned, &n) {
                Ok(h) => {
                    let created = GetLastError() != ERROR_ALREADY_EXISTS;
                    Some((NamedMutex(h), created))
                }
                Err(_) if GetLastError() == ERROR_ACCESS_DENIED => {
                    let h = OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE | windows::Win32::System::Threading::SYNCHRONIZATION_ACCESS_RIGHTS(1), false, &n)
                        .ok()?;
                    Some((NamedMutex(h), false))
                }
                Err(_) => None,
            }
        }
    }

    pub fn wait0(&self) -> Wait {
        let w = unsafe { WaitForSingleObject(self.0, 0) };
        if w == WAIT_OBJECT_0 {
            Wait::Owned
        } else if w == WAIT_ABANDONED {
            Wait::Abandoned
        } else {
            Wait::Busy
        }
    }

    pub fn release(&self) -> bool {
        unsafe { ReleaseMutex(self.0).is_ok() }
    }
}

impl Drop for NamedMutex {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn pipe_path(name: &str) -> HSTRING {
    HSTRING::from(format!(r"\\.\pipe\{name}"))
}

/// One server instance of the pipe (NamedPipeServerStream(name, InOut, MaxAllowedServerInstances, Byte, None, 4096,
/// 4096, security)): duplex, bytes, blocking, up to 255 instances, and a DACL that lets only `sid` in (FullControl).
pub fn pipe_server(name: &str, sid: &str) -> Result<Handle, u32> {
    // PipeAccessRights.FullControl = 0x1F019F (no FILE_EXECUTE / FILE_DELETE_CHILD)
    let sddl = HSTRING::from(format!("D:(A;;0x1f019f;;;{sid})"));
    unsafe {
        let mut sd = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(PCWSTR(sddl.as_ptr()), 1, &mut sd, None).map_err(|e| super::win32_code(&e))?;
        let sa = SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sd.0, bInheritHandle: false.into() };
        let h = CreateNamedPipeW(&pipe_path(name), PIPE_ACCESS_DUPLEX, PIPE_TYPE_BYTE, 255, 4096, 4096, 0, Some(&sa));
        let err = GetLastError();
        let _ = LocalFree(Some(HLOCAL(sd.0)));
        if h.is_invalid() {
            return Err(err.0);
        }
        Ok(Handle(h))
    }
}

/// WaitForConnection: blocks until a client opens this instance (one that came first counts too).
pub fn pipe_wait_for_client(h: &Handle) -> Result<(), u32> {
    unsafe {
        if ConnectNamedPipe(h.0, None).is_ok() {
            return Ok(());
        }
        let e = GetLastError();
        if e == ERROR_PIPE_CONNECTED { Ok(()) } else { Err(e.0) }
    }
}

/// WaitForPipeDrain: until the client has read everything written (FlushFileBuffers on a pipe).
pub fn pipe_drain(h: &Handle) -> Result<(), u32> {
    super::flush(h)
}

/// NamedPipeClientStream(".", name, InOut).Connect(timeout), as .NET Framework 4.8 does it: WaitNamedPipe, then open;
/// a pipe that isn't there yet (or was taken in between) is asked again until the time is up.
pub fn pipe_client(name: &str, timeout_ms: u32) -> Result<Handle, u32> {
    let path = pipe_path(name);
    let start = unsafe { GetTickCount() };
    let mut elapsed = 0u32;
    loop {
        unsafe {
            if !WaitNamedPipeW(&path, timeout_ms.saturating_sub(elapsed)).as_bool() {
                let e = GetLastError();
                if e == ERROR_FILE_NOT_FOUND {
                    // the server isn't created yet (a Choppy starting up or ending): asked again after a moment -
                    // C#'s loop asked again at once and kept a core busy for up to 1.5 s (fix list 6.31)
                    super::sleep_ms(15);
                } else if e.0 == 0 {
                    break;
                } else {
                    return Err(e.0);
                }
            } else {
                match CreateFileW(
                    &path,
                    super::GENERIC_READ | super::GENERIC_WRITE,
                    FILE_SHARE_MODE(0),
                    None,
                    OPEN_EXISTING,
                    FILE_FLAGS_AND_ATTRIBUTES(0),
                    None,
                ) {
                    Ok(h) => return Ok(Handle(h)),
                    Err(e) => {
                        let code = super::win32_code(&e);
                        if code != ERROR_PIPE_BUSY.0 {
                            return Err(code);
                        }
                    }
                }
            }
        }
        elapsed = unsafe { GetTickCount() }.wrapping_sub(start);
        if elapsed >= timeout_ms {
            break;
        }
    }
    Err(super::ERROR_SEM_TIMEOUT) // .NET's TimeoutException
}

/// AllowSetForegroundWindow(ASFW_ANY): the running Choppy may bring its window to the front (this start came from
/// the user's own click).
pub fn allow_set_foreground_any() {
    unsafe {
        let _ = AllowSetForegroundWindow(ASFW_ANY);
    }
}

/// ReadFile on a pipe as PipeStream.Read: a closed or broken pipe is the end (0), not an error.
pub fn pipe_read(h: &Handle, buf: &mut [u8]) -> Result<usize, u32> {
    const ERROR_BROKEN_PIPE: u32 = 109;
    const ERROR_PIPE_NOT_CONNECTED: u32 = 233;
    match super::read(h, buf) {
        Ok(n) => Ok(n as usize),
        Err(ERROR_BROKEN_PIPE) | Err(ERROR_PIPE_NOT_CONNECTED) => Ok(0),
        Err(e) => Err(e),
    }
}

/// WriteFile until all of `data` is written.
pub fn pipe_write(h: &Handle, data: &[u8]) -> Result<(), u32> {
    let mut at = 0;
    while at < data.len() {
        let n = super::write(h, &data[at..])? as usize;
        if n == 0 {
            return Err(super::ERROR_INVALID_HANDLE);
        }
        at += n;
    }
    Ok(())
}
