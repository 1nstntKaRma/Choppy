"""Step 5 gate: the registry writes of C# 1.48 (built from the rust branch) and of the Rust Choppy, compared on a
scratch key. Both run in test mode (CHOPPY_DATA) with CHOPPY_REGTEST=<scratch key>: the Run key, the right-click
verbs and the icon stamp land under HKCU\\<scratch key>\\... - never the real ones - and "installed" means the exe sits
in <data folder>\\Programs\\Choppy. Each program goes through the same steps; after each one the scratch key and the
settings.ini are dumped, with the exe's folder written as <EXE> (both run from the same place in turn).

    python regcheck.py <C# Choppy.exe> <Rust choppy.exe> [work dir]

Prints SAME / DIFF per step; leaves nothing behind (the scratch keys and the work dir are removed).
"""
import os
import shutil
import subprocess
import sys
import tempfile
import time
import uuid
import winreg

RUN = r"Software\Microsoft\Windows\CurrentVersion\Run"
TYPES = {1: "SZ", 2: "EXPAND_SZ", 3: "BINARY", 4: "DWORD", 7: "MULTI_SZ", 11: "QWORD"}


def env_for(data, root):
    e = dict(os.environ)
    e.update(CHOPPY_DATA=data, CHOPPY_PARK="1", CHOPPY_TEST_AUTOCLOSE="1", CHOPPY_REGTEST=root)
    for k in ("CHOPPY_TESTHOOK", "CHOPPY_DPI96"):
        e.pop(k, None)
    return e


def run(exe, data, root, *args, wait=True):
    assert data and root
    p = subprocess.Popen([exe, *args], env=env_for(data, root))
    if not wait:
        return p
    return p.wait(timeout=60)


def dump_key(root):
    out = []

    def walk(path, rel):
        try:
            k = winreg.OpenKey(winreg.HKEY_CURRENT_USER, path)
        except FileNotFoundError:
            return
        with k:
            vals = []
            i = 0
            while True:
                try:
                    n, v, t = winreg.EnumValue(k, i)
                except OSError:
                    break
                vals.append((n, v, t))
                i += 1
            for n, v, t in sorted(vals, key=lambda x: x[0].lower()):
                out.append(f"{rel or '.'} | {n or '(default)'} = {TYPES.get(t, t)} {v!r}")
            subs = []
            i = 0
            while True:
                try:
                    subs.append(winreg.EnumKey(k, i))
                except OSError:
                    break
                i += 1
            for s in sorted(subs, key=str.lower):
                walk(path + "\\" + s, (rel + "\\" if rel else "") + s)

    walk(root, "")
    return out


def delete_tree(path):
    try:
        k = winreg.OpenKey(winreg.HKEY_CURRENT_USER, path, 0, winreg.KEY_ALL_ACCESS)
    except FileNotFoundError:
        return
    with k:
        while True:
            try:
                s = winreg.EnumKey(k, 0)
            except OSError:
                break
            delete_tree(path + "\\" + s)
    winreg.DeleteKey(winreg.HKEY_CURRENT_USER, path)


def set_value(path, name, value):
    with winreg.CreateKey(winreg.HKEY_CURRENT_USER, path) as k:
        winreg.SetValueEx(k, name, 0, winreg.REG_SZ, value)


def ini(data):
    p = os.path.join(data, "settings.ini")
    if not os.path.exists(p):
        return ["(no settings.ini)"]
    return ["ini | " + l for l in open(p, encoding="utf-8-sig").read().splitlines()]


def wait_gone(p, secs=20):
    try:
        p.wait(timeout=secs)
        return True
    except subprocess.TimeoutExpired:
        p.kill()
        return False


def steps(src_exe, work, tag):
    """Runs the steps with one program; returns [(step, lines)]."""
    root = rf"Software\Choppy-step5-regcheck-{uuid.uuid4().hex[:8]}"
    data = os.path.join(work, "data-" + tag)
    os.makedirs(data)
    loose = os.path.join(work, "loose", "Choppy.exe")  # not the installed copy
    os.makedirs(os.path.dirname(loose))
    shutil.copy2(src_exe, loose)
    inst_dir = os.path.join(data, "Programs", "Choppy")
    os.makedirs(inst_dir)
    inst = os.path.join(inst_dir, "Choppy.exe")
    shutil.copy2(src_exe, inst)
    res = []

    def snap(name, code=None):
        lines = dump_key(root) + ini(data)
        for a, b in ((inst_dir, "<EXE>"), (os.path.dirname(loose), "<LOOSE>"), (root, "<ROOT>")):
            lines = [l.replace(a.replace("\\", "\\\\"), b).replace(a, b) for l in lines]
        if code is not None:
            lines.insert(0, f"exit {code}")
        res.append((name, lines))

    try:
        # another copier's startup entry and someone else's, before anything
        set_value(root + "\\" + RUN, "ultracopier", r'"C:\Program Files\Ultracopier\ultracopier.exe"')
        set_value(root + "\\" + RUN, "Other", "x")
        snap("seed")
        snap("1 --menu, not the installed copy: only the icon stamp", run(loose, data, root, "--menu"))
        snap("2 --menu, the installed copy: + the right-click verbs", run(inst, data, root, "--menu"))
        p = run(inst, data, root, "--takeover", "on", wait=False)
        time.sleep(3)
        alive = p.poll() is None
        q = run(inst, data, root, "--quit")  # through the pipe to the running one
        gone = wait_gone(p)
        snap(f"3 --takeover on (stays in the tray: {alive}), then --quit (exit {q}, it went: {gone})")
        set_value(root + "\\" + RUN, "Choppy", r'"C:\old place\Choppy.exe" --tray')
        snap("4 --resume with a stale Run entry: corrected (takeover on)", run(inst, data, root, "--resume"))
        snap("5 --takeover off: Run entry gone, the other copier's back", run(inst, data, root, "--takeover", "off"))
        p = run(inst, data, root, "--takeover", "on", wait=False)
        time.sleep(3)
        q = run(inst, data, root, "--quit")
        wait_gone(p)
        snap(f"6 --takeover on again, --quit (exit {q})")
        snap("7 --uninstall: takeover off, verbs removed", run(inst, data, root, "--uninstall"))
        set_value(root + r"\Software\Choppy", "IconStamp", "0000000000000000")
        snap("8 another icon stamp, --menu: rewritten", run(inst, data, root, "--menu"))
        snap("9 --quit with nothing running", run(inst, data, root, "--quit"))
    finally:
        delete_tree(root)
    return res


def main():
    cs, rs = sys.argv[1], sys.argv[2]
    work = os.path.normpath(os.path.abspath(sys.argv[3] if len(sys.argv) > 3 else tempfile.mkdtemp(prefix="choppy-regcheck-")))
    os.makedirs(work, exist_ok=True)
    try:
        a = steps(cs, os.path.join(work, "cs"), "cs")
        b = steps(rs, os.path.join(work, "rs"), "rs")
        same = True
        for (n, la), (_, lb) in zip(a, b):
            ok = la == lb
            same &= ok
            print(("SAME " if ok else "DIFF ") + n)
            for l in la if ok else []:
                print("     " + l)
            if not ok:
                print("   C#:")
                for l in la:
                    print("     " + l)
                print("   Rust:")
                for l in lb:
                    print("     " + l)
        print("ALL SAME" if same else "DIFFERENT")
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    main()
