//! Custom maps: maps made in the editor and sent here by their mapper, apart from osu!'s ranked
//! system. Each is a set (an `.osz`) in DIR/custom/<id>.osz with its index in DIR/custom.json.
//! A custom map is public as soon as it is sent and gives no pp; a map an admin or a loved
//! nominator approves gives pp, as loved ones do.

use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

/// The biggest `.osz` taken.
pub const MAX_OSZ: usize = 40 << 20;
/// The most files in one.
const MAX_ENTRIES: usize = 200;
/// What an `.osz` may unpack to (as the client's own limit).
const MAX_UNPACKED: u64 = 200 << 20;
/// The biggest map file read.
const MAX_MAP_BYTES: u64 = 8 << 20;
/// The biggest background kept.
const MAX_BACKGROUND: u64 = 4 << 20;
/// Sets one account may have.
const PER_ACCOUNT: usize = 30;
/// Seconds an account waits between two uploads.
const WAIT_SECS: u64 = 30;
/// Everything kept together.
const MAX_TOTAL: u64 = 20 << 30;
/// Fewest notes a difficulty has.
const MIN_NOTES: usize = 10;
const MAX_TEXT: usize = 200;
const AUDIO: [&str; 6] = ["mp3", "ogg", "wav", "flac", "m4a", "opus"];
const IMAGES: [&str; 3] = ["png", "jpg", "jpeg"];
/// What else an archive may hold: the maps, a storyboard, hitsounds, a video.
const OTHER: [&str; 5] = ["osu", "tko", "osb", "mp4", "avi"];

/// One difficulty.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Diff {
    pub version: String,
    /// `net::map_hash` of its file: what plays on it are known by.
    pub hash: u64,
    /// A `.tko` (with the client's effects).
    pub tko: bool,
}

/// Who approved a set, and which difficulties (by hash) that covered. A later update leaves the
/// plays on those difficulties counting, and its own new ones wait for another approval.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Approval {
    pub by: String,
    pub at: u64,
    pub hashes: Vec<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Set {
    pub id: u32,
    /// Lowercased account name, and as written.
    pub owner: String,
    pub owner_name: String,
    pub artist: String,
    pub title: String,
    pub creator: String,
    pub diffs: Vec<Diff>,
    pub created: u64,
    pub updated: u64,
    /// How many times it was sent (1 for the first).
    pub version: u32,
    pub downloads: u32,
    pub bytes: u64,
    /// The background's MIME type, when it has one.
    pub background: Option<String>,
    pub approved: Option<Approval>,
}

impl Set {
    /// Whether every difficulty of what is here now was approved.
    pub fn is_approved(&self) -> bool {
        self.approved
            .as_ref()
            .is_some_and(|a| self.diffs.iter().all(|d| a.hashes.contains(&d.hash)))
    }

    pub fn has(&self, hash: u64) -> bool {
        self.diffs.iter().any(|d| d.hash == hash)
    }
}

/// A difficulty as the API tells it (hashes in hex, as the site's links have them).
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PublicDiff {
    pub version: String,
    pub hash: String,
    pub tko: bool,
    pub approved: bool,
}

/// A set as the API tells it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Public {
    pub id: u32,
    pub artist: String,
    pub title: String,
    pub creator: String,
    pub owner: String,
    pub diffs: Vec<PublicDiff>,
    pub updated: u64,
    pub version: u32,
    pub downloads: u32,
    pub bytes: u64,
    pub background: bool,
    pub approved: bool,
}

impl Set {
    pub fn public(&self) -> Public {
        let on = |h: u64| {
            self.approved
                .as_ref()
                .is_some_and(|a| a.hashes.contains(&h))
        };
        Public {
            id: self.id,
            artist: self.artist.clone(),
            title: self.title.clone(),
            creator: self.creator.clone(),
            owner: self.owner_name.clone(),
            diffs: self
                .diffs
                .iter()
                .map(|d| PublicDiff {
                    version: d.version.clone(),
                    hash: format!("{:x}", d.hash),
                    tko: d.tko,
                    approved: on(d.hash),
                })
                .collect(),
            updated: self.updated,
            version: self.version,
            downloads: self.downloads,
            bytes: self.bytes,
            background: self.background.is_some(),
            approved: self.is_approved(),
        }
    }
}

