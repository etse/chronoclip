#!/usr/bin/env bash
# Run an isolated Chronoclip instance for end-to-end tests, without touching the
# maintainer's real instance, settings, buffer, speakers or microphone.
#
# Usage: test-instance.sh <command> [args]
#   start [--window TITLE] [--set 'key = value']... [--visible] [--home]
#                      start an instance (hidden in the tray unless --visible).
#                      --window: record only the window whose title contains TITLE (not the games).
#                      --set: a settings.toml line (overrides the test defaults below).
#                      --home: fake HOME on disk, so paths show as ~/… (screenshots).
#   player TITLE [WxH] [GRAPH]
#                      start an ffplay test window (moving pattern + quiet 440 Hz tone) that plays
#                      into the instance's silent sink; record it with `start --window TITLE`.
#                      Start the player BEFORE the instance: until its window exists, "Game only"
#                      falls back to all sound (the maintainer's speakers).
#                      GRAPH: own lavfi graph with [out0] video and [out1] audio (av-sync-test).
#   cli ARGS…          run the chronoclip CLI against the instance (save | show | quit).
#   wid                print the instance's X window id (found by process id, not title).
#   screenshot FILE [GEOMETRY]   capture its window with ImageMagick (GEOMETRY e.g. 560x60+0+0).
#   log                print its log without color codes.
#   status             print what is running.
#   stop [--clean]     quit, close players, remove the sink and runtime dir (--clean: also clips/logs).
#
# Environment: CC_NAME (default "test") to run several instances at once;
#              CHRONOCLIP_BIN (default target/release/chronoclip); RUST_LOG (default info).
set -euo pipefail

repo=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
name=${CC_NAME:-test}
uid=$(id -u)
base=$repo/target/cc-$name           # on disk: /tmp may be tmpfs (warning in the UI)
rt=/run/user/$uid/cc-$name           # short: Unix socket paths are limited to ~108 bytes
sink=cc_$name
bin=${CHRONOCLIP_BIN:-$repo/target/release/chronoclip}
state=$base/state

die() { echo "test-instance: $*" >&2; exit 1; }
get() { [[ -f $state ]] && sed -n "s/^$1=//p" "$state" | tail -1; }
get_all() { [[ -f $state ]] && sed -n "s/^$1=//p" "$state"; }
put() { mkdir -p "$base"; echo "$1=$2" >> "$state"; }

cli() { XDG_RUNTIME_DIR=$rt "$bin" "$@"; }

# The silent sink the instance and its players play into (created by whichever runs first).
ensure_sink() {
  [[ -n $(get module) ]] && return
  put module "$(pactl load-module module-null-sink sink_name="$sink" sink_properties=device.description="Chronoclip-test-$name")"
}

