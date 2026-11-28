//! Shrinking (quantising) a GGUF model into a new file with llama.cpp, keeping the original.
//!
//! llama.cpp does the work in a copy of Filescape started for just that (`--quantize-worker`):
//! a model it can't handle can stop that copy without taking the app with it, Stop simply ends
//! it, and the progress it prints ("[ 12/ 291] …", one line per tensor) is read from its output.
//! The result is written under a temporary name and named only once llama.cpp says it's done, so
//! a stopped or failed run never leaves a half-written model that looks finished. The original
//! is only ever read.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// A size to shrink to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub name: &'static str,
    /// llama.cpp's `llama_ftype`.
    ftype: u32,
    /// Bits per weight, roughly, for the size estimate.
    pub bits: f32,
    pub about: &'static str,
}

pub const TARGETS: [Target; 6] = [
    Target { name: "Q8_0", ftype: 7, bits: 8.5, about: "Barely different from the original; half the size of 16-bit" },
    Target { name: "Q6_K", ftype: 18, bits: 6.6, about: "Very close to the original" },
    Target { name: "Q5_K_M", ftype: 17, bits: 5.7, about: "Close to the original" },
    Target { name: "Q4_K_M", ftype: 15, bits: 4.9, about: "The usual choice: a quarter of 16-bit, a little worse" },
    Target { name: "Q3_K_M", ftype: 12, bits: 3.9, about: "Smaller; noticeably worse" },
    Target { name: "Q2_K", ftype: 10, bits: 3.0, about: "Smallest; much worse, for trying out only" },
];

pub fn target(name: &str) -> Option<Target> {
    TARGETS.iter().copied().find(|t| t.name.eq_ignore_ascii_case(name))
}

/// Bits per weight of a model's present precision, if known ("F16", "Q4_K_M"…).
pub fn bits_of(quant: &str) -> Option<f32> {
    let q = quant.to_ascii_uppercase();
    match q.as_str() {
        "F32" => Some(32.0),
        "F16" | "BF16" => Some(16.0),
        _ => target(&q).map(|t| t.bits).or_else(|| {
            // Other quantised kinds: the digit after Q or IQ is close enough.
            let d = q.trim_start_matches('I').strip_prefix('Q')?.chars().next()?.to_digit(10)?;
            Some(d as f32 + 0.5)
        }),
    }
}

/// Whether the model is already quantised (shrinking again loses more than shrinking the
/// original would).
pub fn already_shrunk(quant: Option<&str>) -> bool {
    quant.and_then(bits_of).is_some_and(|b| b < 16.0)
}

/// The new file's size, roughly: the original scaled by bits per weight. Unknown precision is
/// taken as 16-bit.
pub fn estimate(bytes: u64, quant: Option<&str>, t: Target) -> u64 {
    let from = quant.and_then(bits_of).unwrap_or(16.0);
    (bytes as f64 * (t.bits as f64 / from as f64).min(1.0)) as u64
}

/// The new file's name: the original's, with its precision tag replaced (or added).
/// `name` is used instead of the file's when the file is named by its hash (Ollama's and Hugging
/// Face's stores): "org/name (Q8_0)" or "qwen3:8b".
pub fn output_name(input: &Path, name: Option<&str>, quant: Option<&str>, t: Target) -> String {
    let stem = match name {
        Some(n) => {
            let n = n.split(" (").next().unwrap_or(n);
            let n = n.rsplit('/').next().unwrap_or(n);
            n.chars().map(|c| if c.is_alphanumeric() || "-_.".contains(c) { c } else { '-' }).collect::<String>()
        }
        None => input.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
    };
    let stem = if stem.is_empty() { "model".to_string() } else { stem };
    // A split model's part number ("-00001-of-00003") doesn't belong on the single new file.
    let stem = match stem.rfind("-of-") {
        Some(at) if stem[at + 4..].chars().all(|c| c.is_ascii_digit()) => stem[..at].rsplit_once('-').map(|(b, _)| b.to_string()).unwrap_or(stem),
        _ => stem,
    };
    let base = match quant {
        Some(q) if !q.is_empty() => match stem.to_ascii_lowercase().rfind(&q.to_ascii_lowercase()) {
            Some(at) => format!("{}{}", &stem[..at], &stem[at + q.len()..]),
            None => stem,
        },
        _ => stem,
    };
    let base = base.trim_matches(['-', '.', '_']).replace("--", "-");
    format!("{base}-{}.gguf", t.name)
}

/// The temporary name while it's written.
pub fn partial_of(out: &Path) -> PathBuf {
    let n = out.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    out.with_file_name(format!(".{n}.filescape-partial"))
}

