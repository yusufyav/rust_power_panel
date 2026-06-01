use super::cli::{cli_row, cli_temp_color, cli_titled_sep};
use crate::gpu::{detect_gpu, read_gpu_data, FdInfoTracker, GpuBackend, IntelFdInfoTracker};
use crate::sensors::{detect_cpu_temp_path, find_rapl_path, read_u64};
use crate::types::{usage_percent, CombinedProc, GpuData, GpuKind, GpuPowerTracker, PowerTracker};
use std::time::Instant;
use sysinfo::System;

fn render_bar(pct: u32, width: usize) -> (String, String) {
    const GN: &str = "\x1B[92m";
    const YL: &str = "\x1B[93m";
    const OR: &str = "\x1B[38;5;208m";
    const RD: &str = "\x1B[91m";
    const DM: &str = "\x1B[2m";
    const R: &str = "\x1B[0m";

    let filled = (pct as usize * width / 100).min(width);
    let color = if pct >= 75 {
        RD
    } else if pct >= 50 {
        OR
    } else if pct >= 25 {
        YL
    } else {
        GN
    };

    let plain: String = (0..width)
        .map(|i| if i < filled { '█' } else { '░' })
        .collect();

    let mut colored = String::new();
    if filled > 0 {
        colored.push_str(color);
        for _ in 0..filled {
            colored.push('█');
        }
        colored.push_str(R);
    }
    if filled < width {
        colored.push_str(DM);
        for _ in filled..width {
            colored.push('░');
        }
        colored.push_str(R);
    }

    (plain, colored)
}

pub(super) fn fmt_gb(used_mb: u32, total_mb: u32) -> String {
    let used = used_mb as f32 / 1024.0;
    let total = total_mb as f32 / 1024.0;
    if total >= 100.0 {
        format!("{:.0}/{:.0} GB", used, total) // "128/256 GB" = 10
    } else {
        format!("{:.1}/{:.0} GB", used, total) // "14.5/32 GB" = 10 | "7.8/16 GB" = 9
    }
}

fn render_tui_frame(
    cpu_watt: f32,
    cpu_temp: f32,
    cpu_percent: u32,
    gpu: &GpuData,
    ram_used_mb: u32,
    ram_total_mb: u32,
) {
    use std::io::{self, Write};

    const W: usize = 44;
    const BAR: usize = 28;

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

    // pct → right-justified 10-char value string (plain + colored)
    let pct_val = |pct: u32, color: &str| -> (String, String) {
        let s = format!("{:>3}%", pct); // always 4 visible chars
        let pad = " ".repeat(10usize.saturating_sub(s.chars().count()));
        let plain = format!("{}{}", pad, s);
        let colored = format!("{}{}{}{}", pad, color, s, R);
        (plain, colored)
    };

    // "  —" right-justified in 10 chars
    let dash_val = || -> (String, String) {
        let s = "  —";
        let pad = " ".repeat(10usize.saturating_sub(s.chars().count()));
        let plain = format!("{}{}", pad, s);
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
    let (gpu_val_p, gpu_val_r) = if gpu_has_pct {
        pct_val(gpu_pct, GN)
    } else {
        dash_val()
    };
    let gpu_p = format!("GPU  {} {}", gpu_bar_p, gpu_val_p);
    let gpu_r = format!("{GN}GPU{R}  {} {}", gpu_bar_r, gpu_val_r);
    println!("{}", cli_row(&gpu_p, &gpu_r, W));

    // ── RAM bar ────────────────────────────────────────────────────────────
    if ram_total_mb > 0 {
        let ram_pct = usage_percent(ram_used_mb, ram_total_mb).min(100);
        let (ram_bar_p, ram_bar_r) = render_bar(ram_pct, BAR);
        let ram_val = fmt_gb(ram_used_mb, ram_total_mb);
        let ram_p = format!("RAM  {} {}", ram_bar_p, ram_val);
        let ram_r = format!("{YL}RAM{R}  {} {BL}{}{R}", ram_bar_r, ram_val);
        println!("{}", cli_row(&ram_p, &ram_r, W));
    }

    // ── VRAM bar ───────────────────────────────────────────────────────────
    if gpu.vram_total_mb > 0 {
        let vram_pct = usage_percent(gpu.vram_used_mb, gpu.vram_total_mb).min(100);
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
        cpu_temp.floor(),
        cpu_watt,
        gpu.temp.floor(),
        gpu.watt
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

        let combined = CombinedProc::from_gpu(gpu);

        let fmt_v = |v: Option<u32>| -> String {
            match v {
                Some(x) if x > 0 => format!("{:>3}%", x),
                _ => "  —".to_string(),
            }
        };

        // W=44: with SM    → {:<12} {:>7} {:>7} {:>7} {:>7} = 12+1+7+1+7+1+7+1+7 = 44
        //        without SM → {:<14} {:>9} {:>9} {:>9}       = 14+1+9+1+9+1+9     = 44
        if has_compute {
            let hdr_p = format!(
                "{:<12} {:>7} {:>7} {:>7} {:>7}",
                "Process", "GFX", "DEC", "ENC", "SM%"
            );
            let hdr_r = format!(
                "{DM}{:<12} {:>7} {:>7} {:>7} {:>7}{R}",
                "Process", "GFX", "DEC", "ENC", "SM%"
            );
            println!("{}", cli_row(&hdr_p, &hdr_r, W));
        } else {
            let hdr_p = format!("{:<14} {:>9} {:>9} {:>9}", "Process", "GFX", "DEC", "ENC");
            let hdr_r = format!(
                "{DM}{:<14} {:>9} {:>9} {:>9}{R}",
                "Process", "GFX", "DEC", "ENC"
            );
            println!("{}", cli_row(&hdr_p, &hdr_r, W));
        }

        for proc in combined.iter().take(4) {
            if has_compute {
                let name_t: String = if proc.name.chars().count() > 11 {
                    format!("{}…", proc.name.chars().take(10).collect::<String>())
                } else {
                    proc.name.clone()
                };
                let row_p = format!(
                    "  {:<10} {:>7} {:>7} {:>7} {:>7}",
                    name_t,
                    fmt_v(proc.gfx),
                    fmt_v(proc.dec),
                    fmt_v(proc.enc),
                    fmt_v(proc.sm)
                );
                let row_r = format!(
                    "  {PR}{:<10}{R} {WH}{:>7} {:>7} {:>7} {:>7}{R}",
                    name_t,
                    fmt_v(proc.gfx),
                    fmt_v(proc.dec),
                    fmt_v(proc.enc),
                    fmt_v(proc.sm)
                );
                println!("{}", cli_row(&row_p, &row_r, W));
            } else {
                let name_t: String = if proc.name.chars().count() > 13 {
                    format!("{}…", proc.name.chars().take(12).collect::<String>())
                } else {
                    proc.name.clone()
                };
                let row_p = format!(
                    "  {:<12} {:>9} {:>9} {:>9}",
                    name_t,
                    fmt_v(proc.gfx),
                    fmt_v(proc.dec),
                    fmt_v(proc.enc)
                );
                let row_r = format!(
                    "  {PR}{:<12}{R} {WH}{:>9} {:>9} {:>9}{R}",
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
    io::stdout().flush().unwrap();
}

pub(crate) fn run_tui_mode() {
    let rt = tokio::runtime::Runtime::new().unwrap();
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

            render_tui_frame(
                cpu_watt,
                cpu_temp,
                cpu_percent,
                &gpu,
                ram_used_mb,
                ram_total_mb,
            );

            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    });
}
