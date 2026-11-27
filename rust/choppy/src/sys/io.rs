//! The copy's file calls (Native.cs, step 2): reads and writes on open handles, sizes, file information set through a
//! handle (dates + attributes, reserved size, end of file, the rename), Windows' own copy (CopyFileExW and its
//! progress callback), moves, deletes, folders, extra data streams, flushes, free space, where a file starts on the
//! disk.

use core::ffi::c_void;
use std::mem::ManuallyDrop;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use windows::Win32::Foundation::{FILETIME, HANDLE};
use windows::Win32::Storage::FileSystem::{
    COPYFILE_FLAGS, COPYPROGRESSROUTINE_PROGRESS, CopyFileExW, CreateDirectoryW, DeleteFileW, FILE_ATTRIBUTE_NORMAL,
    FILE_BEGIN, FILE_FLAGS_AND_ATTRIBUTES, FILE_INFO_BY_HANDLE_CLASS, FindClose, FindFirstStreamW, FindNextStreamW,
    FileStreamInfo, FindStreamInfoStandard, FlushFileBuffers, GetDiskFreeSpaceExW, GetFileInformationByHandleEx,
    GetFileSizeEx, LPPROGRESS_ROUTINE_CALLBACK_REASON,
    MOVE_FILE_FLAGS, MoveFileExW, MoveFileW, REPLACEFILE_WRITE_THROUGH, ReadFile, RemoveDirectoryW, ReplaceFileW,
    SetFileAttributesW, SetFileInformationByHandle, SetFilePointerEx, SetFileTime, WIN32_FIND_STREAM_DATA, WriteFile,
};
use windows::Win32::System::IO::DeviceIoControl;
use windows::core::{BOOL, PCWSTR};

use super::{FILE_ALLOCATION_INFO_CLASS, FILE_BASIC_INFO_CLASS, FILE_END_OF_FILE_INFO_CLASS, FILE_RENAME_INFO_CLASS};
use super::{FSCTL_GET_RETRIEVAL_POINTERS, Handle, win32_code};
use crate::wstr::WString;

/// GetFileSizeEx. Err = the Win32 error.
pub fn file_size(h: &Handle) -> Result<i64, u32> {
    let mut size = 0i64;
    unsafe { GetFileSizeEx(h.0, &mut size) }.map_err(|e| win32_code(&e))?;
    Ok(size)
}

/// ReadFile on a handle opened without FILE_FLAG_OVERLAPPED: Ok(bytes read; 0 = the end of the file).
pub fn read(h: &Handle, buf: &mut [u8]) -> Result<u32, u32> {
    let mut n = 0u32;
    unsafe { ReadFile(h.0, Some(buf), Some(&mut n), None) }.map_err(|e| win32_code(&e))?;
    Ok(n)
}

/// WriteFile on a handle opened without FILE_FLAG_OVERLAPPED: Ok(bytes written).
pub fn write(h: &Handle, buf: &[u8]) -> Result<u32, u32> {
    let mut n = 0u32;
    unsafe { WriteFile(h.0, Some(buf), Some(&mut n), None) }.map_err(|e| win32_code(&e))?;
    Ok(n)
}

/// SetFilePointerEx(at, FILE_BEGIN).
pub fn set_file_pointer(h: &Handle, at: i64) -> bool {
    unsafe { SetFilePointerEx(h.0, at, None, FILE_BEGIN) }.is_ok()
}

/// FlushFileBuffers: the file's data and its entry on the disk itself. Err = the Win32 error.
pub fn flush(h: &Handle) -> Result<(), u32> {
    unsafe { FlushFileBuffers(h.0) }.map_err(|e| win32_code(&e))
}

fn set_info(h: HANDLE, class: i32, info: *const c_void, size: u32) -> Result<(), u32> {
    unsafe { SetFileInformationByHandle(h, FILE_INFO_BY_HANDLE_CLASS(class), info, size) }.map_err(|e| win32_code(&e))
}

