//! CLI ve TUI kare çizimi — Rust `render_cli_frame` / `render_tui_frame`
//! fonksiyonlarının birebir portu (aynı ANSI dizileri, aynı hizalama).

const std = @import("std");
const sensors = @import("sensors.zig");

const Snapshot = sensors.Snapshot;
const GpuData = sensors.GpuData;
const GpuKind = sensors.GpuKind;

// ── ANSI ────────────────────────────────────────────────────────────────────
const R = "\x1B[0m";
const BD = "\x1B[1m";
const DM = "\x1B[2m";
const CY = "\x1B[96m";
const YL = "\x1B[93m";
const GN = "\x1B[92m";
const WH = "\x1B[97m";
const BL = "\x1B[94m";
const PR = "\x1B[95m";
const RD = "\x1B[91m";
const OR = "\x1B[38;5;208m";

fn cpCount(s: []const u8) usize {
    return std.unicode.utf8CountCodepoints(s) catch s.len;
}

// Codepoint sayısına göre sağa yasla (Rust {:>w} string semantiği).
fn rjustL(l: *Line, s: []const u8, width: usize) void {
    const cp = cpCount(s);
    if (cp < width) l.spaces(width - cp);
    l.raw(s);
}
// Codepoint sayısına göre sola yasla (Rust {:<w}).
fn ljustL(l: *Line, s: []const u8, width: usize) void {
    l.raw(s);
    const cp = cpCount(s);
    if (cp < width) l.spaces(width - cp);
}

// ── Kare yapıcı (sabit tampon) ──────────────────────────────────────────────
pub const Frame = struct {
    buf: []u8,
    len: usize = 0,

    pub fn reset(self: *Frame) void {
        self.len = 0;
    }
    pub fn slice(self: *const Frame) []const u8 {
        return self.buf[0..self.len];
    }
    fn raw(self: *Frame, s: []const u8) void {
        const n = @min(s.len, self.buf.len - self.len);
        @memcpy(self.buf[self.len..][0..n], s[0..n]);
        self.len += n;
    }
    fn print(self: *Frame, comptime fmt: []const u8, args: anytype) void {
        const s = std.fmt.bufPrint(self.buf[self.len..], fmt, args) catch return;
        self.len += s.len;
    }
    fn repeat(self: *Frame, comptime s: []const u8, n: usize) void {
        var i: usize = 0;
        while (i < n) : (i += 1) self.raw(s);
    }
};

// Küçük string yapıcı (plain/colored satırları geçici tutmak için)
const Line = struct {
    buf: [512]u8 = undefined,
    len: usize = 0,
    fn reset(self: *Line) void {
        self.len = 0;
    }
    fn raw(self: *Line, s: []const u8) void {
        const n = @min(s.len, self.buf.len - self.len);
        @memcpy(self.buf[self.len..][0..n], s[0..n]);
        self.len += n;
    }
    fn print(self: *Line, comptime fmt: []const u8, args: anytype) void {
        const s = std.fmt.bufPrint(self.buf[self.len..], fmt, args) catch return;
        self.len += s.len;
    }
    fn spaces(self: *Line, n: usize) void {
        var i: usize = 0;
        while (i < n) : (i += 1) self.raw(" ");
    }
    fn str(self: *const Line) []const u8 {
        return self.buf[0..self.len];
    }
};

fn cliTempColor(t: f32) []const u8 {
    if (t >= 80.0) return RD;
    if (t >= 60.0) return YL;
    return GN;
}

fn usageColor(pct: u32) []const u8 {
    if (pct >= 90) return RD;
    if (pct >= 75) return YL;
    return GN;
}

// Rust cli_row: │ <colored><pad> │   (genişlik = plain codepoint sayısı)
fn cliRow(f: *Frame, plain: []const u8, colored: []const u8, w: usize) void {
    const pad = w -| cpCount(plain);
    f.raw(DM ++ "│" ++ R ++ " ");
    f.raw(colored);
    f.repeat(" ", pad);
    f.raw(" " ++ DM ++ "│" ++ R ++ "\n");
}

