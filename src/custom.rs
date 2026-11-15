//! Custom maps from the game's side: packing a map's folder into an `.osz`, sending it to the
//! server, and the little file that remembers which set on the server
//! the folder is, so the next upload updates it instead of making another.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::download::{CustomSet, Diff, SetInfo};
use crate::online::WEBSITE;

pub mod approved;
pub mod store;

/// The file in a map's folder that keeps its number on the server.
pub const SIDECAR: &str = "taiko-rs.custom";
/// Fewest notes a difficulty needs to be sent (the server's rule too).
pub const MIN_NOTES: usize = 10;
/// The biggest set the server takes.
pub const MAX_OSZ: usize = 40 << 20;
/// The most files in one.
const MAX_FILES: usize = 200;
/// How deep into the folder the packer looks (storyboard elements sit in subfolders).
const MAX_DEPTH: usize = 3;
/// GIF and WebP go along for the animated pictures of `.tko` effects.
const SENDABLE: [&str; 16] = [
    "osu", "tko", "osb", "mp3", "ogg", "wav", "flac", "m4a", "opus", "png", "jpg", "jpeg", "gif",
    "webp", "mp4", "avi",
];
/// Already compressed: keeping them as they are is faster and no bigger.
const PACKED: [&str; 10] = [
    "mp3", "ogg", "m4a", "opus", "png", "jpg", "jpeg", "gif", "webp", "mp4",
];

/// The `[Metadata]` key that names the original mapper of an osu! map this one is an edit of.
/// The server takes a map with an osu! `BeatmapSetID` only with it (the editor's Song setup
/// writes it): the mapper's word that the file is their edit, not a copy.
pub const BASED_ON_KEY: &str = "TaikoRsBasedOn";
/// The longest original mapper name the server keeps.
pub const MAX_ORIGINAL_MAPPER: usize = 64;

/// The osu! set a map file's text names (a `BeatmapSetID` above 0), and the original mapper
/// it credits under `BASED_ON_KEY`.
pub fn edit_of(text: &str) -> (Option<u32>, Option<String>) {
    let (mut section, mut set, mut mapper) = ("", None, None);
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name;
        } else if section == "Metadata"
            && let Some((key, value)) = line.split_once(':')
        {
            let value = value.trim();
            match key.trim() {
                "BeatmapSetID" if set.is_none() => {
                    set = Some(value.parse::<u32>().ok().filter(|id| *id > 0))
                }
                key if key == BASED_ON_KEY && mapper.is_none() => {
                    mapper = Some((!value.is_empty()).then(|| value.to_string()))
                }
                _ => {}
            }
        }
    }
    (set.flatten(), mapper.flatten())
}

/// Where a set's page is on the website.
pub fn page_url(id: u32) -> String {
    format!("{WEBSITE}/c/{id}")
}

/// Where a set in taiko.rs's own custom maps folder came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// Downloaded from the website (or its archive opened).
    Site,
    /// Made in the editor here.
    Local,
    /// A guest's copy of a collab mapping session's set.
    Collab,
    /// Sent by a lobby host.
    Lobby,
    /// Taken from an archive opened in the game.
    Import,
}

/// What `SIDECAR` says of its folder. Version 1 (what the website puts in its archives, and older
/// clients wrote) is `{"id": N}` only; every other field is optional, so both read the same and
/// what a reader doesn't know of is kept when it writes.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Manifest {
    /// The set's number on the website; none for a set never sent there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    /// The website's version of the set this folder holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    /// Seconds since 1970 of that version's upload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    /// The osu! set it was made from, if any (for show: it is never asked about).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub based_on: Option<u32>,
    /// The charts the install brought, by file name, with their content hash (`net::map_hash`,
    /// 16 hex digits). A chart that isn't here is the player's own.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub files: std::collections::BTreeMap<String, String>,
    /// Seconds since 1970 of the install.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<u64>,
    /// Files of the player's own carried over from an older version (paths in the folder).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub own: Vec<String>,
    /// Fields of a newer client, kept as they are.
    #[serde(flatten)]
    pub rest: serde_json::Map<String, serde_json::Value>,
}

impl Manifest {
    /// The manifest in `folder`, if it has one that can be read.
    pub fn read(folder: &Path) -> Option<Manifest> {
        Manifest::parse(&std::fs::read_to_string(folder.join(SIDECAR)).ok()?)
    }

    pub fn parse(text: &str) -> Option<Manifest> {
        serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
    }

