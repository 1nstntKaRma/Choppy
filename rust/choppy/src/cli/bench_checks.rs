//! Cli.cs BenchChecks: the Benchmark's parts that need no drives - verdicts, the plan, sizes, the report, clean-up,
//! Windows' copy on a small file. The same names and order as the C# (step 4's self-test checks).

use std::path::{Path, PathBuf};

use crate::bench::benchmark::{Benchmark, MB, judge, mean, test_drive};
use crate::bench::files;
use crate::bench::report::{self, BenchCase, BenchDrive, BenchReport, BenchResult, BenchRun, BenchTotals, Stamp, text};
use crate::engine::volume_probe::DeviceKind;

/// A self-test line: Some(ok) = a check ("PASS name" / "FAIL name"), None = a line as it is (a SKIP).
pub type Line = (Option<bool>, String);

pub fn bench_checks(desktop: bool) -> Vec<Line> {
    let mut out: Vec<Line> = Vec::new();
    let mut check = |name: &str, ok: bool| out.push((Some(ok), name.to_string()));

    // the time decides: the share of Windows' time Choppy saved; less than 4% apart is a Marginal Tie, nothing else is
    let (v, saved) = judge(&[2.0, 2.0], &[2.0, 2.0]);
    check("bench: same times = Marginal Tie", v == 0 && saved.abs() < 1e-9);
    let (v, saved) = judge(&[1.0, 1.02], &[1.2, 1.22]);
    check("bench: less time = Faster (+17%)", v == 1 && (saved - 0.2 / 1.21).abs() < 1e-9 && text::saved(saved) == "+17%");
    let (v, saved) = judge(&[1.2, 1.22], &[1.0, 1.02]);
    check("bench: more time = Slower (-20%)", v == -1 && text::saved(saved) == "−20%");
    let (v, saved) = judge(&[1.0, 1.0], &[1.03, 1.03]);
    check("bench: 3% apart = Marginal Tie", v == 0 && saved > 0.0);
    let (v, _) = judge(&[1.0, 1.4], &[1.1, 1.1]);
    check("bench: 9% behind with scattered runs is still Slower", v == -1);
    check("bench: averages", mean(&[3.0, 1.0, 2.0]) == 2.0 && mean(&[1.0, 2.0]) == 1.5);
    check("bench: seconds", text::secs(0.4) == "0.40 s" && text::secs(12.34) == "12.3 s" && text::saved(0.001) == "0%");

    // the plan: system drive to each drive and back, and within each
    let mut drives = vec![
        test_drive("D:", false, DeviceKind::Hdd, 1, 180.0, 180.0),
        test_drive("C:", true, DeviceKind::Ssd, 0, 2000.0, 1500.0),
        test_drive("E:", false, DeviceKind::Ssd, 2, 400.0, 350.0),
    ];
    let (d, c, e) = (0usize, 1usize, 2usize);
    let mut routes = Benchmark::plan_routes(&[d, c, e], &drives);
    let has = |rs: &[crate::bench::benchmark::Route], a: usize, b: usize| rs.iter().any(|r| r.from == a && r.to == b);
    check(
        "bench: routes (7 for 3 drives, the system drive first, no route between two others)",
        routes.len() == 7
            && routes[0].from == c
            && routes[0].to == c
            && has(&routes, c, d)
            && has(&routes, d, c)
            && has(&routes, d, d)
            && has(&routes, c, e)
            && has(&routes, e, c)
            && has(&routes, e, e)
            && !has(&routes, d, e),
    );
    let bq = Benchmark::new();
    bq.size_routes(&mut routes, &drives);
    let rt = |rs: &[crate::bench::benchmark::Route], a: usize, b: usize| rs.iter().find(|r| r.from == a && r.to == b).cloned().unwrap_or_default();
    check(
        "bench: sizes by speed (2 GB + 1,000 small files at 350 MB/s and more, 1 GB + 500 below)",
        rt(&routes, c, c).big == 2048 * MB
            && rt(&routes, c, c).small == 1000
            && rt(&routes, c, d).big == 1024 * MB
            && rt(&routes, c, d).small == 500
            && rt(&routes, d, d).big == 1024 * MB
            && rt(&routes, c, e).big == 2048 * MB
            && rt(&routes, e, c).big == 2048 * MB
            && rt(&routes, e, e).big == 1024 * MB,
    );
    check(
        "bench: two cases per route",
        routes.iter().all(|r| r.cases.len() == 2 && r.cases[0].files() == 1 && r.cases[1].files() == r.small && r.est > 0.0),
    );
    // many hard disks: one step smaller everywhere, to keep to the time budget
    let mut many: Vec<BenchDrive> = vec![drives[c].clone()];
    for i in 0..9 {
        many.push(test_drive(&format!("{}:", (b'F' + i as u8) as char), false, DeviceKind::Hdd, 3 + i, 150.0, 150.0));
    }
    let idx: Vec<usize> = (0..many.len()).collect();
    let mut mr = Benchmark::plan_routes(&idx, &many);
    bq.size_routes(&mut mr, &many);
    check(
        "bench: a time budget (many hard disks: a step smaller)",
        mr.len() == 28 && mr[0].big == 1024 * MB && mr.iter().all(|r| r.big <= 1024 * MB) && mr.iter().any(|r| r.big == 512 * MB && r.small == 250),
    );
    let mut thorough = Benchmark::new();
    thorough.thorough = true;
    let mut tr = Benchmark::plan_routes(&[c, d], &drives);
    thorough.size_routes(&mut tr, &drives);
    check(
        "bench: thorough = 4 GB / 2 GB, 2,000 / 1,000 small files, a mixed folder",
        tr.iter().all(|r| r.cases.len() == 3) && tr[0].big == 4096 * MB && tr[0].small == 2000 && tr[1].big == 2048 * MB && tr[1].small == 1000,
    );
    let s1 = files::small_sizes(1000);
    let sum: i64 = s1.iter().sum();
    check(
        "bench: small files 4 KB - 1 MB, the same set every time",
        s1.iter().all(|&x| (4096..=1 << 20).contains(&x)) && sum == files::small_sizes(1000).iter().sum::<i64>() && sum > 150 * MB && sum < 250 * MB,
    );

    // the report: summary, JSON, HTML (escaped), text
    let mut rep = BenchReport::new();
    rep.started = Stamp { year: 2026, month: 11, day: 23, hour: 14, minute: 5, second: 0 };
    rep.mode = "quick".into();
    rep.rounds = 2;
    rep.cpu = Some("Test CPU".into());
    rep.windows = Some("Windows test".into());
    rep.threads = 4;
    rep.ram_gb = 8.0;
    drives[c].label = Some("<b>&amp;".into());
    drives[c].model = Some("Test \"NVMe\"".into());
    let mut g = BenchDrive::new("G:", "");
    g.skip = Some("not a disk of this PC (a virtual or cloud drive)".into());
    rep.drives = Some(vec![drives[c].clone(), drives[d].clone(), g]);
    let big = BenchCase { key: "big".into(), big: vec![1024 * MB], smalls: vec![] };
    let mut r1 = BenchResult { from: drives[c].clone(), to: drives[d].clone(), from_i: c, to_i: d, case: big, ..Default::default() };
    r1.runs.push(BenchRun { choppy: true, done: 5.0, on_disk: 5.1 });
    r1.runs.push(BenchRun { choppy: false, done: 4.0, on_disk: 6.0 });
    r1.runs.push(BenchRun { choppy: false, done: 4.1, on_disk: 6.1 });
    r1.runs.push(BenchRun { choppy: true, done: 5.0, on_disk: 5.2 });
    r1.score();
    let mut r2 = BenchResult {
        from: drives[d].clone(),
        to: drives[c].clone(),
        from_i: d,
        to_i: c,
        case: BenchCase { key: "small".into(), big: vec![], smalls: s1.clone() },
        error: Some("test error".into()),
        ..Default::default()
    };
    r2.score();
    rep.results.push(r1.clone());
    rep.results.push(r2.clone());
    check(
        "bench: scored (on the disk faster, done slower; averages)",
        r1.scored
            && r1.verdict == 1
            && r1.done_verdict == -1
            && !r2.scored
            && rep.summary() == "Choppy is Faster 1 of 1"
            && (r1.choppy_secs - 5.15).abs() < 1e-9
            && (r1.windows_secs - 6.05).abs() < 1e-9,
    );
    check("bench: summary line", report::summary_of(9, 2, 1, true) == "Choppy is Faster 9 of 12 · Marginal Tie 2 of 12 · Slower 1 of 12");
    check("bench: each case says Tie, the summary Marginal Tie", report::word(0) == "Tie" && report::word(1) == "Faster" && report::word(-1) == "Slower");
    check(
        "bench: Never Slower only when a whole run ended without one",
        report::summary_of(10, 9, 0, true) == "Choppy is Faster 10 of 19 · Marginal Tie 9 of 19 · Never Slower"
            && report::summary_of(10, 9, 0, false) == "Choppy is Faster 10 of 19 · Marginal Tie 9 of 19"
            && report::summary_of(5, 0, 0, true) == "Choppy is Faster 5 of 5",
    );
    let tot = rep.totals();
    check(
        "bench: totals (the measured cases only; each one's speed over the times added up)",
        tot.as_ref().is_some_and(|t| {
            t.cases == 1
                && (t.avg_saved - r1.saved).abs() < 1e-9
                && (t.choppy_secs - 5.15).abs() < 1e-9
                && (t.secs_saved() - 0.9).abs() < 1e-9
                && (t.choppy_mbps - 1024.0 / 5.15).abs() < 1e-6
                && t.line() == "+15% faster on average · Choppy 199 MB/s · Windows 169 MB/s · Time saved 0.90 s"
        }) && BenchTotals::of([&r2]).is_none(),
    );
    check(
        "bench: time saved as a span",
        text::signed_span(-12.34) == "−12.3 s" && text::signed_span(0.001) == "0 s" && text::signed_span(75.0) == "1 min 15 s" && text::span(0.41) == "0.41 s",
    );
    let (json, html) = (rep.to_json(), rep.to_html("x.json"));
    let (mut depth, mut in_str, mut esc, mut balanced) = (0i32, false, false, true);
    for ch in json.chars() {
        if in_str {
            if esc {
                esc = false;
            } else if ch == '\\' {
                esc = true;
            } else if ch == '"' {
                in_str = false;
            }
            continue;
        }
        if ch == '"' {
            in_str = true;
        } else if ch == '{' || ch == '[' {
            depth += 1;
        } else if ch == '}' || ch == ']' {
            depth -= 1;
            if depth < 0 {
                balanced = false;
            }
        }
    }
    check(
        "bench: JSON well formed",
        balanced
            && depth == 0
            && !in_str
            && json.contains("\"verdict\": \"faster\"")
            && json.contains("\\\"NVMe\\\"")
            && json.contains("\"error\": \"test error\"")
            && json.contains("\"totals\": { \"cases\": 1,"),
    );
    check(
        "bench: HTML escaped",
        html.contains("&lt;b&gt;&amp;amp;")
            && !html.contains("<b>&amp;</b>")
            && html.contains("Faster 1 of 1")
            && html.contains("Choppy VS Windows")
            && html.contains("not a disk of this PC")
            && !html.contains(">done ")
            && html.contains("<section class=\"totals\">"),
    );

    // test folders: removed with what's in them; a link inside is removed, never followed; a folder that isn't ours stays
    let root = std::env::temp_dir().join(format!("choppy-selftest-bench-{}", &crate::engine::model::new_id()[..8]));
    let r = (|| -> Result<(), String> {
        std::fs::create_dir_all(&root).map_err(|e| files::io_message(&e))?;
        let outside = root.join("outside");
        std::fs::create_dir_all(&outside).map_err(|e| files::io_message(&e))?;
        std::fs::write(outside.join("keep.txt"), "keep").map_err(|e| files::io_message(&e))?;
        let t1 = files::make_folder(&root, false).ok();
        let t1p = t1.clone().unwrap_or_default();
        std::fs::create_dir_all(t1p.join("a").join("b")).map_err(|e| files::io_message(&e))?;
        let x = t1p.join("a").join("b").join("x.bin");
        std::fs::write(&x, "x").map_err(|e| files::io_message(&e))?;
        let mut perm = std::fs::metadata(&x).map_err(|e| files::io_message(&e))?.permissions();
        perm.set_readonly(true);
        let _ = std::fs::set_permissions(&x, perm);
        let link = make_junction(&t1p.join("link"), &outside);
        // a folder of that name that isn't the benchmark's: never used, a numbered one instead
        let foreign = root.join("x");
        std::fs::create_dir_all(&foreign).map_err(|e| files::io_message(&e))?;
        let t2 = files::make_folder(&foreign, false).map_err(|e| e.to_string())?;
        std::fs::remove_dir_all(&t2).map_err(|e| files::io_message(&e))?;
        std::fs::create_dir_all(&t2).map_err(|e| files::io_message(&e))?;
        std::fs::write(t2.join("theirs.txt"), "theirs").map_err(|e| files::io_message(&e))?;
        let t3 = files::make_folder(&foreign, false).ok();
        check(
            "bench: a folder that isn't ours gets a numbered name",
            t1.is_some() && t3.as_ref().is_some_and(|t| t.to_string_lossy().ends_with(&format!("{} 2", files::FOLDER_NAME))),
        );
        files::clean_leftovers();
        check(
            &format!("bench: leftovers removed, links not followed{}", if link { "" } else { " (no link made)" }),
            !t1p.exists() && !t3.as_ref().is_some_and(|t| t.exists()) && outside.join("keep.txt").exists(),
        );
        check("bench: a folder without the marker untouched", t2.join("theirs.txt").exists());
        Ok(())
    })();
    if let Err(e) = r {
        check(&format!("bench: test folders: {e}"), false);
    }
    remove_all(&root);

    // Windows' own copy (the shell's engine, in a child process) of a small file and a folder
    let sdir = std::env::temp_dir().join(format!("choppy-selftest-shell-{}", &crate::engine::model::new_id()[..8]));
    let r = (|| -> Result<(), String> {
        std::fs::create_dir_all(sdir.join("src").join("sub")).map_err(|e| files::io_message(&e))?;
        std::fs::create_dir_all(sdir.join("dst")).map_err(|e| files::io_message(&e))?;
        let mut data = vec![0u8; 300000];
        super::selftest::NetRandom::new(3).next_bytes(&mut data);
        std::fs::write(sdir.join("src").join("sub").join("f.bin"), &data).map_err(|e| files::io_message(&e))?;
        std::fs::write(sdir.join("w.bin"), vec![0u8; 100]).map_err(|e| files::io_message(&e))?;
        std::fs::create_dir_all(sdir.join("warm")).map_err(|e| files::io_message(&e))?;
        let secs = crate::bench::shell_copy::run(Some(&sdir.join("w.bin")), Some(&sdir.join("warm")), &sdir.join("src"), &sdir.join("dst"), &|| false)
            .map_err(|e| match e {
                crate::bench::BenchErr::Error(m) => m,
                crate::bench::BenchErr::Canceled => "The operation was canceled.".into(),
            })?;
        let got = sdir.join("dst").join("src").join("sub").join("f.bin");
        check(
            "bench: Windows' copy (IFileOperation, child process)",
            secs >= 0.0 && std::fs::read(&got).is_ok_and(|b| b == data) && sdir.join("warm").join("w.bin").exists(),
        );
        Ok(())
    })();
    if let Err(msg) = r {
        // a build server without a desktop may have no shell to copy with (like the Ctrl+V hook above)
        if !desktop || msg.contains("80040154") || msg.to_lowercase().contains("not registered") {
            out.push((None, format!("SKIP Windows' copy (no shell here: {msg})")));
        } else {
            out.push((Some(false), format!("bench: Windows' copy: {msg}")));
        }
    }
    remove_all(&sdir);
    out
}

/// Directory.Delete(dir, true): read-only files too.
fn remove_all(dir: &Path) {
    let long = crate::engine::paths::to_long(&crate::wstr::WString::from(dir.display().to_string()));
    files::delete_tree(&long);
}

/// A junction (no administrator rights needed), for the clean-up check.
fn make_junction(link: &Path, target: &Path) -> bool {
    let ok = std::process::Command::new("cmd.exe")
        .args(["/c", "mklink", "/J", &link.display().to_string(), &target.display().to_string()])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .is_ok();
    ok && link.is_dir() && crate::sys::get_file_attributes(&crate::wstr::WString::from(link.display().to_string())) & crate::sys::FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[allow(dead_code)]
fn _unused(_: PathBuf) {}
