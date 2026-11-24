#!/usr/bin/env python3
"""Turn the raw RGBA written by `ROBROZ_SHOT` into a PNG (stdlib only).

Usage: rgba2png.py <in.rgba> <width> <height> <out.png>
"""
import struct
import sys
import zlib


def chunk(kind: bytes, data: bytes) -> bytes:
    body = kind + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)


def main() -> None:
    src, w, h, dst = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
    raw = open(src, "rb").read()
    stride = w * 4
    if len(raw) != stride * h:
        sys.exit(f"{src}: {len(raw)} bytes, expected {stride * h} for {w}x{h}")
    rows = b"".join(b"\x00" + raw[y * stride:(y + 1) * stride] for y in range(h))
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(rows, 9))
    png += chunk(b"IEND", b"")
    open(dst, "wb").write(png)


if __name__ == "__main__":
    main()
