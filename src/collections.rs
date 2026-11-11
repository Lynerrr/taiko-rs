//! Collections: named lists of maps, kept in `taiko.db` (`store`). Each is the player's own or a
//! mirror of one in osu!'s `collection.db`, brought in again whenever that file changes (osu!
//! owns those: a change made in osu! shows up here).
//!
//! A collection holds maps by the `.osu`'s MD5, as osu! does, plus what finds a map again when
//! the library changes: the folder and file it was in, its beatmap ID, and its set ID with the
//! difficulty's name. A map that isn't in the library (deleted, or not downloaded here) stays in
//! the collection.

use std::collections::{HashMap, HashSet};

use crate::library::Entry;
use crate::osudb::OsuDb;
use crate::store::Store;

/// Where a collection comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// Made in taiko.rs.
    Local,
    /// osu!'s `collection.db`.
    Osu,
}

impl Source {
    pub fn sql(self) -> &'static str {
        match self {
            Source::Local => "local",
            Source::Osu => "osu",
        }
    }

    pub fn from_sql(s: &str) -> Option<Source> {
        match s {
            "local" => Some(Source::Local),
            "osu" => Some(Source::Osu),
            _ => None,
        }
    }
}

/// A map in a collection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MapRef {
    /// The `.osu`'s MD5, lowercase hex.
    pub md5: String,
    pub beatmap_id: Option<u32>,
    pub set_id: Option<u32>,
    /// [Version]: the difficulty's name.
    pub version: String,
    /// The map's folder's name (not the whole path) and its `.osu` file's name.
    pub folder: String,
    pub file: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Collection {
    pub name: String,
    pub source: Source,
    pub maps: Vec<MapRef>,
}

/// The meta key keeping which `collection.db` the osu! collections were last brought in from.
pub const OSU_STAMP: &str = "osu_collections_from";

/// osu!'s collections as `collection.db` lists them, each map filled in from `osu!.db` where it
/// knows the MD5.
pub fn from_osu(db: &OsuDb) -> Vec<Collection> {
    db.collections
        .iter()
        .map(|(name, maps)| Collection {
            name: name.clone(),
            source: Source::Osu,
            maps: maps
                .iter()
                .map(|md5| {
                    let md5 = md5.to_ascii_lowercase();
                    match db.by_md5.get(&md5) {
                        Some(m) => MapRef {
                            beatmap_id: m.beatmap_id,
                            set_id: m.set_id,
                            version: m.version.clone(),
                            folder: m.folder.clone(),
                            file: m.file.clone(),
                            md5,
                        },
                        None => MapRef {
                            md5,
                            ..MapRef::default()
                        },
                    }
                })
                .collect(),
        })
        .collect()
}

/// Brings osu!'s collections into `store` in place of the ones brought in before, unless
/// they're from the same `collection.db` as last time and it hasn't changed. Without a
/// `collection.db` the last ones are kept (osu! may be on a drive that isn't there now).
/// Whether they were brought in.
pub fn mirror_osu(store: &mut Store, db: &OsuDb, now: u64) -> Result<bool, String> {
    let Some(stamp) = &db.collections_stamp else {
        return Ok(false);
    };
    if store.meta(OSU_STAMP)?.as_deref() == Some(stamp.as_str()) {
        return Ok(false);
    }
    let collections = from_osu(db);
    store.transaction(|s| {
        s.write_collections(Source::Osu, &collections, now)?;
        s.set_meta(OSU_STAMP, stamp)
    })?;
    Ok(true)
}

/// Where each collection's maps are in `library` (indices), and how many of them aren't there.
#[derive(Debug, PartialEq, Eq)]
pub struct Found {
    pub entries: HashSet<usize>,
    pub missing: usize,
}

/// Finds each collection's maps in `library`: by the folder and file they were in, else their
/// beatmap ID, else their set ID and difficulty name (so a map that was moved or updated is
/// still found).
pub fn find(collections: &[Collection], library: &[Entry]) -> Vec<Found> {
    let lower = |s: &str| s.trim().to_lowercase();
    let mut by_file = HashMap::new();
    let mut by_id = HashMap::new();
    let mut by_version = HashMap::new();
    for (i, e) in library.iter().enumerate() {
        let folder = e.path.parent().and_then(|p| p.file_name());
        if let (Some(folder), Some(file)) = (folder, e.path.file_name()) {
            by_file
                .entry((
                    lower(&folder.to_string_lossy()),
                    lower(&file.to_string_lossy()),
                ))
                .or_insert(i);
        }
        if let Some(id) = e.meta.beatmap_id.filter(|&id| id > 0) {
            by_id.entry(id).or_insert(i);
        }
        if let Some(set) = e.set_id() {
            by_version.entry((set, lower(&e.meta.version))).or_insert(i);
        }
    }
    collections
        .iter()
        .map(|c| {
            let mut found = Found {
                entries: HashSet::new(),
                missing: 0,
            };
            for m in &c.maps {
                let at = (!m.file.is_empty())
                    .then(|| by_file.get(&(lower(&m.folder), lower(&m.file))))
                    .flatten()
                    .or_else(|| m.beatmap_id.and_then(|id| by_id.get(&id)))
                    .or_else(|| {
                        m.set_id
                            .and_then(|set| by_version.get(&(set, lower(&m.version))))
                    });
                match at {
                    Some(&i) => {
                        found.entries.insert(i);
                    }
                    None => found.missing += 1,
                }
            }
            found
        })
        .collect()
}
