# Chronoclip

> An open source, lightweight **replay buffer for World of Warcraft on Linux**. Think NVIDIA ShadowPlay "Instant Replay", but native to Linux, vendor-neutral, and built around WoW running through Wine / Proton / Lutris.

Chronoclip continuously records your gameplay to a rolling buffer on disk. When something worth keeping happens (a boss kill, a clutch arena win, a hilarious wipe) press your hotkey (default **`Ctrl+F9`**) and the last *N* minutes are saved as a normal video file.

> **Status:** 🚧 Pre-alpha. The repository is scaffolded but the app is not implemented yet. See [Roadmap](#roadmap).

## Why

Windows players have ShadowPlay, ReLive, Medal and friends. On Linux the options are either general-purpose (OBS) or CLI-first. Chronoclip is a small, focused, native desktop app for one job: *always be recording WoW, save the good bits with one key*. It is built by a Linux WoW player for their own use and shared in case it is useful to others.

## Features (planned)

- **Always-on replay buffer**: records continuously to a buffer directory and discards old footage automatically.
- **Save on hotkey**: a configurable global hotkey (default `Ctrl+F9`) saves the buffered footage. Works on X11 and on Wayland (via the GlobalShortcuts portal).
- **Configurable buffer length**: from **5 to 45 minutes** (default **10**).
- **Instant saves**: saving remuxes already-encoded segments into an MP4 with no re-encoding, so it takes seconds, not minutes.
- **Hardware encoding**: VAAPI (AMD / Intel) and NVENC (NVIDIA).
- **Game + optional microphone audio** via PipeWire.
- **Fully native and lightweight**: Rust all the way down, native UI with [Slint](https://slint.dev). No Electron, no webview.
- **X11 and Wayland**: X11 first (Linux Mint, and others), Wayland via portals.
- **Safe for your account**: captures through the desktop's official screen-capture APIs. Chronoclip **never** injects into, hooks, or reads the memory of the game process.
- **Tray app** that keeps recording when the window is closed, plus a `chronoclip save` CLI command.

## Tech stack

| Layer | Choice |
|---|---|
| Language | Rust (single Cargo workspace, no Node / JS toolchain) |
| UI | Slint (native, GPU or software rendered) |
| Video capture | XComposite → OpenGL texture (X11, GPU-only path), xdg-desktop-portal ScreenCast + PipeWire (Wayland) |
| Audio capture | PipeWire |
| Encoding / muxing | FFmpeg (libav*, LGPL, dynamically linked): NVENC (via CUDA), VAAPI |
| Global hotkey | GlobalShortcuts portal (Wayland), X11 key grab, fallback `chronoclip save` CLI |
| Tray | Slint `SystemTrayIcon` (StatusNotifierItem) |

The reasoning is recorded in [docs/adr/](docs/adr/). The design is in [docs/architecture.md](docs/architecture.md).

## How it works (short version)

```
 WoW window ──► XComposite / PipeWire ──► NVENC / VAAPI ──► 2 s segments in ~/.cache
 Game audio ──► PipeWire ──────────────┘                     (rolling, oldest deleted)
                                                                    │
            Ctrl+F9 / `chronoclip save` ──► take last N min ────────┘
                                              └──► remux (no re-encode) ──► ~/Videos/Chronoclip/*.mp4
```

## Requirements

| | |
|---|---|
| Display server | X11 (primary) or Wayland |
| Global hotkey on Wayland | A compositor with the GlobalShortcuts portal: KDE Plasma 6, GNOME 48+, Hyprland. Otherwise bind `chronoclip save` in your desktop's keyboard settings |
| Screen capture on Wayland | xdg-desktop-portal with a backend for your desktop (one-time permission prompt) |
| Audio | PipeWire |
| GPU | Any with VAAPI (AMD / Intel) or NVENC (NVIDIA) |
| Tray icon | StatusNotifierItem support (KDE yes; GNOME needs the AppIndicator extension) |

### Reference setup

Chronoclip is developed and tested first on the maintainer's machine. Other setups should work but are best-effort until someone tests them.

| | |
|---|---|
| Distro | Linux Mint 22 (Cinnamon, X11), Ubuntu 24.04 base |
| GPU | NVIDIA, proprietary driver |
| WoW | Retail, DirectX 12 (vkd3d-proton), launched with [Faugus Launcher](https://github.com/Faugus/faugus-launcher) (UMU / Proton) |
| Display mode | Borderless windowed ("Windowed (Fullscreen)") |

## Getting started (development)

- [Rust](https://rustup.rs) stable (`rust-toolchain.toml` picks it automatically)
- Build dependencies. On Linux Mint / Ubuntu / Debian:
  ```sh
  sudo apt install pkg-config libfontconfig1-dev libxkbcommon-dev
  ```
  On Arch: `sudo pacman -S --needed pkgconf fontconfig libxkbcommon`

  The capture pipeline (M2) will add FFmpeg, PipeWire and CUDA/VAAPI dev packages.

```sh
cargo run -p chronoclip            # run the app (shows the window of a running instance if there is one)
cargo run -p chronoclip -- save    # save | show | quit, sent to the running instance
cargo test --workspace             # run tests
```

Settings are stored in `~/.config/chronoclip/settings.toml`.

## Configuration

| Setting | Default | Range / notes |
|---|---|---|
| Save hotkey | `Ctrl+F9` | Any key combination |
| Buffer length | 10 min | 5 to 45 min |
| Buffer location | `~/.cache/chronoclip/buffer` | Must be on disk (SSD), **not** a RAM-backed tmpfs like `/tmp` or `$XDG_RUNTIME_DIR` on many distros |
| Output folder | `~/Videos/Chronoclip` (XDG videos dir) | |
| Resolution / FPS | Native / 60 | 30, 60, 120, 144 |
| Encoder | Auto (best available) | VAAPI, NVENC, software fallback |
| Quality | Auto | Constant quality or constant bitrate |
| Microphone | Off | Separate audio track when enabled |

**Disk usage tip:** at 1080p60 and ~20 Mbit/s, a 10 minute buffer uses about 1.5 GB. A 45 minute buffer at the same bitrate uses about 6.8 GB.

## Roadmap

1. **M0, scaffold** ✅ repo, docs, agent instructions
2. **M1, skeleton app** ✅ Slint window, tray icon, settings persisted to disk, single instance + `chronoclip save` IPC
3. **M2, capture pipeline (X11 + NVIDIA)**: XComposite window capture → GL texture → CUDA → NVENC → rolling segments
4. **M3, save replay**: X11 global hotkey → lossless remux of the last N minutes to MP4
5. **M4, audio**: game audio via PipeWire, optional mic track
6. **M5, polish**: notifications, clip library, autostart, WoW window auto-detection
7. **M6, Wayland + AMD/Intel**: portal/PipeWire capture, GlobalShortcuts portal, VAAPI
8. **M7, packaging**: .deb (Mint/Ubuntu), AppImage, Flatpak, AUR

## Non-goals

- **Windows / macOS support.** Great tools already exist there.
- Streaming, editing, or uploading clips.

## Prior art

- [gpu-screen-recorder](https://git.dec05eba.com/gpu-screen-recorder/about/): excellent Linux GPU recorder with a replay mode. Worth studying.
- [OBS Studio](https://obsproject.com) replay buffer.

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md). The repository is set up for AI coding agents too (Claude Code, opencode, and others); see [AGENTS.md](AGENTS.md).

## License

[MIT](LICENSE). FFmpeg is used under the LGPL via dynamic linking. The UI uses [Slint](https://slint.dev) under its Royalty-free Desktop License (attribution shown in the About page).

## Disclaimer

Chronoclip is a fan-made project and is not affiliated with or endorsed by Blizzard Entertainment. World of Warcraft is a trademark of Blizzard Entertainment, Inc.
