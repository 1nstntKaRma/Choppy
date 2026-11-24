//! Text, names and times the way .NET Framework gets them from Windows: OrdinalIgnoreCase, ToUpper/LowerInvariant,
//! Win32Exception's message, Path.GetFullPath's Win32 parts, the LocalApplicationData folder, new GUIDs, local times.

use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal, LCMAP_LOWERCASE, LCMAP_UPPERCASE, LCMapStringEx, LOCALE_NAME_INVARIANT};
use windows::Win32::Foundation::LPARAM;
use windows::Win32::Storage::FileSystem::{GetFullPathNameW, GetLongPathNameW};
use windows::Win32::System::Com::{CoCreateGuid, CoTaskMemFree};
use windows::Win32::System::Diagnostics::Debug::{
    FORMAT_MESSAGE_ARGUMENT_ARRAY, FORMAT_MESSAGE_FROM_SYSTEM, FORMAT_MESSAGE_IGNORE_INSERTS, FormatMessageW,
};
use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
use windows::Win32::System::Time::{
    DYNAMIC_TIME_ZONE_INFORMATION, FileTimeToSystemTime, GetDynamicTimeZoneInformation, SystemTimeToFileTime,
    SystemTimeToTzSpecificLocalTimeEx,
};
use windows::Win32::UI::Shell::{FOLDERID_LocalAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
use windows::core::{PCWSTR, PWSTR};

use super::Guid;
use crate::wstr::WString;

fn ascii_upper(c: u16) -> u16 {
    if (b'a' as u16..=b'z' as u16).contains(&c) { c - 32 } else { c }
}

/// OrdinalIgnoreCase equality of two equally long strings: ASCII directly, anything else through Windows'
/// upper-case table (CompareStringOrdinal, as .NET Framework does).
pub fn equal_ignore_case(a: &[u16], b: &[u16]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    if a.iter().chain(b.iter()).all(|&c| c < 0x80) {
        return a.iter().zip(b.iter()).all(|(&x, &y)| ascii_upper(x) == ascii_upper(y));
    }
    unsafe { CompareStringOrdinal(a, b, true) == CSTR_EQUAL }
}

fn map_case(s: &[u16], flags: u32) -> Vec<u16> {
    if s.is_empty() {
        return Vec::new();
    }
    if s.iter().all(|&c| c < 0x80) {
        return s
            .iter()
            .map(|&c| {
                if flags == LCMAP_UPPERCASE {
                    ascii_upper(c)
                } else if (b'A' as u16..=b'Z' as u16).contains(&c) {
                    c + 32
                } else {
                    c
                }
            })
            .collect();
    }
    let mut out = vec![0u16; s.len() * 2 + 8];
    let n = unsafe { LCMapStringEx(LOCALE_NAME_INVARIANT, flags, s, Some(&mut out), None, None, LPARAM(0)) };
    if n <= 0 {
        return s.to_vec();
    }
    out.truncate(n as usize);
    out
}

/// C#'s ToUpperInvariant: the invariant culture's simple upper case (LCMapStringEx, as .NET Framework does).
pub fn upper_invariant(s: &[u16]) -> Vec<u16> {
    map_case(s, LCMAP_UPPERCASE)
}

/// C#'s ToLowerInvariant.
pub fn lower_invariant(s: &[u16]) -> Vec<u16> {
    map_case(s, LCMAP_LOWERCASE)
}

/// new Win32Exception(code).Message in .NET Framework 4.8: FormatMessage (system table, no inserts) into 256
/// characters, 4 times more while it doesn't fit (up to 64 K); trailing white space, control characters and periods
/// dropped; "Unknown error (0x..)" when Windows has no text for it.
pub fn win32_message(code: u32) -> String {
    const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
    let mut capacity = 256usize;
    let (buf, n) = loop {
        let mut buf = vec![0u16; capacity + 1];
        let n = unsafe {
            FormatMessageW(
                FORMAT_MESSAGE_IGNORE_INSERTS | FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_ARGUMENT_ARRAY,
                None,
                code,
                0,
                PWSTR(buf.as_mut_ptr()),
                (capacity + 1) as u32,
                None,
            )
        };
        if n != 0 {
            break (buf, n);
        }
        if unsafe { windows::Win32::Foundation::GetLastError() }.0 != ERROR_INSUFFICIENT_BUFFER {
            return format!("Unknown error (0x{:x})", code);
        }
        capacity *= 4;
        if capacity >= 65 * 1024 {
            return format!("Unknown error (0x{:x})", code);
        }
    };
    let text = WString::from_nul_terminated(&buf[..(n as usize).min(buf.len())]);
    let u = text.units();
    let mut i = u.len();
    while i > 0 {
        let c = u[i - 1];
        if c > 32 && c != b'.' as u16 {
            break;
        }
        i -= 1;
    }
    String::from_utf16_lossy(&u[..i])
}

/// The user's LocalApplicationData folder (Environment.SpecialFolder.LocalApplicationData).
pub fn local_app_data() -> Option<WString> {
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT, None).ok()?;
        let s = WString::from_units(p.as_wide().to_vec());
        CoTaskMemFree(Some(p.0 as *const _));
        Some(s)
    }
}

