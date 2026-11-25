//! Finding a setting by name, description or a word people might use for it ("zoom",
//! "threads", "hidden files", "licence"). Each entry points at the section that holds it.

use super::Tab;
use crate::kit;
use eframe::egui::{self, Sense, vec2};

#[derive(Clone, Copy)]
pub struct Entry {
    pub page: Tab,
    /// The section's id, as the page passes it to `PageCx::focus`.
    pub section: &'static str,
    pub section_title: &'static str,
    pub title: &'static str,
    pub keywords: &'static str,
}

const fn e(page: Tab, section: &'static str, section_title: &'static str, title: &'static str, keywords: &'static str) -> Entry {
    Entry { page, section, section_title, title, keywords }
}

pub const ENTRIES: &[Entry] = &[
    e(Tab::General, "appearance", "Appearance", "Theme", "dark light system mode appearance colour color night"),
    e(Tab::General, "appearance", "Appearance", "Interface size", "scale scaling zoom dpi bigger smaller text size font high resolution display"),
    e(Tab::General, "appearance", "Appearance", "Striped rows", "stripes alternating rows zebra lists"),
    e(Tab::General, "appearance", "Appearance", "Backup reminder at start", "backup backups reminder start launch offline data loss warning don't show again disclaimer"),
    e(Tab::General, "appearance", "Appearance", "Explain system files", "system files folders explain what is this pagefile hiberfil winsxs spotlight proc swap safe to delete info badge"),
    e(Tab::General, "sizes", "File sizes", "Size units", "binary decimal kb kib mb gb 1024 1000 units bytes format"),
    e(Tab::General, "scanning", "Scanning", "Live scan results", "live tree progress scanning while found"),
    e(Tab::General, "updates", "Updates", "Check for updates", "update updates version new release notes automatic download latest"),
    e(Tab::Filtering, "summary", "Active rules", "Active filter rules", "filters summary rescan exclusions rules"),
    e(Tab::Filtering, "folders", "Skipped folders", "Folders to skip", "exclude ignore folder directory path skip browse"),
    e(Tab::Filtering, "names", "File names", "File names to skip", "exclude ignore wildcard glob pattern extension tmp skip name"),
    e(Tab::Filtering, "names", "File names", "Only count matching names", "include only wildcard glob pattern extension whitelist"),
    e(Tab::Filtering, "sizeage", "Size and age", "Skip small files", "size minimum smaller bytes kb mb small files"),
    e(Tab::Filtering, "sizeage", "Size and age", "Skip old files", "age days modified date old changed recent"),
    e(Tab::FolderList, "preview", "Preview", "Folder list preview", "preview columns table list"),
    e(Tab::FolderList, "columns", "Columns", "Columns", "column percentage percent bar files count last change modified date name size"),
    e(Tab::FolderList, "rows", "Rows", "Striped rows", "stripes alternating rows zebra"),
    e(Tab::Treemap, "style", "Tile style", "Tile style", "modern classic cushion style tiles look"),
    e(Tab::Treemap, "details", "Tile details", "File labels", "names labels text on tiles"),
    e(Tab::Treemap, "details", "Tile details", "Selection border", "selection border outline colour color highlight"),
    e(Tab::Treemap, "shading", "Classic cushion shading", "Brightness", "brightness light dark shading"),
    e(Tab::Treemap, "shading", "Classic cushion shading", "Cushion shading", "cushion shading flat 3d"),
    e(Tab::Treemap, "shading", "Classic cushion shading", "Height and scale factor", "height scale factor depth cushion"),
    e(Tab::Treemap, "shading", "Classic cushion shading", "Light direction", "light direction angle sun x y"),
    e(Tab::Treemap, "shading", "Classic cushion shading", "Grid", "grid lines colour color outline"),
    e(Tab::Treemap, "shading", "Classic cushion shading", "Block depth", "bevel depth block edges effect"),
    e(Tab::Treemap, "shading", "Classic cushion shading", "Presets", "presets classic soft flat"),
    e(Tab::Treemap, "lighting", "Lighting", "Interactive lighting", "light lighting intensity glow hover pointer effect"),
    e(Tab::Treemap, "navigation", "Navigation", "Smooth zoom", "zoom smooth animation animate motion reduce"),
    e(Tab::Treemap, "navigation", "Navigation", "Zoom sensitivity", "zoom sensitivity wheel trackpad pinch scroll speed"),
    e(Tab::Treemap, "navigation", "Navigation", "Animation speed", "animation speed fast slow motion"),
    e(Tab::Cloud, "sizes", "Sizes", "Size shown", "size on disk logical allocated du metric cloud only default"),
    e(Tab::Cloud, "sizes", "Sizes", "Stored column", "stored column online only downloaded logical size on disk list"),
    e(Tab::Cloud, "status", "Status", "Status icons", "cloud status icons indicators online only downloaded pinned partial onedrive google drive icloud dropbox"),
    e(Tab::Cloud, "status", "Status", "Storage breakdown", "breakdown bar cloud local downloaded only in the cloud percent"),
    e(Tab::Cloud, "downloads", "Keeping files in the cloud", "Ask before downloading", "download hydrate hydration confirm ask cloud files placeholders"),
    e(Tab::Cloud, "downloads", "Keeping files in the cloud", "Scan mounted cloud drives", "rclone fuse mount mounted cloud drive google drive g: pcloud webdav"),
    e(Tab::Cloud, "found", "Cloud folders on this computer", "Cloud folders found", "providers detected sync roots cloudstorage onedrive sharepoint box nextcloud owncloud mega synology"),
    e(Tab::Advanced, "scanning", "Scanning behaviour", "Skip hidden files", "hidden dot files system files skip"),
    e(Tab::Advanced, "filesystem", "Filesystem", "Stay on the scanned drive", "mount mounted drives file system one filesystem symlink symbolic links junction"),
    e(Tab::Advanced, "performance", "Performance", "Scanner threads", "threads cpu cores performance profile network nas speed concurrency"),
    e(Tab::Advanced, "permissions", "Permissions", "Administrator and permissions", "admin administrator elevated sudo root permissions full disk access uac privileges"),
    e(Tab::Advanced, "diagnostics", "Diagnostics", "Settings folder and system details", "diagnostics troubleshooting support config folder details problem report renderer"),
    e(Tab::Licensing, "status", "Licence", "Licence status", "licence license serial evaluation trial expiry expires edition professional updates status"),
    e(Tab::Licensing, "activate", "Activate", "Activate a licence", "licence license key serial activate activation paste offline verify certificate"),
    e(Tab::Licensing, "status", "Licence", "Buy Filescape Professional", "purchase buy price upgrade professional"),
];

