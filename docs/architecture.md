# Architecture

> Status: design, not yet implemented. Update this document as the implementation lands.

## Goals

- Continuous recording with **minimal impact on game FPS** (target: under 3% on a mid-range GPU with hardware encoding).
- **Bounded** disk and memory usage for buffers from 5 to 45 minutes.
- Saving a replay is **near-instant** and lossless (stream copy, no re-encode).
- Works on **X11** (first; reference setup is Linux Mint Cinnamon + NVIDIA) and **Wayland** (later), on NVIDIA, AMD and Intel GPUs.
- No interaction with the game process beyond what the desktop's capture APIs provide.

## Pipeline

```
┌────────────────┐  DMA-BUF /     ┌───────────────┐  encoded packets  ┌──────────────────┐
│ VideoSource    │  GL/CUDA tex.  │ Video encoder │ ────────────────► │                  │
│ (X11, portal + │ ─────────────► │ (NVENC/VAAPI) │                   │  Segment writer  │
│  PipeWire)     │  (bounded ch.) └───────────────┘                   │  (2 s segments)  │
└────────────────┘                                                    │                  │
┌────────────────┐  PCM frames    ┌───────────────┐                   │                  │
│ AudioSource    │ ─────────────► │ Audio encoder │ ────────────────► │                  │
│ (PipeWire)     │                │ (AAC/Opus)    │                   └────────┬─────────┘
└────────────────┘                └───────────────┘                            │ writes
                                                                               ▼
                                         ~/.cache/chronoclip/buffer/seg_000123.ts ...
                                         + in-memory SegmentIndex (ring)
                                                                               │
 hotkey / tray / `chronoclip save` ──► SaveService: snapshot ─► remux ─► ~/Videos/Chronoclip/<ts>.mp4
```

### Components

| Component | Crate | Responsibility |
|---|---|---|
| `Settings` | `chronoclip-core` | Load/save/validate config (TOML in `$XDG_CONFIG_HOME/chronoclip/`). Clamp buffer to 5..=45 min. |
| `SegmentIndex` | `chronoclip-core` | Ordered ring of finished segments (path, start PTS, duration, size). Prunes segments older than `buffer_len + margin` and deletes their files. |
| `SaveService` | `chronoclip-core` | On request: snapshot the index, close the current segment, pick segments covering the last N minutes (starting at a keyframe), remux into one MP4. Runs off the hot path; pinned segments aren't pruned until it finishes. |
| `VideoSource` / `AudioSource` traits | `chronoclip-capture` | `start(target)` → frame stream, `stop()`. Backends: X11 (first), portal+PipeWire (later). |
| `Encoder` / `SegmentWriter` | `chronoclip-encode` | FFmpeg wrappers. Probe and select NVENC / VAAPI / software; closed GOP aligned to segment length so every segment starts with an IDR frame. |
| App | `app/` | Slint UI, tray icon, global hotkey, single-instance IPC + CLI, lifecycle (start/stop, WoW detection), desktop notifications. |

### Linux integration

Implementation order follows the reference setup (Linux Mint Cinnamon on X11, NVIDIA, WoW DX12 via Faugus/Proton): **X11 column first**.

