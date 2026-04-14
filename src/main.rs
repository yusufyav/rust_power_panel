use gtk4::prelude::*;
use gtk4::{glib, Application, ApplicationWindow, Box as GtkBox, CssProvider, Label, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use nvml_wrapper::Nvml;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use sysinfo::{Components, ProcessesToUpdate, System};

const APP_ID: &str = "com.rustpanel.powerpanel";

// ── Veri yapıları ─────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
struct SensorData {
    cpu_temp:     f32,
    cpu_watt:     f32,
    gpu_temp:     u32,
    gpu_watt:     f32,
    gpu_dec:      u32,
    gpu_enc:      u32,
    decoder_proc: String,
}

struct PowerTracker {
    last_energy: u64,
    last_time:   Instant,
    path:        Option<&'static str>,
}

// ── Giriş noktası ─────────────────────────────────────────────────────────────

fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

// ── UI ────────────────────────────────────────────────────────────────────────

fn build_ui(app: &Application) {
    let window = ApplicationWindow::builder()
        .application(app)
        .default_width(340)
        .default_height(1)
        .decorated(false)
        .build();

    // Wayland layer-shell: sağ üst köşe, her zaman üstte
    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_anchor(Edge::Top,   true);
    window.set_anchor(Edge::Right, true);
    window.set_margin(Edge::Top,   60);
    window.set_margin(Edge::Right, 20);

    // ALT+TAB DÜZELTME: Panel klavye odağı almaz, compositor onu görmezden gelir.
    // KeyboardMode::None → Alt+Tab sırasında panel kaybolmaz, focus değişmez.
    window.set_keyboard_mode(KeyboardMode::None);

    // CSS ─────────────────────────────────────────────────────────────────────
    let css = CssProvider::new();
    css.load_from_data("
        window { background-color: transparent; }
        .panel {
            background-color: rgba(10, 10, 10, 0.80);
            border-radius: 18px;
            border: 1px solid rgba(255, 255, 255, 0.15);
            padding: 18px 24px;
        }
        .total-watt {
            color: #00ffcc;
            font-family: 'JetBrains Mono', monospace;
            font-size: 26px; font-weight: bold;
        }
        .lbl-cpu {
            color: #ff9f43;
            font-family: 'JetBrains Mono', monospace;
            font-size: 16px; font-weight: bold;
        }
        .lbl-gpu {
            color: #2ecc71;
            font-family: 'JetBrains Mono', monospace;
            font-size: 16px; font-weight: bold;
        }
        .lbl-util {
            color: #a29bfe;
            font-family: 'JetBrains Mono', monospace;
            font-size: 16px; font-weight: bold;
        }
        .val-watt {
            color: #ffffff;
            font-family: 'JetBrains Mono', monospace;
            font-size: 16px;
        }
        .val-temp {
            color: #ff4757;
            font-family: 'JetBrains Mono', monospace;
            font-size: 16px;
        }
        .val-proc {
            color: #b2bec3;
            font-family: 'JetBrains Mono', monospace;
            font-size: 13px;
        }
        .val-pct {
            color: #dfe6e9;
            font-family: 'JetBrains Mono', monospace;
            font-size: 16px;
        }
        .divider {
            background-color: rgba(255, 255, 255, 0.10);
            min-height: 1px; margin: 4px 0px;
        }
    ");
    gtk4::style_context_add_provider_for_display(
        &gtk4::gdk::Display::default().unwrap(),
        &css,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // Panel ───────────────────────────────────────────────────────────────────
    let panel = GtkBox::new(Orientation::Vertical, 8);
    panel.add_css_class("panel");
    panel.set_size_request(340, -1);

    let total_label = Label::new(Some("⚡    0.0 W"));
    total_label.add_css_class("total-watt");
    total_label.set_halign(gtk4::Align::Center);
    panel.append(&total_label);

    let (cpu_row, cpu_watt_lbl, cpu_temp_lbl) = make_hw_row("⚙", "CPU", "lbl-cpu");
    panel.append(&cpu_row);

    let (gpu_row, gpu_watt_lbl, gpu_temp_lbl) = make_hw_row("▣", "GPU", "lbl-gpu");
    panel.append(&gpu_row);

    let sep = gtk4::Separator::new(Orientation::Horizontal);
    sep.add_css_class("divider");
    panel.append(&sep);

    let (dec_row, dec_proc_lbl, dec_pct_lbl) = make_dec_row();
    panel.append(&dec_row);

    let (enc_row, enc_pct_lbl) = make_enc_row();
    panel.append(&enc_row);

    window.set_child(Some(&panel));

    // Sağ tık → kapat
    let gesture = gtk4::GestureClick::new();
    gesture.set_button(3);
    let win_clone = window.clone();
    gesture.connect_released(move |_, _, _, _| { win_clone.close(); });
    window.add_controller(gesture);

    window.present();

    // ── Sensör thread'i ───────────────────────────────────────────────────────
    let data        = Arc::new(Mutex::new(SensorData::default()));
    let data_writer = data.clone();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let nvml      = Nvml::init().ok();
            // sysinfo 0.38: Components bağımsız struct
            let mut comps = Components::new_with_refreshed_list();
            let mut sys   = System::new_all();

            let mut tracker = PowerTracker {
                path:        find_rapl_path(),
                last_energy: 0,
                last_time:   Instant::now(),
            };
            if let Some(p) = tracker.path {
                tracker.last_energy = read_energy(p).unwrap_or(0);
            }

            loop {
                // ── CPU sıcaklık (btop algoritması) ──────────────────────────
                // sysinfo 0.38: refresh() → bool parametresi alır (keep_removed)
                comps.refresh(false);

                // Önce Tdie ara (gerçek die sıcaklığı), sonra Tctl/k10 fallback
                let mut cpu_temp  = 0.0f32;
                let mut found_die = false;
                for c in &comps {
                    let lbl = c.label().to_lowercase();
                    if lbl == "tdie" {
                        // sysinfo 0.38: temperature() → Option<f32>
                        if let Some(t) = c.temperature() {
                            cpu_temp  = t;
                            found_die = true;
                            break;
                        }
                    }
                }
                if !found_die {
                    for c in &comps {
                        let lbl = c.label().to_lowercase();
                        if lbl == "tctl" || lbl.contains("k10") || lbl.contains("composite") {
                            if let Some(t) = c.temperature() {
                                if t > cpu_temp { cpu_temp = t; }
                            }
                        }
                    }
                }

                // ── CPU güç (RAPL diferansiyel) ───────────────────────────────
                let cpu_watt = if let Some(path) = tracker.path {
                    match read_energy(path) {
                        Ok(current) => {
                            let now     = Instant::now();
                            let elapsed = now.duration_since(tracker.last_time).as_secs_f32();
                            let watts   = if elapsed > 0.1 {
                                let diff = current.saturating_sub(tracker.last_energy);
                                (diff as f32 / elapsed) / 1_000_000.0
                            } else { 0.0 };
                            tracker.last_energy = current;
                            tracker.last_time   = now;
                            if watts > 1.0 && watts < 400.0 { watts } else { 0.0 }
                        }
                        Err(_) => 0.0,
                    }
                } else { 0.0 };

                // ── GPU verileri ──────────────────────────────────────────────
                let mut gpu_watt     = 0.0f32;
                let mut gpu_temp     = 0u32;
                let mut gpu_dec      = 0u32;
                let mut gpu_enc      = 0u32;
                let mut decoder_proc = String::new();

                if let Some(ref n) = nvml {
                    if let Ok(dev) = n.device_by_index(0) {
                        gpu_watt = dev.power_usage().unwrap_or(0) as f32 / 1000.0;
                        gpu_temp = dev.temperature(
                            nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu
                        ).unwrap_or(0);
                        gpu_dec = dev.decoder_utilization()
                            .map(|u| u.utilization).unwrap_or(0);
                        gpu_enc = dev.encoder_utilization()
                            .map(|u| u.utilization).unwrap_or(0);

                        if gpu_dec > 0 {
                            // sysinfo 0.38: refresh_processes(ProcessesToUpdate, bool)
                            sys.refresh_processes(ProcessesToUpdate::All, false);
                            if let Ok(samples) = dev.process_utilization_stats(Some(0)) {
                                if let Some(best) = samples.iter()
                                    .filter(|s| s.dec_util > 0)
                                    .max_by_key(|s| s.dec_util)
                                {
                                    if let Some(proc) = sys.process(
                                        sysinfo::Pid::from(best.pid as usize)
                                    ) {
                                        decoder_proc = proc.name().to_string_lossy().into_owned();
                                    }
                                }
                            }
                        }
                    }
                }

                if let Ok(mut d) = data_writer.lock() {
                    d.cpu_temp     = cpu_temp;
                    d.cpu_watt     = cpu_watt;
                    d.gpu_temp     = gpu_temp;
                    d.gpu_watt     = gpu_watt;
                    d.gpu_dec      = gpu_dec;
                    d.gpu_enc      = gpu_enc;
                    d.decoder_proc = decoder_proc;
                }

                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
        });
    });

    // ── GTK timer: her saniye UI güncelle ─────────────────────────────────────
    glib::timeout_add_local(Duration::from_millis(1000), move || {
        if let Ok(d) = data.lock() {
            total_label.set_text(
                &format!("⚡ {:>6.1} W", d.cpu_watt + d.gpu_watt)
            );
            cpu_watt_lbl.set_text(&format!("{:>6.1} W", d.cpu_watt));
            // Ondalık kısmı kaldır, yuvarlamadan (floor = aşağı kes)
            cpu_temp_lbl.set_text(&format!("{:>3.0} °C", d.cpu_temp.floor()));
            gpu_watt_lbl.set_text(&format!("{:>6.1} W", d.gpu_watt));
            gpu_temp_lbl.set_text(&format!("{:>3} °C",  d.gpu_temp));

            dec_proc_lbl.set_text(
                if d.decoder_proc.is_empty() { "" } else { &d.decoder_proc }
            );
            dec_pct_lbl.set_text(&format!("{:>3} %", d.gpu_dec));
            enc_pct_lbl.set_text(&format!("{:>3} %", d.gpu_enc));
        }
        glib::ControlFlow::Continue
    });
}

