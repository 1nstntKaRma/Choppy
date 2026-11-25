# hook.py: drives a test-mode Choppy (C# or Rust) through its test hook (TestHook.cs's protocol). Never the installed
# one: CHOPPY_DATA keeps its settings and logs in a folder of its own.
import os, time, subprocess, shutil

INI = ("SettingsVersion=2\nView=extended\nCloseWhenDone=1\nCloseAfterSeconds=0\nVerifyCopies=0\nExplorerTakeover=0\n"
       "SpeedUnit=auto\nSpeedLimitMbps=0\nLimitValueMbps=100.663296\nPolicy=Ask\n")


class Choppy:
    def __init__(self, exe, root, name, settings=None, args=(), park=True, env_extra=None):
        self.data = os.path.join(root, name + "-data"); self.hook = os.path.join(root, name + "-hook")
        for d in (self.data, self.hook):
            shutil.rmtree(d, ignore_errors=True); os.makedirs(d)
        open(os.path.join(self.data, "settings.ini"), "w", newline="\r\n").write(settings or INI)
        env = dict(os.environ, CHOPPY_DATA=self.data, CHOPPY_TESTHOOK=self.hook)
        if park: env["CHOPPY_PARK"] = "1"
        if env_extra: env.update(env_extra)
        self.p = subprocess.Popen([exe] + list(args), env=env)
        self.out = os.path.join(self.hook, "out.txt"); self.seen = 0
        t0 = time.time()
        while time.time() - t0 < 20:
            if os.path.exists(self.out) and "ready" in open(self.out, encoding="utf-8", errors="replace").read(): break
            time.sleep(0.05)
        self.seen = len(open(self.out, encoding="utf-8", errors="replace").read().splitlines()) if os.path.exists(self.out) else 0

    def cmd(self, line, timeout=20):
        f = os.path.join(self.hook, "cmd.txt")
        open(f + ".tmp", "w", encoding="utf-8").write(line + "\n"); os.replace(f + ".tmp", f)
        t0 = time.time()
        while time.time() - t0 < timeout:
            lines = open(self.out, encoding="utf-8", errors="replace").read().splitlines() if os.path.exists(self.out) else []
            if len(lines) > self.seen:
                r = lines[self.seen:]; self.seen = len(lines); return r[-1].split("=>")[-1].strip()
            if self.p.poll() is not None: return "exited %s" % self.p.returncode
            time.sleep(0.05)
        return "timeout"

    def quit(self):
        try: self.cmd("quit", 5)
        except Exception: pass
        try: self.p.wait(5)
        except Exception: self.p.kill()
