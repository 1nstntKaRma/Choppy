# edges.py x img1 img2: rows where the color at column x jumps (borders, bars), side by side
import sys
from PIL import Image
x = int(sys.argv[1])
def edges(p):
    im = Image.open(p).convert("RGB"); px = im.load(); out = []
    prev = px[x, 0]
    for y in range(1, im.height):
        c = px[x, y]
        if sum(abs(a - b) for a, b in zip(c, prev)) > 30: out.append(y)
        prev = c
    return out
for p in sys.argv[2:]: print(p.split("/")[-1], edges(p))
