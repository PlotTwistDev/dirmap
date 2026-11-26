//! Runs a GGUF model on this computer with llama.cpp, for Ask AI. One background thread owns the
//! engine and at most one loaded model; the window sends it requests and reads back what happens.
//! A model's answer is only ever text shown to the user: nothing here runs commands, opens files
//! or changes anything on disk because of what a model writes.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Instant;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{LlamaChatMessage, LlamaChatTemplate, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;

/// Who wrote a message in a conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

impl Role {
    fn name(self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Message {
    pub role: Role,
    pub text: String,
}

/// How the model picks its words.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sampling {
    /// 0 picks the likeliest word every time; higher is more varied.
    pub temperature: f32,
    /// The longest answer, in tokens.
    pub max_tokens: u32,
}

impl Default for Sampling {
    fn default() -> Self {
        Sampling { temperature: 0.6, max_tokens: 2048 }
    }
}

pub struct LoadRequest {
    pub path: PathBuf,
    /// Context length in tokens.
    pub context: u32,
    /// Put the model's layers on the graphics chip (Apple silicon).
    pub gpu: bool,
}

pub enum Request {
    Load(LoadRequest),
    /// Answer the last message of a conversation.
    Ask { conversation: Vec<Message>, sampling: Sampling },
    /// The Lab: write up to `steps` tokens after `text` (as a chat message when `chat`, then
    /// `continuation`), one at a time, reporting each token's probabilities.
    Predict { text: String, chat: bool, continuation: String, steps: u32, settings: crate::lab::Settings },
    /// The Lab's speed test.
    Bench(crate::lab::BenchSettings),
    /// The Lab: read `text` (as a chat message when `chat`) and catch every layer's output.
    Look { text: String, chat: bool },
    /// The Lab: what each layer would predict after token `n` of the last look.
    Lens(usize),
    /// The Lab: write after `text` twice, as it is and pushed after layer `layer` (from 0) along
    /// the difference between `towards` and `away`, `strength` times over.
    Steer(Steer),
    Unload,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Steer {
    pub text: String,
    pub chat: bool,
    pub towards: String,
    pub away: String,
    pub layer: usize,
    pub strength: f32,
    pub steps: u32,
    pub settings: crate::lab::Settings,
}

/// What the engine says back.
#[derive(Debug)]
pub enum Event {
    Loading,
    /// Loaded, with its context length and number of layers.
    Loaded { context: u32, seconds: f32, layers: u32 },
    /// More of the answer.
    Text(String),
    Done(Stats),
    /// A Lab run.
    Predicted(crate::lab::Run),
    /// How a speed test is going: (run, of runs).
    Benching(u32, u32),
    /// A speed test's middle results: (read speed, write speed, first token seconds, threads).
    Benched { read_speed: f32, write_speed: f32, first_token: f32, threads: u32 },
    /// A look inside: the tokens and how much each layer changed them.
    Looked(crate::lens::Inside),
    /// What each layer would predict after token `token`: per layer, the likeliest tokens and
    /// their probabilities.
    Lensed { token: usize, layers: Vec<Vec<(String, f32)>> },
    /// Steered writing beside the same without: and how long the push was, as a share of the
    /// layer's typical output.
    Steered { plain: crate::lab::Run, steered: crate::lab::Run, push: f32 },
    Unloaded,
    Error(String),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub prompt_tokens: usize,
    /// Prompt tokens read from the cache of the previous turn instead of again.
    pub reused: usize,
    pub tokens: usize,
    pub prompt_seconds: f32,
    pub seconds: f32,
    /// Stopped by the user or the length limit rather than by the model.
    pub cut_short: bool,
    /// The conversation filled the context, so the oldest messages were left out.
    pub trimmed: bool,
}

/// The window's handle on the engine thread.
pub struct Engine {
    tx: mpsc::Sender<Request>,
    pub rx: mpsc::Receiver<Event>,
    /// Set to stop the answer being written.
    pub stop: Arc<AtomicBool>,
}

impl Engine {
    pub fn start() -> Engine {
        let (tx, req_rx) = mpsc::channel();
        let (ev_tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        std::thread::Builder::new()
            .name("ask-ai".into())
            .spawn(move || worker(req_rx, ev_tx, s))
            .expect("start the Ask AI thread");
        Engine { tx, rx, stop }
    }

    pub fn send(&self, r: Request) {
        self.stop.store(false, Ordering::Relaxed);
        let _ = self.tx.send(r);
    }
}

/// Why a model can't run on this processor, if it can't. llama.cpp is built for AVX2 on x86-64.
pub fn processor_problem() -> Option<&'static str> {
    #[cfg(target_arch = "x86_64")]
    {
        if !(std::is_x86_feature_detected!("avx2") && std::is_x86_feature_detected!("fma")) {
            return Some("This processor lacks AVX2, which Ask AI needs (most PCs made since 2015 have it).");
        }
    }
    None
}

/// The engine, set up once per run of the app.
pub(crate) fn backend() -> Result<&'static LlamaBackend, String> {
    static B: std::sync::OnceLock<Result<LlamaBackend, String>> = std::sync::OnceLock::new();
    B.get_or_init(|| {
        let mut b = LlamaBackend::init().map_err(|e| e.to_string())?;
        b.void_logs();
        Ok(b)
    })
    .as_ref()
    .map_err(Clone::clone)
}

fn worker(rx: mpsc::Receiver<Request>, tx: mpsc::Sender<Event>, stop: Arc<AtomicBool>) {
    let mut next = None;
    loop {
        let req = match next.take() {
            Some(r) => r,
            None => match rx.recv() {
                Ok(Request::Load(r)) => r,
                Ok(Request::Ask { .. } | Request::Predict { .. } | Request::Bench(_) | Request::Look { .. } | Request::Lens(_) | Request::Steer(_)) => {
                    let _ = tx.send(Event::Error("No model is loaded.".into()));
                    continue;
                }
                Ok(Request::Unload) => continue,
                Err(_) => return,
            },
        };
        let _ = tx.send(Event::Loading);
        if let Some(p) = processor_problem() {
            let _ = tx.send(Event::Error(p.into()));
            continue;
        }
        let backend = match backend() {
            Ok(b) => b,
            Err(e) => {
                let _ = tx.send(Event::Error(format!("The AI engine couldn't start: {e}")));
                continue;
            }
        };
        let t = Instant::now();
        let params = LlamaModelParams::default().with_n_gpu_layers(if req.gpu { 999 } else { 0 });
        let model = match LlamaModel::load_from_file(backend, &req.path, &params) {
            Ok(m) => m,
            Err(e) => {
                let _ = tx.send(Event::Error(format!("The model couldn't be loaded: {e}")));
                continue;
            }
        };
        let context = req.context.min(model.n_ctx_train().max(512)).max(512);
        let threads = threads();
        let cparams = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(context))
            .with_n_batch(512)
            .with_n_threads(threads)
            .with_n_threads_batch(threads);
        let mut ctx = match model.new_context(backend, cparams) {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(Event::Error(format!("There isn't enough memory for this model at {context} tokens of context: {e}")));
                continue;
            }
        };
        let template = model.chat_template(None).ok();
        let _ = tx.send(Event::Loaded { context, seconds: t.elapsed().as_secs_f32(), layers: model.n_layer() });
        // Tokens already in the context, from the previous turn.
        let mut cached: Vec<LlamaToken> = Vec::new();
        // The Lab's last look inside, and the model's last steps (read when first needed).
        let mut looked: Option<crate::probe::Layers> = None;
        let mut head: Option<Result<crate::lens::Head, String>> = None;
        loop {
            match rx.recv() {
                Ok(Request::Ask { conversation, sampling }) => {
                    let ev = answer(&model, &mut ctx, template.as_ref(), &conversation, sampling, &mut cached, &stop, &tx);
                    let _ = tx.send(match ev {
                        Ok(s) => Event::Done(s),
                        Err(e) => {
                            cached.clear();
                            ctx.clear_kv_cache();
                            Event::Error(e)
                        }
                    });
                }
                Ok(Request::Predict { text, chat, continuation, steps, settings }) => {
                    let full = if chat { prompt(&model, template.as_ref(), &[Message { role: Role::User, text }]) } else { text };
                    let ev = predict(&model, &mut ctx, &(full + &continuation), steps, &settings, &mut cached, &stop);
                    let _ = tx.send(match ev {
                        Ok(r) => Event::Predicted(r),
                        Err(e) => {
                            cached.clear();
                            ctx.clear_kv_cache();
                            Event::Error(e)
                        }
                    });
                }
                Ok(Request::Bench(b)) => {
                    let ev = bench(&model, &mut ctx, &b, &stop, &tx);
                    // The test filled the context with its own text.
                    cached.clear();
                    ctx.clear_kv_cache();
                    let _ = tx.send(match ev {
                        Ok(Some((r, w, f))) => Event::Benched { read_speed: r, write_speed: w, first_token: f, threads: threads as u32 },
                        Ok(None) => Event::Error("The speed test was stopped.".into()),
                        Err(e) => Event::Error(e),
                    });
                }
                Ok(Request::Look { text, chat }) => {
                    let full = if chat { prompt(&model, template.as_ref(), &[Message { role: Role::User, text }]) } else { text };
                    looked = None;
                    match look(&model, &full, threads) {
                        Ok((layers, inside)) => {
                            let last = layers.tokens - 1;
                            looked = Some(layers);
                            let _ = tx.send(Event::Looked(inside));
                            let head = head.get_or_insert_with(|| crate::lens::Head::read(&req.path));
                            let _ = tx.send(lens(&model, head, looked.as_ref(), last, threads));
                        }
                        Err(e) => {
                            let _ = tx.send(Event::Error(e));
                        }
                    }
                }
                Ok(Request::Steer(st)) => {
                    let full = if st.chat { prompt(&model, template.as_ref(), &[Message { role: Role::User, text: st.text.clone() }]) } else { st.text.clone() };
                    let _ = tx.send(match steer(&model, &full, &st, threads, &stop) {
                        Ok(ev) => ev,
                        Err(e) => Event::Error(e),
                    });
                }
                Ok(Request::Lens(n)) => {
                    let head = head.get_or_insert_with(|| crate::lens::Head::read(&req.path));
                    let _ = tx.send(lens(&model, head, looked.as_ref(), n, threads));
                }
                Ok(Request::Load(r)) => {
                    next = Some(r);
                    break;
                }
                Ok(Request::Unload) => {
                    let _ = tx.send(Event::Unloaded);
                    break;
                }
                Err(_) => return,
            }
        }
    }
}

