//! TJA charts in the library (`taiko_logic::tja` reads them). A `.tja` file holds every course
//! of a song, and each course is an entry of its own, at the file's path with `#` and the
//! course after it (`song.tja#oni`), so everything that tells maps apart by their path keeps
//! doing so, and the entry's folder is the file's. Nothing opens that path as a file:
//! `beatmap::load`, `beatmap::read_meta_file`, `library::file_stats`, `Entry::hash` and the
//! map info read the `.tja` it names.
//!
//! TJA charts are read from their own folder only (`folder_for`, `Tja` beside `Custom` unless
//! the settings name another), never from osu!'s Songs, and stay on this computer: their plays
//! go to no server.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use taiko_logic::tja::{self, Course, CourseKind, Song};

use crate::beatmap::{Beatmap, Meta};

/// The TJA folders the library has read (`add_root`). Only ever added to, so a file read
/// while another test runs is never taken for one of them.
static ROOTS: RwLock<Vec<PathBuf>> = RwLock::new(Vec::new());

/// The TJA folder for the folder picked in settings: that one, else `Tja` in the game's data
/// folder (`config::tja_dir`; None in tests that pick none).
pub fn folder_for(picked: Option<&Path>) -> Option<PathBuf> {
    picked
        .map(Path::to_path_buf)
        .or_else(crate::config::tja_dir)
}

/// Makes `dir` a folder `.tja` files are read from.
pub fn add_root(dir: &Path) {
    let mut roots = ROOTS.write().unwrap_or_else(|e| e.into_inner());
    if !roots.iter().any(|r| r == dir) {
        roots.push(dir.to_path_buf());
    }
}

/// Whether `path` is in a TJA folder, where `.tja` files are read.
pub fn in_folder(path: &Path) -> bool {
    ROOTS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|r| path.starts_with(r))
}

/// A `.tja` file, by its name.
pub fn is_tja_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("tja"))
}

/// The entry path of course `kind` of the chart file `file`.
pub fn course_path(file: &Path, kind: CourseKind) -> PathBuf {
    let mut path = file.as_os_str().to_owned();
    path.push("#");
    path.push(kind.slug());
    path.into()
}

/// The chart file and course an entry path names; None for any other path.
pub fn split(path: &Path) -> Option<(PathBuf, CourseKind)> {
    let name = path.file_name()?.to_str()?;
    let (file, course) = name.rsplit_once('#')?;
    if !is_tja_file(Path::new(file)) {
        return None;
    }
    Some((path.with_file_name(file), CourseKind::parse(course)?))
}

/// Whether `path` is a TJA course's entry path.
pub fn is_course(path: &Path) -> bool {
    split(path).is_some()
}

/// The chart file at `file`, read.
pub fn read(file: &Path) -> Result<Song, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    tja::parse_bytes(&bytes).map_err(|e| format!("{}: {e}", file.display()))
}

/// The course an entry path names, read from its file.
pub fn course(path: &Path) -> Result<Course, String> {
    let (file, kind) =
        split(path).ok_or_else(|| format!("{}: not a TJA course", path.display()))?;
    read(&file)?
        .courses
        .into_iter()
        .find(|c| c.kind == kind)
        .ok_or_else(|| format!("{}: no {} chart", file.display(), kind.name()))
}

/// The entries the chart file `file` makes: each course's path and header, easiest first.
/// None when it can't be read.
pub fn entries(file: &Path) -> Option<Vec<(PathBuf, Meta)>> {
    let song = read(file).ok()?;
    Some(
        song.courses
            .into_iter()
            .map(|c| (course_path(file, c.kind), c.map.meta))
            .collect(),
    )
}

/// A course's chart, for play.
pub fn load(path: &Path) -> Result<Beatmap, String> {
    course(path).map(|c| c.map)
}

/// A course's chart as its online board knows it, sent with its plays.
pub fn board(path: &Path) -> Option<taiko_logic::tja::BoardChart> {
    course(path)
        .ok()
        .map(|c| taiko_logic::tja::BoardChart::of(c.kind, &c.map))
}

/// What a course's local scores and replays are kept by: its chart hash (the same for every
/// copy of the chart, whatever the file's encoding or comments), 0 when it can't be read.
pub fn hash(path: &Path) -> u64 {
    course(path).map_or(0, |c| c.hash.short())
}