/// FileAllocationInfo: room reserved for `bytes` (the size stays). Err = the Win32 error.
pub fn set_allocation(h: &Handle, bytes: i64) -> Result<(), u32> {
    set_info(h.0, FILE_ALLOCATION_INFO_CLASS, &bytes as *const i64 as *const c_void, 8)
}

/// FileEndOfFileInfo: the file's size. Err = the Win32 error.
pub fn set_end_of_file(h: &Handle, bytes: i64) -> Result<(), u32> {
    set_info(h.0, FILE_END_OF_FILE_INFO_CLASS, &bytes as *const i64 as *const c_void, 8)
}

/// FILE_BASIC_INFO as Native.cs declares it (a closing pad keeps its size at 40, as C#'s struct had).
#[repr(C)]
struct BasicInfo {
    creation: i64,
    access: i64,
    write: i64,
    change: i64,
    attributes: u32,
    pad: u32,
}

/// FileBasicInfo: creation and modified dates and the attributes; the access and change dates stay as they are (0).
pub fn set_basic_info(h: &Handle, ctime: i64, mtime: i64, attributes: u32) -> bool {
    let bi = BasicInfo { creation: ctime, access: 0, write: mtime, change: 0, attributes, pad: 0 };
    set_info(h.0, FILE_BASIC_INFO_CLASS, &bi as *const BasicInfo as *const c_void, size_of::<BasicInfo>() as u32).is_ok()
}

// FILE_RENAME_INFO's offsets below are those of a 64-bit process (the only target: rust/.cargo/config.toml)
const _: () = assert!(size_of::<usize>() == 8);

/// Renames an open file through its handle (Native.RenameByHandle). The new name MUST be the full `\\?\` path:
/// Windows resolves a plain name against the process's current folder, not the file's folder. Never replaces an
/// existing file.
pub fn rename_by_handle(h: &Handle, new_path: &WString) -> bool {
    if new_path.is_empty() || !new_path.starts_with(r"\\?\") {
        return false;
    }
    // FILE_RENAME_INFO { union { BOOLEAN ReplaceIfExists; DWORD Flags; }; HANDLE RootDirectory; DWORD FileNameLength;
    // WCHAR FileName[1]; } - in a 64-bit process: the handle at 8, the length at 16, the name at 20
    let (off_len, off_name) = (16usize, 20usize);
    let name = new_path.units();
    let name_bytes = name.len() * 2;
    let size = off_name + name_bytes + 2;
    let mut buf = vec![0u64; size.div_ceil(8)]; // 8-byte aligned, all zero: ReplaceIfExists = FALSE, no root folder
    let bytes: &mut [u8] = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, buf.len() * 8) };
    bytes[off_len..off_len + 4].copy_from_slice(&(name_bytes as u32).to_le_bytes());
    for (i, c) in name.iter().enumerate() {
        bytes[off_name + 2 * i..off_name + 2 * i + 2].copy_from_slice(&c.to_le_bytes());
    }
    set_info(h.0, FILE_RENAME_INFO_CLASS, buf.as_ptr() as *const c_void, size as u32).is_ok()
}

fn file_time(ft: i64) -> FILETIME {
    FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft as u64 >> 32) as u32 }
}

/// SetFileTime: the creation and/or modified date (None = left as it is; the access date always is).
pub fn set_file_time(h: &Handle, creation: Option<i64>, write: Option<i64>) -> bool {
    let c = creation.map(file_time);
    let w = write.map(file_time);
    unsafe {
        SetFileTime(h.0, c.as_ref().map(|x| x as *const FILETIME), None, w.as_ref().map(|x| x as *const FILETIME))
    }
    .is_ok()
}

/// One call of Windows' copy's progress routine (CopyFileExW): the bytes so far, why it was called, and the copy's
/// own open handle (set once the destination is open).
pub struct CopyProgress<'a> {
    pub total: i64,
    pub transferred: i64,
    pub reason: u32,
    pub dest: Option<&'a Handle>,
}

