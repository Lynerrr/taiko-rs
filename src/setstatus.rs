//! Each beatmap set's status on osu! as the mirrors told it, kept in the player's database
//! (`taiko.db`; without one, `set-status.tsv` in the cache folder) so asking the mirrors once is
//! enough: sorting and searching by status work from it at the next start
//! (and for every set, since the sweep in the background asks about them all, slowly).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// What the cache keeps for a set the mirrors don't know.
pub const MISSING: &str = "missing";

/// Seconds before an answer is asked again: a ranked, approved or loved set doesn't change, one
/// that's pending, qualified or in the graveyard might any day.
const FINAL_TTL: u64 = 60 * 86_400;
const MISSING_TTL: u64 = 14 * 86_400;
const OPEN_TTL: u64 = 2 * 86_400;

#[derive(Default)]
pub struct Cache {
    /// Set ID -> (status, when it was asked: Unix seconds).
    sets: HashMap<u32, (String, u64)>,
    dirty: bool,
    /// The sets answered since the cache was last written (the database writes only those).
    changed: HashSet<u32>,
}

/// The `meta` key set once `set-status.tsv` has been brought into the player's database.
const MOVED: &str = "set_status_from_file";

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn file() -> Option<PathBuf> {
    crate::config::cache_dir().map(|d| d.join("set-status.tsv"))
}

impl Cache {
    pub fn load() -> Cache {
        // Tests don't read (or write) the player's.
        if cfg!(test) {
            return Cache::default();
        }
        let text = || file().and_then(|f| std::fs::read_to_string(f).ok());
        if let Some(cache) = crate::store::with(|s| Cache::load_from(s, text)) {
            return cache;
        }
        text().map(|t| Cache::parse(&t)).unwrap_or_default()
    }

    /// The cache as the player's database keeps it; the first time, filled from the file's
    /// `text` (which is left as it is).
    pub fn load_from(
        store: &mut crate::store::Store,
        text: impl FnOnce() -> Option<String>,
    ) -> Result<Cache, String> {
        if store.meta(MOVED)?.is_none() {
            let old = text().map(|t| Cache::parse(&t)).unwrap_or_default();
            store.put_set_statuses(
                old.sets
                    .iter()
                    .map(|(&set, (s, at))| (set, s.as_str(), *at)),
            )?;
            store.set_meta(MOVED, "1")?;
        }
        Ok(Cache {
            sets: store.set_statuses()?,
            ..Cache::default()
        })
    }

    /// Writes the answers since the last write into the player's database.
    pub fn save_to(&mut self, store: &mut crate::store::Store) -> Result<(), String> {
        let rows: Vec<(u32, &str, u64)> = self
            .changed
            .iter()
            .filter_map(|set| self.sets.get(set).map(|(s, at)| (*set, s.as_str(), *at)))
            .collect();
        store.put_set_statuses(rows)?;
        self.changed.clear();
        self.dirty = false;
        Ok(())
    }

    fn parse(text: &str) -> Cache {
        let sets = text
            .lines()
            .filter_map(|line| {
                let mut f = line.split('\t');
                let set = f.next()?.parse().ok()?;
                let status = f.next()?.to_string();
                let at = f.next()?.parse().ok()?;
                (!status.is_empty()).then_some((set, (status, at)))
            })
            .collect();
        Cache {
            sets,
            ..Cache::default()
        }
    }

    /// Writes the cache if it changed since it was last written.
    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        if crate::store::with(|s| self.save_to(s)).is_some() {
            return;
        }
        self.dirty = false;
        let Some(path) = file() else { return };
        let mut rows: Vec<_> = self.sets.iter().collect();
        rows.sort();
        let text: String = rows
            .iter()
            .map(|(set, (status, at))| format!("{set}\t{status}\t{at}\n"))
            .collect();
        let tmp = path.with_extension("tsv.tmp");
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::write(&tmp, text)
            .and_then(|_| std::fs::rename(&tmp, &path))
            .is_err()
        {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    pub fn put(&mut self, set: u32, status: &str, at: u64) {
        self.sets.insert(set, (status.to_string(), at));
        self.dirty = true;
        self.changed.insert(set);
    }

    /// The status the mirrors gave (not `MISSING`).
    pub fn status(&self, set: u32) -> Option<&str> {
        self.sets
            .get(&set)
            .map(|(s, _)| s.as_str())
            .filter(|s| *s != MISSING)
    }

    pub fn iter(&self) -> impl Iterator<Item = (u32, &str)> {
        self.sets
            .iter()
            .map(|(&set, (status, _))| (set, status.as_str()))
    }

    /// Whether `set` should be asked about (again) at `now`.
    pub fn due(&self, set: u32, now: u64) -> bool {
        match self.sets.get(&set) {
            None => true,
            Some((status, at)) => {
                let ttl = match status.as_str() {
                    "ranked" | "approved" | "loved" => FINAL_TTL,
                    MISSING => MISSING_TTL,
                    _ => OPEN_TTL,
                };
                now.saturating_sub(*at) >= ttl
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_are_kept_and_asked_again_when_they_may_have_changed() {
        let mut c = Cache::default();
        assert!(c.due(1, 0));
        c.put(1, "ranked", 1000);
        c.put(2, "pending", 1000);
        c.put(3, MISSING, 1000);
        assert!(!c.due(1, 1000 + 86_400) && !c.due(2, 1000 + 3600) && !c.due(3, 1000 + 86_400));
        assert!(
            c.due(2, 1000 + 3 * 86_400),
            "pending is asked again in days"
        );
        assert!(c.due(3, 1000 + 15 * 86_400));
        assert!(!c.due(1, 1000 + 59 * 86_400) && c.due(1, 1000 + 61 * 86_400));
        assert_eq!(
            (c.status(1), c.status(3), c.status(4)),
            (Some("ranked"), None, None)
        );
    }

    #[test]
    fn the_file_round_trips_and_skips_bad_lines() {
        let c =
            Cache::parse("5\tloved\t10\nnot a line\n6\t\t10\n7\tgraveyard\tx\n8\tpending\t20\n");
        let mut got: Vec<_> = c.iter().collect();
        got.sort();
        assert_eq!(got, vec![(5, "loved"), (8, "pending")]);
    }
}
