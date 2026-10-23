//! Beatmap downloads from public osu! mirrors (catboy.best, then osu.direct): searching for
//! sets and fetching `.osz` archives into the Songs folder, on background threads.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;

/// Mirrors want clients to identify themselves.
const USER_AGENT: &str = concat!("taiko-rs/", env!("CARGO_PKG_VERSION"), " (+https://github.com/Lynerrr/taiko-rs)");

struct Mirror {
    name: &'static str,
    search: &'static str,
    query_key: &'static str,
    amount_key: &'static str,
    download: &'static str,
}

const MIRRORS: [Mirror; 2] = [
    Mirror { name: "catboy.best", search: "https://catboy.best/api/v2/search", query_key: "q", amount_key: "limit", download: "https://catboy.best/d/" },
    Mirror { name: "osu.direct", search: "https://osu.direct/api/v2/search", query_key: "query", amount_key: "amount", download: "https://osu.direct/api/d/" },
];

/// A beatmap set found on a mirror (only its taiko difficulties are kept).
#[derive(Clone, Debug)]
pub struct SetInfo {
    pub id: u32,
    pub artist: String,
    pub title: String,
    pub creator: String,
    pub status: String,
    /// (difficulty name, star rating), easiest first.
    pub diffs: Vec<(String, f32)>,
}

impl SetInfo {
    /// Folder name like osu! uses: "123456 Artist - Title", made safe for any filesystem.
    pub fn folder_name(&self) -> String {
        let raw = format!("{} {} - {}", self.id, self.artist, self.title);
        let clean: String = raw.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
        clean.trim_end_matches(['.', ' ']).chars().take(120).collect()
    }
}

#[derive(Deserialize)]
struct ApiSet {
    id: u32,
    #[serde(default)]
    artist: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    creator: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    beatmaps: Vec<ApiMap>,
}

#[derive(Deserialize)]
struct ApiMap {
    #[serde(default)]
    version: String,
    #[serde(default)]
    mode_int: Option<u8>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    difficulty_rating: f32,
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build().into()
}

/// Parses a mirror's search response (osu! API v2 beatmapset objects).
fn parse_sets(json: &str) -> Result<Vec<SetInfo>, String> {
    let sets: Vec<ApiSet> = serde_json::from_str(json).map_err(|e| format!("unexpected reply: {e}"))?;
    Ok(sets
        .into_iter()
        .filter_map(|s| {
            let mut diffs: Vec<(String, f32)> = s
                .beatmaps
                .iter()
                .filter(|b| b.mode_int == Some(1) || b.mode.as_deref() == Some("taiko"))
                .map(|b| (b.version.clone(), b.difficulty_rating))
                .collect();
            diffs.sort_by(|a, b| a.1.total_cmp(&b.1));
            (!diffs.is_empty()).then_some(SetInfo { id: s.id, artist: s.artist, title: s.title, creator: s.creator, status: s.status, diffs })
        })
        .collect())
}

/// Searches the mirrors in turn for taiko sets matching `query` (empty: recent ones).
pub fn search(query: &str) -> Result<Vec<SetInfo>, String> {
    let agent = agent();
    let mut errors = Vec::new();
    for m in &MIRRORS {
        let result = agent
            .get(m.search)
            .header("User-Agent", USER_AGENT)
            .query(m.query_key, query)
            .query("mode", "1")
            .query(m.amount_key, "50")
            .call()
            .map_err(|e| e.to_string())
            .and_then(|mut r| r.body_mut().with_config().limit(64 << 20).read_to_string().map_err(|e| e.to_string()))
            .and_then(|body| parse_sets(&body));
        match result {
            Ok(sets) => return Ok(sets),
            Err(e) => errors.push(format!("{}: {e}", m.name)),
        }
    }
    Err(errors.join("; "))
}

/// A download in progress. `done`/`total` are bytes (total 0 until known).
pub struct Job {
    pub set: SetInfo,
    pub done: Arc<AtomicU64>,
    pub total: Arc<AtomicU64>,
    pub rx: Receiver<Result<PathBuf, String>>,
}

impl Job {
    pub fn fraction(&self) -> Option<f32> {
        let total = self.total.load(Ordering::Relaxed);
        (total > 0).then(|| self.done.load(Ordering::Relaxed) as f32 / total as f32)
    }
}

