//! The online leaderboards: every play sent in, one JSON line each in DIR/scores.jsonl (kept in
//! memory too), and its presses in DIR/replays/<id>.json.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::online::{self, OnlineScore, Profile, ScoreSubmit};

/// Plays one account may send per minute.
const SUBMITS_PER_MINUTE: usize = 20;
/// Plays a profile lists (best and latest).
const PROFILE_PLAYS: usize = 20;

#[derive(Serialize, Deserialize, Clone)]
struct Stored {
    id: u64,
    /// Lowercased account name.
    player: String,
    score: OnlineScore,
}

#[derive(Serialize)]
struct StoredReplay<'a> {
    version: u32,
    map: &'a crate::net::MapInfo,
    inputs: &'a [(f64, u8)],
}

pub struct Scores {
    file: Option<PathBuf>,
    replays: Option<PathBuf>,
    all: Vec<Stored>,
    next_id: u64,
    submits: HashMap<String, Vec<Instant>>,
}

/// What's wrong with a play sent in, if anything.
fn check(play: &ScoreSubmit) -> Result<(), &'static str> {
    let hits = play.great as u64 + play.ok as u64 + play.miss as u64;
    let fine = (0.0..=1.0).contains(&play.accuracy)
        && (1..=100_000).contains(&hits)
        && play.max_combo as u64 <= hits
        && (0.25..=4.0).contains(&play.map.rate)
        && play.pp.is_none_or(|pp| pp.is_finite() && (0.0..=5000.0).contains(&pp))
        && play.inputs.len() <= 400_000
        && play.map.hash != 0;
    if !fine {
        return Err("that play doesn't add up");
    }
    if play.map.mods.iter().any(|m| m == "AT" || m == "RX") {
        return Err("Autoplay and Relax plays don't go on the leaderboards");
    }
    Ok(())
}

impl Scores {
    /// Reads DIR/scores.jsonl (`dir` None: in memory only, for tests).
    pub fn load(dir: Option<PathBuf>) -> Scores {
        let file = dir.as_ref().map(|d| d.join("scores.jsonl"));
        let all: Vec<Stored> = file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|text| text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
            .unwrap_or_default();
        let next_id = all.iter().map(|s| s.id + 1).max().unwrap_or(1);
        if let Some(f) = &file {
            println!("{} scores ({})", all.len(), f.display());
        }
        Scores { file, replays: dir.map(|d| d.join("replays")), all, next_id, submits: HashMap::new() }
    }

