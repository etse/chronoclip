# ADR 0002: Native Slint UI, Linux only

- **Status:** Accepted
- **Date:** 2026-11-17
- **Supersedes:** [ADR 0001](0001-tech-stack.md)

## Context

ADR 0001 chose Tauri 2 (system webview) with a Svelte frontend and treated Windows as the primary platform. Two things changed:

1. **Performance expectations.** Tauri doesn't bundle Chromium, but its webview (WebKitGTK on Linux) still costs tens of MB of RAM and extra processes, and a web stack adds a second toolchain (Node/pnpm). We want the app as lean as possible next to a running game.
2. **Target user.** The maintainer plays WoW on Linux (Wine/Proton) and builds Chronoclip for that use case, sharing it with other Linux players. Windows already has good replay tools (ShadowPlay, ReLive, Medal, OBS).

## Options considered (UI)

| Option | Pros | Cons |
|---|---|---|
| **Tauri 2 + web UI** (ADR 0001) | Flexible UI, big ecosystem | Webview overhead, two toolchains |
| **Slint** | Native rendering (GPU or software), polished widgets out of the box, declarative `.slint` markup with live preview, low memory, Wayland + X11 | Royalty-free license requires attribution; smaller ecosystem |
| **iced** | Pure Rust, MIT, used by COSMIC | Plainer default look, more styling work |
| **egui** | Very easy, MIT | Immediate-mode "tool" look, less suited to a polished app |
| **GTK4 (gtk-rs) / Qt** | Native on Linux desktops | Heavier, GObject/C++ binding friction |

## Decision

- **Linux only.** Windows and macOS are non-goals.
- **X11 + NVIDIA first**, matching the maintainer's setup (Linux Mint Cinnamon, which defaults to X11; NVIDIA proprietary driver; WoW DX12 via Faugus/Proton). Wayland and VAAPI follow behind the same abstractions.
- **Rust only**, single Cargo workspace. **Slint** for the UI, under the Royalty-free Desktop License (attribution in the About page).
- Capture via **XComposite → GL → CUDA** (X11) and **xdg-desktop-portal + PipeWire** (Wayland); audio via **PipeWire**.
- Encoding via **FFmpeg** (LGPL, dynamic) with **NVENC** and **VAAPI**.
- Global hotkey via XGrabKey on X11, the **GlobalShortcuts portal** on Wayland, and always a `chronoclip save` CLI fallback over a Unix socket.
- Unchanged from ADR 0001: disk-backed segmented buffer, save via remux, MIT license, no hooking of the game.

## Consequences

- Lower memory and CPU use than any webview approach; one toolchain for contributors.
- No cross-platform abstraction tax: backends target Linux APIs directly.
- The first working version targets X11 + NVIDIA; other setups are best-effort until tested.
- Wayland constraints shape the design: one-time capture permission prompt (mitigated by restore tokens), and global hotkeys depend on compositor portal support (mitigated by the CLI fallback).
- No GPL software encoder (`libx264`); software fallback uses an LGPL/BSD-compatible encoder.
- Windows/macOS ports would need a new ADR.
