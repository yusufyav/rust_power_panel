//! PowerPanel — Zig portu (çekirdek + CLI/TUI/debug modları).
//!
//! Rust `src/main.rs`'in birebir klonu. GUI (GTK4 layer-shell) modları sonraki
//! turda eklenecek; bu binary --cli / --tui / --debug / --help / --version
//! modlarını ve tam sensör çekirdeğini içerir.

const std = @import("std");
const os = @import("os.zig");
const sensors = @import("sensors.zig");
const render = @import("render.zig");
const build_options = @import("build_options");

// GUI yalnızca -Dgui=true (varsayılan) derlemede dahil edilir; aksi halde
// gtk hiç derlenmez (lean CLI/TUI/debug binary).
const gui = if (build_options.gui) @import("gui.zig") else struct {
    pub fn run1() void {
        os.stdout("⚠️  Bu binary -Dgui=false ile derlendi; GUI yok.\n");
    }
    pub fn run2() void {
        os.stdout("⚠️  Bu binary -Dgui=false ile derlendi; GUI yok.\n");
    }
};

const VERSION = "0.1.0";

pub fn main(init: std.process.Init.Minimal) void {
    var args = init.args.iterate();
    _ = args.next(); // argv[0]
    const first = args.next();

    if (first) |arg| {
        if (std.mem.eql(u8, arg, "--help") or std.mem.eql(u8, arg, "-h")) {
            printHelp();
        } else if (std.mem.eql(u8, arg, "--cli")) {
            runLoop(.cli);
        } else if (std.mem.eql(u8, arg, "--tui")) {
            runLoop(.tui);
        } else if (std.mem.eql(u8, arg, "--debug")) {
            runDiagnostics();
        } else if (std.mem.eql(u8, arg, "--version") or std.mem.eql(u8, arg, "-v")) {
            os.stdout("PowerPanel v" ++ VERSION ++ "\n");
            os.stdout("Minimal power monitoring tool for Linux\n");
        } else if (std.mem.eql(u8, arg, "--gui2")) {
            gui.run2();
        } else if (std.mem.eql(u8, arg, "--gui")) {
            gui.run1();
        } else {
            var buf: [256]u8 = undefined;
            os.stdout(std.fmt.bufPrint(&buf, "❌ Bilinmeyen parametre: {s}\n", .{arg}) catch "");
            os.stdout("Yardım için: power_panel --help\n");
        }
        return;
    }

    // Argümansız: Rust varsayılanı build_ui → gui1 (etiketli panel) başlat.
    gui.run1();
}

const Mode = enum { cli, tui };

fn runLoop(mode: Mode) void {
    var frame_buf: [65536]u8 = undefined;
    var frame = render.Frame{ .buf = &frame_buf };
    var mon = sensors.Monitor.init(std.heap.c_allocator);

    while (true) {
        const snap = mon.sample();
        switch (mode) {
            .cli => render.renderCli(&frame, &snap),
            .tui => render.renderTui(&frame, &snap),
        }
        os.stdout(frame.slice());
        os.sleepMs(1000);
    }
}

fn printHelp() void {
    os.stdout(
        \\PowerPanel - Minimal Linux Güç İzleme Aracı
        \\
        \\KULLANIM:
        \\  power_panel [SEÇENEKLER]
        \\
        \\SEÇENEKLER:
        \\  --help, -h       Bu yardım mesajını gösterir
        \\  --version, -v    Versiyon bilgisini gösterir
        \\  --cli            CLI (Terminal) modunda çalıştır
        \\  --tui            TUI (Bar görünümlü) modunda çalıştır
        \\  --debug          Sensör teşhisini çalıştır ve çık
        \\
        \\ÖRNEKLER:
        \\  power_panel --cli        # Terminal modunda sürekli güncelleme
        \\  power_panel --debug      # Sensör erişimini ve GPU durumunu kontrol et
        \\
        \\ÖZELLİKLER:
        \\  • CPU/GPU güç tüketimi ve sıcaklık
        \\  • GPU decode/encode kullanımı
        \\  • AMD, Intel, Nvidia desteği
        \\  • Düşük kaynak kullanımı (<10 MB RAM)
        \\
    );
}

