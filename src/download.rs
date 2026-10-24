//! Beatmap downloads from public osu! mirrors (osu.direct, then catboy.best): searching for
//! sets and fetching `.osz` archives into the Songs folder, on background threads.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
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

/// In the order they're tried. osu.direct first: it can sort searches on its side, and
/// catboy.best rate-limits downloads per IP (425) after a handful of sets.
const MIRRORS: [Mirror; 2] = [
    Mirror { name: "osu.direct", search: "https://osu.direct/api/v2/search", query_key: "query", amount_key: "amount", download: "https://osu.direct/api/d/" },
    Mirror { name: "catboy.best", search: "https://catboy.best/api/v2/search", query_key: "q", amount_key: "limit", download: "https://catboy.best/d/" },
];

/// Search categories, like the osu! website's (beatmap status filters).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    HasLeaderboard,
    Ranked,
    Qualified,
    Loved,
    Pending,
    Wip,
    Graveyard,
    Any,
}

impl Category {
    pub const ALL: [Category; 8] = [
        Category::HasLeaderboard,
        Category::Ranked,
        Category::Qualified,
        Category::Loved,
        Category::Pending,
        Category::Wip,
        Category::Graveyard,
        Category::Any,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Category::HasLeaderboard => "Has leaderboard",
            Category::Ranked => "Ranked",
            Category::Qualified => "Qualified",
            Category::Loved => "Loved",
            Category::Pending => "Pending",
            Category::Wip => "WIP",
            Category::Graveyard => "Graveyard",
            Category::Any => "Any",
        }
    }

    /// osu! ranked status codes to ask for (empty: any).
    fn statuses(self) -> &'static [i32] {
        match self {
            Category::HasLeaderboard => &[1, 2, 3, 4],
            Category::Ranked => &[1, 2],
            Category::Qualified => &[3],
            Category::Loved => &[4],
            Category::Pending => &[0],
            Category::Wip => &[-1],
            Category::Graveyard => &[-2],
            Category::Any => &[],
        }
    }
}

/// Search orders, like the osu! website's.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    /// The mirror's order (best match); with no search text, newest ranked first.
    Relevance,
    Title,
    Artist,
    Difficulty,
    Ranked,
    Rating,
    Plays,
    Favourites,
}

impl SortKey {
    pub const ALL: [SortKey; 8] = [
        SortKey::Relevance,
        SortKey::Title,
        SortKey::Artist,
        SortKey::Difficulty,
        SortKey::Ranked,
        SortKey::Rating,
        SortKey::Plays,
        SortKey::Favourites,
    ];

    pub fn name(self) -> &'static str {
        match self {
            SortKey::Relevance => "Relevance",
            SortKey::Title => "Title",
            SortKey::Artist => "Artist",
            SortKey::Difficulty => "Difficulty",
            SortKey::Ranked => "Ranked",
            SortKey::Rating => "Rating",
            SortKey::Plays => "Plays",
            SortKey::Favourites => "Favourites",
        }
    }

    /// The natural direction when first picked (text A-Z, numbers high to low).
    pub fn default_desc(self) -> bool {
        !matches!(self, SortKey::Title | SortKey::Artist | SortKey::Difficulty)
    }

    /// osu.direct's name for it, when it can sort that way itself.
    fn server(self) -> Option<&'static str> {
        match self {
            SortKey::Title => Some("title"),
            SortKey::Artist => Some("artist"),
            SortKey::Ranked => Some("ranked_date"),
            SortKey::Plays => Some("play_count"),
            SortKey::Favourites => Some("favourite_count"),
            SortKey::Relevance | SortKey::Difficulty | SortKey::Rating => None,
        }
    }
}

