"""Step 5, for the user: `--where` of C# 1.48 and of the Rust Choppy with Explorer windows open (it only reads them).
Open a few Explorer windows first (Windows 11: several tabs in one, the last one active; also This PC and a zip
folder), then:

    python wherecheck.py <C# Choppy.exe, e.g. the installed one> <Rust choppy.exe>

Both run in test mode with a temporary data folder; the milliseconds are masked. Prints SAME or both outputs.
"""
import os
import re
import subprocess
import sys
import tempfile


def where(exe, data):
    out = os.path.join(data, "where.txt")
    e = {k: v for k, v in os.environ.items() if not k.startswith("CHOPPY_")}
    e["CHOPPY_DATA"] = data
    subprocess.run([exe, "--where", out], env=e, timeout=60)
    text = open(out, encoding="utf-8-sig").read()
    return re.sub(r"\[\d+ ms\]", "[ms]", text)


def main():
    with tempfile.TemporaryDirectory(prefix="choppy-where-") as d:
        a, b = os.path.join(d, "cs"), os.path.join(d, "rs")
        os.makedirs(a)
        os.makedirs(b)
        ca, rb = where(sys.argv[1], a), where(sys.argv[2], b)
        if ca == rb:
            print("SAME\n" + ca)
        else:
            print("DIFFERENT\n--- C#:\n" + ca + "--- Rust:\n" + rb)


if __name__ == "__main__":
    main()
