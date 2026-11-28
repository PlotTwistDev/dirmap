//! Filescape's own choices about which apps open what, kept with the settings: apps pinned to
//! the top of every Open With list, apps used last for each type, apps added to a type's list,
//! and Filescape's own default apps for types and kinds of file. None of it changes the
//! system's own settings.

use super::{Handler, Type};
use serde::{Deserialize, Serialize};

/// Apps used last for each type, at most this many.
pub const RECENTS: usize = 5;

/// Open With's own memory: apps pinned to the top of every list, and the ones used last for
/// each type of file. Kept by Filescape only: the system's own settings are never touched.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenPrefs {
    /// Apps (by their system id) listed first after the default, in the order pinned.
    pub pinned: Vec<String>,
    /// For each type ([`Type::key`]), apps used last, newest first.
    pub recent: std::collections::HashMap<String, Vec<String>>,
    /// Filescape's own default app for a type, used before the system's.
    pub defaults: std::collections::HashMap<String, Handler>,
    /// Filescape's own default app for a whole kind of file (by [`kind_key`]), used when the
    /// type has none of its own.
    pub kind_defaults: std::collections::HashMap<String, Handler>,
    /// Apps added to a type's list from Choose Another Application ("Remember in Open With").
    pub remembered: std::collections::HashMap<String, Vec<Handler>>,
}


/// Where the app an item opens in comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    /// Filescape's default for this type.
    Type,
    /// Filescape's default for this kind of file.
    Kind,
    /// The system's default.
    System,
}

/// How a kind of file is kept in settings ("Documents").
pub fn kind_key(c: crate::categories::Category) -> String {
    format!("{c:?}")
}

/// A kind's name for people ("documents").
pub fn kind_name(c: crate::categories::Category) -> String {
    let l = c.label();
    l.split_once(' ').map_or(l, |(_, n)| n).to_lowercase()
}

impl OpenPrefs {
    pub fn is_pinned(&self, id: &str) -> bool {
        self.pinned.iter().any(|p| p == id)
    }

    pub fn toggle_pin(&mut self, id: &str) {
        if self.is_pinned(id) {
            self.pinned.retain(|p| p != id);
        } else {
            self.pinned.push(id.to_string());
        }
    }

    /// Filescape's own default for a type: the type's, else its kind's.
    pub fn own_default(&self, t: &Type) -> Option<(Handler, Source)> {
        if let Some(h) = t.key().and_then(|k| self.defaults.get(&k)) {
            return Some((h.clone(), Source::Type));
        }
        let h = t.category().and_then(|c| self.kind_defaults.get(&kind_key(c)))?;
        Some((h.clone(), Source::Kind))
    }

    /// The app an item opens in: Filescape's default for its type, then for its kind, then the
    /// system's (None while the system's list isn't known, or it has no default).
    pub fn default_for(&self, t: &Type, found: Option<&[Handler]>) -> Option<(Handler, Source)> {
        self.own_default(t).or_else(|| found?.iter().find(|h| h.default).map(|h| (h.clone(), Source::System)))
    }

    /// The apps listed for a type: the system's, those remembered for it, and Filescape's
    /// default; `default` marks the one it opens in. Also the system default's id.
    pub fn listing(&self, t: &Type, found: &[Handler]) -> (Vec<Handler>, Option<String>) {
        let system = found.iter().find(|h| h.default).map(|h| h.id.clone());
        let mut list: Vec<Handler> = found.to_vec();
        let extra = t.key().and_then(|k| self.remembered.get(&k)).into_iter().flatten();
        for h in extra.chain(self.own_default(t).as_ref().map(|(h, _)| h)) {
            if !list.iter().any(|x| x.id == h.id) {
                list.push(h.clone());
            }
        }
        let chosen = self.default_for(t, Some(found)).map(|(h, _)| h.id);
        for h in &mut list {
            h.default = chosen.as_ref() == Some(&h.id);
        }
        (list, system)
    }

    /// Add an app to a type's list.
    pub fn remember(&mut self, t: &Type, h: &Handler) {
        let Some(k) = t.key() else { return };
        let r = self.remembered.entry(k).or_default();
        if !r.iter().any(|x| x.id == h.id) {
            let mut h = h.clone();
            h.default = false;
            r.push(h);
        }
    }

    /// Remember an app was used for a type.
    pub fn used(&mut self, t: &Type, id: &str) {
        let Some(k) = t.key() else { return };
        let r = self.recent.entry(k).or_default();
        r.retain(|x| x != id);
        r.insert(0, id.to_string());
        r.truncate(RECENTS);
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn filescape_defaults_come_before_the_systems() {
        
        let h = |id: &str, default: bool| Handler { id: id.into(), name: id.into(), app: PathBuf::new(), default };
        let found = vec![h("vim", true), h("kate", false)];
        let txt = Type::Ext("txt".into());
        let md = Type::Ext("md".into());
        let mut prefs = OpenPrefs::default();
        assert_eq!(prefs.default_for(&txt, Some(&found)).map(|(h, s)| (h.id, s)), Some(("vim".into(), Source::System)));
        assert_eq!(prefs.default_for(&txt, None), None);
        prefs.kind_defaults.insert(kind_key(crate::categories::Category::Documents), h("writer", false));
        assert_eq!(prefs.default_for(&md, Some(&found)).map(|(h, s)| (h.id, s)), Some(("writer".into(), Source::Kind)));
        prefs.defaults.insert("txt".into(), h("kate", false));
        assert_eq!(prefs.default_for(&txt, None).map(|(h, s)| (h.id, s)), Some(("kate".into(), Source::Type)));
        // The list marks Filescape's choice, adds apps it wouldn't have, and names the system's.
        prefs.remember(&txt, &h("zed", false));
        let (list, system) = prefs.listing(&txt, &found);
        let marked: Vec<_> = list.iter().map(|h| (h.id.as_str(), h.default)).collect();
        assert_eq!(marked, [("vim", false), ("kate", true), ("zed", false)]);
        assert_eq!(system.as_deref(), Some("vim"));
        let (list, _) = prefs.listing(&md, &found);
        assert!(list.iter().any(|h| h.id == "writer" && h.default));
        assert_eq!(Type::Folder.category(), None);
        assert_eq!(kind_name(crate::categories::Category::Images), "images");
    }

}
