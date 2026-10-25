//! The online leaderboards: every play sent in, one JSON line each in DIR/scores.jsonl (kept in
//! memory too), and its presses in DIR/replays/<id>.json.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::online::{self, OnlineScore, Profile, ScoreSubmit, Stats};

/// Plays one account may send per minute.
const SUBMITS_PER_MINUTE: usize = 20;
/// Plays a profile lists (best and latest).
const PROFILE_PLAYS: usize = 20;
/// Most played maps a profile lists.
const PROFILE_MAPS: usize = 6;

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
        let mut all = all;
        // Lines from before scores carried their id.
        for s in &mut all {
            s.score.id = s.id;
        }
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
                great: play.great,
                ok: play.ok,
                set_id: m.set_id,
                stars: play.stars.filter(|s| s.is_finite() && (0.0..50.0).contains(s)),
                id,
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

    /// The presses of score `id`, as they were sent in.
    pub fn replay(&self, id: u64) -> Option<online::OnlineReplay> {
        let dir = self.replays.as_ref()?;
        self.all.iter().any(|s| s.id == id).then_some(())?;
        serde_json::from_slice(&std::fs::read(dir.join(format!("{id}.json"))).ok()?).ok()
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

    /// Everyone with pp, highest first, with osu!'s ranking columns.
    pub fn rankings(&self, limit: usize) -> Vec<Ranked> {
        let mut per: HashMap<&str, Vec<&Stored>> = HashMap::new();
        for s in &self.all {
            per.entry(s.player.as_str()).or_default().push(s);
        }
        let mut out: Vec<Ranked> = per
            .values()
            .filter_map(|plays| {
                let (stats, pp) = stats_of(plays);
                (pp > 0.0).then(|| Ranked {
                    rank: 0,
                    name: plays.last().map(|s| s.score.player.clone()).unwrap_or_default(),
                    pp,
                    accuracy: stats.accuracy,
                    plays: plays.len() as u32,
                    ss: stats.ss,
                    s: stats.s,
                    a: stats.a,
                })
            })
            .collect();
        out.sort_by(|a, b| b.pp.total_cmp(&a.pp).then_with(|| a.name.cmp(&b.name)));
        out.truncate(limit);
        for (i, r) in out.iter_mut().enumerate() {
            r.rank = i as u32 + 1;
        }
        out
    }

    /// Plays sent, all told.
    pub fn play_count(&self) -> usize {
        self.all.len()
    }

    /// A map someone has played: its name, cover, stars, and how much it's played.
    pub fn map(&self, map: u64) -> Option<MapRow> {
        self.maps(usize::MAX).into_iter().find(|m| m.hash == map)
    }

    /// The maps played, the most lately played first.
    pub fn maps(&self, limit: usize) -> Vec<MapRow> {
        let mut rows: HashMap<u64, (MapRow, Vec<&str>)> = HashMap::new();
        for s in &self.all {
            let sc = &s.score;
            let (row, players) = rows.entry(sc.map_hash).or_insert_with(|| {
                (MapRow { hash: sc.map_hash, name: sc.map.clone(), set_id: None, stars: None, plays: 0, players: 0, last: 0 }, Vec::new())
            });
            row.plays += 1;
            row.last = row.last.max(sc.date);
            row.set_id = sc.set_id.or(row.set_id);
            // Stars at 1.0x, if anyone played it so.
            if (sc.rate - 1.0).abs() < 1e-6 {
                row.stars = sc.stars.or(row.stars);
            }
            if !players.contains(&s.player.as_str()) {
                players.push(&s.player);
            }
        }
        let mut out: Vec<MapRow> = rows.into_values().map(|(mut r, p)| {
            r.players = p.len() as u32;
            r
        }).collect();
        out.sort_by(|a, b| b.last.cmp(&a.last));
        out.truncate(limit);
        out
    }

    /// A player's pp total and rank among everyone with pp.
    fn standing(&self, player: &str) -> (f64, Option<u32>) {
        let totals = self.totals();
        let Some(&mine) = totals.get(player) else { return (0.0, None) };
        let above = totals.values().filter(|&&pp| pp > mine).count();
        (mine, Some(above as u32 + 1))
    }

    /// The latest plays by anyone, newest first.
    pub fn latest(&self, limit: usize) -> Vec<OnlineScore> {
        self.all.iter().rev().take(limit).map(|s| s.score.clone()).collect()
    }

    /// `player`'s pp history, most played maps and mods (see `Extras`).
    pub fn extras(&self, player: &str) -> Extras {
        let mut mine: Vec<&Stored> = self.all.iter().filter(|s| s.player == player).collect();
        mine.sort_by_key(|s| (s.score.date, s.id));
        let mut best: HashMap<u64, f64> = HashMap::new();
        let mut history: Vec<(u64, f64)> = Vec::new();
        for s in &mine {
            let Some(pp) = s.score.pp else { continue };
            let b = best.entry(s.score.map_hash).or_insert(0.0);
            if pp > *b {
                *b = pp;
                history.push((s.score.date, online::weighted_pp(best.values().copied().collect())));
            }
        }
        let mut count: HashMap<u64, u32> = HashMap::new();
        let mut mods: HashMap<&str, u32> = HashMap::new();
        for s in &mine {
            *count.entry(s.score.map_hash).or_default() += 1;
            for m in &s.score.mods {
                *mods.entry(m.as_str()).or_default() += 1;
            }
        }
        let rows = self.maps(usize::MAX);
        let mut most_played: Vec<(MapRow, u32)> = rows.into_iter().filter_map(|r| count.get(&r.hash).map(|&n| (r, n))).collect();
        most_played.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.last.cmp(&a.0.last)));
        most_played.truncate(PROFILE_MAPS);
        let mut mods: Vec<(String, u32)> = mods.into_iter().map(|(m, n)| (m.to_string(), n)).collect();
        mods.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Extras { history, most_played, mods }
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
        let (stats, _) = stats_of(&mine);
        // First places: maps where theirs is the best score.
        let mut maps: Vec<u64> = mine.iter().map(|s| s.score.map_hash).collect();
        maps.sort_unstable();
        maps.dedup();
        let mut firsts: Vec<OnlineScore> =
            maps.into_iter().filter_map(|m| self.best_on(m).first().filter(|s| s.player == player).map(|s| s.score.clone())).collect();
        firsts.sort_by(|a, b| b.date.cmp(&a.date));
        firsts.truncate(PROFILE_PLAYS);
        Profile { name: name.to_string(), joined, plays: mine.len() as u32, pp, rank, top, recent, stats, firsts, online: false }
    }
}

