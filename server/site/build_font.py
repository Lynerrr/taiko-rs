"""Builds taiko5x7.woff2, the website's font, from the game's own 5x7 LCD font (src/font.rs).

    python3 server/site/build_font.py

Each font pixel is a square of 125 units (1000 per em, so 8 pixels an em): at 16px the glyphs
are drawn with 2px pixels, at 24px with 3px, and so on.
"""

import pathlib
import re

from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

HERE = pathlib.Path(__file__).resolve().parent
SOURCE = HERE.parent.parent / "src" / "font.rs"
PX = 125
ADVANCE = 6 * PX


def tables(text):
    """The GLYPHS and CYRILLIC tables, as lists of five column bytes."""
    out = {}
    for name in ("GLYPHS", "CYRILLIC"):
        body = re.search(r"const %s: \[\[u8; 5\]; \d+\] = \[(.*?)\n\];" % name, text, re.S).group(1)
        out[name] = [[int(b, 16) for b in row] for row in re.findall(r"\[(0x[0-9A-Fa-f]{2}(?:, 0x[0-9A-Fa-f]{2}){4})\]", body) for row in [row.split(", ")]]
    return out["GLYPHS"], out["CYRILLIC"]


def draw(columns):
    pen = TTGlyphPen(None)
    for x, col in enumerate(columns):
        for row in range(7):
            if col >> row & 1:
                # Row 0 is the top; the bottom row sits on the baseline.
                x0, y0 = x * PX, (6 - row) * PX
                pen.moveTo((x0, y0))
                pen.lineTo((x0, y0 + PX))
                pen.lineTo((x0 + PX, y0 + PX))
                pen.lineTo((x0 + PX, y0))
                pen.closePath()
    return pen.glyph()


def main():
    ascii_, cyrillic = tables(SOURCE.read_text(encoding="utf-8"))
    assert len(ascii_) == 95 and len(cyrillic) == 66, (len(ascii_), len(cyrillic))
    chars = [chr(c) for c in range(32, 127)] + [chr(c) for c in range(0x410, 0x450)] + ["Ё", "ё"]
    names = [".notdef"] + ["g%d" % i for i in range(len(chars))]
    glyphs = {".notdef": draw([0x7F, 0x41, 0x41, 0x41, 0x7F])}
    cmap = {}
    for i, (c, cols) in enumerate(zip(chars, ascii_ + cyrillic)):
        glyphs["g%d" % i] = draw(cols)
        cmap[ord(c)] = "g%d" % i
    # Typographic marks drawn as their plain look-alikes, as the game does; but an ellipsis
    # (what the browser cuts long names with) gets its three dots.
    for mark, plain in {"«": '"', "»": '"', "“": '"', "”": '"', "„": '"', "‘": "'", "’": "'", "–": "-", "—": "-", "−": "-", "№": "#", " ": " ", "·": "."}.items():
        cmap[ord(mark)] = cmap[ord(plain)]
    names.append("ellipsis")
    glyphs["ellipsis"] = draw([0x40, 0x00, 0x40, 0x00, 0x40])
    cmap[0x2026] = "ellipsis"

    fb = FontBuilder(1000, isTTF=True)
    fb.setupGlyphOrder(names)
    fb.setupCharacterMap(cmap)
    fb.setupGlyf(glyphs)
    fb.setupHorizontalMetrics({n: (ADVANCE, 0) for n in names})
    fb.setupHorizontalHeader(ascent=1000, descent=-125)
    fb.setupNameTable({"familyName": "taiko5x7", "styleName": "Regular"})
    fb.setupOS2(sTypoAscender=1000, sTypoDescender=-125, sTypoLineGap=0, usWinAscent=1000, usWinDescent=125)
    fb.setupPost()
    fb.font.flavor = "woff2"
    fb.save(HERE / "taiko5x7.woff2")


if __name__ == "__main__":
    main()