fn cliTitledSep(f: *Frame, title: []const u8, w: usize) void {
    const inner = w + 2;
    // prefix = "─── {title} "
    var pb: Line = .{};
    pb.print("─── {s} ", .{title});
    const plen = cpCount(pb.str());
    const remaining = inner -| plen;
    f.raw(DM ++ "├");
    f.raw(pb.str());
    f.repeat("─", remaining);
    f.raw("┤" ++ R ++ "\n");
}

fn floorTemp(t: f32) u32 {
    const v = @floor(t);
    if (v <= 0) return 0;
    return @intFromFloat(v);
}

fn gpuHasPct(kind: GpuKind) bool {
    return kind == .nvidia or kind == .amd;
}

// ── Birleşik proc tablosu ───────────────────────────────────────────────────
const Combined = struct {
    name: []const u8,
    gfx: ?u32 = null,
    dec: ?u32 = null,
    enc: ?u32 = null,
    sm: ?u32 = null,
};

fn buildCombined(gpu: *const GpuData, out: []Combined) usize {
    var n: usize = 0;
    for (gpu.mediaSlice()) |*m| {
        if (n >= out.len) break;
        out[n] = .{
            .name = m.name(),
            .gfx = if (m.gfx > 0) m.gfx else null,
            .dec = if (m.dec > 0) m.dec else null,
            .enc = if (m.enc > 0) m.enc else null,
        };
        n += 1;
    }
    for (gpu.computeSlice()) |*c| {
        const sv: ?u32 = if (c.sm > 0) c.sm else null;
        var found = false;
        for (out[0..n]) |*e| {
            if (std.mem.eql(u8, e.name, c.name())) {
                e.sm = sv;
                found = true;
                break;
            }
        }
        if (!found and n < out.len) {
            out[n] = .{ .name = c.name(), .sm = sv };
            n += 1;
        }
    }
    return n;
}

// İsmi codepoint bazında kısalt: > threshold ise ilk `take` cp + "…"
fn truncName(name: []const u8, threshold: usize, take: usize, buf: []u8) []const u8 {
    if (cpCount(name) <= threshold) return name;
    var view = std.unicode.Utf8View.initUnchecked(name);
    var it = view.iterator();
    var blen: usize = 0;
    var cps: usize = 0;
    while (cps < take) : (cps += 1) {
        const cp = it.nextCodepointSlice() orelse break;
        @memcpy(buf[blen..][0..cp.len], cp);
        blen += cp.len;
    }
    const ell = "…";
    @memcpy(buf[blen..][0..ell.len], ell);
    blen += ell.len;
    return buf[0..blen];
}

