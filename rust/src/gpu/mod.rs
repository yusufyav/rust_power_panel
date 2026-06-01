mod detect;
pub(crate) mod fdinfo;

pub(crate) use detect::detect_gpu;
pub(crate) use fdinfo::{FdInfoTracker, IntelFdInfoTracker};

use crate::sensors::read_u64;
use crate::types::{GpuData, GpuKind, GpuPowerTracker};
use nvml_wrapper::Nvml;
use std::collections::HashMap;
use std::time::Instant;
use sysinfo::{ProcessesToUpdate, System};

pub(crate) enum GpuBackend {
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
        hwmon_path: Option<String>,       // Arc GPU için hwmon
        rapl_uncore_path: Option<String>, // Entegre iGPU için RAPL uncore
    },
    None,
}

pub(crate) fn read_gpu_data(
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
                data.media_procs
                    .sort_by_key(|b| std::cmp::Reverse(b.1 + b.2));

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
                data.compute_procs.sort_by_key(|b| std::cmp::Reverse(b.1));
            }
        }
        GpuBackend::Amd {
            hwmon_path,
            device_path,
            ..
        } => {
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
        GpuBackend::Intel {
            hwmon_path,
            rapl_uncore_path,
        } => {
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
