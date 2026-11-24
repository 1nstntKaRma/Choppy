//! Numbers written and read the way the C# code does with CultureInfo.InvariantCulture on .NET Framework 4.8, so
//! settings, job records, logs and reports keep their exact text: double.ToString() ("G", 15 digits),
//! ToString("F<n>"), double.TryParse(NumberStyles.Float), int.TryParse.

/// The 15 significant digits .NET Framework turns a double into, and the decimal exponent of the first one:
/// x = 0.d1d2...d15 x 10^(exp). None for 0. Rounded half up from the exact value, as .NET Framework does
/// (100000000000000.5 gives 100000000000001; Rust's own 15-digit rounding goes to even).
fn digits15(x: f64) -> Option<(Vec<u8>, i32)> {
    if x == 0.0 {
        return None;
    }
    let s = format!("{:.39e}", x.abs()); // 40 digits: the 16th is exact
    let (mant, exp) = s.split_once('e').unwrap();
    let mut exp: i32 = exp.parse().unwrap();
    let all: Vec<u8> = mant.bytes().filter(|b| b.is_ascii_digit()).map(|b| b - b'0').collect();
    let mut d = all[..15].to_vec();
    if all[15] >= 5 {
        let mut i = 15;
        loop {
            if i == 0 {
                d.insert(0, 1);
                d.truncate(15);
                exp += 1;
                break;
            }
            i -= 1;
            if d[i] == 9 {
                d[i] = 0;
            } else {
                d[i] += 1;
                break;
            }
        }
    }
    Some((d, exp + 1))
}

/// double.ToString(CultureInfo.InvariantCulture) on .NET Framework: "G" with 15 significant digits - fixed notation
/// unless the exponent is below -5 or 15 and above ("1E-05", "1E+15"); "NaN", "Infinity", "-Infinity"; -0 is "0".
pub fn g(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    let Some((mut d, pos)) = digits15(x) else { return "0".into() };
    while d.len() > 1 && *d.last().unwrap() == 0 {
        d.pop();
    }
    let sign = if x < 0.0 { "-" } else { "" };
    let e = pos - 1; // scientific exponent
    let digits: String = d.iter().map(|b| (b + b'0') as char).collect();
    if !(-4..15).contains(&e) {
        let mut m = digits[..1].to_string();
        if digits.len() > 1 {
            m.push('.');
            m.push_str(&digits[1..]);
        }
        return format!("{sign}{m}E{}{:02}", if e < 0 { '-' } else { '+' }, e.abs());
    }
    if pos <= 0 {
        return format!("{sign}0.{}{}", "0".repeat((-pos) as usize), digits);
    }
    let pos = pos as usize;
    if digits.len() <= pos {
        format!("{sign}{}{}", digits, "0".repeat(pos - digits.len()))
    } else {
        format!("{sign}{}.{}", &digits[..pos], &digits[pos..])
    }
}

/// double.ToString("F<decimals>", CultureInfo.InvariantCulture) on .NET Framework: the 15 significant digits, then
/// rounded half away from zero to the decimals asked for (0.125 -> "0.13"; Rust's own formatting gives "0.12").
pub fn f(x: f64, decimals: usize) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    let zero = || if decimals == 0 { "0".to_string() } else { format!("0.{}", "0".repeat(decimals)) };
    let Some((mut d, mut pos)) = digits15(x) else { return zero() };
    // keep the digits up to `decimals` after the point, round half up on the next one
    let keep = pos + decimals as i32;
    if keep < 0 {
        return zero();
    }
    let keep = keep as usize;
    if keep < d.len() {
        let up = d[keep] >= 5;
        d.truncate(keep);
        if up {
            let mut i = d.len();
            loop {
                if i == 0 {
                    d.insert(0, 1);
                    pos += 1;
                    break;
                }
                i -= 1;
                if d[i] == 9 {
                    d[i] = 0;
                } else {
                    d[i] += 1;
                    break;
                }
            }
        }
    }
    let all_zero = d.iter().all(|&b| b == 0);
    let sign = if x < 0.0 && !all_zero { "-" } else { "" };
    // lay the digits out around the point
    let mut int_part = String::new();
    let mut frac = String::new();
    for k in 0..(pos.max(0) as usize) {
        int_part.push(d.get(k).map_or('0', |b| (b + b'0') as char));
    }
    if int_part.is_empty() {
        int_part.push('0');
    }
    for k in 0..decimals {
        let idx = pos + k as i32;
        frac.push(if idx < 0 { '0' } else { d.get(idx as usize).map_or('0', |b| (b + b'0') as char) });
    }
    if decimals == 0 { format!("{sign}{int_part}") } else { format!("{sign}{int_part}.{frac}") }
}

fn is_number_white(c: char) -> bool {
    // .NET's number parsing skips U+0009-U+000D and U+0020
    matches!(c, '\u{9}'..='\u{d}' | ' ')
}

