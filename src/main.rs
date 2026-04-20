use gtk4::prelude::*;
use gtk4::{glib, Application, ApplicationWindow, Box as GtkBox, CssProvider, Label, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use nvml_wrapper::Nvml;
use std::collections::HashMap;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use sysinfo::{Components, ProcessesToUpdate, System};

const APP_ID: &str = "com.github.yusufyav.rust_power_panel";

// ── AMD fdinfo tracker ────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
struct AmdDecInfo {
    media_procs: Vec<(String, u32, u32)>,
}

struct FdInfoTracker {
    prev: HashMap<u64, (u64, u64, Instant)>,
    pdev: String,
    vcn_instances: u32,
}

impl FdInfoTracker {
    fn new(pdev: String, vcn_instances: u32) -> Self {
        Self {
            prev: HashMap::new(),
            pdev,
            vcn_instances,
        }
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
            let Ok(pid) = pid_str.parse::<u32>() else {
                continue;
            };

            let fd_path = format!("/proc/{}/fd", pid);
            let Ok(fd_dir) = fs::read_dir(&fd_path) else {
                continue;
            };

            let mut proc_name = String::new();

            for fd_entry in fd_dir.flatten() {
                let fd_num = fd_entry.file_name();
                let fdinfo_path = format!("/proc/{}/fdinfo/{}", pid, fd_num.to_string_lossy());
                let Ok(content) = fs::read_to_string(&fdinfo_path) else {
                    continue;
                };

                if !content.contains("amdgpu") {
                    continue;
                }
                if !self.pdev.is_empty() && !content.contains(&self.pdev) {
                    continue;
                }

                let mut client_id = None;
                let mut fd_dec: u64 = 0;
                let mut fd_enc: u64 = 0;
                let mut cap_dec: u32 = 0;
                let mut cap_enc: u32 = 0;

                for line in content.lines() {
                    if line.starts_with("drm-client-id:") {
                        client_id = Some(Self::parse_ns(line));
                    } else if line.starts_with("drm-engine-dec:") {
                        fd_dec = fd_dec.max(Self::parse_ns(line));
                    } else if line.starts_with("drm-engine-enc:") {
                        fd_enc = fd_enc.max(Self::parse_ns(line));
                    } else if line.starts_with("drm-engine-capacity-dec:") {
                        cap_dec = Self::parse_ns(line) as u32;
                    } else if line.starts_with("drm-engine-capacity-enc:") {
                        cap_enc = Self::parse_ns(line) as u32;
                    }
                }

                let cid = client_id.unwrap_or(pid as u64);
                let final_cap_dec = if cap_dec > 0 {
                    cap_dec
                } else {
                    self.vcn_instances
                };
                let final_cap_enc = if cap_enc > 0 {
                    cap_enc
                } else {
                    self.vcn_instances
                };

                current
                    .entry(cid)
                    .and_modify(|e| {
                        e.1 = e.1.max(fd_dec);
                        e.2 = e.2.max(fd_enc);
                    })
                    .or_insert_with(|| {
                        if proc_name.is_empty() {
                            proc_name = fs::read_to_string(format!("/proc/{}/comm", pid))
                                .unwrap_or_default()
                                .trim()
                                .to_string();
                        }
                        (
                            proc_name.clone(),
                            fd_dec,
                            fd_enc,
                            final_cap_dec,
                            final_cap_enc,
                        )
                    });
            }
        }

        let mut media_list: Vec<(String, u32, u32)> = Vec::new();

        for (cid, (name, dec_ns, enc_ns, cap_dec, cap_enc)) in &current {
            if let Some(&(prev_dec, prev_enc, prev_t)) = self.prev.get(cid) {
                let elapsed = now.duration_since(prev_t).as_nanos() as u64;
                if elapsed == 0 {
                    continue;
                }

                let dec_d = dec_ns.saturating_sub(prev_dec);
                let enc_d = enc_ns.saturating_sub(prev_enc);

                let dec_p = (((dec_d as f64 / elapsed as f64) * 100.0) as u32) / cap_dec;
                let enc_p = (((enc_d as f64 / elapsed as f64) * 100.0) as u32) / cap_enc;

                if dec_p > 0 || enc_p > 0 {
                    media_list.push((name.clone(), dec_p, enc_p));
                }
            }
        }

        self.prev.clear();
        for (cid, (_, dec_ns, enc_ns, _, _)) in &current {
            self.prev.insert(*cid, (*dec_ns, *enc_ns, now));
        }

        media_list.sort_by(|a, b| (b.1 + b.2).cmp(&(a.1 + a.2)));

        AmdDecInfo {
            media_procs: media_list,
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
    Nvidia(Box<Nvml>),
    Amd {
        hwmon_path: String,
        pdev: String,
        vcn_instances: u32,
    },
    // Intel GPU (i915/xe sürücüsü)
    // Güç: hwmon energy1_input (joule, sadece ayrık DG1/DG2/ArcGPU'larda mevcut)
    // Sıcaklık: hwmon temp1_input (kernel 6.10+ ile bazı kartlarda)
    // Entegre Intel GPU'da ikisi de mevcut olmayabilir → değerler 0 gösterilir
    Intel {
        hwmon_path: Option<String>, // None = entegre/desteklenmez
    },
    None,
}

// ── Veri yapıları ─────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
struct SensorData {
    cpu_temp: f32,
    cpu_watt: f32,
    gpu_temp: f32,
    gpu_watt: f32,
    media_procs: Vec<(String, u32, u32)>,
    gpu_kind: GpuKind,
}

#[derive(Clone, Default, PartialEq)]
enum GpuKind {
    #[default]
    Unknown,
    Nvidia,
    Amd,
    Intel,
}

struct PowerTracker {
    last_energy: u64,
    last_time: Instant,
    path: Option<&'static str>,
}

struct GpuPowerTracker {
    last_energy: u64,
    last_time: Instant,
}

// ── Giriş noktası ─────────────────────────────────────────────────────────────

fn main() -> glib::ExitCode {
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

// ── DIAGNOSTICS (TEŞHİS) MOTORU V4 (Sudo Korumalı) ───────────────
fn run_diagnostics(rapl_path: &Option<&'static str>, gpu: &GpuBackend) {
    println!("\n=== 🔍 POWERPANEL DIAGNOSTICS V4 ===");

    if let Some(path) = rapl_path {
        match read_u64(path) {
            Ok(_) => println!("✅ CPU Power : OK -> {}", path),
            Err(e) => println!("❌ CPU Power : FAIL -> {} (Hata: {})", path, e),
        }
    } else {
        println!("⚠️  CPU Power : NOT FOUND");
    }

    println!("\n-- Çekirdekteki Tüm Donanım Sensörleri (/sys/class/hwmon) --");
    let mut hwmon_count = 0;
    if let Ok(entries) = fs::read_dir("/sys/class/hwmon") {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(name) = fs::read_to_string(path.join("name")) {
                let name = name.trim();
                let mut temp_val = "Yok/Okunamıyor".to_string();

                for i in 1..=4 {
                    if let Ok(val) = fs::read_to_string(path.join(format!("temp{}_input", i))) {
                        if let Ok(num) = val.trim().parse::<f32>() {
                            let label = fs::read_to_string(path.join(format!("temp{}_label", i)))
                                .unwrap_or_default();
                            temp_val =
                                format!("{:.1} °C (temp{} - {})", num / 1000.0, i, label.trim());
                            break;
                        }
                    }
                }

                println!(
                    "   🏷️ İsim: {:<12} | 📁 Yol: {} | 🌡️ Sıcaklık: {}",
                    name,
                    path.display(),
                    temp_val
                );
                hwmon_count += 1;
            }
        }
    }

    if hwmon_count == 0 {
        println!("❌ Çekirdekte hiçbir donanım sensörü bulunamadı!");
        println!("🚨 DİKKAT: Büyük ihtimalle 'sudo' kullanmadınız. Sensörler yetkisiz kullanıcılara kapalı!");
    }

    println!("\n-- Seçilen Ana CPU Sensörü --");
    if let Some(path) = detect_cpu_temp_path() {
        println!("✅ Panel bunu kullanacak: {}", path);
    } else {
        println!("❌ Uygun bir CPU sensörü eşleştirilemedi.");
    }

    println!("\n-- GPU Durumu --");
    match gpu {
        GpuBackend::Intel { hwmon_path } => {
            println!("✅ GPU Type  : Intel (i915/xe)");
            match hwmon_path {
                Some(p) => println!("   hwmon     : {}", p),
                None    => println!("   hwmon     : Yok (entegre GPU veya desteklenmiyor)"),
            }
        }
        GpuBackend::Amd {
            hwmon_path,
            vcn_instances,
            ..
        } => {
            println!("✅ GPU Type  : AMD (VCN Instances: {})", vcn_instances);
            println!("✅ GPU HWMon : {}", hwmon_path);
        }
        GpuBackend::Nvidia(_) => println!("✅ GPU Type  : NVIDIA (NVML Initialized)"),
        GpuBackend::None => println!("❌ GPU Type  : NOT FOUND (Desteklenmeyen kart)"),
    }
    println!("======================================\n");
}

// ── KERNEL'DEN DİREKT CPU SICAKLIK OKUYUCU ─────────────
fn detect_cpu_temp_path() -> Option<String> {
    let base = "/sys/class/hwmon";
    let Ok(entries) = fs::read_dir(base) else {
        return None;
    };

    let mut best_path = None;
    let mut best_score = 0;

    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(name) = fs::read_to_string(path.join("name")) {
            let name = name.trim().to_lowercase();

            // SKORLAMA SİSTEMİ (k10temp ve asusec eklendi)
            let score = match name.as_str() {
                "k10temp" => 100,
                "zenpower" => 90,
                "asusec" => 85, // Senin anakartının özel sensörü
                "nct6775" | "nct6687" => 80,
                "coretemp" => 70,
                "acpitz" => 50,
                "asus" | "wmi" => 40,
                _ => continue,
            };

            if score > best_score {
                let mut target_file = path.join("temp1_input");

                for i in 1..=10 {
                    let label_path = path.join(format!("temp{}_label", i));
                    if let Ok(label) = fs::read_to_string(&label_path) {
                        let label_lower = label.trim().to_lowercase();
                        // asusec "CPU", k10temp "Tctl" yazar. İkisini de kapsadık!
                        if label_lower.contains("tdie")
                            || label_lower.contains("tctl")
                            || label_lower.contains("package")
                            || label_lower.contains("cpu")
                        {
                            target_file = path.join(format!("temp{}_input", i));
                            break;
                        }
                    }
                }

                if fs::metadata(&target_file).is_ok() {
                    best_path = Some(target_file.to_string_lossy().into_owned());
                    best_score = score;
                }
            }
        }
    }

    best_path
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
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Right, true);
    window.set_margin(Edge::Top, 60);
    window.set_margin(Edge::Right, 20);
    window.set_keyboard_mode(KeyboardMode::None);

    let css = CssProvider::new();
    css.load_from_data(
        "
        window { background-color: transparent; }
        .panel {
            background-color: rgba(10, 10, 10, 0.80);
            border-radius: 18px;
            border: 1px solid rgba(255, 255, 255, 0.15);
            padding: 18px 24px;
        }
        .total-watt {
            color: #00ffcc; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 26px; font-weight: bold;
        }
        .lbl-cpu {
            color: #ff9f43; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 16px; font-weight: bold;
        }
        .lbl-gpu {
            color: #2ecc71; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 16px; font-weight: bold;
        }
        .lbl-util {
            color: #a29bfe; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 16px; font-weight: bold;
        }
        .val-watt {
            color: #ffffff; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 16px;
        }
        .val-temp {
            color: #ff4757; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 16px;
        }
        .val-proc {
            color: #b2bec3; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 13px;
        }
        .val-pct {
            color: #dfe6e9; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 16px;
        }
        .divider {
            background-color: rgba(255, 255, 255, 0.10);
            min-height: 1px; margin: 4px 0px;
        }
    ",
    );
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

    let (cpu_row, cpu_watt_lbl, cpu_temp_lbl) = make_hw_row("", "CPU", "lbl-cpu");
    panel.append(&cpu_row);

    let (gpu_row, gpu_watt_lbl, gpu_temp_lbl) = make_hw_row("󰢮", "GPU", "lbl-gpu");
    panel.append(&gpu_row);

    let sep = gtk4::Separator::new(Orientation::Horizontal);
    sep.add_css_class("divider");
    sep.set_visible(false);
    panel.append(&sep);

    let (media_container, media_proc_lbl, media_dec_lbl, media_enc_lbl) = make_media_section();
    media_container.set_visible(false);
    panel.append(&media_container);

    window.set_child(Some(&panel));

    let gesture = gtk4::GestureClick::new();
    gesture.set_button(3);
    let win_clone = window.clone();
    gesture.connect_released(move |_, _, _, _| {
        win_clone.close();
    });
    window.add_controller(gesture);

    window.present();

    let data = Arc::new(Mutex::new(SensorData::default()));
    let data_writer = data.clone();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let mut comps = Components::new_with_refreshed_list();
            let mut sys = System::new_all();

            let gpu_backend = detect_gpu();

            let mut intel_gpu_tracker: Option<GpuPowerTracker> = None;
            let mut fdinfo_tracker = match &gpu_backend {
                GpuBackend::Amd {
                    pdev,
                    vcn_instances,
                    ..
                } => Some(FdInfoTracker::new(pdev.clone(), *vcn_instances)),
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
            run_diagnostics(&tracker.path, &gpu_backend);

            loop {
                comps.refresh(false);
                let mut cpu_temp = 0.0f32;
                let mut found_die = false;
                for c in &comps {
                    let lbl = c.label().to_lowercase();
                    if lbl == "tdie" {
                        if let Some(t) = c.temperature() {
                            cpu_temp = t;
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
                                if t > cpu_temp {
                                    cpu_temp = t;
                                }
                            }
                        }
                    }
                }

                let cpu_watt = if let Some(path) = tracker.path {
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

                let mut gpu_temp = 0.0f32;
                let mut gpu_watt = 0.0f32;
                let mut media_procs: Vec<(String, u32, u32)> = Vec::new();
                let mut gpu_kind = GpuKind::Unknown;

                match &gpu_backend {
                    GpuBackend::Nvidia(nvml) => {
                        gpu_kind = GpuKind::Nvidia;
                        if let Ok(dev) = nvml.device_by_index(0) {
                            gpu_watt = dev.power_usage().unwrap_or(0) as f32 / 1000.0;
                            gpu_temp = dev
                                .temperature(
                                    nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu,
                                )
                                .unwrap_or(0) as f32;

                            let total_dec =
                                dev.decoder_utilization().map(|u| u.utilization).unwrap_or(0);
                            let total_enc =
                                dev.encoder_utilization().map(|u| u.utilization).unwrap_or(0);

                            if total_dec > 0 || total_enc > 0 {
                                sys.refresh_processes(ProcessesToUpdate::All, false);
                                if let Ok(samples) = dev.process_utilization_stats(Some(0)) {
                                    let mut proc_map: HashMap<u32, (u32, u32)> = HashMap::new();
                                    for s in samples {
                                        if s.dec_util > 0 || s.enc_util > 0 {
                                            proc_map
                                                .entry(s.pid)
                                                .and_modify(|e| {
                                                    e.0 = e.0.max(s.dec_util);
                                                    e.1 = e.1.max(s.enc_util);
                                                })
                                                .or_insert((s.dec_util, s.enc_util));
                                        }
                                    }
                                    for (pid, (dec, enc)) in proc_map {
                                        let name = sys
                                            .process(sysinfo::Pid::from(pid as usize))
                                            .map(|p| p.name().to_string_lossy().into_owned())
                                            .unwrap_or_else(|| format!("pid:{}", pid));
                                        media_procs.push((name, dec, enc));
                                    }
                                    media_procs.sort_by(|a, b| (b.1 + b.2).cmp(&(a.1 + a.2)));
                                }
                            }
                        }
                    }

                    GpuBackend::Amd { hwmon_path, .. } => {
                        gpu_kind = GpuKind::Amd;

                        if let Ok(v) = read_u64(&format!("{}/temp1_input", hwmon_path)) {
                            gpu_temp = v as f32 / 1000.0;
                        }
                        if let Ok(v) = read_u64(&format!("{}/power1_average", hwmon_path)) {
                            gpu_watt = v as f32 / 1_000_000.0;
                        }
                        if let Some(ref mut tracker) = fdinfo_tracker {
                            let info = tracker.sample();
                            media_procs = info.media_procs;
                        }
                    }

                    GpuBackend::Intel { hwmon_path } => {
                        gpu_kind = GpuKind::Intel;

                        // Sıcaklık: temp1_input (kernel 6.10+ ayrık Arc GPU'larda)
                        // Entegre Intel GPU'da mevcut değil → 0 gösterilir
                        if let Some(ref path) = hwmon_path {
                            if let Ok(v) = read_u64(&format!("{}/temp1_input", path)) {
                                gpu_temp = v as f32 / 1000.0;
                            }
                            // Güç: energy1_input (joule kümülatif) → delta/dt = Watt
                            // Sadece ayrık Arc/DG1/DG2 GPU'larda mevcut
                            let energy_path = format!("{}/energy1_input", path);
                            if let Ok(current_e) = read_u64(&energy_path) {
                                let now_t = Instant::now();
                                if let Some(ref mut gpt) = intel_gpu_tracker {
                                    let elapsed = now_t
                                        .duration_since(gpt.last_time)
                                        .as_secs_f32();
                                    if elapsed > 0.1 {
                                        let delta = current_e.saturating_sub(gpt.last_energy);
                                        // energy1_input microjoule cinsinden
                                        let w = delta as f32 / elapsed / 1_000_000.0;
                                        if w > 0.5 && w < 300.0 {
                                            gpu_watt = w;
                                        }
                                    }
                                    gpt.last_energy = current_e;
                                    gpt.last_time   = now_t;
                                } else {
                                    intel_gpu_tracker = Some(GpuPowerTracker {
                                        last_energy: current_e,
                                        last_time:   now_t,
                                    });
                                }
                            }
                        }
                        // Intel GPU için DEC/ENC: fdinfo'da "i915" sürücüsü
                        // drm-engine-video satırı mevcut kernellerde var
                        // Şimdilik media_procs boş — ilerleyen versiyonda eklenebilir
                    }

                    GpuBackend::None => {}
                }

                if let Ok(mut d) = data_writer.lock() {
                    d.cpu_temp = cpu_temp;
                    d.cpu_watt = cpu_watt;
                    d.gpu_temp = gpu_temp;
                    d.gpu_watt = gpu_watt;
                    d.media_procs = media_procs;
                    d.gpu_kind = gpu_kind;
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
            gpu_temp_lbl.set_text(&format!("{:>3.0} °C", d.gpu_temp.floor()));

            let valid_gpu = d.gpu_kind != GpuKind::Unknown;

            if valid_gpu && !d.media_procs.is_empty() {
                media_container.set_visible(true);
                let procs_str = d
                    .media_procs
                    .iter()
                    .map(|(n, _, _)| {
                        let text = n.chars().take(14).collect::<String>();
                        if n.chars().count() > 14 {
                            format!("{}…", text)
                        } else {
                            text
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let dec_str = d
                    .media_procs
                    .iter()
                    .map(|(_, dec, _)| format!("{:>3} %", dec))
                    .collect::<Vec<_>>()
                    .join("\n");
                let enc_str = d
                    .media_procs
                    .iter()
                    .map(|(_, _, enc)| format!("{:>3} %", enc))
                    .collect::<Vec<_>>()
                    .join("\n");

                media_proc_lbl.set_text(&procs_str);
                media_dec_lbl.set_text(&dec_str);
                media_enc_lbl.set_text(&enc_str);
            } else {
                media_container.set_visible(false);
            }

            sep.set_visible(valid_gpu && !d.media_procs.is_empty());
        }
        glib::ControlFlow::Continue
    });
}

fn get_vcn_instances(card_idx: u32) -> u32 {
    let base_path = format!(
        "/sys/class/drm/card{}/device/ip_discovery/die/0/VCN",
        card_idx
    );

    let paths_to_check = [
        format!("{}/0/num_inst", base_path),
        format!("{}/num_inst", base_path),
    ];

    for path in &paths_to_check {
        if let Ok(content) = fs::read_to_string(path) {
            if let Ok(count) = content.trim().parse::<u32>() {
                if count > 0 {
                    return count;
                }
            }
        }
    }

    if let Ok(entries) = fs::read_dir(&base_path) {
        let count = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .count() as u32;
        if count > 0 {
            return count;
        }
    }

    1
}

fn detect_gpu() -> GpuBackend {
    // 1. Nvidia: NVML
    if let Ok(nvml) = Nvml::init() {
        if nvml.device_by_index(0).is_ok() {
            return GpuBackend::Nvidia(Box::new(nvml));
        }
    }

    for card_idx in 0..8u32 {
        let vendor_path = format!("/sys/class/drm/card{}/device/vendor", card_idx);
        let Ok(vendor) = fs::read_to_string(&vendor_path) else { continue; };
        let vendor = vendor.trim();

        // 2. AMD: vendor 0x1002
        if vendor == "0x1002" {
            let pdev_path = format!("/sys/class/drm/card{}/device/uevent", card_idx);
            let pdev = fs::read_to_string(&pdev_path)
                .unwrap_or_default()
                .lines()
                .find(|l| l.starts_with("PCI_SLOT_NAME="))
                .map(|l| l.trim_start_matches("PCI_SLOT_NAME=").to_lowercase())
                .unwrap_or_default();

            let vcn_instances = get_vcn_instances(card_idx);
            let hwmon_base = format!("/sys/class/drm/card{}/device/hwmon", card_idx);
            if let Ok(entries) = fs::read_dir(&hwmon_base) {
                for entry in entries.flatten() {
                    let hwmon_path = entry.path().to_string_lossy().into_owned();
                    let has_temp  = fs::metadata(format!("{}/temp1_input",   hwmon_path)).is_ok();
                    let has_power = fs::metadata(format!("{}/power1_average", hwmon_path)).is_ok();
                    if has_temp || has_power {
                        return GpuBackend::Amd { hwmon_path, pdev, vcn_instances };
                    }
                }
            }
        }

        // 3. Intel: vendor 0x8086, sürücü i915 veya xe
        if vendor == "0x8086" {
            // Sürücü kontrolü: i915 veya xe olmalı
            let driver_path = format!("/sys/class/drm/card{}/device/driver", card_idx);
            let driver_name = fs::read_link(&driver_path)
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_default();

            if !matches!(driver_name.as_str(), "i915" | "xe") {
                continue;
            }

            // hwmon yolunu bul — entegre GPU'da olmayabilir
            let hwmon_base = format!("/sys/class/drm/card{}/device/hwmon", card_idx);
            let hwmon_path = fs::read_dir(&hwmon_base)
                .ok()
                .and_then(|mut entries| entries.next())
                .and_then(|e| e.ok())
                .map(|e| e.path().to_string_lossy().into_owned());

            return GpuBackend::Intel { hwmon_path };
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
    CANDIDATES
        .iter()
        .copied()
        .find(|&p| fs::metadata(p).is_ok())
}

fn read_u64(path: &str) -> Result<u64, std::io::Error> {
    let s = fs::read_to_string(path)?;
    s.trim()
        .parse::<u64>()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

fn make_hw_row(icon: &str, name: &str, cls: &str) -> (GtkBox, Label, Label) {
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
        .label(" ")
        .css_classes(vec!["val-temp".to_string()])
        .width_chars(3)
        .xalign(1.0)
        .build();
    let lbl_temp = Label::builder()
        .label("  0 °C")
        .css_classes(vec!["val-temp".to_string()])
        .width_chars(6)
        .xalign(1.0)
        .build();
    row.append(&lbl_icon);
    row.append(&lbl_name);
    row.append(&lbl_watt);
    row.append(&lbl_therm);
    row.append(&lbl_temp);
    (row, lbl_watt, lbl_temp)
}

fn make_media_section() -> (GtkBox, Label, Label, Label) {
    let container = GtkBox::new(Orientation::Vertical, 4);

    let header_row = GtkBox::new(Orientation::Horizontal, 0);
    let lbl_name = Label::builder()
        .label("Name")
        .css_classes(vec!["lbl-util".to_string()])
        .hexpand(true)
        .xalign(0.0)
        .build();
    let lbl_dec_hdr = Label::builder()
        .label("DEC")
        .css_classes(vec!["lbl-util".to_string()])
        .width_chars(6)
        .xalign(1.0)
        .build();
    let lbl_enc_hdr = Label::builder()
        .label("ENC")
        .css_classes(vec!["lbl-util".to_string()])
        .width_chars(6)
        .xalign(1.0)
        .build();
    header_row.append(&lbl_name);
    header_row.append(&lbl_dec_hdr);
    header_row.append(&lbl_enc_hdr);

    let data_row = GtkBox::new(Orientation::Horizontal, 0);
    let lbl_proc = Label::builder()
        .css_classes(vec!["val-proc".to_string()])
        .hexpand(true)
        .xalign(0.0)
        .valign(gtk4::Align::Start)
        .max_width_chars(16)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    let lbl_dec = Label::builder()
        .css_classes(vec!["val-pct".to_string()])
        .width_chars(6)
        .xalign(1.0)
        .valign(gtk4::Align::Start)
        .build();
    let lbl_enc = Label::builder()
        .css_classes(vec!["val-pct".to_string()])
        .width_chars(6)
        .xalign(1.0)
        .valign(gtk4::Align::Start)
        .build();
    data_row.append(&lbl_proc);
    data_row.append(&lbl_dec);
    data_row.append(&lbl_enc);

    container.append(&header_row);
    container.append(&data_row);

    (container, lbl_proc, lbl_dec, lbl_enc)
}