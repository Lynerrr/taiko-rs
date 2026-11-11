-- taiko.db, version 2: each beatmap set's status on osu! as the mirrors told it (was
-- set-status.tsv in the cache folder), so sorting and searching by status work at the next
-- start without asking again (setstatus.rs).

CREATE TABLE set_status (
    set_id INTEGER PRIMARY KEY,
    status TEXT NOT NULL,           -- "ranked", "loved", ..., or "missing" (the mirrors don't know it)
    checked_at INTEGER NOT NULL     -- when it was asked: Unix seconds
);
