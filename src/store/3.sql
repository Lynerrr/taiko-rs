-- taiko.db, version 3: collections, named lists of maps (src/collections.rs). A player's own
-- ("local") or a mirror of osu!'s collection.db ("osu"), which osu! owns: it's replaced
-- whenever that file changes.

CREATE TABLE collections (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'local',   -- 'local' (made in taiko.rs) or 'osu' (collection.db)
    position INTEGER NOT NULL,              -- order in the list, within its source
    created_at INTEGER NOT NULL,            -- Unix seconds
    changed_at INTEGER NOT NULL,
    UNIQUE (source, name)
);

-- The maps in each collection: by the .osu's MD5, as osu! keeps them, and with what finds the
-- map again after it moved or was updated (a new MD5): its IDs, its difficulty name, and the
-- folder and file it was in. A map that isn't in the library stays listed.
CREATE TABLE collection_maps (
    collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    map_md5 TEXT NOT NULL,
    beatmap_id INTEGER,
    set_id INTEGER,
    version TEXT NOT NULL DEFAULT '',       -- [Version], the difficulty's name
    folder TEXT NOT NULL DEFAULT '',        -- the map's folder's name (not the whole path)
    file TEXT NOT NULL DEFAULT '',          -- the .osu file's name
    position INTEGER NOT NULL,              -- order within the collection
    added_at INTEGER NOT NULL,
    PRIMARY KEY (collection_id, map_md5)
);
CREATE INDEX collection_maps_md5 ON collection_maps(map_md5);
