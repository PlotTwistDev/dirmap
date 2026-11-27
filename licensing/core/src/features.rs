//! What is free and what needs a licence (Rich, 23 Nov 2026: the file browser and the disk map are
//! free; 24 Nov: filtering by type or size is premium too, so the free app is a viewer you navigate
//! by hand; everything else is premium; 25 Nov: in the AI Models workspace, discovering,
//! understanding and experimenting by hand are free and automating is premium). Scan exclusions
//! are a scan setting and never checked. [`MATRIX`] is the one table of who gets what.
//!
//! Checks happen only when an operation starts: [`begin`] hands out a [`Permit`], which has no
//! expiry and is never checked again. Work already queued or running (transfers, moves, renames,
//! archive writes, retries and resumes) always finishes, even if the licence expires meanwhile.

use crate::standing::Standing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feature {
    // Free
    FileBrowser,
    DiskMap,
    // Premium
    Filters,
    CustomFilter,
    SequenceFilter,
    Remote,
    RemoteTransfers,
    SequencePlayer,
    BatchRename,
    FolderCompare,
    SplatCompare,
    SplatProjects,
    Duplicates,
    Archives,
    SavedSearches,
    Snapshots,
    CloudManagement,
    // AI Models workspace (Rich, 25 Nov 2026: "Make curiosity free. Charge for productivity."
    // Doing something by hand, one model or one file at a time, is free; automating it is Pro.)
    AiModels,
    AskAi,
    ModelLab,
    AiBatch,
    ModelAutomation,
    Quantize,
    ResearchAutomation,
}

/// Who gets a feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tier {
    /// Everyone, for good, with or without a licence.
    Free,
    /// A Filescape Professional licence, an evaluation, or a temporary or enterprise licence.
    Pro,
}

/// Where a feature lives in the app, to group the matrix on screens and in the manual.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Area {
    Files,
    Storage,
    Cloud,
    Remote,
    AiModels,
}

/// One row of the entitlement matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entitlement {
    pub feature: Feature,
    pub tier: Tier,
    pub area: Area,
    /// Says what needs a licence, in a sentence such as "{name} is part of Filescape Professional."
    pub name: &'static str,
    /// What the row covers, so that a new piece of the app can be placed in the right row.
    pub covers: &'static str,
}

const fn row(feature: Feature, tier: Tier, area: Area, name: &'static str, covers: &'static str) -> Entitlement {
    Entitlement { feature, tier, area, name, covers }
}

use Area as A;
use Feature as F;
use Tier::{Free, Pro};

