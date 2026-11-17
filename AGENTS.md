# AGENTS.md

Instructions for AI coding agents working in this repository (Claude Code, opencode, Codex, Cursor, and others).
This file is the **single source of truth** for agent instructions. Tool-specific files (`CLAUDE.md`, `opencode.json`) only point here. Do not duplicate content into them.

## Project

**Chronoclip** is an open source replay buffer for World of Warcraft (ShadowPlay-style "Instant Replay").
It continuously records gameplay to a rolling on-disk buffer in a temp location. A configurable global hotkey (default `Ctrl+F9`) saves the last N minutes (5 to 45, default 10) as an MP4.

Read before making non-trivial changes:

- [README.md](README.md): features, stack, roadmap
- [docs/architecture.md](docs/architecture.md): pipeline design and module boundaries
- [docs/adr/](docs/adr/): architecture decision records. Do not contradict an accepted ADR without writing a new one.

## Tech stack

- **Tauri 2** desktop shell, **Rust** backend (Cargo workspace)
- **Svelte 5 + TypeScript + Vite** frontend, managed with **pnpm**
- **FFmpeg** (libav*, LGPL, dynamically linked) for encoding and muxing
- Windows Graphics Capture + WASAPI on Windows (primary platform). PipeWire (Linux) and ScreenCaptureKit (macOS) later.

## Repository layout (target)

```
.
├── src/                     # Svelte frontend (UI only, no business logic)
├── src-tauri/               # Tauri app crate: commands, tray, hotkey, wiring
├── crates/
│   ├── chronoclip-core/     # Platform-agnostic: settings, ring buffer, segment index, save/remux
│   ├── chronoclip-capture/  # Capture trait + per-OS backends (cfg-gated)
│   └── chronoclip-encode/   # FFmpeg encoder/muxer wrappers
├── docs/
│   ├── architecture.md
│   └── adr/
└── AGENTS.md
```

Directories that don't exist yet should be created following this layout.

## Commands

> The app is not implemented yet. Once it is, keep this section accurate.

```sh
pnpm install
pnpm tauri dev                     # run app
pnpm tauri build                   # build installer
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm lint && pnpm check && pnpm test
```

Before declaring a task done, run formatting, lint, and tests for every part you touched, and report the actual results.

## Hard rules

1. **Never touch the game process.** No DLL injection, no D3D/Vulkan hooking, no memory reading, no input automation. Capture only through official OS APIs (WGC, PipeWire portal, ScreenCaptureKit). This protects users' Battle.net accounts and is non-negotiable.
2. **Performance is a feature.** The capture → encode → write hot path must not allocate per frame where avoidable, must not block on the UI thread, and must use hardware encoders when available. Saving a replay must be a remux (stream copy), never a re-encode.
3. **Bounded disk usage.** The buffer must always delete segments older than the configured length (plus a small margin). Clean up the temp buffer on startup and shutdown.
4. **Platform code is isolated** behind traits in `chronoclip-capture` and gated with `#[cfg(target_os = "...")]`. `chronoclip-core` must compile on every OS.
5. **Frontend is presentation only.** Business logic lives in Rust and is exposed via typed Tauri commands and events.
6. **License hygiene.** Project is MIT. Do not add GPL dependencies or link FFmpeg statically / with GPL-only components (e.g. a GPL x264 build) without an ADR.
7. **No secrets, telemetry, or network calls** unless an ADR explicitly allows them.

## Coding conventions

### Rust
- Edition 2021+, stable toolchain. `cargo fmt` and `clippy -D warnings` clean.
- Errors: `thiserror` in library crates, `anyhow` only at the app boundary.
- Logging: `tracing`. No `println!` in library code.
- No `unwrap()` / `expect()` outside tests unless the invariant is documented in a comment.
- `unsafe` (FFI to Windows/FFmpeg) must be wrapped in a small safe abstraction with a `// SAFETY:` comment.
- Concurrency: dedicated threads for capture and encoding, bounded channels between them. Drop frames rather than grow memory unbounded.

### TypeScript / Svelte
- `strict` TypeScript, Svelte 5 runes.
- Tauri command calls go through a single typed API module (e.g. `src/lib/api.ts`), not scattered `invoke()` calls.
- Keep components small. Prefer accessible, keyboard-navigable UI.

### General
- Settings changes must be validated in Rust (e.g. buffer length clamped to 5..=45 minutes).
- Write tests for platform-agnostic logic (ring buffer pruning, segment selection for a save, settings validation).
- Keep changes focused. Update docs and this file when you change commands, layout, or architecture.

## Git

- Conventional Commits: `feat:`, `fix:`, `docs:`, `refactor:`, `perf:`, `test:`, `chore:`, `ci:`.
- Small, focused commits. Do not commit build output, recordings, or `.env` files.
- Only commit or push when the user asks.
