//! Beatmap sets' statuses on osu! for the game's song select (sorting and searching by status),
//! from osu!'s own API (v2, client credentials). Players ask about the sets they have; what the
//! server has already (here, or from the map covers' mirror lookups) is answered at once, and the
//! rest is looked up in the background: 50 maps a request, a request a second at most (osu!'s
//! limit is 60 a minute), each set asked once and again only when its status may have changed.
//! The game never talks to osu! or the mirrors for this, so the load doesn't grow with the players.
//!
//! The API client is read from DIR/osu-api (its first line the client ID, the second the secret)
//! or the OSU_CLIENT_ID and OSU_CLIENT_SECRET environment. Without it only what is already known is
//! answered. What was found is kept in DIR/osu-statuses.json.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Deserialize;

const USER_AGENT: &str = "taiko-rs-server (+https://rstaiko.auneu.space)";
/// Maps in one request (the API's limit).
const BATCH: usize = 50;
/// Between two requests: osu! asks for at most 60 a minute.
const PACE: Duration = Duration::from_millis(1100);
/// After the API said to slow down, or failed.
const SLOW_DOWN: Duration = Duration::from_secs(60);
/// How many sets wait to be looked up, at most: a flood of made-up IDs can't fill the memory.
const MAX_QUEUE: usize = 100_000;
/// Seconds before a status is asked again: ranked, approved and loved ones hardly change.
const FINAL_TTL: u64 = 60 * 86_400;
const MISSING_TTL: u64 = 14 * 86_400;
const OPEN_TTL: u64 = 2 * 86_400;
/// What's kept for a set osu! doesn't have.
pub const MISSING: &str = "missing";

struct Inner {
    /// Set -> (status, when it was looked up).
    known: HashMap<u32, (String, u64)>,
    /// Sets waiting, with a map of each to ask about.
    queue: VecDeque<(u32, u32)>,
    queued: HashSet<u32>,
    token: Option<(String, Instant)>,
    dirty: bool,
}

struct Status {
    file: PathBuf,
    credentials: Option<(String, String)>,
    inner: Mutex<Inner>,
    wake: Condvar,
}

static STATUS: OnceLock<Status> = OnceLock::new();

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn due(status: &str, at: u64, now: u64) -> bool {
    let ttl = match status {
        "ranked" | "approved" | "loved" => FINAL_TTL,
        MISSING => MISSING_TTL,
        _ => OPEN_TTL,
    };
    now.saturating_sub(at) >= ttl
}

/// Reads what's known from `dir` and starts the lookups.
pub fn init(dir: PathBuf) {
    let file = dir.join("osu-statuses.json");
    let known: HashMap<u32, (String, u64)> = std::fs::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str::<HashMap<String, (String, u64)>>(&t).ok())
        .map(|raw| raw.into_iter().filter_map(|(k, v)| Some((k.parse().ok()?, v))).collect())
        .unwrap_or_default();
    let credentials = std::fs::read_to_string(dir.join("osu-api"))
        .ok()
        .and_then(|t| {
            let mut lines = t.lines().map(str::trim).filter(|l| !l.is_empty());
            Some((lines.next()?.to_string(), lines.next()?.to_string()))
        })
        .or_else(|| Some((std::env::var("OSU_CLIENT_ID").ok()?, std::env::var("OSU_CLIENT_SECRET").ok()?)));
    println!("{} map sets' statuses known, osu! API {}", known.len(), if credentials.is_some() { "on" } else { "off (no osu-api file)" });
    let s = Status { file, credentials, inner: Mutex::new(Inner { known, queue: VecDeque::new(), queued: HashSet::new(), token: None, dirty: false }), wake: Condvar::new() };
    if STATUS.set(s).is_err() {
        return;
    }
    std::thread::Builder::new().name("osu-statuses".into()).spawn(worker).expect("osu statuses thread");
}

