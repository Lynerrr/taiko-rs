//! `taiko_logic::beatmap` over the map files: the crate reads a map from its text or a reader,
//! these open the file.

use std::path::Path;

pub use taiko_logic::beatmap::*;

/// A map file's header (`Meta`), reading only as far as it goes.
pub fn read_meta_file(path: &Path) -> Option<Meta> {
    if crate::tja_maps::is_course(path) {
        return crate::tja_maps::course(path).ok().map(|c| c.map.meta);
    }
    read_meta_from(std::io::BufReader::new(std::fs::File::open(path).ok()?))
}

/// The map file at `path`, converted for play.
pub fn load(path: &Path) -> Result<Beatmap, String> {
    if crate::tja_maps::is_course(path) {
        return crate::tja_maps::load(path);
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    load_text(&text, is_tko(path))
}
