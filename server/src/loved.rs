//! Maps loved here: sets loved nominators (and admins) have made count for pp on taiko.rs,
//! whatever osu! says of them (graveyard maps, so they can be farmed). Kept in
//! DIR/loved.json as set id -> who and when, and which difficulties when not the whole set (a
//! set's 1.1x, 1.2x... rate edits stay out that way).

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
    /// The difficulties loved (their names, "Inner Oni"); none: the whole set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub versions: Vec<String>,
}

impl Loved {
    /// Whether plays on difficulty `version` of the set give pp.
    pub fn covers(&self, version: &str) -> bool {
        self.versions.is_empty() || self.versions.iter().any(|v| same_version(v, version))
    }
}

/// Difficulty names compared as players type them: case and the spaces round them don't count.
fn same_version(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// The difficulty in a play's map name, "Artist - Title [Version]" (empty without one).
pub fn version_of(map: &str) -> &str {
    map.strip_suffix(']')
        .and_then(|m| m.rsplit_once(" ["))
        .map_or("", |(_, v)| v)
}

struct Store {
    file: Option<PathBuf>,
    sets: Mutex<BTreeMap<u32, Loved>>,
}

static STORE: OnceLock<Store> = OnceLock::new();

fn store() -> &'static Store {
    STORE.get_or_init(|| Store {
        file: None,
        sets: Mutex::new(BTreeMap::new()),
    })
}

/// Reads DIR/loved.json. Without it (tests) the list starts empty and lives in memory.
pub fn init(dir: PathBuf) {
    let file = dir.join("loved.json");
    let sets: BTreeMap<u32, Loved> = crate::load_saved(&file);
    println!("{} map sets loved here ({})", sets.len(), file.display());
    let _ = STORE.set(Store {
        file: Some(file),
        sets: Mutex::new(sets),
    });
}

pub fn get(set: u32) -> Option<Loved> {
    store().sets.lock().unwrap().get(&set).cloned()
}

/// Every set loved here, by id.
pub fn all() -> Vec<(u32, Loved)> {
    store()
        .sets
        .lock()
        .unwrap()
        .iter()
        .map(|(&s, l)| (s, l.clone()))
        .collect()
}

/// Why `set` (just its difficulty `version`, or the whole set) can't be loved, or loved no more
/// (`love` false), as it stands: None when it can.
pub fn refusal(set: u32, love: bool, version: Option<&str>) -> Option<String> {
    let mut sets = store().sets.lock().unwrap().clone();
    change(&mut sets, set, love, "", 0, version).err()
}

/// Loves `set` (by `by`, at `now`), or just its difficulty `version`, or takes it back. Loving the
/// whole of a set some difficulties of which are loved loves the rest too.
pub fn set(set: u32, love: bool, by: &str, now: u64, version: Option<&str>) -> Result<(), String> {
    let s = store();
    let mut sets = s.sets.lock().unwrap();
    change(&mut sets, set, love, by, now, version)?;
    if let Some(file) = &s.file {
        let tmp = file.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(&*sets).unwrap_or_default();
        if let Err(e) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, file)) {
            eprintln!("loved: saving {}: {e}", file.display());
        }
    }
    Ok(())
}

