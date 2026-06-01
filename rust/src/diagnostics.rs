use crate::gpu::GpuBackend;
use crate::sensors::{detect_cpu_temp_path, read_u64};
use std::fs;

pub(crate) fn run_diagnostics(rapl_path: &Option<&'static str>, gpu: &GpuBackend) {
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
        GpuBackend::Intel {
            hwmon_path,
            rapl_uncore_path,
        } => {
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
