#!/usr/bin/env bash
# Build chronoclip_<version>_amd64.deb for Ubuntu 22.04+ and Linux Mint 21+.
#
# Contents: /usr/bin/chronoclip, the bundled LGPL FFmpeg in /usr/lib/chronoclip, the app-menu
# entry, icons, AppStream metadata and license files. Release builds run in an Ubuntu 22.04
# container (CI), so the minimum library versions match the oldest supported system.
#
# Usage: packaging/deb/build.sh [OUTDIR]          (default: target/deb)
# Needs: everything for packaging/stage.sh, dpkg-dev, cargo-about (see INSTALL.md).
set -euo pipefail
repo=$(cd "$(dirname "$0")/../.." && pwd)
out=$(realpath -m "${1:-$repo/target/deb}")
work=$repo/target/deb-work
root=$work/root
app_id=io.github.etse.chronoclip
cd "$repo"

version=$(cargo pkgid -p chronoclip | sed 's/.*[@#]//')
arch=$(dpkg --print-architecture)

# Binary + bundled FFmpeg, laid out under usr/.
rm -rf "$work"
packaging/stage.sh "$root"

# Desktop integration.
install -Dm644 "packaging/linux/$app_id.desktop" "$root/usr/share/applications/$app_id.desktop"
install -Dm644 "packaging/linux/$app_id.metainfo.xml" "$root/usr/share/metainfo/$app_id.metainfo.xml"
mkdir -p "$root/usr/share/icons"
cp -r packaging/icons/hicolor "$root/usr/share/icons/hicolor"

# Documentation and licenses.
doc=$root/usr/share/doc/chronoclip
install -Dm644 packaging/deb/copyright "$doc/copyright"
install -m644 target/ffmpeg/share/doc/ffmpeg/FFMPEG-LICENSE.txt target/ffmpeg/share/doc/ffmpeg/COPYING.LGPLv2.1 "$doc/"
packaging/licenses/generate.sh "$doc/THIRD-PARTY-LICENSES.txt"
gzip -9n "$doc/THIRD-PARTY-LICENSES.txt"
install -m644 README.md "$doc/"

# Dependencies: what the binary links (from dpkg-shlibdeps, with minimum versions) plus what it
# loads at runtime: X11/xkbcommon (UI toolkit), Xcomposite + GL (capture), PulseAudio (sound),
# xdg-open (Play / Open folder). The NVIDIA driver can't be listed: its package name changes
# with every version (nvidia-driver-595, …).
mkdir -p "$work/shlibs/debian"
echo "Source: chronoclip" > "$work/shlibs/debian/control"
shlibs=$(cd "$work/shlibs" && dpkg-shlibdeps -O --ignore-missing-info \
  -l"$root/usr/lib/chronoclip" "$root/usr/bin/chronoclip" 2>/dev/null | sed -n 's/^shlibs:Depends=//p')
[[ -n $shlibs ]] || { echo "dpkg-shlibdeps found no dependencies" >&2; exit 1; }
runtime="libx11-6, libx11-xcb1, libxcb1, libxcursor1, libxi6, libxkbcommon0, libxkbcommon-x11-0, libxcomposite1, libgl1, libpulse0, xdg-utils"

# Same permissions whatever the builder's umask: directories and programs 755, files 644.
chmod -R u=rwX,go=rX "$root"

mkdir -p "$root/DEBIAN"
cat > "$root/DEBIAN/control" <<EOF
Package: chronoclip
Version: $version
Architecture: $arch
Maintainer: Chronoclip contributors <https://github.com/etse/chronoclip/issues>
Installed-Size: $(du -sk --exclude=DEBIAN "$root" | cut -f1)
Depends: $shlibs, $runtime
Section: video
Priority: optional
Homepage: https://github.com/etse/chronoclip
Description: replay buffer for your games on Linux
 Chronoclip keeps the last minutes of your gameplay in a rolling buffer and
 saves them as a video when you press a hotkey (Ctrl+F9 by default), like
 NVIDIA ShadowPlay's Instant Replay. Made for World of Warcraft first, it
 records any game: Steam and Proton, Battle.net, Lutris, Heroic or native.
 .
 Recording runs on the GPU (NVENC), saving copies the video without
 re-encoding, and only the game's own sound is recorded by default. Chronoclip
 never touches the game process.
 .
 Requires an X11 session and an NVIDIA GPU with the proprietary driver
 (version 530 or newer).
EOF

mkdir -p "$out"
deb=$out/chronoclip_${version}_${arch}.deb
dpkg-deb --root-owner-group -Zxz --build "$root" "$deb"
echo "built $deb ($(du -h "$deb" | cut -f1))"
