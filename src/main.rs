use gtk4::prelude::*;
use gtk4::{glib, Application, ApplicationWindow, Box as GtkBox, CssProvider, Grid, Label, Orientation};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use nvml_wrapper::Nvml;
use std::cell::Cell;
use std::collections::HashMap;
use std::fs;
use std::rc::Rc;
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
    media_procs: Vec<(String, u32, u32, u32)>, // (name, dec%, enc%, gfx%)
}

struct FdInfoTracker {
    prev: HashMap<u64, (u64, u64, u64, Instant)>, // (dec_ns, enc_ns, gfx_ns, time)
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
        let mut current: HashMap<u64, (String, u64, u64, u64, u32, u32)> = HashMap::new();

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
                let mut fd_gfx: u64 = 0;
                let mut cap_dec: u32 = 0;
                let mut cap_enc: u32 = 0;

                for line in content.lines() {
                    if line.starts_with("drm-client-id:") {
                        client_id = Some(parse_fdinfo_ns(line));
                    } else if line.starts_with("drm-engine-dec:") {
                        fd_dec = fd_dec.max(parse_fdinfo_ns(line));
                    } else if line.starts_with("drm-engine-enc:") {
                        fd_enc = fd_enc.max(parse_fdinfo_ns(line));
                    } else if line.starts_with("drm-engine-gfx:") {
                        fd_gfx = fd_gfx.max(parse_fdinfo_ns(line));
                    } else if line.starts_with("drm-engine-capacity-dec:") {
                        cap_dec = parse_fdinfo_ns(line) as u32;
                    } else if line.starts_with("drm-engine-capacity-enc:") {
                        cap_enc = parse_fdinfo_ns(line) as u32;
                    }
                }

                let cid = client_id.unwrap_or(pid as u64);
                let final_cap_dec = if cap_dec > 0 { cap_dec } else { self.vcn_instances };
                let final_cap_enc = if cap_enc > 0 { cap_enc } else { self.vcn_instances };

                current
                    .entry(cid)
                    .and_modify(|e| {
                        e.1 = e.1.max(fd_dec);
                        e.2 = e.2.max(fd_enc);
                        e.3 = e.3.max(fd_gfx);
                    })
                    .or_insert_with(|| {
                        if proc_name.is_empty() {
                            proc_name = fs::read_to_string(format!("/proc/{}/comm", pid))
                                .unwrap_or_default()
                                .trim()
                                .to_string();
                        }
                        (proc_name.clone(), fd_dec, fd_enc, fd_gfx, final_cap_dec, final_cap_enc)
                    });
            }
        }

        let mut media_list: Vec<(String, u32, u32, u32)> = Vec::new();

        for (cid, (name, dec_ns, enc_ns, gfx_ns, cap_dec, cap_enc)) in &current {
            if let Some(&(prev_dec, prev_enc, prev_gfx, prev_t)) = self.prev.get(cid) {
                let elapsed = now.duration_since(prev_t).as_nanos() as u64;
                if elapsed == 0 {
                    continue;
                }

                let dec_d = dec_ns.saturating_sub(prev_dec);
                let enc_d = enc_ns.saturating_sub(prev_enc);
                let gfx_d = gfx_ns.saturating_sub(prev_gfx);

                let dec_p = (((dec_d as f64 / elapsed as f64) * 100.0) as u32) / cap_dec;
                let enc_p = (((enc_d as f64 / elapsed as f64) * 100.0) as u32) / cap_enc;
                let gfx_p = ((gfx_d as f64 / elapsed as f64) * 100.0) as u32;

                if dec_p > 0 || enc_p > 0 || gfx_p > 0 {
                    media_list.push((name.clone(), dec_p, enc_p, gfx_p));
                }
            }
        }

        self.prev.clear();
        for (cid, (_, dec_ns, enc_ns, gfx_ns, _, _)) in &current {
            self.prev.insert(*cid, (*dec_ns, *enc_ns, *gfx_ns, now));
        }

        media_list.sort_by(|a, b| (b.1 + b.2 + b.3).cmp(&(a.1 + a.2 + a.3)));

        MediaInfo {
            media_procs: media_list,
        }
    }
}

// ── Intel fdinfo tracker ──────────────────────────────────────────────────────

struct IntelFdInfoTracker {
    prev: HashMap<u64, (u64, u64, Instant)>, // (video_ns, render_ns, time)
}

impl IntelFdInfoTracker {
    fn new() -> Self {
        Self {
            prev: HashMap::new(),
        }
    }

