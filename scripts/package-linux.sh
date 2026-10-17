#!/usr/bin/env bash
# Adapted from FerryDock (MIT); see LICENSES/FerryDock-MIT.txt.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "$(uname -s)" != Linux || "$(uname -m)" != x86_64 ]]; then
  echo "Linux packaging requires an x86-64 Linux host" >&2
  exit 1
fi

version="$(python3 - <<'PY'
import pathlib
import tomllib
print(tomllib.loads(pathlib.Path("Cargo.toml").read_text())["package"]["version"])
PY
)"
if [[ $# -gt 0 && "$1" != "v$version" ]]; then
  echo "Tag $1 does not match Cargo version $version" >&2
  exit 1
fi

test -x target/release/rustleblower
python3 scripts/collect-licenses.py
python3 scripts/package-collector.py
command -v dpkg-deb >/dev/null
command -v dpkg-shlibdeps >/dev/null

output="$PWD/target/packages"
stage="$(mktemp -d "$PWD/target/package-stage.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
mkdir -p "$output"
archive="rustleblower-$version-linux-x86_64"
mkdir -p "$stage/$archive"
install -m755 target/release/rustleblower "$stage/$archive/rustleblower"
mkdir -p "$stage/$archive/libexec"
cp -R target/collector-package "$stage/$archive/libexec/rustleblower"
install -m644 LICENSE "$stage/$archive/"
cp -R LICENSES "$stage/$archive/"
install -m644 target/release-notices/THIRD_PARTY_NOTICES.txt "$stage/$archive/"
sed "s/@VERSION@/$version/g" packaging/linux/INSTALL.txt >"$stage/$archive/INSTALL.txt"
install -m644 packaging/linux/dev.rustleblower.Rustleblower.desktop "$stage/$archive/"
bash scripts/install-icons.sh "$stage/$archive/share"
tar -czf "$output/$archive.tar.gz" -C "$stage" "$archive"

deb="$stage/debian/rustleblower"
mkdir -p "$deb/DEBIAN" "$deb/usr/bin" "$deb/usr/share/applications" \
  "$deb/usr/share/doc/rustleblower" "$stage/debian"
install -m755 target/release/rustleblower "$deb/usr/bin/rustleblower"
mkdir -p "$deb/usr/libexec"
cp -R target/collector-package "$deb/usr/libexec/rustleblower"
install -m644 packaging/linux/dev.rustleblower.Rustleblower.desktop "$deb/usr/share/applications/"
bash scripts/install-icons.sh "$deb/usr/share"
install -m644 LICENSE "$deb/usr/share/doc/rustleblower/copyright"
cp -R LICENSES "$deb/usr/share/doc/rustleblower/"
install -m644 target/release-notices/THIRD_PARTY_NOTICES.txt "$deb/usr/share/doc/rustleblower/"
install -m644 "$stage/$archive/INSTALL.txt" "$deb/usr/share/doc/rustleblower/INSTALL.txt"

# dpkg-shlibdeps derives library/version requirements from the built ELF binary.
cat >"$stage/debian/control" <<'CONTROL'
Source: rustleblower
Section: utils
Priority: optional
Maintainer: Maxime Pelletier <pelletiermaxime@users.noreply.github.com>

Package: rustleblower
Architecture: amd64
Description: Native frontend for the OpenSnitch firewall daemon
CONTROL
dependencies="$(cd "$stage" && dpkg-shlibdeps -O -e debian/rustleblower/usr/bin/rustleblower)"
dependencies="${dependencies#shlibs:Depends=}"
test -n "$dependencies"
installed_size="$(du -sk "$deb/usr" | cut -f1)"
cat >"$deb/DEBIAN/control" <<CONTROL
Package: rustleblower
Version: $version
Architecture: amd64
Section: utils
Priority: optional
Maintainer: Maxime Pelletier <pelletiermaxime@users.noreply.github.com>
Homepage: https://github.com/pelletiermaxime/rustleblower
Installed-Size: $installed_size
Depends: $dependencies, libvulkan1, libegl1, libwayland-client0, libwayland-cursor0, libwayland-egl1
Recommends: fonts-dejavu-core, pkexec
Description: Native frontend for the OpenSnitch firewall daemon
 Review connection prompts, manage application rules, and browse connection
 history and statistics. Requires Wayland or X11, a graphics driver, and an
 existing OpenSnitch daemon installed separately.
CONTROL
dpkg-deb --root-owner-group --build "$deb" "$output/rustleblower_${version}_amd64.deb"
bash scripts/package-appimage.sh "$version" "$stage" "$output"
(
  cd "$output"
  sha256sum "$archive.tar.gz" "rustleblower_${version}_amd64.deb" "$archive.AppImage" >SHA256SUMS
  sha256sum --check SHA256SUMS
)
echo "Packages written to $output"