    /// Writes it into `folder` whole (through a temporary file, so a crash never leaves half a
    /// manifest).
    pub fn write(&self, folder: &Path) -> std::io::Result<()> {
        let mut text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        text.push('\n');
        let part = folder.join(format!("{SIDECAR}.part"));
        std::fs::write(&part, text)?;
        std::fs::rename(&part, folder.join(SIDECAR))
    }

    /// The hash recorded for chart `name`, if the install brought it.
    pub fn file_hash(&self, name: &str) -> Option<u64> {
        u64::from_str_radix(self.files.get(name)?, 16).ok()
    }

    /// Records chart `name` with content hash `hash` as one the install brought.
    pub fn add_file(&mut self, name: &str, hash: u64) {
        self.files.insert(name.to_string(), format!("{hash:016x}"));
    }
}

/// The set number the folder was sent as, if it was.
pub fn sidecar_id(folder: &Path) -> Option<u32> {
    Manifest::read(folder)?.id
}

/// Records that `folder` was sent as version `sent.version` of website set `sent.id`, with its
/// charts `charts` (file names) as they are now.
pub fn mark_sent(folder: &Path, sent: &Sent, charts: &[String]) {
    let mut m = Manifest::read(folder).unwrap_or_default();
    m.id = Some(sent.id);
    m.version = Some(sent.version);
    m.origin.get_or_insert(Origin::Local);
    let now = now_secs();
    m.updated = Some(now);
    m.installed = Some(now);
    m.files.clear();
    for name in charts {
        if let Ok(bytes) = std::fs::read(folder.join(name)) {
            m.add_file(name, crate::net::map_hash(&bytes));
        }
    }
    let _ = m.write(folder);
}

/// Seconds since 1970.
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `(mode, note count)` of a map file's text: the taiko ones are mode 1.
pub fn mode_and_notes(text: &str) -> (Option<u8>, usize) {
    let (mut section, mut mode, mut notes) = ("", None, 0);
    for line in text.lines() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name;
        } else if section == "HitObjects" && !line.is_empty() {
            notes += 1;
        } else if section == "General"
            && mode.is_none()
            && let Some(v) = line.strip_prefix("Mode:")
        {
            mode = v.trim().parse().ok();
        }
    }
    (mode, notes)
}

/// A folder packed for sending.
#[derive(Debug)]
pub struct Packed {
    pub bytes: Vec<u8>,
    /// The taiko difficulties that went (their file names).
    pub sent: Vec<String>,
    /// Difficulties left out, and why.
    pub left_out: Vec<String>,
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = read.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        let hidden = e.file_name().to_string_lossy().starts_with('.');
        let Ok(kind) = e.file_type() else { continue };
        if hidden || kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            if depth < MAX_DEPTH {
                collect(&path, depth + 1, out);
            }
        } else if SENDABLE.contains(&extension(&path).as_str()) {
            out.push(path);
        }
    }
}

/// Packs the folder: the taiko difficulties with enough notes, and everything else a map of
/// the set needs (music, pictures, storyboard, hitsounds). Other modes' maps, backups
/// (`.bak`, `.osu.old`) and the sidecar stay behind, and so do osu! maps that name no original
/// mapper: only the mapper's own maps and edits go to the server.
pub fn pack(folder: &Path) -> Result<Packed, String> {
    pack_with(folder, MIN_NOTES, MAX_OSZ, "the server takes", true)
}

/// The most a set exported to a file (`.rsz` or `.osz`) holds.
pub const MAX_EXPORT: usize = 300 << 20;

/// Packs the folder for a file to give to people (`export`): every taiko difficulty with a
/// note, and what they need.
pub fn pack_export(folder: &Path) -> Result<Packed, String> {
    pack_with(folder, 1, MAX_EXPORT, "a file holds", false)
}

