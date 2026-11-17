# Chronoclip

> An open source, lightweight **replay buffer for World of Warcraft**. Think NVIDIA ShadowPlay "Instant Replay", but vendor-neutral, open, and built around WoW.

Chronoclip continuously records your gameplay to a rolling buffer on disk. When something worth keeping happens (a boss kill, a clutch arena win, a hilarious wipe) press your hotkey (default **`Ctrl+F9`**) and the last *N* minutes are saved as a normal video file.

> **Status:** 🚧 Pre-alpha. The repository is scaffolded but the app is not implemented yet. See [Roadmap](#roadmap).

---

## Features (planned)

- **Always-on replay buffer**: records continuously to a temporary location and discards old footage automatically.
- **Save on hotkey**: a configurable global hotkey (default `Ctrl+F9`) saves the buffered footage.
- **Configurable buffer length**: from **5 to 45 minutes** (default **10**).
- **Instant saves**: saving remuxes already-encoded segments into an MP4 with no re-encoding, so it takes seconds, not minutes.
- **Hardware encoding**: NVENC (NVIDIA), AMF (AMD) and Quick Sync (Intel), with a software (x264) fallback.
- **Game + optional microphone audio.**
- **Low overhead**: native Rust core, no bundled Chromium. The UI uses the OS webview.
- **Safe for your account**: captures the window through official OS capture APIs. Chronoclip **never** injects into, hooks, or reads the memory of the game process.
- **System tray app** with a clean settings UI.

## Tech stack

| Layer | Choice | Why |
|---|---|---|
| Desktop shell | [Tauri 2](https://tauri.app) | Native app with a web UI, but uses the system webview (WebView2 / WebKit), so the footprint is ~10x smaller than Electron. |
| Core / backend | Rust | Predictable performance, no GC pauses, safe concurrency for the capture → encode → write pipeline. |
| UI | Svelte 5 + TypeScript + Vite | Small, fast runtime with an excellent dev loop. |
| Video capture | Windows Graphics Capture (Windows) | Official, injection-free window capture. macOS (ScreenCaptureKit) and Linux (PipeWire) are planned. |
| Audio capture | WASAPI loopback (+ mic) | System/game audio without drivers. |
| Encoding / muxing | FFmpeg (libav*, LGPL, dynamically linked) | Hardware encoders, segment muxing, lossless concat on save. |
| Global hotkey | `tauri-plugin-global-shortcut` | Works while the game is focused. |

The reasoning is recorded in [docs/adr/0001-tech-stack.md](docs/adr/0001-tech-stack.md). The design is in [docs/architecture.md](docs/architecture.md).

## How it works (short version)

```
 WoW window ──► Capture (WGC) ──► HW encoder (NVENC/AMF/QSV) ──► 2 s segments in temp dir
 Game audio ──► WASAPI loopback ─┘                                (rolling, oldest deleted)
                                                                          │
                                       Ctrl+F9 ──► snapshot last N min ───┘
                                                     └──► remux (no re-encode) ──► ~/Videos/Chronoclip/*.mp4
```

## Platform support

| Platform | Status |
|---|---|
| Windows 10 (1903+) / 11 | Primary target |
| macOS 12.3+ | Planned (ScreenCaptureKit) |
| Linux (WoW via Wine/Proton/Lutris) | Planned (PipeWire / xdg-desktop-portal) |

## Getting started (development)

> The app code does not exist yet. These are the intended prerequisites and commands.

**Prerequisites**

- [Rust](https://rustup.rs) (stable, see `rust-toolchain.toml` once added)
- [Node.js](https://nodejs.org) 22 LTS and [pnpm](https://pnpm.io)
- [Tauri 2 prerequisites](https://tauri.app/start/prerequisites/) for your OS
- FFmpeg shared libraries (dev headers), LGPL build

**Commands (once implemented)**

```sh
pnpm install          # install frontend dependencies
pnpm tauri dev        # run the app in dev mode
pnpm tauri build      # produce an installer
cargo test --workspace
pnpm test
```

## Configuration

| Setting | Default | Range / notes |
|---|---|---|
| Save hotkey | `Ctrl+F9` | Any global key combination |
| Buffer length | 10 min | 5 to 45 min |
| Buffer location | OS temp dir (`%TEMP%\chronoclip\buffer`) | Should be on a fast SSD |
| Output folder | `Videos/Chronoclip` | |
| Resolution / FPS | Native / 60 | 30, 60, 120, 144 |
| Encoder | Auto (best available HW) | NVENC, AMF, QSV, x264 |
| Bitrate / quality | Auto | CQP/CRF or CBR |
| Microphone | Off | Separate track when enabled |

**Disk usage tip:** at 1080p60 and ~20 Mbit/s, a 10 minute buffer uses about 1.5 GB. A 45 minute buffer at the same bitrate uses about 6.8 GB.

## Roadmap

1. **M0, scaffold** ✅ repo, docs, agent instructions
2. **M1, skeleton app**: Tauri + Svelte shell, tray icon, settings persisted to disk
3. **M2, capture pipeline (Windows)**: WGC capture → HW encode → rolling segments
4. **M3, save replay**: global hotkey → lossless remux of the last N minutes to MP4
5. **M4, audio**: game audio loopback, optional mic track
6. **M5, polish**: notifications, clip library view, auto-start with Windows, auto-detect WoW running
7. **M6, other platforms**: Linux (PipeWire), macOS (ScreenCaptureKit)

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md). The repository is set up for AI coding agents too (Claude Code, opencode, and others); see [AGENTS.md](AGENTS.md).

## License

[MIT](LICENSE). FFmpeg is used under the LGPL via dynamic linking.

## Disclaimer

Chronoclip is a fan-made project and is not affiliated with or endorsed by Blizzard Entertainment. World of Warcraft is a trademark of Blizzard Entertainment, Inc.
