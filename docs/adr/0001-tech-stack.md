# ADR 0001: Tech stack

- **Status:** Superseded by [ADR 0002](0002-native-ui-and-linux-only.md)
- **Date:** 2026-11-17

## Context

Chronoclip must record gameplay continuously while a demanding game is running, so CPU, GPU, and memory overhead matter. It also needs a decent, modern desktop UI and should be approachable for open source contributors.

## Options considered

| Option | Pros | Cons |
|---|---|---|
| **Electron + Node** | Huge ecosystem, easy UI | Bundles Chromium (~150 MB+, 200 to 400 MB RAM idle); native capture still needs C++ addons |
| **Tauri 2 + Rust** | System webview (small binary, low RAM), Rust core for the hot path, web UI flexibility | WebView differences across OSes; Rust learning curve |
| **C# / .NET + WPF or Avalonia** | Great Windows API access | Weaker cross-platform media story, GC pauses in the hot path |
| **C++ + Qt** | Max control, OBS-like | Slow development, harder for contributors, Qt licensing nuances |
| **Embed libobs** | Proven capture/encode | GPL-2.0 (forces license), heavy, and game capture relies on hooking we want to avoid |

## Decision

- **Tauri 2** desktop shell with a **Rust** backend.
- **Svelte 5 + TypeScript + Vite** frontend (pnpm).
- **FFmpeg** (LGPL, dynamically linked) for hardware encoding and muxing.
- **Windows Graphics Capture** and **WASAPI** on Windows first. PipeWire and ScreenCaptureKit later, behind a capture trait.
- License: **MIT**.

## Consequences

- Low idle footprint; the UI can be closed to tray while recording continues in Rust.
- Contributors need Rust and Node toolchains.
- We must ship FFmpeg shared libraries and comply with the LGPL (dynamic linking, provide source/notice).
- No hooking-based game capture: we rely on WGC, which works for windowed and borderless WoW (the default) and for exclusive fullscreen on modern Windows.
