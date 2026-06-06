use super::style::{temp_css_class, usage_css_class};
use super::Content;
use crate::types::{usage_percent, GpuKind, SensorData};
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Label, Orientation};

// ── Classic içeriği ─────────────────────────────────────────────────────────

pub(super) fn build_classic_content() -> Content {
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
        cpu_temp_lbl.set_text(&format!("{:>2.0}°C ", target.cpu_temp.floor()));
        cpu_pct_lbl.set_css_classes(&[usage_css_class(target.cpu_percent)]);
        cpu_pct_lbl.set_text(&format!("●{:>3}%", target.cpu_percent));

        gpu_watt_lbl.set_text(&format!("{:>6.1} W", target.gpu_watt));
        let gpu_cls = temp_css_class(target.gpu_temp);
        gpu_therm_lbl.set_css_classes(&[gpu_cls]);
        gpu_temp_lbl.set_css_classes(&[gpu_cls]);
        gpu_temp_lbl.set_text(&format!("{:>2.0}°C ", target.gpu_temp.floor()));
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
        .css_classes(vec![cls.to_string(), "hw-icon".to_string()])
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
        .label(" 0°C ")
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
        .css_classes(vec!["lbl-gpu".to_string(), "hw-icon".to_string()])
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
        .css_classes(vec!["lbl-ram".to_string(), "hw-icon".to_string()])
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
