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

fn lower(s: &str) -> String {
    s.trim().to_lowercase()
}

/// Finds maps in the library: by the folder and file they were in, else their beatmap ID, else
/// their set ID and difficulty name (so a map that was moved or updated is still found).
pub struct Finder {
    by_file: HashMap<(String, String), usize>,
    by_id: HashMap<u32, usize>,
    by_version: HashMap<(u32, String), usize>,
}

impl Finder {
    pub fn new(library: &[Entry]) -> Finder {
        let mut f = Finder {
            by_file: HashMap::new(),
            by_id: HashMap::new(),
            by_version: HashMap::new(),
        };
        for (i, e) in library.iter().enumerate() {
            let folder = e.path.parent().and_then(|p| p.file_name());
            if let (Some(folder), Some(file)) = (folder, e.path.file_name()) {
                f.by_file
                    .entry((
                        lower(&folder.to_string_lossy()),
                        lower(&file.to_string_lossy()),
                    ))
                    .or_insert(i);
            }
            if let Some(id) = e.meta.beatmap_id.filter(|&id| id > 0) {
                f.by_id.entry(id).or_insert(i);
            }
            if let Some(set) = e.set_id() {
                f.by_version
                    .entry((set, lower(&e.meta.version)))
                    .or_insert(i);
            }
        }
        f
    }

    /// Where map `m` is in the library, if it's there.
    pub fn at(&self, m: &MapRef) -> Option<usize> {
        self.found(m).map(|(i, _)| i)
    }

    /// Where map `m` is in the library, and whether that's the folder and file it was in (else
    /// it was moved or updated, and found by its IDs).
    pub fn found(&self, m: &MapRef) -> Option<(usize, bool)> {
        let by_file = (!m.file.is_empty())
            .then(|| self.by_file.get(&(lower(&m.folder), lower(&m.file))))
            .flatten();
        if let Some(&i) = by_file {
            return Some((i, true));
        }
        m.beatmap_id
            .and_then(|id| self.by_id.get(&id))
            .or_else(|| {
                m.set_id
                    .and_then(|set| self.by_version.get(&(set, lower(&m.version))))
            })
            .map(|&i| (i, false))
    }
}

/// Finds each collection's maps in `library` (see `Finder`).
pub fn find(collections: &[Collection], library: &[Entry]) -> Vec<Found> {
    let finder = Finder::new(library);
    collections
        .iter()
        .map(|c| {
            let mut found = Found {
                entries: HashSet::new(),
                missing: 0,
            };
            for m in &c.maps {
                match finder.at(m) {
                    Some(i) => {
                        found.entries.insert(i);
                    }
                    None => found.missing += 1,
                }
            }
            found
        })
        .collect()
}

impl MapRef {
    /// Library entry `e` as a collection keeps it (its file read for the MD5); None if the file
    /// can't be read.
    pub fn of(e: &Entry) -> Option<MapRef> {
        let name = |p: Option<&std::ffi::OsStr>| {
            p.map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        Some(MapRef {
            md5: crate::download::md5_hex(&std::fs::read(&e.path).ok()?),
            beatmap_id: e.meta.beatmap_id.filter(|&id| id > 0),
            set_id: e.set_id(),
            version: e.meta.version.clone(),
            folder: name(e.path.parent().and_then(|p| p.file_name())),
            file: name(e.path.file_name()),
        })
    }
}

/// Points collection `c`'s maps that were moved or updated at where they are now (their new
/// folder, file and MD5), so they're found by their file again and the MD5 is the map's now.
/// Whether any changed.
pub fn repoint(c: &mut Collection, finder: &Finder, library: &[Entry]) -> bool {
    let mut changed = false;
    for m in &mut c.maps {
        if let Some((i, false)) = finder.found(m)
            && let Some(now) = MapRef::of(&library[i])
        {
            *m = now;
            changed = true;
        }
    }
    // Two old versions of a map now the same one: kept once.
    if changed {
        let mut seen = HashSet::new();
        c.maps.retain(|m| seen.insert(m.md5.clone()));
    }
    changed
}

/// The sets of collection `c`'s maps that aren't in the library: (set ID, the folder they were
/// in), once each.
pub fn missing_sets(c: &Collection, finder: &Finder) -> Vec<(u32, String)> {
    let mut seen = HashSet::new();
    c.maps
        .iter()
        .filter(|m| finder.at(m).is_none())
        .filter_map(|m| Some((m.set_id?, m.folder.clone())))
        .filter(|(set, _)| seen.insert(*set))
        .collect()
}

/// The longest name a collection can have, in characters.
pub const MAX_NAME: usize = 60;

/// Why a name wasn't taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadName {
    Empty,
    /// One of the player's own is called that already (case aside).
    Taken,
}