/// `pack`, keeping the difficulties with at least `min_notes` notes, and at most `max` bytes
/// in all (`who` says who takes no more: "the server takes"). `own` leaves out the osu! maps
/// that credit no original mapper.
fn pack_with(
    folder: &Path,
    min_notes: usize,
    max: usize,
    who: &str,
    own: bool,
) -> Result<Packed, String> {
    let mut paths = Vec::new();
    collect(folder, 0, &mut paths);
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let (mut files, mut sent, mut left_out) = (0usize, Vec::new(), Vec::new());
    for path in paths {
        let ext = extension(&path);
        let name = path
            .strip_prefix(folder)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = std::fs::read(&path).map_err(|e| format!("{name}: {e}"))?;
        if ext == "osu" || ext == "tko" {
            let (mode, notes) = mode_and_notes(&String::from_utf8_lossy(&bytes));
            if mode != Some(1) {
                continue;
            }
            if notes < min_notes {
                left_out.push(format!("{name} (fewer than {min_notes} notes)"));
                continue;
            }
            if own && matches!(edit_of(&String::from_utf8_lossy(&bytes)), (Some(_), None)) {
                left_out.push(format!("{name} (an osu! map, no original mapper named)"));
                continue;
            }
            sent.push(name.clone());
        }
        let method = if PACKED.contains(&ext.as_str()) {
            zip::CompressionMethod::Stored
        } else {
            zip::CompressionMethod::Deflated
        };
        zip.start_file(
            name.as_str(),
            zip::write::SimpleFileOptions::default().compression_method(method),
        )
        .map_err(|e| e.to_string())?;
        zip.write_all(&bytes).map_err(|e| e.to_string())?;
        files += 1;
        if files > MAX_FILES {
            return Err(format!(
                "too many files in the folder (at most {MAX_FILES})"
            ));
        }
    }
    if sent.is_empty() {
        return Err(if min_notes > 1 {
            format!("no taiko difficulty with at least {min_notes} notes in the map's folder")
        } else {
            "no taiko difficulty with notes in the map's folder".into()
        });
    }
    let bytes = zip.finish().map_err(|e| e.to_string())?.into_inner();
    if bytes.len() > max {
        return Err(format!(
            "the set is {} MB, {who} {} MB at most",
            bytes.len() >> 20,
            max >> 20
        ));
    }
    Ok(Packed {
        bytes,
        sent,
        left_out,
    })
}

/// What the server said about a set it took.
#[derive(Debug, PartialEq)]
pub struct Sent {
    pub id: u32,
    pub version: u32,
}

/// Sends a packed set with the game's `token`: a new set, or a new version of `id`.
pub fn upload(token: &str, id: Option<u32>, osz: &[u8]) -> Result<Sent, String> {
    crate::offline::internet()?;
    let url = match id {
        Some(id) => format!("{WEBSITE}/custom/upload?id={id}"),
        None => format!("{WEBSITE}/custom/upload"),
    };
    let reply = ureq::post(url)
        .config()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(300)))
        .build()
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/octet-stream")
        .send(osz);
    let mut reply = reply.map_err(|e| e.to_string())?;
    let ok = reply.status().is_success();
    let text = reply
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    if !ok {
        return Err(text);
    }
    answer(&text)
}

/// Where the website gives a custom set's archive (with its sidecar in it, so the folder it
/// unpacks to is known as that set).
pub fn download_url(id: u32) -> String {
    format!("{WEBSITE}/c/{id}/download")
}

/// The custom sets on the website matching `query` (song, mapper or sender), approved ones only
/// if `approved`, in `sort` order (newest first for relevance without search text).
pub fn search(
    query: &str,
    approved: bool,
    sort: crate::download::SortKey,
    desc: bool,
) -> Result<Vec<SetInfo>, String> {
    use crate::download::SortKey;
    crate::offline::internet()?;
    let mut reply = ureq::get(format!("{WEBSITE}/api/custom"))
        .query("q", query.trim())
        .config()
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .call()
        .map_err(|e| e.to_string())?;
    let text = reply
        .body_mut()
        .with_config()
        .limit(16 << 20)
        .read_to_string()
        .map_err(|e| e.to_string())?;
    let mut sets = parse_list(&text)?;
    sets.retain(|s| !approved || s.custom.as_ref().is_some_and(|c| c.approved));
    let (sort, desc) = if sort == SortKey::Relevance && query.trim().is_empty() {
        (SortKey::Ranked, true)
    } else {
        (sort, desc)
    };
    crate::download::sort_sets(&mut sets, sort, desc);
    Ok(sets)
}

