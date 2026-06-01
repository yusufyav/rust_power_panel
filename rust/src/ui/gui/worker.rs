#[cfg(debug_assertions)]
use crate::diagnostics::run_diagnostics;
use crate::gpu::{detect_gpu, read_gpu_data, FdInfoTracker, GpuBackend, IntelFdInfoTracker};
use crate::sensors::{detect_cpu_temp_path, find_rapl_path, read_u64};
use crate::types::{GpuPowerTracker, PowerTracker, SensorData};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use sysinfo::{Components, System};

const SENSOR_PRIME_DELAY: Duration = Duration::from_millis(250);

// ── Sensör thread (her iki stil için ortak) ──────────────────────────────────

pub(super) fn spawn_sensor_thread(data_writer: Arc<Mutex<SensorData>>, interval: Duration) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("Tokio runtime başlatılamadı: {e}");
                return;
            }
        };
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
            sys.refresh_cpu_usage();
            tracker.last_time = Instant::now();

            // TEŞHİS MOTORUNU ÇALIŞTIRIYORUZ
            #[cfg(debug_assertions)]
            run_diagnostics(&tracker.path, &gpu_backend);

            // CPU sıcaklık sensörünü kernel'den direkt okumak için
            let cpu_temp_path = detect_cpu_temp_path();

            tokio::time::sleep(SENSOR_PRIME_DELAY).await;

            let gfx_sample_interval = Duration::from_millis(200);
            let mut last_gfx_sample: Option<Instant> = None;
            let mut last_full_read: Option<Instant> = None;
            let mut gfx_max: u32 = 0;

            loop {
                let now = Instant::now();

                // Quick GFX sample every 200ms — catches bursts that slower polling misses.
                if last_gfx_sample
                    .is_none_or(|last| now.saturating_duration_since(last) >= gfx_sample_interval)
                {
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
                    last_gfx_sample = Some(now);
                }

                // Full sensor read: immediately on first tick, then at the configured interval.
                if last_full_read.is_none_or(|last| now.saturating_duration_since(last) >= interval)
                {
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
                                        if t > temp {
                                            temp = t;
                                        }
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

                    let mut gpu = read_gpu_data(
                        &gpu_backend,
                        &mut sys,
                        &mut intel_gpu_tracker,
                        &mut amd_fdinfo_tracker,
                        &mut intel_fdinfo_tracker,
                    );

                    // Use max GFX seen over the current full-read interval.
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
                    last_full_read = Some(now);
                }

                let now = Instant::now();
                let next_gfx_sample = last_gfx_sample
                    .map(|last| {
                        gfx_sample_interval.saturating_sub(now.saturating_duration_since(last))
                    })
                    .unwrap_or(Duration::ZERO);
                let next_full_read = last_full_read
                    .map(|last| interval.saturating_sub(now.saturating_duration_since(last)))
                    .unwrap_or(Duration::ZERO);
                let sleep_duration = next_gfx_sample.min(next_full_read);
                if !sleep_duration.is_zero() {
                    tokio::time::sleep(sleep_duration).await;
                }
            }
        });
    });
}
