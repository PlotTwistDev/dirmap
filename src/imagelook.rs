//! Looking closer at a picture: each channel on its own, the layers of a multilayer OpenEXR
//! file (a render's beauty, diffuse, depth and other passes), and the exposure of high dynamic
//! range pictures. The pixels are kept as half floats at the size shown, so changing the view
//! is quick and needs no second read of the file.

use half::f16;
use std::path::Path;

/// Channels × pixels kept at most (2 bytes each): pictures with many layers are kept smaller.
const SAMPLES: usize = 16 << 20;
/// EXR files whose pixels would take more than this in memory aren't read for their layers.
const EXR_BYTES: usize = 1 << 30;

pub struct Pass {
    /// "" for the main picture, otherwise the pass's name ("diffuse", "depth").
    pub name: String,
    pub w: usize,
    pub h: usize,
    /// Its size in the file.
    pub full: [u32; 2],
    /// Each channel's short name ("R", "G", "B", "A", "Z") and its pixels, row by row.
    pub channels: Vec<(String, Vec<f16>)>,
}

impl Pass {
    fn find(&self, n: &str) -> Option<usize> {
        self.channels.iter().position(|(c, _)| c.eq_ignore_ascii_case(n))
    }

    fn rgb(&self) -> Option<[usize; 3]> {
        Some([self.find("R")?, self.find("G")?, self.find("B")?])
    }

    /// What to call it in the list.
    pub fn label(&self) -> String {
        let chans: Vec<&str> = self.channels.iter().map(|c| c.0.as_str()).collect();
        let name = if self.name.is_empty() { "Main" } else { &self.name };
        format!("{name} ({})", chans.join(""))
    }
}

pub struct Planes {
    pub layers: Vec<Pass>,
    /// Scene-linear light (EXR, HDR), so exposure applies and the sRGB curve is used to show it.
    pub linear: bool,
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct View {
    pub layer: usize,
    /// None: the colour picture; Some(i): channel i alone, in grey.
    pub channel: Option<usize>,
    /// In stops (doublings of light).
    pub exposure: f32,
}

/// Linear light to the sRGB display curve.
pub fn srgb(v: f32) -> f32 {
    let v = v.max(0.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.min(1.0).powf(1.0 / 2.4) - 0.055 }
}

fn byte(v: f32) -> u8 {
    (v * 255.0).round().clamp(0.0, 255.0) as u8
}

/// The size to keep a w × h picture at: no more than `max` across or down, and no more than
/// `SAMPLES` samples over all its channels.
fn fit(w: usize, h: usize, channels: usize, max: usize) -> (usize, usize) {
    let mut s = (max as f64 / w.max(h).max(1) as f64).min(1.0);
    let budget = SAMPLES as f64 / channels.max(1) as f64;
    if (w * h) as f64 * s * s > budget {
        s = (budget / (w * h) as f64).sqrt();
    }
    (((w as f64 * s).round() as usize).max(1), ((h as f64 * s).round() as usize).max(1))
}

/// Shrink one channel by averaging the source pixels under each new one.
fn shrink(src: &[f32], w: usize, h: usize, tw: usize, th: usize) -> Vec<f16> {
    if (tw, th) == (w, h) {
        return src.iter().map(|&v| f16::from_f32(v)).collect();
    }
    let mut out = Vec::with_capacity(tw * th);
    for y in 0..th {
        let (y0, y1) = (y * h / th, ((y + 1) * h / th).max(y * h / th + 1).min(h));
        for x in 0..tw {
            let (x0, x1) = (x * w / tw, ((x + 1) * w / tw).max(x * w / tw + 1).min(w));
            let (mut sum, mut n) = (0.0f64, 0u32);
            for yy in y0..y1 {
                for &v in &src[yy * w + x0..yy * w + x1] {
                    if v.is_finite() {
                        sum += v as f64;
                        n += 1;
                    }
                }
            }
            out.push(f16::from_f32(if n > 0 { (sum / n as f64) as f32 } else { 0.0 }));
        }
    }
    out
}

impl Planes {
    /// A decoded picture's channels.
    pub fn from_image(img: &image::DynamicImage, linear: bool, max: usize) -> Planes {
        let alpha = img.color().has_alpha();
        let grey = !img.color().has_color();
        let names: Vec<&str> = match (grey, alpha) {
            (true, false) => vec!["Y"],
            (true, true) => vec!["Y", "A"],
            (false, false) => vec!["R", "G", "B"],
            (false, true) => vec!["R", "G", "B", "A"],
        };
        let (w, h) = (img.width() as usize, img.height() as usize);
        let (tw, th) = fit(w, h, names.len(), max);
        let small = if (tw, th) != (w, h) { img.resize_exact(tw as u32, th as u32, image::imageops::FilterType::Triangle) } else { img.clone() };
        let f = small.to_rgba32f();
        let pick = |i: usize| -> Vec<f16> { f.pixels().map(|p| f16::from_f32(p[i])).collect() };
        let channels = names
            .iter()
            .map(|&n| {
                let i = match n {
                    "Y" | "R" => 0,
                    "G" => 1,
                    "B" => 2,
                    _ => 3,
                };
                (n.to_string(), pick(i))
            })
            .collect();
        Planes { layers: vec![Pass { name: String::new(), w: tw, h: th, full: [w as u32, h as u32], channels }], linear: linear || matches!(img.color(), image::ColorType::Rgb32F | image::ColorType::Rgba32F) }
    }