struct CopyCtx<'f> {
    f: &'f mut dyn FnMut(&CopyProgress) -> u32,
    panic: Option<Box<dyn std::any::Any + Send>>,
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system" fn copy_progress(
    total: i64,
    transferred: i64,
    _stream_size: i64,
    _stream_transferred: i64,
    _stream_number: u32,
    reason: LPPROGRESS_ROUTINE_CALLBACK_REASON,
    _src: HANDLE,
    dst: HANDLE,
    data: *const c_void,
) -> COPYPROGRESSROUTINE_PROGRESS {
    let ctx = unsafe { &mut *(data as *mut CopyCtx) };
    if ctx.panic.is_some() {
        return COPYPROGRESSROUTINE_PROGRESS(super::PROGRESS_CANCEL);
    }
    // the copy's handle belongs to CopyFileEx: borrowed for this call only, never closed here
    let dest = ManuallyDrop::new(Handle(dst));
    let p = CopyProgress {
        total,
        transferred,
        reason: reason.0,
        dest: if dst.is_invalid() || dst.0.is_null() { None } else { Some(&*dest) },
    };
    // a panic must never unwind into Windows: the copy is canceled, and the panic goes on once CopyFileEx returns
    match catch_unwind(AssertUnwindSafe(|| (ctx.f)(&p))) {
        Ok(v) => COPYPROGRESSROUTINE_PROGRESS(v),
        Err(e) => {
            ctx.panic = Some(e);
            COPYPROGRESSROUTINE_PROGRESS(super::PROGRESS_CANCEL)
        }
    }
}

/// CopyFileExW with a progress routine that answers PROGRESS_CONTINUE or PROGRESS_CANCEL. Err = the Win32 error
/// (ERROR_REQUEST_ABORTED once canceled). A panic in the routine cancels the copy and is raised again here.
pub fn copy_file_ex(src: &WString, dst: &WString, flags: u32, progress: &mut dyn FnMut(&CopyProgress) -> u32) -> Result<(), u32> {
    let s = src.to_wide_z();
    let d = dst.to_wide_z();
    let mut ctx = CopyCtx { f: progress, panic: None };
    let mut cancel = BOOL(0);
    let r = unsafe {
        CopyFileExW(
            PCWSTR(s.as_ptr()),
            PCWSTR(d.as_ptr()),
            Some(copy_progress),
            Some(&mut ctx as *mut CopyCtx as *const c_void),
            Some(&mut cancel),
            COPYFILE_FLAGS(flags),
        )
    }
    .map_err(|e| win32_code(&e));
    if let Some(p) = ctx.panic.take() {
        resume_unwind(p);
    }
    r
}

/// MoveFileExW. Err = the Win32 error.
pub fn move_file_ex(from: &WString, to: &WString, flags: u32) -> Result<(), u32> {
    let a = from.to_wide_z();
    let b = to.to_wide_z();
    unsafe { MoveFileExW(PCWSTR(a.as_ptr()), PCWSTR(b.as_ptr()), MOVE_FILE_FLAGS(flags)) }.map_err(|e| win32_code(&e))
}

/// MoveFileW (File.Move: a move to another drive copies). Err = the Win32 error.
pub fn move_file(from: &WString, to: &WString) -> Result<(), u32> {
    let a = from.to_wide_z();
    let b = to.to_wide_z();
    unsafe { MoveFileW(PCWSTR(a.as_ptr()), PCWSTR(b.as_ptr())) }.map_err(|e| win32_code(&e))
}

/// ReplaceFileW(replaced, replacement, no backup, REPLACEFILE_WRITE_THROUGH) - File.Replace(replacement, replaced,
/// null) as .NET Framework calls it. Err = the Win32 error.
pub fn replace_file(replaced: &WString, replacement: &WString) -> Result<(), u32> {
    let a = replaced.to_wide_z();
    let b = replacement.to_wide_z();
    unsafe { ReplaceFileW(PCWSTR(a.as_ptr()), PCWSTR(b.as_ptr()), PCWSTR::null(), REPLACEFILE_WRITE_THROUGH, None, None) }
        .map_err(|e| win32_code(&e))
}

/// DeleteFileW. Err = the Win32 error.
pub fn delete_file(path: &WString) -> Result<(), u32> {
    let p = path.to_wide_z();
    unsafe { DeleteFileW(PCWSTR(p.as_ptr())) }.map_err(|e| win32_code(&e))
}

