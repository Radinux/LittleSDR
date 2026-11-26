#!/usr/bin/env python3
"""Builds app/src/main/assets/drm_schedule.json from the DRM Consortium's shortwave schedule
(https://www.drm.org/drm-in-the-world/broadcast-schedule/): the page carries its data as JSON
(<script id="drm-sw-data">). Only what the app shows is kept, in the page's own record format,
so the app reads the bundled file and a fresh download with the same code.
Usage: tools/make_drm_schedule.py [saved_page.html]"""
import json
import re
import sys
import urllib.request
from pathlib import Path

URL = "https://www.drm.org/drm-in-the-world/broadcast-schedule/"
dst = Path(__file__).resolve().parent.parent / "app/src/main/assets/drm_schedule.json"

if len(sys.argv) > 1:
    page = Path(sys.argv[1]).read_text(encoding="utf-8", errors="replace")
else:
    req = urllib.request.Request(URL, headers={"User-Agent": "Mozilla/5.0"})
    page = urllib.request.urlopen(req, timeout=30).read().decode("utf-8", "replace")
m = re.search(r'<script id="drm-sw-data" type="application/json">(.*?)</script>', page, flags=re.S)
data = json.loads(m.group(1))
out = {
    "season": data.get("season", ""),
    "updatedAt": data.get("updatedAt", ""),
    "records": [{"region": r.get("region", ""), "values": r["values"], "confidence": r.get("confidence", "")} for r in data["records"]],
}
dst.write_text(json.dumps(out, ensure_ascii=False, separators=(",", ":")), encoding="utf-8")
print(f"{dst}: {out['season']} ({out['updatedAt']}), {len(out['records'])} broadcasts", file=sys.stderr)
