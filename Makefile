.PHONY: all deps build run install clean

# Proje adımız
BIN_NAME = rust_power_panel

# Varsayılan hedef
all: deps run

deps:
	@echo "=> Sistem bağımlılıkları kontrol ediliyor..."
	@if pkg-config --exists gtk4 gtk4-layer-shell; then \
		echo "=> Bağımlılıklar zaten kurulu. Atlanıyor."; \
	else \
		echo "=> Eksik paketler tespit edildi, kuruluyor..."; \
		if command -v pacman > /dev/null; then \
			sudo pacman -S --needed gtk4 gtk4-layer-shell pkgconf; \
		elif command -v apt-get > /dev/null; then \
			sudo apt-get update && sudo apt-get install -y libgtk-4-dev libgtk4-layer-shell-dev pkg-config; \
		elif command -v dnf > /dev/null; then \
			sudo dnf install -y gtk4-devel gtk4-layer-shell-devel pkgconf-pkg-config; \
		else \
			echo "=> Desteklenmeyen paket yöneticisi! Lütfen GTK4 ve gtk4-layer-shell kurun."; \
			exit 1; \
		fi \
	fi

build:
	@echo "=> Rust projesi derleniyor (Release)..."
	cargo build --release

# Geliştirme aşamasında test etmek için (Sudo -E ile çalıştırır ki Wayland değişkenleri silinmesin)
run: build
	@echo "=> $(BIN_NAME) başlatılıyor..."
	sudo -E ./target/release/$(BIN_NAME)

# "Tek Komutla Kurulum" Sihri Burada
install: build
	@echo "=> 1/4: Binary sisteme yükleniyor..."
	@sudo cp target/release/$(BIN_NAME) /usr/local/bin/$(BIN_NAME)_bin
	@sudo chmod +x /usr/local/bin/$(BIN_NAME)_bin

	@echo "=> 2/4: Akıllı Wrapper Script oluşturuluyor..."
	@echo '#!/bin/bash' | sudo tee /usr/local/bin/$(BIN_NAME) > /dev/null
	@echo 'sudo -E /usr/local/bin/$(BIN_NAME)_bin "$$@"' | sudo tee -a /usr/local/bin/$(BIN_NAME) > /dev/null
	@sudo chmod +x /usr/local/bin/$(BIN_NAME)

	@echo "=> 3/4: Kernel Sensör İzinleri (Sudoers) ayarlanıyor..."
	@echo "$$USER ALL=(ALL) NOPASSWD: /usr/local/bin/$(BIN_NAME)_bin" | sudo tee /etc/sudoers.d/$(BIN_NAME) > /dev/null
	@echo "Defaults!/usr/local/bin/$(BIN_NAME)_bin env_keep += \"WAYLAND_DISPLAY XDG_RUNTIME_DIR\"" | sudo tee -a /etc/sudoers.d/$(BIN_NAME) > /dev/null
	@sudo chmod 0440 /etc/sudoers.d/$(BIN_NAME)

	@echo "=> 4/4: Kurulum Tamamlandı! 🎉"
	@echo "Artık terminale sadece '$(BIN_NAME)' yazarak veya Hyprland/Sway config dosyanızdan çağırarak paneli kullanabilirsiniz."

clean:
	@echo "=> Temizleniyor..."
	cargo clean