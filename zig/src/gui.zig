//! GUI modları — GTK4 layer-shell overlay'ler.
//!   run1() → Rust `build_ui`  (etiketli/ikonlu panel, --gui)
//!   run2() → Rust `build_ui2` (bar görünümlü panel, --gui2)
//!
//! Zig 0.16 `@cImport`, glib/gtk header'larındaki `_Pragma` makrolarında
//! çöküyor (translate-c sınırı). Bu yüzden gereken GTK/cairo sembolleri elle
//! `extern` bildirilir. Tüm GObject işaretçileri ABI uyumlu olduğundan hepsi
//! `?*anyopaque` (cast gürültüsü yok).

const std = @import("std");
const os = @import("os.zig");
const sensors = @import("sensors.zig");

// ── C tipleri ───────────────────────────────────────────────────────────────
const Obj = ?*anyopaque;
const GCallback = ?*const fn () callconv(.c) void;
const DrawFunc = ?*const fn (area: Obj, cr: Obj, w: c_int, h: c_int, data: ?*anyopaque) callconv(.c) void;
const SourceFunc = ?*const fn (data: ?*anyopaque) callconv(.c) c_int;

// ── Sabitler (header'lardan birebir) ─────────────────────────────────────────
const G_APPLICATION_DEFAULT_FLAGS: c_uint = 0;
const ORIENTATION_HORIZONTAL: c_uint = 0;
const ORIENTATION_VERTICAL: c_uint = 1;
const ALIGN_START: c_uint = 1;
const ALIGN_END: c_uint = 2;
const ALIGN_CENTER: c_uint = 3;
const JUSTIFY_RIGHT: c_uint = 1;
const ELLIPSIZE_END: c_uint = 3;
const SCROLL_VERTICAL: c_uint = 1;
const STYLE_PRIORITY_APP: c_uint = 600;
const LAYER_OVERLAY: c_uint = 3;
const EDGE_RIGHT: c_uint = 1;
const EDGE_TOP: c_uint = 2;
const KEYBOARD_NONE: c_uint = 0;

// İkonlar (nerd font — Rust main.rs ile birebir)
const ICON_CPU = "\u{f4bc}";
const ICON_GPU = "\u{f08ae}";
const ICON_RAM = "\u{f035b}";
const ICON_VRAM = "\u{f048b}";

// ── extern GTK / GLib / cairo ─────────────────────────────────────────────────
extern fn gtk_application_new(app_id: [*:0]const u8, flags: c_uint) Obj;
extern fn g_application_run(app: Obj, argc: c_int, argv: ?*anyopaque) c_int;
extern fn g_signal_connect_data(instance: Obj, signal: [*:0]const u8, handler: GCallback, data: ?*anyopaque, destroy: ?*anyopaque, flags: c_uint) c_ulong;
extern fn g_object_unref(obj: Obj) void;
extern fn g_timeout_add(interval_ms: c_uint, function: SourceFunc, data: ?*anyopaque) c_uint;

extern fn gtk_application_window_new(app: Obj) Obj;
extern fn gtk_window_set_default_size(window: Obj, w: c_int, h: c_int) void;
extern fn gtk_window_set_decorated(window: Obj, setting: c_int) void;
extern fn gtk_window_set_child(window: Obj, child: Obj) void;
extern fn gtk_window_present(window: Obj) void;
extern fn gtk_window_close(window: Obj) void;
extern fn gtk_widget_set_opacity(widget: Obj, opacity: f64) void;
extern fn gtk_widget_set_size_request(widget: Obj, w: c_int, h: c_int) void;
extern fn gtk_widget_add_css_class(widget: Obj, name: [*:0]const u8) void;
extern fn gtk_widget_set_css_classes(widget: Obj, classes: [*]const ?[*:0]const u8) void;
extern fn gtk_widget_set_hexpand(widget: Obj, expand: c_int) void;
extern fn gtk_widget_set_halign(widget: Obj, a: c_uint) void;
extern fn gtk_widget_set_valign(widget: Obj, a: c_uint) void;
extern fn gtk_widget_set_visible(widget: Obj, visible: c_int) void;
extern fn gtk_widget_queue_draw(widget: Obj) void;
extern fn gtk_widget_add_controller(widget: Obj, controller: Obj) void;

extern fn gtk_layer_init_for_window(window: Obj) void;
extern fn gtk_layer_set_layer(window: Obj, layer: c_uint) void;
extern fn gtk_layer_set_anchor(window: Obj, edge: c_uint, anchor: c_int) void;
extern fn gtk_layer_set_margin(window: Obj, edge: c_uint, margin: c_int) void;
extern fn gtk_layer_set_keyboard_mode(window: Obj, mode: c_uint) void;

extern fn gtk_css_provider_new() Obj;
extern fn gtk_css_provider_load_from_string(provider: Obj, data: [*:0]const u8) void;
extern fn gdk_display_get_default() Obj;
extern fn gtk_style_context_add_provider_for_display(display: Obj, provider: Obj, priority: c_uint) void;

extern fn gtk_box_new(orientation: c_uint, spacing: c_int) Obj;
extern fn gtk_box_append(box: Obj, child: Obj) void;
extern fn gtk_separator_new(orientation: c_uint) Obj;

