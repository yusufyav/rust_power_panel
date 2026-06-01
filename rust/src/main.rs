mod diagnostics;
mod gpu;
mod sensors;
mod types;
mod ui;

use gtk4::glib;
use std::time::Duration;

pub(crate) const APP_ID: &str = "com.github.yusufyav.power_panel";

const DEFAULT_INTERVAL_SECS: f64 = 2.0;
const MIN_INTERVAL_SECS: f64 = 0.1;
const MAX_INTERVAL_SECS: f64 = 3600.0;

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let (args, interval) = strip_interval_args(&args);

    if args.len() > 1 {
        match args[1].as_str() {
            "--help" | "-h" => {
                print_help();
                return glib::ExitCode::SUCCESS;
            }
            "--cli" => {
                ui::run_cli_mode(interval);
                return glib::ExitCode::SUCCESS;
            }
            "--tui" => {
                ui::run_tui_mode(interval);
                return glib::ExitCode::SUCCESS;
            }
            "--gui2" => {
                return ui::run_gui2(interval, &args);
            }
            "--debug" => {
                let rapl_path = sensors::find_rapl_path();
                let gpu = gpu::detect_gpu();
                diagnostics::run_diagnostics(&rapl_path, &gpu);
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

    ui::run_gui(interval, &args)
}

fn strip_interval_args(args: &[String]) -> (Vec<String>, Duration) {
    let mut filtered = Vec::with_capacity(args.len());
    if let Some(argv0) = args.first() {
        filtered.push(argv0.clone());
    }

    let mut interval_secs = DEFAULT_INTERVAL_SECS;
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--interval" {
            match args.get(i + 1) {
                Some(value) if is_control_arg(value) => {
                    eprintln!(
                        "Geçersiz --interval değeri; varsayılan {DEFAULT_INTERVAL_SECS:.1} sn kullanılacak"
                    );
                    interval_secs = DEFAULT_INTERVAL_SECS;
                    i += 1;
                }
                Some(value) => {
                    interval_secs = parse_interval_secs(value);
                    i += 2;
                }
                None => {
                    eprintln!(
                        "Geçersiz --interval değeri; varsayılan {DEFAULT_INTERVAL_SECS:.1} sn kullanılacak"
                    );
                    interval_secs = DEFAULT_INTERVAL_SECS;
                    i += 1;
                }
            }
        } else {
            filtered.push(args[i].clone());
            i += 1;
        }
    }

    (filtered, Duration::from_secs_f64(interval_secs))
}

fn is_control_arg(value: &str) -> bool {
    matches!(
        value,
        "--help"
            | "-h"
            | "--version"
            | "-v"
            | "--cli"
            | "--tui"
            | "--gui2"
            | "--debug"
            | "--interval"
    )
}

fn parse_interval_secs(value: &str) -> f64 {
    match value.parse::<f64>() {
        Ok(parsed) if parsed.is_finite() && parsed > 0.0 => {
            parsed.clamp(MIN_INTERVAL_SECS, MAX_INTERVAL_SECS)
        }
        _ => {
            eprintln!(
                "Geçersiz --interval değeri; varsayılan {DEFAULT_INTERVAL_SECS:.1} sn kullanılacak"
            );
            DEFAULT_INTERVAL_SECS
        }
    }
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
    println!("  --interval <saniye>  Güncelleme aralığı (varsayılan: 2.0)");
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
