//! What kind of model a file is: a language model, an image generator, a speech model and so on.
//!
//! Only from what is already known without reading weights: the GGUF architecture and type, the
//! class a Hugging Face `config.json` or `model_index.json` names, the folder an app keeps it in,
//! and, last, words in its name. A guess from the name or folder is marked as unsure, and a model
//! nothing tells about is [`Kind::Unknown`] rather than assumed to be a language model.

use crate::models::{Format, Model};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Kind {
    Language,
    Image,
    Video,
    Audio,
    Embedding,
    Vision,
    Multimodal,
    Adapter,
    Unknown,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Language => "Language",
            Kind::Image => "Image generation",
            Kind::Video => "Video generation",
            Kind::Audio => "Audio and speech",
            Kind::Embedding => "Embedding",
            Kind::Vision => "Vision",
            Kind::Multimodal => "Multimodal",
            Kind::Adapter => "Adapter or LoRA",
            Kind::Unknown => "Unidentified",
        }
    }

    /// A short label, for filter chips.
    pub fn short(self) -> &'static str {
        match self {
            Kind::Language => "Language",
            Kind::Image => "Image",
            Kind::Video => "Video",
            Kind::Audio => "Audio",
            Kind::Embedding => "Embedding",
            Kind::Vision => "Vision",
            Kind::Multimodal => "Multimodal",
            Kind::Adapter => "Adapters",
            Kind::Unknown => "Other",
        }
    }

    pub fn explain(self) -> &'static str {
        match self {
            Kind::Language => "Reads and writes text: chat, writing, code. The kind Filescape can run, when it's a GGUF file.",
            Kind::Image => "Makes pictures from a description (Stable Diffusion, FLUX and similar), or parts of such a model.",
            Kind::Video => "Makes short videos from a description or a picture.",
            Kind::Audio => "Turns speech into text, or text into speech or music.",
            Kind::Embedding => "Turns text into numbers that capture its meaning, for search and finding similar documents.",
            Kind::Vision => "Sees pictures, usually as the eyes of a language model (a vision projector or encoder).",
            Kind::Multimodal => "A language model that also takes pictures (or sound) as input.",
            Kind::Adapter => "A small add-on that changes another model's style or skills (LoRA). It doesn't run alone.",
            Kind::Unknown => "Nothing in the files says what this model does.",
        }
    }
}

/// The kind of model, and whether the files say so (`true`) or it's a guess from the name or
/// folder (`false`).
pub fn of(m: &Model) -> (Kind, bool) {
    let arch = m.info.arch.as_deref().unwrap_or("").to_ascii_lowercase();
    let hint = m.info.hint.as_deref().unwrap_or("").to_ascii_lowercase();
    let path = m.files.first().map(|f| f.path.to_string_lossy().to_ascii_lowercase().replace('\\', "/")).unwrap_or_default();
    let name = format!("{} {}", m.name.to_ascii_lowercase(), path);

    // What the files themselves say.
    if m.info.companion {
        if arch.contains("vision") || arch.contains("clip") || arch.contains("projector") {
            return (Kind::Vision, true);
        }
        return (Kind::Adapter, true);
    }
    if let Some(k) = from_hint(&hint) {
        return (k, true);
    }
    if m.format == Format::Gguf && !arch.is_empty() {
        if let Some(k) = from_gguf_arch(&arch) {
            return (k, true);
        }
    }

    // Where an app keeps it: ComfyUI and Automatic1111 sort models into folders by what they do.
    let folder = |s: &str| path.contains(&format!("/{s}/"));
    if folder("loras") || folder("lora") || folder("lycoris") || folder("hypernetworks") || (folder("embeddings") && folder("stable-diffusion-webui")) {
        return (Kind::Adapter, false);
    }
    if folder("checkpoints") || folder("stable-diffusion") || folder("unet") || folder("diffusion_models") || folder("vae") || folder("controlnet") || folder("upscale_models") {
        return (Kind::Image, false);
    }
    if folder("clip_vision") {
        return (Kind::Vision, false);
    }

    // Last, words in the name.
    let has = |words: &[&str]| words.iter().any(|w| name.contains(w));
    if has(&["lora", "adapter_model", "-adapter"]) {
        return (Kind::Adapter, false);
    }
    if has(&["text-to-video", "wan2", "hunyuanvideo", "hunyuan-video", "ltx-video", "ltxv", "cogvideo", "mochi", "stable-video", "svd_xt"]) {
        return (Kind::Video, false);
    }
    if has(&["stable-diffusion", "stable_diffusion", "sdxl", "sd_xl", "sd3", "sd-v1", "sd15", "flux", "dreamshaper", "juggernaut", "realvis", "pony", "kandinsky", "pixart", "playground-v", "dall", "imagen", "/unet", "/vae"]) {
        return (Kind::Image, false);
    }
    if has(&["whisper", "wav2vec", "tts", "bark", "musicgen", "audiogen", "parler", "kokoro", "speecht5", "seamless", "encodec", "hubert"]) {
        return (Kind::Audio, false);
    }
    if has(&["embed", "bge-", "e5-", "gte-", "minilm", "mpnet", "sentence-t", "nomic-bert", "jina-", "mxbai"]) {
        return (Kind::Embedding, false);
    }
    if has(&["llava", "-vl", "_vl", "vision", "pixtral", "moondream", "minicpm-v", "idefics", "paligemma", "qwen2.5-omni", "omni"]) {
        return (Kind::Multimodal, false);
    }
    if has(&["clip", "siglip", "dinov", "vit-"]) {
        return (Kind::Vision, false);
    }
    if has(&[
        "llama", "qwen", "gemma", "mistral", "mixtral", "phi-", "phi3", "phi4", "deepseek", "gpt", "falcon", "starcoder", "codellama", "granite", "olmo",
        "smollm", "tinyllama", "command-r", "yi-", "internlm", "chatglm", "glm-4", "baichuan", "vicuna", "zephyr", "hermes", "openchat", "nemotron",
    ]) {
        return (Kind::Language, false);
    }
    (Kind::Unknown, false)
}

