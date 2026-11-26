---
name: test-instance
description: Start, drive and screenshot an isolated Chronoclip instance for end-to-end tests (recording a test window, saving clips, checking status, tray or UI) without touching the maintainer's real instance, settings, buffer, speakers or microphone. Use whenever you run the real chronoclip binary, not just unit tests.
---

# Isolated Chronoclip test instance

Running the real `chronoclip` binary in a test is dangerous by default: it shares the Unix socket,
settings, buffer folder, hotkey and sound devices with the maintainer's own instance. On
2026-11-20 a test's `chronoclip quit` quit the maintainer's real instance and its replay buffer
was deleted. Always use `scripts/test-instance.sh`; it isolates everything.

## What the script isolates

| Shared by default | Test instance |
|---|---|
| Socket in `/run/user/$UID` (a test `quit`/`save` hits the real app) | Own runtime dir `/run/user/$UID/cc-<name>` (short: Unix sockets max ~108 bytes; the scratchpad path is too long) |
| `~/.config`, `~/.cache` (settings, buffer) | `target/cc-<name>/{config,cache}` (on disk; `/tmp` can be tmpfs and the UI warns) |
| Hotkey `Ctrl+F9` | `Ctrl+Alt+Shift+F12` |
| Save chime on the speakers | `save_sound = false`, and `PULSE_SINK` is a silent null sink |
| The real microphone | `PULSE_SOURCE` is the null sink's monitor |
| Sound server lookup via `$XDG_RUNTIME_DIR/pulse` | `PULSE_SERVER` points at the real server, or clips get a silent track |

Windows are found by process id (`wmctrl -lp`), never by title: Nemo shows a folder called
"Chronoclip" too.

## Typical runs

```sh
T=.agents/skills/test-instance/scripts/test-instance.sh
cargo build --release -p chronoclip

# Record a test window with sound (Game only finds the player's stream by its process id).
# Player first: until the window exists, Game only falls back to all sound (your speakers).
$T player cc-test-window 1280x720
$T start --window cc-test-window --set 'buffer_minutes = 5'
sleep 15
$T cli save              # prints the clip path, or the error
$T log | grep -E "WARN|ERROR|found window|recording audio"
$T stop                  # quits, removes the sink and runtime dir; clips stay in target/cc-test/clips

# UI checks: start --visible, then screenshot (window found by pid)
$T start --visible --window no-such-window
$T screenshot /tmp/x.png 560x60+0+0
$T stop --clean
```

`--home` gives the instance a fake home on disk, so the folder fields show `~/.cache/…` and
`~/Videos/Chronoclip` like for a real user (used by the readme-screenshot skill).

Run several instances at once with `CC_NAME=a $T …` and `CC_NAME=b $T …`. Test another binary
with `CHRONOCLIP_BIN=target/stage/usr/bin/chronoclip $T start …` (see the bundled-ffmpeg skill).

## Rules

- Check `$T status` / `pgrep -ax chronoclip` before and after: other instances are left alone,
  but say so in your report if the maintainer's instance is running.
- *All sound* mode records the default output's monitor, i.e. whatever the maintainer is
  listening to (YouTube, Discord). Prefer *Game only* with `$T player`; if you must test *All
  sound*, say so first.
- Clicking in the UI (XTest via `python3 -I` + ctypes on `libXtst.so.6`) moves the maintainer's
  mouse. Mention it before doing it.
- Notifications from the test instance appear on the maintainer's screen; that's fine, but
  expect it. Capture them with `dbus-monitor --session "interface='org.freedesktop.Notifications',member='Notify'"`.
- Always `$T stop` at the end, also after failures, so no null sink or process is left behind.
- Never run Docker yourself; it needs sudo. Ask the maintainer.
