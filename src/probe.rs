//! Reading what a model computes inside, for the Lab: the output of every layer for every token
//! of a text (llama.cpp's "l_out-N" tensors), caught with llama.cpp's evaluation callback; and
//! writing with a steering vector added to one layer's output (a control vector). Both run in a
//! context of their own that's freed afterwards, so nothing about the model, Ask AI's context or
//! anything on disk changes.
//!
//! The safe wrapper doesn't offer the callback, so this uses the raw bindings, kept to this file.

use std::ffi::{CStr, c_void};

use llama_cpp_2::model::LlamaModel;
use llama_cpp_2::token::LlamaToken;
use llama_cpp_sys_2 as sys;

/// Every layer's output for every token.
#[derive(Debug)]
pub struct Layers {
    pub n_embd: usize,
    pub tokens: usize,
    /// One per layer, in order: `tokens` vectors of `n_embd` values each, token after token.
    pub states: Vec<Vec<f32>>,
}

impl Layers {
    /// One token's vector after one layer.
    pub fn at(&self, layer: usize, token: usize) -> &[f32] {
        &self.states[layer][token * self.n_embd..(token + 1) * self.n_embd]
    }
}

/// The most tokens read at once: enough for a sentence or a paragraph, and the layers' outputs
/// stay small (28 layers × 256 tokens × 2,048 values is 59 MB).
pub const MAX_TOKENS: usize = 256;

struct Capture {
    n_embd: usize,
    tokens: usize,
    out: Vec<Option<Vec<f32>>>,
}

/// llama.cpp asks (ask = true) whether we want each tensor computed for us to read, then calls
/// again once it's computed.
unsafe extern "C" fn catch(t: *mut sys::ggml_tensor, ask: bool, user: *mut c_void) -> bool {
    // SAFETY: llama.cpp passes a live tensor and the pointer we gave it, during llama_decode.
    let (t, cap) = unsafe { (&*t, &mut *(user as *mut Capture)) };
    let name = unsafe { CStr::from_ptr(t.name.as_ptr()) }.to_bytes();
    let layer = name.strip_prefix(b"l_out-").and_then(|n| std::str::from_utf8(n).ok()).and_then(|n| n.parse::<usize>().ok());
    if ask {
        return layer.is_some();
    }
    if let Some(i) = layer
        && i < cap.out.len()
        && t.type_ == sys::GGML_TYPE_F32
        && t.ne[0] as usize == cap.n_embd
        && t.ne[1] as usize == cap.tokens
    {
        let n = cap.n_embd * cap.tokens;
        let mut v = vec![0f32; n];
        // SAFETY: the tensor holds n f32 values (checked above); this copies them off the
        // graphics chip too when the model runs there.
        unsafe { sys::ggml_backend_tensor_get(t, v.as_mut_ptr() as *mut c_void, 0, n * 4) };
        cap.out[i] = Some(v);
    }
    true
}

/// The raw model behind the wrapper. `LlamaModel` is a transparent wrapper around the pointer;
/// the sizes are checked here and the model's shape after, so a different layout is refused
/// rather than misread.
fn raw(model: &LlamaModel) -> Result<*mut sys::llama_model, String> {
    if std::mem::size_of::<LlamaModel>() != std::mem::size_of::<*mut sys::llama_model>() {
        return Err("This build can't look inside models.".into());
    }
    // SAFETY: same size, and LlamaModel holds only the non-null model pointer.
    let p = unsafe { *(model as *const LlamaModel as *const *mut sys::llama_model) };
    let ok = !p.is_null()
        && unsafe { sys::llama_model_n_embd(p) } == model.n_embd()
        && unsafe { sys::llama_model_n_layer(p) } as u32 == model.n_layer()
        && unsafe { sys::llama_vocab_n_tokens(sys::llama_model_get_vocab(p)) } == model.n_vocab();
    if ok { Ok(p) } else { Err("This build can't look inside models.".into()) }
}

/// A llama.cpp context of its own, freed when dropped.
struct Ctx(*mut sys::llama_context);