/// A beatmap set found on a mirror (only its taiko difficulties are kept).
#[derive(Clone, Debug)]
pub struct SetInfo {
    pub id: u32,
    pub artist: String,
    pub title: String,
    pub creator: String,
    pub status: String,
    /// Taiko difficulties, easiest first.
    pub diffs: Vec<Diff>,
    pub play_count: u64,
    pub favourite_count: u64,
    pub rating: f32,
    /// ISO 8601, empty if never ranked.
    pub ranked_date: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diff {
    /// osu! beatmap ID (0 when unknown).
    pub id: u32,
    pub version: String,
    pub stars: f32,
}

/// Preview clips are saved under this name; the preview treats such audio as a short clip
/// that starts at the map's preview point.
pub const CLIP_PREFIX: &str = "taiko-rs-preview-clip";

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
    #[serde(default)]
    play_count: u64,
    #[serde(default)]
    favourite_count: u64,
    #[serde(default)]
    rating: f32,
    #[serde(default)]
    ranked_date: Option<String>,
}

#[derive(Deserialize)]
struct ApiMap {
    #[serde(default)]
    id: u32,
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
            let mut diffs: Vec<Diff> = s
                .beatmaps
                .iter()
                .filter(|b| b.mode_int == Some(1) || b.mode.as_deref() == Some("taiko"))
                .map(|b| Diff { id: b.id, version: b.version.clone(), stars: b.difficulty_rating })
                .collect();
            diffs.sort_by(|a, b| a.stars.total_cmp(&b.stars));
            (!diffs.is_empty()).then_some(SetInfo {
                id: s.id,
                artist: s.artist,
                title: s.title,
                creator: s.creator,
                status: s.status,
                diffs,
                play_count: s.play_count,
                favourite_count: s.favourite_count,
                rating: s.rating,
                ranked_date: s.ranked_date.unwrap_or_default(),
            })
        })
        .collect())
}

/// One status (or any) from the first mirror that answers.
fn search_status(agent: &ureq::Agent, query: &str, status: Option<i32>, sort: Option<(&str, bool)>) -> Result<Vec<SetInfo>, String> {
    let mut errors = Vec::new();
    for m in &MIRRORS {
        let mut req = agent.get(m.search).header("User-Agent", USER_AGENT).query(m.query_key, query).query("mode", "1").query(m.amount_key, "50");
        if let Some(st) = status {
            req = req.query("status", st.to_string());
        }
        if let (Some((key, desc)), "osu.direct") = (sort, m.name) {
            req = req.query("sort", format!("{key}:{}", if desc { "desc" } else { "asc" }));
        }
        let result = req
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

/// Searches the mirrors for taiko sets matching `query` in `category`, ordered by `sort`
/// (descending if `desc`). Categories spanning several statuses are fetched per status and
/// merged.
pub fn search(query: &str, category: Category, sort: SortKey, desc: bool) -> Result<Vec<SetInfo>, String> {
    let agent = agent();
    // With no text, "relevance" means newest ranked first, as on the website.
    let (sort, desc) = if sort == SortKey::Relevance && query.trim().is_empty() { (SortKey::Ranked, true) } else { (sort, desc) };
    let server = sort.server().map(|k| (k, desc));
    let statuses: Vec<Option<i32>> = if category.statuses().is_empty() { vec![None] } else { category.statuses().iter().map(|&s| Some(s)).collect() };
    let results: Vec<Result<Vec<SetInfo>, String>> = std::thread::scope(|s| {
        let handles: Vec<_> = statuses.iter().map(|&st| { let agent = &agent; s.spawn(move || search_status(agent, query, st, server)) }).collect();
        handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("search failed".into()))).collect()
    });
    let mut errors = Vec::new();
    let mut sets: Vec<SetInfo> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for r in results {
        match r {
            Ok(found) => sets.extend(found.into_iter().filter(|s| seen.insert(s.id))),
            Err(e) => errors.push(e),
        }
    }
    if sets.is_empty() && !errors.is_empty() {
        return Err(errors.join("; "));
    }
    sort_sets(&mut sets, sort, desc);
    sets.truncate(50);
    Ok(sets)
}

