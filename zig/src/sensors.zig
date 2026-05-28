//! Sensör çekirdeği — Rust `main.rs`'teki tespit + okuma mantığının birebir portu.
//!
//! Donanım tespiti (NVIDIA/AMD/Intel), RAPL güç, CPU sıcaklık, AMD/Intel VCN
//! fdinfo izleme ve /proc tabanlı CPU%/RAM okuması burada toplanır.

const std = @import("std");
const os = @import("os.zig");
const nvml = @import("nvml.zig");

pub const MAX_PROCS = 64;

pub const GpuKind = enum { unknown, nvidia, amd, intel };

pub const PathBuf = struct {
    buf: [256]u8 = undefined,
    len: usize = 0,

    pub fn set(self: *PathBuf, s: []const u8) void {
        const n = @min(s.len, self.buf.len);
        @memcpy(self.buf[0..n], s[0..n]);
        self.len = n;
    }
    pub fn slice(self: *const PathBuf) []const u8 {
        return self.buf[0..self.len];
    }
};

pub const MediaProc = struct {
    name_buf: [64]u8 = undefined,
    name_len: usize = 0,
    dec: u32 = 0,
    enc: u32 = 0,
    gfx: u32 = 0,

    pub fn name(self: *const MediaProc) []const u8 {
        return self.name_buf[0..self.name_len];
    }
    fn setName(self: *MediaProc, s: []const u8) void {
        const n = @min(s.len, self.name_buf.len);
        @memcpy(self.name_buf[0..n], s[0..n]);
        self.name_len = n;
    }
};

pub const ComputeProc = struct {
    name_buf: [64]u8 = undefined,
    name_len: usize = 0,
    sm: u32 = 0,

    pub fn name(self: *const ComputeProc) []const u8 {
        return self.name_buf[0..self.name_len];
    }
};

pub const GpuData = struct {
    temp: f32 = 0,
    watt: f32 = 0,
    kind: GpuKind = .unknown,
    vram_used_mb: u32 = 0,
    vram_total_mb: u32 = 0,
    gfx_percent: u32 = 0,
    media: [MAX_PROCS]MediaProc = undefined,
    media_len: usize = 0,
    compute: [MAX_PROCS]ComputeProc = undefined,
    compute_len: usize = 0,

    pub fn mediaSlice(self: *const GpuData) []const MediaProc {
        return self.media[0..self.media_len];
    }
    pub fn computeSlice(self: *const GpuData) []const ComputeProc {
        return self.compute[0..self.compute_len];
    }
    fn addMedia(self: *GpuData, nm: []const u8, dec: u32, enc: u32, gfx: u32) void {
        if (self.media_len >= MAX_PROCS) return;
        var m = &self.media[self.media_len];
        m.* = .{};
        m.setName(nm);
        m.dec = dec;
        m.enc = enc;
        m.gfx = gfx;
        self.media_len += 1;
    }
    fn addCompute(self: *GpuData, nm: []const u8, sm: u32) void {
        if (self.compute_len >= MAX_PROCS) return;
        var c = &self.compute[self.compute_len];
        c.* = .{};
        const n = @min(nm.len, c.name_buf.len);
        @memcpy(c.name_buf[0..n], nm[0..n]);
        c.name_len = n;
        c.sm = sm;
        self.compute_len += 1;
    }
};

pub const Snapshot = struct {
    cpu_temp: f32 = 0,
    cpu_watt: f32 = 0,
    cpu_percent: u32 = 0,
    ram_used_mb: u32 = 0,
    ram_total_mb: u32 = 0,
    gpu: GpuData = .{},
};

// ── /proc yardımcıları ──────────────────────────────────────────────────────

fn readComm(pid: u32, buf: []u8) []const u8 {
    var pb: [64]u8 = undefined;
    const p = std.fmt.bufPrint(&pb, "/proc/{d}/comm", .{pid}) catch return "";
    const s = os.readTrim(p, buf) orelse return "";
    return s;
}

fn parseFdinfoNs(line: []const u8) u64 {
    var it = std.mem.tokenizeAny(u8, line, " \t");
    _ = it.next(); // alan adı
    const v = it.next() orelse return 0;
    return std.fmt.parseInt(u64, v, 10) catch 0;
}

// ── AMD fdinfo tracker ──────────────────────────────────────────────────────

const PrevAmd = struct { dec: u64, enc: u64, gfx: u64, t: u64 };

const CurAmd = struct {
    name: [64]u8 = undefined,
    name_len: usize = 0,
    dec: u64 = 0,
    enc: u64 = 0,
    gfx: u64 = 0,
    cap_dec: u32 = 0,
    cap_enc: u32 = 0,
};

