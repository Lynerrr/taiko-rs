//! Collections in `taiko.db` (`collections.rs`): osu!'s brought in from `collection.db` when it
//! changed and kept when it's gone, the player's own left alone, and each collection's maps
//! found in the library again after they moved or were updated.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::beatmap::Meta;
use crate::collections::{self, Collection, Found, MapRef, Source};
use crate::library::Entry;
use crate::osudb::{DbMap, OsuDb};
use crate::store::Store;

fn temp_db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "taiko-rs-collections-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("taiko.db")
}

fn db_map(folder: &str, file: &str, version: &str, ids: (u32, u32)) -> DbMap {
    DbMap {
        folder: folder.into(),
        file: file.into(),
        last_played: 0,
        status: 4,
        version: version.into(),
        beatmap_id: (ids.0 > 0).then_some(ids.0),
        set_id: (ids.1 > 0).then_some(ids.1),
    }
}

/// osu!'s databases: four maps it knows, collections with one it doesn't ("ffff").
fn osu_db(stamp: &str, collections: &[(&str, &[&str])]) -> OsuDb {
    let by_md5: HashMap<String, DbMap> = [
        (
            "aaaa",
            db_map("1 A - Song", "A - Song (m) [Oni].osu", "Oni", (11, 1)),
        ),
        (
            "bbbb",
            db_map("1 A - Song", "A - Song (m) [Muzu].osu", "Muzu", (12, 1)),
        ),
        (
            "cccc",
            db_map("2 B - Tune", "B - Tune (m) [Inner].osu", "Inner", (0, 2)),
        ),
        (
            "dddd",
            db_map("Home made", "C - Mine (me) [Edit].osu", "Edit", (0, 0)),
        ),
    ]
    .into_iter()
    .map(|(md5, m)| (md5.to_string(), m))
    .collect();
    OsuDb {
        by_md5,
        collections: collections
            .iter()
            .map(|(name, maps)| {
                (
                    name.to_string(),
                    maps.iter().map(|m| m.to_string()).collect(),
                )
            })
            .collect(),
        collections_stamp: Some(stamp.to_string()),
    }
}

fn names(collections: &[Collection]) -> Vec<(&str, Source, usize)> {
    collections
        .iter()
        .map(|c| (c.name.as_str(), c.source, c.maps.len()))
        .collect()
}

#[test]
fn osus_collections_come_in_with_what_finds_their_maps_and_read_back_after_reopening() {
    let path = temp_db("in");
    let db = osu_db(
        "osu/collection.db 100 1700000000",
        &[("practice", &["aaaa", "CCCC", "ffff"]), ("pool", &["dddd"])],
    );
    let mut store = Store::open(&path).unwrap();
    assert!(collections::mirror_osu(&mut store, &db, 1_700_000_100).unwrap());
    drop(store);

    let store = Store::open(&path).unwrap();
    let got = store.collections().unwrap();
    assert_eq!(got, collections::from_osu(&db));
    assert_eq!(
        got[0].maps,
        vec![
            MapRef {
                md5: "aaaa".into(),
                beatmap_id: Some(11),
                set_id: Some(1),
                version: "Oni".into(),
                folder: "1 A - Song".into(),
                file: "A - Song (m) [Oni].osu".into(),
            },
            // osu! writes MD5s in lowercase; one that isn't is still found in osu!.db.
            MapRef {
                md5: "cccc".into(),
                beatmap_id: None,
                set_id: Some(2),
                version: "Inner".into(),
                folder: "2 B - Tune".into(),
                file: "B - Tune (m) [Inner].osu".into(),
            },
            // Not in osu!.db: kept by its MD5 alone.
            MapRef {
                md5: "ffff".into(),
                ..MapRef::default()
            },
        ]
    );
    assert_eq!(
        names(&got),
        vec![("practice", Source::Osu, 3), ("pool", Source::Osu, 1)]
    );
}

#[test]
fn osus_collections_come_in_again_only_when_collection_db_changed_and_stay_when_its_gone() {
    let path = temp_db("again");
    let mut store = Store::open(&path).unwrap();
    let first = osu_db(
        "osu/collection.db 100 1",
        &[("a", &["aaaa"]), ("b", &["bbbb"])],
    );
    assert!(collections::mirror_osu(&mut store, &first, 1).unwrap());

    // The same file as last time isn't read into the database again.
    let same = osu_db("osu/collection.db 100 1", &[("other", &["cccc"])]);
    assert!(!collections::mirror_osu(&mut store, &same, 2).unwrap());
    assert_eq!(
        names(&store.collections().unwrap()),
        vec![("a", Source::Osu, 1), ("b", Source::Osu, 1)]
    );

    // Changed in osu!: "a" lost its map and gained two, "b" was deleted, "c" made.
    let changed = osu_db(
        "osu/collection.db 140 2",
        &[("c", &["dddd"]), ("a", &["bbbb", "cccc"])],
    );
    assert!(collections::mirror_osu(&mut store, &changed, 3).unwrap());
    let got = store.collections().unwrap();
    assert_eq!(got, collections::from_osu(&changed));

    // No collection.db now (osu! uninstalled, or on a drive that isn't there): kept as it was.
    let gone = OsuDb {
        collections_stamp: None,
        ..osu_db("", &[])
    };
    assert!(!collections::mirror_osu(&mut store, &gone, 4).unwrap());
    drop(store);
    assert_eq!(Store::open(&path).unwrap().collections().unwrap(), got);
}