fn threads() -> i32 {
    let n = std::thread::available_parallelism().map_or(4, |n| n.get());
    // Hyper-threads don't help: about one per core, leaving one for the window.
    (if n > 4 { n / 2 } else { n.saturating_sub(1) }).clamp(1, 16) as i32
}

/// The conversation as the model reads it, with its own chat template (or ChatML when it has
/// none that llama.cpp knows).
fn prompt(model: &LlamaModel, template: Option<&LlamaChatTemplate>, conv: &[Message]) -> String {
    if let Some(t) = template {
        let msgs: Vec<LlamaChatMessage> = conv.iter().filter_map(|m| LlamaChatMessage::new(m.role.name().into(), m.text.clone()).ok()).collect();
        if let Ok(p) = model.apply_chat_template(t, &msgs, true) {
            return p;
        }
    }
    chatml(conv)
}

pub fn chatml(conv: &[Message]) -> String {
    let mut p = String::new();
    for m in conv {
        p.push_str(&format!("<|im_start|>{}\n{}<|im_end|>\n", m.role.name(), m.text));
    }
    p.push_str("<|im_start|>assistant\n");
    p
}

#[allow(clippy::too_many_arguments)]
fn answer(
    model: &LlamaModel,
    ctx: &mut llama_cpp_2::context::LlamaContext,
    template: Option<&LlamaChatTemplate>,
    conv: &[Message],
    sampling: Sampling,
    cached: &mut Vec<LlamaToken>,
    stop: &AtomicBool,
    tx: &mpsc::Sender<Event>,
) -> Result<Stats, String> {
    let t = Instant::now();
    let n_ctx = ctx.n_ctx() as usize;
    // Room kept for the answer when fitting the conversation in; the answer may use more, up to
    // the end of the context.
    let room = (sampling.max_tokens as usize).min((n_ctx / 4).max(128));
    let limit = sampling.max_tokens as usize;
    // Leave out the oldest messages (keeping a system message) until the prompt and an answer fit.
    let mut conv = conv.to_vec();
    let mut trimmed = false;
    let tokens = loop {
        let text = prompt(model, template, &conv);
        let toks = model.vocab().tokenize(text.as_bytes(), true, true);
        if toks.len() + room <= n_ctx {
            break toks;
        }
        let first = usize::from(conv.first().is_some_and(|m| m.role == Role::System));
        if conv.len() <= first + 1 {
            return Err(format!("The question is too long for this model's context ({n_ctx} tokens). Shorten it, or choose a longer context."));
        }
        conv.remove(first);
        trimmed = true;
    };
    let batch_size = ctx.n_batch() as usize;
    let mut batch = LlamaBatch::new(batch_size.max(1), 1);
    let Some(same) = feed(ctx, &mut batch, &tokens, cached, stop)? else {
        return Ok(Stats { prompt_tokens: tokens.len(), cut_short: true, trimmed, ..Default::default() });
    };
    let prompt_seconds = t.elapsed().as_secs_f32();
    let mut sampler = if sampling.temperature <= 0.0 {
        LlamaSampler::greedy()
    } else {
        LlamaSampler::chain_simple([
            LlamaSampler::top_k(40),
            LlamaSampler::top_p(0.95, 1),
            LlamaSampler::min_p(0.05, 1),
            LlamaSampler::temp(sampling.temperature),
            LlamaSampler::dist(rand_seed()),
        ])
    };
    let mut decoder = Utf8Stream::default();
    let mut n = 0usize;
    let mut cut_short = true;
    let gen_start = Instant::now();
    while n < limit && cached.len() < n_ctx {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let tok = sampler.sample(ctx, batch.n_tokens() - 1);
        sampler.accept(tok);
        if model.vocab().is_eog(tok) {
            cut_short = false;
            break;
        }
        let piece = model.vocab().token_to_piece(tok, false, None);
        if let Some(s) = decoder.push(&piece) {
            let _ = tx.send(Event::Text(s));
        }
        batch.clear();
        batch.add(tok, cached.len() as i32, &[0], true).map_err(|e| e.to_string())?;
        ctx.decode(&mut batch).map_err(|e| format!("The model stopped: {e}"))?;
        cached.push(tok);
        n += 1;
    }
    if let Some(s) = decoder.finish() {
        let _ = tx.send(Event::Text(s));
    }
    Ok(Stats {
        prompt_tokens: tokens.len(),
        reused: same,
        tokens: n,
        prompt_seconds,
        seconds: gen_start.elapsed().as_secs_f32(),
        cut_short,
        trimmed,
    })
}

