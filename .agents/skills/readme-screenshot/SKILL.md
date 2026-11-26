---
name: readme-screenshot
description: Retake the Chronoclip window screenshot in the README (docs/images/chronoclip-window.png) reproducibly, after the window's layout or texts changed. Use when the UI changed visibly or when closing a milestone that touched the UI.
---

# README screenshot

```sh
cargo build --release -p chronoclip
.agents/skills/readme-screenshot/scripts/readme-screenshot.sh     # → docs/images/chronoclip-window.png
```

Pass a different output path to try it out without touching the README image.

What the script sets up (through the test-instance skill), so the shot looks like a normal
user's window:

- **Recording state**: a 1920×1080 test window titled "World of Warcraft" with a silent sound
  stream, so the status line reads "Recording 1920×1080 @ 60 fps · buffer 0:26 / 10:00 · … ·
  game sound".
- **Default settings**, including `Ctrl+F9` and the save sound on (the test defaults would show
  `Ctrl+Alt+Shift+F12`).
- **Paths as `~/…`**: a fake home on disk (`target/cc-shot/home`); not under `/tmp`, which can be
  tmpfs and makes the window show a warning.
- The window is raised before the capture and captured by its window id (found by process id).

It refuses to run while another chronoclip runs: that one holds `Ctrl+F9`, and the shot would
show the hotkey in red as taken. Ask the maintainer to quit theirs first.

Afterwards:

1. Look at the image (layout, no red text, status line complete).
2. If the window shows new or different settings, update the image's `alt` text in
   `README.md` (it lists the settings shown).
3. Commit the image together with the UI change, when the maintainer asks.
