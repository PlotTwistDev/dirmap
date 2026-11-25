//! Pictures and sound for files dirmap can't decode itself: a poster frame of a video and the
//! first page of a PDF come from the system's own thumbnailer, and sound plays through the
//! system's player.
//!
//! - Windows: the shell's thumbnail handlers (the ones Explorer uses) through
//!   IShellItemImageFactory, and MCI for sound.
//! - macOS: Quick Look (`qlmanage -t`) for pictures, AVAudioPlayer for sound (`macaudio.rs`).
//! - Linux and other systems: `pdftoppm`, `ffmpegthumbnailer` or `ffmpeg` for pictures and
//!   `ffplay`, `gst-play-1.0`, `paplay` or `aplay` for sound, whichever is installed.
//!
//! Everything here runs off the UI thread: the thumbnailers on the preview and thumbnail
//! workers, the player on a thread of its own. Commands are run directly (never through a
//! shell) with the file's path as one argument.

use std::path::Path;
#[cfg(not(windows))]
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

/// Kinds of file that need the system for a picture or sound.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Media {
    Video,
    Audio,
    Pdf,
}

pub fn media_of(name: &str) -> Option<Media> {
    use crate::formats::Category as C;
    Some(match crate::formats::by_name(name)?.category {
        C::Video => Media::Video,
        C::Audio => Media::Audio,
        C::Pdf => Media::Pdf,
        _ => return None,
    })
}

/// RGBA pixels and their size.
pub type Picture = (usize, usize, Vec<u8>);

/// A picture of the file from the system (a video's poster frame, a PDF's first page, an
/// album cover), no larger than `max` across.
pub fn thumbnail(path: &Path, max: u32) -> Result<Picture, String> {
    imp::thumbnail(path, max)
}