impl Drop for Ctx {
    fn drop(&mut self) {
        // SAFETY: made by llama_init_from_model and freed only here.
        unsafe { sys::llama_free(self.0) };
    }
}

/// A batch, freed when dropped.
struct Batch(sys::llama_batch);

impl Drop for Batch {
    fn drop(&mut self) {
        // SAFETY: made by llama_batch_init and freed only here.
        unsafe { sys::llama_batch_free(self.0) };
    }
}

/// A context for up to `n` tokens, reporting to `cap` if given. `cap` must outlive the context.
fn context(m: *mut sys::llama_model, n: usize, threads: i32, cap: Option<&mut Capture>) -> Result<Ctx, String> {
    // SAFETY: plain llama.cpp calls on a live model.
    unsafe {
        let mut p = sys::llama_context_default_params();
        p.n_ctx = (n as u32 + 16).next_multiple_of(256);
        p.n_batch = p.n_ctx;
        p.n_ubatch = p.n_ctx;
        p.n_seq_max = 1;
        p.n_threads = threads;
        p.n_threads_batch = threads;
        if let Some(cap) = cap {
            p.cb_eval = Some(catch);
            p.cb_eval_user_data = cap as *mut Capture as *mut c_void;
        }
        let ctx = sys::llama_init_from_model(m, p);
        if ctx.is_null() { Err("There isn't enough memory to look inside this model.".into()) } else { Ok(Ctx(ctx)) }
    }
}

/// Reads `tokens` at positions from `from`, asking for the scores after each when `all`, or only
/// after the last.
fn decode(ctx: &Ctx, tokens: &[LlamaToken], from: usize, all: bool) -> Result<(), String> {
    // SAFETY: the batch has room for every token, and is freed by Batch.
    unsafe {
        let b = Batch(sys::llama_batch_init(tokens.len() as i32, 0, 1));
        for (i, t) in tokens.iter().enumerate() {
            *b.0.token.add(i) = t.0;
            *b.0.pos.add(i) = (from + i) as i32;
            *b.0.n_seq_id.add(i) = 1;
            *(*b.0.seq_id.add(i)) = 0;
            *b.0.logits.add(i) = (all || i + 1 == tokens.len()) as i8;
        }
        let mut batch = b.0;
        batch.n_tokens = tokens.len() as i32;
        match sys::llama_decode(ctx.0, batch) {
            0 => Ok(()),
            rc => Err(format!("The model couldn't read the text (llama.cpp error {rc}).")),
        }
    }
}

/// Reads `tokens` with a fresh context and returns every layer's output for each of them.
pub fn layers(model: &LlamaModel, tokens: &[LlamaToken], threads: i32) -> Result<Layers, String> {
    if tokens.is_empty() {
        return Err("Type some text first.".into());
    }
    if tokens.len() > MAX_TOKENS {
        return Err(format!("That's {} tokens; the Lab looks inside at most {MAX_TOKENS} at once. Shorten the text.", tokens.len()));
    }
    let m = raw(model)?;
    let n_embd = model.n_embd() as usize;
    let n_layer = model.n_layer() as usize;
    let n = tokens.len();
    let mut cap = Capture { n_embd, tokens: n, out: vec![None; n_layer] };
    {
        let ctx = context(m, n, threads, Some(&mut cap))?;
        // Every token's scores, so llama.cpp keeps all of them through the last layer.
        decode(&ctx, tokens, 0, true)?;
    }
    let states: Option<Vec<Vec<f32>>> = cap.out.into_iter().collect();
    let states = states.ok_or("This model doesn't show its layers' outputs, so the Lab can't look inside it.")?;
    Ok(Layers { n_embd, tokens: n, states })
}

/// A push added to one layer's output for every token: `vector` (n_embd values) after layer
/// `layer` (counted from 0, as [`Layers`] counts them; the first layer can't be pushed).
pub struct Push<'a> {
    pub layer: usize,
    pub vector: &'a [f32],
}

