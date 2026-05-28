const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    // -Dgui=false ile GTK4 GUI'siz (yalnızca CLI/TUI/debug) lean derleme yapılır.
    const gui = b.option(bool, "gui", "GTK4 GUI modlarını derle (varsayılan: true)") orelse true;

    const options = b.addOptions();
    options.addOption(bool, "gui", gui);

    const mod = b.createModule(.{
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
        // Release modlarında sembolleri sil (Rust profilindeki strip=true karşılığı)
        .strip = optimize != .Debug,
    });
    mod.addOptions("build_options", options);

    if (gui) {
        mod.linkSystemLibrary("gtk4", .{});
        mod.linkSystemLibrary("gtk4-layer-shell-0", .{});
        mod.linkSystemLibrary("cairo", .{});
    }

    const exe = b.addExecutable(.{
        .name = "power_panel",
        .root_module = mod,
    });
    b.installArtifact(exe);

    const run_cmd = b.addRunArtifact(exe);
    run_cmd.step.dependOn(b.getInstallStep());
    if (b.args) |args| run_cmd.addArgs(args);

    const run_step = b.step("run", "PowerPanel'i çalıştır (örn: zig build run -- --cli)");
    run_step.dependOn(&run_cmd.step);
}