pub const FdInfoTracker = struct {
    prev: std.AutoHashMap(u64, PrevAmd),
    pdev: PathBuf,
    vcn_instances: u32,
    alloc: std.mem.Allocator,

    pub fn init(alloc: std.mem.Allocator, pdev: PathBuf, vcn: u32) FdInfoTracker {
        return .{
            .prev = std.AutoHashMap(u64, PrevAmd).init(alloc),
            .pdev = pdev,
            .vcn_instances = vcn,
            .alloc = alloc,
        };
    }

    pub fn sample(self: *FdInfoTracker, out: *GpuData) void {
        const now = os.nowNs();
        var arena = std.heap.ArenaAllocator.init(self.alloc);
        defer arena.deinit();
        const a = arena.allocator();
        var current = std.AutoHashMap(u64, CurAmd).init(a);

        var procs = os.DirIter.open("/proc") orelse return;
        defer procs.close();

        var fdinfo_buf: [8192]u8 = undefined;
        const pdev = self.pdev.slice();

        while (procs.next()) |pe| {
            const pid = std.fmt.parseInt(u32, pe.name, 10) catch continue;

            var fdpath: [64]u8 = undefined;
            const fp = std.fmt.bufPrint(&fdpath, "/proc/{d}/fd", .{pid}) catch continue;
            var fds = os.DirIter.open(fp) orelse continue;
            defer fds.close();

            while (fds.next()) |fe| {
                var fipath: [96]u8 = undefined;
                const fip = std.fmt.bufPrint(&fipath, "/proc/{d}/fdinfo/{s}", .{ pid, fe.name }) catch continue;
                const content = os.readFile(fip, &fdinfo_buf) orelse continue;

                if (std.mem.indexOf(u8, content, "amdgpu") == null) continue;
                if (pdev.len != 0 and std.mem.indexOf(u8, content, pdev) == null) continue;

                var client_id: ?u64 = null;
                var fd_dec: u64 = 0;
                var fd_enc: u64 = 0;
                var fd_gfx: u64 = 0;
                var cap_dec: u32 = 0;
                var cap_enc: u32 = 0;

                var lines = std.mem.splitScalar(u8, content, '\n');
                while (lines.next()) |line| {
                    if (std.mem.startsWith(u8, line, "drm-client-id:")) {
                        client_id = parseFdinfoNs(line);
                    } else if (std.mem.startsWith(u8, line, "drm-engine-dec:")) {
                        fd_dec = @max(fd_dec, parseFdinfoNs(line));
                    } else if (std.mem.startsWith(u8, line, "drm-engine-enc:")) {
                        fd_enc = @max(fd_enc, parseFdinfoNs(line));
                    } else if (std.mem.startsWith(u8, line, "drm-engine-gfx:")) {
                        fd_gfx = @max(fd_gfx, parseFdinfoNs(line));
                    } else if (std.mem.startsWith(u8, line, "drm-engine-capacity-dec:")) {
                        cap_dec = @intCast(parseFdinfoNs(line));
                    } else if (std.mem.startsWith(u8, line, "drm-engine-capacity-enc:")) {
                        cap_enc = @intCast(parseFdinfoNs(line));
                    }
                }

                const cid = client_id orelse @as(u64, pid);
                const final_cap_dec = if (cap_dec > 0) cap_dec else self.vcn_instances;
                const final_cap_enc = if (cap_enc > 0) cap_enc else self.vcn_instances;

                const gop = current.getOrPut(cid) catch continue;
                if (gop.found_existing) {
                    gop.value_ptr.dec = @max(gop.value_ptr.dec, fd_dec);
                    gop.value_ptr.enc = @max(gop.value_ptr.enc, fd_enc);
                    gop.value_ptr.gfx = @max(gop.value_ptr.gfx, fd_gfx);
                } else {
                    var v = CurAmd{ .dec = fd_dec, .enc = fd_enc, .gfx = fd_gfx, .cap_dec = final_cap_dec, .cap_enc = final_cap_enc };
                    var nb: [64]u8 = undefined;
                    const nm = readComm(pid, &nb);
                    const n = @min(nm.len, v.name.len);
                    @memcpy(v.name[0..n], nm[0..n]);
                    v.name_len = n;
                    gop.value_ptr.* = v;
                }
            }
        }

        // Δ hesapla
        var it = current.iterator();
        while (it.next()) |e| {
            const cur = e.value_ptr.*;
            if (self.prev.get(e.key_ptr.*)) |p| {
                const elapsed = now -| p.t;
                if (elapsed == 0) continue;
                const dec_d = cur.dec -| p.dec;
                const enc_d = cur.enc -| p.enc;
                const gfx_d = cur.gfx -| p.gfx;
                const ef = @as(f64, @floatFromInt(elapsed));
                const dec_p: u32 = @as(u32, @intFromFloat(@as(f64, @floatFromInt(dec_d)) / ef * 100.0)) / cur.cap_dec;
                const enc_p: u32 = @as(u32, @intFromFloat(@as(f64, @floatFromInt(enc_d)) / ef * 100.0)) / cur.cap_enc;
                const gfx_p: u32 = @intFromFloat(@as(f64, @floatFromInt(gfx_d)) / ef * 100.0);
                if (dec_p > 0 or enc_p > 0 or gfx_p > 0) {
                    out.addMedia(cur.name[0..cur.name_len], dec_p, enc_p, gfx_p);
                }
            }
        }

        // prev'i yenile
        self.prev.clearRetainingCapacity();
        var it2 = current.iterator();
        while (it2.next()) |e| {
            self.prev.put(e.key_ptr.*, .{ .dec = e.value_ptr.dec, .enc = e.value_ptr.enc, .gfx = e.value_ptr.gfx, .t = now }) catch {};
        }

        sortMediaBySum(out);
    }
};

