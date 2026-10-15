#!/usr/bin/env bash
# Builds a universal (Apple Silicon + Intel) Furkst.app and zips it into dist/.
# Run on macOS from the repository root: ./scripts/bundle-macos.sh
set -euo pipefail

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
APP=dist/Furkst.app

rustup target add "${TARGETS[@]}"
for target in "${TARGETS[@]}"; do
    cargo build --release --locked -p furkst --target "$target"
done

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/furkst" \
    "target/${TARGETS[0]}/release/furkst" "target/${TARGETS[1]}/release/furkst"

# The app icon, built from the generated 1024 px logo with macOS's own tools.
ICONSET=$(mktemp -d)/Furkst.iconset
mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" crates/furkst-app/assets/icon.png --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
    sips -z $((size * 2)) $((size * 2)) crates/furkst-app/assets/icon.png --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/Furkst.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Furkst</string>
    <key>CFBundleDisplayName</key><string>Furkst</string>
    <key>CFBundleIdentifier</key><string>dev.furkst.Furkst</string>
    <key>CFBundleExecutable</key><string>furkst</string>
    <key>CFBundleIconFile</key><string>Furkst</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleVersion</key><string>${VERSION}</string>
    <key>CFBundleShortVersionString</key><string>${VERSION}</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Ad-hoc signature: required to run on Apple Silicon. Not notarized, see README.
codesign --force --deep --sign - "$APP"

(cd dist && rm -f "Furkst-${VERSION}-macos-universal.zip" && ditto -c -k --keepParent Furkst.app "Furkst-${VERSION}-macos-universal.zip")
echo "Built $APP and dist/Furkst-${VERSION}-macos-universal.zip"
