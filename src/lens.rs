//! "What each layer would say": the Lab's look at a model's thinking layer by layer (often called
//! the logit lens). Each layer's output for a token is put through the model's own last steps
//! (its final normalisation and output weights) as if it were the last layer, which shows what
//! the model would predict if it stopped there. The output weights are read from the GGUF file,
//! a row at a time, by several threads, and turned into numbers by llama.cpp's own decoders (so
//! every number type a model can be stored in works); nothing is written.

use crate::gguf::{self, Header, Tensor};
use llama_cpp_sys_2 as sys;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Turns rows of one ggml number type into f32 values: plain floats here, packed types with
/// llama.cpp's decoder for them.
#[derive(Clone, Copy)]
struct Decoder {
    to_float: Option<unsafe extern "C" fn(*const std::ffi::c_void, *mut f32, i64)>,
    kind: &'static str,
    row_bytes: usize,
    row_len: usize,
}

impl Decoder {
    fn new(kind: u32, row_len: usize) -> Result<Decoder, String> {
        let name = gguf::type_name(kind);
        let unreadable = || format!("The output weights are stored as {name}, which the Lab can't read.");
        if kind >= sys::GGML_TYPE_COUNT as u32 {
            return Err(unreadable());
        }
        // SAFETY: a valid type number (checked above); the table is static.
        let traits = unsafe { sys::ggml_get_type_traits(kind as sys::ggml_type).as_ref() }.ok_or_else(unreadable)?;
        // ggml has no decoder for F32 itself; the plain types are read here.
        let to_float = if matches!(name, "F32" | "F16" | "BF16") { None } else { Some(traits.to_float.ok_or_else(unreadable)?) };
        if traits.blck_size <= 0 || row_len as i64 % traits.blck_size != 0 {
            return Err(unreadable());
        }
        let row_bytes = row_len / traits.blck_size as usize * traits.type_size;
        Ok(Decoder { to_float, kind: name, row_bytes, row_len })
    }

    fn decode(&self, raw: &[u8], out: &mut [f32]) {
        assert!(raw.len() >= self.row_bytes && out.len() >= self.row_len);
        match self.to_float {
            // SAFETY: raw holds a whole row of this type and out has room for its values (checked).
            Some(f) => unsafe { f(raw.as_ptr() as *const _, out.as_mut_ptr(), self.row_len as i64) },
            None => {
                let mut v = Vec::with_capacity(self.row_len);
                crate::weights::decode(self.kind, &raw[..self.row_bytes], &mut v);
                out[..v.len()].copy_from_slice(&v);
            }
        }
    }
}

/// What one layer would predict.
#[derive(Clone, Debug, PartialEq)]
pub struct Guess {
    /// (token id, probability), likeliest first.
    pub top: Vec<(u32, f32)>,
}

/// What the Lab shows after looking inside: the tokens read, and how much each layer changed
/// each token's vector.
#[derive(Clone, Debug, Default)]
pub struct Inside {
    pub tokens: Vec<String>,
    /// `[layer][token]`: how far the layer moved the vector, as a share of its length before
    /// (0.1 is a tenth). The first layer is NaN: what came before it isn't caught.
    pub change: Vec<Vec<f32>>,
    pub seconds: f32,
}

/// How much each layer changed each token's vector.
pub fn changes(l: &crate::probe::Layers) -> Vec<Vec<f32>> {
    (0..l.states.len())
        .map(|layer| {
            (0..l.tokens)
                .map(|t| {
                    if layer == 0 {
                        return f32::NAN;
                    }
                    let (a, b) = (l.at(layer - 1, t), l.at(layer, t));
                    let moved = a.iter().zip(b).map(|(x, y)| (y - x) * (y - x)).sum::<f32>().sqrt();
                    let len = a.iter().map(|x| x * x).sum::<f32>().sqrt();
                    if len > 0.0 { moved / len } else { f32::NAN }
                })
                .collect()
        })
        .collect()
}

/// The model's last steps, read from its file.
pub struct Head {
    path: std::path::PathBuf,
    norm: Vec<f32>,
    /// LayerNorm models have a bias and subtract the mean; RMSNorm ones don't.
    bias: Option<Vec<f32>>,
    eps: f32,
    output: Tensor,
    decoder: Decoder,
    n_embd: usize,
    n_vocab: usize,
}

fn values(path: &Path, t: &Tensor) -> Result<Vec<f32>, String> {
    let d = Decoder::new(t.kind, t.elements() as usize).map_err(|_| format!("{} is stored in a way the Lab can't read.", t.name))?;
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    f.seek(SeekFrom::Start(t.offset)).map_err(|e| e.to_string())?;
    let mut b = vec![0u8; d.row_bytes];
    f.read_exact(&mut b).map_err(|e| e.to_string())?;
    let mut out = vec![0f32; d.row_len];
    d.decode(&b, &mut out);
    Ok(out)
}