    /// Every layer of an OpenEXR file, the main picture first. Each part's channels are grouped
    /// by the name before their last dot ("diffuse.R" → the "diffuse" layer).
    pub fn read_exr(path: &Path, max: usize) -> Result<Planes, String> {
        use exr::prelude::*;
        let meta = exr::meta::MetaData::read_from_file(path, false).map_err(|e| format!("Couldn't read it: {e}"))?;
        let bytes: usize = meta.headers.iter().map(|h| h.layer_size.area() * h.channels.list.len() * 4).sum();
        if bytes > EXR_BYTES || meta.headers.iter().any(|h| h.deep) {
            return Err("Too large to read every layer.".into());
        }
        let img = read().no_deep_data().largest_resolution_level().all_channels().all_layers().all_attributes().from_file(path).map_err(|e| format!("Couldn't read it: {e}"))?;
        let mut layers: Vec<Pass> = Vec::new();
        let total_channels: usize = img.layer_data.iter().map(|l| l.channel_data.list.len()).sum();
        for part in img.layer_data.iter() {
            let part_name = part.attributes.layer_name.as_ref().map(|t| t.to_string()).unwrap_or_default();
            let (w, h) = (part.size.width(), part.size.height());
            let (tw, th) = fit(w, h, total_channels, max);
            for ch in part.channel_data.list.iter() {
                if ch.sampling != Vec2(1, 1) {
                    continue; // subsampled chroma: rare, and not lined up with the rest
                }
                let full = ch.name.to_string();
                let (group, short) = match full.rsplit_once('.') {
                    Some((g, s)) => (g.to_string(), s.to_string()),
                    None => (String::new(), full.clone()),
                };
                let name = match (part_name.is_empty(), group.is_empty()) {
                    (true, _) => group,
                    (false, true) => part_name.clone(),
                    (false, false) => format!("{part_name}.{group}"),
                };
                let src: Vec<f32> = match &ch.sample_data {
                    FlatSamples::F16(v) => v.iter().map(|x| x.to_f32()).collect(),
                    FlatSamples::F32(v) => v.clone(),
                    FlatSamples::U32(v) => v.iter().map(|&x| x as f32).collect(),
                };
                if src.len() != w * h {
                    continue;
                }
                let px = shrink(&src, w, h, tw, th);
                match layers.iter_mut().find(|l| l.name == name && l.w == tw && l.h == th) {
                    Some(l) => l.channels.push((short, px)),
                    None => layers.push(Pass { name, w: tw, h: th, full: [w as u32, h as u32], channels: vec![(short, px)] }),
                }
            }
        }
        if layers.is_empty() {
            return Err("No layers Filescape can show.".into());
        }
        // Colour channels in the usual order, then the main picture (or the first in colour) first.
        let rank = |n: &str| ["R", "G", "B", "A", "Y"].iter().position(|c| c.eq_ignore_ascii_case(n)).unwrap_or(5);
        for l in &mut layers {
            l.channels.sort_by_key(|c| rank(&c.0));
        }
        let main = |l: &Pass| {
            let n = l.name.to_ascii_lowercase();
            (l.rgb().is_none() as u8, !(n.is_empty() || n == "rgba" || n == "beauty" || n == "combined") as u8)
        };
        let first = (0..layers.len()).min_by_key(|&i| (main(&layers[i]), i)).unwrap_or(0);
        let l = layers.remove(first);
        layers.insert(0, l);
        Ok(Planes { layers, linear: true })
    }

