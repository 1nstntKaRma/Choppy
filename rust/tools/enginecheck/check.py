# check.py: step 2's gate. The C# Choppy 1.48 (built from this branch) and the Rust one run the same copies and moves
# with --cli on freshly built, identical fixture trees; then the destination trees (and what a move leaves at the
# source) are compared: names, sizes, contents, created / modified dates, attributes, extra data streams. The --json
# reports and the job logs are compared too (times, speeds and the run folders masked).
#
# Usage: python check.py <cs Choppy.exe> <rust choppy.exe> [scenario ...]
#   env WORK (default %TEMP%\choppy-engine), DRIVES="D X F" (hard disk / other drives to use; missing ones skipped)
# Prints SAME / DIFF per scenario; the dumps and diffs stay in WORK\out. Test folders are removed at the end.
import ctypes, ctypes.wintypes as wt, difflib, hashlib, json, os, random, re, shutil, subprocess, sys, time

k32 = ctypes.WinDLL('kernel32', use_last_error=True)


class FILETIME(ctypes.Structure):
    _fields_ = [('lo', wt.DWORD), ('hi', wt.DWORD)]


class WFAD(ctypes.Structure):
    _fields_ = [('attr', wt.DWORD), ('c', FILETIME), ('a', FILETIME), ('w', FILETIME), ('sh', wt.DWORD), ('sl', wt.DWORD)]


class FSD(ctypes.Structure):
    _fields_ = [('size', ctypes.c_longlong), ('name', ctypes.c_wchar * 296)]


k32.GetFileAttributesExW.argtypes = [wt.LPCWSTR, ctypes.c_int, ctypes.POINTER(WFAD)]
k32.SetFileAttributesW.argtypes = [wt.LPCWSTR, wt.DWORD]
k32.CreateFileW.restype = wt.HANDLE
k32.CreateFileW.argtypes = [wt.LPCWSTR, wt.DWORD, wt.DWORD, ctypes.c_void_p, wt.DWORD, wt.DWORD, wt.HANDLE]
k32.SetFileTime.argtypes = [wt.HANDLE, ctypes.POINTER(FILETIME), ctypes.POINTER(FILETIME), ctypes.POINTER(FILETIME)]
k32.CloseHandle.argtypes = [wt.HANDLE]
k32.FindFirstStreamW.restype = wt.HANDLE
k32.FindFirstStreamW.argtypes = [wt.LPCWSTR, ctypes.c_int, ctypes.POINTER(FSD), wt.DWORD]
k32.FindNextStreamW.argtypes = [wt.HANDLE, ctypes.POINTER(FSD)]
k32.FindClose.argtypes = [wt.HANDLE]
INVALID = wt.HANDLE(-1).value


def L(p):
    p = os.path.abspath(p) if not p.startswith('\\\\') else p
    if p.startswith('\\\\?\\'):
        return p
    if p.startswith('\\\\'):
        return '\\\\?\\UNC\\' + p[2:]
    return '\\\\?\\' + p


def ft(v):
    return FILETIME(v & 0xFFFFFFFF, v >> 32)


def info(p):
    d = WFAD()
    if not k32.GetFileAttributesExW(L(p), 0, ctypes.byref(d)):
        return None
    t = lambda f: (f.hi << 32) | f.lo
    return d.attr, t(d.c), t(d.w), (d.sh << 32) | d.sl


def set_times(p, c, w, is_dir=False, link=False):
    flags = (0x02000000 if is_dir else 0) | (0x00200000 if link else 0)  # backup semantics, the link itself
    h = k32.CreateFileW(L(p), 0x100, 7, None, 3, flags, None)  # FILE_WRITE_ATTRIBUTES
    if h == INVALID or h is None:
        raise OSError(ctypes.get_last_error(), 'SetFileTime open ' + p)
    k32.SetFileTime(h, ctypes.byref(ft(c)), None, ctypes.byref(ft(w)))
    k32.CloseHandle(h)


def streams(p):
    d = FSD()
    h = k32.FindFirstStreamW(L(p), 0, ctypes.byref(d), 0)
    if h == INVALID or h is None:
        return []
    r = []
    while True:
        if d.name != '::$DATA':
            r.append(d.name)
        if not k32.FindNextStreamW(h, ctypes.byref(d)):
            break
    k32.FindClose(h)
    return sorted(r)


