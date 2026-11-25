#!/usr/bin/env bash
# Gera dist/Capivara-<versão>-macos.dmg (binário universal Intel + Apple Silicon).
#   BUNDLE_ENGINES=1 ./packaging/macos/build-app.sh   # embute yt-dlp, ffmpeg e deno
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
OUT="$ROOT/dist"
APP="$ROOT/target/macos/Capivara.app"

rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources/bin" "$OUT"
lipo -create -output "$APP/Contents/MacOS/capivara" \
  target/aarch64-apple-darwin/release/capivara target/x86_64-apple-darwin/release/capivara

if [ "${BUNDLE_ENGINES:-1}" = "1" ]; then
  TMP="$(mktemp -d)"
  curl -fsSL -o "$APP/Contents/Resources/bin/yt-dlp" https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos
  # ffmpeg/ffprobe nativos (arm64 + Intel) unidos num binário universal
  for bin in ffmpeg ffprobe; do
    for arch in arm64 amd64; do
      curl -fsSL -o "$TMP/$bin-$arch.zip" "https://ffmpeg.martin-riedl.de/redirect/latest/macos/$arch/release/$bin.zip"
      mkdir -p "$TMP/$arch"
      unzip -oq "$TMP/$bin-$arch.zip" -d "$TMP/$arch"
    done
    lipo -create -output "$APP/Contents/Resources/bin/$bin" "$TMP/arm64/$bin" "$TMP/amd64/$bin"
  done
  ARCH_DENO=$([ "$(uname -m)" = "arm64" ] && echo aarch64 || echo x86_64)
  curl -fsSL -o "$TMP/deno.zip" "https://github.com/denoland/deno/releases/latest/download/deno-$ARCH_DENO-apple-darwin.zip"
  unzip -oq "$TMP/deno.zip" -d "$APP/Contents/Resources/bin"
  chmod +x "$APP/Contents/Resources/bin/"*
  rm -rf "$TMP"
fi

# Ícone: SVG → PNG (Quick Look) → .icns
ICONSET="$ROOT/target/macos/capivara.iconset"
rm -rf "$ICONSET"; mkdir -p "$ICONSET"
if qlmanage -t -s 1024 -o "$ROOT/target/macos" ui/icon.svg >/dev/null 2>&1; then
  for s in 16 32 64 128 256 512; do
    sips -z $s $s "$ROOT/target/macos/icon.svg.png" --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
    sips -z $((s*2)) $((s*2)) "$ROOT/target/macos/icon.svg.png" --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
  done
  iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/capivara.icns" || true
fi

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Capivara</string>
  <key>CFBundleDisplayName</key><string>Capivara</string>
  <key>CFBundleIdentifier</key><string>app.capivara.desktop</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleExecutable</key><string>capivara</string>
  <key>CFBundleIconFile</key><string>capivara</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
EOF

# assinatura ad-hoc (evita "app danificado" no Apple Silicon)
codesign --force --deep --sign - "$APP" || true

DMG="$OUT/Capivara-$VERSION-macos.dmg"
rm -f "$DMG"
hdiutil create -volname Capivara -srcfolder "$APP" -ov -format UDZO "$DMG"
echo "✔ $DMG"
