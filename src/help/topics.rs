//! Help pages the app links to, by path under `docs/src` (with an optional `#heading`). Every
//! constant here is checked by the tests, so a renamed page or heading fails the build's tests
//! instead of leaving a dead Help link.

// A catalogue of linkable places; the tests check every one exists, even those the app doesn't link to yet.
#![allow(dead_code)]

pub const HOME: &str = "index.md";
pub const QUICK_START: &str = "getting-started/quick-start.md";
pub const INTERFACE: &str = "getting-started/interface.md";
pub const TOOLBAR: &str = "getting-started/interface.md#toolbar";
pub const SCANNING: &str = "getting-started/scanning.md";
pub const WHILE_SCANNING: &str = "getting-started/scanning.md#while-it-scans";
pub const RESULTS: &str = "getting-started/results.md";
pub const SIZES: &str = "disk-usage/sizes.md";
pub const PERCENTAGES: &str = "disk-usage/sizes.md#percentages";
pub const FILE_TYPES: &str = "disk-usage/file-types.md#the-file-types-list";
pub const UNREADABLE: &str = "disk-usage/hidden-and-unreadable.md#unreadable-folders-and-files";
pub const TREEMAP: &str = "views/treemap.md";
pub const TREEMAP_NAV: &str = "views/treemap-navigation.md";
pub const ZOOM_TO_SELECTION: &str = "views/treemap-navigation.md#zooming-to-the-selection";
pub const BACK_FORWARD: &str = "views/treemap-navigation.md#back-and-forward";
pub const SUNBURST: &str = "views/sunburst.md";
pub const TREE: &str = "views/directory-tree.md";
pub const COLOR_MODES: &str = "views/color-modes.md";
pub const FILTERING: &str = "filtering/overview.md";
pub const HIGHLIGHT_OR_ONLY: &str = "filtering/overview.md#highlight-or-show-matches-only";
pub const CATEGORIES: &str = "filtering/categories.md";
pub const SYNTAX: &str = "filtering/syntax.md";
pub const EXAMPLES: &str = "filtering/examples.md";
pub const SEQUENCES: &str = "filtering/sequences.md";
pub const PRESETS: &str = "filtering/presets.md";
pub const SCAN_FILTERS: &str = "filtering/scan-filters.md";
pub const HISTOGRAM: &str = "analysis/sizes.md";
pub const DUPLICATES: &str = "analysis/duplicates.md";
pub const DUPLICATES_HOW: &str = "analysis/duplicates.md#how-detection-works";
pub const SNAPSHOTS: &str = "analysis/snapshots.md";
pub const FILE_OPERATIONS: &str = "analysis/file-operations.md";
pub const SETTINGS: &str = "settings/reference.md";
pub const SETTINGS_GENERAL: &str = "settings/reference.md#general";
pub const SETTINGS_FILTERING: &str = "settings/reference.md#filtering";
pub const SETTINGS_FOLDER_LIST: &str = "settings/reference.md#folder-list";
pub const SETTINGS_TREEMAP: &str = "settings/reference.md#treemap";
pub const SETTINGS_ADVANCED: &str = "settings/reference.md#advanced";
pub const ADMINISTRATOR: &str = "settings/administrator.md";
pub const KEYBOARD: &str = "reference/keyboard.md";
pub const MOUSE: &str = "reference/mouse.md";
pub const TROUBLESHOOTING: &str = "troubleshooting/index.md";
pub const WHATS_NEW: &str = "about/whats-new.md";
pub const ABOUT: &str = "about/about.md";

/// Every constant above, for the tests.
#[cfg(test)]
pub const ALL: &[&str] = &[
    HOME, QUICK_START, INTERFACE, TOOLBAR, SCANNING, WHILE_SCANNING, RESULTS, SIZES, PERCENTAGES, FILE_TYPES, UNREADABLE,
    TREEMAP, TREEMAP_NAV, ZOOM_TO_SELECTION, BACK_FORWARD, SUNBURST, TREE, COLOR_MODES, FILTERING, HIGHLIGHT_OR_ONLY,
    CATEGORIES, SYNTAX, EXAMPLES, SEQUENCES, PRESETS, SCAN_FILTERS, HISTOGRAM, DUPLICATES, DUPLICATES_HOW, SNAPSHOTS,
    FILE_OPERATIONS, SETTINGS, SETTINGS_GENERAL, SETTINGS_FILTERING, SETTINGS_FOLDER_LIST, SETTINGS_TREEMAP,
    SETTINGS_ADVANCED, ADMINISTRATOR, KEYBOARD, MOUSE, TROUBLESHOOTING, WHATS_NEW, ABOUT,
];
