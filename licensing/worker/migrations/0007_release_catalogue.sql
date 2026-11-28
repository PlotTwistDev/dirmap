-- Release catalogue (28 Nov 2026): every version, so customers can download the newest version
-- their licence unlocks, and older ones. Announcing a release on the admin Releases page adds it
-- here too; older versions can be added without announcing them.
CREATE TABLE releases (
  version TEXT PRIMARY KEY,
  released_at INTEGER NOT NULL,         -- the build's FILESCAPE_RELEASE_DATE, as in the manifest
  manifest TEXT NOT NULL,               -- JSON of filescape_licence::release::ReleaseManifest
  listed INTEGER NOT NULL DEFAULT 1,    -- 0: hidden from the download lists (e.g. a pulled build)
  added_at INTEGER NOT NULL,
  actor TEXT NOT NULL
);
CREATE INDEX releases_date ON releases(released_at);

-- The release announced now, if any, starts the list.
INSERT INTO releases (version, released_at, manifest, listed, added_at, actor)
  SELECT json_extract(value, '$.version'), json_extract(value, '$.released_at'), value, 1, json_extract(value, '$.released_at'), 'migration'
  FROM settings WHERE key = 'latest_release' AND json_extract(value, '$.version') IS NOT NULL;