/// Orders merged results (and does the sorts no mirror can do).
fn sort_sets(sets: &mut [SetInfo], sort: SortKey, desc: bool) {
    use std::cmp::Ordering;
    let hardest = |s: &SetInfo| s.diffs.last().map(|d| d.stars).unwrap_or(0.0);
    let order = |a: &SetInfo, b: &SetInfo| -> Ordering {
        match sort {
            SortKey::Relevance => Ordering::Equal,
            SortKey::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
            SortKey::Artist => a.artist.to_lowercase().cmp(&b.artist.to_lowercase()),
            SortKey::Difficulty => hardest(a).total_cmp(&hardest(b)),
            // Unranked sets have no date: they go last either way.
            SortKey::Ranked => match (a.ranked_date.is_empty(), b.ranked_date.is_empty()) {
                (false, false) => a.ranked_date.cmp(&b.ranked_date),
                (x, y) => y.cmp(&x).reverse(),
            },
            SortKey::Rating => a.rating.total_cmp(&b.rating),
            SortKey::Plays => a.play_count.cmp(&b.play_count),
            SortKey::Favourites => a.favourite_count.cmp(&b.favourite_count),
        }
    };
    if sort != SortKey::Relevance {
        sets.sort_by(|a, b| if desc { order(b, a) } else { order(a, b) });
    }
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

/// Passes over the mirrors before a download gives up: osu.direct often answers 503 for a
/// moment and catboy.best rate-limits (425), and the next try usually works.
const ROUNDS: u64 = 3;

fn fetch(id: u32, name: &str, songs: &Path, done: &AtomicU64, total: &AtomicU64) -> Result<PathBuf, String> {
    std::fs::create_dir_all(songs).map_err(|e| format!("{}: {e}", songs.display()))?;
    let dest = songs.join(name);
    let archive = songs.join(format!("{name}.osz.part"));
    let mut errors: [Option<String>; MIRRORS.len()] = Default::default();
    // A mirror that doesn't have the set (404) isn't asked again.
    let mut gone = [false; MIRRORS.len()];
    for round in 0..ROUNDS {
        if gone.iter().all(|&g| g) {
            break;
        }
        if round > 0 {
            // osu.direct's 503s ask for up to 30 s.
            std::thread::sleep(Duration::from_secs(5 * round));
        }
        for (k, m) in MIRRORS.iter().enumerate() {
            if gone[k] {
                continue;
            }
            done.store(0, Ordering::Relaxed);
            total.store(0, Ordering::Relaxed);
            match download_to(&format!("{}{id}", m.download), &archive, done, total) {
                Ok(()) => {
                    let result = extract(&archive, &dest);
                    let _ = std::fs::remove_file(&archive);
                    match result {
                        Ok(()) => return Ok(dest),
                        Err(e) => (errors[k], gone[k]) = (Some(e), true),
                    }
                }
                Err((e, retry)) => {
                    let _ = std::fs::remove_file(&archive);
                    (errors[k], gone[k]) = (Some(e), !retry);
                }
            }
        }
    }
    Err(MIRRORS.iter().zip(errors).filter_map(|(m, e)| Some(format!("{}: {}", m.name, e?))).collect::<Vec<_>>().join("; "))
}

/// Errors say whether trying again later could help (not for a set the mirror doesn't have).
fn download_to(url: &str, path: &Path, done: &AtomicU64, total: &AtomicU64) -> Result<(), (String, bool)> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_connect(Some(Duration::from_secs(15))).build().into();
    let mut response = agent.get(url).header("User-Agent", USER_AGENT).call().map_err(|e| match e {
        ureq::Error::StatusCode(404) => ("doesn't have this set".to_string(), false),
        e => (e.to_string(), true),
    })?;
    let body = response.body_mut();
    if let Some(len) = body.content_length() {
        total.store(len, Ordering::Relaxed);
    }
    // A mirror that has no such set answers with a small JSON error instead of an archive.
    if body.mime_type().is_some_and(|m| m.contains("json") || m.contains("html")) {
        let text = body.read_to_string().unwrap_or_default();
        return Err((if text.is_empty() { "no archive".into() } else { text.chars().take(120).collect() }, false));
    }
    let mut reader = body.with_config().limit(512 << 20).reader();
    let mut file = std::fs::File::create(path).map_err(|e| (e.to_string(), false))?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        // A dropped transfer is worth another try.
        let n = reader.read(&mut buf).map_err(|e| (e.to_string(), true))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| (e.to_string(), false))?;
        done.fetch_add(n as u64, Ordering::Relaxed);
    }
    file.flush().map_err(|e| (e.to_string(), false))?;
    Ok(())
}