| Concern | X11 (first) | Wayland (later) |
|---|---|---|
| Video capture | `XCompositeRedirectWindow` on the WoW window → named pixmap → `GLX_EXT_texture_from_pixmap` texture → drawn with a full-screen-triangle shader into our own RGBA8 texture (NVIDIA can't attach pixmap textures to an FBO, so no blit) → registered with CUDA (`cuGraphicsGLRegisterImage`) → `cuMemcpy2D` into the encoder's CUDA frame → NVENC. Frames never touch the CPU. **Copy mode:** NVIDIA refuses `glXCreatePixmap` (BadAlloc) on the backing pixmap of windows rendered via Vulkan/DXVK (Wine/Proton games, Battle.net, Chrome); for those the X server first copies the window into a pixmap we own (`XCopyArea`, a server-side GPU copy) and that pixmap is bound instead. A resize or minimize ends the session; the recorder then starts a new one at the new size. | xdg-desktop-portal ScreenCast (`ashpd`) → PipeWire stream, DMA-BUF when available. Persist the portal **restore token** so the user picks the WoW window only once. |
| Audio | PipeWire: capture the WoW/Wine app stream node, or the default sink monitor | same |
| Encoder | NVENC (NVIDIA, via FFmpeg `h264_nvenc` / `hevc_nvenc` / `av1_nvenc` with CUDA hw frames), VAAPI (AMD/Intel) | same |
| Global hotkey | XGrabKey (`global-hotkey` crate) | GlobalShortcuts portal (KDE Plasma 6, GNOME 48+, Hyprland). Fallback: user binds `chronoclip save` in their desktop settings |
| Tray | Slint `SystemTrayIcon` (StatusNotifierItem); GNOME needs the AppIndicator extension | same |
| IPC (`chronoclip save`) | Unix socket in `$XDG_RUNTIME_DIR/chronoclip.sock` | same |
| Notifications | `org.freedesktop.Notifications` | same |

Software encoding fallback must be LGPL/BSD-compatible (e.g. OpenH264), never `libx264`. The UI warns that software encoding costs FPS.

## Key design decisions

### Disk-backed segmented buffer (not RAM)
A 45 minute buffer at 1440p can be 5 to 10 GB, too much for RAM. Short fixed-length segments (~2 s) on disk give:
- O(1) pruning (delete the oldest file),
- precise save boundaries (± one segment),
- crash resilience (finished segments are valid files).

The default location is `$XDG_CACHE_HOME/chronoclip/buffer`, because `/tmp` and `$XDG_RUNTIME_DIR` are RAM-backed tmpfs on many distros.

### Keyframe-aligned segments
GOP length equals segment length (e.g. 120 frames at 60 fps), so any suffix of segments concatenates cleanly without re-encoding.

### Save is a remux
Stream-copy concatenation into MP4 is I/O bound and takes seconds even for 45 minutes. Capture keeps running during a save.

### Threading
- Capture thread → bounded channel → encoder thread → segment writer.
- If the encoder falls behind, drop frames (and count them) rather than buffering unboundedly.
- The Slint event loop only receives status updates and sends commands. Closing the window to tray destroys it; recording continues headless.

### Capture target
**Borderless windowed is the primary supported mode** (it's what the maintainer uses, and it gives a stable window size and painless alt-tab). Exclusive fullscreen is best-effort.

Prefer capturing the WoW window over the full monitor. On X11, find it automatically: Proton/Wine windows for `Wow.exe` (WM_CLASS `wow.exe` / `steam_app_*`, or `_NET_WM_PID` → `/proc/<pid>/cmdline` containing `Wow.exe`); let the user pick a window manually if detection fails. The game's graphics API (DX12 → vkd3d-proton → Vulkan) doesn't matter, because we capture the X11 window, not the swapchain. On Wayland the user picks the window in the portal dialog the first time; the restore token remembers it.

## Disk usage estimate

`size ≈ bitrate (Mbit/s) × 60 × minutes / 8` MB

| Bitrate | 10 min | 45 min |
|---|---|---|
| 12 Mbit/s (1080p60, efficient) | ~0.9 GB | ~4.0 GB |
| 20 Mbit/s (1080p60, high) | ~1.5 GB | ~6.8 GB |
| 40 Mbit/s (1440p/4K) | ~3.0 GB | ~13.5 GB |

The UI should show this estimate when the user changes buffer length or quality.

## Measured (M2, RTX 4080, release build)

| | |
|---|---|
| Capture + GPU copy per frame | ~0.4 ms (1280×720, debug build) |
| CPU while recording 2560×1368 @ 60 fps | ~2% of one core |
| GPU / NVENC load | ~8% / ~9% |
| Memory | ~100 MB private; NVIDIA driver libraries and GPU mappings add ~330 MB shared/mapped |

## Decisions made in M2

- Segment container: **MPEG-TS**, 2 s, H.264 with SPS/PPS repeated at every IDR. Segments concatenate without re-encoding.
- FFmpeg bindings: **`ffmpeg-next`** for muxing, raw `ffi` for the CUDA hwcontext.
- X11 + NVIDIA capture: **GLX texture-from-pixmap → shader copy → CUDA interop**. No NvFBC.
- Libraries (X11, GLX, GL, CUDA) are loaded at runtime with `libloading`/`x11-dl`, so users need no `-dev` packages.
- Recording uses the CUDA *primary* context, shared by FFmpeg and the capture code.

## Open questions

- Cinnamon "unredirects" fullscreen windows for performance. WoW's borderless "Windowed (Fullscreen)" mode may still be flagged `_NET_WM_STATE_FULLSCREEN` and get unredirected. Our own `XCompositeRedirectWindow` should keep an off-screen pixmap regardless; verify this in M2. Exclusive fullscreen is lower priority.
- Wayland zero-copy: PipeWire DMA-BUF → VAAPI surface import; NVIDIA (DMA-BUF → CUDA → NVENC).
- Packaging: Flatpak (natural fit for portals, but GPU encoder access needs care) vs AppImage vs AUR.
