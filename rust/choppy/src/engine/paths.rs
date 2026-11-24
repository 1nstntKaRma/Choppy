//! Long-path (\\?\) helpers (Paths.cs). The engine works only with long-form paths; the UI shows the short form.

use std::sync::OnceLock;

use crate::sys;
use crate::wstr::{WString, w};

pub const LONG_PREFIX: &str = r"\\?\";
pub const LONG_UNC_PREFIX: &str = r"\\?\UNC\";

static START_DIR: OnceLock<WString> = OnceLock::new();

/// The folder Choppy was started from (Program.StartDir): relative paths on the command line are resolved against it.
pub fn set_start_dir(dir: WString) {
    let _ = START_DIR.set(dir);
}
pub fn start_dir() -> Option<&'static WString> {
    START_DIR.get()
}

/// Repairs a path that came through a command line: Explorer passes a drive root as "C:\" and the backslash
/// before the closing quote turns into a quote character (C:"). Also drops stray quotes around it.
pub fn fix_arg(path: &WString) -> WString {
    if path.is_empty() {
        return path.clone();
    }
    let mut p = path.trim();
    if p.ends_with("\"") {
        p = p.trim_end_char('"') + "\\";
    }
    if p.starts_with("\"") {
        p = p.trim_start_char('"');
    }
    p
}

pub fn to_long(path: &WString) -> WString {
    let path = fix_arg(path);
    if path.is_empty() {
        return path;
    }
    if path.starts_with(LONG_PREFIX) {
        return trim_end(&path);
    }
    if path.starts_with(r"\\.\") {
        return path;
    }
    // Path.GetFullPath, against the start folder; the C# keeps the path as given when .NET throws on it
    let full = net_full_path(&path).unwrap_or_else(|| path.clone());
    let full = trim_end(&full);
    if full.starts_with(r"\\") {
        return w(LONG_UNC_PREFIX) + &full.substring(2);
    }
    w(LONG_PREFIX) + &full
}

/// Removes the \\?\ prefix for display and for APIs that don't need it.
pub fn to_short(path: &WString) -> WString {
    if path.is_empty() {
        return path.clone();
    }
    if path.starts_with_ignore_case_str(LONG_UNC_PREFIX) {
        return w(r"\\") + &path.substring(LONG_UNC_PREFIX.len());
    }
    if path.starts_with(LONG_PREFIX) {
        return path.substring(LONG_PREFIX.len());
    }
    path.clone()
}

fn trim_end(p: &WString) -> WString {
    // keep "C:\" and "\\?\C:\" roots intact
    let mut p = p.clone();
    while p.len() > 3 && (p.ends_with("\\") || p.ends_with("/")) {
        let t = p.slice(0, p.len() - 1);
        if t.ends_with(":") {
            break;
        }
        p = t;
    }
    p
}

pub fn combine(dir: &WString, rel: &WString) -> WString {
    if rel.is_empty() {
        return dir.clone();
    }
    if dir.ends_with("\\") {
        return dir.clone() + rel;
    }
    dir.clone() + "\\" + rel
}

pub fn file_name(path: &WString) -> WString {
    match path.last_index_of('\\') {
        None => path.clone(),
        Some(i) => path.substring(i + 1),
    }
}

/// The folder a path sits in; None for a drive root (and for a name with no folder).
pub fn parent(path: &WString) -> Option<WString> {
    if path.ends_with(":\\") || path.ends_with(":") {
        return None; // a drive root has no parent
    }
    let i = path.last_index_of('\\')?;
    if i == 0 {
        return None;
    }
    if i == path.len() - 1 {
        return parent(&path.slice(0, i));
    }
    let mut p = path.slice(0, i);
    if p.ends_with(":") {
        p += "\\";
    }
    Some(p)
}

/// True when 'inner' is 'outer' or sits inside it.
pub fn is_same_or_inside(inner: &WString, outer: &WString) -> bool {
    let a = to_short(inner).trim_end_char('\\') + "\\";
    let b = to_short(outer).trim_end_char('\\') + "\\";
    a.starts_with_ignore_case(&b)
}

pub fn same_path(a: &WString, b: &WString) -> bool {
    to_short(a).trim_end_char('\\').eq_ignore_case(&to_short(b).trim_end_char('\\'))
}

pub fn is_unc(long_path: &WString) -> bool {
    long_path.starts_with_ignore_case_str(LONG_UNC_PREFIX) || (long_path.starts_with(r"\\") && !long_path.starts_with(LONG_PREFIX))
}

