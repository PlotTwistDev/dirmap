//! JPEG frames the usual decoder (the `image` crate) can't read: 12-bit samples (medical,
//! scientific and some camera files) and arithmetic coding. Decoded with `craft-jpeg` from
//! craft-libs (MIT or Apache-2.0), a clean-room decoder written from the JPEG standard.

/// Pixels decoded at most (a 16,384 × 16,384 picture).
const MAX_PIXELS: u64 = 16_384 * 16_384;
/// Bytes of decoded samples at most.
const MAX_ALLOC: u64 = 1 << 30;

/// Whether `craft-jpeg` should read this JPEG: arithmetic-coded frames, and Huffman-coded ones
/// that aren't 8-bit. Only looks at the markers before the first frame.
pub fn needs(bytes: &[u8]) -> bool {
    match craft_jpeg::frame(bytes) {
        Some(f) if (0xC9..=0xCB).contains(&f.marker) => true,
        Some(f) if matches!(f.marker, 0xC0..=0xC2) => f.precision != 8,
        _ => false,
    }
}

/// The frame's sample precision, in bits.
pub fn precision(bytes: &[u8]) -> Option<u8> {
    craft_jpeg::frame(bytes).map(|f| f.precision)
}

fn limits() -> craft_jpeg::Limits {
    let mut l = craft_jpeg::Limits::default();
    l.max_pixels = MAX_PIXELS;
    l.max_alloc = MAX_ALLOC;
    l
}

/// The picture, 16 bits a channel for a 12-bit file.
pub fn decode(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    let d = craft_jpeg::decode(bytes, &limits(), craft_jpeg::adobe_transform(bytes)).map_err(|e| format!("Couldn't read the JPEG: {e}"))?;
    let (w, h) = (d.width, d.height);
    let n = w as usize * h as usize;
    let too_short = || "Couldn't read the JPEG: it's cut short".to_string();
    match d.samples {
        craft_jpeg::Samples::U8(s) => {
            let rgba = craft_jpeg::to_rgba8(&s, d.components).map_err(|_| format!("JPEGs with {} colour channels aren't supported", d.components))?;
            image::RgbaImage::from_raw(w, h, rgba).map(image::DynamicImage::ImageRgba8).ok_or_else(too_short)
        }
        craft_jpeg::Samples::U16(s) => {
            let c = d.components as usize;
            if s.len() < n * c {
                return Err(too_short());
            }
            let mut out = Vec::with_capacity(n * 4);
            for px in s.chunks_exact(c).take(n) {
                let [r, g, b] = match c {
                    1 => [px[0]; 3],
                    3 => [px[0], px[1], px[2]],
                    // Ink amounts (0 = no ink), as craft-jpeg gives them.
                    4 => {
                        let k = 1.0 - px[3] as f32 / 65535.0;
                        let ch = |v: u16| ((1.0 - v as f32 / 65535.0) * k * 65535.0).round() as u16;
                        [ch(px[0]), ch(px[1]), ch(px[2])]
                    }
                    other => return Err(format!("JPEGs with {other} colour channels aren't supported")),
                };
                out.extend_from_slice(&[r, g, b, u16::MAX]);
            }
            image::ImageBuffer::<image::Rgba<u16>, _>::from_raw(w, h, out).map(image::DynamicImage::ImageRgba16).ok_or_else(too_short)
        }
    }
}

/// The samples of one grey or RGB frame as they're stored (0 to 4095 for a 12-bit file), for
/// DICOM, whose pixel values mean something: width, height, channels and the samples.
pub fn samples(bytes: &[u8]) -> Result<(u32, u32, usize, Vec<f64>), String> {
    let bits = precision(bytes).unwrap_or(8);
    let d = craft_jpeg::decode(bytes, &limits(), None).map_err(|e| format!("Couldn't decode its JPEG picture: {e}"))?;
    let c = d.components as usize;
    if c != 1 && c != 3 {
        return Err(format!("Its JPEG picture has {c} colour channels, which isn't supported"));
    }
    let v: Vec<f64> = match d.samples {
        craft_jpeg::Samples::U8(s) => s.into_iter().map(f64::from).collect(),
        // Scaled up to 16 bits by the decoder: back to the stored range.
        craft_jpeg::Samples::U16(s) => {
            let top = f64::from((1u32 << bits.clamp(9, 16)) - 1);
            s.into_iter().map(|x| (f64::from(x) * top / 65535.0).round()).collect()
        }
    };
    Ok((d.width, d.height, c, v))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// An 8 × 8 grey 12-bit sequential JPEG (SOF1), from craft-libs' craft-jpeg fuzz corpus
    /// (`fuzz/corpus/decode/sof1-12bit.jpg`, MIT or Apache-2.0).
    pub(crate) fn twelve_bit() -> Vec<u8> {
        hex("ffd8ffdb00430001010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101ffc1000b0c0008000801011100ffc4002600010000000000000000000000000000000d100100000000000000000000000000000000ffda0008010100003f007701ffd9")
    }

    /// A 16 × 16 grey 12-bit arithmetic-coded JPEG (SOF9), from the same corpus (`sof9-12bit.jpg`).
    fn arithmetic() -> Vec<u8> {
        hex("ffd8ffdb00430001010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101010101ffc9000b0c0010001001011100ffda0008010100003f00ff00fd84568c94a3cbe56350a1926660981fbb17114ecacfd4c1a0fbbb582d9d383589609649298154d16d6c5c73bbea25ecdb71d26344cfe4429916e2281460c8c6b6bfe6adec74a03f9042572aa4f4fc52818ec543bd2085b06d163d3874a2f112fa53665b5581f90f3b2293a0ffd9")
    }

    #[test]
    fn twelve_bit_and_arithmetic_frames() {
        let a = twelve_bit();
        assert!(needs(&a));
        assert_eq!(precision(&a), Some(12));
        // The usual decoder can't read it; this one can.
        assert!(image::load_from_memory(&a).is_err());
        let img = decode(&a).unwrap();
        assert_eq!((img.width(), img.height()), (8, 8));
        assert!(matches!(img, image::DynamicImage::ImageRgba16(_)));
        let (w, h, c, v) = samples(&a).unwrap();
        assert_eq!((w, h, c, v.len()), (8, 8, 1, 64));
        assert!(v.iter().all(|x| (0.0..=4095.0).contains(x)));
        let b = arithmetic();
        assert!(needs(&b));
        assert_eq!(decode(&b).unwrap().width(), 16);
    }

    #[test]
    fn ordinary_and_damaged_files() {
        let mut ok = Vec::new();
        image::DynamicImage::ImageRgb8(image::RgbImage::new(4, 4)).write_to(&mut std::io::Cursor::new(&mut ok), image::ImageFormat::Jpeg).unwrap();
        assert!(!needs(&ok));
        assert!(!needs(b"not a jpeg"));
        let mut cut = twelve_bit();
        cut.truncate(100);
        let _ = decode(&cut); // an error or a partial picture, never a panic
        assert!(decode(b"\xff\xd8\xff\xd9").is_err());
    }
}
