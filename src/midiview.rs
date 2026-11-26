//! MIDI files as a piano roll: every note drawn against time and pitch, coloured by channel, with
//! the length, tempo, tracks and instruments for the details. Read with midly (Unlicense).

use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};
use std::path::Path;

/// MIDI files larger than this aren't read (real ones are rarely over a megabyte).
const MAX_FILE: u64 = 16 << 20;
/// Notes kept from a huge file.
const MAX_NOTES: usize = 200_000;

pub struct Note {
    pub start: f64,
    pub end: f64,
    pub key: u8,
    pub channel: u8,
    pub velocity: u8,
}

pub struct Song {
    pub notes: Vec<Note>,
    pub secs: f64,
    pub facts: Vec<(&'static str, String)>,
}

/// General MIDI instrument families, by program number / 8.
const FAMILIES: [&str; 16] = [
    "Piano",
    "Chromatic percussion",
    "Organ",
    "Guitar",
    "Bass",
    "Strings",
    "Ensemble",
    "Brass",
    "Reed",
    "Pipe",
    "Synth lead",
    "Synth pad",
    "Synth effects",
    "Ethnic",
    "Percussive",
    "Sound effects",
];

/// Tick → seconds, through the tempo changes.
struct Clock {
    /// (tick, seconds at that tick, seconds per tick from there)
    marks: Vec<(u64, f64, f64)>,
}

impl Clock {
    fn new(timing: Timing, mut tempos: Vec<(u64, u32)>) -> Clock {
        match timing {
            Timing::Timecode(fps, sub) => {
                let per = 1.0 / (fps.as_f32() as f64 * sub as f64).max(1.0);
                Clock { marks: vec![(0, 0.0, per)] }
            }
            Timing::Metrical(tpb) => {
                let tpb = tpb.as_int().max(1) as f64;
                tempos.sort_by_key(|t| t.0);
                let mut marks = vec![(0u64, 0.0f64, 500_000.0 / 1e6 / tpb)];
                for (tick, us) in tempos {
                    let (t0, s0, per) = *marks.last().unwrap();
                    let s = s0 + (tick - t0) as f64 * per;
                    let per = us.max(1) as f64 / 1e6 / tpb;
                    if tick == t0 {
                        *marks.last_mut().unwrap() = (tick, s0, per);
                    } else {
                        marks.push((tick, s, per));
                    }
                }
                Clock { marks }
            }
        }
    }

