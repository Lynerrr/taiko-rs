//! Replays and the score index's rows, as data: what a replay file holds and how a play is
//! re-played from it, and what a line of the index says. Reading and writing the files is the
//! client's (`src/scores.rs`); this works on their bytes and lines.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::beatmap::Beatmap;
use crate::mods::{HalfRule, Mods};
use crate::pp;
use crate::sim::{Game, Mash, Scoring, Settings, Song, Stats, WindowRule, Windows};

/// 5: judged with osu!'s hit windows, whole ms less a half (below: the OD's exact widths, see
/// `WindowRule`). 4: Half by `HalfRule::OnTheBeat`. 3: scored with `Scoring::Standard` (below:
/// `Classic`). 2: judged with `Mash::Nearest` (1: with the `anti_mash` setting).
const REPLAY_VERSION: u32 = 5;

/// A recorded play: the map time each drum key was judged at, plus what the judging depended
/// on. Playing the inputs back through the same map reproduces the play exactly.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Replay {
    pub version: u32,
    /// Hash of the .osu file (as multiplayer uses) and where it was.
    pub map_hash: u64,
    pub map_path: String,
    pub map_label: String,
    pub player: String,
    /// Unix seconds.
    pub date: u64,
    pub rate: f64,
    #[serde(default)]
    pub keep_pitch: bool,
    /// Version 1 only: the mash penalty setting the play had (see `mash`).
    pub anti_mash: bool,
    /// Mods played with (acronyms); `rate` already includes DT/HT.
    #[serde(default)]
    pub mods: Vec<String>,
    /// The scroll speed setting played at (replays from before it: the usual).
    #[serde(default = "usual_scroll_speed")]
    pub scroll_speed: f64,
    pub score: u64,
    pub accuracy: f64,
    pub max_combo: u32,
    pub great: u32,
    pub ok: u32,
    pub miss: u32,
    pub ur: f64,
    /// (map time ms, key 0-3: left rim, left centre, right centre, right rim).
    pub inputs: Vec<(f64, u8)>,
    /// Random's colouring (`Game::seed`; 0 for plays from before it).
    #[serde(default)]
    pub seed: u64,
}

/// One line of the score index.
#[derive(Clone, Debug)]
pub struct ScoreRow {
    pub map_hash: u64,
    pub date: u64,
    pub score: u64,
    pub accuracy: f64,
    pub max_combo: u32,
    pub miss: u32,
    pub rate: f64,
    pub player: String,
    pub file: PathBuf,
    /// Mods, as "HDHR" ("" without).
    pub mods: String,
    /// Performance points (None: not worked out yet).
    pub pp: Option<f64>,
}

impl ScoreRow {
    /// Its line in the index.
    pub fn line(&self) -> String {
        let pp = self.pp.map(|p| format!("{p:.2}")).unwrap_or_default();
        let name = self
            .file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        format!(
            "{:016x}\t{}\t{}\t{:.6}\t{}\t{}\t{:.2}\t{}\t{}\t{}\t{pp}\n",
            self.map_hash,
            self.date,
            self.score,
            self.accuracy,
            self.max_combo,
            self.miss,
            self.rate,
            self.player,
            name,
            self.mods
        )
    }

    /// A line of the index, its replay file in `base`. None if the line isn't one.
    pub fn parse(line: &str, base: &Path) -> Option<ScoreRow> {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 9 {
            return None;
        }
        Some(ScoreRow {
            map_hash: u64::from_str_radix(f[0], 16).ok()?,
            date: f[1].parse().ok()?,
            score: f[2].parse().ok()?,
            accuracy: f[3].parse().ok()?,
            max_combo: f[4].parse().ok()?,
            miss: f[5].parse().ok()?,
            rate: f[6].parse().ok()?,
            player: f[7].to_string(),
            file: base.join(f[8]),
            mods: f.get(9).unwrap_or(&"").to_string(),
            pp: f.get(10).and_then(|p| p.parse().ok()),
        })
    }

    /// The row of a replay saved as `file`, with its pp.
    pub fn of(replay: &Replay, file: PathBuf, pp: Option<f64>) -> ScoreRow {
        ScoreRow {
            map_hash: replay.map_hash,
            date: replay.date,
            score: replay.score,
            accuracy: replay.accuracy,
            max_combo: replay.max_combo,
            miss: replay.miss,
            rate: replay.rate,
            player: index_name(&replay.player),
            file,
            mods: Mods::from_list(&replay.mods).label(),
            pp,
        }
    }
}

impl Replay {
    /// What pp needs to know about the play.
    pub fn pp_play(&self) -> pp::Play {
        pp::Play {
            great: self.great,
            ok: self.ok,
            miss: self.miss,
            max_combo: self.max_combo,
            rate: self.rate,
            mods: Mods::from_list(&self.mods),
            scroll_speed: self.scroll_speed,
            seed: self.seed,
            half: self.half_rule(),
        }
    }

