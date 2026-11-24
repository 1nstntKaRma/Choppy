//! Format.cs, the parts the copy window uses ("." as the decimal point, like the README demo).

/// "1.6 GB", "600 MB", "18 MB" (Format.Size).
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
        format!("{v:.0}")
    } else if v >= 10.0 {
        trim(format!("{v:.1}"))
    } else {
        trim(format!("{v:.2}"))
    };
    format!("{n} {}", u[i.min(u.len() - 1)])
}

/// "0.#" / "0.##": no trailing zeros.
fn trim(s: String) -> String {
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// Same unit for both numbers: "11.8 GB" / "19.0 GB" (Format.Pair).
pub fn pair(a: i64, total: i64) -> (String, String) {
    let u = ["B", "KB", "MB", "GB", "TB"];
    let mut div = 1.0f64;
    let mut i = 0;
    while total as f64 / div >= 1000.0 && i < u.len() - 1 {
        div *= 1024.0;
        i += 1;
    }
    if i == 0 {
        (format!("{} {}", (a as f64 / div).round(), u[i]), format!("{} {}", (total as f64 / div).round(), u[i]))
    } else {
        (format!("{:.1} {}", a as f64 / div, u[i]), format!("{:.1} {}", total as f64 / div, u[i]))
    }
}

/// Speed number + unit (Format.Speed): local disks MB/s, network Mbps.
pub fn speed(bytes_per_sec: f64, mbps: bool) -> (String, &'static str) {
    if mbps {
        let v = bytes_per_sec * 8.0 / 1e6;
        if v >= 1000.0 {
            (if v >= 10000.0 { format!("{:.0}", v / 1000.0) } else { format!("{:.1}", v / 1000.0) }, "Gbps")
        } else if v >= 10.0 {
            (format!("{v:.0}"), "Mbps")
        } else {
            (format!("{v:.1}"), "Mbps")
        }
    } else {
        let v = bytes_per_sec / (1024.0 * 1024.0);
        if v >= 1000.0 {
            (format!("{:.1}", v / 1024.0), "GB/s")
        } else if v >= 100.0 {
            (format!("{v:.0}"), "MB/s")
        } else {
            (format!("{v:.1}"), "MB/s")
        }
    }
}

/// "12 s", "37 min", "1 h 5 min" (Format.Short; Math.Round = to even).
pub fn short(secs: f64) -> String {
    if secs < 60.0 {
        return format!("{} s", secs.round_ties_even().max(0.0) as i64);
    }
    let min = secs / 60.0;
    if min < 60.0 {
        return format!("{} min", min.round_ties_even() as i64);
    }
    let h = (secs / 3600.0) as i64;
    let m = ((secs as i64) / 60) % 60;
    if m > 0 { format!("{h} h {m} min") } else { format!("{h} h") }
}

/// "1,234" (Format.Count).
pub fn count(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

/// The speed limit (kept in Mbps) in the unit on screen, a whole number, at least 1 (Format.Limit).
pub fn limit(mbps: f64, as_mbps: bool) -> String {
    let v = if as_mbps { mbps } else { mbps * 1e6 / 8.0 / (1024.0 * 1024.0) };
    format!("{}", v.round().max(1.0))
}

/// A typed limit: whole numbers only (0 = not one) (Format.ParseLimit).
pub fn parse_limit(text: &str) -> i64 {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return 0;
    }
    text.parse::<i64>().unwrap_or(0).max(0)
}

pub fn limit_to_mbps(value: f64, as_mbps: bool) -> f64 {
    if as_mbps { value } else { value * (1024.0 * 1024.0) * 8.0 / 1e6 }
}