/// What a client asks about its maps: where a map stands.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Status {
    pub id: u32,
    pub owner: String,
    pub approved: bool,
}

// ---- looking into an archive ----

#[derive(Debug)]
pub struct Inspected {
    pub artist: String,
    pub title: String,
    pub creator: String,
    pub diffs: Vec<Diff>,
    pub background: Option<(String, Vec<u8>)>,
}

/// A map file's `[Section] key: value` lines.
struct Ini {
    values: HashMap<(String, String), String>,
    notes: usize,
}

impl Ini {
    fn parse(text: &str) -> Ini {
        let mut values = HashMap::new();
        let (mut section, mut notes) = (String::new(), 0);
        for line in text.lines() {
            let line = line.trim_start_matches('\u{feff}').trim();
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name.to_string();
                continue;
            }
            if section == "HitObjects" {
                notes += 1;
            } else if let Some((k, v)) = line.split_once(':') {
                values
                    .entry((section.clone(), k.trim().to_string()))
                    .or_insert_with(|| v.trim().to_string());
            }
        }
        Ini { values, notes }
    }

    fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.values
            .get(&(section.to_string(), key.to_string()))
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }

    /// The first `0,0,"file"` event.
    fn background(text: &str) -> Option<String> {
        let mut events = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                events = line == "[Events]";
            } else if events && let Some(rest) = line.strip_prefix("0,0,") {
                let name = rest.split(',').next()?.trim().trim_matches('"');
                return (!name.is_empty()).then(|| name.to_string());
            }
        }
        None
    }
}