/// The entitlement matrix: the one place that says what is free and what is Pro. The app, its
/// background work, its menus and the command line all ask [`begin`], which reads this table, so a
/// feature is never free in one place and Pro in another.
///
/// Rules every row keeps:
/// - The check happens once, where an operation starts (see [`begin`]). A scheduled or repeated job
///   checks again at the start of each run, never midway.
/// - A licence ending never deletes or hides what the user made: files, settings, saved searches,
///   snapshots, experiment results and downloaded models stay where they are.
/// - Local AI work is never metered: no limits on model size, model count, local inference, GPU
///   use or experiments, and installing an optional extension never needs a licence.
pub const MATRIX: [Entitlement; 24] = [
    row(F::FileBrowser, Free, A::Files, "The file browser",
        "The Files workspace: browse, tabs, address bar, the filter box, preview, open, copy, move, rename, delete, New."),
    row(F::DiskMap, Free, A::Storage, "The disk map",
        "Scanning, scan exclusions, the treemap, sunburst, directory tree, file types and sizes, and cloud sizes and status."),
    row(F::Filters, Pro, A::Storage, "Filtering by type or size",
        "The Show buttons and the Files Show menu, Larger than and Smaller than, picking a range on the size chart, and highlighting a file type on the map. Turning a filter off is always free."),
    row(F::CustomFilter, Pro, A::Storage, "The Custom filter", "The custom pattern filter (Ctrl+F), filter presets and recent filters."),
    row(F::SequenceFilter, Pro, A::Storage, "Finding numbered sequences", "The frame-number (sequence) filter."),
    row(F::Remote, Pro, A::Remote, "Connecting to servers", "The Remote workspace: SFTP, FTP and WebDAV, the Site Manager and the SSH terminal."),
    row(F::RemoteTransfers, Pro, A::Remote, "Server transfers", "Starting new transfers to or from a server. Retries and resumes are never checked."),
    row(F::SequencePlayer, Pro, A::Files, "The sequence player", "Playing numbered image sequences."),
    row(F::BatchRename, Pro, A::Files, "Batch rename", "Renaming several files at once by rules."),
    row(F::FolderCompare, Pro, A::Files, "Folder compare", "Compare Folders."),
    row(F::SplatCompare, Pro, A::Files, "Comparing splat scenes",
        "Compare Scenes: two Gaussian splat scenes side by side, split or as their difference, with cameras turning together. Viewing, inspecting and converting one scene is free."),
    row(F::SplatProjects, Pro, A::Files, "Splat training project breakdowns",
        "What a Gaussian splat training project or capture folder takes, part by part, and its timeline of saved scenes. Recognising the project and opening its scenes is free."),
    row(F::Duplicates, Pro, A::Storage, "The duplicate finder", "Finding duplicate files across a scan."),
    row(F::Archives, Pro, A::Files, "Archives", "Browsing into and extracting archives."),
    row(F::SavedSearches, Pro, A::Files, "Saved searches", "Saving a search to run again."),
    row(F::Snapshots, Pro, A::Storage, "Snapshots", "Saving a scan as a snapshot, in the app or from the command line, and comparing with one."),
    row(F::CloudManagement, Pro, A::Cloud, "Managing cloud storage",
        "The Reclaimable view, and freeing up or keeping folders or several items at once, in the app or from the command line. One file at a time is free."),
    row(F::AiModels, Free, A::AiModels, "The AI Models workspace",
        "Finding models, the library, model details and architecture, the model storage treemap, tensors and weight statistics, histograms, health and integrity checks, will-it-run and memory estimates, comparing models by hand, finding copies and unused downloads, and deleting or moving one model at a time."),
    row(F::AskAi, Free, A::AiModels, "Ask AI",
        "Downloading a model with the user's approval and running it locally with any GPU: chat, questions about a file, summaries, image descriptions, semantic search and a suggested name for one file."),
    row(F::ModelLab, Free, A::AiModels, "The Model Lab",
        "The tokenizer, token probabilities, sampling controls, a basic benchmark, activation and attention views, hidden states, probing by hand, interventions that change nothing on disk, and installing an optional research extension."),
    row(F::AiBatch, Pro, A::AiModels, "Processing files with AI in bulk",
        "Running AI over many files at once: batch descriptions or summaries, AI-suggested batch renaming, and automatic categorising or organising."),
    row(F::ModelAutomation, Pro, A::AiModels, "Managing models in bulk",
        "Moving several models at once, removing copies automatically, and scheduled model maintenance."),
    row(F::Quantize, Pro, A::AiModels, "Model quantisation", "Quantising a model and other advanced optimisation. The result is always a new file; the original is kept."),
    row(F::ResearchAutomation, Pro, A::AiModels, "Automated model research",
        "Automated benchmark suites and experiments, experiment history and revisions, reusable research workflows, AI-assisted research, advanced reports and export, multi-model orchestration and custom automation."),
];

impl Feature {
    pub const ALL: [Feature; MATRIX.len()] = {
        let mut all = [Feature::FileBrowser; MATRIX.len()];
        let mut i = 0;
        while i < MATRIX.len() {
            all[i] = MATRIX[i].feature;
            i += 1;
        }
        all
    };

    /// This feature's row of [`MATRIX`].
    pub fn entitlement(self) -> &'static Entitlement {
        MATRIX.iter().find(|e| e.feature == self).expect("every feature has a row in MATRIX")
    }

    pub fn tier(self) -> Tier {
        self.entitlement().tier
    }

    pub fn name(self) -> &'static str {
        self.entitlement().name
    }

    pub fn is_free(self) -> bool {
        self.tier() == Tier::Free
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_feature_has_exactly_one_row() {
        for f in Feature::ALL {
            assert_eq!(MATRIX.iter().filter(|e| e.feature == f).count(), 1, "{f:?}");
            assert!(!f.name().is_empty() && !f.entitlement().covers.is_empty());
        }
    }

    /// Changing who gets a feature is a pricing decision: update this list only with Rich's approval.
    #[test]
    fn the_free_features_are_the_approved_ones() {
        let free: Vec<Feature> = Feature::ALL.into_iter().filter(|f| f.is_free()).collect();
        assert_eq!(free, [Feature::FileBrowser, Feature::DiskMap, Feature::AiModels, Feature::AskAi, Feature::ModelLab]);
    }
}
