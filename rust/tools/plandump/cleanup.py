# cleanup.py <base folder>...: removes fixture.py's trees (the junction first, so nothing is followed).
import os, shutil, stat, sys


def writable(func, path, exc):
    os.chmod(path, stat.S_IWRITE)  # the fixture's read-only file
    func(path)


for base in sys.argv[1:]:
    base = os.path.abspath(base)
    link = os.path.join(base, 'src', 'link')
    if os.path.lexists(link):
        os.rmdir(link)  # removes the junction itself, never what it points to
    if os.path.exists(base):
        shutil.rmtree('\\\\?\\' + base, onexc=writable)
    print(base, 'gone' if not os.path.exists(base) else 'STILL THERE')
