#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."
swift build -c release
binary_directory="$(swift build -c release --show-bin-path)"
app="build/Windman.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary_directory/Windman" "$app/Contents/MacOS/Windman"
cp Resources/Info.plist "$app/Contents/Info.plist"
cp THIRD_PARTY_NOTICES.md "$app/Contents/Resources/THIRD_PARTY_NOTICES.md"
signing_identity="${WINDMAN_SIGNING_IDENTITY:-}"
if [ -z "$signing_identity" ]; then
    development_identities="$(security find-identity -v -p codesigning | sed -nE '/"Apple Development:/s/^[[:space:]]*[0-9]+\) ([0-9A-F]{40}) .*/\1/p')"
    if [[ -n "$development_identities" && "$development_identities" != *$'\n'* ]]; then
        signing_identity="$development_identities"
    else
        signing_identity="-"
    fi
fi
codesign --force --sign "$signing_identity" "$app"
echo "Built $app"
