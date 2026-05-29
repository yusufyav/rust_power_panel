# PowerPanel — Rust uygulaması
#
# Kullanım: proje kökündeki rust/ dizininden çalıştırın.
#   cd rust && make install

.PHONY: all deps build server server-arm64 run install uninstall clean permissions help

# ── Ayarlar ──────────────────────────────────────────────────────────────────
BIN_NAME    = power_panel
INSTALL_DIR = /usr/local/bin
UDEV_RULES  = /etc/udev/rules.d/99-$(BIN_NAME).rules

all: deps build

# ── Yardım ───────────────────────────────────────────────────────────────────
help:
	@echo ""
	@echo "PowerPanel — Rust uygulaması"
	@echo "Çalıştırma dizini: rust/"
	@echo "────────────────────────────────────────"
	@echo "  make               Bağımlılıkları kontrol et + derle"
	@echo "  make server        Statik musl binary — GUI yok, Proxmox/sunucu için"
	@echo "  make server-arm64  ARM64 sunucu için statik musl binary"
	@echo "  make run           Derle + çalıştır"
	@echo "  make permissions   Sensör izinlerini kur (udev, tek seferlik)"
	@echo "  make install       Sisteme kur (/usr/local/bin/$(BIN_NAME))"
	@echo "  make uninstall     Kurulumu kaldır"
	@echo "  make clean         Build dosyalarını temizle"
	@echo "  make help          Bu mesajı göster"
	@echo ""
	@echo "NOT: 'make run' öncesinde 'make permissions' yapılmamışsa"
	@echo "     güç değerleri 0.0 W görünebilir (RAPL erişim sorunu)."
	@echo ""

# ── Sistem bağımlılıkları ─────────────────────────────────────────────────────
deps:
	@command -v cargo > /dev/null 2>&1 \
	  || { echo "❌ Rust/Cargo kurulu değil. Kurmak için: https://rustup.rs"; exit 1; }
	@echo "=> Sistem bağımlılıkları kontrol ediliyor..."
	@MISSING=""; \
	pkg-config --exists gtk4 2>/dev/null \
	  || MISSING="$$MISSING gtk4"; \
	{ pkg-config --exists gtk4-layer-shell 2>/dev/null \
	  || pkg-config --exists gtk4-layer-shell-0 2>/dev/null; } \
	  || MISSING="$$MISSING gtk4-layer-shell"; \
	if [ -z "$$MISSING" ]; then \
		echo "=> Bağımlılıklar zaten kurulu. Atlanıyor."; \
	else \
		echo "=> Eksik paketler:$$MISSING — kuruluyor..."; \
		if command -v pacman > /dev/null; then \
			sudo pacman -S --needed gtk4 gtk4-layer-shell pkgconf; \
		elif command -v apt-get > /dev/null; then \
			sudo apt-get update && sudo apt-get install -y \
				libgtk-4-dev libgtk4-layer-shell-dev pkg-config; \
		elif command -v dnf > /dev/null; then \
			sudo dnf install -y \
				gtk4-devel gtk4-layer-shell-devel pkgconf-pkg-config; \
		elif command -v zypper > /dev/null; then \
			sudo zypper install -y \
				gtk4-devel gtk4-layer-shell-devel pkg-config; \
		else \
			echo "=> HATA: Desteklenmeyen paket yöneticisi!"; \
			echo "   Lütfen GTK4 ve gtk4-layer-shell paketlerini elle kurun."; \
			exit 1; \
		fi \
	fi

# ── Derleme ───────────────────────────────────────────────────────────────────
build:
	@echo "=> $(BIN_NAME) derleniyor (release)..."
	@cargo build --release
	@echo "=> Derleme tamamlandı: target/release/$(BIN_NAME)"

# Proxmox/sunucu: GTK yok, musl ile tamamen statik binary
server:
	@command -v cargo > /dev/null 2>&1 \
	  || { echo "❌ Rust/Cargo kurulu değil. Kurmak için: https://rustup.rs"; exit 1; }
	@rustup target add x86_64-unknown-linux-musl 2>/dev/null || true
	@cargo build --release --no-default-features --target x86_64-unknown-linux-musl
	@echo "=> Sunucu binary'si hazır: target/x86_64-unknown-linux-musl/release/$(BIN_NAME)"
	@echo "=> Proxmox'a kopyalamak için:"
	@echo "   scp target/x86_64-unknown-linux-musl/release/$(BIN_NAME) root@<proxmox-ip>:/usr/local/bin/$(BIN_NAME)"

