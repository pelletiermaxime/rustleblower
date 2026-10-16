#!/usr/bin/env python3
"""Export the approved colour mascot at each freedesktop icon size."""
from pathlib import Path
import hashlib
import json
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
ICONS = ROOT / "packaging/linux/icons"
NAME = "dev.rustleblower.Rustleblower.png"
SIZES = (16, 22, 24, 32, 48, 64, 128, 256, 512)


def main():
    converter = shutil.which("magick") or shutil.which("convert")
    if converter is None:
        raise SystemExit("Install ImageMagick to regenerate icons; builds use the checked-in PNGs")
    manifest = {"icon": NAME.removesuffix(".png"), "sizes": []}
    for size in SIZES:
        source = "tiny" if size <= 24 else "compact" if size == 32 else "full"
        destination = ICONS / "hicolor" / f"{size}x{size}" / "apps" / NAME
        destination.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run([
            converter, str(ICONS / "source" / f"{source}.png"),
            "-resize", f"{size}x{size}", "-strip", "PNG32:" + str(destination),
        ], check=True)
        rgba = subprocess.run([converter, str(destination), "-depth", "8", "RGBA:-"], check=True, capture_output=True).stdout
        assert len(rgba) == size * size * 4
        argb = bytearray(len(rgba))
        for offset in range(0, len(rgba), 4):
            argb[offset:offset + 4] = bytes((rgba[offset + 3], *rgba[offset:offset + 3]))
        manifest["sizes"].append({
            "size": size, "drawing": source,
            "png_sha256": hashlib.sha256(destination.read_bytes()).hexdigest(),
            "argb_sha256": hashlib.sha256(argb).hexdigest(),
        })
        print(f"{size}px: {source}")
    (ICONS / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
