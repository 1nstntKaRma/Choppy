# measure.py: open time (process start -> visible window, EnumWindows), memory after ~10 s of animation (working
# set, private bytes) and CPU while animating, for WPF (test-mode C# 1.48, a speed-limited copy in a scratch folder),
# A and B. Runs each 5 times for the open time.
import ctypes, ctypes.wintypes as wt, subprocess, sys, time, os, shutil, statistics, json
u = ctypes.windll.user32; k = ctypes.windll.kernel32; ps = ctypes.windll.psapi
u.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
S = sys.argv[1]; T = sys.argv[2]
k.OpenProcess.restype = wt.HANDLE
class PMC(ctypes.Structure):
    _fields_ = [("cb", wt.DWORD), ("PageFaultCount", wt.DWORD), ("PeakWorkingSetSize", ctypes.c_size_t), ("WorkingSetSize", ctypes.c_size_t),
                ("QuotaPeakPagedPoolUsage", ctypes.c_size_t), ("QuotaPagedPoolUsage", ctypes.c_size_t), ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
                ("QuotaNonPagedPoolUsage", ctypes.c_size_t), ("PagefileUsage", ctypes.c_size_t), ("PeakPagefileUsage", ctypes.c_size_t), ("PrivateUsage", ctypes.c_size_t)]
def mem(pid):
    h = k.OpenProcess(0x0410, False, pid); m = PMC(); m.cb = ctypes.sizeof(m)
    ps.GetProcessMemoryInfo(h, ctypes.byref(m), m.cb); k.CloseHandle(h)
    return m.WorkingSetSize / 2**20, m.PrivateUsage / 2**20
def cpu(pid):
    h = k.OpenProcess(0x0410, False, pid); a, b, c, d = wt.FILETIME(), wt.FILETIME(), wt.FILETIME(), wt.FILETIME()
    k.GetProcessTimes(h, ctypes.byref(a), ctypes.byref(b), ctypes.byref(c), ctypes.byref(d)); k.CloseHandle(h)
    f = lambda t: (t.dwHighDateTime << 32 | t.dwLowDateTime) / 1e7
    return f(c) + f(d)
def visible(pid):
    found = []
    @ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)
    def cb(h, l):
        d = wt.DWORD(); u.GetWindowThreadProcessId(h, ctypes.byref(d)); r = wt.RECT(); u.GetWindowRect(h, ctypes.byref(r))
        if d.value == pid and u.IsWindowVisible(h) and r.right - r.left > 300: found.append(h)
        return True
    u.EnumWindows(cb, 0); return bool(found)
def run(cmd, env=None, sample=False):
    t0 = time.perf_counter(); p = subprocess.Popen(cmd, env=env)
    while not visible(p.pid):
        if time.perf_counter() - t0 > 30: break
        time.sleep(0.002)
    t_open = time.perf_counter() - t0
    res = {"open": t_open}
    if sample:
        time.sleep(5.0 - (time.perf_counter() - t0) if time.perf_counter() - t0 < 5 else 0)
        c0 = cpu(p.pid); w0 = time.perf_counter(); time.sleep(5.0); c1 = cpu(p.pid); w1 = time.perf_counter()
        res["cpu"] = 100 * (c1 - c0) / (w1 - w0)
        res["ws"], res["priv"] = mem(p.pid)
    p.kill(); p.wait(); time.sleep(0.8)
    return res
# WPF: test mode, a 1 GB file copied inside the scratch folder at 57 MB/s (the window animates like the demo)
data = os.path.join(S, "mdata"); src = os.path.join(S, "msrc"); dst = os.path.join(S, "mdst")
def wpf_cmd():
    for d in (data, dst): shutil.rmtree(d, ignore_errors=True); os.makedirs(d)
    open(os.path.join(data, "settings.ini"), "w").write("SettingsVersion=2\nView=extended\nSpeedLimitMbps=478.150656\nLimitValueMbps=478.150656\nCloseWhenDone=1\nCloseAfterSeconds=0\nVerifyCopies=0\nExplorerTakeover=0\n")
    return [os.path.join(S, "cs148", "Choppy.exe"), "copy", os.path.join(src, "Wedding 2025 - full ceremony.mp4"), dst]
os.makedirs(src, exist_ok=True)
with open(os.path.join(src, "Wedding 2025 - full ceremony.mp4"), "wb") as f: f.truncate(1 << 30)
wenv = dict(os.environ, CHOPPY_DATA=data)
out = {}
for name, cmd, env in [("WPF", None, wenv), ("A", [os.path.join(T, "choppy-look-a.exe"), "--x", "200", "--y", "40"], None),
                       ("B", [os.path.join(T, "choppy-look-b.exe"), "--x", "200", "--y", "40"], None)]:
    opens = []
    for i in range(5):
        r = run(cmd or wpf_cmd(), env, sample=(i == 4))
        opens.append(r["open"])
    r["opens"] = [round(x, 3) for x in opens]; r["open_median"] = round(statistics.median(opens), 3)
    out[name] = r; print(name, json.dumps(r))
shutil.rmtree(src, ignore_errors=True); shutil.rmtree(dst, ignore_errors=True); shutil.rmtree(data, ignore_errors=True)
json.dump(out, open(os.path.join(S, "measure.json"), "w"), indent=1)
