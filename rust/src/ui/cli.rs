use crate::gpu::{detect_gpu, read_gpu_data, FdInfoTracker, GpuBackend, IntelFdInfoTracker};
use crate::sensors::{detect_cpu_temp_path, find_rapl_path, read_u64};
use crate::types::{usage_percent, CombinedProc, GpuData, GpuKind, GpuPowerTracker, PowerTracker};
use std::time::{Duration, Instant};
use sysinfo::System;

const SENSOR_PRIME_DELAY: Duration = Duration::from_millis(250);

pub(super) fn cli_row(plain: &str, colored: &str, w: usize) -> String {
    let pad = w.saturating_sub(plain.chars().count());
    format!(
        "\x1B[2m│\x1B[0m {}{} \x1B[2m│\x1B[0m",
        colored,
        " ".repeat(pad)
    )
}

pub(super) fn cli_temp_color(t: f32) -> &'static str {
    if t >= 80.0 {
        "\x1B[91m"
    } else if t >= 60.0 {
        "\x1B[93m"
    } else {
        "\x1B[92m"
    }
}

pub(super) fn cli_titled_sep(title: &str, w: usize) -> String {
    let inner = w + 2;
    let prefix = format!("─── {} ", title);
    let plen = prefix.chars().count();
    let remaining = inner.saturating_sub(plen);
    format!("\x1B[2m├{}{}┤\x1B[0m", prefix, "─".repeat(remaining))
}