fn sortMediaBySum(out: *GpuData) void {
    const S = struct {
        fn lt(_: void, a: MediaProc, b: MediaProc) bool {
            return (a.dec + a.enc + a.gfx) > (b.dec + b.enc + b.gfx);
        }
    };
    std.mem.sort(MediaProc, out.media[0..out.media_len], {}, S.lt);
}

// ── Intel fdinfo tracker ────────────────────────────────────────────────────

const PrevIntel = struct { video: u64, render: u64, t: u64 };

const CurIntel = struct {
    name: [64]u8 = undefined,
    name_len: usize = 0,
    video: u64 = 0,
    render: u64 = 0,
};

pub const IntelFdInfoTracker = struct {
    prev: std.AutoHashMap(u64, PrevIntel),
    alloc: std.mem.Allocator,

    pub fn init(alloc: std.mem.Allocator) IntelFdInfoTracker {
        return .{ .prev = std.AutoHashMap(u64, PrevIntel).init(alloc), .alloc = alloc };
    }

    pub fn sample(self: *IntelFdInfoTracker, out: *GpuData) void {
        const now = os.nowNs();
        var arena = std.heap.ArenaAllocator.init(self.alloc);
        defer arena.deinit();
        const a = arena.allocator();
        var current = std.AutoHashMap(u64, CurIntel).init(a);

        var procs = os.DirIter.open("/proc") orelse return;
        defer procs.close();
        var fdinfo_buf: [8192]u8 = undefined;

        while (procs.next()) |pe| {
            const pid = std.fmt.parseInt(u32, pe.name, 10) catch continue;
            var fdpath: [64]u8 = undefined;
            const fp = std.fmt.bufPrint(&fdpath, "/proc/{d}/fd", .{pid}) catch continue;
            var fds = os.DirIter.open(fp) orelse continue;
            defer fds.close();

            while (fds.next()) |fe| {
                var fipath: [96]u8 = undefined;
                const fip = std.fmt.bufPrint(&fipath, "/proc/{d}/fdinfo/{s}", .{ pid, fe.name }) catch continue;
                const content = os.readFile(fip, &fdinfo_buf) orelse continue;

                if (std.mem.indexOf(u8, content, "i915") == null and std.mem.indexOf(u8, content, "xe") == null) continue;

                var client_id: ?u64 = null;
                var video_ns: u64 = 0;
                var render_ns: u64 = 0;
                var lines = std.mem.splitScalar(u8, content, '\n');
                while (lines.next()) |line| {
                    if (std.mem.startsWith(u8, line, "drm-client-id:")) {
                        client_id = parseFdinfoNs(line);
                    } else if (std.mem.startsWith(u8, line, "drm-engine-video:")) {
                        video_ns = @max(video_ns, parseFdinfoNs(line));
                    } else if (std.mem.startsWith(u8, line, "drm-engine-render:")) {
                        render_ns = @max(render_ns, parseFdinfoNs(line));
                    }
                }
                if (video_ns == 0 and render_ns == 0) continue;

                const cid = client_id orelse @as(u64, pid);
                const gop = current.getOrPut(cid) catch continue;
                if (gop.found_existing) {
                    gop.value_ptr.video = @max(gop.value_ptr.video, video_ns);
                    gop.value_ptr.render = @max(gop.value_ptr.render, render_ns);
                } else {
                    var v = CurIntel{ .video = video_ns, .render = render_ns };
                    var nb: [64]u8 = undefined;
                    const nm = readComm(pid, &nb);
                    const n = @min(nm.len, v.name.len);
                    @memcpy(v.name[0..n], nm[0..n]);
                    v.name_len = n;
                    gop.value_ptr.* = v;
                }
            }
        }

        var it = current.iterator();
        while (it.next()) |e| {
            const cur = e.value_ptr.*;
            if (self.prev.get(e.key_ptr.*)) |p| {
                const elapsed = now -| p.t;
                if (elapsed == 0) continue;
                const vd = cur.video -| p.video;
                const rd = cur.render -| p.render;
                const ef = @as(f64, @floatFromInt(elapsed));
                const vp: u32 = @intFromFloat(@as(f64, @floatFromInt(vd)) / ef * 100.0);
                const rp: u32 = @intFromFloat(@as(f64, @floatFromInt(rd)) / ef * 100.0);
                if (vp > 0 or rp > 0) {
                    out.addMedia(cur.name[0..cur.name_len], vp, 0, rp);
                }
            }
        }

        self.prev.clearRetainingCapacity();
        var it2 = current.iterator();
        while (it2.next()) |e| {
            self.prev.put(e.key_ptr.*, .{ .video = e.value_ptr.video, .render = e.value_ptr.render, .t = now }) catch {};
        }

        // Intel: video (.dec slotu) azalan sıraya göre
        const S = struct {
            fn lt(_: void, x: MediaProc, y: MediaProc) bool {
                return x.dec > y.dec;
            }
        };
        std.mem.sort(MediaProc, out.media[0..out.media_len], {}, S.lt);
    }
};

