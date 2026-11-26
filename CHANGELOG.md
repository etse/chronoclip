# Changelog

All notable changes to Chronoclip. The release notes on GitHub are taken from this file, so
write each section for users. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

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
