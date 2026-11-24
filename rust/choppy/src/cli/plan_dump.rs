//! `--plan-dump out.txt` (Cli.PlanDump, added to the C# on the rust branch for the move): a read-only check that the
//! Rust Choppy sees the drives and picks the plan exactly as the C# one does. Both print the same lines:
//! - drives: what VolumeProbe sees on every drive (and on \\localhost\C$, a network share);
//! - plans: the plan Plan.Choose picks for every pair of drives, for each kind of job (copy, move, a retried move)
//!   and each shape of scan (none, empty, one big file, a few, many small, mixed);
//! - paths: Paths.ToLong / ToShort / FixArg / Parent / FileName on tricky input, IsSameOrInside / SamePath, NumberedName;
//! - errors: Native.ErrorText for Win32 codes 0-1999 and a few more;
//! - in test mode (CHOPPY_DATA) also settings: the settings as loaded (and settings.ini after), and a job record
//!   written and read back.
//!
//! `--plan-dump out.txt --scan copy|move <dest> <src>...`: the Scanner's result on those sources instead (with one
//! thread and with four), and the plan chosen on it.

use std::fs;

use super::{append_line, drive_roots, write_all_text};
use crate::engine::big_copy::{PipeTuner, PipeTuning};
use crate::engine::model::{ConflictPolicy, DirItem, FileItem, JobSpec, OpKind, ScanResult};
use crate::engine::netfmt;
use crate::engine::paths;
use crate::engine::plan::Plan;
use crate::engine::scanner::Scanner;
use crate::engine::settings::{Settings, app_paths, journal};
use crate::engine::usb_link;
use crate::engine::volume_probe::{self, VolumeInfo};
use crate::sys;
use crate::wstr::{WString, w};

fn b(x: bool) -> &'static str {
    if x { "1" } else { "0" }
}

fn opt(s: &Option<String>) -> String {
    s.clone().unwrap_or_else(|| "(null)".into())
}

fn vol_text(v: &VolumeInfo) -> String {
    let usb = match &v.usb {
        None => "-".to_string(),
        Some(u) => format!(
            "{} {}, can {}{}, port {}, bcd {:04X}, uasp {}, heldback {}, advice {}, ceiling {}",
            u.speed,
            u.speed_text(),
            u.capable,
            if u.capable_known { "" } else { "?" },
            if u.port_known { if u.port_usb3 { "3" } else { "2" } } else { "?" },
            u.bcd_usb,
            b(u.uasp),
            b(u.held_back()),
            u.advice().unwrap_or_else(|| "-".into()),
            netfmt::g(usb_link::ceiling_mbps(u.speed))
        ),
    };
    format!(
        "root {} | id {} | kind {} {} | fs {} | label {} | disk {} | sector {} | removable {} | fsflags {:08X} | onusb {} | rtt {} | maxfile {} | clone {} | slowseek {} | usb {}",
        v.root,
        v.id,
        v.kind.name(),
        v.kind_text(),
        opt(&v.file_system),
        opt(&v.label),
        v.disk,
        v.sector,
        b(v.removable),
        v.fs_flags,
        b(v.on_usb),
        if v.rtt_ms >= 0.0 { "measured".to_string() } else { netfmt::g(v.rtt_ms) },
        v.max_file_size(),
        b(v.can_clone()),
        b(v.is_slow_seek()),
        usb
    )
}

fn pipe_text(t: &PipeTuning) -> String {
    format!(
        "{} (chunk {} slots {} align {} batch {} phased {} prealloc {} cached {} tail {} depth {} step {})",
        PipeTuner::describe(t),
        t.chunk,
        t.slots,
        t.align,
        t.write_batch,
        b(t.phased),
        b(t.prealloc),
        t.cached_write_chunk,
        b(t.tail_after_read),
        t.read_depth,
        t.write_step
    )
}

