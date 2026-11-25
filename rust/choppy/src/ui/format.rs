//! Format.cs: sizes, speeds, times and dates as the windows show them. Numbers follow the user's locale (C#'s
//! CultureInfo.CurrentCulture: its decimal point and thousands separator); custom formats round half away from zero
//! like .NET Framework.

use std::sync::OnceLock;
use std::time::Duration;

use crate::engine::netfmt;
use crate::sys;

struct Culture {
    decimal: String,
    group: String,
    h24: bool,
}

fn culture() -> &'static Culture {
    static C: OnceLock<Culture> = OnceLock::new();
    C.get_or_init(|| {
        let l = sys::locale_info();
        Culture { decimal: l.decimal, group: l.group, h24: l.short_time.contains('H') }
    })
}

/// `v` with up to `max` decimals and at least `min` (.NET's "0.##" = (2, 0), "0.0" = (1, 1), "0" = (0, 0)).
pub fn num(v: f64, max: usize, min: usize) -> String {
    let s = netfmt::f(v, max);
    let s = if max > min && s.contains('.') {
        let (a, b) = s.split_once('.').unwrap_or((&s, ""));
        let mut b = b.to_string();
        while b.len() > min && b.ends_with('0') {
            b.pop();
        }
        if b.is_empty() { a.to_string() } else { format!("{a}.{b}") }
    } else {
        s
    };
    // "-0" stays "0" as .NET writes it for a custom format that rounds to zero
    let s = if s.starts_with('-') && s[1..].chars().all(|c| c == '0' || c == '.') { s[1..].to_string() } else { s };
    s.replace('.', &culture().decimal)
}

/// "N0": whole number with the thousands separator.
pub fn count(n: i64) -> String {
    group(&n.unsigned_abs().to_string(), n < 0, &culture().group)
}

/// "N0" with the invariant culture ("1,000").
pub fn count_inv(n: i64) -> String {
    group(&n.unsigned_abs().to_string(), n < 0, ",")
}

fn group(digits: &str, neg: bool, sep: &str) -> String {
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push_str(sep);
        }
        out.push(c);
    }
    if neg { format!("-{out}") } else { out }
}

/// "1.6 GB", "600 MB", "18 KB", "512 B".
pub fn size(bytes: i64) -> String {
    let bytes = bytes.max(0);
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let u = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut i = 0;
    while v >= 1000.0 && i < u.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if v >= 1000.0 {
        v /= 1024.0;
        i += 1;
    }
    let n = if v >= 100.0 {
        num(v, 0, 0)
    } else if v >= 10.0 {
        num(v, 1, 0)
    } else {
        num(v, 2, 0)
    };
    // (C# indexes past "TB" only for sizes beyond any disk)
    format!("{n} {}", u[i.min(u.len() - 1)])
}

/// Same unit for both numbers: "11.8 GB" / "19.0 GB".
pub fn pair(a: i64, total: i64) -> (String, String) {
    let u = ["B", "KB", "MB", "GB", "TB"];
    let mut div = 1.0f64;
    let mut i = 0;
    while total as f64 / div >= 1000.0 && i < u.len() - 1 {
        div *= 1024.0;
        i += 1;
    }
    let (mx, mn) = if i == 0 { (0, 0) } else { (1, 1) };
    (format!("{} {}", num(a as f64 / div, mx, mn), u[i]), format!("{} {}", num(total as f64 / div, mx, mn), u[i]))
}

/// Speed number + unit. Network jobs default to Mbps (what the ISP talks in), local disks to MB/s.
pub fn speed(bytes_per_sec: f64, mbps: bool) -> (String, &'static str) {
    if mbps {
        let v = bytes_per_sec * 8.0 / 1e6;
        if v >= 1000.0 {
            (if v >= 10000.0 { num(v / 1000.0, 0, 0) } else { num(v / 1000.0, 1, 1) }, "Gbps")
        } else if v >= 10.0 {
            (num(v, 0, 0), "Mbps")
        } else {
            (num(v, 1, 1), "Mbps")
        }
    } else {
        let v = bytes_per_sec / (1024.0 * 1024.0);
        if v >= 1000.0 {
            (num(v / 1024.0, 1, 1), "GB/s")
        } else if v >= 100.0 {
            (num(v, 0, 0), "MB/s")
        } else {
            (num(v, 1, 1), "MB/s")
        }
    }
}

/// "under a second", "12 s", "3 min 4 s", "1 h 5 min".
pub fn duration(t: Duration) -> String {
    let s = t.as_secs_f64();
    if s < 1.0 {
        return "under a second".into();
    }
    if s < 60.0 {
        return format!("{} s", s as i64);
    }
    let secs = t.as_secs();
    if s / 60.0 < 60.0 {
        return format!("{} min {} s", (s / 60.0) as i64, secs % 60);
    }
    format!("{} h {} min", (s / 3600.0) as i64, (secs / 60) % 60)
}

