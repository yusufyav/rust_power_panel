.PHONY: all deps build run install uninstall clean permissions

# ── Proje ayarları ────────────────────────────────────────────────────────────
BIN_NAME    = rust_power_panel
INSTALL_DIR = /usr/local/bin
UDEV_RULES  = /etc/udev/rules.d/99-$(BIN_NAME).rules

# ── Varsayılan hedef ──────────────────────────────────────────────────────────
all: deps build

# ── Sistem bağımlılıkları ─────────────────────────────────────────────────────
deps:
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

# ── Sensör izinleri (udev — sudo gerektirmez) ─────────────────────────────────
permissions:
	@echo "=> Sensör izinleri ayarlanıyor (udev kuralı)..."
	@printf '%s\n' \
		'# $(BIN_NAME) — CPU/GPU güç ve sıcaklık sensörlerine kullanıcı erişimi' \
		'# RAPL (CPU güç tüketimi — Intel ve AMD)' \
		'SUBSYSTEM=="powercap", ACTION=="add|change", RUN+="/bin/chmod -R o+r /sys/class/powercap/"' \
		'# RAPL (CPU güç tüketimi — Intel ve AMD)' \
		'SUBSYSTEM=="powercap", ACTION=="add|change", RUN+="/bin/chmod -R o+r /sys/class/powercap/intel-rapl:0/"' \
		'# hwmon (CPU/GPU sıcaklık sensörleri)' \
		'SUBSYSTEM=="hwmon", ACTION=="add|change", RUN+="/bin/chmod -R o+r /sys/class/hwmon/"' \
		| sudo tee $(UDEV_RULES) > /dev/null
	@sudo chmod 0644 $(UDEV_RULES)
	@sudo udevadm control --reload-rules
	@sudo udevadm trigger --subsystem-match=powercap
	@sudo udevadm trigger --subsystem-match=hwmon
	@echo "=> ✅ Sensör izinleri aktif."

# ── Geliştirme: direkt çalıştır ───────────────────────────────────────────────
# make permissions yapıldıktan sonra sudo gerekmez.
run: build
	@echo "=> $(BIN_NAME) başlatılıyor..."
	@./target/release/$(BIN_NAME)

# ── Kurulum ───────────────────────────────────────────────────────────────────
# Klavye kısayolu kullanımına uygun: aç/kapat toggle script ile çalışır.
install: build permissions
	@echo ""
	@echo "=> [1/2] Binary kuruluyor: $(INSTALL_DIR)/$(BIN_NAME)"
	@sudo cp target/release/$(BIN_NAME) $(INSTALL_DIR)/$(BIN_NAME)
	@sudo chmod 0755 $(INSTALL_DIR)/$(BIN_NAME)
	@# Eski _bin wrapper'ı temizle (geçmiş kurulumdan kalma)
	@if [ -f "$(INSTALL_DIR)/$(BIN_NAME)_bin" ]; then \
		echo "=> Eski wrapper binary temizleniyor..."; \
		sudo rm -f $(INSTALL_DIR)/$(BIN_NAME)_bin; \
	fi
	@echo ""
	@echo "=> [2/2] Kurulum tamamlandı! 🎉"
	@echo ""
	@echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
	@echo " KDE Plasma Klavye Kısayolu Kurulumu"
	@echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
	@echo ""
	@echo " System Settings → Shortcuts → Custom Shortcuts"
	@echo " → Edit → New → Global Shortcut → Command/URL"
	@echo " Komut:"
	@echo "   bash -c 'pgrep -x $(BIN_NAME) && pkill -x $(BIN_NAME) || $(INSTALL_DIR)/$(BIN_NAME)'"
	@echo ""
	@echo " Bu komut:"
	@echo "   • Panel açıksa kapatır"
	@echo "   • Panel kapalıysa açar"
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