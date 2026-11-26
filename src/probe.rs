//! Reading what a model computes inside, for the Lab: the output of every layer for every token
//! of a text (llama.cpp's "l_out-N" tensors), caught with llama.cpp's evaluation callback in a
//! context of its own. Only reads: nothing about the model changes.
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
    // SAFETY: plain llama.cpp calls on a model that outlives them; the context, batch and capture
    // live until freed below, and the callback only runs inside llama_decode.
    unsafe {
        let mut p = sys::llama_context_default_params();
        p.n_ctx = (n as u32 + 16).next_multiple_of(256);
        p.n_batch = p.n_ctx;
        p.n_ubatch = p.n_ctx;
        p.n_seq_max = 1;
        p.n_threads = threads;
        p.n_threads_batch = threads;
        p.cb_eval = Some(catch);
        p.cb_eval_user_data = &mut cap as *mut Capture as *mut c_void;
        let ctx = sys::llama_init_from_model(m, p);
        if ctx.is_null() {
            return Err("There isn't enough memory to look inside this model.".into());
        }
        let batch = sys::llama_batch_init(n as i32, 0, 1);
        for (i, t) in tokens.iter().enumerate() {
            *batch.token.add(i) = t.0;
            *batch.pos.add(i) = i as i32;
            *batch.n_seq_id.add(i) = 1;
            *(*batch.seq_id.add(i)) = 0;
            // Every token's output, so llama.cpp keeps all of them through the last layer.
            *batch.logits.add(i) = 1;
        }
        let mut b = batch;
        b.n_tokens = n as i32;
        let rc = sys::llama_decode(ctx, b);
        sys::llama_batch_free(batch);
        sys::llama_free(ctx);
        if rc != 0 {
            return Err(format!("The model couldn't read the text (llama.cpp error {rc})."));
        }
    }
    let states: Option<Vec<Vec<f32>>> = cap.out.into_iter().collect();
    let states = states.ok_or("This model doesn't show its layers' outputs, so the Lab can't look inside it.")?;
    Ok(Layers { n_embd, tokens: n, states })
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
}
