mod diagnostics;
mod gpu;
mod sensors;
mod types;
mod ui;

use gtk4::glib;

pub(crate) const APP_ID: &str = "com.github.yusufyav.power_panel";

fn main() -> glib::ExitCode {
    let args: Vec<String> = std::env::args().collect();

    if args.len() > 1 {
        match args[1].as_str() {
            "--help" | "-h" => {
                print_help();
                return glib::ExitCode::SUCCESS;
            }
            "--cli" => {
                ui::run_cli_mode();
                return glib::ExitCode::SUCCESS;
            }
            "--tui" => {
                ui::run_tui_mode();
                return glib::ExitCode::SUCCESS;
            }
            "--gui2" => {
                return ui::run_gui2(&args);
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

    ui::run_gui()
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
