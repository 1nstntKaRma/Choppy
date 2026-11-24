# drive.py outdir name exe [args...]; env ACTIONS="move x y; click x y; dbl x y; key vk; char text; wait s; shot tag [extra_right]"
# Starts a window app, drives it with posted window messages (the real mouse and keyboard are never touched),
# and saves screen captures of the window (plus `extra_right` pixels beside it, for a pop-up that reaches past it).
import ctypes, ctypes.wintypes as wt, subprocess, sys, time, os
from PIL import ImageGrab
u = ctypes.windll.user32
u.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
u.PostMessageW.argtypes = [wt.HWND, wt.UINT, wt.WPARAM, wt.LPARAM]
outdir, name, cmd = sys.argv[1], sys.argv[2], sys.argv[3:]
p = subprocess.Popen(cmd)
def find(pid):
    found = []
    @ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)
    def cb(h, l):
        d = wt.DWORD(); u.GetWindowThreadProcessId(h, ctypes.byref(d))
        r = wt.RECT(); u.GetWindowRect(h, ctypes.byref(r))
        if d.value == pid and u.IsWindowVisible(h) and r.right - r.left > 300: found.append(h)
        return True
    u.EnumWindows(cb, 0)
    return found[0] if found else None
h = None
t0 = time.time()
while h is None and time.time() - t0 < 20:
    h = find(p.pid); time.sleep(0.02)
time.sleep(1.2)
def lp(x, y): return (int(y) << 16) | (int(x) & 0xFFFF)
WM_MOUSEMOVE, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_LBUTTONDBLCLK, WM_KEYDOWN, WM_KEYUP, WM_CHAR = 0x200, 0x201, 0x202, 0x203, 0x100, 0x101, 0x102
def rects(pid):
    out = []
    @ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)
    def cb(hh, l):
        d = wt.DWORD(); u.GetWindowThreadProcessId(hh, ctypes.byref(d))
        if d.value == pid and u.IsWindowVisible(hh):
            r = wt.RECT(); ctypes.windll.dwmapi.DwmGetWindowAttribute(hh, 9, ctypes.byref(r), ctypes.sizeof(r)); out.append((r.left, r.top, r.right, r.bottom))
        return True
    u.EnumWindows(cb, 0)
    return out
def shot(tag, extra=0):
    from PIL import Image
    r = wt.RECT(); ctypes.windll.dwmapi.DwmGetWindowAttribute(h, 9, ctypes.byref(r), ctypes.sizeof(r))
    box = (r.left, r.top, r.right + extra, r.bottom)
    img = ImageGrab.grab(bbox=box, all_screens=True)
    # only this app's own windows (main + pop-up); anything else on the screen is blanked out
    mask = Image.new("L", img.size, 0)
    for (a, b, c, d) in rects(p.pid):
        mask.paste(255, (a - box[0], b - box[1], c - box[0], d - box[1]))
    out = Image.new("RGB", img.size, (255, 255, 255)); out.paste(img, (0, 0), mask)
    f = os.path.join(outdir, f"{name}-{tag}.png"); out.save(f); print(f, out.size)
for a in [a.strip() for a in os.environ.get("ACTIONS", "shot main").split(";") if a.strip()]:
    w = a.split()
    if w[0] == "move": u.PostMessageW(h, WM_MOUSEMOVE, 0, lp(w[1], w[2]))
    elif w[0] == "click":
        u.PostMessageW(h, WM_MOUSEMOVE, 0, lp(w[1], w[2])); time.sleep(0.05)
        u.PostMessageW(h, WM_LBUTTONDOWN, 1, lp(w[1], w[2])); time.sleep(0.05)
        u.PostMessageW(h, WM_LBUTTONUP, 0, lp(w[1], w[2]))
    elif w[0] == "dbl":
        u.PostMessageW(h, WM_MOUSEMOVE, 0, lp(w[1], w[2])); time.sleep(0.05)
        u.PostMessageW(h, WM_LBUTTONDOWN, 1, lp(w[1], w[2])); u.PostMessageW(h, WM_LBUTTONUP, 0, lp(w[1], w[2]))
        u.PostMessageW(h, WM_LBUTTONDBLCLK, 1, lp(w[1], w[2])); u.PostMessageW(h, WM_LBUTTONUP, 0, lp(w[1], w[2]))
    elif w[0] == "key":
        vk = int(w[1], 0); u.PostMessageW(h, WM_KEYDOWN, vk, 1); time.sleep(0.03); u.PostMessageW(h, WM_KEYUP, vk, 0xC0000001)
    elif w[0] == "char":
        # key down / up only: the app's TranslateMessage makes the WM_CHAR
        for ch in w[1]:
            vk = ord(ch.upper()); sc = u.MapVirtualKeyW(vk, 0)
            u.PostMessageW(h, WM_KEYDOWN, vk, 1 | (sc << 16)); time.sleep(0.03); u.PostMessageW(h, WM_KEYUP, vk, 0xC0000001 | (sc << 16)); time.sleep(0.05)
    elif w[0] == "focus":
        u.PostMessageW(h, 0x0006, 1, 0); u.PostMessageW(h, 0x0007, 0, 0)   # WM_ACTIVATE active, WM_SETFOCUS
    elif w[0] == "wait": time.sleep(float(w[1]))
    elif w[0] == "shot": shot(w[1], int(w[2]) if len(w) > 2 else 0)
    time.sleep(0.25)
p.kill()
