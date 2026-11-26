//! Comparing two models from what their headers say: their facts side by side, how their numbers
//! are stored, which tensors they share and how those differ, and which metadata differs. The
//! weights themselves aren't compared. Tensors are matched by name, with Hugging Face names
//! translated to GGUF's, so a GGUF download can be compared with the original it was made from.

use crate::inspect::Inspection;
use std::collections::HashMap;

/// One fact from either model, side by side.
pub struct FactRow {
    pub label: &'static str,
    pub a: Option<String>,
    pub b: Option<String>,
}

impl FactRow {
    pub fn differs(&self) -> bool {
        self.a.is_some() && self.b.is_some() && self.a != self.b
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Same,
    /// Same shape, stored in another number type.
    Kind,
    /// Another shape.
    Shape,
    OnlyA,
    OnlyB,
}

impl Change {
    pub fn label(self) -> &'static str {
        match self {
            Change::Same => "same",
            Change::Kind => "number type",
            Change::Shape => "shape",
            Change::OnlyA => "only in the first",
            Change::OnlyB => "only in the second",
        }
    }
}

/// A tensor of either model, matched by name.
pub struct TensorRow {
    /// The name, as GGUF would write it.
    pub name: String,
    pub a: Option<usize>,
    pub b: Option<usize>,
    pub change: Change,
}

/// What the two models have in common, in one sentence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Same tensors, shapes and number types.
    SameLayout,
    /// Same tensors and shapes, other number types: likely one model at two precisions.
    SamePrecisionDiffers,
    /// Most tensors match by name and shape.
    MostlyShared,
    /// Same kinds of layers, other sizes (a smaller or larger model of the same family).
    SameFamily,
    Different,
    /// One of them has no tensor list (it couldn't be read), so only facts are compared.
    NoTensors,
}

impl Verdict {
    pub fn explain(self) -> &'static str {
        match self {
            Verdict::SameLayout => {
                "Same layout: every tensor has the same name, shape and number type. They may be the same model, or two fine-tunes of one."
            }
            Verdict::SamePrecisionDiffers => {
                "Same architecture and size, stored at different precisions. Most likely one model compressed two ways (or a fine-tune of it)."
            }
            Verdict::MostlyShared => "Mostly the same architecture: most tensors match, but some are added, removed or reshaped.",
            Verdict::SameFamily => "The same kind of model at a different size: the tensors have the same names but other shapes.",
            Verdict::Different => "Different models: few tensors have the same name and shape.",
            Verdict::NoTensors => "Only the facts and metadata can be compared: one of the models has no tensor list Filescape could read.",
        }
    }
}

pub struct Comparison {
    pub facts: Vec<FactRow>,
    pub tensors: Vec<TensorRow>,
    pub verdict: Verdict,
    /// Tensors of each kind of change, by `Change`.
    pub same: usize,
    pub kind: usize,
    pub shape: usize,
    pub only_a: usize,
    pub only_b: usize,
    /// Bytes of each part, by `Part::ALL`, in each model.
    pub parts: [(u64, u64); 7],
    /// Metadata that differs: (key, value in a, value in b). Vocabulary arrays are left out.
    pub meta: Vec<(String, Option<String>, Option<String>)>,
}

/// GGUF's name for a Hugging Face tensor name (`model.layers.3.self_attn.q_proj.weight` →
/// `blk.3.attn_q.weight`). Names already in GGUF form, or not known, are kept.
pub fn canonical(name: &str) -> String {
    let n = name.strip_prefix("model.language_model.").map(|r| format!("model.{r}")).unwrap_or_else(|| name.to_string());
    let (stem, suffix) = match n.rsplit_once('.') {
        Some((s, x)) if x == "weight" || x == "bias" => (s.to_string(), x),
        _ => return n,
    };
    let whole = match stem.as_str() {
        "model.embed_tokens" | "transformer.wte" => Some("token_embd"),
        "lm_head" => Some("output"),
        "model.norm" | "transformer.ln_f" => Some("output_norm"),
        _ => None,
    };
    if let Some(w) = whole {
        return format!("{w}.{suffix}");
    }
    let Some(rest) = stem.strip_prefix("model.layers.") else { return n };
    let Some((layer, part)) = rest.split_once('.') else { return n };
    let mapped = match part {
        "self_attn.q_proj" => "attn_q",
        "self_attn.k_proj" => "attn_k",
        "self_attn.v_proj" => "attn_v",
        "self_attn.o_proj" => "attn_output",
        "self_attn.q_norm" => "attn_q_norm",
        "self_attn.k_norm" => "attn_k_norm",
        "input_layernorm" => "attn_norm",
        "post_attention_layernorm" => "ffn_norm",
        "mlp.gate_proj" => "ffn_gate",
        "mlp.up_proj" => "ffn_up",
        "mlp.down_proj" => "ffn_down",
        "mlp.gate" => "ffn_gate_inp",
        _ => return n,
    };
    format!("blk.{layer}.{mapped}.{suffix}")
}

