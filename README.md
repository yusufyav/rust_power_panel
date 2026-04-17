# ⚡ PowerPanel

A blazing-fast, strictly native GTK4 layer-shell hardware monitor for modern Linux desktops (Wayland). Built with 100% Rust.

## ✨ Features
* **Zero C-Dependencies (for AMD):** Direct IP Discovery and sysfs reading for absolute accuracy matching `amdgpu_top`.
* **Smart UI:** Dynamic, auto-hiding process tables. Only expands when video codecs (DEC/ENC) are active.
* **Wayland Native:** Perfectly anchors to your screen using `gtk4-layer-shell`.
* **Featherweight:** Minimal memory footprint with aggressive Rust optimizations.

## 🚀 Quick Start (One Command)

Just clone and make! The setup will automatically detect your distro (Arch, Debian/Ubuntu, Fedora), install the required GTK4 dependencies, compile the release binary, and run it.

```bash
git clone [https://github.com/yusufyav/rust_power_panel.git](https://github.com/yusufyav/rust_power_panel.git)
cd rust_power_panel
make

Manual Installation
If you want to install it system-wide after testing:

make install


🛠️ Stack
Rust (Standard environment)

GTK4 + gtk4-layer-shell

Nerd Fonts (Required for icons like 󰻠, 󰢮, )