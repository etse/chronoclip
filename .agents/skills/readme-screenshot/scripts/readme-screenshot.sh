#!/usr/bin/env bash
# Retake the README screenshots of the Chronoclip window:
#   docs/images/chronoclip-window.png    Home page: recording a 1920×1080 "World of Warcraft" test
#                                        window with game sound, one saved clip in the last clip card
#   docs/images/chronoclip-settings.png  Settings page (top)
#   docs/images/chronoclip-games.png     Games tab: two games the user added, the built-in list
# Default settings (Ctrl+F9, 10 min, …) and home paths shown as ~.
#
# Usage: readme-screenshot.sh [OUTPUT_DIR]   (default: docs/images)
# Clicks the Settings tab with XTest: the mouse pointer jumps briefly.
set -euo pipefail
repo=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
out=$(realpath -m "${1:-$repo/docs/images}")
export CC_NAME=shot
T=$repo/.agents/skills/test-instance/scripts/test-instance.sh

if pgrep -x chronoclip >/dev/null || pgrep -x AppRun >/dev/null; then
  echo "Another chronoclip is running; it holds Ctrl+F9, which would show as an error in the shot." >&2
  echo "Ask the maintainer to quit it (chronoclip quit) first." >&2
  exit 1
fi
mkdir -p "$out"
trap '"$T" stop --clean >/dev/null' EXIT

# Left click at screen coordinates (XTest through ctypes; no xdotool needed).
click() {
  python3 -I - "$1" "$2" <<'EOF'
import ctypes, sys, time
x11 = ctypes.CDLL("libX11.so.6"); xt = ctypes.CDLL("libXtst.so.6")
x11.XOpenDisplay.restype = ctypes.c_void_p
d = ctypes.c_void_p(x11.XOpenDisplay(None))
xt.XTestFakeMotionEvent(d, -1, int(sys.argv[1]), int(sys.argv[2]), 0)
xt.XTestFakeButtonEvent(d, 1, 1, 0); xt.XTestFakeButtonEvent(d, 1, 0, 0)
x11.XFlush(d); time.sleep(0.1); x11.XCloseDisplay(d)
EOF
}

# A silent "game": its sound stream exists (so the status says "Game sound") but plays silence
# into the instance's null sink. Started before the instance (no all-sound fallback).
# The title ends in an invisible zero-width space: a real window called "World of Warcraft"
# (a file manager folder, a browser tab) must not be the one recorded.
wow=$'World of Warcraft\u200b'
"$T" player "$wow" 1920x1080 "testsrc2=s=1920x1080:r=60[out0];sine=f=440,volume=0[out1]"
# Default settings and a fake home on disk so folders show as ~/…
"$T" start --visible --home --window "$wow" --set 'hotkey = "Ctrl+F9"' --set 'save_sound = true' \
  --set "output_dir = \"$repo/target/cc-shot/home/Videos/Chronoclip\"" \
  --set 'games = [{ name = "Minecraft", title_contains = "Minecraft" }, { name = "Old School RuneScape", title_contains = "Old School RuneScape" }]'
sleep 10
"$T" cli save > /dev/null                            # fills the last clip card
sleep 16                                             # buffer shows some content again
wid=$("$T" wid)
wmctrl -i -a "$wid"                                  # raise it so nothing covers it
sleep 1
# chronoclip-window.png: the released AppStream metadata links to this name, keep it.
"$T" screenshot "$out/chronoclip-window.png"

read -r x y < <(xwininfo -id "$wid" | awk '/Absolute upper-left X/{x=$4} /Absolute upper-left Y/{y=$4} END {print x, y}')
click $((x + 355)) $((y + 24))                       # "Settings" tab in the title bar (Home · Games · Settings · About)
sleep 1
"$T" screenshot "$out/chronoclip-settings.png"

click $((x + 279)) $((y + 24))                       # "Games" tab
sleep 1
"$T" screenshot "$out/chronoclip-games.png"
file "$out"/chronoclip-{window,settings,games}.png
