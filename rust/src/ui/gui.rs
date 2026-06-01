use super::tui::fmt_gb;
#[cfg(debug_assertions)]
use crate::diagnostics::run_diagnostics;
use crate::gpu::{detect_gpu, read_gpu_data, FdInfoTracker, GpuBackend, IntelFdInfoTracker};
use crate::sensors::{detect_cpu_temp_path, find_rapl_path, read_u64};
use crate::types::{
    usage_percent, CombinedProc, GpuKind, GpuPowerTracker, PowerTracker, SensorData,
};
use crate::APP_ID;
use gtk4::prelude::*;
use gtk4::{
    glib, Application, ApplicationWindow, Box as GtkBox, CssProvider, Grid, Label, Orientation,
};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use sysinfo::{Components, System};

// ── GUI stilleri ────────────────────────────────────────────────────────────
// Çalışma zamanında geçiş yapılabilen panel stilleri. Yeni stil eklemek:
// variant + name/stack_name/next match kolları + bir build_*_content fonksiyonu.

#[derive(Clone, Copy, PartialEq)]
enum GuiStyle {
    Classic,
    Bars,
}

impl GuiStyle {
    fn name(self) -> &'static str {
        match self {
            GuiStyle::Classic => "Classic",
            GuiStyle::Bars => "Bars",
        }
    }

    fn label(self) -> String {
        format!("⟳ {}", self.name())
    }

    fn stack_name(self) -> &'static str {
        match self {
            GuiStyle::Classic => "classic",
            GuiStyle::Bars => "bars",
        }
    }

    fn next(self) -> Self {
        match self {
            GuiStyle::Classic => GuiStyle::Bars,
            GuiStyle::Bars => GuiStyle::Classic,
        }
    }
}

// Bir stil içeriği: kök widget + her-tick güncelleme closure'ı.
type Content = (GtkBox, Box<dyn Fn(&SensorData)>);

// Tüm stillerin CSS'i tek provider'da. Kart arka planı paylaşılan .panel-card'da;
// çakışan içerik sınıfları (.total-watt, .lbl-cpu, .val-pct ...) her stilde farklı
// boyutta olduğundan .panel / .panel2 ebeveyni altında kapsamlanır.
const PANEL_CSS: &str = "
    window { background-color: transparent; }

    .panel-card {
        background-color: rgba(10, 10, 10, 0.82);
        border-radius: 18px;
        border: 1px solid rgba(255, 255, 255, 0.15);
        padding: 14px 18px;
    }

    .style-toggle {
        color: #a0a8b0;
        font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 11px;
        background-image: none;
        background-color: rgba(255, 255, 255, 0.06);
        border: 1px solid rgba(255, 255, 255, 0.12);
        border-radius: 8px;
        box-shadow: none;
        padding: 1px 8px;
        min-height: 0;
        min-width: 0;
    }
    .style-toggle:hover {
        background-image: none;
        background-color: rgba(255, 255, 255, 0.14);
    }

    /* ── Paylaşılan process tablosu ── */
    .proc-hdr {
        color: #a29bfe; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 12px; font-weight: bold;
    }
    .proc-val {
        color: #b2bec3; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 12px;
    }
    .proc-num {
        color: #dfe6e9; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 12px;
    }
    .divider {
        background-color: rgba(255, 255, 255, 0.10);
        min-height: 1px; margin: 4px 0px;
    }

    /* ── Classic (.panel) ── */
    .panel .total-watt {
        color: #00ffcc; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 26px; font-weight: bold;
    }
    .panel .lbl-cpu {
        color: #ff9f43; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 16px; font-weight: bold;
    }
    .panel .lbl-gpu {
        color: #2ecc71; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 16px; font-weight: bold;
    }
    .panel .val-watt {
        color: #ffffff; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 16px;
    }
    .panel .val-temp-cool {
        color: #4cd964; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 16px;
    }
    .panel .val-temp-warm {
        color: #ff9f43; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 16px;
    }
    .panel .val-temp-hot {
        color: #ff4757; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 16px;
    }
    .panel .lbl-ram {
        color: #00cec9; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 16px; font-weight: bold;
    }
    .panel .val-vram {
        color: #74b9ff; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 14px;
    }
    .panel .val-pct {
        color: #dfe6e9; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 16px;
    }

    /* ── Bars (.panel2) ── */
    .panel2 .brand-lbl {
        color: #a0a8b0; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 13px;
    }
    .panel2 .total-watt {
        color: #00ffcc; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 22px; font-weight: bold;
    }
    .panel2 .lbl-cpu {
        color: #ff9f43; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 14px; font-weight: bold;
    }
    .panel2 .lbl-gpu {
        color: #2ecc71; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 14px; font-weight: bold;
    }
    .panel2 .lbl-ram {
        color: #00cec9; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 14px; font-weight: bold;
    }
    .panel2 .val-pct {
        color: #dfe6e9; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 13px;
    }
    .panel2 .stat-lbl {
        font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
        font-size: 13px;
    }