/// What a profile page shows beyond `Profile`: how their pp grew, the maps they keep coming
/// back to and the mods they play with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extras {
    /// (when, pp total right after that play), one point per play that changed it.
    pub history: Vec<(u64, f64)>,
    /// Their most played maps, with how many times.
    pub most_played: Vec<(MapRow, u32)>,
    /// Mods by how many of their plays had them on.
    pub mods: Vec<(String, u32)>,
}

/// A row of the rankings.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Ranked {
    pub rank: u32,
    pub name: String,
    pub pp: f64,
    pub accuracy: f64,
    pub plays: u32,
    pub ss: u32,
    pub s: u32,
    pub a: u32,
}

/// A map someone has played.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct MapRow {
    pub hash: u64,
    /// "Artist - Title [Version]".
    pub name: String,
    pub set_id: Option<u32>,
    /// At 1.0x (None if nobody played it so, or older clients).
    pub stars: Option<f64>,
    pub plays: u32,
    pub players: u32,
    /// When it was last played (Unix seconds).
    pub last: u64,
}

/// One player's totals from their plays, and their pp total.
fn stats_of(plays: &[&Stored]) -> (Stats, f64) {
    let mut st = Stats::default();
    // Best score and best pp on each map.
    let mut by_score: HashMap<u64, &OnlineScore> = HashMap::new();
    let mut by_pp: HashMap<u64, &OnlineScore> = HashMap::new();
    for s in plays {
        let sc = &s.score;
        st.total_score += sc.score;
        st.total_hits += (sc.great + sc.ok) as u64;
        st.max_combo = st.max_combo.max(sc.max_combo);
        let b = by_score.entry(sc.map_hash).or_insert(sc);
        if sc.score > b.score {
            *b = sc;
        }
        if sc.pp.is_some() {
            let b = by_pp.entry(sc.map_hash).or_insert(sc);
            if sc.pp > b.pp {
                *b = sc;
            }
        }
    }
    for sc in by_score.values() {
        st.ranked_score += sc.score;
        match sc.grade() {
            "SS" => st.ss += 1,
            "S" => st.s += 1,
            "A" => st.a += 1,
            _ => {}
        }
    }
    // Accuracy as osu! has it: the best plays' (by pp), each counting 5% less than the last.
    let mut best: Vec<&OnlineScore> = if by_pp.is_empty() { by_score.into_values().collect() } else { by_pp.into_values().collect() };
    best.sort_by(|a, b| b.pp.unwrap_or(0.0).total_cmp(&a.pp.unwrap_or(0.0)));
    let (mut sum, mut weight) = (0.0, 0.0);
    for (i, sc) in best.iter().enumerate() {
        let w = 0.95f64.powi(i as i32);
        sum += sc.accuracy * w;
        weight += w;
    }
    st.accuracy = if weight > 0.0 { sum / weight } else { 0.0 };
    let pp = online::weighted_pp(best.iter().filter_map(|s| s.pp).collect());
    (st, pp)
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
            stars: Some(5.0),
        }
    }

    #[test]
    fn extras_follow_the_pp_total_and_count_maps_and_mods() {
        let mut s = Scores::load(None);
        s.submit("a", "A", play(7, 500, 100.0), 1).unwrap();
        s.submit("a", "A", play(7, 400, 50.0), 2).unwrap();
        s.submit("a", "A", play(8, 300, 100.0), 3).unwrap();
        s.submit("b", "B", play(7, 900, 200.0), 4).unwrap();
        let e = s.extras("a");
        // The worse play on map 7 didn't move the total.
        assert_eq!(e.history.len(), 2);
        assert!((e.history[1].1 - (100.0 + 95.0)).abs() < 1e-9);
        assert_eq!(e.most_played.iter().map(|(m, n)| (m.hash, *n)).collect::<Vec<_>>(), [(7, 2), (8, 1)]);
        assert_eq!(e.mods, [("HD".to_string(), 3)]);
        assert_eq!(s.latest(2).iter().map(|p| p.player.as_str()).collect::<Vec<_>>(), ["B", "A"]);
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
        assert_eq!(ranks.iter().map(|r| (r.name.as_str(), r.plays, r.rank)).collect::<Vec<_>>(), [("A", 3, 1), ("B", 1, 2)]);
        assert_eq!(s.map(7).map(|m| (m.name, m.plays, m.players)), Some(("A - T [Oni]".into(), 3, 2)));
        assert_eq!((p.stats.ranked_score, p.stats.total_score, p.firsts.len()), (500 + 100, 500 + 400 + 100, 1));
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
        assert_eq!(again.map_scores(7)[0].id, 1, "scores carry their id to ask for the replay by");
        let r = again.replay(1).unwrap();
        assert_eq!((r.version, r.inputs.clone()), (2, vec![(1000.0, 1)]));
        assert!(again.replay(99).is_none());
        let again = Scores::load(Some(dir.clone()));
        assert_eq!(again.map_scores(7).len(), 1);
        assert_eq!(again.next_id, 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
