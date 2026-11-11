//! The player's own data in one SQLite database, `taiko.db` in the data folder: local scores,
//! when each map was last played, hidden maps, each map's own offset, the maps whose skin is
//! off and the music player's favourites. These used to be text files of their own next to
//! config.txt; `config.txt` itself stays a text file (players edit it by hand) and so do the
//! replays (the database keeps their file names).
//!
//! This is the only module that writes SQL. Each kind of data is read whole and written either
//! a row at a time (a new score, a play) or whole (the rest, as their files were), so the code
//! around it keeps its in-memory sets and maps as before.
//!
//! The schema's versions are the `store/N.sql` files, applied in order and counted in SQLite's
//! `user_version`. A database from a newer build (a higher version than this build knows) is
//! not touched: `open` refuses it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};

use crate::scores::ScoreRow;

pub mod import;

/// The database's file name in the data folder.
pub const FILE: &str = "taiko.db";

/// The schema, one script per version; version N is `MIGRATIONS[N - 1]`.
const MIGRATIONS: &[&str] = &[include_str!("store/1.sql")];

/// The schema version this build writes.
pub const VERSION: u32 = MIGRATIONS.len() as u32;

pub struct Store {
    conn: Connection,
}

fn err(e: rusqlite::Error) -> String {
    e.to_string()
}

/// A path as the database keeps it (as the text files did: `Path::display`).
fn path_text(p: &Path) -> String {
    p.display().to_string()
}

impl Store {
    /// Opens (or makes) the database at `path` and brings its schema up to this build's.
    pub fn open(path: &Path) -> Result<Store, String> {
        let conn = Connection::open(path).map_err(err)?;
        // One writer (the game), readers never block it, and a crash loses at most the last
        // write rather than corrupting the file.
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(err)?;
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(err)?;
        Store::start(conn)
    }

    /// A database in memory only, for tests.
    #[cfg(test)]
    pub fn in_memory() -> Result<Store, String> {
        Store::start(Connection::open_in_memory().map_err(err)?)
    }

    fn start(conn: Connection) -> Result<Store, String> {
        conn.busy_timeout(std::time::Duration::from_secs(2))
            .map_err(err)?;
        let mut store = Store { conn };
        store.upgrade()?;
        Ok(store)
    }

    /// The schema version the file is at.
    pub fn version(&self) -> Result<u32, String> {
        self.conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(err)
    }

