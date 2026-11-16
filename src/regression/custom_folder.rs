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
    s.set_custom_dir(f.custom.clone());
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

/// Custom set 12 in the custom maps folder, made from osu! set 622136 (whose IDs its chart
/// keeps), with the manifest the website's archive brings when `id` is given.
fn custom_set(f: &Folders, id: Option<u32>) -> PathBuf {
    let path = put(&f.custom, "c12 Regression - Time Traveler", 1);
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace("BeatmapSetID:-1", "BeatmapSetID:622136");
    std::fs::write(&path, text).unwrap();
    if let Some(id) = id {
        std::fs::write(
            path.parent().unwrap().join(custom::SIDECAR),
            format!("{{\"id\": {id}}}\n"),
        )
        .unwrap();
    }
    path
}

/// Song select over `f`, offline (the tests' server isn't the official one, and the website
/// can't be asked), once the custom maps folder's manifests are read.
fn offline_client(f: &Folders) -> (State, Instant, usize) {
    let (mut s, mut now) = client(f);
    s.cfg.check_updates = true;
    refresh(&mut s, &mut now);
    s.select.set_cursor(0);
    assert!(
        wait_for(&mut s, &mut now, |s| s.custom_call.is_idle()
            && s.custom_asked.is_some()),
        "the manifests are read"
    );
    let i = s.select.selected().expect("the set is selected");
    (s, now, i)
}

#[test]
fn a_set_in_the_custom_folder_is_custom_and_epic_offline_by_its_manifest() {
    let f = Folders::new("offline");
    let path = custom_set(&f, Some(12));
    let (mut s, _now, i) = offline_client(&f);
    assert!(!online::is_official(&s.cfg.online_server));
    assert_eq!(s.library[i].path, path);
    assert!(s.library[i].is_own());
    let folder = path.parent().unwrap().to_path_buf();
    assert_eq!(s.select.custom_folder.get(&folder), Some(&12));
    assert_eq!(s.map_status(i).map(|x| x.0), Some("CUSTOM"));
    s.select.server_epic.custom.insert(12);
    assert!(s.select.is_epic(&s.library[i]), "custom set 12 is EPIC");
    // A mark song select had put on it by mistake doesn't make it nobody's.
    s.select.edited.insert(path.clone());
    assert_eq!(s.map_status(i).map(|x| x.0), Some("CUSTOM"));
}

#[test]
fn a_custom_folder_set_made_from_an_osu_set_is_never_checked_against_it() {
    let f = Folders::new("no-osu");
    custom_set(&f, Some(12));
    let (mut s, now, i) = offline_client(&f);
    assert_eq!(s.library[i].meta.set_id, Some(622_136), "the file keeps it");
    assert_eq!(s.library[i].set_id(), None, "but it isn't osu!'s");
    s.poll_set_check(now);
    s.poll_set_check(now + Duration::from_secs(2));
    assert!(s.set_check_due.is_none() && s.set_check.is_none());
    assert!(!s.select.updates.contains_key(&622_136));
    // osu! set 622136 having a newer version is nothing to it.
    s.select.updates.insert(622_136, ui::SetUpdate::Available);
    assert_eq!(s.select.update_of(&s.library[i]), None);
    // Its own: the website's.
    s.select.custom_updates.insert(12, ui::SetUpdate::Available);
    assert_eq!(
        s.select.update_of(&s.library[i]),
        Some(&ui::SetUpdate::Available)
    );
    // Loved or EPIC osu! set 622136 doesn't make it either.
    s.select.server_epic.sets.insert(622_136, Vec::new());
    assert!(!s.select.is_epic(&s.library[i]));
}

#[test]
fn a_set_in_the_custom_folder_never_sent_is_not_submitted_and_not_osus() {
    let f = Folders::new("local");
    custom_set(&f, None);
    let (mut s, now, i) = offline_client(&f);
    assert!(s.select.custom_folder.is_empty());
    assert_eq!(s.map_status(i).map(|x| x.0), Some("NOT SUBMITTED"));
    s.poll_set_check(now);
    s.poll_set_check(now + Duration::from_secs(2));
    assert!(s.set_check_due.is_none() && s.set_check.is_none());
}

#[test]
fn the_same_set_in_osus_songs_folder_is_still_checked_against_osu() {
    // Outside taiko.rs's own folder nothing changes: no marker, an osu! set as before.
    let f = Folders::new("legacy");
    let path = put(&f.songs, "622136 Regression - Time Traveler", 1);
    let (mut s, now, i) = offline_client(&f);
    assert_eq!(s.library[i].path, path);
    assert!(!s.library[i].is_own());
    assert_eq!(s.library[i].set_id(), Some(622_136));
    s.poll_set_check(now);
    assert_eq!(s.set_check_due.map(|d| d.0), Some(622_136));
}