/// Reads `tokens` into the context, keeping what's already there from an earlier turn. The last
/// token's scores are ready in `batch` afterwards. Returns how many tokens were kept, or None if
/// stopped.
fn feed(ctx: &mut llama_cpp_2::context::LlamaContext, batch: &mut LlamaBatch, tokens: &[LlamaToken], cached: &mut Vec<LlamaToken>, stop: &AtomicBool) -> Result<Option<usize>, String> {
    let mut same = cached.iter().zip(tokens).take_while(|(a, b)| a == b).count().min(tokens.len().saturating_sub(1));
    if same < cached.len() {
        // Some models (recurrent ones) can't forget part of what they read: start again.
        if !ctx.clear_kv_cache_seq(Some(0), Some(same as u32), None).map_err(|e| e.to_string())? {
            ctx.clear_kv_cache();
            same = 0;
        }
        cached.truncate(same);
    }
    let batch_size = ctx.n_batch() as usize;
    let mut pos = same;
    while pos < tokens.len() {
        if stop.load(Ordering::Relaxed) {
            return Ok(None);
        }
        batch.clear();
        let end = (pos + batch_size).min(tokens.len());
        for (i, tok) in tokens[pos..end].iter().enumerate() {
            let p = pos + i;
            batch.add(*tok, p as i32, &[0], p == tokens.len() - 1).map_err(|e| e.to_string())?;
        }
        ctx.decode(batch).map_err(|e| format!("The model couldn't read the text: {e}"))?;
        cached.extend_from_slice(&tokens[pos..end]);
        pos = end;
    }
    Ok(Some(same))
}

