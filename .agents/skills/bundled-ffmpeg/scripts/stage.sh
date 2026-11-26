#!/usr/bin/env bash
# Build Chronoclip the way the packages ship it and lay it out as installed:
#   STAGE/usr/bin/chronoclip            (runpath $ORIGIN/../lib/chronoclip)
#   STAGE/usr/lib/chronoclip/libav*.so* (the LGPL FFmpeg from packaging/ffmpeg/build.sh)
#
# Usage: stage.sh [STAGE]        (default: target/stage)
# Builds FFmpeg into target/ffmpeg first if it isn't there. The Rust build goes to
# target/bundled, so normal development builds (against the system FFmpeg) are untouched.
set -euo pipefail
repo=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
stage=$(realpath -m "${1:-$repo/target/stage}")
ffmpeg=$repo/target/ffmpeg
cd "$repo"

[[ -f $ffmpeg/lib/pkgconfig/libavcodec.pc ]] || packaging/ffmpeg/build.sh "$ffmpeg"

export PKG_CONFIG_PATH=$ffmpeg/lib/pkgconfig
export CARGO_TARGET_DIR=$repo/target/bundled
export RUSTFLAGS='-C link-arg=-Wl,-rpath,$ORIGIN/../lib/chronoclip'
cargo build --release -p chronoclip

rm -rf "$stage"
mkdir -p "$stage/usr/bin" "$stage/usr/lib/chronoclip"
cp "$CARGO_TARGET_DIR/release/chronoclip" "$stage/usr/bin/"
cp -P "$ffmpeg"/lib/libav*.so.* "$stage/usr/lib/chronoclip/"

# The binary must find exactly the bundled FFmpeg, nothing from the system.
missing=$(env -u LD_LIBRARY_PATH ldd "$stage/usr/bin/chronoclip" | grep -E 'libav|libsw' \
  | grep -vF "=> $stage/usr/bin/../lib/chronoclip/" || true)
[[ -z $missing ]] || { echo "FFmpeg not resolved from the stage:"$'\n'"$missing" >&2; exit 1; }
echo "staged in $stage"
