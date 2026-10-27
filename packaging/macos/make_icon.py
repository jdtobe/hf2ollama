#!/usr/bin/env python3
"""Draws the hf2ollama app icon with no third-party dependencies.

Run from the repository root:

    python3 packaging/macos/make_icon.py

It writes:

    crates/hf2ollama-gui/assets/icon-1024.png   bundle/DMG icon source
    crates/hf2ollama-gui/assets/icon-256.png    window icon used at runtime

The artwork is deliberately simple: a teal rounded tile with a download arrow
into a tray, matching the app's accent colours. It can be replaced later by
swapping the two PNGs (keep the 1024 source square and transparent outside the
rounded tile).
"""

import binascii
import math
import struct
import zlib
from pathlib import Path

SIZE = 1024
SCALE = 2  # supersampling factor
W = SIZE * SCALE

ACCENT_TOP = (0x0B, 0x6E, 0x66)
ACCENT_BOTTOM = (0x3C, 0xC2, 0xB3)
GLYPH = (0xF2, 0xFF, 0xFC)
GLYPH_SHADOW = (0x04, 0x3B, 0x36)


def rounded_rect(x, y, left, top, right, bottom, radius):
    """Signed distance to a rounded rectangle."""
    cx = max(left + radius, min(x, right - radius))
    cy = max(top + radius, min(y, bottom - radius))
    dx = x - cx
    dy = y - cy
    dist = math.hypot(dx, dy) - radius
    if left + radius <= x <= right - radius:
        dist = max(top - y, y - bottom, dist)
    elif top + radius <= y <= bottom - radius:
        dist = max(left - x, x - right, dist)
    return dist


def segment_distance(x, y, ax, ay, bx, by):
    vx, vy = bx - ax, by - ay
    wx, wy = x - ax, y - ay
    t = 0.0 if vx == vy == 0 else max(0.0, min(1.0, (wx * vx + wy * vy) / (vx * vx + vy * vy)))
    return math.hypot(x - (ax + t * vx), y - (ay + t * vy))


def in_arrow(x, y):
    # Shaft and head of a downward arrow, in 1024 coordinates.
    if 470 <= x <= 554 and 300 <= y <= 548:
        return True
    # Triangle head.
    if 548 <= y <= 700:
        half = 148 * (700 - y) / (700 - 548)
        return abs(x - 512) <= half
    return False


def in_tray(x, y):
    # A U-shaped tray with round 30 px caps, stroke 52.
    if y < 650:
        return False
    for ax, ay, bx, by in (
        (332, 650, 332, 744),
        (332, 770, 692, 770),
        (692, 650, 692, 744),
    ):
        if segment_distance(x, y, ax, ay, bx, by) <= 26:
            return True
    return False


def render():
    hi = bytearray(W * W * 4)
    margin = 64
    radius = 220
    for py in range(W):
        y = (py + 0.5) / SCALE
        for px in range(W):
            x = (px + 0.5) / SCALE
            d = rounded_rect(x, y, margin, margin, SIZE - margin, SIZE - margin, radius)
            # One sample per supersampled pixel; the downsample below smooths
            # the edges.
            if d > 0.75:
                continue
            t = (x + y) / (2.0 * SIZE)
            t = min(1.0, max(0.0, t))
            r = int(ACCENT_TOP[0] + (ACCENT_BOTTOM[0] - ACCENT_TOP[0]) * t)
            g = int(ACCENT_TOP[1] + (ACCENT_BOTTOM[1] - ACCENT_TOP[1]) * t)
            b = int(ACCENT_TOP[2] + (ACCENT_BOTTOM[2] - ACCENT_TOP[2]) * t)
            # Soft top highlight.
            lift = max(0.0, 1.0 - y / (SIZE * 0.55)) * 0.16
            r = min(255, int(r + (255 - r) * lift))
            g = min(255, int(g + (255 - g) * lift))
            b = min(255, int(b + (255 - b) * lift))
            # Dark rim just inside the tile edge.
            if d > -6:
                shade = 0.22
                r = int(r * (1 - shade))
                g = int(g * (1 - shade))
                b = int(b * (1 - shade))
            # Glyph shadow, then glyph.
            if in_arrow(x - 8, y - 10) or in_tray(x - 8, y - 10):
                r, g, b = GLYPH_SHADOW
            if in_arrow(x, y) or in_tray(x, y):
                r, g, b = GLYPH
            i = (py * W + px) * 4
            hi[i : i + 4] = bytes((r, g, b, 255))

    # Box-filter down to SIZE. Average in premultiplied space so the
    # transparent corners do not leave a dark fringe.
    out = bytearray(SIZE * SIZE * 4)
    for y in range(SIZE):
        for x in range(SIZE):
            ar = ag = ab = aa = 0
            for sy in range(SCALE):
                row = (y * SCALE + sy) * W
                for sx in range(SCALE):
                    i = (row + x * SCALE + sx) * 4
                    a = hi[i + 3]
                    ar += hi[i] * a
                    ag += hi[i + 1] * a
                    ab += hi[i + 2] * a
                    aa += a
            o = (y * SIZE + x) * 4
            if aa:
                out[o] = ar // aa
                out[o + 1] = ag // aa
                out[o + 2] = ab // aa
                out[o + 3] = aa // (SCALE * SCALE)
    return out


def write_png(path, width, height, rgba):
    raw = bytearray()
    stride = width * 4
    for y in range(height):
        raw.append(0)
        raw.extend(rgba[y * stride : (y + 1) * stride])

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", binascii.crc32(kind + data) & 0xFFFFFFFF)

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    png += chunk(b"IEND", b"")
    path.write_bytes(png)


def downscale(rgba):
    out = bytearray(256 * 256 * 4)
    for y in range(256):
        for x in range(256):
            ar = ag = ab = aa = 0
            for sy in range(4):
                row = (y * 4 + sy) * SIZE
                for sx in range(4):
                    i = (row + x * 4 + sx) * 4
                    a = rgba[i + 3]
                    ar += rgba[i] * a
                    ag += rgba[i + 1] * a
                    ab += rgba[i + 2] * a
                    aa += a
            o = (y * 256 + x) * 4
            if aa:
                out[o] = ar // aa
                out[o + 1] = ag // aa
                out[o + 2] = ab // aa
                out[o + 3] = aa // 16
    return out


def main():
    root = Path(__file__).resolve().parents[2]
    assets = root / "crates" / "hf2ollama-gui" / "assets"
    assets.mkdir(parents=True, exist_ok=True)
    big = render()
    write_png(assets / "icon-1024.png", SIZE, SIZE, big)
    write_png(assets / "icon-256.png", 256, 256, downscale(big))
    print("wrote", assets / "icon-1024.png")
    print("wrote", assets / "icon-256.png")


if __name__ == "__main__":
    main()