impl Head {
    /// Finds the final normalisation and the output weights (or the token embeddings, for models
    /// that share them).
    pub fn read(path: &Path) -> Result<Head, String> {
        let h: Header = gguf::read(path, true).map_err(|e| e.to_string())?;
        let find = |n: &str| h.tensors.iter().find(|t| t.name == n).cloned();
        let norm_t = find("output_norm.weight").ok_or("This model has no final normalisation the Lab recognises.")?;
        let output = find("output.weight").or_else(|| find("token_embd.weight")).ok_or("This model has no output weights the Lab recognises.")?;
        let n_embd = output.dims.first().copied().unwrap_or(0) as usize;
        let n_vocab = output.dims.get(1).copied().unwrap_or(0) as usize;
        let decoder = Decoder::new(output.kind, n_embd)?;
        let norm = values(path, &norm_t)?;
        if norm.len() != n_embd || n_vocab == 0 {
            return Err("The model's output weights don't have the shape the Lab expects.".into());
        }
        let bias = match find("output_norm.bias") {
            Some(t) => Some(values(path, &t)?),
            None => None,
        };
        let eps = ["attention.layer_norm_rms_epsilon", "attention.layer_norm_epsilon"]
            .iter()
            .find_map(|k| match h.get(&format!("{}.{k}", h.str("general.architecture").unwrap_or(""))) {
                Some(gguf::Value::Float(v)) => Some(*v as f32),
                _ => None,
            })
            .unwrap_or(1e-6);
        Ok(Head { path: path.to_path_buf(), norm, bias, eps, output, decoder, n_embd, n_vocab })
    }

    pub fn n_embd(&self) -> usize {
        self.n_embd
    }

    /// The final normalisation of one vector.
    pub fn normalise(&self, x: &[f32]) -> Vec<f32> {
        normalise(x, &self.norm, self.bias.as_deref(), self.eps)
    }

    /// The `n` likeliest tokens for each of `xs` (vectors already normalised), with
    /// probabilities over the whole vocabulary. One pass over the output weights serves them all.
    pub fn top(&self, xs: &[Vec<f32>], n: usize, threads: usize) -> Result<Vec<Guess>, String> {
        let row_bytes = self.decoder.row_bytes;
        let threads = threads.clamp(1, 32);
        let per = self.n_vocab.div_ceil(threads);
        let parts: Vec<Result<Vec<Acc>, String>> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..threads)
                .map(|k| {
                    let (from, to) = (k * per, ((k + 1) * per).min(self.n_vocab));
                    s.spawn(move || self.scan(xs, n, from, to, row_bytes))
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("A worker stopped".into()))).collect()
        });
        let mut total: Vec<Acc> = xs.iter().map(|_| Acc::new(n)).collect();
        for p in parts {
            for (t, a) in total.iter_mut().zip(p?) {
                t.merge(a);
            }
        }
        Ok(total.into_iter().map(|a| a.finish()).collect())
    }

    /// Scores rows `from..to` of the output weights against every vector.
    fn scan(&self, xs: &[Vec<f32>], n: usize, from: usize, to: usize, row_bytes: usize) -> Result<Vec<Acc>, String> {
        let mut acc: Vec<Acc> = xs.iter().map(|_| Acc::new(n)).collect();
        if from >= to {
            return Ok(acc);
        }
        let mut f = std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(&self.path).map_err(|e| e.to_string())?);
        f.seek(SeekFrom::Start(self.output.offset + (from * row_bytes) as u64)).map_err(|e| e.to_string())?;
        let mut raw = vec![0u8; row_bytes];
        let mut row = vec![0f32; self.n_embd];
        for id in from..to {
            f.read_exact(&mut raw).map_err(|e| e.to_string())?;
            self.decoder.decode(&raw, &mut row);
            for (a, x) in acc.iter_mut().zip(xs) {
                a.add(id as u32, dot(&row, x));
            }
        }
        Ok(acc)
    }
}

pub fn normalise(x: &[f32], w: &[f32], bias: Option<&[f32]>, eps: f32) -> Vec<f32> {
    let n = x.len() as f32;
    match bias {
        // RMSNorm: x / rms(x) * w.
        None => {
            let rms = (x.iter().map(|v| v * v).sum::<f32>() / n + eps).sqrt();
            x.iter().zip(w).map(|(v, w)| v / rms * w).collect()
        }
        // LayerNorm: (x - mean) / sd * w + b.
        Some(b) => {
            let mean = x.iter().sum::<f32>() / n;
            let var = x.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / n;
            let sd = (var + eps).sqrt();
            x.iter().zip(w).zip(b).map(|((v, w), b)| (v - mean) / sd * w + b).collect()
        }
    }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    // Four running sums: faster, and the compiler can use vector instructions.
    let mut s = [0f32; 4];
    let (ca, cb) = (a.chunks_exact(4), b.chunks_exact(4));
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (x, y) in ca.zip(cb) {
        for i in 0..4 {
            s[i] += x[i] * y[i];
        }
    }
    s.iter().sum::<f32>() + ra.iter().zip(rb).map(|(x, y)| x * y).sum::<f32>()
}

