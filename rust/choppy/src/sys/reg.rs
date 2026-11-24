//! HKEY_CURRENT_USER values (Microsoft.Win32.Registry as the C# Settings uses it).

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW,
    RegDeleteTreeW, RegDeleteValueW, RegEnumValueW, RegOpenKeyExW, RegQueryInfoKeyW, RegSetValueExW,
};
use windows::core::{PCWSTR, PWSTR};

use crate::wstr::WString;

/// A registry value as RegEnumValueW gives it: its type and its bytes.
#[derive(Clone, Debug)]
pub struct RegValue {
    pub kind: u32,
    pub data: Vec<u8>,
}

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn err(e: WIN32_ERROR) -> Result<(), u32> {
    if e == ERROR_SUCCESS { Ok(()) } else { Err(e.0) }
}

/// Every value of HKCU\<subkey> (name, type, bytes). Ok(None) = the key isn't there (RegistryKey.OpenSubKey's null).
pub fn reg_read_values(subkey: &str) -> Result<Option<Vec<(WString, RegValue)>>, u32> {
    let sk = WString::from(subkey).to_wide_z();
    let mut h = HKEY::default();
    let e = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(sk.as_ptr()), None, KEY_READ, &mut h) };
    if e == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    err(e)?;
    let key = Key(h);
    let (mut max_name, mut max_data) = (0u32, 0u32);
    err(unsafe {
        RegQueryInfoKeyW(key.0, None, None, None, None, None, None, None, Some(&mut max_name), Some(&mut max_data), None, None)
    })?;
    let mut out = Vec::new();
    let mut name = vec![0u16; max_name as usize + 2];
    let mut data = vec![0u8; max_data as usize + 2];
    let mut index = 0u32;
    loop {
        let mut name_len = name.len() as u32;
        let mut data_len = data.len() as u32;
        let mut kind = 0u32;
        let e = unsafe {
            RegEnumValueW(
                key.0,
                index,
                Some(PWSTR(name.as_mut_ptr())),
                &mut name_len,
                None,
                Some(&mut kind),
                Some(data.as_mut_ptr()),
                Some(&mut data_len),
            )
        };
        if e == ERROR_NO_MORE_ITEMS {
            break;
        }
        if e == ERROR_MORE_DATA {
            // a value grew since RegQueryInfoKey: room for it, then the same index again
            name.resize(name.len() * 2 + 256, 0);
            data.resize(data.len() * 2 + 1024, 0);
            continue;
        }
        err(e)?;
        out.push((
            WString::from_units(name[..name_len as usize].to_vec()),
            RegValue { kind, data: data[..data_len as usize].to_vec() },
        ));
        index += 1;
    }
    Ok(Some(out))
}

/// Creates HKCU\<subkey> when needed (read + write, as RegistryKey.CreateSubKey), sets each value as REG_SZ, then
/// deletes the named values (a missing one is fine). Stops at the first failure.
pub fn reg_write_values(subkey: &str, values: &[(&str, String)], delete: &[&str]) -> Result<(), u32> {
    let sk = WString::from(subkey).to_wide_z();
    let mut h = HKEY::default();
    err(unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(sk.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE,
            None,
            &mut h,
            None,
        )
    })?;
    let key = Key(h);
    for (name, value) in values {
        let n = WString::from(*name).to_wide_z();
        // REG_SZ: the UTF-16 text and its closing NUL
        let bytes: Vec<u8> = WString::from(value.as_str()).to_wide_z().iter().flat_map(|u| u.to_le_bytes()).collect();
        err(unsafe { RegSetValueExW(key.0, PCWSTR(n.as_ptr()), None, REG_SZ, Some(&bytes)) })?;
    }
    for name in delete {
        let n = WString::from(*name).to_wide_z();
        let e = unsafe { RegDeleteValueW(key.0, PCWSTR(n.as_ptr())) };
        if e != ERROR_FILE_NOT_FOUND {
            err(e)?;
        }
    }
    Ok(())
}

/// Removes HKCU\<subkey> and everything under it (tests only: a scratch key).
pub fn reg_delete_tree(subkey: &str) -> Result<(), u32> {
    let sk = WString::from(subkey).to_wide_z();
    let e = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(sk.as_ptr())) };
    if e == ERROR_FILE_NOT_FOUND { Ok(()) } else { err(e) }
}
