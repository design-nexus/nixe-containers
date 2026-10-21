//! Host CPU, memory, and engine disk samples.

use crate::engines::{docker, incus};
use crate::models::{HEALTH_POINTS, HostHealth, HostId};
use crate::prefs::RemoteProfile;
use std::collections::HashMap;

#[derive(Default)]
pub struct Sampler {
    prev_cpu: Option<(u64, u64)>,
    hist: HashMap<HostId, HostHealth>,
}

impl Sampler {
    pub async fn sample(&mut self, remote: &RemoteProfile) -> Vec<HostHealth> {
        let local = self.sample_local(remote).await;
        let mut out = vec![local];
        if remote.configured() {
            out.push(self.sample_remote(remote).await);
        }
        out
    }

    async fn sample_local(&mut self, remote: &RemoteProfile) -> HostHealth {
        let (cpu, idle_total) = cpu_percent(self.prev_cpu);
        self.prev_cpu = idle_total;
        let (mem_used, mem_total) = meminfo();
        let mut disk_used = None;
        let mut disk_total = None;
        if let Ok((used, total)) = docker::host_disk(HostId::Local, remote).await {
            disk_used = Some(used + disk_used.unwrap_or(0));
            disk_total = total.or(disk_total);
        }
        if let Ok((_, _, used, total)) = incus::host_resources(HostId::Local, remote).await {
            if let Some(u) = used {
                disk_used = Some(disk_used.unwrap_or(0) + u);
            }
            if let Some(t) = total {
                disk_total = Some(disk_total.unwrap_or(0) + t);
            }
        }
        push(self, HostId::Local, cpu, mem_used, mem_total, disk_used, disk_total)
    }

    async fn sample_remote(&mut self, remote: &RemoteProfile) -> HostHealth {
        let mut mem_used = None;
        let mut mem_total = None;
        let mut disk_used = None;
        let mut disk_total = None;
        if remote.docker_enabled {
            if let Ok(total) = docker::host_memory(HostId::Remote, remote).await {
                mem_total = total.or(mem_total);
            }
            if let Ok((used, total)) = docker::host_disk(HostId::Remote, remote).await {
                disk_used = Some(used + disk_used.unwrap_or(0));
                disk_total = total.or(disk_total);
            }
        }
        if remote.incus_enabled
            && let Ok((mu, mt, du, dt)) = incus::host_resources(HostId::Remote, remote).await
        {
            mem_used = mu.or(mem_used);
            mem_total = mt.or(mem_total);
            if let Some(u) = du {
                disk_used = Some(disk_used.unwrap_or(0) + u);
            }
            if let Some(t) = dt {
                disk_total = Some(disk_total.unwrap_or(0) + t);
            }
        }
        push(self, HostId::Remote, None, mem_used, mem_total, disk_used, disk_total)
    }
}

fn push(
    sampler: &mut Sampler,
    host: HostId,
    cpu: Option<f64>,
    mem_used: Option<u64>,
    mem_total: Option<u64>,
    disk_used: Option<u64>,
    disk_total: Option<u64>,
) -> HostHealth {
    let slot = sampler.hist.entry(host).or_insert_with(|| HostHealth {
        host,
        cpu: None,
        mem_used: None,
        mem_total: None,
        disk_used: None,
        disk_total: None,
        cpu_hist: Vec::new(),
        mem_hist: Vec::new(),
        disk_hist: Vec::new(),
    });
    slot.cpu = cpu;
    slot.mem_used = mem_used;
    slot.mem_total = mem_total;
    slot.disk_used = disk_used;
    slot.disk_total = disk_total;
    push_point(&mut slot.cpu_hist, cpu.unwrap_or(0.0) / 100.0, cpu.is_some());
    let mem_frac = match (mem_used, mem_total) {
        (Some(u), Some(t)) if t > 0 => Some(u as f64 / t as f64),
        _ => None,
    };
    push_point(&mut slot.mem_hist, mem_frac.unwrap_or(0.0), mem_frac.is_some());
    let disk_frac = match (disk_used, disk_total) {
        (Some(u), Some(t)) if t > 0 => Some(u as f64 / t as f64),
        (Some(u), None) => {
            let max = slot.disk_hist.iter().cloned().fold(u as f64, f64::max).max(1.0);
            Some(u as f64 / max)
        }
        _ => None,
    };
    push_point(&mut slot.disk_hist, disk_frac.unwrap_or(0.0), disk_frac.is_some());
    slot.clone()
}

fn push_point(hist: &mut Vec<f64>, value: f64, keep: bool) {
    if !keep {
        return;
    }
    hist.push(value.clamp(0.0, 1.0));
    if hist.len() > HEALTH_POINTS {
        hist.remove(0);
    }
}

fn cpu_percent(prev: Option<(u64, u64)>) -> (Option<f64>, Option<(u64, u64)>) {
    let Some((idle, total)) = read_cpu() else { return (None, prev) };
    let pct = prev.and_then(|(pi, pt)| {
        let di = idle.saturating_sub(pi);
        let dt = total.saturating_sub(pt);
        if dt == 0 { None } else { Some(((dt - di) as f64 / dt as f64) * 100.0) }
    });
    (pct, Some((idle, total)))
}

fn read_cpu() -> Option<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/stat").ok()?;
    let line = text.lines().next()?;
    let mut parts = line.split_whitespace();
    if parts.next() != Some("cpu") {
        return None;
    }
    let nums: Vec<u64> = parts.filter_map(|s| s.parse().ok()).collect();
    if nums.len() < 5 {
        return None;
    }
    let idle = nums[3] + nums.get(4).copied().unwrap_or(0);
    let total = nums.iter().sum();
    Some((idle, total))
}

fn meminfo() -> (Option<u64>, Option<u64>) {
    let Ok(text) = std::fs::read_to_string("/proc/meminfo") else { return (None, None) };
    let mut total = None;
    let mut avail = None;
    for line in text.lines() {
        let mut bits = line.split_whitespace();
        let key = bits.next().unwrap_or("");
        let val = bits.next().and_then(|s| s.parse::<u64>().ok());
        match key {
            "MemTotal:" => total = val.map(|n| n * 1024),
            "MemAvailable:" => avail = val.map(|n| n * 1024),
            _ => {}
        }
    }
    match (total, avail) {
        (Some(t), Some(a)) => (Some(t.saturating_sub(a)), Some(t)),
        (Some(t), None) => (None, Some(t)),
        _ => (None, None),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cpu_line_parses_idle_and_total() {
        let line = "cpu  10 0 10 80 0 0 0 0";
        let nums: Vec<u64> = line.split_whitespace().skip(1).filter_map(|s| s.parse().ok()).collect();
        let idle = nums[3] + nums[4];
        let total: u64 = nums.iter().sum();
        assert_eq!(idle, 80);
        assert_eq!(total, 100);
    }
}