/// The Lab: writes up to `steps` tokens after `text`, choosing each with the Lab's own sampling
/// (`crate::lab`) so the probabilities shown are the ones picked from.
fn predict(
    model: &LlamaModel,
    ctx: &mut llama_cpp_2::context::LlamaContext,
    text: &str,
    steps: u32,
    settings: &crate::lab::Settings,
    cached: &mut Vec<LlamaToken>,
    stop: &AtomicBool,
) -> Result<crate::lab::Run, String> {
    use crate::lab::{Choice, Rng, Step};
    let t = Instant::now();
    let n_ctx = ctx.n_ctx() as usize;
    let tokens = model.vocab().tokenize(text.as_bytes(), true, true);
    if tokens.is_empty() {
        return Err("Type some text first.".into());
    }
    if tokens.len() + steps as usize > n_ctx {
        return Err(format!("The text is too long for this model's context ({n_ctx} tokens). Shorten it, or choose a longer context."));
    }
    let mut batch = LlamaBatch::new((ctx.n_batch() as usize).max(1), 1);
    if feed(ctx, &mut batch, &tokens, cached, stop)?.is_none() {
        return Ok(crate::lab::Run { prompt_tokens: tokens.len(), ..Default::default() });
    }
    let piece = |id: i32| String::from_utf8_lossy(&model.vocab().token_to_piece(LlamaToken(id), false, None)).into_owned();
    let mut rng = Rng::new(settings.seed);
    let mut out = Vec::new();
    for _ in 0..steps {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let logits = ctx.get_logits_ith(batch.n_tokens() - 1);
        let (chosen, raw, adjusted, top, kept) = crate::lab::step(logits, settings, &mut rng, 10);
        let choice = |i: usize, raw: f32, adjusted: f32| Choice { id: i as i32, text: piece(i as i32), raw, adjusted };
        let tok = LlamaToken(chosen as i32);
        let end = model.vocab().is_eog(tok);
        out.push(Step { chosen: choice(chosen, raw, adjusted), top: top.into_iter().map(|(i, r, a)| choice(i, r, a)).collect(), kept, end });
        if end || cached.len() + 1 >= n_ctx {
            break;
        }
        batch.clear();
        batch.add(tok, cached.len() as i32, &[0], true).map_err(|e| e.to_string())?;
        ctx.decode(&mut batch).map_err(|e| format!("The model stopped: {e}"))?;
        cached.push(tok);
    }
    Ok(crate::lab::Run { steps: out, prompt_tokens: tokens.len(), seconds: t.elapsed().as_secs_f32() })
}

