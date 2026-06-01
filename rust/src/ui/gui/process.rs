use super::Content;
use crate::types::{CombinedProc, GpuKind, SensorData};
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Grid, Label, Orientation};
use std::cell::Cell;
use std::rc::Rc;

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
pub(super) fn build_proc_section() -> Content {
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
