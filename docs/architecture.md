# Architecture

> Status: design, not yet implemented. Update this document as the implementation lands.

## Goals

- Continuous recording with **minimal impact on game FPS** (target: under 3% on a mid-range GPU with hardware encoding).
- **Bounded** disk and memory usage for buffers from 5 to 45 minutes.
- Saving a replay is **near-instant** and lossless (stream copy, no re-encode).
- No interaction with the game process beyond what the OS capture APIs provide.

## Pipeline

```
┌──────────────┐  GPU texture   ┌───────────────┐  encoded packets  ┌──────────────────┐
│ Video capture │ ─────────────► │ Video encoder │ ────────────────► │                  │
│ (WGC)         │  (bounded ch.) │ (NVENC/AMF/   │                   │  Segment writer  │
└──────────────┘                └───────────────┘                   │  (2 s segments,  │
┌──────────────┐  PCM frames    ┌───────────────┐                   │  MPEG-TS/fMP4)   │
│ Audio capture │ ─────────────► │ Audio encoder │ ────────────────► │                  │
│ (WASAPI)      │                │ (AAC/Opus)    │                   └────────┬─────────┘
└──────────────┘                └───────────────┘                            │ writes
                                                                             ▼
                                                    temp dir: buffer/seg_000123.ts ...
                                                    + in-memory SegmentIndex (ring)
                                                                             │
                     hotkey ──► SaveService: snapshot index ─► remux ─► Videos/Chronoclip/<ts>.mp4
```

### Components

| Component | Crate | Responsibility |
|---|---|---|
| `Settings` | `chronoclip-core` | Load/save/validate config (TOML in the OS config dir). Clamp buffer to 5..=45 min. |
| `SegmentIndex` | `chronoclip-core` | Ordered ring of finished segments (path, start PTS, duration, size). Prunes segments older than `buffer_len + margin` and deletes their files. |
| `SaveService` | `chronoclip-core` | On request: snapshot the index, force-close the current segment, pick segments covering the last N minutes, starting at a keyframe, and remux into a single MP4 (`-c copy` equivalent). Runs off the hot path. |
| `CaptureSource` trait | `chronoclip-capture` | `start(target) -> stream of frames`, `stop()`. Backends: `wgc` (Windows), `pipewire` (Linux), `sck` (macOS). |
| `AudioSource` trait | `chronoclip-capture` | System loopback and optional microphone. |
| `Encoder` / `SegmentWriter` | `chronoclip-encode` | FFmpeg wrappers. Select best hardware encoder, configure closed GOP aligned to segment length so each segment starts with a keyframe. |
| App wiring | `src-tauri` | Tray icon, global hotkey, Tauri commands/events, lifecycle (start/stop recording, detect WoW window). |
| UI | `src/` | Settings, status (recording / buffer fill / disk use), recent clips. |

## Key design decisions

### Disk-backed segmented buffer (not RAM)
A 45 minute buffer at 1440p can be 5 to 10 GB, too much for RAM. Short fixed-length segments (~2 s) on disk give:
- O(1) pruning (delete the oldest file),
- precise save boundaries (± one segment),
- crash resilience (finished segments are valid files).

### Keyframe-aligned segments
The encoder uses a GOP equal to the segment length (e.g. 120 frames at 60 fps). Every segment starts with an IDR frame, so any suffix of segments can be concatenated without re-encoding.

### Save is a remux
Concatenating segments into MP4 with stream copy is I/O bound and takes seconds even for 45 minutes. The capture pipeline keeps running while a save is in progress; segments being saved are pinned (not pruned) until the save completes.

### Capture target
Capture the WoW window (`Wow.exe` / `WowClassic.exe`) via Windows Graphics Capture when found, falling back to the monitor it is on. Never hook the game.

### Threading
- Capture thread → bounded channel → encoder thread → segment writer.
- If the encoder falls behind, drop frames (and record a metric) rather than buffering unboundedly.
- The Tauri/UI thread only sends commands and receives status events.

## Disk usage estimate

`size ≈ bitrate (Mbit/s) × 60 × minutes / 8` MB

| Bitrate | 10 min | 45 min |
|---|---|---|
| 12 Mbit/s (1080p60, efficient) | ~0.9 GB | ~4.0 GB |
| 20 Mbit/s (1080p60, high) | ~1.5 GB | ~6.8 GB |
| 40 Mbit/s (1440p/4K) | ~3.0 GB | ~13.5 GB |

The UI should show this estimate when the user changes buffer length or quality.

## Open questions

- Segment container: MPEG-TS (robust, simple) vs fragmented MP4 (smaller overhead).
- FFmpeg bindings: `ffmpeg-next` vs `rsmpeg` vs a thin custom FFI layer.
- Zero-copy GPU path: WGC texture → NVENC/AMF via D3D11 hwcontext, to avoid a CPU readback.
- Linux: WoW under Wine/Proton through the PipeWire screencast portal, which requires a one-time user prompt.
