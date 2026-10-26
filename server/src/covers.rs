//! Map covers on the website, without the NSFW ones. Each map set's info comes from the beatmap
//! mirrors (on a thread of its own) and is kept in DIR/covers.json; sets can also be blocked by
//! hand, one set id a line in DIR/covers-blocked.txt. A set not looked up yet shows no cover
//! until it has been. The same lookup says which sets give pp: only ranked, approved and loved
//! ones, as on osu!.

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
    /// On osu!: "ranked", "loved", "graveyard"...; empty in entries from before it was kept.
    #[serde(default)]
    pub status: String,
    /// Unix seconds it was looked up (0: before this was kept).
    #[serde(default)]
    pub checked: u64,
}

/// Sets whose status can still change (qualified, pending...) are looked up again this often.
const RECHECK_SECS: u64 = 24 * 3600;
/// How long a submit waits on the mirrors for a set never looked up.
const SUBMIT_WAIT: Duration = Duration::from_secs(10);

/// Whether a set with this status gives pp.
fn status_gives_pp(status: &str) -> bool {
    matches!(status, "ranked" | "approved" | "loved")
}

/// Whether plays on `set` give pp, from what's known of it (None: not known yet).
fn gives_pp_by(set: Option<&Set>) -> Option<bool> {
    match set {
        Some(Set::Found(info)) if !info.status.is_empty() => Some(status_gives_pp(&info.status)),
        Some(Set::Missing) => Some(false),
        _ => None,
    }
}

/// Whether a known set should be looked up again: it predates statuses being kept, or its
/// status may have changed since.
fn stale(set: &Set, now: u64) -> bool {
    match set {
        Set::Found(info) => info.status.is_empty() || (!status_gives_pp(&info.status) && now.saturating_sub(info.checked) >= RECHECK_SECS),
        Set::Missing => false,
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
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
                    c.keep(set, found);
                }
                c.asked.lock().unwrap().remove(&set);
                // Gently: the mirrors are shared.
                std::thread::sleep(Duration::from_millis(500));
            }
        })
        .expect("covers thread");
}

impl Covers {
    /// Takes in what the mirrors said about `set` and saves it.
    fn keep(&self, set: u32, found: Set) {
        let text = {
            let mut sets = self.sets.lock().unwrap();
            sets.insert(set, found);
            let all: HashMap<String, &Set> = sets.iter().map(|(k, v)| (k.to_string(), v)).collect();
            serde_json::to_string(&all).unwrap_or_default()
        };
        let tmp = self.file.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &self.file)) {
            eprintln!("covers: saving {}: {e}", self.file.display());
        }
    }

    /// What's known of `set`, asking the lookup thread about it if nothing is or it's stale.
    fn known(&self, set: u32) -> Option<Set> {
        let known = self.sets.lock().unwrap().get(&set).cloned();
        if known.as_ref().is_none_or(|s| stale(s, now_secs())) && self.asked.lock().unwrap().insert(set) {
            let _ = self.tx.lock().unwrap().send(set);
        }
        known
    }
}

/// Whether set `set`'s cover may show. One not looked up yet doesn't, and is looked up.
pub fn allowed(set: u32) -> bool {
    let Some(c) = COVERS.get() else { return true };
    let blocked = std::fs::read_to_string(&c.blocked_file).is_ok_and(|t| t.lines().any(|l| l.trim().parse() == Ok(set)));
    allowed_by(c.known(set).as_ref(), blocked).unwrap_or(false)
}

/// Whether plays on `set` give pp (None: not known yet, and it's being looked up). Plays on maps
/// with no set never do. Without `init` (tests) every play does.
pub fn gives_pp(set: Option<u32>) -> Option<bool> {
    let Some(c) = COVERS.get() else { return Some(true) };
    let Some(set) = set else { return Some(false) };
    gives_pp_by(c.known(set).as_ref())
}

/// Asks the mirrors right away (for a while) about `set` if whether it gives pp isn't known yet,
/// so a play just sent on it counts at once. Call it without any lock held.
pub fn learn(set: Option<u32>) {
    if gives_pp(set).is_some() {
        return;
    }
    let (Some(set), Some(c)) = (set, COVERS.get()) else { return };
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(look_up(set));
    });
    if let Ok(Some(found)) = rx.recv_timeout(SUBMIT_WAIT) {
        c.keep(set, found);
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
        status: String,
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
                    status: r.status,
                    checked: now_secs(),
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
    fn only_ranked_approved_and_loved_sets_give_pp() {
        let with = |status: &str| Set::Found(Info { status: status.into(), checked: 100, ..Default::default() });
        for (status, gives) in [("ranked", true), ("approved", true), ("loved", true), ("qualified", false), ("pending", false), ("wip", false), ("graveyard", false)] {
            assert_eq!(gives_pp_by(Some(&with(status))), Some(gives), "{status}");
        }
        assert_eq!(gives_pp_by(Some(&Set::Missing)), Some(false));
        assert_eq!(gives_pp_by(Some(&with(""))), None, "looked up before statuses were kept");
        assert_eq!(gives_pp_by(None), None);
        // Statuses that may still change are asked about again, a day on.
        assert!(stale(&with(""), 100));
        assert!(!stale(&with("ranked"), 100 + RECHECK_SECS * 10));
        assert!(!stale(&with("qualified"), 100 + RECHECK_SECS - 1));
        assert!(stale(&with("qualified"), 100 + RECHECK_SECS));
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