// ── Yardımcı fonksiyonlar ─────────────────────────────────────────────────────

fn find_rapl_path() -> Option<&'static str> {
    const CANDIDATES: &[&str] = &[
        "/sys/class/powercap/intel-rapl:0/energy_uj",
        "/sys/class/powercap/intel-rapl/intel-rapl:0/energy_uj",
        "/sys/class/powercap/amd-energy-pkg/energy_uj",
        "/sys/class/powercap/amd_energy/energy1_input",
    ];
    for &p in CANDIDATES {
        if fs::metadata(p).is_ok() {
            return Some(p);
        }
    }
    None
}

fn read_energy(path: &str) -> Result<u64, std::io::Error> {
    let s = fs::read_to_string(path)?;
    s.trim().parse::<u64>()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// CPU / GPU satırı: [ikon 3ch][ad 4ch][watt 8ch][🌡 2ch][temp 7ch]
fn make_hw_row(icon: &str, name: &str, cls: &str) -> (GtkBox, Label, Label) {
    let row = GtkBox::new(Orientation::Horizontal, 0);

    let lbl_icon = Label::builder()
        .label(icon).css_classes(vec![cls.to_string()])
        .width_chars(3).xalign(0.0).build();

    let lbl_name = Label::builder()
        .label(name).css_classes(vec![cls.to_string()])
        .width_chars(4).xalign(0.0).build();

    let lbl_watt = Label::builder()
        .label("   0.0 W").css_classes(vec!["val-watt".to_string()])
        .width_chars(8).xalign(1.0).build();

    let lbl_therm = Label::builder()
        .label(" 🌡").css_classes(vec!["val-temp".to_string()])
        .width_chars(2).xalign(1.0).build();

    let lbl_temp = Label::builder()
        .label("  0 °C").css_classes(vec!["val-temp".to_string()])
        .width_chars(7).xalign(1.0).build();

    row.append(&lbl_icon);
    row.append(&lbl_name);
    row.append(&lbl_watt);
    row.append(&lbl_therm);
    row.append(&lbl_temp);
    (row, lbl_watt, lbl_temp)
}

/// DEC satırı: [◈ 3ch][DEC 4ch][process adı — genişler, kırpar][yüzde 5ch]
fn make_dec_row() -> (GtkBox, Label, Label) {
    let row = GtkBox::new(Orientation::Horizontal, 0);

    let lbl_icon = Label::builder()
        .label("◈").css_classes(vec!["lbl-util".to_string()])
        .width_chars(3).xalign(0.0).build();

    let lbl_name = Label::builder()
        .label("DEC").css_classes(vec!["lbl-util".to_string()])
        .width_chars(4).xalign(0.0).build();

    let lbl_proc = Label::builder()
        .label("").css_classes(vec!["val-proc".to_string()])
        .hexpand(true).xalign(0.5)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();

    let lbl_pct = Label::builder()
        .label("  0 %").css_classes(vec!["val-pct".to_string()])
        .width_chars(5).xalign(1.0).build();

    row.append(&lbl_icon);
    row.append(&lbl_name);
    row.append(&lbl_proc);
    row.append(&lbl_pct);
    (row, lbl_proc, lbl_pct)
}

/// ENC satırı: [◉ 3ch][ENC 4ch][boşluk — genişler][yüzde 5ch]
fn make_enc_row() -> (GtkBox, Label) {
    let row = GtkBox::new(Orientation::Horizontal, 0);

    let lbl_icon = Label::builder()
        .label("◉").css_classes(vec!["lbl-util".to_string()])
        .width_chars(3).xalign(0.0).build();

    let lbl_name = Label::builder()
        .label("ENC").css_classes(vec!["lbl-util".to_string()])
        .width_chars(4).xalign(0.0).build();

    let spacer = Label::builder().label("").hexpand(true).build();

    let lbl_pct = Label::builder()
        .label("  0 %").css_classes(vec!["val-pct".to_string()])
        .width_chars(5).xalign(1.0).build();

    row.append(&lbl_icon);
    row.append(&lbl_name);
    row.append(&spacer);
    row.append(&lbl_pct);
    (row, lbl_pct)
}