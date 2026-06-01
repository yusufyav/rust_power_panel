use super::GpuBackend;
use nvml_wrapper::Nvml;
use std::fs;

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
    let base_paths = [
        "/sys/class/powercap/intel-rapl/intel-rapl:0",
        "/sys/class/powercap/intel-rapl:0",
    ];

    for base in &base_paths {
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

pub(crate) fn detect_gpu() -> GpuBackend {
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
                        let device_path = format!("/sys/class/drm/card{}/device", card_idx);
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
            let driver_path = format!("/sys/class/drm/card{}/device/driver", card_idx);
            let driver_name = fs::read_link(&driver_path)
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_default();

            if !matches!(driver_name.as_str(), "i915" | "xe") {
                continue;
            }

            let hwmon_path = find_intel_gpu_hwmon();
            let rapl_uncore_path = find_intel_rapl_uncore();

            return GpuBackend::Intel {
                hwmon_path,
                rapl_uncore_path,
            };
        }
    }

    GpuBackend::None
}