extern fn gtk_label_new(text: ?[*:0]const u8) Obj;
extern fn gtk_label_set_text(label: Obj, text: [*:0]const u8) void;
extern fn gtk_label_set_markup(label: Obj, markup: [*:0]const u8) void;
extern fn gtk_label_set_use_markup(label: Obj, setting: c_int) void;
extern fn gtk_label_set_xalign(label: Obj, xalign: f32) void;
extern fn gtk_label_set_width_chars(label: Obj, n: c_int) void;
extern fn gtk_label_set_max_width_chars(label: Obj, n: c_int) void;
extern fn gtk_label_set_ellipsize(label: Obj, mode: c_uint) void;
extern fn gtk_label_set_justify(label: Obj, justify: c_uint) void;

extern fn gtk_grid_new() Obj;
extern fn gtk_grid_set_row_spacing(grid: Obj, spacing: c_uint) void;
extern fn gtk_grid_attach(grid: Obj, child: Obj, col: c_int, row: c_int, w: c_int, h: c_int) void;

extern fn gtk_drawing_area_new() Obj;
extern fn gtk_drawing_area_set_content_height(area: Obj, h: c_int) void;
extern fn gtk_drawing_area_set_draw_func(area: Obj, draw_func: DrawFunc, user_data: ?*anyopaque, destroy: ?*anyopaque) void;

extern fn gtk_gesture_click_new() Obj;
extern fn gtk_gesture_single_set_button(gesture: Obj, button: c_uint) void;
extern fn gtk_event_controller_scroll_new(flags: c_uint) Obj;

extern fn cairo_set_source_rgb(cr: Obj, r: f64, g: f64, b: f64) void;
extern fn cairo_set_source_rgba(cr: Obj, r: f64, g: f64, b: f64, a: f64) void;
extern fn cairo_rectangle(cr: Obj, x: f64, y: f64, w: f64, h: f64) void;
extern fn cairo_fill(cr: Obj) void;

// ── CSS ───────────────────────────────────────────────────────────────────────
const FONT = "font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;";

