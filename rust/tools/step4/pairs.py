# pairs.py screen_dir out_dir [states_dir]: screen.py's captures (and states.py's) as pictures to look at, and numbers.
#   <scene>-<125|100>.png          WPF (C# 1.48) | Rust, side by side (monitor 1)
#   <scene>-<125|100>-overlay.png  WPF in red, Rust in cyan: gray where they agree, color fringes where they don't
#   state-<name>.png (+ -overlay)  states.py's pictures the same way (state-ask-rust.png: the Rust one only)
#   summary.txt                    per scene: sizes, pixels that differ (and visibly: by more than 48 of 255), and
#                                  whether each program's window is the same on monitor 2 as on monitor 1
# The numbers are taken from the captures as they are; the pictures blur what describes this PC (the Benchmark's
# drive labels, disk models and network share), the same area in both.
import os, sys
from PIL import Image, ImageChops, ImageDraw, ImageFilter

# 100 % pixels (x0, y0, x1, y1), scaled for 125 %
MASKS = {"benchmark": [(100, 253, 836, 492)]}


def masked(im, scene, dpi):
    for (x0, y0, x1, y1) in MASKS.get(scene, []):
        k = dpi / 96; box = tuple(int(round(v * k)) for v in (x0, y0, x1, y1))
        im.paste(im.crop(box).filter(ImageFilter.GaussianBlur(6 * k)), box)
    return im

src, out = sys.argv[1], sys.argv[2]
os.makedirs(out, exist_ok=True)


def load(n):
    p = os.path.join(src, n)
    return Image.open(p).convert("RGB") if os.path.exists(p) else None


def differ(a, b):
    """(share of pixels that differ at all, share that differ visibly) for two pictures of the same size."""
    if a.size != b.size: return None
    d = ImageChops.difference(a, b).convert("L")
    h = d.histogram(); n = a.width * a.height
    return (n - h[0]) / n, sum(h[49:]) / n


def pair(a, b, stem):
    w, h = a.width + b.width + 12, max(a.height, b.height) + 24
    o = Image.new("RGB", (w, h), (255, 255, 255)); dr = ImageDraw.Draw(o)
    dr.text((4, 5), f"WPF (C# 1.48) {a.width} x {a.height}", fill=(0, 0, 0))
    dr.text((a.width + 16, 5), f"Rust {b.width} x {b.height}", fill=(0, 0, 0))
    o.paste(a, (0, 24)); o.paste(b, (a.width + 12, 24)); o.save(os.path.join(out, f"{stem}.png"), optimize=True)
    A, B = a.convert("L"), b.convert("L"); W, H = max(A.width, B.width), max(A.height, B.height)
    AA = Image.new("L", (W, H)); AA.paste(A); BB = Image.new("L", (W, H)); BB.paste(B)
    Image.merge("RGB", (AA, BB, BB)).save(os.path.join(out, f"{stem}-overlay.png"), optimize=True)


lines = ["scene       scale  WPF size     Rust size    differ  visibly  same on monitor 2 (WPF / Rust)"]
for dpi, pct in ((120, 125), (96, 100)):
    for scene in ("report", "compact", "queue", "settings", "unithelp", "about", "benchmark", "conflict"):
        a, b = load(f"cs-{scene}-{dpi}-m1.png"), load(f"rs-{scene}-{dpi}-m1.png")
        if a is None or b is None: continue
        d = differ(a, b)
        a, b = masked(a, scene, dpi), masked(b, scene, dpi)
        pair(a, b, f"{scene}-{pct}")
        same = []
        for p in ("cs", "rs"):
            m1, m2 = load(f"{p}-{scene}-{dpi}-m1.png"), load(f"{p}-{scene}-{dpi}-m2.png")
            dd = differ(m1, m2) if m1 and m2 else None
            same.append("missing" if dd is None else "same" if dd[0] == 0 else f"{dd[0] * 100:.2f}% differ")
        ds = "size differs" if d is None else f"{d[0] * 100:5.2f}%  {d[1] * 100:5.2f}%"
        lines.append(f"{scene:11} {pct:4}%  {a.width:4} x {a.height:<4}  {b.width:4} x {b.height:<4}  {ds}   {same[0]} / {same[1]}")
if len(sys.argv) > 3:
    src = sys.argv[3]
    lines += ["", "state (test hook pictures, 144 DPI)  WPF size     Rust size    differ  visibly"]
    for st in ("run", "run-compact", "compared", "bench-done"):
        a, b = load(f"cs-{st}.png"), load(f"rs-{st}.png")
        if a is None or b is None: continue
        d = differ(a, b)
        pair(a, b, f"state-{st}")
        ds = "size differs" if d is None else f"{d[0] * 100:5.2f}%  {d[1] * 100:5.2f}%"
        lines.append(f"{st:36} {a.width:4} x {a.height:<4}  {b.width:4} x {b.height:<4}  {ds}")
    ask = load("rs-ask.png")
    if ask: ask.save(os.path.join(out, "state-ask-rust.png"), optimize=True)
open(os.path.join(out, "summary.txt"), "w", newline="\r\n").write("\n".join(lines) + "\n")
print("\n".join(lines))
