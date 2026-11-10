//! Local scores and replays: every finished play is saved as a replay (the judged inputs) and
//! indexed, so song select can show a leaderboard per map and play any of them back. What a
//! replay and an index line hold is taiko-logic's (`taiko_logic::scores`); here they are read
//! from and written to the data folder.

use std::io::Write;
use std::path::{Path, PathBuf};

use taiko_logic::scores::{self as logic, rename_rows};
pub use taiko_logic::scores::{Replay, ScoreRow, new_version, top};

fn dir() -> Option<PathBuf> {
    Some(crate::config::data_dir()?.join("replays"))
}

fn index_path() -> Option<PathBuf> {
    Some(dir()?.join("scores.tsv"))
}

/// Reads the score index.
pub fn load_index() -> Vec<ScoreRow> {
    let Some(text) = index_path().and_then(|p| std::fs::read_to_string(p).ok()) else {
        return Vec::new();
    };
    let base = dir().unwrap_or_default();
    text.lines()
        .filter_map(|line| ScoreRow::parse(line, &base))
        .filter(|r| r.file.is_file())
        .collect()
}

/// Saves a replay and adds it to the index, with its pp.
pub fn save(replay: &Replay, pp: Option<f64>) -> Result<ScoreRow, String> {
    let dir = dir().ok_or("no data folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = format!("{}-{:016x}.json", replay.date, replay.map_hash);
    let file = dir.join(&name);
    std::fs::write(&file, replay.to_json()?).map_err(|e| e.to_string())?;
    let row = ScoreRow::of(replay, file, pp);
    let index = index_path().ok_or("no data folder")?;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(index)
        .map_err(|e| e.to_string())?;
    f.write_all(row.line().as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(row)
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

/// Moves the local plays saved as `from` to the name `to`: the index right away, their replays
/// in the background. Returns how many plays moved.
pub fn rename_player(rows: &mut [ScoreRow], from: &str, to: &str) -> Result<usize, String> {
    let files = rename_rows(rows, from, to);
    if files.is_empty() {
        return Ok(0);
    }
    rewrite_index(rows)?;
    let (from, to) = (from.to_string(), to.to_string());
    let moved = files.len();
    std::thread::spawn(move || {
        for file in files {
            let Ok(mut r) = load_replay(&file) else {
                continue;
            };
            if r.player != from {
                continue;
            }
            r.player = to.clone();
            let Ok(bytes) = r.to_json() else {
                continue;
            };
            let tmp = file.with_extension("json.tmp");
            if std::fs::write(&tmp, bytes).is_err() || std::fs::rename(&tmp, &file).is_err() {
                eprintln!("couldn't rename the player in {}", file.display());
            }
        }
    });
    Ok(moved)
}

pub fn load_replay(path: &Path) -> Result<Replay, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Replay::from_json(&bytes)
}

/// A replay game with no audio, at its start.
pub fn replay_game(
    map: &crate::beatmap::Beatmap,
    track: &std::sync::Arc<crate::audio::Track>,
    r: &Replay,
) -> crate::game::Game {
    logic::replay_game(map, track.clone(), r)
}

/// Plays a replay through its map, silently, for everything the results screen shows that the
/// replay doesn't keep (hit errors, drumrolls, big notes) and the hit windows. None if the map
/// can't be read.
pub fn simulate(map_path: &Path, r: &Replay) -> Option<(crate::game::Stats, crate::game::Windows)> {
    let map = crate::beatmap::load(map_path).ok()?;
    Some(logic::simulate(&map, r))
}

/// Where online replays are kept once fetched (outside the score index; in the temporary
/// folder without a data folder).
pub fn online_replay_path(id: u64) -> Option<PathBuf> {
    let dir = dir()
        .map(|d| d.join("online"))
        .unwrap_or_else(|| std::env::temp_dir().join("taiko-rs-online-replays"));
    Some(dir.join(format!("{id}.json")))
}

/// Saves a replay fetched from the server where `online_replay_path` says.
pub fn save_online(id: u64, replay: &Replay) -> Result<PathBuf, String> {
    let path = online_replay_path(id).ok_or("no data folder")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, replay.to_json()?).map_err(|e| e.to_string())?;
    Ok(path)
}

/// The letter of a saved play (the index has no counts: only an all-GREAT play is at 100%).
fn row_grade(row: &ScoreRow) -> &'static str {
    crate::online::grade(1, u32::from(row.accuracy < 1.0), row.miss, row.accuracy)
}

/// The best letter of the plays on a map, like osu!'s song select shows.
pub fn best_grade(rows: &[ScoreRow], map_hash: u64) -> Option<&'static str> {
    let rank = |g: &str| ["D", "C", "B", "A", "S", "SS"].iter().position(|l| *l == g);
    rows.iter()
        .filter(|r| r.map_hash == map_hash)
        .map(row_grade)
        .max_by_key(|g| rank(g))
}

/// "3d" style age of a Unix time (for "3d ago").
pub fn age(date: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let s = now.saturating_sub(date);
    match s {
        0..=59 => tr!("now", "今", "сейчас").into(),
        60..=3599 => tr!("{}m", "{}分", "{}м"; s / 60),
        3600..=86_399 => tr!("{}h", "{}時間", "{}ч"; s / 3600),
        86_400..=2_591_999 => tr!("{}d", "{}日", "{}д"; s / 86_400),
        _ => tr!("{}mo", "{}ヶ月", "{}мес"; s / 2_592_000),
    }
}
