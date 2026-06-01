# PowerPanel — Zig portu

Rust `rust/src/main.rs`'in **Zig 0.16** ile yazılmış klonu. Mevcut Rust uygulaması
olduğu gibi durur (`../rust/src/main.rs`); bu dizin bağımsız bir Zig
implementasyonudur.

## Durum

Tam klon — **sensör çekirdeği + tüm modlar** (CLI/TUI/GUI/GUI2/debug):

| Mod | Bayrak | Durum |
|---|---|---|
| CLI | `--cli` | ✅ Rust ile birebir kare (aynı ANSI, hizalama) |
| TUI | `--tui` | ✅ Bar görünümlü, birebir |
| **GUI** (etiketli panel) | `--gui` (veya argümansız) | ✅ GTK4 layer-shell overlay — Rust `build_ui` portu |
| **GUI2** (bar panel) | `--gui2` | ✅ GTK4 layer-shell overlay — Rust `build_ui2` portu |
| Debug | `--debug` | ✅ Sensör teşhisi |
| Help/Version | `--help` / `--version` | ✅ |

Tüm modlar tamamlandı; Rust uygulamasının tam klonu.

Desteklenen donanım: **NVIDIA** (NVML), **AMD** (hwmon + fdinfo VCN),
**Intel** iGPU (RAPL uncore) / Arc (hwmon). CPU sıcaklık (hwmon skorlama),
CPU güç (RAPL diferansiyel), CPU%/RAM (`/proc/stat`, `/proc/meminfo`).

## Tasarım notları

- **Saf Linux syscall katmanı** (`src/os.zig`): Zig 0.16 dosya/IO API'sini
  `std.Io`'ya taşıdı ve her çağrı `io: Io` ister (kararsız). PowerPanel
  yalnızca Linux/Wayland hedeflediğinden doğrudan `std.os.linux`
  (open/read/getdents64/clock_gettime) kullanmak hem en sağlam hem en hafif yol.
- **GTK elle `extern`** (`src/gui.zig`): Zig 0.16 `@cImport`, glib/gtk
  header'larındaki `_Pragma` makrolarında çöküyor (translate-c sınırı). Bu
  yüzden gereken ~50 GTK/cairo sembolü elle `extern` bildirildi; tüm GObject
  işaretçileri ABI-uyumlu olduğundan `?*anyopaque`. GUI tek-thread'de iki
  g_timeout ile döner (200ms GFX örnekleme + her 5. tik tam okuma & UI).
- **NVML runtime `dlopen`** (`src/nvml.zig`): link zamanında bağımlılık yok;
  NVIDIA sürücüsü olmayan makinelerde de binary çalışır (Rust `nvml-wrapper`
  ile aynı yaklaşım).
- **Sıfır heap alloc / kare**: CLI/TUI kareleri sabit tampona yazılır; proc
  listeleri sabit dizilerde tutulur. fdinfo izleyiciler yalnızca küçük
  hashmap'ler için (per-örnek arena + kalıcı prev map) ayırım yapar.
- Sıcaklık yuvarlama (`floor`), capacity bölmesi ve fdinfo dedup mantığı Rust
  ile birebir korundu (CLAUDE.md "dokunulmayacak alanlar").

## Derleme

Zig 0.16 gerekir.

```bash
zig build -Doptimize=ReleaseFast              # tam (GTK4 GUI dahil)
zig build -Doptimize=ReleaseFast -Dgui=false  # lean: GTK linkleme yok, sadece CLI/TUI/debug
# veya Makefile:
make build      # ReleaseFast + strip
make cli        # derle + --cli çalıştır
```

GUI (`-Dgui=true`, varsayılan) için sistemde `gtk4`, `gtk4-layer-shell`, `cairo`
geliştirme paketleri gerekir. `-Dgui=false` ile bu bağımlılıklar tamamen düşer.

## Boyut karşılaştırması (bu makine)

| Binary | Boyut |
|---|---|
| Rust release (`rust/target/release`) | ~1.0 MB |
| Zig ReleaseFast + strip (GUI dahil) | ~323 KB |
| Zig ReleaseFast + strip (`-Dgui=false`) | ~343 KB |
| Zig ReleaseSmall + strip (`-Dgui=false`) | ~111 KB |

GTK4 dinamik linklendiği için GUI'li binary diske ek yük getirmez.

## Kullanım

```bash
./zig-out/bin/power_panel --cli      # sürekli terminal güncelleme
./zig-out/bin/power_panel --tui      # bar görünümlü
./zig-out/bin/power_panel --debug    # sensör erişim teşhisi
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

RAPL/hwmon erişimi için ana projedeki `make permissions` (udev kuralı) geçerli.
