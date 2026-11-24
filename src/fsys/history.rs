//! Where a file manager tab is, and where it has been.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A place the file manager can show.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Location {
    /// The drives and volumes ("This PC" / "Computer").
    Computer,
    Dir(PathBuf),
}

impl Location {
    pub fn dir(&self) -> Option<&Path> {
        match self {
            Location::Dir(p) => Some(p),
            Location::Computer => None,
        }
    }

    /// One level up: the parent folder, or Computer above a drive or volume root.
    pub fn up(&self) -> Option<Location> {
        let p = self.dir()?;
        if super::remote::is_remote(p) {
            // A server's top folder goes up to Computer.
            return Some(super::remote::parent(p).map(Location::Dir).unwrap_or(Location::Computer));
        }
        match p.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => Some(Location::Dir(parent.to_path_buf())),
            _ => Some(Location::Computer),
        }
    }
}

/// Back and Forward for one tab.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct History {
    pub here: Location,
    back: Vec<Location>,
    fwd: Vec<Location>,
}

/// How far back a tab remembers.
const LIMIT: usize = 200;

impl History {
    pub fn new(here: Location) -> Self {
        Self { here, back: Vec::new(), fwd: Vec::new() }
    }

    /// Go somewhere new: the current place goes on the Back list and Forward is forgotten.
    pub fn go(&mut self, to: Location) {
        if to == self.here {
            return;
        }
        let from = std::mem::replace(&mut self.here, to);
        self.back.push(from);
        if self.back.len() > LIMIT {
            self.back.remove(0);
        }
        self.fwd.clear();
    }

    pub fn can_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_forward(&self) -> bool {
        !self.fwd.is_empty()
    }

    pub fn back(&mut self) -> bool {
        let Some(to) = self.back.pop() else { return false };
        let from = std::mem::replace(&mut self.here, to);
        self.fwd.push(from);
        true
    }

    pub fn forward(&mut self) -> bool {
        let Some(to) = self.fwd.pop() else { return false };
        let from = std::mem::replace(&mut self.here, to);
        self.back.push(from);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(p: &str) -> Location {
        Location::Dir(PathBuf::from(p))
    }

    #[test]
    fn back_and_forward() {
        let mut h = History::new(d("/a"));
        assert!(!h.can_back() && !h.can_forward());
        h.go(d("/a/b"));
        h.go(d("/a/b/c"));
        h.go(d("/a/b/c")); // same place: no new step
        assert!(h.back());
        assert_eq!(h.here, d("/a/b"));
        assert!(h.back());
        assert_eq!(h.here, d("/a"));
        assert!(!h.back());
        assert!(h.forward());
        assert_eq!(h.here, d("/a/b"));
        h.go(d("/x")); // a new place forgets Forward
        assert!(!h.can_forward());
        assert!(h.back());
        assert_eq!(h.here, d("/a/b"));
    }

    #[test]
    fn up_ends_at_computer() {
        assert_eq!(d("/a/b").up(), Some(d("/a")));
        assert_eq!(d("/").up(), Some(Location::Computer));
        assert_eq!(Location::Computer.up(), None);
        if cfg!(windows) {
            assert_eq!(d(r"C:\Users").up(), Some(d(r"C:\")));
            assert_eq!(d(r"C:\").up(), Some(Location::Computer));
            assert_eq!(d(r"\\server\share\x").up(), Some(d(r"\\server\share\")));
        }
    }

    #[test]
    fn history_is_bounded() {
        let mut h = History::new(d("/0"));
        for i in 1..=500 {
            h.go(d(&format!("/{i}")));
        }
        let mut n = 0;
        while h.back() {
            n += 1;
        }
        assert_eq!(n, LIMIT);
    }
}
