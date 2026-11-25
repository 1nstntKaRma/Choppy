//! Cli.cs BenchCli: Choppy against Windows' own copy on every drive, headless: writes out.json and out.html (default:
//! the Logs folder). --list only lists the drives and the time it would take, into out.json as text. Exit code 0 =
//! done, 3 = stopped, 1 = error.
//!   choppy.exe --benchmark [out.json] [--thorough] [--network] [--drives C,D] [--list] [--log progress.txt]
//!                          [--cancel-after sec] [--tiny] [--choppy "engine switches"] [--cases ..] [--big MB]
//!                          [--rounds n] [--routes C>D,..]

use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::bench::benchmark::Benchmark;
use crate::bench::report::text;
use crate::engine::netfmt;
use crate::wstr::WString;

/// A path from the command line, against the folder Choppy was started from.
fn full(p: &str) -> PathBuf {
    let pb = PathBuf::from(p);
    if pb.is_absolute() || p.starts_with('\\') || p.get(1..2) == Some(":") {
        return pb;
    }
    match crate::engine::paths::start_dir() {
        Some(d) => PathBuf::from(d.to_os()).join(p),
        None => pb,
    }
}

fn int(s: &str) -> i32 {
    netfmt::parse_int(s).unwrap_or_else(|| panic!("Input string was not in a correct format."))
}

pub fn bench_cli(args: &[WString]) -> i32 {
    let a: Vec<String> = args.iter().map(|x| x.to_string_lossy()).collect();
    let mut b = Benchmark::new();
    let (mut out_path, mut log): (Option<PathBuf>, Option<PathBuf>) = (None, None);
    let mut cancel_after = 0.0;
    let mut list = false;
    let mut i = 1;
    while i < a.len() {
        let next = |i: &mut usize| -> String {
            *i += 1;
            a.get(*i).cloned().unwrap_or_else(|| panic!("Index was outside the bounds of the array."))
        };
        match a[i].as_str() {
            "--quick" => b.thorough = false,
            "--thorough" => b.thorough = true,
            "--network" => b.network = true,
            "--tiny" => b.tiny = true,
            "--list" => list = true,
            "--log" => log = Some(full(&next(&mut i))),
            "--cancel-after" => cancel_after = netfmt::parse_double(&next(&mut i)).unwrap_or_else(|| panic!("Input string was not in a correct format.")),
            // tests: Choppy's side with fixed engine switches (as --cli takes them), only some cases, a fixed big size, more rounds
            "--choppy" => {
                let opts: Vec<WString> = next(&mut i).split(' ').filter(|s| !s.is_empty()).map(WString::from).collect();
                b.tweak = Some(Arc::new(move |sp| {
                    let mut k = 0;
                    while k < opts.len() {
                        super::job::engine_option(&opts, &mut k, sp);
                        k += 1;
                    }
                }));
            }
            "--cases" => b.cases = Some(next(&mut i).split(',').map(|s| s.to_string()).collect()),
            "--big" => b.big_mb = int(&next(&mut i)),
            "--rounds" => b.rounds_override = int(&next(&mut i)),
            "--routes" => b.routes_only = Some(next(&mut i).to_uppercase().split(',').map(|s| s.to_string()).collect()),
            "--drives" => {
                let mut o = HashSet::new();
                for s in next(&mut i).split(',') {
                    if !s.trim().is_empty() {
                        o.insert(format!("{}:", s.trim().trim_end_matches(['\\', ':']).to_uppercase()));
                    }
                }
                b.only = Some(o);
            }
            s => {
                if out_path.is_none() && !s.starts_with("--") {
                    out_path = Some(full(s));
                }
            }
        }
        i += 1;
    }
    if list {
        let mut drives = Benchmark::find_drives(b.network, b.only.as_ref());
        let est = b.preview(&mut drives);
        let mut sb = String::new();
        for d in drives.iter() {
            sb += &format!(
                "{} {} | {} | {} | {} | {} | disk {} | {}\r\n",
                d.letter,
                d.label.as_deref().unwrap_or(""),
                if d.vol.is_some() || d.network { d.kind() } else { String::new() },
                d.model.as_deref().unwrap_or(""),
                d.bus.as_deref().unwrap_or(""),
                d.file_system.as_deref().unwrap_or(""),
                d.vol.as_ref().map(|v| v.disk).unwrap_or(-1),
                match &d.skip {
                    None => "tested".to_string(),
                    Some(s) => format!("skipped: {s}"),
                }
            );
        }
        sb += &format!("about {}{}\r\n", text::duration(est), if b.thorough { " (thorough)" } else { " (quick)" });
        if let Some(p) = &out_path {
            let mut bytes = vec![0xEF, 0xBB, 0xBF];
            bytes.extend_from_slice(sb.as_bytes());
            std::fs::write(p, bytes).unwrap_or_else(|e| panic!("{}", crate::bench::files::io_message(&e)));
        }
        return 0;
    }
    let lw: Option<Arc<Mutex<std::fs::File>>> = log.as_ref().map(|l| {
        if let Some(d) = l.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let mut f = std::fs::File::create(l).unwrap_or_else(|e| panic!("{}", crate::bench::files::io_message(&e)));
        let _ = f.write_all(&[0xEF, 0xBB, 0xBF]);
        Arc::new(Mutex::new(f))
    });
    if let Some(w) = &lw {
        let w = w.clone();
        b.log = Some(Arc::new(move |line| {
            let _ = writeln!(crate::engine::lock(&w), "{line}\r");
        }));
    }
    let b = Arc::new(b);
    if cancel_after > 0.0 {
        let bb = b.clone();
        std::thread::spawn(move || {
            crate::sys::sleep_ms((cancel_after * 1000.0) as u32);
            bb.cancel();
        });
    }
    let rep = b.run();
    let (html, _json) = rep.save(out_path.as_deref()).unwrap_or_else(|e| panic!("{}", crate::bench::files::io_message(&e)));
    if let Some(w) = &lw {
        let mut f = crate::engine::lock(w);
        let _ = write!(f, "{}\r\n", rep.summary());
        if let Some(t) = rep.totals() {
            let _ = write!(f, "{}\r\n", t.line());
        }
        let _ = write!(f, "{}\r\n", html.display());
    }
    if rep.canceled {
        3
    } else if rep.error.is_some() {
        1
    } else {
        0
    }
}