fn extension(name: &str) -> String {
    name.rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

/// Looks through an `.osz`: what it holds has to be safe, and be taiko maps with notes, their
/// music and nothing odd. Says what is wrong in words for the mapper.
pub fn inspect(bytes: &[u8]) -> Result<Inspected, String> {
    if bytes.len() > MAX_OSZ {
        return Err(format!("the set is too big ({} MB at most)", MAX_OSZ >> 20));
    }
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| "that isn't a beatmap archive (.osz)".to_string())?;
    if zip.len() > MAX_ENTRIES {
        return Err("too many files in the archive".into());
    }
    // First what is in it, by name, and how big.
    let mut names: Vec<String> = Vec::new();
    let mut unpacked = 0u64;
    for i in 0..zip.len() {
        let entry = zip.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_symlink() {
            return Err("the archive has a link in it".into());
        }
        if entry.is_dir() {
            continue;
        }
        let path = entry
            .enclosed_name()
            .ok_or("the archive has a bad file path")?;
        let name = path.to_string_lossy().replace('\\', "/");
        let ext = extension(&name);
        if !(AUDIO.contains(&ext.as_str())
            || IMAGES.contains(&ext.as_str())
            || OTHER.contains(&ext.as_str()))
        {
            return Err(format!("{name}: files like this can't be sent"));
        }
        unpacked += entry.size();
        names.push(name);
    }
    if unpacked > MAX_UNPACKED {
        return Err("the archive unpacks to too much".into());
    }
    let mut diffs: Vec<Diff> = Vec::new();
    let (mut artist, mut title, mut creator) = (String::new(), String::new(), String::new());
    let mut background: Option<(String, Vec<u8>)> = None;
    let mut audio_needed: Option<String> = None;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        let name = entry
            .enclosed_name()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let ext = extension(&name);
        if ext != "osu" && ext != "tko" {
            continue;
        }
        let mut file = Vec::new();
        entry
            .by_ref()
            .take(MAX_MAP_BYTES + 1)
            .read_to_end(&mut file)
            .map_err(|e| e.to_string())?;
        if file.len() as u64 > MAX_MAP_BYTES {
            return Err(format!("{name} is too big for a map file"));
        }
        let text = String::from_utf8_lossy(&file);
        let ini = Ini::parse(&text);
        if ini.get("General", "Mode") != Some("1") {
            return Err(format!("{name} isn't a taiko map (Mode: 1)"));
        }
        if ini.notes < MIN_NOTES {
            return Err(format!("{name} has fewer than {MIN_NOTES} notes"));
        }
        if ini
            .get("Metadata", "BeatmapSetID")
            .and_then(|v| v.parse::<i64>().ok())
            .is_some_and(|id| id > 0)
        {
            return Err(format!(
                "{name} is an osu! beatmap (it has a BeatmapSetID): only your own maps can be sent"
            ));
        }
        let field = |section: &str, key: &str| ini.get(section, key).unwrap_or("").to_string();
        let version = field("Metadata", "Version");
        if version.is_empty() {
            return Err(format!("{name} has no difficulty name"));
        }
        if diffs.iter().any(|d| d.version == version) {
            return Err(format!("two difficulties are named {version}"));
        }
        if artist.is_empty() {
            artist = field("Metadata", "Artist");
            title = field("Metadata", "Title");
            creator = field("Metadata", "Creator");
            audio_needed = ini.get("General", "AudioFilename").map(str::to_string);
            if let Some(bg) = Ini::background(&text) {
                background = Some((bg, Vec::new()));
            }
        }
        diffs.push(Diff {
            version,
            hash: crate::net::map_hash(&file),
            tko: ext == "tko",
        });
    }
    if diffs.is_empty() {
        return Err("there is no taiko map (.osu) in it".into());
    }
    for (what, value) in [("artist", &artist), ("title", &title), ("mapper", &creator)] {
        if value.trim().is_empty() {
            return Err(format!("the map has no {what} name"));
        }
        if value.chars().count() > MAX_TEXT {
            return Err(format!("the {what} name is too long"));
        }
    }
    let audio = audio_needed.ok_or("the map names no music file")?;
    if !names.iter().any(|n| n.eq_ignore_ascii_case(&audio)) {
        return Err(format!("the music ({audio}) isn't in the archive"));
    }
    // The background's bytes, if it is there and fits.
    if let Some((name, bytes)) = &mut background {
        let wanted = name.replace('\\', "/");
        let found = (0..zip.len()).find(|&i| {
            zip.by_index(i).is_ok_and(|e| {
                e.enclosed_name().is_some_and(|p| {
                    p.to_string_lossy()
                        .replace('\\', "/")
                        .eq_ignore_ascii_case(&wanted)
                })
            })
        });
        match found {
            Some(i) => {
                let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
                entry
                    .by_ref()
                    .take(MAX_BACKGROUND + 1)
                    .read_to_end(bytes)
                    .map_err(|e| e.to_string())?;
            }
            None => bytes.clear(),
        }
    }
    let background = background.and_then(|(_, bytes)| {
        let kind = crate::avatars::kind(&bytes)?;
        (bytes.len() as u64 <= MAX_BACKGROUND && matches!(kind, "image/jpeg" | "image/png"))
            .then(|| (kind.to_string(), bytes))
    });
    diffs.sort_by(|a, b| a.version.cmp(&b.version));
    Ok(Inspected {
        artist: artist.trim().to_string(),
        title: title.trim().to_string(),
        creator: creator.trim().to_string(),
        diffs,
        background,
    })
}

// ---- the store ----

struct Inner {
    sets: BTreeMap<u32, Set>,
    next: u32,
    /// By account: when it last sent one.
    sent: HashMap<String, u64>,
}

pub struct Store {
    dir: Option<PathBuf>,
    inner: Mutex<Inner>,
}

static STORE: OnceLock<Store> = OnceLock::new();

/// Reads DIR/custom.json. Without it (tests) there are none, kept in memory.
pub fn init(dir: Option<PathBuf>) {
    let _ = STORE.set(Store::open(dir));
}

fn store() -> &'static Store {
    STORE.get_or_init(|| Store::open(None))
}

