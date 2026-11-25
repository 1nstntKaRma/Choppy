"""Step 5 gate: Choppy opens faster than 1.48 - the window-latency method of the testing notes. Test mode only
(CHOPPY_DATA, windows parked off-screen, the copy window closing itself when done).
  warm: `Choppy.exe --tray` (settings.ini ExplorerTakeover=1) started and left 8 s to settle (its quick first window
        built and gone), then `Choppy.exe copy <file> <new folder>` timed until a visible window titled "Choppy"
        ("Choppy", or "12% ... - Choppy" while copying) of the tray process exists (the second start hands the job over through the pipe);
  cold: nothing running, the same command timed until the started process shows its window.

    python latency.py <exe> [<exe>...] [--runs 7]

Prints each run and the median per exe and mode. Test files in C:\\choppy-step5-latency, removed afterwards.
"""
import ctypes
import ctypes.wintypes as wt
import os
import shutil
import statistics
import subprocess
import sys
import time

ROOT = r"C:\choppy-step5-latency"
user32 = ctypes.windll.user32
EnumProc = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)


def visible_choppy_window(pid):
    found = []

    def cb(h, _):
        if user32.IsWindowVisible(h):
            p = wt.DWORD()
            user32.GetWindowThreadProcessId(h, ctypes.byref(p))
            if p.value == pid:
                buf = ctypes.create_unicode_buffer(256)
                user32.GetWindowTextW(h, buf, 256)
                # "Choppy", or while copying "12% · ... - Choppy"
                if buf.value == "Choppy" or buf.value.endswith(" - Choppy"):
                    found.append(h)
                    return False
        return True

    user32.EnumWindows(EnumProc(cb), 0)
    return bool(found)


def env_for(data):
    e = {k: v for k, v in os.environ.items() if not k.startswith("CHOPPY_")}
    e.update(CHOPPY_DATA=data, CHOPPY_PARK="1", CHOPPY_TEST_AUTOCLOSE="1")
    return e


def data_dir(tag, takeover):
    d = os.path.join(ROOT, "data-" + tag)
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, "settings.ini"), "w", encoding="utf-8", newline="") as f:
        # a speed limit keeps each copy going a few seconds: 1.48 shows no window for a copy that ends before the
        # window is built (its close at completion comes first)
        f.write(f"ExplorerTakeover={1 if takeover else 0}\r\nSettingsVersion=2\r\nCloseWhenDone=1\r\nCloseAfterSeconds=0\r\n"
                "SpeedLimitMbps=8\r\nLimitValueMbps=8\r\n")
    return d


def wait_until(cond, secs, step=0.0005):
    end = time.perf_counter() + secs
    while time.perf_counter() < end:
        if cond():
            return True
        time.sleep(step)
    return False


def one(exe, data, src, n, server_pid=None):
    dest = os.path.join(ROOT, f"dst-{n}")
    os.makedirs(dest)
    t = time.perf_counter()
    p = subprocess.Popen([exe, "copy", src, dest], env=env_for(data))
    pid = server_pid or p.pid
    ok = wait_until(lambda: visible_choppy_window(pid), 15)
    took = time.perf_counter() - t
    # the copy finishes and its window closes itself; the second start ended long ago
    wait_until(lambda: not visible_choppy_window(pid), 30, 0.01)
    if server_pid is None:
        try:
            p.wait(timeout=15)
        except subprocess.TimeoutExpired:
            p.kill()
    else:
        p.wait(timeout=15)
    return took if ok else None


def measure(exe, tag, runs):
    src = os.path.join(ROOT, "src.bin")
    out = {}
    # warm: in the tray first
    data = data_dir(tag + "-warm", True)
    tray = subprocess.Popen([exe, "--tray"], env=env_for(data))
    try:
        time.sleep(8)
        times = [one(exe, data, src, f"{tag}-w{i}", tray.pid) for i in range(runs + 1)][1:]  # the first: untimed
        out["already running"] = times
    finally:
        subprocess.run([exe, "--quit"], env=env_for(data), timeout=30)
        try:
            tray.wait(timeout=10)
        except subprocess.TimeoutExpired:
            tray.kill()
    data = data_dir(tag + "-cold", False)
    times = [one(exe, data, src, f"{tag}-c{i}") for i in range(runs + 1)][1:]
    out["not running"] = times
    return out


def main():
    args = sys.argv[1:]
    runs = 7
    if "--runs" in args:
        i = args.index("--runs")
        runs = int(args[i + 1])
        del args[i:i + 2]
    shutil.rmtree(ROOT, ignore_errors=True)
    os.makedirs(ROOT)
    with open(os.path.join(ROOT, "src.bin"), "wb") as f:
        f.write(os.urandom(3 << 20))
    try:
        for k, exe in enumerate(args):
            exe = os.path.abspath(exe)
            res = measure(exe, f"x{k}", runs)
            for mode, times in res.items():
                good = [t for t in times if t is not None]
                med = statistics.median(good) if good else float("nan")
                print(f"{exe} | {mode}: median {med:.3f} s  (runs: {' '.join(f'{t:.3f}' if t else 'none' for t in times)})")
    finally:
        time.sleep(0.5)
        shutil.rmtree(ROOT, ignore_errors=True)


if __name__ == "__main__":
    main()
