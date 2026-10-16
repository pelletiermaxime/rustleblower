#!/usr/bin/env bash
# Install the checked-in size-specific icons into a package's share directory.
set -euo pipefail
cd "$(dirname "$0")/.."
share="${1:?Pass the owned package share directory}"
for icon in packaging/linux/icons/hicolor/*/apps/*.png; do
  relative="${icon#packaging/linux/icons/}"
  install -Dm644 "$icon" "$share/icons/$relative"
done