def digest(p):
    h = hashlib.sha1()
    with open(L(p), 'rb') as f:
        for b in iter(lambda: f.read(1 << 20), b''):
            h.update(b)
    return h.hexdigest()[:16]


T0 = 132000000000000000  # 2019-04-14, the fixture's dates count up from here


def write(p, data, attr=0, t=0, zone=None):
    os.makedirs(os.path.dirname(L(p)), exist_ok=True)
    with open(L(p), 'wb') as f:
        f.write(data)
    if zone is not None:
        with open(L(p) + ':Zone.Identifier', 'wb') as f:
            f.write(zone)
    set_times(p, T0 + t * 10_000_000 * 3600, T0 + t * 10_000_000 * 3600 + 7_654_321)
    if attr:
        k32.SetFileAttributesW(L(p), attr)


def rnd(n, seed):
    return random.Random(seed).randbytes(n)


ZONE = b'[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=https://example.com/x\r\n'
LONG = os.path.join('long', *(['d' * 60] * 5))


def build_source(root, pair=False):
    """The source tree: odd sizes, attributes, unicode and emoji names, a path past 260 characters, extra streams, big
    files for the pipeline, many small files, an empty folder, hidden and system folders, a junction."""
    w = lambda rel, *a, **k: write(os.path.join(root, rel.replace('/', os.sep)), *a, **k)
    w('a.txt', b'a' * 100, t=1)
    w('Same.TXT', b'same-12345', t=2)
    w('hidden.txt', b'hid', 0x2 | 0x1, t=3)
    w('system.txt', b'system!', 0x4 | 0x20, t=4)
    w('nci.txt', b'not index', 0x2000 | 0x20, t=5)
    w('zone.txt', rnd(1500, 1), t=6, zone=ZONE)
    w('empty.txt', b'', t=7)
    w('x.bin', b'x', t=8)
    if pair:
        # a real file of the job that only looks like a leftover: "keep.bin.chpart" is also keep.bin's temp name. Only
        # where the destination has it already (copied by an interrupted run): two workers making both at once race
        # for the name, and Windows' name tunneling then hands one the other's creation date - in both programs
        w('keep.bin', rnd(100, 12), t=23)
        w('keep.bin.chpart', rnd(50, 13), t=24)
    w('mid.bin', rnd((5 << 20) + 77, 2), t=9)
    w('big1.bin', rnd(40 << 20, 3), t=10, zone=ZONE)
    w('big2.bin', rnd((96 << 20) + 12345, 4), 0x1 | 0x20, t=11)
    # NTFS-compressed: never through the pipeline (Windows' own copy takes it)
    w('packed.bin', rnd((34 << 20) + 5, 11), t=22)
    subprocess.run(['compact', '/c', '/q', os.path.join(root, 'packed.bin')], capture_output=True)
    set_times(os.path.join(root, 'packed.bin'), T0 + 22 * 10_000_000 * 3600, T0 + 22 * 10_000_000 * 3600 + 7_654_321)
    w('sub1/b.txt', rnd(2000, 5), t=12)
    w('sub1/sub2/c.txt', b'c', t=13)
    w('sub1/sub2/ü-é.txt', rnd(50, 6), t=14)
    w('sub1/sub2/\U0001F600.bin', rnd(77, 7), t=15)
    w('sub1/sub2/ISTANBUL.txt', b'istan', t=16)
    os.makedirs(L(os.path.join(root, 'sub1', 'empty')), exist_ok=True)
    w('sub1/zz/last.txt', b'last-file', t=17)
    w('hid/h.txt', b'hhhh', t=18)
    w('sysdir/s.txt', b'ssss', t=19)
    w('conflict-dir/inner.txt', b'innr', t=20)
    w(os.path.join(LONG, 'deep file.txt'), rnd(123, 8), t=21)
    for i in range(200):
        w('many/f%03d.dat' % i, rnd((i * 517) % 70000, 100 + i), t=30 + i)
    subprocess.run(['cmd', '/c', 'mklink', '/J', os.path.join(root, 'link'), os.path.join(root, 'sub1')], capture_output=True)
    # folder dates last (adding entries changes them), deepest first; then the folder attributes
    dirs = []
    for d, subs, _ in os.walk(L(root)):
        subs[:] = [s for s in subs if s != 'link']
        dirs.append(d)
    for i, d in enumerate(sorted(dirs, key=lambda x: -x.count('\\'))):
        set_times(d, T0 - (i + 1) * 10_000_000 * 86400, T0 - (i + 1) * 10_000_000 * 86400 + 1234567, True)
    if os.path.lexists(L(os.path.join(root, 'link'))):  # no junctions on a network share
        set_times(os.path.join(root, 'link'), T0 + 7, T0 + 8, True, True)
    k32.SetFileAttributesW(L(os.path.join(root, 'hid')), 0x2 | 0x10)
    k32.SetFileAttributesW(L(os.path.join(root, 'sysdir')), 0x4 | 0x20 | 0x10)