";

fn temp_css_class(temp: f32) -> &'static str {
    if temp >= 80.0 {
        "val-temp-hot"
    } else if temp >= 60.0 {
        "val-temp-warm"
    } else {
        "val-temp-cool"
    }
}

fn usage_css_class(pct: u32) -> &'static str {
    if pct >= 90 {
        "val-temp-hot"
    } else if pct >= 75 {
        "val-temp-warm"
    } else {
        "val-pct"
    }
}

fn temp_hex_color(t: f32) -> &'static str {
    if t >= 80.0 {
        "#ff4757"
    } else if t >= 60.0 {
        "#ff9f43"
    } else {
        "#4cd964"
    }
}

fn make_style_toggle(label: &str) -> gtk4::Button {
    gtk4::Button::builder()
        .label(label)
        .css_classes(vec!["style-toggle".to_string()])
        .halign(gtk4::Align::Start)
        .build()
}

// ── Classic içeriği ─────────────────────────────────────────────────────────

fn build_classic_content() -> Content {
    let panel = GtkBox::new(Orientation::Vertical, 8);
    panel.add_css_class("panel");

    let total_label = Label::new(Some("⚡    0.0 W"));
    total_label.add_css_class("total-watt");
    total_label.set_halign(gtk4::Align::Center);
    panel.append(&total_label);

    let (cpu_row, cpu_watt_lbl, cpu_therm_lbl, cpu_temp_lbl, cpu_pct_lbl) =
        make_hw_row("\u{f4bc}", "CPU", "lbl-cpu");
    panel.append(&cpu_row);

    let (gpu_row, gpu_watt_lbl, gpu_therm_lbl, gpu_temp_lbl, gpu_pct_lbl) =
        make_hw_row("\u{f08ae}", "GPU", "lbl-gpu");
    panel.append(&gpu_row);

    let (ram_row, ram_lbl, ram_pct_lbl) = make_ram_row();
    panel.append(&ram_row);

    let (vram_row, vram_lbl, vram_pct_lbl) = make_vram_row();
    vram_row.set_visible(false);
    panel.append(&vram_row);

    let update = Box::new(move |target: &SensorData| {
        total_label.set_text(&format!("⚡ {:>6.1} W", target.cpu_watt + target.gpu_watt));

        cpu_watt_lbl.set_text(&format!("{:>6.1} W", target.cpu_watt));
        let cpu_cls = temp_css_class(target.cpu_temp);
        cpu_therm_lbl.set_css_classes(&[cpu_cls]);
        cpu_temp_lbl.set_css_classes(&[cpu_cls]);
        cpu_temp_lbl.set_text(&format!("{:>3.0}°C", target.cpu_temp.floor()));
        cpu_pct_lbl.set_css_classes(&[usage_css_class(target.cpu_percent)]);
        cpu_pct_lbl.set_text(&format!("●{:>3}%", target.cpu_percent));

        gpu_watt_lbl.set_text(&format!("{:>6.1} W", target.gpu_watt));
        let gpu_cls = temp_css_class(target.gpu_temp);
        gpu_therm_lbl.set_css_classes(&[gpu_cls]);
        gpu_temp_lbl.set_css_classes(&[gpu_cls]);
        gpu_temp_lbl.set_text(&format!("{:>3.0}°C", target.gpu_temp.floor()));
        let gpu_has_pct = matches!(target.gpu_kind, GpuKind::Nvidia | GpuKind::Amd);
        let gpu_pct_text = if gpu_has_pct {
            format!("●{:>3}%", target.gpu_gfx_percent)
        } else {
            "●  —".to_string()
        };
        gpu_pct_lbl.set_css_classes(&[if gpu_has_pct {
            usage_css_class(target.gpu_gfx_percent)
        } else {
            "val-pct"
        }]);
        gpu_pct_lbl.set_text(&gpu_pct_text);

        let valid_gpu = target.gpu_kind != GpuKind::Unknown;

        if target.ram_total_mb > 0 {
            let ram_pct = usage_percent(target.ram_used_mb, target.ram_total_mb);
            ram_lbl.set_text(&format!(
                "{:>5}/{:>5} MB ",
                target.ram_used_mb, target.ram_total_mb
            ));
            ram_pct_lbl.set_css_classes(&[usage_css_class(ram_pct)]);
            ram_pct_lbl.set_text(&format!("●{:>3}%", ram_pct));
        }

        if valid_gpu && target.vram_total_mb > 0 {
            vram_row.set_visible(true);
            vram_lbl.set_text(&format!(
                "{:>5}/{:>5} MB ",
                target.vram_used_mb, target.vram_total_mb
            ));
            let vram_pct = usage_percent(target.vram_used_mb, target.vram_total_mb);
            vram_pct_lbl.set_css_classes(&[usage_css_class(vram_pct)]);
            vram_pct_lbl.set_text(&format!("●{:>3}%", vram_pct));
        } else {
            vram_row.set_visible(false);
        }
    });

    (panel, update)
}

