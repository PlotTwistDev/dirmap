//! Every Filescape version, for the download lists on account.filescape.app.
//!
//! A perpetual licence unlocks every version released before its maintenance ends, for good, so a
//! customer whose maintenance has ended needs the newest version it unlocks, not just the latest.
//! Each version's downloads live in R2 under `<version>/<os>` (SETUP.md step 9), next to
//! `latest/<os>`; a release can also name its own links (only on filescape.app).

use crate::db::{self, Stmt};
use crate::{Ctx, Db, Net};
use filescape_licence::Ts;
use filescape_licence::release::{ReleaseManifest, is_ours};

#[derive(Debug, Clone)]
pub struct Entry {
    pub m: ReleaseManifest,
    pub listed: bool,
}

/// One download: a label and a link.
pub type Link = (&'static str, String);

impl Entry {
    /// The downloads, by platform: the release's own link where it gave one, else its R2 copy.
    pub fn links(&self) -> Vec<Link> {
        let v = &self.m.version;
        let own = |u: &Option<String>| u.as_deref().filter(|u| is_ours(u)).map(String::from);
        let r2 = |os: &str| format!("https://downloads.filescape.app/{v}/{os}");
        let mut out = vec![("macOS", own(&self.m.downloads.macos).unwrap_or_else(|| r2("macos"))), ("Windows", own(&self.m.downloads.windows).unwrap_or_else(|| r2("windows")))];
        match own(&self.m.downloads.linux) {
            Some(u) => out.push(("Linux", u)),
            None => out.extend([("Linux AppImage", r2("linux-appimage")), (".deb", r2("linux-deb")), (".rpm", r2("linux-rpm")), (".tar.gz", r2("linux"))]),
        }
        out
    }
}

/// The statement that adds `m` to the catalogue, or updates it (keeping whether it's listed).
pub fn upsert_stmt(m: &ReleaseManifest, now: Ts, actor: &str) -> Stmt {
    let mut m = m.clone();
    m.signed_at = 0;
    (
        "INSERT INTO releases (version, released_at, manifest, listed, added_at, actor) VALUES (?1, ?2, ?3, 1, ?4, ?5) \
         ON CONFLICT(version) DO UPDATE SET released_at = excluded.released_at, manifest = excluded.manifest, actor = excluded.actor",
        vec![(&m.version).into(), m.released_at.into(), serde_json::to_string(&m).unwrap().into(), now.into(), actor.into()],
    )
}

impl<D: Db, N: Net> Ctx<'_, D, N> {
    /// Newest first. `all`: hidden ones too (admin).
    pub async fn catalogue(&self, all: bool) -> Result<Vec<Entry>, String> {
        let sql = if all { "SELECT manifest, listed FROM releases ORDER BY released_at DESC" } else { "SELECT manifest, listed FROM releases WHERE listed = 1 ORDER BY released_at DESC" };
        let rows = self.db.all(sql, vec![]).await?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                let m = serde_json::from_str::<ReleaseManifest>(&db::text(r, "manifest")?).ok()?;
                Some(Entry { m, listed: db::int(r, "listed").unwrap_or(1) != 0 })
            })
            .collect())
    }

    /// Adds an older version to the download lists without announcing it.
    pub async fn add_release(&self, m: &ReleaseManifest, actor: &str) -> Result<(), String> {
        self.db
            .batch(vec![
                upsert_stmt(m, self.now, actor),
                ("INSERT INTO events (licence_id, at, what, detail, actor) VALUES (NULL, ?1, 'setting', ?2, ?3)", vec![self.now.into(), format!("release listed: {}", m.version).into(), actor.into()]),
            ])
            .await
            .map(|_| ())
    }

    /// Shows or hides a version in the download lists. Returns whether it exists.
    pub async fn set_listed(&self, version: &str, listed: bool, actor: &str) -> Result<bool, String> {
        let changes = self
            .db
            .batch(vec![
                ("UPDATE releases SET listed = ?1 WHERE version = ?2", vec![(listed as i64).into(), version.into()]),
                (
                    "INSERT INTO events (licence_id, at, what, detail, actor) SELECT NULL, ?1, 'setting', ?2, ?3 WHERE changes() = 1",
                    vec![self.now.into(), format!("release {version}: {}", if listed { "listed" } else { "hidden" }).into(), actor.into()],
                ),
            ])
            .await?;
        Ok(changes.first() == Some(&1))
    }
}

/// The newest listed version released by `until` (what a licence whose maintenance ends then
/// unlocks), if any.
pub fn newest_unlocked(entries: &[Entry], until: Ts) -> Option<&Entry> {
    entries.iter().filter(|e| e.listed && e.m.released_at <= until).max_by_key(|e| e.m.released_at)
}