/// Raw bytes per transfer message (well under the room's line limit once base64-encoded).
pub const PART_BYTES: usize = 96 * 1024;

/// Packs a map set folder into an `.osz` (stored, not compressed: audio and images already
/// are). Refuses absurdly large folders.
pub fn pack_folder(dir: &Path) -> Result<Vec<u8>, String> {
    use std::io::Write as _;
    const LIMIT: u64 = 300 << 20;
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).map_err(|e| e.to_string())?.flatten() {
            let p = e.path();
            if p.is_dir() {
                if files.len() < 10_000 {
                    stack.push(p);
                }
            } else {
                files.push(p);
            }
        }
    }
    let size: u64 = files.iter().filter_map(|f| std::fs::metadata(f).ok()).map(|m| m.len()).sum();
    if size > LIMIT {
        return Err(format!("the set is too big to send ({} MB)", size >> 20));
    }
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::with_capacity(size as usize + 4096)));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for f in files {
        let Ok(rel) = f.strip_prefix(dir) else { continue };
        let name = rel.to_string_lossy().replace('\\', "/");
        let bytes = std::fs::read(&f).map_err(|e| e.to_string())?;
        zip.start_file(name, options).map_err(|e| e.to_string())?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    Ok(zip.finish().map_err(|e| e.to_string())?.into_inner())
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut lookup = [255u8; 256];
    for (i, &c) in B64.iter().enumerate() {
        lookup[c as usize] = i as u8;
    }
    let bytes: Vec<u8> = text.bytes().filter(|&c| c != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            let v = lookup[c as usize];
            if v == 255 {
                return None;
            }
            n |= (v as u32) << (18 - 6 * i);
        }
        for i in 0..chunk.len().saturating_sub(1) {
            out.push((n >> (16 - 8 * i)) as u8);
        }
    }
    Some(out)
}

/// A folder name from another machine, made safe to create here.
pub fn safe_folder_name(name: &str) -> String {
    let clean: String = name.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
    let clean = clean.trim().trim_matches('.').chars().take(120).collect::<String>();
    if clean.is_empty() { "map from host".into() } else { clean }
}