/// A new random GUID (Guid.NewGuid).
pub fn new_guid() -> Guid {
    unsafe { CoCreateGuid() }.unwrap_or_else(|_| Guid::from_u128(0))
}

/// GetFullPathNameW (the Win32 part of Path.GetFullPath). None when it fails.
pub fn full_path_name(path: &WString) -> Option<WString> {
    let p = path.to_wide_z();
    let mut buf = vec![0u16; 260];
    loop {
        let n = unsafe { GetFullPathNameW(PCWSTR(p.as_ptr()), Some(&mut buf), None) } as usize;
        if n == 0 {
            return None;
        }
        if n < buf.len() {
            return Some(WString::from_units(buf[..n].to_vec()));
        }
        buf.resize(n + 1, 0);
    }
}

/// GetLongPathNameW (8.3 short names expanded). Err = the Win32 error.
pub fn long_path_name(path: &WString) -> Result<WString, u32> {
    let p = path.to_wide_z();
    let mut buf = vec![0u16; 260];
    loop {
        let n = unsafe { GetLongPathNameW(PCWSTR(p.as_ptr()), Some(&mut buf)) } as usize;
        if n == 0 {
            return Err(unsafe { windows::Win32::Foundation::GetLastError() }.0);
        }
        if n < buf.len() {
            return Ok(WString::from_units(buf[..n].to_vec()));
        }
        buf.resize(n + 1, 0);
    }
}

/// One GetLongPathNameW call into a buffer of `capacity` characters, as .NET's PathHelper makes it: (its result, what
/// it wrote, GetLastError). `path` ends at its first NUL (.NET puts one in to try a shorter path).
pub fn long_path_name_with(path: &[u16], capacity: usize) -> (u32, Vec<u16>, u32) {
    let mut p: Vec<u16> = path.iter().copied().take_while(|&c| c != 0).collect();
    p.push(0);
    let mut buf = vec![0u16; capacity.max(1)];
    let r = unsafe { GetLongPathNameW(PCWSTR(p.as_ptr()), Some(&mut buf)) };
    let err = if r == 0 { unsafe { windows::Win32::Foundation::GetLastError() }.0 } else { 0 };
    (r, buf, err)
}

/// ExpandEnvironmentStringsW (%NAME% replaced).
pub fn expand_environment_strings(s: &WString) -> WString {
    let p = s.to_wide_z();
    let mut buf = vec![0u16; s.len() + 256];
    loop {
        let n = unsafe { ExpandEnvironmentStringsW(PCWSTR(p.as_ptr()), Some(&mut buf)) } as usize;
        if n == 0 {
            return s.clone();
        }
        if n <= buf.len() {
            return WString::from_nul_terminated(&buf[..n]);
        }
        buf.resize(n, 0);
    }
}

/// A local date and time (SYSTEMTIME's fields).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LocalTime {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub millisecond: u16,
}

/// The last FILETIME a .NET DateTime holds (9999-12-31 23:59:59.9999999).
const MAX_FILE_TIME: i64 = 2650467743999999999;

fn to_file_time(ft: i64) -> FILETIME {
    FILETIME { dwLowDateTime: ft as u32, dwHighDateTime: (ft as u64 >> 32) as u32 }
}

/// A FILETIME (UTC) as local time with the time zone's daylight-saving rules for that date, as
/// DateTime.FromFileTimeUtc(ft).ToLocalTime() gives it: None (C#'s DateTime.MinValue) when .NET can't hold the time,
/// the standard offset where Windows has no rule for the date (1601), and .NET's last date where the offset would go
/// past it - all three measured side by side with the C# (plan dump).
pub fn file_time_to_local(ft: i64) -> Option<LocalTime> {
    if !(0..=MAX_FILE_TIME).contains(&ft) {
        return None;
    }
    unsafe {
        let mut utc = SYSTEMTIME::default();
        FileTimeToSystemTime(&to_file_time(ft), &mut utc).ok()?;
        let mut tz = DYNAMIC_TIME_ZONE_INFORMATION::default();
        GetDynamicTimeZoneInformation(&mut tz);
        let mut local = SYSTEMTIME::default();
        let mut lf = FILETIME::default();
        let offset = if SystemTimeToTzSpecificLocalTimeEx(Some(&tz), &utc, &mut local).is_ok()
            && SystemTimeToFileTime(&local, &mut lf).is_ok()
        {
            ((((lf.dwHighDateTime as u64) << 32) | lf.dwLowDateTime as u64) as i64) - ft
        } else {
            -(tz.Bias as i64) * 600_000_000
        };
        let local_ft = (ft + offset).clamp(0, MAX_FILE_TIME);
        let mut st = SYSTEMTIME::default();
        FileTimeToSystemTime(&to_file_time(local_ft), &mut st).ok()?;
        Some(LocalTime {
            year: st.wYear,
            month: st.wMonth,
            day: st.wDay,
            hour: st.wHour,
            minute: st.wMinute,
            second: st.wSecond,
            millisecond: st.wMilliseconds,
        })
    }
}