// ── Teşhis (--debug) ────────────────────────────────────────────────────────
fn runDiagnostics() void {
    var buf: [512]u8 = undefined;
    os.stdout("\n=== 🔍 POWERPANEL DIAGNOSTICS (Zig) ===\n");

    // CPU power (RAPL)
    if (sensors.findRaplPath()) |path| {
        if (os.readU64(path)) |_| {
            os.stdout(std.fmt.bufPrint(&buf, "✅ CPU Power : OK -> {s}\n", .{path}) catch "");
        } else {
            os.stdout(std.fmt.bufPrint(&buf, "❌ CPU Power : FAIL -> {s}\n", .{path}) catch "");
        }
    } else {
        os.stdout("⚠️  CPU Power : NOT FOUND\n");
    }

    os.stdout("\n-- Çekirdekteki Tüm Donanım Sensörleri (/sys/class/hwmon) --\n");
    var hwmon_count: u32 = 0;
    if (os.DirIter.open("/sys/class/hwmon")) |*itc| {
        var it = itc.*;
        defer it.close();
        while (it.next()) |e| {
            var pb: [256]u8 = undefined;
            const np = std.fmt.bufPrint(&pb, "/sys/class/hwmon/{s}/name", .{e.name}) catch continue;
            var nb: [64]u8 = undefined;
            const name = os.readTrim(np, &nb) orelse continue;

            var temp_buf: [128]u8 = undefined;
            var temp_str: []const u8 = "Yok/Okunamıyor";
            var i: u32 = 1;
            while (i <= 4) : (i += 1) {
                var tp: [256]u8 = undefined;
                const tpath = std.fmt.bufPrint(&tp, "/sys/class/hwmon/{s}/temp{d}_input", .{ e.name, i }) catch continue;
                if (os.readU64(tpath)) |raw| {
                    var lp: [256]u8 = undefined;
                    const lpath = std.fmt.bufPrint(&lp, "/sys/class/hwmon/{s}/temp{d}_label", .{ e.name, i }) catch "";
                    var lb: [64]u8 = undefined;
                    const label = os.readTrim(lpath, &lb) orelse "";
                    const c = @as(f32, @floatFromInt(raw)) / 1000.0;
                    temp_str = std.fmt.bufPrint(&temp_buf, "{d:.1} °C (temp{d} - {s})", .{ c, i, label }) catch "?";
                    break;
                }
            }
            os.stdout(std.fmt.bufPrint(&buf, "   🏷️ İsim: {s:<12} | 🌡️ Sıcaklık: {s}\n", .{ name, temp_str }) catch "");
            hwmon_count += 1;
        }
    }
    if (hwmon_count == 0) {
        os.stdout("❌ Çekirdekte hiçbir donanım sensörü bulunamadı!\n");
        os.stdout("🚨 DİKKAT: Sensörler yetkisiz kullanıcılara kapalı olabilir (udev kuralı?).\n");
    }

    os.stdout("\n-- Seçilen Ana CPU Sensörü --\n");
    if (sensors.detectCpuTempPath()) |p| {
        var pp = p;
        os.stdout(std.fmt.bufPrint(&buf, "✅ Panel bunu kullanacak: {s}\n", .{pp.slice()}) catch "");
    } else {
        os.stdout("❌ Uygun bir CPU sensörü eşleştirilemedi.\n");
    }

    os.stdout("\n-- GPU Durumu --\n");
    const gpu = sensors.detectGpu();
    switch (gpu) {
        .nvidia => os.stdout("✅ GPU Type  : NVIDIA (NVML Initialized)\n"),
        .amd => |amd| {
            os.stdout(std.fmt.bufPrint(&buf, "✅ GPU Type  : AMD (VCN Instances: {d})\n", .{amd.vcn}) catch "");
            var hw = amd.hwmon;
            os.stdout(std.fmt.bufPrint(&buf, "✅ GPU HWMon : {s}\n", .{hw.slice()}) catch "");
        },
        .intel => |intel| {
            os.stdout("✅ GPU Type  : Intel (i915/xe)\n");
            if (intel.hwmon) |h| {
                var hh = h;
                os.stdout(std.fmt.bufPrint(&buf, "   hwmon     : {s}\n", .{hh.slice()}) catch "");
            } else os.stdout("   hwmon     : Yok (entegre GPU)\n");
            if (intel.rapl_uncore) |r| {
                var rr = r;
                os.stdout(std.fmt.bufPrint(&buf, "   RAPL iGPU : {s}\n", .{rr.slice()}) catch "");
            } else os.stdout("   RAPL iGPU : Yok\n");
        },
        .none => os.stdout("❌ GPU Type  : NOT FOUND (Desteklenmeyen kart)\n"),
    }
    os.stdout("======================================\n\n");
}
