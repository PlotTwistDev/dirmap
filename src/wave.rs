//! The shape of a sound, for drawing a waveform on its tile: how loud each stretch is, read
//! through the system's own decoders on a worker thread.
//!
//! - macOS: AVFoundation (see `macaudio.rs`).
//! - Windows: Media Foundation.
//! - Linux and other systems: `ffmpeg`, when it's installed.

use std::path::Path;

/// Samples per second the sound is read at: plenty for a picture of it.
const RATE: u32 = 2000;
/// Only the first this many seconds are read (a long recording's shape is still clear).
const MAX_SECS: f64 = 20.0 * 60.0;

/// Loudness (0..1, the loudest stretch is 1) of `buckets` equal stretches of the sound.
pub fn peaks(path: &Path, buckets: usize) -> Result<Vec<f32>, String> {
    let s = imp::samples(path, RATE, MAX_SECS)?;
    if s.is_empty() || buckets == 0 {
        return Err("No sound".into());
    }
    Ok(to_peaks(&s, buckets))
}

/// Mean loudness of each stretch, scaled so the loudest is 1.
pub fn to_peaks(s: &[i16], buckets: usize) -> Vec<f32> {
    let n = s.len();
    let mut out: Vec<f32> = (0..buckets)
        .map(|b| {
            let (lo, hi) = (b * n / buckets, ((b + 1) * n / buckets).max(b * n / buckets + 1).min(n));
            if lo >= hi {
                return 0.0;
            }
            let sum: f64 = s[lo..hi].iter().map(|&v| (v as f64 / 32768.0).powi(2)).sum();
            (sum / (hi - lo) as f64).sqrt() as f32
        })
        .collect();
    let top = out.iter().cloned().fold(0.0f32, f32::max);
    if top > 0.0 {
        out.iter_mut().for_each(|v| *v /= top);
    }
    out
}

#[cfg(target_os = "macos")]
mod imp {
    pub use crate::macaudio::samples;
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use std::io::Read;
    use std::path::Path;
    use std::process::{Command, Stdio};

    pub fn samples(path: &Path, rate: u32, max_secs: f64) -> Result<Vec<i16>, String> {
        let ffmpeg = std::env::var_os("PATH")
            .and_then(|paths| std::env::split_paths(&paths).map(|d| d.join("ffmpeg")).find(|p| p.is_file()))
            .ok_or("Needs ffmpeg")?;
        let mut child = Command::new(ffmpeg)
            .args(["-nostdin", "-loglevel", "error", "-i"])
            .arg(path)
            .args(["-vn", "-ac", "1", "-ar", &rate.to_string(), "-t", &format!("{max_secs}"), "-f", "s16le", "-"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let mut buf = Vec::new();
        child.stdout.take().ok_or("no output")?.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        let _ = child.wait();
        Ok(buf.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect())
    }
}

#[cfg(windows)]
mod imp {
    use std::path::Path;
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    use windows::core::HSTRING;

    const FIRST_AUDIO: u32 = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;

    pub fn samples(path: &Path, rate: u32, max_secs: f64) -> Result<Vec<i16>, String> {
        let e = |x: windows::core::Error| x.message().to_string();
        // SAFETY: Media Foundation and COM calls with valid arguments; every result is checked,
        // and each buffer is read only within the length it reports, while locked.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(e)?;
            let url = HSTRING::from(path.as_os_str());
            let reader = MFCreateSourceReaderFromURL(&url, None).map_err(e)?;
            let _ = reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false);
            reader.SetStreamSelection(FIRST_AUDIO, true).map_err(|_| "No sound in this file".to_string())?;
            let mt = MFCreateMediaType().map_err(e)?;
            mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).map_err(e)?;
            mt.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM).map_err(e)?;
            mt.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16).map_err(e)?;
            reader.SetCurrentMediaType(FIRST_AUDIO, None, &mt).map_err(e)?;
            let cur = reader.GetCurrentMediaType(FIRST_AUDIO).map_err(e)?;
            let ch = cur.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS).unwrap_or(2).max(1) as usize;
            let sr = cur.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).unwrap_or(44_100).max(1);
            // Each output sample is the loudest of this many frames.
            let step = (sr / rate).max(1) as usize;
            let cap = (rate as f64 * max_secs) as usize;
            let mut out = Vec::new();
            let (mut peak, mut k) = (0i16, 0usize);
            while out.len() < cap {
                let mut flags = 0u32;
                let mut sample: Option<IMFSample> = None;
                reader.ReadSample(FIRST_AUDIO, 0, None, Some(&mut flags), None, Some(&mut sample)).map_err(e)?;
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    break;
                }
                let Some(sample) = sample else { continue };
                let buf = sample.ConvertToContiguousBuffer().map_err(e)?;
                let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
                buf.Lock(&mut p, None, Some(&mut len)).map_err(e)?;
                let data = std::slice::from_raw_parts(p as *const i16, len as usize / 2);
                for frame in data.chunks_exact(ch) {
                    let v = frame.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0).min(i16::MAX as u16) as i16;
                    peak = peak.max(v);
                    k += 1;
                    if k == step {
                        out.push(peak);
                        peak = 0;
                        k = 0;
                    }
                }
                let _ = buf.Unlock();
            }
            out.truncate(cap);
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_follow_loudness() {
        let mut s = vec![0i16; 4000];
        for (i, v) in s.iter_mut().enumerate().skip(2000) {
            *v = if i % 2 == 0 { 16000 } else { -16000 };
        }
        let p = to_peaks(&s, 4);
        assert_eq!(p.len(), 4);
        assert!(p[0] < 0.01 && p[1] < 0.01);
        assert!((p[2] - 1.0).abs() < 1e-6 && (p[3] - 1.0).abs() < 1e-6);
    }
}