cmd_start() {
  local window="" visible=0 fake_home=0 sets=()
  while (($#)); do
    case $1 in
      --window) window=$2; shift 2 ;;
      --set) sets+=("$2"); shift 2 ;;
      --visible) visible=1; shift ;;
      --home) fake_home=1; shift ;;
      *) die "unknown option $1" ;;
    esac
  done
  [[ -n $(get pid) ]] && kill -0 "$(get pid)" 2>/dev/null && die "already running (pid $(get pid)); stop it first"
  [[ -x $bin ]] || die "no binary at $bin (cargo build --release -p chronoclip)"
  local others
  others=$(pgrep -ax chronoclip || true)
  [[ -n $others ]] && echo "note: other chronoclip processes are running (left alone):"$'\n'"$others"

  rm -rf "$base/config" "$base/cache" "$base/home" "$rt"
  sed -i '/^pid=/d' "$state" 2>/dev/null || true
  mkdir -p "$base/clips" "$rt"
  chmod 700 "$rt"

  local config_dir=$base/config/chronoclip
  ((fake_home)) && config_dir=$base/home/.config/chronoclip
  mkdir -p "$config_dir"
  # Test defaults: a hotkey nobody uses, no chime, clips inside the instance folder.
  local -A defaults=(
    [hotkey]='"Ctrl+Alt+Shift+F12"'
    [save_sound]=false
    [output_dir]="\"$base/clips\""
  )
  : > "$config_dir/settings.toml"
  for key in "${!defaults[@]}"; do
    printf '%s\n' "${sets[@]}" | grep -qE "^\s*$key\s*=" || echo "$key = ${defaults[$key]}" >> "$config_dir/settings.toml"
  done
  ((${#sets[@]})) && printf '%s\n' "${sets[@]}" >> "$config_dir/settings.toml"

  ensure_sink

  local env=(
    XDG_RUNTIME_DIR="$rt"
    PULSE_SERVER="unix:/run/user/$uid/pulse/native"  # the isolated runtime dir hides the sound server
    PULSE_SINK="$sink"                               # the save chime goes to the silent sink
    PULSE_SOURCE="$sink.monitor"                     # never record the real microphone
    RUST_LOG="${RUST_LOG:-info}"
  )
  if ((fake_home)); then
    env+=(HOME="$base/home")
  else
    env+=(XDG_CONFIG_HOME="$base/config" XDG_CACHE_HOME="$base/cache")
  fi
  [[ -n $window ]] && env+=(CHRONOCLIP_WINDOW="$window")
  local args=()
  ((visible)) || args+=(--hidden)

  if ((fake_home)); then
    env -u XDG_CONFIG_HOME -u XDG_CACHE_HOME -u XDG_DATA_HOME "${env[@]}" "$bin" "${args[@]}" > "$base/app.log" 2>&1 &
  else
    env "${env[@]}" "$bin" "${args[@]}" > "$base/app.log" 2>&1 &
  fi
  put pid $!
  for _ in $(seq 50); do
    [[ -S $rt/chronoclip.sock ]] && { echo "started pid $(get pid) (config $config_dir, log $base/app.log)"; return; }
    sleep 0.2
  done
  die "instance did not create its socket; see $base/app.log"
}

cmd_player() {
  local title=${1:?usage: player TITLE [WxH] [GRAPH]} size=${2:-640x360}
  local graph=${3:-"testsrc2=s=$size:r=60[out0];sine=f=440:r=48000,volume=0.3[out1]"}
  ensure_sink
  PULSE_SINK=$sink ffplay -hide_banner -loglevel error -window_title "$title" \
    -x "${size%x*}" -y "${size#*x}" -left 60 -top 60 -f lavfi "$graph" > /dev/null 2>&1 &
  put player $!
  sleep 1
  echo "player pid $! ($title, $size)"
}

cmd_wid() {
  local pid
  pid=$(get pid)
  [[ -n $pid ]] || die "not running"
  wmctrl -lp | awk -v p="$pid" '$3 == p {print $1; exit}'
}

cmd_screenshot() {
  local file=${1:?usage: screenshot FILE [GEOMETRY]} wid
  wid=$(cmd_wid)
  [[ -n $wid ]] || die "no window (started with --visible?)"
  if [[ -n ${2:-} ]]; then import -window "$wid" -crop "$2" "$file"; else import -window "$wid" "$file"; fi
  echo "$file"
}

cmd_stop() {
  local pid
  pid=$(get pid)
  if [[ -n $pid ]] && kill -0 "$pid" 2>/dev/null; then
    cli quit > /dev/null 2>&1 || true
    for _ in $(seq 50); do kill -0 "$pid" 2>/dev/null || break; sleep 0.2; done
    kill "$pid" 2>/dev/null || true
  fi
  # ffplay (SDL) doesn't always act on SIGTERM; give it a moment, then force it.
  for player in $(get_all player); do
    kill "$player" 2>/dev/null || continue
    for _ in $(seq 15); do kill -0 "$player" 2>/dev/null || break; sleep 0.2; done
    kill -9 "$player" 2>/dev/null || true
  done
  [[ -n $(get module) ]] && pactl unload-module "$(get module)" 2>/dev/null || true
  rm -rf "$rt" "$state"
  [[ ${1:-} == --clean ]] && rm -rf "$base"
  echo "stopped"
}

case ${1:-} in
  start) shift; cmd_start "$@" ;;
  player) shift; cmd_player "$@" ;;
  cli) shift; cli "$@" ;;
  wid) cmd_wid ;;
  screenshot) shift; cmd_screenshot "$@" ;;
  log) sed 's/\x1b\[[0-9;]*m//g' "$base/app.log" ;;
  status) cat "$state" 2>/dev/null || echo "not running"; pgrep -ax chronoclip || true ;;
  stop) shift; cmd_stop "$@" ;;
  *) sed -n '2,27p' "$0"; exit 1 ;;
esac
