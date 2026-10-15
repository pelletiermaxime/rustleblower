#!/usr/bin/env python3
"""Collect dependency license documents for release archives from Cargo sources."""
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def main():
    metadata = json.loads(subprocess.check_output([
        sys.executable, str(ROOT / "scripts/cargo.py"), "metadata", "--locked",
        "--offline", "--format-version", "1", "--filter-platform", "x86_64-unknown-linux-gnu",
    ], cwd=ROOT))
    sections = ["Rustleblower dependency notices\n\nThe application is MIT licensed. Dependencies retain their licenses below.\nThis inventory may also include test/build dependencies.\n"]
    missing = []
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if package["name"] == "rustleblower":
            continue
        directory = Path(package["manifest_path"]).parent
        paths = set()
        for pattern in ("[Ll][Ii][Cc][Ee][Nn][Ss][Ee]*", "[Ll][Ii][Cc][Ee][Nn][Cc][Ee]*", "[Cc][Oo][Pp][Yy][Ii][Nn][Gg]*", "[Nn][Oo][Tt][Ii][Cc][Ee]*", "[Uu][Nn][Ll][Ii][Cc][Ee][Nn][Ss][Ee]*"):
            for candidate in directory.glob(pattern):
                if candidate.is_file():
                    paths.add(candidate)
                elif candidate.is_dir():
                    paths.update(p for p in candidate.rglob("*") if p.is_file())
        if package.get("license_file"):
            paths.add(directory / package["license_file"])
        sections.append(f"\n{'=' * 72}\n{package['name']} {package['version']}\nLicense: {package.get('license') or 'see license file'}\nSource: {package.get('repository') or package.get('source') or 'Cargo registry'}\n")
        if not paths:
            missing.append(package["name"])
        for path in sorted(paths):
            sections.append(f"\n--- {path.relative_to(directory)} ---\n" + path.read_text(errors="replace"))
    output = ROOT / "target/release-notices/THIRD_PARTY_NOTICES.txt"
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text("\n".join(sections))
    print(json.dumps({"notices": str(output), "packages": len(metadata["packages"]) - 1, "metadata_only": missing}))


if __name__ == "__main__":
    main()
