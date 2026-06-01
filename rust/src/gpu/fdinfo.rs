use crate::types::MediaInfo;
use std::collections::HashMap;
use std::fs;
use std::time::Instant;

fn parse_fdinfo_ns(line: &str) -> u64 {
    line.split_whitespace()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

pub(crate) struct FdInfoTracker {
    pub(crate) prev: HashMap<u64, (u64, u64, u64, Instant)>, // (dec_ns, enc_ns, gfx_ns, time)
    pub(crate) pdev: String,
    pub(crate) vcn_instances: u32,
}

impl FdInfoTracker {
    pub(crate) fn new(pdev: String, vcn_instances: u32) -> Self {
        Self {
            prev: HashMap::new(),
            pdev,
            vcn_instances,
        }
    }

    pub(crate) fn sample(&mut self) -> MediaInfo {
        let now = Instant::now();
        let mut current: HashMap<u64, (String, u64, u64, u64, u32, u32)> = HashMap::new();

        let Ok(proc_dir) = fs::read_dir("/proc") else {
            return MediaInfo::default();
        };

        for entry in proc_dir.flatten() {
            let fname = entry.file_name();
            let pid_str = fname.to_string_lossy();
            let Ok(pid) = pid_str.parse::<u32>() else {
                continue;
            };

            let fd_path = format!("/proc/{}/fd", pid);
            let Ok(fd_dir) = fs::read_dir(&fd_path) else {
                continue;
            };

            let mut proc_name = String::new();

            for fd_entry in fd_dir.flatten() {
                let fd_num = fd_entry.file_name();
                let fdinfo_path = format!("/proc/{}/fdinfo/{}", pid, fd_num.to_string_lossy());
                let Ok(content) = fs::read_to_string(&fdinfo_path) else {
                    continue;
                };

                if !content.contains("amdgpu") {
                    continue;
                }
                if !self.pdev.is_empty() && !content.contains(&self.pdev) {
                    continue;
                }

                let mut client_id = None;
                let mut fd_dec: u64 = 0;
                let mut fd_enc: u64 = 0;
                let mut fd_gfx: u64 = 0;
                let mut cap_dec: u32 = 0;
                let mut cap_enc: u32 = 0;

                for line in content.lines() {
                    if line.starts_with("drm-client-id:") {
                        client_id = Some(parse_fdinfo_ns(line));
                    } else if line.starts_with("drm-engine-dec:") {
                        fd_dec = fd_dec.max(parse_fdinfo_ns(line));
                    } else if line.starts_with("drm-engine-enc:") {
                        fd_enc = fd_enc.max(parse_fdinfo_ns(line));
                    } else if line.starts_with("drm-engine-gfx:") {
                        fd_gfx = fd_gfx.max(parse_fdinfo_ns(line));
                    } else if line.starts_with("drm-engine-capacity-dec:") {
                        cap_dec = parse_fdinfo_ns(line) as u32;
                    } else if line.starts_with("drm-engine-capacity-enc:") {
                        cap_enc = parse_fdinfo_ns(line) as u32;
                    }
                }

                let cid = client_id.unwrap_or(pid as u64);
                let final_cap_dec = if cap_dec > 0 {
                    cap_dec
                } else {
                    self.vcn_instances
                };
                let final_cap_enc = if cap_enc > 0 {
                    cap_enc
                } else {
                    self.vcn_instances
                };

                current
                    .entry(cid)
                    .and_modify(|e| {
                        e.1 = e.1.max(fd_dec);
                        e.2 = e.2.max(fd_enc);
                        e.3 = e.3.max(fd_gfx);
                    })
                    .or_insert_with(|| {
                        if proc_name.is_empty() {
                            proc_name = fs::read_to_string(format!("/proc/{}/comm", pid))
                                .unwrap_or_default()
                                .trim()
                                .to_string();
                        }
                        (
                            proc_name.clone(),
                            fd_dec,
                            fd_enc,
                            fd_gfx,
                            final_cap_dec,
                            final_cap_enc,
                        )
                    });
            }
        }

        let mut media_list: Vec<(String, u32, u32, u32)> = Vec::new();

        for (cid, (name, dec_ns, enc_ns, gfx_ns, cap_dec, cap_enc)) in &current {
            if let Some(&(prev_dec, prev_enc, prev_gfx, prev_t)) = self.prev.get(cid) {
                let elapsed = now.duration_since(prev_t).as_nanos() as u64;
                if elapsed == 0 {
                    continue;
                }

                let dec_d = dec_ns.saturating_sub(prev_dec);
                let enc_d = enc_ns.saturating_sub(prev_enc);
                let gfx_d = gfx_ns.saturating_sub(prev_gfx);

                let dec_p = (((dec_d as f64 / elapsed as f64) * 100.0) as u32) / cap_dec;
                let enc_p = (((enc_d as f64 / elapsed as f64) * 100.0) as u32) / cap_enc;
                let gfx_p = ((gfx_d as f64 / elapsed as f64) * 100.0) as u32;

                if dec_p > 0 || enc_p > 0 || gfx_p > 0 {
                    media_list.push((name.clone(), dec_p, enc_p, gfx_p));
                }
            }
        }

        self.prev.clear();
        for (cid, (_, dec_ns, enc_ns, gfx_ns, _, _)) in &current {
            self.prev.insert(*cid, (*dec_ns, *enc_ns, *gfx_ns, now));
        }

        media_list.sort_by_key(|b| std::cmp::Reverse(b.1 + b.2 + b.3));

        MediaInfo {
            media_procs: media_list,
        }
    }
}

pub(crate) struct IntelFdInfoTracker {
    pub(crate) prev: HashMap<u64, (u64, u64, Instant)>, // (video_ns, render_ns, time)
}

impl IntelFdInfoTracker {
    pub(crate) fn new() -> Self {
        Self {
            prev: HashMap::new(),
        }
    }

    pub(crate) fn sample(&mut self) -> MediaInfo {
        let now = Instant::now();
        let mut current: HashMap<u64, (String, u64, u64)> = HashMap::new();

        let Ok(proc_dir) = fs::read_dir("/proc") else {
            return MediaInfo::default();
        };

        for entry in proc_dir.flatten() {
            let fname = entry.file_name();
            let pid_str = fname.to_string_lossy();
            let Ok(pid) = pid_str.parse::<u32>() else {
                continue;
            };

            let fd_path = format!("/proc/{}/fd", pid);
            let Ok(fd_dir) = fs::read_dir(&fd_path) else {
                continue;
            };

            let mut proc_name = String::new();

            for fd_entry in fd_dir.flatten() {
                let fd_num = fd_entry.file_name();
                let fdinfo_path = format!("/proc/{}/fdinfo/{}", pid, fd_num.to_string_lossy());
                let Ok(content) = fs::read_to_string(&fdinfo_path) else {
                    continue;
                };

                // Intel GPU: i915 veya xe sürücüsü
                if !content.contains("i915") && !content.contains("xe") {
                    continue;
                }

                let mut client_id = None;
                let mut video_ns: u64 = 0;
                let mut render_ns: u64 = 0;

                for line in content.lines() {
                    if line.starts_with("drm-client-id:") {
                        client_id = Some(parse_fdinfo_ns(line));
                    } else if line.starts_with("drm-engine-video:") {
                        video_ns = video_ns.max(parse_fdinfo_ns(line));
                    } else if line.starts_with("drm-engine-render:") {
                        render_ns = render_ns.max(parse_fdinfo_ns(line));
                    }
                }

                if video_ns == 0 && render_ns == 0 {
                    continue;
                }

                let cid = client_id.unwrap_or(pid as u64);

                current
                    .entry(cid)
                    .and_modify(|e| {
                        e.1 = e.1.max(video_ns);
                        e.2 = e.2.max(render_ns);
                    })
                    .or_insert_with(|| {
                        if proc_name.is_empty() {
                            proc_name = fs::read_to_string(format!("/proc/{}/comm", pid))
                                .unwrap_or_default()
                                .trim()
                                .to_string();
                        }
                        (proc_name.clone(), video_ns, render_ns)
                    });
            }
        }

        let mut media_list: Vec<(String, u32, u32, u32)> = Vec::new();

        for (cid, (name, video_ns, render_ns)) in &current {
            if let Some(&(prev_video, prev_render, prev_t)) = self.prev.get(cid) {
                let elapsed = now.duration_since(prev_t).as_nanos() as u64;
                if elapsed == 0 {
                    continue;
                }

                let video_d = video_ns.saturating_sub(prev_video);
                let render_d = render_ns.saturating_sub(prev_render);
                let video_p = ((video_d as f64 / elapsed as f64) * 100.0) as u32;
                let render_p = ((render_d as f64 / elapsed as f64) * 100.0) as u32;

                if video_p > 0 || render_p > 0 {
                    media_list.push((name.clone(), video_p, 0, render_p));
                }
            }
        }

        self.prev.clear();
        for (cid, (_, video_ns, render_ns)) in &current {
            self.prev.insert(*cid, (*video_ns, *render_ns, now));
        }

        media_list.sort_by_key(|b| std::cmp::Reverse(b.1));

        MediaInfo {
            media_procs: media_list,
        }
    }
}
