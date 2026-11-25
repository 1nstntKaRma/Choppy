# states.py --cs C#.exe --rs rust.exe --out dir [--only cs|rs] [--work dir]
# The states that move, drawn by each program's test hook ("shot": off-screen at 144 DPI, laid out at the window's
# DPI, as WPF's RenderTargetBitmap does), in test mode with parked windows (nothing shows on screen):
#   run        a copy at 12 MB/s (the speed limit on), extended      run-compact  the same, compact
#   compared   the conflict window after Compare and picking "already there"
#   bench-done a tiny Benchmark run on C: (the test files are removed by the run)
#   ask        "Cancel copy?" (the Rust one only: C#'s hook waits while a modal dialog is up)
# With CHOPPY_DUMP=1 each picture gets <png>.layout.txt (element rectangles) for cmp.py. Test files go in --work.
import argparse, os, shutil, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hook import Choppy, INI

ap = argparse.ArgumentParser()
ap.add_argument("--cs", required=True); ap.add_argument("--rs", required=True); ap.add_argument("--out", required=True)
ap.add_argument("--only"); ap.add_argument("--work", default=r"C:\choppy-step4-states")
a = ap.parse_args(); os.makedirs(a.out, exist_ok=True)
env = {"CHOPPY_DUMP": "1"}
t = lambda s: time.mktime(time.strptime(s, "%Y-%m-%d %H:%M:%S"))

for name, exe in (("cs", a.cs), ("rs", a.rs)):
    if a.only and a.only != name: continue
    shot = lambda c, what, tag: print(name, tag, c.cmd(f"shot {what} " + os.path.join(a.out, f"{name}-{tag}.png")))
    # a running copy: three big files at 12 MB/s
    shutil.rmtree(a.work, ignore_errors=True)
    big, dst = os.path.join(a.work, "Videos"), os.path.join(a.work, "Backup")
    os.makedirs(big); os.makedirs(dst)
    for n, mb in (("Cover.jpg", 3), ("Drone - coastline 4K.mp4", 300), ("Wedding 2025 - full ceremony.mp4", 600)):
        with open(os.path.join(big, n), "wb") as f: f.truncate(mb << 20)
    ini = INI.replace("SpeedLimitMbps=0", "SpeedLimitMbps=100.663296")
    c = Choppy(exe, a.work, name, settings=ini, args=["copy"] + [os.path.join(big, f) for f in sorted(os.listdir(big))] + [dst], env_extra=env)
    time.sleep(9.0); shot(c, "main", "run")
    c.cmd("press view"); time.sleep(1.0); shot(c, "main", "run-compact"); c.cmd("press view"); time.sleep(0.5)
    if name == "rs":
        c.cmd("press cancel"); time.sleep(0.8); shot(c, "dialog", "ask"); c.cmd("dialog 2"); time.sleep(0.3)
    c.cmd("press cancel!"); time.sleep(1.5); c.quit()
    # the conflict window after Compare
    shutil.rmtree(a.work, ignore_errors=True)
    src, cdst = os.path.join(a.work, "Incoming"), os.path.join(a.work, "Documents")
    os.makedirs(src); os.makedirs(cdst)
    for d, n, b, size, when in ((src, "Budget.xlsx", b"y", 4096, "2025-11-14 08:00:00"), (src, "Holiday notes.txt", b"x", 2048, "2025-11-13 04:13:20"),
                                (cdst, "Budget.xlsx", b"w", 4096, "2025-11-14 08:00:00"), (cdst, "Holiday notes.txt", b"z", 1024, "2025-11-01 14:26:40")):
        p = os.path.join(d, n); open(p, "wb").write(b * size); os.utime(p, (t(when), t(when)))
    c = Choppy(exe, a.work, name, args=["copy", os.path.join(src, "Holiday notes.txt"), os.path.join(src, "Budget.xlsx"), cdst], env_extra=env)
    time.sleep(2.5); c.cmd("conflict compare"); time.sleep(1.0); c.cmd("conflict pickexisting"); time.sleep(0.3)
    shot(c, "conflict", "compared"); c.cmd("conflict skip all"); time.sleep(1.0); c.quit()
    # a tiny Benchmark on C:
    shutil.rmtree(a.work, ignore_errors=True)
    c = Choppy(exe, a.work, name, env_extra=env)
    c.cmd("benchmark open"); time.sleep(4); c.cmd("benchmark start tiny C")
    for i in range(90):
        time.sleep(1)
        if "done" in c.cmd("benchmark state"): break
    time.sleep(0.5); shot(c, "benchmark", "bench-done"); c.cmd("benchmark close"); time.sleep(0.5); c.quit()
shutil.rmtree(a.work, ignore_errors=True)
