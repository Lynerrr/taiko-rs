//! Moving taiko.rs's maps out of osu!'s Songs folder into its own (`config::custom_dir`).
//! Older games put website sets, editor maps and imported `.tko` sets in osu!'s Songs folder,
//! where osu! lists them as its own. Only what is surely taiko.rs's moves: a set folder with the
//! website's marker, one named the way the game names website sets (`c<id> Artist - Title`), or
//! one whose charts are all `.tko` (osu! can't play those). An osu! set's folder (`<id> Artist -
//! Title`) stays where it is even with a taiko.rs difficulty in it: moving it would take the set
//! away from osu!.
//!
//! Each set folder moves whole, by a rename, or across drives by a copy checked file by file
//! before the original goes. What moved is noted in `moved.json` beside the custom folder's
//! versions, so it can all be moved back.

use std::path::{Path, PathBuf};

use super::{Manifest, Origin, SIDECAR};

/// The list of the folders moved, in the versions folder.
const MOVED: &str = "moved.json";

/// A set folder in a Songs folder that is taiko.rs's.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub folder: PathBuf,
    /// Its website set, if it's known.
    pub id: Option<u32>,
    /// What its files take on disk.
    pub bytes: u64,
}

/// One folder moved: where it was and where it is.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Moved {
    pub from: PathBuf,
    pub to: PathBuf,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Journal {
    #[serde(default)]
    moved: Vec<Moved>,
}

fn journal_path(custom: &Path) -> PathBuf {
    crate::config::custom_versions_dir(custom).join(MOVED)
}

fn read_journal(custom: &Path) -> Journal {
    std::fs::read_to_string(journal_path(custom))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write_journal(custom: &Path, j: &Journal) -> std::io::Result<()> {
    let path = journal_path(custom);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(j).map_err(std::io::Error::other)?;
    let part = path.with_extension("json.part");
    std::fs::write(&part, text)?;
    std::fs::rename(&part, &path)
}

/// The chart files (.osu, .tko) in a set folder and its subfolders one level down, as the
/// library reads them.
fn charts(folder: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(
                std::fs::read_dir(&p)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| crate::library::is_map_file(p)),
            );
        } else if crate::library::is_map_file(&p) {
            out.push(p);
        }
    }
    out
}

fn is_tko(path: &Path) -> bool {
    path.extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("tko"))
}

/// The website set a folder's name says it is: `c12 Artist - Title`.
fn named_id(name: &str) -> Option<u32> {
    let (num, rest) = name.strip_prefix('c')?.split_once(' ')?;
    (!num.is_empty() && num.bytes().all(|b| b.is_ascii_digit()) && !rest.is_empty())
        .then(|| num.parse().ok())
        .flatten()
}

/// Whether a folder's name is an osu! set's: `123456 Artist - Title`.
fn osu_named(name: &str) -> bool {
    name.split_once(' ')
        .is_some_and(|(num, _)| !num.is_empty() && num.bytes().all(|b| b.is_ascii_digit()))
}

fn size_of(path: &Path) -> u64 {
    match std::fs::symlink_metadata(path) {
        Ok(md) if md.is_dir() => std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| size_of(&e.path()))
            .sum(),
        Ok(md) => md.len(),
        Err(_) => 0,
    }
}

/// Whether set folder `folder` is surely taiko.rs's, and then what it is.
pub fn examine(folder: &Path) -> Option<Found> {
    let name = folder.file_name()?.to_string_lossy().into_owned();
    if osu_named(&name) {
        return None;
    }
    // Only charts right in the folder count: a folder whose charts are all a level down (a
    // Songs folder, a folder of packs) is no set.
    let charts: Vec<PathBuf> = charts(folder)
        .into_iter()
        .filter(|c| c.parent() == Some(folder))
        .collect();
    if charts.is_empty() {
        return None;
    }
    let marked = folder.join(SIDECAR).is_file();
    let id = super::sidecar_id(folder).or_else(|| named_id(&name));
    let ours = marked || named_id(&name).is_some() || charts.iter().all(|c| is_tko(c));
    ours.then(|| Found {
        folder: folder.to_path_buf(),
        id,
        bytes: size_of(folder),
    })
}

/// taiko.rs's set folders in the Songs folders `roots` (never in `custom` itself).
pub fn find(roots: &[PathBuf], custom: &Path) -> Vec<Found> {
    let mut found = Vec::new();
    for root in roots {
        if root.starts_with(custom) {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(root) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_dir()
                    && !p.starts_with(custom)
                    && !custom.starts_with(p)
                    && !p.starts_with(crate::config::custom_versions_dir(custom))
            })
            .collect();
        dirs.sort();
        found.extend(dirs.iter().filter_map(|d| examine(d)));
    }
    found
}

