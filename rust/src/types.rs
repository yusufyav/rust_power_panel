use std::time::Instant;

#[derive(Debug, Default, Clone)]
pub(crate) struct MediaInfo {
    pub(crate) media_procs: Vec<(String, u32, u32, u32)>, // (name, dec%, enc%, gfx%)
}

pub(crate) struct GpuData {
    pub(crate) temp: f32,
    pub(crate) watt: f32,
    pub(crate) media_procs: Vec<(String, u32, u32, u32)>, // (name, dec%, enc%, gfx%)
    pub(crate) compute_procs: Vec<(String, u32)>,
    pub(crate) kind: GpuKind,
    pub(crate) vram_used_mb: u32,
    pub(crate) vram_total_mb: u32,
    pub(crate) gfx_percent: u32,
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
pub(crate) struct SensorData {
    pub(crate) cpu_temp: f32,
    pub(crate) cpu_watt: f32,
    pub(crate) gpu_temp: f32,
    pub(crate) gpu_watt: f32,
    pub(crate) media_procs: Vec<(String, u32, u32, u32)>, // (name, dec%, enc%, gfx%)
    pub(crate) compute_procs: Vec<(String, u32)>,
    pub(crate) gpu_kind: GpuKind,
    pub(crate) vram_used_mb: u32,
    pub(crate) vram_total_mb: u32,
    pub(crate) gpu_gfx_percent: u32,
    pub(crate) cpu_percent: u32,
    pub(crate) ram_used_mb: u32,
    pub(crate) ram_total_mb: u32,
}

#[derive(Clone, Default, PartialEq)]
pub(crate) enum GpuKind {
    #[default]
    Unknown,
    Nvidia,
    Amd,
    Intel,
}

pub(crate) struct PowerTracker {
    pub(crate) last_energy: u64,
    pub(crate) last_time: Instant,
    pub(crate) path: Option<&'static str>,
}

pub(crate) struct GpuPowerTracker {
    pub(crate) last_energy: u64,
    pub(crate) last_time: Instant,
}

pub(crate) struct CombinedProc {
    pub(crate) name: String,
    pub(crate) gfx: Option<u32>,
    pub(crate) dec: Option<u32>,
    pub(crate) enc: Option<u32>,
    pub(crate) sm: Option<u32>,
}

impl CombinedProc {
    pub(crate) fn from_gpu(gpu: &GpuData) -> Vec<Self> {
        Self::from_processes(&gpu.media_procs, &gpu.compute_procs)
    }

    pub(crate) fn from_sensor(data: &SensorData) -> Vec<Self> {
        Self::from_processes(&data.media_procs, &data.compute_procs)
    }

    fn from_processes(
        media_procs: &[(String, u32, u32, u32)],
        compute_procs: &[(String, u32)],
    ) -> Vec<Self> {
        let mut combined = Vec::new();
        for (name, dec, enc, gfx) in media_procs {
            combined.push(Self {
                name: name.clone(),
                gfx: positive(*gfx),
                dec: positive(*dec),
                enc: positive(*enc),
                sm: None,
            });
        }
        for (name, sm) in compute_procs {
            let sm = positive(*sm);
            if let Some(entry) = combined.iter_mut().find(|entry| entry.name == *name) {
                entry.sm = sm;
            } else {
                combined.push(Self {
                    name: name.clone(),
                    gfx: None,
                    dec: None,
                    enc: None,
                    sm,
                });
            }
        }
        combined
    }
}

pub(crate) fn usage_percent(used: u32, total: u32) -> u32 {
    used.saturating_mul(100).checked_div(total).unwrap_or(0)
}

fn positive(value: u32) -> Option<u32> {
    (value > 0).then_some(value)
}