/// "name (2).ext" style names for Keep both.
pub fn numbered_name(file_name: &WString, n: i32, is_dir: bool) -> WString {
    if is_dir {
        return file_name.clone() + &format!(" ({n})");
    }
    match file_name.last_index_of('.') {
        Some(dot) if dot > 0 => file_name.slice(0, dot) + &format!(" ({n})") + &file_name.substring(dot),
        _ => file_name.clone() + &format!(" ({n})"),
    }
}

pub fn exists(long_path: &WString) -> bool {
    sys::get_file_attributes(long_path) != sys::INVALID_FILE_ATTRIBUTES
}

pub fn dir_exists(long_path: &WString) -> bool {
    let a = sys::get_file_attributes(long_path);
    a != sys::INVALID_FILE_ATTRIBUTES && (a & sys::FILE_ATTRIBUTE_DIRECTORY) != 0
}

// ---------------- Path.GetFullPath as .NET Framework 4.8 does it ----------------
// The C# ToLong calls Path.IsPathRooted, Path.Combine(Program.StartDir, path) and Path.GetFullPath, and keeps the path
// as given when one of them throws. What .NET adds around GetFullPathNameW is repeated here, so a path comes out the
// same: invalid characters, white space, wildcards and stray colons throw; white space at the end is trimmed; 8.3
// short names (PROGRA~1) are expanded where the path exists.

fn is_sep(c: u16) -> bool {
    c == b'\\' as u16 || c == b'/' as u16
}

/// PathInternal's invalid path characters: " < > | and the control characters (NUL included).
fn has_illegal_chars(p: &WString) -> bool {
    p.units().iter().any(|&c| c < 32 || c == b'"' as u16 || c == b'<' as u16 || c == b'>' as u16 || c == b'|' as u16)
}

/// Path.IsPathRooted.
fn is_path_rooted(p: &WString) -> bool {
    let u = p.units();
    (!u.is_empty() && is_sep(u[0])) || (u.len() >= 2 && u[1] == b':' as u16)
}

/// Path.Combine(a, b) for a b that isn't rooted.
fn net_combine(a: &WString, b: &WString) -> WString {
    if a.is_empty() {
        return b.clone();
    }
    if b.is_empty() {
        return a.clone();
    }
    let last = a.at(a.len() - 1);
    if !is_sep(last) && last != b':' as u16 { a.clone() + "\\" + b } else { a.clone() + b }
}

fn is_valid_drive_char(c: u16) -> bool {
    (b'A' as u16..=b'Z' as u16).contains(&c) || (b'a' as u16..=b'z' as u16).contains(&c)
}

/// PathInternal.IsDevice: \\?\ or \\.\ (either slash).
fn is_device(p: &WString) -> bool {
    let u = p.units();
    u.len() >= 4 && is_sep(u[0]) && is_sep(u[1]) && (u[2] == b'?' as u16 || u[2] == b'.' as u16) && is_sep(u[3])
}

/// PathInternal.PathStartSkip: spaces before "C:" or "\" are skipped.
fn path_start_skip(u: &[u16]) -> usize {
    let mut i = 0;
    while i < u.len() && u[i] == b' ' as u16 {
        i += 1;
    }
    if (i > 0 && i < u.len() && is_sep(u[i])) || (i + 1 < u.len() && u[i + 1] == b':' as u16 && is_valid_drive_char(u[i])) {
        return i;
    }
    0
}

/// PathInternal.HasInvalidVolumeSeparator: a colon anywhere but right after the drive letter.
fn has_invalid_volume_separator(p: &WString) -> bool {
    let u = p.units();
    let extended = p.starts_with(LONG_PREFIX);
    let s = if extended { LONG_PREFIX.len() } else { path_start_skip(u) };
    let colon = b':' as u16;
    (u.len() > s && u[s] == colon)
        || (u.len() >= s + 2 && u[s + 1] == colon && !is_valid_drive_char(u[s]))
        || (u.len() > s + 2 && u[s + 2..].contains(&colon))
}

/// PathInternal.GetRootLength for a full path: "C:\" = 3, "\\server\share" up to the share's end, "\\?\C:\" = 7.
fn root_length(u: &[u16]) -> usize {
    let mut i = 0;
    let mut volume_separator = 2;
    let mut unc = false;
    let device = u.len() >= 4 && is_sep(u[0]) && is_sep(u[1]) && (u[2] == b'?' as u16 || u[2] == b'.' as u16) && is_sep(u[3]);
    if device {
        // \\?\UNC\server\share or \\?\C:\
        let rest: Vec<u16> = u[4..].iter().take(4).copied().collect();
        if rest.len() == 4 && WString::from_units(rest).eq_ignore_case_str("UNC\\") {
            i = 8;
            unc = true;
        } else {
            i = 4;
            volume_separator = 6;
        }
    }
    if !unc && u.len() >= 2 && is_sep(u[0]) && is_sep(u[1]) && !device {
        i = 2;
        unc = true;
    }
    if unc {
        // the server and the share
        let mut n = 2;
        while i < u.len() && (!is_sep(u[i]) || {
            n -= 1;
            n > 0
        }) {
            i += 1;
        }
        return i;
    }
    if u.len() > volume_separator - 1 && u[volume_separator - 1] == b':' as u16 {
        i = volume_separator;
        if u.len() > i && is_sep(u[i]) {
            i += 1;
        }
        return i;
    }
    if !u.is_empty() && is_sep(u[0]) { 1 } else { i }
}

