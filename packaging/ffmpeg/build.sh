#!/usr/bin/env bash
# Build the LGPL FFmpeg that ships with Chronoclip's .deb and AppImage.
#
# Shared libraries only, no GPL or nonfree parts, and only what Chronoclip uses:
# NVENC H.264 (through CUDA, loaded from the NVIDIA driver at runtime), the AAC encoder,
# MPEG-TS and MP4. Every library gets the runpath $ORIGIN, so they find each other wherever
# they are installed (e.g. /usr/lib/chronoclip).
#
# Usage: packaging/ffmpeg/build.sh [PREFIX]          (default: target/ffmpeg)
# Needs:  a C compiler, make, pkg-config, nasm, curl, xz.
# Writes: PREFIX/lib/*.so*, PREFIX/include, PREFIX/lib/pkgconfig,
#         PREFIX/share/doc/ffmpeg/{FFMPEG-LICENSE.txt,COPYING.LGPLv2.1}
set -euo pipefail

FFMPEG_VERSION=8.1.3
FFMPEG_SHA256=7138d28c96d9d3e3af4ee3d8cad72741f8ffb40da90c1112235dea3ecd3178a3
FFMPEG_URL=https://ffmpeg.org/releases/ffmpeg-$FFMPEG_VERSION.tar.xz
# n12.1 headers: NVENC API 12.1, needs NVIDIA driver 530.41 or newer. MIT licensed; only used
# at build time (FFmpeg loads libcuda/libnvidia-encode from the driver at runtime).
NVHEADERS_VERSION=12.1.14.1
NVHEADERS_SHA256=043e46265517855f13bcb98b3a225b2761b2a83983b1645fa1d05876f1c3a781
NVHEADERS_URL=https://github.com/FFmpeg/nv-codec-headers/releases/download/n$NVHEADERS_VERSION/nv-codec-headers-$NVHEADERS_VERSION.tar.gz

repo=$(cd "$(dirname "$0")/../.." && pwd)
prefix=$(realpath -m "${1:-$repo/target/ffmpeg}")
work=$repo/target/ffmpeg-build
downloads=$repo/target/ffmpeg-src

CONFIGURE_FLAGS=(
  --enable-shared --disable-static
  --disable-programs --disable-doc --disable-debug --disable-network
  # Nothing from the build machine sneaks in (libx264, xcb, zlib, …).
  --disable-autodetect --disable-everything
  --disable-avdevice --disable-avfilter --disable-swscale --disable-swresample
  # NVENC through CUDA (dlopen'ed from the NVIDIA driver).
  --enable-ffnvcodec --enable-cuda --enable-nvenc
  --enable-encoder=h264_nvenc,aac
  # Reading AAC from the MPEG-TS segments (saving a clip) needs the decoder to learn the
  # sample rate and channels; nothing is decoded otherwise.
  --enable-decoder=aac
  --enable-muxer=mpegts,mp4
  --enable-demuxer=mpegts,mov
  --enable-parser=h264,aac
  --enable-bsf=aac_adtstoasc,h264_mp4toannexb,extract_extradata
  --enable-protocol=file
)

fetch() { # url sha256 file
  local file=$downloads/$3
  if [[ ! -f $file ]] || ! echo "$2  $file" | sha256sum -c --quiet - 2>/dev/null; then
    echo "Downloading $1"
    mkdir -p "$downloads"
    curl -sSfL -o "$file.part" "$1"
    mv "$file.part" "$file"
  fi
  echo "$2  $file" | sha256sum -c --quiet - || { echo "checksum mismatch for $file" >&2; exit 1; }
}

command -v nasm >/dev/null || { echo "nasm is missing (sudo apt install nasm)" >&2; exit 1; }

fetch "$FFMPEG_URL" "$FFMPEG_SHA256" "ffmpeg-$FFMPEG_VERSION.tar.xz"
fetch "$NVHEADERS_URL" "$NVHEADERS_SHA256" "nv-codec-headers-$NVHEADERS_VERSION.tar.gz"

rm -rf "$work"
mkdir -p "$work"
tar -xf "$downloads/nv-codec-headers-$NVHEADERS_VERSION.tar.gz" -C "$work"
tar -xf "$downloads/ffmpeg-$FFMPEG_VERSION.tar.xz" -C "$work"

make -C "$work/nv-codec-headers-$NVHEADERS_VERSION" PREFIX="$work/nvheaders" install >/dev/null

cd "$work/ffmpeg-$FFMPEG_VERSION"
PKG_CONFIG_PATH=$work/nvheaders/lib/pkgconfig ./configure --prefix="$prefix" "${CONFIGURE_FLAGS[@]}"
# Runpath $ORIGIN on every library. Added to the generated makefile config because configure
# evaluates --extra-ldsoflags and loses the `$`. library.mak expands the link flags twice
# (inside $(eval)), so the dollar is escaped twice ($$$$ → $$ → $); the quotes keep it from
# the shell.
grep -q '^LDSOFLAGS=' ffbuild/config.mak
sed -i '/^LDSOFLAGS=/ s/$/ -Wl,-rpath,'"'"'$$$$ORIGIN'"'"'/' ffbuild/config.mak
grep -q "^LDSOFLAGS=.*-rpath,'\$\$\$\$ORIGIN'" ffbuild/config.mak
make -j"$(nproc)"
rm -rf "$prefix"
make install
for lib in "$prefix"/lib/libav*.so.*[0-9]; do
  [[ -L $lib ]] && continue
  readelf -d "$lib" | grep -q 'RUNPATH.*\[\$ORIGIN\]' || { echo "$lib has no \$ORIGIN runpath" >&2; exit 1; }
done

# Refuse to ship anything that isn't LGPL.
grep -q '^#define CONFIG_GPL 0' config.h && grep -q '^#define CONFIG_NONFREE 0' config.h \
  || { echo "FFmpeg was configured with GPL or nonfree parts" >&2; exit 1; }

doc=$prefix/share/doc/ffmpeg
mkdir -p "$doc"
cp COPYING.LGPLv2.1 "$doc/"
cat > "$doc/FFMPEG-LICENSE.txt" <<EOF
FFmpeg in Chronoclip
====================

Chronoclip ships these FFmpeg libraries as separate shared libraries:
$(cd "$prefix/lib" && ls libav*.so.* | grep -E '\.so\.[0-9]+$' | sed 's/^/  /')

They are FFmpeg $FFMPEG_VERSION (https://ffmpeg.org), licensed under the GNU Lesser General
Public License version 2.1 or later (see COPYING.LGPLv2.1). This build contains no GPL or
nonfree components.

Source code: $FFMPEG_URL
  SHA-256: $FFMPEG_SHA256
  The same tarball is attached to every Chronoclip release on
  https://github.com/etse/chronoclip/releases.
Build script: packaging/ffmpeg/build.sh in the Chronoclip source code (it also gives every
library the runpath \$ORIGIN).
Configured with:
  ./configure --prefix=<install dir> \\
$(printf '    %s \\\n' "${CONFIGURE_FLAGS[@]}" | sed '$ s/ \\$//')
Built with nv-codec-headers $NVHEADERS_VERSION (MIT license, headers only):
  $NVHEADERS_URL

You may replace these libraries with your own build of a compatible FFmpeg version: put
them in the same directory (for the .deb: /usr/lib/chronoclip).
EOF

echo "FFmpeg $FFMPEG_VERSION installed to $prefix"
