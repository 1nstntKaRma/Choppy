//! What the Benchmark needs from Windows (step 4): its one-at-a-time mutex, the machine's description (HKLM, memory),
//! a drive's size and free space and a network drive's share, sequential reads, a hidden child process, and Windows'
//! own copy (IFileOperation, no UI).

use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::process::CommandExt;
use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::NetworkManagement::WNet::WNetGetConnectionW;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use windows::Win32::UI::Shell::{FileOperation, IFileOperation, IShellItem, SHCreateItemFromParsingName};
use windows::core::{HSTRING, PCWSTR};

use crate::wstr::WString;

/// A named mutex held by this thread (BenchFiles.Held): released and closed when dropped.
pub struct Held(HANDLE);

impl Drop for Held {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

/// Takes a named mutex if no one else holds it (Mutex.WaitOne(0); one left by a process that was killed counts as
/// free). The same thread may take it again.
pub fn named_mutex_try(name: &str) -> Option<Held> {
    unsafe {
        let h = CreateMutexW(None, false, &HSTRING::from(name)).ok()?;
        let w = WaitForSingleObject(h, 0);
        if w == WAIT_OBJECT_0 || w == WAIT_ABANDONED {
            return Some(Held(h));
        }
        let _ = CloseHandle(h);
        None
    }
}

/// A REG_SZ value under HKEY_LOCAL_MACHINE.
pub fn hklm_string(subkey: &str, name: &str) -> Option<String> {
    let (k, n) = (HSTRING::from(subkey), HSTRING::from(name));
    let mut buf = vec![0u16; 1024];
    let mut size = (buf.len() * 2) as u32;
    let e = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, &k, &n, RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut size)) };
    if e.is_err() {
        return None;
    }
    Some(WString::from_nul_terminated(&buf).to_string_lossy())
}

/// A REG_DWORD value under HKEY_LOCAL_MACHINE.
pub fn hklm_dword(subkey: &str, name: &str) -> Option<u32> {
    let (k, n) = (HSTRING::from(subkey), HSTRING::from(name));
    let mut v = 0u32;
    let mut size = 4u32;
    let e = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, &k, &n, RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as *mut _), Some(&mut size)) };
    if e.is_err() { None } else { Some(v) }
}

/// The PC's whole memory in bytes (MEMORYSTATUSEX.ullTotalPhys).
pub fn total_memory() -> Option<u64> {
    let mut m = MEMORYSTATUSEX { dwLength: size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    unsafe { GlobalMemoryStatusEx(&mut m) }.ok()?;
    Some(m.ullTotalPhys)
}

/// DriveInfo.TotalSize and AvailableFreeSpace (the free space this user may use).
pub fn disk_space(root: &WString) -> Option<(u64, u64)> {
    let p = root.to_wide_z();
    let (mut free, mut total, mut all_free) = (0u64, 0u64, 0u64);
    unsafe { GetDiskFreeSpaceExW(PCWSTR(p.as_ptr()), Some(&mut free), Some(&mut total), Some(&mut all_free)) }.ok()?;
    Some((total, free))
}

/// The share a network drive letter is mapped to (WNetGetConnection), or None.
pub fn wnet_remote_name(letter: &str) -> Option<String> {
    let mut buf = vec![0u16; 512];
    let mut n = buf.len() as u32;
    let e = unsafe { WNetGetConnectionW(&HSTRING::from(letter), Some(windows::core::PWSTR(buf.as_mut_ptr())), &mut n) };
    if e.0 != 0 {
        return None;
    }
    Some(WString::from_nul_terminated(&buf).to_string_lossy())
}

/// A file opened to be read from start to end (FileOptions.SequentialScan).
pub fn open_sequential(p: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().read(true).share_mode(1).custom_flags(super::FILE_FLAG_SEQUENTIAL_SCAN).open(p)
}

/// A child process without a console window (CreateNoWindow).
pub fn spawn_hidden(exe: &Path, args: &[&str]) -> std::io::Result<std::process::Child> {
    std::process::Command::new(exe).args(args).creation_flags(0x08000000).spawn()
}

/// IFileOperation: copies one file or folder into a folder with the shell's copy engine, FOF_NO_UI (silent, no
/// confirmations, no error UI, folders made without asking) + FOF_ALLOWUNDO as a paste in Explorer. True = the
/// engine aborted something. Err = the COM error's message.
pub fn shell_copy_item(src: &str, dest_dir: &str) -> Result<bool, String> {
    const FOF_NO_UI: u32 = 0x0004 | 0x0010 | 0x0200 | 0x0400;
    const FOF_ALLOWUNDO: u32 = 0x0040;
    unsafe {
        let r = (|| -> windows::core::Result<bool> {
            let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
            op.SetOperationFlags(windows::Win32::UI::Shell::FILEOPERATION_FLAGS(FOF_NO_UI | FOF_ALLOWUNDO))?;
            let s: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(src), None)?;
            let d: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(dest_dir), None)?;
            op.CopyItem(&s, &d, PCWSTR::null(), None)?;
            op.PerformOperations()?;
            Ok(op.GetAnyOperationsAborted()?.as_bool())
        })();
        r.map_err(|e| {
            let m = e.message();
            if m.is_empty() { format!("Error HRESULT E_FAIL has been returned from a call to a COM component (0x{:08X}).", e.code().0) } else { m }
        })
    }
}