/// double.TryParse(s, NumberStyles.Float, CultureInfo.InvariantCulture): white space around, a sign, digits with
/// one '.', an exponent; "Infinity", "-Infinity", "NaN" as .NET writes them. No thousands separators.
pub fn parse_double(s: &str) -> Option<f64> {
    let t = s.trim_matches(is_number_white);
    match t {
        "NaN" => return Some(f64::NAN),
        "Infinity" | "+Infinity" => return Some(f64::INFINITY),
        "-Infinity" => return Some(f64::NEG_INFINITY),
        _ => {}
    }
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let mut digits = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
        digits += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let start = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j == start {
            return None;
        }
        i = j;
    }
    if i != b.len() {
        return None;
    }
    t.parse::<f64>().ok()
}

/// int.TryParse(s): white space around, a sign, digits; within int's range.
pub fn parse_int(s: &str) -> Option<i32> {
    let t = s.trim_matches(is_number_white);
    let b = t.as_bytes();
    let digits = if !b.is_empty() && (b[0] == b'+' || b[0] == b'-') { &b[1..] } else { b };
    if digits.is_empty() || !digits.iter().all(|c| c.is_ascii_digit()) {
        return None;
    }
    t.parse::<i32>().ok()
}

/// long.Parse / TryParse.
pub fn parse_long(s: &str) -> Option<i64> {
    let t = s.trim_matches(is_number_white);
    let b = t.as_bytes();
    let digits = if !b.is_empty() && (b[0] == b'+' || b[0] == b'-') { &b[1..] } else { b };
    if digits.is_empty() || !digits.iter().all(|c| c.is_ascii_digit()) {
        return None;
    }
    t.parse::<i64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g_like_net_framework() {
        assert_eq!(g(0.0), "0");
        assert_eq!(g(-0.0), "0");
        assert_eq!(g(2.0), "2");
        assert_eq!(g(0.75), "0.75");
        assert_eq!(g(1.03), "1.03");
        assert_eq!(g(12.0 * 8.0 * 1.048576), "100.663296");
        assert_eq!(g(0.1 + 0.2), "0.3");
        assert_eq!(g(1e15), "1E+15");
        assert_eq!(g(123456789012345.0), "123456789012345");
        assert_eq!(g(0.0001), "0.0001");
        assert_eq!(g(0.00001), "1E-05");
        assert_eq!(g(-1.5e-7), "-1.5E-07");
        assert_eq!(g(1.5e300), "1.5E+300");
        assert_eq!(g(f64::NAN), "NaN");
        // measured on .NET Framework 4.8 (PowerShell 5.1)
        assert_eq!(g(0.000123456), "0.000123456");
        assert_eq!(g(999999999999999.0), "999999999999999");
        assert_eq!(g(100000000000000.5), "100000000000001");
        assert_eq!(g(2.5e-5), "2.5E-05");
        assert_eq!(g(3e-5), "3E-05");
        assert_eq!(g(4.9406564584124654E-324), "4.94065645841247E-324");
        assert_eq!(g(1.7976931348623157e308), "1.79769313486232E+308");
        assert_eq!(g(1e16), "1E+16");
        assert_eq!(g(12345678901234567890.0), "1.23456789012346E+19");
        assert_eq!(g(f64::INFINITY), "Infinity");
    }

    #[test]
    fn f_like_net_framework() {
        assert_eq!(f(0.125, 2), "0.13");
        assert_eq!(f(2.0, 2), "2.00");
        assert_eq!(f(9.995, 2), "10.00");
        assert_eq!(f(-0.004, 2), "0.00");
        assert_eq!(f(12.345678, 1), "12.3");
        assert_eq!(f(0.0, 1), "0.0");
        assert_eq!(f(1234.5, 0), "1235");
        assert_eq!(f(-1.0, 1), "-1.0");
        assert_eq!(f(0.05, 1), "0.1");
        assert_eq!(f(1e20, 2), "100000000000000000000.00");
        assert_eq!(f(2.675, 2), "2.68");
        assert_eq!(f(1.005, 2), "1.01");
    }

    #[test]
    fn parse_like_net_framework() {
        assert_eq!(parse_double(" 12.5 "), Some(12.5));
        assert_eq!(parse_double("1e3"), Some(1000.0));
        assert_eq!(parse_double(".5"), Some(0.5));
        assert_eq!(parse_double("5."), Some(5.0));
        assert_eq!(parse_double("1,000"), None);
        assert_eq!(parse_double("inf"), None);
        assert_eq!(parse_double(""), None);
        assert_eq!(parse_int(" 42"), Some(42));
        assert_eq!(parse_int("+7"), Some(7));
        assert_eq!(parse_int("4.0"), None);
        assert_eq!(parse_int("99999999999"), None);
    }
}
