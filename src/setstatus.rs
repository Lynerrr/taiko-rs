//! Each beatmap set's status on osu! as the mirrors told it, kept in the cache folder so asking
//! the mirrors once is enough: sorting and searching by status work from it at the next start
//! (and for every set, since the sweep in the background asks about them all, slowly).

use std::collections::HashMap;
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
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
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
        file().and_then(|f| std::fs::read_to_string(f).ok()).map(|t| Cache::parse(&t)).unwrap_or_default()
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
        Cache { sets, dirty: false }
    }

    /// Writes the cache if it changed since it was last written.
    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let Some(path) = file() else { return };
        let mut rows: Vec<_> = self.sets.iter().collect();
        rows.sort();
        let text: String = rows.iter().map(|(set, (status, at))| format!("{set}\t{status}\t{at}\n")).collect();
        let tmp = path.with_extension("tsv.tmp");
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &path)).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    pub fn put(&mut self, set: u32, status: &str, at: u64) {
        self.sets.insert(set, (status.to_string(), at));
        self.dirty = true;
    }

    /// The status the mirrors gave (not `MISSING`).
    pub fn status(&self, set: u32) -> Option<&str> {
        self.sets.get(&set).map(|(s, _)| s.as_str()).filter(|s| *s != MISSING)
    }

    pub fn iter(&self) -> impl Iterator<Item = (u32, &str)> {
        self.sets.iter().map(|(&set, (status, _))| (set, status.as_str()))
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
        assert!(c.due(2, 1000 + 3 * 86_400), "pending is asked again in days");
        assert!(c.due(3, 1000 + 15 * 86_400));
        assert!(!c.due(1, 1000 + 59 * 86_400) && c.due(1, 1000 + 61 * 86_400));
        assert_eq!((c.status(1), c.status(3), c.status(4)), (Some("ranked"), None, None));
    }

    #[test]
    fn the_file_round_trips_and_skips_bad_lines() {
        let c = Cache::parse("5\tloved\t10\nnot a line\n6\t\t10\n7\tgraveyard\tx\n8\tpending\t20\n");
        let mut got: Vec<_> = c.iter().collect();
        got.sort();
        assert_eq!(got, vec![(5, "loved"), (8, "pending")]);
    }
}