// ── GPU backend ─────────────────────────────────────────────────────────────

pub const GpuBackend = union(enum) {
    nvidia: nvml.Nvml,
    amd: struct { hwmon: PathBuf, device: PathBuf, pdev: PathBuf, vcn: u32 },
    intel: struct { hwmon: ?PathBuf, rapl_uncore: ?PathBuf },
    none,
};

fn vcnNumInst(comptime fmt: []const u8, card_idx: u32) ?u32 {
    var pb: [200]u8 = undefined;
    var nb: [32]u8 = undefined;
    const p = std.fmt.bufPrint(&pb, fmt, .{card_idx}) catch return null;
    const s = os.readTrim(p, &nb) orelse return null;
    const c = std.fmt.parseInt(u32, s, 10) catch return null;
    return if (c > 0) c else null;
}

fn getVcnInstances(card_idx: u32) u32 {
    if (vcnNumInst("/sys/class/drm/card{d}/device/ip_discovery/die/0/VCN/0/num_inst", card_idx)) |c| return c;
    if (vcnNumInst("/sys/class/drm/card{d}/device/ip_discovery/die/0/VCN/num_inst", card_idx)) |c| return c;
    // alt dizinleri say
    var pb: [200]u8 = undefined;
    const base = std.fmt.bufPrint(&pb, "/sys/class/drm/card{d}/device/ip_discovery/die/0/VCN", .{card_idx}) catch return 1;
    if (os.DirIter.open(base)) |*itc| {
        var it = itc.*;
        defer it.close();
        var count: u32 = 0;
        while (it.next()) |e| {
            if (e.kind == os.DT_DIR) count += 1;
        }
        if (count > 0) return count;
    }
    return 1;
}

fn findIntelGpuHwmon() ?PathBuf {
    var it = os.DirIter.open("/sys/class/hwmon") orelse return null;
    defer it.close();
    var pb: [256]u8 = undefined;
    var nb: [64]u8 = undefined;
    while (it.next()) |e| {
        const np = std.fmt.bufPrint(&pb, "/sys/class/hwmon/{s}/name", .{e.name}) catch continue;
        const name = os.readTrim(np, &nb) orelse continue;
        if (std.mem.eql(u8, name, "i915") or std.mem.eql(u8, name, "xe")) {
            var r = PathBuf{};
            const path = std.fmt.bufPrint(&pb, "/sys/class/hwmon/{s}", .{e.name}) catch continue;
            r.set(path);
            return r;
        }
    }
    return null;
}

fn findIntelRaplUncore() ?PathBuf {
    const bases = [_][]const u8{
        "/sys/class/powercap/intel-rapl/intel-rapl:0",
        "/sys/class/powercap/intel-rapl:0",
    };
    var pb: [256]u8 = undefined;
    var nb: [64]u8 = undefined;
    for (bases) |base| {
        var it = os.DirIter.open(base) orelse continue;
        defer it.close();
        while (it.next()) |e| {
            const np = std.fmt.bufPrint(&pb, "{s}/{s}/name", .{ base, e.name }) catch continue;
            const name = os.readTrim(np, &nb) orelse continue;
            var lb: [64]u8 = undefined;
            const lower = std.ascii.lowerString(lb[0..@min(name.len, lb.len)], name[0..@min(name.len, lb.len)]);
            if (std.mem.indexOf(u8, lower, "uncore") != null) {
                const ep = std.fmt.bufPrint(&pb, "{s}/{s}/energy_uj", .{ base, e.name }) catch continue;
                if (os.exists(ep)) {
                    var r = PathBuf{};
                    r.set(ep);
                    return r;
                }
            }
        }
    }
    return null;
}

