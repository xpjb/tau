#!/usr/bin/env python3
"""Regenerate the beta PNG/ICO from its SVG; requires rsvg-convert."""
from pathlib import Path
import struct
import subprocess

root = Path(__file__).resolve().parent
svg = root / "tau-beta.svg"

def png(size):
    return subprocess.check_output([
        "rsvg-convert", "--width", str(size), "--height", str(size), str(svg)
    ])

(root / "tau-beta.png").write_bytes(png(256))
android = root.parent / "android/res/drawable-nodpi/icon.png"
android.parent.mkdir(parents=True, exist_ok=True)
android.write_bytes(png(512))
sizes = (16, 20, 24, 32, 48, 64, 128, 256)
images = [png(size) for size in sizes]
offset = 6 + 16 * len(images)
entries = []
for size, image in zip(sizes, images):
    entries.append(struct.pack("<BBBBHHII", size % 256, size % 256,
                               0, 0, 1, 32, len(image), offset))
    offset += len(image)
(root / "tau-beta.ico").write_bytes(
    struct.pack("<HHH", 0, 1, len(images)) + b"".join(entries) + b"".join(images)
)