/// Writes up to `steps` tokens after `tokens` in a fresh context, with `push` added if given,
/// choosing each with the Lab's sampling. Stops early at an end token or when `stop` is set.
pub fn write(
    model: &LlamaModel,
    tokens: &[LlamaToken],
    steps: u32,
    settings: &crate::lab::Settings,
    push: Option<Push>,
    threads: i32,
    stop: &std::sync::atomic::AtomicBool,
) -> Result<crate::lab::Run, String> {
    use crate::lab::{Choice, Rng, Step};
    let t = std::time::Instant::now();
    if tokens.is_empty() {
        return Err("Type some text first.".into());
    }
    let m = raw(model)?;
    let n_embd = model.n_embd() as usize;
    let n_layer = model.n_layer() as usize;
    let ctx = context(m, tokens.len() + steps as usize, threads, None)?;
    if let Some(p) = &push {
        if p.layer == 0 || p.layer >= n_layer || p.vector.len() != n_embd {
            return Err("That layer can't be steered.".into());
        }
        // One vector per layer from the second; only the chosen one is used.
        let mut data = vec![0f32; n_embd * (n_layer - 1)];
        data[(p.layer - 1) * n_embd..p.layer * n_embd].copy_from_slice(p.vector);
        // SAFETY: data holds n_embd values for each layer from 1, as llama.cpp expects; it copies them.
        let rc = unsafe { sys::llama_set_adapter_cvec(ctx.0, data.as_ptr(), data.len(), n_embd as i32, p.layer as i32, p.layer as i32) };
        if rc != 0 {
            return Err("This model can't be steered.".into());
        }
    }
    decode(&ctx, tokens, 0, false)?;
    let piece = |id: i32| String::from_utf8_lossy(&model.vocab().token_to_piece(LlamaToken(id), false, None)).into_owned();
    let n_vocab = model.n_vocab() as usize;
    let mut rng = Rng::new(settings.seed);
    let mut out = Vec::new();
    let mut pos = tokens.len();
    for _ in 0..steps {
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        // SAFETY: the last decode asked for the scores of its last token, n_vocab of them.
        let logits = unsafe { std::slice::from_raw_parts(sys::llama_get_logits_ith(ctx.0, -1), n_vocab) };
        let (chosen, raw, adjusted, top, kept) = crate::lab::step(logits, settings, &mut rng, 10);
        let choice = |i: usize, raw: f32, adjusted: f32| Choice { id: i as i32, text: piece(i as i32), raw, adjusted };
        let tok = LlamaToken(chosen as i32);
        let end = model.vocab().is_eog(tok);
        out.push(Step { chosen: choice(chosen, raw, adjusted), top: top.into_iter().map(|(i, r, a)| choice(i, r, a)).collect(), kept, end });
        if end {
            break;
        }
        decode(&ctx, &[tok], pos, false)?;
        pos += 1;
    }
    Ok(crate::lab::Run { steps: out, prompt_tokens: tokens.len(), seconds: t.elapsed().as_secs_f32() })
}

/// A direction to steer in, worked out by hand from two words or phrases: the difference between
/// the average outputs of layer `layer` for `towards` and for `away`, leaving out a start token.
/// Either may be empty, to push towards (or away from) one alone.
/// Also returns its length as a share of the layer's typical output length, to show how strong
/// a push is.
pub fn direction(model: &LlamaModel, towards: &[LlamaToken], away: &[LlamaToken], layer: usize, threads: i32) -> Result<(Vec<f32>, f32), String> {
    let mean = |toks: &[LlamaToken]| -> Result<(Vec<f32>, f32), String> {
        // Nothing on one side: steer from nothing in particular.
        if toks.is_empty() {
            return Ok((vec![0.0; model.n_embd() as usize], 0.0));
        }
        let l = layers(model, toks, threads)?;
        if layer >= l.states.len() {
            return Err("The model doesn't have that many layers.".into());
        }
        let skip = usize::from(toks.len() > 1 && model.vocab().bos() == toks[0]);
        let n = (l.tokens - skip) as f32;
        let mut m = vec![0f32; l.n_embd];
        let mut len = 0f32;
        for t in skip..l.tokens {
            let v = l.at(layer, t);
            m.iter_mut().zip(v).for_each(|(a, b)| *a += b / n);
            len += v.iter().map(|x| x * x).sum::<f32>().sqrt() / n;
        }
        Ok((m, len))
    };
    let (a, la) = mean(towards)?;
    let (b, lb) = mean(away)?;
    let d: Vec<f32> = a.iter().zip(&b).map(|(x, y)| x - y).collect();
    let typical = if la > 0.0 && lb > 0.0 { (la + lb) / 2.0 } else { la.max(lb) };
    let share = if typical > 0.0 { d.iter().map(|x| x * x).sum::<f32>().sqrt() / typical } else { 0.0 };
    Ok((d, share))
}