/// The best `n` scores seen, and a running log-sum-exp for the probabilities.
struct Acc {
    n: usize,
    best: Vec<(u32, f32)>,
    max: f32,
    sum: f64,
}

impl Acc {
    fn new(n: usize) -> Acc {
        Acc { n, best: Vec::with_capacity(n + 1), max: f32::NEG_INFINITY, sum: 0.0 }
    }

    fn add(&mut self, id: u32, score: f32) {
        if score > self.max {
            self.sum = self.sum * ((self.max - score) as f64).exp() + 1.0;
            self.max = score;
        } else {
            self.sum += ((score - self.max) as f64).exp();
        }
        if self.best.len() < self.n || score > self.best.last().map_or(f32::NEG_INFINITY, |b| b.1) {
            let at = self.best.partition_point(|b| b.1 >= score);
            self.best.insert(at, (id, score));
            self.best.truncate(self.n);
        }
    }

    fn merge(&mut self, o: Acc) {
        if o.max > self.max {
            self.sum = self.sum * ((self.max - o.max) as f64).exp() + o.sum;
            self.max = o.max;
        } else if o.max.is_finite() {
            self.sum += o.sum * ((o.max - self.max) as f64).exp();
        }
        for (id, s) in o.best {
            if self.best.len() < self.n || s > self.best.last().map_or(f32::NEG_INFINITY, |b| b.1) {
                let at = self.best.partition_point(|b| b.1 >= s);
                self.best.insert(at, (id, s));
                self.best.truncate(self.n);
            }
        }
    }

    fn finish(self) -> Guess {
        let (max, sum) = (self.max, self.sum.max(f64::MIN_POSITIVE));
        Guess { top: self.best.into_iter().map(|(id, s)| (id, (((s - max) as f64).exp() / sum) as f32)).collect() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rms_and_layer_norm() {
        let x = [1.0, -1.0, 1.0, -1.0];
        let w = [2.0; 4];
        let r = normalise(&x, &w, None, 0.0);
        assert!(r.iter().zip([2.0, -2.0, 2.0, -2.0]).all(|(a, b)| (a - b).abs() < 1e-5), "{r:?}");
        let x = [1.0, 2.0, 3.0, 4.0];
        let l = normalise(&x, &[1.0; 4], Some(&[0.5; 4]), 0.0);
        assert!((l.iter().sum::<f32>() - 2.0).abs() < 1e-4, "mean 0 plus bias 0.5 each: {l:?}");
    }

    #[test]
    fn accumulators_match_a_plain_softmax() {
        let scores = [3.0f32, -1.0, 7.5, 2.0, 7.0, 0.0, -20.0];
        let p = crate::lab::softmax(&scores, 1.0);
        // Split in two parts and merged, as the threads do.
        let (mut a, mut b) = (Acc::new(3), Acc::new(3));
        for (i, &s) in scores.iter().enumerate() {
            if i % 2 == 0 { a.add(i as u32, s) } else { b.add(i as u32, s) }
        }
        a.merge(b);
        let g = a.finish();
        assert_eq!(g.top.iter().map(|t| t.0).collect::<Vec<_>>(), [2, 4, 0]);
        for (id, q) in g.top {
            assert!((p[id as usize] - q).abs() < 1e-5);
        }
    }

    #[test]
    fn decodes_plain_and_packed_rows() {
        // F32 is read here; Q8_0 by llama.cpp (one block: an f16 scale, then 32 signed bytes).
        let d = Decoder::new(0, 2).unwrap();
        let mut out = [0f32; 2];
        d.decode(&[0, 0, 128, 63, 0, 0, 0, 192], &mut out);
        assert_eq!(out, [1.0, -2.0]);
        let q = Decoder::new(8, 32).unwrap();
        assert_eq!(q.row_bytes, 34);
        let mut raw = vec![0u8; 34];
        raw[..2].copy_from_slice(&half::f16::from_f32(0.5).to_bits().to_le_bytes());
        for i in 0..32 {
            raw[2 + i] = (i as i8 - 16) as u8;
        }
        let mut out = [0f32; 32];
        q.decode(&raw, &mut out);
        assert!(out.iter().enumerate().all(|(i, v)| *v == (i as f32 - 16.0) * 0.5), "{out:?}");
        // A row that isn't a whole number of blocks is refused.
        assert!(Decoder::new(8, 33).is_err());
    }

    #[test]
    fn dot_handles_remainders() {
        let a: Vec<f32> = (0..7).map(|i| i as f32).collect();
        assert_eq!(dot(&a, &a), (0..7).map(|i| (i * i) as f32).sum::<f32>());
    }
}