    fn secs(&self, tick: u64) -> f64 {
        let i = self.marks.partition_point(|m| m.0 <= tick).saturating_sub(1);
        let (t0, s0, per) = self.marks[i];
        s0 + (tick.saturating_sub(t0)) as f64 * per
    }
}

pub fn read(path: &Path) -> Result<Song, String> {
    let size = std::fs::metadata(path).map_err(|e| format!("Couldn't read it: {e}"))?.len();
    if size > MAX_FILE {
        return Err("Too big to preview.".into());
    }
    let bytes = std::fs::read(path).map_err(|e| format!("Couldn't read it: {e}"))?;
    parse(&bytes)
}

fn parse(bytes: &[u8]) -> Result<Song, String> {
    let smf = Smf::parse(bytes).map_err(|_| "Not a MIDI file Filescape can read.".to_string())?;
    // First pass: tempo changes, from every track (format 1 keeps them in the first).
    let mut tempos = Vec::new();
    let mut name = None;
    let mut bpm = None;
    let mut timesig = None;
    for track in &smf.tracks {
        let mut tick = 0u64;
        for ev in track {
            tick += ev.delta.as_int() as u64;
            if let TrackEventKind::Meta(m) = ev.kind {
                match m {
                    MetaMessage::Tempo(us) => {
                        tempos.push((tick, us.as_int()));
                        bpm.get_or_insert(60_000_000.0 / us.as_int().max(1) as f64);
                    }
                    MetaMessage::TrackName(n) if name.is_none() => {
                        let n = String::from_utf8_lossy(n).trim().to_string();
                        if !n.is_empty() {
                            name = Some(n);
                        }
                    }
                    MetaMessage::TimeSignature(num, den, ..) if timesig.is_none() => timesig = Some(format!("{num}/{}", 1u32 << den.min(6))),
                    _ => {}
                }
            }
        }
    }
    let clock = Clock::new(smf.header.timing, tempos);
    let mut notes = Vec::new();
    let mut programs: [Option<u8>; 16] = [None; 16];
    for track in &smf.tracks {
        let mut tick = 0u64;
        // (channel, key) -> (start tick, velocity)
        let mut on: std::collections::HashMap<(u8, u8), (u64, u8)> = Default::default();
        for ev in track {
            tick += ev.delta.as_int() as u64;
            let TrackEventKind::Midi { channel, message } = ev.kind else { continue };
            let ch = channel.as_int();
            let end = |key: u8, on: &mut std::collections::HashMap<(u8, u8), (u64, u8)>, notes: &mut Vec<Note>| {
                if let Some((start, velocity)) = on.remove(&(ch, key)) {
                    notes.push(Note { start: clock.secs(start), end: clock.secs(tick), key, channel: ch, velocity });
                }
            };
            match message {
                MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                    let key = key.as_int();
                    // A note struck again before it was released ends the first.
                    end(key, &mut on, &mut notes);
                    on.insert((ch, key), (tick, vel.as_int()));
                }
                MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => end(key.as_int(), &mut on, &mut notes),
                MidiMessage::ProgramChange { program } => {
                    programs[ch as usize].get_or_insert(program.as_int());
                }
                _ => {}
            }
            if notes.len() >= MAX_NOTES {
                break;
            }
        }
        // Notes never released end with the track.
        let ends: Vec<(u8, u8)> = on.keys().copied().collect();
        for (ch, key) in ends {
            let (start, velocity) = on.remove(&(ch, key)).unwrap();
            notes.push(Note { start: clock.secs(start), end: clock.secs(tick), key, channel: ch, velocity });
        }
    }
    notes.sort_by(|a, b| a.start.total_cmp(&b.start));
    let secs = notes.iter().map(|n| n.end).fold(0.0, f64::max);

    let mut facts = Vec::new();
    if let Some(n) = name {
        facts.push(("Title", n));
    }
    let m = (secs / 60.0) as u64;
    facts.push(("Length", format!("{m}:{:02}", (secs - m as f64 * 60.0).round() as u64 % 60)));
    let used: Vec<u8> = {
        let mut c: Vec<u8> = notes.iter().map(|n| n.channel).collect();
        c.sort_unstable();
        c.dedup();
        c
    };
    facts.push(("Notes", format!("{} on {} channel{}", notes.len(), used.len(), if used.len() == 1 { "" } else { "s" })));
    facts.push(("Tracks", smf.tracks.len().to_string()));
    if let Some(b) = bpm {
        facts.push(("Tempo", format!("{b:.0} bpm{}", timesig.map(|t| format!(", {t}")).unwrap_or_default())));
    }
    let mut kinds: Vec<&str> = used.iter().map(|&c| if c == 9 { "Drums" } else { FAMILIES[(programs[c as usize].unwrap_or(0) / 8) as usize] }).collect();
    kinds.dedup();
    let mut seen = Vec::new();
    kinds.retain(|k| if seen.contains(k) { false } else { seen.push(*k); true });
    if !kinds.is_empty() {
        facts.push(("Instruments", kinds.join(", ")));
    }
    Ok(Song { notes, secs, facts })
}

/// A colour for each channel.
fn colour(ch: u8) -> [u8; 3] {
    const C: [[u8; 3]; 8] = [[86, 156, 255], [255, 140, 66], [102, 204, 120], [236, 94, 120], [180, 130, 255], [255, 205, 60], [70, 200, 210], [240, 120, 200]];
    if ch == 9 { [200, 200, 200] } else { C[ch as usize % 8] }
}