/// RemoveDirectoryW (only an empty folder goes).
pub fn remove_directory(path: &WString) -> bool {
    let p = path.to_wide_z();
    unsafe { RemoveDirectoryW(PCWSTR(p.as_ptr())) }.is_ok()
}

/// CreateDirectoryW. Err = the Win32 error.
pub fn create_directory(path: &WString) -> Result<(), u32> {
    let p = path.to_wide_z();
    unsafe { CreateDirectoryW(PCWSTR(p.as_ptr()), None) }.map_err(|e| win32_code(&e))
}

/// SetFileAttributesW.
pub fn set_file_attributes(path: &WString, attributes: u32) -> bool {
    let p = path.to_wide_z();
    unsafe { SetFileAttributesW(PCWSTR(p.as_ptr()), FILE_FLAGS_AND_ATTRIBUTES(attributes)) }.is_ok()
}

/// GetDiskFreeSpaceExW: the free bytes this user may use (quotas count). None = it couldn't tell.
pub fn disk_free_space_ex(dir: &WString) -> Option<u64> {
    let p = dir.to_wide_z();
    let mut free = 0u64;
    unsafe { GetDiskFreeSpaceExW(PCWSTR(p.as_ptr()), Some(&mut free), None, None) }.ok()?;
    Some(free)
}

/// The names FindFirstStreamW / FindNextStreamW list for a file ("::$DATA", ":Zone.Identifier:$DATA"). Err = the
/// error of FindFirstStreamW (ERROR_HANDLE_EOF: a file system with no streams at all).
pub fn stream_names(path: &WString) -> Result<Vec<WString>, u32> {
    let p = path.to_wide_z();
    let mut d = WIN32_FIND_STREAM_DATA::default();
    let h = unsafe { FindFirstStreamW(PCWSTR(p.as_ptr()), FindStreamInfoStandard, &mut d as *mut _ as *mut c_void, None) }
        .map_err(|e| win32_code(&e))?;
    let mut r = Vec::new();
    loop {
        r.push(WString::from_nul_terminated(&d.cStreamName));
        if unsafe { FindNextStreamW(h, &mut d as *mut _ as *mut c_void) }.is_err() {
            break;
        }
    }
    unsafe {
        let _ = FindClose(h);
    }
    Ok(r)
}

/// FSCTL_GET_RETRIEVAL_POINTERS from the file's first cluster: the cluster on the volume where its data starts (no
/// administrator rights needed). None = Windows didn't say (a small file kept inside its entry, an error).
pub fn first_lcn(h: &Handle) -> Option<i64> {
    let vcn = 0i64;
    // RETRIEVAL_POINTERS_BUFFER: count (+pad), starting VCN, then { next VCN, LCN } - the first LCN is enough
    let mut rp = [0i64; 4];
    let mut got = 0u32;
    let r = unsafe {
        DeviceIoControl(
            h.0,
            FSCTL_GET_RETRIEVAL_POINTERS,
            Some(&vcn as *const i64 as *const c_void),
            8,
            Some(rp.as_mut_ptr() as *mut c_void),
            32,
            Some(&mut got),
            None,
        )
    };
    // ERROR_MORE_DATA: more extents than fit - the first one is in
    let more = matches!(&r, Err(e) if win32_code(e) == super::ERROR_MORE_DATA);
    if (r.is_ok() || more) && (rp[0] as i32) > 0 { Some(rp[3]) } else { None }
}

/// FILE_ATTRIBUTE_NORMAL as the flags of CreateFileW take it.
pub const ATTRIBUTE_NORMAL: u32 = FILE_ATTRIBUTE_NORMAL.0;

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtSetInformationFile(h: HANDLE, iosb: *mut [usize; 2], info: *const c_void, len: u32, class: i32) -> i32;
    fn NtQueryInformationFile(h: HANDLE, iosb: *mut [usize; 2], info: *mut c_void, len: u32, class: i32) -> i32;
}

