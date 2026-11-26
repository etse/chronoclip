# AGENTS.md

Instructions for AI coding agents working in this repository (Claude Code, opencode, Codex, Cursor, and others).
This file is the **single source of truth** for agent instructions. Tool-specific files (`CLAUDE.md`, `opencode.json`) only point here. Do not duplicate content into them.

## Session start

At the start of every new session, **before anything else**, read [docs/roadmap.md](docs/roadmap.md) and give the maintainer a short milestone summary:

1. A compact table of all milestones with their status (✅ / 🚧 / ⬜).
2. The current or next milestone, with its open tasks (unchecked boxes) or, if it has no checklist yet, its one-line scope.
3. Anything uncommitted (`git status --short`) or noted as pending in the roadmap.

Keep it brief (under ~15 lines), then answer the maintainer's first message. If that message is itself a task, give the summary first and then do the task.

## Project

**Chronoclip** is an open source replay buffer for World of Warcraft **on Linux** (ShadowPlay-style "Instant Replay"). WoW runs through Wine / Proton / Lutris.
It continuously records gameplay to a rolling on-disk buffer. A configurable global hotkey (default `Ctrl+F9`) saves the last N minutes (5 to 45, default 10) as an MP4.

The maintainer builds it for their own use as a Linux gamer and shares it with others. **Linux is the only target.** Windows and macOS are non-goals; don't add cross-platform abstractions for them.

**Reference setup (build and verify against this first):** Linux Mint 22 Cinnamon on **X11** (Ubuntu 24.04 base), **NVIDIA** GPU with the proprietary driver, WoW Retail on **DirectX 12** (vkd3d-proton) in **borderless windowed** mode, launched through **Faugus Launcher** (UMU/Proton). This makes **X11 + NVENC** the first backend to implement; Wayland and VAAPI come later.

Read before making non-trivial changes:

- [README.md](README.md): features, stack, roadmap
- [docs/architecture.md](docs/architecture.md): pipeline design and module boundaries
- [docs/roadmap.md](docs/roadmap.md): milestone status and task checklists. Tick tasks off as you finish them.
- [docs/adr/](docs/adr/): architecture decision records. Do not contradict an accepted ADR without writing a new one.

## Tech stack

- **Rust** only (Cargo workspace). No Node/JS toolchain, no webview, no Electron/Tauri.
- **Slint** for the UI (`.slint` files).
- **FFmpeg** (libav*, LGPL, dynamically linked) for encoding (VAAPI, NVENC) and muxing.
- **X11** (first): XComposite window capture → GLX texture-from-pixmap → CUDA interop → NVENC, XGrabKey hotkeys.
- **Wayland** (later): xdg-desktop-portal (ScreenCast, GlobalShortcuts) via `ashpd` + PipeWire.
- **PipeWire** for audio on both, through its PulseAudio API (`libpulse`, loaded at runtime; per-application capture with monitor streams).

## Repository layout (target)

```
.
├── app/                       # Binary crate `chronoclip`: main, tray, hotkeys, IPC, CLI, UI wiring
│   └── ui/                    # Slint UI files (*.slint), presentation only
├── crates/
│   ├── chronoclip-core/       # Pure logic: settings, hotkeys, segment index, paths, IPC protocol
│   ├── chronoclip-capture/    # X11: window finding, capture (XComposite → GLX → CUDA), hotkey grab; audio (libpulse); Wayland later
│   └── chronoclip-encode/     # FFmpeg: NVENC encoder (CUDA frames), AAC encoder, MPEG-TS segment writer, MP4 remux
├── packaging/
│   ├── stage.sh               # binary + bundled FFmpeg → STAGE/usr/{bin,lib/chronoclip} (runpath $ORIGIN/../lib/chronoclip)
│   ├── deb/                   # build.sh → target/deb/chronoclip_<version>_amd64.deb (dpkg-deb), copyright
│   ├── appimage/              # build.sh → target/appimage/Chronoclip-<version>-x86_64.AppImage (appimagetool, static runtime)
│   ├── check-release.sh       # version/date/CHANGELOG agree? → release notes
│   ├── ffmpeg/                # build.sh: the LGPL FFmpeg (8.1.3, NVENC/AAC/MPEG-TS/MP4 only) shipped in packages
│   ├── icons/                 # make-icons.py (Pillow) → hicolor PNGs + app/ui/icons; regenerate, don't edit PNGs
│   ├── linux/                 # io.github.etse.chronoclip.desktop + .metainfo.xml (validate with desktop-file-validate / appstreamcli)
│   └── licenses/              # cargo-about config; generate.sh writes THIRD-PARTY-LICENSES.txt at package time
├── docs/
│   ├── architecture.md
│   └── adr/
└── AGENTS.md
```

