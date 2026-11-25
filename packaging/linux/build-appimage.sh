#!/usr/bin/env bash
# Gera Capivara-<versão>-<arch>.AppImage com yt-dlp, ffmpeg e deno embutidos.
#
#   ./packaging/linux/build-appimage.sh                # arquitetura da máquina
#   BUNDLE_ENGINES=0 ./packaging/linux/build-appimage.sh   # AppImage leve (motores baixados pelo app)
#
# Requer: cargo, curl, tar (xz), unzip, file.
set -euo pipefail

ARCH="${ARCH:-$(uname -m)}"
case "$ARCH" in
  x86_64)  YT=yt-dlp_linux;         FF=linux64;    DN=x86_64-unknown-linux-gnu ;;
  aarch64) YT=yt-dlp_linux_aarch64; FF=linuxarm64; DN=aarch64-unknown-linux-gnu ;;
  *) echo "arquitetura não suportada: $ARCH" >&2; exit 1 ;;
esac

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
OUT="$ROOT/dist"
WORK="$ROOT/target/appimage"
APPDIR="$WORK/Capivara.AppDir"

echo "==> compilando capivara $VERSION ($ARCH)"
cargo build --release ${TARGET:+--target "$TARGET"}
BIN="target/${TARGET:+$TARGET/}release/capivara"

rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/scalable/apps" "$OUT"
install -m755 "$BIN" "$APPDIR/usr/bin/capivara"

if [ "${BUNDLE_ENGINES:-1}" = "1" ]; then
  echo "==> embutindo motores"
  TMP="$(mktemp -d)"
  curl -fsSL -o "$APPDIR/usr/bin/yt-dlp" "https://github.com/yt-dlp/yt-dlp/releases/latest/download/$YT"
  curl -fsSL -o "$TMP/ff.tar.xz" "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-$FF-gpl.tar.xz"
  tar -xJf "$TMP/ff.tar.xz" -C "$TMP"
  install -m755 "$TMP"/ffmpeg-*/bin/ffmpeg "$APPDIR/usr/bin/ffmpeg"
  install -m755 "$TMP"/ffmpeg-*/bin/ffprobe "$APPDIR/usr/bin/ffprobe"
  curl -fsSL -o "$TMP/deno.zip" "https://github.com/denoland/deno/releases/latest/download/deno-$DN.zip"
  unzip -oq "$TMP/deno.zip" -d "$APPDIR/usr/bin"
  chmod +x "$APPDIR/usr/bin/yt-dlp" "$APPDIR/usr/bin/deno"
  rm -rf "$TMP"
fi

cp ui/icon.svg "$APPDIR/capivara.svg"
cp ui/icon.svg "$APPDIR/usr/share/icons/hicolor/scalable/apps/capivara.svg"
ln -sf capivara.svg "$APPDIR/.DirIcon"
cp packaging/linux/capivara.desktop "$APPDIR/capivara.desktop"
cp packaging/linux/capivara.desktop "$APPDIR/usr/share/applications/capivara.desktop"

cat > "$APPDIR/AppRun" <<'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
export PATH="$HERE/usr/bin:$PATH"
exec "$HERE/usr/bin/capivara" "$@"
EOF
chmod +x "$APPDIR/AppRun"

TOOL="$WORK/appimagetool-$ARCH.AppImage"
if [ ! -x "$TOOL" ]; then
  echo "==> baixando appimagetool"
  curl -fsSL -o "$TOOL" "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$ARCH.AppImage"
  chmod +x "$TOOL"
fi

echo "==> gerando AppImage"
# APPIMAGE_EXTRACT_AND_RUN: funciona em CI/containers sem FUSE.
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$ARCH" "$TOOL" --no-appstream "$APPDIR" "$OUT/Capivara-$VERSION-$ARCH.AppImage"
echo "✔ $OUT/Capivara-$VERSION-$ARCH.AppImage"