/// Keys whose values are too long or too obviously different to list.
fn skip_key(k: &str) -> bool {
    k.starts_with("tokenizer.ggml.tokens")
        || k.starts_with("tokenizer.ggml.token_type")
        || k.starts_with("tokenizer.ggml.scores")
        || k.starts_with("tokenizer.ggml.merges")
        || k.starts_with("tokenizer.chat_template")
}

pub fn compare(a: &Inspection, b: &Inspection) -> Comparison {
    // Facts, in the first model's order, then any only the second has.
    let mut facts: Vec<FactRow> = Vec::new();
    for f in a.facts.iter().chain(&b.facts) {
        if facts.iter().any(|r| r.label == f.label) {
            continue;
        }
        let get = |d: &Inspection| d.facts.iter().find(|x| x.label == f.label).map(|x| x.value.clone());
        facts.push(FactRow { label: f.label, a: get(a), b: get(b) });
    }
    // Tensors, matched by name.
    let mut by_name: HashMap<String, usize> = HashMap::new();
    for (i, t) in b.tensors.iter().enumerate() {
        by_name.insert(canonical(&t.name), i);
    }
    let mut tensors = Vec::with_capacity(a.tensors.len().max(b.tensors.len()));
    let mut used = vec![false; b.tensors.len()];
    for (i, t) in a.tensors.iter().enumerate() {
        let name = canonical(&t.name);
        let j = by_name.get(&name).copied();
        let change = match j {
            None => Change::OnlyA,
            Some(j) => {
                used[j] = true;
                let u = &b.tensors[j];
                if u.shape != t.shape {
                    Change::Shape
                } else if u.kind != t.kind {
                    Change::Kind
                } else {
                    Change::Same
                }
            }
        };
        tensors.push(TensorRow { name, a: Some(i), b: j, change });
    }
    for (j, t) in b.tensors.iter().enumerate() {
        if !used[j] {
            tensors.push(TensorRow { name: canonical(&t.name), a: None, b: Some(j), change: Change::OnlyB });
        }
    }
    let count = |c: Change| tensors.iter().filter(|r| r.change == c).count();
    let (same, kind, shape, only_a, only_b) = (count(Change::Same), count(Change::Kind), count(Change::Shape), count(Change::OnlyA), count(Change::OnlyB));
    let total = tensors.len().max(1);
    // A model may tie its output to its embeddings (no output tensor) or keep extra small tables
    // such as rope frequencies: a few unmatched tensors don't make two layouts different.
    let unmatched = only_a + only_b;
    let small_gap = unmatched <= 2 || unmatched * 100 <= total;
    let verdict = if a.tensors.is_empty() || b.tensors.is_empty() {
        Verdict::NoTensors
    } else if shape == 0 && small_gap && same + kind > 0 {
        if kind == 0 { Verdict::SameLayout } else { Verdict::SamePrecisionDiffers }
    } else if (same + kind) * 10 >= total * 7 {
        Verdict::MostlyShared
    } else if (same + kind + shape) * 10 >= total * 7 && shape > 0 {
        Verdict::SameFamily
    } else {
        Verdict::Different
    };
    let mut parts = [(0u64, 0u64); 7];
    for t in &a.tensors {
        parts[crate::inspect::part_of(&canonical(&t.name)).index()].0 += t.bytes;
    }
    for t in &b.tensors {
        parts[crate::inspect::part_of(&canonical(&t.name)).index()].1 += t.bytes;
    }
    // Metadata.
    let bm: HashMap<&str, &str> = b.meta.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let mut meta = Vec::new();
    for (k, v) in &a.meta {
        if skip_key(k) {
            continue;
        }
        match bm.get(k.as_str()) {
            Some(w) if *w == v => {}
            Some(w) => meta.push((k.clone(), Some(v.clone()), Some(w.to_string()))),
            None => meta.push((k.clone(), Some(v.clone()), None)),
        }
    }
    let am: HashMap<&str, ()> = a.meta.iter().map(|(k, _)| (k.as_str(), ())).collect();
    for (k, v) in &b.meta {
        if !skip_key(k) && !am.contains_key(k.as_str()) {
            meta.push((k.clone(), None, Some(v.clone())));
        }
    }
    Comparison { facts, tensors, verdict, same, kind, shape, only_a, only_b, parts, meta }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspect::{Fact, How, Tensor};

    fn t(name: &str, kind: &str, shape: &[u64]) -> Tensor {
        let elements = shape.iter().product();
        Tensor { name: name.into(), kind: kind.into(), shape: shape.to_vec(), elements, bytes: elements, file: 0, offset: 0 }
    }

    fn model(tensors: Vec<Tensor>) -> Inspection {
        Inspection {
            tensors,
            facts: vec![Fact { label: "Layers", value: "2".into(), how: How::Verified }],
            meta: vec![("general.name".into(), "A".into())],
            ..Default::default()
        }
    }

    #[test]
    fn hugging_face_names_become_gguf_names() {
        assert_eq!(canonical("model.layers.12.self_attn.q_proj.weight"), "blk.12.attn_q.weight");
        assert_eq!(canonical("model.layers.0.mlp.down_proj.weight"), "blk.0.ffn_down.weight");
        assert_eq!(canonical("model.layers.0.post_attention_layernorm.weight"), "blk.0.ffn_norm.weight");
        assert_eq!(canonical("model.embed_tokens.weight"), "token_embd.weight");
        assert_eq!(canonical("lm_head.weight"), "output.weight");
        assert_eq!(canonical("model.language_model.norm.weight"), "output_norm.weight");
        assert_eq!(canonical("blk.3.attn_k.weight"), "blk.3.attn_k.weight");
        assert_eq!(canonical("vision_tower.patch_embed.weight"), "vision_tower.patch_embed.weight");
    }

    #[test]
    fn a_quantised_copy_of_the_original() {
        let a = model(vec![
            t("model.embed_tokens.weight", "BF16", &[100, 8]),
            t("model.layers.0.self_attn.q_proj.weight", "BF16", &[8, 8]),
            t("model.layers.0.input_layernorm.weight", "BF16", &[8]),
        ]);
        let mut b = model(vec![t("token_embd.weight", "Q4_K", &[100, 8]), t("blk.0.attn_q.weight", "Q4_K", &[8, 8]), t("blk.0.attn_norm.weight", "F32", &[8])]);
        b.meta[0].1 = "B".into();
        let c = compare(&a, &b);
        assert_eq!(c.verdict, Verdict::SamePrecisionDiffers);
        assert_eq!((c.same, c.kind, c.shape, c.only_a, c.only_b), (0, 3, 0, 0, 0));
        assert_eq!(c.meta, vec![("general.name".to_string(), Some("A".to_string()), Some("B".to_string()))]);
        assert!(!c.facts[0].differs());
    }

    #[test]
    fn verdicts() {
        let a = model(vec![t("a.weight", "F16", &[4, 4]), t("b.weight", "F16", &[4, 4]), t("c.weight", "F16", &[4])]);
        assert_eq!(compare(&a, &a).verdict, Verdict::SameLayout);
        let bigger = model(vec![t("a.weight", "F16", &[8, 8]), t("b.weight", "F16", &[8, 8]), t("c.weight", "F16", &[8])]);
        assert_eq!(compare(&a, &bigger).verdict, Verdict::SameFamily);
        let other = model(vec![t("x.weight", "F16", &[4, 4]), t("y.weight", "F16", &[4, 4]), t("z.weight", "F16", &[4])]);
        let c = compare(&a, &other);
        assert_eq!(c.verdict, Verdict::Different);
        assert_eq!((c.only_a, c.only_b), (3, 3));
    }
}
