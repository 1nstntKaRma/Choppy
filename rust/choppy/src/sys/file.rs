//! Files: handles, attributes, folder listings.

use core::ffi::c_void;

use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows::Win32::Storage::FileSystem::{
    FILE_CREATION_DISPOSITION, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_MODE, FIND_FIRST_EX_LARGE_FETCH, FindClose,
    FindExInfoBasic, FindExSearchNameMatch, FindFirstFileExW, FindNextFileW, GetFileAttributesExW, GetFileAttributesW,
    GetFileExInfoStandard, WIN32_FILE_ATTRIBUTE_DATA, WIN32_FIND_DATAW, CreateFileW,
};
use windows::core::PCWSTR;

use super::win32_code;
use crate::wstr::WString;

/// An open file or device; closed when dropped (C#'s SafeFileHandle).
pub struct Handle(pub(super) HANDLE);

// A kernel handle may be used from any thread.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// CreateFileW. Err = the Win32 error.
pub fn create_file(path: &WString, access: u32, share: u32, disposition: u32, flags: u32) -> Result<Handle, u32> {
    let p = path.to_wide_z();
    unsafe {
        CreateFileW(
            PCWSTR(p.as_ptr()),
            access,
            FILE_SHARE_MODE(share),
            None,
            FILE_CREATION_DISPOSITION(disposition),
            FILE_FLAGS_AND_ATTRIBUTES(flags),
            None,
        )
    }
    .map(Handle)
    .map_err(|e| win32_code(&e))
}

/// GetFileAttributesW: INVALID_FILE_ATTRIBUTES when it isn't there (or can't be reached).
pub fn get_file_attributes(path: &WString) -> u32 {
    let p = path.to_wide_z();
    unsafe { GetFileAttributesW(PCWSTR(p.as_ptr())) }
}

fn file_time(f: FILETIME) -> i64 {
    (((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64) as i64
}

/// WIN32_FILE_ATTRIBUTE_DATA, with the FILETIMEs as C#'s longs.
#[derive(Clone, Copy, Debug, Default)]
pub struct FileAttrData {
    pub attributes: u32,
    pub creation: i64,
    pub access: i64,
    pub write: i64,
    pub size_high: u32,
    pub size_low: u32,
}

impl FileAttrData {
    pub fn size(&self) -> i64 {
        ((self.size_high as i64) << 32) | self.size_low as i64
    }
}

/// GetFileAttributesExW (GetFileExInfoStandard). Err = the Win32 error.
pub fn get_file_attributes_ex(path: &WString) -> Result<FileAttrData, u32> {
    let p = path.to_wide_z();
    let mut d = WIN32_FILE_ATTRIBUTE_DATA::default();
    unsafe { GetFileAttributesExW(PCWSTR(p.as_ptr()), GetFileExInfoStandard, &mut d as *mut _ as *mut c_void) }
        .map_err(|e| win32_code(&e))?;
    Ok(FileAttrData {
        attributes: d.dwFileAttributes,
        creation: file_time(d.ftCreationTime),
        access: file_time(d.ftLastAccessTime),
        write: file_time(d.ftLastWriteTime),
        size_high: d.nFileSizeHigh,
        size_low: d.nFileSizeLow,
    })
}

/// One entry of a folder listing (WIN32_FIND_DATAW).
#[derive(Clone, Debug)]
pub struct FindData {
    pub attributes: u32,
    pub creation: i64,
    pub access: i64,
    pub write: i64,
    pub size_high: u32,
    pub size_low: u32,
    pub name: WString,
}

impl FindData {
    pub fn size(&self) -> i64 {
        ((self.size_high as i64) << 32) | self.size_low as i64
    }
    fn of(fd: &WIN32_FIND_DATAW) -> FindData {
        FindData {
            attributes: fd.dwFileAttributes,
            creation: file_time(fd.ftCreationTime),
            access: file_time(fd.ftLastAccessTime),
            write: file_time(fd.ftLastWriteTime),
            size_high: fd.nFileSizeHigh,
            size_low: fd.nFileSizeLow,
            name: WString::from_nul_terminated(&fd.cFileName),
        }
    }
}

/// A folder listing: FindFirstFileExW (basic info, large fetch), then FindNextFileW until it says no more (any
/// error ends it, as the C# loops do). Closed when dropped.
pub struct Find {
    h: HANDLE,
    first: Option<FindData>,
    done: bool,
}

/// Starts a listing of `pattern` ("<dir>\*"). Err = the Win32 error (ERROR_FILE_NOT_FOUND: nothing matches).
pub fn find_first(pattern: &WString) -> Result<Find, u32> {
    let p = pattern.to_wide_z();
    let mut fd = WIN32_FIND_DATAW::default();
    let h = unsafe {
        FindFirstFileExW(
            PCWSTR(p.as_ptr()),
            FindExInfoBasic,
            &mut fd as *mut _ as *mut c_void,
            FindExSearchNameMatch,
            None,
            FIND_FIRST_EX_LARGE_FETCH,
        )
    }
    .map_err(|e| win32_code(&e))?;
    Ok(Find { h, first: Some(FindData::of(&fd)), done: false })
}

impl Iterator for Find {
    type Item = FindData;
    fn next(&mut self) -> Option<FindData> {
        if let Some(f) = self.first.take() {
            return Some(f);
        }
        if self.done {
            return None;
        }
        let mut fd = WIN32_FIND_DATAW::default();
        if unsafe { FindNextFileW(self.h, &mut fd) }.is_ok() {
            Some(FindData::of(&fd))
        } else {
            self.done = true;
            None
        }
    }
}

impl Drop for Find {
    fn drop(&mut self) {
        unsafe {
            let _ = FindClose(self.h);
        }
    }
}

/// The sizes Windows' structs have here (the "struct size" self-test: WIN32_FIND_DATAW 592, WIN32_FILE_ATTRIBUTE_DATA 36).
pub fn struct_sizes() -> (usize, usize) {
    (size_of::<WIN32_FIND_DATAW>(), size_of::<WIN32_FILE_ATTRIBUTE_DATA>())
}