impl Store {
    pub fn open(dir: Option<PathBuf>) -> Store {
        let mut sets: BTreeMap<u32, Set> = BTreeMap::new();
        if let Some(dir) = &dir {
            let _ = std::fs::create_dir_all(dir.join("custom"));
            sets = crate::load_saved(&dir.join("custom.json"));
            println!("{} custom map sets ({})", sets.len(), dir.display());
        }
        let next = sets.keys().next_back().map_or(1, |k| k + 1);
        Store {
            dir,
            inner: Mutex::new(Inner {
                sets,
                next,
                sent: HashMap::new(),
            }),
        }
    }

    fn save(&self, inner: &Inner) -> Result<(), String> {
        let Some(dir) = &self.dir else { return Ok(()) };
        let file = dir.join("custom.json");
        let tmp = file.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(&inner.sets).unwrap_or_default();
        std::fs::write(&tmp, text)
            .and_then(|_| std::fs::rename(&tmp, &file))
            .map_err(|e| format!("couldn't save it: {e}"))
    }

    fn path(&self, id: u32, ext: &str) -> Option<PathBuf> {
        self.dir
            .as_ref()
            .map(|d| d.join("custom").join(format!("{id}.{ext}")))
    }

    /// A set sent by `owner` (a new one, or a new version of their set `id`).
    pub fn upload(
        &self,
        owner: &str,
        owner_name: &str,
        id: Option<u32>,
        bytes: &[u8],
        now: u64,
    ) -> Result<Set, String> {
        let found = inspect(bytes)?;
        let mut inner = self.inner.lock().unwrap();
        if inner
            .sent
            .get(owner)
            .is_some_and(|&at| now < at.saturating_add(WAIT_SECS))
        {
            return Err(format!("wait {WAIT_SECS} seconds between two sets"));
        }
        let total: u64 = inner.sets.values().map(|s| s.bytes).sum();
        if total + bytes.len() as u64 > MAX_TOTAL {
            return Err("the server is full".into());
        }
        let mine = inner.sets.values().filter(|s| s.owner == owner).count();
        let set = match id {
            Some(id) => {
                let old = inner.sets.get(&id).ok_or("there is no such set")?;
                if old.owner != owner {
                    return Err("that set is not yours".into());
                }
                Set {
                    artist: found.artist.clone(),
                    title: found.title.clone(),
                    creator: found.creator.clone(),
                    diffs: found.diffs.clone(),
                    updated: now,
                    version: old.version + 1,
                    bytes: bytes.len() as u64,
                    background: found.background.as_ref().map(|b| b.0.clone()),
                    ..old.clone()
                }
            }
            None => {
                if mine >= PER_ACCOUNT {
                    return Err(format!(
                        "you have {PER_ACCOUNT} sets already: delete one first"
                    ));
                }
                // The same maps sent twice are one set.
                if let Some(same) = inner
                    .sets
                    .values()
                    .find(|s| found.diffs.iter().any(|d| s.has(d.hash)))
                {
                    return Err(format!(
                        "these maps are already on the server (set {} by {})",
                        same.id, same.owner_name
                    ));
                }
                let id = inner.next;
                inner.next += 1;
                Set {
                    id,
                    owner: owner.to_string(),
                    owner_name: owner_name.to_string(),
                    artist: found.artist.clone(),
                    title: found.title.clone(),
                    creator: found.creator.clone(),
                    diffs: found.diffs.clone(),
                    created: now,
                    updated: now,
                    version: 1,
                    downloads: 0,
                    bytes: bytes.len() as u64,
                    background: found.background.as_ref().map(|b| b.0.clone()),
                    approved: None,
                }
            }
        };
        if let (Some(osz), Some(bg)) = (self.path(set.id, "osz"), self.path(set.id, "bg")) {
            std::fs::write(&osz, bytes).map_err(|e| format!("couldn't keep the set: {e}"))?;
            match &found.background {
                Some((_, image)) => {
                    let _ = std::fs::write(&bg, image);
                }
                None => {
                    let _ = std::fs::remove_file(&bg);
                }
            }
        }
        inner.sets.insert(set.id, set.clone());
        inner.sent.insert(owner.to_string(), now);
        self.save(&inner)?;
        Ok(set)
    }