// build_ui (etiketli panel)
const CSS1 =
    \\window { background-color: transparent; }
    \\.panel { background-color: rgba(10, 10, 10, 0.80); border-radius: 18px; border: 1px solid rgba(255, 255, 255, 0.15); padding: 18px 24px; }
    \\.total-watt { color: #00ffcc; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 26px; font-weight: bold; }
    \\.lbl-cpu { color: #ff9f43; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 16px; font-weight: bold; }
    \\.lbl-gpu { color: #2ecc71; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 16px; font-weight: bold; }
    \\.lbl-util { color: #a29bfe; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 16px; font-weight: bold; }
    \\.val-watt { color: #ffffff; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 16px; }
    \\.val-temp { color: #ff4757; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 16px; }
    \\.val-temp-cool { color: #4cd964; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 16px; }
    \\.val-temp-warm { color: #ff9f43; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 16px; }
    \\.val-temp-hot { color: #ff4757; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 16px; }
    \\.lbl-ram { color: #00cec9; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 16px; font-weight: bold; }
    \\.val-vram { color: #74b9ff; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 14px; }
    \\.val-proc { color: #b2bec3; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 13px; }
    \\.proc-hdr { color: #a29bfe; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 13px; font-weight: bold; }
    \\.proc-val { color: #b2bec3; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 12px; }
    \\.proc-num { color: #dfe6e9; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 12px; }
    \\.val-pct { color: #dfe6e9; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 16px; }
    \\.divider { background-color: rgba(255, 255, 255, 0.10); min-height: 1px; margin: 4px 0px; }
;

// build_ui2 (bar panel)
const CSS2 =
    \\window { background-color: transparent; }
    \\.panel2 { background-color: rgba(10, 10, 10, 0.82); border-radius: 18px; border: 1px solid rgba(255, 255, 255, 0.15); padding: 14px 18px; }
    \\.brand-lbl { color: #a0a8b0; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 13px; }
    \\.total-watt { color: #00ffcc; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 22px; font-weight: bold; }
    \\.lbl-cpu { color: #ff9f43; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 14px; font-weight: bold; }
    \\.lbl-gpu { color: #2ecc71; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 14px; font-weight: bold; }
    \\.lbl-ram { color: #00cec9; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 14px; font-weight: bold; }
    \\.val-pct { color: #dfe6e9; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 13px; }
    \\.stat-lbl { font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 13px; }
    \\.divider { background-color: rgba(255, 255, 255, 0.10); min-height: 1px; margin: 2px 0px; }
    \\.proc-hdr { color: #a29bfe; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 12px; font-weight: bold; }
    \\.proc-val { color: #b2bec3; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 12px; }
    \\.proc-num { color: #dfe6e9; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace; font-size: 12px; }
;

// ── Paylaşılan yardımcılar ───────────────────────────────────────────────────
fn label(text: ?[*:0]const u8, css: [*:0]const u8) Obj {
    const l = gtk_label_new(text);
    gtk_widget_add_css_class(l, css);
    return l;
}

fn divider(margin_class: [*:0]const u8) Obj {
    const sep = gtk_separator_new(ORIENTATION_HORIZONTAL);
    gtk_widget_add_css_class(sep, margin_class);
    return sep;
}

fn setClasses(widget: Obj, cls: [*:0]const u8) void {
    var arr = [_]?[*:0]const u8{ cls, null };
    gtk_widget_set_css_classes(widget, &arr);
}

fn tempHex(t: f32) [*:0]const u8 {
    if (t >= 80.0) return "#ff4757";
    if (t >= 60.0) return "#ff9f43";
    return "#4cd964";
}

fn tempClass(t: f32) [*:0]const u8 {
    if (t >= 80.0) return "val-temp-hot";
    if (t >= 60.0) return "val-temp-warm";
    return "val-temp-cool";
}

fn usageClass(pct: u32) [*:0]const u8 {
    if (pct >= 90) return "val-temp-hot";
    if (pct >= 75) return "val-temp-warm";
    return "val-pct";
}

fn floorTemp(t: f32) u32 {
    const v = @floor(t);
    return if (v <= 0) 0 else @intFromFloat(v);
}

fn gpuHasPct(kind: sensors.GpuKind) bool {
    return kind == .nvidia or kind == .amd;
}

// Sabit tampon + null sonlandırmalı yapıcı
const Buf = struct {
    data: [2048]u8 = undefined,
    len: usize = 0,
    fn print(self: *Buf, comptime fmt: []const u8, args: anytype) void {
        const s = std.fmt.bufPrint(self.data[self.len..], fmt, args) catch return;
        self.len += s.len;
    }
    fn raw(self: *Buf, s: []const u8) void {
        const n = @min(s.len, self.data.len - 1 - self.len);
        @memcpy(self.data[self.len..][0..n], s[0..n]);
        self.len += n;
    }
    fn z(self: *Buf) [*:0]const u8 {
        self.data[self.len] = 0;
        return @ptrCast(&self.data);
    }
};

fn fmtGbBuf(buf: *Buf, used_mb: u32, total_mb: u32) [*:0]const u8 {
    const used = @as(f32, @floatFromInt(used_mb)) / 1024.0;
    const total = @as(f32, @floatFromInt(total_mb)) / 1024.0;
    if (total >= 100.0) buf.print("{d:.0}/{d:.0} GB", .{ used, total }) else buf.print("{d:.1}/{d:.0} GB", .{ used, total });
    return buf.z();
}

// ── Proc bölümü (her iki GUI ortak) ──────────────────────────────────────────
const ProcWidgets = struct {
    container: Obj = null,
    proc: Obj = null,
    gfx: Obj = null,
    dec: Obj = null,
    enc: Obj = null,
    sm: Obj = null,
};

fn makeProcSection() ProcWidgets {
    const grid = gtk_grid_new();
    gtk_grid_set_row_spacing(grid, 4);

    const name_hdr = label("Process", "proc-hdr");
    gtk_widget_set_hexpand(name_hdr, 1);
    gtk_label_set_xalign(name_hdr, 0.0);
    const heads = [_][*:0]const u8{ "GFX", "DEC", "ENC", "SM%" };
    var hdr_widgets: [4]Obj = undefined;
    for (heads, 0..) |h, i| {
        const w = label(h, "proc-hdr");
        gtk_label_set_xalign(w, 1.0);
        hdr_widgets[i] = w;
    }

    var pw = ProcWidgets{ .container = grid };
    pw.proc = label("", "proc-val");
    gtk_widget_set_hexpand(pw.proc, 1);
    gtk_label_set_xalign(pw.proc, 0.0);
    gtk_widget_set_valign(pw.proc, ALIGN_START);
    gtk_label_set_max_width_chars(pw.proc, 12);
    gtk_label_set_ellipsize(pw.proc, ELLIPSIZE_END);

    const nums = [_]*Obj{ &pw.gfx, &pw.dec, &pw.enc, &pw.sm };
    for (nums) |np| {
        const w = label("", "proc-num");
        gtk_label_set_xalign(w, 1.0);
        gtk_label_set_justify(w, JUSTIFY_RIGHT);
        gtk_widget_set_valign(w, ALIGN_START);
        np.* = w;
    }

    gtk_grid_attach(grid, name_hdr, 0, 0, 1, 1);
    gtk_grid_attach(grid, hdr_widgets[0], 1, 0, 1, 1);
    gtk_grid_attach(grid, hdr_widgets[1], 2, 0, 1, 1);
    gtk_grid_attach(grid, hdr_widgets[2], 3, 0, 1, 1);
    gtk_grid_attach(grid, hdr_widgets[3], 4, 0, 1, 1);
    gtk_grid_attach(grid, pw.proc, 0, 1, 1, 1);
    gtk_grid_attach(grid, pw.gfx, 1, 1, 1, 1);
    gtk_grid_attach(grid, pw.dec, 2, 1, 1, 1);
    gtk_grid_attach(grid, pw.enc, 3, 1, 1, 1);
    gtk_grid_attach(grid, pw.sm, 4, 1, 1, 1);
    return pw;
}

const Combined = struct {
    name: []const u8,
    gfx: ?u32 = null,
    dec: ?u32 = null,
    enc: ?u32 = null,
    sm: ?u32 = null,
};

fn appendTrunc(buf: *Buf, name: []const u8) void {
    const cp = std.unicode.utf8CountCodepoints(name) catch name.len;
    if (cp <= 11) {
        buf.raw(name);
        return;
    }
    var view = std.unicode.Utf8View.initUnchecked(name);
    var it = view.iterator();
    var taken: usize = 0;
    while (taken < 10) : (taken += 1) {
        const s = it.nextCodepointSlice() orelse break;
        buf.raw(s);
    }
    buf.raw("…");
}

fn appendVal(buf: *Buf, v: ?u32) void {
    if (v) |x| {
        if (x > 0) {
            buf.print("{d:>3}%", .{x});
            return;
        }
    }
    buf.raw("   —");
}

// Proc tablosunu doldur + container/separator görünürlüğünü ayarla.
fn updateProcs(pw: *const ProcWidgets, sep: Obj, gpu: *const sensors.GpuData, valid_gpu: bool) void {
    const has_media = valid_gpu and gpu.media_len > 0;
    const has_compute = valid_gpu and gpu.compute_len > 0;
    const has_procs = has_media or has_compute;

    if (has_procs) {
        gtk_widget_set_visible(pw.container, 1);

        var combined: [sensors.MAX_PROCS * 2]Combined = undefined;
        var n: usize = 0;
        for (gpu.mediaSlice()) |*m| {
            if (n >= combined.len) break;
            combined[n] = .{
                .name = m.name(),
                .gfx = if (m.gfx > 0) m.gfx else null,
                .dec = if (m.dec > 0) m.dec else null,
                .enc = if (m.enc > 0) m.enc else null,
            };
            n += 1;
        }
        for (gpu.computeSlice()) |*cp| {
            const sv: ?u32 = if (cp.sm > 0) cp.sm else null;
            var found = false;
            for (combined[0..n]) |*e| {
                if (std.mem.eql(u8, e.name, cp.name())) {
                    e.sm = sv;
                    found = true;
                    break;
                }
            }
            if (!found and n < combined.len) {
                combined[n] = .{ .name = cp.name(), .sm = sv };
                n += 1;
            }
        }

        var names: Buf = .{};
        var gfxs: Buf = .{};
        var decs: Buf = .{};
        var encs: Buf = .{};
        var sms: Buf = .{};
        for (combined[0..n], 0..) |e, i| {
            if (i > 0) {
                names.raw("\n");
                gfxs.raw("\n");
                decs.raw("\n");
                encs.raw("\n");
                sms.raw("\n");
            }
            appendTrunc(&names, e.name);
            appendVal(&gfxs, e.gfx);
            appendVal(&decs, e.dec);
            appendVal(&encs, e.enc);
            appendVal(&sms, e.sm);
        }
        gtk_label_set_text(pw.proc, names.z());
        gtk_label_set_text(pw.gfx, gfxs.z());
        gtk_label_set_text(pw.dec, decs.z());
        gtk_label_set_text(pw.enc, encs.z());
        gtk_label_set_text(pw.sm, sms.z());
    } else {
        gtk_widget_set_visible(pw.container, 0);
    }

    const sep_vis: c_int = if (valid_gpu and (has_procs or gpu.vram_total_mb > 0)) 1 else 0;
    gtk_widget_set_visible(sep, sep_vis);
}

// Sağ tık → pencereyi kapat (data = pencere).
fn onReleased(_: Obj, _: c_int, _: f64, _: f64, data: ?*anyopaque) callconv(.c) void {
    gtk_window_close(data);
}

// Ortak layer-shell pencere kurulumu.
fn makeWindow(app: *anyopaque) Obj {
    const window = gtk_application_window_new(app);
    gtk_window_set_default_size(window, 340, 1);
    gtk_window_set_decorated(window, 0);
    gtk_layer_init_for_window(window);
    gtk_layer_set_layer(window, LAYER_OVERLAY);
    gtk_layer_set_anchor(window, EDGE_TOP, 1);
    gtk_layer_set_anchor(window, EDGE_RIGHT, 1);
    gtk_layer_set_margin(window, EDGE_TOP, 60);
    gtk_layer_set_margin(window, EDGE_RIGHT, 20);
    gtk_layer_set_keyboard_mode(window, KEYBOARD_NONE);
    return window;
}

fn loadCss(css: [*:0]const u8) void {
    const provider = gtk_css_provider_new();
    gtk_css_provider_load_from_string(provider, css);
    gtk_style_context_add_provider_for_display(gdk_display_get_default(), provider, STYLE_PRIORITY_APP);
}

// ════════════════════════════════════════════════════════════════════════════
//  gui2 — bar görünümlü panel (build_ui2)
// ════════════════════════════════════════════════════════════════════════════
const Ctx2 = struct {
    mon: sensors.Monitor = undefined,
    loop_count: u32 = 0,
    gfx_max: u32 = 0,
    opacity: f64 = 1.0,

    window: Obj = null,
    total_label: Obj = null,
    cpu_bar: Obj = null,
    gpu_bar: Obj = null,
    ram_bar: Obj = null,
    vram_bar: Obj = null,
    cpu_val: Obj = null,
    gpu_val: Obj = null,
    ram_val: Obj = null,
    vram_val: Obj = null,
    vram_row: Obj = null,
    cpu_stat: Obj = null,
    gpu_stat: Obj = null,
    sep2: Obj = null,
    proc: ProcWidgets = .{},

    cpu_pct: u32 = 0,
    gpu_pct: u32 = 0,
    ram_pct: u32 = 0,
    vram_pct: u32 = 0,
};

var ctx2: Ctx2 = .{};

fn drawBar(_: Obj, cr: Obj, width: c_int, height: c_int, data: ?*anyopaque) callconv(.c) void {
    const pct_ptr: *u32 = @ptrCast(@alignCast(data));
    const pct = @min(pct_ptr.*, 100);
    const filled_w: i32 = @intFromFloat(@as(f64, @floatFromInt(pct)) / 100.0 * @as(f64, @floatFromInt(width)));
    var r: f64 = 0.18;
    var g: f64 = 0.80;
    var b: f64 = 0.44;
    if (pct >= 75) {
        r = 0.91;
        g = 0.30;
        b = 0.24;
    } else if (pct >= 50) {
        r = 0.90;
        g = 0.49;
        b = 0.13;
    } else if (pct >= 25) {
        r = 0.95;
        g = 0.77;
        b = 0.06;
    }
    const y: f64 = 1.0;
    const h: f64 = @floatFromInt(height - 2);
    if (filled_w > 0) {
        cairo_set_source_rgb(cr, r, g, b);
        cairo_rectangle(cr, 0.0, y, @floatFromInt(filled_w), h);
        cairo_fill(cr);
    }
    if (filled_w < width) {
        cairo_set_source_rgba(cr, r, g, b, 0.15);
        cairo_rectangle(cr, @floatFromInt(filled_w), y, @floatFromInt(width - filled_w), h);
        cairo_fill(cr);
    }
}

fn makeBarRow(lbl_text: [*:0]const u8, lbl_css: [*:0]const u8, bar_out: *Obj, val_out: *Obj, pct_ptr: *u32) Obj {
    const row = gtk_box_new(ORIENTATION_HORIZONTAL, 6);
    gtk_widget_set_valign(row, ALIGN_CENTER);

    const l = label(lbl_text, lbl_css);
    gtk_label_set_width_chars(l, 4);
    gtk_label_set_xalign(l, 0.0);

    const bar = gtk_drawing_area_new();
    gtk_widget_set_hexpand(bar, 1);
    gtk_drawing_area_set_content_height(bar, 12);
    gtk_widget_set_valign(bar, ALIGN_CENTER);
    gtk_drawing_area_set_draw_func(bar, drawBar, pct_ptr, null);

    const val = label("", "val-pct");
    gtk_label_set_width_chars(val, 11);
    gtk_label_set_xalign(val, 1.0);

    gtk_box_append(row, l);
    gtk_box_append(row, bar);
    gtk_box_append(row, val);

    bar_out.* = bar;
    val_out.* = val;
    return row;
}

fn onScroll(_: Obj, _: f64, dy: f64, data: ?*anyopaque) callconv(.c) c_int {
    _ = data;
    var n = ctx2.opacity - dy * 0.05;
    if (n < 0.3) n = 0.3;
    if (n > 1.0) n = 1.0;
    ctx2.opacity = n;
    gtk_widget_set_opacity(ctx2.window, n);
    return 1; // Propagation::Stop
}

fn activate2(app: *anyopaque, _: ?*anyopaque) callconv(.c) void {
    const window = makeWindow(app);
    ctx2.window = window;
    loadCss(CSS2);

    const panel = gtk_box_new(ORIENTATION_VERTICAL, 6);
    gtk_widget_add_css_class(panel, "panel2");
    gtk_widget_set_size_request(panel, 340, -1);

    // Başlık satırı
    const title_row = gtk_box_new(ORIENTATION_HORIZONTAL, 0);
    const brand = label("PowerPanel", "brand-lbl");
    gtk_widget_set_hexpand(brand, 1);
    gtk_label_set_xalign(brand, 0.0);
    gtk_widget_set_valign(brand, ALIGN_END);
    ctx2.total_label = label("⚡  0.0 W", "total-watt");
    gtk_label_set_xalign(ctx2.total_label, 1.0);
    gtk_box_append(title_row, brand);
    gtk_box_append(title_row, ctx2.total_label);
    gtk_box_append(panel, title_row);

    gtk_box_append(panel, divider("divider"));

    gtk_box_append(panel, makeBarRow("CPU", "lbl-cpu", &ctx2.cpu_bar, &ctx2.cpu_val, &ctx2.cpu_pct));
    gtk_box_append(panel, makeBarRow("GPU", "lbl-gpu", &ctx2.gpu_bar, &ctx2.gpu_val, &ctx2.gpu_pct));
    gtk_box_append(panel, makeBarRow("RAM", "lbl-ram", &ctx2.ram_bar, &ctx2.ram_val, &ctx2.ram_pct));
    ctx2.vram_row = makeBarRow("VRAM", "lbl-gpu", &ctx2.vram_bar, &ctx2.vram_val, &ctx2.vram_pct);
    gtk_widget_set_visible(ctx2.vram_row, 0);
    gtk_box_append(panel, ctx2.vram_row);

    gtk_box_append(panel, divider("divider"));

    // Stats strip
    const stats_row = gtk_box_new(ORIENTATION_HORIZONTAL, 0);
    ctx2.cpu_stat = label(null, "stat-lbl");
    gtk_label_set_use_markup(ctx2.cpu_stat, 1);
    gtk_label_set_markup(ctx2.cpu_stat, "<span foreground='#ff9f43'><b>CPU</b></span>  --°C    0.0W");
    gtk_widget_set_hexpand(ctx2.cpu_stat, 1);
    gtk_label_set_xalign(ctx2.cpu_stat, 0.0);
    ctx2.gpu_stat = label(null, "stat-lbl");
    gtk_label_set_use_markup(ctx2.gpu_stat, 1);
    gtk_label_set_markup(ctx2.gpu_stat, "<span foreground='#2ecc71'><b>GPU</b></span>  --°C    0.0W");
    gtk_label_set_xalign(ctx2.gpu_stat, 1.0);
    gtk_box_append(stats_row, ctx2.cpu_stat);
    gtk_box_append(stats_row, ctx2.gpu_stat);
    gtk_box_append(panel, stats_row);

    ctx2.sep2 = divider("divider");
    gtk_widget_set_visible(ctx2.sep2, 0);
    gtk_box_append(panel, ctx2.sep2);
    ctx2.proc = makeProcSection();
    gtk_widget_set_visible(ctx2.proc.container, 0);
    gtk_box_append(panel, ctx2.proc.container);

    gtk_window_set_child(window, panel);

    const gesture = gtk_gesture_click_new();
    gtk_gesture_single_set_button(gesture, 3);
    _ = g_signal_connect_data(gesture, "released", @as(GCallback, @ptrCast(&onReleased)), window, null, 0);
    gtk_widget_add_controller(window, gesture);

    const scroll = gtk_event_controller_scroll_new(SCROLL_VERTICAL);
    _ = g_signal_connect_data(scroll, "scroll", @as(GCallback, @ptrCast(&onScroll)), null, null, 0);
    gtk_widget_add_controller(window, scroll);

    gtk_window_present(window);
    _ = g_timeout_add(200, tick2, null);
}

fn tick2(_: ?*anyopaque) callconv(.c) c_int {
    ctx2.loop_count += 1;
    ctx2.gfx_max = @max(ctx2.gfx_max, ctx2.mon.quickGfx());
    if (ctx2.loop_count % 5 == 1) {
        var snap = ctx2.mon.sample();
        snap.gpu.gfx_percent = @max(snap.gpu.gfx_percent, ctx2.gfx_max);
        ctx2.gfx_max = 0;
        updateUi2(&snap);
    }
    return 1;
}

fn updateUi2(snap: *const sensors.Snapshot) void {
    const gpu = &snap.gpu;
    var b: Buf = .{};

    b.print("⚡ {d:>6.1} W", .{snap.cpu_watt + gpu.watt});
    gtk_label_set_text(ctx2.total_label, b.z());

    ctx2.cpu_pct = @min(snap.cpu_percent, 100);
    gtk_widget_queue_draw(ctx2.cpu_bar);
    b.len = 0;
    b.print("{d:>3}%", .{snap.cpu_percent});
    gtk_label_set_text(ctx2.cpu_val, b.z());

    const has_pct = gpuHasPct(gpu.kind);
    ctx2.gpu_pct = if (has_pct) @min(gpu.gfx_percent, 100) else 0;
    gtk_widget_queue_draw(ctx2.gpu_bar);
    b.len = 0;
    if (has_pct) b.print("{d:>3}%", .{gpu.gfx_percent}) else b.raw("  —");
    gtk_label_set_text(ctx2.gpu_val, b.z());

    if (snap.ram_total_mb > 0) {
        ctx2.ram_pct = @min(snap.ram_used_mb * 100 / snap.ram_total_mb, 100);
        gtk_widget_queue_draw(ctx2.ram_bar);
        var rb: Buf = .{};
        gtk_label_set_text(ctx2.ram_val, fmtGbBuf(&rb, snap.ram_used_mb, snap.ram_total_mb));
    }

    const valid_gpu = gpu.kind != .unknown;
    if (valid_gpu and gpu.vram_total_mb > 0) {
        ctx2.vram_pct = @min(gpu.vram_used_mb * 100 / gpu.vram_total_mb, 100);
        gtk_widget_queue_draw(ctx2.vram_bar);
        var vb: Buf = .{};
        gtk_label_set_text(ctx2.vram_val, fmtGbBuf(&vb, gpu.vram_used_mb, gpu.vram_total_mb));
        gtk_widget_set_visible(ctx2.vram_row, 1);
    } else {
        gtk_widget_set_visible(ctx2.vram_row, 0);
    }

    var cb: Buf = .{};
    cb.print("<span foreground='#ff9f43'><b>CPU</b></span>  <span foreground='{s}'>{d:>3}°C</span>  <span foreground='#ffffff'>{d:>5.1}W</span>", .{ tempHex(snap.cpu_temp), floorTemp(snap.cpu_temp), snap.cpu_watt });
    gtk_label_set_markup(ctx2.cpu_stat, cb.z());
    var gb: Buf = .{};
    gb.print("<span foreground='#2ecc71'><b>GPU</b></span>  <span foreground='{s}'>{d:>3}°C</span>  <span foreground='#ffffff'>{d:>5.1}W</span>", .{ tempHex(gpu.temp), floorTemp(gpu.temp), gpu.watt });
    gtk_label_set_markup(ctx2.gpu_stat, gb.z());

    updateProcs(&ctx2.proc, ctx2.sep2, gpu, valid_gpu);
}

pub fn run2() void {
    ctx2 = .{};
    ctx2.mon = sensors.Monitor.init(std.heap.c_allocator);
    const app = gtk_application_new("com.github.yusufyav.power_panel", G_APPLICATION_DEFAULT_FLAGS);
    defer g_object_unref(app);
    _ = g_signal_connect_data(app, "activate", @as(GCallback, @ptrCast(&activate2)), null, null, 0);
    _ = g_application_run(app, 0, null);
}

// ════════════════════════════════════════════════════════════════════════════
//  gui1 — etiketli/ikonlu panel (build_ui)
// ════════════════════════════════════════════════════════════════════════════
const Ctx1 = struct {
    mon: sensors.Monitor = undefined,
    loop_count: u32 = 0,
    gfx_max: u32 = 0,

    window: Obj = null,
    total_label: Obj = null,
    cpu_watt: Obj = null,
    cpu_therm: Obj = null,
    cpu_temp: Obj = null,
    cpu_pct: Obj = null,
    gpu_watt: Obj = null,
    gpu_therm: Obj = null,
    gpu_temp: Obj = null,
    gpu_pct: Obj = null,
    ram_lbl: Obj = null,
    ram_pct: Obj = null,
    vram_row: Obj = null,
    vram_lbl: Obj = null,
    vram_pct: Obj = null,
    sep: Obj = null,
    proc: ProcWidgets = .{},
};

var ctx1: Ctx1 = .{};

// make_hw_row: [icon(3)] [name(hexpand)] [watt(8)] [therm(3)] [temp(5)] [pct(5)]
fn makeHwRow(icon: [*:0]const u8, name: [*:0]const u8, cls: [*:0]const u8, watt_out: *Obj, therm_out: *Obj, temp_out: *Obj, pct_out: *Obj) Obj {
    const row = gtk_box_new(ORIENTATION_HORIZONTAL, 0);

    const lbl_icon = label(icon, cls);
    gtk_label_set_width_chars(lbl_icon, 3);
    gtk_label_set_xalign(lbl_icon, 0.0);

    const lbl_name = label(name, cls);
    gtk_widget_set_hexpand(lbl_name, 1);
    gtk_label_set_xalign(lbl_name, 0.0);

    const lbl_watt = label("   0.0 W", "val-watt");
    gtk_label_set_width_chars(lbl_watt, 8);
    gtk_label_set_xalign(lbl_watt, 1.0);

    const lbl_therm = label(" ", "val-temp-cool");
    gtk_label_set_width_chars(lbl_therm, 3);
    gtk_label_set_xalign(lbl_therm, 1.0);

    const lbl_temp = label("  0°C", "val-temp-cool");
    gtk_label_set_width_chars(lbl_temp, 5);
    gtk_label_set_xalign(lbl_temp, 1.0);

    const lbl_pct = label("●  0%", "val-pct");
    gtk_label_set_width_chars(lbl_pct, 5);
    gtk_label_set_xalign(lbl_pct, 1.0);

    gtk_box_append(row, lbl_icon);
    gtk_box_append(row, lbl_name);
    gtk_box_append(row, lbl_watt);
    gtk_box_append(row, lbl_therm);
    gtk_box_append(row, lbl_temp);
    gtk_box_append(row, lbl_pct);

    watt_out.* = lbl_watt;
    therm_out.* = lbl_therm;
    temp_out.* = lbl_temp;
    pct_out.* = lbl_pct;
    return row;
}

fn makeRamRow(ram_out: *Obj, pct_out: *Obj) Obj {
    const row = gtk_box_new(ORIENTATION_HORIZONTAL, 0);
    const lbl_icon = label(ICON_RAM, "lbl-ram");
    gtk_label_set_width_chars(lbl_icon, 3);
    gtk_label_set_xalign(lbl_icon, 0.0);
    const lbl_name = label("RAM", "lbl-ram");
    gtk_widget_set_hexpand(lbl_name, 1);
    gtk_label_set_xalign(lbl_name, 0.0);
    const lbl_ram = label("    0/    0 MB ", "val-vram");
    gtk_label_set_width_chars(lbl_ram, 15);
    gtk_label_set_xalign(lbl_ram, 1.0);
    const lbl_pct = label("  0%", "val-pct");
    gtk_label_set_width_chars(lbl_pct, 5);
    gtk_label_set_xalign(lbl_pct, 1.0);
    gtk_box_append(row, lbl_icon);
    gtk_box_append(row, lbl_name);
    gtk_box_append(row, lbl_ram);
    gtk_box_append(row, lbl_pct);
    ram_out.* = lbl_ram;
    pct_out.* = lbl_pct;
    return row;
}

fn makeVramRow(vram_out: *Obj, pct_out: *Obj) Obj {
    const row = gtk_box_new(ORIENTATION_HORIZONTAL, 0);
    const lbl_icon = label(ICON_VRAM, "lbl-gpu");
    gtk_label_set_width_chars(lbl_icon, 3);
    gtk_label_set_xalign(lbl_icon, 0.0);
    const lbl_name = label("VRAM", "lbl-gpu");
    gtk_widget_set_hexpand(lbl_name, 1);
    gtk_label_set_xalign(lbl_name, 0.0);
    const lbl_vram = label("    0/    0 MB ", "val-vram");
    gtk_label_set_width_chars(lbl_vram, 15);
    gtk_label_set_xalign(lbl_vram, 1.0);
    const lbl_gfx = label("●  0%", "val-pct");
    gtk_label_set_width_chars(lbl_gfx, 5);
    gtk_label_set_xalign(lbl_gfx, 1.0);
    gtk_box_append(row, lbl_icon);
    gtk_box_append(row, lbl_name);
    gtk_box_append(row, lbl_vram);
    gtk_box_append(row, lbl_gfx);
    vram_out.* = lbl_vram;
    pct_out.* = lbl_gfx;
    return row;
}

fn activate1(app: *anyopaque, _: ?*anyopaque) callconv(.c) void {
    const window = makeWindow(app);
    ctx1.window = window;
    loadCss(CSS1);

    const panel = gtk_box_new(ORIENTATION_VERTICAL, 8);
    gtk_widget_add_css_class(panel, "panel");
    gtk_widget_set_size_request(panel, 340, -1);

    ctx1.total_label = label("⚡    0.0 W", "total-watt");
    gtk_widget_set_halign(ctx1.total_label, ALIGN_CENTER);
    gtk_box_append(panel, ctx1.total_label);

    gtk_box_append(panel, makeHwRow(ICON_CPU, "CPU", "lbl-cpu", &ctx1.cpu_watt, &ctx1.cpu_therm, &ctx1.cpu_temp, &ctx1.cpu_pct));
    gtk_box_append(panel, makeHwRow(ICON_GPU, "GPU", "lbl-gpu", &ctx1.gpu_watt, &ctx1.gpu_therm, &ctx1.gpu_temp, &ctx1.gpu_pct));
    gtk_box_append(panel, makeRamRow(&ctx1.ram_lbl, &ctx1.ram_pct));
    ctx1.vram_row = makeVramRow(&ctx1.vram_lbl, &ctx1.vram_pct);
    gtk_widget_set_visible(ctx1.vram_row, 0);
    gtk_box_append(panel, ctx1.vram_row);

    ctx1.sep = divider("divider");
    gtk_widget_set_visible(ctx1.sep, 0);
    gtk_box_append(panel, ctx1.sep);

    ctx1.proc = makeProcSection();
    gtk_widget_set_visible(ctx1.proc.container, 0);
    gtk_box_append(panel, ctx1.proc.container);

    gtk_window_set_child(window, panel);

    const gesture = gtk_gesture_click_new();
    gtk_gesture_single_set_button(gesture, 3);
    _ = g_signal_connect_data(gesture, "released", @as(GCallback, @ptrCast(&onReleased)), window, null, 0);
    gtk_widget_add_controller(window, gesture);

    gtk_window_present(window);
    _ = g_timeout_add(200, tick1, null);
}

fn tick1(_: ?*anyopaque) callconv(.c) c_int {
    ctx1.loop_count += 1;
    ctx1.gfx_max = @max(ctx1.gfx_max, ctx1.mon.quickGfx());
    if (ctx1.loop_count % 5 == 1) {
        var snap = ctx1.mon.sample();
        snap.gpu.gfx_percent = @max(snap.gpu.gfx_percent, ctx1.gfx_max);
        ctx1.gfx_max = 0;
        updateUi1(&snap);
    }
    return 1;
}

fn updateUi1(snap: *const sensors.Snapshot) void {
    const gpu = &snap.gpu;
    var b: Buf = .{};

    b.print("⚡ {d:>6.1} W", .{snap.cpu_watt + gpu.watt});
    gtk_label_set_text(ctx1.total_label, b.z());

    // CPU
    b.len = 0;
    b.print("{d:>6.1} W", .{snap.cpu_watt});
    gtk_label_set_text(ctx1.cpu_watt, b.z());
    const cpu_cls = tempClass(snap.cpu_temp);
    setClasses(ctx1.cpu_therm, cpu_cls);
    setClasses(ctx1.cpu_temp, cpu_cls);
    b.len = 0;
    b.print("{d:>3}°C", .{floorTemp(snap.cpu_temp)});
    gtk_label_set_text(ctx1.cpu_temp, b.z());
    setClasses(ctx1.cpu_pct, usageClass(snap.cpu_percent));
    b.len = 0;
    b.print("●{d:>3}%", .{snap.cpu_percent});
    gtk_label_set_text(ctx1.cpu_pct, b.z());

    // GPU
    b.len = 0;
    b.print("{d:>6.1} W", .{gpu.watt});
    gtk_label_set_text(ctx1.gpu_watt, b.z());
    const gpu_cls = tempClass(gpu.temp);
    setClasses(ctx1.gpu_therm, gpu_cls);
    setClasses(ctx1.gpu_temp, gpu_cls);
    b.len = 0;
    b.print("{d:>3}°C", .{floorTemp(gpu.temp)});
    gtk_label_set_text(ctx1.gpu_temp, b.z());
    const has_pct = gpuHasPct(gpu.kind);
    setClasses(ctx1.gpu_pct, if (has_pct) usageClass(gpu.gfx_percent) else "val-pct");
    b.len = 0;
    if (has_pct) b.print("●{d:>3}%", .{gpu.gfx_percent}) else b.raw("●  —");
    gtk_label_set_text(ctx1.gpu_pct, b.z());

    const valid_gpu = gpu.kind != .unknown;

    // RAM
    if (snap.ram_total_mb > 0) {
        const ram_pct = snap.ram_used_mb * 100 / snap.ram_total_mb;
        b.len = 0;
        b.print("{d:>5}/{d:>5} MB ", .{ snap.ram_used_mb, snap.ram_total_mb });
        gtk_label_set_text(ctx1.ram_lbl, b.z());
        setClasses(ctx1.ram_pct, usageClass(ram_pct));
        b.len = 0;
        b.print("●{d:>3}%", .{ram_pct});
        gtk_label_set_text(ctx1.ram_pct, b.z());
    }

    // VRAM
    if (valid_gpu and gpu.vram_total_mb > 0) {
        gtk_widget_set_visible(ctx1.vram_row, 1);
        b.len = 0;
        b.print("{d:>5}/{d:>5} MB ", .{ gpu.vram_used_mb, gpu.vram_total_mb });
        gtk_label_set_text(ctx1.vram_lbl, b.z());
        const vram_pct = gpu.vram_used_mb * 100 / gpu.vram_total_mb;
        setClasses(ctx1.vram_pct, usageClass(vram_pct));
        b.len = 0;
        b.print("●{d:>3}%", .{vram_pct});
        gtk_label_set_text(ctx1.vram_pct, b.z());
    } else {
        gtk_widget_set_visible(ctx1.vram_row, 0);
    }

    updateProcs(&ctx1.proc, ctx1.sep, gpu, valid_gpu);
}

pub fn run1() void {
    ctx1 = .{};
    ctx1.mon = sensors.Monitor.init(std.heap.c_allocator);
    const app = gtk_application_new("com.github.yusufyav.power_panel", G_APPLICATION_DEFAULT_FLAGS);
    defer g_object_unref(app);
    _ = g_signal_connect_data(app, "activate", @as(GCallback, @ptrCast(&activate1)), null, null, 0);
    _ = g_application_run(app, 0, null);
}
