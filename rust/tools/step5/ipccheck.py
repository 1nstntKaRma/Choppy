"""Step 5: the single instance live, in test mode only (CHOPPY_DATA; the names end in "-test", so nothing here can meet
the installed Choppy). A Choppy in the tray takes a second start's copy through the pipe (relative paths resolved in
the second start's folder); C# 1.48 and Rust hand jobs to each other both ways (the same protocol); --quit reaches the
running one; the administrator copy (--elevated-job on a genuine record, run here without the prompt) has its own
"-admin" instance, copies and leaves its result.

    python ipccheck.py <C# Choppy.exe> <Rust choppy.exe>

Test files in C:\\choppy-step5-ipc, removed afterwards.
"""
import hashlib
import os
import shutil
import subprocess
import sys
import time

ROOT = r"C:\choppy-step5-ipc"


def env_for(data, **extra):
    e = {k: v for k, v in os.environ.items() if not k.startswith("CHOPPY_")}
    e.update(CHOPPY_DATA=data, CHOPPY_PARK="1", CHOPPY_TEST_AUTOCLOSE="1", **extra)
    return e


def data_dir(name, takeover):
    d = os.path.join(ROOT, "data-" + name)
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, "settings.ini"), "w", encoding="utf-8", newline="") as f:
        f.write(f"ExplorerTakeover={1 if takeover else 0}\r\nSettingsVersion=2\r\nCloseAfterSeconds=0\r\n")
    return d


def pipes():
    return sorted(p for p in os.listdir("\\\\.\\pipe\\") if p.startswith("Choppy-"))


def wait_for(cond, secs):
    end = time.time() + secs
    while time.time() < end:
        if cond():
            return True
        time.sleep(0.05)
    return cond()


def same_tree(a, b):
    for dp, _, fs in os.walk(a):
        for f in fs:
            p = os.path.join(dp, f)
            q = os.path.join(b, os.path.relpath(p, a))
            if not os.path.isfile(q) or open(p, "rb").read() != open(q, "rb").read():
                return False
    return True


results = []


def check(name, ok):
    results.append(ok)
    print(("PASS " if ok else "FAIL ") + name)


def hand_over(server_exe, client_exe, label):
    data = data_dir(label, True)
    tray = subprocess.Popen([server_exe, "--tray"], env=env_for(data))
    try:
        check(f"{label}: the tray Choppy waits (its pipe ends in -test)", wait_for(lambda: tray.poll() is None and any(p.endswith("-test") for p in pipes()), 10))
        dest = os.path.join(ROOT, "dst-" + label)
        os.makedirs(dest, exist_ok=True)
        t = time.perf_counter()
        # relative source and destination, from the test folder: made absolute by the second start
        r = subprocess.run([client_exe, "copy", "src", "dst-" + label], cwd=ROOT, env=env_for(data), timeout=30)
        took = time.perf_counter() - t
        check(f"{label}: the second start handed it over and ended (exit {r.returncode}, {took:.3f} s)", r.returncode == 0 and took < 5)
        check(f"{label}: the running one copied it", wait_for(lambda: same_tree(os.path.join(ROOT, "src"), os.path.join(dest, "src")), 20))
        check(f"{label}: the tray Choppy still runs", tray.poll() is None)
        q = subprocess.run([client_exe, "--quit"], env=env_for(data), timeout=30)
        gone = wait_for(lambda: tray.poll() is not None, 10)
        check(f"{label}: --quit reached it (exit {q.returncode}; the tray Choppy's exit {tray.poll()})", gone and tray.returncode == 0)
    finally:
        if tray.poll() is None:
            tray.kill()


def elevated(exe, label):
    data = data_dir("adm-" + label, False)
    jobs = os.path.join(data, "Jobs")
    os.makedirs(jobs, exist_ok=True)
    dest = os.path.join(ROOT, "dst-adm-" + label)
    os.makedirs(dest, exist_ok=True)
    rec = os.path.join(jobs, f"step5{label}.admin")
    hook = os.path.join(data, "hook")
    os.makedirs(hook, exist_ok=True)
    # a test window stays open: the test hook (carried in the record too) quits it once the result is there
    text = (f"op=Copy\r\ndest={dest}\r\nverify=0\r\npolicy=Ask\r\nlimit=0\r\norigin=Command line\r\n"
            f"src={os.path.join(ROOT, 'src')}\r\nparent={os.getpid()}\r\ntestdata={data}\r\ntesthook={hook}\r\ntestpark=1\r\n")
    with open(rec, "wb") as f:
        f.write(b"\xef\xbb\xbf" + text.encode("utf-8"))
    sha = hashlib.sha256(open(rec, "rb").read()).hexdigest()
    # a fresh environment, as an elevated start gets: test mode comes from the record
    e = {k: v for k, v in os.environ.items() if not k.startswith("CHOPPY_")}
    p = subprocess.Popen([exe, "--elevated-job", rec, sha], env=e)
    seen = wait_for(lambda: any(x.endswith("-test-admin") for x in pipes()) or p.poll() is not None, 10)
    admin_pipe = any(x.endswith("-test-admin") for x in pipes())
    res = rec + ".result"
    wait_for(lambda: os.path.exists(res) and "end" in open(res, encoding="utf-8-sig").read(), 30)
    with open(os.path.join(hook, "cmd.txt"), "w", encoding="utf-8") as f:
        f.write("quit\r\n")
    try:
        code = p.wait(timeout=30)
    except subprocess.TimeoutExpired:
        p.kill()
        code = "killed"
    check(f"{label} administrator copy: its own -test-admin instance ({admin_pipe and seen}), exit {code}", code == 0 and admin_pipe)
    check(f"{label} administrator copy: copied", same_tree(os.path.join(ROOT, "src"), os.path.join(dest, "src")))
    lines = open(res, encoding="utf-8-sig").read().splitlines() if os.path.exists(res) else []
    check(f"{label} administrator copy: result left ({len(lines)} lines, ends with 'end')", bool(lines) and lines[-1].startswith("end"))
    # the log's name carries the time and the job's id
    return [l for l in lines if not l.startswith(("seconds=", "end", "log="))]


def main():
    cs, rs = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
    shutil.rmtree(ROOT, ignore_errors=True)
    src = os.path.join(ROOT, "src")
    os.makedirs(os.path.join(src, "sub"))
    for i, size in enumerate([0, 1, 4096, 1 << 20, 3_000_017]):
        with open(os.path.join(src, f"f{i}.bin"), "wb") as f:
            f.write(os.urandom(size))
    with open(os.path.join(src, "sub", "ü é 😀.txt"), "w", encoding="utf-8") as f:
        f.write("unicode name\n")
    try:
        before = pipes()
        hand_over(rs, rs, "rust-to-rust")
        hand_over(cs, rs, "rust-to-cs")
        hand_over(rs, cs, "cs-to-rust")
        a = elevated(cs, "cs")
        b = elevated(rs, "rust")
        check(f"administrator copy: the result files say the same ({len(a)} lines besides the times)", a == b)
        check("the installed Choppy's pipe untouched", [p for p in pipes() if not p.endswith(("-test", "-test-admin"))] == [p for p in before if not p.endswith(("-test", "-test-admin"))])
        print("ALL PASS" if all(results) else f"{results.count(False)} FAILED")
    finally:
        time.sleep(0.5)
        shutil.rmtree(ROOT, ignore_errors=True)


if __name__ == "__main__":
    main()