/// How many of `all` are the player's own (they come first).
pub fn own_len(all: &[Collection]) -> usize {
    all.iter().take_while(|c| c.source == Source::Local).count()
}

/// Editing the player's own collections, which come first in the list (`Store::collections`'s
/// order); osu!'s after them are only read.
pub struct Own<'a>(pub &'a mut Vec<Collection>);

impl Own<'_> {
    /// How many of the player's own there are (they're `0..len`).
    pub fn len(&self) -> usize {
        own_len(self.0)
    }

    fn check(&self, name: &str, but: Option<usize>) -> Result<String, BadName> {
        let name: String = name.trim().chars().take(MAX_NAME).collect();
        if name.is_empty() {
            return Err(BadName::Empty);
        }
        let taken = self.0[..self.len()]
            .iter()
            .enumerate()
            .any(|(k, c)| Some(k) != but && c.name.to_lowercase() == name.to_lowercase());
        if taken {
            return Err(BadName::Taken);
        }
        Ok(name)
    }

    /// A new collection of the player's own holding `maps`, after the others; where it is.
    pub fn add(&mut self, name: &str, maps: Vec<MapRef>) -> Result<usize, BadName> {
        let name = self.check(name, None)?;
        let at = self.len();
        self.0.insert(
            at,
            Collection {
                name,
                source: Source::Local,
                maps,
            },
        );
        Ok(at)
    }

    pub fn rename(&mut self, k: usize, name: &str) -> Result<(), BadName> {
        let name = self.check(name, Some(k))?;
        if k < self.len() {
            self.0[k].name = name;
        }
        Ok(())
    }

    pub fn delete(&mut self, k: usize) {
        if k < self.len() {
            self.0.remove(k);
        }
    }

    /// Moves collection `k` one place up (or down) among the player's own; where it is now.
    pub fn shift(&mut self, k: usize, up: bool) -> usize {
        let to = if up {
            k.checked_sub(1)
        } else {
            Some(k + 1).filter(|&to| to < self.len())
        };
        match to.filter(|_| k < self.len()) {
            Some(to) => {
                self.0.swap(k, to);
                to
            }
            None => k,
        }
    }

    /// osu!'s collection `k` copied into the player's own (named like it, with a number after
    /// it if they have one of that name already); where the copy is.
    pub fn copy(&mut self, k: usize) -> Option<usize> {
        let c = self.0.get(k).filter(|c| c.source == Source::Osu)?.clone();
        let mut name = c.name.clone();
        let mut n = 1;
        while self.check(&name, None) == Err(BadName::Taken) {
            n += 1;
            name = format!("{} {n}", c.name);
        }
        self.add(&name, c.maps).ok()
    }

    /// Puts `maps` (library indices, each with how it's kept) in collection `k`, or takes them
    /// out if they're all in it already; whether they went in.
    pub fn toggle(&mut self, k: usize, maps: &[(usize, MapRef)], finder: &Finder) -> bool {
        if k >= self.len() {
            return false;
        }
        let c = &mut self.0[k];
        let here: HashSet<usize> = c.maps.iter().filter_map(|m| finder.at(m)).collect();
        if maps.iter().all(|(i, _)| here.contains(i)) {
            let out: HashSet<usize> = maps.iter().map(|(i, _)| *i).collect();
            c.maps
                .retain(|m| finder.at(m).is_none_or(|i| !out.contains(&i)));
            false
        } else {
            for (i, m) in maps {
                if !here.contains(i) && !c.maps.iter().any(|old| old.md5 == m.md5) {
                    c.maps.push(m.clone());
                }
            }
            true
        }
    }
}

