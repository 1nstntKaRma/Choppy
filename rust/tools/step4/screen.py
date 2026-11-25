# screen.py --cs C#.exe --rs rust.exe --out dir [--dpi 120|96] [--only cs|rs] [--monitors 1,2] [--scenes a,b] [--work dir]
# Step 4's pictures: each window of the C# 1.48 (built from the rust branch: its test hook has CHOPPY_DPI96) and of the
# Rust port, shown on screen in test mode (CHOPPY_DATA, never the installed Choppy), moved onto monitor 1 or 2 with
# SetWindowPos (no activation) and captured with PrintWindow, which takes the window's own pixels (nothing that
# overlaps it). The real mouse and keyboard are never used: the windows are driven through the test hook.
# --dpi 96 starts both with CHOPPY_DPI96=1 (their UI thread DPI-unaware: the windows lay out and draw at 100 %) and
# captures from a DPI-unaware thread, so the pictures are the windows' 96-DPI pixels (the screen shows them stretched).
# Writes <out>/<cs|rs>-<scene>-<dpi>-m<n>.png. Test files go in --work (default C:\choppy-step4-screen), removed.
# The pop-ups (queue, the speed unit card "unithelp") are captured on their own.
import argparse, ctypes, ctypes.wintypes as wt, os, shutil, sys, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from hook import Choppy, INI
from PIL import Image

u = ctypes.windll.user32; g = ctypes.windll.gdi32
u.SetThreadDpiAwarenessContext.restype = ctypes.c_void_p
u.SetThreadDpiAwarenessContext.argtypes = [ctypes.c_void_p]
PMV2, UNAWARE = -4, -1
u.SetThreadDpiAwarenessContext(PMV2)

ap = argparse.ArgumentParser()
ap.add_argument("--cs", required=True); ap.add_argument("--rs", required=True); ap.add_argument("--out", required=True)
ap.add_argument("--dpi", type=int, default=120); ap.add_argument("--only"); ap.add_argument("--monitors", default="1,2")
ap.add_argument("--scenes", default="report,compact,queue,settings,unithelp,about,benchmark,conflict")
# a folder whose path says nothing about the PC's user (it shows in the copy window)
ap.add_argument("--work", default=r"C:\choppy-step4-screen")
a = ap.parse_args()
scenes = a.scenes.split(","); os.makedirs(a.out, exist_ok=True)


def monitors():
    out = []
    MONITORENUMPROC = ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HMONITOR, wt.HDC, ctypes.POINTER(wt.RECT), wt.LPARAM)
    class MI(ctypes.Structure): _fields_ = [("cb", wt.DWORD), ("m", wt.RECT), ("w", wt.RECT), ("f", wt.DWORD)]
    def cb(h, dc, r, l):
        mi = MI(); mi.cb = ctypes.sizeof(MI); u.GetMonitorInfoW(h, ctypes.byref(mi))
        out.append((mi.f & 1, (mi.w.left, mi.w.top, mi.w.right, mi.w.bottom))); return True
    u.EnumDisplayMonitors(0, 0, MONITORENUMPROC(cb), 0)
    out.sort(key=lambda m: -m[0])   # the primary first: monitor 1
    return [m[1] for m in out]


def windows(pid):
    out = []
    @ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)
    def cb(h, l):
        d = wt.DWORD(); u.GetWindowThreadProcessId(h, ctypes.byref(d))
        if d.value == pid and u.IsWindowVisible(h):
            b = ctypes.create_unicode_buffer(256); u.GetWindowTextW(h, b, 256); out.append((h, b.value))
        return True
    u.EnumWindows(cb, 0)
    return out


def find(c, test, timeout=15):
    t0 = time.time()
    while time.time() - t0 < timeout:
        for h, t in windows(c.p.pid):
            if test(t): return h
        time.sleep(0.1)
    return None


def popup(c, before, timeout=5):
    """The pop-up window that appeared since `before` (a set of window handles)."""
    t0 = time.time()
    while time.time() - t0 < timeout:
        new = [h for h, t in windows(c.p.pid) if h not in before]
        if new: time.sleep(0.5); return new[0]
        time.sleep(0.1)
    return None


def place(h, x, y):
    u.SetWindowPos(h, 0, x, y, 0, 0, 0x1 | 0x4 | 0x10)   # NOSIZE | NOZORDER | NOACTIVATE


def capture(h, path):
    old = u.SetThreadDpiAwarenessContext(UNAWARE if a.dpi == 96 else PMV2)
    try:
        r = wt.RECT(); u.GetWindowRect(h, ctypes.byref(r)); w, hh = r.right - r.left, r.bottom - r.top
        sdc = u.GetDC(0); mdc = g.CreateCompatibleDC(sdc); bmp = g.CreateCompatibleBitmap(sdc, w, hh); g.SelectObject(mdc, bmp)
        u.PrintWindow(h, mdc, 2)   # PW_RENDERFULLCONTENT
        class BIH(ctypes.Structure):
            _fields_ = [("a", wt.DWORD), ("w", ctypes.c_long), ("h", ctypes.c_long), ("p", wt.WORD), ("b", wt.WORD), ("c", wt.DWORD),
                        ("s", wt.DWORD), ("x", ctypes.c_long), ("y", ctypes.c_long), ("u", wt.DWORD), ("i", wt.DWORD)]
        buf = ctypes.create_string_buffer(w * hh * 4); bi = BIH(40, w, -hh, 1, 32, 0, 0, 0, 0, 0, 0)
        g.GetDIBits(mdc, bmp, 0, hh, buf, ctypes.byref(bi), 0)
        Image.frombuffer("RGB", (w, hh), buf, "raw", "BGRX", 0, 1).save(path)
        g.DeleteObject(bmp); g.DeleteDC(mdc); u.ReleaseDC(0, sdc)
        return (w, hh)
    finally:
        u.SetThreadDpiAwarenessContext(old)


