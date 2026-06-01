use super::super::tui::fmt_gb;
use super::style::temp_hex_color;
use super::Content;
use crate::types::{usage_percent, GpuKind, SensorData};
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Label, Orientation};
use std::cell::Cell;
use std::rc::Rc;

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

pub(super) fn build_bars_content() -> Content {
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