/// "12 s", "37 min", "1 h 5 min" (elapsed / remaining in the copy window). Math.Round: halves to even.
pub fn short(t: Duration) -> String {
    let s = t.as_secs_f64();
    if s < 60.0 {
        return format!("{} s", s.round_ties_even().max(0.0) as i64);
    }
    if s / 60.0 < 60.0 {
        return format!("{} min", (s / 60.0).round_ties_even() as i64);
    }
    let h = (s / 3600.0) as i64;
    let m = (t.as_secs() / 60) % 60;
    if m > 0 { format!("{h} h {m} min") } else { format!("{h} h") }
}

/// The speed limit (kept in Mbps) in the unit shown, as a whole number (at least 1).
pub fn limit(mbps: f64, as_mbps: bool) -> String {
    let v = if as_mbps { mbps } else { mbps * 1e6 / 8.0 / (1024.0 * 1024.0) };
    num(v.round_ties_even().max(1.0), 0, 0)
}

/// A typed limit: whole numbers only (0 = not one). int.TryParse with NumberStyles.None: digits only.
pub fn parse_limit(text: &str) -> i32 {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return 0;
    }
    text.parse::<i32>().ok().filter(|v| *v > 0).unwrap_or(0)
}

pub fn limit_to_mbps(value: f64, as_mbps: bool) -> f64 {
    if as_mbps { value } else { value * (1024.0 * 1024.0) * 8.0 / 1e6 }
}

pub fn remaining(t: Duration) -> String {
    let s = t.as_secs_f64();
    if s < 5.0 {
        return "a few seconds remaining".into();
    }
    if s < 60.0 {
        return format!("{} s remaining", (s / 5.0).ceil() as i64 * 5);
    }
    if s / 60.0 < 60.0 {
        return format!("{} min remaining", (s / 60.0).ceil() as i64);
    }
    let h = (s / 3600.0) as i64;
    let m = (t.as_secs() / 60) % 60;
    format!("{h} h {}remaining", if m > 0 { format!("{m} min ") } else { String::new() })
}

/// A local time as the windows show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct When {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

impl When {
    /// From a local FILETIME (100 ns since 1601, already local).
    pub fn from_local_file_time(ft: i64) -> When {
        let days = ft.div_euclid(864_000_000_000);
        let rest = ft.rem_euclid(864_000_000_000) / 10_000_000;
        let (y, m, d) = civil(days - 134774); // days since 1970-01-01
        When { year: y, month: m, day: d, hour: (rest / 3600) as u32, minute: ((rest / 60) % 60) as u32 }
    }
    fn day_of_week(&self) -> usize {
        let z = days_from_civil(self.year, self.month, self.day);
        ((z + 4).rem_euclid(7)) as usize // 1970-01-01 was a Thursday
    }
}

fn civil(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y as i64 - 1 } else { y as i64 };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// The user's clock style (24 h or AM/PM), with English names to match the English UI.
fn time_text(w: &When) -> String {
    if culture().h24 {
        format!("{:02}:{:02}", w.hour, w.minute)
    } else {
        let h = if w.hour.is_multiple_of(12) { 12 } else { w.hour % 12 };
        format!("{h}:{:02} {}", w.minute, if w.hour < 12 { "AM" } else { "PM" })
    }
}

pub fn today() -> When {
    When::from_local_file_time(sys::now_local_file_time())
}

/// "14:14", or "Mon 14:14" when it isn't today.
pub fn clock(w: &When) -> String {
    let s = time_text(w);
    let t = today();
    if (w.year, w.month, w.day) != (t.year, t.month, t.day) { format!("{} {s}", DAYS[w.day_of_week()]) } else { s }
}

/// "21 Nov 2026, 14:14"; None (C#'s DateTime.MinValue) = "unknown".
pub fn date(w: Option<&When>) -> String {
    match w {
        None => "unknown".into(),
        Some(w) => format!("{} {} {}, {}", w.day, MONTHS[(w.month as usize).clamp(1, 12) - 1], w.year, time_text(w)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_as_net_writes_them() {
        assert_eq!(num(2.125, 2, 0).replace(&culture().decimal, "."), "2.13");
        assert_eq!(num(10.04, 1, 0).replace(&culture().decimal, "."), "10");
        assert_eq!(num(0.04, 1, 1).replace(&culture().decimal, "."), "0.0");
        assert_eq!(count_inv(1234567), "1,234,567");
        let d = &culture().decimal;
        assert_eq!(size(1_600_000_000), format!("1{d}49 GB"));
        assert_eq!(size(999), "999 B");
        assert_eq!(size(1023 * 1024), "1 MB");
        assert_eq!(short(Duration::from_secs_f64(29.5)), "30 s");
        assert_eq!(short(Duration::from_secs_f64(30.5)), "30 s");
        assert_eq!(short(Duration::from_secs(3900)), "1 h 5 min");
        assert_eq!(duration(Duration::from_secs(62)), "1 min 2 s");
        assert_eq!(limit(478.150656, false), "57");
    }

    #[test]
    fn dates() {
        // 2026-11-21 14:14 local
        let ft = (days_from_civil(2026, 11, 21) + 134774) * 864_000_000_000 + (14 * 3600 + 14 * 60) * 10_000_000;
        let w = When::from_local_file_time(ft);
        assert_eq!((w.year, w.month, w.day, w.hour, w.minute), (2026, 11, 21, 14, 14));
        assert_eq!(DAYS[w.day_of_week()], "Sat");
    }
}
