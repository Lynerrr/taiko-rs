//! taiko.rs's own folder of custom maps (`config::custom_dir`), apart from osu!'s Songs folder:
//! scanned whatever Songs folder is picked, looked at again like the Songs folders, and a lobby
//! host may send a set from it. osu! sets are still downloaded into osu!'s Songs folder.

use super::harness::*;
use super::maps;
use crate::*;
use std::path::{Path, PathBuf};

/// A Songs folder of osu!'s and taiko.rs's own custom maps folder, side by side.
struct Folders {
    root: PathBuf,
    songs: PathBuf,
    custom: PathBuf,
}

impl Folders {
    fn new(tag: &str) -> Folders {
        let root = std::env::temp_dir().join(format!(
            "taiko-rs-regression-custom-folder-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let (songs, custom) = (root.join("osu!/Songs"), root.join("taiko-rs/Custom"));
        std::fs::create_dir_all(&songs).unwrap();
        std::fs::create_dir_all(&custom).unwrap();
        Folders {
            root,
            songs,
            custom,
        }
    }
}

impl Drop for Folders {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Writes regression chart `n` as the only map of set folder `set` in `dir`.
fn put(dir: &Path, set: &str, n: usize) -> PathBuf {
    let chart = &maps::all()[n];
    let path = dir.join(set).join(format!("{}.osu", chart.name));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, chart.text()).unwrap();
    path
}

/// Song select with nothing listed yet, scanning `f`'s Songs folder (as if picked in settings)
/// and `f`'s custom maps folder.
fn client(f: &Folders) -> (State, Instant) {
    let mut s = state_with(config(), Vec::new());
    s.songs_dir = f.songs.clone();
    s.song_dirs = vec![f.songs.clone()];
    s.custom_dir = f.custom.clone();
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    (s, now)
}

fn refresh(s: &mut State, now: &mut Instant) {
    s.refresh_library(true);
    assert!(
        wait_for(s, now, |s| s.scan.is_none()),
        "the scan ends in time"
    );
}

fn listed(s: &State) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = s.library.iter().map(|e| e.path.clone()).collect();
    paths.sort();
    paths
}

#[test]
fn a_set_in_the_custom_folder_is_listed_beside_the_picked_songs_folder() {
    let f = Folders::new("listed");
    let osu = put(&f.songs, "1 Regression - osu", 0);
    let custom = put(&f.custom, "c12 Regression - custom", 1);
    let (mut s, mut now) = client(&f);
    refresh(&mut s, &mut now);
    let mut both = vec![osu.clone(), custom.clone()];
    both.sort();
    assert_eq!(listed(&s), both);
    // Kept apart from osu!'s: downloads, osu!.db and the Songs folder setting stay osu!'s.
    assert_eq!(s.songs_dir, f.songs);
    assert_eq!(s.song_dirs, vec![f.songs.clone()]);
    assert_eq!(s.scan_roots(), vec![f.songs.clone(), f.custom.clone()]);

    // A set put into the custom folder joins on the next look.
    let more = put(&f.custom, "Regression - new", 2);
    refresh(&mut s, &mut now);
    assert!(listed(&s).contains(&more), "{:?}", listed(&s));
    assert_eq!(s.library.len(), 3);
}

#[test]
fn the_custom_folder_alone_is_scanned_when_no_songs_folder_is_found() {
    let f = Folders::new("alone");
    let custom = put(&f.custom, "c12 Regression - custom", 1);
    let (mut s, mut now) = client(&f);
    s.song_dirs.clear();
    refresh(&mut s, &mut now);
    assert_eq!(listed(&s), vec![custom]);
}

#[test]
fn an_unplugged_songs_folder_keeps_its_maps_listed_beside_the_custom_folder() {
    let f = Folders::new("unplugged");
    let osu = put(&f.songs, "1 Regression - osu", 0);
    put(&f.custom, "c12 Regression - custom", 1);
    let (mut s, mut now) = client(&f);
    refresh(&mut s, &mut now);
    assert_eq!(s.library.len(), 2);
    let away = f.songs.with_file_name("Songs away");
    std::fs::rename(&f.songs, &away).unwrap();
    s.refresh_library(true);
    assert!(s.scan.is_none(), "nothing to look at while it's away");
    assert_eq!(
        s.toast.as_ref().map(|t| t.text.as_str()),
        Some("the songs folder can't be read just now")
    );
    assert!(listed(&s).contains(&osu), "osu!'s maps stay listed");
    std::fs::rename(&away, &f.songs).unwrap();
}

#[test]
fn a_lobby_host_may_send_a_set_from_the_custom_folder() {
    let f = Folders::new("send");
    let custom = put(&f.custom, "c12 Regression - custom", 1);
    let (s, _) = client(&f);
    let roots = s.lobby_send_roots();
    assert!(download::is_set_folder(custom.parent().unwrap(), &roots));
    // The folder itself, or one outside every root, is never a set to send.
    assert!(!download::is_set_folder(&f.custom, &roots));
    assert!(!download::is_set_folder(&f.root, &roots));
}