/// The sets of an `/api/custom` answer. A difficulty's number is its place in the set, from 1.
fn parse_list(text: &str) -> Result<Vec<SetInfo>, String> {
    #[derive(serde::Deserialize)]
    struct ApiDiff {
        #[serde(default)]
        version: String,
        #[serde(default)]
        tko: bool,
    }
    #[derive(serde::Deserialize)]
    struct ApiSet {
        id: u32,
        #[serde(default)]
        artist: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        creator: String,
        #[serde(default)]
        diffs: Vec<ApiDiff>,
        #[serde(default)]
        updated: u64,
        #[serde(default)]
        version: u32,
        #[serde(default)]
        downloads: u64,
        #[serde(default)]
        bytes: u64,
        #[serde(default)]
        approved: bool,
        #[serde(default)]
        background: bool,
    }
    let sets: Vec<ApiSet> =
        serde_json::from_str(text).map_err(|e| format!("unexpected reply: {e}"))?;
    Ok(sets
        .into_iter()
        .filter(|s| !s.diffs.is_empty())
        .map(|s| SetInfo {
            id: s.id,
            artist: s.artist,
            title: s.title,
            creator: s.creator,
            status: if s.approved {
                "custom approved"
            } else {
                "custom"
            }
            .into(),
            diffs: s
                .diffs
                .iter()
                .zip(1..)
                .map(|(d, id)| Diff {
                    id,
                    version: d.version.clone(),
                    stars: 0.0,
                })
                .collect(),
            play_count: s.downloads,
            custom: Some(CustomSet {
                approved: s.approved,
                version: s.version,
                bytes: s.bytes,
                updated: s.updated,
                tko: s.diffs.iter().any(|d| d.tko),
                background: s.background,
            }),
            ..SetInfo::default()
        })
        .collect())
}

/// Where a custom set's preview is kept: its archive, and what it unpacks to.
fn preview_dir(set: &SetInfo) -> PathBuf {
    let version = set.custom.as_ref().map_or(0, |c| c.version);
    crate::config::cache_dir()
        .unwrap_or_else(|| std::env::temp_dir().join("taiko-rs"))
        .join("previews")
        .join(format!("c{}-v{version}", set.id))
}

/// The archive a preview of this version of the set already fetched, if there is one: the
/// download takes it instead of asking the website again.
pub fn cached_archive(set: &SetInfo) -> Option<PathBuf> {
    let file = preview_dir(set).join("set.osz");
    file.is_file().then_some(file)
}

/// Fetches a custom set for the download tab's preview: the website has no single difficulty
/// to give, so the whole archive comes (custom sets are small) and is unpacked into the cache.
/// Returns the folder its maps are in.
pub fn fetch_preview(set: &SetInfo) -> Result<PathBuf, String> {
    let dir = preview_dir(set);
    let maps = dir.join("maps");
    if maps.is_dir() {
        return Ok(maps);
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let archive = dir.join("set.osz");
    if !archive.is_file() {
        let part = dir.join("set.osz.part");
        let (done, total, cancel) = Default::default();
        crate::download::download_to(&download_url(set.id), &part, &done, &total, &cancel)
            .map_err(|(e, _)| e)?;
        std::fs::rename(&part, &archive).map_err(|e| e.to_string())?;
    }
    let unpacking = dir.join("maps.part");
    let _ = std::fs::remove_dir_all(&unpacking);
    crate::download::unpack(&archive, &unpacking)?;
    std::fs::rename(&unpacking, &maps).map_err(|e| e.to_string())?;
    Ok(maps)
}

/// A custom set as the website has it now.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SiteSet {
    pub id: u32,
    /// Approved: its plays give pp.
    pub approved: bool,
    /// Its difficulties' content hashes (`Entry::hash`): what a folder with this version of the
    /// set has.
    pub hashes: Vec<u64>,
    /// Seconds since 1970 of its last upload.
    pub updated: u64,
}

/// These custom sets as the website has them; None when it can't be asked.
pub fn site_sets(ids: &[u32]) -> Option<Vec<SiteSet>> {
    crate::offline::internet().ok()?;
    let list: Vec<String> = ids.iter().map(u32::to_string).collect();
    let mut reply = ureq::get(format!("{WEBSITE}/api/custom?ids={}", list.join(",")))
        .config()
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .call()
        .ok()?;
    let text = reply.body_mut().read_to_string().ok()?;
    sets_in(&text)
}

fn sets_in(text: &str) -> Option<Vec<SiteSet>> {
    #[derive(serde::Deserialize)]
    struct ApiDiff {
        #[serde(default)]
        hash: String,
    }
    #[derive(serde::Deserialize)]
    struct ApiSet {
        id: u32,
        #[serde(default)]
        approved: bool,
        #[serde(default)]
        diffs: Vec<ApiDiff>,
        #[serde(default)]
        updated: u64,
    }
    let sets: Vec<ApiSet> = serde_json::from_str(text).ok()?;
    Some(
        sets.into_iter()
            .map(|s| SiteSet {
                id: s.id,
                approved: s.approved,
                hashes: s
                    .diffs
                    .iter()
                    .filter_map(|d| u64::from_str_radix(&d.hash, 16).ok())
                    .collect(),
                updated: s.updated,
            })
            .collect(),
    )
}