#[cfg(test)]
mod tests {
    use super::*;
    use llama_cpp_2::model::params::LlamaModelParams;

    /// FILESCAPE_TEST_GGUF=path cargo test --release -- --ignored probe
    #[test]
    #[ignore]
    fn reads_every_layer() {
        let Ok(path) = std::env::var("FILESCAPE_TEST_GGUF") else { return };
        let backend = crate::llm::backend().unwrap();
        let model = LlamaModel::load_from_file(backend, path, &LlamaModelParams::default()).unwrap();
        let toks = model.vocab().tokenize(b"Hello there, my friend", true, true);
        let l = layers(&model, &toks, 2).unwrap();
        assert_eq!(l.states.len(), model.n_layer() as usize);
        assert_eq!(l.tokens, toks.len());
        assert!(l.states.iter().all(|s| s.len() == toks.len() * l.n_embd));
        assert!(l.states.iter().flatten().all(|v| v.is_finite()));
        // Each layer changes the vectors.
        assert_ne!(l.at(0, 0), l.at(l.states.len() - 1, 0));
        // The same text gives the same values.
        let again = layers(&model, &toks, 2).unwrap();
        assert_eq!(again.states, l.states);
    }

    /// FILESCAPE_TEST_GGUF=path cargo test --release -- --ignored probe
    #[test]
    #[ignore]
    fn steering_changes_the_text() {
        let Ok(path) = std::env::var("FILESCAPE_TEST_GGUF") else { return };
        let backend = crate::llm::backend().unwrap();
        let model = LlamaModel::load_from_file(backend, path, &LlamaModelParams::default()).unwrap();
        let tok = |s: &str| model.vocab().tokenize(s.as_bytes(), true, true);
        let stop = std::sync::atomic::AtomicBool::new(false);
        let greedy = crate::lab::Settings { temperature: 0.0, ..Default::default() };
        let prompt = tok("I went to the shop and");
        let ids = |r: &crate::lab::Run| r.steps.iter().map(|s| s.chosen.id).collect::<Vec<_>>();
        let plain = write(&model, &prompt, 12, &greedy, None, 2, &stop).unwrap();
        assert_eq!(plain.steps.len(), 12);
        // The same again, and a push of nothing, write the same.
        assert_eq!(ids(&write(&model, &prompt, 12, &greedy, None, 2, &stop).unwrap()), ids(&plain));
        let zero = vec![0f32; model.n_embd() as usize];
        assert_eq!(ids(&write(&model, &prompt, 12, &greedy, Some(Push { layer: 1, vector: &zero }), 2, &stop).unwrap()), ids(&plain));
        // A real push changes it.
        let (d, share) = direction(&model, &tok("love"), &tok("hate"), 1, 2).unwrap();
        assert!(share > 0.0 && d.iter().all(|v| v.is_finite()));
        let big: Vec<f32> = d.iter().map(|v| v * 20.0).collect();
        assert_ne!(ids(&write(&model, &prompt, 12, &greedy, Some(Push { layer: 1, vector: &big }), 2, &stop).unwrap()), ids(&plain));
        // The first layer and layers past the last are refused.
        assert!(write(&model, &prompt, 1, &greedy, Some(Push { layer: 0, vector: &zero }), 2, &stop).is_err());
        assert!(write(&model, &prompt, 1, &greedy, Some(Push { layer: model.n_layer() as usize, vector: &zero }), 2, &stop).is_err());
    }
}
