#!/usr/bin/env bash
# Build Chronoclip-<version>-x86_64.AppImage: one file that runs on most Linux distros with
# glibc 2.35 or newer (Ubuntu 22.04+, Debian 12+, Fedora 36+, Arch, openSUSE Tumbleweed, …).
#
# It carries the binary and the bundled LGPL FFmpeg; nothing else needs bundling: the binary
# links only glibc, libgcc and fontconfig (on AppImage's exclude list, i.e. on every desktop)
# and loads X11, GL, CUDA/NVENC and PulseAudio from the system at runtime.
# Release builds run in an Ubuntu 22.04 container (CI), like the .deb.
#
# Usage: packaging/appimage/build.sh [OUTDIR]          (default: target/appimage)
# Needs: everything for packaging/stage.sh, cargo-about, curl.
set -euo pipefail

APPIMAGETOOL_VERSION=1.9.1
APPIMAGETOOL_SHA256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
# Static runtime: needs only `fusermount3` (fuse3) on the user's system, no libfuse2.
RUNTIME_VERSION=20251108
RUNTIME_SHA256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d

repo=$(cd "$(dirname "$0")/../.." && pwd)
out=$(realpath -m "${1:-$repo/target/appimage}")
work=$repo/target/appimage-work
appdir=$work/Chronoclip.AppDir
tools=$repo/target/appimage-tools
app_id=io.github.etse.chronoclip
cd "$repo"

fetch() { # url sha256 file
  local file=$tools/$3
  if [[ ! -f $file ]] || ! echo "$2  $file" | sha256sum -c --quiet - 2>/dev/null; then
    mkdir -p "$tools"
    curl -sSfL -o "$file.part" "$1"
    mv "$file.part" "$file"
  fi
  echo "$2  $file" | sha256sum -c --quiet - || { echo "checksum mismatch for $file" >&2; exit 1; }
}
fetch "https://github.com/AppImage/appimagetool/releases/download/$APPIMAGETOOL_VERSION/appimagetool-x86_64.AppImage" \
  "$APPIMAGETOOL_SHA256" "appimagetool-$APPIMAGETOOL_VERSION-x86_64.AppImage"
fetch "https://github.com/AppImage/type2-runtime/releases/download/$RUNTIME_VERSION/runtime-x86_64" \
  "$RUNTIME_SHA256" "runtime-$RUNTIME_VERSION-x86_64"
chmod +x "$tools/appimagetool-$APPIMAGETOOL_VERSION-x86_64.AppImage"

version=$(cargo pkgid -p chronoclip | sed 's/.*[@#]//')

# Binary + bundled FFmpeg: AppDir/usr/{bin,lib/chronoclip}.
rm -rf "$work"
packaging/stage.sh "$appdir"

# Desktop integration (also read by AppImage integration tools such as AppImageLauncher).
install -Dm644 "packaging/linux/$app_id.desktop" "$appdir/usr/share/applications/$app_id.desktop"
install -Dm644 "packaging/linux/$app_id.metainfo.xml" "$appdir/usr/share/metainfo/$app_id.metainfo.xml"
mkdir -p "$appdir/usr/share/icons"
cp -r packaging/icons/hicolor "$appdir/usr/share/icons/hicolor"
cp "packaging/linux/$app_id.desktop" "$appdir/$app_id.desktop"
cp "packaging/icons/hicolor/256x256/apps/$app_id.png" "$appdir/$app_id.png"
ln -s "$app_id.png" "$appdir/.DirIcon"
# AppRun starts the real binary; its runpath ($ORIGIN/../lib/chronoclip) finds FFmpeg.
ln -s usr/bin/chronoclip "$appdir/AppRun"

# Licenses.
doc=$appdir/usr/share/doc/chronoclip
install -Dm644 LICENSE "$doc/LICENSE"
install -m644 target/ffmpeg/share/doc/ffmpeg/FFMPEG-LICENSE.txt target/ffmpeg/share/doc/ffmpeg/COPYING.LGPLv2.1 "$doc/"
packaging/licenses/generate.sh "$doc/THIRD-PARTY-LICENSES.txt"

chmod -R u=rwX,go=rX "$appdir"

mkdir -p "$out"
image=$out/Chronoclip-$version-x86_64.AppImage
# Extract-and-run: works without FUSE (containers, CI).
APPIMAGE_EXTRACT_AND_RUN=1 ARCH=x86_64 "$tools/appimagetool-$APPIMAGETOOL_VERSION-x86_64.AppImage" \
  --no-appstream --runtime-file "$tools/runtime-$RUNTIME_VERSION-x86_64" "$appdir" "$image"
echo "built $image ($(du -h "$image" | cut -f1))"
