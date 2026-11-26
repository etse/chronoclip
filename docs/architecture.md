# Architecture

> Status: X11 + NVIDIA capture (M2), saving replays (M3) and audio (M4) are implemented; Wayland and VAAPI are still design. Update this document as the implementation lands.

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
┌────────────────┐  PCM + time    ┌───────────────┐                   │                  │
│ AudioCapture   │ ─────────────► │ AudioTimeline │ ────────────────► │                  │
│ (PipeWire via  │                │ → AAC encoder │                   └────────┬─────────┘
│  libpulse)     │                └───────────────┘                            │ writes
└────────────────┘                                                             │
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
| `SegmentIndex` | `chronoclip-core` | Ordered ring of finished segments (seq, stream, path, duration, size). Prunes segments older than the buffer length, selects segments for a save, and holds back pinned segments from deletion. |
| `ReplayBuffer` | `app/` (`buffer.rs`) | The `SegmentIndex` plus the segment being written, behind a mutex + condvar. Shared by the writer thread (adds segments, deletes pruned files) and saves. |
| Save | `app/` (`save.rs`), remux in `chronoclip-encode` | See [Saving a replay](#saving-a-replay). Runs on its own thread; capture keeps running. |
| `X11WindowCapture` | `chronoclip-capture` | Window capture into CUDA frames. Wayland (portal + PipeWire) comes later behind the same recorder interface. |
| `AudioCapture` | `chronoclip-capture` | One thread per audio track with a PulseAudio main loop; hands interleaved `f32` stereo at 48 kHz plus its arrival time to an `AudioSink`. See [Audio](#audio). |
| `AudioTimeline` | `chronoclip-core` | Places captured chunks on the session clock: silence for gaps, drops overlap, corrects device clock drift. |
| Audio tracks | `app/` (`audio.rs`) | `AudioSink` per track: timeline → `AacEncoder` → writer, timestamps relative to the current window session. |
| `Encoder` / `SegmentWriter` | `chronoclip-encode` | FFmpeg wrappers. NVENC with a closed GOP aligned to segment length so every segment starts with an IDR frame; AAC (FFmpeg's built-in encoder). The segment writer cuts on video keyframes and writes the audio streams into the same files. |
| App | `app/` | Slint UI, tray icon, global hotkey, single-instance IPC + CLI, lifecycle (start/stop, WoW detection), desktop notifications. |

### Linux integration

Implementation order follows the reference setup (Linux Mint Cinnamon on X11, NVIDIA, WoW DX12 via Faugus/Proton): **X11 column first**.

| Concern | X11 (first) | Wayland (later) |
|---|---|---|
| Video capture | `XCompositeRedirectWindow` on the WoW window → named pixmap → `GLX_EXT_texture_from_pixmap` texture → drawn with a full-screen-triangle shader into our own RGBA8 texture (NVIDIA can't attach pixmap textures to an FBO, so no blit) → registered with CUDA (`cuGraphicsGLRegisterImage`) → `cuMemcpy2D` into the encoder's CUDA frame → NVENC. Frames never touch the CPU. **Copy mode:** NVIDIA refuses `glXCreatePixmap` (BadAlloc) on the backing pixmap of windows rendered via Vulkan/DXVK (Wine/Proton games, Battle.net, Chrome); for those the X server first copies the window into a pixmap we own (`XCopyArea`, a server-side GPU copy) and that pixmap is bound instead. A resize or minimize ends the session; the recorder then starts a new one at the new size. | xdg-desktop-portal ScreenCast (`ashpd`) → PipeWire stream, DMA-BUF when available. Persist the portal **restore token** so the user picks the WoW window only once. |
| Audio | PulseAudio client API (`libpulse.so.0`, loaded at runtime), served by `pipewire-pulse`: the WoW stream alone via a monitor stream (`pa_stream_set_monitor_stream`), or `@DEFAULT_MONITOR@` for all sound; microphone from the default source | same (the portal is only needed for video) |
| Encoder | NVENC (NVIDIA, via FFmpeg `h264_nvenc` / `hevc_nvenc` / `av1_nvenc` with CUDA hw frames), VAAPI (AMD/Intel) | same |
| Global hotkey | `XGrabKey` on the root window, once per Caps Lock / Num Lock combination; own code in `chronoclip-capture` (`X11Hotkey`) with its own thread and X connection. A key taken by another client (`BadAccess`) is reported in the UI | GlobalShortcuts portal (KDE Plasma 6, GNOME 48+, Hyprland). Fallback: user binds `chronoclip save` in their desktop settings |
| Tray | Slint `SystemTrayIcon` (StatusNotifierItem); GNOME needs the AppIndicator extension | same |
| IPC (`chronoclip save`) | Unix socket in `$XDG_RUNTIME_DIR/chronoclip.sock` | same |
| Notifications | `org.freedesktop.Notifications` via `notify-rust` (pure-Rust D-Bus, zbus); **Show in folder** via `org.freedesktop.FileManager1.ShowItems`, falling back to `xdg-open` on the folder | same |
| Save chime | Generated in `chronoclip-core::chime` (two sine notes, 48 kHz mono), played with `libpulse-simple.so.0` (loaded at runtime) on its own thread | same |
| Autostart | `$XDG_CONFIG_HOME/autostart/chronoclip.desktop` (`Exec="<binary>" --hidden`), written/removed by the settings switch; repointed at startup if the binary moved | same |

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

## Saving a replay

Triggers: the hotkey, the window's **Save replay** button, the tray menu, and `chronoclip save` over IPC (which waits for the result and prints the clip path). All go through `Controller::start_save`; only one save runs at a time, a second request is refused with a message.

1. **Wait for the open segment** (≤ 2 s + slack): the writer reports which segment it is filling; the save waits until that one is finished, so the clip contains the moment the key was pressed. A recording session that ends (window hidden, closed or resized) finishes its segment right away.
2. **Select and pin:** the newest segments covering the buffer length, oldest first, stopping where the frame size changed (`Segment::stream`), since MP4 holds one set of codec parameters. Segments with `seq >= first` are pinned: pruning still takes them out of the index but defers deleting the files until the save ends, so disk use stays bounded right after.
3. **Check free space** in the clips folder (`statvfs`): clip size ≈ segment size, plus 5 % or 256 MB headroom.
4. **Remux** (`chronoclip_encode::remux_to_mp4`): stream copy into `.<name>.mp4.part`, then rename to `Chronoclip_YYYY-MM-DD_HH-MM-SS.mp4` (`_2`, `_3` on collision). The first segment is probed fully for the SPS/PPS; the rest are opened without `avformat_find_stream_info`, which would decode a frame per segment. Timestamps are rebased to start at 0; a file whose timestamps jump back (new encoder session) or more than 1 s forward is attached one frame after the previous packet. `+faststart` puts the `moov` atom first so clips start instantly in browsers and chat apps.
5. **Unpin**, which deletes segments that were pruned meanwhile, and report the result in the window, the tray tooltip and to the IPC caller.
6. **Consume:** after a successful save, the saved segments are deleted from the buffer, so the next clip starts where this one ended and clips never overlap (joining clips is a job for an editor). Segments recorded during the save stay. A save right after another one contains just the segment that was open at the press; with nothing at all left it says "Nothing new recorded since the last clip".

Stopping the recorder (quit, or a settings change that restarts it) waits for a running save before deleting the buffer.

**One game per clip:** when the game is started again (its window disappeared and a new one appears, or the window belongs to a new process), the recorder sends `Discard` to the writer, which deletes all segments of the previous game, after waiting for a running save. Closing the game doesn't delete anything, so the last minutes before quitting can still be saved; minimizing or resizing doesn't count as a restart.

### Measured (M3, RTX 4080, release build, glxgears 2560×1368 @ 60 fps)

| Clip | Segments | Size | Remux | Press → file ready |
|---|---|---|---|---|
| 20 s | 10 | 53 MB | 80 ms | 1.5 s |
| 5 min (full buffer) | 151 | 816 MB | 0.7 s | 1.3 s |
| 45 min (full buffer) | 1350 | 7.3 GB | 5.6 s | 6.0 s |

"Press → file ready" includes the wait for the open segment (0 to 2 s). Clips check out with ffprobe/ffmpeg: exact duration and frame count, every frame 1/60 s apart across all segment joins, one keyframe per segment, full decode without errors, `moov` first. The buffer stays bounded (1351 segments, 7.1 GB for 45 min) and is deleted on quit. Segments were still in the page cache (64 GB RAM), so the remux ran at ≈ 1.3 GB/s; from a cold cache it is bounded by the disk.

## Decisions made in M3

- **Global hotkey: own `XGrabKey` code**, not the `global-hotkey` crate: no extra dependency (we already load Xlib), and full control over Caps Lock / Num Lock and over reporting a key that is already taken. Lives in `chronoclip-capture`, the crate that holds the X11 FFI. Wayland will use the GlobalShortcuts portal (M9).
- **Clips include the moment of the press:** a save waits for the open segment (up to one segment length) instead of saving instantly without the last ≤ 2 s.
- **One save at a time:** a second request while saving is refused rather than queued.
- **A clip never spans a resolution change**; segments of separate sessions at the same size are joined, without the gap.
- **Slint software renderer** instead of femtovg (OpenGL): femtovg drew text on fractional pixels, so labels in some rows looked wavy. The software renderer snaps text to whole pixels, keeps OpenGL out of the UI (only our GLX capture uses it) and makes the build smaller. A small settings window doesn't need GPU rendering.
- **`+faststart`** although it rewrites the file once at the end: compatibility (browsers, Discord) over a few seconds on very long clips.

## Audio

Up to two audio tracks, chosen in the settings: **Sound** = *Game only* (default), *All sound* or *Off*, plus an optional **microphone** track. Track 1 is the default track (players and Discord play only that one).

1. **Capture** (`AudioCapture`, one thread per track): a non-threaded PulseAudio main loop with a 10 ms fragment size. *Game only* lists the playback streams (sink inputs) every 2 s and records the game's stream as a monitor stream: the one whose `application.process.id` is the recorded window's process, whose process/application name is a WoW executable, or whose application name is "World of Warcraft…" or the window title. The name match is needed in practice: WoW under Proton/Faugus plays through Wine as a *native PipeWire* client, and pipewire-pulse then shows the stream with only `application.name = "World of Warcraft"`, no process id (the pid is only on the PipeWire client object): only the game, without voice chat or music. While no game stream exists (game not started, or it has no sound device yet), it records the default output's monitor instead and says so in the status line ("all sound (game audio not found)"), and switches over when the stream appears. *All sound* records the default output's monitor and reconnects when the default output changes. If the sound server is missing or fails, the track keeps going with silence and the capture retries every 3 s.
2. **Timeline** (`AudioTimeline`, unit tested): every chunk's position on the session clock comes from its arrival time. The first chunk is placed exactly; after that the samples are written back to back and only corrected when they are clearly off by more than 20 ms: a gap of 60 ms or more (stream paused) becomes silence, audio that arrived earlier than its position (device clock fast) is cut, and lag (device clock slow) is filled once the *smallest* offset of the last 100 chunks exceeds 20 ms, because the sound server delivers in ~21 ms quanta and arrival times are only ever late. With no audio at all for 200 ms the track is filled with silence, so it never stalls.
3. **Encode** (`AacEncoder`): AAC-LC, 48 kHz stereo, 160 kbit/s, FFmpeg's built-in encoder. A new encoder per window session; its timestamps count from the instant of video frame 0 of that session, so audio and video share one clock. The priming packet (before 0) is dropped, since MPEG-TS can't store negative timestamps.
4. **Write**: the packets go through the same bounded channel to the writer, tagged with their session; packets of a session that already ended are dropped. The segment writer puts them into the open segment (cut on video keyframes) in arrival order, without interleaving.
5. **Remux**: all streams are copied. The video decides each segment's offset on the clip timeline; the segment's audio gets the same offset, so it stays in sync across joins and window sessions. Audio before the clip start or overlapping already written audio is dropped. Tracks are named ("Game audio" / "All sound", "Microphone").

### Measured (M4, RTX 4080, release build)

| | |
|---|---|
| A/V sync, ffplay test video with a flash and a beep every second, *Game only*, 40 s | audio 1 to 18 ms before the picture (the video is sampled at 60 Hz, so ±1 frame); one timeline correction (the start) |
| Same, 10 min clip | 600 flashes, 600 beeps, beeps exactly 1.000 s apart in the clip (except where the test video loops); offset −40 to +19 ms, shifting only when ffplay loops its file (ffplay's own sync); two timeline corrections in 10 min (start, and 10 ms at 9:26): the USB headset's clock is within ~35 ppm |
| Same, 46.5 min recording with a window resize at 22.5 min (virtual null sink) | clip starts at the resize (by design): 24 min, 1440 flashes and beeps, all exactly 1 s apart; offset median 0.0 ms, −30 to +10 ms; no drift (first minute −3.4 ms, last −0.7 ms); saved in 1.0 s |
| *All sound* + microphone, window closed and reopened mid-clip | both tracks −21 to +9 ms, also after the new session; the two sessions join without the 3 s gap |
| CPU (1280×720 @ 60 test video, 120 s) | 1.9 % of one core without sound, 3.2 % with *Game only*: sound costs ~1.2 % of one core |

Clips decode without errors in FFmpeg and play in mpv; `moov` is first. Not checked yet: Firefox and Discord upload.

## Decisions made in M4

- **PulseAudio client API instead of the native PipeWire API.** `pipewire-pulse` is installed on every desktop that runs PipeWire, the API records a single application's stream (monitor streams), and `libpulse.so.0` can be loaded at runtime like X11 and CUDA, so no new build dependency (the `pipewire` crate needs `libpipewire-0.3-dev` + bindgen). It also works on plain PulseAudio systems. The native API can come later behind `AudioCapture` if needed.
- **Game stream found by process id or name**: the same process as the captured window, a WoW executable name, or the application name "World of Warcraft" / the window title (what native-PipeWire Wine streams carry; verified in-game). Falls back to all sound rather than silence, so a clip never loses sound just because detection failed; the log then lists the streams it saw.
- **AAC in the segments, not Opus:** MP4 + AAC plays everywhere (browsers, Discord, phones) without remuxing again.
- **Arrival-time timestamps with drift correction** instead of resampling: simple and in sync within a frame; the cost is a short cut or silence every half minute or so on devices whose clock is far off. Resampling can replace it if that turns out audible.
- **Microphone as a separate track**, not mixed into the game track: the clip keeps both editable. Discord and browsers only play track 1, so the mic isn't heard there (mixing is not planned for M4).

## Decisions made in M5

- **Chime as the in-game feedback**, notifications second: Cinnamon (and most desktops) hide notifications over fullscreen windows, and borderless WoW counts as fullscreen. The chime is synthesized, so there is no sound file to ship or license. It goes to the default output, so *All sound* records it.
- **`notify-rust` pinned to 4.18**: 4.19 moved to zbus 6, while Slint (accesskit) uses zbus 5; staying on 4.18 keeps one zbus in the binary.
- **Autostart is the file, not a setting**: the switch reads whether `~/.config/autostart/chronoclip.desktop` exists, so it can't disagree with what the desktop does. Entries without our `X-Chronoclip-Autostart` marker are never rewritten.
- **Tray state from the recorder status**: three pre-rendered icons (red, grey, amber dot), switched in `tray.slint`.
- **No clip library**: dropped from M5; the output folder and the notification's **Show in folder** cover it for now.

## Packaging (M6)

- **Own LGPL FFmpeg in every package** (`packaging/ffmpeg/build.sh`): FFmpeg 8.1.3, shared libraries, `--disable-everything` plus only `h264_nvenc` (CUDA through nv-codec-headers n12.1.14.1 → NVIDIA driver 530+), the AAC encoder and decoder, MPEG-TS and MP4 muxers, MPEG-TS and MOV demuxers, the h264/aac parsers and three bitstream filters. 3.1 MB, depends only on glibc. The script fails if `CONFIG_GPL` or `CONFIG_NONFREE` is set. Why not the distro's: different versions per Ubuntu release (one .deb for all), and Ubuntu's build is GPL (`--enable-libx264`).
- **The AAC decoder is needed** although nothing is decoded: opening an MPEG-TS segment (saving a clip) only learns the audio's sample rate and channels from it.
- **Runpaths instead of wrapper scripts:** the binary has `$ORIGIN/../lib/chronoclip` (same layout in the .deb and the AppImage), each FFmpeg library has `$ORIGIN`. Set in FFmpeg's generated `config.mak`, because configure loses the `$` of `--extra-ldsoflags`; the script checks the result with `readelf`.
- **Verified** (RTX 4080): all 84 tests including the GPU, audio and hotkey tests pass against the bundled FFmpeg; an installed-layout binary loads only the bundled libraries and saves clips with H.264 + AAC.
- **.deb with plain `dpkg-deb`** (`packaging/deb/build.sh`), no cargo-deb: dependencies are what `dpkg-shlibdeps` finds in the binary (glibc, libgcc, fontconfig) plus the libraries the app *loads at runtime*, listed by hand from what a recording session actually maps (X11/xcb/xkbcommon for Slint/winit, Xcomposite + GL for capture, PulseAudio, xdg-utils). The NVIDIA driver can't be a dependency (its package name changes per version); the description names it.
- **AppImage without linuxdeploy** (`packaging/appimage/build.sh`): the only non-system library is our FFmpeg, so the AppDir is the staged layout plus desktop files; appimagetool 1.9.1 with the static type2 runtime (needs `fusermount3`, not `libfuse2`). Inside an AppImage the binary runs from a temporary mount, so autostart uses `$APPIMAGE`.
- **Built in Ubuntu 22.04** (CI container) for the glibc 2.35 baseline: one .deb for Ubuntu 22.04–26.04 / Mint 21–23, one AppImage for current distros. CI installs or runs both on 8 systems for every packaging change and every release.
- **Releases from tags** (`.github/workflows/release.yml`): `packaging/check-release.sh` makes the tag, `Cargo.toml`, the metainfo `<release>` and `CHANGELOG.md` agree; the FFmpeg source tarball is attached because the LGPL notice in both packages promises it. v0.1.0 published 2026-11-26.

## Open questions

- Cinnamon "unredirects" fullscreen windows for performance. WoW's borderless "Windowed (Fullscreen)" mode may still be flagged `_NET_WM_STATE_FULLSCREEN` and get unredirected. Our own `XCompositeRedirectWindow` should keep an off-screen pixmap regardless; verify this in M2. Exclusive fullscreen is lower priority.
- Wayland zero-copy: PipeWire DMA-BUF → VAAPI surface import; NVIDIA (DMA-BUF → CUDA → NVENC).