/// A name for `name` in `root` that nothing has yet: `name`, else `name (2)`, ...
fn free_name(root: &Path, name: &std::ffi::OsStr) -> PathBuf {
    let first = root.join(name);
    if !first.exists() {
        return first;
    }
    let name = name.to_string_lossy();
    (2..)
        .map(|n| root.join(format!("{name} ({n})")))
        .find(|p| !p.exists())
        .unwrap_or(first)
}

fn remove_tree(path: &Path) -> std::io::Result<()> {
    std::fs::remove_dir_all(path)
}

/// Whether every file under `from` is under `to` with the same size.
fn same_files(from: &Path, to: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(from) else {
        return false;
    };
    rd.flatten().all(|e| {
        let (a, b) = (e.path(), to.join(e.file_name()));
        match (std::fs::symlink_metadata(&a), std::fs::symlink_metadata(&b)) {
            (Ok(x), Ok(y)) if x.is_dir() && y.is_dir() => same_files(&a, &b),
            (Ok(x), Ok(y)) => x.is_file() && y.is_file() && x.len() == y.len(),
            _ => false,
        }
    })
}

/// Moves folder `from` to `to` (which mustn't exist yet): a rename, or (`rename` failing
/// across drives) a copy beside `to`, checked against the original before it takes `to`'s
/// place and the original goes (`remove`). If anything fails, `from` is as it was (or, if a
/// half-deleted original can't be put back whole, the whole copy stays at `to`): a failure
/// never loses a file.
pub(crate) fn move_folder(
    from: &Path,
    to: &Path,
    rename: impl Fn(&Path, &Path) -> std::io::Result<()>,
    remove: impl Fn(&Path) -> std::io::Result<()>,
) -> Result<(), String> {
    let shown = |e: std::io::Error| format!("{}: {e}", from.display());
    match rename(from, to) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() != std::io::ErrorKind::CrossesDevices => return Err(shown(e)),
        Err(_) => {}
    }
    let part = to.with_file_name(format!(
        "{}.part",
        to.file_name().unwrap_or_default().to_string_lossy()
    ));
    let _ = std::fs::remove_dir_all(&part);
    let copied = crate::config::copy_all(from, &part, &std::sync::atomic::AtomicU64::new(0))
        .map_err(shown)
        .and_then(|()| {
            if same_files(from, &part) {
                Ok(())
            } else {
                Err(format!("{}: the copy isn't whole", from.display()))
            }
        })
        .and_then(|()| std::fs::rename(&part, to).map_err(shown));
    if let Err(e) = copied {
        let _ = std::fs::remove_dir_all(&part);
        return Err(e);
    }
    if let Err(e) = remove(from) {
        // Some of it may have gone (a file in use): what went comes back from the copy, which
        // only goes once the original is whole again; else both stay, nothing lost.
        if put_back(to, from).is_ok() && same_files(to, from) {
            let _ = std::fs::remove_dir_all(to);
        }
        return Err(shown(e));
    }
    Ok(())
}

/// Copies back into `to` what of `from` is missing there.
fn put_back(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(from)?.is_dir() {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            put_back(&e.path(), &to.join(e.file_name()))?;
        }
        Ok(())
    } else if std::fs::symlink_metadata(to).is_err() {
        std::fs::copy(from, to).map(drop)
    } else {
        Ok(())
    }
}

/// Moves the set folders `found` (each right in one of the Songs folders `roots`, never one of
/// them) into `custom`, each whole, with its manifest saying what it
/// is. Returns what moved, and for each one that couldn't, why (it stays where it was).
pub fn move_all(found: &[Found], roots: &[PathBuf], custom: &Path) -> (Vec<Moved>, Vec<String>) {
    move_all_with(found, roots, custom, |a, b| std::fs::rename(a, b))
}

