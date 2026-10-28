"""Builds misaki.bin, the Japanese part of the 5x7 font (src/font.rs), from Misaki Gothic 2nd
(https://littlelimit.net/misaki.htm, free to use, change and pass on; see THIRD-PARTY.md):

    python3 misaki.py misaki_gothic_2nd.bdf misaki.bin

Every glyph the built-in font doesn't already have, in codepoint order, 12 bytes each: the
codepoint (u32, little endian), how many columns wide it is (7, or 3 for halfwidth kana), then
7 columns, bit 0 the top row (like the built-in glyphs). Misaki's glyphs are 7x7 on an 8x8 cell:
the descender row and the spacing column are left out.
"""

import struct
import sys

# Already drawn by src/font.rs: ASCII, Cyrillic, and marks shown as their ASCII look-alikes.
OWN = set(range(0x80)) | set(range(0x410, 0x450)) | {0x401, 0x451, 0xA0} | set(map(ord, '«»“”„‘’–—−…№'))


def glyphs(path):
    enc = bits = bbx = dwidth = None
    for line in open(path, encoding='latin-1'):
        p = line.split()
        if not p:
            continue
        if p[0] == 'ENCODING':
            enc = int(p[1])
        elif p[0] == 'DWIDTH':
            dwidth = int(p[1])
        elif p[0] == 'BBX':
            bbx = list(map(int, p[1:]))
        elif p[0] == 'BITMAP':
            bits = []
        elif p[0] == 'ENDCHAR':
            yield enc, dwidth, bbx, bits
            bits = None
        elif bits is not None:
            bits.append(int(p[0], 16))


def main(src, out):
    rows = []
    for enc, dwidth, (w, h, xoff, yoff), bits in glyphs(src):
        if enc < 0 or enc in OWN:
            continue
        width = 7 if dwidth >= 8 else 3
        cols = [0] * 7
        top = 7 - (yoff + h)
        for i, row in enumerate(bits):
            r = top + i
            if not 0 <= r < 7:
                continue
            for c in range(w):
                x = xoff + c
                if x < width and row & (0x80 >> c):
                    cols[x] |= 1 << r
        rows.append((enc, struct.pack("<IB7B", enc, width, *cols)))
    rows.sort()
    with open(out, 'wb') as f:
        f.write(b"".join(r for _, r in rows))
    print(f'{len(rows)} glyphs')


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
