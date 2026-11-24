# overlay.py a.png b.png out.png [x0 y0 x1 y1] [zoom]: a in red, b in cyan (grayscale); aligned = gray, offsets = color fringes
import sys
from PIL import Image, ImageChops
a = Image.open(sys.argv[1]).convert("L"); b = Image.open(sys.argv[2]).convert("L")
w, h = max(a.width, b.width), max(a.height, b.height)
A = Image.new("L", (w, h)); A.paste(a); B = Image.new("L", (w, h)); B.paste(b)
img = Image.merge("RGB", (A, B, B))
if len(sys.argv) > 4:
    box = tuple(int(v) for v in sys.argv[4:8]); img = img.crop(box)
z = int(sys.argv[8]) if len(sys.argv) > 8 else 1
if z > 1: img = img.resize((img.width * z, img.height * z), Image.NEAREST)
img.save(sys.argv[3])
# row profile: first/last ink rows of text lines in the left half
def rows(im):
    px = im.load(); out = []; inside = False
    for y in range(im.height):
        ink = any(px[x, y] > 150 for x in range(20, 400))
        if ink and not inside: start = y; inside = True
        if not ink and inside: out.append((start, y)); inside = False
    return out
print("A rows", rows(a)); print("B rows", rows(b))
