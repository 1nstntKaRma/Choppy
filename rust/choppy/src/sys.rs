//! Win32 calls (Native.cs). The only module allowed to use `unsafe`, always behind safe wrappers. Every path the
//! engine hands in is already in \\?\ long form (as in Native.cs). Later steps add the calls they need here.
#![allow(unsafe_code)]

mod device;
mod file;
mod reg;
mod text;
mod volume;

pub use device::*;
pub use file::*;
pub use reg::*;
pub use text::*;
pub use volume::*;

pub const ERROR_SUCCESS: u32 = 0;
pub const ERROR_FILE_NOT_FOUND: u32 = 2;
pub const ERROR_PATH_NOT_FOUND: u32 = 3;
pub const ERROR_ACCESS_DENIED: u32 = 5;
pub const ERROR_INVALID_HANDLE: u32 = 6;
pub const ERROR_NOT_READY: u32 = 21;
pub const ERROR_SHARING_VIOLATION: u32 = 32;
pub const ERROR_LOCK_VIOLATION: u32 = 33;
pub const ERROR_HANDLE_EOF: u32 = 38;
pub const ERROR_BAD_NETPATH: u32 = 53;
pub const ERROR_NETWORK_BUSY: u32 = 54;
pub const ERROR_DEV_NOT_EXIST: u32 = 55;
pub const ERROR_UNEXP_NET_ERR: u32 = 59;
pub const ERROR_NETNAME_DELETED: u32 = 64;
pub const ERROR_NETWORK_ACCESS_DENIED: u32 = 65;
pub const ERROR_BAD_NET_NAME: u32 = 67;
pub const ERROR_FILE_EXISTS: u32 = 80;
pub const ERROR_DISK_FULL: u32 = 112;
pub const ERROR_SEM_TIMEOUT: u32 = 121;
pub const ERROR_INVALID_NAME: u32 = 123;
pub const ERROR_DIR_NOT_EMPTY: u32 = 145;
pub const ERROR_ALREADY_EXISTS: u32 = 183;
pub const ERROR_FILENAME_EXCED_RANGE: u32 = 206;
pub const ERROR_NO_MORE_FILES: u32 = 18;
pub const ERROR_REQUEST_ABORTED: u32 = 1235;
pub const ERROR_CONNECTION_ABORTED: u32 = 1236;
pub const ERROR_NETWORK_UNREACHABLE: u32 = 1231;
pub const ERROR_HOST_UNREACHABLE: u32 = 1232;
pub const ERROR_CONNECTION_UNAVAIL: u32 = 1201;
pub const ERROR_NOT_CONNECTED: u32 = 2250;
pub const ERROR_HANDLE_DISK_FULL: u32 = 39;
pub const ERROR_CRC: u32 = 23;
pub const ERROR_IO_DEVICE: u32 = 1117;
pub const ERROR_IO_PENDING: u32 = 997;
pub const ERROR_INVALID_PARAMETER: u32 = 87;
pub const ERROR_FILE_TOO_LARGE: u32 = 223;
pub const ERROR_MORE_DATA: u32 = 234;
pub const ERROR_NO_MORE_ITEMS: u32 = 259;

pub const COPY_FILE_FAIL_IF_EXISTS: u32 = 0x00000001;
pub const COPY_FILE_ALLOW_DECRYPTED_DESTINATION: u32 = 0x00000008;
pub const COPY_FILE_NO_BUFFERING: u32 = 0x00001000;

pub const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
pub const MOVEFILE_COPY_ALLOWED: u32 = 0x2;
pub const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

pub const PROGRESS_CONTINUE: u32 = 0;
pub const PROGRESS_CANCEL: u32 = 1;
pub const CALLBACK_CHUNK_FINISHED: u32 = 0;
pub const CALLBACK_STREAM_SWITCH: u32 = 1;

pub const GENERIC_READ: u32 = 0x80000000;
pub const GENERIC_WRITE: u32 = 0x40000000;
pub const DELETE: u32 = 0x00010000;
pub const FILE_READ_ATTRIBUTES: u32 = 0x0080;
pub const FILE_WRITE_ATTRIBUTES: u32 = 0x0100;
pub const FILE_SHARE_READ: u32 = 1;
pub const FILE_SHARE_WRITE: u32 = 2;
pub const FILE_SHARE_DELETE: u32 = 4;
pub const CREATE_NEW: u32 = 1;
pub const CREATE_ALWAYS: u32 = 2;
pub const OPEN_EXISTING: u32 = 3;
pub const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x02000000;
pub const FILE_FLAG_SEQUENTIAL_SCAN: u32 = 0x08000000;
pub const FILE_FLAG_NO_BUFFERING: u32 = 0x20000000;
pub const FILE_FLAG_OVERLAPPED: u32 = 0x40000000;
pub const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x00200000;

