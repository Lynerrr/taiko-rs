//! Bringing the player's old text files into `taiko.db`, once.
//!
//! The files are read with the same parsers the game used for them, written in one transaction,
//! read back inside it and compared with what was read; only then is the transaction committed,
//! with the time of the import in `meta`. Anything that goes wrong (a file that exists but can't
//! be read, a write that fails, a read-back that differs) rolls the whole import back: the
//! database stays without the data and without the mark, the game keeps using the files, and
//! the next launch tries again. The files themselves are only ever read, never changed, moved
//! or removed.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::Store;
use crate::scores::ScoreRow;

/// The `meta` key the import's time (Unix seconds) is kept under.
pub const IMPORTED: &str = "imported_files";

/// What the old files held, as the game read them.
#[derive(Debug, Default, PartialEq)]
pub struct Files {
    /// Every row of the score index, with or without its replay file (the game skips the ones
    /// without when it reads them, so nothing it shows depends on keeping them out).
    pub scores: Vec<ScoreRow>,
    pub plays: HashMap<PathBuf, u64>,
    pub hidden_songs: HashSet<PathBuf>,
    pub hidden_maps: HashSet<PathBuf>,
    pub map_offsets: HashMap<u64, f64>,
    pub map_skins_off: HashSet<String>,
    pub favourites: HashSet<String>,
}

/// A file's text: None when there's no file, an error when there is one that can't be read
/// (which the game took for an empty file; the import doesn't guess).
fn read(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| format!("{} isn't UTF-8 text", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("couldn't read {}: {e}", path.display())),
    }
}

/// Reads the old files in the data folder `data`.
pub fn read_files(data: &Path) -> Result<Files, String> {
    use crate::config::{MAP_OFFSETS, MAP_SKINS_OFF, OLD_SIGN_MAP_OFFSETS, parse_lines};
    let replays = data.join(crate::scores::REPLAYS);
    let mut files = Files::default();
    if let Some(text) = read(&replays.join(crate::scores::INDEX))? {
        files.scores = crate::scores::parse_index(&text, &replays);
    }
    if let Some(text) = read(&data.join("plays.tsv"))? {
        files.plays = crate::library::parse_plays(&text);
    }
    if let Some(text) = read(&data.join("hidden.tsv"))? {
        let h = crate::ui::Hidden::parse(&text);
        (files.hidden_songs, files.hidden_maps) = (h.songs, h.maps);
    }
    // As `config::load_map_offsets` takes them: the file with osu!'s sign, else the one from
    // before it turned round.
    if let Some(text) = read(&data.join(MAP_OFFSETS))? {
        files.map_offsets = crate::config::parse_map_offsets(&text);
    } else if let Some(text) = read(&data.join(OLD_SIGN_MAP_OFFSETS))? {
        files.map_offsets = crate::config::parse_map_offsets(&text)
            .into_iter()
            .map(|(h, ms)| (h, -ms))
            .collect();
    }
    if let Some(text) = read(&data.join(MAP_SKINS_OFF))? {
        files.map_skins_off = parse_lines(&text);
    }
    if let Some(text) = read(&data.join(crate::playlist::FAVOURITES))? {
        files.favourites = parse_lines(&text);
    }
    Ok(files)
}

/// What the database holds, read the same way (`replays`: the replays' folder).
pub fn read_store(store: &Store, replays: &Path) -> Result<Files, String> {
    let (hidden_songs, hidden_maps) = store.hidden()?;
    Ok(Files {
        scores: store.scores(replays)?,
        plays: store.plays()?,
        hidden_songs,
        hidden_maps,
        map_offsets: store.map_offsets()?,
        map_skins_off: store.map_skins_off()?,
        favourites: store.favourites()?,
    })
}

/// Whether the old files have been brought in.
pub fn imported(store: &Store) -> Result<bool, String> {
    Ok(store.meta(IMPORTED)?.is_some())
}

/// Brings the old files of the data folder `data` in, unless they already are. Returns whether
/// it did. On an error nothing is kept (see the module's comment).
pub fn import(store: &mut Store, data: &Path, now: u64) -> Result<bool, String> {
    if imported(store)? {
        return Ok(false);
    }
    let files = read_files(data)?;
    let replays = data.join(crate::scores::REPLAYS);
    store.transaction(|s| {
        s.write_scores(&files.scores)?;
        s.write_plays(&files.plays)?;
        s.write_hidden(&files.hidden_songs, &files.hidden_maps)?;
        s.write_map_offsets(&files.map_offsets)?;
        s.write_names(super::Names::SkinsOff, &files.map_skins_off)?;
        s.write_names(super::Names::Favourites, &files.favourites)?;
        let back = read_store(s, &replays)?;
        if back != files {
            return Err("the database didn't read back what the files hold".into());
        }
        s.set_meta(IMPORTED, &now.to_string())
    })?;
    Ok(true)
}
