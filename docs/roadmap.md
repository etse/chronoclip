# Roadmap

Status: ✅ done · 🚧 in progress · ⬜ not started

| # | Milestone | Status |
|---|---|---|
| M0 | Scaffold: repo, docs, agent instructions | ✅ |
| M1 | Skeleton app: Slint window, tray, settings, single instance, `chronoclip save/show/quit` | ✅ |
| M2 | Capture pipeline (X11 + NVIDIA) | ✅ |
| M3 | Save replay: global hotkey, lossless remux to MP4 | ✅ |
| M4 | Audio: game audio via PipeWire, optional mic track | ✅ |
| M5 | Polish: save notification and chime, tray state, start with desktop | 🚧 |
| M6 | Packaging: .deb, AppImage, Flatpak, AUR | ⬜ |
| M7 | Better-looking UI (nice to have): visual redesign, theming, icons | ⬜ |
| M8 | Other games (long term): record any game, not just WoW: window picker, game detection, per-game clips | ⬜ |
| M9 | Wayland + AMD/Intel: portal/PipeWire capture, GlobalShortcuts portal, VAAPI | ⬜ |

## M5: Polish

Goal: you notice every save, even in-game; you can see at a glance whether Chronoclip is recording; it starts by itself at login.

- [x] Save chime: a short chime generated in code (no sound file), played through the sound server (`libpulse`, already loaded) when a clip is saved; a different short tone on a failed save. Setting "Play a sound when saving", **on** by default
- [x] Desktop notification (`org.freedesktop.Notifications` via `notify-rust`, no system dependency): "Clip saved · 10:00 · 1.4 GB" with **Play** and **Show in folder** actions; save errors notify too. Cinnamon hides notifications over fullscreen windows by default, so the chime is the in-game feedback
- [ ] Tray icon shows state: red while recording, grey while waiting for WoW, warning icon on errors; tooltip unchanged
- [ ] Start with desktop: settings switch that writes/removes `~/.config/autostart/chronoclip.desktop` (`Exec=<binary> --hidden`); default off; reflects the file's actual state
- [ ] WoW detection: already in place since M2/M4 (waits for `Wow.exe`, starts and restarts by itself); check WoW Classic if available, then tick off
- [ ] Docs: README (features, settings), architecture.md (notifications, chime), INSTALL.md only if dependencies change
- [ ] Verified in-game: chime audible in WoW (borderless) on save; notification after alt-tab; tray icon colors; autostart after a re-login

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