fn make_hw_row(icon: &str, name: &str, cls: &str) -> (GtkBox, Label, Label, Label, Label) {
    let row = GtkBox::new(Orientation::Horizontal, 0);
    let lbl_icon = Label::builder()
        .label(icon)
        .css_classes(vec![cls.to_string()])
        .width_chars(3)
        .xalign(0.0)
        .build();
    let lbl_name = Label::builder()
        .label(name)
        .css_classes(vec![cls.to_string()])
        .hexpand(true)
        .xalign(0.0)
        .build();
    let lbl_watt = Label::builder()
        .label("   0.0 W")
        .css_classes(vec!["val-watt".to_string()])
        .width_chars(8)
        .xalign(1.0)
        .build();
    let lbl_therm = Label::builder()
        .label(" ")
        .css_classes(vec!["val-temp-cool".to_string()])
        .width_chars(3)
        .xalign(1.0)
        .build();
    let lbl_temp = Label::builder()
        .label("  0°C")
        .css_classes(vec!["val-temp-cool".to_string()])
        .width_chars(5)
        .xalign(1.0)
        .build();
    let lbl_pct = Label::builder()
        .label("●  0%")
        .css_classes(vec!["val-pct".to_string()])
        .width_chars(5)
        .xalign(1.0)
        .build();
    row.append(&lbl_icon);
    row.append(&lbl_name);
    row.append(&lbl_watt);
    row.append(&lbl_therm);
    row.append(&lbl_temp);
    row.append(&lbl_pct);
    (row, lbl_watt, lbl_therm, lbl_temp, lbl_pct)
}

fn make_vram_row() -> (GtkBox, Label, Label) {
    let row = GtkBox::new(Orientation::Horizontal, 0);
    let lbl_icon = Label::builder()
        .label("\u{f048b}")
        .css_classes(vec!["lbl-gpu".to_string()])
        .width_chars(3)
        .xalign(0.0)
        .build();
    let lbl_name = Label::builder()
        .label("VRAM")
        .css_classes(vec!["lbl-gpu".to_string()])
        .hexpand(true)
        .xalign(0.0)
        .build();
    let lbl_vram = Label::builder()
        .label("    0/    0 MB ")
        .css_classes(vec!["val-vram".to_string()])
        .width_chars(15)
        .xalign(1.0)
        .build();
    let lbl_gfx = Label::builder()
        .label("●  0%")
        .css_classes(vec!["val-pct".to_string()])
        .width_chars(5)
        .xalign(1.0)
        .build();
    row.append(&lbl_icon);
    row.append(&lbl_name);
    row.append(&lbl_vram);
    row.append(&lbl_gfx);
    (row, lbl_vram, lbl_gfx)
}

fn make_ram_row() -> (GtkBox, Label, Label) {
    let row = GtkBox::new(Orientation::Horizontal, 0);
    let lbl_icon = Label::builder()
        .label("\u{f035b}")
        .css_classes(vec!["lbl-ram".to_string()])
        .width_chars(3)
        .xalign(0.0)
        .build();
    let lbl_name = Label::builder()
        .label("RAM")
        .css_classes(vec!["lbl-ram".to_string()])
        .hexpand(true)
        .xalign(0.0)
        .build();
    let lbl_ram = Label::builder()
        .label("    0/    0 MB ")
        .css_classes(vec!["val-vram".to_string()])
        .width_chars(15)
        .xalign(1.0)
        .build();
    let lbl_pct = Label::builder()
        .label("  0%")
        .css_classes(vec!["val-pct".to_string()])
        .width_chars(5)
        .xalign(1.0)
        .build();
    row.append(&lbl_icon);
    row.append(&lbl_name);
    row.append(&lbl_ram);
    row.append(&lbl_pct);
    (row, lbl_ram, lbl_pct)
}

// ── Paylaşılan process bölümü (manuel açılır) ───────────────────────────────