    /// The picture for a view, as RGBA pixels.
    pub fn render(&self, v: &View) -> (usize, usize, Vec<u8>) {
        let l = &self.layers[v.layer.min(self.layers.len() - 1)];
        let gain = 2f32.powf(v.exposure);
        let show = |x: f32| if self.linear { srgb(x * gain) } else { x };
        let n = l.w * l.h;
        let mut out = vec![255u8; n * 4];
        match v.channel.filter(|&c| c < l.channels.len()) {
            Some(c) => {
                let (name, px) = &l.channels[c];
                // Depth and other non-picture channels: spread from nearest to farthest.
                let picture = matches!(name.to_ascii_uppercase().as_str(), "R" | "G" | "B" | "A" | "Y");
                let (lo, hi) = if picture {
                    (0.0, 1.0)
                } else {
                    px.iter().map(|x| x.to_f32()).filter(|x| x.is_finite()).fold((f32::MAX, f32::MIN), |(a, b), x| (a.min(x), b.max(x)))
                };
                let span = (hi - lo).max(1e-6);
                let alpha = name.eq_ignore_ascii_case("A");
                for (i, x) in px.iter().enumerate() {
                    let x = x.to_f32();
                    let g = if !picture { byte((x - lo) / span) } else if alpha { byte(x) } else { byte(show(x)) };
                    out[i * 4..i * 4 + 3].fill(g);
                }
            }
            None => {
                let a = l.find("A");
                match l.rgb() {
                    Some([r, g, b]) => {
                        for i in 0..n {
                            out[i * 4] = byte(show(l.channels[r].1[i].to_f32()));
                            out[i * 4 + 1] = byte(show(l.channels[g].1[i].to_f32()));
                            out[i * 4 + 2] = byte(show(l.channels[b].1[i].to_f32()));
                        }
                    }
                    None => {
                        // No colour: the first channel in grey (Y, Z…).
                        let c = l.find("Y").unwrap_or(0);
                        return self.render(&View { channel: Some(c), ..*v }).pipe_alpha(l, a);
                    }
                }
                if let Some(a) = a {
                    for i in 0..n {
                        out[i * 4 + 3] = byte(l.channels[a].1[i].to_f32());
                    }
                }
            }
        }
        (l.w, l.h, out)
    }

    /// The values under a point of the layer (0..1 across and down), for the readout.
    pub fn sample(&self, v: &View, fx: f32, fy: f32) -> Vec<(String, f32)> {
        let l = &self.layers[v.layer.min(self.layers.len() - 1)];
        let x = ((fx * l.w as f32) as usize).min(l.w - 1);
        let y = ((fy * l.h as f32) as usize).min(l.h - 1);
        l.channels.iter().map(|(n, px)| (n.clone(), px[y * l.w + x].to_f32())).collect()
    }
}

trait PipeAlpha {
    fn pipe_alpha(self, l: &Pass, a: Option<usize>) -> Self;
}

impl PipeAlpha for (usize, usize, Vec<u8>) {
    fn pipe_alpha(mut self, l: &Pass, a: Option<usize>) -> Self {
        if let Some(a) = a {
            for i in 0..l.w * l.h {
                self.2[i * 4 + 3] = byte(l.channels[a].1[i].to_f32());
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_and_exposure() {
        let mut img = image::Rgba32FImage::new(4, 2);
        for (x, _, p) in img.enumerate_pixels_mut() {
            *p = image::Rgba([0.25 * x as f32, 0.5, 2.0, 1.0]);
        }
        let planes = Planes::from_image(&image::DynamicImage::ImageRgba32F(img), true, 100);
        assert!(planes.linear);
        assert_eq!(planes.layers[0].label(), "Main (RGBA)");
        let (w, h, px) = planes.render(&View::default());
        assert_eq!((w, h), (4, 2));
        assert_eq!(px[2], 255); // blue at 2.0 clips
        assert_eq!(px[0], 0);
        let (_, _, dark) = planes.render(&View { exposure: -2.0, ..Default::default() });
        assert_eq!(dark[2], 188); // 2.0 / 4 = 0.5, which is 0.735 on the sRGB curve
        let (_, _, red) = planes.render(&View { channel: Some(0), ..Default::default() });
        assert_eq!(red[3 * 4], red[3 * 4 + 1]); // grey
        assert!(red[3 * 4] > red[4]);
        let s = planes.sample(&View::default(), 0.99, 0.0);
        assert_eq!(s[0], ("R".to_string(), 0.75));
    }

    #[test]
    fn shrinks_by_averaging() {
        let src = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
        let out = shrink(&src, 4, 2, 2, 1);
        assert_eq!(out.iter().map(|v| v.to_f32()).collect::<Vec<_>>(), vec![2.5, 4.5]);
        assert_eq!(fit(10_000, 5_000, 4, 2048), (2048, 1024));
    }

    #[test]
    fn reads_exr_layers() {
        use exr::prelude::*;
        let dir = std::env::temp_dir().join(format!("filescape-exr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("render.exr");
        let size = Vec2(8, 4);
        let flat = |v: f32| FlatSamples::F32(vec![v; 32]);
        let ch = |n: &str, v: f32| AnyChannel::new(n, flat(v));
        let layer = Layer::new(
            size,
            LayerAttributes::default(),
            Encoding::FAST_LOSSLESS,
            AnyChannels::sort(vec![ch("R", 0.5), ch("G", 0.25), ch("B", 0.125), ch("A", 1.0), ch("diffuse.R", 1.0), ch("diffuse.G", 0.0), ch("diffuse.B", 0.0), ch("Z", 10.0)].into()),
        );
        Image::from_layer(layer).write().to_file(&p).unwrap();
        let planes = Planes::read_exr(&p, 2048).unwrap();
        let names: Vec<String> = planes.layers.iter().map(|l| l.label()).collect();
        assert_eq!(names, ["Main (RGBAZ)", "diffuse (RGB)"]);
        let (w, h, px) = planes.render(&View { layer: 1, ..Default::default() });
        assert_eq!((w, h), (8, 4));
        assert_eq!(&px[..4], &[255, 0, 0, 255]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
