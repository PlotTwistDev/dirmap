//! What is free and what needs a licence (Rich, 23 Nov 2026: the file browser, the disk map and
//! basic filtering are free; everything else is premium).
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
    /// Show category buttons, size limits, Highlight or Show matches only, scan exclusions.
    BasicFilters,
    // Premium
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
        Feature::BasicFilters,
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
        matches!(self, Feature::FileBrowser | Feature::DiskMap | Feature::BasicFilters)
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
