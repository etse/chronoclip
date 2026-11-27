# Roadmap

Status: ✅ done · 🚧 in progress · ⬜ not started

| # | Milestone | Status |
|---|---|---|
| M0 | Scaffold: repo, docs, agent instructions | ✅ |
| M1 | Skeleton app: Slint window, tray, settings, single instance, `chronoclip save/show/quit` | ✅ |
| M2 | Capture pipeline (X11 + NVIDIA) | ✅ |
| M3 | Save replay: global hotkey, lossless remux to MP4 | ✅ |
| M4 | Audio: game audio via PipeWire, optional mic track | ✅ |
| M5 | Polish: save notification and chime, tray state, start with desktop | ✅ |
| M6 | Packaging: .deb (Ubuntu/Mint) and AppImage on GitHub Releases | ✅ |
| M7 | Modern dark UI: Battle.net-style tabs, Discord-like look, custom title bar | ✅ |
| M8 | Other games: record any game, not just WoW: game detection, window picker, per-game clips | 🚧 |
| M9 | Wayland + AMD/Intel: portal/PipeWire capture, GlobalShortcuts portal, VAAPI | ⬜ |

## M8: Other games

Goal: Chronoclip records whatever game you play, not just WoW: it finds the game by itself (asking once when unsure), names it properly, and saves its clips in a folder per game. WoW behaves exactly as before.

Decisions (maintainer, 2026-11-27):
- **How automatic:** a "looks like a game" score, plus a "Record this game?" question for unknown games; the answer is remembered.
- **Two games open:** the first one found is recorded until it closes.
- **Clips:** a folder per game, e.g. `Elden Ring/Elden Ring_2026-11-27_21-04-33.mp4`.
- **Game changes:** the buffer is cleared, so a clip never mixes two games.

Test games: WoW (Faugus/Battle.net), Elden Ring, Satisfactory, Palworld, Path of Exile 2 (Steam/Proton).

