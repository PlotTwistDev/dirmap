//! File and folder icons. On Windows these are the shell's own icons (what Explorer shows);
//! elsewhere simple drawn glyphs are used.

use eframe::egui::{self, Color32, Rect, TextureHandle};
use std::collections::HashMap;

#[derive(Default)]
pub struct Icons {
    /// Keyed by lowercase extension; "<dir>" for folders. `None` = the OS had no icon.
    cache: HashMap<String, Option<TextureHandle>>,
}

const DIR_KEY: &str = "<dir>";

impl Icons {
    /// Draw the icon for a folder (`is_dir`) or a file with extension `ext` into `rect`.
    /// `tint` colors the fallback glyph.
    pub fn paint(&mut self, ui: &egui::Ui, rect: Rect, is_dir: bool, ext: &str, tint: Color32) {
        let key = if is_dir { DIR_KEY.to_string() } else { ext.to_ascii_lowercase() };
        let ctx = ui.ctx().clone();
        let tex = self.cache.entry(key).or_insert_with_key(|k| load(&ctx, k));
        match tex {
            Some(t) => {
                let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(t.id(), rect, uv, Color32::WHITE);
            }
            None => paint_glyph(ui.painter(), rect, is_dir, ext, tint),
        }
    }
}

/// Fallback icons: a manila folder, or a page with a folded corner in the extension's color.
/// Drawn in proportion, so they work from list size (16 px) up to the Files view's big icons,
/// where the page also carries the extension.
fn paint_glyph(p: &egui::Painter, r: Rect, is_dir: bool, ext: &str, tint: Color32) {
    let s = r.height() / 16.0;
    let r = r.shrink(s);
    let round = 1.5 * s;
    if is_dir {
        let c = Color32::from_rgb(232, 188, 80);
        let tab = Rect::from_min_size(r.min + egui::vec2(0.0, s), egui::vec2(r.width() * 0.45, 3.0 * s));
        let body = Rect::from_min_max(r.min + egui::vec2(0.0, 3.0 * s), r.max - egui::vec2(0.0, s));
        p.rect_filled(tab, round * 0.7, c.gamma_multiply(0.85));
        p.rect_filled(body, round, c);
        if s >= 2.0 {
            // A lighter front flap gives the big folder some depth.
            let flap = Rect::from_min_max(egui::pos2(body.min.x, body.min.y + body.height() * 0.22), body.max);
            p.rect_filled(flap, round, Color32::from_rgb(245, 205, 105));
        }
    } else {
        let w = r.height() * 0.78;
        let page = Rect::from_center_size(r.center(), egui::vec2(w, r.height()));
        p.rect_filled(page, round, Color32::from_gray(225));
        let band = Rect::from_min_max(egui::pos2(page.min.x, page.max.y - 5.0 * s), page.max);
        p.rect_filled(band, round, tint);
        let fold = page.right_top();
        let f = 4.0 * s;
        p.add(egui::Shape::convex_polygon(
            vec![fold + egui::vec2(-f, 0.0), fold + egui::vec2(0.0, f), fold + egui::vec2(-f, f)],
            Color32::from_gray(160),
            egui::Stroke::NONE,
        ));
        if s >= 2.0 && !ext.is_empty() {
            let label: String = ext.to_uppercase().chars().take(5).collect();
            let size = (band.height() * 0.62).min(band.width() / (label.chars().count() as f32 * 0.62));
            let text = if luma(tint) > 150.0 { Color32::from_gray(30) } else { Color32::WHITE };
            p.text(band.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(size), text);
        }
    }
}

fn luma(c: Color32) -> f32 {
    0.299 * c.r() as f32 + 0.587 * c.g() as f32 + 0.114 * c.b() as f32
}

#[cfg(not(windows))]
fn load(_ctx: &egui::Context, _key: &str) -> Option<TextureHandle> {
    None
}

