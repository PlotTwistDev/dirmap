//! Will a model run on this computer? Its memory need at a given context length, against the
//! computer's memory and graphics memory. Estimates, in the way llama.cpp lays a model out:
//! the weights, the key/value cache that grows with the context, and working space.

use crate::inspect::Inspection;

/// What this computer has. Graphics memory is found where that's cheap and reliable: Apple's
/// unified memory, NVIDIA (nvidia-smi) and AMD on Linux.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hardware {
    pub ram: u64,
    pub gpus: Vec<Gpu>,
    /// The graphics chip shares the main memory (Apple silicon).
    pub unified: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gpu {
    pub name: String,
    pub vram: u64,
}

impl Hardware {
    /// Graphics memory a model can use: most of it, less what the screen and other apps need.
    pub fn usable_vram(&self) -> u64 {
        if self.unified {
            // macOS lets the GPU use about three quarters of memory by default.
            self.ram / 4 * 3
        } else {
            self.gpus.iter().map(|g| g.vram).max().map_or(0, |v| v.saturating_sub(v / 10).saturating_sub(300 << 20))
        }
    }

    /// Main memory a model can use, leaving room for the system and other apps.
    pub fn usable_ram(&self) -> u64 {
        self.ram.saturating_sub((self.ram / 5).max(2 << 30))
    }
}

pub fn detect() -> Hardware {
    let ram = crate::player::total_ram();
    let unified = cfg!(all(target_os = "macos", target_arch = "aarch64"));
    let mut gpus = Vec::new();
    if !unified {
        gpus.extend(nvidia());
        #[cfg(target_os = "linux")]
        gpus.extend(amd_linux());
    }
    Hardware { ram, gpus, unified }
}

/// NVIDIA cards, from nvidia-smi when it's installed (it comes with the driver).
fn nvidia() -> Vec<Gpu> {
    let mut cmd = std::process::Command::new("nvidia-smi");
    cmd.args(["--query-gpu=name,memory.total", "--format=csv,noheader,nounits"]).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let Ok(out) = cmd.output() else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (name, mib) = l.rsplit_once(',')?;
            Some(Gpu { name: name.trim().to_string(), vram: mib.trim().parse::<u64>().ok()? << 20 })
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn amd_linux() -> Vec<Gpu> {
    let mut out = Vec::new();
    for e in std::fs::read_dir("/sys/class/drm").into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        if let Some(v) = std::fs::read_to_string(e.path().join("device/mem_info_vram_total")).ok().and_then(|s| s.trim().parse::<u64>().ok())
            && v > 512 << 20
        {
            out.push(Gpu { name: "AMD graphics".into(), vram: v });
        }
    }
    out
}

/// Memory a model needs at one context length.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Need {
    pub weights: u64,
    pub cache: u64,
    pub work: u64,
}

impl Need {
    pub fn total(&self) -> u64 {
        self.weights + self.cache + self.work
    }
}

/// The memory to run the model with `context` tokens, keeping the cache at `cache_bytes` per
/// number (2 for F16, about 1.06 for Q8_0). None when the model's dimensions aren't known.
pub fn need(d: &Inspection, weights: u64, context: u64, cache_bytes: f64) -> Option<Need> {
    let per_token = d.shape.kv_per_token(cache_bytes)?;
    let cache = (per_token * context as f64) as u64;
    // Working space: llama.cpp's compute buffers grow with the batch and the vocabulary; a
    // few hundred MB covers typical use.
    let work = (256u64 << 20) + weights / 50;
    Some(Need { weights, cache, work })
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Verdict {
    /// All on the graphics chip: fast.
    Gpu,
    /// Split between graphics and main memory; the share on the graphics chip.
    Split(f32),
    /// In main memory, on the processor: works, slower.
    Cpu,
    /// More than this computer has.
    TooBig,
}

pub fn verdict(n: &Need, hw: &Hardware, vram: u64) -> Verdict {
    let total = n.total();
    if vram > 0 && total <= vram {
        return Verdict::Gpu;
    }
    let ram = hw.usable_ram();
    if hw.unified {
        return if total <= ram { Verdict::Cpu } else { Verdict::TooBig };
    }
    if vram > 0 && total <= vram + ram {
        return Verdict::Split((vram as f64 / total as f64) as f32);
    }
    if total <= ram { Verdict::Cpu } else { Verdict::TooBig }
}

/// Context lengths to offer, up to the model's own longest.
pub fn contexts(max: Option<u64>) -> Vec<u64> {
    let max = max.unwrap_or(8192).max(512);
    let mut v: Vec<u64> = [512u64, 1024, 2048, 4096, 8192, 16384, 32768, 65536, 131072, 262144, 524288, 1048576].into_iter().filter(|&c| c < max).collect();
    v.push(max);
    v
}

/// "4K", "128K", "1M" tokens.
pub fn tokens(n: u64) -> String {
    if n >= 1 << 20 && n % (1 << 20) == 0 {
        format!("{}M", n >> 20)
    } else if n >= 1024 && n % 1024 == 0 {
        format!("{}K", n / 1024)
    } else {
        crate::inspect::group(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspect::Shape;

    fn llama8b() -> Inspection {
        Inspection {
            shape: Shape { layers: Some(32), hidden: Some(4096), heads: Some(32), kv_heads: Some(8), head_dim: None, context: Some(131072) },
            ..Default::default()
        }
    }

    #[test]
    fn cache_grows_with_context() {
        // Llama 3 8B: 2 × 32 layers × 8 heads × 128 × 2 bytes = 128 KiB per token.
        let d = llama8b();
        assert_eq!(d.shape.kv_per_token(2.0), Some(131072.0));
        let n = need(&d, 4_900_000_000, 8192, 2.0).unwrap();
        assert_eq!(n.cache, 1 << 30);
        assert!(n.total() > n.weights + n.cache);
        assert!(need(&Inspection::default(), 1, 1, 2.0).is_none());
    }

    #[test]
    fn verdicts() {
        let n = Need { weights: 5 << 30, cache: 1 << 30, work: 1 << 28 };
        let pc = Hardware { ram: 32 << 30, gpus: vec![Gpu { name: "x".into(), vram: 8 << 30 }], unified: false };
        assert_eq!(verdict(&n, &pc, pc.usable_vram()), Verdict::Gpu);
        let small = Hardware { ram: 16 << 30, gpus: vec![Gpu { name: "x".into(), vram: 4 << 30 }], unified: false };
        assert!(matches!(verdict(&n, &small, small.usable_vram()), Verdict::Split(s) if s > 0.4 && s < 0.7));
        let none = Hardware { ram: 16 << 30, gpus: vec![], unified: false };
        assert_eq!(verdict(&n, &none, 0), Verdict::Cpu);
        let tiny = Hardware { ram: 8 << 30, gpus: vec![], unified: false };
        assert_eq!(verdict(&n, &tiny, 0), Verdict::TooBig);
        let mac = Hardware { ram: 16 << 30, gpus: vec![], unified: true };
        assert_eq!(verdict(&n, &mac, mac.usable_vram()), Verdict::Gpu);
        assert_eq!(contexts(Some(40960)), [512, 1024, 2048, 4096, 8192, 16384, 32768, 40960]);
        assert_eq!(tokens(40960), "40K");
        assert_eq!(tokens(131072), "128K");
    }
}