Inside `app/src/`: `main.rs` (CLI + startup), `controller.rs` (handles commands from UI/tray/IPC/hotkey on any thread, owns the recorder and the hotkey, starts saves), `recorder.rs` (capture/encode thread + writer thread), `audio.rs` (audio tracks: capture → timeline → AAC, aligned to the video session clock), `buffer.rs` (`ReplayBuffer`: segment index shared by the writer and saves, pinning), `save.rs` (one save: pick segments → remux → MP4), `hotkey.rs` (picks the global hotkey backend for the session), `ui.rs` (Slint ↔ settings glue), `ipc.rs` (Unix socket, single instance), `tray.rs` (wires Slint's built-in `SystemTrayIcon`, defined in `ui/tray.slint`).

## Commands

System dependencies are listed in [INSTALL.md](INSTALL.md).

```sh
cargo run -p chronoclip            # start the app (or show the running one)
cargo run -p chronoclip -- save    # talk to the running instance: save | show | quit
RUST_LOG=debug cargo run -p chronoclip
CHRONOCLIP_WINDOW=glxgears cargo run -p chronoclip   # record any window by title instead of WoW
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# Against the bundled LGPL FFmpeg (what the packages ship); needs nasm:
packaging/ffmpeg/build.sh             # → target/ffmpeg
PKG_CONFIG_PATH=$PWD/target/ffmpeg/lib/pkgconfig CARGO_TARGET_DIR=target/bundled LD_LIBRARY_PATH=$PWD/target/ffmpeg/lib \
  cargo test --release --workspace
packaging/deb/build.sh                # → target/deb/chronoclip_<version>_amd64.deb (CI: .github/workflows/package.yml)
packaging/appimage/build.sh           # → target/appimage/Chronoclip-<version>-x86_64.AppImage
cargo test --workspace -- --ignored   # GPU/X11/audio tests: need X11, NVIDIA, glxgears, a sound server and paplay; the hotkey test presses keys via XTest
```

Before declaring a task done, run format, clippy, and tests, and report the actual results. Capture, hotkey, audio, and encoder code usually needs a real desktop session to verify. Say clearly what was tested automatically and what still needs a manual in-game check.

## Skills

Step-by-step guides with scripts for recurring jobs live in **`.agents/skills/<name>/SKILL.md`**
(read natively by opencode; `.claude/skills` is a symlink to it for Claude Code). Use them instead
of improvising:

| Skill | Use it to |
|---|---|
| `test-instance` | Run the real app in a test, isolated from the maintainer's instance, settings, sound and mic |
| `bundled-ffmpeg` | Build the packages' LGPL FFmpeg, test against it, lay out an installed copy |
| `av-sync-test` | Measure A/V sync of clips (flash + beep every second) |
| `milestone` | Plan, tick, verify and close milestones in `docs/roadmap.md` |
| `readme-screenshot` | Retake the README window screenshot |
| `release` | Cut a release (or rehearse one with a dry run) |

New skills go in `.agents/skills/` too (one folder with `SKILL.md` and optional `scripts/`).

## Releases

A pushed tag `vX.Y.Z` publishes a GitHub release with the .deb, the AppImage, the FFmpeg source
and checksums (`.github/workflows/release.yml`); notes come from `CHANGELOG.md`. Before tagging,
`Cargo.toml`, the metainfo `<release>` entry and `CHANGELOG.md` must agree
(`packaging/check-release.sh X.Y.Z`). Follow the `release` skill, and **never push a tag
without the maintainer's go**: it publishes to everyone. Add user-facing changes to
`CHANGELOG.md` under `## [Unreleased]` as you make them.

## Hard rules

1. **Never touch the game process.** No DLL injection, no D3D/Vulkan/OpenGL hooking (including Vulkan layers or `LD_PRELOAD` tricks), no memory reading, no input automation. Capture only through X11 (XComposite) or the portal/PipeWire APIs. This protects users' Battle.net accounts and is non-negotiable.
2. **Performance is a feature.** The capture → encode → write hot path must not allocate per frame where avoidable, must not block the UI thread, and must use hardware encoders when available. Keep frames on the GPU: no `XGetImage`/CPU readback in the normal path (X11: XComposite pixmap → GL texture → CUDA → NVENC; Wayland: DMA-BUF import). Saving a replay must be a remux (stream copy), never a re-encode.
3. **Bounded disk usage.** The buffer must always delete segments older than the configured length (plus a small margin). Clean up the buffer dir on startup and shutdown. Never default the buffer to a RAM-backed tmpfs (`/tmp`, `$XDG_RUNTIME_DIR`).
4. **X11 first, Wayland must stay possible.** Implement X11 first, but keep backends behind the `VideoSource` / hotkey abstractions so a Wayland path (portal) can be added. Every hotkey path also has the `chronoclip save` CLI fallback.
5. **Keep `chronoclip-core` pure.** No FFI, D-Bus, or PipeWire there, so it stays fast to compile and easy to unit-test.
6. **UI is presentation only.** Business logic lives in Rust crates. `.slint` files hold layout, styling, and bindings, not logic.
7. **License hygiene.** Project is MIT. No GPL dependencies; in particular no static FFmpeg and no `libx264`/`libx265`. Slint is used under its Royalty-free Desktop License: keep the attribution in the About page. New licensing implications need an ADR.
8. **No telemetry or network calls** unless an ADR explicitly allows them.
9. **Every system dependency is documented in [INSTALL.md](INSTALL.md).** When you add or remove anything users or developers must install (apt packages, drivers, runtime libraries), update INSTALL.md in the same change: the user section for runtime needs, the developer section for build-only needs, with the reason for each. Prefer loading optional system libraries at runtime (`libloading`) over requiring `-dev` packages.

## Coding conventions

### Rust
- Stable toolchain. `cargo fmt` and `clippy -D warnings` clean.
- Errors: `thiserror` in library crates, `anyhow` only in `app/`.
- Logging: `tracing`. No `println!` in library code.
- No `unwrap()` / `expect()` outside tests unless the invariant is documented in a comment.
- `unsafe` (FFI to FFmpeg, CUDA, GL, PipeWire, X11) must be wrapped in a small safe abstraction with a `// SAFETY:` comment.
- Concurrency: dedicated threads for capture and encoding with bounded channels between them. Drop frames rather than grow memory unbounded. The Slint event loop thread only receives state updates (`slint::invoke_from_event_loop`).
- Paths: use XDG dirs via the `directories` crate, never hard-coded paths.

### Slint
- One component per file where reasonable, under `app/ui/`.
- Expose state through Slint `global`s / properties and callbacks; keep Rust ↔ UI glue in one module in `app/`.
- Keyboard-navigable and readable at 100% to 200% scaling.

### General
- Settings are validated in Rust (e.g. buffer length clamped to 5..=45 minutes).
- Write tests for pure logic (segment pruning, segment selection for a save, settings validation).
- Keep changes focused. Update docs and this file when you change commands, layout, or architecture.

## Git

- Conventional Commits: `feat:`, `fix:`, `docs:`, `refactor:`, `perf:`, `test:`, `chore:`, `ci:`.
- Small, focused commits. Do not commit build output, recordings, or `.env` files.
- Only commit or push when the user asks.