server-arm64:
	@command -v cargo > /dev/null 2>&1 \
	  || { echo "❌ Rust/Cargo kurulu değil. Kurmak için: https://rustup.rs"; exit 1; }
	@rustup target add aarch64-unknown-linux-musl 2>/dev/null || true
	@cargo build --release --no-default-features --target aarch64-unknown-linux-musl
	@echo "=> ARM64 sunucu binary'si hazır: target/aarch64-unknown-linux-musl/release/$(BIN_NAME)"
	@echo "=> Proxmox'a kopyalamak için:"
	@echo "   scp target/aarch64-unknown-linux-musl/release/$(BIN_NAME) root@<proxmox-ip>:/usr/local/bin/$(BIN_NAME)"

# ── Sensör izinleri (udev — sudo gerektirmez) ─────────────────────────────────
permissions:
	@if [ -f "$(UDEV_RULES)" ]; then \
		echo "=> Sensör izinleri zaten kurulu ($(UDEV_RULES)). Atlanıyor."; \
	else \
		echo "=> Sensör izinleri ayarlanıyor (udev kuralı)..."; \
		printf '%s\n' \
			'# $(BIN_NAME) — CPU/GPU güç ve sıcaklık sensörlerine kullanıcı erişimi' \
			'SUBSYSTEM=="powercap", ACTION=="add|change", RUN+="/bin/chmod -R o+r /sys/class/powercap/"' \
			'SUBSYSTEM=="powercap", ACTION=="add|change", RUN+="/bin/chmod -R o+r /sys/class/powercap/intel-rapl:0/"' \
			'SUBSYSTEM=="hwmon", ACTION=="add|change", RUN+="/bin/chmod -R o+r /sys/class/hwmon/"' \
			| sudo tee $(UDEV_RULES) > /dev/null; \
		sudo chmod 0644 $(UDEV_RULES); \
		sudo udevadm control --reload-rules; \
		sudo udevadm trigger --subsystem-match=powercap; \
		sudo udevadm trigger --subsystem-match=hwmon; \
		echo "=> ✅ Sensör izinleri aktif."; \
	fi

# ── Geliştirme: direkt çalıştır ───────────────────────────────────────────────
run: build
	@if [ ! -f "$(UDEV_RULES)" ]; then \
		echo "⚠️  Sensör izinleri kurulu değil — güç değerleri 0.0 W görünebilir."; \
		echo "   Düzeltmek için: make permissions"; \
	fi
	@echo "=> $(BIN_NAME) başlatılıyor..."
	@./target/release/$(BIN_NAME)

# ── Kurulum ───────────────────────────────────────────────────────────────────
install: build permissions
	@echo ""
	@echo "=> Binary kuruluyor: $(INSTALL_DIR)/$(BIN_NAME)"
	@sudo cp target/release/$(BIN_NAME) $(INSTALL_DIR)/$(BIN_NAME)
	@sudo chmod 0755 $(INSTALL_DIR)/$(BIN_NAME)
	@if [ -f "$(INSTALL_DIR)/$(BIN_NAME)_bin" ]; then \
		sudo rm -f $(INSTALL_DIR)/$(BIN_NAME)_bin; \
	fi
	@echo ""
	@echo "=> ✅ Kurulum tamamlandı!"
	@echo ""
	@echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
	@echo " KDE Plasma Klavye Kısayolu (aç/kapat toggle)"
	@echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
	@echo " System Settings → Shortcuts → Custom Shortcuts"
	@echo " → Edit → New → Global Shortcut → Command/URL"
	@echo " Komut:"
	@echo "   bash -c 'pgrep -x $(BIN_NAME) && pkill -x $(BIN_NAME) || $(INSTALL_DIR)/$(BIN_NAME)'"
	@echo ""
	@echo " Terminalde çalıştırmak için: $(BIN_NAME)"
	@echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# ── Kaldırma ─────────────────────────────────────────────────────────────────
uninstall:
	@echo "=> $(BIN_NAME) kaldırılıyor..."
	@pkill -x $(BIN_NAME) 2>/dev/null || true
	@sudo rm -f $(INSTALL_DIR)/$(BIN_NAME)
	@sudo rm -f $(INSTALL_DIR)/$(BIN_NAME)_bin
	@sudo rm -f $(UDEV_RULES)
	@sudo udevadm control --reload-rules
	@echo "=> $(BIN_NAME) tamamen kaldırıldı."

# ── Temizlik ──────────────────────────────────────────────────────────────────
clean:
	@echo "=> Build dosyaları temizleniyor..."
	@cargo clean
	@echo "=> Temizlendi."