/// The `collection.db` version written when there's none to keep (osu! stable's of 2025-01-08,
/// which reads the same format).
const NEW_DB_VERSION: i32 = 20250108;

/// What exporting did: how many collections went in new, and how many of osu!'s gained maps.
#[derive(Debug, PartialEq, Eq)]
pub struct Exported {
    pub added: usize,
    pub merged: usize,
}

/// Writes the player's own collections (`own`, each map's MD5 now by `md5_of`) into osu!'s
/// `collection.db` at `path`. osu!'s own collections stay as they are: one with the same name
/// as one of the player's gains its maps, the rest come after. A `collection.db` that can't be
/// read is left alone. Before it's replaced, the file is copied to `collection.db.bak`; the new
/// one is written beside it, read back, and only then put in its place.
pub fn export_to_osu(
    path: &std::path::Path,
    own: &[Collection],
    md5_of: impl Fn(&MapRef) -> String,
) -> Result<Exported, String> {
    let shown = |e: std::io::Error| format!("{}: {e}", path.display());
    let (version, mut all) = match std::fs::read(path) {
        Ok(b) => {
            let version = crate::osudb::collections_version(&b);
            let all = crate::osudb::parse_collections(&b);
            match (version, all) {
                (Some(v), Some(all)) => (v, all),
                _ => {
                    return Err(
                        "osu!'s collection.db couldn't be read, so it's left as it is".into(),
                    );
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (NEW_DB_VERSION, Vec::new()),
        Err(e) => return Err(shown(e)),
    };
    let mut done = Exported {
        added: 0,
        merged: 0,
    };
    for c in own.iter().filter(|c| c.source == Source::Local) {
        let md5s: Vec<String> = c
            .maps
            .iter()
            .map(&md5_of)
            .filter(|m| !m.is_empty())
            .collect();
        match all.iter_mut().find(|(name, _)| *name == c.name) {
            Some((_, theirs)) => {
                let before = theirs.len();
                for m in md5s {
                    if !theirs.contains(&m) {
                        theirs.push(m);
                    }
                }
                if theirs.len() > before {
                    done.merged += 1;
                }
            }
            None => {
                all.push((c.name.clone(), md5s));
                done.added += 1;
            }
        }
    }
    let bytes = crate::osudb::write_collections(version, &all);
    if path.exists() {
        std::fs::copy(path, path.with_extension("db.bak")).map_err(shown)?;
    }
    let fresh = path.with_extension("db.taiko-rs");
    std::fs::write(&fresh, &bytes).map_err(shown)?;
    let back = std::fs::read(&fresh).map_err(shown)?;
    if crate::osudb::parse_collections(&back).as_ref() != Some(&all) {
        let _ = std::fs::remove_file(&fresh);
        return Err(
            "the new collection.db didn't read back the same; osu!'s is left as it is".into(),
        );
    }
    std::fs::rename(&fresh, path).map_err(shown)?;
    Ok(done)
}

/// Whether osu! stable is running (it writes `collection.db` again when it closes, so an export
/// then would be lost).
pub fn osu_running() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console window flashing up.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq osu!.exe", "/NH"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("osu!.exe"))
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        // Through Wine: a process whose command line names osu!.exe.
        std::fs::read_dir("/proc").is_ok_and(|dir| {
            dir.flatten().any(|p| {
                std::fs::read(p.path().join("cmdline"))
                    .is_ok_and(|c| c.windows(8).any(|w| w.eq_ignore_ascii_case(b"osu!.exe")))
            })
        })
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("pgrep")
            .args(["-f", "osu!.exe"])
            .output()
            .is_ok_and(|o| o.status.success())
    }
    #[cfg(not(any(
        windows,
        target_os = "linux",
        target_os = "android",
        target_os = "macos"
    )))]
    {
        false
    }
}
