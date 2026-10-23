#!/usr/bin/env -S uv run --script
# Usage: uv run bench/gen-network.py OUTDIR SCREENS FORMATS CREATIVE.png
# Generates a large screens.json (N screens, F formats) and a creative.json
# with one file per format, to time the CLI at network scale.
import json, random, sys, shutil, pathlib
out = pathlib.Path(sys.argv[1]); N = int(sys.argv[2]); F = int(sys.argv[3])
src = pathlib.Path(sys.argv[4])
random.seed(1)
sizes = [(1080,1920),(1920,1080),(3840,2160),(1080,1080),(5760,1080),(768,1280),(2160,3840)]
formats, files = [], []
for i in range(F):
    w,h = sizes[i % len(sizes)]
    formats.append({"id": f"f{i}", "pixels": [w,h], "physical_mm": [w//2, h//2],
        "media": {"static": ["image/png","image/jpeg"], "motion": ["video/mp4"], "slot_s": 10, "fps": 30, "max_bytes": 52428800},
        "safe_area": {"top": 60, "right": 40, "bottom": 60, "left": 40},
        "blocked_regions": [{"name": "logo", "rect_px": [w-200, 20, 160, 80]}]})
    name = f"c{i}.png"
    shutil.copy(src, out / name)
    files.append({"file": name, "media_type": "image/png", "pixels": [1080,1920],
        "targets": [f"network.example/net/f{i}"],
        "elements": [{"role": "headline", "text": "Hi", "rect_px": [90,210,700,260], "cap_height_px": 92},
                     {"role": "logo", "rect_px": [90,160,260,60]}]})
screens = []
for s in range(N):
    sc = {"id": f"S-{s:05d}", "format": f"f{s % F}", "venue": "v1",
          "viewing": {"distance_m": [2, 2 + s % 20], "dwell_s": 4, "light": "indoor"}}
    if s % 7 == 0:
        sc["overrides"] = {"blocked_regions": [{"name": "column", "rect_px": [0, 100, 50, 50]}]}
    screens.append(sc)
doc = {"opencrs": "0.1", "publisher": "network.example", "updated_at": "2026-10-20T09:00:00+11:00",
       "expires_at": "2027-01-20T00:00:00+11:00",
       "networks": [{"id": "net", "name": "Big", "rules": {"legibility": {"ratio": 150}},
                     "venues": [{"id": "v1", "name": "V", "venue_type": 205}],
                     "formats": formats, "screens": screens}]}
(out / "screens.json").write_text(json.dumps(doc, indent=1))
(out / "creative.json").write_text(json.dumps({"opencrs": "0.1",
    "campaign": {"id": "c", "advertiser": "A", "category": "IAB8"}, "files": files}, indent=1))
# The same files, each aimed at N / F single screens instead of a format, so a
# check produces one report per screen (N reports in all).
per = N // F
for i, f in enumerate(files):
    f["targets"] = [f"network.example/net/screen/S-{i + F * k:05d}" for k in range(per)]
(out / "creative-screens.json").write_text(json.dumps({"opencrs": "0.1",
    "campaign": {"id": "c", "advertiser": "A", "category": "IAB8"}, "files": files}, indent=1))
