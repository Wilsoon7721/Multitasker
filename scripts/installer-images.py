# Draws the installer's side panel and header from the logo's shapes (src-tauri/icons/icon.svg), as the 24-bit
# bitmaps NSIS needs. Run with: python scripts/installer-images.py
from pathlib import Path

from PIL import Image, ImageDraw

BLUE = (47, 91, 234)
WHITE = (255, 255, 255)
YELLOW = (255, 211, 77)
SCALE = 4  # drawn larger, then shrunk, for smooth edges

ICONS = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"


def mix(colour, background, opacity):
    return tuple(round(c * opacity + b * (1 - opacity)) for c, b in zip(colour, background))


def draw_mark(draw, left, top, unit, background):
    """The logo's three bars and dot, `unit` pixels to one unit of the 512-unit logo, its top-left bar at left, top."""
    bars = [(0, 0, 304, 1), (0, 96, 240, 0.75), (0, 192, 176, 0.5)]
    for x, y, width, opacity in bars:
        box = [left + x * unit, top + y * unit, left + (x + width) * unit, top + (y + 64) * unit]
        draw.rounded_rectangle(box, radius=32 * unit, fill=mix(WHITE, background, opacity))
    cx, cy, r = left + 272 * unit, top + 224 * unit, 40 * unit
    draw.ellipse([cx - r, cy - r, cx + r, cy + r], fill=YELLOW)


def save(image, size, name):
    image.resize(size, Image.LANCZOS).convert("RGB").save(ICONS / name)
    print(f"wrote {ICONS / name}")


def sidebar():
    """The welcome and finish pages' panel, 164x314: the mark on the logo's blue, in the upper part."""
    width, height = 164 * SCALE, 314 * SCALE
    image = Image.new("RGB", (width, height), BLUE)
    unit = 96 * SCALE / 304
    draw_mark(ImageDraw.Draw(image), (width - 304 * unit) / 2, 72 * SCALE, unit, BLUE)
    save(image, (164, 314), "installer-sidebar.bmp")


def header():
    """The other pages' header, 150x57: the whole logo, small, on the header's white."""
    width, height = 150 * SCALE, 57 * SCALE
    image = Image.new("RGB", (width, height), WHITE)
    draw = ImageDraw.Draw(image)
    size = 40 * SCALE
    left, top = width - size - 10 * SCALE, (height - size) / 2
    draw.rounded_rectangle([left, top, left + size, top + size], radius=size * 112 / 512, fill=BLUE)
    unit = size / 512
    draw_mark(draw, left + 104 * unit, top + 128 * unit, unit, BLUE)
    save(image, (150, 57), "installer-header.bmp")


sidebar()
header()
