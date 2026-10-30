#!/usr/bin/env python3
"""Draws the app icon drafts in the KDE Breeze manner: flat fills, transparent background, the motif touches the canvas edges.

Usage: gen-app-icon-drafts.py <geojson of Germany, or "-"> <output folder>
Writes <out>/svg/<key>.svg and <out>/<key>.png, a sheet with the draft large on a checkerboard and small on Breeze Light and
Dark. Needs rsvg-convert and Pillow.
"""
import json
import math
import subprocess
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

BLUE = "#3daee9"
DEEP = "#2980b9"
DARKER = "#1b6da3"
SUN = "#fdbc4b"
ORANGE = "#f67400"
GRAY = "#7f8c8d"

CLOUD = "M11 33a11 11 0 0 1-1-22h1.2A13 13 0 0 1 35.6 9.3 11 11 0 0 1 37 33Z"


def svg(body):
    return f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48">{body}</svg>'


def line(x0, y0, x1, y1, color, width):
    return f'<path d="M{x0} {y0}L{x1} {y1}" stroke="{color}" stroke-width="{width}" stroke-linecap="round" fill="none"/>'


def sector(cx, cy, r, a0, a1, color):
    x0, y0 = cx + r * math.cos(a0), cy + r * math.sin(a0)
    x1, y1 = cx + r * math.cos(a1), cy + r * math.sin(a1)
    large = 1 if a1 - a0 > math.pi else 0
    return f'<path d="M{cx} {cy}L{x0:.2f} {y0:.2f}A{r} {r} 0 {large} 1 {x1:.2f} {y1:.2f}Z" fill="{color}"/>'


def ring(cx, cy, r, color, width):
    return f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="none" stroke="{color}" stroke-width="{width}"/>'


def rain_cloud():
    drops = "".join(line(x + 3, 37, x, 45.5, DEEP, 5) for x in (10, 21.5, 33))
    return svg(f'<path d="{CLOUD}" fill="{BLUE}" transform="translate(0 -1.5)"/>{drops}')


def radar():
    return svg(
        f'<circle cx="24" cy="24" r="24" fill="{DEEP}"/>'
        + sector(24, 24, 24, -math.pi / 2, 0.12, BLUE)
        + ring(24, 24, 16, DARKER, 2)
        + ring(24, 24, 8, DARKER, 2)
        + line(24, 24, 24, 0, "#ffffff", 2.4)
        + f'<circle cx="35" cy="16" r="4.2" fill="{SUN}"/><circle cx="14" cy="31" r="3" fill="{SUN}"/>'
    )


def sun_cloud():
    rays = "".join(
        line(32 + 12.5 * math.cos(a), 16 + 12.5 * math.sin(a), 32 + 14.5 * math.cos(a), 16 + 14.5 * math.sin(a), SUN, 3)
        for a in [-math.pi * k / 4 for k in range(-1, 4)]
    )
    cloud = f'<g transform="translate(0 15)"><path d="{CLOUD}" fill="{BLUE}"/></g>'
    return svg(f'<circle cx="32" cy="16" r="10" fill="{SUN}"/>{rays}{cloud}')


def storm():
    bolt = f'<path d="M27 21L14 36h9l-4 12 15-16h-9l5-11Z" fill="{SUN}"/>'
    return svg(f'<path d="{CLOUD}" fill="{BLUE}" transform="translate(0 -1.5)"/>{bolt}')


def umbrella():
    canopy = (
        f'<path d="M0 26A24 24 0 0 1 48 26Z" fill="{BLUE}"/>'
        f'<path d="M15 26A24 24 0 0 1 24 2 24 24 0 0 1 33 26Z" fill="{DEEP}"/>'
    )
    handle = f'<path d="M24 2v38a5 5 0 0 1-10 0" fill="none" stroke="{GRAY}" stroke-width="3.6" stroke-linecap="round"/>'
    handle = handle.replace("M24 2v38", "M24 26v15.6").replace("a5 5 0 0 1-10 0", "a4.6 4.6 0 0 1-9.2 0")
    tip = f'<circle cx="24" cy="1.6" r="1.6" fill="{GRAY}"/>'
    return svg(tip + canopy + handle)


def germany(path):
    rings = json.load(open(path, encoding="utf-8"))
    ring_points = max(rings, key=len)

    def my(lat):
        return math.asinh(math.tan(math.radians(lat)))

    xs = [p[0] for p in ring_points]
    ys = [my(p[1]) for p in ring_points]
    scale = 48 / (max(ys) - min(ys))
    x_scale = scale * math.pi / 180
    width = (max(xs) - min(xs)) * x_scale
    x0 = (48 - width) / 2
    points = [(x0 + (p[0] - min(xs)) * x_scale, (max(ys) - my(p[1])) * scale) for p in ring_points]
    outline = "M" + "L".join(f"{x:.2f} {y:.2f}" for x, y in points[::2]) + "Z"
    blobs = (
        f'<clipPath id="c"><path d="{outline}"/></clipPath><g clip-path="url(#c)">'
        f'<ellipse cx="17" cy="20" rx="14" ry="9" fill="{DEEP}" transform="rotate(-20 17 20)"/>'
        f'<ellipse cx="18" cy="20" rx="7" ry="4" fill="{SUN}" transform="rotate(-20 18 20)"/>'
        f'<ellipse cx="30" cy="38" rx="9" ry="6" fill="{DEEP}"/><ellipse cx="30" cy="38" rx="4" ry="2.6" fill="{SUN}"/></g>'
    )
    return svg(f'<path d="{outline}" fill="{BLUE}"/>{blobs}')


