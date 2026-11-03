#!/usr/bin/env python3
"""Decode an Epson ESC/P2 (+ remote mode) printer stream, as produced by the TM-C3500 driver.

usage: escpdump.py file.prn [--raster] [--max N]
"""
import sys, struct

def u16(b, o): return struct.unpack_from('<H', b, o)[0]
def u32(b, o): return struct.unpack_from('<I', b, o)[0]

def remote(b, o, out):
    """parse REMOTE1 block starting at o (after 'REMOTE1'), return new offset"""
    while o < len(b):
        if b[o] == 0x1b and b[o+1:o+4] == b'\0\0\0':
            out.append((o, 'REMOTE_EXIT', b'')); return o + 4
        cmd = b[o:o+2].decode('latin1'); n = u16(b, o + 2)
        out.append((o, 'RM ' + cmd, b[o+4:o+4+n])); o += 4 + n
    return o

def unpack_rle(data, nbytes):
    """TIFF/packbits-style Epson RLE. returns (decoded, consumed)"""
    res = bytearray(); i = 0
    while len(res) < nbytes and i < len(data):
        c = data[i]; i += 1
        if c < 128:
            res += data[i:i+c+1]; i += c + 1
        else:
            res += bytes([data[i]]) * (257 - c); i += 1
    return bytes(res), i

def parse(b):
    out = []; o = 0
    while o < len(b):
        if b[o] != 0x1b:
            s = o
            while o < len(b) and b[o] != 0x1b: o += 1
            out.append((s, 'DATA', b[s:o])); continue
        c = b[o+1]
        if c == 0x01:  # ESC 01 @EJL ...  until ESC
            e = b.index(b'\x1b', o + 2); out.append((o, 'EJL', b[o+2:e])); o = e
        elif c == ord('@'): out.append((o, 'ESC @ (init)', b'')); o += 2
        elif c == 0 and b[o+2:o+4] == b'\0\0':
            out.append((o, 'ESC 000 (exit)', b'')); o += 4
        elif c == ord('('):
            k = chr(b[o+2]); n = u16(b, o + 3); p = b[o+5:o+5+n]
            if k == 'R' and p[1:] == b'REMOTE1':
                out.append((o, 'ESC(R REMOTE1', b'')); o = remote(b, o + 5 + n, out)
            else:
                out.append((o, 'ESC(' + k, p)); o += 5 + n
        elif c == ord('i'):  # ESC i c comp bits bytes(u16) lines(u16)
            col, comp, bits = b[o+2], b[o+3], b[o+4]
            nb, nl = u16(b, o + 5), u16(b, o + 7)
            total = nb * nl
            if comp == 1:
                dec, used = unpack_rle(b[o+9:], total)
            else:
                dec, used = b[o+9:o+9+total], total
            out.append((o, 'ESC i', dict(color=col, comp=comp, bits=bits, bytes=nb, lines=nl, raw=used, data=dec)))
            o += 9 + used
        elif c in (ord('U'), ord('r'), ord('+')):
            out.append((o, 'ESC ' + chr(c), b[o+2:o+3])); o += 3
        elif c == ord('\\'):
            out.append((o, 'ESC \\ (rel horiz)', b[o+2:o+4])); o += 4
        else:
            out.append((o, 'ESC ?%02x' % c, b'')); o += 2
    return out

def fmt(p):
    if isinstance(p, dict):
        d = p['data']; ink = sum(bin(x).count('1') for x in d)
        return 'c=%d comp=%d bits=%d bytes/line=%d lines=%d enc=%d set_bits=%d' % (
            p['color'], p['comp'], p['bits'], p['bytes'], p['lines'], p['raw'], ink)
    if len(p) > 48: return p[:48].hex(' ') + ' ... (%d B)' % len(p)
    return p.hex(' ')

if __name__ == '__main__':
    b = open(sys.argv[1], 'rb').read()
    mx = int(sys.argv[sys.argv.index('--max') + 1]) if '--max' in sys.argv else 10**9
    for i, (o, name, p) in enumerate(parse(b)):
        if i >= mx: break
        if name == 'DATA' and not any(p): name = 'NULs'; p = b'' if len(p) < 4 else ('%d zero bytes' % len(p)).encode()
        print('%06x %-16s %s' % (o, name, p.decode() if name == 'NULs' else fmt(p)))