def make_files(work):
    shutil.rmtree(work, ignore_errors=True)
    src, csrc, cdst = (os.path.join(work, n) for n in ("src", "csrc", "cdst"))
    for d in (src, csrc, cdst): os.makedirs(d)
    for n, size in (("Notes.txt", 12_345), ("Report.pdf", 2_400_000), ("Photo.jpg", 3_100_000)):
        open(os.path.join(src, n), "wb").write(bytes([65 + len(n)]) * size)
    t = lambda s: time.mktime(time.strptime(s, "%Y-%m-%d %H:%M:%S"))
    for d, n, b, size, when in ((csrc, "Budget.xlsx", b"y", 4096, "2025-11-14 08:00:00"), (csrc, "Holiday notes.txt", b"x", 2048, "2025-11-13 04:13:20"),
                                (cdst, "Budget.xlsx", b"w", 4096, "2025-11-14 08:00:00"), (cdst, "Holiday notes.txt", b"z", 1024, "2025-11-01 14:26:40")):
        p = os.path.join(d, n); open(p, "wb").write(b * size); os.utime(p, (t(when), t(when)))
    return src, csrc, cdst


mons = monitors()
print("monitors (work areas):", mons)
progs = [("cs", a.cs), ("rs", a.rs)]
for mi in [int(m) for m in a.monitors.split(",")]:
    wx, wy = mons[mi - 1][0] + 240, mons[mi - 1][1] + 40
    for name, exe in progs:
        if a.only and a.only != name: continue
        src, csrc, cdst = make_files(a.work)
        env = {"CHOPPY_DPI96": "1"} if a.dpi == 96 else {}
        ini = INI.replace("CloseWhenDone=1", "CloseWhenDone=0")
        args = ["copy"] + [os.path.join(src, f) for f in sorted(os.listdir(src))] + [os.path.join(a.work, "dst")]
        os.makedirs(os.path.join(a.work, "dst"))
        c = Choppy(exe, a.work, name, settings=ini, args=args, park=False, env_extra=env)
        tag = lambda s: os.path.join(a.out, f"{name}-{s}-{a.dpi}-m{mi}.png")
        def shot(scene, h):
            place(h, wx, wy); time.sleep(0.8); print(name, scene, "m%d" % mi, capture(h, tag(scene)))
        main = find(c, lambda t: t == "Choppy")   # the title has the percentage while it copies
        if "report" in scenes and main: shot("report", main)
        if "compact" in scenes and main:
            c.cmd("press view"); time.sleep(1.0); shot("compact", main); c.cmd("press view"); time.sleep(0.6)
        if "queue" in scenes and main:
            place(main, wx, wy); time.sleep(0.4)
            before = {h for h, t in windows(c.p.pid)}; c.cmd("press queue"); h = popup(c, before)
            if h: print(name, "queue", "m%d" % mi, capture(h, tag("queue")))
            c.cmd("press queue"); time.sleep(0.6)
        if "settings" in scenes:
            c.cmd("settings open"); h = find(c, lambda t: t == "Choppy settings")
            if h: shot("settings", h)
            if h and "unithelp" in scenes:
                before = {hh for hh, t in windows(c.p.pid)}; c.cmd("set unithelp 1"); p = popup(c, before)
                if p: print(name, "unithelp", "m%d" % mi, capture(p, tag("unithelp")))
                c.cmd("set unithelp 0"); time.sleep(0.4)
            c.cmd("settings close"); time.sleep(0.4)
        if "about" in scenes:
            c.cmd("about"); h = find(c, lambda t: t == "About Choppy")
            if h: shot("about", h)
            c.cmd("dialog 1"); time.sleep(0.4)
        if "benchmark" in scenes:
            c.cmd("benchmark open"); h = find(c, lambda t: t == "Choppy Benchmark"); time.sleep(4.0)
            if h: shot("benchmark", h)
            c.cmd("benchmark close"); time.sleep(0.6)
        if "conflict" in scenes:
            c.cmd("enqueue " + "|".join(["copy", os.path.join(csrc, "Holiday notes.txt"), os.path.join(csrc, "Budget.xlsx"), cdst]))
            h = find(c, lambda t: t == "File already exists - Choppy")
            if h: time.sleep(1.0); shot("conflict", h)
            c.cmd("conflict skip all"); time.sleep(0.8)
        c.quit()
shutil.rmtree(a.work, ignore_errors=True)
