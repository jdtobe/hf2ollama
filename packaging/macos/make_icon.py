#!/usr/bin/env python3
"""Rasterize the canonical Roost tail SVG without third-party dependencies.

Run from the repository root with `python3 packaging/macos/make_icon.py`.
The geometry and colors below are direct copies of docs/design/roost-tail.svg.
"""
import binascii
import math
import struct
import zlib
from pathlib import Path

SIZE = 1024
SCALE = 2
CANVAS = 96
BACKGROUND = (0x10, 0x20, 0x1D)
HUB = (0xF7, 0xF3, 0xE8)
PATHS = (
    ((26, 70, 40, 41, 58, 27, 79, 23), (0x3C, 0xC2, 0xB3)),
    ((26, 70, 42, 47, 61, 37, 82, 37), (0x8B, 0xE0, 0xD2)),
    ((26, 70, 44, 54, 64, 50, 82, 51), (0xF4, 0xB8, 0x60)),
    ((26, 70, 46, 61, 66, 62, 80, 67), (0xF1, 0x7C, 0x67)),
)


def cubic(p, a, b, c, d):
    q = 1 - p
    return (
        q**3 * a[0] + 3 * q * q * p * b[0] + 3 * q * p * p * c[0] + p**3 * d[0],
        q**3 * a[1] + 3 * q * q * p * b[1] + 3 * q * p * p * c[1] + p**3 * d[1],
    )


def segment_distance(x, y, a, b):
    vx, vy = b[0] - a[0], b[1] - a[1]
    t = max(0.0, min(1.0, ((x - a[0]) * vx + (y - a[1]) * vy) / (vx * vx + vy * vy)))
    return math.hypot(x - (a[0] + t * vx), y - (a[1] + t * vy))


def rounded_rect_distance(x, y):
    # SVG: rect width=96 height=96 rx=24.
    cx, cy = max(24, min(x, 72)), max(24, min(y, 72))
    return math.hypot(x - cx, y - cy) - 24


def render(size):
    pixels = bytearray(size * size * 4)
    pixels[:] = bytes((*BACKGROUND, 255)) * (size * size)
    def disk(cx, cy, radius, color):
        lo_x, hi_x = max(0, int(cx - radius - 1)), min(size, int(cx + radius + 2))
        lo_y, hi_y = max(0, int(cy - radius - 1)), min(size, int(cy + radius + 2))
        rr = radius * radius
        for py in range(lo_y, hi_y):
            for px in range(lo_x, hi_x):
                if (px + 0.5 - cx) ** 2 + (py + 0.5 - cy) ** 2 <= rr:
                    i = (py * size + px) * 4
                    pixels[i:i + 4] = bytes((*color, 255))
    scale = size / CANVAS
    # Draw a sampled cubic stroke. The samples are points on the exact SVG
    # cubics; their overlap produces the SVG's 7px round-cap strokes.
    for path, color in PATHS:
        a, b, c, d = (path[0:2], path[2:4], path[4:6], path[6:8])
        for step in range(size + 1):
            x, y = cubic(step / size, a, b, c, d)
            disk(x * scale, y * scale, 3.5 * scale, color)
    disk(26 * scale, 70 * scale, 6 * scale, HUB)
    return pixels


def write_png(path, width, height, rgba):
    raw = bytearray()
    stride = width * 4
    for y in range(height):
        raw.append(0)
        raw.extend(rgba[y * stride:(y + 1) * stride])
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", binascii.crc32(kind + data) & 0xffffffff)
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b""))


def downscale(source):
    out = bytearray(256 * 256 * 4)
    for y in range(256):
        for x in range(256):
            colors = [source[((y * 4 + sy) * SIZE + x * 4 + sx) * 4:((y * 4 + sy) * SIZE + x * 4 + sx) * 4 + 4] for sy in range(4) for sx in range(4)]
            i = (y * 256 + x) * 4
            out[i:i + 4] = bytes(sum(c[n] for c in colors) // 16 for n in range(4))
    return out


def main():
    root = Path(__file__).resolve().parents[2]
    assets = root / "crates" / "roost-gui" / "assets"
    big = render(SIZE)
    write_png(assets / "icon-1024.png", SIZE, SIZE, big)
    write_png(assets / "icon-256.png", 256, 256, downscale(big))
    print("wrote", assets / "icon-1024.png")
    print("wrote", assets / "icon-256.png")

if __name__ == "__main__":
    main()