pub const DRIVE_UNKNOWN: u32 = 0;
pub const DRIVE_NO_ROOT_DIR: u32 = 1;
pub const DRIVE_REMOVABLE: u32 = 2;
pub const DRIVE_FIXED: u32 = 3;
pub const DRIVE_REMOTE: u32 = 4;
pub const DRIVE_CDROM: u32 = 5;
pub const DRIVE_RAMDISK: u32 = 6;

/// GetFileAttributesW's failure (C#'s INVALID_FILE_ATTRIBUTES = -1).
pub const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
pub const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
pub const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
pub const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
pub const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
pub const FILE_ATTRIBUTE_ARCHIVE: u32 = 0x20;
pub const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
pub const FILE_ATTRIBUTE_SPARSE_FILE: u32 = 0x200;
pub const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
pub const FILE_ATTRIBUTE_COMPRESSED: u32 = 0x800;
pub const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;
pub const FILE_ATTRIBUTE_NOT_CONTENT_INDEXED: u32 = 0x2000;
pub const FILE_ATTRIBUTE_ENCRYPTED: u32 = 0x4000;
pub const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x40000;
pub const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x400000;

pub const FILE_BASIC_INFO_CLASS: i32 = 0;
pub const FILE_RENAME_INFO_CLASS: i32 = 3;
pub const FILE_ALLOCATION_INFO_CLASS: i32 = 5;
pub const FILE_END_OF_FILE_INFO_CLASS: i32 = 6;

/// where a file's data starts on the disk (no admin rights needed): read many small files in disk order
pub const FSCTL_GET_RETRIEVAL_POINTERS: u32 = 0x00090073;
/// Storage queries (seek penalty, bus type)
pub const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D1400;
/// which physical disk a volume sits on (two volumes on one disk share the heads)
pub const IOCTL_STORAGE_GET_DEVICE_NUMBER: u32 = 0x002D1080;

pub const MEM_COMMIT: u32 = 0x1000;
pub const MEM_RESERVE: u32 = 0x2000;
pub const MEM_RELEASE: u32 = 0x8000;
pub const PAGE_READWRITE: u32 = 0x04;

/// Keeps the PC from sleeping while the calling thread asks for it (the screen may still turn off).
pub const ES_CONTINUOUS: u32 = 0x80000000;
pub const ES_SYSTEM_REQUIRED: u32 = 0x1;

/// ReFS: a copy can share the original's blocks
pub const FILE_SUPPORTS_BLOCK_REFCOUNTING: u32 = 0x08000000;

/// The message for a Win32 error, as the copy window and the logs show it (Native.ErrorText).
pub fn error_text(code: u32) -> String {
    match code {
        ERROR_FILE_NOT_FOUND => "The file is gone (it was moved or deleted).".into(),
        ERROR_PATH_NOT_FOUND => "The folder is gone or can't be reached.".into(),
        ERROR_ACCESS_DENIED => "Access denied.".into(),
        ERROR_SHARING_VIOLATION => "The file is open in another program.".into(),
        ERROR_LOCK_VIOLATION => "Part of the file is locked by another program.".into(),
        ERROR_DISK_FULL | ERROR_HANDLE_DISK_FULL => "The destination is full.".into(),
        ERROR_FILE_TOO_LARGE => "Too big for the destination's file system.".into(),
        ERROR_BAD_NETPATH | ERROR_BAD_NET_NAME | ERROR_NETNAME_DELETED | ERROR_UNEXP_NET_ERR | ERROR_NETWORK_UNREACHABLE
        | ERROR_HOST_UNREACHABLE | ERROR_CONNECTION_ABORTED | ERROR_NOT_CONNECTED | ERROR_SEM_TIMEOUT => {
            "The network connection dropped.".into()
        }
        ERROR_INVALID_NAME => "The name isn't allowed at the destination.".into(),
        ERROR_FILENAME_EXCED_RANGE => "The path is too long for the destination.".into(),
        ERROR_CRC | ERROR_IO_DEVICE => "Read error on the disk (bad sector?).".into(),
        ERROR_REQUEST_ABORTED => "Stopped.".into(),
        // new Win32Exception(code).Message, then " (code)" - C#'s int, so a code past 2^31 shows negative
        _ => format!("{} ({})", win32_message(code), code as i32),
    }
}

/// The Win32 error code inside a `windows` crate error (HRESULT_FROM_WIN32), as GetLastError gave it.
fn win32_code(e: &windows::core::Error) -> u32 {
    let hr = e.code().0 as u32;
    if hr & 0xFFFF_0000 == 0x8007_0000 { hr & 0xFFFF } else { hr }
}
