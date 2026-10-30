#!/usr/bin/env python3
"""Turns Natural Earth's 50m countries (public domain) into assets/map/world.bin, the app's own offline map.

Usage: gen-world.py ne_50m_admin_0_countries.geojson assets/map/world.bin
"""
import json
import struct
import sys

countries = json.load(open(sys.argv[1], encoding="utf-8"))
rings = []
for feature in countries["features"]:
    geometry = feature["geometry"]
    polygons = geometry["coordinates"] if geometry["type"] == "MultiPolygon" else [geometry["coordinates"]]
    for polygon in polygons:
        rings.extend(polygon)
with open(sys.argv[2], "wb") as out:
    out.write(struct.pack("<I", len(rings)))
    for ring in rings:
        out.write(struct.pack("<I", len(ring)))
        for lon, lat in ring:
            out.write(struct.pack("<ff", lon, lat))
print(len(rings), "rings")
