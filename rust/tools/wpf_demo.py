# Today's WPF window with the README demo (test mode, never the installed Choppy): D:\Home Videos -> X:\Backup\Home
# Videos at 57 MB/s. Captures compact at ~17 s and extended at ~29 s from the screen, then cancels and cleans up.
import ctypes, ctypes.wintypes as wt, subprocess, sys, time, os, json, shutil
from PIL import ImageGrab
u = ctypes.windll.user32; u.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
S = sys.argv[1]; exe = os.path.join(S, "cs148", "Choppy.exe"); out = os.path.join(S, "shots")
src = r"D:\Home Videos"; dst_root = r"X:\Backup"; dst = dst_root + r"\Home Videos"
MB = 1024 * 1024
files = [("Birthday party.mp4", 600*MB), ("Cover.jpg", 6*MB), ("Drone - coastline 4K.mp4", 1126*MB),
         ("Road trip north - day 2.mov", 250*MB), ("Thumbnails.zip", 12*MB), ("Wedding 2025 - full ceremony.mp4", 1536*MB)]
assert not os.path.exists(src) and not os.path.exists(dst_root)
os.makedirs(src); os.makedirs(dst)
for n, s in files:
    with open(os.path.join(src, n), "wb") as f: f.truncate(s)
data = os.path.join(S, "wpfdata"); hook = os.path.join(S, "wpfhook")
for d in (data, hook): shutil.rmtree(d, ignore_errors=True); os.makedirs(d)
open(os.path.join(data, "settings.ini"), "w").write("SettingsVersion=2\nView=compact\nSpeedLimitMbps=478.150656\nLimitValueMbps=478.150656\nCloseWhenDone=1\nCloseAfterSeconds=0\nVerifyCopies=0\nExplorerTakeover=0\nSpeedUnit=auto\n")
env = dict(os.environ, CHOPPY_DATA=data, CHOPPY_TESTHOOK=hook)
def cmd(line):
    open(os.path.join(hook, "cmd.txt"), "w", encoding="utf-8").write(line + "\n")
    for _ in range(100):
        time.sleep(0.05)
        if not os.path.exists(os.path.join(hook, "cmd.txt")): break
    time.sleep(0.1)
def state():
    f = os.path.join(hook, "st.json"); cmd("state " + f)
    try: return json.load(open(f, encoding="utf-8-sig"))
    except Exception: return {}
def window(pid):
    found = []
    @ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)
    def cb(h, l):
        d = wt.DWORD(); u.GetWindowThreadProcessId(h, ctypes.byref(d)); r = wt.RECT(); u.GetWindowRect(h, ctypes.byref(r))
        if d.value == pid and u.IsWindowVisible(h) and r.right - r.left > 300: found.append(h)
        return True
    u.EnumWindows(cb, 0); return found[0] if found else None
def grab(h, name):
    r = wt.RECT(); ctypes.windll.dwmapi.DwmGetWindowAttribute(h, 9, ctypes.byref(r), ctypes.sizeof(r))
    img = ImageGrab.grab(bbox=(r.left, r.top, r.right, r.bottom), all_screens=True); img.save(os.path.join(out, name)); print(name, img.size)
p = subprocess.Popen([exe, "copy"] + [os.path.join(src, n) for n, _ in files] + [dst], env=env)
try:
    h = None
    for _ in range(400):
        h = window(p.pid)
        if h: break
        time.sleep(0.05)
    def secs(st):
        e = st.get("elapsed", "")
        return int(e.split()[0]) if e.endswith(" s") else -1
    for target, name in ((17, "wpf-compact.png"), (29, "wpf-ext.png")):
        while True:
            st = state(); s = secs(st)
            if s >= target: break
            time.sleep(0.2)
        grab(h, name); print(json.dumps(st))
        if target == 17:
            cmd("press view"); time.sleep(0.5)
    cmd("press cancel!"); time.sleep(1.5); cmd("quit"); time.sleep(1.5)
finally:
    try: p.kill()
    except Exception: pass
    time.sleep(1)
    for d in (dst, src):
        for n in os.listdir(d): os.remove(os.path.join(d, n))
        os.rmdir(d)
    os.rmdir(dst_root)
    print("cleaned", os.path.exists(src), os.path.exists(dst_root))
