# PowerPanel — Zig portu
#
# Kullanım: proje kökündeki zig/ dizininden çalıştırın.
#   cd zig && make install
#
# ⚠️  Rust ve Zig sürümleri aynı binary adını kullanır (power_panel).
#    İkisi aynı anda kurulu olmamalı; biri kuruluyken diğeri kurulursa üzerine yazar.

.PHONY: all build small server server-arm64 run cli tui debug clean help permissions install uninstall

# ── Ayarlar ──────────────────────────────────────────────────────────────────
BIN_NAME     = power_panel
INSTALL_NAME = power_panel
INSTALL_DIR  = /usr/local/bin
UDEV_RULES   = /etc/udev/rules.d/99-$(INSTALL_NAME).rules
BIN          = zig-out/bin/$(BIN_NAME)

all: build

# ── Yardım ───────────────────────────────────────────────────────────────────
help:
	@echo ""
	@echo "PowerPanel — Zig portu"
	@echo "Çalıştırma dizini: zig/"
	@echo "────────────────────────────────────────"
	@echo "  make               Derle (ReleaseFast)"
	@echo "  make small         ReleaseSmall + strip (en küçük binary)"
	@echo "  make server        Statik musl binary — GUI yok, Proxmox/sunucu için"
	@echo "  make server-arm64  ARM64 sunucu için statik musl binary"
	@echo "  make run           Derle + çalıştır (--gui)"
	@echo "  make cli           CLI modunda çalıştır"
	@echo "  make tui           TUI modunda çalıştır"
	@echo "  make debug         Sensör teşhisi"
	@echo "  make permissions   Sensör izinlerini kur (udev, tek seferlik)"
	@echo "  make install       Sisteme kur (/usr/local/bin/$(INSTALL_NAME))"
	@echo "  make uninstall     Kurulumu kaldır"
	@echo "  make clean         zig-out + .zig-cache temizle"
	@echo "  make help          Bu mesajı göster"
	@echo ""
	@echo "NOT: 'make run' öncesinde 'make permissions' yapılmamışsa"
	@echo "     güç değerleri 0.0 W görünebilir (RAPL erişim sorunu)."
	@echo ""

# ── Derleme ───────────────────────────────────────────────────────────────────
build:
	@command -v zig > /dev/null 2>&1 \
	  || { echo "❌ Zig kurulu değil. Kurun: https://ziglang.org/download"; exit 1; }
	@zig build -Doptimize=ReleaseFast
	@echo "=> Derleme tamamlandı: $(BIN)"

small:
	@zig build -Doptimize=ReleaseSmall

# Proxmox/sunucu: GTK yok, musl ile tamamen statik binary
server:
	@command -v zig > /dev/null 2>&1 \
	  || { echo "❌ Zig kurulu değil. Kurun: https://ziglang.org/download"; exit 1; }
	@zig build -Doptimize=ReleaseFast -Dgui=false -Dtarget=x86_64-linux-musl
	@echo "=> Sunucu binary'si hazır: $(BIN)"
	@echo "=> Proxmox'a kopyalamak için:"
	@echo "   scp $(BIN) root@<proxmox-ip>:/usr/local/bin/power_panel"

server-arm64:
	@command -v zig > /dev/null 2>&1 \
	  || { echo "❌ Zig kurulu değil. Kurun: https://ziglang.org/download"; exit 1; }
	@zig build -Doptimize=ReleaseFast -Dgui=false -Dtarget=aarch64-linux-musl
	@echo "=> ARM64 sunucu binary'si hazır: $(BIN)"
	@echo "=> Proxmox'a kopyalamak için:"
	@echo "   scp $(BIN) root@<proxmox-ip>:/usr/local/bin/power_panel"

# ── Sensör izinleri (udev — sudo gerektirmez) ─────────────────────────────────
permissions:
	@if [ -f "$(UDEV_RULES)" ]; then \
		echo "=> Sensör izinleri zaten kurulu ($(UDEV_RULES)). Atlanıyor."; \
	else \
		echo "=> Sensör izinleri ayarlanıyor (udev kuralı)..."; \
		printf '%s\n' \
			'# $(INSTALL_NAME) — CPU/GPU güç ve sıcaklık sensörlerine kullanıcı erişimi' \
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
	@echo "=> $(INSTALL_NAME) başlatılıyor (--gui)..."
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
	@echo ""
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
	@echo " Terminalde: $(INSTALL_NAME)         # GUI (etiketli panel)"
	@echo "             $(INSTALL_NAME) --gui2  # GUI (bar panel)"
	@echo "             $(INSTALL_NAME) --cli   # terminal modu"
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
