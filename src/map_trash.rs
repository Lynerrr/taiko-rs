//! Deleting maps without losing them: a difficulty's file or a whole set's folder goes to the
//! system's recycle bin (Windows), Trash (macOS) or freedesktop trash (Linux). Where there's
//! none (Android), or the system won't take it (a drive without a trash of its own), it goes to
//! a `Trash` folder of the game's own instead, each item in a folder of its own beside a note
//! of where it came from. Nothing is ever removed for good: an item is only taken away from
//! where it was once it's whole somewhere else.

use std::path::{Path, PathBuf};

/// Where deleted maps go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Bin {
    /// The system's recycle bin, else (it won't take something) the game's own folder.
    System { fallback: Option<PathBuf> },
    /// Only the game's own folder.
    Folder(PathBuf),
}

/// Where an item went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Binned {
    System,
    /// The game's own folder: the item's new path.
    Folder(PathBuf),
}

/// Why an item didn't go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// It's not there (any more).
    Missing,
    /// It's still where it was: why (in use, no permission, no room).
    Stayed(String),
}

/// The note beside each item in the game's own folder: the path it was deleted from.
pub const FROM_NOTE: &str = "deleted-from.txt";

impl Bin {
    /// The game's: the system's recycle bin where there is one. Tests never reach the player's
    /// recycle bin or data folder: theirs is a folder under the temporary one.
    pub fn for_game() -> Bin {
        if cfg!(test) {
            return Bin::Folder(
                std::env::temp_dir().join(format!("taiko-rs-test-trash-{}", std::process::id())),
            );
        }
        let own = own_folder();
        match own {
            Some(dir) if !has_system_bin() => Bin::Folder(dir),
            fallback => Bin::System { fallback },
        }
    }

    /// Whether deleted maps go to the system's recycle bin (else the game's own folder).
    pub fn is_system(&self) -> bool {
        matches!(self, Bin::System { .. })
    }
}

/// Whether this platform has a recycle bin the game can put files in.
pub const fn has_system_bin() -> bool {
    !cfg!(target_os = "android")
}

/// The game's own `Trash` folder, next to its own Songs and Skins.
fn own_folder() -> Option<PathBuf> {
    #[cfg(target_os = "android")]
    {
        Some(crate::config::user_dir(false)?.join("Trash"))
    }
    #[cfg(not(target_os = "android"))]
    {
        crate::config::own_dir("Trash")
    }
}

/// Moves `path` (a file or a folder) to `bin`.
pub fn throw(bin: &Bin, path: &Path) -> Result<Binned, Failure> {
    if std::fs::symlink_metadata(path).is_err() {
        return Err(Failure::Missing);
    }
    match bin {
        Bin::Folder(dir) => into_folder(dir, path, true).map(Binned::Folder),
        Bin::System { fallback } => match to_system(path) {
            Ok(()) => Ok(Binned::System),
            Err(why) => {
                eprintln!("{}: the recycle bin didn't take it: {why}", path.display());
                match fallback {
                    Some(dir) => into_folder(dir, path, true).map(Binned::Folder),
                    None => Err(Failure::Stayed(why)),
                }
            }
        },
    }
}

#[cfg(not(target_os = "android"))]
fn to_system(path: &Path) -> Result<(), String> {
    // On a thread of its own: on Windows the recycle bin is a COM call, and the thread it's
    // made on must not have COM set up another way already (audio, the window).
    let path = path.to_path_buf();
    std::thread::scope(|s| {
        s.spawn(|| trash::delete(&path).map_err(|e| e.to_string()))
            .join()
            .unwrap_or_else(|_| Err("the recycle bin call failed".into()))
    })
}

#[cfg(target_os = "android")]
fn to_system(_path: &Path) -> Result<(), String> {
    Err("no recycle bin here".into())
}

/// Moves `path` into a new folder of its own in `bin` (with a note of where it came from) and
/// returns its new path. `rename`: try a plain rename first (false: copy, as across drives).
/// A copy is checked whole before the original goes; if the original can't all go (a file in
/// use), what of it went is put back and the copy is dropped, so it stays as it was.
pub(crate) fn into_folder(bin: &Path, path: &Path, rename: bool) -> Result<PathBuf, Failure> {
    let stayed = |e: std::io::Error| Failure::Stayed(e.to_string());
    if std::fs::symlink_metadata(path).is_err() {
        return Err(Failure::Missing);
    }
    let name = path
        .file_name()
        .ok_or_else(|| Failure::Stayed("it has no name".into()))?;
    std::fs::create_dir_all(bin).map_err(stayed)?;
    let slot = free_slot(bin).map_err(stayed)?;
    let to = slot.join(name);
    let note = |slot: &Path| {
        let _ = std::fs::write(
            slot.join(FROM_NOTE),
            format!("{}\n", path.to_string_lossy()),
        );
    };
    if rename && std::fs::rename(path, &to).is_ok() {
        note(&slot);
        return Ok(to);
    }
    if let Err(e) = copy_all(path, &to) {
        let _ = std::fs::remove_dir_all(&slot);
        return Err(stayed(e));
    }
    if !same_tree(path, &to) {
        let _ = std::fs::remove_dir_all(&slot);
        return Err(Failure::Stayed("the copy didn't match".into()));
    }
    if let Err(e) = remove_all(path) {
        // Some of it may have gone: back from the copy, and the copy goes.
        let back = put_back(&to, path);
        if back.is_ok() {
            let _ = std::fs::remove_dir_all(&slot);
        }
        return Err(stayed(e));
    }
    note(&slot);
    Ok(to)
}

