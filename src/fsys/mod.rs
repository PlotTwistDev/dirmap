//! The file browsing service: reading folders, standard places and navigation history,
//! independently of scanning. See docs/design/dirmap-2.0.md (D.2).

pub mod history;
pub mod listing;
pub mod places;
pub mod watch;
pub mod archive;
pub mod remote;

use std::path::Path;

/// A path as the user should see it: server paths as their address (`sftp://host/dir`).
pub trait PathShow {
    fn shown(&self) -> String;
}

impl PathShow for Path {
    fn shown(&self) -> String {
        remote::show(self).unwrap_or_else(|| self.display().to_string())
    }
}