def build_dest(root, src_root):
    """What is already at the destination: conflicts of every kind, leftovers of a stopped copy, a file where a folder
    goes, a folder where a file goes, copies an interrupted run finished (same size and date)."""
    w = lambda rel, *a, **k: write(os.path.join(root, rel.replace('/', os.sep)), *a, **k)
    w('a.txt', b'old' * 18 + b'x', t=0)  # older, another size
    w('keep.bin.chpart', rnd(50, 13), t=24)  # the job's own file, already copied: a resume never deletes it
    w('hidden.txt', b'older hidden', 0x1, t=0)  # read-only: a replace clears it first
    w('same.txt', b'newer-same', t=500)  # the same name in another case, newer
    w('big1.bin.chpart', b'\0' * 1000, t=0)  # a leftover of a stopped copy
    w('SUB1/B.TXT', rnd(2000, 5), t=12)  # same size + date as the source: a resumed copy finished it
    w('sub1/sub2', b'abc', t=0)  # a FILE where a folder goes
    w('sub1/zz/last.txt', b'last-file', t=17)
    w('sub1/zz/LAST.txt.CHPART', b'1', t=0)
    w('conflict-dir', b'8 bytes!', t=0)  # a file with a folder's name
    w('many/f005.dat', b'1', t=900)  # newer
    w('many/f006.dat', rnd((6 * 517) % 70000, 106), t=36)  # identical (resume: already there)
    w('mid.bin', rnd((5 << 20) + 77, 2), t=9)  # identical
    w('zone.txt', rnd(1500, 99), t=6)  # same size + date, other bytes (a power cut): a move must not trust it
    os.makedirs(L(os.path.join(root, 'hid')), exist_ok=True)  # an existing folder (merged)


def dump(root, existed=()):
    """Every entry under root: kind, size, attributes, dates, content, streams. Folders that existed before the run
    show no dates (Windows changes them as entries come and go)."""
    out = []
    base = L(root)
    for d, subs, files in os.walk(base):
        junctions = [s for s in subs if os.path.isjunction(os.path.join(d, s))]
        subs[:] = sorted(s for s in subs if s not in junctions)  # never followed
        for n in sorted(subs + junctions) + sorted(files):
            p = os.path.join(d, n)
            rel = p[len(base) + 1:]
            if n in junctions:
                out.append('J %s' % rel)
                continue
            i = info(p)
            if i is None:
                out.append('%s | gone?' % rel)
                continue
            attr, c, wt_, size = i
            if attr & 0x10:
                dates = '-' if rel.lower() in existed else 'c %d m %d' % (c, wt_)
                out.append('D %s | %X | %s | %s' % (rel, attr, dates, ' '.join(streams(p))))
            else:
                ss = ['%s=%s' % (s, digest(p + s[:-6])) for s in streams(p)]
                out.append('F %s | %d | %X | c %d m %d | %s | %s' % (rel, size, attr, c, wt_, digest(p), ' '.join(ss)))
    return out


def names(root):
    base = L(root)
    r = set()
    for d, subs, files in os.walk(base):
        for n in subs + files:
            r.add(os.path.join(d, n)[len(base) + 1:].lower())
    return r


def rm(root):
    def onexc(func, path, exc):
        k32.SetFileAttributesW(path if path.startswith('\\\\?\\') else L(path), 0x80)
        try:
            func(path)
        except OSError:
            pass
    if os.path.lexists(L(root)):
        # junctions first (rmtree must not walk into them)
        for d, subs, _ in os.walk(L(root)):
            for s in list(subs):
                if os.path.islink(os.path.join(d, s)) or os.path.isjunction(os.path.join(d, s)):
                    os.rmdir(os.path.join(d, s))
                    subs.remove(s)
        shutil.rmtree(L(root), onexc=onexc)


