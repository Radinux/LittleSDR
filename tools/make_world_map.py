#!/usr/bin/env python3
"""Builds app/src/main/assets/world.bin from SDR++Brown's res/cty/map.json (Natural Earth 1:110m
admin-0 countries, public domain). Format, little-endian:
  int32 ringCount, then per ring: int32 pointCount, pointCount x (float32 lon, float32 lat)
Only outer rings are kept (holes are irrelevant at this scale)."""
import json
import struct
import sys
from pathlib import Path

root = Path(__file__).resolve().parent.parent
src = root / "third_party/sdrpp-brown/root/res/cty/map.json"
dst = root / "app/src/main/assets/world.bin"

features = json.loads(src.read_text())["features"]
rings = []
for f in features:
    g = f["geometry"]
    polys = g["coordinates"] if g["type"] == "MultiPolygon" else [g["coordinates"]]
    for poly in polys:
        outer = poly[0]
        if len(outer) >= 4:
            rings.append(outer)

with dst.open("wb") as out:
    out.write(struct.pack("<i", len(rings)))
    for r in rings:
        out.write(struct.pack("<i", len(r)))
        for lon, lat in r:
            out.write(struct.pack("<ff", lon, lat))
print(f"{dst}: {len(rings)} rings, {sum(len(r) for r in rings)} points, {dst.stat().st_size} bytes", file=sys.stderr)
