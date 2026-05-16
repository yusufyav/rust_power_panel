use gtk4::prelude::*;
use gtk4::{glib, Application, ApplicationWindow, Box as GtkBox, CssProvider, Label, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use nvml_wrapper::Nvml;
use std::collections::HashMap;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use sysinfo::{Components, ProcessesToUpdate, System};

const APP_ID: &str = "com.github.yusufyav.power_panel";

fn parse_fdinfo_ns(line: &str) -> u64 {
    line.split_whitespace()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

// ── AMD fdinfo tracker ────────────────────────────────────────────────────────

#[derive(Debug, Default, Clone)]
struct MediaInfo {
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

    fn sample(&mut self) -> MediaInfo {
        let now = Instant::now();
        let mut current: HashMap<u64, (String, u64, u64, u32, u32)> = HashMap::new();

        let Ok(proc_dir) = fs::read_dir("/proc") else {
            return MediaInfo::default();
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
                        client_id = Some(parse_fdinfo_ns(line));
                    } else if line.starts_with("drm-engine-dec:") {
                        fd_dec = fd_dec.max(parse_fdinfo_ns(line));
                    } else if line.starts_with("drm-engine-enc:") {
                        fd_enc = fd_enc.max(parse_fdinfo_ns(line));
                    } else if line.starts_with("drm-engine-capacity-dec:") {
                        cap_dec = parse_fdinfo_ns(line) as u32;
                    } else if line.starts_with("drm-engine-capacity-enc:") {
                        cap_enc = parse_fdinfo_ns(line) as u32;
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

        MediaInfo {
            media_procs: media_list,
        }
    }
}

// ── Intel fdinfo tracker ──────────────────────────────────────────────────────

struct IntelFdInfoTracker {
    prev: HashMap<u64, (u64, Instant)>,
}

impl IntelFdInfoTracker {
    fn new() -> Self {
        Self {
            prev: HashMap::new(),
        }
    }

    fn sample(&mut self) -> MediaInfo {
        let now = Instant::now();
        let mut current: HashMap<u64, (String, u64)> = HashMap::new();

        let Ok(proc_dir) = fs::read_dir("/proc") else {
            return MediaInfo::default();
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

                // Intel GPU: i915 veya xe sürücüsü
                if !content.contains("i915") && !content.contains("xe") {
                    continue;
                }

                let mut client_id = None;
                let mut video_ns: u64 = 0;

                for line in content.lines() {
                    if line.starts_with("drm-client-id:") {
                        client_id = Some(parse_fdinfo_ns(line));
                    } else if line.starts_with("drm-engine-video:") {
                        // Intel'de video engine decode+encode toplamını verir
                        video_ns = video_ns.max(parse_fdinfo_ns(line));
                    }
                }

                if video_ns == 0 {
                    continue;
                }

                let cid = client_id.unwrap_or(pid as u64);

                current
                    .entry(cid)
                    .and_modify(|e| {
                        e.1 = e.1.max(video_ns);
                    })
                    .or_insert_with(|| {
                        if proc_name.is_empty() {
                            proc_name = fs::read_to_string(format!("/proc/{}/comm", pid))
                                .unwrap_or_default()
                                .trim()
                                .to_string();
                        }
                        (proc_name.clone(), video_ns)
                    });
            }
        }

        let mut media_list: Vec<(String, u32, u32)> = Vec::new();

        for (cid, (name, video_ns)) in &current {
            if let Some(&(prev_video, prev_t)) = self.prev.get(cid) {
                let elapsed = now.duration_since(prev_t).as_nanos() as u64;
                if elapsed == 0 {
                    continue;
                }

                let video_d = video_ns.saturating_sub(prev_video);
                let video_p = ((video_d as f64 / elapsed as f64) * 100.0) as u32;

                if video_p > 0 {
                    // Intel'de ayrı dec/enc yok, toplamı "DEC" sütununda göster
                    media_list.push((name.clone(), video_p, 0));
                }
            }
        }

        self.prev.clear();
        for (cid, (_, video_ns)) in &current {
            self.prev.insert(*cid, (*video_ns, now));
        }

        media_list.sort_by(|a, b| b.1.cmp(&a.1));

        MediaInfo {
            media_procs: media_list,
        }
    }
}

// ── GPU backend ───────────────────────────────────────────────────────────────

enum GpuBackend {
    Nvidia(Box<Nvml>),
    Amd {
        hwmon_path: String,
        pdev: String,
        vcn_instances: u32,
        device_path: String,
    },
    // Intel GPU (i915/xe sürücüsü)
    // Güç: RAPL uncore domain (entegre iGPU) veya hwmon energy1_input (ayrık Arc)
    // Sıcaklık: hwmon temp1_input (kernel 6.10+ ile bazı kartlarda)
    Intel {
        hwmon_path: Option<String>,      // Arc GPU için hwmon
        rapl_uncore_path: Option<String>, // Entegre iGPU için RAPL uncore
    },
    None,
}

// ── Veri yapıları ─────────────────────────────────────────────────────────────

struct GpuData {
    temp: f32,
    watt: f32,
    media_procs: Vec<(String, u32, u32)>,
    compute_procs: Vec<(String, u32)>,
    kind: GpuKind,
    vram_used_mb: u32,
    vram_total_mb: u32,
    gfx_percent: u32,
}

impl Default for GpuData {
    fn default() -> Self {
        Self {
            temp: 0.0,
            watt: 0.0,
            media_procs: Vec::new(),
            compute_procs: Vec::new(),
            kind: GpuKind::default(),
            vram_used_mb: 0,
            vram_total_mb: 0,
            gfx_percent: 0,
        }
    }
}

#[derive(Clone, Default)]
struct SensorData {
    cpu_temp: f32,
    cpu_watt: f32,
    gpu_temp: f32,
    gpu_watt: f32,
    media_procs: Vec<(String, u32, u32)>,
    compute_procs: Vec<(String, u32)>,
    gpu_kind: GpuKind,
    vram_used_mb: u32,
    vram_total_mb: u32,
    gpu_gfx_percent: u32,
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
    let args: Vec<String> = std::env::args().collect();

    // CLI modu kontrolü
    if args.len() > 1 {
        match args[1].as_str() {
            "--help" | "-h" => {
                print_help();
                return glib::ExitCode::SUCCESS;
            }
            "--cli" => {
                run_cli_mode();
                return glib::ExitCode::SUCCESS;
            }
            "--debug" => {
                let rapl_path = find_rapl_path();
                let gpu = detect_gpu();
                run_diagnostics(&rapl_path, &gpu);
                return glib::ExitCode::SUCCESS;
            }
            "--version" | "-v" => {
                println!("PowerPanel v0.1.0");
                println!("Minimal power monitoring tool for Linux");
                return glib::ExitCode::SUCCESS;
            }
            _ => {
                eprintln!("❌ Bilinmeyen parametre: {}", args[1]);
                eprintln!("Yardım için: {} --help", args[0]);
                return glib::ExitCode::FAILURE;
            }
        }
    }

    // GUI modu (varsayılan)
    let app = Application::builder().application_id(APP_ID).build();
    app.connect_activate(build_ui);
    app.run()
}

fn print_help() {
    println!("PowerPanel - Minimal Linux Güç İzleme Aracı");
    println!();
    println!("KULLANIM:");
    println!("  power_panel [SEÇENEKLER]");
    println!();
    println!("SEÇENEKLER:");
    println!("  --help, -h       Bu yardım mesajını gösterir");
    println!("  --version, -v    Versiyon bilgisini gösterir");
    println!("  --cli            CLI (Terminal) modunda çalıştır");
    println!("  --debug          Sensör teşhisini çalıştır ve çık");
    println!();
    println!("ÖRNEKLER:");
    println!("  power_panel              # GUI modunda çalıştır (varsayılan)");
    println!("  power_panel --cli        # Terminal modunda sürekli güncelleme");
    println!("  power_panel --debug      # Sensör erişimini ve GPU durumunu kontrol et");
    println!();
    println!("ÖZELLİKLER:");
    println!("  • CPU/GPU güç tüketimi ve sıcaklık");
    println!("  • GPU decode/encode kullanımı");
    println!("  • AMD, Intel, Nvidia desteği");
    println!("  • Düşük kaynak kullanımı (<10 MB RAM)");
}

fn cli_row(plain: &str, colored: &str, w: usize) -> String {
    let pad = w.saturating_sub(plain.chars().count());
    format!("\x1B[2m│\x1B[0m {}{} \x1B[2m│\x1B[0m", colored, " ".repeat(pad))
}

fn cli_temp_color(t: f32) -> &'static str {
    if t >= 80.0 { "\x1B[91m" } else if t >= 60.0 { "\x1B[93m" } else { "\x1B[92m" }
}

fn cli_titled_sep(title: &str, w: usize) -> String {
    let inner = w + 2;
    let prefix = format!("─── {} ", title);
    let plen = prefix.chars().count();
    let remaining = inner.saturating_sub(plen);
    format!("\x1B[2m├{}{}┤\x1B[0m", prefix, "─".repeat(remaining))
}

fn render_cli_frame(cpu_watt: f32, cpu_temp: f32, gpu: &GpuData) {
    use std::io::{self, Write};

    const W: usize = 38; // inner content width (between flanking spaces inside │)

    // ANSI
    const R:  &str = "\x1B[0m";
    const BD: &str = "\x1B[1m";
    const DM: &str = "\x1B[2m";
    const CY: &str = "\x1B[96m";
    const YL: &str = "\x1B[93m";
    const GN: &str = "\x1B[92m";
    const WH: &str = "\x1B[97m";
    const BL: &str = "\x1B[94m";
    const PR: &str = "\x1B[95m";

    let top = format!("{DM}┌{}┐{R}", "─".repeat(W + 2));
    let mid = format!("{DM}├{}┤{R}", "─".repeat(W + 2));
    let bot = format!("{DM}└{}┘{R}", "─".repeat(W + 2));

    let total = cpu_watt + gpu.watt;

    print!("\x1B[2J\x1B[H");

    // ── Title ─────────────────────────────────────────────────────────────────
    // "PowerPanel" 10 + 21 spaces + "{:6.1}W" 7 = 38
    let title_p = format!("PowerPanel{:<21}{:6.1}W", "", total);
    let title_r = format!("{BD}{CY}PowerPanel{R}{:<21}{BD}{WH}{:6.1}W{R}", "", total);
    println!("{top}");
    println!("{}", cli_row(&title_p, &title_r, W));
    println!("{mid}");

    // ── CPU ───────────────────────────────────────────────────────────────────
    // "CPU  " 5 + "{:6.1}W" 7 + "   " 3 + "{:3.0}°C" 5 = 20, pad 18
    let cpu_tc = cli_temp_color(cpu_temp);
    let cpu_p = format!("CPU  {:6.1}W   {:3.0}°C", cpu_watt, cpu_temp.floor());
    let cpu_r = format!("{YL}CPU{R}  {WH}{:6.1}W{R}   {cpu_tc}{:3.0}°C{R}", cpu_watt, cpu_temp.floor());
    println!("{}", cli_row(&cpu_p, &cpu_r, W));

    // ── GPU ───────────────────────────────────────────────────────────────────
    let gpu_tc = cli_temp_color(gpu.temp);
    let gpu_p = format!("GPU  {:6.1}W   {:3.0}°C", gpu.watt, gpu.temp.floor());
    let gpu_r = format!("{GN}GPU{R}  {WH}{:6.1}W{R}   {gpu_tc}{:3.0}°C{R}", gpu.watt, gpu.temp.floor());
    println!("{}", cli_row(&gpu_p, &gpu_r, W));

    // ── VRAM + GFX ────────────────────────────────────────────────────────────
    if gpu.vram_total_mb > 0 {
        // "VRAM  " 6 + "{:5}/{:5} MB" 13 + "  GFX " 6 + "{:3}%" 4 = 29, pad 9
        let vram_p = format!("VRAM  {:5}/{:5} MB  GFX {:3}%",
            gpu.vram_used_mb, gpu.vram_total_mb, gpu.gfx_percent);
        let vram_r = format!("{DM}VRAM{R}  {BL}{:5}/{:5} MB{R}  {DM}GFX{R} {WH}{:3}%{R}",
            gpu.vram_used_mb, gpu.vram_total_mb, gpu.gfx_percent);
        println!("{}", cli_row(&vram_p, &vram_r, W));
    }

    // ── Video section ─────────────────────────────────────────────────────────
    if !gpu.media_procs.is_empty() {
        println!("{}", cli_titled_sep("Video", W));
        let hdr_p = format!("{:<18} {:>5} {:>5}", "Process", "DEC", "ENC");
        let hdr_r = format!("{DM}{:<18} {:>5} {:>5}{R}", "Process", "DEC", "ENC");
        println!("{}", cli_row(&hdr_p, &hdr_r, W));
        for (name, dec, enc) in gpu.media_procs.iter().take(4) {
            let name_t: String = if name.chars().count() > 15 {
                format!("{}…", name.chars().take(14).collect::<String>())
            } else {
                name.clone()
            };
            let proc_p = format!("  {:<16} {:>4}% {:>4}%", name_t, dec, enc);
            let proc_r = format!("  {PR}{:<16}{R} {WH}{:>4}% {:>4}%{R}", name_t, dec, enc);
            println!("{}", cli_row(&proc_p, &proc_r, W));
        }
    }

    // ── CUDA section ──────────────────────────────────────────────────────────
    if !gpu.compute_procs.is_empty() {
        println!("{}", cli_titled_sep("CUDA", W));
        let has_sm = gpu.compute_procs.iter().any(|(_, sm)| *sm > 0);
        if has_sm {
            let hdr_p = format!("{:<32} {:>5}", "CUDA", "SM%");
            let hdr_r = format!("{DM}{:<32} {:>5}{R}", "CUDA", "SM%");
            println!("{}", cli_row(&hdr_p, &hdr_r, W));
            for (name, sm) in gpu.compute_procs.iter().take(4) {
                let name_t: String = if name.chars().count() > 15 {
                    format!("{}…", name.chars().take(14).collect::<String>())
                } else {
                    name.clone()
                };
                let proc_p = format!("  {:<30} {:>4}%", name_t, sm);
                let proc_r = format!("  {PR}{:<30}{R} {WH}{:>4}%{R}", name_t, sm);
                println!("{}", cli_row(&proc_p, &proc_r, W));
            }
        } else {
            for (name, _) in gpu.compute_procs.iter().take(4) {
                let name_t: String = if name.chars().count() > 15 {
                    format!("{}…", name.chars().take(14).collect::<String>())
                } else {
                    name.clone()
                };
                let proc_p = format!("  {name_t}");
                let proc_r = format!("  {PR}{name_t}{R}");
                println!("{}", cli_row(&proc_p, &proc_r, W));
            }
        }
    }

    println!("{bot}");
    io::stdout().flush().unwrap();
}

fn run_cli_mode() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let gpu_backend = detect_gpu();
        let mut sys = System::new();

        let mut intel_gpu_tracker: Option<GpuPowerTracker> = None;
        let mut amd_fdinfo_tracker = match &gpu_backend {
            GpuBackend::Amd { pdev, vcn_instances, .. } => {
                Some(FdInfoTracker::new(pdev.clone(), *vcn_instances))
            }
            _ => None,
        };
        let mut intel_fdinfo_tracker = match &gpu_backend {
            GpuBackend::Intel { .. } => Some(IntelFdInfoTracker::new()),
            _ => None,
        };

        let mut cpu_tracker = PowerTracker {
            path: find_rapl_path(),
            last_energy: 0,
            last_time: Instant::now(),
        };
        if let Some(p) = cpu_tracker.path {
            cpu_tracker.last_energy = read_u64(p).unwrap_or(0);
        }

        let cpu_temp_path = detect_cpu_temp_path();

        loop {
            let cpu_temp = if let Some(ref path) = cpu_temp_path {
                read_u64(path).map(|v| v as f32 / 1000.0).unwrap_or(0.0)
            } else {
                0.0
            };

            let cpu_watt = if let Some(path) = cpu_tracker.path {
                match read_u64(path) {
                    Ok(current) => {
                        let now = Instant::now();
                        let elapsed = now.duration_since(cpu_tracker.last_time).as_secs_f32();
                        let watts = if elapsed > 0.1 {
                            let diff = current.saturating_sub(cpu_tracker.last_energy);
                            (diff as f32 / elapsed) / 1_000_000.0
                        } else {
                            0.0
                        };
                        cpu_tracker.last_energy = current;
                        cpu_tracker.last_time = now;
                        if watts > 1.0 && watts < 400.0 { watts } else { 0.0 }
                    }
                    Err(_) => 0.0,
                }
            } else {
                0.0
            };

            let gpu = read_gpu_data(
                &gpu_backend,
                &mut sys,
                &mut intel_gpu_tracker,
                &mut amd_fdinfo_tracker,
                &mut intel_fdinfo_tracker,
            );

            render_cli_frame(cpu_watt, cpu_temp, &gpu);

            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    });
}

