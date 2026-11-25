# cmp.py a.layout.txt b.layout.txt [all]: WPF's and Rust's element rectangles side by side, matched by name in order
import sys, re
def load(p):
    out = []
    for l in open(p, encoding="utf-8-sig"):
        m = re.match(r"\s*(.*?)\s+x=\s*([-\d.]+) y=\s*([-\d.]+) w=\s*([-\d.]+) h=\s*([-\d.]+)", l.rstrip("\r\n"))
        if m: out.append((m.group(1).strip(), *[float(v) for v in m.groups()[1:]]))
        elif l.startswith("window"): print(p.replace("\\", "/").split("/")[-1], l.strip())
    return out
a, b = load(sys.argv[1]), load(sys.argv[2])
j = 0; bad = 0
for (n, x, y, w, h) in a:
    k = next((i for i in range(j, len(b)) if b[i][0] == n), None)
    if k is None:
        print(f"  {n[:40]:40}  only in WPF  {x:7.1f} {y:7.1f} {w:7.1f} {h:7.1f}"); continue
    j = k + 1
    _, x2, y2, w2, h2 = b[k]
    d = (round(x2 - x, 1), round(y2 - y, 1), round(w2 - w, 1), round(h2 - h, 1))
    flag = "" if all(abs(v) < 0.6 for v in d) else "  <<"
    if flag: bad += 1
    if flag or len(sys.argv) > 3:
        print(f"  {n[:40]:40}  WPF {x:7.1f} {y:7.1f} {w:7.1f} {h:7.1f}   Rust d {d}{flag}")
print("differ:", bad, "of", len(a))