def drop():
    shape = "M24 0C24 0 7 19 7 31a17 17 0 0 0 34 0C41 19 24 0 24 0Z"
    return svg(
        f'<clipPath id="d"><path d="{shape}"/></clipPath><path d="{shape}" fill="{BLUE}"/><g clip-path="url(#d)">'
        + ring(24, 31, 7, DEEP, 3)
        + ring(24, 31, 14, DEEP, 3)
        + f'</g><circle cx="24" cy="31" r="3" fill="{SUN}"/>'
    )


def radar_cloud():
    cloud = f'<g transform="translate(6.5 11) scale(0.73)"><path d="{CLOUD}" fill="#ffffff"/></g>'
    return svg(
        f'<circle cx="24" cy="24" r="24" fill="{DEEP}"/>'
        + sector(24, 24, 24, -math.pi / 2, 0.15, BLUE)
        + ring(24, 24, 17, DARKER, 1.8)
        + cloud
        + line(16, 38, 14.5, 42, "#ffffff", 2.6)
        + line(25, 38, 23.5, 42, "#ffffff", 2.6)
        + line(34, 36, 32.5, 40, "#ffffff", 2.6)
    )


def checker(size, cell=16):
    image = Image.new("RGB", (size, size), "#ffffff")
    draw = ImageDraw.Draw(image)
    for y in range(0, size, cell):
        for x in range(0, size, cell):
            if (x // cell + y // cell) % 2:
                draw.rectangle((x, y, x + cell - 1, y + cell - 1), fill="#cfd3d6")
    return image


def render(svg_path, size):
    png = svg_path.with_suffix(f".{size}.png")
    subprocess.run(["rsvg-convert", "-w", str(size), "-h", str(size), "-o", str(png), str(svg_path)], check=True)
    image = Image.open(png).convert("RGBA")
    png.unlink()
    return image


def sheet(svg_path, label, out):
    font = ImageFont.truetype("/usr/share/fonts/noto/NotoSans-Bold.ttf", 26)
    small = ImageFont.truetype("/usr/share/fonts/noto/NotoSans-Regular.ttf", 15)
    canvas = Image.new("RGB", (640, 360), "#1e1e2e")
    draw = ImageDraw.Draw(canvas)
    width = draw.textlength(label, font=font)
    draw.rounded_rectangle((12, 10, 12 + width + 24, 52), 8, fill="#89b4fa")
    draw.text((24, 14), label, font=font, fill="#11111b")
    board = checker(280)
    big = render(svg_path, 280)
    board.paste(big, (0, 0), big)
    canvas.paste(board, (12, 66))
    for row, (color, ink, name) in enumerate((("#eff0f1", "#232629", "Breeze Light"), ("#2a2e32", "#eff0f1", "Breeze Dark"))):
        top = 66 + row * 142
        draw.rectangle((304, top, 628, top + 138), fill=color)
        draw.text((314, top + 6), name, font=small, fill=ink)
        x = 316
        for size in (48, 32, 22, 16):
            icon = render(svg_path, size)
            canvas.paste(icon, (x, top + 92 - size // 2), icon)
            draw.text((x, top + 118), str(size), font=small, fill=ink)
            x += size + 36
    canvas.save(out)


def main():
    germany_file, out = sys.argv[1], Path(sys.argv[2])
    (out / "svg").mkdir(parents=True, exist_ok=True)
    drafts = {
        "a": ("Regenwolke", rain_cloud()),
        "b": ("Radarschirm", radar()),
        "c": ("Sonne und Wolke", sun_cloud()),
        "d": ("Gewitterwolke", storm()),
        "e": ("Regenschirm", umbrella()),
        "f": ("Deutschland mit Regen", germany(germany_file)),
        "g": ("Tropfen mit Radarringen", drop()),
        "h": ("Radarschirm mit Wolke", radar_cloud()),
    }
    for key, (label, body) in drafts.items():
        path = out / "svg" / f"{key}.svg"
        path.write_text(body, encoding="utf-8")
        sheet(path, label, out / f"{key}.png")
        box = render(path, 480).getbbox()
        print(key, label, "bbox on 48 grid:", [round(v / 10, 1) for v in box])


main()
