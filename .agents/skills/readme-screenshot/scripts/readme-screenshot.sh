#!/usr/bin/env bash
# Retake docs/images/chronoclip-window.png: the Chronoclip window recording a 1920×1080
# "World of Warcraft" test window with game sound, default settings, home paths shown as ~.
#
# Usage: readme-screenshot.sh [OUTPUT]   (default: docs/images/chronoclip-window.png)
set -euo pipefail
repo=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
out=$(realpath -m "${1:-$repo/docs/images/chronoclip-window.png}")
export CC_NAME=shot
T=$repo/.agents/skills/test-instance/scripts/test-instance.sh

if pgrep -x chronoclip >/dev/null; then
  echo "Another chronoclip is running; it holds Ctrl+F9, which would show as an error in the shot." >&2
  echo "Ask the maintainer to quit it (chronoclip quit) first." >&2
  exit 1
fi
trap '"$T" stop --clean >/dev/null' EXIT
# A silent "game": its sound stream exists (so the status says "game sound") but plays silence
# into the instance's null sink.
"$T" player "World of Warcraft" 1920x1080 "testsrc2=s=1920x1080:r=60[out0];sine=f=440,volume=0[out1]"
# Default settings (Ctrl+F9, 10 min, …) and a fake home on disk so folders show as ~/…
"$T" start --visible --home --window "World of Warcraft" --set 'hotkey = "Ctrl+F9"' --set 'save_sound = true' \
  --set "output_dir = \"$repo/target/cc-shot/home/Videos/Chronoclip\""
sleep 26                                           # let the buffer show some content
wmctrl -i -a "$("$T" wid)"                           # raise it so nothing covers it
sleep 1
"$T" screenshot "$out"
file "$out"
