#!/usr/bin/env python3
"""Stage the portable collector and its replaceable shared libraries on Debian."""
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
build = (ROOT / "target/bandwidth").resolve()
manifest = json.loads((build / "build.json").read_text())
assert manifest["status"] == "passed" and manifest["portable"]
output = ROOT / "target/collector-package"
if output.exists():
    shutil.rmtree(output)
output.mkdir()
for name in ("collector", "accounting.bpf.o", "build.json"):
    shutil.copy2(build / name, output / name)
shutil.copytree(build / "lib", output / "lib")
notices = ["Collector shared libraries\n\nThese libraries remain dynamically linked and may be replaced in lib/.\n"
           "The collector's ELF search path is $ORIGIN/lib; no LD_LIBRARY_PATH is required.\n"
           "The desktop makes a fresh private copy of this bundle for each monitoring session.\n"
           "Copyright and license texts accompany each library below. Ubuntu source packages\n"
           "(including upstream source and packaging changes) are linked for each version.\n"]
packages = set()
for name, source in manifest["libraries"].items():
    candidates = [source, str(Path(source).resolve()), source.replace("/usr/lib/", "/lib/")]
    for candidate in candidates:
        match = subprocess.run(["dpkg-query", "-S", candidate], capture_output=True, text=True)
        if match.returncode == 0:
            package = match.stdout.split(": ", 1)[0]
            break
    else:
        raise RuntimeError(f"Cannot identify packaged library license: {name} ({source})")
    notices.append(f"\n{name}: {package}\n")
    if package in packages:
        continue
    packages.add(package)
    source_package, version = subprocess.check_output(
        ["dpkg-query", "-W", "-f", "${source:Package} ${source:Version}", package], text=True).split()
    notices.append(f"Source: https://launchpad.net/ubuntu/+source/{source_package}/{version}\n")
    copyright = Path("/usr/share/doc") / package.split(":")[0] / "copyright"
    notices.append(copyright.read_text())
    # Debian copyright files can refer to these standard full license texts.
    for license_name in ("LGPL-2.1", "LGPL-3", "GPL-2", "GPL-3"):
        reference = Path("/usr/share/common-licenses") / license_name
        if reference.is_file():
            destination = output / "licenses" / license_name
            destination.parent.mkdir(exist_ok=True)
            shutil.copyfile(reference, destination)
(output / "THIRD_PARTY_NOTICES.txt").write_text("\n".join(notices))
print(json.dumps({"collector": str(output), "library_packages": sorted(packages)}))