/// A new, empty folder in `bin`, named for now.
fn free_slot(bin: &Path) -> std::io::Result<PathBuf> {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    for n in 0u32.. {
        let slot = bin.join(if n == 0 {
            secs.to_string()
        } else {
            format!("{secs}-{n}")
        });
        match std::fs::create_dir(&slot) {
            Ok(()) => return Ok(slot),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

fn copy_all(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(from)?.is_dir() {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            copy_all(&e.path(), &to.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(drop)
    }
}

fn remove_all(p: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(p)?.is_dir() {
        std::fs::remove_dir_all(p)
    } else {
        std::fs::remove_file(p)
    }
}

/// Whether `copy` holds every file of `original` at its size.
fn same_tree(original: &Path, copy: &Path) -> bool {
    let (Ok(a), Ok(b)) = (
        std::fs::symlink_metadata(original),
        std::fs::symlink_metadata(copy),
    ) else {
        return false;
    };
    if a.is_dir() != b.is_dir() {
        return false;
    }
    if !a.is_dir() {
        return a.len() == b.len();
    }
    let Ok(rd) = std::fs::read_dir(original) else {
        return false;
    };
    rd.flatten()
        .all(|e| same_tree(&e.path(), &copy.join(e.file_name())))
}

/// Copies back from `copy` whatever of `original` is gone.
fn put_back(copy: &Path, original: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(copy)?.is_dir() {
        std::fs::create_dir_all(original)?;
        for e in std::fs::read_dir(copy)? {
            let e = e?;
            put_back(&e.path(), &original.join(e.file_name()))?;
        }
        Ok(())
    } else if std::fs::symlink_metadata(original).is_err() {
        std::fs::copy(copy, original).map(drop)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of its own under the temporary one, empty.
    fn temp(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("taiko-rs-map-trash-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A set folder: two charts, music and a background in a subfolder.
    fn set(root: &Path) -> PathBuf {
        let set = root.join("1 Artist - Title");
        std::fs::create_dir_all(set.join("bg")).unwrap();
        std::fs::write(set.join("a [Easy].osu"), "easy").unwrap();
        std::fs::write(set.join("a [Oni].osu"), "oni").unwrap();
        std::fs::write(set.join("audio.mp3"), vec![7u8; 4096]).unwrap();
        std::fs::write(set.join("bg/bg.jpg"), "jpeg").unwrap();
        set
    }

    fn from_note(moved: &Path) -> String {
        std::fs::read_to_string(moved.parent().unwrap().join(FROM_NOTE)).unwrap()
    }

    #[test]
    fn a_tests_bin_is_a_temporary_folder_never_the_players() {
        match Bin::for_game() {
            Bin::Folder(dir) => assert!(dir.starts_with(std::env::temp_dir())),
            other => panic!("tests reached {other:?}"),
        }
    }

    #[test]
    fn a_chart_goes_into_the_folder_with_a_note_of_where_it_was() {
        let root = temp("chart");
        let set = set(&root.join("songs"));
        let chart = set.join("a [Oni].osu");
        let bin = Bin::Folder(root.join("trash"));
        let Ok(Binned::Folder(moved)) = throw(&bin, &chart) else {
            panic!("not moved");
        };
        assert!(!chart.exists());
        assert!(moved.starts_with(root.join("trash")));
        assert_eq!(std::fs::read_to_string(&moved).unwrap(), "oni");
        assert_eq!(from_note(&moved).trim(), chart.to_string_lossy());
        // The rest of the set stays.
        assert!(set.join("a [Easy].osu").is_file() && set.join("audio.mp3").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_whole_set_goes_as_its_folder() {
        let root = temp("set");
        let set = set(&root.join("songs"));
        let moved = into_folder(&root.join("trash"), &set, true).unwrap();
        assert!(!set.exists());
        assert_eq!(std::fs::read(moved.join("audio.mp3")).unwrap(), [7u8; 4096]);
        assert!(moved.join("bg/bg.jpg").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn across_drives_it_is_copied_whole_then_the_original_goes() {
        let root = temp("copy");
        let set = set(&root.join("songs"));
        let moved = into_folder(&root.join("trash"), &set, false).unwrap();
        assert!(!set.exists());
        assert_eq!(std::fs::read(moved.join("audio.mp3")).unwrap(), [7u8; 4096]);
        assert_eq!(
            std::fs::read_to_string(moved.join("a [Easy].osu")).unwrap(),
            "easy"
        );
        assert!(moved.join("bg/bg.jpg").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_same_name_twice_keeps_both() {
        let root = temp("twice");
        let songs = root.join("songs");
        let bin = root.join("trash");
        std::fs::create_dir_all(&songs).unwrap();
        let chart = songs.join("x.osu");
        std::fs::write(&chart, "one").unwrap();
        let first = into_folder(&bin, &chart, true).unwrap();
        std::fs::write(&chart, "two").unwrap();
        let second = into_folder(&bin, &chart, true).unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read_to_string(first).unwrap(), "one");
        assert_eq!(std::fs::read_to_string(second).unwrap(), "two");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_file_is_said_to_be_missing_and_nothing_is_made() {
        let root = temp("missing");
        let bin = root.join("trash");
        let gone = root.join("songs/1 a/nothing.osu");
        assert_eq!(
            throw(&Bin::Folder(bin.clone()), &gone),
            Err(Failure::Missing)
        );
        assert_eq!(into_folder(&bin, &gone, false), Err(Failure::Missing));
        assert!(!bin.exists(), "no empty folder left in the bin");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_bin_that_cant_be_made_leaves_the_file_where_it_was() {
        let root = temp("no-bin");
        let chart = root.join("x.osu");
        std::fs::write(&chart, "keep").unwrap();
        // A file where the bin's folder would be.
        let bin = root.join("trash");
        std::fs::write(&bin, "in the way").unwrap();
        assert!(matches!(
            throw(&Bin::Folder(bin), &chart),
            Err(Failure::Stayed(_))
        ));
        assert_eq!(std::fs::read_to_string(&chart).unwrap(), "keep");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A set whose folder can't be changed (as a file in use on Windows): it stays whole, and
    /// the copy made of it doesn't stay in the bin.
    #[cfg(unix)]
    #[test]
    fn a_locked_set_stays_whole_and_the_bin_keeps_no_copy() {
        use std::os::unix::fs::PermissionsExt;
        let root = temp("locked");
        let songs = root.join("songs");
        let set = set(&songs);
        let bin = root.join("trash");
        let lock = |mode| {
            std::fs::set_permissions(&songs, std::fs::Permissions::from_mode(mode)).unwrap();
            std::fs::set_permissions(&set, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        lock(0o555);
        // Root can change anything: nothing to check then.
        let writable = std::fs::write(set.join("probe"), "").is_ok();
        let chart = set.join("a [Oni].osu");
        let one = into_folder(&bin, &chart, true);
        let whole = into_folder(&bin, &set, false);
        lock(0o755);
        if !writable {
            assert!(matches!(one, Err(Failure::Stayed(_))), "{one:?}");
            assert!(matches!(whole, Err(Failure::Stayed(_))), "{whole:?}");
            assert_eq!(std::fs::read_to_string(&chart).unwrap(), "oni");
            assert_eq!(std::fs::read(set.join("audio.mp3")).unwrap(), [7u8; 4096]);
            assert!(set.join("bg/bg.jpg").is_file());
            let left: Vec<_> = std::fs::read_dir(&bin).unwrap().flatten().collect();
            assert!(left.is_empty(), "the bin kept {left:?}");
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A chart open with no sharing (as osu! or an editor might hold it on Windows) stays.
    #[cfg(windows)]
    #[test]
    fn a_locked_chart_stays_where_it_was() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = temp("locked");
        let set = set(&root.join("songs"));
        let chart = set.join("a [Oni].osu");
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&chart)
            .unwrap();
        let result = into_folder(&root.join("trash"), &chart, true);
        drop(held);
        assert!(matches!(result, Err(Failure::Stayed(_))), "{result:?}");
        assert_eq!(std::fs::read_to_string(&chart).unwrap(), "oni");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn put_back_restores_only_what_went() {
        let root = temp("put-back");
        let set = set(&root.join("songs"));
        let copy = root.join("copy");
        copy_all(&set, &copy).unwrap();
        std::fs::remove_file(set.join("audio.mp3")).unwrap();
        std::fs::remove_dir_all(set.join("bg")).unwrap();
        std::fs::write(set.join("a [Easy].osu"), "edited meanwhile").unwrap();
        put_back(&copy, &set).unwrap();
        assert_eq!(std::fs::read(set.join("audio.mp3")).unwrap(), [7u8; 4096]);
        assert!(set.join("bg/bg.jpg").is_file());
        assert_eq!(
            std::fs::read_to_string(set.join("a [Easy].osu")).unwrap(),
            "edited meanwhile"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