/// From GGUF `general.architecture`, which llama.cpp knows exactly.
fn from_gguf_arch(arch: &str) -> Option<Kind> {
    Some(match arch {
        "bert" | "nomic-bert" | "nomic-bert-moe" | "jina-bert-v2" | "jina-bert-v3" | "neo-bert" | "modern-bert" | "t5encoder" => Kind::Embedding,
        "clip" | "vision projector" | "mllama_vision" => Kind::Vision,
        "whisper" | "wavtokenizer-dec" | "outetts" => Kind::Audio,
        a if a.contains("vl") || a.ends_with("omni") => Kind::Multimodal,
        // Image models converted to GGUF by stable-diffusion.cpp and ComfyUI-GGUF.
        "flux" | "sd1" | "sdxl" | "sd3" | "aura" | "ltxv" | "hyvid" | "wan" | "lumina2" => {
            if matches!(arch, "ltxv" | "hyvid" | "wan") {
                Kind::Video
            } else {
                Kind::Image
            }
        }
        // Everything else llama.cpp loads as a GGUF architecture is a text model.
        _ => Kind::Language,
    })
}

/// From the class a Hugging Face config names: `Qwen3ForCausalLM`, `diffusers:FluxPipeline`,
/// `WhisperForConditionalGeneration`…
fn from_hint(h: &str) -> Option<Kind> {
    if h.is_empty() {
        return None;
    }
    let has = |words: &[&str]| words.iter().any(|w| h.contains(w));
    Some(if h.starts_with("diffusers:") {
        if has(&["video", "wan", "ltx", "hunyuan", "cogvideo", "mochi"]) {
            Kind::Video
        } else if has(&["audio", "music"]) {
            Kind::Audio
        } else {
            Kind::Image
        }
    } else if has(&["whisper", "speech", "wav2vec", "audio", "tts", "hubert", "musicgen", "encodec"]) {
        Kind::Audio
    } else if has(&["forimagetexttotext", "forvision2seq", "llava", "vl", "conditionalgeneration"]) && !has(&["t5", "bart", "pegasus", "marian"]) {
        Kind::Multimodal
    } else if has(&["clipmodel", "siglip", "visionmodel", "vitmodel", "dinov"]) {
        Kind::Vision
    } else if has(&["causallm", "lmhead", "forconditionalgeneration", "seq2seq"]) {
        Kind::Language
    } else if has(&["bertmodel", "robertamodel", "xlmroberta", "mpnet", "sentence", "embedding", "e5", "bge", "modernbert"]) {
        Kind::Embedding
    } else {
        return None;
    })
}

/// Whether a model writes text, so it can chat (Ask AI) and predict tokens (the Lab): not an
/// embedding, speech or vision-only model. Unidentified models are given the benefit of the doubt.
pub fn chats(m: &Model) -> bool {
    matches!(of(m).0, Kind::Language | Kind::Multimodal | Kind::Unknown)
}