Findings while planning (2026-11-27, on the maintainer's desktop):
- The **Battle.net launcher** has the same Proton signals as a game: WM_CLASS `steam_app_faugus…`, and `SteamAppId`/`GAMEID`/`WINEPREFIX` in its environment. Launchers must be skipped explicitly.
- `_NET_WM_BYPASS_COMPOSITOR`: Chrome, Slack, Discord and 1Password set `2` ("please compose me"). SDL games set `1`.
- GameMode runs but only knows games started with `gamemoderun`. NVIDIA's GPU process list marks nothing as special (Xorg, Cinnamon and Chrome all use the GPU), and it's NVIDIA-only.
- Steam `appmanifest_*.acf` files also list tools (Proton, Steam Linux Runtime, Steamworks Redistributables); filter them.

Detection only reads window properties and `/proc/<pid>/{comm,cmdline,environ}` (kernel metadata). Never `/proc/<pid>/maps` or `mem` (hard rule 1).

- [ ] Core: a `Game` model (name, how it's matched: executable, Steam app id, window title) and a games list in the settings, with WoW built in; replaces the WoW-only `WindowMatcher` (tests for matching and settings validation)
- [ ] Window picker: "Choose a window…" on Home lists open windows (title, executable); picking one adds it to your games and records it
- [ ] Game names from launchers: Steam (`steam_app_<id>` / `SteamAppId` → `appmanifest`, tools filtered out), Lutris (`pga.db`), Faugus, Heroic; the executable name as fallback
- [ ] Game score: signals for (Wine/Proton/Steam launched, bypass-compositor `1`, fullscreen or monitor-sized, GameMode) and against (known launchers and helpers, browsers, Electron apps); unit tests on recorded window lists, including the Battle.net launcher above
- [ ] "Record this game?" notification for a likely but unknown game (Always / Not this one), remembered by executable
- [ ] Per-game clips: folder and file name after the game (safe file names); status texts, tray, "nothing recorded" message name the game
- [ ] Switching games: when the recorded game closes, look again; a different game clears the buffer first
- [ ] Settings: a Games card (detected/added games, remove, never record)
- [ ] Texts: "for World of Warcraft" → "for your games" in README, About, desktop file, metainfo, CLI help, autostart file
- [ ] Docs: README, architecture.md (detection), AGENTS.md (project description), CHANGELOG
- [ ] Verified: fmt, clippy, tests; test instance with test windows (known game, unknown game → question, launcher ignored, switch clears buffer)
- [ ] Verified in-game: WoW as before; Elden Ring, Satisfactory, Palworld and Path of Exile 2 detected with the right names, clips in their own folders, launchers (Battle.net, Steam) never recorded

Out of scope: recording several games at once, per-game settings (fps, sound), Wayland (M9), an online game database (no network calls, hard rule 8).

## M7: Modern dark UI

Goal: a window that looks like the apps gamers use every day (Battle.net launcher, Discord): dark, clean, one accent colour, big clear status and a big **Save replay** button, while the Rust side and the recording stay as they are.

Decisions (maintainer, 2026-11-26): **top tabs** like the Battle.net launcher (Home · Settings · About), **Discord blurple** accent (`#5865F2`, red stays for the record dot), extras: hotkey recorder, last clip card, pulsing record dot, custom title bar. Dark only.

Approach: own Slint components instead of the stock `fluent` widgets (they can't be restyled this far). The window keeps getting its data through the same properties and callbacks; new data (buffer fill, last clip) comes from Rust like today. Theme + Home page first, then a screenshot for the maintainer's feedback before the rest.

- [x] Theme (`app/ui/theme.slint`): dark palette, accent, radii, spacing, type scale; bundled **Inter** font (OFL, license file shipped); dark system title bar hint where the system one is used
- [x] Component kit (`app/ui/components/`): buttons (primary/secondary/ghost), toggle, slider, dropdown (own popup), text field, card, tab bar, progress bar, status badge, icon (**Lucide** SVGs, ISC); hover/pressed/focus states, keyboard focus visible
  - `app/ui/components.slint` (button, icon button, keycap, card, progress bar, tab) and `controls.slint` (toggle, slider, segmented control, dropdown, text field, hotkey field). Fixed: the focus scopes swallowed the first click (`focus-on-click: false`)
- [x] Custom title bar: frameless window with our own dark bar (logo, tabs, minimize/close), drag to move, double-click to maximize, resize edges; setting to fall back to the system title bar if the window manager misbehaves
  - Slint's built-in `WindowMoveArea`, `maximized`/`minimized`; edge resizing via winit `drag_resize_window` (`unstable-winit-030`). Fixed after the maintainer's test: the window manager keeps the mouse release, so the edge handle stayed pressed and swallowed the next click; a release is now sent right after the resize starts. Fallback toggle verified (Cinnamon's title bar appears at once)
- [x] Home page: status card (state, game, resolution, fps, sound), buffer bar (filled/length, size), big **Save replay** button with the hotkey, last clip card (length, size, time; **Play**, **Show in folder**), save progress/result
  - Maintainer: "I like the look on the home page" (2026-11-26)
- [x] Settings page: the existing settings in cards (Recording, Sound, Saving, Folders, Startup), same auto-save behaviour, folder warnings and errors in the new style
- [x] Hotkey recorder: click the field, press the combination (Esc cancels); same validation as today
  - Verified with XTest: Ctrl+Shift+F10 recorded, saved and registered; Tab gives a clear error
- [x] About page: version, links, licenses; keeps Slint's "Made with Slint" attribution (license requirement)
  - Slint badge kept small (132 px). Maintainer on Settings/About: "I like it, this is good enough for a first version. Looks more modern!" (2026-11-27)
- [x] Pulsing record dot, animated only while the window is visible (no redraws while hidden or minimized)
  - About +1.2 % of one core while the window is visible; nothing when hidden
- [x ] Checks: readable at 100–200 % scaling, keyboard navigation (Tab, Enter, Space, arrows), no CPU while hidden; README screenshots retaken (readme-screenshot skill); licenses (Inter, Lucide) in the packages' license files and INSTALL/README if needed
  - Done: Inter and Lucide license texts in `THIRD-PARTY-LICENSES.txt` and the .deb copyright file; README screenshots (Home + Settings) and text, AppStream screenshots. Keyboard pass (2026-11-27): tabs and **Save replay** were not reachable and the dropdown list was mouse-only; fixed (focusable tabs and Save button, Up/Down in the dropdown, focus ring only for keyboard focus). Order Home: tabs → Save replay → Play → Show in folder; Settings: every control in reading order, the page scrolls to the focused one. Keys verified against settings.toml: slider ←/→, segmented ←/→, dropdown ↓, toggles Space, hotkey field Enter/Esc. 200 % (`SLINT_SCALE_FACTOR=2`, 1840×1280) and the 720×540 minimum: no clipping on any page
- [x ] Verified in-game: the maintainer uses the new window while playing (status, save, settings)
  - Maintainer: resize bug "verified … fixed. Did my manual test. Seems ok" (2026-11-27)

Out of scope: styling the tray *menu* (drawn by the desktop), clip library (dropped in M5), light theme.

## M6: Packaging

Goal: Ubuntu and Linux Mint users install Chronoclip from a **.deb**, everyone else runs the **AppImage**; both are downloaded from GitHub Releases and need no FFmpeg or build tools. First release: **v0.1.0**.

Both packages carry the same **LGPL FFmpeg** (built by us, shared libraries): the distros' FFmpeg differs in version between releases (Ubuntu 24.04 has 6.1, 26.04 newer), and Ubuntu's is a GPL build (`--enable-gpl --enable-libx264`) that we may not ship (AGENTS rule 7). With our own FFmpeg in `/usr/lib/chronoclip/`, **one .deb works on Ubuntu 22.04 / 24.04 / 26.04 and Mint 21 / 22 / 23**. X11, OpenGL, CUDA/NVENC (NVIDIA driver) and PulseAudio still come from the host.

- [x] Groundwork: app-menu entry (`chronoclip.desktop`), hicolor icons (16–512 px), AppStream metainfo, `chronoclip --version`; notifications and autostart use the installed icon
- [x] License files for binaries: third-party Rust licenses (`cargo-about`), FFmpeg's LGPL text, its exact source and build configuration, Slint attribution (already in About)
  - Rust crates: `packaging/licenses/generate.sh` (incl. the Slint royalty-free text that cargo-about 0.9 can't include). FFmpeg: `build.sh` writes `FFMPEG-LICENSE.txt` (source URL, SHA-256, configure flags) and the LGPL text; the release workflow must also attach the FFmpeg source tarball, as that file promises
- [x] LGPL FFmpeg build script (`packaging/ffmpeg/`): a fixed FFmpeg release, shared libs, no GPL/nonfree parts, only what Chronoclip uses (`h264_nvenc` + CUDA via `nv-codec-headers`, `aac`, mpegts/mp4 muxing and demuxing); the app is compiled against it
- [x] Build environment: everything built in an Ubuntu 22.04 container (oldest glibc we support), in CI; locally only with the maintainer's OK (Docker needs sudo)
  - `.github/workflows/package.yml`: Ubuntu 22.04 container on GitHub Actions (Rust, FFmpeg and cargo-about cached); result `Depends: libc6 (>= 2.35)`, i.e. 22.04's glibc
- [x] .deb: `packaging/deb/build.sh` (plain `dpkg-deb`; cargo-deb not needed), binary in `/usr/bin`, FFmpeg in `/usr/lib/chronoclip/` (rpath), desktop entry, icons, metainfo, licenses; depends on the X11/GL/PulseAudio/fontconfig/xkbcommon runtime packages, recommends the NVIDIA driver; installs and removes cleanly with apt
  - 8.5 MB. CI installs it on Ubuntu 22.04, 24.04 and 26.04, runs it (bundled FFmpeg loaded, all runtime libraries and `xdg-open` present, menu entry, icon, metainfo) and removes it without leftovers (2026-11-26). Ubuntu's minimized container images skip `/usr/share/doc`, so license files are checked in the package. Install on the maintainer's Mint machine and in-game test: see Verified
- [x] AppImage: same binary and FFmpeg plus the other non-system libraries, static AppImage runtime (no `libfuse2` needed on Ubuntu 24.04+); autostart points at the `.AppImage` file (`$APPIMAGE`), not the temporary mount
  - `packaging/appimage/build.sh` (appimagetool 1.9.1, type2-runtime 20251108, both pinned by SHA-256): 12 MB; only FFmpeg is bundled (the binary links just glibc, libgcc and fontconfig). On Mint: records and saves H.264 + AAC from its mount with the bundled FFmpeg; the autostart entry is repointed from the mount to `$APPIMAGE`. CI runs it on Ubuntu 22.04, Debian 12, Fedora, Arch and openSUSE
- [x] GitHub Releases: pushing a tag `v*` builds the .deb and the AppImage in CI and attaches them with SHA-256 checksums and release notes
  - `.github/workflows/release.yml` + `packaging/check-release.sh` + `CHANGELOG.md` + the `release` skill. Also attaches the FFmpeg source tarball. Dry run by hand (`gh workflow run release.yml -f version=0.1.0`) publishes nothing. Dry run skipped (maintainer). **v0.1.0 published 2026-11-26**: https://github.com/etse/chronoclip/releases/tag/v0.1.0 (all 11 jobs green; downloads match SHA256SUMS)
- [x] Docs: INSTALL.md user section (.deb and AppImage, NVIDIA requirement), README install section, architecture.md (packaging decisions), AGENTS.md (how to cut a release)
- [x] Verified: the .deb installs, records and saves on the maintainer's Mint machine (in-game) and removes cleanly; the AppImage records and saves on Mint; both start in Ubuntu 22.04 and 26.04 containers and the AppImage in a Fedora container in CI
  - .deb on the maintainer's Mint machine: "it works" (2026-11-26). AppImage (released v0.1.0) on Mint: "install is verified, its ok" (2026-11-26). AppImage runs on Ubuntu 22.04, Debian 12, Fedora, Arch and openSUSE in CI

Out of scope: Flatpak and AUR (dropped: the sandbox fights window capture, process detection and the global hotkey; Flathub would be the reason to revisit), AMD/Intel and Wayland (M9).

## M5: Polish

Goal: you notice every save, even in-game; you can see at a glance whether Chronoclip is recording; it starts by itself at login.

- [x] Save chime: a short chime generated in code (no sound file), played through the sound server (`libpulse`, already loaded) when a clip is saved; a different short tone on a failed save. Setting "Play a sound when saving", **on** by default
- [x] Desktop notification (`org.freedesktop.Notifications` via `notify-rust`, no system dependency): "Clip saved · 10:00 · 1.4 GB" with **Play** and **Show in folder** actions; save errors notify too. Cinnamon hides notifications over fullscreen windows by default, so the chime is the in-game feedback
- [x] Tray icon shows state: red while recording, grey while waiting for WoW, warning icon on errors; tooltip unchanged
- [x] Start with desktop: settings switch that writes/removes `~/.config/autostart/chronoclip.desktop` (`Exec=<binary> --hidden`); default off; reflects the file's actual state
- [x] WoW detection: already in place since M2/M4 (waits for `Wow.exe`, starts and restarts by itself); verified with Retail (`_forever_`); Classic not tested
- [x] Docs: README (features, settings), architecture.md (notifications, chime), INSTALL.md only if dependencies change
- [x] Verified in-game: chime audible in WoW (borderless) on save; notification after alt-tab; tray icon colors; autostart after a re-login
  - Maintainer: "it seems good" (2026-11-26)

Out of scope: clip library (dropped for now), trimming or editing clips, uploading or sharing, visual redesign (M7).

## M4: Audio

Goal: clips have the game's sound in sync with the video (track 1, plays everywhere), and optionally the microphone as a separate track 2. Audio is captured with PipeWire, encoded to AAC and written into the same 2 s segments as the video, so saving stays a remux.

- [x] Decide the PipeWire binding (spike): `pipewire` crate (needs `libpipewire-0.3-dev` + clang to build) vs. loading `libpipewire-0.3` at runtime vs. FFmpeg's `pulse` input via pipewire-pulse. Criteria: per-app capture, latency/timestamps, build and runtime dependencies (AGENTS rule 9). Record the choice in architecture.md (an ADR if it adds a build dependency). → PulseAudio API (`libpulse.so.0` at runtime, served by pipewire-pulse): per-app monitor streams, no build dependency
- [x] `chronoclip-capture`: `AudioSource` with two modes: **game only** captures the WoW (Wine/Proton) output stream, found by its process like the window; **all sound** captures the default output's monitor (game + Discord, music, …). Game only falls back to all sound when no WoW stream exists. Follows the stream/device when it is recreated (device change, game restart)
- [x] `chronoclip-capture`: microphone source (default input), only while the setting is on
- [x] `chronoclip-core`: audio timeline logic (unit tested): map capture timestamps to the video clock, insert silence for gaps (game silent, stream recreated) so the track stays continuous and in sync
- [x] `chronoclip-encode`: AAC encoder (FFmpeg's built-in `aac`, LGPL), 48 kHz stereo, ~160 kbit/s; segment writer muxes video + 1 or 2 audio streams into the MPEG-TS segments, cut on video keyframes
- [x] Recorder: audio thread(s) → bounded channel → writer, sharing one clock with video; audio starts and ends with each video session; a capture failure (no PipeWire, stream gone) logs, keeps the track going with silence and retries, instead of stopping
- [x] Remux: copy all streams; rebase every stream with the same offset at segment joins so audio stays in sync; mark track 1 as default ("Game") and track 2 as "Microphone"; still works for video-only segments
- [x] Settings/UI: audio setting "Game only" (default) / "All sound" / "Off", and the existing "Record microphone" (default off); changes apply with a recorder restart; audio status (capturing WoW / all sound / unavailable) in the window
- [x] Docs: architecture.md (audio path, decisions, measurements), INSTALL.md (PipeWire runtime/dev packages), README (features, configuration)
- [x] A clip only contains the current game: when the game is started again (its window reappears, or a new process), the previous game's recording is deleted (after a running save). Not when the game closes, so its end can still be saved afterwards
- [x] Clips never overlap: a save deletes the saved segments, so the next clip starts where the previous one ended (requested in-game; verified with three saves in a row: 16 s, 10 s, 2 s)
- [x] Settings save automatically (no Save button): lists and switches at once, the slider on release, text fields on Enter or leaving the field; unchanged values restart nothing (requested in-game: "All sound" was picked but never saved)
- [x] Verified with a test tone: A/V offset < 1 frame at the start and end of a 45 min clip and across a window resize; clip plays with sound in mpv, Firefox and Discord; CPU cost of audio measured
  - In-game finding: WoW (Proton via Faugus) is a native PipeWire client; through pipewire-pulse its stream has only `application.name = "World of Warcraft"`, so "game only" first fell back to all sound. Fixed by also matching the application name / window title; verified against running WoW (`finds_running_wow_stream`)
  - Done: 40 s and 10 min clips (no drift, offset within ffplay's own sync), window closed/reopened mid-clip, *All sound* + mic tracks, mpv playback, CPU (+1.2 % of one core); see architecture.md. Clips play with sound in Firefox and Discord (maintainer).
  - Long run (2026-11-26): 46.5 min recording with a resize at 22.5 min. As designed, the clip starts at the resize, so it was 24 min: median offset 0.0 ms (−30 to +10 ms, ±1 frame), no drift (first minute −3.4 ms, last −0.7 ms), no gaps, in sync right after the resize. Accepted instead of a separate 45 min run; the test sink runs on the system clock, so device drift was covered by the 10 min headset run
- [x] Verified in-game: "Game only" has WoW sound without Discord/music; "All sound" includes Discord; mic track with own voice when enabled
  - "Game only" records WoW sound and leaves out Discord and YouTube; "All sound" includes browser audio; the mic track (track 2) has finger snaps as clear peaks over a quiet background (maintainer, 2026-11-20). Players, browsers and Discord play only track 1

Out of scope: volume controls, mixing the mic into the game track, picking individual apps or devices (later, with M8), noise suppression.

## M3: Save replay

Goal: pressing the hotkey (default `Ctrl+F9`), clicking Save in the window or tray, or running `chronoclip save` writes the last N minutes as an MP4 to the output folder within seconds, by stream copy, while recording continues.

- [x] `chronoclip-core`: pick the segments for a save: the newest ones covering the buffer length, starting at the oldest segment that still exists (unit tested)
- [x] `chronoclip-core`: clip file naming (`Chronoclip_YYYY-MM-DD_HH-MM-SS.mp4`, no overwrites on collision) and free-space check against the snapshot size (unit tested)
- [x] Segment pinning: segments in a running save are not deleted by pruning until the save finishes; pruning catches up afterwards so disk use stays bounded
- [x] Include the moment of the press: the save waits for the in-progress segment to finish (≤ 2 s) before taking the snapshot
- [x] `chronoclip-encode`: remux MPEG-TS segments → one MP4 (stream copy, rebased continuous timestamps, `+faststart`), written to a temp file and renamed when complete
- [x] SaveService in `app/`: runs on its own thread, never blocks capture or the UI; one save at a time (a second request while saving is rejected with a message)
- [x] Controller: `save_replay` (UI button, tray, IPC) triggers the save; `chronoclip save` returns the clip path or the error
- [x] Global hotkey on X11 (XGrabKey, ignoring NumLock/CapsLock state) behind the `GlobalHotkey` backend switch (`app/src/hotkey.rs`, X11 now, portal later); re-registered when the setting changes; a grab failure (key taken by another app) is shown in the UI
- [x] Feedback: last saved clip and save errors in the UI status and tray tooltip; "Open folder" for the output folder (desktop notifications come in M5)
- [x] Docs: architecture.md (save path, decisions), INSTALL.md if dependencies change, README usage
- [x] Verified: glxgears save at 5 and 45 min buffers (45 min: 7.3 GB clip in 6.0 s); duration and frame count match, every join frame-exact, seeks and decodes cleanly in FFmpeg; save time measured (see architecture.md)
- [x] Verified in-game: hotkey works while WoW (borderless) has focus; clip looks good; game stayed smooth in light play (heavy content like raids not tested yet)

Out of scope: audio (M4), notifications and clip library (M5), Wayland GlobalShortcuts portal (M9; `chronoclip save` is the fallback there). Trimming or editing clips is not planned.

## M2: Capture pipeline (X11 + NVIDIA)

Goal: while the app runs, the WoW window is continuously encoded with NVENC into ~2 s MPEG-TS segments in the buffer folder, and segments older than the buffer length are deleted.

- [x] `chronoclip-core`: segment index with retention-based pruning (unit tested)
- [x] `chronoclip-encode`: NVENC H.264 encoder (CUDA frames), keyframe-aligned GOP, segment writer
- [x] `chronoclip-capture`: find the WoW window (Proton `Wow.exe`), XComposite + GLX texture-from-pixmap capture
- [x] GPU path: GL blit → CUDA–GL interop → CUDA frame, no CPU readback
- [x] Recorder: capture/encode thread → bounded channel → writer thread; frame pacing at the configured FPS
- [x] App: start/stop recorder, restart on settings change, status in UI and tray tooltip
- [x] Buffer folder cleaned on start and exit
- [x] Verified: glxgears at 2560×1368 @ 60 fps, segments decode and join into an MP4, overhead measured (see architecture.md)
- [x] Verified in-game: WoW (DX12, borderless windowed, 2560×1440 @ 60) via Faugus: game stays smooth, ~3 frames skipped, segments play in mpv. Uses copy mode (see architecture.md).

Out of scope: saving clips (M3), audio (M4), VAAPI/software encoders (M9). A window resize restarts the recorder at the new size.

## Later milestones

Details are added when a milestone starts. See the [README roadmap](../README.md#roadmap) for one-line summaries.
