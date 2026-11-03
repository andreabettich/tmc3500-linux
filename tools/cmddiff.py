#!/usr/bin/env python3
"""For each capture, print command (non-raster) differences vs its reference capture (from NOTES.md)."""
import sys, os, re, hashlib
sys.path.insert(0, os.path.dirname(__file__))
from escpdump import parse, fmt
D = sys.argv[1]
def sig(name):
    b = open(os.path.join(D, name + '.prn'), 'rb').read()
    cmds = []; rast = []
    for o, n, p in parse(b):
        if n == 'ESC i':
            rast.append((p['color'], p['bytes'], p['lines'], hashlib.md5(p['data']).hexdigest()[:6]))
            cmds.append('ESC i c=%d b=%d l=%d' % (p['color'], p['bytes'], p['lines']))
        elif n in ('DATA',) and not any(p): continue
        elif n in ('RM TI', 'RM JH'): cmds.append(n)
        else: cmds.append(n + ' ' + (fmt(p) if not isinstance(p, dict) else ''))
    return cmds, rast
notes = open(os.path.join(D, 'NOTES.md'), encoding='utf-8-sig').read()
refs = dict(re.findall(r'^\| (\d{3}_[^ |]+) \|.*?vs (\d{3}_[^ :;]+)[:;]', notes, re.M))
import difflib
for f in sorted(x[:-4] for x in os.listdir(D) if x.endswith('.prn')):
    r = refs.get(f, '000_default_t1' if f[0] in '0' else None)
    if not r or r == f: continue
    a, ra = sig(r); b, rb = sig(f)
    # collapse identical consecutive
    d = [l for l in difflib.unified_diff(a, b, lineterm='', n=0) if l[:1] in '+-' and l[:3] not in ('+++', '---')]
    rd = 'raster same' if ra == rb else 'raster differs (%d vs %d blocks)' % (len(ra), len(rb))
    print('### %s  (ref %s)  %s' % (f, r, rd))
    for l in d[:14]: print('   ', l[:150])
    if len(d) > 14: print('    ... %d more' % (len(d) - 14))