    pub fn get(&self, id: u32) -> Option<Set> {
        self.inner.lock().unwrap().sets.get(&id).cloned()
    }

    /// Every set, newest first (by last update), matching `query` in its names when given.
    pub fn list(&self, query: &str) -> Vec<Set> {
        let query = query.trim().to_lowercase();
        let mut sets: Vec<Set> = self
            .inner
            .lock()
            .unwrap()
            .sets
            .values()
            .filter(|s| {
                query.is_empty()
                    || [&s.artist, &s.title, &s.creator, &s.owner_name]
                        .iter()
                        .any(|f| f.to_lowercase().contains(&query))
            })
            .cloned()
            .collect();
        sets.sort_by_key(|s| std::cmp::Reverse((s.updated, s.id)));
        sets
    }

    pub fn file(&self, id: u32) -> Option<Vec<u8>> {
        std::fs::read(self.path(id, "osz")?).ok()
    }

    pub fn background(&self, id: u32) -> Option<(String, Vec<u8>)> {
        let kind = self.get(id)?.background?;
        Some((kind, std::fs::read(self.path(id, "bg")?).ok()?))
    }

    pub fn count_download(&self, id: u32) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(s) = inner.sets.get_mut(&id) {
            s.downloads += 1;
        }
        // Counted in memory and saved with the next change: a download isn't worth a write.
    }

    /// Approves set `id` (or takes it back); false when nothing changed.
    pub fn approve(&self, id: u32, on: bool, by: &str, now: u64) -> Result<bool, String> {
        let mut inner = self.inner.lock().unwrap();
        let set = inner.sets.get_mut(&id).ok_or("there is no such set")?;
        let was = set.approved.clone();
        if on && set.is_approved() {
            return Ok(false);
        }
        set.approved = on.then(|| {
            // Everything approved before still is, and what is here now joins it.
            let mut hashes = was.as_ref().map(|a| a.hashes.clone()).unwrap_or_default();
            for d in &set.diffs {
                if !hashes.contains(&d.hash) {
                    hashes.push(d.hash);
                }
            }
            Approval {
                by: by.to_string(),
                at: now,
                hashes,
            }
        });
        let changed = set.approved != was;
        if changed {
            self.save(&inner)?;
        }
        Ok(changed)
    }

    /// Removes set `id`, when `by` owns it (or `admin`).
    pub fn delete(&self, id: u32, by: &str, admin: bool) -> Result<(), String> {
        let mut inner = self.inner.lock().unwrap();
        let set = inner.sets.get(&id).ok_or("there is no such set")?;
        if set.owner != by && !admin {
            return Err("that set is not yours".into());
        }
        inner.sets.remove(&id);
        for ext in ["osz", "bg"] {
            if let Some(p) = self.path(id, ext) {
                let _ = std::fs::remove_file(p);
            }
        }
        self.save(&inner)
    }

    /// Whether plays on a map with this hash give pp: it was approved with its set.
    pub fn approved(&self, hash: u64) -> bool {
        self.inner.lock().unwrap().sets.values().any(|s| {
            s.approved
                .as_ref()
                .is_some_and(|a| a.hashes.contains(&hash))
        })
    }

    /// The set holding the map with this hash.
    pub fn by_hash(&self, hash: u64) -> Option<Set> {
        self.inner
            .lock()
            .unwrap()
            .sets
            .values()
            .find(|s| s.has(hash))
            .cloned()
    }

    /// Where each of these maps stands (the ones that are custom only).
    pub fn statuses(&self, hashes: &[u64]) -> Vec<(u64, Status)> {
        let inner = self.inner.lock().unwrap();
        hashes
            .iter()
            .filter_map(|&h| {
                let s = inner.sets.values().find(|s| s.has(h))?;
                Some((
                    h,
                    Status {
                        id: s.id,
                        owner: s.owner_name.clone(),
                        approved: s.approved.as_ref().is_some_and(|a| a.hashes.contains(&h)),
                    },
                ))
            })
            .collect()
    }
}

