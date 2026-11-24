#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"

usage() {
    cat <<'EOF'
Usage: scripts/icon.sh [output.icns]

Draws Windman's magnifying glass on a dark rounded tile at every icon size.
Defaults to assets/Windman.icns. No source image is needed.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    usage
    exit 0
fi
if (($# > 1)); then
    usage >&2
    exit 1
fi

output="${1:-$root/assets/Windman.icns}"
if [[ "$output" != *.icns ]]; then
    echo "output must have an .icns extension" >&2
    exit 1
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/windman-icon.XXXXXX")"
trap 'rm -rf "$work"' EXIT
swift "$root/scripts/icon.swift" "$work/Windman.icns"
mkdir -p "$(dirname "$output")"
mv "$work/Windman.icns" "$output"
echo "$output"
