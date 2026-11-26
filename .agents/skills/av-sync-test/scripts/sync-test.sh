#!/usr/bin/env bash
# A/V sync test: a test window flashes white and beeps at the start of every second; an isolated
# Chronoclip instance records it ("Game only"), saves a clip, and analyze_sync.py measures the
# offset between each flash and its beep across the clip.
#
# Usage: sync-test.sh [--minutes N] [--resize-at SECONDS] [--buffer MINUTES]
#   --minutes    how long to record (default 2; use 46.5 with --buffer 45 for a full-length clip)
#   --resize-at  resize the window from 640x360 to 800x450 after this many seconds (optional;
#                note that a clip starts at the last resize, by design)
#   --buffer     buffer length in minutes, 5–45 (default: enough for --minutes, at least 5)
# The beeps go to a silent sink, so nobody hears them. Results go to target/cc-sync/.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(git -C "$here" rev-parse --show-toplevel)
export CC_NAME=sync
T=$repo/.agents/skills/test-instance/scripts/test-instance.sh
minutes=2 resize_at="" buffer=""
while (($#)); do
  case $1 in
    --minutes) minutes=$2; shift 2 ;;
    --resize-at) resize_at=$2; shift 2 ;;
    --buffer) buffer=$2; shift 2 ;;
    *) sed -n '2,13p' "$0"; exit 1 ;;
  esac
done
seconds=$(python3 -I -c "print(int(float('$minutes') * 60))")
[[ -n $buffer ]] || buffer=$(python3 -I -c "import math; print(min(45, max(5, math.ceil($seconds / 60))))")
title=cc-sync-test
graph="color=c=black:s=640x360:r=60,drawbox=x=0:y=0:w=iw:h=ih:c=white:t=fill:enable='lt(mod(t\,1)\,0.1)'[out0];sine=f=1000:r=48000,volume=volume=0:enable='gte(mod(t\,1)\,0.1)'[out1]"

trap '"$T" stop >/dev/null' EXIT
"$T" player "$title" 640x360 "$graph"              # before the instance: no all-sound fallback
"$T" start --window "$title" --set "buffer_minutes = $buffer"
echo "recording ${seconds} s (buffer ${buffer} min)…"
if [[ -n $resize_at ]]; then
  sleep "$resize_at"
  player=$(sed -n 's/^player=//p' "$repo/target/cc-$CC_NAME/state" | tail -1)
  wid=$(wmctrl -lp | awk -v p="$player" '$3 == p {print $1; exit}')
  wmctrl -i -r "$wid" -e 0,-1,-1,800,450
  echo "resized at ${resize_at} s"
  sleep $((seconds - resize_at))
else
  sleep "$seconds"
fi
clip=$("$T" cli save)
echo "clip: $clip"
"$T" log | grep -E "WARN|ERROR|skipped|resized" || true
python3 -I "$here/analyze_sync.py" "$clip" | tee "$repo/target/cc-$CC_NAME/result.txt"
