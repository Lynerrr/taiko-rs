//! Local scores and replays: every finished play is saved as a replay (the judged inputs) and
//! indexed, so song select can show a leaderboard per map and play any of them back.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 2: judged with `Mash::Nearest` (1: with the `anti_mash` setting).
const REPLAY_VERSION: u32 = 2;

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
    pub score: u64,
    pub accuracy: f64,
    pub max_combo: u32,
    pub great: u32,
    pub ok: u32,
    pub miss: u32,
    pub ur: f64,
    /// (map time ms, key 0-3: left rim, left centre, right centre, right rim).
    pub inputs: Vec<(f64, u8)>,
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
}

fn dir() -> Option<PathBuf> {
    Some(crate::config::user_dir(false)?.join("replays"))
}

fn index_path() -> Option<PathBuf> {
    Some(dir()?.join("scores.tsv"))
}

/// Reads the score index.
pub fn load_index() -> Vec<ScoreRow> {
    let Some(text) = index_path().and_then(|p| std::fs::read_to_string(p).ok()) else { return Vec::new() };
    let base = dir().unwrap_or_default();
    text.lines()
        .filter_map(|line| {
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
            })
        })
        .filter(|r| r.file.is_file())
        .collect()
}

/// Saves a replay and adds it to the index.
pub fn save(replay: &Replay) -> Result<ScoreRow, String> {
    let dir = dir().ok_or("no data folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = format!("{}-{:016x}.json", replay.date, replay.map_hash);
    let file = dir.join(&name);
    std::fs::write(&file, serde_json::to_vec(replay).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let player: String = replay.player.chars().filter(|c| *c != '\t' && *c != '\n').collect();
    let mods = crate::mods::Mods::from_list(&replay.mods).label();
    let line = format!(
        "{:016x}\t{}\t{}\t{:.6}\t{}\t{}\t{:.2}\t{}\t{}\t{}\n",
        replay.map_hash, replay.date, replay.score, replay.accuracy, replay.max_combo, replay.miss, replay.rate, player, name, mods
    );
    let index = index_path().ok_or("no data folder")?;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(index).map_err(|e| e.to_string())?;
    f.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    Ok(ScoreRow {
        map_hash: replay.map_hash,
        date: replay.date,
        score: replay.score,
        accuracy: replay.accuracy,
        max_combo: replay.max_combo,
        miss: replay.miss,
        rate: replay.rate,
        player,
        file,
        mods,
    })
}

pub fn load_replay(path: &Path) -> Result<Replay, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let r: Replay = serde_json::from_slice(&bytes).map_err(|e| format!("not a replay: {e}"))?;
    if r.version > REPLAY_VERSION {
        return Err("replay from a newer version".into());
    }
    Ok(r)
}

impl Replay {
    /// How the play's presses are judged.
    pub fn mash(&self) -> crate::game::Mash {
        if self.version >= 2 { crate::game::Mash::Nearest } else { crate::game::Mash::legacy(self.anti_mash) }
    }
}

pub fn new_version() -> u32 {
    REPLAY_VERSION
}

/// The best scores on a map, highest first.
pub fn top(rows: &[ScoreRow], map_hash: u64, n: usize) -> Vec<&ScoreRow> {
    let mut v: Vec<&ScoreRow> = rows.iter().filter(|r| r.map_hash == map_hash).collect();
    v.sort_by(|a, b| b.score.cmp(&a.score).then(b.date.cmp(&a.date)));
    v.truncate(n);
    v
}

/// "3 d ago" style age of a Unix time.
pub fn age(date: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let s = now.saturating_sub(date);
    match s {
        0..=59 => "now".into(),
        60..=3599 => format!("{}m", s / 60),
        3600..=86_399 => format!("{}h", s / 3600),
        86_400..=2_591_999 => format!("{}d", s / 86_400),
        _ => format!("{}mo", s / 2_592_000),
    }
}
