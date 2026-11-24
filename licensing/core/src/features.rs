//! What is free and what needs a licence (Rich, 23 Nov 2026: the file browser and the disk map are
//! free; 24 Nov: filtering by type or size is premium too, so the free app is a viewer you navigate
//! by hand; everything else is premium). Scan exclusions are a scan setting and never checked.
//!
//! Checks happen only when an operation starts: [`begin`] hands out a [`Permit`], which has no
//! expiry and is never checked again. Work already queued or running (transfers, moves, renames,
//! archive writes, retries and resumes) always finishes, even if the licence expires meanwhile.

use crate::standing::Standing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feature {
    // Free
    /// Files workspace: browse, tabs, address bar, preview, open, copy, move, rename, delete, New.
    FileBrowser,
    /// Scanning, treemap, sunburst, directory tree, file types, sizes.
    DiskMap,
    // Premium
    /// Filtering by type or size: the Show buttons and the Files Show menu, Larger than and
    /// Smaller than, picking a range on the size chart, and highlighting a file type on the map.
    Filters,
    /// Custom pattern filter (Ctrl+F), filter presets and recent filters.
    CustomFilter,
    /// The frame-number (sequence) filter.
    SequenceFilter,
    /// Remote workspace: connect to SFTP, FTP or WebDAV, Site Manager, SSH terminal.
    Remote,
    /// Starting new transfers to or from a server (retries and resumes are never gated).
    RemoteTransfers,
    SequencePlayer,
    BatchRename,
    FolderCompare,
    Duplicates,
    Archives,
    SavedSearches,
    Snapshots,
}

impl Feature {
    pub const ALL: [Feature; 14] = [
        Feature::FileBrowser,
        Feature::DiskMap,
        Feature::Filters,
        Feature::CustomFilter,
        Feature::SequenceFilter,
        Feature::Remote,
        Feature::RemoteTransfers,
        Feature::SequencePlayer,
        Feature::BatchRename,
        Feature::FolderCompare,
        Feature::Duplicates,
        Feature::Archives,
        Feature::SavedSearches,
        Feature::Snapshots,
    ];

    /// The one table of free features.
    pub fn is_free(self) -> bool {
        matches!(self, Feature::FileBrowser | Feature::DiskMap)
    }
}

/// Proof that an operation was allowed when it started. Deliberately carries no time and has no
/// methods that consult the licence again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permit(pub Feature);

pub fn allows(standing: &Standing, feature: Feature) -> bool {
    feature.is_free() || standing.is_licensed()
}

/// Call where an operation starts. `None` means show the licence prompt instead.
pub fn begin(standing: &Standing, feature: Feature) -> Option<Permit> {
    allows(standing, feature).then_some(Permit(feature))
}