/// Whether `dir` is a folder with a `.tja` file in it, or in a folder in it.
pub fn holds_tja(dir: &Path) -> bool {
    let has = |d: &Path| {
        std::fs::read_dir(d)
            .map(|rd| {
                rd.flatten()
                    .any(|e| is_tja_file(&e.path()) && e.path().is_file())
            })
            .unwrap_or(false)
    };
    dir.is_dir()
        && (has(dir)
            || std::fs::read_dir(dir)
                .is_ok_and(|rd| rd.flatten().any(|e| e.path().is_dir() && has(&e.path()))))
}

/// Copies a `.tja` file (with the audio it names) or a folder holding one into the TJA folder
/// `root`, into a folder of its own named after it: where it went.
pub fn import(from: &Path, root: &Path) -> Result<PathBuf, String> {
    if from.starts_with(root) {
        return Err("it's in the TJA folder already".into());
    }
    let name = |p: &Path, what: &str| {
        p.file_stem()
            .or_else(|| p.file_name())
            .map(|n| crate::download::safe_folder_name(&n.to_string_lossy()))
            .filter(|n| !n.is_empty())
            .ok_or_else(|| format!("{what} has no name"))
    };
    std::fs::create_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))?;
    if from.is_dir() {
        if !holds_tja(from) {
            return Err("there's no .tja chart in it".into());
        }
        let to = free_folder(root, &name(from, "the folder")?);
        copy_tree(from, &to)?;
        return Ok(to);
    }
    let song = read(from)?;
    let to = free_folder(root, &name(from, "the chart")?);
    let file_name = from.file_name().ok_or("the chart has no name")?;
    let copy = |src: &Path, dst: &Path| {
        if let Some(dir) = dst.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::copy(src, dst)
            .map(|_| ())
            .map_err(|e| format!("{}: {e}", src.display()))
    };
    copy(from, &to.join(file_name))?;
    // Its audio, where the chart says it is (beside it, or in a folder below).
    let wave = Path::new(&song.wave);
    let below = wave
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)));
    if !song.wave.is_empty() && below {
        let audio = from.with_file_name(wave);
        if audio.is_file() {
            copy(&audio, &to.join(wave))?;
        }
    }
    Ok(to)
}

/// `root/name`, or `root/name (2)` and so on when that's taken.
fn free_folder(root: &Path, name: &str) -> PathBuf {
    let mut to = root.join(name);
    let mut n = 2;
    while to.exists() {
        to = root.join(format!("{name} ({n})"));
        n += 1;
    }
    to
}

/// Copies the folder `from` (and what's in it) to `to`.
fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    let rd = std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?;
    for e in rd.flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            copy_tree(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst).map_err(|e| format!("{}: {e}", src.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_course_path_names_its_file_and_course() {
        let file = Path::new("/maps/Song/song.TJA");
        let p = course_path(file, CourseKind::Edit);
        assert_eq!(p, Path::new("/maps/Song/song.TJA#edit"));
        assert_eq!(split(&p), Some((file.to_path_buf(), CourseKind::Edit)));
        assert_eq!(p.parent(), file.parent());
        for other in [
            "/maps/a.osu",
            "/maps/a#oni.osu",
            "/maps/a.tja",
            "/maps/a.tja#hardest",
        ] {
            assert_eq!(split(Path::new(other)), None, "{other}");
        }
    }

    #[test]
    fn a_chart_is_imported_with_its_audio_into_a_folder_of_its_own() {
        let tmp = std::env::temp_dir().join(format!("taiko-tja-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let (from, root) = (tmp.join("dl"), tmp.join("Tja"));
        std::fs::create_dir_all(from.join("snd")).unwrap();
        let chart = from.join("Song.tja");
        std::fs::write(
            &chart,
            "TITLE:Song\nBPM:120\nWAVE:snd/song.ogg\nCOURSE:Oni\n#START\n1,\n#END\n",
        )
        .unwrap();
        std::fs::write(from.join("snd/song.ogg"), b"ogg").unwrap();
        std::fs::write(from.join("other.txt"), b"not copied").unwrap();
        let to = import(&chart, &root).unwrap();
        assert_eq!(to, root.join("Song"));
        assert!(to.join("Song.tja").is_file() && to.join("snd/song.ogg").is_file());
        assert!(!to.join("other.txt").exists());
        // Again: beside it, not over it.
        assert_eq!(import(&chart, &root).unwrap(), root.join("Song (2)"));
        // A folder holding a chart, whole.
        let folder = import(&from, &root).unwrap();
        assert!(folder.join("other.txt").is_file() && folder.join("Song.tja").is_file());
        assert!(import(&to, &root).is_err(), "already there");
        assert!(import(&from.join("snd"), &root).is_err(), "no chart in it");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
