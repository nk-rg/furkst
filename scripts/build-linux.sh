#!/usr/bin/env bash
# Builds Furkst for Linux (x86-64) into dist/: the executable with a desktop entry and its icon, as a .tar.gz.
# Run on Linux from the repository root: ./scripts/build-linux.sh
set -euo pipefail

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
NAME="Furkst-${VERSION}-linux-x64"
STAGE="dist/${NAME}"

cargo build --release --locked -p furkst

rm -rf "$STAGE"
mkdir -p "$STAGE"
cp target/release/furkst "$STAGE/furkst"
cp crates/furkst-app/assets/icon.png "$STAGE/furkst.png"
cat > "$STAGE/furkst.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Furkst
Comment=A fast, friendly git client
Exec=furkst %F
Icon=furkst
Terminal=false
Categories=Development;RevisionControl;
DESKTOP

tar -czf "dist/${NAME}.tar.gz" -C dist "$NAME"
echo "Built dist/${NAME}.tar.gz"