/// Receives a map set from the multiplayer host: feed the parts (None aborts); the job
/// reports the unpacked folder like a mirror download.
pub fn receive(set: SetInfo, name: String, total: u64, songs: PathBuf) -> (Job, Sender<Option<Vec<u8>>>) {
    let (feed, parts) = mpsc::channel::<Option<Vec<u8>>>();
    let (tx, rx) = mpsc::channel();
    let done = Arc::new(AtomicU64::new(0));
    let total_bytes = Arc::new(AtomicU64::new(total));
    let d = done.clone();
    std::thread::spawn(move || {
        let result = (|| {
            std::fs::create_dir_all(&songs).map_err(|e| e.to_string())?;
            let name = safe_folder_name(&name);
            let dest = songs.join(&name);
            let archive = songs.join(format!("{name}.osz.part"));
            let mut file = std::fs::File::create(&archive).map_err(|e| e.to_string())?;
            let mut got = 0u64;
            while got < total {
                match parts.recv() {
                    Ok(Some(bytes)) => {
                        file.write_all(&bytes).map_err(|e| e.to_string())?;
                        got += bytes.len() as u64;
                        d.store(got, Ordering::Relaxed);
                    }
                    _ => {
                        drop(file);
                        let _ = std::fs::remove_file(&archive);
                        return Err("the transfer was interrupted".to_string());
                    }
                }
            }
            drop(file);
            let result = extract(&archive, &dest);
            let _ = std::fs::remove_file(&archive);
            result.map(|_| dest)
        })();
        let _ = tx.send(result);
    });
    (Job { set, done, total: total_bytes, rx }, feed)
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

/// Fetches what a preview needs without downloading the set: the difficulty's `.osu`, the
/// set's ~10 s music clip (from its preview point) and its cover, cached per set. Returns the
/// `.osu` path; the clip and cover sit next to it.
pub fn fetch_preview(set: u32, beatmap: u32) -> Result<PathBuf, String> {
    let dir = crate::config::user_dir(true).unwrap_or_else(|| std::env::temp_dir().join("taiko-rs")).join("previews").join(set.to_string());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Short timeout: a throttled mirror shouldn't hold the preview up.
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(8))).build().into();
    let get = |url: &str| -> Result<(Vec<u8>, String), String> {
        let mut r = agent.get(url).header("User-Agent", USER_AGENT).call().map_err(|e| e.to_string())?;
        let mime = r.body().mime_type().unwrap_or("").to_string();
        let bytes = r.body_mut().with_config().limit(32 << 20).read_to_vec().map_err(|e| e.to_string())?;
        Ok((bytes, mime))
    };
    let osu = dir.join(format!("{beatmap}.osu"));
    if !osu.is_file() {
        // osu! itself serves any difficulty's .osu without a login; the mirrors are the fallback
        // (catboy.best answers 425 once it rate-limits us, which browsing quickly did).
        let sources = [
            ("osu.ppy.sh", format!("https://osu.ppy.sh/osu/{beatmap}")),
            ("osu.direct", format!("https://osu.direct/api/osu/{beatmap}")),
            ("catboy.best", format!("https://catboy.best/osu/{beatmap}")),
        ];
        let mut errors = Vec::new();
        let mut saved = false;
        for (name, url) in &sources {
            match get(url) {
                Ok((bytes, _)) if bytes.starts_with(b"osu file format") || bytes.windows(15).take(64).any(|w| w == b"osu file format") => {
                    std::fs::write(&osu, bytes).map_err(|e| e.to_string())?;
                    saved = true;
                    break;
                }
                Ok(_) => errors.push(format!("{name}: not a beatmap")),
                Err(e) => errors.push(format!("{name}: {e}")),
            }
        }
        if !saved {
            return Err(format!("difficulty: {}", errors.join("; ")));
        }
    }
    let have_clip = std::fs::read_dir(&dir).map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().starts_with(CLIP_PREFIX))).unwrap_or(false);
    if !have_clip {
        let (bytes, mime) = get(&format!("https://b.ppy.sh/preview/{set}.mp3")).map_err(|e| format!("music: {e}"))?;
        let ext = if mime.contains("ogg") || bytes.starts_with(b"OggS") { "ogg" } else { "mp3" };
        std::fs::write(dir.join(format!("{CLIP_PREFIX}.{ext}")), bytes).map_err(|e| e.to_string())?;
    }
    let cover = dir.join("cover.jpg");
    if !cover.is_file() {
        // Optional: the preview works without a background.
        if let Ok((bytes, _)) = get(&format!("https://assets.ppy.sh/beatmaps/{set}/covers/cover.jpg")) {
            let _ = std::fs::write(&cover, bytes);
        }
    }
    Ok(osu)
}

/// The clip file in a preview folder.
pub fn clip_in(dir: &Path) -> Option<String> {
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).find(|n| n.starts_with(CLIP_PREFIX))
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
        let names: Vec<(&str, f32)> = sets[0].diffs.iter().map(|d| (d.version.as_str(), d.stars)).collect();
        assert_eq!(names, vec![("Kantan", 1.2), ("Oni", 4.5)]);
        assert_eq!(sets[0].folder_name(), "7 A - T_ x_");
        assert_eq!(set_id_of_folder(Path::new("/s/7 A - T_ x_")), Some(7));
        assert_eq!(set_id_of_folder(Path::new("/s/Some folder")), None);
    }

    #[test]
    fn base64_round_trips() {
        for len in 0..40usize {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(base64_decode(&base64_encode(&data)).unwrap(), data);
        }
        assert_eq!(base64_encode(b"taiko"), "dGFpa28=");
        assert_eq!(safe_folder_name("../../etc"), "_.._etc");
    }

}
