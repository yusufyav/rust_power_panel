use gtk4::prelude::*;
use gtk4::{glib, Application, ApplicationWindow, Box as GtkBox, CssProvider, Label, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use nvml_wrapper::Nvml;
use std::collections::HashMap;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use sysinfo::{Components, ProcessesToUpdate, System};

const APP_ID: &str = "com.rustpanel.powerpanel";

// ── AMD fdinfo tracker ────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
struct AmdDecInfo {
    dec_procs: Vec<(String, u32)>,
    enc_procs: Vec<(String, u32)>,
}

struct FdInfoTracker {
    prev: HashMap<u64, (u64, u64, Instant)>,
    pdev: String,
    vcn_instances: u32, 
}

impl FdInfoTracker {
    fn new(pdev: String, vcn_instances: u32) -> Self {
        Self { prev: HashMap::new(), pdev, vcn_instances }
    }

    fn sample(&mut self) -> AmdDecInfo {
        let now = Instant::now();
        let mut current: HashMap<u64, (String, u64, u64, u32, u32)> = HashMap::new();

        let Ok(proc_dir) = fs::read_dir("/proc") else {
            return AmdDecInfo::default();
        };

        for entry in proc_dir.flatten() {
            let fname = entry.file_name();
            let pid_str = fname.to_string_lossy();
            let Ok(pid) = pid_str.parse::<u32>() else { continue };

            let fd_path = format!("/proc/{}/fd", pid);
            let Ok(fd_dir) = fs::read_dir(&fd_path) else { continue };

            let mut proc_name = String::new();

            for fd_entry in fd_dir.flatten() {
                let fd_num = fd_entry.file_name();
                let fdinfo_path = format!("/proc/{}/fdinfo/{}", pid, fd_num.to_string_lossy());
                let Ok(content) = fs::read_to_string(&fdinfo_path) else { continue };

                if !content.contains("amdgpu") { continue; }
                if !self.pdev.is_empty() && !content.contains(&self.pdev) { continue; }

                let mut client_id = None;
                let mut fd_dec: u64 = 0;
                let mut fd_enc: u64 = 0;
                let mut cap_dec: u32 = 0;
                let mut cap_enc: u32 = 0;

                for line in content.lines() {
                    if line.starts_with("drm-client-id:") {
                        client_id = Some(Self::parse_ns(line));
                    } else if line.starts_with("drm-engine-dec:") {
                        fd_dec += Self::parse_ns(line);
                    } else if line.starts_with("drm-engine-enc:") {
                        fd_enc += Self::parse_ns(line);
                    } else if line.starts_with("drm-engine-capacity-dec:") {
                        cap_dec = Self::parse_ns(line) as u32; 
                    } else if line.starts_with("drm-engine-capacity-enc:") {
                        cap_enc = Self::parse_ns(line) as u32;
                    }
                }

                let cid = client_id.unwrap_or(pid as u64);
                let final_cap_dec = if cap_dec > 0 { cap_dec } else { self.vcn_instances };
                let final_cap_enc = if cap_enc > 0 { cap_enc } else { self.vcn_instances };

                current.entry(cid).or_insert_with(|| {
                    if proc_name.is_empty() {
                        proc_name = fs::read_to_string(format!("/proc/{}/comm", pid))
                            .unwrap_or_default().trim().to_string();
                    }
                    (proc_name.clone(), fd_dec, fd_enc, final_cap_dec, final_cap_enc)
                });
            }
        }

        let mut dec_list: Vec<(String, u32)> = Vec::new();
        let mut enc_list: Vec<(String, u32)> = Vec::new();

        for (cid, (name, dec_ns, enc_ns, cap_dec, cap_enc)) in &current {
            if let Some(&(prev_dec, prev_enc, prev_t)) = self.prev.get(cid) {
                let elapsed = now.duration_since(prev_t).as_nanos() as u64;
                if elapsed == 0 { continue; }

                let dec_d = dec_ns.saturating_sub(prev_dec);
                let enc_d = enc_ns.saturating_sub(prev_enc);

                let dec_p = (((dec_d as f64 / elapsed as f64) * 100.0) as u32) / cap_dec;
                let enc_p = (((enc_d as f64 / elapsed as f64) * 100.0) as u32) / cap_enc;

                if dec_p > 0 { dec_list.push((name.clone(), dec_p)); }
                if enc_p > 0 { enc_list.push((name.clone(), enc_p)); }
            }
        }

        self.prev.clear();
        for (cid, (_, dec_ns, enc_ns, _, _)) in &current {
            self.prev.insert(*cid, (*dec_ns, *enc_ns, now));
        }

        dec_list.sort_by(|a, b| b.1.cmp(&a.1));
        enc_list.sort_by(|a, b| b.1.cmp(&a.1));

        AmdDecInfo {
            dec_procs: dec_list,
            enc_procs: enc_list,
        }
    }

    fn parse_ns(line: &str) -> u64 {
        line.split_whitespace()
            .nth(1)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }
}

// ── GPU backend ───────────────────────────────────────────────────────────────

enum GpuBackend {
    Nvidia(Nvml),
    Amd { hwmon_path: String, pdev: String, vcn_instances: u32 },
    None,
}

// ── Veri yapıları ─────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
struct SensorData {
    cpu_temp:     f32,
    cpu_watt:     f32,
    gpu_temp:     f32,
    gpu_watt:     f32,
    // Artık tek bir String değil, tüm uygulamaları ve yüzdelerini liste halinde tutuyoruz
    dec_procs:    Vec<(String, u32)>, 
    enc_procs:    Vec<(String, u32)>,
    gpu_kind:     GpuKind,
}

#[derive(Clone, Default, PartialEq)]
enum GpuKind { #[default] Unknown, Nvidia, Amd }

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

    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_anchor(Edge::Top,   true);
    window.set_anchor(Edge::Right, true);
    window.set_margin(Edge::Top,   60);
    window.set_margin(Edge::Right, 20);
    window.set_keyboard_mode(KeyboardMode::None);

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
            color: #00ffcc; font-family: 'JetBrains Mono', monospace;
            font-size: 26px; font-weight: bold;
        }
        .lbl-cpu {
            color: #ff9f43; font-family: 'JetBrains Mono', monospace;
            font-size: 16px; font-weight: bold;
        }
        .lbl-gpu {
            color: #2ecc71; font-family: 'JetBrains Mono', monospace;
            font-size: 16px; font-weight: bold;
        }
        .lbl-util {
            color: #a29bfe; font-family: 'JetBrains Mono', monospace;
            font-size: 16px; font-weight: bold;
        }
        .val-watt {
            color: #ffffff; font-family: 'JetBrains Mono', monospace;
            font-size: 16px;
        }
        .val-temp {
            color: #ff4757; font-family: 'JetBrains Mono', monospace;
            font-size: 16px;
        }
        .val-proc {
            color: #b2bec3; font-family: 'JetBrains Mono', monospace;
            font-size: 13px;
        }
        .val-pct {
            color: #dfe6e9; font-family: 'JetBrains Mono', monospace;
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

    let (dec_row, dec_proc_lbl, dec_pct_lbl) = make_codec_row("◈", "DEC");
    panel.append(&dec_row);

    let (enc_row, enc_proc_lbl, enc_pct_lbl) = make_codec_row("◉", "ENC");
    panel.append(&enc_row);

    window.set_child(Some(&panel));

    let gesture = gtk4::GestureClick::new();
    gesture.set_button(3);
    let win_clone = window.clone();
    gesture.connect_released(move |_, _, _, _| { win_clone.close(); });
    window.add_controller(gesture);

    window.present();

    let data        = Arc::new(Mutex::new(SensorData::default()));
    let data_writer = data.clone();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let mut comps = Components::new_with_refreshed_list();
            let mut sys   = System::new_all();

            let gpu_backend = detect_gpu();

            let mut fdinfo_tracker = match &gpu_backend {
                GpuBackend::Amd { pdev, vcn_instances, .. } => {
                    Some(FdInfoTracker::new(pdev.clone(), *vcn_instances))
                }
                _ => None,
            };

            let mut tracker = PowerTracker {
                path:        find_rapl_path(),
                last_energy: 0,
                last_time:   Instant::now(),
            };
            if let Some(p) = tracker.path {
                tracker.last_energy = read_u64(p).unwrap_or(0);
            }

            loop {
                comps.refresh(false);
                let mut cpu_temp  = 0.0f32;
                let mut found_die = false;
                for c in &comps {
                    let lbl = c.label().to_lowercase();
                    if lbl == "tdie" {
                        if let Some(t) = c.temperature() {
                            cpu_temp = t; found_die = true; break;
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

                let cpu_watt = if let Some(path) = tracker.path {
                    match read_u64(path) {
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

                let mut gpu_temp = 0.0f32;
                let mut gpu_watt = 0.0f32;
                let mut dec_procs = Vec::new();
                let mut enc_procs = Vec::new();
                let mut gpu_kind = GpuKind::Unknown;

                match &gpu_backend {
                    GpuBackend::Nvidia(nvml) => {
                        gpu_kind = GpuKind::Nvidia;
                        if let Ok(dev) = nvml.device_by_index(0) {
                            gpu_watt = dev.power_usage().unwrap_or(0) as f32 / 1000.0;
                            gpu_temp = dev.temperature(
                                nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu
                            ).unwrap_or(0) as f32;
                            
                            let total_dec = dev.decoder_utilization().map(|u| u.utilization).unwrap_or(0);
                            let total_enc = dev.encoder_utilization().map(|u| u.utilization).unwrap_or(0);

                            // Nvidia için de tüm uygulamaları listeye çeviriyoruz
                            if total_dec > 0 || total_enc > 0 {
                                sys.refresh_processes(ProcessesToUpdate::All, false);
                                if let Ok(samples) = dev.process_utilization_stats(Some(0)) {
                                    for s in samples {
                                        let mut name = String::new();
                                        if let Some(proc) = sys.process(sysinfo::Pid::from(s.pid as usize)) {
                                            name = proc.name().to_string_lossy().into_owned();
                                        }
                                        if s.dec_util > 0 { dec_procs.push((name.clone(), s.dec_util)); }
                                        if s.enc_util > 0 { enc_procs.push((name.clone(), s.enc_util)); }
                                    }
                                }
                                dec_procs.sort_by(|a, b| b.1.cmp(&a.1));
                                enc_procs.sort_by(|a, b| b.1.cmp(&a.1));
                            }
                        }
                    }

                    GpuBackend::Amd { hwmon_path, .. } => {
                        gpu_kind = GpuKind::Amd;

                        let temp_path = format!("{}/temp1_input", hwmon_path);
                        if let Ok(v) = read_u64(&temp_path) {
                            gpu_temp = v as f32 / 1000.0;
                        }
                        
                        let power_path = format!("{}/power1_average", hwmon_path);
                        if let Ok(v) = read_u64(&power_path) {
                            gpu_watt = v as f32 / 1_000_000.0;
                        }

                        if let Some(ref mut tracker) = fdinfo_tracker {
                            let info = tracker.sample();
                            dec_procs = info.dec_procs;
                            enc_procs = info.enc_procs;
                        }
                    }

                    GpuBackend::None => {}
                }

                if let Ok(mut d) = data_writer.lock() {
                    d.cpu_temp  = cpu_temp;
                    d.cpu_watt  = cpu_watt;
                    d.gpu_temp  = gpu_temp;
                    d.gpu_watt  = gpu_watt;
                    d.dec_procs = dec_procs;
                    d.enc_procs = enc_procs;
                    d.gpu_kind  = gpu_kind;
                }

                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
        });
    });

    glib::timeout_add_local(Duration::from_millis(1000), move || {
        if let Ok(d) = data.lock() {
            total_label.set_text(&format!("⚡ {:>6.1} W", d.cpu_watt + d.gpu_watt));
            cpu_watt_lbl.set_text(&format!("{:>6.1} W", d.cpu_watt));
            cpu_temp_lbl.set_text(&format!("{:>3.0} °C", d.cpu_temp.floor()));
            gpu_watt_lbl.set_text(&format!("{:>6.1} W", d.gpu_watt));
            gpu_temp_lbl.set_text(&format!("{:>5.0} °C", d.gpu_temp.floor()));

            let valid_gpu = d.gpu_kind != GpuKind::Unknown;

            // Sadece çalışan DEC işlemi varsa satırı göster
            if valid_gpu && !d.dec_procs.is_empty() {
                dec_row.set_visible(true);
                let procs_str = d.dec_procs.iter().map(|(n, _)| format!("[{}]", n)).collect::<Vec<_>>().join("\n");
                let pct_str   = d.dec_procs.iter().map(|(_, p)| format!("{:>3} %", p)).collect::<Vec<_>>().join("\n");
                dec_proc_lbl.set_text(&procs_str);
                dec_pct_lbl.set_text(&pct_str);
            } else {
                dec_row.set_visible(false);
            }

            // Sadece çalışan ENC işlemi varsa satırı göster
            if valid_gpu && !d.enc_procs.is_empty() {
                enc_row.set_visible(true);
                let procs_str = d.enc_procs.iter().map(|(n, _)| format!("[{}]", n)).collect::<Vec<_>>().join("\n");
                let pct_str   = d.enc_procs.iter().map(|(_, p)| format!("{:>3} %", p)).collect::<Vec<_>>().join("\n");
                enc_proc_lbl.set_text(&procs_str);
                enc_pct_lbl.set_text(&pct_str);
            } else {
                enc_row.set_visible(false);
            }

            // DEC veya ENC'den en az biri aktifse aradaki ince çizgiyi göster, ikisi de yoksa çizgiyi gizle
            sep.set_visible(valid_gpu && (!d.dec_procs.is_empty() || !d.enc_procs.is_empty()));
        }
        glib::ControlFlow::Continue
    });
}

fn detect_gpu() -> GpuBackend {
    if let Ok(nvml) = Nvml::init() {
        if nvml.device_by_index(0).is_ok() {
            return GpuBackend::Nvidia(nvml);
        }
    }

    for card_idx in 0..8u32 {
        let vendor_path = format!("/sys/class/drm/card{}/device/vendor", card_idx);
        if let Ok(vendor) = fs::read_to_string(&vendor_path) {
            if vendor.trim() != "0x1002" { continue; }

            let pdev_path = format!("/sys/class/drm/card{}/device/uevent", card_idx);
            let pdev = fs::read_to_string(&pdev_path)
                .unwrap_or_default()
                .lines()
                .find(|l| l.starts_with("PCI_SLOT_NAME="))
                .map(|l| l.trim_start_matches("PCI_SLOT_NAME=").to_lowercase())
                .unwrap_or_default();

            let vcn_instances = 2; 

            let hwmon_base = format!("/sys/class/drm/card{}/device/hwmon", card_idx);
            if let Ok(entries) = fs::read_dir(&hwmon_base) {
                for entry in entries.flatten() {
                    let hwmon_path = entry.path().to_string_lossy().into_owned();
                    let has_temp  = fs::metadata(format!("{}/temp1_input", hwmon_path)).is_ok();
                    let has_power = fs::metadata(format!("{}/power1_average", hwmon_path)).is_ok();
                    if has_temp || has_power {
                        return GpuBackend::Amd { hwmon_path, pdev, vcn_instances };
                    }
                }
            }
        }
    }

    GpuBackend::None
}

fn find_rapl_path() -> Option<&'static str> {
    const CANDIDATES: &[&str] = &[
        "/sys/class/powercap/intel-rapl:0/energy_uj",
        "/sys/class/powercap/intel-rapl/intel-rapl:0/energy_uj",
        "/sys/class/powercap/amd-energy-pkg/energy_uj",
        "/sys/class/powercap/amd_energy/energy1_input",
    ];
    for &p in CANDIDATES {
        if fs::metadata(p).is_ok() { return Some(p); }
    }
    None
}

fn read_u64(path: &str) -> Result<u64, std::io::Error> {
    let s = fs::read_to_string(path)?;
    s.trim().parse::<u64>().map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

fn make_hw_row(icon: &str, name: &str, cls: &str) -> (GtkBox, Label, Label) {
    let row = GtkBox::new(Orientation::Horizontal, 0);
    let lbl_icon = Label::builder().label(icon).css_classes(vec![cls.to_string()])
        .width_chars(3).xalign(0.0).build();
    let lbl_name = Label::builder().label(name).css_classes(vec![cls.to_string()])
        .width_chars(4).xalign(0.0).build();
    let lbl_watt = Label::builder().label("   0.0 W")
        .css_classes(vec!["val-watt".to_string()])
        .width_chars(8).xalign(1.0).build();
    let lbl_therm = Label::builder().label(" 🌡")
        .css_classes(vec!["val-temp".to_string()])
        .width_chars(2).xalign(1.0).build();
    let lbl_temp = Label::builder().label("  0 °C")
        .css_classes(vec!["val-temp".to_string()])
        .width_chars(7).xalign(1.0).build();
    row.append(&lbl_icon); row.append(&lbl_name); row.append(&lbl_watt);
    row.append(&lbl_therm); row.append(&lbl_temp);
    (row, lbl_watt, lbl_temp)
}

// Kritik dokunuş: Çok satırlı (multiline) yazıldığında ikonun ve isimlerin hep üstte (Start) kalması için
// .valign(gtk4::Align::Start) parametresi eklendi. Böylece alt satıra inildiğinde ortalanma bozulmayacak.
fn make_codec_row(icon: &str, name: &str) -> (GtkBox, Label, Label) {
    let row = GtkBox::new(Orientation::Horizontal, 0);
    let lbl_icon = Label::builder().label(icon)
        .css_classes(vec!["lbl-util".to_string()])
        .width_chars(3).xalign(0.0).valign(gtk4::Align::Start).build();
    let lbl_name = Label::builder().label(name)
        .css_classes(vec!["lbl-util".to_string()])
        .width_chars(4).xalign(0.0).valign(gtk4::Align::Start).build();
    
    // .ellipsize kaldırıldı, artık metin sığmayınca taşacak veya alt alta eklenecek
    let lbl_proc = Label::builder().label("")
        .css_classes(vec!["val-proc".to_string()])
        .hexpand(true).xalign(0.5).valign(gtk4::Align::Start).build();
        
    let lbl_pct = Label::builder().label("  0 %")
        .css_classes(vec!["val-pct".to_string()])
        .width_chars(6).xalign(1.0).valign(gtk4::Align::Start).build();
        
    row.append(&lbl_icon); row.append(&lbl_name);
    row.append(&lbl_proc); row.append(&lbl_pct);
    (row, lbl_proc, lbl_pct)
}