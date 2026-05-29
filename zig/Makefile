.PHONY: all build small run cli tui debug clean help permissions install uninstall

# ── Ayarlar ──────────────────────────────────────────────────────────────────
# BIN_NAME    : zig build çıktısının adı (build.zig'de sabit: power_panel)
# INSTALL_NAME: sisteme kurulurken kullanılacak ad. Rust sürümüyle ÇAKIŞMAMASI
#               için varsayılan "power_panel-zig". Rust sürümünün yerine kurmak
#               için: make install INSTALL_NAME=power_panel
BIN_NAME     = power_panel
INSTALL_NAME = power_panel-zig
INSTALL_DIR  = /usr/local/bin
UDEV_RULES   = /etc/udev/rules.d/99-$(INSTALL_NAME).rules
BIN          = zig-out/bin/$(BIN_NAME)

all: build

help:
	@echo ""
	@echo "PowerPanel (Zig portu) — Hedefler"
	@echo "────────────────────────────────────────"
	@echo "  make build        ReleaseFast + strip derle"
	@echo "  make small        ReleaseSmall + strip (en küçük binary)"
	@echo "  make run          Derle + çalıştır (--gui)"
	@echo "  make cli          CLI modunda çalıştır"
	@echo "  make tui          TUI modunda çalıştır"
	@echo "  make debug        Sensör teşhisi"
	@echo "  make permissions  Sensör izinlerini kur (udev, tek seferlik)"
	@echo "  make install      Sisteme kur ($(INSTALL_NAME)) + izinler"
	@echo "  make uninstall    Kurulumu kaldır"
	@echo "  make clean        zig-out + .zig-cache temizle"
	@echo ""
	@echo "NOT: Rust sürümünün yerine kurmak için:"
	@echo "     make install INSTALL_NAME=power_panel"
	@echo ""

# ── Derleme ───────────────────────────────────────────────────────────────────
build:
	@command -v zig > /dev/null 2>&1 \
	  || { echo "❌ Zig kurulu değil. Kurun: https://ziglang.org/download"; exit 1; }
	zig build -Doptimize=ReleaseFast

small:
	zig build -Doptimize=ReleaseSmall

# ── Sensör izinleri (udev — sudo gerektirmez) ─────────────────────────────────
permissions:
	@if [ -f "$(UDEV_RULES)" ]; then \
		echo "=> Sensör izinleri zaten kurulu ($(UDEV_RULES)). Atlanıyor."; \
	else \
		echo "=> Sensör izinleri ayarlanıyor (udev kuralı)..."; \
		printf '%s\n' \
			'# $(INSTALL_NAME) — CPU/GPU güç ve sıcaklık sensörlerine kullanıcı erişimi' \
			'# RAPL (CPU güç tüketimi — Intel ve AMD)' \
			'SUBSYSTEM=="powercap", ACTION=="add|change", RUN+="/bin/chmod -R o+r /sys/class/powercap/"' \
			'# RAPL (CPU güç tüketimi — Intel ve AMD)' \
			'SUBSYSTEM=="powercap", ACTION=="add|change", RUN+="/bin/chmod -R o+r /sys/class/powercap/intel-rapl:0/"' \
			'# hwmon (CPU/GPU sıcaklık sensörleri)' \
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
	@if [ ! -f "$(UDEV_RULES)" ] && [ ! -f "/etc/udev/rules.d/99-$(BIN_NAME).rules" ]; then \
		echo "⚠️  Sensör izinleri kurulu değil — güç değerleri 0.0 W görünebilir."; \
		echo "   Düzeltmek için: make permissions"; \
	fi
	@echo "=> $(BIN_NAME) başlatılıyor (--gui)..."
	@./$(BIN)

cli: build
	@./$(BIN) --cli

tui: build
	@./$(BIN) --tui

debug: build
	@./$(BIN) --debug

# ── Kurulum ───────────────────────────────────────────────────────────────────
install: build permissions
	@echo ""
	@echo "=> Binary kuruluyor: $(INSTALL_DIR)/$(INSTALL_NAME)"
	@sudo cp $(BIN) $(INSTALL_DIR)/$(INSTALL_NAME)
	@sudo chmod 0755 $(INSTALL_DIR)/$(INSTALL_NAME)
	@echo "=> ✅ Kurulum tamamlandı!"
	@echo ""
	@echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
	@echo " KDE Plasma Klavye Kısayolu (aç/kapat toggle)"
	@echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
	@echo " System Settings → Shortcuts → Custom Shortcuts"
	@echo " → Edit → New → Global Shortcut → Command/URL"
	@echo " Komut:"
	@echo "   bash -c 'pgrep -x $(INSTALL_NAME) && pkill -x $(INSTALL_NAME) || $(INSTALL_DIR)/$(INSTALL_NAME)'"
	@echo ""
	@echo " Terminalde: $(INSTALL_NAME)        # --gui (etiketli panel)"
	@echo "             $(INSTALL_NAME) --gui2 # bar panel"
	@echo "             $(INSTALL_NAME) --cli  # terminal"
	@echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# ── Kaldırma ─────────────────────────────────────────────────────────────────
uninstall:
	@echo "=> $(INSTALL_NAME) kaldırılıyor..."
	@pkill -x $(INSTALL_NAME) 2>/dev/null || true
	@sudo rm -f $(INSTALL_DIR)/$(INSTALL_NAME)
	@sudo rm -f $(UDEV_RULES)
	@sudo udevadm control --reload-rules
	@echo "=> $(INSTALL_NAME) kaldırıldı (udev kuralı dahil)."

# ── Temizlik ──────────────────────────────────────────────────────────────────
clean:
	@echo "=> Build dosyaları temizleniyor..."
	@rm -rf zig-out .zig-cache
	@echo "=> Temizlendi."
