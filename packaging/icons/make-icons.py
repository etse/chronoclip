"""Render the Chronoclip icon (rounded square, replay arrow, red record dot) at all sizes.

Run from the repository root: `python3 packaging/icons/make-icons.py` (needs Pillow).
Writes packaging/icons/hicolor/<size>x<size>/apps/<APP_ID>.png and the app's window/tray icons.
"""
import math
import pathlib

from PIL import Image, ImageDraw

APP_ID = "io.github.etse.chronoclip"
SIZES = [16, 24, 32, 48, 64, 128, 256, 512]
S = 1024
RED, GREY, AMBER = (229, 72, 77), (139, 141, 152), (245, 166, 35)


def draw(dot):
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([32, 32, S - 32, S - 32], radius=220, fill=(31, 36, 48, 255))
    cx = cy = S / 2
    fg = (236, 238, 243, 255)
    r, w = 310, 72
    start, end = -40, 250  # degrees clockwise from 3 o'clock; gap at the top
    d.arc([cx - r, cy - r, cx + r, cy + r], start=start, end=end, fill=fg, width=w)
    a = math.radians(start)
    rc = r - w / 2
    px, py = cx + rc * math.cos(a), cy + rc * math.sin(a)
    tx, ty = math.sin(a), -math.cos(a)
    nx, ny = math.cos(a), math.sin(a)
    length, half = 130, 120
    d.polygon([(px + tx * length, py + ty * length), (px + nx * half, py + ny * half),
               (px - nx * half, py - ny * half)], fill=fg)
    rr = 145
    d.ellipse([cx - rr, cy - rr, cx + rr, cy + rr], fill=dot + (255,))
    return img


def save(img, size, path):
    path.parent.mkdir(parents=True, exist_ok=True)
    img.resize((size, size), Image.LANCZOS).save(path, optimize=True)


root = pathlib.Path(__file__).resolve().parent
ui = root.parent.parent / "app" / "ui" / "icons"
master = draw(RED)
for size in SIZES:
    save(master, size, root / "hicolor" / f"{size}x{size}" / "apps" / f"{APP_ID}.png")
save(master, 256, ui / "chronoclip-256.png")
save(master, 64, ui / "chronoclip-64.png")
save(draw(GREY), 64, ui / "chronoclip-64-idle.png")
save(draw(AMBER), 64, ui / "chronoclip-64-error.png")
