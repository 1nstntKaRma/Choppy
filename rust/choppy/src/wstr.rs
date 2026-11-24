//! UTF-16 strings as C# and Windows hold them. Paths and file names are kept as UTF-16 code units, so a name with a
//! lone surrogate (NTFS allows it) copies as it is, and lengths and indexes count UTF-16 units like C#'s
//! (`Length`, `Substring`, `LastIndexOf`) - NTFS's 255-unit name limit is counted the same way.
//! Comparisons are ordinal like C#'s `==` and `StringComparison.Ordinal`; the `*_ignore_case` ones are
//! `StringComparison.OrdinalIgnoreCase` (Windows' own upper-case table, as .NET Framework uses).
#![deny(unsafe_code)]

use std::fmt;
use std::ops::{Add, AddAssign};

use crate::sys;

#[derive(Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WString(Vec<u16>);

/// A BMP character as one UTF-16 unit (every character the engine searches for is ASCII).
fn unit(c: char) -> u16 {
    debug_assert!((c as u32) < 0x10000);
    c as u32 as u16
}

impl WString {
    pub fn new() -> Self {
        WString(Vec::new())
    }
    pub fn from_units(v: Vec<u16>) -> Self {
        WString(v)
    }
    /// From a null-terminated or exact buffer: everything up to the first NUL.
    pub fn from_nul_terminated(buf: &[u16]) -> Self {
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        WString(buf[..n].to_vec())
    }
    pub fn from_os(s: &std::ffi::OsStr) -> Self {
        use std::os::windows::ffi::OsStrExt;
        WString(s.encode_wide().collect())
    }
    pub fn to_os(&self) -> std::ffi::OsString {
        use std::os::windows::ffi::OsStringExt;
        std::ffi::OsString::from_wide(&self.0)
    }
    pub fn units(&self) -> &[u16] {
        &self.0
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// For Win32 calls: the units plus a closing NUL.
    pub fn to_wide_z(&self) -> Vec<u16> {
        let mut v = Vec::with_capacity(self.0.len() + 1);
        v.extend_from_slice(&self.0);
        v.push(0);
        v
    }
    /// Lone surrogates become U+FFFD (as .NET's UTF-8 encoder writes them).
    pub fn to_string_lossy(&self) -> String {
        String::from_utf16_lossy(&self.0)
    }
    pub fn at(&self, i: usize) -> u16 {
        self.0[i]
    }

    pub fn eq_str(&self, s: &str) -> bool {
        self.0.iter().copied().eq(s.encode_utf16())
    }
    pub fn starts_with(&self, s: &str) -> bool {
        let w: Vec<u16> = s.encode_utf16().collect();
        self.0.starts_with(&w)
    }
    pub fn ends_with(&self, s: &str) -> bool {
        let w: Vec<u16> = s.encode_utf16().collect();
        self.0.ends_with(&w)
    }
    pub fn starts_with_w(&self, s: &WString) -> bool {
        self.0.starts_with(&s.0)
    }
    pub fn contains_char(&self, c: char) -> bool {
        self.0.contains(&unit(c))
    }
    /// C#'s IndexOf(char, startIndex).
    pub fn index_of(&self, c: char, from: usize) -> Option<usize> {
        let u = unit(c);
        self.0.iter().skip(from).position(|&x| x == u).map(|i| i + from)
    }
    /// C#'s LastIndexOf(char).
    pub fn last_index_of(&self, c: char) -> Option<usize> {
        let u = unit(c);
        self.0.iter().rposition(|&x| x == u)
    }
    /// C#'s Substring(start).
    pub fn substring(&self, start: usize) -> WString {
        WString(self.0[start..].to_vec())
    }
    /// C#'s Substring(start, length).
    pub fn slice(&self, start: usize, len: usize) -> WString {
        WString(self.0[start..start + len].to_vec())
    }
    pub fn push_str(&mut self, s: &str) {
        self.0.extend(s.encode_utf16());
    }
    pub fn push_w(&mut self, s: &WString) {
        self.0.extend_from_slice(&s.0);
    }
    /// C#'s TrimEnd(c): every trailing c.
    pub fn trim_end_char(&self, c: char) -> WString {
        let u = unit(c);
        let mut n = self.0.len();
        while n > 0 && self.0[n - 1] == u {
            n -= 1;
        }
        WString(self.0[..n].to_vec())
    }
    /// C#'s TrimStart(c).
    pub fn trim_start_char(&self, c: char) -> WString {
        let u = unit(c);
        let n = self.0.iter().take_while(|&&x| x == u).count();
        WString(self.0[n..].to_vec())
    }
    /// C#'s Trim(): white space at both ends (char.IsWhiteSpace).
    pub fn trim(&self) -> WString {
        let s = self.0.iter().position(|&c| !is_white_space(c)).unwrap_or(self.0.len());
        let e = self.0.iter().rposition(|&c| !is_white_space(c)).map_or(s, |i| i + 1);
        WString(self.0[s..e.max(s)].to_vec())
    }
    /// string.IsNullOrWhiteSpace.
    pub fn is_white_space(&self) -> bool {
        self.0.iter().all(|&c| is_white_space(c))
    }

    // ---- OrdinalIgnoreCase ----
    pub fn eq_ignore_case(&self, other: &WString) -> bool {
        self.0.len() == other.0.len() && sys::equal_ignore_case(&self.0, &other.0)
    }
    pub fn eq_ignore_case_str(&self, other: &str) -> bool {
        self.eq_ignore_case(&WString::from(other))
    }
    pub fn starts_with_ignore_case(&self, prefix: &WString) -> bool {
        self.0.len() >= prefix.0.len() && sys::equal_ignore_case(&self.0[..prefix.0.len()], &prefix.0)
    }
    pub fn starts_with_ignore_case_str(&self, prefix: &str) -> bool {
        self.starts_with_ignore_case(&WString::from(prefix))
    }
    /// C#'s ToUpperInvariant (also the key that makes a map OrdinalIgnoreCase).
    pub fn to_upper_invariant(&self) -> WString {
        WString(sys::upper_invariant(&self.0))
    }
    /// C#'s ToLowerInvariant.
    pub fn to_lower_invariant(&self) -> WString {
        WString(sys::lower_invariant(&self.0))
    }
}

/// char.IsWhiteSpace for one UTF-16 unit (Unicode White_Space; the same set .NET Framework 4.8 uses).
pub fn is_white_space(c: u16) -> bool {
    matches!(c, 0x09..=0x0D | 0x20 | 0x85 | 0xA0 | 0x1680 | 0x2000..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000)
}

impl From<&str> for WString {
    fn from(s: &str) -> Self {
        WString(s.encode_utf16().collect())
    }
}
impl From<String> for WString {
    fn from(s: String) -> Self {
        WString::from(s.as_str())
    }
}
impl From<&String> for WString {
    fn from(s: &String) -> Self {
        WString::from(s.as_str())
    }
}

impl Add<&str> for WString {
    type Output = WString;
    fn add(mut self, rhs: &str) -> WString {
        self.push_str(rhs);
        self
    }
}
impl Add<&WString> for WString {
    type Output = WString;
    fn add(mut self, rhs: &WString) -> WString {
        self.push_w(rhs);
        self
    }
}
impl Add<&String> for WString {
    type Output = WString;
    fn add(mut self, rhs: &String) -> WString {
        self.push_str(rhs);
        self
    }
}
impl AddAssign<&str> for WString {
    fn add_assign(&mut self, rhs: &str) {
        self.push_str(rhs);
    }
}
impl AddAssign<&WString> for WString {
    fn add_assign(&mut self, rhs: &WString) {
        self.push_w(rhs);
    }
}

impl fmt::Display for WString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_lossy())
    }
}
impl fmt::Debug for WString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.to_string_lossy())
    }
}

/// `w("...")`: a WString from a literal.
pub fn w(s: &str) -> WString {
    WString::from(s)
}