fn change(
    sets: &mut BTreeMap<u32, Loved>,
    set: u32,
    love: bool,
    by: &str,
    now: u64,
    version: Option<&str>,
) -> Result<(), String> {
    let version = version.map(str::trim).filter(|v| !v.is_empty());
    let fresh = |versions: Vec<String>| Loved {
        by: by.to_string(),
        at: now,
        versions,
    };
    match (love, version, sets.get_mut(&set)) {
        (true, None, Some(l)) if l.versions.is_empty() => {
            Err("that set is loved here already".into())
        }
        (true, None, _) => {
            sets.insert(set, fresh(Vec::new()));
            Ok(())
        }
        (true, Some(_), Some(l)) if l.versions.is_empty() => {
            Err("the whole set is loved here already".into())
        }
        (true, Some(v), Some(l)) if l.covers(v) => Err(format!("[{v}] is loved here already")),
        (true, Some(v), Some(l)) => {
            l.versions.push(v.to_string());
            Ok(())
        }
        (true, Some(v), None) => {
            sets.insert(set, fresh(vec![v.to_string()]));
            Ok(())
        }
        (false, _, None) => Err("that set isn't loved here".into()),
        (false, None, Some(_)) => {
            sets.remove(&set);
            Ok(())
        }
        (false, Some(_), Some(l)) if l.versions.is_empty() => Err(
            "the whole set is loved here: unlove the set, then love the difficulties to keep"
                .into(),
        ),
        (false, Some(v), Some(l)) => {
            let before = l.versions.len();
            l.versions.retain(|x| !same_version(x, v));
            if l.versions.len() == before {
                return Err(format!("[{v}] isn't loved here"));
            }
            if l.versions.is_empty() {
                sets.remove(&set);
            }
            Ok(())
        }
    }
}

/// Whether plays on `set` (the map `hash`, difficulty `version`) give pp: a custom map by its
/// approval, loved here (that difficulty, or the whole set), else what osu! says (`covers`).
pub fn gives_pp(set: Option<u32>, hash: u64, version: &str) -> Option<bool> {
    if crate::custom::by_hash(hash).is_some() {
        Some(crate::custom::approved(hash))
    } else if set.and_then(get).is_some_and(|l| l.covers(version)) {
        Some(true)
    } else {
        crate::covers::gives_pp(set)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_difficulties_are_loved_and_unloved_apart_from_the_whole_set() {
        let mut sets = BTreeMap::new();
        let go = |sets: &mut BTreeMap<u32, Loved>, love, version| {
            change(sets, 7, love, "Ryuji", 5, version)
        };
        assert!(go(&mut sets, true, Some(" Inner Oni ")).is_ok());
        assert!(go(&mut sets, true, Some("Oni")).is_ok());
        assert!(go(&mut sets, true, Some("inner oni")).is_err());
        let l = &sets[&7];
        assert_eq!(l.versions, ["Inner Oni", "Oni"]);
        assert!(l.covers("INNER ONI") && l.covers("Oni") && !l.covers("Inner Oni 1.1x"));
        // Unloving the last one unloves the set.
        assert!(go(&mut sets, false, Some("Muzukashii")).is_err());
        assert!(go(&mut sets, false, Some("oni")).is_ok());
        assert!(go(&mut sets, false, Some("Inner Oni")).is_ok());
        assert!(!sets.contains_key(&7));
        // The whole set: every difficulty, and none to love or unlove apart.
        assert!(go(&mut sets, true, Some("Oni")).is_ok());
        assert!(go(&mut sets, true, None).is_ok());
        assert!(sets[&7].versions.is_empty() && sets[&7].covers("Inner Oni 1.2x"));
        assert!(go(&mut sets, true, None).is_err());
        assert!(go(&mut sets, true, Some("Oni")).is_err());
        assert!(go(&mut sets, false, Some("Oni")).is_err());
        assert!(go(&mut sets, false, None).is_ok());
        assert!(go(&mut sets, false, None).is_err());
        // An empty name is the whole set.
        assert!(go(&mut sets, true, Some("  ")).is_ok());
        assert!(sets[&7].versions.is_empty());
    }

    #[test]
    fn a_plays_difficulty_comes_from_its_map_name_and_old_entries_read_as_whole_sets() {
        assert_eq!(version_of("A - T [Inner Oni 1.1x]"), "Inner Oni 1.1x");
        assert_eq!(version_of("A [x] - T [Oni]"), "Oni");
        assert_eq!(version_of("A - T"), "");
        let old: BTreeMap<u32, Loved> =
            serde_json::from_str(r#"{"7": {"by": "Ryuji", "at": 5}}"#).unwrap();
        assert!(old[&7].versions.is_empty());
        let text = serde_json::to_string(&old).unwrap();
        assert!(!text.contains("versions"), "{text}");
    }
}
