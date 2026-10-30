//! Maps loved here: sets loved nominators (and admins) have made count for pp on taiko.rs,
//! whatever osu! says of them (graveyard maps, so they can be farmed). Kept in
//! DIR/loved.json as set id -> who and when.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Loved {
    /// Who loved it (their name as written).
    pub by: String,
    /// Unix seconds.
    pub at: u64,
}

struct Store {
    file: Option<PathBuf>,
    sets: Mutex<BTreeMap<u32, Loved>>,
}

static STORE: OnceLock<Store> = OnceLock::new();

fn store() -> &'static Store {
    STORE.get_or_init(|| Store { file: None, sets: Mutex::new(BTreeMap::new()) })
}

/// Reads DIR/loved.json. Without it (tests) the list starts empty and lives in memory.
pub fn init(dir: PathBuf) {
    let file = dir.join("loved.json");
    let sets: BTreeMap<u32, Loved> = crate::load_saved(&file);
    println!("{} map sets loved here ({})", sets.len(), file.display());
    let _ = STORE.set(Store { file: Some(file), sets: Mutex::new(sets) });
}

pub fn contains(set: Option<u32>) -> bool {
    set.is_some_and(|s| store().sets.lock().unwrap().contains_key(&s))
}

pub fn get(set: u32) -> Option<Loved> {
    store().sets.lock().unwrap().get(&set).cloned()
}

/// Every set loved here, by id.
pub fn all() -> Vec<(u32, Loved)> {
    store().sets.lock().unwrap().iter().map(|(&s, l)| (s, l.clone())).collect()
}

/// Loves `set` (by `by`, at `now`) or takes it back; false when nothing changed.
pub fn set(set: u32, love: bool, by: &str, now: u64) -> bool {
    let s = store();
    let mut sets = s.sets.lock().unwrap();
    let changed = if love { sets.insert(set, Loved { by: by.to_string(), at: now }).is_none() } else { sets.remove(&set).is_some() };
    if changed {
        if let Some(file) = &s.file {
            let tmp = file.with_extension("json.tmp");
            let text = serde_json::to_string_pretty(&*sets).unwrap_or_default();
            if let Err(e) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, file)) {
                eprintln!("loved: saving {}: {e}", file.display());
            }
        }
    }
    changed
}

/// Whether plays on `set` give pp: loved here, else what osu! says (`covers`).
pub fn gives_pp(set: Option<u32>) -> Option<bool> {
    if contains(set) { Some(true) } else { crate::covers::gives_pp(set) }
}
