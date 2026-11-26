#!/usr/bin/env bash
# Check that everything agrees on the version before a release, then print the release notes.
#
# Usage: packaging/check-release.sh VERSION [NOTES_FILE]     (VERSION without "v", e.g. 0.1.0)
# Checks: the workspace version in Cargo.toml, a <release version="VERSION" date="DATE"> entry in
# the AppStream metainfo, and a "## [VERSION] - DATE" section in CHANGELOG.md with the same date.
# Writes the release notes (the CHANGELOG section plus install instructions) to NOTES_FILE.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
version=${1:?usage: $0 VERSION [NOTES_FILE]}
version=${version#v}
notes=${2:-}
cd "$repo"
fail() { echo "release check: $*" >&2; exit 1; }

cargo_version=$(cargo pkgid -p chronoclip | sed 's/.*[@#]//')
[[ $cargo_version == "$version" ]] || fail "Cargo.toml says $cargo_version, the tag says $version"

metainfo=packaging/linux/io.github.etse.chronoclip.metainfo.xml
meta_date=$(sed -n "s/.*<release version=\"$version\" date=\"\([0-9-]*\)\".*/\1/p" "$metainfo")
[[ -n $meta_date ]] || fail "no <release version=\"$version\" date=\"…\"> in $metainfo"

changelog_date=$(sed -n "s/^## \[$version\] - \([0-9-]*\)\$/\1/p" CHANGELOG.md)
[[ -n $changelog_date ]] || fail "no '## [$version] - YYYY-MM-DD' section in CHANGELOG.md"
[[ $changelog_date == "$meta_date" ]] || fail "dates differ: CHANGELOG.md $changelog_date, metainfo $meta_date"

section=$(awk -v v="## [$version] " 'index($0, v) == 1 {on = 1; next} on && /^## \[/ {exit} on' CHANGELOG.md)
[[ -n ${section//[[:space:]]/} ]] || fail "the CHANGELOG.md section for $version is empty"

echo "release check: version $version, date $meta_date: OK"
[[ -z $notes ]] && exit 0

cat > "$notes" <<EOF
$section

## Install

**Ubuntu 22.04+ / Linux Mint 21+:** download \`chronoclip_${version}_amd64.deb\` and double-click it, or run
\`sudo apt install ./chronoclip_${version}_amd64.deb\`. Start **Chronoclip** from the app menu.

**Other distros** (glibc 2.35+: Debian 12+, Fedora 36+, Arch, openSUSE, …): download
\`Chronoclip-${version}-x86_64.AppImage\`, make it executable (\`chmod +x\`) and run it.

Requires an X11 session and an NVIDIA GPU with the proprietary driver (530 or newer). Details:
[INSTALL.md](https://github.com/etse/chronoclip/blob/v${version}/INSTALL.md).

## Files

- \`SHA256SUMS\`: checksums of all files (\`sha256sum -c SHA256SUMS --ignore-missing\`).
- \`ffmpeg-*.tar.xz\`: **not needed to install Chronoclip**; both packages already contain FFmpeg.
  It is the source code of those FFmpeg libraries, provided because FFmpeg's license (LGPL 2.1+)
  requires it to be available. How they are built: \`packaging/ffmpeg/build.sh\` in the source code.
EOF
echo "release notes written to $notes"