/// The Lab: reads `text` in a context of its own and catches every layer's output.
fn look(model: &LlamaModel, text: &str, threads: i32) -> Result<(crate::probe::Layers, crate::lens::Inside), String> {
    let t = Instant::now();
    let tokens = model.vocab().tokenize(text.as_bytes(), true, true);
    let layers = crate::probe::layers(model, &tokens, threads)?;
    let inside = crate::lens::Inside {
        tokens: tokens.iter().map(|t| String::from_utf8_lossy(&model.vocab().token_to_piece(*t, false, None)).into_owned()).collect(),
        change: crate::lens::changes(&layers),
        seconds: t.elapsed().as_secs_f32(),
    };
    Ok((layers, inside))
}

/// The Lab's steering: the same text written without and with a push.
fn steer(model: &LlamaModel, text: &str, st: &Steer, threads: i32, stop: &AtomicBool) -> Result<Event, String> {
    let tok = |s: &str| model.vocab().tokenize(s.as_bytes(), true, true);
    if st.towards.trim().is_empty() && st.away.trim().is_empty() {
        return Err("Type a word to steer towards, or away from.".into());
    }
    let tokens = tok(text);
    if tokens.len() + st.steps as usize > crate::probe::MAX_TOKENS * 4 {
        return Err("The text is too long to steer. Shorten it.".into());
    }
    let side = |s: &str| if s.trim().is_empty() { Vec::new() } else { tok(s) };
    let (d, share) = crate::probe::direction(model, &side(&st.towards), &side(&st.away), st.layer, threads)?;
    let v: Vec<f32> = d.iter().map(|x| x * st.strength).collect();
    let plain = crate::probe::write(model, &tokens, st.steps, &st.settings, None, threads, stop)?;
    let steered = crate::probe::write(model, &tokens, st.steps, &st.settings, Some(crate::probe::Push { layer: st.layer, vector: &v }), threads, stop)?;
    Ok(Event::Steered { plain, steered, push: share * st.strength.abs() })
}

/// How many guesses the Lab shows for each layer.
const LENS_TOP: usize = 5;

/// What each layer would predict after token `n` of the last look.
fn lens(model: &LlamaModel, head: &Result<crate::lens::Head, String>, layers: Option<&crate::probe::Layers>, n: usize, threads: i32) -> Event {
    let head = match head {
        Ok(h) => h,
        Err(e) => return Event::Error(e.clone()),
    };
    let Some(l) = layers.filter(|l| n < l.tokens) else { return Event::Error("Look inside first.".into()) };
    if l.n_embd != head.n_embd() {
        return Event::Error("The model's output weights don't match its layers.".into());
    }
    let xs: Vec<Vec<f32>> = (0..l.states.len()).map(|layer| head.normalise(l.at(layer, n))).collect();
    match head.top(&xs, LENS_TOP, threads as usize) {
        Ok(guesses) => {
            let piece = |id: u32| String::from_utf8_lossy(&model.vocab().token_to_piece(LlamaToken(id as i32), false, None)).into_owned();
            Event::Lensed { token: n, layers: guesses.into_iter().map(|g| g.top.into_iter().map(|(id, p)| (piece(id), p)).collect()).collect() }
        }
        Err(e) => Event::Error(e),
    }
}