/// Downloads set `set` into `songs` (as "<id> <artist> - <title>/") on a background thread.
/// The job reports the extracted folder.
pub fn start(set: SetInfo, songs: PathBuf) -> Job {
    let (tx, rx) = mpsc::channel();
    let done = Arc::new(AtomicU64::new(0));
    let total = Arc::new(AtomicU64::new(0));
    let (d, t) = (done.clone(), total.clone());
    let (id, name) = (set.id, set.folder_name());
    std::thread::spawn(move || {
        let _ = tx.send(fetch(id, &name, &songs, &d, &t));
    });
    Job { set, done, total, rx }
}

fn fetch(id: u32, name: &str, songs: &Path, done: &AtomicU64, total: &AtomicU64) -> Result<PathBuf, String> {
    std::fs::create_dir_all(songs).map_err(|e| format!("{}: {e}", songs.display()))?;
    let dest = songs.join(name);
    let archive = songs.join(format!("{name}.osz.part"));
    let mut errors = Vec::new();
    for m in &MIRRORS {
        done.store(0, Ordering::Relaxed);
        total.store(0, Ordering::Relaxed);
        match download_to(&format!("{}{id}", m.download), &archive, done, total) {
            Ok(()) => {
                let result = extract(&archive, &dest);
                let _ = std::fs::remove_file(&archive);
                match result {
                    Ok(()) => return Ok(dest),
                    Err(e) => errors.push(format!("{}: {e}", m.name)),
                }
            }
            Err(e) => {
                let _ = std::fs::remove_file(&archive);
                errors.push(format!("{}: {e}", m.name));
            }
        }
    }
    Err(errors.join("; "))
}

fn download_to(url: &str, path: &Path, done: &AtomicU64, total: &AtomicU64) -> Result<(), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(15))).build().into();
    let mut response = agent.get(url).header("User-Agent", USER_AGENT).call().map_err(|e| e.to_string())?;
    let body = response.body_mut();
    if let Some(len) = body.content_length() {
        total.store(len, Ordering::Relaxed);
    }
    // A mirror that has no such set answers with a small JSON error instead of an archive.
    if body.mime_type().is_some_and(|m| m.contains("json") || m.contains("html")) {
        let text = body.read_to_string().unwrap_or_default();
        return Err(if text.is_empty() { "no archive".into() } else { text.chars().take(120).collect() });
    }
    let mut reader = body.with_config().limit(512 << 20).reader();
    let mut file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done.fetch_add(n as u64, Ordering::Relaxed);
    }
    file.flush().map_err(|e| e.to_string())?;
    Ok(())
}

/// Unpacks an `.osz` into `dest` (via a temporary folder, so a half-extracted set never shows up).
fn extract(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("not a beatmap archive ({e})"))?;
    let tmp = dest.with_extension("partial");
    let _ = std::fs::remove_dir_all(&tmp);
    // `extract` rejects entries that would escape the destination (zip-slip).
    zip.extract(&tmp).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(dest);
    std::fs::rename(&tmp, dest).map_err(|e| e.to_string())?;
    Ok(())
}

/// The beatmap set ID a library folder or map belongs to: "123456 Artist - Title" folders, as
/// osu! names them.
pub fn set_id_of_folder(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    let digits: String = name.chars().take_while(|c| c.is_ascii_digit()).collect();
    (!digits.is_empty() && name[digits.len()..].starts_with(' ')).then(|| digits.parse().ok()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search_results_and_keeps_taiko_difficulties() {
        let json = r#"[{"id": 7, "artist": "A", "title": "T: x?", "creator": "c", "status": "ranked", "beatmaps": [
            {"version": "Oni", "mode": "taiko", "mode_int": 1, "difficulty_rating": 4.5},
            {"version": "Kantan", "mode": "taiko", "mode_int": 1, "difficulty_rating": 1.2},
            {"version": "Insane", "mode": "osu", "mode_int": 0, "difficulty_rating": 5.0}]},
            {"id": 8, "beatmaps": [{"version": "4K", "mode": "mania", "mode_int": 3, "difficulty_rating": 2.0}]}]"#;
        let sets = parse_sets(json).unwrap();
        assert_eq!(sets.len(), 1, "sets without taiko difficulties are dropped");
        assert_eq!(sets[0].diffs, vec![("Kantan".to_string(), 1.2), ("Oni".to_string(), 4.5)]);
        assert_eq!(sets[0].folder_name(), "7 A - T_ x_");
        assert_eq!(set_id_of_folder(Path::new("/s/7 A - T_ x_")), Some(7));
        assert_eq!(set_id_of_folder(Path::new("/s/Some folder")), None);
    }
}