fn render_cli_frame(
    cpu_watt: f32,
    cpu_temp: f32,
    cpu_percent: u32,
    gpu: &GpuData,
    ram_used_mb: u32,
    ram_total_mb: u32,
) {
    use std::io::{self, Write};

    const W: usize = 38; // inner content width (between flanking spaces inside │)

    // ANSI
    const R: &str = "\x1B[0m";
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
    let cpu_uc = if cpu_percent >= 90 {
        "\x1B[91m"
    } else if cpu_percent >= 75 {
        "\x1B[93m"
    } else {
        "\x1B[92m"
    };
    let cpu_p = format!(
        "CPU  {:6.1}W   {:3.0}°C   ●{:>3}%",
        cpu_watt,
        cpu_temp.floor(),
        cpu_percent
    );
    let cpu_r = format!(
        "{YL}CPU{R}  {WH}{:6.1}W{R}   {cpu_tc}{:3.0}°C{R}   {cpu_uc}●{:>3}%{R}",
        cpu_watt,
        cpu_temp.floor(),
        cpu_percent
    );
    println!("{}", cli_row(&cpu_p, &cpu_r, W));

    // ── GPU ───────────────────────────────────────────────────────────────────
    let gpu_tc = cli_temp_color(gpu.temp);
    let gpu_has_pct = !matches!(gpu.kind, GpuKind::Unknown | GpuKind::Intel);
    let gpu_uc = if gpu.gfx_percent >= 90 {
        "\x1B[91m"
    } else if gpu.gfx_percent >= 75 {
        "\x1B[93m"
    } else {
        "\x1B[92m"
    };
    let gpu_p = if gpu_has_pct {
        format!(
            "GPU  {:6.1}W   {:3.0}°C   ●{:>3}%",
            gpu.watt,
            gpu.temp.floor(),
            gpu.gfx_percent
        )
    } else {
        format!("GPU  {:6.1}W   {:3.0}°C   ●  —", gpu.watt, gpu.temp.floor())
    };
    let gpu_r = if gpu_has_pct {
        format!(
            "{GN}GPU{R}  {WH}{:6.1}W{R}   {gpu_tc}{:3.0}°C{R}   {gpu_uc}●{:>3}%{R}",
            gpu.watt,
            gpu.temp.floor(),
            gpu.gfx_percent
        )
    } else {
        format!(
            "{GN}GPU{R}  {WH}{:6.1}W{R}   {gpu_tc}{:3.0}°C{R}   ●  —",
            gpu.watt,
            gpu.temp.floor()
        )
    };
    println!("{}", cli_row(&gpu_p, &gpu_r, W));

    // ── RAM ───────────────────────────────────────────────────────────────────
    if ram_total_mb > 0 {
        let ram_pct = usage_percent(ram_used_mb, ram_total_mb);
        let ram_uc = if ram_pct >= 90 {
            "\x1B[91m"
        } else if ram_pct >= 75 {
            "\x1B[93m"
        } else {
            "\x1B[92m"
        };
        let ram_p = format!(
            "RAM   {:>5}/{:>5} MB   ●{:>3}%",
            ram_used_mb, ram_total_mb, ram_pct
        );
        let ram_r = format!(
            "{YL}RAM{R}   {BL}{:>5}/{:>5} MB{R}   {ram_uc}●{:>3}%{R}",
            ram_used_mb, ram_total_mb, ram_pct
        );
        println!("{}", cli_row(&ram_p, &ram_r, W));
    }

    // ── VRAM ──────────────────────────────────────────────────────────────────
    if gpu.vram_total_mb > 0 {
        let vram_pct = usage_percent(gpu.vram_used_mb, gpu.vram_total_mb);
        let vram_uc = if vram_pct >= 90 {
            "\x1B[91m"
        } else if vram_pct >= 75 {
            "\x1B[93m"
        } else {
            "\x1B[92m"
        };
        let vram_p = format!(
            "VRAM  {:>5}/{:>5} MB   ●{:>3}%",
            gpu.vram_used_mb, gpu.vram_total_mb, vram_pct
        );
        let vram_r = format!(
            "{GN}VRAM{R}  {BL}{:>5}/{:>5} MB{R}   {vram_uc}●{:>3}%{R}",
            gpu.vram_used_mb, gpu.vram_total_mb, vram_pct
        );
        println!("{}", cli_row(&vram_p, &vram_r, W));
    }

    // ── Process section (GFX / DEC / ENC / SM% combined) ─────────────────────
    let has_compute = !gpu.compute_procs.is_empty();
    if !gpu.media_procs.is_empty() || has_compute {
        println!("{}", cli_titled_sep("Procs", W));

        let combined = CombinedProc::from_gpu(gpu);

        let fmt_v = |v: Option<u32>| -> String {
            match v {
                Some(x) if x > 0 => format!("{:>4}%", x),
                _ => "   —".to_string(),
            }
        };

        if has_compute {
            let hdr_p = format!(
                "{:<12} {:>5} {:>5} {:>5} {:>5}",
                "Process", "GFX", "DEC", "ENC", "SM%"
            );
            let hdr_r = format!(
                "{DM}{:<12} {:>5} {:>5} {:>5} {:>5}{R}",
                "Process", "GFX", "DEC", "ENC", "SM%"
            );
            println!("{}", cli_row(&hdr_p, &hdr_r, W));
        } else {
            let hdr_p = format!("{:<12} {:>5} {:>5} {:>5}", "Process", "GFX", "DEC", "ENC");
            let hdr_r = format!(
                "{DM}{:<12} {:>5} {:>5} {:>5}{R}",
                "Process", "GFX", "DEC", "ENC"
            );
            println!("{}", cli_row(&hdr_p, &hdr_r, W));
        }

        for proc in combined.iter().take(4) {
            let name_t: String = if proc.name.chars().count() > 11 {
                format!("{}…", proc.name.chars().take(10).collect::<String>())
            } else {
                proc.name.clone()
            };
            if has_compute {
                let row_p = format!(
                    "  {:<10} {:>5} {:>5} {:>5} {:>5}",
                    name_t,
                    fmt_v(proc.gfx),
                    fmt_v(proc.dec),
                    fmt_v(proc.enc),
                    fmt_v(proc.sm)
                );
                let row_r = format!(
                    "  {PR}{:<10}{R} {WH}{:>5} {:>5} {:>5} {:>5}{R}",
                    name_t,
                    fmt_v(proc.gfx),
                    fmt_v(proc.dec),
                    fmt_v(proc.enc),
                    fmt_v(proc.sm)
                );
                println!("{}", cli_row(&row_p, &row_r, W));
            } else {
                let row_p = format!(
                    "  {:<10} {:>5} {:>5} {:>5}",
                    name_t,
                    fmt_v(proc.gfx),
                    fmt_v(proc.dec),
                    fmt_v(proc.enc)
                );
                let row_r = format!(
                    "  {PR}{:<10}{R} {WH}{:>5} {:>5} {:>5}{R}",
                    name_t,
                    fmt_v(proc.gfx),
                    fmt_v(proc.dec),
                    fmt_v(proc.enc)
                );
                println!("{}", cli_row(&row_p, &row_r, W));
            }
        }
    }

    println!("{bot}");
    let _ = io::stdout().flush();
}

pub(crate) fn run_cli_mode(interval: Duration) {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Tokio runtime başlatılamadı: {e}");
            std::process::exit(1);
        }
    };
    rt.block_on(async {
        let gpu_backend = detect_gpu();
        let mut sys = System::new();

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

        let mut cpu_tracker = PowerTracker {
            path: find_rapl_path(),
            last_energy: 0,
            last_time: Instant::now(),
        };
        if let Some(p) = cpu_tracker.path {
            cpu_tracker.last_energy = read_u64(p).unwrap_or(0);
        }
        sys.refresh_cpu_usage();
        cpu_tracker.last_time = Instant::now();
        tokio::time::sleep(SENSOR_PRIME_DELAY).await;

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

            render_cli_frame(
                cpu_watt,
                cpu_temp,
                cpu_percent,
                &gpu,
                ram_used_mb,
                ram_total_mb,
            );

            tokio::time::sleep(interval).await;
        }
    });
}