    /// Takes a play by `player` (lowercased key, `name` as written). Returns its place on the map
    /// (if it's their best there), and their pp total and rank.
    pub fn submit(&mut self, player: &str, name: &str, play: ScoreSubmit, now: u64) -> Result<(Option<u32>, f64, Option<u32>), String> {
        check(&play)?;
        let times = self.submits.entry(player.to_string()).or_default();
        times.retain(|t| t.elapsed() < Duration::from_secs(60));
        if times.len() >= SUBMITS_PER_MINUTE {
            return Err("too many plays at once, slow down".into());
        }
        times.push(Instant::now());
        let m = &play.map;
        let label = online::clean_text(&format!("{} - {} [{}]", m.artist, m.title, m.version), 160);
        let id = self.next_id;
        self.next_id += 1;
        let stored = Stored {
            id,
            player: player.to_string(),
            score: OnlineScore {
                player: name.to_string(),
                map: label,
                map_hash: m.hash,
                score: play.score,
                accuracy: play.accuracy,
                max_combo: play.max_combo,
                miss: play.miss,
                rate: m.rate,
                mods: m.mods.iter().map(|a| online::clean_text(a, 4)).take(40).collect(),
                pp: play.pp,
                date: now,
            },
        };
        if let Some(dir) = &self.replays {
            let replay = StoredReplay { version: play.replay_version, map: &play.map, inputs: &play.inputs };
            let saved = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(dir.join(format!("{id}.json")), serde_json::to_vec(&replay).unwrap_or_default()));
            if let Err(e) = saved {
                eprintln!("saving replay {id}: {e}");
            }
        }
        if let Some(file) = &self.file {
            let line = serde_json::to_string(&stored).map_err(|e| e.to_string())? + "\n";
            let written = std::fs::OpenOptions::new().create(true).append(true).open(file).and_then(|mut f| f.write_all(line.as_bytes()));
            if let Err(e) = written {
                eprintln!("saving score {id}: {e}");
                return Err("the server couldn't save it".into());
            }
        }
        let map = stored.score.map_hash;
        self.all.push(stored);
        let board = self.best_on(map);
        let map_rank = board.iter().position(|s| s.id == id).map(|p| p as u32 + 1);
        let (pp, rank) = self.standing(player);
        Ok((map_rank, pp, rank))
    }

    /// Each player's best score (by score) on `map`, highest first.
    fn best_on(&self, map: u64) -> Vec<&Stored> {
        let mut best: HashMap<&str, &Stored> = HashMap::new();
        for s in self.all.iter().filter(|s| s.score.map_hash == map) {
            let b = best.entry(&s.player).or_insert(s);
            if s.score.score > b.score.score {
                *b = s;
            }
        }
        let mut v: Vec<&Stored> = best.into_values().collect();
        v.sort_by(|a, b| b.score.score.cmp(&a.score.score).then(a.id.cmp(&b.id)));
        v
    }

    pub fn map_scores(&self, map: u64) -> Vec<OnlineScore> {
        self.best_on(map).into_iter().take(online::MAP_SCORES).map(|s| s.score.clone()).collect()
    }

    /// Everyone's pp total, by player key.
    fn totals(&self) -> HashMap<&str, f64> {
        let mut best: HashMap<(&str, u64), f64> = HashMap::new();
        for s in &self.all {
            if let Some(pp) = s.score.pp {
                let b = best.entry((s.player.as_str(), s.score.map_hash)).or_insert(0.0);
                *b = b.max(pp);
            }
        }
        let mut per: HashMap<&str, Vec<f64>> = HashMap::new();
        for ((p, _), pp) in best {
            per.entry(p).or_default().push(pp);
        }
        per.into_iter().map(|(p, v)| (p, online::weighted_pp(v))).collect()
    }

    /// Everyone with pp, highest first: (name as written, pp total, plays).
    pub fn rankings(&self, limit: usize) -> Vec<(String, f64, u32)> {
        let mut names: HashMap<&str, (&str, u32)> = HashMap::new();
        for s in &self.all {
            let e = names.entry(s.player.as_str()).or_insert((s.score.player.as_str(), 0));
            *e = (s.score.player.as_str(), e.1 + 1);
        }
        let mut out: Vec<(String, f64, u32)> = self
            .totals()
            .into_iter()
            .filter(|&(_, pp)| pp > 0.0)
            .map(|(p, pp)| {
                let (name, plays) = names.get(p).copied().unwrap_or((p, 0));
                (name.to_string(), pp, plays)
            })
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out.truncate(limit);
        out
    }

    /// The name of the map with this hash, from a play on it.
    pub fn map_name(&self, map: u64) -> Option<String> {
        self.all.iter().find(|s| s.score.map_hash == map).map(|s| s.score.map.clone())
    }

    /// A player's pp total and rank among everyone with pp.
    fn standing(&self, player: &str) -> (f64, Option<u32>) {
        let totals = self.totals();
        let Some(&mine) = totals.get(player) else { return (0.0, None) };
        let above = totals.values().filter(|&&pp| pp > mine).count();
        (mine, Some(above as u32 + 1))
    }

    /// `player`'s profile (`name` as written, `joined` when the account was made).
    pub fn profile(&self, player: &str, name: &str, joined: u64) -> Profile {
        let mine: Vec<&Stored> = self.all.iter().filter(|s| s.player == player).collect();
        // Best pp per map.
        let mut best: HashMap<u64, &Stored> = HashMap::new();
        for s in mine.iter().filter(|s| s.score.pp.is_some()) {
            let b = best.entry(s.score.map_hash).or_insert(s);
            if s.score.pp > b.score.pp {
                *b = s;
            }
        }
        let mut top: Vec<OnlineScore> = best.into_values().map(|s| s.score.clone()).collect();
        top.sort_by(|a, b| b.pp.unwrap_or(0.0).total_cmp(&a.pp.unwrap_or(0.0)));
        top.truncate(PROFILE_PLAYS);
        let recent = mine.iter().rev().take(PROFILE_PLAYS).map(|s| s.score.clone()).collect();
        let (pp, rank) = self.standing(player);
        Profile { name: name.to_string(), joined, plays: mine.len() as u32, pp, rank, top, recent }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(map: u64, score: u64, pp: f64) -> ScoreSubmit {
        ScoreSubmit {
            map: crate::net::MapInfo { hash: map, artist: "A".into(), title: "T".into(), version: "Oni".into(), rate: 1.0, from: None, set_id: None, mods: vec!["HD".into()] },
            score,
            accuracy: 0.95,
            max_combo: 100,
            great: 90,
            ok: 8,
            miss: 2,
            pp: Some(pp),
            date: 0,
            replay_version: 2,
            inputs: vec![(1000.0, 1)],
        }
    }

    #[test]
    fn leaderboards_keep_each_players_best() {
        let mut s = Scores::load(None);
        assert_eq!(s.submit("a", "A", play(7, 500, 100.0), 1).unwrap(), (Some(1), 100.0, Some(1)));
        assert_eq!(s.submit("b", "B", play(7, 900, 200.0), 2).unwrap(), (Some(1), 200.0, Some(1)));
        // A worse play than their best isn't on the board.
        assert_eq!(s.submit("a", "A", play(7, 400, 50.0), 3).unwrap().0, None);
        let board = s.map_scores(7);
        assert_eq!(board.iter().map(|r| (r.player.as_str(), r.score)).collect::<Vec<_>>(), [("B", 900), ("A", 500)]);
        // Another map adds to A's total, weighted.
        let (_, pp, rank) = s.submit("a", "A", play(8, 100, 150.0), 4).unwrap();
        assert!((pp - (150.0 + 95.0)).abs() < 1e-9 && rank == Some(1));
        let p = s.profile("a", "A", 0);
        assert_eq!((p.plays, p.rank, p.top.len()), (3, Some(1), 2));
        assert_eq!(p.top[0].pp, Some(150.0));
        assert_eq!(p.recent[0].map_hash, 8);
        let ranks = s.rankings(10);
        assert_eq!(ranks.iter().map(|r| (r.0.as_str(), r.2)).collect::<Vec<_>>(), [("A", 3), ("B", 1)]);
        assert_eq!(s.map_name(7).as_deref(), Some("A - T [Oni]"));
    }

    #[test]
    fn odd_plays_are_turned_away() {
        let mut s = Scores::load(None);
        let mut p = play(7, 1, 1.0);
        p.accuracy = 1.5;
        assert!(s.submit("a", "A", p, 0).is_err());
        let mut p = play(7, 1, 1.0);
        p.map.mods = vec!["AT".into()];
        assert!(s.submit("a", "A", p, 0).is_err());
        for _ in 0..SUBMITS_PER_MINUTE {
            s.submit("c", "C", play(7, 1, 1.0), 0).unwrap();
        }
        assert!(s.submit("c", "C", play(7, 1, 1.0), 0).is_err(), "slowed down");
    }

    #[test]
    fn scores_survive_a_restart() {
        let dir = std::env::temp_dir().join(format!("taiko-server-scores-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Scores::load(Some(dir.clone()));
        s.submit("a", "A", play(7, 500, 100.0), 1).unwrap();
        assert!(dir.join("replays/1.json").is_file());
        let again = Scores::load(Some(dir.clone()));
        assert_eq!(again.map_scores(7).len(), 1);
        assert_eq!(again.next_id, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
