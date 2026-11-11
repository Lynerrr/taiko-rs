-- taiko.db, version 1: the player's own data that used to be kept in text files next to
-- config.txt (see src/store.rs). Replays stay files; `scores.replay` is the file's name in the
-- replays folder.

-- Small facts about the database itself, such as when the old files were brought in.
CREATE TABLE meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- The local scores (was replays/scores.tsv), one row per finished play.
CREATE TABLE scores (
    id INTEGER PRIMARY KEY,
    map_hash INTEGER NOT NULL,      -- the .osu's content hash, as a u64's bits
    date INTEGER NOT NULL,          -- Unix seconds
    score INTEGER NOT NULL,
    accuracy REAL NOT NULL,
    max_combo INTEGER NOT NULL,
    miss INTEGER NOT NULL,
    rate REAL NOT NULL,
    player TEXT NOT NULL,
    replay TEXT NOT NULL,           -- the replay's file name in the replays folder
    mods TEXT NOT NULL,             -- "HDHR", "" without
    pp REAL                         -- NULL: not worked out yet
);
CREATE INDEX scores_map ON scores(map_hash, score DESC);

-- When each map was last played in taiko.rs (was plays.tsv), by the .osu's path.
CREATE TABLE plays (
    path TEXT PRIMARY KEY,
    last_played INTEGER NOT NULL
);

-- Maps hidden from song select (was hidden.tsv): whole songs by folder, difficulties by file.
CREATE TABLE hidden (
    kind TEXT NOT NULL CHECK (kind IN ('song', 'map')),
    path TEXT NOT NULL,
    PRIMARY KEY (kind, path)
);

-- Each map's own offset on top of the global one (was map_offsets_osu.tsv), by content hash.
CREATE TABLE map_offsets (
    map_hash INTEGER PRIMARY KEY,
    offset_ms REAL NOT NULL
);

-- The map folders whose own skin is turned off (was map_skins_off.txt).
CREATE TABLE map_skins_off (
    folder TEXT PRIMARY KEY
);

-- The music player's favourite songs (was favourite-songs.txt), as "folder/audio file".
CREATE TABLE favourites (
    song TEXT PRIMARY KEY
);