    /// Applies the versions the file doesn't have yet, each in its own transaction.
    fn upgrade(&mut self) -> Result<(), String> {
        let at = self.version()?;
        if at > VERSION {
            return Err(format!(
                "{FILE} is from a newer version of taiko.rs (schema {at}, this one knows {VERSION})"
            ));
        }
        for (i, script) in MIGRATIONS.iter().enumerate().skip(at as usize) {
            let tx = self.conn.transaction().map_err(err)?;
            tx.execute_batch(script).map_err(err)?;
            tx.pragma_update(None, "user_version", i as u32 + 1)
                .map_err(err)?;
            tx.commit().map_err(err)?;
        }
        Ok(())
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>, String> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()
            .map_err(err)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value",
                [key, value],
            )
            .map(drop)
            .map_err(err)
    }

    /// Runs `f` in one transaction: all of its writes land, or none do.
    pub fn transaction<T>(
        &mut self,
        f: impl FnOnce(&Store) -> Result<T, String>,
    ) -> Result<T, String> {
        self.conn.execute_batch("BEGIN IMMEDIATE").map_err(err)?;
        match f(self) {
            Ok(v) => {
                self.conn.execute_batch("COMMIT").map_err(err)?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    // Scores.

    /// Every local score, oldest first, each replay file in `replays`.
    pub fn scores(&self, replays: &Path) -> Result<Vec<ScoreRow>, String> {
        let mut q = self
            .conn
            .prepare(
                "SELECT map_hash, date, score, accuracy, max_combo, miss, rate, player, replay,
                        mods, pp
                 FROM scores ORDER BY id",
            )
            .map_err(err)?;
        let rows = q
            .query_map([], |r| {
                Ok(ScoreRow {
                    map_hash: r.get::<_, i64>(0)? as u64,
                    date: r.get::<_, i64>(1)? as u64,
                    score: r.get::<_, i64>(2)? as u64,
                    accuracy: r.get(3)?,
                    max_combo: r.get(4)?,
                    miss: r.get(5)?,
                    rate: r.get(6)?,
                    player: r.get(7)?,
                    file: replays.join(r.get::<_, String>(8)?),
                    mods: r.get(9)?,
                    pp: r.get(10)?,
                })
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    pub fn add_score(&self, row: &ScoreRow) -> Result<(), String> {
        let replay = row
            .file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.conn
            .execute(
                "INSERT INTO scores (map_hash, date, score, accuracy, max_combo, miss, rate,
                                     player, replay, mods, pp)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    row.map_hash as i64,
                    row.date as i64,
                    row.score as i64,
                    row.accuracy,
                    row.max_combo,
                    row.miss,
                    row.rate,
                    row.player,
                    replay,
                    row.mods,
                    row.pp,
                ],
            )
            .map(drop)
            .map_err(err)
    }

    /// Replaces every score with `rows`, in their order.
    pub fn replace_scores(&mut self, rows: &[ScoreRow]) -> Result<(), String> {
        self.transaction(|s| s.write_scores(rows))
    }

    /// `replace_scores` inside a transaction already open.
    pub(crate) fn write_scores(&self, rows: &[ScoreRow]) -> Result<(), String> {
        self.conn.execute("DELETE FROM scores", []).map_err(err)?;
        rows.iter().try_for_each(|r| self.add_score(r))
    }

    // Plays.

    /// When each map was last played, by its path.
    pub fn plays(&self) -> Result<HashMap<PathBuf, u64>, String> {
        let mut q = self
            .conn
            .prepare("SELECT path, last_played FROM plays")
            .map_err(err)?;
        let rows = q
            .query_map([], |r| {
                Ok((
                    PathBuf::from(r.get::<_, String>(0)?),
                    r.get::<_, i64>(1)? as u64,
                ))
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Notes that the map at `path` was played at `at` (a later time than one kept wins).
    pub fn add_play(&self, path: &Path, at: u64) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO plays (path, last_played) VALUES (?1, ?2)
                 ON CONFLICT (path) DO UPDATE SET last_played = max(last_played, excluded.last_played)",
                params![path_text(path), at as i64],
            )
            .map(drop)
            .map_err(err)
    }

    /// Replaces every play with `plays` (inside a transaction already open).
    pub(crate) fn write_plays(&self, plays: &HashMap<PathBuf, u64>) -> Result<(), String> {
        self.conn.execute("DELETE FROM plays", []).map_err(err)?;
        plays.iter().try_for_each(|(p, &at)| self.add_play(p, at))
    }

    // Hidden maps.

    /// (Songs hidden by folder, difficulties hidden by file).
    pub fn hidden(&self) -> Result<(HashSet<PathBuf>, HashSet<PathBuf>), String> {
        let mut q = self
            .conn
            .prepare("SELECT kind, path FROM hidden")
            .map_err(err)?;
        let rows = q
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(err)?;
        let (mut songs, mut maps) = (HashSet::new(), HashSet::new());
        for row in rows {
            let (kind, path) = row.map_err(err)?;
            if kind == "song" {
                songs.insert(PathBuf::from(path));
            } else {
                maps.insert(PathBuf::from(path));
            }
        }
        Ok((songs, maps))
    }

    pub fn replace_hidden(
        &mut self,
        songs: &HashSet<PathBuf>,
        maps: &HashSet<PathBuf>,
    ) -> Result<(), String> {
        self.transaction(|s| s.write_hidden(songs, maps))
    }

    pub(crate) fn write_hidden(
        &self,
        songs: &HashSet<PathBuf>,
        maps: &HashSet<PathBuf>,
    ) -> Result<(), String> {
        self.conn.execute("DELETE FROM hidden", []).map_err(err)?;
        let mut q = self
            .conn
            .prepare("INSERT INTO hidden (kind, path) VALUES (?1, ?2)")
            .map_err(err)?;
        for (kind, set) in [("song", songs), ("map", maps)] {
            for p in set {
                q.execute(params![kind, path_text(p)]).map_err(err)?;
            }
        }
        Ok(())
    }

    // Each map's own offset.

    pub fn map_offsets(&self) -> Result<HashMap<u64, f64>, String> {
        let mut q = self
            .conn
            .prepare("SELECT map_hash, offset_ms FROM map_offsets")
            .map_err(err)?;
        let rows = q
            .query_map([], |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?)))
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    pub fn replace_map_offsets(&mut self, offsets: &HashMap<u64, f64>) -> Result<(), String> {
        self.transaction(|s| s.write_map_offsets(offsets))
    }

    pub(crate) fn write_map_offsets(&self, offsets: &HashMap<u64, f64>) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM map_offsets", [])
            .map_err(err)?;
        let mut q = self
            .conn
            .prepare("INSERT INTO map_offsets (map_hash, offset_ms) VALUES (?1, ?2)")
            .map_err(err)?;
        for (&h, &ms) in offsets {
            q.execute(params![h as i64, ms]).map_err(err)?;
        }
        Ok(())
    }

    // Sets of names: the map folders whose skin is off, the favourite songs.

    fn names(&self, table: Names) -> Result<HashSet<String>, String> {
        let (t, c) = table.sql();
        let mut q = self
            .conn
            .prepare(&format!("SELECT {c} FROM {t}"))
            .map_err(err)?;
        let rows = q.query_map([], |r| r.get(0)).map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    fn replace_names(&mut self, table: Names, names: &HashSet<String>) -> Result<(), String> {
        self.transaction(|s| s.write_names(table, names))
    }

    pub(crate) fn write_names(&self, table: Names, names: &HashSet<String>) -> Result<(), String> {
        let (t, c) = table.sql();
        self.conn
            .execute(&format!("DELETE FROM {t}"), [])
            .map_err(err)?;
        let mut q = self
            .conn
            .prepare(&format!("INSERT INTO {t} ({c}) VALUES (?1)"))
            .map_err(err)?;
        for n in names {
            q.execute([n]).map_err(err)?;
        }
        Ok(())
    }

    pub fn map_skins_off(&self) -> Result<HashSet<String>, String> {
        self.names(Names::SkinsOff)
    }

    pub fn replace_map_skins_off(&mut self, folders: &HashSet<String>) -> Result<(), String> {
        self.replace_names(Names::SkinsOff, folders)
    }

    pub fn favourites(&self) -> Result<HashSet<String>, String> {
        self.names(Names::Favourites)
    }

    pub fn replace_favourites(&mut self, songs: &HashSet<String>) -> Result<(), String> {
        self.replace_names(Names::Favourites, songs)
    }
}

/// The tables that are a set of names, and their column.
#[derive(Clone, Copy)]
pub(crate) enum Names {
    SkinsOff,
    Favourites,
}

impl Names {
    fn sql(self) -> (&'static str, &'static str) {
        match self {
            Names::SkinsOff => ("map_skins_off", "folder"),
            Names::Favourites => ("favourites", "song"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_database_from_a_newer_build_is_left_alone() {
        let dir = std::env::temp_dir().join(format!("taiko-rs-store-newer-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE);
        drop(Store::open(&path).unwrap());
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", VERSION + 1)
            .unwrap();
        drop(conn);
        let e = Store::open(&path).err().expect("opened a newer schema");
        assert!(e.contains("newer version"), "{e}");
        let conn = Connection::open(&path).unwrap();
        let v: u32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, VERSION + 1, "the newer file was changed");
        drop(conn);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_transaction_writes_nothing() {
        let mut s = Store::in_memory().unwrap();
        s.set_meta("a", "1").unwrap();
        let r: Result<(), String> = s.transaction(|s| {
            s.set_meta("a", "2")?;
            s.set_meta("b", "3")?;
            Err("stop".into())
        });
        assert!(r.is_err());
        assert_eq!(s.meta("a").unwrap().as_deref(), Some("1"));
        assert_eq!(s.meta("b").unwrap(), None);
    }
}