// ── CLI kare ────────────────────────────────────────────────────────────────
pub fn renderCli(f: *Frame, snap: *const Snapshot) void {
    const W: usize = 38;
    const gpu = &snap.gpu;
    const cpu_watt = snap.cpu_watt;
    const cpu_temp = snap.cpu_temp;
    const cpu_percent = snap.cpu_percent;
    const total = cpu_watt + gpu.watt;

    f.reset();
    f.raw("\x1B[2J\x1B[H");

    f.print(DM ++ "┌", .{});
    f.repeat("─", W + 2);
    f.raw("┐" ++ R ++ "\n");

    // Title
    {
        var p: Line = .{};
        var c: Line = .{};
        p.raw("PowerPanel");
        p.spaces(21);
        p.print("{d:6.1}W", .{total});
        c.raw(BD ++ CY ++ "PowerPanel" ++ R);
        c.spaces(21);
        c.print(BD ++ WH ++ "{d:6.1}W" ++ R, .{total});
        cliRow(f, p.str(), c.str(), W);
    }

    midSep(f, W);

    // CPU
    {
        const tc = cliTempColor(cpu_temp);
        const uc = if (cpu_percent >= 90) RD else if (cpu_percent >= 75) YL else GN;
        var p: Line = .{};
        var c: Line = .{};
        p.print("CPU  {d:6.1}W   {d:>3}°C   ●{d:>3}%", .{ cpu_watt, floorTemp(cpu_temp), cpu_percent });
        c.print(YL ++ "CPU" ++ R ++ "  " ++ WH ++ "{d:6.1}W" ++ R ++ "   ", .{cpu_watt});
        c.print("{s}{d:>3}°C" ++ R ++ "   ", .{ tc, floorTemp(cpu_temp) });
        c.print("{s}●{d:>3}%" ++ R, .{ uc, cpu_percent });
        cliRow(f, p.str(), c.str(), W);
    }

    // GPU
    {
        const tc = cliTempColor(gpu.temp);
        const has_pct = gpuHasPct(gpu.kind);
        const uc = if (gpu.gfx_percent >= 90) RD else if (gpu.gfx_percent >= 75) YL else GN;
        var p: Line = .{};
        var c: Line = .{};
        if (has_pct) {
            p.print("GPU  {d:6.1}W   {d:>3}°C   ●{d:>3}%", .{ gpu.watt, floorTemp(gpu.temp), gpu.gfx_percent });
            c.print(GN ++ "GPU" ++ R ++ "  " ++ WH ++ "{d:6.1}W" ++ R ++ "   ", .{gpu.watt});
            c.print("{s}{d:>3}°C" ++ R ++ "   ", .{ tc, floorTemp(gpu.temp) });
            c.print("{s}●{d:>3}%" ++ R, .{ uc, gpu.gfx_percent });
        } else {
            p.print("GPU  {d:6.1}W   {d:>3}°C   ●  —", .{ gpu.watt, floorTemp(gpu.temp) });
            c.print(GN ++ "GPU" ++ R ++ "  " ++ WH ++ "{d:6.1}W" ++ R ++ "   ", .{gpu.watt});
            c.print("{s}{d:>3}°C" ++ R ++ "   ●  —", .{ tc, floorTemp(gpu.temp) });
        }
        cliRow(f, p.str(), c.str(), W);
    }

    // RAM
    if (snap.ram_total_mb > 0) {
        const ram_pct = snap.ram_used_mb * 100 / snap.ram_total_mb;
        const uc = if (ram_pct >= 90) RD else if (ram_pct >= 75) YL else GN;
        var p: Line = .{};
        var c: Line = .{};
        p.print("RAM   {d:>5}/{d:>5} MB   ●{d:>3}%", .{ snap.ram_used_mb, snap.ram_total_mb, ram_pct });
        c.print(YL ++ "RAM" ++ R ++ "   " ++ BL ++ "{d:>5}/{d:>5} MB" ++ R ++ "   ", .{ snap.ram_used_mb, snap.ram_total_mb });
        c.print("{s}●{d:>3}%" ++ R, .{ uc, ram_pct });
        cliRow(f, p.str(), c.str(), W);
    }

    // VRAM
    if (gpu.vram_total_mb > 0) {
        const vram_pct = gpu.vram_used_mb * 100 / gpu.vram_total_mb;
        const uc = if (vram_pct >= 90) RD else if (vram_pct >= 75) YL else GN;
        var p: Line = .{};
        var c: Line = .{};
        p.print("VRAM  {d:>5}/{d:>5} MB   ●{d:>3}%", .{ gpu.vram_used_mb, gpu.vram_total_mb, vram_pct });
        c.print(GN ++ "VRAM" ++ R ++ "  " ++ BL ++ "{d:>5}/{d:>5} MB" ++ R ++ "   ", .{ gpu.vram_used_mb, gpu.vram_total_mb });
        c.print("{s}●{d:>3}%" ++ R, .{ uc, vram_pct });
        cliRow(f, p.str(), c.str(), W);
    }

    // Procs
    const has_compute = gpu.compute_len > 0;
    if (gpu.media_len > 0 or has_compute) {
        cliTitledSep(f, "Procs", W);
        var combined: [sensors.MAX_PROCS * 2]Combined = undefined;
        const cn = buildCombined(gpu, &combined);

        // header
        {
            var p: Line = .{};
            var c: Line = .{};
            if (has_compute) {
                p.print("{s:<12} {s:>5} {s:>5} {s:>5} {s:>5}", .{ "Process", "GFX", "DEC", "ENC", "SM%" });
                c.print(DM ++ "{s:<12} {s:>5} {s:>5} {s:>5} {s:>5}" ++ R, .{ "Process", "GFX", "DEC", "ENC", "SM%" });
            } else {
                p.print("{s:<12} {s:>5} {s:>5} {s:>5}", .{ "Process", "GFX", "DEC", "ENC" });
                c.print(DM ++ "{s:<12} {s:>5} {s:>5} {s:>5}" ++ R, .{ "Process", "GFX", "DEC", "ENC" });
            }
            cliRow(f, p.str(), c.str(), W);
        }

        var idx: usize = 0;
        while (idx < cn and idx < 4) : (idx += 1) {
            const e = combined[idx];
            var nbuf: [80]u8 = undefined;
            const nt = truncName(e.name, 11, 10, &nbuf);
            var gb: [8]u8 = undefined;
            var db: [8]u8 = undefined;
            var ebuf: [8]u8 = undefined;
            var sb: [8]u8 = undefined;
            const gv = fmtVal4(e.gfx, &gb);
            const dv = fmtVal4(e.dec, &db);
            const ev = fmtVal4(e.enc, &ebuf);
            const sv = fmtVal4(e.sm, &sb);
            var p: Line = .{};
            var c: Line = .{};
            // plain
            p.raw("  ");
            ljustL(&p, nt, 10);
            p.raw(" ");
            rjustL(&p, gv, 5);
            p.raw(" ");
            rjustL(&p, dv, 5);
            p.raw(" ");
            rjustL(&p, ev, 5);
            if (has_compute) {
                p.raw(" ");
                rjustL(&p, sv, 5);
            }
            // colored
            c.raw("  " ++ PR);
            ljustL(&c, nt, 10);
            c.raw(R ++ " " ++ WH);
            rjustL(&c, gv, 5);
            c.raw(" ");
            rjustL(&c, dv, 5);
            c.raw(" ");
            rjustL(&c, ev, 5);
            if (has_compute) {
                c.raw(" ");
                rjustL(&c, sv, 5);
            }
            c.raw(R);
            cliRow(f, p.str(), c.str(), W);
        }
    }

    botSep(f, W);
}