/// The open file has nothing but its data: no extended attributes (FileEaInformation - WSL keeps Linux permissions
/// there) and no extra data streams (FileStreamInfo - ":Zone.Identifier", the mark of a download). Windows' own copy
/// keeps both; a file that has either (or that can't be asked) goes that way. True = data only.
pub fn data_only(h: &Handle) -> bool {
    let mut iosb = [0usize; 2];
    let mut ea = 0u32;
    if unsafe { NtQueryInformationFile(h.0, &mut iosb, &mut ea as *mut u32 as *mut c_void, 4, 7) } < 0 || ea != 0 {
        return false;
    }
    // FILE_STREAM_INFO entries: { NextEntryOffset u32, StreamNameLength u32, StreamSize i64, StreamAllocationSize i64,
    // StreamName } - only "::$DATA" (one entry, no next) is a file of data only; ERROR_HANDLE_EOF = no streams at all
    let mut buf = [0u64; 64];
    match unsafe { GetFileInformationByHandleEx(h.0, FileStreamInfo, buf.as_mut_ptr() as *mut c_void, 512) } {
        Ok(()) => {
            let b: &[u8] = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, 512) };
            let next = u32::from_le_bytes(b[0..4].try_into().unwrap());
            let len = u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize;
            let name: Vec<u16> = (0..len / 2).map(|i| u16::from_le_bytes([b[24 + 2 * i], b[25 + 2 * i]])).collect();
            next == 0 && name == "::$DATA".encode_utf16().collect::<Vec<u16>>()
        }
        Err(e) => win32_code(&e) == super::ERROR_HANDLE_EOF,
    }
}

/// rename_by_handle for a copy on a local drive, from its temp name to its real name: when both are in one folder, the
/// plain-name rename (rename_in_folder: Windows doesn't open the folder a second time - 20,000 small files on the NVMe,
/// 8 at once: 5.5-6.9 s with the full path, 5.0-5.2 s this way, 3.3 s with no rename at all); if that is refused for
/// any reason but the name being taken, the full-path rename.
pub fn rename_local(h: &Handle, from: &WString, to: &WString) -> bool {
    let (f, t) = (from.units(), to.units());
    let slash = |u: &[u16]| u.iter().rposition(|&c| c == b'\\' as u16);
    if let (Some(a), Some(b)) = (slash(f), slash(t))
        && a == b
        && f[..a] == t[..b]
    {
        match rename_in_folder(h, &WString::from_units(t[b + 1..].to_vec())) {
            Ok(()) => return true,
            Err(STATUS_OBJECT_NAME_COLLISION) => return false,
            Err(_) => {}
        }
    }
    rename_by_handle(h, to)
}

const STATUS_OBJECT_NAME_COLLISION: i32 = 0xC0000035u32 as i32;

/// Renames an open file within its own folder (NtSetInformationFile, FileRenameInformation with a plain name and no
/// root folder: the file system renames it in place, no second open of the folder). Never replaces an existing file.
/// Err = the NTSTATUS.
pub fn rename_in_folder(h: &Handle, new_name: &WString) -> Result<(), i32> {
    let name = new_name.units();
    if name.is_empty() || name.contains(&(b'\\' as u16)) || name.contains(&(b'/' as u16)) {
        return Err(-1);
    }
    let (off_len, off_name) = (16usize, 20usize);
    let name_bytes = name.len() * 2;
    let size = off_name + name_bytes + 2;
    let mut buf = vec![0u64; size.div_ceil(8)];
    let bytes: &mut [u8] = unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr() as *mut u8, buf.len() * 8) };
    bytes[off_len..off_len + 4].copy_from_slice(&(name_bytes as u32).to_le_bytes());
    for (i, c) in name.iter().enumerate() {
        bytes[off_name + 2 * i..off_name + 2 * i + 2].copy_from_slice(&c.to_le_bytes());
    }
    let mut iosb = [0usize; 2];
    let st = unsafe { NtSetInformationFile(h.0, &mut iosb, buf.as_ptr() as *const c_void, size as u32, 10) };
    if st >= 0 { Ok(()) } else { Err(st) }
}
