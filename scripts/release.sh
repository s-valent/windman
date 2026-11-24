#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$root"

usage() {
    cat <<'EOF'
Usage: scripts/release.sh [--debug] [--universal] [--dmg] [--notarize]

Builds Windman.app in dist/, signs it, and optionally packages it.
--debug builds target/debug with ad-hoc signing (used by bundle.sh).

Environment:
  SIGN_IDENTITY   codesign identity (default: first Developer ID Application,
                  then Apple Development, then ad-hoc)
  BUNDLE_ID       CFBundleIdentifier (default: dev.windman.launcher)
  BUNDLE_VERSION  CFBundleShortVersionString / CFBundleVersion
                  (default: version from Cargo.toml)
    NOTARY_PROFILE  notarytool keychain profile, required with --notarize
    APP_ICON        custom .icns file (default: generate assets/Windman.icns)

  --debug         debug build, ad-hoc signed by default
  --universal     build arm64 + x86_64 and merge with lipo
  --dmg           also create dist/Windman-<version>.dmg
  --notarize      submit for notarization and staple the app
                  (requires a Developer ID Application identity and network)

First-time notarization setup:
  xcrun notarytool store-credentials windman-notary \
      --apple-id you@example.com --team-id TEAMID --password APP_PASSWORD
  NOTARY_PROFILE=windman-notary scripts/release.sh --universal --dmg --notarize
EOF
}

debug=0
universal=0
dmg=0
notarize=0
for argument in "$@"; do
    case "$argument" in
        --debug) debug=1 ;;
        --universal) universal=1 ;;
        --dmg) dmg=1 ;;
        --notarize) notarize=1 ;;
        -h | --help) usage; exit 0 ;;
        *) echo "unknown argument: $argument" >&2; usage >&2; exit 1 ;;
    esac
done

if ((debug)) && ((universal)); then
    echo "--debug cannot be combined with --universal" >&2
    exit 1
fi
if ((debug)) && ((notarize)); then
    echo "--debug cannot be combined with --notarize" >&2
    exit 1
fi

version="${BUNDLE_VERSION:-$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)}"
bundle_id="${BUNDLE_ID:-dev.windman.launcher}"
app="$root/dist/Windman.app"
icon="${APP_ICON:-$root/assets/Windman.icns}"
if [[ -z "${APP_ICON:-}" ]]; then
    "$root/scripts/icon.sh" "$icon" >/dev/null
elif [[ ! -f "$icon" ]]; then
    echo "app icon not found: $icon" >&2
    exit 1
fi

build_binary() {
    if ((debug)); then
        cargo build
        echo "target/debug/windman"
        return
    fi
    if ((universal)); then
        for target in aarch64-apple-darwin x86_64-apple-darwin; do
            if ! rustup target list --installed | grep -qx "$target"; then
                echo "missing Rust target $target" >&2
                echo "install it with: rustup target add $target" >&2
                exit 1
            fi
            cargo build --release --target "$target"
        done
        mkdir -p target/universal
        lipo -create \
            target/aarch64-apple-darwin/release/windman \
            target/x86_64-apple-darwin/release/windman \
            -output target/universal/windman
        echo "target/universal/windman"
    else
        cargo build --release
        echo "target/release/windman"
    fi
}

identity="${SIGN_IDENTITY:-}"
if [[ -z "$identity" ]] && ! ((debug)); then
    identity="$(security find-identity -v -p codesigning 2>/dev/null |
        sed -n 's/.*"\(Developer ID Application:.*\)"/\1/p' | head -1)"
fi
if [[ -z "$identity" ]] && ! ((debug)); then
    identity="$(security find-identity -v -p codesigning 2>/dev/null |
        sed -n 's/.*"\(Apple Development:.*\)"/\1/p' | head -1)"
fi
if [[ -z "$identity" ]]; then
    identity="-"
    if ! ((debug)); then
        echo "warning: no signing identity found, using ad-hoc signing" >&2
        echo "warning: the app will not pass Gatekeeper on other Macs" >&2
    fi
elif [[ "$identity" != Developer\ ID\ Application:* ]]; then
    echo "warning: '$identity' is not a Developer ID Application identity" >&2
    echo "warning: notarization and distribution outside this Mac will fail" >&2
fi

binary="$(build_binary)"

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/windman"
if ! ((debug)); then
    strip -x "$app/Contents/MacOS/windman" 2>/dev/null || true
fi

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleExecutable</key>
    <string>windman</string>
    <key>CFBundleIdentifier</key>
    <string>${bundle_id}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>Windman</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>${version}</string>
    <key>CFBundleVersion</key>
    <string>${version}</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>LSUIElement</key>
    <true/>
</dict>
</plist>
PLIST

cp "$icon" "$app/Contents/Resources/Windman.icns"
plutil -insert CFBundleIconFile -string Windman.icns "$app/Contents/Info.plist"

sign_args=(--force --sign "$identity")
if [[ "$identity" != "-" ]]; then
    sign_args+=(--options runtime --timestamp)
fi
codesign "${sign_args[@]}" "$app"
codesign --verify --deep --strict "$app"

echo "signed with: $identity"
echo "$app"

if ((notarize)); then
    if [[ "$identity" != Developer\ ID\ Application:* ]]; then
        echo "notarization requires a Developer ID Application identity" >&2
        exit 1
    fi
    if [[ -z "${NOTARY_PROFILE:-}" ]]; then
        echo "set NOTARY_PROFILE to a notarytool keychain profile" >&2
        usage >&2
        exit 1
    fi
    archive="$root/dist/Windman-$version.zip"
    rm -f "$archive"
    ditto -c -k --keepParent "$app" "$archive"
    xcrun notarytool submit "$archive" --keychain-profile "$NOTARY_PROFILE" --wait
    xcrun stapler staple "$app"
    xcrun stapler validate "$app"
    spctl -a -vv "$app"
    echo "$archive"
fi

if ((dmg)); then
    image="$root/dist/Windman-$version.dmg"
    rm -f "$image"
    hdiutil create -volname Windman -srcfolder "$app" -ov -format UDZO "$image"
    if [[ "$identity" != "-" ]]; then
        codesign --force --sign "$identity" --timestamp "$image"
    fi
    echo "$image"
fi