    /// A replay file's contents. Fails on anything else, and on a replay from a newer version.
    pub fn from_json(bytes: &[u8]) -> Result<Replay, String> {
        let r: Replay = serde_json::from_slice(bytes).map_err(|e| format!("not a replay: {e}"))?;
        if r.version > REPLAY_VERSION {
            return Err("replay from a newer version".into());
        }
        Ok(r)
    }

    /// What a replay file holds.
    pub fn to_json(&self) -> Result<Vec<u8>, String> {
        serde_json::to_vec(self).map_err(|e| e.to_string())
    }
}

fn usual_scroll_speed() -> f64 {
    1.0
}

/// A player name as the index can hold it (a tab or a line break would split the line).
pub fn index_name(name: &str) -> String {
    name.chars().filter(|c| *c != '\t' && *c != '\n').collect()
}

/// Puts `from`'s rows under `to` and returns their replay files.
pub fn rename_rows(rows: &mut [ScoreRow], from: &str, to: &str) -> Vec<PathBuf> {
    let (from, to) = (index_name(from), index_name(to));
    rows.iter_mut()
        .filter(|r| r.player == from)
        .map(|r| {
            r.player = to.clone();
            r.file.clone()
        })
        .collect()
}

impl Replay {
    /// How the play's presses are judged.
    pub fn mash(&self) -> Mash {
        if self.version >= 2 {
            Mash::Nearest
        } else {
            Mash::legacy(self.anti_mash)
        }
    }

    /// Which notes Half took off.
    pub fn half_rule(&self) -> HalfRule {
        if self.version >= 4 {
            HalfRule::OnTheBeat
        } else {
            HalfRule::EveryOther
        }
    }

    /// How wide its hit windows were.
    pub fn window_rule(&self) -> WindowRule {
        if self.version >= 5 {
            WindowRule::Osu
        } else {
            WindowRule::Exact
        }
    }

    /// How its score was worked out.
    pub fn scoring(&self) -> Scoring {
        if self.version >= 3 {
            Scoring::Standard
        } else {
            Scoring::Classic
        }
    }
}

/// A replay game with no audio, at its start, with the player's `settings`.
pub fn replay_game(map: &Beatmap, track: Arc<dyn Song>, r: &Replay, settings: &Settings) -> Game {
    let mut g = Game::with_half(
        map.clone(),
        track,
        None,
        Mods::from_list(&r.mods),
        r.seed,
        r.half_rule(),
    )
    .with_settings(settings);
    g.set_replay(Some((r.inputs.clone(), 0)));
    g.set_mash(r.mash());
    g.set_scoring(r.scoring());
    g.set_window_rule(r.window_rule());
    g.set_mute_hits(true);
    g
}

/// No sound, at the rate a replay was played (DT, HT): what `simulate` keeps time to.
struct Silent(f64);

impl Song for Silent {
    fn frames(&self) -> usize {
        0
    }

    fn rate(&self) -> f64 {
        self.0
    }

    fn into_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }
}

/// Plays a replay through its map, silently, for everything the results screen shows that the
/// replay doesn't keep (hit errors, drumrolls, big notes) and the hit windows, with the
/// player's `settings`.
pub fn simulate(map: &Beatmap, r: &Replay, settings: &Settings) -> (Stats, Windows) {
    let mut g = replay_game(map, Arc::new(Silent(r.rate)), r, settings);
    // Frame by frame, as it was played.
    let end = g.end_time() + 2000.0;
    let mut t = g.time();
    while t < end {
        t = (t + 8.0).min(end);
        g.set_time(t);
    }
    (g.stats().clone(), g.windows())
}

pub fn new_version() -> u32 {
    REPLAY_VERSION
}

/// The best scores on a map, highest first.
impl ScoreRow {
    /// A Relax play: on boards of its own. `mods` is a `Mods::label`, acronyms with nothing
    /// between them, each two letters long.
    fn relax(&self) -> bool {
        self.mods.as_bytes().chunks(2).any(|m| m == b"RX")
    }
}

pub fn top(rows: &[ScoreRow], map_hash: u64, n: usize, relax: bool) -> Vec<&ScoreRow> {
    let mut v: Vec<&ScoreRow> = rows
        .iter()
        .filter(|r| r.map_hash == map_hash && r.relax() == relax)
        .collect();
    v.sort_by(|a, b| b.score.cmp(&a.score).then(b.date.cmp(&a.date)));
    v.truncate(n);
    v
}

#[cfg(test)]
mod board_tests {
    use super::*;

