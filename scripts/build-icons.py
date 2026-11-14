"""Finish the app icon set after `npx tauri icon`.

`tauri icon` shrinks the big artwork for every size, which blurs the
grid at 16-24 px and leaves out the 20 and 40 px sizes Windows asks
for at 125% and 250% display scaling. This script:

  * rebuilds src-tauri/icons/icon.ico with 16, 20, 24, 32, 40, 48, 64
    and 256 px frames, taking 16/20/24 from the hand-drawn,
    pixel-aligned files in assets/icon-small/;
  * puts the hand-drawn 16 px frame into icon.icns (is32 + s8mk).

Usage (needs Python 3 with Pillow):
  python scripts/build-icons.py <icon-1024.png>
"""
import struct
import sys
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "src-tauri" / "icons"
SMALL = ROOT / "assets" / "icon-small"
ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 256]


def rle(channel: bytes) -> bytes:
    """Apple icns run-length encoding for one 8-bit channel."""
    out = bytearray()
    lit = bytearray()

    def flush():
        nonlocal lit
        while lit:
            out.append(len(lit[:128]) - 1)
            out.extend(lit[:128])
            lit = lit[128:]

    i = 0
    while i < len(channel):
        j = i
        while j < len(channel) and channel[j] == channel[i] and j - i < 130:
            j += 1
        if j - i >= 3:
            flush()
            out.append(j - i + 125)
            out.append(channel[i])
            i = j
        else:
            lit.append(channel[i])
            i += 1
    flush()
    return bytes(out)


def build_ico(big: Image.Image) -> None:
    frames = []
    for n in ICO_SIZES:
        hand = SMALL / f"{n}.png"
        if hand.exists():
            frames.append(Image.open(hand).convert("RGBA"))
        else:
            frames.append(big.resize((n, n), Image.LANCZOS))
    frames[-1].save(
        ICONS / "icon.ico",
        format="ICO",
        sizes=[(n, n) for n in ICO_SIZES],
        append_images=frames[:-1],
    )


def patch_icns() -> None:
    im = Image.open(SMALL / "16.png").convert("RGBA")
    r, g, b, a = (c.tobytes() for c in im.split())
    new = {b"is32": rle(r) + rle(g) + rle(b), b"s8mk": a}
    path = ICONS / "icon.icns"
    data = path.read_bytes()
    parts, seen, i = [], set(), 8
    while i < len(data):
        kind, length = struct.unpack(">4sI", data[i : i + 8])
        body = new.get(kind, data[i + 8 : i + length])
        seen.add(kind)
        parts.append(struct.pack(">4sI", kind, len(body) + 8) + body)
        i += length
    for kind in (b"is32", b"s8mk"):
        if kind not in seen:
            parts.append(struct.pack(">4sI", kind, len(new[kind]) + 8) + new[kind])
    body = b"".join(parts)
    path.write_bytes(b"icns" + struct.pack(">I", len(body) + 8) + body)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    big = Image.open(sys.argv[1]).convert("RGBA")
    build_ico(big)
    patch_icns()
    print("icon.ico and icon.icns updated")