def run(exe, data, args, timeout=600):
    os.makedirs(data, exist_ok=True)
    env = dict(os.environ, CHOPPY_DATA=data)
    t = time.time()
    p = subprocess.run([exe, '--cli'] + args, env=env, capture_output=True, timeout=timeout)
    return p.returncode, time.time() - t


def masked_json(path, roots):
    try:
        raw = open(path, encoding='utf-8-sig').read()
    except OSError:
        return ['(no json)']
    for r in roots:
        raw = raw.replace(json.dumps(r)[1:-1], '<ROOT>').replace(json.dumps(r.replace('\\\\?\\', ''))[1:-1], '<ROOT>')
    j = json.loads(raw)
    if j.get('tune'):
        # the tuner measured: which way won is timing (each program measures its own runs)
        j['badge'] = j['badge'].replace(' · tuned', '')
        j['engine'] = '*'
    for k in ('seconds', 'runSeconds', 'avgMbps', 'peakMbps', 'rttMs', 'tune', 'log'):
        j[k] = '*'
    j['notes'] = sorted(j['notes'])
    j['failedItems'] = sorted(j['failedItems'])
    return ['%s: %s' % (k, json.dumps(j[k], ensure_ascii=False)) for k in j]


def masked_log(data, roots):
    logs = sorted(os.listdir(os.path.join(data, 'Logs'))) if os.path.isdir(os.path.join(data, 'Logs')) else []
    if not logs:
        return ['(no log)']
    lines = open(os.path.join(data, 'Logs', logs[-1]), encoding='utf-8-sig').read().splitlines()
    out = []
    for l in lines:
        for r in roots:
            l = l.replace(r.replace('\\\\?\\', ''), '<ROOT>')
        l = re.sub(r'^Started .*, finished .* \(', 'Started *, finished * (', l)
        out.append(l)
    head, body = out[:4], out[4:]
    # notes and failures come in the order the workers met them
    return head + sorted(body) + ['(log name %s)' % re.sub(r'^\d{4}-\d\d-\d\d \d\d-\d\d-\d\d [0-9a-f]{12}\.txt$', 'date id.txt', logs[-1])]


# ---------------- scenarios ----------------
# (name, source drive, dest drive, op, extra args, dest kind: None | 'conflicts', sources (relative; None = whole tree))
def scenarios(drives):
    c = 'C'
    hd1 = next((d for d in drives if d in 'DX'), None)
    hd2 = next((d for d in drives if d in 'X' and d != hd1), None)
    usb = 'F' if 'F' in drives else None
    s = [
        ('copy-new', c, c, 'copy', [], None),
        ('copy-skip', c, c, 'copy', ['--policy', 'skip'], 'conflicts'),
        ('copy-replace', c, c, 'copy', ['--policy', 'replace'], 'conflicts'),
        ('copy-keepboth', c, c, 'copy', ['--policy', 'keepboth'], 'conflicts'),
        ('copy-newer', c, c, 'copy', ['--policy', 'newer'], 'conflicts'),
        ('copy-ask', c, c, 'copy', [], 'conflicts'),
        ('copy-resume', c, c, 'copy', ['--resume', '--policy', 'replace'], 'conflicts'),
        ('copy-verify', c, c, 'copy', ['--verify'], None),
        ('copy-own-folder', c, c, 'copy', [], 'own'),
        ('copy-fat32', c, c, 'copy', ['--max-file', '10000000'], None),
        ('move-same', c, c, 'move', ['--policy', 'replace'], 'conflicts'),
        ('move-same-new', c, c, 'move', [], None),
        ('copy-net', c, 'NET', 'copy', [], None),
        ('copy-net-verify', c, 'NET', 'copy', ['--verify'], None),
        ('copy-from-net', 'NET', c, 'copy', [], None),
        # Windows' own copy for every file: the way a copy within one ReFS drive goes (Plan.Clone), and any file the
        # pipeline can't take
        ('copy-copyfile', c, c, 'copy', ['--engine', 'copyfile'], None),
        ('copy-limit', c, c, 'copy', ['--limit', '1600'], None),
        ('copy-small-batch', c, c, 'copy', ['--small', 'batch'], None),
        ('copy-no-fast', c, c, 'copy', ['--no-fast'], None),
        ('copy-pause', c, c, 'copy', ['--limit', '800', '--pause', '0.4,1.2'], None),
        ('copy-skip-big', c, c, 'copy', ['--limit', '400', '--skip-at', 'big2.bin'], None),
        # one 3 GB file: the warm-up, all four trial ways and the cached end fit in it (the pipeline switches ways
        # between stretches, so each switch must keep the bytes in order)
        ('copy-tuner', c, c, 'copy', [], 'tuner'),
        # the progress log's lines (--log): the same format
        ('copy-log', c, c, 'copy', ['--limit', '1600', '--log', '{LOG}'], None),
        # canceled midway: compared by rule (every file there is whole, no .chpart left), not side by side
        ('cancel', c, c, 'copy', ['--limit', '400', '--cancel-after', '1.5'], 'cancel'),
    ]
    if hd1:
        s += [
            ('copy-ssd-hdd', c, hd1, 'copy', [], None),
            ('copy-hdd-ssd', hd1, c, 'copy', [], None),
            ('copy-one-hdd', hd1, hd1, 'copy', [], None),
            ('move-cross', c, hd1, 'move', ['--policy', 'keepboth'], 'conflicts'),
            ('move-resume', c, hd1, 'move', ['--resume', '--policy', 'replace'], 'conflicts'),
            ('move-verify', hd1, c, 'move', ['--verify'], None),
        ]
    if hd1 and hd2:
        s.append(('copy-hdd-hdd', hd1, hd2, 'copy', [], None))
    if usb:
        s += [('copy-ssd-usb', c, usb, 'copy', [], None), ('copy-one-usb', usb, usb, 'copy', [], None)]
    return s


