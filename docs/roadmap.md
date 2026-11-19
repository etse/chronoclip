# Roadmap

Status: ✅ done · 🚧 in progress · ⬜ not started

| # | Milestone | Status |
|---|---|---|
| M0 | Scaffold: repo, docs, agent instructions | ✅ |
| M1 | Skeleton app: Slint window, tray, settings, single instance, `chronoclip save/show/quit` | ✅ |
| M2 | Capture pipeline (X11 + NVIDIA) | ✅ |
| M3 | Save replay: global hotkey, lossless remux to MP4 | 🚧 |
| M4 | Audio: game audio via PipeWire, optional mic track | ⬜ |
| M5 | Polish: notifications, clip library, autostart, WoW auto-detection | ⬜ |
| M6 | Wayland + AMD/Intel: portal/PipeWire capture, GlobalShortcuts portal, VAAPI | ⬜ |
| M7 | Packaging: .deb, AppImage, Flatpak, AUR | ⬜ |
| M8 | Better-looking UI (nice to have): visual redesign, theming, icons | ⬜ |

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
- [ ] Verified: glxgears save at 5 and 45 min buffers; clip plays and seeks in mpv and a browser, duration matches, no gap or stutter at segment joins; save time measured
- [ ] Verified in-game: hotkey works while WoW (borderless) has focus, game stays smooth during a save

Out of scope: audio (M4), notifications and clip library (M5), Wayland GlobalShortcuts portal (M6; `chronoclip save` is the fallback there). Trimming or editing clips is not planned.

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

Out of scope: saving clips (M3), audio (M4), VAAPI/software encoders (M6). A window resize restarts the recorder at the new size.

## Later milestones

Details are added when a milestone starts. See the [README roadmap](../README.md#roadmap) for one-line summaries.