#[cfg(windows)]
fn load(ctx: &egui::Context, key: &str) -> Option<TextureHandle> {
    let rgba = win::shell_icon(key)?;
    let img = egui::ColorImage::from_rgba_unmultiplied([rgba.0, rgba.1], &rgba.2);
    Some(ctx.load_texture(format!("icon:{key}"), img, egui::TextureOptions::LINEAR))
}

#[cfg(windows)]
mod win {
    use windows_sys::Win32::Graphics::Gdi::{
        DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        DIB_RGB_COLORS,
    };
    use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
    use windows_sys::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGFI_USEFILEATTRIBUTES};
    use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO};

    /// The shell's 32px icon for a folder or for files with this extension, as RGBA.
    /// SHGFI_USEFILEATTRIBUTES means no file needs to exist and nothing touches the disk.
    pub fn shell_icon(key: &str) -> Option<(usize, usize, Vec<u8>)> {
        let (name, attr) = if key == super::DIR_KEY {
            ("folder".to_string(), FILE_ATTRIBUTE_DIRECTORY)
        } else {
            (format!("file.{key}"), FILE_ATTRIBUTE_NORMAL)
        };
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            let mut info: SHFILEINFOW = std::mem::zeroed();
            let ok = SHGetFileInfoW(
                wide.as_ptr(),
                attr,
                &mut info,
                std::mem::size_of::<SHFILEINFOW>() as u32,
                SHGFI_ICON | SHGFI_LARGEICON | SHGFI_USEFILEATTRIBUTES,
            );
            if ok == 0 || info.hIcon.is_null() {
                return None;
            }
            let out = icon_to_rgba(info.hIcon);
            DestroyIcon(info.hIcon);
            out
        }
    }

    unsafe fn icon_to_rgba(icon: windows_sys::Win32::UI::WindowsAndMessaging::HICON) -> Option<(usize, usize, Vec<u8>)> {
        unsafe {
            let mut ii: ICONINFO = std::mem::zeroed();
            if GetIconInfo(icon, &mut ii) == 0 {
                return None;
            }
            let result = (|| {
                if ii.hbmColor.is_null() {
                    return None; // monochrome icon; use the fallback glyph
                }
                let mut bm: BITMAP = std::mem::zeroed();
                if GetObjectW(ii.hbmColor, std::mem::size_of::<BITMAP>() as i32, &mut bm as *mut _ as *mut _) == 0 {
                    return None;
                }
                let (w, h) = (bm.bmWidth as usize, bm.bmHeight as usize);
                let color = dib(ii.hbmColor, w, h)?;
                let mut rgba = Vec::with_capacity(w * h * 4);
                let has_alpha = color.chunks(4).any(|p| p[3] != 0);
                let mask = if has_alpha { None } else { dib(ii.hbmMask, w, h) };
                for (i, p) in color.chunks(4).enumerate() {
                    // BGRA -> RGBA. Old icons without alpha use the AND mask instead.
                    let a = match &mask {
                        Some(m) => if m[i * 4] == 0 { 255 } else { 0 },
                        None => p[3],
                    };
                    rgba.extend_from_slice(&[p[2], p[1], p[0], a]);
                }
                Some((w, h, rgba))
            })();
            if !ii.hbmColor.is_null() {
                DeleteObject(ii.hbmColor);
            }
            if !ii.hbmMask.is_null() {
                DeleteObject(ii.hbmMask);
            }
            result
        }
    }

    /// Read a bitmap as top-down 32-bit BGRA.
    unsafe fn dib(bmp: windows_sys::Win32::Graphics::Gdi::HBITMAP, w: usize, h: usize) -> Option<Vec<u8>> {
        unsafe {
            let mut bmi: BITMAPINFO = std::mem::zeroed();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut buf = vec![0u8; w * h * 4];
            let dc = GetDC(std::ptr::null_mut());
            let lines = GetDIBits(dc, bmp, 0, h as u32, buf.as_mut_ptr() as *mut _, &mut bmi, DIB_RGB_COLORS);
            ReleaseDC(std::ptr::null_mut(), dc);
            (lines == h as i32).then_some(buf)
        }
    }
}