/// PathHelper.TryExpandShortFileName (.NET Framework 4.8), quirk included. GetLongPathName on the \\?\ form of the
/// path; where the path doesn't exist, on the part of it that does (the rest put back). The answer goes into a buffer
/// the size of the input: when a partial answer doesn't fit, .NET grows the buffer but has already taken its
/// shortening NUL out, so it skips that level and expands one level higher (or not at all at the root). Measured:
/// "C:\PROGRA~1\aaa" (15 characters) stays as it is, "C:\PROGRA~1\aaaa" (16) becomes "C:\Program Files\aaaa".
fn expand_short_names(full: &WString) -> WString {
    let u = full.units();
    let (mut input, root_difference, dos_unc): (Vec<u16>, usize, bool) = if is_device(full) {
        let mut v = u.to_vec();
        v[2] = b'?' as u16;
        (v, 0, false)
    } else if u.len() >= 2 && is_sep(u[0]) && is_sep(u[1]) {
        (w(LONG_UNC_PREFIX).units().iter().chain(u[2..].iter()).copied().collect(), 6, true)
    } else {
        (w(LONG_PREFIX).units().iter().chain(u.iter()).copied().collect(), 4, false)
    };
    let root = root_length(u) + root_difference;
    let input_len = input.len();
    let mut capacity = input_len + 1;
    let mut found = input_len - 1;
    let mut output: Option<Vec<u16>> = None;
    loop {
        let (result, out, err) = sys::long_path_name_with(&input, capacity);
        // the temporary NUL is taken out again
        if input[found] == 0 {
            input[found] = b'\\' as u16;
        }
        let result = result as usize;
        if result == 0 {
            if err != sys::ERROR_FILE_NOT_FOUND && err != sys::ERROR_PATH_NOT_FOUND {
                break;
            }
            // not there: try further up the path
            found -= 1;
            while found > root && input[found] != b'\\' as u16 {
                found -= 1;
            }
            if found <= root {
                break;
            }
            input[found] = 0;
        } else if result > capacity {
            // too small: a bigger buffer, and the same call again (now on the whole path)
            capacity = result;
            let _ = sys::long_path_name_with(&input, capacity);
        } else {
            let mut o = out[..result].to_vec();
            if found < input_len - 1 {
                o.extend_from_slice(&input[found..]);
            }
            output = Some(o);
            break;
        }
    }
    let mut chosen = output.unwrap_or(input);
    if dos_unc {
        chosen[root_difference] = b'\\' as u16;
    }
    WString::from_units(chosen[root_difference..].to_vec())
}

/// Path.IsPathRooted + Path.Combine(StartDir, ..) + Path.GetFullPath; None where .NET throws.
fn net_full_path(path: &WString) -> Option<WString> {
    if has_illegal_chars(path) {
        return None; // IsPathRooted / Combine: "Illegal characters in path"
    }
    let p = match start_dir() {
        Some(sd) if !is_path_rooted(path) => {
            if has_illegal_chars(sd) {
                return None;
            }
            net_combine(sd, path)
        }
        _ => path.clone(),
    };
    if p.is_white_space() {
        return None;
    }
    let mut full = sys::full_path_name(&p)?;
    // Win32 trims only spaces at the end; .NET trims the rest of its white space too
    let trim_end_chars = [0x9u16, 0xA, 0xB, 0xC, 0xD, 0x20, 0x85, 0xA0];
    let mut n = full.len();
    while n > 0 && trim_end_chars.contains(&full.at(n - 1)) {
        n -= 1;
    }
    full = full.slice(0, n);
    if has_illegal_chars(&full) {
        return None;
    }
    if full.contains_char('~') {
        full = expand_short_names(&full);
    }
    if full.is_white_space() || full.len() >= 32767 {
        return None; // "The path is not of a legal form" / too long
    }
    // the FileIOPermission checks GetFullPath still emulates: wildcards, colons past the drive
    if !is_device(&full) {
        if full.contains_char('*') || full.contains_char('?') {
            return None;
        }
        if has_invalid_volume_separator(&full) {
            return None;
        }
    }
    Some(full)
}