fn plan_text(p: &Plan) -> String {
    let mut s = format!(
        "{} | workers {} max {} | big {} from {} | pipeline {} from {} {} | ways {}",
        p.kind,
        p.workers,
        p.max_workers,
        p.big_slots,
        p.big_threshold,
        b(p.pipeline),
        p.pipe_threshold,
        pipe_text(&p.pipe),
        p.pipe_ways.len()
    );
    for way in &p.pipe_ways {
        s += &format!(" [{}]", pipe_text(way));
    }
    s += &format!(
        " | instant {} network {} clone {} oneusb {} smallbatch {} | tune {} {} s, x{} tuned {} {} | {} | {}",
        b(p.instant_move),
        b(p.network),
        b(p.clone),
        b(p.one_usb),
        b(p.small_batch),
        b(p.can_tune),
        netfmt::g(p.tune_seconds),
        netfmt::g(p.tune_margin),
        b(p.tuned),
        p.tune_note.clone().unwrap_or_else(|| "-".into()),
        p.badge(),
        p.why()
    );
    s
}

const GB: i64 = 1 << 30;
const MB: i64 = 1 << 20;
const KB: i64 = 1 << 10;

/// The shapes of scan each pair is planned for: (name, files, median, largest); files -1 = no scan.
const SHAPES: [(&str, i64, i64, i64); 12] = [
    ("none", -1, 0, 0),
    ("empty", 0, 0, 0),
    ("1 x 2 GB", 1, 2 * GB, 2 * GB),
    ("3 x 1 GB", 3, GB, GB),
    ("4 x 1 GB", 4, GB, GB),
    ("20 x 4 MB", 20, 4 * MB, 4 * MB),
    ("20 x 16 MB", 20, 16 * MB, 16 * MB),
    ("40 x 512 KB", 40, 512 * KB, 512 * KB),
    ("40 x 2 MB", 40, 2 * MB, 2 * MB),
    ("100 x 64 KB", 100, 64 * KB, MB),
    ("100 x 16 MB", 100, 16 * MB, 16 * MB),
    ("1001 mixed", 1001, 100 * KB, 2 * GB),
];

fn shape(files: i64, median: i64, largest: i64) -> ScanResult {
    let mut s = ScanResult::default();
    for _ in 0..files {
        s.files.push(std::sync::Arc::new(FileItem::default()));
    }
    s.median_size = median;
    if files > 0 {
        s.largest.push((w("big.bin"), largest));
    }
    s
}

/// Paths.ToLong and friends get these (the same list as the C#).
const PATH_INPUTS: [&str; 51] = [
    r"C:\a\b\",
    r"C:\",
    "C:\"",
    "C:",
    r"C:\\x\\\\y",
    "D:\\Games\"",
    "\"D:\\Games\"",
    "  C:\\x  ",
    r"\\srv\share\x",
    r"\\srv\share\",
    r"\\srv\share",
    r"\\?\C:\x\\",
    "\\\\?\\C:\"",
    r"\\?\UNC\srv\share\x",
    r"\\.\COM1",
    r"relative\x",
    r".\x\..\y",
    "..",
    r"C:\a\..\b\.\c",
    r"C:\a/b//c",
    "C:\\a\\b. ",
    r"C:\a\b...",
    r"C:\a*b",
    r"C:\a?b",
    r"C:\a|b",
    r"C:\a<b",
    "a:b",
    r"C:\x\f:s",
    ":x",
    r"1:\x",
    "C:x",
    r"\x",
    r"C:\PROGRA~1\x",
    r"C:\PROGRA~1\Common Files",
    r"C:\PROGRA~1",
    r"C:\PROGRA~1\Common Files\x",
    r"C:\PROGRA~2\..\PROGRA~1",
    r"C:\PROGRA~1\aaa",
    r"C:\PROGRA~1\aaaa",
    r"C:\PROGRA~1\COMMON~1\x",
    r"\\localhost\C$\PROGRA~1\x",
    r"C:\Users\Public\..\Public",
    r"C:\con",
    r"C:\a\b\NUL",
    "",
    "   ",
    "\t C:\\tab",
    "\u{e9}\\\u{fc}",
    "C:\\x\\\u{1F600}.txt",
    "C:\\x\u{00A0}",
    "C:\\x\\\u{2028}",
];

