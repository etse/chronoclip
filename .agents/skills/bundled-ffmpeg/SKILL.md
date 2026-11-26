---
name: bundled-ffmpeg
description: Build the LGPL FFmpeg that Chronoclip's packages ship (packaging/ffmpeg/build.sh), compile and test Chronoclip against it, and lay out an installed-style copy (usr/bin + usr/lib/chronoclip). Use when changing the FFmpeg configuration or version, touching encode/remux code that packages depend on, or preparing a .deb/AppImage.
---

# Bundled LGPL FFmpeg

Development builds link the **system** FFmpeg (Ubuntu 24.04: 6.1, a GPL build). The packages ship
**our own** FFmpeg 8.1.3 instead (`packaging/ffmpeg/build.sh`): LGPL only, shared libraries,
about 3 MB, just what Chronoclip uses. Anything that works with the system FFmpeg can still
fail with ours, because ours leaves almost everything out. Always test both when it matters.

Background and decisions: `docs/architecture.md`, section *Packaging (M6)*.

## Build and test

```sh
packaging/ffmpeg/build.sh                     # → target/ffmpeg (~10 s; needs nasm, see INSTALL.md)

export PKG_CONFIG_PATH=$PWD/target/ffmpeg/lib/pkgconfig CARGO_TARGET_DIR=$PWD/target/bundled \
       LD_LIBRARY_PATH=$PWD/target/ffmpeg/lib
cargo test --release --workspace --no-fail-fast -- --include-ignored --skip finds_running_wow_stream
```

`finds_running_wow_stream` needs WoW running; skip it unless it is. Expect every other test,
including the GPU, audio and hotkey tests, to pass (84 at the time of writing).

## Installed layout

```sh
.agents/skills/bundled-ffmpeg/scripts/stage.sh          # → target/stage/usr/{bin,lib/chronoclip}
CHRONOCLIP_BIN=target/stage/usr/bin/chronoclip .agents/skills/test-instance/scripts/test-instance.sh start …
```

`stage.sh` (a wrapper around `packaging/stage.sh`, which `packaging/deb/build.sh` also uses) links the binary with the runpath `$ORIGIN/../lib/chronoclip` (the layout of both the
.deb and the AppImage) and fails if `ldd` resolves any FFmpeg library from outside the stage.
Then record and save with the test-instance skill and check the clip:
`ffprobe -v error -show_entries stream=codec_name,sample_rate,channels CLIP` must list `h264`
and `aac`. While it runs, `grep libav /proc/<pid>/maps` shows which libraries were loaded.

## Changing the FFmpeg configuration

Edit `CONFIGURE_FLAGS` in `packaging/ffmpeg/build.sh`, rebuild, and run the full test suite
against it. Lessons so far:

- The **AAC decoder** is required although nothing is decoded: opening an MPEG-TS segment
  (saving a clip) learns the audio's sample rate and channels from it. Symptom without it:
  `Could not find codec parameters … unspecified sample rate`, `mp4: sample rate not set`.
- The runpath `$ORIGIN` is added to `ffbuild/config.mak` after configure (configure loses the `$`
  of `--extra-ldsoflags`, and `library.mak` expands the flags twice, hence `$$$$ORIGIN`). The
  script verifies it with `readelf`; don't move it back into configure.
- Never add GPL or nonfree parts (`--enable-gpl`, `libx264`, …): the script refuses, and AGENTS.md
  rule 7 forbids it. `iconv` showing up as "external" is glibc, not a new dependency.
- Keep nv-codec-headers at n12.1 unless the minimum NVIDIA driver (530) may rise: newer headers
  need newer drivers on every user's machine.

## Updating the FFmpeg version

Change `FFMPEG_VERSION` and `FFMPEG_SHA256` (verify the `.asc` signature: key
`FCF9 86EA 15E6 E293 A564 4F10 B432 2F04 D676 58D8`). The Rust bindings (`ffmpeg-next` in
`Cargo.toml`) must support that FFmpeg major version. Update the version in `docs/architecture.md`
and `INSTALL.md`, then rerun the full test suite and `stage.sh`.
