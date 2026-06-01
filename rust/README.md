# ⚡ PowerPanel

A blazing-fast, strictly native GTK4 layer-shell hardware monitor for modern Linux desktops (Wayland). Built with 100% Rust.

Displays CPU/GPU power consumption, temperature, and video engine (DEC/ENC) utilization as a lightweight overlay pinned to the top-right of your screen.

## ✨ Features

- **Multi-GPU support** — NVIDIA (NVML), AMD (hwmon + fdinfo VCN), Intel iGPU (RAPL) / Arc (hwmon)
- **Accurate AMD DEC/ENC tracking** — `drm-client-id` deduplication matches `amdgpu_top` accuracy
- **Intel video engine** — i915/xe `drm-engine-video` utilization via fdinfo
- **Smart CPU sensor detection** — scored hwmon scanner picks the right sensor for AMD and Intel
- **Dynamic UI** — media table auto-hides when no video activity, expands per-process when active
- **Wayland native** — anchored overlay via `gtk4-layer-shell`, no X11 dependency
- **Featherweight** — < 10 MB RAM, aggressive release profile (LTO, strip, panic=abort)
- **CLI mode** — terminal output with 1-second refresh (`--cli`)
- **Sensor diagnostics** — `--debug` flag prints hwmon/RAPL/GPU status and exits

## 🚀 Quick Start

```bash
git clone https://github.com/yusufyav/rust_power_panel.git
cd rust_power_panel
make
```

The `make` command auto-detects your distro, installs GTK4 dependencies if missing, and compiles a release binary.

## 📦 Installation

```bash
make install
```

Copies the binary to `/usr/local/bin/power_panel` and sets up udev rules so sensors are readable without `sudo`.

To uninstall:

```bash
make uninstall
```

## ⌨️ KDE Plasma Keyboard Shortcut

After `make install`, bind the following toggle command to a shortcut:

```
System Settings → Shortcuts → Custom Shortcuts → New → Global Shortcut → Command/URL
```

```bash
bash -c 'pgrep -x power_panel && pkill -x power_panel || /usr/local/bin/power_panel'
```

Opens the panel if closed, closes it if open.

## 🖥️ Usage

```
power_panel              # GUI overlay (default)
power_panel --cli        # Terminal mode, updates every second
power_panel --debug      # Print sensor diagnostics and exit
power_panel --version    # Show version
power_panel --help       # Show help
```

## Tab Tamamlama (bash)

Repo kökünden:

```bash
# Geçici (oturum için)
source /yol/completions/power_panel.bash

# Kalıcı (kullanıcı)
mkdir -p ~/.local/share/bash-completion/completions
cp completions/power_panel.bash ~/.local/share/bash-completion/completions/power_panel
```

## 🛠️ Makefile Targets

| Target | Description |
|---|---|
| `make` | Install deps + build |
| `make run` | Build + run directly |
| `make permissions` | Write udev rules for sensor access (one-time) |
| `make install` | Install to `/usr/local/bin` |
| `make uninstall` | Remove binary and udev rules |
| `make clean` | Remove build artifacts |
| `make help` | Show all targets |

## 🔧 Stack

- **Rust** — single `src/main.rs`, no unsafe
- **GTK4** + **gtk4-layer-shell** — Wayland overlay
- **nvml-wrapper** — NVIDIA power/temperature/process stats
- **Nerd Fonts** — required for icons (`󰻠` CPU, `󰢮` GPU, `` thermometer)