/// A run in progress.
pub struct Run {
    pub out: PathBuf,
    /// Tensors done and in all (0 until the first line).
    pub done: Arc<AtomicU32>,
    pub total: Arc<AtomicU32>,
    pub stop: Arc<AtomicBool>,
    /// What came of it: `None` while running.
    pub result: Arc<Mutex<Option<Result<Checked, String>>>>,
}

/// Starts shrinking `input` (a GGUF file; the first part of a split one) into `out`, which mustn't
/// exist. Returns at once.
pub fn start(input: &Path, input_bytes: u64, out: &Path, t: Target, threads: u32) -> Result<Run, String> {
    if std::fs::symlink_metadata(out).is_ok() {
        return Err(format!("{} already exists.", out.display()));
    }
    let partial = partial_of(out);
    let _ = std::fs::remove_file(&partial);
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = Command::new(exe)
        .arg("--quantize-worker")
        .arg(input)
        .arg(&partial)
        .arg(t.ftype.to_string())
        .arg(threads.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't start: {e}"))?;
    let run = Run {
        out: out.to_path_buf(),
        done: Arc::default(),
        total: Arc::default(),
        stop: Arc::default(),
        result: Arc::default(),
    };
    let (done, total, stop, result) = (run.done.clone(), run.total.clone(), run.stop.clone(), run.result.clone());
    let out = out.to_path_buf();
    let input = input.to_path_buf();
    let ftype = t.ftype;
    let stderr = child.stderr.take();
    std::thread::spawn(move || {
        let last = Arc::new(Mutex::new(String::new()));
        let reader = stderr.map(|e| {
            let (done, total, last) = (done.clone(), total.clone(), last.clone());
            std::thread::spawn(move || {
                for line in BufReader::new(e).lines().map_while(Result::ok) {
                    if let Some((d, t)) = progress(&line) {
                        done.store(d, Ordering::Relaxed);
                        total.store(t, Ordering::Relaxed);
                    } else if !line.trim().is_empty() {
                        *last.lock().unwrap() = line;
                    }
                }
            })
        });
        let r = wait(&mut child, &stop);
        if let Some(h) = reader {
            let _ = h.join();
        }
        // Checked before it gets its name, so a damaged result is never listed as a model.
        let input = input.clone();
        let r = r.and_then(|()| verify(&input, input_bytes, &partial, ftype)).and_then(|c| std::fs::rename(&partial, &out).map(|()| c).map_err(|e| format!("The new file couldn't be named: {e}")));
        if r.is_err() {
            let _ = std::fs::remove_file(&partial);
        }
        let r = r.map_err(|e| {
            let why = last.lock().unwrap().trim().to_string();
            if why.is_empty() || e == STOPPED { e } else { format!("{e} llama.cpp said: {why}") }
        });
        *result.lock().unwrap() = Some(r);
    });
    Ok(run)
}

/// One side of a before-and-after comparison, read from the file's header.
#[derive(Clone, Debug, PartialEq)]
pub struct Side {
    pub bytes: u64,
    /// The precision the file says it has ("F16", "Q4_K_M"), if it says.
    pub precision: Option<String>,
    pub tensors: usize,
    /// Numbers in the model (parameters), counted from the tensors.
    pub params: u64,
}

impl Side {
    /// Bits a parameter takes on disk, measured: the file's size over its parameters.
    pub fn bits(&self) -> Option<f64> {
        (self.params > 0).then(|| self.bytes as f64 * 8.0 / self.params as f64)
    }
}

/// What checking the new file found: the original and the new file side by side.
#[derive(Clone, Debug, PartialEq)]
pub struct Checked {
    pub before: Side,
    pub after: Side,
    /// Tensors whose number type changed (the rest, such as norms, stay precise on purpose).
    pub changed: usize,
    /// What was checked, in plain sentences, for the window to list.
    pub checks: Vec<String>,
}

fn side(path: &Path, h: &crate::gguf::Header) -> Side {
    Side {
        bytes: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        precision: h.u64("general.file_type").and_then(crate::models::ftype_name).map(str::to_string),
        tensors: h.tensors.len(),
        params: h.tensors.iter().map(|t| t.elements()).sum(),
    }
}

/// Reopens a new file and checks it against the original: it must read as a model, say the
/// precision asked for, hold every tensor of the original with the same shape, and be long
/// enough for all of them. Any failure means the file isn't kept.
pub fn verify(input: &Path, input_bytes: u64, out: &Path, ftype: u32) -> Result<Checked, String> {
    use crate::gguf;
    let bad = |why: String| Err(format!("The new file failed its check, so it wasn't kept: {why}. The original is unchanged."));
    let new = match gguf::read(out, true) {
        Ok(h) => h,
        Err(e) => return bad(format!("it can't be read as a model ({e})")),
    };
    let len = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
    let mut checks = vec!["Reads as a GGUF model".to_string()];
    if new.u64("general.file_type") != Some(ftype as u64) {
        let says = new.u64("general.file_type").and_then(crate::models::ftype_name).unwrap_or("nothing");
        return bad(format!("it says its precision is {says}"));
    }
    checks.push(format!("Says its precision is {}", crate::models::ftype_name(ftype as u64).unwrap_or("the one asked for")));
    if let Some(t) = new.tensors.iter().find(|t| t.offset.checked_add(t.bytes).is_none_or(|end| end > len)) {
        return bad(format!("the file ends before the end of {}", t.name));
    }
    checks.push(format!("All {} tensors are inside the file", new.tensors.len()));
    let after = side(out, &new);
    // The original was read moments ago by the worker; if it can't be now, the new file still
    // passed its own checks.
    let Ok(old) = gguf::read(input, true) else { return Ok(Checked { before: Side { bytes: input_bytes, precision: None, tensors: 0, params: 0 }, after, changed: 0, checks }) };
    // A model split across files: the first file holds only its share of the tensors.
    let split = old.u64("split.count").is_some_and(|n| n > 1);
    let mut changed = 0;
    if !split {
        let mine: HashMap<&str, &gguf::Tensor> = new.tensors.iter().map(|t| (t.name.as_str(), t)).collect();
        for t in &old.tensors {
            match mine.get(t.name.as_str()) {
                None => return bad(format!("{} is missing", t.name)),
                Some(n) if n.dims != t.dims => {
                    return bad(format!("{} has a different shape", t.name));
                }
                Some(n) if n.kind != t.kind => changed += 1,
                Some(_) => {}
            }
        }
        if new.tensors.len() != old.tensors.len() {
            return bad(format!("it has {} tensors, the original {}", new.tensors.len(), old.tensors.len()));
        }
        checks.push("Every tensor of the original is there, with the same shape".into());
    }
    let mut before = side(input, &old);
    before.bytes = input_bytes.max(before.bytes);
    if split {
        // Only the first file's tensors were read: the count would be a share, not the model.
        before.params = 0;
        before.tensors = 0;
    }
    Ok(Checked { before, after, changed, checks })
}

const STOPPED: &str = "Stopped. The original is unchanged and no new file was kept.";

fn wait(child: &mut Child, stop: &AtomicBool) -> Result<(), String> {
    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(STOPPED.into());
        }
        match child.try_wait() {
            Ok(Some(s)) if s.success() => return Ok(()),
            Ok(Some(_)) => return Err("Shrinking failed; the original is unchanged.".into()),
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// "[  12/ 291] blk.0.attn_q.weight …" → (12, 291).
fn progress(line: &str) -> Option<(u32, u32)> {
    let rest = line.trim_start().strip_prefix('[')?;
    let (inside, _) = rest.split_once(']')?;
    let (d, t) = inside.split_once('/')?;
    Some((d.trim().parse().ok()?, t.trim().parse().ok()?))
}

/// In the copy started by [`start`]: does the work and ends the process. Call first thing in
/// `main`.
pub fn serve_if_asked() {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if args.get(1).map(|a| a == "--quantize-worker") != Some(true) {
        return;
    }
    let code = match (args.get(2), args.get(3), args.get(4).and_then(|a| a.to_str()?.parse::<u32>().ok()), args.get(5).and_then(|a| a.to_str()?.parse::<i32>().ok())) {
        (Some(input), Some(out), Some(ftype), Some(threads)) => work(Path::new(input), Path::new(out), ftype, threads),
        _ => 2,
    };
    std::process::exit(code);
}

fn work(input: &Path, out: &Path, ftype: u32, threads: i32) -> i32 {
    use llama_cpp_sys_2 as sys;
    let (Some(i), Some(o)) = (cstr(input), cstr(out)) else { return 2 };
    // SAFETY: plain calls into llama.cpp with valid, NUL-terminated paths and its own default
    // parameters; the original is opened for reading only.
    unsafe {
        sys::llama_backend_init();
        let mut p = sys::llama_model_quantize_default_params();
        p.nthread = threads;
        p.ftype = ftype as _;
        // An already quantised model can be shrunk further (the app warns about the quality).
        p.allow_requantize = true;
        let r = sys::llama_model_quantize(i.as_ptr(), o.as_ptr(), &p);
        sys::llama_backend_free();
        if r == 0 { 0 } else { 1 }
    }
}

fn cstr(p: &Path) -> Option<std::ffi::CString> {
    std::ffi::CString::new(p.to_str()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_sizes() {
        let q4 = target("q4_k_m").unwrap();
        assert_eq!(output_name(Path::new("/m/Qwen3-8B-F16.gguf"), None, Some("F16"), q4), "Qwen3-8B-Q4_K_M.gguf");
        assert_eq!(output_name(Path::new("/m/llama-bf16-00001-of-00003.gguf"), None, Some("BF16"), q4), "llama-Q4_K_M.gguf");
        assert_eq!(output_name(Path::new("/m/sha256-abcdef"), Some("qwen3:8b"), Some("Q8_0"), q4), "qwen3-8b-Q4_K_M.gguf");
        assert_eq!(output_name(Path::new("/b/aa11"), Some("org/Tiny-F16 (F16)"), Some("F16"), q4), "Tiny-Q4_K_M.gguf");
        assert_eq!(output_name(Path::new("/m/tiny.gguf"), None, None, q4), "tiny-Q4_K_M.gguf");
        assert_eq!(estimate(16_000, Some("F16"), q4), 4_900);
        assert_eq!(estimate(16_000, Some("Q4_K_M"), target("Q8_0").unwrap()), 16_000);
        assert!(already_shrunk(Some("Q8_0")) && already_shrunk(Some("IQ4_XS")) && !already_shrunk(Some("BF16")) && !already_shrunk(None));
        assert_eq!(progress("[  12/ 291]  blk.0.attn_q.weight - [ 2048,  2048], type = f16"), Some((12, 291)));
        assert_eq!(progress("llama_model_loader: loaded meta data"), None);
    }

    #[test]
    fn the_new_file_is_checked_against_the_original() {
        use crate::gguf::write_test;
        let d = std::env::temp_dir().join(format!("filescape-qv-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let input = d.join("in.gguf");
        let out = d.join("out.gguf");
        write_test(&input, &[], &[("general.file_type", 1)], &[("blk.0.w", &[256, 4], 1), ("norm", &[256], 0)], 0);
        let q8 = target("Q8_0").unwrap().ftype;
        write_test(&out, &[], &[("general.file_type", q8)], &[("blk.0.w", &[256, 4], 8), ("norm", &[256], 0)], 0);
        let c = verify(&input, 0, &out, q8).unwrap();
        assert_eq!((c.before.precision.as_deref(), c.after.precision.as_deref()), (Some("F16"), Some("Q8_0")));
        assert_eq!((c.before.params, c.after.params, c.changed), (1280, 1280, 1));
        assert!(c.after.bytes < c.before.bytes && c.after.bits().unwrap() < c.before.bits().unwrap());
        assert_eq!(c.checks.len(), 4);
        // The wrong precision, a missing tensor, a changed shape, a cut-short file: not kept.
        assert!(verify(&input, 0, &out, target("Q4_K_M").unwrap().ftype).unwrap_err().contains("says its precision is Q8_0"));
        write_test(&out, &[], &[("general.file_type", q8)], &[("blk.0.w", &[256, 4], 8)], 0);
        assert!(verify(&input, 0, &out, q8).unwrap_err().contains("norm is missing"));
        write_test(&out, &[], &[("general.file_type", q8)], &[("blk.0.w", &[256, 2], 8), ("norm", &[256], 0)], 0);
        assert!(verify(&input, 0, &out, q8).unwrap_err().contains("different shape"));
        write_test(&out, &[], &[("general.file_type", q8)], &[("blk.0.w", &[256, 4], 8), ("norm", &[256], 0)], 0);
        let f = std::fs::OpenOptions::new().write(true).open(&out).unwrap();
        f.set_len(f.metadata().unwrap().len() - 600).unwrap();
        assert!(verify(&input, 0, &out, q8).unwrap_err().contains("ends before the end of"));
        std::fs::write(&out, b"nope").unwrap();
        assert!(verify(&input, 0, &out, q8).unwrap_err().contains("can't be read as a model"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// Shrinks a real model through the worker. Needs FILESCAPE_TEST_GGUF (a small GGUF).
    #[test]
    #[ignore]
    fn shrinks_a_model() {
        let input = PathBuf::from(std::env::var("FILESCAPE_TEST_GGUF").unwrap());
        let p = std::env::temp_dir().join(format!("filescape-q-{}.gguf", std::process::id()));
        assert_eq!(work(&input, &p, target("Q8_0").unwrap().ftype, 2), 0);
        let n = std::fs::metadata(&p).unwrap().len();
        assert!(n > 1000 && n < std::fs::metadata(&input).unwrap().len() * 2);
        std::fs::remove_file(&p).unwrap();
    }
}