def base_for(drive, work):
    if drive == 'C':
        return os.path.join(work, 'trees')
    if drive == 'NET':
        return '\\\\localhost\\C$' + os.path.join(work, 'trees-net')[2:]
    return drive + ':\\choppy-engine-check'


def one(name, sd, dd, op, extra, kind, who, exe, work):
    src_base = os.path.join(base_for(sd, work), name, who)
    dst_base = os.path.join(base_for(dd, work), name, who)
    src = os.path.join(src_base, 'src')
    dst = os.path.join(dst_base, 'dst') if kind != 'own' else src
    rm(src_base)
    if dst_base != src_base:
        rm(dst_base)
    build_source(src, pair=(kind == 'conflicts'))
    if kind == 'conflicts':
        build_dest(dst, src)
    if kind == 'own':
        # copied into its own folder: "name - Copy", and "(2)" where a "- Copy" is in the way
        write(os.path.join(src, 'x - Copy.bin'), b'in the way', t=600)
        sources = [os.path.join(src, n) for n in ('a.txt', 'x.bin', 'sub1', 'big1.bin')]
    else:
        sources = [os.path.join(src, n) for n in sorted(os.listdir(L(src)))]
    if kind == 'cancel':
        kind = None
        cancel = True
    else:
        cancel = False
    if kind == 'tuner':
        rm(src)
        os.makedirs(L(src))
        with open(L(os.path.join(src, 'huge.bin')), 'wb') as f:
            for i in range(48):
                f.write(rnd(64 << 20, 1000 + i))
        set_times(os.path.join(src, 'huge.bin'), T0 + 77, T0 + 78)
        sources = [os.path.join(src, 'huge.bin')]
        kind = None
    existed = names(dst) if os.path.isdir(L(dst)) else set()
    src_existed = names(src)
    data = os.path.join(work, 'data', name + '-' + who)
    rm(data)
    js = os.path.join(work, 'out', '%s.%s.json' % (name, who))
    log = os.path.join(work, 'out', '%s.%s.log' % (name, who))
    extra = [log if a == '{LOG}' else a for a in extra]
    code, secs = run(exe, data, [op] + sources + [dst, '--json', js] + extra)
    roots = [src_base, dst_base, L(src_base), L(dst_base)]
    if cancel:
        # whatever is there is whole and identical to its source; nothing half written is left
        bad = []
        base = L(dst)
        n = 0
        for d, subs, files in os.walk(base):
            for f in files:
                p = os.path.join(d, f)
                rel = p[len(base) + 1:]
                n += 1
                sp = os.path.join(L(src), rel)
                if f.lower().endswith('.chpart'):
                    bad.append('left: ' + rel)
                elif not os.path.isfile(sp) or info(p)[3] != info(sp)[3] or digest(p) != digest(sp):
                    bad.append('not whole: ' + rel)
        return ['exit %d' % code, 'files there whole, no .chpart: %s' % ('yes' if not bad else bad)], secs, (src_base, dst_base, data)
    out = ['exit %d' % code, '== dest'] + dump(dst, existed)
    if op == 'move' or kind == 'own':
        # the source's folders existed before: Windows dates them when entries leave
        out += ['== source'] + (dump(src, src_existed) if os.path.isdir(L(src)) else ['(gone)'])
    out += ['== json'] + masked_json(js, roots) + ['== log'] + masked_log(data, roots)
    if os.path.isfile(log):
        # one line per 50 ms: the numbers and the current file come from timing, the format must not
        shape = re.compile(r"^\d+\.\ds (Queued|Analyzing|Copying|Waiting|Finishing|Done|Canceled|Failed) \d+/\d+ files "
                           r"-?\d+/\d+ bytes \d+\.\d Mbps workers=\d+ badge='[^']*' cur=.* status=.*$")
        raw = open(log, 'rb').read()
        lines = raw[3:].decode('utf-8').split('\r\n')
        bad = [l for l in lines[:-1] if not shape.match(l)]
        out += ['== progress log', 'bom %s, lines end CR LF %s, all lines in the format: %s' % (
            raw[:3] == b'\xef\xbb\xbf', lines[-1] == '', 'yes' if lines[:-1] and not bad else bad[:3] or 'no lines')]
    return out, secs, (src_base, dst_base, data)


