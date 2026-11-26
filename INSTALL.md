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
| **NVIDIA GPU + proprietary driver** (530 or newer) | Hardware video encoding (NVENC) and GPU capture (CUDA). AMD/Intel support is planned. | `nvidia-smi` shows your GPU |
| **PipeWire** (with `pipewire-pulse`) or **PulseAudio** | Recording game sound and the microphone. Default on Linux Mint / Ubuntu and practically every desktop. Without it, clips have a silent audio track. | `pactl info` prints a `Server Name` |

FFmpeg is **not** needed: the packages bring their own (LGPL, about 3 MB, only what Chronoclip uses).

### Install on Ubuntu / Linux Mint (.deb)

Works on Ubuntu 22.04, 24.04, 26.04 and Linux Mint 21, 22, 23.

1. **NVIDIA driver:** open *Driver Manager* (Mint) or *Additional Drivers* (Ubuntu) and pick the recommended `nvidia-driver-XXX`. It includes NVENC and CUDA, so there's nothing extra to install.
2. **Chronoclip:** download `chronoclip_<version>_amd64.deb` from [GitHub Releases](https://github.com/etse/chronoclip/releases) and double-click it, or:
   ```sh
   sudo apt install ./chronoclip_*_amd64.deb
   ```
   apt installs the few system libraries it needs (X11, OpenGL, PulseAudio; all normally present already).
3. Start **Chronoclip** from the app menu (Sound & Video).

To remove it: `sudo apt remove chronoclip`. Your settings (`~/.config/chronoclip`), saved clips (`~/Videos/Chronoclip`) and, if you turned it on, the autostart entry (`~/.config/autostart/chronoclip.desktop`) stay; delete them by hand if you like.

What the .deb installs: `/usr/bin/chronoclip`, FFmpeg in `/usr/lib/chronoclip/`, the menu entry, icons and license files (`/usr/share/doc/chronoclip/`).

### Install on other distros (AppImage)

One file that runs on distros with glibc 2.35 or newer: Debian 12+, Fedora 36+, Arch, openSUSE Tumbleweed, Ubuntu 22.04+ and their relatives.

1. **NVIDIA driver** from your distro (see the table below).
2. Download `Chronoclip-<version>-x86_64.AppImage` from [GitHub Releases](https://github.com/etse/chronoclip/releases), then:
   ```sh
   chmod +x Chronoclip-*-x86_64.AppImage
   ./Chronoclip-*-x86_64.AppImage
   ```

It needs FUSE 3 (`fusermount3`, package `fuse3`), installed by default on current desktops; without it, start it with `--appimage-extract-and-run`. Keep the file where it is if you turn on *Start in the tray when you log in*: the autostart entry points at the `.AppImage` file (it follows if the file is moved and started from the new place). AppImageLauncher, if installed, may offer to move it to `~/Applications` and add it to the menu.

### Runtime libraries

Everything Chronoclip loads from the system is part of every Linux X11 desktop (the .deb depends on these packages):

| Runtime library | Package (Mint / Ubuntu) | Used for |
|---|---|---|
| `libcuda.so.1`, `libnvidia-encode.so.1` | part of the NVIDIA driver | GPU copy (CUDA) and encoding (NVENC) |
| `libX11.so.6`, `libX11-xcb.so.1`, `libxcb.so.1`, `libXcursor.so.1`, `libXi.so.6`, `libxkbcommon.so.0`, `libxkbcommon-x11.so.0`, `libfontconfig.so.1` | `libx11-6`, `libx11-xcb1`, `libxcb1`, `libxcursor1`, `libxi6`, `libxkbcommon0`, `libxkbcommon-x11-0`, `libfontconfig1` | The window, tray and fonts (Slint / winit) |
| `libXcomposite.so.1`, `libGL.so.1` (GLX) | `libxcomposite1`, `libgl1` | Finding and capturing the WoW window on the GPU, and the global hotkey |
| `libpulse.so.0`, `libpulse-simple.so.0` | `libpulse0` (installed with PipeWire's `pipewire-pulse` or PulseAudio) | Recording sound through the PulseAudio API, which PipeWire provides, and playing the save chime |
| `xdg-open` | `xdg-utils` | **Play** and **Show in folder** / **Open** buttons |
| `libavcodec`, `libavformat`, `libavutil` | bundled with Chronoclip (`/usr/lib/chronoclip`) | Encoding and writing video files |

| | Mint / Ubuntu / Debian | Arch | Fedora |
|---|---|---|---|
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
| `nasm`, a C compiler, `make`, `curl` | `sudo apt install nasm build-essential curl` | `packaging/ffmpeg/build.sh` builds the LGPL FFmpeg that packages ship (into `target/ffmpeg`, ~10 s) |
| `dpkg-dev` | `sudo apt install dpkg-dev` (usually installed) | `packaging/deb/build.sh` builds the .deb (`dpkg-shlibdeps`, `dpkg-deb`) |

Packages don't use the distro's FFmpeg: they carry FFmpeg 8.1.3 built by `packaging/ffmpeg/build.sh` (LGPL only, about 3 MB, just NVENC, AAC, MPEG-TS and MP4). To build and test against it, see [AGENTS.md](AGENTS.md#commands).

See [AGENTS.md](AGENTS.md#commands) for test, lint and other commands.