    fn sample(&mut self) -> MediaInfo {
        let now = Instant::now();
        let mut current: HashMap<u64, (String, u64, u64)> = HashMap::new();

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
                let mut render_ns: u64 = 0;

                for line in content.lines() {
                    if line.starts_with("drm-client-id:") {
                        client_id = Some(parse_fdinfo_ns(line));
                    } else if line.starts_with("drm-engine-video:") {
                        video_ns = video_ns.max(parse_fdinfo_ns(line));
                    } else if line.starts_with("drm-engine-render:") {
                        render_ns = render_ns.max(parse_fdinfo_ns(line));
                    }
                }

                if video_ns == 0 && render_ns == 0 {
                    continue;
                }

                let cid = client_id.unwrap_or(pid as u64);

                current
                    .entry(cid)
                    .and_modify(|e| {
                        e.1 = e.1.max(video_ns);
                        e.2 = e.2.max(render_ns);
                    })
                    .or_insert_with(|| {
                        if proc_name.is_empty() {
                            proc_name = fs::read_to_string(format!("/proc/{}/comm", pid))
                                .unwrap_or_default()
                                .trim()
                                .to_string();
                        }
                        (proc_name.clone(), video_ns, render_ns)
                    });
            }
        }

        let mut media_list: Vec<(String, u32, u32, u32)> = Vec::new();

        for (cid, (name, video_ns, render_ns)) in &current {
            if let Some(&(prev_video, prev_render, prev_t)) = self.prev.get(cid) {
                let elapsed = now.duration_since(prev_t).as_nanos() as u64;
                if elapsed == 0 {
                    continue;
                }

                let video_d = video_ns.saturating_sub(prev_video);
                let render_d = render_ns.saturating_sub(prev_render);
                let video_p = ((video_d as f64 / elapsed as f64) * 100.0) as u32;
                let render_p = ((render_d as f64 / elapsed as f64) * 100.0) as u32;

                if video_p > 0 || render_p > 0 {
                    media_list.push((name.clone(), video_p, 0, render_p));
                }
            }
        }

        self.prev.clear();
        for (cid, (_, video_ns, render_ns)) in &current {
            self.prev.insert(*cid, (*video_ns, *render_ns, now));
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
    media_procs: Vec<(String, u32, u32, u32)>, // (name, dec%, enc%, gfx%)
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
    media_procs: Vec<(String, u32, u32, u32)>, // (name, dec%, enc%, gfx%)
    compute_procs: Vec<(String, u32)>,
    gpu_kind: GpuKind,
    vram_used_mb: u32,
    vram_total_mb: u32,
    gpu_gfx_percent: u32,
    cpu_percent: u32,
    ram_used_mb: u32,
    ram_total_mb: u32,
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
            "--tui" => {
                run_tui_mode();
                return glib::ExitCode::SUCCESS;
            }
            "--gui2" => {
                let app = Application::builder().application_id(APP_ID).build();
                app.connect_activate(build_ui2);
                let argv0 = args.first().map(String::as_str).unwrap_or("power_panel");
                return app.run_with_args(&[argv0]);
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
    println!("  --tui            TUI (Bar görünümlü) modunda çalıştır");
    println!("  --gui2           Alternatif bar-görünümlü GUI modunda çalıştır");
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

fn render_cli_frame(cpu_watt: f32, cpu_temp: f32, cpu_percent: u32, gpu: &GpuData, ram_used_mb: u32, ram_total_mb: u32) {
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
    let cpu_tc = cli_temp_color(cpu_temp);
    let cpu_uc = if cpu_percent >= 90 { "\x1B[91m" } else if cpu_percent >= 75 { "\x1B[93m" } else { "\x1B[92m" };
    let cpu_p = format!("CPU  {:6.1}W   {:3.0}°C   ●{:>3}%", cpu_watt, cpu_temp.floor(), cpu_percent);
    let cpu_r = format!("{YL}CPU{R}  {WH}{:6.1}W{R}   {cpu_tc}{:3.0}°C{R}   {cpu_uc}●{:>3}%{R}", cpu_watt, cpu_temp.floor(), cpu_percent);
    println!("{}", cli_row(&cpu_p, &cpu_r, W));

    // ── GPU ───────────────────────────────────────────────────────────────────
    let gpu_tc = cli_temp_color(gpu.temp);
    let gpu_has_pct = !matches!(gpu.kind, GpuKind::Unknown | GpuKind::Intel);
    let gpu_uc = if gpu.gfx_percent >= 90 { "\x1B[91m" } else if gpu.gfx_percent >= 75 { "\x1B[93m" } else { "\x1B[92m" };
    let gpu_p = if gpu_has_pct {
        format!("GPU  {:6.1}W   {:3.0}°C   ●{:>3}%", gpu.watt, gpu.temp.floor(), gpu.gfx_percent)
    } else {
        format!("GPU  {:6.1}W   {:3.0}°C   ●  —", gpu.watt, gpu.temp.floor())
    };
    let gpu_r = if gpu_has_pct {
        format!("{GN}GPU{R}  {WH}{:6.1}W{R}   {gpu_tc}{:3.0}°C{R}   {gpu_uc}●{:>3}%{R}", gpu.watt, gpu.temp.floor(), gpu.gfx_percent)
    } else {
        format!("{GN}GPU{R}  {WH}{:6.1}W{R}   {gpu_tc}{:3.0}°C{R}   ●  —", gpu.watt, gpu.temp.floor())
    };
    println!("{}", cli_row(&gpu_p, &gpu_r, W));

    // ── RAM ───────────────────────────────────────────────────────────────────
    if ram_total_mb > 0 {
        let ram_pct = ram_used_mb * 100 / ram_total_mb;
        let ram_uc = if ram_pct >= 90 { "\x1B[91m" } else if ram_pct >= 75 { "\x1B[93m" } else { "\x1B[92m" };
        let ram_p = format!("RAM   {:>5}/{:>5} MB   ●{:>3}%", ram_used_mb, ram_total_mb, ram_pct);
        let ram_r = format!("{YL}RAM{R}   {BL}{:>5}/{:>5} MB{R}   {ram_uc}●{:>3}%{R}", ram_used_mb, ram_total_mb, ram_pct);
        println!("{}", cli_row(&ram_p, &ram_r, W));
    }

    // ── VRAM ──────────────────────────────────────────────────────────────────
    if gpu.vram_total_mb > 0 {
        let vram_pct = gpu.vram_used_mb * 100 / gpu.vram_total_mb;
        let vram_uc = if vram_pct >= 90 { "\x1B[91m" } else if vram_pct >= 75 { "\x1B[93m" } else { "\x1B[92m" };
        let vram_p = format!("VRAM  {:>5}/{:>5} MB   ●{:>3}%", gpu.vram_used_mb, gpu.vram_total_mb, vram_pct);
        let vram_r = format!("{GN}VRAM{R}  {BL}{:>5}/{:>5} MB{R}   {vram_uc}●{:>3}%{R}", gpu.vram_used_mb, gpu.vram_total_mb, vram_pct);
        println!("{}", cli_row(&vram_p, &vram_r, W));
    }

    // ── Process section (GFX / DEC / ENC / SM% combined) ─────────────────────
    let has_compute = !gpu.compute_procs.is_empty();
    if !gpu.media_procs.is_empty() || has_compute {
        println!("{}", cli_titled_sep("Procs", W));

        // combined: (name, gfx, dec, enc, sm)
        let mut combined: Vec<(String, Option<u32>, Option<u32>, Option<u32>, Option<u32>)> = Vec::new();
        for (name, dec, enc, gfx) in &gpu.media_procs {
            let g = if *gfx > 0 { Some(*gfx) } else { None };
            let d = if *dec > 0 { Some(*dec) } else { None };
            let e = if *enc > 0 { Some(*enc) } else { None };
            combined.push((name.clone(), g, d, e, None));
        }
        for (name, sm) in &gpu.compute_procs {
            let s = if *sm > 0 { Some(*sm) } else { None };
            if let Some(entry) = combined.iter_mut().find(|(n, ..)| n == name) {
                entry.4 = s;
            } else {
                combined.push((name.clone(), None, None, None, s));
            }
        }

        let fmt_v = |v: Option<u32>| -> String {
            match v {
                Some(x) if x > 0 => format!("{:>4}%", x),
                _ => "   —".to_string(),
            }
        };

        if has_compute {
            let hdr_p = format!("{:<12} {:>5} {:>5} {:>5} {:>5}", "Process", "GFX", "DEC", "ENC", "SM%");
            let hdr_r = format!("{DM}{:<12} {:>5} {:>5} {:>5} {:>5}{R}", "Process", "GFX", "DEC", "ENC", "SM%");
            println!("{}", cli_row(&hdr_p, &hdr_r, W));
        } else {
            let hdr_p = format!("{:<12} {:>5} {:>5} {:>5}", "Process", "GFX", "DEC", "ENC");
            let hdr_r = format!("{DM}{:<12} {:>5} {:>5} {:>5}{R}", "Process", "GFX", "DEC", "ENC");
            println!("{}", cli_row(&hdr_p, &hdr_r, W));
        }

        for (name, gfx, dec, enc, sm) in combined.iter().take(4) {
            let name_t: String = if name.chars().count() > 11 {
                format!("{}…", name.chars().take(10).collect::<String>())
            } else {
                name.clone()
            };
            if has_compute {
                let row_p = format!("  {:<10} {:>5} {:>5} {:>5} {:>5}", name_t, fmt_v(*gfx), fmt_v(*dec), fmt_v(*enc), fmt_v(*sm));
                let row_r = format!("  {PR}{:<10}{R} {WH}{:>5} {:>5} {:>5} {:>5}{R}", name_t, fmt_v(*gfx), fmt_v(*dec), fmt_v(*enc), fmt_v(*sm));
                println!("{}", cli_row(&row_p, &row_r, W));
            } else {
                let row_p = format!("  {:<10} {:>5} {:>5} {:>5}", name_t, fmt_v(*gfx), fmt_v(*dec), fmt_v(*enc));
                let row_r = format!("  {PR}{:<10}{R} {WH}{:>5} {:>5} {:>5}{R}", name_t, fmt_v(*gfx), fmt_v(*dec), fmt_v(*enc));
                println!("{}", cli_row(&row_p, &row_r, W));
            }
        }
    }

    println!("{bot}");
    io::stdout().flush().unwrap();
}

fn render_bar(pct: u32, width: usize) -> (String, String) {
    const GN: &str = "\x1B[92m";
    const YL: &str = "\x1B[93m";
    const OR: &str = "\x1B[38;5;208m";
    const RD: &str = "\x1B[91m";
    const DM: &str = "\x1B[2m";
    const R:  &str = "\x1B[0m";

    let filled = (pct as usize * width / 100).min(width);
    let seg = width / 4;
    let colors = [GN, YL, OR, RD];

    let plain: String = (0..width).map(|i| if i < filled { '█' } else { '░' }).collect();
    let mut colored = String::new();

    for s in 0..4usize {
        let start = s * seg;
        let end = if s == 3 { width } else { (s + 1) * seg };
        let seg_filled = filled.saturating_sub(start).min(end - start);
        let seg_empty  = (end - start) - seg_filled;
        if seg_filled > 0 {
            colored.push_str(colors[s]);
            for _ in 0..seg_filled { colored.push('█'); }
            colored.push_str(R);
        }
        if seg_empty > 0 {
            colored.push_str(DM);
            for _ in 0..seg_empty { colored.push('░'); }
            colored.push_str(R);
        }
    }

    (plain, colored)
}

fn fmt_gb(used_mb: u32, total_mb: u32) -> String {
    let used  = used_mb  as f32 / 1024.0;
    let total = total_mb as f32 / 1024.0;
    if total >= 100.0 {
        format!("{:.0}/{:.0} GB", used, total)   // "128/256 GB" = 10
    } else {
        format!("{:.1}/{:.0} GB", used, total)   // "14.5/32 GB" = 10 | "7.8/16 GB" = 9
    }
}

fn render_tui_frame(cpu_watt: f32, cpu_temp: f32, cpu_percent: u32, gpu: &GpuData, ram_used_mb: u32, ram_total_mb: u32) {
    use std::io::{self, Write};

    const W:   usize = 44;
    const BAR: usize = 28;

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

    // pct → right-justified 10-char value string (plain + colored)
    let pct_val = |pct: u32, color: &str| -> (String, String) {
        let s = format!("{:>3}%", pct);           // always 4 visible chars
        let pad = " ".repeat(10usize.saturating_sub(s.chars().count()));
        let plain   = format!("{}{}", pad, s);
        let colored = format!("{}{}{}{}", pad, color, s, R);
        (plain, colored)
    };

    // "  —" right-justified in 10 chars
    let dash_val = || -> (String, String) {
        let s = "  —";
        let pad = " ".repeat(10usize.saturating_sub(s.chars().count()));
        let plain   = format!("{}{}", pad, s);
        let colored = format!("{}{DM}{}{R}", pad, s);
        (plain, colored)
    };

    print!("\x1B[2J\x1B[H");

    // ── Title ──────────────────────────────────────────────────────────────
    // "PowerPanel" (10) + {:>33.1} (33) + "W" (1) = 44
    let title_p = format!("PowerPanel{:>33.1}W", total);
    let title_r = format!("{BD}{CY}PowerPanel{R}{:>33.1}{BD}{WH}W{R}", total);
    println!("{top}");
    println!("{}", cli_row(&title_p, &title_r, W));
    println!("{mid}");

    // ── CPU bar ────────────────────────────────────────────────────────────
    // "CPU  " (5) + bar (28) + " " (1) + val (10) = 44
    let (cpu_bar_p, cpu_bar_r) = render_bar(cpu_percent.min(100), BAR);
    let (cpu_val_p, cpu_val_r) = pct_val(cpu_percent, CY);
    let cpu_p = format!("CPU  {} {}", cpu_bar_p, cpu_val_p);
    let cpu_r = format!("{YL}CPU{R}  {} {}", cpu_bar_r, cpu_val_r);
    println!("{}", cli_row(&cpu_p, &cpu_r, W));

    // ── GPU bar ────────────────────────────────────────────────────────────
    let gpu_has_pct = !matches!(gpu.kind, GpuKind::Unknown | GpuKind::Intel);
    let gpu_pct = if gpu_has_pct { gpu.gfx_percent } else { 0 };
    let (gpu_bar_p, gpu_bar_r) = render_bar(gpu_pct.min(100), BAR);
    let (gpu_val_p, gpu_val_r) = if gpu_has_pct { pct_val(gpu_pct, GN) } else { dash_val() };
    let gpu_p = format!("GPU  {} {}", gpu_bar_p, gpu_val_p);
    let gpu_r = format!("{GN}GPU{R}  {} {}", gpu_bar_r, gpu_val_r);
    println!("{}", cli_row(&gpu_p, &gpu_r, W));

    // ── RAM bar ────────────────────────────────────────────────────────────
    if ram_total_mb > 0 {
        let ram_pct = (ram_used_mb * 100 / ram_total_mb).min(100);
        let (ram_bar_p, ram_bar_r) = render_bar(ram_pct, BAR);
        let ram_val = fmt_gb(ram_used_mb, ram_total_mb);
        let ram_p = format!("RAM  {} {}", ram_bar_p, ram_val);
        let ram_r = format!("{YL}RAM{R}  {} {BL}{}{R}", ram_bar_r, ram_val);
        println!("{}", cli_row(&ram_p, &ram_r, W));
    }

    // ── VRAM bar ───────────────────────────────────────────────────────────
    if gpu.vram_total_mb > 0 {
        let vram_pct = (gpu.vram_used_mb * 100 / gpu.vram_total_mb).min(100);
        let (vram_bar_p, vram_bar_r) = render_bar(vram_pct, BAR);
        let vram_val = fmt_gb(gpu.vram_used_mb, gpu.vram_total_mb);
        let vram_p = format!("VRAM {} {}", vram_bar_p, vram_val);
        let vram_r = format!("{GN}VRAM{R} {} {BL}{}{R}", vram_bar_r, vram_val);
        println!("{}", cli_row(&vram_p, &vram_r, W));
    }

    // ── Temp & Power strip ─────────────────────────────────────────────────
    // "CPU  " (5) + temp (5) + "  " (2) + watt (6) + "        " (8) + "GPU  " (5) + temp (5) + "  " (2) + watt (6) = 44
    println!("{mid}");
    let cpu_tc = cli_temp_color(cpu_temp);
    let gpu_tc = cli_temp_color(gpu.temp);
    let stats_p = format!(
        "CPU  {:>3.0}°C  {:>5.1}W        GPU  {:>3.0}°C  {:>5.1}W",
        cpu_temp.floor(), cpu_watt, gpu.temp.floor(), gpu.watt
    );
    let stats_r = format!(
        "{YL}CPU{R}  {cpu_tc}{:>3.0}°C{R}  {WH}{:>5.1}W{R}        {GN}GPU{R}  {gpu_tc}{:>3.0}°C{R}  {WH}{:>5.1}W{R}",
        cpu_temp.floor(), cpu_watt, gpu.temp.floor(), gpu.watt
    );
    println!("{}", cli_row(&stats_p, &stats_r, W));

    // ── Process table ──────────────────────────────────────────────────────
    let has_compute = !gpu.compute_procs.is_empty();
    if !gpu.media_procs.is_empty() || has_compute {
        println!("{}", cli_titled_sep("Procs", W));

        let mut combined: Vec<(String, Option<u32>, Option<u32>, Option<u32>, Option<u32>)> = Vec::new();
        for (name, dec, enc, gfx) in &gpu.media_procs {
            let g = if *gfx > 0 { Some(*gfx) } else { None };
            let d = if *dec > 0 { Some(*dec) } else { None };
            let e = if *enc > 0 { Some(*enc) } else { None };
            combined.push((name.clone(), g, d, e, None));
        }
        for (name, sm) in &gpu.compute_procs {
            let s = if *sm > 0 { Some(*sm) } else { None };
            if let Some(entry) = combined.iter_mut().find(|(n, ..)| n == name) {
                entry.4 = s;
            } else {
                combined.push((name.clone(), None, None, None, s));
            }
        }

        let fmt_v = |v: Option<u32>| -> String {
            match v {
                Some(x) if x > 0 => format!("{:>3}%", x),
                _ => "  —".to_string(),
            }
        };

        // W=44: with SM    → {:<12} {:>7} {:>7} {:>7} {:>7} = 12+1+7+1+7+1+7+1+7 = 44
        //        without SM → {:<14} {:>9} {:>9} {:>9}       = 14+1+9+1+9+1+9     = 44
        if has_compute {
            let hdr_p = format!("{:<12} {:>7} {:>7} {:>7} {:>7}", "Process", "GFX", "DEC", "ENC", "SM%");
            let hdr_r = format!("{DM}{:<12} {:>7} {:>7} {:>7} {:>7}{R}", "Process", "GFX", "DEC", "ENC", "SM%");
            println!("{}", cli_row(&hdr_p, &hdr_r, W));
        } else {
            let hdr_p = format!("{:<14} {:>9} {:>9} {:>9}", "Process", "GFX", "DEC", "ENC");
            let hdr_r = format!("{DM}{:<14} {:>9} {:>9} {:>9}{R}", "Process", "GFX", "DEC", "ENC");
            println!("{}", cli_row(&hdr_p, &hdr_r, W));
        }

        for (name, gfx, dec, enc, sm) in combined.iter().take(4) {
            if has_compute {
                let name_t: String = if name.chars().count() > 11 {
                    format!("{}…", name.chars().take(10).collect::<String>())
                } else { name.clone() };
                let row_p = format!("  {:<10} {:>7} {:>7} {:>7} {:>7}", name_t, fmt_v(*gfx), fmt_v(*dec), fmt_v(*enc), fmt_v(*sm));
                let row_r = format!("  {PR}{:<10}{R} {WH}{:>7} {:>7} {:>7} {:>7}{R}", name_t, fmt_v(*gfx), fmt_v(*dec), fmt_v(*enc), fmt_v(*sm));
                println!("{}", cli_row(&row_p, &row_r, W));
            } else {
                let name_t: String = if name.chars().count() > 13 {
                    format!("{}…", name.chars().take(12).collect::<String>())
                } else { name.clone() };
                let row_p = format!("  {:<12} {:>9} {:>9} {:>9}", name_t, fmt_v(*gfx), fmt_v(*dec), fmt_v(*enc));
                let row_r = format!("  {PR}{:<12}{R} {WH}{:>9} {:>9} {:>9}{R}", name_t, fmt_v(*gfx), fmt_v(*dec), fmt_v(*enc));
                println!("{}", cli_row(&row_p, &row_r, W));
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

            sys.refresh_memory();
            sys.refresh_cpu_usage();
            let ram_used_mb = (sys.used_memory() / 1_048_576) as u32;
            let ram_total_mb = (sys.total_memory() / 1_048_576) as u32;
            let cpu_percent = sys.global_cpu_usage() as u32;

            render_cli_frame(cpu_watt, cpu_temp, cpu_percent, &gpu, ram_used_mb, ram_total_mb);

            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    });
}

fn run_tui_mode() {
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

            sys.refresh_memory();
            sys.refresh_cpu_usage();
            let ram_used_mb = (sys.used_memory() / 1_048_576) as u32;
            let ram_total_mb = (sys.total_memory() / 1_048_576) as u32;
            let cpu_percent = sys.global_cpu_usage() as u32;

            render_tui_frame(cpu_watt, cpu_temp, cpu_percent, &gpu, ram_used_mb, ram_total_mb);

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

                // Primary GPU utilization: nvmlDeviceGetUtilizationRates — same API as nvidia-smi/nvidia-settings
                if let Ok(util) = dev.utilization_rates() {
                    data.gfx_percent = util.gpu;
                }

                // Per-process stats: codec DEC/ENC and SM% per PID (separate from total utilization)
                if let Ok(samples) = dev.process_utilization_stats(Some(0)) {
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
                    data.media_procs.push((name, dec, enc, 0));
                }
                data.media_procs.sort_by(|a, b| (b.1 + b.2).cmp(&(a.1 + a.2)));

                // CUDA → compute_procs: only show processes with active SM%
                for pid in &cuda_pids {
                    let sm = sm_by_pid.get(pid).copied().unwrap_or(0);
                    if sm == 0 {
                        continue;
                    }
                    let name = sys
                        .process(sysinfo::Pid::from(*pid as usize))
                        .map(|p| p.name().to_string_lossy().into_owned())
                        .unwrap_or_else(|| format!("pid:{}", pid));
                    data.compute_procs.push((name, sm));
                }
                data.compute_procs.sort_by(|a, b| b.1.cmp(&a.1));
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

fn usage_css_class(pct: u32) -> &'static str {
    if pct >= 90 { "val-temp-hot" }
    else if pct >= 75 { "val-temp-warm" }
    else { "val-pct" }
}

fn temp_hex_color(t: f32) -> &'static str {
    if t >= 80.0 { "#ff4757" } else if t >= 60.0 { "#ff9f43" } else { "#4cd964" }
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
        .lbl-ram {
            color: #00cec9; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 16px; font-weight: bold;
        }
        .val-vram {
            color: #74b9ff; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 14px;
        }
        .val-proc {
            color: #b2bec3; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 13px;
        }
        .proc-hdr {
            color: #a29bfe; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 13px; font-weight: bold;
        }
        .proc-val {
            color: #b2bec3; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 12px;
        }
        .proc-num {
            color: #dfe6e9; font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 12px;
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

    let (cpu_row, cpu_watt_lbl, cpu_therm_lbl, cpu_temp_lbl, cpu_pct_lbl) = make_hw_row("", "CPU", "lbl-cpu");
    panel.append(&cpu_row);

    let (gpu_row, gpu_watt_lbl, gpu_therm_lbl, gpu_temp_lbl, gpu_pct_lbl) = make_hw_row("󰢮", "GPU", "lbl-gpu");
    panel.append(&gpu_row);

    let (ram_row, ram_lbl, ram_pct_lbl) = make_ram_row();
    panel.append(&ram_row);

    let (vram_row, vram_lbl, vram_pct_lbl) = make_vram_row();
    vram_row.set_visible(false);
    panel.append(&vram_row);

    let sep = gtk4::Separator::new(Orientation::Horizontal);
    sep.add_css_class("divider");
    sep.set_visible(false);
    panel.append(&sep);

    let (proc_container, proc_lbl, gfx_lbl, dec_lbl, enc_lbl, sm_lbl) = make_process_section();
    proc_container.set_visible(false);
    panel.append(&proc_container);

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
                GpuBackend::Amd { pdev, vcn_instances, .. } => Some(FdInfoTracker::new(pdev.clone(), *vcn_instances)),
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
        cpu_temp_lbl.set_text(&format!("{:>3.0}°C", target.cpu_temp.floor()));
        cpu_pct_lbl.set_css_classes(&[usage_css_class(target.cpu_percent)]);
        cpu_pct_lbl.set_text(&format!("●{:>3}%", target.cpu_percent));

        gpu_watt_lbl.set_text(&format!("{:>6.1} W", target.gpu_watt));
        let gpu_cls = temp_css_class(target.gpu_temp);
        gpu_therm_lbl.set_css_classes(&[gpu_cls]);
        gpu_temp_lbl.set_css_classes(&[gpu_cls]);
        gpu_temp_lbl.set_text(&format!("{:>3.0}°C", target.gpu_temp.floor()));
        let gpu_has_pct = matches!(target.gpu_kind, GpuKind::Nvidia | GpuKind::Amd);
        let gpu_pct_text = if gpu_has_pct { format!("●{:>3}%", target.gpu_gfx_percent) } else { "●  —".to_string() };
        gpu_pct_lbl.set_css_classes(&[if gpu_has_pct { usage_css_class(target.gpu_gfx_percent) } else { "val-pct" }]);
        gpu_pct_lbl.set_text(&gpu_pct_text);

        let valid_gpu = target.gpu_kind != GpuKind::Unknown;

        if target.ram_total_mb > 0 {
            let ram_pct = target.ram_used_mb * 100 / target.ram_total_mb;
            ram_lbl.set_text(&format!("{:>5}/{:>5} MB ", target.ram_used_mb, target.ram_total_mb));
            ram_pct_lbl.set_css_classes(&[usage_css_class(ram_pct)]);
            ram_pct_lbl.set_text(&format!("●{:>3}%", ram_pct));
        }

        if valid_gpu && target.vram_total_mb > 0 {
            vram_row.set_visible(true);
            vram_lbl.set_text(&format!("{:>5}/{:>5} MB ", target.vram_used_mb, target.vram_total_mb));
            let vram_pct = target.vram_used_mb * 100 / target.vram_total_mb;
            vram_pct_lbl.set_css_classes(&[usage_css_class(vram_pct)]);
            vram_pct_lbl.set_text(&format!("●{:>3}%", vram_pct));
        } else {
            vram_row.set_visible(false);
        }

        let has_media = valid_gpu && !target.media_procs.is_empty();
        let has_compute = valid_gpu && !target.compute_procs.is_empty();
        let has_procs = has_media || has_compute;

        if has_procs {
            proc_container.set_visible(true);

            // combined: (name, gfx, dec, enc, sm) — all Option<u32>
            let mut combined: Vec<(String, Option<u32>, Option<u32>, Option<u32>, Option<u32>)> = Vec::new();
            for (name, dec, enc, gfx) in &target.media_procs {
                let gfx_v = if *gfx > 0 { Some(*gfx) } else { None };
                let dec_v = if *dec > 0 { Some(*dec) } else { None };
                let enc_v = if *enc > 0 { Some(*enc) } else { None };
                combined.push((name.clone(), gfx_v, dec_v, enc_v, None));
            }
            for (name, sm) in &target.compute_procs {
                let sm_v = if *sm > 0 { Some(*sm) } else { None };
                if let Some(entry) = combined.iter_mut().find(|(n, ..)| n == name) {
                    entry.4 = sm_v;
                } else {
                    combined.push((name.clone(), None, None, None, sm_v));
                }
            }

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

            proc_lbl.set_text(&combined.iter().map(|(n, ..)| trunc(n)).collect::<Vec<_>>().join("\n"));
            gfx_lbl.set_text(&combined.iter().map(|(_, g, ..)| fmt_val(*g)).collect::<Vec<_>>().join("\n"));
            dec_lbl.set_text(&combined.iter().map(|(_, _, d, ..)| fmt_val(*d)).collect::<Vec<_>>().join("\n"));
            enc_lbl.set_text(&combined.iter().map(|(_, _, _, e, _)| fmt_val(*e)).collect::<Vec<_>>().join("\n"));
            sm_lbl.set_text(&combined.iter().map(|(_, _, _, _, s)| fmt_val(*s)).collect::<Vec<_>>().join("\n"));
        } else {
            proc_container.set_visible(false);
        }

        sep.set_visible(valid_gpu && (has_procs || target.vram_total_mb > 0));
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
        .label(" ")
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

fn make_process_section() -> (Grid, Label, Label, Label, Label, Label) {
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

    // Row 0: headers, Row 1: data
    grid.attach(&lbl_name_hdr, 0, 0, 1, 1);
    grid.attach(&lbl_gfx_hdr,  1, 0, 1, 1);
    grid.attach(&lbl_dec_hdr,  2, 0, 1, 1);
    grid.attach(&lbl_enc_hdr,  3, 0, 1, 1);
    grid.attach(&lbl_sm_hdr,   4, 0, 1, 1);
    grid.attach(&lbl_proc,     0, 1, 1, 1);
    grid.attach(&lbl_gfx,      1, 1, 1, 1);
    grid.attach(&lbl_dec,      2, 1, 1, 1);
    grid.attach(&lbl_enc,      3, 1, 1, 1);
    grid.attach(&lbl_sm,       4, 1, 1, 1);

    (grid, lbl_proc, lbl_gfx, lbl_dec, lbl_enc, lbl_sm)
}

// ── Bar-GUI helpers ───────────────────────────────────────────────────────────

fn draw_bar_fn(cr: &gtk4::cairo::Context, width: i32, height: i32, pct: u32) {
    let filled_w = (pct.min(100) as f64 / 100.0 * width as f64) as i32;
    let seg = (width / 4).max(1);
    const COLORS: [(f64, f64, f64); 4] = [
        (0.18, 0.80, 0.44), // green
        (0.95, 0.77, 0.06), // yellow
        (0.90, 0.49, 0.13), // orange
        (0.91, 0.30, 0.24), // red
    ];
    let y = 1.0_f64;
    let h = (height - 2) as f64;
    for s in 0..4i32 {
        let x0 = s * seg;
        let x1 = if s == 3 { width } else { (s + 1) * seg };
        let sw = x1 - x0;
        let sf = (filled_w - x0).clamp(0, sw);
        let (r, g, b) = COLORS[s as usize];
        if sf > 0 {
            cr.set_source_rgb(r, g, b);
            cr.rectangle(x0 as f64, y, sf as f64, h);
            cr.fill().ok();
        }
        if sf < sw {
            cr.set_source_rgba(r, g, b, 0.15);
            cr.rectangle((x0 + sf) as f64, y, (sw - sf) as f64, h);
            cr.fill().ok();
        }
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

fn build_ui2(app: &Application) {
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
        .panel2 {
            background-color: rgba(10, 10, 10, 0.82);
            border-radius: 18px;
            border: 1px solid rgba(255, 255, 255, 0.15);
            padding: 14px 18px;
        }
        .brand-lbl {
            color: #a0a8b0;
            font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 13px;
        }
        .total-watt {
            color: #00ffcc;
            font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 22px; font-weight: bold;
        }
        .lbl-cpu {
            color: #ff9f43;
            font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 14px; font-weight: bold;
        }
        .lbl-gpu {
            color: #2ecc71;
            font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 14px; font-weight: bold;
        }
        .lbl-ram {
            color: #00cec9;
            font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 14px; font-weight: bold;
        }
        .val-pct {
            color: #dfe6e9;
            font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 13px;
        }
        .stat-lbl {
            font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 13px;
        }
        .divider {
            background-color: rgba(255, 255, 255, 0.10);
            min-height: 1px; margin: 2px 0px;
        }
        .proc-hdr {
            color: #a29bfe;
            font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 12px; font-weight: bold;
        }
        .proc-val {
            color: #b2bec3;
            font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 12px;
        }
        .proc-num {
            color: #dfe6e9;
            font-family: 'JetBrainsMono Nerd Font', 'JetBrains Mono', monospace;
            font-size: 12px;
        }
        ",
    );
    gtk4::style_context_add_provider_for_display(
        &gtk4::gdk::Display::default().unwrap(),
        &css,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // ── Layout ─────────────────────────────────────────────────────────────
    let panel = GtkBox::new(Orientation::Vertical, 6);
    panel.add_css_class("panel2");
    panel.set_size_request(340, -1);

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

    // Bar rows: CPU, GPU (pct val), RAM, VRAM (GB val)
    let (cpu_row, cpu_bar, cpu_pct_cell, cpu_val_lbl) = make_bar_row_2("CPU", "lbl-cpu", 5);
    panel.append(&cpu_row);

    let (gpu_row, gpu_bar, gpu_pct_cell, gpu_val_lbl) = make_bar_row_2("GPU", "lbl-gpu", 5);
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

    // Process section
    let sep2 = gtk4::Separator::new(Orientation::Horizontal);
    sep2.add_css_class("divider");
    sep2.set_visible(false);
    panel.append(&sep2);

    let (proc_container, proc_lbl, gfx_lbl, dec_lbl, enc_lbl, sm_lbl) = make_process_section();
    proc_container.set_visible(false);
    panel.append(&proc_container);

    window.set_child(Some(&panel));

    let gesture = gtk4::GestureClick::new();
    gesture.set_button(3);
    let win_clone = window.clone();
    gesture.connect_released(move |_, _, _, _| win_clone.close());
    window.add_controller(gesture);

    window.present();

    // ── Background sensor thread (same as build_ui) ─────────────────────
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
                GpuBackend::Amd { pdev, vcn_instances, .. } => Some(FdInfoTracker::new(pdev.clone(), *vcn_instances)),
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

            let cpu_temp_path = detect_cpu_temp_path();
            let mut loop_count: u32 = 0;
            let mut gfx_max: u32 = 0;

            loop {
                loop_count += 1;

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
                                if let Some(t) = c.temperature() { temp = t; found_die = true; break; }
                            }
                        }
                        if !found_die {
                            for c in &comps {
                                let lbl = c.label().to_lowercase();
                                if lbl == "tctl" || lbl.contains("k10") || lbl.contains("composite") || lbl.contains("package") {
                                    if let Some(t) = c.temperature() { if t > temp { temp = t; } }
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
                                } else { 0.0 };
                                tracker.last_energy = current;
                                tracker.last_time = now;
                                if watts > 1.0 && watts < 400.0 { watts } else { 0.0 }
                            }
                            Err(_) => 0.0,
                        }
                    } else { 0.0 };

                    let mut gpu = read_gpu_data(
                        &gpu_backend, &mut sys,
                        &mut intel_gpu_tracker,
                        &mut amd_fdinfo_tracker,
                        &mut intel_fdinfo_tracker,
                    );
                    gpu.gfx_percent = gpu.gfx_percent.max(gfx_max);
                    gfx_max = 0;

                    sys.refresh_memory();
                    sys.refresh_cpu_usage();
                    let ram_used_mb  = (sys.used_memory()  / 1_048_576) as u32;
                    let ram_total_mb = (sys.total_memory() / 1_048_576) as u32;
                    let cpu_percent  = sys.global_cpu_usage() as u32;

                    if let Ok(mut d) = data_writer.lock() {
                        d.cpu_temp       = cpu_temp;
                        d.cpu_watt       = cpu_watt_raw;
                        d.gpu_temp       = gpu.temp;
                        d.gpu_watt       = gpu.watt;
                        d.media_procs    = gpu.media_procs;
                        d.compute_procs  = gpu.compute_procs;
                        d.gpu_kind       = gpu.kind;
                        d.vram_used_mb   = gpu.vram_used_mb;
                        d.vram_total_mb  = gpu.vram_total_mb;
                        d.gpu_gfx_percent = gpu.gfx_percent;
                        d.cpu_percent    = cpu_percent;
                        d.ram_used_mb    = ram_used_mb;
                        d.ram_total_mb   = ram_total_mb;
                    }
                }

                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        });
    });

    // ── GTK update loop ─────────────────────────────────────────────────────
    glib::timeout_add_local(Duration::from_millis(1000), move || {
        let target = match data.lock() {
            Ok(d) => d.clone(),
            Err(_) => return glib::ControlFlow::Continue,
        };

        total_label.set_text(&format!("⚡ {:>6.1} W", target.cpu_watt + target.gpu_watt));

        // CPU bar
        cpu_pct_cell.set(target.cpu_percent.min(100));
        cpu_bar.queue_draw();
        cpu_val_lbl.set_text(&format!("{:>3}%", target.cpu_percent));

        // GPU bar
        let gpu_has_pct = matches!(target.gpu_kind, GpuKind::Nvidia | GpuKind::Amd);
        let gpu_pct = if gpu_has_pct { target.gpu_gfx_percent.min(100) } else { 0 };
        gpu_pct_cell.set(gpu_pct);
        gpu_bar.queue_draw();
        let gpu_val_str = if gpu_has_pct { format!("{:>3}%", target.gpu_gfx_percent) } else { "  —".to_string() };
        gpu_val_lbl.set_text(&gpu_val_str);

        // RAM bar
        if target.ram_total_mb > 0 {
            let ram_pct = (target.ram_used_mb * 100 / target.ram_total_mb).min(100);
            ram_pct_cell.set(ram_pct);
            ram_bar.queue_draw();
            ram_val_lbl.set_text(&fmt_gb(target.ram_used_mb, target.ram_total_mb));
        }

        // VRAM bar
        let valid_gpu = target.gpu_kind != GpuKind::Unknown;
        if valid_gpu && target.vram_total_mb > 0 {
            let vram_pct = (target.vram_used_mb * 100 / target.vram_total_mb).min(100);
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

        // Process section
        let has_media   = valid_gpu && !target.media_procs.is_empty();
        let has_compute = valid_gpu && !target.compute_procs.is_empty();
        let has_procs   = has_media || has_compute;

        if has_procs {
            proc_container.set_visible(true);

            let mut combined: Vec<(String, Option<u32>, Option<u32>, Option<u32>, Option<u32>)> = Vec::new();
            for (name, dec, enc, gfx) in &target.media_procs {
                combined.push((name.clone(),
                    if *gfx > 0 { Some(*gfx) } else { None },
                    if *dec > 0 { Some(*dec) } else { None },
                    if *enc > 0 { Some(*enc) } else { None },
                    None));
            }
            for (name, sm) in &target.compute_procs {
                let sv = if *sm > 0 { Some(*sm) } else { None };
                if let Some(e) = combined.iter_mut().find(|(n, ..)| n == name) {
                    e.4 = sv;
                } else {
                    combined.push((name.clone(), None, None, None, sv));
                }
            }

            let trunc = |n: &str| -> String {
                if n.chars().count() > 11 { format!("{}…", n.chars().take(10).collect::<String>()) }
                else { n.to_string() }
            };
            let fmt_val = |v: Option<u32>| -> String {
                match v {
                    Some(x) if x > 0 => format!("{:>3}%", x),
                    _ => "   —".to_string(),
                }
            };

            proc_lbl.set_text(&combined.iter().map(|(n, ..)| trunc(n)).collect::<Vec<_>>().join("\n"));
            gfx_lbl.set_text(&combined.iter().map(|(_, g, ..)| fmt_val(*g)).collect::<Vec<_>>().join("\n"));
            dec_lbl.set_text(&combined.iter().map(|(_, _, d, ..)| fmt_val(*d)).collect::<Vec<_>>().join("\n"));
            enc_lbl.set_text(&combined.iter().map(|(_, _, _, e, _)| fmt_val(*e)).collect::<Vec<_>>().join("\n"));
            sm_lbl.set_text(&combined.iter().map(|(_, _, _, _, s)| fmt_val(*s)).collect::<Vec<_>>().join("\n"));
        } else {
            proc_container.set_visible(false);
        }

        sep2.set_visible(valid_gpu && (has_procs || target.vram_total_mb > 0));
        glib::ControlFlow::Continue
    });
}