def usage(cs, rs, work):
    # too few arguments: the usage line on stderr, exit 1 (the same in both)
    r = []
    for exe in (cs, rs):
        p = subprocess.run([exe, '--cli', 'copy', 'only-one'], env=dict(os.environ, CHOPPY_DATA=os.path.join(work, 'data', 'usage')),
                           capture_output=True, timeout=60)
        r.append((p.returncode, p.stderr))
    print(('SAME' if r[0] == r[1] else 'DIFF') + ' usage            exit %d, stderr %r' % r[1])
    return r[0] == r[1]


def main():
    cs, rs = sys.argv[1], sys.argv[2]
    only = sys.argv[3:]
    work = os.environ.get('WORK') or os.path.join(os.environ['TEMP'], 'choppy-engine')
    drives = [d for d in os.environ.get('DRIVES', 'D X F').split() if os.path.isdir(d + ':\\')]
    os.makedirs(os.path.join(work, 'out'), exist_ok=True)
    bad = 0
    if not only or 'usage' in only:
        bad += 0 if usage(cs, rs, work) else 1
    cleanup = set()
    try:
        for sc in scenarios(drives):
            if only and sc[0] not in only:
                continue
            res = {}
            for who, exe in (('cs', cs), ('rs', rs)):
                out, secs, dirs = one(*sc, who, exe, work)
                res[who] = (out, secs)
                cleanup.update(dirs)
                open(os.path.join(work, 'out', '%s.%s.txt' % (sc[0], who)), 'w', encoding='utf-8').write('\n'.join(out) + '\n')
            a, b = res['cs'][0], res['rs'][0]
            n_entries = sum(1 for x in a if x[:2] in ('F ', 'D '))
            if a == b:
                print('SAME %-16s %4d entries  (C# %.1f s, Rust %.1f s)' % (sc[0], n_entries, res['cs'][1], res['rs'][1]))
            else:
                bad += 1
                print('DIFF %s' % sc[0])
                for l in list(difflib.unified_diff(a, b, 'cs', 'rs', n=0, lineterm=''))[:40]:
                    print('   ' + l)
            sys.stdout.flush()
            for d in sorted(cleanup):
                rm(d)
            cleanup.clear()
    finally:
        for d in sorted(cleanup):
            rm(d)
        for drive in drives + ['C', 'NET']:
            b = base_for(drive, work)
            if os.path.isdir(L(b)) and not os.listdir(L(b)):
                os.rmdir(L(b))
    print('ALL SAME' if bad == 0 else '%d DIFF' % bad)
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
