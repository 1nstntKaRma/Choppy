# fixture.py <base folder>: a source tree and a destination with conflicts, for --plan-dump --scan (run-all.sh).
# Odd sizes, hidden + read-only, unicode and emoji names, a path past 260 characters, a junction, name conflicts in
# another case, ".chpart" leftovers, a file where a folder goes, names in the way of "name - Copy".
import os, sys, subprocess, ctypes

base = os.path.abspath(sys.argv[1])
L = lambda p: '\\\\?\\' + p  # long form for deep paths


def write(p, n, attr=None):
    os.makedirs(os.path.dirname(L(p)), exist_ok=True)
    with open(L(p), 'wb') as f:
        f.write(bytes((i * 7) & 255 for i in range(n)) if n < 100000 else b'\0' * n)
    if attr:
        ctypes.windll.kernel32.SetFileAttributesW(L(p), attr)


src = os.path.join(base, 'src')
dst = os.path.join(base, 'dst')
write(os.path.join(src, 'a.txt'), 100)
write(os.path.join(src, 'big.bin'), 5 << 20)
write(os.path.join(src, 'Same.TXT'), 10)
write(os.path.join(src, 'hidden.txt'), 3, 0x2 | 0x1)  # hidden + read-only
write(os.path.join(src, 'sub1', 'b.txt'), 2000)
write(os.path.join(src, 'sub1', 'sub2', 'c.txt'), 1)
write(os.path.join(src, 'sub1', 'sub2', 'ü-é.txt'), 50)
write(os.path.join(src, 'sub1', 'sub2', '\U0001F600.bin'), 77)
write(os.path.join(src, 'sub1', 'sub2', 'ISTANBUL.txt'), 5)
os.makedirs(os.path.join(src, 'sub1', 'empty'), exist_ok=True)
write(os.path.join(src, 'sub1', 'zz', 'last.txt'), 9)
write(os.path.join(src, 'conflict-dir', 'inner.txt'), 4)
deep = os.path.join(src, 'long', *(['d' * 60] * 5))
write(os.path.join(deep, 'deep file.txt'), 123)
for i in range(30):
    write(os.path.join(src, 'many', 'f%02d.dat' % i), 1000 * i)
subprocess.run(['cmd', '/c', 'mklink', '/J', os.path.join(src, 'link'), os.path.join(src, 'sub1')], capture_output=True)

write(os.path.join(dst, 'a.txt'), 55)                 # a conflict, another size
write(os.path.join(dst, 'big.bin.chpart'), 1000)      # a leftover from a stopped copy
write(os.path.join(dst, 'same.txt'), 10)              # the same name in another case
write(os.path.join(dst, 'SUB1', 'B.TXT'), 2000)       # the folder exists, a file in it too
write(os.path.join(dst, 'sub1', 'sub2'), 3)           # a FILE where a folder goes
write(os.path.join(dst, 'sub1', 'zz', 'last.txt'), 9)
write(os.path.join(dst, 'sub1', 'zz', 'LAST.txt.CHPART'), 1)
write(os.path.join(dst, 'conflict-dir'), 8)           # a file with a folder's name
write(os.path.join(dst, 'many', 'f05.dat'), 1)
os.makedirs(os.path.join(dst, 'a.txt - Copy.txt'), exist_ok=True)  # a folder in the way of "a - Copy" naming
write(os.path.join(dst, 'x - Copy.bin'), 1)
write(os.path.join(dst, 'x.bin'), 1)
write(os.path.join(dst, 'folder', 'f.txt'), 1)
write(os.path.join(dst, 'folder - Copy', 'f.txt'), 1)
print('ok', base)