pub fn detectGpu() GpuBackend {
    // 1. NVIDIA
    if (nvml.Nvml.init()) |n| {
        return .{ .nvidia = n };
    }

    var pb: [160]u8 = undefined;
    var vb: [32]u8 = undefined;
    var card_idx: u32 = 0;
    while (card_idx < 8) : (card_idx += 1) {
        const vp = std.fmt.bufPrint(&pb, "/sys/class/drm/card{d}/device/vendor", .{card_idx}) catch continue;
        const vendor = os.readTrim(vp, &vb) orelse continue;

        // 2. AMD
        if (std.mem.eql(u8, vendor, "0x1002")) {
            var pdev = PathBuf{};
            var ueb: [4096]u8 = undefined;
            const uep = std.fmt.bufPrint(&pb, "/sys/class/drm/card{d}/device/uevent", .{card_idx}) catch continue;
            if (os.readFile(uep, &ueb)) |content| {
                var lines = std.mem.splitScalar(u8, content, '\n');
                while (lines.next()) |line| {
                    if (std.mem.startsWith(u8, line, "PCI_SLOT_NAME=")) {
                        const val = std.mem.trim(u8, line["PCI_SLOT_NAME=".len..], " \t\r");
                        var lower: [64]u8 = undefined;
                        const lv = std.ascii.lowerString(lower[0..@min(val.len, lower.len)], val[0..@min(val.len, lower.len)]);
                        pdev.set(lv);
                        break;
                    }
                }
            }
            const vcn = getVcnInstances(card_idx);

            var hb: [160]u8 = undefined;
            const hwbase = std.fmt.bufPrint(&hb, "/sys/class/drm/card{d}/device/hwmon", .{card_idx}) catch continue;
            if (os.DirIter.open(hwbase)) |*itc| {
                var it = itc.*;
                defer it.close();
                while (it.next()) |e| {
                    var hp: [224]u8 = undefined;
                    const hwmon_path = std.fmt.bufPrint(&hp, "{s}/{s}", .{ hwbase, e.name }) catch continue;
                    var tcheck: [256]u8 = undefined;
                    const temp_f = std.fmt.bufPrint(&tcheck, "{s}/temp1_input", .{hwmon_path}) catch continue;
                    const has_temp = os.exists(temp_f);
                    var pcheck: [256]u8 = undefined;
                    const pow_f = std.fmt.bufPrint(&pcheck, "{s}/power1_average", .{hwmon_path}) catch continue;
                    const has_power = os.exists(pow_f);
                    if (has_temp or has_power) {
                        var hwmon = PathBuf{};
                        hwmon.set(hwmon_path);
                        var device = PathBuf{};
                        const dp = std.fmt.bufPrint(&pb, "/sys/class/drm/card{d}/device", .{card_idx}) catch continue;
                        device.set(dp);
                        return .{ .amd = .{ .hwmon = hwmon, .device = device, .pdev = pdev, .vcn = vcn } };
                    }
                }
            }
        }

        // 3. Intel
        if (std.mem.eql(u8, vendor, "0x8086")) {
            const dp = std.fmt.bufPrint(&pb, "/sys/class/drm/card{d}/device/driver", .{card_idx}) catch continue;
            var lb: [256]u8 = undefined;
            const drv = os.readLinkBasename(dp, &lb) orelse "";
            if (!std.mem.eql(u8, drv, "i915") and !std.mem.eql(u8, drv, "xe")) continue;
            return .{ .intel = .{ .hwmon = findIntelGpuHwmon(), .rapl_uncore = findIntelRaplUncore() } };
        }
    }

    return .none;
}

// ── CPU sıcaklık / RAPL tespiti ─────────────────────────────────────────────

fn hwmonScore(name: []const u8) i32 {
    if (std.mem.eql(u8, name, "k10temp")) return 100;
    if (std.mem.eql(u8, name, "coretemp")) return 95;
    if (std.mem.eql(u8, name, "zenpower")) return 90;
    if (std.mem.eql(u8, name, "asusec")) return 85;
    if (std.mem.eql(u8, name, "nct6775") or std.mem.eql(u8, name, "nct6687")) return 80;
    if (std.mem.eql(u8, name, "acpitz")) return 50;
    if (std.mem.eql(u8, name, "asus") or std.mem.eql(u8, name, "wmi")) return 40;
    return -1;
}

