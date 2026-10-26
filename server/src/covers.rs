//! Map covers on the website, without the NSFW ones. Each map set's info comes from the beatmap
//! mirrors (on a thread of its own) and is kept in DIR/covers.json; sets can also be blocked by
//! hand, one set id a line in DIR/covers-blocked.txt. A set not looked up yet shows no cover
//! until it has been.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// What the mirrors say about a set.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Set {
    Found(Info),
    /// No mirror knows it.
    Missing,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Info {
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist_unicode: String,
    #[serde(default)]
    pub title_unicode: String,
    #[serde(default)]
    pub nsfw: bool,
    #[serde(default)]
    pub versions: Vec<String>,
}

struct Covers {
    file: PathBuf,
    blocked_file: PathBuf,
    sets: Mutex<HashMap<u32, Set>>,
    asked: Mutex<HashSet<u32>>,
    tx: Mutex<Sender<u32>>,
}

static COVERS: OnceLock<Covers> = OnceLock::new();

/// Sets that may show a cover: what's known about each (None: not looked up yet) and whether
/// it's blocked by hand.
fn allowed_by(set: Option<&Set>, blocked: bool) -> Option<bool> {
    if blocked {
        return Some(false);
    }
    match set {
        Some(Set::Found(info)) => Some(!info.nsfw),
        Some(Set::Missing) => Some(true),
        None => None,
    }
}

/// Reads DIR/covers.json and starts looking sets up. Without it (tests) every cover shows.
pub fn init(dir: PathBuf) {
    let file = dir.join("covers.json");
    let sets: HashMap<u32, Set> = std::fs::read_to_string(&file)
        .ok()
        .and_then(|text| serde_json::from_str::<HashMap<String, serde_json::Value>>(&text).ok())
        .map(|raw| raw.into_iter().filter_map(|(k, v)| Some((k.parse().ok()?, serde_json::from_value(v).ok()?))).collect())
        .unwrap_or_default();
    println!("{} map sets' covers known ({})", sets.len(), file.display());
    let (tx, rx) = mpsc::channel::<u32>();
    let covers = Covers { file, blocked_file: dir.join("covers-blocked.txt"), sets: Mutex::new(sets), asked: Mutex::new(HashSet::new()), tx: Mutex::new(tx) };
    if COVERS.set(covers).is_err() {
        return;
    }
    std::thread::Builder::new()
        .name("covers".into())
        .spawn(move || {
            for set in rx {
                let Some(c) = COVERS.get() else { return };
                if let Some(found) = look_up(set) {
                    let text = {
                        let mut sets = c.sets.lock().unwrap();
                        sets.insert(set, found);
                        let all: HashMap<String, &Set> = sets.iter().map(|(k, v)| (k.to_string(), v)).collect();
                        serde_json::to_string(&all).unwrap_or_default()
                    };
                    let tmp = c.file.with_extension("json.tmp");
                    if let Err(e) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &c.file)) {
                        eprintln!("covers: saving {}: {e}", c.file.display());
                    }
                }
                // Gently: the mirrors are shared.
                std::thread::sleep(Duration::from_millis(500));
            }
        })
        .expect("covers thread");
}

/// Whether set `set`'s cover may show. One not looked up yet doesn't, and is looked up.
pub fn allowed(set: u32) -> bool {
    let Some(c) = COVERS.get() else { return true };
    let blocked = std::fs::read_to_string(&c.blocked_file).is_ok_and(|t| t.lines().any(|l| l.trim().parse() == Ok(set)));
    let known = c.sets.lock().unwrap().get(&set).cloned();
    match allowed_by(known.as_ref(), blocked) {
        Some(ok) => ok,
        None => {
            if c.asked.lock().unwrap().insert(set) {
                let _ = c.tx.lock().unwrap().send(set);
            }
            false
        }
    }
}

/// A set's info from the mirrors: None when they couldn't be asked (try again later).
fn look_up(set: u32) -> Option<Set> {
    #[derive(Deserialize)]
    struct Beatmap {
        #[serde(default)]
        version: String,
    }
    #[derive(Deserialize)]
    struct Remote {
        #[serde(default)]
        artist: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        artist_unicode: Option<String>,
        #[serde(default)]
        title_unicode: Option<String>,
        #[serde(default)]
        nsfw: bool,
        #[serde(default)]
        beatmaps: Vec<Beatmap>,
    }
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(15))).http_status_as_error(false).build().into();
    let mut missing = 0;
    let mirrors = ["https://catboy.best/api/v2/s/", "https://osu.direct/api/v2/s/"];
    for base in mirrors {
        let Ok(mut reply) = agent.get(&format!("{base}{set}")).header("User-Agent", "taiko.rs-server (+https://rstaiko.auneu.space)").call() else { continue };
        match reply.status().as_u16() {
            200 => {
                let Some(r) = reply.body_mut().read_to_string().ok().and_then(|t| serde_json::from_str::<Remote>(&t).ok()) else { continue };
                return Some(Set::Found(Info {
                    artist_unicode: r.artist_unicode.unwrap_or_else(|| r.artist.clone()),
                    title_unicode: r.title_unicode.unwrap_or_else(|| r.title.clone()),
                    artist: r.artist,
                    title: r.title,
                    nsfw: r.nsfw,
                    versions: r.beatmaps.into_iter().map(|b| b.version).collect(),
                }));
            }
            404 => missing += 1,
            _ => {}
        }
    }
    (missing == mirrors.len()).then_some(Set::Missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nsfw_and_blocked_sets_hide_their_cover_and_unknown_ones_wait() {
        let safe = Set::Found(Info { nsfw: false, ..Default::default() });
        let nsfw = Set::Found(Info { nsfw: true, ..Default::default() });
        assert_eq!(allowed_by(Some(&safe), false), Some(true));
        assert_eq!(allowed_by(Some(&nsfw), false), Some(false));
        assert_eq!(allowed_by(Some(&safe), true), Some(false), "blocked by hand");
        assert_eq!(allowed_by(Some(&Set::Missing), false), Some(true));
        assert_eq!(allowed_by(None, false), None);
    }

    #[test]
    fn the_old_covers_file_still_reads() {
        let old = r#"{"2474178": {"Found": {"artist": "USAO", "title": "Dynamite", "artist_unicode": "USAO", "title_unicode": "Dynamite", "nsfw": false, "versions": ["kaBOOM"]}}}"#;
        let raw: HashMap<String, serde_json::Value> = serde_json::from_str(old).unwrap();
        let set: Set = serde_json::from_value(raw["2474178"].clone()).unwrap();
        assert!(matches!(set, Set::Found(Info { nsfw: false, .. })));
        // And what's written reads back.
        let back: Set = serde_json::from_str(&serde_json::to_string(&set).unwrap()).unwrap();
        assert_eq!(back, set);
    }
}
