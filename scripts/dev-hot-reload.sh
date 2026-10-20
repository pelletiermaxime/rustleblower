#!/usr/bin/env bash
# GPUI Fast's Subsecond integration uses the DX 0.7.10 protocol.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

rustleblower_dx="${RUSTLEBLOWER_DX:-dx}"
if ! command -v "$rustleblower_dx" >/dev/null 2>&1; then
    echo 'Install Dioxus CLI 0.7.10: cargo install dioxus-cli --version 0.7.10 --locked' >&2
    exit 1
fi
if [[ "$("$rustleblower_dx" --version)" != 'dioxus 0.7.10 '* ]]; then
    echo 'Rustleblower hot reload requires Dioxus CLI 0.7.10 to match GPUI Fast.' >&2
    exit 1
fi

python3 scripts/prepare_dependencies.py
# A development session gets its own socket, database and preferences. The
# backend refuses an existing socket; this script never removes one.
rustleblower_dev_dir="${RUSTLEBLOWER_DEV_DIR:-$PWD/.verification/dev}"
mkdir -p "$rustleblower_dev_dir"
rustleblower_dev_dir="$(cd "$rustleblower_dev_dir" && pwd)"
rustleblower_app_args=()
rustleblower_custom_args=false
for rustleblower_arg in "$@"; do
    if [[ "$rustleblower_arg" == --args || "$rustleblower_arg" == --args=* ]]; then
        rustleblower_custom_args=true
    fi
done
if [[ "$rustleblower_custom_args" == false ]]; then
    rustleblower_app_args=(--args="$(python3 - "$rustleblower_dev_dir" <<'PY'
import shlex
import sys
from pathlib import Path
directory = Path(sys.argv[1])
print(shlex.join(['--socket', str(directory / 'ui.sock'), '--database', str(directory / 'history.sqlite3')]))
PY
)")
fi
export RUST_LOG="${RUST_LOG:-warn,gpui=info}"
exec "$rustleblower_dx" serve --platform desktop --renderer native \
    --package rustleblower-dev --bin rustleblower-dev --features hot-reload \
    --hot-patch --cargo-args=--locked \
    "${rustleblower_app_args[@]}" "$@"