pub fn detectCpuTempPath() ?PathBuf {
    var it = os.DirIter.open("/sys/class/hwmon") orelse return null;
    defer it.close();

    var best: ?PathBuf = null;
    var best_score: i32 = 0;

    var pb: [256]u8 = undefined;
    var nb: [64]u8 = undefined;

    while (it.next()) |e| {
        const np = std.fmt.bufPrint(&pb, "/sys/class/hwmon/{s}/name", .{e.name}) catch continue;
        const raw = os.readTrim(np, &nb) orelse continue;
        var lowbuf: [64]u8 = undefined;
        const name = std.ascii.lowerString(lowbuf[0..@min(raw.len, lowbuf.len)], raw[0..@min(raw.len, lowbuf.len)]);

        const score = hwmonScore(name);
        if (score < 0) continue;
        if (score <= best_score) continue;

        // varsayılan temp1_input
        var target: [256]u8 = undefined;
        var target_path = std.fmt.bufPrint(&target, "/sys/class/hwmon/{s}/temp1_input", .{e.name}) catch continue;

        // etiketli sensör ara (tdie/tctl/package id/cpu)
        var i: u32 = 1;
        while (i <= 10) : (i += 1) {
            var lp: [256]u8 = undefined;
            const labelp = std.fmt.bufPrint(&lp, "/sys/class/hwmon/{s}/temp{d}_label", .{ e.name, i }) catch continue;
            var labbuf: [64]u8 = undefined;
            const lab_raw = os.readTrim(labelp, &labbuf) orelse continue;
            var lowlab: [64]u8 = undefined;
            const lab = std.ascii.lowerString(lowlab[0..@min(lab_raw.len, lowlab.len)], lab_raw[0..@min(lab_raw.len, lowlab.len)]);
            if (std.mem.indexOf(u8, lab, "tdie") != null or
                std.mem.indexOf(u8, lab, "tctl") != null or
                std.mem.indexOf(u8, lab, "package id") != null or
                std.mem.indexOf(u8, lab, "cpu") != null)
            {
                target_path = std.fmt.bufPrint(&target, "/sys/class/hwmon/{s}/temp{d}_input", .{ e.name, i }) catch continue;
                break;
            }
        }

        if (os.exists(target_path)) {
            var r = PathBuf{};
            r.set(target_path);
            best = r;
            best_score = score;
        }
    }
    return best;
}

pub fn findRaplPath() ?[]const u8 {
    const candidates = [_][]const u8{
        "/sys/class/powercap/intel-rapl:0/energy_uj",
        "/sys/class/powercap/intel-rapl/intel-rapl:0/energy_uj",
        "/sys/class/powercap/amd-energy-pkg/energy_uj",
        "/sys/class/powercap/amd_energy/energy1_input",
    };
    for (candidates) |c| {
        if (os.exists(c)) return c;
    }
    return null;
}

// ── Monitor ─────────────────────────────────────────────────────────────────

