//! What a sound file says about itself: its tags (title, artist, album…), its format (length,
//! sample rate, bit depth, channels, bit rate) and its cover picture, read with lofty (pure
//! Rust). Runs on worker threads.

use lofty::prelude::*;
use std::path::Path;

/// Tags and format, for the details, and the cover as RGBA.
#[derive(Default)]
pub struct SoundInfo {
    /// (label, value) rows: Title, Artist, Album, Year, Length, Format.
    pub facts: Vec<(&'static str, String)>,
    pub cover: Option<(u32, u32, Vec<u8>)>,
    /// Length in seconds, when the file says.
    pub secs: Option<f64>,
}

/// Covers are decoded no larger than this across.
const COVER_MAX: u32 = 512;

pub fn read(path: &Path) -> SoundInfo {
    let Ok(file) = lofty::read_from_path(path) else { return SoundInfo::default() };
    let mut facts = Vec::new();
    let tag = file.primary_tag().or_else(|| file.first_tag());
    if let Some(t) = tag {
        let mut add = |label: &'static str, v: Option<String>| {
            if let Some(v) = v.map(|v| v.trim().to_string()).filter(|v| !v.is_empty()) {
                facts.push((label, v));
            }
        };
        add("Title", t.title().map(|s| s.into_owned()));
        add("Artist", t.artist().map(|s| s.into_owned()));
        add("Album", t.album().map(|s| s.into_owned()));
        add("Year", t.date().map(|d| d.year.to_string()));
        add("Genre", t.genre().map(|s| s.into_owned()));
    }
    let p = file.properties();
    let secs = p.duration().as_secs_f64();
    if secs > 0.0 {
        facts.push(("Length", crate::media::clock(secs)));
    }
    let mut fmt = Vec::new();
    if let Some(r) = p.sample_rate() {
        fmt.push(if r % 1000 == 0 { format!("{} kHz", r / 1000) } else { format!("{:.1} kHz", r as f64 / 1000.0) });
    }
    if let Some(b) = p.bit_depth() {
        fmt.push(format!("{b}-bit"));
    }
    match p.channels() {
        Some(1) => fmt.push("mono".into()),
        Some(2) => fmt.push("stereo".into()),
        Some(n) if n > 2 => fmt.push(format!("{n} channels")),
        _ => {}
    }
    if let Some(k) = p.audio_bitrate().filter(|k| *k > 0) {
        fmt.push(format!("{k} kbps"));
    }
    if !fmt.is_empty() {
        facts.push(("Format", fmt.join(" · ")));
    }
    let cover = tag.and_then(|t| {
        // The front cover if it's marked, else the first picture.
        let pics = t.pictures();
        let pic = pics.iter().find(|p| p.pic_type() == lofty::picture::PictureType::CoverFront).or(pics.first())?;
        let img = image::load_from_memory(pic.data()).ok()?;
        let img = if img.width() > COVER_MAX || img.height() > COVER_MAX { img.thumbnail(COVER_MAX, COVER_MAX) } else { img };
        let rgba = img.to_rgba8();
        Some((rgba.width(), rgba.height(), rgba.into_raw()))
    });
    SoundInfo { facts, cover, secs: (secs > 0.0).then_some(secs) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_wav() {
        // One second of 16-bit mono silence at 8 kHz.
        let rate = 8000u32;
        let data = vec![0u8; rate as usize * 2];
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        let p = std::env::temp_dir().join(format!("fs-sound-{}.wav", std::process::id()));
        std::fs::write(&p, &wav).unwrap();
        let info = read(&p);
        let _ = std::fs::remove_file(&p);
        let get = |k| info.facts.iter().find(|f| f.0 == k).map(|f| f.1.clone());
        assert_eq!(get("Length").as_deref(), Some("0:01"));
        assert_eq!(get("Format").as_deref(), Some("8 kHz · 16-bit · mono · 128 kbps"));
        assert!(info.cover.is_none());
        assert_eq!(info.secs, Some(1.0));
        assert!(read(Path::new("/no/such/file.mp3")).facts.is_empty());
    }
}