/// Entries matching every word of `query`, best first: name matches before keyword matches.
pub fn find(query: &str) -> Vec<&'static Entry> {
    let q = query.to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    let mut hits: Vec<(u8, &Entry)> = ENTRIES
        .iter()
        .filter_map(|e| {
            let hay = format!("{} {} {} {}", e.title, e.section_title, e.page.name(), e.keywords).to_lowercase();
            let hay = hay.replace("licence", "licence license");
            if !words.iter().all(|w| hay.contains(w)) {
                return None;
            }
            let title = e.title.to_lowercase();
            let rank = if title.starts_with(&q) {
                0
            } else if words.iter().all(|w| title.contains(w)) {
                1
            } else {
                2
            };
            Some((rank, e))
        })
        .collect();
    hits.sort_by_key(|(r, _)| *r);
    hits.into_iter().map(|(_, e)| e).collect()
}

/// One result in the sidebar: the setting's name, and where it lives.
pub fn result(ui: &mut egui::Ui, e: &Entry, current: bool) -> egui::Response {
    let t = kit::tokens(ui);
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 44.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, e.title));
    let p = ui.painter();
    if current {
        p.rect_filled(rect, kit::CONTROL_RADIUS, t.accent_soft);
    } else if resp.hovered() {
        p.rect_filled(rect, kit::CONTROL_RADIUS, t.pal.hover);
    }
    kit::icons::paint(p, egui::Rect::from_center_size(egui::pos2(rect.left() + 18.0, rect.top() + 15.0), vec2(14.0, 14.0)), e.page.icon(), t.weak);
    let clip = p.with_clip_rect(rect.shrink(2.0));
    clip.text(egui::pos2(rect.left() + 34.0, rect.top() + 15.0), egui::Align2::LEFT_CENTER, e.title, egui::FontId::proportional(kit::LABEL - 0.5), t.strong);
    clip.text(
        egui::pos2(rect.left() + 34.0, rect.top() + 32.0),
        egui::Align2::LEFT_CENTER,
        format!("{} › {}", e.page.name(), e.section_title),
        egui::FontId::proportional(kit::SMALL),
        t.weak,
    );
    kit::focus_ring(ui, rect, &resp, kit::CONTROL_RADIUS as f32);
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_settings_by_everyday_words() {
        let first = |q: &str| find(q).first().map(|e| (e.page, e.title));
        assert_eq!(first("zoom").map(|x| x.0), Some(Tab::Treemap));
        assert_eq!(first("threads"), Some((Tab::Advanced, "Scanner threads")));
        assert_eq!(first("hidden files"), Some((Tab::Advanced, "Skip hidden files")));
        assert_eq!(first("licence").map(|x| x.0), Some(Tab::Licensing));
        assert_eq!(first("license").map(|x| x.0), Some(Tab::Licensing));
        assert_eq!(first("dark").map(|x| x.1), Some("Theme"));
        assert!(find("no such setting at all").is_empty());
    }
}