/// Whether the copy of custom set `site` in `folder` is an older version than the website's:
/// a difficulty of the website's isn't in it, and nothing in it changed since the upload (a
/// mapper's own folder they've gone on editing isn't old, it's ahead).
pub fn outdated(folder: &Path, site: &SiteSet) -> bool {
    if site.hashes.is_empty() {
        return false;
    }
    let Ok(dir) = std::fs::read_dir(folder) else {
        return false;
    };
    let mut hashes = Vec::new();
    let mut newest = 0;
    for entry in dir.flatten() {
        let path = entry.path();
        let ext = extension(&path);
        if ext != "osu" && ext != "tko" {
            continue;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        hashes.push(crate::net::map_hash(&bytes));
        newest = newest.max(crate::library::file_stats(&path).1);
    }
    site.hashes.iter().any(|h| !hashes.contains(h)) && newest < site.updated
}

fn answer(text: &str) -> Result<Sent, String> {
    let json: serde_json::Value =
        serde_json::from_str(text).map_err(|_| "the server's answer made no sense".to_string())?;
    let field = |k: &str| {
        json.get(k)
            .and_then(serde_json::Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
    };
    Ok(Sent {
        id: field("id").ok_or("the server's answer made no sense")?,
        version: field("version").unwrap_or(1),
    })
}

/// Packs the map's folder into a file in `dir` named after the folder: `.rsz` when a
/// difficulty of it is a `.tko` (for taiko.rs only), `.osz` otherwise (osu! opens it too). An
/// older export of the same name is replaced. Returns the file.
pub fn export(folder: &Path, dir: &Path) -> Result<PathBuf, String> {
    let packed = pack_export(folder)?;
    let tko = packed
        .sent
        .iter()
        .any(|n| n.to_ascii_lowercase().ends_with(".tko"));
    let stem: String = folder
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "map".into())
        .chars()
        .map(|c| {
            if "<>:\"/\\|?*".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let file = dir.join(format!(
        "{}.{}",
        stem.trim(),
        if tko { "rsz" } else { "osz" }
    ));
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // Written whole first, so a half-written file is never mistaken for the set.
    let part = file.with_extension("part");
    std::fs::write(&part, &packed.bytes)
        .and_then(|_| std::fs::rename(&part, &file))
        .map_err(|e| {
            let _ = std::fs::remove_file(&part);
            format!("{}: {e}", file.display())
        })?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn map(mode: u8, notes: usize) -> String {
        let mut text = format!(
            "osu file format v14\r\n\r\n[General]\r\nAudioFilename: audio.mp3\r\nMode: {mode}\r\n\r\n[HitObjects]\r\n"
        );
        for k in 0..notes {
            text += &format!("256,192,{},1,0\r\n", 1000 + k * 200);
        }
        text
    }

    /// Records that `folder` is website set `id`, keeping the rest of its manifest.
    fn write_sidecar(folder: &Path, id: u32) {
        let mut m = Manifest::read(folder).unwrap_or_default();
        m.id = Some(id);
        m.write(folder).unwrap();
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("taiko-custom-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sb")).unwrap();
        dir
    }

    #[test]
    fn mode_and_notes_are_read() {
        assert_eq!(mode_and_notes(&map(1, 12)), (Some(1), 12));
        assert_eq!(mode_and_notes(&map(0, 3)), (Some(0), 3));
        assert_eq!(mode_and_notes("nothing"), (None, 0));
    }

    #[test]
    fn the_sidecar_remembers_the_set() {
        let dir = folder("sidecar");
        assert_eq!(sidecar_id(&dir), None);
        write_sidecar(&dir, 42);
        assert_eq!(sidecar_id(&dir), Some(42));
        std::fs::write(dir.join(SIDECAR), "junk").unwrap();
        assert_eq!(sidecar_id(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_manifest_reads_version_one_and_keeps_what_it_doesnt_know() {
        // What the website puts in its archives.
        let v1 = Manifest::parse("{\"id\": 12}\n").unwrap();
        assert_eq!(v1.id, Some(12));
        assert_eq!(v1.version, None);
        assert!(v1.files.is_empty());
        // A local set, never sent: no id at all.
        assert_eq!(Manifest::parse("{\"origin\": \"local\"}").unwrap().id, None);
        assert_eq!(Manifest::parse("junk"), None);
        // Round trip, with a newer client's field kept.
        let dir = folder("manifest");
        let mut m = Manifest::parse("{\"id\": 12, \"later\": [1, 2]}").unwrap();
        m.version = Some(4);
        m.updated = Some(1_763_251_200);
        m.origin = Some(Origin::Site);
        m.based_on = Some(622_136);
        m.installed = Some(1_763_300_000);
        m.add_file("a [Inner Oni].tko", 0x4afe_7841_b490_d3f8);
        m.write(&dir).unwrap();
        let text = std::fs::read_to_string(dir.join(SIDECAR)).unwrap();
        assert!(
            text.contains("\"a [Inner Oni].tko\": \"4afe7841b490d3f8\""),
            "{text}"
        );
        assert!(text.contains("\"origin\": \"site\""), "{text}");
        let back = Manifest::read(&dir).unwrap();
        assert_eq!(back, m);
        assert_eq!(back.rest.get("later"), Some(&serde_json::json!([1, 2])));
        assert_eq!(
            back.file_hash("a [Inner Oni].tko"),
            Some(0x4afe_7841_b490_d3f8)
        );
        assert_eq!(back.file_hash("other.tko"), None);
        assert!(!dir.join(format!("{SIDECAR}.part")).exists());
        // Writing the id again keeps the rest.
        write_sidecar(&dir, 13);
        let again = Manifest::read(&dir).unwrap();
        assert_eq!(again.id, Some(13));
        assert_eq!(again.version, Some(4));
        assert_eq!(again.files, m.files);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_sent_folder_records_its_version_and_charts() {
        let dir = folder("sent");
        std::fs::write(dir.join("a [Oni].tko"), map(1, 20)).unwrap();
        std::fs::write(dir.join("a [Mine].osu"), map(1, 20)).unwrap();
        mark_sent(
            &dir,
            &Sent { id: 9, version: 3 },
            &["a [Oni].tko".to_string()],
        );
        let m = Manifest::read(&dir).unwrap();
        assert_eq!(
            (m.id, m.version, m.origin),
            (Some(9), Some(3), Some(Origin::Local))
        );
        assert_eq!(
            m.file_hash("a [Oni].tko"),
            Some(crate::net::map_hash(map(1, 20).as_bytes()))
        );
        assert_eq!(m.files.len(), 1, "only what went");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn packing_takes_the_set_and_leaves_the_rest() {
        let dir = folder("pack");
        std::fs::write(dir.join("a [Oni].osu"), map(1, 20)).unwrap();
        std::fs::write(dir.join("a [Inner].tko"), map(1, 30)).unwrap();
        std::fs::write(dir.join("a [Empty].osu"), map(1, 2)).unwrap();
        std::fs::write(dir.join("a [Std].osu"), map(0, 50)).unwrap();
        std::fs::write(dir.join("a [Oni].osu.old"), map(1, 20)).unwrap();
        std::fs::write(dir.join("a [Oni].osu.bak"), map(1, 20)).unwrap();
        std::fs::write(dir.join("audio.mp3"), [1u8; 64]).unwrap();
        std::fs::write(dir.join("bg.png"), [2u8; 16]).unwrap();
        std::fs::write(dir.join("sb/star.png"), [3u8; 16]).unwrap();
        // An effect's animated pictures go along.
        std::fs::write(dir.join("fx.gif"), [4u8; 16]).unwrap();
        std::fs::write(dir.join("sb/fx.webp"), [5u8; 16]).unwrap();
        std::fs::write(dir.join("notes.txt"), "mine").unwrap();
        write_sidecar(&dir, 7);
        let packed = pack(&dir).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(packed.bytes.clone())).unwrap();
        let mut names: Vec<String> = (0..zip.len())
            .map(|i| {
                zip.by_index(i)
                    .unwrap()
                    .enclosed_name()
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "a [Inner].tko",
                "a [Oni].osu",
                "audio.mp3",
                "bg.png",
                "fx.gif",
                "sb/fx.webp",
                "sb/star.png"
            ]
        );
        assert_eq!(packed.sent, ["a [Inner].tko", "a [Oni].osu"]);
        assert_eq!(packed.left_out.len(), 1);
        assert!(packed.left_out[0].contains("Empty"));
        // What went in is the file itself, byte for byte (the server hashes it).
        let mut got = Vec::new();
        zip.by_name("a [Oni].osu")
            .unwrap()
            .read_to_end(&mut got)
            .unwrap();
        assert_eq!(got, std::fs::read(dir.join("a [Oni].osu")).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_edit_of_an_osu_map_goes_only_with_its_original_mapper_named() {
        let osu = |version: &str, credit: &str| {
            map(1, 20).replace(
                "[HitObjects]",
                &format!(
                    "[Metadata]\r\nVersion:{version}\r\nBeatmapSetID:123\r\n{credit}\r\n[HitObjects]"
                ),
            )
        };
        assert_eq!(edit_of(&map(1, 20)), (None, None));
        assert_eq!(edit_of(&osu("Oni", "")), (Some(123), None));
        assert_eq!(edit_of(&osu("Oni", "TaikoRsBasedOn:  ")), (Some(123), None));
        assert_eq!(
            edit_of(&osu("Oni", "TaikoRsBasedOn: Someone Else ")),
            (Some(123), Some("Someone Else".to_string()))
        );
        assert_eq!(
            edit_of(
                &map(1, 20).replace("[HitObjects]", "[Metadata]\nBeatmapSetID:-1\n[HitObjects]")
            ),
            (None, None)
        );
        let dir = folder("edit");
        std::fs::write(
            dir.join("a [Mine].osu"),
            osu("Mine", "TaikoRsBasedOn:Someone"),
        )
        .unwrap();
        std::fs::write(dir.join("a [Theirs].osu"), osu("Theirs", "")).unwrap();
        std::fs::write(dir.join("audio.mp3"), [1u8; 8]).unwrap();
        let packed = pack(&dir).unwrap();
        assert_eq!(packed.sent, ["a [Mine].osu"]);
        assert_eq!(
            packed.left_out,
            ["a [Theirs].osu (an osu! map, no original mapper named)"]
        );
        // A file to give to people keeps the whole set.
        assert_eq!(pack_export(&dir).unwrap().sent.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_without_a_sendable_map_is_refused_in_words() {
        let dir = folder("none");
        std::fs::write(dir.join("a.osu"), map(1, 3)).unwrap();
        std::fs::write(dir.join("audio.mp3"), [1u8; 8]).unwrap();
        assert!(pack(&dir).unwrap_err().contains("at least 10 notes"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_servers_answer_is_read() {
        assert_eq!(
            answer(r#"{"id": 5, "version": 2, "url": "/c/5"}"#),
            Ok(Sent { id: 5, version: 2 })
        );
        assert!(answer("nope").is_err());
        assert!(answer("{}").is_err());
    }

    #[test]
    fn approved_sets_are_picked_from_the_list() {
        let text = r#"[{"id": 1, "approved": true}, {"id": 2, "approved": false}, {"id": 3, "approved": true}]"#;
        let approved = |sets: Vec<SiteSet>| -> Vec<u32> {
            sets.iter().filter(|s| s.approved).map(|s| s.id).collect()
        };
        assert_eq!(sets_in(text).map(approved), Some(vec![1, 3]));
        assert_eq!(sets_in("[]"), Some(vec![]));
        assert_eq!(sets_in("oops"), None);
    }

    #[test]
    fn a_sets_difficulties_are_known_by_their_hashes() {
        let text = r#"[{"id":12,"diffs":[{"version":"Go backwards (TKO)","hash":"4afe7841b490d3f8","tko":true}],"updated":1794723058,"approved":true}]"#;
        assert_eq!(
            sets_in(text),
            Some(vec![SiteSet {
                id: 12,
                approved: true,
                hashes: vec![0x4afe_7841_b490_d3f8],
                updated: 1_794_723_058,
            }])
        );
    }

    #[test]
    fn an_older_copy_of_a_custom_set_is_outdated_and_a_mappers_newer_edit_is_not() {
        let dir = folder("outdated");
        std::fs::write(dir.join("a [Oni].tko"), "old version").unwrap();
        let new = crate::net::map_hash(b"new version");
        let now = crate::library::file_stats(&dir.join("a [Oni].tko")).1;
        let site = |updated| SiteSet {
            id: 1,
            approved: true,
            hashes: vec![new],
            updated,
        };
        // Uploaded after this copy was made: an older version.
        assert!(outdated(&dir, &site(now + 60)));
        // The folder changed after the upload: the mapper's own edit, ahead of the site.
        assert!(!outdated(&dir, &site(now - 60)));
        // The copy has the website's difficulty: up to date, whatever else is in the folder.
        std::fs::write(dir.join("a [Oni].tko"), "new version").unwrap();
        std::fs::write(dir.join("a [Mine].osu"), "home-made").unwrap();
        assert!(!outdated(&dir, &site(now + 3600)));
        assert!(!outdated(&dir, &SiteSet::default()));
    }

    #[test]
    fn a_sets_page_is_on_our_site() {
        assert_eq!(page_url(9), format!("{WEBSITE}/c/9"));
    }

    #[test]
    fn an_export_is_an_rsz_with_a_tko_and_an_osz_without() {
        let dir = folder("export");
        let out = dir.join("Exports");
        std::fs::write(dir.join("a [Oni].osu"), map(1, 3)).unwrap();
        std::fs::write(dir.join("audio.mp3"), [1u8; 64]).unwrap();
        write_sidecar(&dir, 7);
        // Only an .osu, with fewer notes than an upload needs: still a file to give to people.
        let osz = export(&dir, &out).unwrap();
        assert_eq!(osz.extension().unwrap(), "osz");
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&osz).unwrap()).unwrap();
        assert!(zip.by_name("a [Oni].osu").is_ok() && zip.by_name("audio.mp3").is_ok());
        assert!(zip.by_name(SIDECAR).is_err(), "the set number stays home");
        // With a .tko it is taiko.rs's own archive, and an export again replaces it.
        std::fs::write(dir.join("a [Fx].tko"), map(1, 3)).unwrap();
        let rsz = export(&dir, &out).unwrap();
        assert_eq!(rsz.extension().unwrap(), "rsz");
        assert_eq!(rsz.file_stem(), osz.file_stem());
        let again = export(&dir, &out).unwrap();
        assert_eq!(again, rsz);
        let maps = crate::download::archive_maps(&rsz).unwrap();
        assert_eq!(maps.len(), 2, "both difficulties count as maps");
        // Nothing to play: nothing written.
        let empty = folder("export-empty");
        assert!(export(&empty, &empty.join("Exports")).is_err());
        assert!(!empty.join("Exports").exists());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&empty);
    }

    #[test]
    fn the_websites_list_becomes_custom_sets_that_no_osu_set_is_taken_for() {
        let text = r#"[
            {"id": 4, "artist": "A", "title": "T/x", "creator": "M", "owner": "M",
             "diffs": [{"version": "Oni", "hash": "1", "tko": false, "approved": true},
                       {"version": "Oni (fx)", "hash": "2", "tko": true, "approved": true}],
             "updated": 20, "version": 3, "downloads": 7, "bytes": 1048576,
             "background": true, "approved": true},
            {"id": 9, "artist": "B", "title": "U", "creator": "N", "diffs": [], "approved": false},
            {"id": 5, "artist": "C", "title": "V", "creator": "N",
             "diffs": [{"version": "Muzukashii"}], "updated": 30}
        ]"#;
        let sets = parse_list(text).unwrap();
        // A set with no difficulty has nothing to play.
        assert_eq!(sets.iter().map(|s| s.id).collect::<Vec<_>>(), [4, 5]);
        let a = &sets[0];
        let custom = a.custom.as_ref().unwrap();
        assert!(custom.approved && custom.tko);
        assert_eq!(
            (custom.version, custom.bytes, a.play_count),
            (3, 1 << 20, 7)
        );
        assert_eq!(a.diffs.iter().map(|d| d.id).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(a.diffs[1].version, "Oni (fx)");
        assert!(!sets[1].custom.as_ref().unwrap().approved);
        // Its folder isn't read as osu! set 4, and it isn't the same set as osu!'s 4.
        let folder = a.folder_name();
        assert_eq!(folder, "c4 A - T_x");
        assert_eq!(crate::download::set_id_of_folder(Path::new(&folder)), None);
        let osu = SetInfo {
            id: 4,
            ..SetInfo::default()
        };
        assert!(!a.same(&osu) && a.same(&a.clone()));
        assert!(osu.is_osu_set(Some(4)) && !a.is_osu_set(Some(4)));
    }

    #[test]
    fn custom_sets_list_newest_first_without_search_text() {
        use crate::download::{SortKey, sort_sets};
        let mut sets = parse_list(
            r#"[{"id": 1, "title": "b", "diffs": [{"version": "x"}], "updated": 10, "downloads": 5},
                {"id": 2, "title": "a", "diffs": [{"version": "x"}], "updated": 30, "downloads": 1},
                {"id": 3, "title": "c", "diffs": [{"version": "x"}], "updated": 20, "downloads": 9}]"#,
        )
        .unwrap();
        let ids = |s: &[SetInfo]| s.iter().map(|s| s.id).collect::<Vec<_>>();
        sort_sets(&mut sets, SortKey::Ranked, true);
        assert_eq!(ids(&sets), [2, 3, 1]);
        sort_sets(&mut sets, SortKey::Plays, true);
        assert_eq!(ids(&sets), [3, 1, 2]);
        sort_sets(&mut sets, SortKey::Title, false);
        assert_eq!(ids(&sets), [2, 1, 3]);
    }
}
