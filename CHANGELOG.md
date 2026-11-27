# Changelog

All notable changes to Chronoclip. The release notes on GitHub are taken from this file, so
write each section for users. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **Record other games:** *Choose window* on Home (or *Add a game window…* on the new **Games** tab)
  lists your open windows; pick the game and Chronoclip records it, now and whenever it runs.
  A window that can't be recorded isn't added; Home says why.
- **176 built-in games** are recognized by themselves: World of Warcraft and other Blizzard games,
  and the most played Steam games (by their Steam app id). More can be added with a pull request
  (see CONTRIBUTING.md).
- **Installed games are found by themselves:** every installed Steam game and known games in Wine
  prefixes (Battle.net through Faugus, Lutris, Heroic, Bottles or Steam). The Games tab lists them;
  *Don't record* switches one off.
- **Search** on the Games tab and in *Choose window*: type part of a name, program or Steam id.
- Games get their proper name from Steam, Heroic, Lutris or Faugus Launcher (e.g. "ELDEN RING"),
  else the window title.
- Chronoclip stays with the game you started, even when you start another one. To switch, use
  *Choose window*.
- **Games tab** lists the games Chronoclip records (World of Warcraft is built in), with
  **Remove** for one you added by mistake.

## [0.2.0] - 2026-11-27

A new, modern dark look, in the style of the Battle.net launcher and Discord.

### Added

- **Home page:** what Chronoclip is doing at a glance (recording, waiting or a problem), the game,
  resolution and sound, the replay buffer filling up, a big **Save replay** button that shows your
  hotkey, and your last clip with **Play** and **Show in folder**. The record dot pulses while
  recording.
- **Settings** in cards (Recording, Sound, Saving, Folders, Window & startup) with switches,
  sliders and buttons instead of plain form fields. Still saved right away.
- **Hotkey recorder:** click the hotkey and press the new combination (Esc cancels), instead of
  typing it.
- **About page** with links to the project, releases and issue tracker, why Chronoclip is safe for
  your account, and credits.
- **Own title bar** with the Home · Settings · About tabs; drag it to move the window, double-click
  to maximize, drag the edges to resize. Prefer your desktop's title bar? Switch off
  *Chronoclip title bar* in Settings.
- Everything can be used with the keyboard (Tab, Enter, Space, arrow keys).

### Changed

- The window is larger (920×640, at least 720×540) and uses the Inter typeface and Lucide icons,
  bundled with Chronoclip so it looks the same on every distro.

## [0.1.0] - 2026-11-26

First release: a replay buffer for World of Warcraft on Linux (X11 + NVIDIA).

### Added

- Always-on replay buffer of 5 to 45 minutes (default 10), recorded on the GPU with NVENC;
  the oldest footage is deleted automatically.
- Save the last minutes with a global hotkey (Ctrl+F9 by default), the **Save replay** button,
  the tray menu or `chronoclip save`. Saving copies the video without re-encoding, so it takes
  seconds even for long clips, while recording continues.
- Finds the WoW window by itself (Wine / Proton, also via Faugus, Lutris or Steam) and starts
  recording when the game appears.
- Sound: only the game's own audio by default (no voice chat or music), or all sound; an
  optional microphone on its own track.
- A chime and a desktop notification (with **Play** and **Show in folder**) when a clip is
  saved, and a different tone when saving fails.
- Tray icon that shows the state: red while recording, grey while waiting for WoW, amber on
  errors.
- Optional start in the tray when you log in.
- Packages: a .deb for Ubuntu 22.04+ and Linux Mint 21+, and an AppImage for other distros
  (glibc 2.35+). Both bring their own LGPL FFmpeg.