fn midSep(f: *Frame, w: usize) void {
    f.raw(DM ++ "├");
    f.repeat("─", w + 2);
    f.raw("┤" ++ R ++ "\n");
}
fn botSep(f: *Frame, w: usize) void {
    f.raw(DM ++ "└");
    f.repeat("─", w + 2);
    f.raw("┘" ++ R ++ "\n");
}

// CLI fmt_v: Some(x>0) -> "{:>4}%", aksi "   —"
fn fmtVal4(v: ?u32, buf: []u8) []const u8 {
    if (v) |x| {
        if (x > 0) return std.fmt.bufPrint(buf, "{d:>4}%", .{x}) catch "   —";
    }
    return "   —";
}

// TUI fmt_v: Some(x>0) -> "{:>3}%", aksi "  —"
fn fmtVal3(v: ?u32, buf: []u8) []const u8 {
    if (v) |x| {
        if (x > 0) return std.fmt.bufPrint(buf, "{d:>3}%", .{x}) catch "  —";
    }
    return "  —";
}

// ── TUI bar ─────────────────────────────────────────────────────────────────
fn renderBar(f_plain: *Line, f_col: *Line, pct: u32, width: usize) void {
    const filled = @min(@as(usize, pct) * width / 100, width);
    const color = if (pct >= 75) RD else if (pct >= 50) OR else if (pct >= 25) YL else GN;
    // plain
    var i: usize = 0;
    while (i < width) : (i += 1) {
        f_plain.raw(if (i < filled) "█" else "░");
    }
    // colored
    if (filled > 0) {
        f_col.raw(color);
        var j: usize = 0;
        while (j < filled) : (j += 1) f_col.raw("█");
        f_col.raw(R);
    }
    if (filled < width) {
        f_col.raw(DM);
        var j: usize = filled;
        while (j < width) : (j += 1) f_col.raw("░");
        f_col.raw(R);
    }
}

fn fmtGb(f: *Line, used_mb: u32, total_mb: u32) void {
    const used = @as(f32, @floatFromInt(used_mb)) / 1024.0;
    const total = @as(f32, @floatFromInt(total_mb)) / 1024.0;
    if (total >= 100.0) {
        f.print("{d:.0}/{d:.0} GB", .{ used, total });
    } else {
        f.print("{d:.1}/{d:.0} GB", .{ used, total });
    }
}