/// A key that's the same for versions of one model saved at different precisions or by
/// different apps: "qwen3 8b" for `qwen3:8b`, `Qwen3-8B-Q4_K_M`, `Qwen/Qwen3-8B-GGUF (Q8_0)` and
/// `Qwen3-8B` (safetensors). The publisher, the precision and words such as "GGUF" are left out;
/// everything else (size, version, "instruct") is kept, so different models stay apart.
pub fn family_key(m: &Model) -> String {
    static QUANT: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let quant = QUANT.get_or_init(|| {
        regex::Regex::new(r"(?i)\b(i?q[1-8](_[0-9a-z]{1,3}){0,3}|bf16|fp16|f16|f32|fp32|fp8|mxfp4|int[48]|gguf|ggml|safetensors|latest|w[48]a16|awq|gptq|mlx|[0-9]bit)\b").unwrap()
    });
    // Ollama names models "family:tag"; the tag is a size and precision, as in a file name.
    let mut name = m.name.to_ascii_lowercase();
    // "Qwen/Qwen3-8B-GGUF (Q4_K_M)": the publisher and the precision in brackets go.
    if let Some(i) = name.find(" (") {
        name.truncate(i);
    }
    if let Some((_, after)) = name.rsplit_once('/') {
        name = after.to_string();
    }
    let spaced: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '_' { c } else { ' ' }).collect();
    let cleaned = quant.replace_all(&spaced, " ");
    let words: Vec<&str> = cleaned.split(|c: char| c.is_whitespace() || c == '_').filter(|w| !w.is_empty()).collect();
    let key = words.join(" ");
    if key.is_empty() { m.name.to_ascii_lowercase() } else { key }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{File, Info, Source};
    use std::path::PathBuf;

    fn model(name: &str, path: &str, format: Format, info: Info) -> Model {
        Model {
            name: name.into(),
            source: Source::Folder,
            format,
            info,
            files: vec![File { path: PathBuf::from(path), size: 1 << 30, not_local: false }],
            size: 1 << 30,
            copies: Vec::new(),
            extra: 0,
            same_as: Vec::new(),
            orphan: false,
        }
    }

    #[test]
    fn files_win_over_names() {
        let gguf = |arch: &str| Info { arch: Some(arch.into()), ..Default::default() };
        assert_eq!(of(&model("x", "/m/x.gguf", Format::Gguf, gguf("qwen3"))), (Kind::Language, true));
        assert_eq!(of(&model("flux-ish", "/m/x.gguf", Format::Gguf, gguf("llama"))), (Kind::Language, true));
        assert_eq!(of(&model("x", "/m/x.gguf", Format::Gguf, gguf("nomic-bert"))), (Kind::Embedding, true));
        assert_eq!(of(&model("x", "/m/x.gguf", Format::Gguf, gguf("qwen2vl"))), (Kind::Multimodal, true));
        let mmproj = Info { arch: Some("vision projector".into()), companion: true, ..Default::default() };
        assert_eq!(of(&model("x", "/m/mmproj.gguf", Format::Gguf, mmproj)), (Kind::Vision, true));
        let hint = |h: &str| Info { hint: Some(h.into()), ..Default::default() };
        assert_eq!(of(&model("x", "/m/model.safetensors", Format::Safetensors, hint("diffusers:FluxPipeline"))), (Kind::Image, true));
        assert_eq!(of(&model("x", "/m/model.safetensors", Format::Safetensors, hint("WhisperForConditionalGeneration"))), (Kind::Audio, true));
        assert_eq!(of(&model("x", "/m/model.safetensors", Format::Safetensors, hint("Qwen3ForCausalLM"))), (Kind::Language, true));
    }

    #[test]
    fn versions_of_one_model_share_a_key() {
        let k = |n: &str| family_key(&model(n, "/m/x.gguf", Format::Gguf, Info::default()));
        assert_eq!(k("qwen3:8b"), "qwen3 8b");
        assert_eq!(k("Qwen3-8B-Q4_K_M"), "qwen3 8b");
        assert_eq!(k("Qwen/Qwen3-8B-GGUF (Q8_0)"), "qwen3 8b");
        assert_eq!(k("Qwen/Qwen3-8B"), "qwen3 8b");
        assert_eq!(k("qwen3-8b-q8_0"), "qwen3 8b");
        assert_eq!(k("Llama-3.2-3B-Instruct-IQ4_XS"), "llama 3.2 3b instruct");
        assert_ne!(k("qwen3:8b"), k("qwen3:14b"));
        assert_ne!(k("Hermes-3-Llama-3.1-8B"), k("Llama-3.1-8B"));
    }

    #[test]
    fn guesses_are_marked_and_nothing_is_assumed() {
        let none = Info::default();
        assert_eq!(of(&model("sd_xl_base_1.0", "/d/sd_xl_base_1.0.safetensors", Format::Safetensors, none.clone())), (Kind::Image, false));
        assert_eq!(of(&model("thing", "/ComfyUI/models/loras/thing.safetensors", Format::Safetensors, none.clone())), (Kind::Adapter, false));
        assert_eq!(of(&model("ggml-whisper-base", "/d/w.bin", Format::PyTorch, none.clone())), (Kind::Audio, false));
        assert_eq!(of(&model("weights", "/d/weights.onnx", Format::Onnx, none.clone())), (Kind::Unknown, false));
    }
}