pub const Monitor = struct {
    backend: GpuBackend,
    cpu_temp_path: ?PathBuf,
    rapl_path: ?[]const u8,
    cpu_energy_prev: u64 = 0,
    cpu_time_prev: u64 = 0,
    gpu_energy_prev: u64 = 0,
    gpu_time_prev: u64 = 0,
    gpu_has_prev: bool = false,
    amd_tracker: ?FdInfoTracker = null,
    intel_tracker: ?IntelFdInfoTracker = null,
    cpu_stat_total_prev: u64 = 0,
    cpu_stat_idle_prev: u64 = 0,
    alloc: std.mem.Allocator,

    pub fn init(alloc: std.mem.Allocator) Monitor {
        const backend = detectGpu();
        var m = Monitor{
            .backend = backend,
            .cpu_temp_path = detectCpuTempPath(),
            .rapl_path = findRaplPath(),
            .alloc = alloc,
        };
        switch (m.backend) {
            .amd => |amd| m.amd_tracker = FdInfoTracker.init(alloc, amd.pdev, amd.vcn),
            .intel => m.intel_tracker = IntelFdInfoTracker.init(alloc),
            else => {},
        }
        if (m.rapl_path) |p| m.cpu_energy_prev = os.readU64(p) orelse 0;
        m.cpu_time_prev = os.nowNs();
        // CPU% için ilk taban örneği
        _ = m.sampleCpuStat();
        return m;
    }

    fn sampleCpuStat(self: *Monitor) u32 {
        var buf: [512]u8 = undefined;
        const content = os.readFile("/proc/stat", &buf) orelse return 0;
        var lines = std.mem.splitScalar(u8, content, '\n');
        const first = lines.next() orelse return 0;
        var it = std.mem.tokenizeAny(u8, first, " \t");
        const tag = it.next() orelse return 0;
        if (!std.mem.eql(u8, tag, "cpu")) return 0;

        var vals: [10]u64 = .{0} ** 10;
        var n: usize = 0;
        while (n < vals.len) : (n += 1) {
            const t = it.next() orelse break;
            vals[n] = std.fmt.parseInt(u64, t, 10) catch 0;
        }
        // idle_all = idle + iowait ; nonidle = user+nice+system+irq+softirq+steal
        const idle_all = vals[3] + vals[4];
        const nonidle = vals[0] + vals[1] + vals[2] + vals[5] + vals[6] + vals[7];
        const total = idle_all + nonidle;

        const dt = total -| self.cpu_stat_total_prev;
        const di = idle_all -| self.cpu_stat_idle_prev;
        self.cpu_stat_total_prev = total;
        self.cpu_stat_idle_prev = idle_all;
        if (dt == 0) return 0;
        const usage = (@as(f64, @floatFromInt(dt - di)) / @as(f64, @floatFromInt(dt))) * 100.0;
        if (usage < 0) return 0;
        return @intFromFloat(usage);
    }

    fn readCpuTemp(self: *Monitor) f32 {
        if (self.cpu_temp_path) |*p| {
            if (os.readU64(p.slice())) |v| return @as(f32, @floatFromInt(v)) / 1000.0;
        }
        return 0;
    }

    fn readCpuWatt(self: *Monitor) f32 {
        const path = self.rapl_path orelse return 0;
        const current = os.readU64(path) orelse return 0;
        const now = os.nowNs();
        const elapsed = @as(f32, @floatFromInt(now -| self.cpu_time_prev)) / 1_000_000_000.0;
        var watts: f32 = 0;
        if (elapsed > 0.1) {
            const diff = current -| self.cpu_energy_prev;
            watts = (@as(f32, @floatFromInt(diff)) / elapsed) / 1_000_000.0;
        }
        self.cpu_energy_prev = current;
        self.cpu_time_prev = now;
        return if (watts > 1.0 and watts < 400.0) watts else 0;
    }

    fn readMem(self: *Monitor) struct { used: u32, total: u32 } {
        _ = self;
        var buf: [4096]u8 = undefined;
        const content = os.readFile("/proc/meminfo", &buf) orelse return .{ .used = 0, .total = 0 };
        var total_kb: u64 = 0;
        var avail_kb: u64 = 0;
        var lines = std.mem.splitScalar(u8, content, '\n');
        while (lines.next()) |line| {
            if (std.mem.startsWith(u8, line, "MemTotal:")) {
                total_kb = parseMeminfoKb(line);
            } else if (std.mem.startsWith(u8, line, "MemAvailable:")) {
                avail_kb = parseMeminfoKb(line);
            }
        }
        const used_kb = total_kb -| avail_kb;
        return .{ .used = @intCast(used_kb / 1024), .total = @intCast(total_kb / 1024) };
    }

    /// GUI 200ms hızlı GFX örneklemesi (Rust gui2 döngüsündeki gfx_max için).
    pub fn quickGfx(self: *Monitor) u32 {
        switch (self.backend) {
            .nvidia => |*n| return n.utilGpu(),
            .amd => |amd| {
                var pb: [288]u8 = undefined;
                if (bufU64(&pb, "{s}/gpu_busy_percent", amd.device.slice())) |v| return @intCast(v);
                return 0;
            },
            else => return 0,
        }
    }

    pub fn readGpu(self: *Monitor) GpuData {
        var data = GpuData{};
        switch (self.backend) {
            .nvidia => |*n| {
                data.kind = .nvidia;
                data.watt = n.powerWatt();
                data.temp = n.tempC();
                const mem = n.memMb();
                data.vram_used_mb = mem.used;
                data.vram_total_mb = mem.total;
                data.gfx_percent = n.utilGpu();
                readNvidiaProcs(n, &data, self.alloc);
            },
            .amd => |amd| {
                data.kind = .amd;
                var pb: [288]u8 = undefined;
                if (bufU64(&pb, "{s}/temp1_input", amd.hwmon.slice())) |v|
                    data.temp = @as(f32, @floatFromInt(v)) / 1000.0;
                if (bufU64(&pb, "{s}/power1_average", amd.hwmon.slice())) |v|
                    data.watt = @as(f32, @floatFromInt(v)) / 1_000_000.0;
                if (bufU64(&pb, "{s}/mem_info_vram_used", amd.device.slice())) |v|
                    data.vram_used_mb = @intCast(v / 1_048_576);
                if (bufU64(&pb, "{s}/mem_info_vram_total", amd.device.slice())) |v|
                    data.vram_total_mb = @intCast(v / 1_048_576);
                if (bufU64(&pb, "{s}/gpu_busy_percent", amd.device.slice())) |v|
                    data.gfx_percent = @intCast(v);
                if (self.amd_tracker) |*t| t.sample(&data);
            },
            .intel => |intel| {
                data.kind = .intel;
                var pb: [288]u8 = undefined;
                if (intel.hwmon) |*h| {
                    if (bufU64(&pb, "{s}/temp1_input", h.slice())) |v|
                        data.temp = @as(f32, @floatFromInt(v)) / 1000.0;
                }
                if (intel.rapl_uncore) |*r| {
                    self.readIntelPower(r.slice(), &data, 0.1, 100.0);
                } else if (intel.hwmon) |*h| {
                    var eb: [288]u8 = undefined;
                    const ep = std.fmt.bufPrint(&eb, "{s}/energy1_input", .{h.slice()}) catch return data;
                    self.readIntelPower(ep, &data, 0.5, 300.0);
                }
                if (self.intel_tracker) |*t| t.sample(&data);
            },
            .none => {},
        }
        return data;
    }

    fn readIntelPower(self: *Monitor, path: []const u8, data: *GpuData, lo: f32, hi: f32) void {
        const current = os.readU64(path) orelse return;
        const now = os.nowNs();
        if (self.gpu_has_prev) {
            const elapsed = @as(f32, @floatFromInt(now -| self.gpu_time_prev)) / 1_000_000_000.0;
            if (elapsed > 0.1) {
                const delta = current -| self.gpu_energy_prev;
                const w = @as(f32, @floatFromInt(delta)) / elapsed / 1_000_000.0;
                if (w > lo and w < hi) data.watt = w;
            }
        } else {
            self.gpu_has_prev = true;
        }
        self.gpu_energy_prev = current;
        self.gpu_time_prev = now;
    }

    pub fn sample(self: *Monitor) Snapshot {
        var snap = Snapshot{};
        snap.cpu_temp = self.readCpuTemp();
        snap.cpu_watt = self.readCpuWatt();
        snap.gpu = self.readGpu();
        const mem = self.readMem();
        snap.ram_used_mb = mem.used;
        snap.ram_total_mb = mem.total;
        snap.cpu_percent = self.sampleCpuStat();
        return snap;
    }
};