/// The Lab's speed test: reads `read` tokens of ordinary text and writes `write` more, `runs`
/// times from an empty context. Returns the middle read speed, write speed and time to the first
/// token, or None if stopped.
fn bench(
    model: &LlamaModel,
    ctx: &mut llama_cpp_2::context::LlamaContext,
    b: &crate::lab::BenchSettings,
    stop: &AtomicBool,
    tx: &mpsc::Sender<Event>,
) -> Result<Option<(f32, f32, f32)>, String> {
    let n_ctx = ctx.n_ctx() as usize;
    let read = (b.read as usize).min(n_ctx.saturating_sub(b.write as usize + 8)).max(1);
    let write = (b.write as usize).min(n_ctx - read).max(1);
    // Ordinary text, repeated to length: what's read doesn't change the speed much, but special
    // tokens or one token over and over could.
    let sample = model.vocab().tokenize(BENCH_TEXT.as_bytes(), false, true);
    if sample.is_empty() {
        return Err("This model's vocabulary couldn't read the test text.".into());
    }
    let tokens: Vec<LlamaToken> = sample.iter().copied().cycle().take(read).collect();
    let batch_size = ctx.n_batch() as usize;
    let mut batch = LlamaBatch::new(batch_size.max(1), 1);
    let (mut rs, mut ws, mut fs) = (Vec::new(), Vec::new(), Vec::new());
    for run in 0..b.runs.max(1) {
        let _ = tx.send(Event::Benching(run + 1, b.runs.max(1)));
        ctx.clear_kv_cache();
        let mut fresh = Vec::new();
        let t = Instant::now();
        if feed(ctx, &mut batch, &tokens, &mut fresh, stop)?.is_none() {
            return Ok(None);
        }
        let read_secs = t.elapsed().as_secs_f32();
        let t = Instant::now();
        let mut first = 0.0;
        for i in 0..write {
            if stop.load(Ordering::Relaxed) {
                return Ok(None);
            }
            // The likeliest token: sampling isn't what's being timed.
            let logits = ctx.get_logits_ith(batch.n_tokens() - 1);
            let tok = LlamaToken(logits.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map_or(0, |(i, _)| i as i32));
            batch.clear();
            batch.add(tok, (read + i) as i32, &[0], true).map_err(|e| e.to_string())?;
            ctx.decode(&mut batch).map_err(|e| format!("The model stopped: {e}"))?;
            if i == 0 {
                first = t.elapsed().as_secs_f32();
            }
        }
        let write_secs = t.elapsed().as_secs_f32();
        rs.push(read as f32 / read_secs.max(1e-6));
        ws.push(write as f32 / write_secs.max(1e-6));
        fs.push(read_secs + first);
    }
    Ok(Some((crate::lab::median(&rs), crate::lab::median(&ws), crate::lab::median(&fs))))
}

/// What the speed test reads: plain prose, in English.
const BENCH_TEXT: &str = "The library stood at the end of a quiet street, its windows bright against the evening. \
Inside, people read newspapers, searched for old maps and asked the staff about books they half remembered. \
Every afternoon a class of children arrived to hear a story, and every evening the last visitor left as the lights went out. ";

fn rand_seed() -> u32 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos() ^ d.as_secs() as u32)
}

/// Joins token pieces into text, holding back a character split across tokens until it's whole.
#[derive(Default)]
pub struct Utf8Stream {
    pending: Vec<u8>,
}

impl Utf8Stream {
    pub fn push(&mut self, bytes: &[u8]) -> Option<String> {
        self.pending.extend_from_slice(bytes);
        let valid = match std::str::from_utf8(&self.pending) {
            Ok(_) => self.pending.len(),
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            // Not UTF-8 at all: show what there is rather than hold it forever.
            Err(_) => self.pending.len(),
        };
        if valid == 0 {
            return None;
        }
        let rest = self.pending.split_off(valid);
        let s = String::from_utf8_lossy(&std::mem::replace(&mut self.pending, rest)).into_owned();
        Some(s)
    }