impl Song {
    /// The piano roll, `w` × `h` pixels, RGBA.
    pub fn roll(&self, w: u32, h: u32) -> Vec<u8> {
        let (w, h) = (w.max(16) as usize, h.max(16) as usize);
        let mut px = vec![0u8; w * h * 4];
        let bg = [28u8, 30, 36];
        for p in px.chunks_exact_mut(4) {
            p.copy_from_slice(&[bg[0], bg[1], bg[2], 255]);
        }
        let lo = self.notes.iter().map(|n| n.key).min().unwrap_or(48).saturating_sub(2);
        let hi = self.notes.iter().map(|n| n.key).max().unwrap_or(72).saturating_add(2).min(127);
        let span = (hi - lo).max(12) as f64 + 1.0;
        let row = h as f64 / span;
        let y_of = |k: u8| ((hi as f64 - k as f64) * row) as usize;
        // Black keys a shade darker, C lines brighter: a keyboard to read pitch by.
        for k in lo..=hi {
            let y0 = y_of(k).min(h - 1);
            let y1 = ((y0 as f64 + row).ceil() as usize).min(h);
            let shade: i16 = if matches!(k % 12, 1 | 3 | 6 | 8 | 10) { -6 } else { 0 };
            for y in y0..y1 {
                for x in 0..w {
                    let p = &mut px[(y * w + x) * 4..][..3];
                    for (i, v) in p.iter_mut().enumerate() {
                        *v = (bg[i] as i16 + shade).clamp(0, 255) as u8;
                    }
                }
            }
            if k % 12 == 0 && y1 < h {
                for x in 0..w {
                    px[((y1 - 1) * w + x) * 4..][..3].copy_from_slice(&[50, 54, 64]);
                }
            }
        }
        let total = self.secs.max(0.001);
        for n in &self.notes {
            let x0 = (n.start / total * w as f64) as usize;
            let x1 = ((n.end / total * w as f64).ceil() as usize).max(x0 + 1).min(w);
            let y0 = y_of(n.key).min(h - 1);
            let y1 = ((y0 as f64 + row).ceil() as usize).max(y0 + 1).min(h);
            let c = colour(n.channel);
            let a = 0.45 + 0.55 * (n.velocity as f64 / 127.0);
            for y in y0..y1 {
                for x in x0..x1 {
                    let p = &mut px[(y * w + x) * 4..][..3];
                    for i in 0..3 {
                        p[i] = (p[i] as f64 * (1.0 - a) + c[i] as f64 * a) as u8;
                    }
                }
            }
        }
        px
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A format-0 file: 96 ticks a beat, 120 bpm, C4 for a beat then E4 for two.
    fn song() -> Vec<u8> {
        let track: Vec<u8> = vec![
            0x00, 0xFF, 0x03, 0x04, b'T', b'u', b'n', b'e', // name
            0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, // tempo 500000 us
            0x00, 0xC0, 0x00, // piano
            0x00, 0x90, 60, 100, // C4 on
            0x60, 0x80, 60, 0, // off after 96
            0x00, 0x90, 64, 80, // E4 on
            0x81, 0x40, 0x90, 64, 0, // off after 192 (note on, velocity 0)
            0x00, 0xFF, 0x2F, 0x00,
        ];
        let mut out = b"MThd\0\0\0\x06\0\0\0\x01\0\x60MTrk".to_vec();
        out.extend((track.len() as u32).to_be_bytes());
        out.extend(track);
        out
    }

    #[test]
    fn reads_notes_and_draws_them() {
        let s = parse(&song()).unwrap();
        assert_eq!(s.notes.len(), 2);
        assert_eq!((s.notes[0].key, s.notes[0].start, s.notes[0].end), (60, 0.0, 0.5));
        assert_eq!((s.notes[1].key, s.notes[1].start, s.notes[1].end), (64, 0.5, 1.5));
        assert!((s.secs - 1.5).abs() < 1e-9);
        assert!(s.facts.contains(&("Title", "Tune".into())));
        assert!(s.facts.contains(&("Tempo", "120 bpm".into())));
        assert!(s.facts.contains(&("Instruments", "Piano".into())));
        let px = s.roll(150, 100);
        assert_eq!(px.len(), 150 * 100 * 4);
        // The first third has the low note, the rest the high one: some pixel in each is blue.
        let blue = |x: usize| (0..100).any(|y| px[(y * 150 + x) * 4 + 2] > 150);
        assert!(blue(10) && blue(120));
        assert!(parse(b"not midi").is_err());
    }
}