// ── TUI kare ────────────────────────────────────────────────────────────────
pub fn renderTui(f: *Frame, snap: *const Snapshot) void {
    const W: usize = 44;
    const BAR: usize = 28;
    const gpu = &snap.gpu;
    const total = snap.cpu_watt + gpu.watt;

    f.reset();
    f.raw("\x1B[2J\x1B[H");

    f.raw(DM ++ "┌");
    f.repeat("─", W + 2);
    f.raw("┐" ++ R ++ "\n");

    // Title: "PowerPanel" (10) + {:>33.1} + "W"
    {
        var p: Line = .{};
        var c: Line = .{};
        p.print("PowerPanel{d:>33.1}W", .{total});
        c.print(BD ++ CY ++ "PowerPanel" ++ R ++ "{d:>33.1}" ++ BD ++ WH ++ "W" ++ R, .{total});
        cliRow(f, p.str(), c.str(), W);
    }
    midSep(f, W);

    // CPU bar
    barRow(f, "CPU", YL, CY, snap.cpu_percent, @min(snap.cpu_percent, 100), true, BAR, W);
    // GPU bar
    const has_pct = gpuHasPct(gpu.kind);
    const gpu_pct = if (has_pct) gpu.gfx_percent else 0;
    barRow(f, "GPU", GN, GN, gpu_pct, @min(gpu_pct, 100), has_pct, BAR, W);

    // RAM bar
    if (snap.ram_total_mb > 0) {
        const ram_pct = @min(snap.ram_used_mb * 100 / snap.ram_total_mb, 100);
        var valp: Line = .{};
        fmtGb(&valp, snap.ram_used_mb, snap.ram_total_mb);
        gbBarRow(f, "RAM ", YL, ram_pct, valp.str(), BAR, W);
    }
    // VRAM bar
    if (gpu.vram_total_mb > 0) {
        const vram_pct = @min(gpu.vram_used_mb * 100 / gpu.vram_total_mb, 100);
        var valp: Line = .{};
        fmtGb(&valp, gpu.vram_used_mb, gpu.vram_total_mb);
        gbBarRow(f, "VRAM", GN, vram_pct, valp.str(), BAR, W);
    }

    // Temp & power strip
    midSep(f, W);
    {
        const cpu_tc = cliTempColor(snap.cpu_temp);
        const gpu_tc = cliTempColor(gpu.temp);
        var p: Line = .{};
        var c: Line = .{};
        p.print("CPU  {d:>3}°C  {d:>5.1}W        GPU  {d:>3}°C  {d:>5.1}W", .{ floorTemp(snap.cpu_temp), snap.cpu_watt, floorTemp(gpu.temp), gpu.watt });
        c.print(YL ++ "CPU" ++ R ++ "  {s}{d:>3}°C" ++ R ++ "  " ++ WH ++ "{d:>5.1}W" ++ R ++ "        ", .{ cpu_tc, floorTemp(snap.cpu_temp), snap.cpu_watt });
        c.print(GN ++ "GPU" ++ R ++ "  {s}{d:>3}°C" ++ R ++ "  " ++ WH ++ "{d:>5.1}W" ++ R, .{ gpu_tc, floorTemp(gpu.temp), gpu.watt });
        cliRow(f, p.str(), c.str(), W);
    }

    // Procs
    const has_compute = gpu.compute_len > 0;
    if (gpu.media_len > 0 or has_compute) {
        cliTitledSep(f, "Procs", W);
        var combined: [sensors.MAX_PROCS * 2]Combined = undefined;
        const cn = buildCombined(gpu, &combined);

        {
            var p: Line = .{};
            var c: Line = .{};
            if (has_compute) {
                p.print("{s:<12} {s:>7} {s:>7} {s:>7} {s:>7}", .{ "Process", "GFX", "DEC", "ENC", "SM%" });
                c.print(DM ++ "{s:<12} {s:>7} {s:>7} {s:>7} {s:>7}" ++ R, .{ "Process", "GFX", "DEC", "ENC", "SM%" });
            } else {
                p.print("{s:<14} {s:>9} {s:>9} {s:>9}", .{ "Process", "GFX", "DEC", "ENC" });
                c.print(DM ++ "{s:<14} {s:>9} {s:>9} {s:>9}" ++ R, .{ "Process", "GFX", "DEC", "ENC" });
            }
            cliRow(f, p.str(), c.str(), W);
        }

        var idx: usize = 0;
        while (idx < cn and idx < 4) : (idx += 1) {
            const e = combined[idx];
            var nbuf: [80]u8 = undefined;
            var gb: [8]u8 = undefined;
            var db: [8]u8 = undefined;
            var ebuf: [8]u8 = undefined;
            var sb: [8]u8 = undefined;
            const gv = fmtVal3(e.gfx, &gb);
            const dv = fmtVal3(e.dec, &db);
            const ev = fmtVal3(e.enc, &ebuf);
            const sv = fmtVal3(e.sm, &sb);
            var p: Line = .{};
            var c: Line = .{};
            if (has_compute) {
                const nt = truncName(e.name, 11, 10, &nbuf);
                p.raw("  ");
                ljustL(&p, nt, 10);
                inline for (.{ gv, dv, ev, sv }) |v| {
                    p.raw(" ");
                    rjustL(&p, v, 7);
                }
                c.raw("  " ++ PR);
                ljustL(&c, nt, 10);
                c.raw(R ++ " " ++ WH);
                rjustL(&c, gv, 7);
                inline for (.{ dv, ev, sv }) |v| {
                    c.raw(" ");
                    rjustL(&c, v, 7);
                }
                c.raw(R);
            } else {
                const nt = truncName(e.name, 13, 12, &nbuf);
                p.raw("  ");
                ljustL(&p, nt, 12);
                inline for (.{ gv, dv, ev }) |v| {
                    p.raw(" ");
                    rjustL(&p, v, 9);
                }
                c.raw("  " ++ PR);
                ljustL(&c, nt, 12);
                c.raw(R ++ " " ++ WH);
                rjustL(&c, gv, 9);
                inline for (.{ dv, ev }) |v| {
                    c.raw(" ");
                    rjustL(&c, v, 9);
                }
                c.raw(R);
            }
            cliRow(f, p.str(), c.str(), W);
        }
    }

    botSep(f, W);
}

