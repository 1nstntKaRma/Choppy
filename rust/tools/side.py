# side.py out.png zoom x0 y0 x1 y1 img1 img2 ... : the same crop of each image, stacked vertically, zoomed (nearest)
import sys
from PIL import Image
out, z = sys.argv[1], int(sys.argv[2]); box = tuple(int(v) for v in sys.argv[3:7]); ims = [Image.open(p).convert("RGB").crop(box) for p in sys.argv[7:]]
W = ims[0].width * z; H = sum(i.height * z + 4 for i in ims)
o = Image.new("RGB", (W, H), (255, 0, 255)); y = 0
for i in ims: o.paste(i.resize((i.width * z, i.height * z), Image.NEAREST), (0, y)); y += i.height * z + 4
o.save(out)