/// What's known of the sets `maps` are (set ID, the ID of one of its maps): (set, status), the
/// sets not known yet left out. With `ask`, those and the ones that may have changed are looked
/// up in the background, for the next time.
pub fn lookup(maps: &[(u32, u32)], ask: bool) -> Vec<(u32, String)> {
    let Some(s) = STATUS.get() else { return Vec::new() };
    let mut inner = s.inner.lock().unwrap();
    let now = now();
    let mut out = Vec::new();
    let mut woke = false;
    for &(set, map) in maps {
        // Something the covers' mirror lookups have seen costs nothing.
        let mirror = crate::covers::osu_status(set).filter(|m| m != MISSING);
        let known = inner.known.get(&set).map(|(st, at)| (st.clone(), *at)).or_else(|| mirror.map(|m| (m, now)));
        let stale = known.as_ref().is_none_or(|(st, at)| due(st, *at, now));
        if let Some((status, _)) = known {
            out.push((set, status));
        }
        if ask && stale && s.credentials.is_some() && map > 0 && inner.queue.len() < MAX_QUEUE && inner.queued.insert(set) {
            inner.queue.push_back((set, map));
            woke = true;
        }
    }
    if woke {
        s.wake.notify_one();
    }
    out
}

impl Status {
    fn save(&self, inner: &mut Inner) {
        if !inner.dirty {
            return;
        }
        inner.dirty = false;
        let all: HashMap<String, &(String, u64)> = inner.known.iter().map(|(k, v)| (k.to_string(), v)).collect();
        let tmp = self.file.with_extension("json.tmp");
        let text = serde_json::to_string(&all).unwrap_or_default();
        if let Err(e) = std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, &self.file)) {
            eprintln!("osu statuses: saving {}: {e}", self.file.display());
        }
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).http_status_as_error(false).build().into()
}

/// A bearer token for the public API (client credentials), kept until it's near its end.
fn token(s: &Status, agent: &ureq::Agent) -> Option<String> {
    #[derive(Deserialize)]
    struct Token {
        access_token: String,
        #[serde(default)]
        expires_in: u64,
    }
    if let Some((t, until)) = &s.inner.lock().unwrap().token {
        if Instant::now() < *until {
            return Some(t.clone());
        }
    }
    let (id, secret) = s.credentials.as_ref()?;
    let body = serde_json::json!({ "client_id": id.parse::<u64>().ok().map(serde_json::Value::from).unwrap_or_else(|| id.clone().into()), "client_secret": secret, "grant_type": "client_credentials", "scope": "public" }).to_string();
    let mut reply = agent.post("https://osu.ppy.sh/oauth/token").header("User-Agent", USER_AGENT).header("Content-Type", "application/json").header("Accept", "application/json").send(body).ok()?;
    if reply.status().as_u16() != 200 {
        eprintln!("osu statuses: the token answer was {}", reply.status());
        return None;
    }
    let t: Token = serde_json::from_str(&reply.body_mut().read_to_string().ok()?).ok()?;
    let until = Instant::now() + Duration::from_secs(t.expires_in.saturating_sub(600).max(60));
    s.inner.lock().unwrap().token = Some((t.access_token.clone(), until));
    Some(t.access_token)
}

/// The API's answer for a batch of maps: (map, set, status) of each map it knows.
fn ask(agent: &ureq::Agent, token: &str, maps: &[u32]) -> Result<Vec<(u32, u32, String)>, Duration> {
    #[derive(Deserialize)]
    struct Map {
        id: u32,
        beatmapset_id: u32,
        status: String,
    }
    #[derive(Deserialize)]
    struct Answer {
        beatmaps: Vec<Map>,
    }
    let mut req = agent.get("https://osu.ppy.sh/api/v2/beatmaps").header("User-Agent", USER_AGENT).header("Authorization", &format!("Bearer {token}")).header("Accept", "application/json");
    for m in maps {
        req = req.query("ids[]", m.to_string());
    }
    let mut reply = req.call().map_err(|e| {
        eprintln!("osu statuses: {e}");
        SLOW_DOWN
    })?;
    match reply.status().as_u16() {
        200 => {
            let text = reply.body_mut().read_to_string().map_err(|_| SLOW_DOWN)?;
            let a: Answer = serde_json::from_str(&text).map_err(|e| {
                eprintln!("osu statuses: unreadable answer: {e}");
                SLOW_DOWN
            })?;
            Ok(a.beatmaps.into_iter().map(|m| (m.id, m.beatmapset_id, m.status)).collect())
        }
        code => {
            eprintln!("osu statuses: the API answered {code}");
            Err(SLOW_DOWN)
        }
    }
}