fn make_process_grid() -> (Grid, Label, Label, Label, Label, Label) {
    // returns: (container, proc, gfx, dec, enc, sm)
    // GtkGrid ensures header and data columns share the same allocated width,
    // so xalign(1.0) right-aligns both header text and values to the same right edge.
    let grid = Grid::builder().row_spacing(4).build();

    let lbl_name_hdr = Label::builder()
        .label("Process")
        .css_classes(vec!["proc-hdr".to_string()])
        .hexpand(true)
        .xalign(0.0)
        .build();
    let lbl_gfx_hdr = Label::builder()
        .label("GFX")
        .css_classes(vec!["proc-hdr".to_string()])
        .xalign(1.0)
        .build();
    let lbl_dec_hdr = Label::builder()
        .label("DEC")
        .css_classes(vec!["proc-hdr".to_string()])
        .xalign(1.0)
        .build();
    let lbl_enc_hdr = Label::builder()
        .label("ENC")
        .css_classes(vec!["proc-hdr".to_string()])
        .xalign(1.0)
        .build();
    let lbl_sm_hdr = Label::builder()
        .label("SM%")
        .css_classes(vec!["proc-hdr".to_string()])
        .xalign(1.0)
        .build();

    let lbl_proc = Label::builder()
        .css_classes(vec!["proc-val".to_string()])
        .hexpand(true)
        .xalign(0.0)
        .valign(gtk4::Align::Start)
        .max_width_chars(12)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    let lbl_gfx = Label::builder()
        .css_classes(vec!["proc-num".to_string()])
        .xalign(1.0)
        .justify(gtk4::Justification::Right)
        .valign(gtk4::Align::Start)
        .build();
    let lbl_dec = Label::builder()
        .css_classes(vec!["proc-num".to_string()])
        .xalign(1.0)
        .justify(gtk4::Justification::Right)
        .valign(gtk4::Align::Start)
        .build();
    let lbl_enc = Label::builder()
        .css_classes(vec!["proc-num".to_string()])
        .xalign(1.0)
        .justify(gtk4::Justification::Right)
        .valign(gtk4::Align::Start)
        .build();
    let lbl_sm = Label::builder()
        .css_classes(vec!["proc-num".to_string()])
        .xalign(1.0)
        .justify(gtk4::Justification::Right)
        .valign(gtk4::Align::Start)
        .build();

    grid.attach(&lbl_name_hdr, 0, 0, 1, 1);
    grid.attach(&lbl_gfx_hdr, 1, 0, 1, 1);
    grid.attach(&lbl_dec_hdr, 2, 0, 1, 1);
    grid.attach(&lbl_enc_hdr, 3, 0, 1, 1);
    grid.attach(&lbl_sm_hdr, 4, 0, 1, 1);
    grid.attach(&lbl_proc, 0, 1, 1, 1);
    grid.attach(&lbl_gfx, 1, 1, 1, 1);
    grid.attach(&lbl_dec, 2, 1, 1, 1);
    grid.attach(&lbl_enc, 3, 1, 1, 1);
    grid.attach(&lbl_sm, 4, 1, 1, 1);

    (grid, lbl_proc, lbl_gfx, lbl_dec, lbl_enc, lbl_sm)
}

