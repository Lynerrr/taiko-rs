//! Local scores and replays: every finished play is saved as a replay (the judged inputs) and
//! indexed, so song select can show a leaderboard per map and play any of them back.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// 3: scored with `Scoring::Standard` (below: `Classic`). 2: judged with `Mash::Nearest` (1: with
/// the `anti_mash` setting).
const REPLAY_VERSION: u32 = 3;

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
    /// Performance points (None: not worked out yet, or Autoplay / Relax).
    pub pp: Option<f64>,
}

impl ScoreRow {
    /// Its line in the index.
    fn line(&self) -> String {
        let pp = self.pp.map(|p| format!("{p:.2}")).unwrap_or_default();
        let name = self.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        format!(
            "{:016x}\t{}\t{}\t{:.6}\t{}\t{}\t{:.2}\t{}\t{}\t{}\t{pp}\n",
            self.map_hash, self.date, self.score, self.accuracy, self.max_combo, self.miss, self.rate, self.player, name, self.mods
        )
    }
}

fn dir() -> Option<PathBuf> {
    Some(crate::config::data_dir()?.join("replays"))
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
                pp: f.get(10).and_then(|p| p.parse().ok()),
            })
        })
        .filter(|r| r.file.is_file())
        .collect()
}

/// Saves a replay and adds it to the index, with its pp.
pub fn save(replay: &Replay, pp: Option<f64>) -> Result<ScoreRow, String> {
    let dir = dir().ok_or("no data folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = format!("{}-{:016x}.json", replay.date, replay.map_hash);
    let file = dir.join(&name);
    std::fs::write(&file, serde_json::to_vec(replay).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let row = ScoreRow {
        map_hash: replay.map_hash,
        date: replay.date,
        score: replay.score,
        accuracy: replay.accuracy,
        max_combo: replay.max_combo,
        miss: replay.miss,
        rate: replay.rate,
        player: replay.player.chars().filter(|c| *c != '\t' && *c != '\n').collect(),
        file,
        mods: crate::mods::Mods::from_list(&replay.mods).label(),
        pp,
    };
    let index = index_path().ok_or("no data folder")?;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(index).map_err(|e| e.to_string())?;
    f.write_all(row.line().as_bytes()).map_err(|e| e.to_string())?;
    Ok(row)
}

impl Replay {
    /// What pp needs to know about the play.
    pub fn pp_play(&self) -> crate::pp::Play {
        crate::pp::Play {
            great: self.great,
            ok: self.ok,
            miss: self.miss,
            max_combo: self.max_combo,
            rate: self.rate,
            mods: crate::mods::Mods::from_list(&self.mods),
            scroll_speed: self.scroll_speed,
            seed: self.seed,
        }
    }
}

fn usual_scroll_speed() -> f64 {
    1.0
}

/// Works out the pp of saved scores from before pp (on a background thread): (replay file, pp)
/// for each whose map is still there.
pub fn backfill_pp(files: Vec<PathBuf>) -> std::sync::mpsc::Receiver<Vec<(PathBuf, f64)>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let found = files
            .into_iter()
            .filter_map(|file| {
                let r = load_replay(&file).ok()?;
                let pp = crate::pp::play_pp(Path::new(&r.map_path), &r.pp_play())?;
                Some((file, pp))
            })
            .collect();
        let _ = tx.send(found);
    });
    rx
}

/// Writes the whole index again (after the pp of old scores were worked out).
pub fn rewrite_index(rows: &[ScoreRow]) -> Result<(), String> {
    let index = index_path().ok_or("no data folder")?;
    let text: String = rows.iter().map(ScoreRow::line).collect();
    let tmp = index.with_extension("tsv.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &index).map_err(|e| e.to_string())
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

    /// How its score was worked out.
    pub fn scoring(&self) -> crate::game::Scoring {
        if self.version >= 3 { crate::game::Scoring::Standard } else { crate::game::Scoring::Classic }
    }
}

/// Plays a replay through its map, silently, for everything the results screen shows that the
/// replay doesn't keep (hit errors, drumrolls, big notes) and the hit windows. None if the map
/// can't be read.
pub fn simulate(map_path: &Path, r: &Replay) -> Option<(crate::game::Stats, crate::game::Windows)> {
    let map = crate::beatmap::load(map_path).ok()?;
    let track = std::sync::Arc::new(crate::audio::Track { samples: Vec::new(), rate: r.rate });
    let mut g = crate::video::replay_game(&map, &track, r);
    // Frame by frame, as it was played.
    let end = g.end_time() + 2000.0;
    let mut t = g.time;
    while t < end {
        t = (t + 8.0).min(end);
        g.set_time(t);
    }
    Some((g.stats.clone(), g.windows))
}

/// Where online replays are kept once fetched (outside the score index; in the temporary
/// folder without a data folder).
pub fn online_replay_path(id: u64) -> Option<PathBuf> {
    let dir = dir().map(|d| d.join("online")).unwrap_or_else(|| std::env::temp_dir().join("taiko-rs-online-replays"));
    Some(dir.join(format!("{id}.json")))
}

/// Saves a replay fetched from the server where `online_replay_path` says.
pub fn save_online(id: u64, replay: &Replay) -> Result<PathBuf, String> {
    let path = online_replay_path(id).ok_or("no data folder")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, serde_json::to_vec(replay).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(path)
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

/// The letter of a saved play (the index has no counts: only an all-GREAT play is at 100%).
fn row_grade(row: &ScoreRow) -> &'static str {
    crate::online::grade(1, u32::from(row.accuracy < 1.0), row.miss, row.accuracy)
}

/// The best letter of the plays on a map, like osu!'s song select shows.
pub fn best_grade(rows: &[ScoreRow], map_hash: u64) -> Option<&'static str> {
    let rank = |g: &str| ["D", "C", "B", "A", "S", "SS"].iter().position(|l| *l == g);
    rows.iter().filter(|r| r.map_hash == map_hash).map(row_grade).max_by_key(|g| rank(g))
}

/// "3d" style age of a Unix time (for "3d ago").
pub fn age(date: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let s = now.saturating_sub(date);
    match s {
        0..=59 => tr!("now", "今", "сейчас").into(),
        60..=3599 => tr!("{}m", "{}分", "{}м"; s / 60),
        3600..=86_399 => tr!("{}h", "{}時間", "{}ч"; s / 3600),
        86_400..=2_591_999 => tr!("{}d", "{}日", "{}д"; s / 86_400),
        _ => tr!("{}mo", "{}ヶ月", "{}мес"; s / 2_592_000),
    }
}
