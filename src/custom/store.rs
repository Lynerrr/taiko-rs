//! taiko.rs's own custom maps folder (`config::custom_dir`): a website set is installed and
//! updated as a whole folder. Its new version is put together beside the folder (in
//! `config::custom_versions_dir`), the player's own files are carried over, and then two renames
//! swap it in. The version it replaces is kept for going back to (`KEEP_VERSIONS`). A journal
//! names each swap while it's under way, so a crash between the renames is finished (or undone)
//! at the next start.
//!
//! The folder keeps its name across versions: favourites, skins turned off and collections know
//! a set by its folder's name, and plays by its path.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::{Manifest, Origin, SIDECAR, now_secs};

/// How many older versions of a set are kept.
pub const KEEP_VERSIONS: usize = 1;
/// The journal of swaps under way, in the versions folder.
pub const JOURNAL: &str = "journal.json";
/// What a kept older version's folder is called in its set's versions folder: `old-<n>`, the
/// highest `n` the newest.
const OLD: &str = "old-";
/// Where a new version is put together, in its set's versions folder.
const STAGE: &str = "new.partial";

/// Journal writes from download threads take turns.
static JOURNAL_TURN: Mutex<()> = Mutex::new(());

/// What a website set being installed is.
#[derive(Clone, Debug, Default)]
pub struct Install {
    /// Its number on the website.
    pub id: u32,
    /// The website's version, when known.
    pub version: Option<u32>,
    /// Seconds since 1970 of that version's upload, when known.
    pub updated: Option<u64>,
    pub origin: Option<Origin>,
    /// Its folder's name, if it has none here yet (`SetInfo::folder_name`).
    pub name: String,
}

/// One swap under way: `folder`'s version goes to `old` (if there was one), then `stage` takes
/// its place.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Swap {
    pub folder: PathBuf,
    pub old: Option<PathBuf>,
    pub stage: PathBuf,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Journal {
    #[serde(default)]
    swaps: Vec<Swap>,
}

fn versions(custom: &Path) -> PathBuf {
    crate::config::custom_versions_dir(custom)
}

/// The versions folder of website set `id`.
pub fn set_versions(custom: &Path, id: u32) -> PathBuf {
    versions(custom).join(id.to_string())
}

fn read_journal(custom: &Path) -> Journal {
    std::fs::read_to_string(versions(custom).join(JOURNAL))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write_journal(custom: &Path, j: &Journal) -> std::io::Result<()> {
    let dir = versions(custom);
    let path = dir.join(JOURNAL);
    if j.swaps.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    std::fs::create_dir_all(&dir)?;
    let part = dir.join(format!("{JOURNAL}.part"));
    std::fs::write(
        &part,
        serde_json::to_string_pretty(j).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(&part, &path)
}

fn journal_add(custom: &Path, swap: &Swap) -> std::io::Result<()> {
    let _turn = JOURNAL_TURN.lock().unwrap_or_else(|e| e.into_inner());
    let mut j = read_journal(custom);
    j.swaps.retain(|s| s.folder != swap.folder);
    j.swaps.push(swap.clone());
    write_journal(custom, &j)
}

fn journal_done(custom: &Path, folder: &Path) {
    let _turn = JOURNAL_TURN.lock().unwrap_or_else(|e| e.into_inner());
    let mut j = read_journal(custom);
    j.swaps.retain(|s| s.folder != folder);
    if let Err(e) = write_journal(custom, &j) {
        eprintln!("custom maps journal: {e}");
    }
}

/// The set folder in `custom` whose manifest names website set `id`, if there is one.
pub fn folder_of(custom: &Path, id: u32) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(custom)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && super::sidecar_id(p) == Some(id))
        .collect();
    found.sort();
    found.into_iter().next()
}

/// `name` in `custom`, or with a number if a folder of another set has it.
fn free_name(custom: &Path, name: &str, id: u32) -> PathBuf {
    let free = |p: &Path| !p.exists() || super::sidecar_id(p) == Some(id);
    let first = custom.join(name);
    if free(&first) {
        return first;
    }
    (2..)
        .map(|n| custom.join(format!("{name} ({n})")))
        .find(|p| free(p))
        .unwrap_or(first)
}

/// The kept older versions of a set (`old-<n>` folders), oldest first.
fn old_versions(set: &Path) -> Vec<(u32, PathBuf)> {
    let mut out: Vec<(u32, PathBuf)> = std::fs::read_dir(set)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_str()?.strip_prefix(OLD)?.parse().ok()?;
            e.path().is_dir().then(|| (n, e.path()))
        })
        .collect();
    out.sort();
    out
}