/// IsSameOrInside / SamePath get these pairs.
const PATH_PAIRS: [(&str, &str); 10] = [
    (r"\\?\C:\a\b\c", r"\\?\C:\a\b"),
    (r"\\?\C:\a\bc", r"\\?\C:\a\b"),
    (r"\\?\C:\A\B", r"\\?\c:\a\b\"),
    (r"C:\x", r"\\?\C:\X"),
    (r"\\?\UNC\srv\share\x", r"\\srv\SHARE"),
    ("\\\\?\\C:\\\u{c4}\\x", "\\\\?\\C:\\\u{e4}"),
    ("\\\\?\\C:\\\u{df}", "\\\\?\\C:\\SS"),
    ("\\\\?\\C:\\\u{131}", "\\\\?\\C:\\I"),
    ("\\\\?\\C:\\\u{3a3}", "\\\\?\\C:\\\u{3c3}"),
    (r"\\?\C:\a", r"\\?\C:\a"),
];

/// NumberedName gets these.
const NUMBERED: [(&str, i32, bool); 6] =
    [("photo.jpg", 2, false), ("README", 3, false), (".hidden", 2, false), ("dir.name", 2, true), ("a.b.c", 10, false), ("x.", 4, false)];

/// Native.ErrorText gets 0-1999 and these.
const MORE_ERRORS: [i64; 11] = [2250, 4390, 4392, 6800, 8202, 10054, 10060, 12002, 0x7FFFFFFF, -1, -2147024891];

fn opt_w(p: &Option<WString>) -> String {
    match p {
        None => "(null)".into(),
        Some(x) => x.to_string_lossy(),
    }
}

pub fn plan_dump(args: &[WString]) -> i32 {
    let Some(out_file) = args.get(1) else { return 1 };
    let mut sb = String::new();
    if args.get(2).is_some_and(|a| a.eq_str("--scan")) {
        dump_scan(args, &mut sb);
    } else {
        append_line(&mut sb, "drives");
        let mut vols: Vec<(String, VolumeInfo)> = Vec::new();
        for root in drive_roots() {
            let v = volume_probe::probe(&paths::to_long(&root));
            let letter = root.slice(0, 2).to_string_lossy();
            append_line(&mut sb, &format!("{} {}", letter, vol_text(&v)));
            vols.push((letter, v));
        }
        // a network share too (this PC's own, through the network): network plans for every shape
        let net = volume_probe::probe(&paths::to_long(&w(r"\\localhost\C$\")));
        append_line(&mut sb, &format!("UNC {}", vol_text(&net).replace(" | rtt measured", " | rtt -")));
        vols.push(("UNC".into(), net));
        append_line(&mut sb, "plans");
        let ops: [(&str, OpKind, bool); 3] = [("copy", OpKind::Copy, false), ("move", OpKind::Move, false), ("retry move", OpKind::Move, true)];
        for (a, va) in &vols {
            for (bn, vb) in &vols {
                for (op_name, op, retry) in &ops {
                    let mut spec = JobSpec::new(*op);
                    if *retry {
                        spec.explicit_items = Some(Vec::new());
                    }
                    for (name, files, median, largest) in SHAPES {
                        let scan = (files >= 0).then(|| shape(files, median, largest));
                        let p = Plan::choose(&spec, Some(va), Some(vb), scan.as_ref());
                        append_line(&mut sb, &format!("{a}>{bn} {op_name} {name} | {}", plan_text(&p)));
                    }
                }
            }
        }
        // no drive at one end (a job with no sources)
        let p = Plan::choose(&JobSpec::new(OpKind::Copy), None, vols.first().map(|v| &v.1), None);
        append_line(&mut sb, &format!("(none)>first copy none | {}", plan_text(&p)));

        append_line(&mut sb, "paths");
        for input in PATH_INPUTS {
            let i = w(input);
            let long = paths::to_long(&i);
            append_line(
                &mut sb,
                &format!(
                    "[{}] long [{}] short [{}] fix [{}] parent [{}] name [{}] unc {} exists {}",
                    i,
                    long,
                    paths::to_short(&long),
                    paths::fix_arg(&i),
                    opt_w(&paths::parent(&long)),
                    paths::file_name(&long),
                    b(paths::is_unc(&long)),
                    b(paths::exists(&long))
                ),
            );
        }
        for (x, y) in PATH_PAIRS {
            append_line(
                &mut sb,
                &format!(
                    "[{}] [{}] inside {} same {}",
                    x,
                    y,
                    b(paths::is_same_or_inside(&w(x), &w(y))),
                    b(paths::same_path(&w(x), &w(y)))
                ),
            );
        }
        for (name, n, dir) in NUMBERED {
            append_line(&mut sb, &format!("[{}] {} {} => [{}]", name, n, b(dir), paths::numbered_name(&w(name), n, dir)));
        }
        append_line(&mut sb, "errors");
        for code in (0..2000i64).chain(MORE_ERRORS) {
            append_line(&mut sb, &format!("{} {}", code, sys::error_text(code as i32 as u32)));
        }
        if app_paths::test_mode() {
            dump_settings(&mut sb);
        }
    }
    write_all_text(out_file, &sb, false).expect("--plan-dump: the output file");
    0
}

fn dump_settings(sb: &mut String) {
    append_line(sb, "settings");
    let s = Settings::load();
    append_line(
        sb,
        &format!(
            "verify {} | policy {} | limit {} | value {} | close {} after {} | takeover {} | unit {} | backup {} | extended {} | runs {} {}",
            b(s.verify_copies),
            s.policy.name(),
            netfmt::g(s.speed_limit_mbps),
            netfmt::g(s.limit_value_mbps),
            b(s.close_when_done),
            s.close_after_seconds,
            b(s.explorer_takeover),
            opt(&s.speed_unit),
            opt(&s.ultracopier_run_backup),
            b(s.extended),
            s.bench_runs_quick,
            s.bench_runs_thorough
        ),
    );
    append_line(sb, "settings.ini");
    if let Ok(t) = fs::read_to_string(app_paths::data_dir().join("settings.ini")) {
        sb.push_str(t.trim_start_matches('\u{feff}')); // File.ReadAllText drops a byte order mark
    }
    append_line(sb, "journal");
    let mut spec = JobSpec {
        id: "plandump".into(),
        op: OpKind::Move,
        dest: paths::to_long(&w(r"X:\Dest folder\")),
        verify: true,
        policy: ConflictPolicy::KeepBoth,
        speed_limit_mbps: 57.0 * 8.0 * 1.048576,
        origin: "Right-click".into(),
        ..Default::default()
    };
    spec.sources.push(paths::to_long(&w(r"C:\a\b.txt")));
    spec.sources.push(paths::to_long(&w("\\\\srv\\share\\\u{e9} \u{fc}")));
    spec.explicit_items = Some(vec![FileItem { src: w(r"\\?\C:\a\c.bin"), rel: w(r"sub\c.bin"), ..Default::default() }]);
    spec.explicit_dirs = Some(vec![DirItem { src: w(r"\\?\C:\a\sub"), rel: w(r"x\sub"), ..Default::default() }]);
    let _ = fs::create_dir_all(app_paths::job_dir());
    let f = app_paths::job_dir().join("plandump.job");
    journal::write(&spec, &f, &["extra=1".to_string()]).expect("job record");
    sb.push_str(&fs::read_to_string(&f).unwrap_or_default());
    let r = journal::read(&f).expect("job record").expect("a job");
    let _ = fs::remove_file(&f);
    let items: Vec<String> = r.explicit_items.iter().flatten().map(|i| format!("{} > {}", i.src, i.rel)).collect();
    let dirs: Vec<String> = r.explicit_dirs.iter().flatten().map(|d| format!("{} > {} @{}", d.src, d.rel, d.depth)).collect();
    let sources: Vec<String> = r.sources.iter().map(|s| s.to_string_lossy()).collect();
    append_line(
        sb,
        &format!(
            "read {} | {} | {} | verify {} | {} | limit {} | {} | {} | {} | {} | {}",
            r.id,
            r.op.name(),
            r.dest,
            b(r.verify),
            r.policy.name(),
            netfmt::g(r.speed_limit_mbps),
            r.origin,
            sources.join(" ; "),
            items.join(" ; "),
            dirs.join(" ; "),
            r.title()
        ),
    );
    // titles
    let mut t = JobSpec::new(OpKind::Copy);
    t.sources.push(paths::to_long(&w(r"C:\x\photo.jpg")));
    append_line(sb, &t.title());
    t.sources.push(paths::to_long(&w(r"C:\x\other")));
    append_line(sb, &t.title());
    t.op = OpKind::Move;
    t.explicit_items = Some(vec![FileItem::default()]);
    append_line(sb, &t.title());
    t.explicit_items = Some(vec![FileItem::default(), FileItem::default(), FileItem::default()]);
    t.elevated = true;
    append_line(sb, &t.title());
    // records: an administrator copy has none; one still being written is removed once it is written;
    // unfinished ones come back to resume, one with nothing to do is deleted
    let has = |id: &str| b(app_paths::job_dir().join(format!("{id}.job")).exists());
    let mut el = JobSpec { id: "plandump-admin".into(), dest: paths::to_long(&w(r"C:\x")), elevated: true, ..Default::default() };
    el.sources.push(paths::to_long(&w(r"C:\y")));
    journal::save(&el);
    journal::save_soon(&mut el);
    let mut soon = JobSpec { id: "plandump-soon".into(), dest: paths::to_long(&w(r"C:\x")), ..Default::default() };
    soon.sources.push(paths::to_long(&w(r"C:\y")));
    journal::save_soon(&mut soon);
    journal::remove(&soon);
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let mut kept = JobSpec {
        id: "plandump-kept".into(),
        op: OpKind::Move,
        dest: paths::to_long(&w(r"C:\x")),
        policy: ConflictPolicy::Replace,
        ..Default::default()
    };
    kept.sources.push(paths::to_long(&w(r"C:\y")));
    journal::save(&kept);
    let _ = fs::write(app_paths::job_dir().join("plandump-empty.job"), "op=Copy\r\n");
    let _ = fs::write(app_paths::job_dir().join("plandump-x.jobx"), "op=Copy\r\ndest=C:\\x\r\nsrc=C:\\y\r\n");
    append_line(sb, &format!("records admin {} soon {} kept {}", has("plandump-admin"), has("plandump-soon"), has("plandump-kept")));
    for u in journal::load_unfinished() {
        let sources: Vec<String> = u.sources.iter().map(|s| s.to_string_lossy()).collect();
        append_line(
            sb,
            &format!(
                "unfinished {} | {} | {} | resume {} | {} | {}",
                u.id,
                u.op.name(),
                u.policy.name(),
                b(u.resume_mode),
                u.dest,
                sources.join(" ; ")
            ),
        );
    }
    append_line(sb, &format!("after load empty {}", has("plandump-empty")));
    if let Ok(list) = fs::read_dir(app_paths::job_dir()) {
        for e in list.flatten() {
            let _ = fs::remove_file(e.path());
        }
    }
    // local times of file times (the conflict window shows them)
    for ft in [0i64, 132000000000000000, 132700000000000000, 133500000000000000, 134400243957103472, -1, 2650467743999999999, 2650467744000000000] {
        let text = match crate::engine::model::file_times::to_local(ft) {
            None => "min".to_string(),
            Some(t) => format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
                t.year, t.month, t.day, t.hour, t.minute, t.second, t.millisecond
            ),
        };
        append_line(sb, &format!("time {ft} {text}"));
    }
    for sp in 0..=4 {
        append_line(
            sb,
            &format!(
                "usb {sp} {} | {} | {} | {}",
                usb_link::name(sp),
                usb_link::short(sp),
                netfmt::g(usb_link::ceiling_mbps(sp)),
                usb_link::aka(sp)
            ),
        );
    }
}

fn dump_scan(args: &[WString], sb: &mut String) {
    let op = if args.get(3).is_some_and(|a| a.eq_ignore_case_str("move")) { OpKind::Move } else { OpKind::Copy };
    let mut spec = JobSpec::new(op);
    spec.dest = paths::to_long(&args[4]);
    for a in &args[5..] {
        spec.sources.push(paths::to_long(a));
    }
    let src_vol = spec.sources.first().map(volume_probe::probe);
    let dst_vol = volume_probe::probe(&spec.dest);
    append_line(sb, &format!("scan {}", op.name().to_lowercase()));
    append_line(sb, &format!("src {}", src_vol.as_ref().map_or("(null)".into(), vol_text)));
    append_line(sb, &format!("dst {}", vol_text(&dst_vol)));
    // as CopyJob.RunCopy: 8 listings at once on a network, else 1 (source) and 2 (destination); then 4 and 4
    let src_threads = if src_vol.as_ref().is_some_and(|v| v.is_network()) { 8 } else { 1 };
    let dst_threads = if dst_vol.is_network() { 8 } else { 2 };
    for (st, dt) in [(src_threads, dst_threads), (4, 4)] {
        let scanner = Scanner::new(|| false);
        let res = scanner.run(&spec, st, dt);
        append_line(sb, &format!("threads {st} {dt}"));
        append_line(sb, &format!("problems {}", res.problems.len()));
        for p in &res.problems {
            append_line(sb, p);
        }
        append_line(sb, &format!("files {}", res.files.len()));
        for f in &res.files {
            append_line(
                sb,
                &format!(
                    "{} | {} | {} | {:X} | c {} a {} m {} | dest {} dir {} {} {} | temp {}",
                    f.src,
                    f.rel,
                    f.size(),
                    f.attr,
                    f.ctime,
                    f.atime,
                    f.mtime,
                    b(f.dest_exists),
                    b(f.dest_is_dir),
                    f.dest_size,
                    f.dest_mtime,
                    b(f.temp_exists)
                ),
            );
        }
        append_line(sb, &format!("dirs {}", res.dirs.len()));
        for d in &res.dirs {
            append_line(
                sb,
                &format!(
                    "{} | {} | {:X} | c {} a {} m {} | existed {} isfile {} | depth {}",
                    d.src,
                    d.rel,
                    d.attr,
                    d.ctime,
                    d.atime,
                    d.mtime,
                    b(d.dest_existed),
                    b(d.dest_is_file),
                    d.depth
                ),
            );
        }
        append_line(sb, &format!("total {} conflicts {} median {}", res.total_bytes, res.conflicts, res.median_size));
        let largest: Vec<String> = res.largest.iter().map(|(n, s)| format!("{n}={s}")).collect();
        append_line(sb, &format!("largest {}", largest.join(", ")));
        append_line(
            sb,
            &format!(
                "seen {} {} | {}",
                scanner.files_seen.load(std::sync::atomic::Ordering::SeqCst),
                scanner.bytes_seen.load(std::sync::atomic::Ordering::SeqCst),
                scanner.stage()
            ),
        );
        let p = Plan::choose(&JobSpec::new(OpKind::Copy), src_vol.as_ref(), Some(&dst_vol), Some(&res));
        append_line(sb, &format!("plan {}", plan_text(&p)));
    }
}