// Alt process kutusu: ayraç + manuel aç/kapa düğmesi (▾/▴ Procs) + tablo.
// Tablo varsayılan kapalı; düğme ile açılır. Stil değişiminden bağımsız paylaşılır.
fn build_proc_section() -> Content {
    let footer = GtkBox::new(Orientation::Vertical, 6);

    let sep = gtk4::Separator::new(Orientation::Horizontal);
    sep.add_css_class("divider");
    footer.append(&sep);

    // Yön ikonu: kapalıyken aşağı (tıkla → panel aşağı açılır),
    // açıkken yukarı (tıkla → panel yukarı kapanır).
    let toggle = gtk4::Button::builder()
        .label("\u{f1a09}")
        .css_classes(vec!["style-toggle".to_string()])
        .halign(gtk4::Align::Start)
        .build();
    footer.append(&toggle);

    let (grid, proc_lbl, gfx_lbl, dec_lbl, enc_lbl, sm_lbl) = make_process_grid();
    grid.set_visible(false);
    footer.append(&grid);

    // Manuel aç/kapa: tablo görünürlüğü kullanıcı kontrolünde.
    let expanded = Rc::new(Cell::new(false));
    {
        let grid = grid.clone();
        let toggle_lbl = toggle.clone();
        let expanded = expanded.clone();
        toggle.connect_clicked(move |_| {
            let e = !expanded.get();
            expanded.set(e);
            grid.set_visible(e);
            toggle_lbl.set_label(if e { "\u{f1a0a}" } else { "\u{f1a09}" });
        });
    }

    let update = Box::new(move |target: &SensorData| {
        let valid_gpu = target.gpu_kind != GpuKind::Unknown;
        let has_media = valid_gpu && !target.media_procs.is_empty();
        let has_compute = valid_gpu && !target.compute_procs.is_empty();

        if !(has_media || has_compute) {
            proc_lbl.set_text("");
            gfx_lbl.set_text("");
            dec_lbl.set_text("");
            enc_lbl.set_text("");
            sm_lbl.set_text("");
            return;
        }

        let combined = CombinedProc::from_sensor(target);

        let trunc = |n: &str| -> String {
            if n.chars().count() > 11 {
                format!("{}…", n.chars().take(10).collect::<String>())
            } else {
                n.to_string()
            }
        };
        let fmt_val = |v: Option<u32>| -> String {
            match v {
                Some(x) if x > 0 => format!("{:>3}%", x),
                _ => "   —".to_string(),
            }
        };

        proc_lbl.set_text(
            &combined
                .iter()
                .map(|proc| trunc(&proc.name))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        gfx_lbl.set_text(
            &combined
                .iter()
                .map(|proc| fmt_val(proc.gfx))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        dec_lbl.set_text(
            &combined
                .iter()
                .map(|proc| fmt_val(proc.dec))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        enc_lbl.set_text(
            &combined
                .iter()
                .map(|proc| fmt_val(proc.enc))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        sm_lbl.set_text(
            &combined
                .iter()
                .map(|proc| fmt_val(proc.sm))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    });

    (footer, update)
}

// ── Bars içeriği yardımcıları ────────────────────────────────────────────────

fn draw_bar_fn(cr: &gtk4::cairo::Context, width: i32, height: i32, pct: u32) {
    let pct = pct.min(100);
    let filled_w = (pct as f64 / 100.0 * width as f64) as i32;
    let (r, g, b) = if pct >= 75 {
        (0.91, 0.30, 0.24) // red
    } else if pct >= 50 {
        (0.90, 0.49, 0.13) // orange
    } else if pct >= 25 {
        (0.95, 0.77, 0.06) // yellow
    } else {
        (0.18, 0.80, 0.44) // green
    };
    let y = 1.0_f64;
    let h = (height - 2) as f64;
    if filled_w > 0 {
        cr.set_source_rgb(r, g, b);
        cr.rectangle(0.0, y, filled_w as f64, h);
        cr.fill().ok();
    }
    if filled_w < width {
        cr.set_source_rgba(r, g, b, 0.15);
        cr.rectangle(filled_w as f64, y, (width - filled_w) as f64, h);
        cr.fill().ok();
    }
}

fn make_bar_row_2(
    label: &str,
    css_class: &str,
    val_width_chars: i32,
) -> (GtkBox, gtk4::DrawingArea, Rc<Cell<u32>>, Label) {
    let row = GtkBox::new(Orientation::Horizontal, 6);
    row.set_valign(gtk4::Align::Center);

    let lbl = Label::builder()
        .label(label)
        .css_classes(vec![css_class.to_string()])
        .width_chars(4)
        .xalign(0.0)
        .build();

    let pct_cell = Rc::new(Cell::new(0u32));
    let pct_draw = pct_cell.clone();

    let bar = gtk4::DrawingArea::new();
    bar.set_hexpand(true);
    bar.set_content_height(12);
    bar.set_valign(gtk4::Align::Center);
    bar.set_draw_func(move |_, cr, w, h| draw_bar_fn(cr, w, h, pct_draw.get()));

    let val_lbl = Label::builder()
        .label("")
        .css_classes(vec!["val-pct".to_string()])
        .width_chars(val_width_chars)
        .xalign(1.0)
        .build();

    row.append(&lbl);
    row.append(&bar);
    row.append(&val_lbl);

    (row, bar, pct_cell, val_lbl)
}

// ── Bars içeriği ────────────────────────────────────────────────────────────

fn build_bars_content() -> Content {
    let panel = GtkBox::new(Orientation::Vertical, 6);
    panel.add_css_class("panel2");

    // Title row: brand left, total watts right
    let title_row = GtkBox::new(Orientation::Horizontal, 0);
    let brand_lbl = Label::builder()
        .label("PowerPanel")
        .css_classes(vec!["brand-lbl".to_string()])
        .hexpand(true)
        .xalign(0.0)
        .valign(gtk4::Align::End)
        .build();
    let total_label = Label::builder()
        .label("⚡  0.0 W")
        .css_classes(vec!["total-watt".to_string()])
        .xalign(1.0)
        .build();
    title_row.append(&brand_lbl);
    title_row.append(&total_label);
    panel.append(&title_row);

    let sep_top = gtk4::Separator::new(Orientation::Horizontal);
    sep_top.add_css_class("divider");
    panel.append(&sep_top);

    // Bar rows: CPU, GPU (pct val), RAM, VRAM (GB val).
    // val_width_chars equal across rows so all bars get identical width.
    let (cpu_row, cpu_bar, cpu_pct_cell, cpu_val_lbl) = make_bar_row_2("CPU", "lbl-cpu", 11);
    panel.append(&cpu_row);

    let (gpu_row, gpu_bar, gpu_pct_cell, gpu_val_lbl) = make_bar_row_2("GPU", "lbl-gpu", 11);
    panel.append(&gpu_row);

    let (ram_row, ram_bar, ram_pct_cell, ram_val_lbl) = make_bar_row_2("RAM", "lbl-ram", 11);
    panel.append(&ram_row);

    let (vram_row, vram_bar, vram_pct_cell, vram_val_lbl) = make_bar_row_2("VRAM", "lbl-gpu", 11);
    vram_row.set_visible(false);
    panel.append(&vram_row);

    // Stats strip: CPU temp/watt | GPU temp/watt
    let sep_mid = gtk4::Separator::new(Orientation::Horizontal);
    sep_mid.add_css_class("divider");
    panel.append(&sep_mid);

    let stats_row = GtkBox::new(Orientation::Horizontal, 0);
    let cpu_stat_lbl = Label::builder()
        .use_markup(true)
        .label("<span foreground='#ff9f43'><b>CPU</b></span>  --°C    0.0W")
        .css_classes(vec!["stat-lbl".to_string()])
        .hexpand(true)
        .xalign(0.0)
        .build();
    let gpu_stat_lbl = Label::builder()
        .use_markup(true)
        .label("<span foreground='#2ecc71'><b>GPU</b></span>  --°C    0.0W")
        .css_classes(vec!["stat-lbl".to_string()])
        .xalign(1.0)
        .build();
    stats_row.append(&cpu_stat_lbl);
    stats_row.append(&gpu_stat_lbl);
    panel.append(&stats_row);

    let update = Box::new(move |target: &SensorData| {
        total_label.set_text(&format!("⚡ {:>6.1} W", target.cpu_watt + target.gpu_watt));

        // CPU bar
        cpu_pct_cell.set(target.cpu_percent.min(100));
        cpu_bar.queue_draw();
        cpu_val_lbl.set_text(&format!("{:>3}%", target.cpu_percent));

        // GPU bar
        let gpu_has_pct = matches!(target.gpu_kind, GpuKind::Nvidia | GpuKind::Amd);
        let gpu_pct = if gpu_has_pct {
            target.gpu_gfx_percent.min(100)
        } else {
            0
        };
        gpu_pct_cell.set(gpu_pct);
        gpu_bar.queue_draw();
        let gpu_val_str = if gpu_has_pct {
            format!("{:>3}%", target.gpu_gfx_percent)
        } else {
            "  —".to_string()
        };
        gpu_val_lbl.set_text(&gpu_val_str);

        // RAM bar
        if target.ram_total_mb > 0 {
            let ram_pct = usage_percent(target.ram_used_mb, target.ram_total_mb).min(100);
            ram_pct_cell.set(ram_pct);
            ram_bar.queue_draw();
            ram_val_lbl.set_text(&fmt_gb(target.ram_used_mb, target.ram_total_mb));
        }

        // VRAM bar
        let valid_gpu = target.gpu_kind != GpuKind::Unknown;
        if valid_gpu && target.vram_total_mb > 0 {
            let vram_pct = usage_percent(target.vram_used_mb, target.vram_total_mb).min(100);
            vram_pct_cell.set(vram_pct);
            vram_bar.queue_draw();
            vram_val_lbl.set_text(&fmt_gb(target.vram_used_mb, target.vram_total_mb));
            vram_row.set_visible(true);
        } else {
            vram_row.set_visible(false);
        }

        // Stats strip
        let cpu_tc = temp_hex_color(target.cpu_temp);
        let gpu_tc = temp_hex_color(target.gpu_temp);
        cpu_stat_lbl.set_markup(&format!(
            "<span foreground='#ff9f43'><b>CPU</b></span>  <span foreground='{}'>{:>3.0}°C</span>  <span foreground='#ffffff'>{:>5.1}W</span>",
            cpu_tc, target.cpu_temp.floor(), target.cpu_watt
        ));
        gpu_stat_lbl.set_markup(&format!(
            "<span foreground='#2ecc71'><b>GPU</b></span>  <span foreground='{}'>{:>3.0}°C</span>  <span foreground='#ffffff'>{:>5.1}W</span>",
            gpu_tc, target.gpu_temp.floor(), target.gpu_watt
        ));
    });

    (panel, update)
}

// ── Sensör thread (her iki stil için ortak) ──────────────────────────────────

fn spawn_sensor_thread(data_writer: Arc<Mutex<SensorData>>) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let mut comps = Components::new_with_refreshed_list();
            let mut sys = System::new();

            let gpu_backend = detect_gpu();

            let mut intel_gpu_tracker: Option<GpuPowerTracker> = None;
            let mut amd_fdinfo_tracker = match &gpu_backend {
                GpuBackend::Amd {
                    pdev,
                    vcn_instances,
                    ..
                } => Some(FdInfoTracker::new(pdev.clone(), *vcn_instances)),
                _ => None,
            };
            let mut intel_fdinfo_tracker = match &gpu_backend {
                GpuBackend::Intel { .. } => Some(IntelFdInfoTracker::new()),
                _ => None,
            };

            let mut tracker = PowerTracker {
                path: find_rapl_path(),
                last_energy: 0,
                last_time: Instant::now(),
            };
            if let Some(p) = tracker.path {
                tracker.last_energy = read_u64(p).unwrap_or(0);
            }

            // TEŞHİS MOTORUNU ÇALIŞTIRIYORUZ
            #[cfg(debug_assertions)]
            run_diagnostics(&tracker.path, &gpu_backend);

            // CPU sıcaklık sensörünü kernel'den direkt okumak için
            let cpu_temp_path = detect_cpu_temp_path();

            let mut loop_count: u32 = 0;
            let mut gfx_max: u32 = 0;

            loop {
                loop_count += 1;

                // Quick GFX sample every 200ms — catches bursts that 1s polling misses
                match &gpu_backend {
                    GpuBackend::Nvidia(nvml) => {
                        if let Ok(dev) = nvml.device_by_index(0) {
                            if let Ok(util) = dev.utilization_rates() {
                                gfx_max = gfx_max.max(util.gpu);
                            }
                        }
                    }
                    GpuBackend::Amd { device_path, .. } => {
                        if let Ok(v) = read_u64(&format!("{}/gpu_busy_percent", device_path)) {
                            gfx_max = gfx_max.max(v as u32);
                        }
                    }
                    _ => {}
                }

                // Full sensor read: immediately on first tick, then every 1000ms
                if loop_count % 5 == 1 {
                    let cpu_temp = if let Some(ref path) = cpu_temp_path {
                        read_u64(path).map(|v| v as f32 / 1000.0).unwrap_or(0.0)
                    } else {
                        comps.refresh(false);
                        let mut temp = 0.0f32;
                        let mut found_die = false;
                        for c in &comps {
                            let lbl = c.label().to_lowercase();
                            if lbl == "tdie" {
                                if let Some(t) = c.temperature() {
                                    temp = t;
                                    found_die = true;
                                    break;
                                }
                            }
                        }
                        if !found_die {
                            for c in &comps {
                                let lbl = c.label().to_lowercase();
                                if lbl == "tctl"
                                    || lbl.contains("k10")
                                    || lbl.contains("composite")
                                    || lbl.contains("package")
                                {
                                    if let Some(t) = c.temperature() {
                                        if t > temp {
                                            temp = t;
                                        }
                                    }
                                }
                            }
                        }
                        temp
                    };

                    let cpu_watt_raw = if let Some(path) = tracker.path {
                        match read_u64(path) {
                            Ok(current) => {
                                let now = Instant::now();
                                let elapsed = now.duration_since(tracker.last_time).as_secs_f32();
                                let watts = if elapsed > 0.1 {
                                    let diff = current.saturating_sub(tracker.last_energy);
                                    (diff as f32 / elapsed) / 1_000_000.0
                                } else {
                                    0.0
                                };
                                tracker.last_energy = current;
                                tracker.last_time = now;
                                if watts > 1.0 && watts < 400.0 {
                                    watts
                                } else {
                                    0.0
                                }
                            }
                            Err(_) => 0.0,
                        }
                    } else {
                        0.0
                    };

                    let mut gpu = read_gpu_data(
                        &gpu_backend,
                        &mut sys,
                        &mut intel_gpu_tracker,
                        &mut amd_fdinfo_tracker,
                        &mut intel_fdinfo_tracker,
                    );

                    // Use max GFX seen over the last 5 ticks
                    gpu.gfx_percent = gpu.gfx_percent.max(gfx_max);
                    gfx_max = 0;

                    sys.refresh_memory();
                    sys.refresh_cpu_usage();
                    let ram_used_mb = (sys.used_memory() / 1_048_576) as u32;
                    let ram_total_mb = (sys.total_memory() / 1_048_576) as u32;
                    let cpu_percent = sys.global_cpu_usage() as u32;

                    if let Ok(mut d) = data_writer.lock() {
                        d.cpu_temp = cpu_temp;
                        d.cpu_watt = cpu_watt_raw;
                        d.gpu_temp = gpu.temp;
                        d.gpu_watt = gpu.watt;
                        d.media_procs = gpu.media_procs;
                        d.compute_procs = gpu.compute_procs;
                        d.gpu_kind = gpu.kind;
                        d.vram_used_mb = gpu.vram_used_mb;
                        d.vram_total_mb = gpu.vram_total_mb;
                        d.gpu_gfx_percent = gpu.gfx_percent;
                        d.cpu_percent = cpu_percent;
                        d.ram_used_mb = ram_used_mb;
                        d.ram_total_mb = ram_total_mb;
                    }
                }

                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        });
    });
}

// ── Stil-geçişli pencere (ortak iskelet) ─────────────────────────────────────

fn build_switchable_ui(app: &Application, initial: GuiStyle) {
    let window = ApplicationWindow::builder()
        .application(app)
        .default_width(340)
        .default_height(1)
        .decorated(false)
        .build();

    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Right, true);
    window.set_margin(Edge::Top, 60);
    window.set_margin(Edge::Right, 20);
    window.set_keyboard_mode(KeyboardMode::None);

    let css = CssProvider::new();
    css.load_from_data(PANEL_CSS);
    gtk4::style_context_add_provider_for_display(
        &gtk4::gdk::Display::default().unwrap(),
        &css,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // Tek sensör thread'i + tek paylaşılan veri.
    let data = Arc::new(Mutex::new(SensorData::default()));
    spawn_sensor_thread(data.clone());

    // Kart: sabit üst kontrol (stil toggle) + stack içerik + paylaşılan process bölümü.
    let card = GtkBox::new(Orientation::Vertical, 8);
    card.add_css_class("panel-card");
    card.set_size_request(340, -1);

    // Üst kontrol çubuğu: stil toggle (her iki stilde aynı konumda, sabit).
    let header = GtkBox::new(Orientation::Horizontal, 0);
    let style_toggle = make_style_toggle(&initial.label());
    header.append(&style_toggle);
    card.append(&header);

    // İçerikler bir kez inşa edilir.
    let (classic_root, classic_update) = build_classic_content();
    let (bars_root, bars_update) = build_bars_content();

    let stack = gtk4::Stack::new();
    stack.set_transition_type(gtk4::StackTransitionType::None);
    stack.set_hhomogeneous(false);
    stack.set_vhomogeneous(false);
    stack.add_named(&classic_root, Some(GuiStyle::Classic.stack_name()));
    stack.add_named(&bars_root, Some(GuiStyle::Bars.stack_name()));
    stack.set_visible_child_name(initial.stack_name());
    card.append(&stack);

    // Paylaşılan process bölümü (manuel açılır), stilden bağımsız.
    let (proc_section, proc_update) = build_proc_section();
    card.append(&proc_section);

    // Stil toggle: görünen içeriği değiştir, etiketini güncelle.
    let style = Rc::new(Cell::new(initial));
    {
        let stack = stack.clone();
        let style = style.clone();
        let style_toggle_lbl = style_toggle.clone();
        style_toggle.connect_clicked(move |_| {
            let next = style.get().next();
            style.set(next);
            stack.set_visible_child_name(next.stack_name());
            style_toggle_lbl.set_label(&next.label());
        });
    }

    window.set_child(Some(&card));

    // Sağ tık → kapat
    let gesture = gtk4::GestureClick::new();
    gesture.set_button(3);
    let win_clone = window.clone();
    gesture.connect_released(move |_, _, _, _| win_clone.close());
    window.add_controller(gesture);

    // Tekerlek → opaklık
    let opacity = Rc::new(Cell::new(1.0_f64));
    let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
    let win_op = window.clone();
    let op_cell = opacity.clone();
    scroll.connect_scroll(move |_, _, dy| {
        let new = (op_cell.get() - dy * 0.05).clamp(0.3, 1.0);
        op_cell.set(new);
        win_op.set_opacity(new);
        glib::Propagation::Stop
    });
    window.add_controller(scroll);

    window.present();

    // Tek güncelleme döngüsü her iki içeriği ve process bölümünü besler.
    glib::timeout_add_local(Duration::from_millis(1000), move || {
        let target = match data.lock() {
            Ok(d) => d.clone(),
            Err(_) => return glib::ControlFlow::Continue,
        };
        classic_update(&target);
        bars_update(&target);
        proc_update(&target);
        glib::ControlFlow::Continue
    });
}

pub(crate) fn run_gui() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(|app| build_switchable_ui(app, GuiStyle::Classic));
    app.run()
}

pub(crate) fn run_gui2(args: &[String]) -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(|app| build_switchable_ui(app, GuiStyle::Bars));
    let argv0 = args.first().map(String::as_str).unwrap_or("power_panel");
    app.run_with_args(&[argv0])
}
