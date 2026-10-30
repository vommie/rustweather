#!/usr/bin/env python3
"""Turns GeoNames' cities15000.txt (CC BY 4.0) into assets/map/cities.tsv for the temperature layer: name, longitude, latitude,
largest city first.

Usage: gen-cities.py cities15000.txt assets/map/cities.tsv
Source: https://download.geonames.org/export/dump/cities15000.zip
"""
import re
import sys


def main() -> None:
    source, target = sys.argv[1], sys.argv[2]
    cities = []
    with open(source, encoding="utf-8") as lines:
        for line in lines:
            row = line.rstrip("\n").split("\t")
            # Parts of a city are no cities of their own, and the app's font has Latin letters.
            if row[7] == "PPLX":
                continue
            name = row[1] if not re.search(r"[^\u0000-\u024F\u1E00-\u1EFF]", row[1]) else row[2]
            lat, lon, population = float(row[4]), float(row[5]), int(row[14])
            cities.append((population, name, lon, lat))
    cities.sort(key=lambda c: (-c[0], c[1]))
    with open(target, "w", encoding="utf-8") as out:
        for _, name, lon, lat in cities:
            out.write(f"{name}\t{lon:.3f}\t{lat:.3f}\n")


if __name__ == "__main__":
    main()