/// The sets of a batch whose map osu! doesn't have. The pairs come from players: a map it does
/// have, under some other set, says nothing about that set.
fn unanswered(batch: &[(u32, u32)], found: &[(u32, u32, String)]) -> Vec<u32> {
    batch.iter().filter(|(set, map)| !found.iter().any(|(m, s, _)| m == map || s == set)).map(|(set, _)| *set).collect()
}

fn worker() {
    let Some(s) = STATUS.get() else { return };
    let agent = agent();
    let mut last_save = Instant::now();
    let mut batches = 0u32;
    loop {
        let batch: Vec<(u32, u32)> = {
            let mut inner = s.inner.lock().unwrap();
            while inner.queue.is_empty() {
                s.save(&mut inner);
                inner = s.wake.wait(inner).unwrap();
            }
            let n = inner.queue.len().min(BATCH);
            inner.queue.drain(..n).collect()
        };
        let Some(token) = token(s, &agent) else {
            requeue(s, &batch);
            std::thread::sleep(SLOW_DOWN);
            continue;
        };
        match ask(&agent, &token, &batch.iter().map(|&(_, m)| m).collect::<Vec<_>>()) {
            Ok(found) => {
                let mut inner = s.inner.lock().unwrap();
                let at = now();
                // A map osu! doesn't have: its set is looked up again in a while.
                for set in unanswered(&batch, &found) {
                    if inner.known.get(&set).is_none_or(|(status, _)| status == MISSING) {
                        inner.known.insert(set, (MISSING.to_string(), at));
                    }
                }
                for (_, set, status) in found {
                    inner.known.insert(set, (status, at));
                }
                for (set, _) in &batch {
                    inner.queued.remove(set);
                }
                inner.dirty = true;
                batches += 1;
                if batches % 20 == 0 {
                    println!("osu statuses: {} sets known, {} waiting", inner.known.len(), inner.queue.len());
                }
                if last_save.elapsed() >= Duration::from_secs(30) {
                    s.save(&mut inner);
                    last_save = Instant::now();
                }
                drop(inner);
                std::thread::sleep(PACE);
            }
            Err(wait) => {
                // The token may have been the trouble: ask for a new one.
                s.inner.lock().unwrap().token = None;
                requeue(s, &batch);
                std::thread::sleep(wait);
            }
        }
    }
}

fn requeue(s: &Status, batch: &[(u32, u32)]) {
    let mut inner = s.inner.lock().unwrap();
    for &item in batch.iter().rev() {
        inner.queue.push_front(item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_map_of_another_set_doesnt_make_a_set_missing() {
        let found = vec![(10, 1, "ranked".to_string()), (20, 2, "ranked".to_string())];
        let batch = [(1, 10), (2, 11), (3, 20), (4, 30)];
        assert_eq!(unanswered(&batch, &found), [4]);
    }

    #[test]
    fn statuses_that_may_change_are_asked_again_sooner() {
        assert!(!due("ranked", 100, 100 + 59 * 86_400) && due("ranked", 100, 100 + 61 * 86_400));
        assert!(!due("pending", 100, 100 + 86_400) && due("pending", 100, 100 + 2 * 86_400));
        assert!(due(MISSING, 100, 100 + 15 * 86_400));
    }

    #[test]
    fn nothing_is_answered_without_the_service() {
        assert!(lookup(&[(1, 2)], true).is_empty());
    }
}
