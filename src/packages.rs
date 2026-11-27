//! Folders that are one document to their app (packages): a Logic Pro project, a PhotoCraft
//! document saved as a bundle, a LightCraft library. They preview like a file, and the Kind
//! column names them, though they still open like any folder.

use std::path::Path;

/// What the Kind column calls a package, from its name alone.
pub fn kind(name: &str) -> Option<&'static str> {
    if crate::daw::is_logic(name) {
        Some("Logic Pro project")
    } else if crate::craftfile::of(name) == Some(crate::craftfile::Craft::PhotoCraft) {
        Some("PhotoCraft picture")
    } else if crate::lightcraft::is_library_name(name) {
        Some("LightCraft library")
    } else {
        None
    }
}

/// A folder that previews like a file: by its name, or a LightCraft library under another name
/// (one look for its catalogue, not on a server).
pub fn is_package(path: &Path, name: &str) -> bool {
    kind(name).is_some()
        || (!crate::fsys::remote::is_remote(path)
            && (crate::lightcraft::is_library(path)
                || crate::splat::project::layout(path).is_some()))
}
