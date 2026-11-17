#!/usr/bin/env python3
"""Generate the application and tray icons.

Plain Python on purpose: the project has no npm/bundler step, so an icon
pipeline that needs Pillow or Inkscape would be one more thing to install.
The icon is drawn at 4x and box-filtered down, which gives clean edges at
16-32 px.

Usage: python3 scripts/generate-icons.py
"""

import os
import struct
import zlib

OUT_DIR = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "src-tauri", "icons")
SUPERSAMPLE = 4

BACKGROUND_TOP = (43, 50, 65, 255)
BACKGROUND_BOTTOM = (19, 23, 31, 255)
INK = (245, 247, 250, 255)
ACCENT = (224, 82, 95, 255)


def blend(dst, offset, color, coverage):
    if coverage <= 0:
        return
    if coverage > 1:
        coverage = 1
    sr, sg, sb, sa = color
    da = dst[offset + 3] / 255.0
    a = coverage
    out_a = a + da * (1 - a)
    if out_a <= 0:
        dst[offset:offset + 4] = b"\x00\x00\x00\x00"
        return
    for channel, src in enumerate((sr, sg, sb)):
        d = dst[offset + channel] / 255.0
        value = (src / 255.0) * a + d * da * (1 - a)
        dst[offset + channel] = max(0, min(255, round(value / out_a * 255)))


def fill_rect(px, n, x0, y0, x1, y1, color):
    x0 = max(0, int(round(x0)))
    y0 = max(0, int(round(y0)))
    x1 = min(n, int(round(x1)))
    y1 = min(n, int(round(y1)))
    for y in range(y0, y1):
        row = y * n * 4
        for x in range(x0, x1):
            blend(px, row + x * 4, color, 1.0)


def fill_round_rect(px, n, x0, y0, x1, y1, radius, color):
    """Filled rounded rectangle sampled at pixel centres (no AA needed at 4x)."""
    for y in range(max(0, int(y0)), min(n, int(y1))):
        cy = y + 0.5
        for x in range(max(0, int(x0)), min(n, int(x1))):
            cx = x + 0.5
            dx = max(x0 + radius - cx, 0, cx - (x1 - radius))
            dy = max(y0 + radius - cy, 0, cy - (y1 - radius))
            if dx * dx + dy * dy <= radius * radius:
                blend(px, (y * n + x) * 4, color, 1.0)


def fill_gradient_round_rect(px, n, x0, y0, x1, y1, radius, top, bottom):
    for y in range(max(0, int(y0)), min(n, int(y1))):
        t = (y - y0) / max(1.0, (y1 - y0))
        color = tuple(round(top[i] + (bottom[i] - top[i]) * t) for i in range(4))
        cy = y + 0.5
        for x in range(max(0, int(x0)), min(n, int(x1))):
            cx = x + 0.5
            dx = max(x0 + radius - cx, 0, cx - (x1 - radius))
            dy = max(y0 + radius - cy, 0, cy - (y1 - radius))
            if dx * dx + dy * dy <= radius * radius:
                blend(px, (y * n + x) * 4, color, 1.0)


def fill_circle(px, n, cx, cy, radius, color):
    for y in range(max(0, int(cy - radius - 1)), min(n, int(cy + radius + 1))):
        for x in range(max(0, int(cx - radius - 1)), min(n, int(cx + radius + 1))):
            dx = x + 0.5 - cx
            dy = y + 0.5 - cy
            if dx * dx + dy * dy <= radius * radius:
                blend(px, (y * n + x) * 4, color, 1.0)


