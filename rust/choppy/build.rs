//! Embeds what the C# build puts in Choppy.exe: res/app.manifest (PerMonitorV2, longPathAware, supportedOS),
//! res/Choppy.ico (byte for byte the C# project's src/app.manifest and src/Choppy.ico, so the build doesn't need
//! src/ once the C# is gone) and the version info. FileVersion = VERSION ("1.48" -> 1.48.0.0): release.yml checks it.
//! The resources are compiled with the Windows SDK's rc.exe (installed with the VS Build Tools) - no crate needed.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let here = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo = here.join("..").join("..");
    let version_file = repo.join("VERSION");
    let manifest = here.join("res").join("app.manifest");
    let icon = here.join("res").join("Choppy.ico");
    for p in [&version_file, &manifest, &icon] {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CHOPPY_RC");

    // build.ps1's Get-Stamp: Major.Minor.Max(0, Build); the file version adds .0
    let raw = fs::read_to_string(&version_file).expect("VERSION");
    let mut parts: Vec<u32> = raw.trim().split('.').map(|s| s.trim().parse().expect("VERSION is a number")).collect();
    parts.resize(3, 0);
    let (major, minor, build) = (parts[0], parts[1], parts[2]);
    let file_version = format!("{major}.{minor}.{build}.0");
    let product_version = format!("{major}.{minor}.{build}");
    println!("cargo:rustc-env=CHOPPY_VERSION={product_version}");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let rc = out.join("choppy.rc");
    let res = out.join("choppy.res");
    let q = |p: &Path| p.display().to_string().replace('\\', "\\\\");
    // the same strings as the C# exe's version info (Choppy.csproj: Product, Company, Copyright, Description)
    let text = format!(
        r#"1 24 "{manifest}"
32512 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {major},{minor},{build},0
PRODUCTVERSION {major},{minor},{build},0
FILEFLAGSMASK 0x3F
FILEFLAGS 0x0
FILEOS 0x4
FILETYPE 0x1
FILESUBTYPE 0x0
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "000004b0"
    BEGIN
      VALUE "Comments", "Choppy - fast, clear file copy"
      VALUE "CompanyName", "1nstntKaRma"
      VALUE "FileDescription", "Choppy"
      VALUE "FileVersion", "{file_version}"
      VALUE "InternalName", "Choppy.exe"
      VALUE "LegalCopyright", "Copyright (c) 2026 1nstntKaRma"
      VALUE "OriginalFilename", "Choppy.exe"
      VALUE "ProductName", "Choppy"
      VALUE "ProductVersion", "{product_version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0, 1200
  END
END
"#,
        manifest = q(&manifest),
        icon = q(&icon),
    );
    fs::write(&rc, text).unwrap();
    let rc_exe = find_rc().expect("rc.exe not found: install the Windows SDK (VS Build Tools) or set CHOPPY_RC");
    let status = Command::new(&rc_exe).arg("/nologo").arg("/fo").arg(&res).arg(&rc).status().expect("rc.exe");
    assert!(status.success(), "rc.exe failed");
    // link.exe takes a compiled .res file as an input
    println!("cargo:rustc-link-arg-bins={}", res.display());
}

/// rc.exe from CHOPPY_RC, or the newest Windows 10/11 SDK in its default place.
fn find_rc() -> Option<PathBuf> {
    if let Ok(p) = env::var("CHOPPY_RC") {
        return Some(PathBuf::from(p));
    }
    let arch = if env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("x86") { "x86" } else { "x64" };
    let mut roots = Vec::new();
    for var in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Ok(pf) = env::var(var) {
            roots.push(PathBuf::from(pf).join("Windows Kits").join("10").join("bin"));
        }
    }
    let mut best: Option<(Vec<u32>, PathBuf)> = None;
    for root in roots {
        let Ok(dirs) = fs::read_dir(&root) else { continue };
        for d in dirs.flatten() {
            let name = d.file_name().to_string_lossy().to_string();
            let ver: Vec<u32> = name.split('.').filter_map(|s| s.parse().ok()).collect();
            let exe = d.path().join(arch).join("rc.exe");
            if ver.len() == 4 && exe.exists() && best.as_ref().is_none_or(|(b, _)| ver > *b) {
                best = Some((ver, exe));
            }
        }
    }
    best.map(|(_, p)| p)
}
