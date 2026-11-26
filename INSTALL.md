# Installing Chronoclip

This file lists **everything you need to install**, for:

1. [**Users**](#users-running-chronoclip): what must be on your system to *run* Chronoclip.
2. [**Developers**](#developers-building-from-source): what you additionally need to *build* it from source.

> Commands are for **Linux Mint 22 / Ubuntu 24.04** (the reference setup). Package names for other distros are in the tables.

---

## Users: running Chronoclip

### Requirements

| Requirement | Why | How to check |
|---|---|---|
| **X11 session** | Window capture and the hotkey use X11 (Wayland support is planned). Linux Mint Cinnamon, MATE and Xfce use X11 by default. | `echo $XDG_SESSION_TYPE` prints `x11` |
| **NVIDIA GPU + proprietary driver** (535 or newer) | Hardware video encoding (NVENC) and GPU capture (CUDA). AMD/Intel support is planned. | `nvidia-smi` shows your GPU |
| **FFmpeg libraries** (version 6 or newer) | Video and audio encoding and file writing | `ffmpeg -hide_banner -encoders \| grep h264_nvenc` |
| **PipeWire** (with `pipewire-pulse`) or **PulseAudio** | Recording game sound and the microphone. Default on Linux Mint 22 / Ubuntu 24.04 and practically every desktop. Without it, clips have a silent audio track. | `pactl info` prints a `Server Name` |

### Install (Linux Mint / Ubuntu)

1. **NVIDIA driver:** open *Driver Manager* (Mint) or *Additional Drivers* (Ubuntu) and pick the recommended `nvidia-driver-XXX`. It includes NVENC and CUDA, so there's nothing extra to install.
2. **FFmpeg libraries:**
   ```sh
   sudo apt install ffmpeg
   ```

That's it. Everything else Chronoclip needs is already part of every Linux X11 desktop:

| Runtime library | Package (Mint / Ubuntu) | Used for |
|---|---|---|
| `libcuda.so.1`, `libnvidia-encode.so.1` | part of the NVIDIA driver | GPU copy (CUDA) and encoding (NVENC) |
| `libX11.so.6`, `libXcomposite.so.1` | `libx11-6`, `libxcomposite1` | Finding and capturing the WoW window, and the global hotkey |
| `libGL.so.1` (GLX) | `libgl1` | Reading the window image on the GPU |
| `libavcodec`, `libavformat`, `libavutil` | installed with `ffmpeg` | Encoding and writing video files |
| `libpulse.so.0`, `libpulse-simple.so.0` | `libpulse0` (installed with PipeWire's `pipewire-pulse` or PulseAudio) | Recording sound through the PulseAudio API, which PipeWire provides, and playing the save chime |

| | Mint / Ubuntu / Debian | Arch | Fedora |
|---|---|---|---|
| FFmpeg libraries | `ffmpeg` | `ffmpeg` | `ffmpeg-libs` (RPM Fusion) |
| NVIDIA driver | Driver Manager / `nvidia-driver-XXX` | `nvidia` | `akmod-nvidia` (RPM Fusion) |
| Sound (PulseAudio API) | `pipewire-pulse` (default) | `pipewire-pulse` | `pipewire-pulseaudio` (default) |

---

## Developers: building from source

You need everything from the user section, plus:

### 1. Rust

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

`rust-toolchain.toml` in the repo selects the right toolchain automatically.

### 2. Build dependencies (Linux Mint / Ubuntu)

```sh
sudo apt install pkg-config clang libclang-dev \
  libfontconfig1-dev libxkbcommon-dev \
  libavcodec-dev libavformat-dev libavutil-dev libswscale-dev
```

| Package (Mint / Ubuntu) | Arch | Why |
|---|---|---|
| `pkg-config` | `pkgconf` | Finds system libraries during the build |
| `clang`, `libclang-dev` | `clang` | Generates the Rust bindings to FFmpeg (bindgen) |
| `libfontconfig1-dev` | `fontconfig` | Font lookup for the Slint UI |
| `libxkbcommon-dev` | `libxkbcommon` | Keyboard handling for the Slint UI |
| `libavcodec-dev`, `libavformat-dev`, `libavutil-dev`, `libswscale-dev` | `ffmpeg` | FFmpeg headers: encoding and muxing |

X11, OpenGL/GLX, CUDA, NVENC and libpulse are loaded at runtime from the libraries your desktop and NVIDIA driver already provide, so they need **no** `-dev` packages.

### 3. Build and run

```sh
cargo run -p chronoclip
```

### 4. Optional: GPU and X11 tests

The capture/encoder tests need an NVIDIA GPU and `glxgears`. The hotkey test grabs a key on your X server and presses it through XTest (`libXtst.so.6`, package `libxtst6`, installed on practically every X11 desktop). The audio test plays a quiet tone with `paplay` (package `pulseaudio-utils`) made with the `ffmpeg` command line tool, and records it:

```sh
sudo apt install mesa-utils libxtst6 pulseaudio-utils   # glxgears; XTest; paplay
cargo test --workspace -- --ignored
```

The hotkey test briefly grabs `Ctrl+Shift+Super+F10` and `F11`; it fails if another program already uses them.

### 5. Optional: packaging tools

Only needed to change icons, check the app metadata or build the license file that ships with packages:

| Tool | Install (Mint / Ubuntu) | Used for |
|---|---|---|
| Pillow | `sudo apt install python3-pil` | `python3 packaging/icons/make-icons.py` renders all icon sizes |
| `desktop-file-validate`, `appstreamcli` | `sudo apt install desktop-file-utils appstream` | Checking `packaging/linux/*.desktop` and `*.metainfo.xml` |
| cargo-about | `cargo install cargo-about --locked --features cli` | `packaging/licenses/generate.sh` writes `THIRD-PARTY-LICENSES.txt` |

See [AGENTS.md](AGENTS.md#commands) for test, lint and other commands.