fn read_gpu_data(
    gpu_backend: &GpuBackend,
    sys: &mut System,
    intel_gpu_tracker: &mut Option<GpuPowerTracker>,
    amd_fdinfo_tracker: &mut Option<FdInfoTracker>,
    intel_fdinfo_tracker: &mut Option<IntelFdInfoTracker>,
) -> GpuData {
    let mut data = GpuData::default();

    match gpu_backend {
        GpuBackend::Nvidia(nvml) => {
            data.kind = GpuKind::Nvidia;
            if let Ok(dev) = nvml.device_by_index(0) {
                data.watt = dev.power_usage().unwrap_or(0) as f32 / 1000.0;
                data.temp = dev
                    .temperature(nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu)
                    .unwrap_or(0) as f32;

                if let Ok(mem) = dev.memory_info() {
                    data.vram_used_mb = (mem.used / 1_048_576) as u32;
                    data.vram_total_mb = (mem.total / 1_048_576) as u32;
                }

                // CUDA processes: running_compute_processes is CUDA-specific (not graphics/WebGPU)
                let cuda_pids: std::collections::HashSet<u32> = dev
                    .running_compute_processes()
                    .map(|v| v.into_iter().map(|p| p.pid).collect())
                    .unwrap_or_default();

                let mut codec_map: HashMap<u32, (u32, u32)> = HashMap::new();
                let mut sm_by_pid: HashMap<u32, u32> = HashMap::new();

                if let Ok(samples) = dev.process_utilization_stats(Some(0)) {
                    if !samples.is_empty() {
                        let sm_sum: u32 = samples.iter().map(|s| s.sm_util).sum();
                        data.gfx_percent = sm_sum.min(100);

                        for s in &samples {
                            if s.dec_util > 0 || s.enc_util > 0 {
                                codec_map
                                    .entry(s.pid)
                                    .and_modify(|e| {
                                        e.0 = e.0.max(s.dec_util);
                                        e.1 = e.1.max(s.enc_util);
                                    })
                                    .or_insert((s.dec_util, s.enc_util));
                            }
                            if s.sm_util > 0 {
                                sm_by_pid
                                    .entry(s.pid)
                                    .and_modify(|e| *e = (*e).max(s.sm_util))
                                    .or_insert(s.sm_util);
                            }
                        }
                    }
                }

                let needs_names = !codec_map.is_empty() || !cuda_pids.is_empty();
                if needs_names {
                    sys.refresh_processes(ProcessesToUpdate::All, false);
                }

                // Codec → media_procs (DEC/ENC), independent of CUDA
                for (pid, (dec, enc)) in codec_map {
                    let name = sys
                        .process(sysinfo::Pid::from(pid as usize))
                        .map(|p| p.name().to_string_lossy().into_owned())
                        .unwrap_or_else(|| format!("pid:{}", pid));
                    data.media_procs.push((name, dec, enc));
                }
                data.media_procs.sort_by(|a, b| (b.1 + b.2).cmp(&(a.1 + a.2)));

                // CUDA → compute_procs (SM% if active, 0 if model loaded but idle)
                for pid in &cuda_pids {
                    let sm = sm_by_pid.get(pid).copied().unwrap_or(0);
                    let name = sys
                        .process(sysinfo::Pid::from(*pid as usize))
                        .map(|p| p.name().to_string_lossy().into_owned())
                        .unwrap_or_else(|| format!("pid:{}", pid));
                    data.compute_procs.push((name, sm));
                }
                data.compute_procs.sort_by(|a, b| b.1.cmp(&a.1));

                // Fallback: utilization_rates() if process stats returned nothing
                if data.gfx_percent == 0 {
                    if let Ok(util) = dev.utilization_rates() {
                        data.gfx_percent = util.gpu;
                    }
                }
            }
        }
        GpuBackend::Amd { hwmon_path, device_path, .. } => {
            data.kind = GpuKind::Amd;
            if let Ok(v) = read_u64(&format!("{}/temp1_input", hwmon_path)) {
                data.temp = v as f32 / 1000.0;
            }
            if let Ok(v) = read_u64(&format!("{}/power1_average", hwmon_path)) {
                data.watt = v as f32 / 1_000_000.0;
            }
            if let Ok(used) = read_u64(&format!("{}/mem_info_vram_used", device_path)) {
                data.vram_used_mb = (used / 1_048_576) as u32;
            }
            if let Ok(total) = read_u64(&format!("{}/mem_info_vram_total", device_path)) {
                data.vram_total_mb = (total / 1_048_576) as u32;
            }
            if let Ok(gfx) = read_u64(&format!("{}/gpu_busy_percent", device_path)) {
                data.gfx_percent = gfx as u32;
            }
            if let Some(tracker) = amd_fdinfo_tracker {
                let info = tracker.sample();
                data.media_procs = info.media_procs;
            }
        }
        GpuBackend::Intel { hwmon_path, rapl_uncore_path } => {
            data.kind = GpuKind::Intel;
            if let Some(ref path) = hwmon_path {
                if let Ok(v) = read_u64(&format!("{}/temp1_input", path)) {
                    data.temp = v as f32 / 1000.0;
                }
            }

            if let Some(ref rapl_path) = rapl_uncore_path {
                if let Ok(current_e) = read_u64(rapl_path) {
                    let now_t = Instant::now();
                    if let Some(ref mut gpt) = intel_gpu_tracker {
                        let elapsed = now_t.duration_since(gpt.last_time).as_secs_f32();
                        if elapsed > 0.1 {
                            let delta = current_e.saturating_sub(gpt.last_energy);
                            let w = delta as f32 / elapsed / 1_000_000.0;
                            if w > 0.1 && w < 100.0 {
                                data.watt = w;
                            }
                        }
                        gpt.last_energy = current_e;
                        gpt.last_time = now_t;
                    } else {
                        *intel_gpu_tracker = Some(GpuPowerTracker {
                            last_energy: current_e,
                            last_time: now_t,
                        });
                    }
                }
            } else if let Some(ref path) = hwmon_path {
                let energy_path = format!("{}/energy1_input", path);
                if let Ok(current_e) = read_u64(&energy_path) {
                    let now_t = Instant::now();
                    if let Some(ref mut gpt) = intel_gpu_tracker {
                        let elapsed = now_t.duration_since(gpt.last_time).as_secs_f32();
                        if elapsed > 0.1 {
                            let delta = current_e.saturating_sub(gpt.last_energy);
                            let w = delta as f32 / elapsed / 1_000_000.0;
                            if w > 0.5 && w < 300.0 {
                                data.watt = w;
                            }
                        }
                        gpt.last_energy = current_e;
                        gpt.last_time = now_t;
                    } else {
                        *intel_gpu_tracker = Some(GpuPowerTracker {
                            last_energy: current_e,
                            last_time: now_t,
                        });
                    }
                }
            }

            if let Some(tracker) = intel_fdinfo_tracker {
                let info = tracker.sample();
                data.media_procs = info.media_procs;
            }
        }
        GpuBackend::None => {}
    }

    data
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
        GpuBackend::Intel { hwmon_path, rapl_uncore_path } => {
            println!("✅ GPU Type  : Intel (i915/xe)");
            match hwmon_path {
                Some(p) => println!("   hwmon     : {}", p),
                None => println!("   hwmon     : Yok (entegre GPU)"),
            }
            match rapl_uncore_path {
                Some(p) => println!("   RAPL iGPU : {} (Entegre GPU güç)", p),
                None => println!("   RAPL iGPU : Yok"),
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

            // SKORLAMA SİSTEMİ (Intel coretemp yükseltildi)
            let score = match name.as_str() {
                "k10temp" => 100,
                "coretemp" => 95, // Intel CPU için öncelikli
                "zenpower" => 90,
                "asusec" => 85,
                "nct6775" | "nct6687" => 80,
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
                        // Intel: "Package id 0", AMD: "Tctl/Tdie"
                        if label_lower.contains("tdie")
                            || label_lower.contains("tctl")
                            || label_lower.contains("package id")
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
// ── UI helpers ───────────────────────────────────────────────────────────────

fn temp_css_class(temp: f32) -> &'static str {
    if temp >= 80.0 { "val-temp-hot" }
    else if temp >= 60.0 { "val-temp-warm" }
    else { "val-temp-cool" }
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
        .val-temp-cool {
            color: #4cd964; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 16px;
        }
        .val-temp-warm {
            color: #ff9f43; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 16px;
        }
        .val-temp-hot {
            color: #ff4757; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 16px;
        }
        .val-vram {
            color: #74b9ff; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 14px;
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

    let (cpu_row, cpu_watt_lbl, cpu_therm_lbl, cpu_temp_lbl) = make_hw_row("", "CPU", "lbl-cpu");
    panel.append(&cpu_row);

    let (gpu_row, gpu_watt_lbl, gpu_therm_lbl, gpu_temp_lbl) = make_hw_row("󰢮", "GPU", "lbl-gpu");
    panel.append(&gpu_row);

    let (vram_row, vram_lbl, gfx_lbl) = make_vram_row();
    vram_row.set_visible(false);
    panel.append(&vram_row);

    let sep = gtk4::Separator::new(Orientation::Horizontal);
    sep.add_css_class("divider");
    sep.set_visible(false);
    panel.append(&sep);

    let (media_container, media_proc_lbl, media_dec_lbl, media_enc_lbl) = make_media_section();
    media_container.set_visible(false);
    panel.append(&media_container);

    let compute_sep = gtk4::Separator::new(Orientation::Horizontal);
    compute_sep.add_css_class("divider");
    compute_sep.set_visible(false);
    panel.append(&compute_sep);

    let (compute_container, compute_proc_lbl, compute_sm_lbl, compute_sm_hdr) = make_compute_section();
    compute_container.set_visible(false);
    panel.append(&compute_container);

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
            let mut cpu_watt_ema: f32 = 0.0;
            let mut gpu_watt_ema: f32 = 0.0;

            loop {
                loop_count += 1;

                // Quick GFX sample every 200ms — catches bursts that 1s polling misses
                match &gpu_backend {
                    GpuBackend::Nvidia(nvml) => {
                        if let Ok(dev) = nvml.device_by_index(0) {
                            if let Ok(util) = dev.utilization_rates() {
                                gfx_max = gfx_max.max(util.gpu);
                            }
                            if let Ok(samples) = dev.process_utilization_stats(Some(0)) {
                                let sm: u32 = samples.iter()
                                    .map(|s| s.sm_util)
                                    .sum::<u32>()
                                    .min(100);
                                gfx_max = gfx_max.max(sm);
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
                                        if t > temp { temp = t; }
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
                                if watts > 1.0 && watts < 400.0 { watts } else { 0.0 }
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

                    // EMA smoothing: initialize directly on first non-zero reading to avoid cold-start lag
                    cpu_watt_ema = if cpu_watt_ema == 0.0 && cpu_watt_raw > 0.0 {
                        cpu_watt_raw
                    } else {
                        cpu_watt_ema * 0.8 + cpu_watt_raw * 0.2
                    };
                    gpu_watt_ema = if gpu_watt_ema == 0.0 && gpu.watt > 0.0 {
                        gpu.watt
                    } else {
                        gpu_watt_ema * 0.8 + gpu.watt * 0.2
                    };

                    if let Ok(mut d) = data_writer.lock() {
                        d.cpu_temp = cpu_temp;
                        d.cpu_watt = cpu_watt_ema;
                        d.gpu_temp = gpu.temp;
                        d.gpu_watt = gpu_watt_ema;
                        d.media_procs = gpu.media_procs;
                        d.compute_procs = gpu.compute_procs;
                        d.gpu_kind = gpu.kind;
                        d.vram_used_mb = gpu.vram_used_mb;
                        d.vram_total_mb = gpu.vram_total_mb;
                        d.gpu_gfx_percent = gpu.gfx_percent;
                    }
                }

                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        });
    });

    glib::timeout_add_local(Duration::from_millis(1000), move || {
        let target = match data.lock() {
            Ok(d) => d.clone(),
            Err(_) => return glib::ControlFlow::Continue,
        };

        total_label.set_text(&format!("⚡ {:>6.1} W", target.cpu_watt + target.gpu_watt));

        cpu_watt_lbl.set_text(&format!("{:>6.1} W", target.cpu_watt));
        let cpu_cls = temp_css_class(target.cpu_temp);
        cpu_therm_lbl.set_css_classes(&[cpu_cls]);
        cpu_temp_lbl.set_css_classes(&[cpu_cls]);
        cpu_temp_lbl.set_text(&format!("{:>3.0} °C", target.cpu_temp.floor()));

        gpu_watt_lbl.set_text(&format!("{:>6.1} W", target.gpu_watt));
        let gpu_cls = temp_css_class(target.gpu_temp);
        gpu_therm_lbl.set_css_classes(&[gpu_cls]);
        gpu_temp_lbl.set_css_classes(&[gpu_cls]);
        gpu_temp_lbl.set_text(&format!("{:>3.0} °C", target.gpu_temp.floor()));

        let valid_gpu = target.gpu_kind != GpuKind::Unknown;

        if valid_gpu && target.vram_total_mb > 0 {
            vram_row.set_visible(true);
            vram_lbl.set_text(&format!(
                "{:>5}/{:>5} MB",
                target.vram_used_mb, target.vram_total_mb
            ));
            gfx_lbl.set_text(&format!("{:>3}%", target.gpu_gfx_percent));
        } else {
            vram_row.set_visible(false);
        }

        let has_media = valid_gpu && !target.media_procs.is_empty();
        if has_media {
            media_container.set_visible(true);
            let procs_str = target.media_procs.iter()
                .map(|(n, _, _)| {
                    let text = n.chars().take(14).collect::<String>();
                    if n.chars().count() > 14 { format!("{}…", text) } else { text }
                })
                .collect::<Vec<_>>()
                .join("\n");
            media_proc_lbl.set_text(&procs_str);
            let dec_str = target.media_procs.iter()
                .map(|(_, dec, _)| format!("{:>3} %", dec))
                .collect::<Vec<_>>().join("\n");
            let enc_str = target.media_procs.iter()
                .map(|(_, _, enc)| format!("{:>3} %", enc))
                .collect::<Vec<_>>().join("\n");
            media_dec_lbl.set_text(&dec_str);
            media_enc_lbl.set_text(&enc_str);
        } else {
            media_container.set_visible(false);
        }

        let has_compute = valid_gpu && !target.compute_procs.is_empty();
        if has_compute {
            compute_container.set_visible(true);
            let has_sm = target.compute_procs.iter().any(|(_, sm)| *sm > 0);
            compute_sm_hdr.set_visible(has_sm);
            compute_sm_lbl.set_visible(has_sm);
            let procs_str = target.compute_procs.iter()
                .map(|(n, _)| {
                    let text = n.chars().take(14).collect::<String>();
                    if n.chars().count() > 14 { format!("{}…", text) } else { text }
                })
                .collect::<Vec<_>>()
                .join("\n");
            compute_proc_lbl.set_text(&procs_str);
            if has_sm {
                let sm_str = target.compute_procs.iter()
                    .map(|(_, sm)| format!("{:>3} %", sm))
                    .collect::<Vec<_>>().join("\n");
                compute_sm_lbl.set_text(&sm_str);
            }
        } else {
            compute_container.set_visible(false);
        }

        compute_sep.set_visible(has_media && has_compute);
        sep.set_visible(valid_gpu && (has_media || has_compute || target.vram_total_mb > 0));
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

fn find_intel_gpu_hwmon() -> Option<String> {
    let Ok(entries) = fs::read_dir("/sys/class/hwmon") else {
        return None;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(name) = fs::read_to_string(path.join("name")) {
            let name = name.trim();
            // Intel GPU sensörleri i915 veya xe adıyla tanımlanır
            if matches!(name, "i915" | "xe") {
                return Some(path.to_string_lossy().into_owned());
            }
        }
    }
    None
}

fn find_intel_rapl_uncore() -> Option<String> {
    // Intel RAPL uncore domain (entegre iGPU güç tüketimi)
    // Genelde: /sys/class/powercap/intel-rapl/intel-rapl:0/intel-rapl:0:1/energy_uj
    // veya:     /sys/class/powercap/intel-rapl:0/intel-rapl:0:1/energy_uj
    
    let base_paths = [
        "/sys/class/powercap/intel-rapl/intel-rapl:0",
        "/sys/class/powercap/intel-rapl:0",
    ];

    for base in &base_paths {
        // Alt zone'ları tara
        if let Ok(entries) = fs::read_dir(base) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name_file = path.join("name");
                
                if let Ok(name) = fs::read_to_string(&name_file) {
                    let name_lower = name.trim().to_lowercase();
                    // uncore = iGPU güç tüketimi
                    if name_lower.contains("uncore") {
                        let energy_file = path.join("energy_uj");
                        if fs::metadata(&energy_file).is_ok() {
                            return Some(energy_file.to_string_lossy().into_owned());
                        }
                    }
                }
            }
        }
    }
    
    None
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
        let Ok(vendor) = fs::read_to_string(&vendor_path) else {
            continue;
        };
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
                    let has_temp = fs::metadata(format!("{}/temp1_input", hwmon_path)).is_ok();
                    let has_power = fs::metadata(format!("{}/power1_average", hwmon_path)).is_ok();
                    if has_temp || has_power {
                        let device_path =
                            format!("/sys/class/drm/card{}/device", card_idx);
                        return GpuBackend::Amd {
                            hwmon_path,
                            pdev,
                            vcn_instances,
                            device_path,
                        };
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

            // Intel GPU için hwmon sensörünü /sys/class/hwmon altında ara (Arc GPU)
            let hwmon_path = find_intel_gpu_hwmon();
            
            // RAPL uncore yolunu bul (entegre iGPU güç tüketimi)
            let rapl_uncore_path = find_intel_rapl_uncore();

            return GpuBackend::Intel { hwmon_path, rapl_uncore_path };
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

fn make_hw_row(icon: &str, name: &str, cls: &str) -> (GtkBox, Label, Label, Label) {
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
        .css_classes(vec!["val-temp-cool".to_string()])
        .width_chars(3)
        .xalign(1.0)
        .build();
    let lbl_temp = Label::builder()
        .label("  0 °C")
        .css_classes(vec!["val-temp-cool".to_string()])
        .width_chars(6)
        .xalign(1.0)
        .build();
    row.append(&lbl_icon);
    row.append(&lbl_name);
    row.append(&lbl_watt);
    row.append(&lbl_therm);
    row.append(&lbl_temp);
    (row, lbl_watt, lbl_therm, lbl_temp)
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
        .label("0 / 0 MB")
        .css_classes(vec!["val-vram".to_string()])
        .width_chars(14)
        .xalign(1.0)
        .build();
    let lbl_gfx = Label::builder()
        .label("  0%")
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

fn make_media_section() -> (GtkBox, Label, Label, Label) {
    let container = GtkBox::new(Orientation::Vertical, 4);

    let header_row = GtkBox::new(Orientation::Horizontal, 0);
    let lbl_name_hdr = Label::builder()
        .label("Video")
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
    header_row.append(&lbl_name_hdr);
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

fn make_compute_section() -> (GtkBox, Label, Label, Label) {
    let container = GtkBox::new(Orientation::Vertical, 4);

    let header_row = GtkBox::new(Orientation::Horizontal, 0);
    let lbl_cuda_hdr = Label::builder()
        .label("CUDA")
        .css_classes(vec!["lbl-util".to_string()])
        .hexpand(true)
        .xalign(0.0)
        .build();
    let lbl_sm_hdr = Label::builder()
        .label("SM%")
        .css_classes(vec!["lbl-util".to_string()])
        .width_chars(6)
        .xalign(1.0)
        .build();
    header_row.append(&lbl_cuda_hdr);
    header_row.append(&lbl_sm_hdr);

    let data_row = GtkBox::new(Orientation::Horizontal, 0);
    let lbl_proc = Label::builder()
        .css_classes(vec!["val-proc".to_string()])
        .hexpand(true)
        .xalign(0.0)
        .valign(gtk4::Align::Start)
        .max_width_chars(16)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();
    let lbl_sm = Label::builder()
        .css_classes(vec!["val-pct".to_string()])
        .width_chars(6)
        .xalign(1.0)
        .valign(gtk4::Align::Start)
        .build();
    data_row.append(&lbl_proc);
    data_row.append(&lbl_sm);

    container.append(&header_row);
    container.append(&data_row);

    (container, lbl_proc, lbl_sm, lbl_sm_hdr)
}