    pub fn finish(&mut self) -> Option<String> {
        (!self.pending.is_empty()).then(|| String::from_utf8_lossy(&std::mem::take(&mut self.pending)).into_owned())
    }
}

/// An answer split into the model's thinking (between `<think>` and `</think>`, as Qwen 3 and
/// DeepSeek write) and the reply itself.
pub fn split_thinking(text: &str) -> (Option<&str>, &str) {
    let t = text.trim_start();
    let Some(rest) = t.strip_prefix("<think>") else { return (None, text) };
    match rest.find("</think>") {
        Some(i) => (Some(rest[..i].trim()), rest[i + "</think>".len()..].trim_start()),
        // Still thinking.
        None => (Some(rest.trim()), ""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_characters_wait_until_whole() {
        let mut d = Utf8Stream::default();
        let e = "é".as_bytes();
        assert_eq!(d.push(b"caf"), Some("caf".into()));
        assert_eq!(d.push(&e[..1]), None);
        assert_eq!(d.push(&e[1..]), Some("é".into()));
        let crab = "🦀".as_bytes();
        assert_eq!(d.push(&crab[..3]), None);
        assert_eq!(d.push(&[crab[3], b'!']), Some("🦀!".into()));
        assert_eq!(d.finish(), None);
    }

    #[test]
    fn thinking_is_separated() {
        assert_eq!(split_thinking("<think>\nhmm\n</think>\n\nHello"), (Some("hmm"), "Hello"));
        assert_eq!(split_thinking("<think>still going"), (Some("still going"), ""));
        assert_eq!(split_thinking("Plain answer"), (None, "Plain answer"));
    }

    #[test]
    fn chatml_layout() {
        let c = [Message { role: Role::System, text: "Be brief.".into() }, Message { role: Role::User, text: "Hi".into() }];
        assert_eq!(chatml(&c), "<|im_start|>system\nBe brief.<|im_end|>\n<|im_start|>user\nHi<|im_end|>\n<|im_start|>assistant\n");
    }

    /// The Lab's step-by-step writing, on a real model. FILESCAPE_TEST_GGUF=path cargo test --release -- --ignored llm
    #[test]
    #[ignore]
    fn predicts_with_probabilities() {
        let Ok(path) = std::env::var("FILESCAPE_TEST_GGUF") else { return };
        let e = Engine::start();
        e.send(Request::Load(LoadRequest { path: path.into(), context: 512, gpu: false }));
        let settings = crate::lab::Settings { temperature: 0.0, ..Default::default() };
        let run = |e: &Engine, continuation: &str, steps| {
            e.send(Request::Predict { text: "Hello there".into(), chat: false, continuation: continuation.into(), steps, settings });
            loop {
                match e.rx.recv_timeout(std::time::Duration::from_secs(120)).unwrap() {
                    Event::Predicted(r) => return r,
                    Event::Error(err) => panic!("{err}"),
                    _ => {}
                }
            }
        };
        let a = run(&e, "", 5);
        assert!(!a.steps.is_empty() && a.prompt_tokens > 0);
        for s in &a.steps {
            // Greedy: the chosen token is the likeliest, and the probabilities are sane.
            assert_eq!(s.chosen.id, s.top[0].id);
            assert!(s.top.windows(2).all(|w| w[0].raw >= w[1].raw));
            assert!(s.chosen.raw > 0.0 && s.chosen.raw <= 1.0 && s.chosen.adjusted == 1.0);
        }
        // The same again gives the same tokens, and carrying on from the first token matches.
        assert_eq!(run(&e, "", 5).steps.iter().map(|s| s.chosen.id).collect::<Vec<_>>(), a.steps.iter().map(|s| s.chosen.id).collect::<Vec<_>>());
        let b = run(&e, &a.steps[0].chosen.text, 1);
        assert_eq!(b.steps.len(), 1);
    }

    /// Looking inside a real model: the last layer's guess matches what the model itself writes.
    #[test]
    #[ignore]
    fn looks_inside() {
        let Ok(path) = std::env::var("FILESCAPE_TEST_GGUF") else { return };
        let e = Engine::start();
        e.send(Request::Load(LoadRequest { path: path.into(), context: 512, gpu: false }));
        let next = |e: &Engine| loop {
            match e.rx.recv_timeout(std::time::Duration::from_secs(120)).unwrap() {
                Event::Error(err) => panic!("{err}"),
                Event::Loading | Event::Loaded { .. } => {}
                ev => return ev,
            }
        };
        e.send(Request::Look { text: "The capital of France is".into(), chat: false });
        let Event::Looked(inside) = next(&e) else { panic!("expected a look") };
        assert!(!inside.tokens.is_empty());
        assert!(inside.change.iter().skip(1).flatten().all(|c| c.is_finite() && *c >= 0.0));
        let Event::Lensed { token, layers } = next(&e) else { panic!("expected the layers' guesses") };
        assert_eq!(token, inside.tokens.len() - 1);
        assert_eq!(layers.len(), inside.change.len());
        for l in &layers {
            assert!(!l.is_empty() && l.windows(2).all(|w| w[0].1 >= w[1].1) && l[0].1 <= 1.0);
        }
        let settings = crate::lab::Settings { temperature: 0.0, ..Default::default() };
        e.send(Request::Predict { text: "The capital of France is".into(), chat: false, continuation: String::new(), steps: 1, settings });
        let Event::Predicted(run) = next(&e) else { panic!("expected a prediction") };
        let last = layers.last().unwrap();
        assert_eq!(last[0].0, run.steps[0].chosen.text, "the last layer's guess is the model's own");
        assert!((last[0].1 - run.steps[0].chosen.raw).abs() < 0.02, "{} vs {}", last[0].1, run.steps[0].chosen.raw);
        // Another token of the same look.
        e.send(Request::Lens(0));
        let Event::Lensed { token: 0, .. } = next(&e) else { panic!("expected token 0") };
    }

    /// The Lab's speed test on a real model.
    #[test]
    #[ignore]
    fn speed_test_runs() {
        let Ok(path) = std::env::var("FILESCAPE_TEST_GGUF") else { return };
        let e = Engine::start();
        e.send(Request::Load(LoadRequest { path: path.into(), context: 1024, gpu: false }));
        e.send(Request::Bench(crate::lab::BenchSettings { read: 128, write: 16, runs: 3 }));
        let mut runs = 0;
        loop {
            match e.rx.recv_timeout(std::time::Duration::from_secs(120)).unwrap() {
                Event::Benching(_, of) => {
                    runs += 1;
                    assert_eq!(of, 3);
                }
                Event::Benched { read_speed, write_speed, first_token, threads } => {
                    assert!(read_speed > 0.0 && write_speed > 0.0 && first_token > 0.0 && threads > 0);
                    break;
                }
                Event::Error(err) => panic!("{err}"),
                _ => {}
            }
        }
        assert_eq!(runs, 3);
        // Prediction still works after the test emptied the context.
        e.send(Request::Predict { text: "Hi".into(), chat: false, continuation: String::new(), steps: 2, settings: Default::default() });
        loop {
            match e.rx.recv_timeout(std::time::Duration::from_secs(120)).unwrap() {
                Event::Predicted(r) => {
                    assert!(!r.steps.is_empty());
                    break;
                }
                Event::Error(err) => panic!("{err}"),
                _ => {}
            }
        }
    }

    /// Runs a real GGUF model end to end. FILESCAPE_TEST_GGUF=path cargo test --release -- --ignored llm
    #[test]
    #[ignore]
    fn runs_a_model() {
        let Ok(path) = std::env::var("FILESCAPE_TEST_GGUF") else { return };
        let e = Engine::start();
        e.send(Request::Load(LoadRequest { path: path.into(), context: 1024, gpu: false }));
        e.send(Request::Ask { conversation: vec![Message { role: Role::User, text: "Say hello.".into() }], sampling: Sampling { temperature: 0.0, max_tokens: 16 } });
        let mut text = String::new();
        loop {
            match e.rx.recv_timeout(std::time::Duration::from_secs(120)).unwrap() {
                Event::Text(t) => text.push_str(&t),
                Event::Done(s) => {
                    assert!(s.tokens > 0);
                    break;
                }
                Event::Error(err) => panic!("{err}"),
                _ => {}
            }
        }
        // A second turn reuses the first turn's tokens.
        e.send(Request::Ask {
            conversation: vec![
                Message { role: Role::User, text: "Say hello.".into() },
                Message { role: Role::Assistant, text },
                Message { role: Role::User, text: "Again.".into() },
            ],
            sampling: Sampling { temperature: 0.0, max_tokens: 8 },
        });
        loop {
            match e.rx.recv_timeout(std::time::Duration::from_secs(120)).unwrap() {
                Event::Done(s) => {
                    assert!(s.reused > 0, "{s:?}");
                    break;
                }
                Event::Error(err) => panic!("{err}"),
                _ => {}
            }
        }
    }
}