pub(crate) fn move_all_with(
    found: &[Found],
    roots: &[PathBuf],
    custom: &Path,
    rename: impl Fn(&Path, &Path) -> std::io::Result<()>,
) -> (Vec<Moved>, Vec<String>) {
    let (mut moved, mut failed) = (Vec::new(), Vec::new());
    if let Err(e) = std::fs::create_dir_all(custom) {
        failed.push(format!("{}: {e}", custom.display()));
        return (moved, failed);
    }
    let mut journal = read_journal(custom);
    for f in found {
        let Some(name) = f.folder.file_name() else {
            continue;
        };
        // Looked at again as it is now: only a set folder of taiko.rs's (never a Songs folder,
        // nor one holding the custom folder) ever moves.
        let in_a_root = f
            .folder
            .parent()
            .is_some_and(|p| roots.iter().any(|r| r == p));
        if !in_a_root
            || roots.contains(&f.folder)
            || custom.starts_with(&f.folder)
            || f.folder.starts_with(custom)
            || f.folder
                .starts_with(crate::config::custom_versions_dir(custom))
            || examine(&f.folder).is_none()
        {
            failed.push(format!(
                "{}: not a taiko.rs set any more",
                f.folder.display()
            ));
            continue;
        }
        let to = free_name(custom, name);
        match move_folder(&f.folder, &to, &rename, remove_tree) {
            Ok(()) => {
                let mut m = Manifest::read(&to).unwrap_or_default();
                if m.id.is_none() {
                    m.id = f.id;
                }
                if m.origin.is_none() {
                    m.origin = Some(if m.id.is_some() {
                        Origin::Site
                    } else {
                        Origin::Import
                    });
                }
                let _ = m.write(&to);
                let one = Moved {
                    from: f.folder.clone(),
                    to,
                };
                journal.moved.retain(|j| j.to != one.to);
                journal.moved.push(one.clone());
                // Noted at once: a crash later still knows where this one went.
                let _ = write_journal(custom, &journal);
                moved.push(one);
            }
            Err(e) => failed.push(e),
        }
    }
    (moved, failed)
}

/// How many folders were moved and are still where they went (what `move_back` would move).
pub fn moved_count(custom: &Path) -> usize {
    read_journal(custom)
        .moved
        .iter()
        .filter(|m| m.to.is_dir() && !m.from.exists())
        .count()
}

/// Moves every folder `move_all` moved back to where it was, unless something took its old
/// place (then it stays). Returns what moved back (`from` the custom folder, `to` the Songs
/// folder), and why the others couldn't.
pub fn move_back(custom: &Path) -> (Vec<Moved>, Vec<String>) {
    move_back_with(custom, |a, b| std::fs::rename(a, b))
}

pub(crate) fn move_back_with(
    custom: &Path,
    rename: impl Fn(&Path, &Path) -> std::io::Result<()>,
) -> (Vec<Moved>, Vec<String>) {
    let mut journal = read_journal(custom);
    let (mut back, mut failed) = (Vec::new(), Vec::new());
    journal.moved.retain(|m| {
        if !m.to.is_dir() {
            // Gone since (deleted, or moved by hand): nothing to move back.
            return false;
        }
        if m.from.exists() {
            failed.push(format!("{}: something else is there now", m.from.display()));
            return true;
        }
        match move_folder(&m.to, &m.from, &rename, remove_tree) {
            Ok(()) => {
                back.push(Moved {
                    from: m.to.clone(),
                    to: m.from.clone(),
                });
                false
            }
            Err(e) => {
                failed.push(e);
                true
            }
        }
    });
    let _ = write_journal(custom, &journal);
    (back, failed)
}

/// `path` with the folder it was in moved as `moves` say.
pub fn moved_path(path: &Path, moves: &[Moved]) -> Option<PathBuf> {
    moves
        .iter()
        .find_map(|m| Some(m.to.join(path.strip_prefix(&m.from).ok()?)))
}