    fn row(map_hash: u64, score: u64, mods: &str) -> ScoreRow {
        ScoreRow {
            map_hash,
            date: 0,
            score,
            accuracy: 1.0,
            max_combo: 1,
            miss: 0,
            rate: 1.0,
            player: "p".into(),
            file: PathBuf::new(),
            mods: mods.into(),
            pp: None,
        }
    }

    #[test]
    fn renaming_moves_only_that_players_rows() {
        let mut rows = vec![row(1, 10, ""), row(2, 20, ""), row(3, 30, "")];
        rows[1].player = "friend".into();
        rows[2].file = PathBuf::from("c.json");
        let files = rename_rows(&mut rows, "p", "new\tname");
        assert_eq!(files, vec![PathBuf::new(), PathBuf::from("c.json")]);
        let names: Vec<&str> = rows.iter().map(|r| r.player.as_str()).collect();
        assert_eq!(names, ["newname", "friend", "newname"]);
        assert!(rename_rows(&mut rows, "nobody", "x").is_empty());
    }

    #[test]
    fn a_maps_local_board_keeps_relax_plays_apart() {
        let rows = [
            row(7, 1, ""),
            row(7, 2, "HDRX"),
            row(7, 3, "RX"),
            row(8, 4, "RX"),
        ];
        let scores = |relax| -> Vec<u64> {
            top(&rows, 7, usize::MAX, relax)
                .iter()
                .map(|r| r.score)
                .collect()
        };
        assert_eq!(scores(false), [1]);
        assert_eq!(scores(true), [3, 2]);
    }

    #[test]
    fn relax_is_read_by_whole_acronyms() {
        // An R ending one acronym and an X starting the next isn't Relax.
        assert!(!row(7, 1, "PRXA").relax());
        assert!(row(7, 1, "PRRX").relax());
    }
}

#[cfg(test)]
mod replay_tests {
    use super::*;
    use crate::beatmap;

    /// The replay of a play of the calibration map (OD 0: GREAT within 50 ms, OK within 120),
    /// dons hit `err` ms off by turns half as late, four times as late, 1.5 and 4.5 times as
    /// early, every fifth let go by; with what the play judged them as.
    fn played(err: f64) -> (Beatmap, Replay, Stats) {
        let map = beatmap::calibration_map(500.0, 0, 64);
        let mut g = Game::new(map.clone(), Arc::new(Silent(1.0)), None);
        g.set_mash(Mash::Nearest);
        g.set_scoring(Scoring::Standard);
        g.set_window_rule(WindowRule::Osu);
        let times: Vec<f64> = g.map().notes.iter().map(|n| n.time).collect();
        let mut inputs = Vec::new();
        for (i, t) in times.into_iter().enumerate() {
            // Every fifth one let go by.
            if i % 5 != 4 {
                let at = t + err * [0.5, 4.0, -1.5, -4.5][i % 4];
                g.press_at(1, at, None);
                inputs.push((at, 1));
            }
        }
        let end = g.end_time() + 2000.0;
        g.set_time(end);
        let s = g.stats().clone();
        let replay = Replay {
            version: new_version(),
            map_hash: 1,
            map_path: String::new(),
            map_label: String::new(),
            player: "p".into(),
            date: 0,
            rate: 1.0,
            keep_pitch: false,
            anti_mash: false,
            mods: Vec::new(),
            scroll_speed: 1.0,
            score: s.score,
            accuracy: 0.0,
            max_combo: s.max_combo,
            great: s.great,
            ok: s.ok,
            miss: s.miss,
            ur: 0.0,
            inputs,
            seed: g.seed(),
        };
        (map, replay, s)
    }

    #[test]
    fn a_replay_read_back_plays_the_same_play_again() {
        let (map, replay, played) = played(20.0);
        assert!(played.great > 0 && played.ok > 0 && played.miss > 0);
        let back = Replay::from_json(&replay.to_json().unwrap()).unwrap();
        let (again, _) = simulate(&map, &back, &Settings::default());
        assert_eq!(
            (
                again.great,
                again.ok,
                again.miss,
                again.max_combo,
                again.score
            ),
            (
                played.great,
                played.ok,
                played.miss,
                played.max_combo,
                played.score
            )
        );
        assert_eq!(again.errors, played.errors);
    }

    #[test]
    fn a_replay_from_a_newer_version_is_not_read() {
        let (_, mut replay, _) = played(0.0);
        replay.version = new_version() + 1;
        let err = Replay::from_json(&replay.to_json().unwrap()).unwrap_err();
        assert_eq!(err, "replay from a newer version");
        assert!(
            Replay::from_json(b"{}")
                .unwrap_err()
                .starts_with("not a replay")
        );
    }
}