/// Where the version in a set folder goes when another takes its place.
fn next_old(set: &Path) -> PathBuf {
    let n = old_versions(set).last().map_or(1, |(n, _)| n + 1);
    set.join(format!("{OLD}{n}"))
}

fn is_chart(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("osu") || e.eq_ignore_ascii_case("tko"))
}

fn modified_secs(path: &Path) -> u64 {
    crate::library::file_stats(path).1
}

/// Every file under `dir`, relative to it.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = e.path();
            let Ok(kind) = e.file_type() else { continue };
            if kind.is_dir() {
                walk(root, &path, out);
            } else if kind.is_file()
                && let Ok(rel) = path.strip_prefix(root)
            {
                out.push(rel.to_path_buf());
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// The files in set folder `folder` that are the player's own, not its installed version's (and
/// so go along to a new version): a chart the install didn't bring (a mapper's own difficulty),
/// one changed since, and anything else put in or changed after the install, each with its
/// editor backup. Without a list of the install's charts (a version 1 manifest) only what changed
/// after the manifest was written counts: a chart the new version dropped is the old version's.
pub fn own_files(folder: &Path) -> Vec<PathBuf> {
    let manifest = Manifest::read(folder).unwrap_or_default();
    let installed = manifest
        .installed
        .unwrap_or_else(|| modified_secs(&folder.join(SIDECAR)));
    let files = files_under(folder);
    let mut own: Vec<PathBuf> = files
        .iter()
        .filter(|rel| {
            let name = rel.to_string_lossy();
            if name == SIDECAR || name.ends_with(".part") {
                return false;
            }
            if manifest.own.iter().any(|o| Path::new(o) == rel.as_path()) {
                return true;
            }
            let changed = modified_secs(&folder.join(rel)) > installed;
            let top_chart = rel.parent() == Some(Path::new("")) && is_chart(rel);
            if top_chart && !manifest.files.is_empty() {
                // One the install brought is the player's only once its contents changed.
                return match manifest.file_hash(&name) {
                    None => true,
                    Some(hash) => {
                        changed
                            && std::fs::read(folder.join(rel))
                                .is_ok_and(|b| crate::net::map_hash(&b) != hash)
                    }
                };
            }
            changed
        })
        .cloned()
        .collect();
    // An own chart's backup goes with it.
    let backups: Vec<PathBuf> = own
        .iter()
        .filter(|rel| is_chart(rel))
        .map(|rel| PathBuf::from(format!("{}.bak", rel.to_string_lossy())))
        .filter(|bak| files.contains(bak) && !own.contains(bak))
        .collect();
    own.extend(backups);
    own.sort();
    own.dedup();
    own
}

/// The osu! set a folder's charts say they were made from, if any.
fn based_on(dir: &Path) -> Option<u32> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_chart(p))
        .find_map(|p| super::edit_of(&std::fs::read_to_string(p).ok()?).0)
}