fn parseMeminfoKb(line: []const u8) u64 {
    var it = std.mem.tokenizeAny(u8, line, " \t");
    _ = it.next(); // etiket
    const v = it.next() orelse return 0;
    return std.fmt.parseInt(u64, v, 10) catch 0;
}

fn bufU64(buf: []u8, comptime fmt: []const u8, arg: []const u8) ?u64 {
    const p = std.fmt.bufPrint(buf, fmt, .{arg}) catch return null;
    return os.readU64(p);
}

fn readNvidiaProcs(n: *const nvml.Nvml, data: *GpuData, alloc: std.mem.Allocator) void {
    var arena = std.heap.ArenaAllocator.init(alloc);
    defer arena.deinit();
    const a = arena.allocator();

    // CUDA pid kümesi
    var pinfo: [MAX_PROCS]nvml.ProcessInfo = undefined;
    const np = n.computePids(&pinfo);

    // codec_map: pid -> {dec,enc}, sm_by_pid: pid -> sm
    const Codec = struct { dec: u32, enc: u32 };
    var codec = std.AutoHashMap(u32, Codec).init(a);
    var sm_by = std.AutoHashMap(u32, u32).init(a);

    var samples: [256]nvml.ProcUtilSample = undefined;
    const ns = n.procUtil(&samples);
    for (samples[0..ns]) |s| {
        if (s.decUtil > 0 or s.encUtil > 0) {
            const gop = codec.getOrPut(s.pid) catch continue;
            if (gop.found_existing) {
                gop.value_ptr.dec = @max(gop.value_ptr.dec, s.decUtil);
                gop.value_ptr.enc = @max(gop.value_ptr.enc, s.encUtil);
            } else gop.value_ptr.* = .{ .dec = s.decUtil, .enc = s.encUtil };
        }
        if (s.smUtil > 0) {
            const gop = sm_by.getOrPut(s.pid) catch continue;
            if (gop.found_existing) {
                gop.value_ptr.* = @max(gop.value_ptr.*, s.smUtil);
            } else gop.value_ptr.* = s.smUtil;
        }
    }

    // codec -> media_procs
    var cit = codec.iterator();
    while (cit.next()) |e| {
        var nb: [64]u8 = undefined;
        var name = readComm(e.key_ptr.*, &nb);
        var fallback: [16]u8 = undefined;
        if (name.len == 0) name = std.fmt.bufPrint(&fallback, "pid:{d}", .{e.key_ptr.*}) catch "pid";
        data.addMedia(name, e.value_ptr.dec, e.value_ptr.enc, 0);
    }
    // media: (dec+enc) azalan
    const SM = struct {
        fn lt(_: void, x: MediaProc, y: MediaProc) bool {
            return (x.dec + x.enc) > (y.dec + y.enc);
        }
    };
    std.mem.sort(MediaProc, data.media[0..data.media_len], {}, SM.lt);

    // CUDA -> compute_procs (yalnızca aktif SM%)
    for (pinfo[0..np]) |pi| {
        const pid = pi.pid;
        const sm = sm_by.get(pid) orelse 0;
        if (sm == 0) continue;
        var nb: [64]u8 = undefined;
        var name = readComm(pid, &nb);
        var fallback: [16]u8 = undefined;
        if (name.len == 0) name = std.fmt.bufPrint(&fallback, "pid:{d}", .{pid}) catch "pid";
        data.addCompute(name, sm);
    }
    const SC = struct {
        fn lt(_: void, x: ComputeProc, y: ComputeProc) bool {
            return x.sm > y.sm;
        }
    };
    std.mem.sort(ComputeProc, data.compute[0..data.compute_len], {}, SC.lt);
}
