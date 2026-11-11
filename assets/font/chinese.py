"""Bake Simplified Chinese glyphs from the 12px Fusion Pixel Font release.

Run `python assets/font/chinese.py` with Pillow and fonttools[woff] installed.
Pass `--font PATH` to use the release's Simplified Chinese WOFF2 file directly.
"""

from __future__ import annotations

import argparse
import hashlib
import io
import pathlib
import urllib.request
import zipfile

from PIL import Image, ImageDraw, ImageFont
from fontTools.ttLib import TTFont


ROOT = pathlib.Path(__file__).resolve().parents[2]
SOURCE_URL = (
    "https://github.com/TakWolf/fusion-pixel-font/releases/download/2026.10.25/"
    "fusion-pixel-font-12px-monospaced-otf.woff2-v2026.10.25.zip"
)
SOURCE_NAME = "fusion-pixel-12px-monospaced-zh_hans.otf.woff2"
SOURCE_SHA256 = "1cd966a06a28c648f3a47d0147d259d9daa4b01572aac6cef418999f28755ecf"
OUTPUT = ROOT / "assets/font/chinese.bin"
FONT_SIZE = 12
BASELINE = 10
# Remove the empty top row and right column. The layout already adds its own gap.
SIDE = FONT_SIZE - 1
CHARACTERS = ROOT / "assets/font/chinese-characters.txt"


def bake(data: bytes) -> None:
    digest = hashlib.sha256(data).hexdigest()
    if digest != SOURCE_SHA256:
        raise SystemExit(f"unexpected source font SHA-256: {digest}")
    characters = set("".join(CHARACTERS.read_text(encoding="utf-8").splitlines()))
    outlines = io.BytesIO()
    with TTFont(io.BytesIO(data)) as source:
        cmap = source.getBestCmap()
        source.flavor = None
        source.save(outlines)
    outlines.seek(0)
    font = ImageFont.truetype(outlines, FONT_SIZE)
    binary = [0] * 128 + [255] * 128
    output = bytearray()
    for char in sorted(characters):
        codepoint = ord(char)
        if codepoint not in cmap:
            raise SystemExit(f"source font has no glyph for {char} (U+{codepoint:04X})")
        image = Image.new("L", (FONT_SIZE, FONT_SIZE), 0)
        ImageDraw.Draw(image).text((0, BASELINE), char, font=font, fill=255, anchor="ls")
        image = image.point(binary)
        bounds = image.getbbox()
        if bounds is None or bounds[1] < 1 or bounds[2] > SIDE:
            raise SystemExit(f"glyph {char} (U+{codepoint:04X}) does not fit its {SIDE} px cell")
        image = image.crop((0, 1, SIDE, FONT_SIZE))

        # Server-rendered cards still use the LCD grid.
        lcd = image.resize((7, 7), Image.Resampling.BOX)
        threshold = min(80, lcd.getextrema()[1])
        columns = [
            sum((1 << y) for y in range(7) if lcd.getpixel((x, y)) >= threshold)
            for x in range(7)
        ]
        output.extend(codepoint.to_bytes(4, "little"))
        output.extend(columns)
        output.extend(image.tobytes())

    OUTPUT.write_bytes(output)
    print(f"wrote {len(characters)} glyphs ({len(output)} bytes) to {OUTPUT.relative_to(ROOT)}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--font", type=pathlib.Path)
    args = parser.parse_args()
    if args.font:
        bake(args.font.read_bytes())
    else:
        with urllib.request.urlopen(SOURCE_URL, timeout=120) as response:
            data = response.read()
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            entry = next(
                name for name in archive.namelist() if pathlib.PurePosixPath(name).name == SOURCE_NAME
            )
            bake(archive.read(entry))


if __name__ == "__main__":
    main()
