# Copyright (C) 2026 ItsAlphaNeon
# SPDX-License-Identifier: GPL-3.0-or-later
# This file is part of Lemon Studio: https://github.com/ItsAlphaNeon/Lemon-Studio

"""Generate Fruitbat Studio's own app icon.

    python tools/logo/fruitbat-icon.py

Writes:
  crates/os-app/assets/logo/icon-fruitbat.png  the source art, 1024 px
  crates/os-app/assets/lemonstudio.ico         the exe / window / taskbar icon, 16..256 px
  crates/os-app/assets/lemonstudio.png         the same icon at 256 px (Linux window icon and desktop entry)

The mark is a lemon-yellow bat over a plum tile: deliberately unlike the circular lemon-slice mark the app used
before, and built from two shapes only so it stays readable at 16 px.
"""
import math
import os

from PIL import Image, ImageDraw, ImageFilter

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
ASSETS = os.path.join(ROOT, "crates", "os-app", "assets")
S = 1024  # source art
TOP = (58, 30, 92)      # plum, top of the tile
BOTTOM = (24, 18, 32)   # near-black, bottom of the tile
WING_TOP = (255, 224, 92)
WING_BOTTOM = (255, 166, 32)


def tile():
    """The rounded-square background: a vertical gradient inside a large corner radius."""
    grad = Image.new("RGB", (1, S))
    for y in range(S):
        t = y / (S - 1)
        grad.putpixel((0, y), tuple(round(a + (b - a) * t) for a, b in zip(TOP, BOTTOM)))
    grad = grad.resize((S, S))
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, S - 1, S - 1], radius=int(S * 0.22), fill=255)
    tile = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    tile.paste(grad, (0, 0), mask)
    # A faint inner edge, so the tile reads as a surface rather than a flat sticker.
    edge = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(edge).rounded_rectangle(
        [int(S * 0.012), int(S * 0.012), S - int(S * 0.012), S - int(S * 0.012)],
        radius=int(S * 0.21),
        outline=(255, 255, 255, 26),
        width=max(1, int(S * 0.006)),
    )
    return Image.alpha_composite(tile, edge)


def px(x, y):
    # One transform for every shape: the whole mark is scaled about the tile's centre and lifted a touch, so the
    # wings never run into the rounded corners.
    x = 0.5 + (x - 0.5) * 0.90
    y = 0.5 + (y - 0.5) * 0.90 - 0.012
    return (round(x * S), round(y * S))


def wing(mask, sign):
    """One wing: a swept panel with three fingers hanging from its trailing edge.

    The fingers are what make the silhouette a bat rather than a manta: the trailing edge falls to a point between
    each pair, so even the 16 px copy keeps the notched outline.
    """
    d = ImageDraw.Draw(mask)
    # The panel: shoulder to tip along a lifted top edge, back along a straight trailing edge.
    top = []
    for i in range(28):
        t = i / 27
        x = 0.50 + sign * (0.035 + 0.43 * t)
        y = 0.425 - 0.085 * math.sin(t * math.pi * 0.92)
        top.append(px(x, y))
    # A pointed tip, not a cut end: the top edge falls to it and the trailing edge leaves from it.
    panel = top + [px(0.50 + sign * 0.478, 0.475), px(0.50 + sign * 0.440, 0.552), px(0.50 + sign * 0.035, 0.552)]
    d.polygon(panel, fill=255)
    # Two fingers per wing, wide enough to survive a 16 px render: three thin ones blur into a bar on the taskbar.
    for cx, reach in [(0.175, 0.105), (0.345, 0.095)]:
        x = 0.50 + sign * cx
        d.polygon([px(x - 0.052, 0.545), px(x + 0.052, 0.545), px(x, 0.545 + reach)], fill=255)


def bat_mask():
    """The bat: two wings, body, head and ears, drawn white on black."""
    mask = Image.new("L", (S, S), 0)
    wing(mask, 1)
    wing(mask, -1)
    d = ImageDraw.Draw(mask)
    # Body and head, wide enough to overlap both wing panels (no seam left where they meet).
    d.ellipse([*px(0.436, 0.445), *px(0.564, 0.675)], fill=255)
    d.ellipse([*px(0.440, 0.345), *px(0.560, 0.505)], fill=255)
    # Ears: short, wide triangles — at taskbar size they are most of what says "bat".
    for sign in (1, -1):
        d.polygon([px(0.50 + sign * 0.014, 0.400), px(0.50 + sign * 0.040, 0.312), px(0.50 + sign * 0.068, 0.396)], fill=255)
    return mask


def bat():
    grad = Image.new("RGB", (1, S))
    for y in range(S):
        t = y / (S - 1)
        grad.putpixel((0, y), tuple(round(a + (b - a) * t) for a, b in zip(WING_TOP, WING_BOTTOM)))
    grad = grad.resize((S, S))
    mark = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    mark.paste(grad, (0, 0), bat_mask())
    # A soft plum shadow under the silhouette, so the yellow lifts off the tile.
    shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    shadow.paste((10, 4, 18, 150), (0, 0), bat_mask())
    shadow = shadow.filter(ImageFilter.GaussianBlur(S * 0.018))
    return Image.alpha_composite(shadow, mark)


def main():
    art = Image.alpha_composite(tile(), bat())
    src = os.path.join(ASSETS, "logo", "icon-fruitbat.png")
    art.save(src)
    # The exe and window icon: every size Windows asks for, each downscaled on its own so the 16 px copy stays crisp.
    sizes = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
    frames = [art.resize((n, n), Image.LANCZOS) for n in sizes]
    # Pillow writes one frame per requested size, each resampled from the 256 px copy.
    frames[-1].save(os.path.join(ASSETS, "lemonstudio.ico"), sizes=[(n, n) for n in sizes])
    frames[-1].save(os.path.join(ASSETS, "lemonstudio.png"))
    print("wrote", src)


if __name__ == "__main__":
    main()