// pct değerli bar satırı (CPU/GPU)
fn barRow(f: *Frame, label: []const u8, lbl_color: []const u8, val_color: []const u8, pct: u32, bar_pct: u32, has_pct: bool, bar: usize, w: usize) void {
    var bar_p: Line = .{};
    var bar_c: Line = .{};
    renderBar(&bar_p, &bar_c, bar_pct, bar);

    // pct değeri 10 karaktere sağa yaslı
    var sp: Line = .{};
    var sc: Line = .{};
    if (has_pct) {
        var tmp: [16]u8 = undefined;
        const vs = std.fmt.bufPrint(&tmp, "{d:>3}%", .{pct}) catch "  0%";
        const padn = 10 -| cpCount(vs);
        sp.spaces(padn);
        sp.raw(vs);
        sc.spaces(padn);
        sc.raw(val_color);
        sc.raw(vs);
        sc.raw(R);
    } else {
        const vs = "  —";
        const padn = 10 -| cpCount(vs);
        sp.spaces(padn);
        sp.raw(vs);
        sc.spaces(padn);
        sc.raw(DM);
        sc.raw(vs);
        sc.raw(R);
    }

    var p: Line = .{};
    var c: Line = .{};
    p.print("{s}  ", .{label});
    p.raw(bar_p.str());
    p.raw(" ");
    p.raw(sp.str());
    c.raw(lbl_color);
    c.print("{s}" ++ R ++ "  ", .{label});
    c.raw(bar_c.str());
    c.raw(" ");
    c.raw(sc.str());
    cliRow(f, p.str(), c.str(), w);
}

// GB değerli bar satırı (RAM/VRAM)
fn gbBarRow(f: *Frame, label: []const u8, lbl_color: []const u8, pct: u32, val: []const u8, bar: usize, w: usize) void {
    var bar_p: Line = .{};
    var bar_c: Line = .{};
    renderBar(&bar_p, &bar_c, pct, bar);

    var p: Line = .{};
    var c: Line = .{};
    p.print("{s} ", .{label});
    p.raw(bar_p.str());
    p.raw(" ");
    p.raw(val);
    c.raw(lbl_color);
    c.print("{s}" ++ R ++ " ", .{label});
    c.raw(bar_c.str());
    c.raw(" " ++ BL);
    c.raw(val);
    c.raw(R);
    cliRow(f, p.str(), c.str(), w);
}
