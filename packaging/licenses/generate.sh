#!/usr/bin/env bash
# Write THIRD-PARTY-LICENSES.txt: the licenses of all Rust crates in the chronoclip binary.
# Usage: packaging/licenses/generate.sh OUTPUT_FILE
# Needs: cargo-about (`cargo install cargo-about --locked --features cli`), python3.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
out=${1:?usage: $0 OUTPUT_FILE}
cd "$here/../.."

# cargo-about warns that LicenseRef-Slint-* has no license file; the section below adds it.
cargo about generate --config "$here/about.toml" --manifest-path app/Cargo.toml --fail \
  "$here/about.hbs" 2> >(grep -v "LicenseRef-Slint-Royalty-free-2.0 has no license file" >&2) > "$out"

# Slint uses the royalty-free license (LicenseRef-Slint-Royalty-free-2.0), shipped by the
# crates in LICENSES/. cargo-about 0.9 doesn't include LicenseRef-* texts, so append it.
cargo metadata --format-version 1 --manifest-path app/Cargo.toml | python3 -I -c '
import json, pathlib, sys
meta = json.load(sys.stdin)
slint = sorted((p for p in meta["packages"] if "LicenseRef-Slint-Royalty-free" in (p.get("license") or "")),
               key=lambda p: p["name"])
if not slint:
    sys.exit("no Slint crates found")
text = (pathlib.Path(slint[0]["manifest_path"]).parent / "LICENSES" / "LicenseRef-Slint-Royalty-free-2.0.md").read_text()
print("-" * 80)
print("Slint Royalty-free Desktop, Mobile, and Web Applications License 2.0")
print("(LicenseRef-Slint-Royalty-free-2.0, chosen from GPL-3.0-only OR this OR LicenseRef-Slint-Software-3.0)")
print()
print("Used by:")
for p in slint:
    print("  " + p["name"] + " " + p["version"])
print()
print(text)
' >> "$out"
