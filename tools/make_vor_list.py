#!/usr/bin/env python3
"""Builds app/src/main/assets/vor.json from OurAirports' navaids (public domain,
https://ourairports.com/data/): every VOR, VOR-DME and VORTAC as
[ident, name, frequency kHz, latitude, longitude, country, declination deg (east +)].
The declination is the beacon's own alignment ("slaved variation") when known: its radials
are magnetic, counted from that north.
Usage: tools/make_vor_list.py [navaids.csv]"""
import csv
import io
import json
import sys
import urllib.request
from pathlib import Path

URL = "https://davidmegginson.github.io/ourairports-data/navaids.csv"
dst = Path(__file__).resolve().parent.parent / "app/src/main/assets/vor.json"

if len(sys.argv) > 1:
    text = Path(sys.argv[1]).read_text(encoding="utf-8")
else:
    text = urllib.request.urlopen(URL, timeout=60).read().decode("utf-8")

out = []
for r in csv.DictReader(io.StringIO(text)):
    if r["type"] not in ("VOR", "VOR-DME", "VORTAC"):
        continue
    try:
        khz = int(float(r["frequency_khz"]))
        lat, lon = float(r["latitude_deg"]), float(r["longitude_deg"])
    except ValueError:
        continue
    if not 108_000 <= khz <= 118_000:
        continue
    var = r["slaved_variation_deg"] or r["magnetic_variation_deg"] or "0"
    out.append([r["ident"], r["name"].strip(), khz, round(lat, 4), round(lon, 4), r["iso_country"], round(float(var), 1)])
out.sort(key=lambda v: (v[5], v[0]))
dst.write_text(json.dumps(out, ensure_ascii=False, separators=(",", ":")), encoding="utf-8")
print(f"{dst}: {len(out)} VOR", file=sys.stderr)