/// Installs website set `set` from `archive` into `custom`, whole: into the folder that already
/// holds it (whatever its name), else a new one named `set.name`. The folder's version so far is
/// kept, and the player's own files in it (`own_files`) go along to the new one. Returns the
/// set's folder. If anything fails before the swap, nothing in `custom` has changed.
pub fn install(custom: &Path, archive: &Path, set: &Install) -> Result<PathBuf, String> {
    std::fs::create_dir_all(custom).map_err(|e| format!("{}: {e}", custom.display()))?;
    let folder = folder_of(custom, set.id).unwrap_or_else(|| free_name(custom, &set.name, set.id));
    let dir = set_versions(custom, set.id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let stage = dir.join(STAGE);
    let _ = std::fs::remove_dir_all(&stage);
    crate::download::unpack(archive, &stage)?;
    let built = (|| {
        // What the archive brought: the manifest's list of the install's charts.
        let mut manifest = Manifest::read(&stage).unwrap_or_default();
        manifest.files.clear();
        for e in std::fs::read_dir(&stage)
            .map_err(|e| e.to_string())?
            .flatten()
        {
            let path = e.path();
            if path.is_file() && is_chart(&path) {
                let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                manifest.add_file(
                    &e.file_name().to_string_lossy(),
                    crate::net::map_hash(&bytes),
                );
            }
        }
        manifest.own.clear();
        if folder.is_dir() {
            for rel in own_files(&folder) {
                let to = stage.join(&rel);
                // The new version's file of that name wins; the old one stays with the old version.
                if to.exists() {
                    continue;
                }
                manifest.own.push(rel.to_string_lossy().replace('\\', "/"));
                if let Some(parent) = to.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::copy(folder.join(&rel), &to)
                    .map_err(|e| format!("{}: {e}", rel.display()))?;
            }
        }
        manifest.id = Some(set.id);
        if set.version.is_some() {
            manifest.version = set.version;
        }
        if set.updated.is_some() {
            manifest.updated = set.updated;
        }
        manifest.origin = set.origin.or(manifest.origin).or(Some(Origin::Site));
        manifest.based_on = manifest.based_on.or_else(|| based_on(&stage));
        manifest.installed = Some(now_secs());
        manifest.write(&stage).map_err(|e| e.to_string())
    })();
    if let Err(e) = built {
        let _ = std::fs::remove_dir_all(&stage);
        return Err(e);
    }
    let old = folder.is_dir().then(|| next_old(&dir));
    swap(
        custom,
        &Swap {
            folder: folder.clone(),
            old,
            stage,
        },
    )?;
    prune(&dir);
    Ok(folder)
}

/// Does `s`: journaled first, so `recover` can finish or undo it after a crash.
fn swap(custom: &Path, s: &Swap) -> Result<(), String> {
    journal_add(custom, s).map_err(|e| format!("journal: {e}"))?;
    if let Some(old) = &s.old
        && let Err(e) = std::fs::rename(&s.folder, old)
    {
        journal_done(custom, &s.folder);
        let _ = std::fs::remove_dir_all(&s.stage);
        return Err(format!("{}: {e}", s.folder.display()));
    }
    if let Err(e) = std::fs::rename(&s.stage, &s.folder) {
        // Back as it was.
        if let Some(old) = &s.old {
            let _ = std::fs::rename(old, &s.folder);
        }
        journal_done(custom, &s.folder);
        return Err(format!("{}: {e}", s.folder.display()));
    }
    journal_done(custom, &s.folder);
    Ok(())
}

/// Keeps a set's newest `KEEP_VERSIONS` older versions, and nothing half put together.
fn prune(dir: &Path) {
    let old = old_versions(dir);
    for (_, path) in old.iter().take(old.len().saturating_sub(KEEP_VERSIONS)) {
        let _ = std::fs::remove_dir_all(path);
    }
}

/// The newest kept older version of the set in `folder`, with its manifest.
pub fn previous_version(custom: &Path, folder: &Path) -> Option<(PathBuf, Manifest)> {
    let id = super::sidecar_id(folder)?;
    let (_, path) = old_versions(&set_versions(custom, id)).pop()?;
    let manifest = Manifest::read(&path).unwrap_or_default();
    Some((path, manifest))
}

/// Puts the kept older version of the set in `folder` back in its place; the version it
/// replaces is kept instead, so going back can be undone the same way (or by updating again).
pub fn roll_back(custom: &Path, folder: &Path) -> Result<(), String> {
    let (previous, _) = previous_version(custom, folder).ok_or("no older version is kept")?;
    let dir = previous.parent().ok_or("no versions folder")?.to_path_buf();
    swap(
        custom,
        &Swap {
            folder: folder.to_path_buf(),
            old: Some(next_old(&dir)),
            stage: previous,
        },
    )?;
    prune(&dir);
    Ok(())
}

/// After a crash: finishes each swap the journal names whose new version is ready, else puts
/// the old version back, and drops what was half put together. Returns the swaps it looked at.
pub fn recover(custom: &Path) -> Vec<Swap> {
    let journal = {
        let _turn = JOURNAL_TURN.lock().unwrap_or_else(|e| e.into_inner());
        read_journal(custom)
    };
    for s in &journal.swaps {
        let result = if s.folder.exists() {
            // The new version is in place, or the swap never started: what's left is spare.
            if s.stage.exists() && s.stage.file_name() == Some(STAGE.as_ref()) {
                std::fs::remove_dir_all(&s.stage)
            } else {
                Ok(())
            }
        } else if s.stage.is_dir() {
            std::fs::rename(&s.stage, &s.folder)
        } else if let Some(old) = s.old.as_ref().filter(|o| o.is_dir()) {
            std::fs::rename(old, &s.folder)
        } else {
            Ok(())
        };
        match result {
            Ok(()) => journal_done(custom, &s.folder),
            Err(e) => eprintln!("custom maps: {} not recovered ({e})", s.folder.display()),
        }
    }
    // Stages no journal names: an install stopped before its swap.
    for set in std::fs::read_dir(versions(custom))
        .into_iter()
        .flatten()
        .flatten()
    {
        let stage = set.path().join(STAGE);
        if stage.is_dir() && !read_journal(custom).swaps.iter().any(|s| s.stage == stage) {
            let _ = std::fs::remove_dir_all(&stage);
        }
    }
    journal.swaps
}
