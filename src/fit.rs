//! Will a model run on this computer? Its memory need at a given context length, against the
//! computer's memory and graphics memory. Estimates, in the way llama.cpp lays a model out:
//! the weights, the key/value cache that grows with the context, and working space.

use crate::inspect::Inspection;

/// What this computer has. Graphics memory is found where that's cheap and reliable: Apple's
/// unified memory, NVIDIA (nvidia-smi) and AMD on Linux.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hardware {
    pub ram: u64,
    /// Main memory free for a model now, without closing anything (None when unknown).
    pub ram_free: Option<u64>,
    pub gpus: Vec<Gpu>,
    /// The graphics chip shares the main memory (Apple silicon).
    pub unified: bool,
    /// The processor's name, as llama.cpp reports it ("" when unknown).
    pub cpu: String,
    /// Threads the processor runs at once.
    pub threads: usize,
    /// Instruction sets the runtime uses that this processor has (AVX2, NEON, ...).
    pub cpu_features: Vec<&'static str>,
    /// Where Filescape's own runtime (llama.cpp) can run a model, as it lists them.
    pub runtime: Vec<Device>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gpu {
    pub name: String,
    pub vram: u64,
    /// Free now, when the driver says.
    pub vram_free: Option<u64>,
}

/// One of the places llama.cpp can run a model: the processor, or a graphics chip it was built
/// for (Metal on Apple silicon).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Device {
    /// "CPU", "Metal", ...
    pub backend: String,
    /// "Apple M2", "Intel(R) Core(TM) i7-…".
    pub description: String,
    pub total: u64,
    pub free: u64,
    pub gpu: bool,
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

    /// Memory Filescape's own runtime can give a model: graphics memory where it uses the
    /// graphics chip (Apple silicon), main memory elsewhere.
    pub fn usable_here(&self) -> u64 {
        if filescape_gpu() && self.unified { self.usable_vram() } else { self.usable_ram() }
    }
}

/// Whether Filescape's own runtime puts models on the graphics chip: on Apple silicon, where
/// llama.cpp uses Metal. Elsewhere it is built for the processor only.
pub fn filescape_gpu() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
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
    let runtime = runtime_devices();
    let cpu = runtime.iter().find(|d| d.backend.eq_ignore_ascii_case("cpu")).map(|d| d.description.trim().to_string()).unwrap_or_default();
    Hardware {
        ram,
        ram_free: free_ram(),
        gpus,
        unified,
        cpu,
        threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
        cpu_features: cpu_features(),
        runtime,
    }
}

/// The name of what Filescape's runtime runs a model on: the graphics chip when `gpu`, else the
/// processor.
pub fn device_name(gpu: bool) -> String {
    runtime_devices().into_iter().find(|d| if gpu { d.gpu } else { d.backend.eq_ignore_ascii_case("cpu") }).map(|d| d.description.trim().to_string()).unwrap_or_default()
}

/// The devices llama.cpp lists. Not asked on a processor it can't run on, where its code
/// would stop the app.
fn runtime_devices() -> Vec<Device> {
    if crate::llm::processor_problem().is_some() {
        return Vec::new();
    }
    llama_cpp_2::list_llama_ggml_backend_devices()
        .into_iter()
        .map(|d| Device {
            gpu: matches!(d.device_type, llama_cpp_2::LlamaBackendDeviceType::Gpu | llama_cpp_2::LlamaBackendDeviceType::IntegratedGpu),
            backend: d.backend,
            description: d.description,
            total: d.memory_total as u64,
            free: d.memory_free as u64,
        })
        .collect()
}

/// Instruction sets that make models faster, among those this build of llama.cpp uses.
fn cpu_features() -> Vec<&'static str> {
    #[allow(unused_mut)]
    let mut v = Vec::new();
    #[cfg(target_arch = "x86_64")]
    {
        for (f, on) in [("AVX2", std::is_x86_feature_detected!("avx2")), ("FMA", std::is_x86_feature_detected!("fma")), ("F16C", std::is_x86_feature_detected!("f16c"))] {
            if on {
                v.push(f);
            }
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        v.push("NEON");
        if std::arch::is_aarch64_feature_detected!("dotprod") {
            v.push("dot product");
        }
    }
    v
}

/// Main memory free now, counting what the system would hand back from its caches.
pub fn free_ram() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let s = std::fs::read_to_string("/proc/meminfo").ok()?;
        let kb = s.lines().find(|l| l.starts_with("MemAvailable:"))?.split_whitespace().nth(1)?.parse::<u64>().ok()?;
        Some(kb * 1024)
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("/usr/bin/vm_stat").stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
        parse_vm_stat(&String::from_utf8_lossy(&out.stdout))
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        // SAFETY: the struct is plain data with its length set, as the call needs.
        unsafe {
            let mut m: MEMORYSTATUSEX = std::mem::zeroed();
            m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
            if GlobalMemoryStatusEx(&mut m) != 0 { Some(m.ullAvailPhys) } else { None }
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        None
    }
}