/// Where a map file that isn't at `path` any more is, if it was moved into `custom`.
pub fn found_moved(path: &Path, custom: &Path) -> Option<PathBuf> {
    if path.exists() {
        return None;
    }
    moved_path(path, &read_journal(custom).moved).filter(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Place {
        root: PathBuf,
        songs: PathBuf,
        custom: PathBuf,
    }

    impl Place {
        fn new(name: &str) -> Place {
            let root =
                std::env::temp_dir().join(format!("taiko-migrate-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let (songs, custom) = (root.join("osu!/Songs"), root.join("data/Custom"));
            std::fs::create_dir_all(&songs).unwrap();
            Place {
                root,
                songs,
                custom,
            }
        }

        fn set(&self, name: &str, files: &[(&str, &str)]) -> PathBuf {
            let dir = self.songs.join(name);
            for (file, text) in files {
                let path = dir.join(file);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text).unwrap();
            }
            dir
        }
    }

    impl Drop for Place {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    const MARKER: (&str, &str) = (SIDECAR, "{\"id\": 12}\n");

    fn folders(found: &[Found]) -> Vec<String> {
        found
            .iter()
            .map(|f| f.folder.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn only_what_is_surely_taiko_rss_is_found() {
        let p = Place::new("find");
        p.set(
            "c12 KNOWER - Time Traveler",
            &[("a [Oni].osu", "osu"), MARKER],
        );
        // A website set without its marker (older games lost it): its name says it.
        p.set("c7 Artist - Old", &[("a [Oni].osu", "osu")]);
        p.set(
            "Mine - Song",
            &[("a [Oni].tko", "tko"), ("audio.mp3", "mp3")],
        );
        // osu!'s, even with a taiko.rs difficulty or marker in it.
        p.set(
            "622136 KNOWER - Time Traveler",
            &[("a [Oni].osu", "osu"), ("a [Mine].tko", "tko")],
        );
        p.set("622137 Other - Song", &[("a [Oni].osu", "osu"), MARKER]);
        // An unsubmitted osu! map: nothing says it's taiko.rs's.
        p.set("Someone - Unsubmitted", &[("a [Oni].osu", "osu")]);
        p.set(
            "Mixed - Song",
            &[("a [Oni].osu", "osu"), ("a [Ura].tko", "tko")],
        );
        p.set("c3 No - Charts", &[("audio.mp3", "mp3")]);
        let found = find(std::slice::from_ref(&p.songs), &p.custom);
        assert_eq!(
            folders(&found),
            [
                "Mine - Song",
                "c12 KNOWER - Time Traveler",
                "c7 Artist - Old"
            ]
        );
        let ids: Vec<Option<u32>> = found.iter().map(|f| f.id).collect();
        assert_eq!(ids, [None, Some(12), Some(7)]);
        assert_eq!(found[0].bytes, 6, "tko and mp3");
        // The custom folder itself is never looked in, even listed as a Songs folder.
        std::fs::create_dir_all(p.custom.join("c9 In - Custom")).unwrap();
        std::fs::write(p.custom.join("c9 In - Custom/a.tko"), "tko").unwrap();
        assert!(find(std::slice::from_ref(&p.custom), &p.custom).is_empty());
    }

    #[test]
    fn a_set_moves_whole_with_its_manifest_and_can_move_back() {
        let p = Place::new("move");
        let site = p.set(
            "c12 KNOWER - Time Traveler",
            &[("a [Oni].osu", "osu"), ("sb/x.png", "png"), MARKER],
        );
        let old = p.set("c7 Artist - Old", &[("a [Oni].osu", "osu")]);
        let found = find(std::slice::from_ref(&p.songs), &p.custom);
        let (moved, failed) = move_all(&found, std::slice::from_ref(&p.songs), &p.custom);
        assert!(failed.is_empty(), "{failed:?}");
        assert_eq!(moved.len(), 2);
        assert!(!site.exists() && !old.exists());
        let to = p.custom.join("c12 KNOWER - Time Traveler");
        assert_eq!(std::fs::read_to_string(to.join("sb/x.png")).unwrap(), "png");
        let m = Manifest::read(&to).unwrap();
        assert_eq!((m.id, m.origin), (Some(12), Some(Origin::Site)));
        // The marker-less one gets one, from its name.
        assert_eq!(
            Manifest::read(&p.custom.join("c7 Artist - Old"))
                .unwrap()
                .id,
            Some(7)
        );
        assert_eq!(moved_count(&p.custom), 2);
        assert_eq!(
            moved_path(&site.join("a [Oni].osu"), &moved),
            Some(to.join("a [Oni].osu"))
        );
        assert_eq!(
            found_moved(&site.join("a [Oni].osu"), &p.custom),
            Some(to.join("a [Oni].osu"))
        );
        // Back where they were, unless something took the place.
        std::fs::create_dir_all(&old).unwrap();
        let (back, failed) = move_back(&p.custom);
        assert_eq!(back.len(), 1);
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert!(site.join("a [Oni].osu").is_file() && !to.exists());
        assert!(p.custom.join("c7 Artist - Old").is_dir(), "stays");
        assert_eq!(moved_count(&p.custom), 0, "its old place is taken");
    }

    #[test]
    fn a_name_taken_in_the_custom_folder_gets_a_number() {
        let p = Place::new("clash");
        let set = p.set("Mine - Song", &[("a [Oni].tko", "tko")]);
        std::fs::create_dir_all(p.custom.join("Mine - Song")).unwrap();
        let (moved, _) = move_all(
            &[examine(&set).unwrap()],
            std::slice::from_ref(&p.songs),
            &p.custom,
        );
        assert_eq!(moved[0].to, p.custom.join("Mine - Song (2)"));
        assert_eq!(
            Manifest::read(&moved[0].to).unwrap().origin,
            Some(Origin::Import)
        );
    }

    #[test]
    fn across_drives_it_is_copied_checked_and_then_the_original_goes() {
        let p = Place::new("drives");
        let set = p.set(
            "Mine - Song",
            &[("a [Oni].tko", "tko"), ("sub/b.png", "png")],
        );
        let other_drive =
            |_: &Path, _: &Path| Err(std::io::Error::from(std::io::ErrorKind::CrossesDevices));
        let (moved, failed) = move_all_with(
            &[examine(&set).unwrap()],
            std::slice::from_ref(&p.songs),
            &p.custom,
            other_drive,
        );
        assert!(failed.is_empty(), "{failed:?}");
        let to = &moved[0].to;
        assert_eq!(
            std::fs::read_to_string(to.join("sub/b.png")).unwrap(),
            "png"
        );
        assert!(!set.exists());
        assert!(!p.custom.join("Mine - Song.part").exists());
        // Back the same way.
        let (back, failed) = move_back_with(&p.custom, other_drive);
        assert!(failed.is_empty(), "{failed:?}");
        assert_eq!(back.len(), 1);
        assert!(set.join("a [Oni].tko").is_file() && !to.exists());
    }

    #[test]
    fn an_original_that_only_half_goes_is_put_back_whole() {
        let p = Place::new("half");
        let set = p.set(
            "Mine - Song",
            &[("a [Oni].tko", "tko"), ("audio.mp3", "mp3")],
        );
        let to = p.custom.join("Mine - Song");
        std::fs::create_dir_all(&p.custom).unwrap();
        let other_drive =
            |_: &Path, _: &Path| Err(std::io::Error::from(std::io::ErrorKind::CrossesDevices));
        // The delete takes one file, then a file in use stops it.
        let half = |path: &Path| {
            std::fs::remove_file(path.join("audio.mp3"))?;
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        };
        assert!(move_folder(&set, &to, other_drive, half).is_err());
        assert_eq!(
            std::fs::read_to_string(set.join("audio.mp3")).unwrap(),
            "mp3"
        );
        assert_eq!(
            std::fs::read_to_string(set.join("a [Oni].tko")).unwrap(),
            "tko"
        );
        assert!(!to.exists(), "the copy goes once the original is whole");
    }

    #[test]
    fn a_songs_folder_or_one_holding_the_custom_folder_never_moves() {
        let p = Place::new("guard");
        p.set("Mine - Song", &[("a [Oni].tko", "tko")]);
        // A Songs folder passed as a set: it's no set (its charts are a level down).
        let songs = Found {
            folder: p.songs.clone(),
            id: None,
            bytes: 0,
        };
        assert_eq!(examine(&p.songs), None, "a Songs folder is no set");
        for roots in [vec![p.songs.clone()], vec![p.root.join("osu!")], Vec::new()] {
            let (moved, failed) = move_all(std::slice::from_ref(&songs), &roots, &p.custom);
            assert!(moved.is_empty() && failed.len() == 1, "{roots:?}");
            assert!(p.songs.join("Mine - Song/a [Oni].tko").is_file());
        }
        // A folder of packs: its charts are a level down too.
        p.set("Packs/Pack Set", &[("a [Oni].tko", "tko")]);
        assert_eq!(examine(&p.songs.join("Packs")), None);
        // A folder with the custom folder inside it, even with .tko charts.
        let holder = p.songs.join("taiko-rs");
        let custom = holder.join("Custom");
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::write(holder.join("x.tko"), "tko").unwrap();
        assert!(
            find(std::slice::from_ref(&p.songs), &custom)
                .iter()
                .all(|f| f.folder != holder)
        );
        let (moved, _) = move_all(
            &[Found {
                folder: holder.clone(),
                id: None,
                bytes: 0,
            }],
            std::slice::from_ref(&p.songs),
            &custom,
        );
        assert!(moved.is_empty());
        assert!(holder.join("x.tko").is_file() && custom.is_dir());
    }

    #[test]
    fn a_folder_that_cant_move_stays_where_it_was() {
        let p = Place::new("locked");
        let set = p.set("Mine - Song", &[("a [Oni].tko", "tko")]);
        let locked =
            |_: &Path, _: &Path| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied));
        let (moved, failed) = move_all_with(
            &[examine(&set).unwrap()],
            std::slice::from_ref(&p.songs),
            &p.custom,
            locked,
        );
        assert!(moved.is_empty());
        assert_eq!(failed.len(), 1);
        assert!(set.join("a [Oni].tko").is_file());
        assert_eq!(moved_count(&p.custom), 0);
    }
}
