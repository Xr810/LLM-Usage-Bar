#!/usr/bin/env python3
"""Regenerate the app icon and the menu-bar status icons.

Run by hand when the mark changes — this is a design-time tool, not part of
the build, so it adds no dependency to `pnpm build`. It needs Python 3 with
Pillow, and `iconutil` for the .icns (macOS only).

    python3 scripts/generate_icons.py

Both marks are the same traffic light. The menu bar lights one lamp to report
status, and the app icon lights all three, so the thing in the Dock and the
thing in the menu bar are recognisably the same object.

They differ in orientation because their frames do. A menu bar icon is capped
at 18pt tall, so stacking the lamps makes each one 4.6pt; laid out along the
free axis they are ~14pt. An app icon is square, where the upright signal head
is the shape that fills it.

Android and iOS icons are left alone: this app builds for macOS only, and
regenerating assets no build consumes would only invite them to drift.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

ICONS = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"
TRAY = ICONS / "tray" / "macos"

# Lamp colours, shared by both marks. Saturated enough to read at 5pt, and
# distinguishable by lamp position as well as by hue, so status survives a
# colour-blind reader and a greyscale screenshot.
RED = (255, 69, 58)
AMBER = (255, 176, 32)
GREEN = (48, 209, 88)


# --------------------------------------------------------------------------
# Menu bar
# --------------------------------------------------------------------------

# The menu bar hard-caps an icon at 18pt tall and derives its width from the
# source aspect ratio (tray-icon, macos/mod.rs: `icon_height: f64 = 18.0`).
# Stacked vertically, three lamps in 18pt are 4.6pt each — legible but timid.
# Laid out horizontally the lamp diameter is set by the *height* instead, so
# each one is ~14pt, three times the area, at the cost of a ~48pt-wide item.
TRAY_SS = 8
TRAY_HEIGHT = 36  # 18pt at 2x, the device resolution of the slot
LAMP_U = 30.0
GAP_U = 2.0
PAD_U = 4.0

# The tray icon carries colour, so it is not a template image and macOS will
# not invert it for the bar's appearance. Everything structural is therefore a
# mid grey — the one value that holds up on both a white and a black bar.
STRUCTURE = (142, 142, 147)


def tray_icon(lit: str | None = None, mono: bool = False, height: int = TRAY_HEIGHT):
    short_u = LAMP_U + 2 * PAD_U
    long_u = 3 * LAMP_U + 2 * GAP_U + 2 * PAD_U
    scale = height / short_u
    width = round(long_u * scale)

    canvas = (width * TRAY_SS, height * TRAY_SS)
    img = Image.new("RGBA", canvas, (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    u = canvas[0] / long_u

    # Stroked, not filled: the bar shows through, so the lit lamp is the only
    # solid mass on screen and the eye goes straight to it.
    stroke = max(1, round(2.0 * u))
    housing = (0, 0, 0, 255) if mono else STRUCTURE + (235,)
    draw.rounded_rectangle(
        [stroke / 2, stroke / 2, canvas[0] - stroke / 2, canvas[1] - stroke / 2],
        radius=(short_u / 2 - 0.5) * u,
        outline=housing,
        width=stroke,
    )

    for index, (name, colour) in enumerate(
        (("red", RED), ("yellow", AMBER), ("green", GREEN))
    ):
        cx = (PAD_U + LAMP_U / 2 + index * (LAMP_U + GAP_U)) * u
        cy = canvas[1] / 2
        if mono:
            fill = (0, 0, 0, 255) if name == lit else (0, 0, 0, 90)
        else:
            fill = colour + (255,) if name == lit else STRUCTURE + (78,)
        r = LAMP_U * u / 2
        draw.ellipse([cx - r, cy - r, cx + r, cy + r], fill=fill)

    return img.resize((width, height), Image.LANCZOS)


def write_tray_icons() -> None:
    for name, lit in (
        ("status_red", "red"),
        ("status_yellow", "yellow"),
        ("status_green", "green"),
        ("status_unknown", None),
    ):
        tray_icon(lit).save(TRAY / f"{name}.png")

    # Monochrome fallback, used only if a colour icon fails to decode. As a
    # template image macOS tints it, so it must be black-on-alpha.
    tray_icon(None, mono=True, height=54).save(TRAY / "statusbar_template_3x.png")
    tray_icon(None, mono=True, height=18).save(TRAY / "statusTemplate.png")
    tray_icon(None, mono=True, height=36).save(TRAY / "statusTemplate@2x.png")


# --------------------------------------------------------------------------
# App icon
# --------------------------------------------------------------------------

CANVAS = 1024
BODY = 824  # Apple's icon body; the margin is where the shadow lives
SS = 2


def superellipse_mask(size: int, n: float = 5.0) -> Image.Image:
    """Apple's squircle: |x|^n + |y|^n = 1, n near 5.

    Not a rounded rectangle. The difference is small and it is the difference
    between looking like a Mac icon and looking close to one.
    """
    mask = Image.new("L", (size, size), 0)
    pixels = mask.load()
    half = size / 2.0
    for row in range(size):
        ny = abs((row + 0.5 - half) / half)
        if ny >= 1.0:
            continue
        nx = (1.0 - ny**n) ** (1.0 / n)
        left, right = half - nx * half, half + nx * half
        for col in range(int(left), int(right) + 1):
            coverage = 1.0
            if col < left:
                coverage = 1.0 - (left - col)
            elif col + 1 > right:
                coverage = right - col
            if coverage > 0:
                pixels[col, row] = int(255 * min(1.0, coverage))
    return mask


def vertical_gradient(size: int, top: tuple, bottom: tuple) -> Image.Image:
    column = Image.new("RGB", (1, size))
    for y in range(size):
        t = y / max(1, size - 1)
        column.putpixel(
            (0, y), tuple(round(top[i] + (bottom[i] - top[i]) * t) for i in range(3))
        )
    return column.resize((size, size), Image.BILINEAR)


def icon_body(top: tuple, bottom: tuple) -> Image.Image:
    size = BODY * SS
    shape = superellipse_mask(size)
    plate = vertical_gradient(size, top, bottom).convert("RGBA")
    plate.putalpha(shape)

    # A soft wash across the top reads as a light source rather than a sticker.
    gloss = Image.new("L", (size, size), 0)
    ImageDraw.Draw(gloss).ellipse(
        [-size * 0.35, -size * 0.95, size * 1.35, size * 0.42], fill=42
    )
    gloss = gloss.filter(ImageFilter.GaussianBlur(size * 0.05))
    highlight = Image.new("RGBA", (size, size), (255, 255, 255, 255))
    highlight.putalpha(
        Image.composite(gloss, Image.new("L", (size, size), 0), shape)
    )
    plate.alpha_composite(highlight)

    full = CANVAS * SS
    canvas = Image.new("RGBA", (full, full), (0, 0, 0, 0))
    shadow = Image.new("RGBA", (full, full), (0, 0, 0, 0))
    offset = (CANVAS - BODY) // 2 * SS
    shadow.paste(
        Image.new("RGBA", (size, size), (0, 0, 0, 90)),
        (offset, offset + 22 * SS),
        shape,
    )
    canvas.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(26 * SS)))
    canvas.alpha_composite(plate, (offset, offset))
    return canvas


def lamp(layer: Image.Image, cx: float, cy: float, r: float, colour: tuple) -> None:
    size = layer.size[0]
    halo = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    # A halo tight enough not to bleed into the neighbouring lamps: at 32px a
    # wide one merges all three into a single smear.
    ImageDraw.Draw(halo).ellipse(
        [cx - r * 1.35, cy - r * 1.35, cx + r * 1.35, cy + r * 1.35],
        fill=colour + (90,),
    )
    layer.alpha_composite(halo.filter(ImageFilter.GaussianBlur(r * 0.42)))
    ImageDraw.Draw(layer).ellipse([cx - r, cy - r, cx + r, cy + r], fill=colour + (255,))

    # A blurred wash on the upper half, not a hard crescent — at small sizes a
    # crisp highlight reads as a second shape instead of as light.
    sheen = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    ImageDraw.Draw(sheen).ellipse(
        [cx - r * 0.78, cy - r * 0.88, cx + r * 0.78, cy + r * 0.06],
        fill=(255, 255, 255, 58),
    )
    layer.alpha_composite(sheen.filter(ImageFilter.GaussianBlur(r * 0.3)))


def app_icon() -> Image.Image:
    img = icon_body((44, 49, 66), (22, 24, 34))
    size = CANVAS * SS
    draw = ImageDraw.Draw(img)

    # The signal head is taller than the lamps strictly need, so the silhouette
    # still reads as a traffic light rather than as three loose dots.
    width, height = 316 * SS, 638 * SS
    x, y = (size - width) / 2, (size - height) / 2
    draw.rounded_rectangle(
        [x, y, x + width, y + height],
        radius=148 * SS,
        fill=(13, 14, 19, 255),
        outline=(255, 255, 255, 40),
        width=5 * SS,
    )

    for index, colour in enumerate((RED, AMBER, GREEN)):
        lamp(img, size / 2, y + (116 + index * 203) * SS, 94 * SS, colour)
    return img


# Windows Store tiles, kept in step so the marks do not diverge per platform.
SQUARE_LOGOS = {
    "Square30x30Logo.png": 30,
    "Square44x44Logo.png": 44,
    "Square71x71Logo.png": 71,
    "Square89x89Logo.png": 89,
    "Square107x107Logo.png": 107,
    "Square142x142Logo.png": 142,
    "Square150x150Logo.png": 150,
    "Square284x284Logo.png": 284,
    "Square310x310Logo.png": 310,
    "StoreLogo.png": 50,
}

ICNS_SIZES = [16, 32, 64, 128, 256, 512, 1024]


def write_app_icons() -> None:
    master = app_icon()

    def at(size: int) -> Image.Image:
        return master.resize((size, size), Image.LANCZOS)

    at(512).save(ICONS / "icon.png")
    at(32).save(ICONS / "32x32.png")
    at(64).save(ICONS / "64x64.png")
    at(128).save(ICONS / "128x128.png")
    at(256).save(ICONS / "128x128@2x.png")
    for name, size in SQUARE_LOGOS.items():
        at(size).save(ICONS / name)

    at(256).save(
        ICONS / "icon.ico",
        sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)],
    )

    if not shutil.which("iconutil"):
        print("iconutil not found; skipped icon.icns", file=sys.stderr)
        return
    with tempfile.TemporaryDirectory() as work:
        iconset = Path(work) / "icon.iconset"
        iconset.mkdir()
        for size in ICNS_SIZES:
            if size <= 512:
                at(size).save(iconset / f"icon_{size}x{size}.png")
            if size >= 32:
                at(size).save(iconset / f"icon_{size // 2}x{size // 2}@2x.png")
        subprocess.run(
            ["iconutil", "-c", "icns", str(iconset), "-o", str(ICONS / "icon.icns")],
            check=True,
        )


if __name__ == "__main__":
    write_tray_icons()
    write_app_icons()
    print(f"regenerated app and tray icons in {ICONS}")