/// The archive with `taiko-rs.custom` added to it (the set's number): unpacked by the game, it
/// marks the folder as a custom set, so the game lists it so and its owner can update it.
pub fn with_sidecar(bytes: &[u8], id: u32) -> Option<Vec<u8>> {
    let mut src = zip::ZipArchive::new(std::io::Cursor::new(bytes)).ok()?;
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for i in 0..src.len() {
        out.raw_copy_file(src.by_index_raw(i).ok()?).ok()?;
    }
    out.start_file("taiko-rs.custom", zip::write::SimpleFileOptions::default())
        .ok()?;
    std::io::Write::write_all(&mut out, format!("{{\"id\": {id}}}\n").as_bytes()).ok()?;
    Some(out.finish().ok()?.into_inner())
}

// The global store's own short ways.
pub fn upload(
    owner: &str,
    owner_name: &str,
    id: Option<u32>,
    bytes: &[u8],
    now: u64,
) -> Result<Set, String> {
    store().upload(owner, owner_name, id, bytes, now)
}
pub fn get(id: u32) -> Option<Set> {
    store().get(id)
}
pub fn list(query: &str) -> Vec<Set> {
    store().list(query)
}
pub fn file(id: u32) -> Option<Vec<u8>> {
    store().file(id)
}
pub fn background(id: u32) -> Option<(String, Vec<u8>)> {
    store().background(id)
}
pub fn count_download(id: u32) {
    store().count_download(id)
}
pub fn approve(id: u32, on: bool, by: &str, now: u64) -> Result<bool, String> {
    store().approve(id, on, by, now)
}
pub fn delete(id: u32, by: &str, admin: bool) -> Result<(), String> {
    store().delete(id, by, admin)
}
pub fn approved(hash: u64) -> bool {
    STORE.get().is_some_and(|s| s.approved(hash))
}
pub fn by_hash(hash: u64) -> Option<Set> {
    STORE.get().and_then(|s| s.by_hash(hash))
}
pub fn statuses(hashes: &[u64]) -> Vec<(u64, Status)> {
    store().statuses(hashes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn map(version: &str, notes: usize, mode: u8) -> String {
        let mut text = format!(
            "osu file format v14\r\n\r\n[General]\r\nAudioFilename: audio.mp3\r\nMode: {mode}\r\n\r\n[Metadata]\r\nTitle:Song\r\nArtist:Band\r\nCreator:me\r\nVersion:{version}\r\n\r\n[Events]\r\n0,0,\"bg.png\",0,0\r\n\r\n[TimingPoints]\r\n0,500,4,1,0,100,1,0\r\n\r\n[HitObjects]\r\n"
        );
        for k in 0..notes {
            text += &format!("256,192,{},1,0,0:0:0:0:\r\n", 1000 + k * 250);
        }
        text
    }

    /// A tiny PNG: the signature, an IHDR for 2x2 and an end.
    fn png() -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        v.extend([0, 0, 0, 13]);
        v.extend(b"IHDR");
        v.extend([0, 0, 0, 2, 0, 0, 0, 2, 8, 2, 0, 0, 0]);
        v.extend([0, 0, 0, 0]);
        v
    }

    fn osz(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in files {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn good() -> Vec<u8> {
        osz(&[
            ("Band - Song (me) [Oni].osu", map("Oni", 20, 1).into_bytes()),
            (
                "Band - Song (me) [Inner].osu",
                map("Inner", 30, 1).into_bytes(),
            ),
            ("audio.mp3", vec![0; 100]),
            ("bg.png", png()),
        ])
    }

    #[test]
    fn a_good_set_is_read() {
        let found = inspect(&good()).unwrap();
        assert_eq!(
            (
                found.artist.as_str(),
                found.title.as_str(),
                found.creator.as_str()
            ),
            ("Band", "Song", "me")
        );
        assert_eq!(found.diffs.len(), 2);
        assert_eq!(found.diffs[0].version, "Inner", "sorted by name");
        assert!(found.background.is_some());
        assert_ne!(found.diffs[0].hash, found.diffs[1].hash);
    }

    #[test]
    fn what_cannot_be_sent_is_refused_in_words() {
        let say = |files: &[(&str, Vec<u8>)]| inspect(&osz(files)).err().unwrap_or_default();
        let m = |v: &str, n: usize, mode: u8| map(v, n, mode).into_bytes();
        assert!(inspect(b"not a zip").unwrap_err().contains("archive"));
        assert!(say(&[("a.osu", m("Oni", 20, 0)), ("audio.mp3", vec![1])]).contains("taiko"));
        assert!(say(&[("a.osu", m("Oni", 3, 1)), ("audio.mp3", vec![1])]).contains("fewer"));
        assert!(say(&[("a.osu", m("Oni", 20, 1))]).contains("music"));
        assert!(
            say(&[
                ("a.osu", m("Oni", 20, 1)),
                ("audio.mp3", vec![1]),
                ("run.exe", vec![1])
            ])
            .contains("can't be sent")
        );
        assert!(say(&[("audio.mp3", vec![1])]).contains("no taiko map"));
        assert!(
            say(&[
                ("a.osu", m("Oni", 20, 1)),
                ("b.osu", m("Oni", 21, 1)),
                ("audio.mp3", vec![1])
            ])
            .contains("two difficulties")
        );
        // A map of osu!'s own.
        let ranked = map("Oni", 20, 1).replace("Version:Oni", "Version:Oni\r\nBeatmapSetID:123");
        assert!(
            say(&[("a.osu", ranked.into_bytes()), ("audio.mp3", vec![1])]).contains("osu! beatmap")
        );
        // A path out of the folder.
        assert!(say(&[("../evil.osu", m("Oni", 20, 1)), ("audio.mp3", vec![1])]).contains("path"));
    }

    #[test]
    fn a_background_that_is_not_a_picture_is_dropped_not_kept() {
        let set = osz(&[
            ("a.osu", map("Oni", 20, 1).into_bytes()),
            ("audio.mp3", vec![1]),
            ("bg.png", b"just words".to_vec()),
        ]);
        assert!(inspect(&set).unwrap().background.is_none());
    }

    #[test]
    fn a_download_carries_the_sets_number() {
        let packed = with_sidecar(&good(), 12).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(packed)).unwrap();
        let mut text = String::new();
        zip.by_name("taiko-rs.custom")
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text.trim(), "{\"id\": 12}");
        assert_eq!(zip.len(), 5, "what was in it is still there");
        assert!(with_sidecar(b"nope", 1).is_none());
    }

    #[test]
    fn too_big_is_refused() {
        let big = vec![0u8; MAX_OSZ + 1];
        assert!(inspect(&big).unwrap_err().contains("too big"));
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("taiko-custom-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn uploading_keeps_the_set_and_updating_is_only_for_its_owner() {
        let dir = scratch("upload");
        let store = Store::open(Some(dir.clone()));
        let first = store.upload("amy", "Amy", None, &good(), 1000).unwrap();
        assert_eq!(
            (first.id, first.version, first.owner_name.as_str()),
            (1, 1, "Amy")
        );
        assert!(store.file(1).is_some() && store.background(1).is_some());
        // Too soon, by anyone's account.
        assert!(
            store
                .upload("amy", "Amy", Some(1), &good(), 1010)
                .unwrap_err()
                .contains("wait")
        );
        // The same maps again as a new set: refused.
        assert!(
            store
                .upload("bob", "Bob", None, &good(), 5000)
                .unwrap_err()
                .contains("already")
        );
        // Someone else's set can't be updated.
        assert!(
            store
                .upload("bob", "Bob", Some(1), &good(), 5000)
                .unwrap_err()
                .contains("not yours")
        );
        // The owner's update is version 2.
        let changed = osz(&[
            ("a.osu", map("Oni", 25, 1).into_bytes()),
            ("audio.mp3", vec![1]),
        ]);
        let second = store.upload("amy", "Amy", Some(1), &changed, 5000).unwrap();
        assert_eq!(
            (second.version, second.created, second.updated),
            (2, 1000, 5000)
        );
        assert_eq!(second.diffs.len(), 1);
        // It survives a restart.
        let again = Store::open(Some(dir.clone()));
        assert_eq!(again.get(1).unwrap().version, 2);
        assert_eq!(again.list("").len(), 1);
        assert_eq!(again.list("nothing like it").len(), 0);
        assert_eq!(
            again.list("amy").len(),
            1,
            "found by its mapper's account too"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_account_has_a_limited_number_of_sets_and_a_wait_between() {
        let store = Store::open(None);
        for k in 0..PER_ACCOUNT {
            let set = osz(&[
                ("a.osu", map(&format!("D{k}"), 20 + k, 1).into_bytes()),
                ("audio.mp3", vec![1]),
            ]);
            store
                .upload("amy", "Amy", None, &set, 1000 + k as u64 * 100)
                .unwrap();
        }
        let more = osz(&[
            ("a.osu", map("Last", 99, 1).into_bytes()),
            ("audio.mp3", vec![1]),
        ]);
        assert!(
            store
                .upload("amy", "Amy", None, &more, 99_999)
                .unwrap_err()
                .contains("already"),
            "30 sets"
        );
    }

    #[test]
    fn approval_gives_pp_to_what_was_approved_and_a_new_version_waits() {
        let store = Store::open(None);
        let set = store.upload("amy", "Amy", None, &good(), 1000).unwrap();
        let (a, b) = (set.diffs[0].hash, set.diffs[1].hash);
        assert!(!store.approved(a), "public, but no pp");
        assert!(store.approve(1, true, "admin", 2000).unwrap());
        assert!(
            !store.approve(1, true, "admin", 2001).unwrap(),
            "nothing new"
        );
        assert!(store.approved(a) && store.approved(b));
        assert!(store.get(1).unwrap().is_approved());
        // An update changes one difficulty: the old hash still counts, the new one waits.
        let changed = osz(&[
            ("a.osu", map("Oni", 20, 1).into_bytes()),
            ("b.osu", map("Inner", 31, 1).into_bytes()),
            ("audio.mp3", vec![1]),
        ]);
        let updated = store.upload("amy", "Amy", Some(1), &changed, 5000).unwrap();
        let new_hash = updated
            .diffs
            .iter()
            .find(|d| d.version == "Inner")
            .unwrap()
            .hash;
        assert!(!store.get(1).unwrap().is_approved(), "pending again");
        assert!(store.approved(b), "the plays on the old one stay");
        assert!(!store.approved(new_hash));
        assert!(store.approve(1, true, "admin", 6000).unwrap());
        assert!(store.approved(new_hash));
        // Taking it back removes pp from all of it.
        assert!(store.approve(1, false, "admin", 7000).unwrap());
        assert!(!store.approved(a) && !store.approved(new_hash));
    }

    #[test]
    fn a_map_is_found_by_its_hash_and_deleting_is_for_the_owner_or_an_admin() {
        let store = Store::open(None);
        let set = store.upload("amy", "Amy", None, &good(), 1000).unwrap();
        let h = set.diffs[0].hash;
        assert_eq!(store.by_hash(h).unwrap().id, 1);
        let st = store.statuses(&[h, 12345]);
        assert_eq!(st.len(), 1, "only custom ones are told");
        assert_eq!(
            st[0].1,
            Status {
                id: 1,
                owner: "Amy".into(),
                approved: false
            }
        );
        assert!(store.delete(1, "bob", false).is_err());
        store.delete(1, "bob", true).unwrap();
        assert!(store.get(1).is_none() && store.by_hash(h).is_none());
    }
}
