.PHONY: all deps build run install uninstall clean

# ── Proje ayarları ────────────────────────────────────────────────────────────
BIN_NAME    = rust_power_panel
INSTALL_DIR = /usr/local/bin
SYSTEMD_DIR = $(HOME)/.config/systemd/user
UDEV_RULES  = /etc/udev/rules.d/99-$(BIN_NAME).rules
SUDOERS_OLD = /etc/sudoers.d/$(BIN_NAME)

# ── Varsayılan hedef ──────────────────────────────────────────────────────────
all: deps build

# ── Sistem bağımlılıkları ─────────────────────────────────────────────────────
deps:
	@echo "=> Sistem bağımlılıkları kontrol ediliyor..."
	@MISSING=""; \
	pkg-config --exists gtk4 2>/dev/null          || MISSING="$$MISSING gtk4"; \
	pkg-config --exists gtk4-layer-shell 2>/dev/null \
	  || pkg-config --exists gtk4-layer-shell-0 2>/dev/null \
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

# ── Geliştirme: sudo olmadan direkt çalıştır ──────────────────────────────────
# Not: Sensör izinleri udev ile ayarlandıysa (make permissions) sudo gerekmez.
run: build
	@echo "=> $(BIN_NAME) başlatılıyor..."
	@./target/release/$(BIN_NAME)

# ── Sensör izinleri (udev — önerilen yöntem) ─────────────────────────────────
# GTK4/Wayland uygulaması root olarak çalışmamalı.
# Bunun yerine ilgili sysfs dosyalarına normal kullanıcı erişimi açılır.
permissions:
	@echo "=> Sensör izinleri ayarlanıyor (udev kuralı)..."
	@echo "# $(BIN_NAME) — CPU güç ve sıcaklık sensörlerine kullanıcı erişimi" \
		| sudo tee $(UDEV_RULES) > /dev/null
	@echo "# RAPL (CPU güç tüketimi)" \
		| sudo tee -a $(UDEV_RULES) > /dev/null
	@echo 'SUBSYSTEM=="powercap", ACTION=="add|change", \
		RUN+="/bin/chmod o+r /sys/class/powercap/intel-rapl:0/energy_uj"' \
		| sudo tee -a $(UDEV_RULES) > /dev/null
	@echo 'SUBSYSTEM=="powercap", ACTION=="add|change", \
		RUN+="/bin/chmod -R o+r /sys/class/powercap/"' \
		| sudo tee -a $(UDEV_RULES) > /dev/null
	@echo "# hwmon (CPU/GPU sıcaklık sensörleri)" \
		| sudo tee -a $(UDEV_RULES) > /dev/null
	@echo 'SUBSYSTEM=="hwmon", ACTION=="add|change", \
		RUN+="/bin/chmod -R o+r /sys/class/hwmon/"' \
		| sudo tee -a $(UDEV_RULES) > /dev/null
	@sudo chmod 0644 $(UDEV_RULES)
	@echo "=> udev kuralı oluşturuldu: $(UDEV_RULES)"
	@echo "=> Kurallar yükleniyor..."
	@sudo udevadm control --reload-rules
	@sudo udevadm trigger --subsystem-match=powercap
	@sudo udevadm trigger --subsystem-match=hwmon
	@echo ""
	@echo "   ✅ Sensör izinleri aktif."
	@echo "   Bir sonraki oturum açılışında kalıcı olarak geçerli olacak."
	@echo ""
	@# Eski sudoers dosyası varsa temizle (geçmiş kurulumlardan kalma)
	@if [ -f "$(SUDOERS_OLD)" ]; then \
		echo "=> Eski sudoers kaydı temizleniyor: $(SUDOERS_OLD)"; \
		sudo rm -f $(SUDOERS_OLD); \
	fi

# ── Kurulum ───────────────────────────────────────────────────────────────────
install: build permissions
	@echo ""
	@echo "=> [1/3] Binary kuruluyor: $(INSTALL_DIR)/$(BIN_NAME)"
	@sudo cp target/release/$(BIN_NAME) $(INSTALL_DIR)/$(BIN_NAME)
	@sudo chmod 0755 $(INSTALL_DIR)/$(BIN_NAME)
	@# Eski _bin wrapper'ı temizle (geçmiş kurulumdan kalma)
	@if [ -f "$(INSTALL_DIR)/$(BIN_NAME)_bin" ]; then \
		echo "=> Eski wrapper binary temizleniyor..."; \
		sudo rm -f $(INSTALL_DIR)/$(BIN_NAME)_bin; \
	fi
	@echo ""
	@echo "=> [2/3] systemd --user servisi kuruluyor..."
	@mkdir -p $(SYSTEMD_DIR)
	@printf '[Unit]\nDescription=Rust Power Panel\nAfter=graphical-session.target\nPartOf=graphical-session.target\n\n[Service]\nType=simple\nExecStart=%s\nRestart=on-failure\nRestartSec=5\nEnvironment=WAYLAND_DISPLAY=%%E{WAYLAND_DISPLAY}\nEnvironment=XDG_RUNTIME_DIR=%%E{XDG_RUNTIME_DIR}\nEnvironment=DISPLAY=%%E{DISPLAY}\n\n[Install]\nWantedBy=graphical-session.target\n' \
		$(INSTALL_DIR)/$(BIN_NAME) \
		> $(SYSTEMD_DIR)/$(BIN_NAME).service
	@systemctl --user daemon-reload
	@systemctl --user enable $(BIN_NAME).service
	@echo ""
	@echo "=> [3/3] Kurulum tamamlandı! 🎉"
	@echo ""
	@echo "   Kullanım:"
	@echo "     Terminalde çalıştır : $(BIN_NAME)"
	@echo "     Oturum açılışında   : systemctl --user start $(BIN_NAME)"
	@echo "     Otomatik başlatma   : Servis zaten etkin (enable edildi)"
	@echo "     Durdur              : systemctl --user stop $(BIN_NAME)"
	@echo "     Kapat               : Panele sağ tıkla"
	@echo ""
	@echo "   KDE Plasma autostart için:"
	@echo "     System Settings → Autostart → Add → $(INSTALL_DIR)/$(BIN_NAME)"

# ── Kaldırma ─────────────────────────────────────────────────────────────────
uninstall:
	@echo "=> $(BIN_NAME) kaldırılıyor..."
	@systemctl --user stop $(BIN_NAME).service 2>/dev/null || true
	@systemctl --user disable $(BIN_NAME).service 2>/dev/null || true
	@rm -f $(SYSTEMD_DIR)/$(BIN_NAME).service
	@systemctl --user daemon-reload
	@sudo rm -f $(INSTALL_DIR)/$(BIN_NAME)
	@sudo rm -f $(INSTALL_DIR)/$(BIN_NAME)_bin
	@sudo rm -f $(UDEV_RULES)
	@sudo rm -f $(SUDOERS_OLD)
	@sudo udevadm control --reload-rules
	@echo "=> $(BIN_NAME) tamamen kaldırıldı."

# ── Temizlik ──────────────────────────────────────────────────────────────────
clean:
	@echo "=> Build dosyaları temizleniyor..."
	@cargo clean
	@echo "=> Temizlendi."