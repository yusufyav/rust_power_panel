.PHONY: all build run cli tui debug small clean help

BIN = zig-out/bin/power_panel

all: build

help:
	@echo "PowerPanel (Zig portu) — Hedefler"
	@echo "  make build    ReleaseFast + strip derle"
	@echo "  make small    ReleaseSmall + strip (en küçük binary)"
	@echo "  make run      Derle + GUI uyarısıyla çalıştır"
	@echo "  make cli      CLI modunda çalıştır"
	@echo "  make tui      TUI modunda çalıştır"
	@echo "  make debug    Sensör teşhisi"
	@echo "  make clean    zig-out + .zig-cache temizle"

build:
	zig build -Doptimize=ReleaseFast

small:
	zig build -Doptimize=ReleaseSmall

run: build
	./$(BIN)

cli: build
	./$(BIN) --cli

tui: build
	./$(BIN) --tui

debug: build
	./$(BIN) --debug

clean:
	rm -rf zig-out .zig-cache
