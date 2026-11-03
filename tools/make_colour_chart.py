#!/usr/bin/env python3
"""Generate the colour-chart PNG (360 dpi) and its patch layout for the phase-3 Windows capture session.

usage: make_colour_chart.py OUT_DIR [brand_colors.csv]

brand_colors.csv: lines "name,#RRGGBB" (optional; added as extra patches).
Layout: patches of PATCH px on a PITCH grid starting at (MARGIN, MARGIN), row-major, 40 per row.
"""
import json, os, struct, sys, zlib

PATCH, PITCH, MARGIN, COLS = 30, 36, 18, 40
# 1470 px: the last patch column ends at 1452, plus the right margin; inside the 1475-dot printable
# width. (The session-2 chart used 1440, so its rows were too long: see testdata/colour/README.)
W = 2 * MARGIN + (COLS - 1) * PITCH + PATCH
assert W <= 1475

def patches(brand):
    out = []
    lv = [round(i * 255 / 8) for i in range(9)]
    for r in lv:
        for g in lv:
            for b in lv:
                out.append(("cube", (r, g, b)))
    for v in range(0, 256, 5):
        out.append(("grey", (v, v, v)))
    for i in range(1, 33):
        v = 255 - min(255, i * 8)
        out += [("ramp_c", (v, 255, 255)), ("ramp_m", (255, v, 255)), ("ramp_y", (255, 255, v))]
    for name, rgb in brand:
        out.append(("brand:" + name, rgb))
    return out

def png(path, w, h, rows):
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    assert len(rows) == h and all(len(r) == w * 3 for r in rows), "row length"
    raw = b"".join(b"\x00" + r for r in rows)
    ppm = round(360 / 0.0254)
    data = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
            + chunk(b"pHYs", struct.pack(">IIB", ppm, ppm, 1)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))
    open(path, "wb").write(data)

def main():
    out = sys.argv[1]
    brand = []
    if len(sys.argv) > 2:
        for line in open(sys.argv[2]):
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            name, hexrgb = [s.strip() for s in line.split(",", 1)]
            h = hexrgb.lstrip("#")
            brand.append((name, (int(h[0:2], 16), int(h[2:4], 16), int(h[4:6], 16))))
    ps = patches(brand)
    rows_needed = (len(ps) + COLS - 1) // COLS
    H = 2 * MARGIN + rows_needed * PITCH
    assert H <= 2400, "too many patches for one 108 x 174.8 mm label"
    img = [bytearray(b"\xff" * (W * 3)) for _ in range(H)]
    assert MARGIN + (COLS - 1) * PITCH + PATCH <= W
    layout = {"dpi": 360, "patch": PATCH, "pitch": PITCH, "margin": MARGIN, "cols": COLS, "width": W, "height": H, "patches": []}
    for i, (kind, rgb) in enumerate(ps):
        x0, y0 = MARGIN + (i % COLS) * PITCH, MARGIN + (i // COLS) * PITCH
        for y in range(y0, y0 + PATCH):
            img[y][x0 * 3:(x0 + PATCH) * 3] = bytes(rgb) * PATCH
        layout["patches"].append({"kind": kind, "rgb": list(rgb), "x": x0, "y": y0})
    os.makedirs(out, exist_ok=True)
    png(os.path.join(out, "c1_colour_chart.png"), W, H, [bytes(r) for r in img])
    json.dump(layout, open(os.path.join(out, "c1_colour_chart.layout.json"), "w"), indent=0)
    print(f"{len(ps)} patches, {rows_needed} rows")

if __name__ == "__main__":
    main()
