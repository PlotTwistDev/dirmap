//! The Lab's arithmetic: turning a model's scores for every token into probabilities, and showing
//! what each sampling setting does to them. It's done here rather than by llama.cpp's samplers so
//! that what the Lab shows is exactly what it picks from. The order matches Ask AI's sampler
//! chain: top-k, top-p, min-p, then temperature.

/// The sampling settings the Lab lets you play with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// 0: always the likeliest token.
    pub temperature: f32,
    /// Keep only the k likeliest tokens; 0 keeps them all.
    pub top_k: u32,
    /// Keep the likeliest tokens that together make up this share; 1 keeps them all.
    pub top_p: f32,
    /// Drop tokens less likely than this share of the likeliest one; 0 keeps them all.
    pub min_p: f32,
    /// The same seed and settings pick the same tokens again.
    pub seed: u64,
}

impl Default for Settings {
    /// Ask AI's settings.
    fn default() -> Self {
        Settings { temperature: 0.6, top_k: 40, top_p: 0.95, min_p: 0.05, seed: 1 }
    }
}

/// One of the likeliest next tokens.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub id: i32,
    pub text: String,
    /// The model's own probability.
    pub raw: f32,
    /// After the settings: 0 when a setting left it out.
    pub adjusted: f32,
}

/// One token the Lab wrote, with what else it could have been.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub chosen: Choice,
    /// The likeliest tokens by the model's own probability, the chosen one among them or not.
    pub top: Vec<Choice>,
    /// How many tokens the settings left to pick from.
    pub kept: usize,
    /// The model chose to end here (an end-of-text token).
    pub end: bool,
}

/// What a Lab run produced.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Run {
    pub steps: Vec<Step>,
    /// Tokens in the text the model read.
    pub prompt_tokens: usize,
    pub seconds: f32,
}

/// Probabilities from scores (softmax), safely for large scores.
pub fn softmax(logits: &[f32], temperature: f32) -> Vec<f32> {
    let t = if temperature > 0.0 { temperature } else { 1.0 };
    let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let e: Vec<f32> = logits.iter().map(|l| ((l - max) / t).exp()).collect();
    let sum: f32 = e.iter().sum();
    e.into_iter().map(|x| x / sum).collect()
}

/// The token indices the settings keep, likeliest first, and their probabilities after the
/// settings (summing to 1).
pub fn adjust(logits: &[f32], s: &Settings) -> Vec<(usize, f32)> {
    let raw = softmax(logits, 1.0);
    let mut order: Vec<usize> = (0..logits.len()).collect();
    order.sort_unstable_by(|&a, &b| raw[b].total_cmp(&raw[a]).then(a.cmp(&b)));
    if s.top_k > 0 {
        order.truncate(s.top_k as usize);
    }
    if s.top_p < 1.0 {
        // Over what top-k left, as llama.cpp does.
        let total: f32 = order.iter().map(|&i| raw[i]).sum();
        let mut acc = 0.0;
        let mut keep = 0;
        for &i in &order {
            acc += raw[i] / total;
            keep += 1;
            if acc >= s.top_p {
                break;
            }
        }
        order.truncate(keep.max(1));
    }
    if s.min_p > 0.0
        && let Some(&best) = order.first()
    {
        let floor = raw[best] * s.min_p;
        order.retain(|&i| raw[i] >= floor);
    }
    if s.temperature <= 0.0 {
        return vec![(order[0], 1.0)];
    }
    let kept: Vec<f32> = order.iter().map(|&i| logits[i]).collect();
    let p = softmax(&kept, s.temperature);
    order.into_iter().zip(p).collect()
}

/// Picks from `adjusted` with a uniform number in [0, 1).
pub fn pick(adjusted: &[(usize, f32)], r: f32) -> usize {
    let mut acc = 0.0;
    for &(i, p) in adjusted {
        acc += p;
        if r < acc {
            return i;
        }
    }
    adjusted.last().map_or(0, |a| a.0)
}

/// A small, fast random number generator (SplitMix64): the same seed gives the same picks.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    /// A number in [0, 1).
    pub fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// Everything about one step but the token texts: the pick, the `n` likeliest tokens and how
/// many were kept. Returns (chosen index, its raw and adjusted probability, top (index, raw,
/// adjusted), kept).
pub fn step(logits: &[f32], s: &Settings, rng: &mut Rng, n: usize) -> (usize, f32, f32, Vec<(usize, f32, f32)>, usize) {
    let raw = softmax(logits, 1.0);
    let adjusted = adjust(logits, s);
    let chosen = if s.temperature <= 0.0 { adjusted[0].0 } else { pick(&adjusted, rng.next()) };
    let adj_of = |i: usize| adjusted.iter().find(|a| a.0 == i).map_or(0.0, |a| a.1);
    let mut top: Vec<usize> = (0..logits.len()).collect();
    let n = n.min(top.len());
    if n > 0 {
        top.select_nth_unstable_by(n - 1, |&a, &b| raw[b].total_cmp(&raw[a]));
        top.truncate(n);
    }
    top.sort_unstable_by(|&a, &b| raw[b].total_cmp(&raw[a]).then(a.cmp(&b)));
    let top = top.into_iter().map(|i| (i, raw[i], adj_of(i))).collect();
    (chosen, raw[chosen], adj_of(chosen), top, adjusted.len())
}

/// What a speed test measures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BenchSettings {
    /// Tokens of text to read, as when a long question or document is sent.
    pub read: u32,
    /// Tokens to write.
    pub write: u32,
    /// The test is run this many times and the middle result kept.
    pub runs: u32,
}

impl Default for BenchSettings {
    fn default() -> Self {
        BenchSettings { read: 512, write: 128, runs: 3 }
    }
}