/// Free, inactive, speculative and purgeable pages from macOS's `vm_stat`: what a new program
/// can have without the system swapping.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_vm_stat(s: &str) -> Option<u64> {
    let page: u64 = s.lines().next()?.split("page size of ").nth(1)?.split_whitespace().next()?.parse().ok()?;
    let mut pages = 0u64;
    let mut found = false;
    for l in s.lines() {
        let Some((k, v)) = l.split_once(':') else { continue };
        if matches!(k.trim(), "Pages free" | "Pages inactive" | "Pages speculative" | "Pages purgeable") {
            pages += v.trim().trim_end_matches('.').parse::<u64>().ok()?;
            found = true;
        }
    }
    found.then_some(pages * page)
}

/// NVIDIA cards, from nvidia-smi when it's installed (it comes with the driver).
fn nvidia() -> Vec<Gpu> {
    let mut cmd = std::process::Command::new("nvidia-smi");
    cmd.args(["--query-gpu=name,memory.total,memory.free", "--format=csv,noheader,nounits"]).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let Ok(out) = cmd.output() else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (rest, free) = l.rsplit_once(',')?;
            let (name, total) = rest.rsplit_once(',')?;
            Some(Gpu { name: name.trim().to_string(), vram: total.trim().parse::<u64>().ok()? << 20, vram_free: free.trim().parse::<u64>().ok().map(|m| m << 20) })
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
            let used = std::fs::read_to_string(e.path().join("device/mem_info_vram_used")).ok().and_then(|s| s.trim().parse::<u64>().ok());
            out.push(Gpu { name: "AMD graphics".into(), vram: v, vram_free: used.map(|u| v.saturating_sub(u)) });
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

/// Where Filescape's own Ask AI and Lab would run it: on the graphics chip only where its
/// runtime uses one (Apple silicon), otherwise in main memory on the processor.
pub fn verdict_here(n: &Need, hw: &Hardware) -> Verdict {
    if filescape_gpu() {
        return verdict(n, hw, hw.usable_vram());
    }
    if n.total() <= hw.usable_ram() { Verdict::Cpu } else { Verdict::TooBig }
}

/// A quick check from the file size alone, for the library's "Fits this computer" filter:
/// the weights, working space and the cache at a short context (a few thousand tokens) fit in
/// the memory Filescape can give a model.
pub fn fits_roughly(size: u64, hw: &Hardware) -> bool {
    let need = size + size / 50 + (256 << 20) + (512 << 20);
    hw.usable_here() > 0 && need <= hw.usable_here()
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
        let pc = Hardware { ram: 32 << 30, gpus: vec![Gpu { name: "x".into(), vram: 8 << 30, vram_free: None }], unified: false, ..Default::default() };
        assert_eq!(verdict(&n, &pc, pc.usable_vram()), Verdict::Gpu);
        let small = Hardware { ram: 16 << 30, gpus: vec![Gpu { name: "x".into(), vram: 4 << 30, vram_free: None }], unified: false, ..Default::default() };
        assert!(matches!(verdict(&n, &small, small.usable_vram()), Verdict::Split(s) if s > 0.4 && s < 0.7));
        let none = Hardware { ram: 16 << 30, gpus: vec![], unified: false, ..Default::default() };
        assert_eq!(verdict(&n, &none, 0), Verdict::Cpu);
        let tiny = Hardware { ram: 8 << 30, gpus: vec![], unified: false, ..Default::default() };
        assert_eq!(verdict(&n, &tiny, 0), Verdict::TooBig);
        let mac = Hardware { ram: 16 << 30, gpus: vec![], unified: true, ..Default::default() };
        assert_eq!(verdict(&n, &mac, mac.usable_vram()), Verdict::Gpu);
        assert_eq!(contexts(Some(40960)), [512, 1024, 2048, 4096, 8192, 16384, 32768, 40960]);
        assert_eq!(tokens(40960), "40K");
        assert_eq!(tokens(131072), "128K");
    }

    #[test]
    fn filescape_itself_runs_on_the_processor_off_apple_silicon() {
        let n = Need { weights: 5 << 30, cache: 1 << 30, work: 1 << 28 };
        let pc = Hardware { ram: 32 << 30, gpus: vec![Gpu { name: "x".into(), vram: 24 << 30, vram_free: None }], unified: false, ..Default::default() };
        assert_eq!(verdict(&n, &pc, pc.usable_vram()), Verdict::Gpu);
        if !filescape_gpu() {
            assert_eq!(verdict_here(&n, &pc), Verdict::Cpu);
            assert!(fits_roughly(4 << 30, &pc));
            assert!(!fits_roughly(40 << 30, &pc));
        }
    }

    #[test]
    fn free_memory_from_vm_stat() {
        let s = "Mach Virtual Memory Statistics: (page size of 16384 bytes)\nPages free:                               10.\nPages active:                            500.\nPages inactive:                           20.\nPages speculative:                         5.\nPages purgeable:                           1.\n";
        assert_eq!(parse_vm_stat(s), Some(36 * 16384));
        assert_eq!(parse_vm_stat("nonsense"), None);
    }
}
