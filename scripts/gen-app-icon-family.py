#!/usr/bin/env python3
"""Draws app icon drafts in the frame of RustView and RustCut: their dark tile, the light field and one orange symbol with a dark halo.

Usage: gen-app-icon-family.py <output folder>
Writes <out>/svg/<key>.svg and <out>/<key>.png with the sheets of gen-app-icon-drafts.py. Needs rsvg-convert and Pillow.
"""
import importlib.util
import math
import sys
from pathlib import Path

HALO = "#222630"
ORANGE = "#ff7a2f"
FIELD = "#eef1f6"
CLOUD = "M42 96a19 19 0 0 1-2-38 25 25 0 0 1 48-6 22 22 0 0 1 0 44Z"

spec = importlib.util.spec_from_file_location("drafts", Path(__file__).with_name("gen-app-icon-drafts.py"))
drafts = importlib.util.module_from_spec(spec)
spec.loader.exec_module(drafts)


def frame(symbol):
    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#2d3340"/><stop offset="1" stop-color="#171a21"/></linearGradient>
    <linearGradient id="tile" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#c3cbda"/></linearGradient>
  </defs>
  <rect x="4" y="4" width="120" height="120" rx="28" fill="url(#bg)"/>
  <rect x="16" y="16" width="96" height="96" rx="14" fill="url(#tile)"/>
{symbol}
</svg>
"""


def stroked(shapes, width=8, fill="none"):
    """The same shapes twice: a wide dark halo, then the orange line."""
    body = "".join(shapes)
    return (
        f'<g fill="{fill}" stroke-linecap="round" stroke-linejoin="round">'
        f'<g stroke="{HALO}" stroke-width="{width + 5}">{body}</g>'
        f'<g stroke="{ORANGE}" stroke-width="{width}">{body}</g></g>'
    )


def path(d):
    return f'<path d="{d}"/>'


def rays(cx, cy, inner, outer, count=8, skip=()):
    out = []
    for index in range(count):
        if index in skip:
            continue
        angle = index * 2 * math.pi / count
        out.append(
            path(
                f"M{cx + inner * math.cos(angle):.1f} {cy + inner * math.sin(angle):.1f}"
                f"L{cx + outer * math.cos(angle):.1f} {cy + outer * math.sin(angle):.1f}"
            )
        )
    return out


def sun_cloud():
    sun = stroked([f'<circle cx="48" cy="46" r="13"/>', *rays(48, 46, 21, 29, skip=(0, 1))], 7)
    cloud = "M50 98a15 15 0 0 1-2-30 20 20 0 0 1 38-5 17.500 17.500 0 0 1 0 35Z"
    return sun + stroked([path(cloud)], 7, FIELD)


def rain_cloud():
    cloud = "M42 78a16 16 0 0 1-2-32 22 22 0 0 1 42-5 18.500 18.500 0 0 1 2 37Z"
    return stroked([path(cloud)], 7, FIELD) + stroked([path("M46 88l-3 9"), path("M63 88l-3 9"), path("M80 88l-3 9")], 7)


def thermometer():
    tube = "M62 66V38a10 10 0 0 1 20 0v28a19 19 0 1 1-20 0Z"
    level = stroked([path("M72 82V50")], 8)
    return (
        stroked([path(tube)], 7, FIELD)
        + level
        + f'<circle cx="72" cy="82" r="10" fill="{HALO}"/><circle cx="72" cy="82" r="7.500" fill="{ORANGE}"/>'
        + stroked([path("M34 40h12"), path("M34 56h12"), path("M34 72h8")], 6)
    )


def umbrella():
    canopy = "M30 66a34 34 0 0 1 68 0c-6-6-11-6-17 0-6-6-11-6-17 0-6-6-11-6-17 0-6-6-11-6-17 0Z"
    return stroked([path("M64 68v20a8 8 0 0 0 16 0")], 7) + stroked([path(canopy), path("M64 30v-4")], 7, FIELD)


def radar_cloud():
    ring = stroked(['<circle cx="64" cy="64" r="36"/>'], 7)
    sweep = f'<path d="M64 64L64 28A36 36 0 0 1 96 48Z" fill="{ORANGE}" fill-opacity="0.45"/>'
    cloud = "M50 84a11 11 0 0 1-1-22 15 15 0 0 1 29-3 12.500 12.500 0 0 1 0 25Z"
    return ring + sweep + stroked([path(cloud)], 6, FIELD)


def sun_only():
    return stroked(['<circle cx="64" cy="64" r="18"/>', *rays(64, 64, 29, 38)], 8)


def main():
    out = Path(sys.argv[1])
    (out / "svg").mkdir(parents=True, exist_ok=True)
    motifs = {
        "a": ("Sun and cloud", sun_cloud()),
        "b": ("Rain cloud", rain_cloud()),
        "c": ("Thermometer", thermometer()),
        "d": ("Umbrella", umbrella()),
        "e": ("Radar with cloud", radar_cloud()),
        "f": ("Sun", sun_only()),
    }
    for key, (label, symbol) in motifs.items():
        file = out / "svg" / f"{key}.svg"
        file.write_text(frame(symbol), encoding="utf-8")
        drafts.sheet(file, label, out / f"{key}.png")


if __name__ == "__main__":
    main()