#[test]
fn the_players_own_collections_are_listed_first_and_left_alone_by_osus() {
    let path = temp_db("own");
    let mut store = Store::open(&path).unwrap();
    let mine = vec![
        Collection {
            name: "warm up".into(),
            source: Source::Local,
            maps: vec![MapRef {
                md5: "bbbb".into(),
                beatmap_id: Some(12),
                ..MapRef::default()
            }],
        },
        // The same name as one of osu!'s: they're told apart by where they're from.
        Collection {
            name: "practice".into(),
            source: Source::Local,
            maps: Vec::new(),
        },
    ];
    store
        .transaction(|s| s.write_collections(Source::Local, &mine, 1))
        .unwrap();
    let db = osu_db("x 1 1", &[("practice", &["aaaa"])]);
    collections::mirror_osu(&mut store, &db, 2).unwrap();
    let db = osu_db("x 1 2", &[("drills", &["cccc"])]);
    collections::mirror_osu(&mut store, &db, 3).unwrap();
    drop(store);
    let got = Store::open(&path).unwrap().collections().unwrap();
    assert_eq!(got[..2], mine[..]);
    assert_eq!(
        names(&got),
        vec![
            ("warm up", Source::Local, 1),
            ("practice", Source::Local, 0),
            ("drills", Source::Osu, 1),
        ]
    );
}

#[test]
fn a_collection_db_naming_a_collection_or_map_twice_comes_in_as_one() {
    let mut store = Store::in_memory().unwrap();
    let db = osu_db(
        "x 1 1",
        &[
            ("dup", &["aaaa", "aaaa"]),
            ("dup", &["bbbb"]),
            ("one", &["cccc"]),
        ],
    );
    assert!(collections::mirror_osu(&mut store, &db, 1).unwrap());
    let got = store.collections().unwrap();
    assert_eq!(
        names(&got),
        vec![("dup", Source::Osu, 2), ("one", Source::Osu, 1)]
    );
    let md5s: Vec<_> = got[0].maps.iter().map(|m| m.md5.as_str()).collect();
    assert_eq!(md5s, ["aaaa", "bbbb"]);
}

fn entry(path: &str, version: &str, beatmap_id: Option<u32>, set_id: Option<u32>) -> Entry {
    let meta = Meta {
        title: "T".into(),
        artist: "A".into(),
        version: version.into(),
        beatmap_id,
        set_id,
        ..Meta::default()
    };
    Entry::from_parts(PathBuf::from(path), meta, 1000, 1_700_000_000)
}

#[test]
fn maps_are_found_where_they_were_else_by_id_else_by_set_and_difficulty() {
    let songs = Path::new("/songs");
    let at = |p: &str| songs.join(p).display().to_string();
    let library = vec![
        // Where osu! has it (case aside, as on Windows).
        entry(
            &at("1 a - song/A - Song (m) [ONI].osu"),
            "Oni",
            Some(11),
            Some(1),
        ),
        // Moved to another folder: found by its beatmap ID.
        entry(&at("elsewhere/renamed.osu"), "Muzu", Some(12), Some(1)),
        // Updated (a new file name) with no beatmap ID: found by its set and difficulty.
        entry(
            &at("2 B - Tune/B - Tune (m) [Inner] v2.osu"),
            " inner ",
            None,
            Some(2),
        ),
        // No IDs and moved: not found.
        entry(&at("moved/C - Mine (me) [Edit].osu"), "Edit", None, None),
        // In no collection.
        entry(&at("9 Z/z.osu"), "Z", Some(99), Some(9)),
    ];
    let db = osu_db(
        "x",
        &[
            ("all", &["aaaa", "bbbb", "cccc", "dddd", "ffff"]),
            ("none here", &["ffff"]),
        ],
    );
    let found = collections::find(&collections::from_osu(&db), &library);
    assert_eq!(
        found,
        vec![
            Found {
                entries: HashSet::from([0, 1, 2]),
                missing: 2,
            },
            Found {
                entries: HashSet::new(),
                missing: 1,
            },
        ]
    );
}

#[test]
fn a_database_from_before_collections_gains_them_and_keeps_its_data() {
    let path = temp_db("upgrade");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(include_str!("../store/1.sql")).unwrap();
        conn.execute_batch(include_str!("../store/2.sql")).unwrap();
        conn.pragma_update(None, "user_version", 2).unwrap();
        conn.execute(
            "INSERT INTO set_status (set_id, status, checked_at) VALUES (7, 'ranked', 5)",
            [],
        )
        .unwrap();
    }
    let mut store = Store::open(&path).unwrap();
    assert_eq!(store.version().unwrap(), 3);
    assert_eq!(
        store.set_statuses().unwrap(),
        HashMap::from([(7, ("ranked".to_string(), 5))])
    );
    assert!(store.collections().unwrap().is_empty());
    let db = osu_db("x 1 1", &[("a", &["aaaa"])]);
    assert!(collections::mirror_osu(&mut store, &db, 1).unwrap());
    assert_eq!(
        names(&store.collections().unwrap()),
        vec![("a", Source::Osu, 1)]
    );
}