def draw_icon(size):
    n = size * SUPERSAMPLE
    px = bytearray(n * n * 4)

    unit = n / 100.0
    margin = 2.0 * unit
    fill_gradient_round_rect(
        px, n,
        margin, margin, n - margin, n - margin,
        22.0 * unit,
        BACKGROUND_TOP, BACKGROUND_BOTTOM,
    )

    # Four corner brackets, like a screen-region selection.
    inset = 26.0 * unit
    arm = 17.0 * unit
    weight = 6.2 * unit
    left, top = inset, inset
    right, bottom = n - inset, n - inset

    fill_round_rect(px, n, left, top, left + arm, top + weight, weight / 2, INK)
    fill_round_rect(px, n, left, top, left + weight, top + arm, weight / 2, INK)
    fill_round_rect(px, n, right - arm, top, right, top + weight, weight / 2, INK)
    fill_round_rect(px, n, right - weight, top, right, top + arm, weight / 2, INK)
    fill_round_rect(px, n, left, bottom - weight, left + arm, bottom, weight / 2, INK)
    fill_round_rect(px, n, left, bottom - arm, left + weight, bottom, weight / 2, INK)
    fill_round_rect(px, n, right - arm, bottom - weight, right, bottom, weight / 2, INK)
    fill_round_rect(px, n, right - weight, bottom - arm, right, bottom, weight / 2, INK)

    # Recording dot in the lower-right of the frame.
    fill_circle(px, n, n - 33.0 * unit, n - 33.0 * unit, 6.4 * unit, ACCENT)

    # Downsample with a box filter.
    out = bytearray(size * size * 4)
    factor = SUPERSAMPLE * SUPERSAMPLE
    for y in range(size):
        for x in range(size):
            r = g = b = a = 0
            for sy in range(SUPERSAMPLE):
                base = ((y * SUPERSAMPLE + sy) * n + x * SUPERSAMPLE) * 4
                for sx in range(SUPERSAMPLE):
                    r += px[base + sx * 4]
                    g += px[base + sx * 4 + 1]
                    b += px[base + sx * 4 + 2]
                    a += px[base + sx * 4 + 3]
            offset = (y * size + x) * 4
            out[offset] = round(r / factor)
            out[offset + 1] = round(g / factor)
            out[offset + 2] = round(b / factor)
            out[offset + 3] = round(a / factor)
    return out


def write_png(path, size, rgba):
    def chunk(tag, data):
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    raw = bytearray()
    stride = size * 4
    for y in range(size):
        raw.append(0)
        raw.extend(rgba[y * stride:(y + 1) * stride])
    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    payload = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )
    with open(path, "wb") as handle:
        handle.write(payload)
    return len(payload)


def draw_tray_icon(size):
    """Draw a crisp camera glyph on a transparent background for the tray."""
    n = size * SUPERSAMPLE
    px = bytearray(n * n * 4)

    unit = n / 100.0

    WHITE = (248, 248, 242, 255)
    DARK = (40, 42, 54, 255)
    PURPLE = (189, 147, 249, 255)

    # Top viewfinder/shutter bump
    fill_round_rect(px, n, 33.0 * unit, 14.0 * unit, 51.0 * unit, 26.0 * unit, 3.5 * unit, WHITE)
    # Flash indicator
    fill_circle(px, n, 74.0 * unit, 36.0 * unit, 4.5 * unit, PURPLE)

    # Camera body with rounded corners
    fill_round_rect(px, n, 10.0 * unit, 24.0 * unit, 90.0 * unit, 84.0 * unit, 14.0 * unit, WHITE)

    # Outer lens housing (dark cutout)
    fill_circle(px, n, 50.0 * unit, 54.0 * unit, 22.0 * unit, DARK)

    # Lens aperture ring (Dracula purple)
    fill_circle(px, n, 50.0 * unit, 54.0 * unit, 17.5 * unit, PURPLE)

    # Center lens aperture (dark)
    fill_circle(px, n, 50.0 * unit, 54.0 * unit, 13.0 * unit, DARK)

    # Glass glint highlight (white)
    fill_circle(px, n, 46.0 * unit, 50.0 * unit, 3.5 * unit, (255, 255, 255, 255))

    # Downsample with box filter
    out = bytearray(size * size * 4)
    factor = SUPERSAMPLE * SUPERSAMPLE
    for y in range(size):
        for x in range(size):
            r = g = b = a = 0
            for sy in range(SUPERSAMPLE):
                base = ((y * SUPERSAMPLE + sy) * n + x * SUPERSAMPLE) * 4
                for sx in range(SUPERSAMPLE):
                    r += px[base + sx * 4]
                    g += px[base + sx * 4 + 1]
                    b += px[base + sx * 4 + 2]
                    a += px[base + sx * 4 + 3]
            offset = (y * size + x) * 4
            out[offset] = round(r / factor)
            out[offset + 1] = round(g / factor)
            out[offset + 2] = round(b / factor)
            out[offset + 3] = round(a / factor)
    return out


def main():
    os.makedirs(OUT_DIR, exist_ok=True)
    targets = [
        ("32x32.png", 32),
        ("128x128.png", 128),
        ("128x128@2x.png", 256),
        ("icon.png", 512),
        ("tray.png", 32),
    ]
    for name, size in targets:
        path = os.path.join(OUT_DIR, name)
        rgba = draw_tray_icon(size) if name == "tray.png" else draw_icon(size)
        written = write_png(path, size, rgba)
        print(f"{name}: {size}x{size}, {written} bytes")


if __name__ == "__main__":
    main()