/// Play a video in the system's quick viewer: Quick Look on macOS (a floating player that
/// closes with Space or Escape), the default video app elsewhere.
pub fn play_video(path: &Path) -> Result<(), String> {
    if cfg!(target_os = "macos") {
        std::process::Command::new("/usr/bin/qlmanage")
            .arg("-p")
            .arg(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    } else {
        crate::actions::open(path)
    }
}

// ---- the player ----

/// How the player is doing, for the UI.
#[derive(Clone, Default, Debug)]
pub struct PlayState {
    pub playing: bool,
    /// Seconds into the sound.
    pub position: f64,
    /// Seconds long, when the player can tell.
    pub length: Option<f64>,
    pub error: Option<String>,
    pub ended: bool,
}

pub(crate) enum Cmd {
    Play,
    Pause,
    /// Seconds from the start (where the player can).
    Seek(f64),
}

/// A sound playing on a thread of its own. Dropping it stops the sound.
pub struct Player {
    tx: mpsc::Sender<Cmd>,
    state: Arc<Mutex<PlayState>>,
    pub path: std::path::PathBuf,
}

impl Player {
    /// Start playing a file.
    pub fn start(path: &Path) -> Player {
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(PlayState::default()));
        let (p, s) = (path.to_path_buf(), state.clone());
        let _ = std::thread::Builder::new().name("dirmap-play".into()).spawn(move || imp::play(&p, &rx, &s));
        let _ = tx.send(Cmd::Play);
        Player { tx, state, path: path.to_path_buf() }
    }

    pub fn state(&self) -> PlayState {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn play(&self) {
        let _ = self.tx.send(Cmd::Play);
    }

    pub fn pause(&self) {
        let _ = self.tx.send(Cmd::Pause);
    }

    pub fn seek(&self, secs: f64) {
        let _ = self.tx.send(Cmd::Seek(secs));
    }

    /// Whether this player can jump to a time.
    pub fn can_seek(&self) -> bool {
        cfg!(windows) || self.state().length.is_some()
    }
}

/// Seconds as "m:ss".
pub fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

fn set(state: &Mutex<PlayState>, f: impl FnOnce(&mut PlayState)) {
    if let Ok(mut s) = state.lock() {
        f(&mut s);
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::SIZE;
    use windows_sys::Win32::Graphics::Gdi::{BITMAP, DeleteObject, GetObjectW, HBITMAP};
    use windows_sys::Win32::Media::Multimedia::mciSendStringW;
    use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
    use windows_sys::Win32::UI::Shell::SHCreateItemFromParsingName;
    use windows_sys::core::GUID;

    const IID_ISHELLITEMIMAGEFACTORY: GUID = GUID::from_u128(0xbcc18b79_ba16_442f_80c4_8a59c30c463b);
    /// SIIGBF_THUMBNAILONLY: a real thumbnail or nothing (dirmap draws its own type icons).
    const SIIGBF_THUMBNAILONLY: i32 = 0x8;

    #[repr(C)]
    struct FactoryVtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        get_image: unsafe extern "system" fn(*mut c_void, SIZE, i32, *mut HBITMAP) -> i32,
    }

    pub fn thumbnail(path: &Path, max: u32) -> Result<Picture, String> {
        shell_thumbnail(path, max).or_else(|e| {
            // Windows has no PDF thumbnail handler of its own (Acrobat and others add one), but
            // it can draw PDF pages itself.
            if media_of(&path.to_string_lossy()) == Some(Media::Pdf) { pdf_page(path, max) } else { Err(e) }
        })
    }

    /// The first page of a PDF, drawn by Windows.Data.Pdf, no larger than `max` across.
    fn pdf_page(path: &Path, max: u32) -> Result<Picture, String> {
        use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
        use windows::Storage::StorageFile;
        use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};
        let fail = |e: windows::core::Error| format!("Couldn't draw the PDF: {}", e.message());
        let file = StorageFile::GetFileFromPathAsync(&windows::core::HSTRING::from(path.as_os_str())).and_then(|op| op.join()).map_err(fail)?;
        let doc = PdfDocument::LoadFromFileAsync(&file).and_then(|op| op.join()).map_err(fail)?;
        let page = doc.GetPage(0).map_err(fail)?;
        let size = page.Size().map_err(fail)?;
        let opts = PdfPageRenderOptions::new().map_err(fail)?;
        // The longer side gets `max` pixels; PNG is the default encoding.
        if size.Height > size.Width {
            opts.SetDestinationHeight(max).map_err(fail)?;
        } else {
            opts.SetDestinationWidth(max).map_err(fail)?;
        }
        let stream = InMemoryRandomAccessStream::new().map_err(fail)?;
        page.RenderWithOptionsToStreamAsync(&stream, &opts).and_then(|op| op.join()).map_err(fail)?;
        let len = stream.Size().map_err(fail)? as u32;
        let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0).map_err(fail)?).map_err(fail)?;
        reader.LoadAsync(len).and_then(|op| op.join()).map_err(fail)?;
        let mut png = vec![0u8; len as usize];
        reader.ReadBytes(&mut png).map_err(fail)?;
        let img = image::load_from_memory_with_format(&png, image::ImageFormat::Png).map_err(|e| format!("Couldn't draw the PDF: {e}"))?.to_rgba8();
        Ok((img.width() as usize, img.height() as usize, img.into_raw()))
    }

    fn shell_thumbnail(path: &Path, max: u32) -> Result<Picture, String> {
        let wide: Vec<u16> = path.as_os_str().encode_wide_z();
        unsafe {
            // Each worker thread joins COM once; repeated calls just return S_FALSE.
            CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as _);
            let mut obj: *mut c_void = std::ptr::null_mut();
            let hr = SHCreateItemFromParsingName(wide.as_ptr(), std::ptr::null_mut(), &IID_ISHELLITEMIMAGEFACTORY, &mut obj);
            if hr < 0 || obj.is_null() {
                return Err("No thumbnail".into());
            }
            let vtbl = *(obj as *mut *const FactoryVtbl);
            let mut hbm: HBITMAP = std::ptr::null_mut();
            let size = SIZE { cx: max as i32, cy: max as i32 };
            let hr = ((*vtbl).get_image)(obj, size, SIIGBF_THUMBNAILONLY, &mut hbm);
            ((*vtbl).release)(obj);
            if hr < 0 || hbm.is_null() {
                return Err("No thumbnail".into());
            }
            let out = (|| {
                let mut bm: BITMAP = std::mem::zeroed();
                if GetObjectW(hbm, std::mem::size_of::<BITMAP>() as i32, &mut bm as *mut _ as *mut _) == 0 {
                    return None;
                }
                let (w, h) = (bm.bmWidth as usize, bm.bmHeight.unsigned_abs() as usize);
                let bgra = crate::icons::win_dib(hbm, w, h)?;
                // Thumbnails without alpha come back with it all zero: opaque.
                let opaque = bgra.chunks(4).all(|p| p[3] == 0);
                let mut rgba = Vec::with_capacity(w * h * 4);
                for p in bgra.chunks(4) {
                    rgba.extend_from_slice(&[p[2], p[1], p[0], if opaque { 255 } else { p[3] }]);
                }
                Some((w, h, rgba))
            })();
            DeleteObject(hbm as _);
            out.ok_or_else(|| "No thumbnail".to_string())
        }
    }

    trait WideZ {
        fn encode_wide_z(&self) -> Vec<u16>;
    }
    impl WideZ for std::ffi::OsStr {
        fn encode_wide_z(&self) -> Vec<u16> {
            use std::os::windows::ffi::OsStrExt;
            self.encode_wide().chain(std::iter::once(0)).collect()
        }
    }

    /// An MCI command; the reply text.
    fn mci(cmd: &str) -> Result<String, String> {
        let wide: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
        let mut buf = [0u16; 256];
        let err = unsafe { mciSendStringW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32, std::ptr::null_mut()) };
        if err != 0 {
            return Err(format!("This sound couldn't be played (MCI error {err})."));
        }
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Ok(String::from_utf16_lossy(&buf[..n]))
    }

    pub fn play(path: &Path, rx: &mpsc::Receiver<Cmd>, state: &Mutex<PlayState>) {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let alias = format!("dirmap{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let p = path.display().to_string().replace('"', "");
        if let Err(e) = mci(&format!("open \"{p}\" type mpegvideo alias {alias}")) {
            set(state, |s| s.error = Some(e));
            return;
        }
        let _ = mci(&format!("set {alias} time format milliseconds"));
        let length = mci(&format!("status {alias} length")).ok().and_then(|t| t.trim().parse::<f64>().ok()).map(|ms| ms / 1000.0);
        set(state, |s| s.length = length);
        loop {
            match rx.recv_timeout(Duration::from_millis(150)) {
                Ok(Cmd::Play) => {
                    let ended = state.lock().map(|s| s.ended).unwrap_or(false);
                    let r = if ended { mci(&format!("play {alias} from 0")) } else { mci(&format!("play {alias}")) };
                    set(state, |s| {
                        s.ended = false;
                        if let Err(e) = r {
                            s.error = Some(e);
                        }
                    });
                }
                Ok(Cmd::Pause) => {
                    let _ = mci(&format!("pause {alias}"));
                }
                Ok(Cmd::Seek(t)) => {
                    let playing = state.lock().map(|s| s.playing).unwrap_or(false);
                    let ms = (t * 1000.0) as u64;
                    let _ = if playing { mci(&format!("play {alias} from {ms}")) } else { mci(&format!("seek {alias} to {ms}")) };
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            let mode = mci(&format!("status {alias} mode")).unwrap_or_default();
            let pos = mci(&format!("status {alias} position")).ok().and_then(|t| t.trim().parse::<f64>().ok()).map(|ms| ms / 1000.0);
            set(state, |s| {
                s.playing = mode.trim() == "playing";
                if let Some(p) = pos {
                    s.position = p;
                }
                if mode.trim() == "stopped" && s.length.is_some_and(|l| s.position >= l - 0.3) {
                    s.ended = true;
                }
            });
        }
        let _ = mci(&format!("close {alias}"));
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    use std::process::{Child, Command, Stdio};
    use std::time::Instant;

    /// A program on the PATH.
    fn find(prog: &str) -> Option<PathBuf> {
        if cfg!(target_os = "macos") {
            for d in ["/usr/bin", "/opt/homebrew/bin", "/usr/local/bin"] {
                let p = Path::new(d).join(prog);
                if p.is_file() {
                    return Some(p);
                }
            }
        }
        std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|d| d.join(prog)).find(|p| p.is_file()))
    }

    fn quiet(cmd: &mut Command) -> &mut Command {
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
    }

    /// Run a thumbnailer, waiting at most 15 seconds.
    fn run(mut cmd: Command) -> bool {
        let Ok(mut child) = quiet(&mut cmd).spawn() else { return false };
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(s)) => return s.success(),
                Ok(None) if start.elapsed() < Duration::from_secs(15) => std::thread::sleep(Duration::from_millis(30)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
            }
        }
    }

    fn read_png(p: &Path, max: u32) -> Result<Picture, String> {
        let img = image::open(p).map_err(|e| e.to_string())?;
        let img = if img.width() > max || img.height() > max { img.thumbnail(max, max) } else { img };
        let rgba = img.to_rgba8();
        Ok((rgba.width() as usize, rgba.height() as usize, rgba.into_raw()))
    }

    pub fn thumbnail(path: &Path, max: u32) -> Result<Picture, String> {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("dirmap-thumb-{}-{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let out = dir.join("t.png");
        let size = max.to_string();
        let kind = path.file_name().and_then(|n| media_of(&n.to_string_lossy()));
        let mut tries: Vec<Command> = Vec::new();
        if cfg!(target_os = "macos") {
            let mut c = Command::new("/usr/bin/qlmanage");
            c.arg("-t").arg("-s").arg(&size).arg("-o").arg(&dir).arg(path);
            tries.push(c);
        } else if kind == Some(Media::Pdf) {
            if let Some(p) = find("pdftoppm") {
                let mut c = Command::new(p);
                c.args(["-png", "-singlefile", "-f", "1", "-l", "1", "-scale-to", &size]).arg(path).arg(dir.join("t"));
                tries.push(c);
            }
        } else {
            if let Some(p) = find("ffmpegthumbnailer") {
                let mut c = Command::new(p);
                c.arg("-i").arg(path).arg("-o").arg(&out).args(["-s", &size, "-c", "png"]);
                tries.push(c);
            }
            if let Some(p) = find("ffmpeg") {
                let mut c = Command::new(p);
                let scale = format!("scale='min({size},iw)':-2");
                c.args(["-hide_banner", "-loglevel", "error", "-y", "-ss", "1"]).arg("-i").arg(path).args(["-frames:v", "1", "-vf", &scale]).arg(&out);
                tries.push(c);
            }
        }
        let mut result = Err("No thumbnail".to_string());
        for c in tries {
            if run(c) {
                // qlmanage names its output after the file.
                let made = if out.is_file() {
                    Some(out.clone())
                } else {
                    std::fs::read_dir(&dir).ok().and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|x| x == "png")))
                };
                if let Some(m) = made {
                    result = read_png(&m, max);
                    if result.is_ok() {
                        break;
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        result
    }

    /// A player program, and its arguments before the file (starting `at` seconds in where it can).
    fn player(at: f64) -> Option<(PathBuf, Vec<String>, bool)> {
        if cfg!(target_os = "macos") {
            return Some((PathBuf::from("/usr/bin/afplay"), Vec::new(), false));
        }
        if let Some(p) = find("ffplay") {
            return Some((p, vec!["-nodisp".into(), "-autoexit".into(), "-loglevel".into(), "quiet".into(), "-ss".into(), format!("{at:.2}")], true));
        }
        for prog in ["gst-play-1.0", "paplay", "aplay"] {
            if let Some(p) = find(prog) {
                let args = if prog == "gst-play-1.0" { vec!["--quiet".into(), "--no-interactive".into()] } else { Vec::new() };
                return Some((p, args, false));
            }
        }
        None
    }

    fn signal(child: &Child, sig: i32) {
        unsafe {
            libc::kill(child.id() as i32, sig);
        }
    }

    pub fn play(path: &Path, rx: &mpsc::Receiver<Cmd>, state: &Mutex<PlayState>) {
        // macOS: AVAudioPlayer, which pauses at once and seeks (afplay plays on what it has
        // buffered after being stopped, and can't start part way).
        #[cfg(target_os = "macos")]
        {
            crate::macaudio::play(path, rx, state);
            return;
        }
        #[allow(unreachable_code)]
        let mut child: Option<Child> = None;
        let mut paused = false;
        // Position: where the current run started, plus time spent playing.
        let mut base = 0.0;
        let mut since: Option<Instant> = None;
        let start_at = |at: f64, child: &mut Option<Child>| -> Result<bool, String> {
            if let Some(mut c) = child.take() {
                signal(&c, libc::SIGCONT);
                let _ = c.kill();
                let _ = c.wait();
            }
            let Some((prog, args, seeks)) = player(at) else {
                return Err("No sound player is installed (ffplay, gst-play-1.0, paplay or aplay).".into());
            };
            let mut cmd = Command::new(prog);
            cmd.args(args).arg(path);
            *child = Some(quiet(&mut cmd).spawn().map_err(|e| e.to_string())?);
            Ok(seeks)
        };
        let mut can_seek = false;
        loop {
            match rx.recv_timeout(Duration::from_millis(150)) {
                Ok(Cmd::Play) => {
                    if let (Some(c), true) = (&child, paused) {
                        signal(c, libc::SIGCONT);
                        paused = false;
                        since = Some(Instant::now());
                    } else if child.is_none() {
                        base = 0.0;
                        match start_at(0.0, &mut child) {
                            Ok(s) => {
                                can_seek = s;
                                since = Some(Instant::now());
                                paused = false;
                            }
                            Err(e) => set(state, |s| s.error = Some(e)),
                        }
                    }
                }
                Ok(Cmd::Pause) => {
                    if let (Some(c), false) = (&child, paused) {
                        signal(c, libc::SIGSTOP);
                        paused = true;
                        base += since.take().map_or(0.0, |t| t.elapsed().as_secs_f64());
                    }
                }
                Ok(Cmd::Seek(t)) if can_seek => {
                    let was_paused = paused || child.is_none();
                    if start_at(t, &mut child).is_ok() {
                        base = t;
                        since = Some(Instant::now());
                        paused = false;
                        if was_paused && let Some(c) = &child {
                            signal(c, libc::SIGSTOP);
                            paused = true;
                            since = None;
                        }
                    }
                }
                Ok(Cmd::Seek(_)) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            let mut ended = false;
            if let Some(c) = &mut child
                && let Ok(Some(_)) = c.try_wait()
            {
                child = None;
                ended = true;
                since = None;
                base = 0.0;
            }
            let pos = base + since.map_or(0.0, |t| t.elapsed().as_secs_f64());
            set(state, |s| {
                s.playing = child.is_some() && !paused;
                s.position = pos;
                if ended {
                    s.ended = true;
                    s.position = 0.0;
                } else if s.playing {
                    s.ended = false;
                }
            });
        }
        if let Some(mut c) = child {
            signal(&c, libc::SIGCONT);
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_clock() {
        assert_eq!(media_of("Clip.MOV"), Some(Media::Video));
        assert_eq!(media_of("song.flac"), Some(Media::Audio));
        assert_eq!(media_of("paper.pdf"), Some(Media::Pdf));
        assert_eq!(media_of("notes.txt"), None);
        assert_eq!(clock(75.4), "1:15");
        assert_eq!(clock(3725.0), "1:02:05");
    }

    #[test]
    fn a_missing_file_has_no_thumbnail() {
        assert!(thumbnail(Path::new("/no/such/file.mp4"), 128).is_err());
    }
}