/// One speed test, as kept in the results file.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BenchResult {
    pub model: String,
    pub path: std::path::PathBuf,
    /// When it ran, seconds since 1970.
    pub when: i64,
    pub context: u32,
    /// On the graphics chip (Apple silicon) rather than the processor.
    pub gpu: bool,
    pub threads: u32,
    pub read: u32,
    pub write: u32,
    pub runs: u32,
    /// Tokens read a second (the middle run).
    pub read_speed: f32,
    /// Tokens written a second (the middle run).
    pub write_speed: f32,
    /// Seconds before the first token of an answer: reading the text, then one token.
    pub first_token: f32,
}

/// The middle value (the mean of the two middle ones for an even count).
pub fn median(v: &[f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mut v = v.to_vec();
    v.sort_by(f32::total_cmp);
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

/// Where speed test results are kept: beside Filescape's settings.
pub fn results_file() -> Option<std::path::PathBuf> {
    Some(crate::templates::app_folder()?.join("lab-speed.json"))
}

/// Results kept so far, newest first. A missing or unreadable file is no results.
pub fn load_results(file: &std::path::Path) -> Vec<BenchResult> {
    std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Adds a result (newest first, at most 200 kept) and saves.
pub fn save_result(file: &std::path::Path, r: BenchResult) -> Result<Vec<BenchResult>, String> {
    let mut all = load_results(file);
    all.insert(0, r);
    all.truncate(200);
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&all).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, file).map_err(|e| e.to_string())?;
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: [f32; 5] = [2.0, 1.0, 0.5, 0.0, -3.0];

    #[test]
    fn softmax_sums_to_one_and_orders() {
        let p = softmax(&L, 1.0);
        assert!((p.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        assert!(p[0] > p[1] && p[1] > p[2] && p[3] > p[4]);
        // Huge scores don't overflow.
        let p = softmax(&[1000.0, 999.0], 1.0);
        assert!((p[0] - 0.7311).abs() < 1e-3, "{p:?}");
    }

    #[test]
    fn settings_cut_and_reshape() {
        let all = Settings { temperature: 1.0, top_k: 0, top_p: 1.0, min_p: 0.0, seed: 1 };
        assert_eq!(adjust(&L, &all).len(), 5);
        let k2 = adjust(&L, &Settings { top_k: 2, ..all });
        assert_eq!(k2.iter().map(|a| a.0).collect::<Vec<_>>(), [0, 1]);
        assert!((k2[0].1 + k2[1].1 - 1.0).abs() < 1e-5);
        // Top-p 0.5: the first token alone is over half.
        assert_eq!(adjust(&L, &Settings { top_p: 0.5, ..all }).len(), 1);
        // Min-p 0.3: drops tokens under 30% of the likeliest's probability.
        let raw = softmax(&L, 1.0);
        let m = adjust(&L, &Settings { min_p: 0.3, ..all });
        assert!(m.iter().all(|a| raw[a.0] >= raw[0] * 0.3));
        assert_eq!(m.len(), 2, "57% and 21% are over 30% of 57%; 13% isn't");
        // Low temperature sharpens; temperature 0 keeps only the likeliest.
        let cold = adjust(&L, &Settings { temperature: 0.2, ..all });
        assert!(cold[0].1 > 0.99);
        assert_eq!(adjust(&L, &Settings { temperature: 0.0, ..all }), vec![(0, 1.0)]);
    }

    #[test]
    fn same_seed_same_picks() {
        let s = Settings { temperature: 1.5, top_k: 0, top_p: 1.0, min_p: 0.0, seed: 7 };
        let run = |seed| {
            let mut r = Rng::new(seed);
            (0..50).map(|_| step(&L, &s, &mut r, 3).0).collect::<Vec<_>>()
        };
        assert_eq!(run(7), run(7));
        assert_ne!(run(7), run(8));
        // Every pick is one the settings allow, and over many the likeliest wins most often.
        let picks = run(3);
        assert!(picks.iter().filter(|&&i| i == 0).count() > picks.iter().filter(|&&i| i == 4).count());
    }

    #[test]
    fn step_reports_top_and_kept() {
        let s = Settings { temperature: 0.0, top_k: 3, ..Settings::default() };
        let (chosen, raw, adj, top, kept) = step(&L, &s, &mut Rng::new(1), 4);
        assert_eq!((chosen, adj, kept), (0, 1.0, 1));
        assert!((raw - softmax(&L, 1.0)[0]).abs() < 1e-6);
        assert_eq!(top.iter().map(|t| t.0).collect::<Vec<_>>(), [0, 1, 2, 3]);
        assert_eq!(top[3].2, 0.0, "left out by the settings");
    }

    #[test]
    fn medians() {
        assert_eq!(median(&[]), 0.0);
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
    }

    #[test]
    fn results_are_kept_newest_first() {
        let d = std::env::temp_dir().join(format!("filescape-lab-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let f = d.join("lab-speed.json");
        assert!(load_results(&f).is_empty());
        let r = |when| BenchResult {
            model: "M".into(),
            path: "/m.gguf".into(),
            when,
            context: 4096,
            gpu: false,
            threads: 4,
            read: 512,
            write: 128,
            runs: 3,
            read_speed: 100.0,
            write_speed: 10.0,
            first_token: 5.2,
        };
        save_result(&f, r(1)).unwrap();
        let all = save_result(&f, r(2)).unwrap();
        assert_eq!(all.iter().map(|r| r.when).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(load_results(&f), all);
        std::fs::write(&f, "not json").unwrap();
        assert!(load_results(&f).is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn rng_is_in_range() {
        let mut r = Rng::new(42);
        for _ in 0..1000 {
            let x = r.next();
            assert!((0.0..1.0).contains(&x));
        }
    }
}
