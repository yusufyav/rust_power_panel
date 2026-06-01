use std::fs;

pub(crate) fn read_u64(path: &str) -> Result<u64, std::io::Error> {
    let s = fs::read_to_string(path)?;
    s.trim()
        .parse::<u64>()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

pub(crate) fn find_rapl_path() -> Option<&'static str> {
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

pub(crate) fn detect_cpu_temp_path() -> Option<String> {
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

            let score = match name.as_str() {
                "k10temp" => 100,
                "coretemp" => 95,
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
