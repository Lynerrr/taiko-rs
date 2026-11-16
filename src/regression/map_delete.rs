//! Deleting maps in song select: Delete asks about the selected difficulty, Shift+Delete about
//! its whole set, and the right-click menu offers both. Cancel is picked at first; Delete sends
//! the files to the bin (in tests a temporary folder, never the player's), takes the maps off the
//! list and out of the library cache at once, and a custom set notes the difficulty deleted.

use super::harness::{self, *};
use super::maps;
use super::snapshot::assert_snapshot;
use crate::*;

/// A Songs folder with one set of three difficulties (and one more set beside it), removed when
/// dropped.
struct Set {
    songs: Songs,
    folder: PathBuf,
    paths: Vec<PathBuf>,
}

fn set(tag: &str) -> Set {
    let charts = maps::all();
    let songs = Songs::new(tag, &charts[3..4]);
    let folder = songs.dir.join("Regression - Set");
    std::fs::create_dir_all(&folder).unwrap();
    let paths = charts[..3]
        .iter()
        .map(|c| {
            let p = folder.join(format!("{}.osu", c.name));
            std::fs::write(&p, c.text()).unwrap();
            p
        })
        .collect();
    Set {
        songs,
        folder,
        paths,
    }
}

fn client(set: &Set) -> (State, Instant) {
    let mut library = set.songs.entries();
    for p in &set.paths {
        library.push(Entry::new(
            p.clone(),
            beatmap::read_meta_file(p).expect("a chart's metadata"),
        ));
    }
    let mut s = state_with(harness::config(), library);
    s.songs_dir = set.songs.dir.clone();
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    // Background stars and the like can still reorder the list: done before anything is picked.
    settle(&mut s, &mut now);
    (s, now)
}

/// Puts the cursor on the difficulty at `path`, looked for again if a frame moved the rows.
fn select(s: &mut State, now: Instant, path: &Path) {
    let selected = |s: &State| s.select.selected().map(|i| s.library[i].path.clone());
    for _ in 0..5 {
        let at = s
            .select
            .rows()
            .iter()
            .position(|&i| s.library[i].path == path)
            .expect("the difficulty is listed");
        s.select.set_cursor(at);
        frame(s, now);
        if selected(s).as_deref() == Some(path) {
            return;
        }
    }
    assert_eq!(
        selected(s).as_deref(),
        Some(path),
        "the difficulty is selected"
    );
}

fn listed(s: &State, path: &Path) -> bool {
    s.select.rows().iter().any(|&i| s.library[i].path == path)
}

/// Where `original` went in the tests' bin, by the note left beside it.
fn in_bin(s: &State, original: &Path) -> Option<PathBuf> {
    let map_trash::Bin::Folder(bin) = &s.map_bin else {
        panic!("tests' maps go to a folder of their own");
    };
    std::fs::read_dir(bin).ok()?.flatten().find_map(|slot| {
        let note = std::fs::read_to_string(slot.path().join(map_trash::FROM_NOTE)).ok()?;
        (note.trim_end() == original.to_string_lossy())
            .then(|| slot.path().join(original.file_name().unwrap()))
    })
}

fn toast(s: &mut State) -> String {
    s.toast.take().map(|t| t.text).unwrap_or_default()
}

/// The confirmation as it draws itself. Over song select only its buttons and the top bar take
/// clicks. The golden is the confirmation alone: the map panel behind it fills in its stats in
/// the background, sooner or later, and made the whole screen's golden flaky.
fn asked(s: &mut State, now: Instant) -> String {
    // Two frames: a frame hands its canvas the hit areas of the one before it, and keeps its
    // own in `State::hits`.
    frame(s, now);
    frame(s, now);
    let taking: Vec<String> = s.hits.iter().map(|h| format!("{:?}", h.hit)).collect();
    assert_eq!(
        taking,
        ["Key(Escape)", "DeleteMaps", "Changelog", "Account"],
        "nothing under the confirmation takes a click"
    );
    let ask = s.select.deleting.clone().expect("asked");
    let mut c = crate::hit::Canvas {
        audit: true,
        width: WIDTH,
        ..Default::default()
    };
    crate::screens::select::delete::draw_delete_ask(&mut c, &ask);
    screen(&c, &[])
}

#[test]
fn delete_asks_first_and_cancel_keeps_the_map() {
    let set = set("delete-cancel");
    let (mut s, now) = client(&set);
    let path = set.paths[1].clone();
    select(&mut s, now, &path);

    press(&mut s, KeyCode::Delete, now);
    let ask = s.select.deleting.clone().expect("Delete asks first");
    assert!(
        !ask.set && !ask.whole,
        "one difficulty of three, not its folder"
    );
    assert!(!ask.on_delete, "Cancel is picked at first");
    assert_snapshot("map-delete-ask-difficulty", &asked(&mut s, now));

    // Enter on Cancel: nothing goes.
    press(&mut s, KeyCode::Enter, now);
    assert!(s.select.deleting.is_none());
    // Esc too, even with Delete picked.
    press(&mut s, KeyCode::Delete, now);
    press(&mut s, KeyCode::ArrowRight, now);
    press(&mut s, KeyCode::Escape, now);
    assert!(s.select.deleting.is_none());
    frame(&mut s, now);

    assert!(path.is_file(), "the file stays");
    assert!(listed(&s, &path), "and so does its row");
    assert!(in_bin(&s, &path).is_none());
}

#[test]
fn a_difficulty_goes_to_the_bin_and_the_rest_of_its_set_stays() {
    let set = set("delete-difficulty");
    // The library cache as the last launch left it, every map in it.
    let cache = set.songs.dir.with_extension("cache.json");
    let found = std::sync::atomic::AtomicUsize::new(0);
    library::scan_cached(
        std::slice::from_ref(&set.songs.dir),
        &found,
        &cache,
        &mut |_| {},
    );
    library::set_test_cache(Some(cache.clone()));

    let (mut s, now) = client(&set);
    let path = set.paths[0].clone();
    select(&mut s, now, &path);
    press(&mut s, KeyCode::Delete, now);
    press(&mut s, KeyCode::ArrowRight, now);
    press(&mut s, KeyCode::Enter, now);
    frame(&mut s, now);
    library::set_test_cache(None);

    assert!(!path.exists(), "the file left the Songs folder");
    let binned = in_bin(&s, &path).expect("into the bin, with a note of where it was");
    assert!(binned.is_file());
    for other in &set.paths[1..] {
        assert!(
            other.is_file() && listed(&s, other),
            "the set's other difficulties stay"
        );
    }
    assert!(set.folder.is_dir(), "and so does its folder");
    assert!(!listed(&s, &path), "the row goes at once, no rescan");
    assert!(toast(&mut s).starts_with("deleted Regression - "));

    // The next launch doesn't list it while the Songs folder is looked through.
    let mut early = Vec::new();
    library::scan_cached(
        std::slice::from_ref(&set.songs.dir),
        &found,
        &cache,
        &mut |e| early = e,
    );
    assert!(!early.is_empty(), "the cache is read first");
    assert!(
        early.iter().all(|e| e.path != path),
        "without the deleted map"
    );
    assert!(early.iter().any(|e| e.path == set.paths[1]));
    let _ = std::fs::remove_file(&cache);
}

#[test]
fn shift_delete_takes_the_whole_set_folder() {
    let set = set("delete-set");
    let (mut s, now) = client(&set);
    select(&mut s, now, &set.paths[2]);
    s.modifiers_shift = true;
    press(&mut s, KeyCode::Delete, now);
    s.modifiers_shift = false;
    let ask = s.select.deleting.clone().expect("Shift+Delete asks first");
    assert!(ask.set && ask.whole, "the whole set, folder and all");
    assert_eq!(ask.maps.len(), 3);
    assert_snapshot("map-delete-ask-set", &asked(&mut s, now));

    press(&mut s, KeyCode::Tab, now);
    press(&mut s, KeyCode::Space, now);
    frame(&mut s, now);
    assert!(!set.folder.exists(), "the folder left the Songs folder");
    let binned = in_bin(&s, &set.folder).expect("the folder went to the bin whole");
    for p in &set.paths {
        assert!(binned.join(p.file_name().unwrap()).is_file());
        assert!(!listed(&s, p));
    }
    assert!(listed(&s, &set.songs.paths[0]), "the other set stays");
}

#[test]
fn a_sets_last_difficulty_takes_its_folder_with_it() {
    let set = set("delete-last");
    let (mut s, now) = client(&set);
    let only = set.songs.paths[0].clone();
    select(&mut s, now, &only);
    press(&mut s, KeyCode::Delete, now);
    let ask = s.select.deleting.clone().expect("asked");
    assert!(!ask.set && ask.whole, "no chart is left to keep the folder");
    press(&mut s, KeyCode::ArrowRight, now);
    press(&mut s, KeyCode::Enter, now);
    assert!(!only.parent().unwrap().exists());
    assert!(in_bin(&s, only.parent().unwrap()).is_some());
}

#[test]
fn the_song_menu_offers_deleting_a_difficulty_and_its_set() {
    let set = set("delete-menu");
    let (mut s, now) = client(&set);
    let path = set.paths[1].clone();
    select(&mut s, now, &path);
    let open = |s: &mut State| {
        s.select.menu = Some(ui::SongMenu {
            row: s.select.cursor(),
            header: false,
            skin: false,
            previous: None,
            x: 100.0,
            y: 100.0,
        });
    };
    let item = |s: &State, action: ui::MenuAction| {
        s.select
            .menu_items(&s.library)
            .iter()
            .position(|(_, a)| *a == action)
            .expect("the song menu offers it")
    };

    open(&mut s);
    let k = item(&s, ui::MenuAction::DeleteSet);
    s.click(crate::hit::Hit::SongMenu(k), None);
    assert!(
        s.select.deleting.as_ref().is_some_and(|a| a.set),
        "the set, asked first"
    );
    press(&mut s, KeyCode::Escape, now);

    open(&mut s);
    let k = item(&s, ui::MenuAction::DeleteDifficulty);
    s.click(crate::hit::Hit::SongMenu(k), None);
    assert!(s.select.menu.is_none(), "the menu closes");
    let ask = s
        .select
        .deleting
        .clone()
        .expect("the difficulty, asked first");
    assert!(!ask.set);
    assert_eq!(s.library[ask.maps[0]].path, path);
    // The confirmation's red Delete, clicked.
    s.click(crate::hit::Hit::DeleteMaps, None);
    frame(&mut s, now);
    assert!(!path.exists() && in_bin(&s, &path).is_some());
}

#[test]
fn a_custom_sets_deleted_difficulty_is_noted_in_its_marker() {
    let set = set("delete-custom");
    std::fs::write(set.folder.join(custom::SIDECAR), "{\"id\": 12}\n").unwrap();
    let (mut s, now) = client(&set);
    let path = set.paths[0].clone();
    select(&mut s, now, &path);
    let hash = s.library[s.select.selected().unwrap()].hash();
    press(&mut s, KeyCode::Delete, now);
    press(&mut s, KeyCode::ArrowRight, now);
    press(&mut s, KeyCode::Enter, now);
    assert!(!path.exists());
    assert_eq!(
        custom::deleted(&set.folder),
        vec![hash],
        "the website still having it doesn't bring it back"
    );
}

#[test]
fn a_deleted_map_dropped_in_again_comes_back_at_once() {
    use std::io::Write as _;
    let set = set("delete-reimport");
    let (mut s, mut now) = client(&set);
    let path = set.paths[0].clone();
    let bytes = std::fs::read(&path).unwrap();
    select(&mut s, now, &path);
    let hash = s.library[s.select.selected().unwrap()].hash();
    press(&mut s, KeyCode::Delete, now);
    press(&mut s, KeyCode::ArrowRight, now);
    press(&mut s, KeyCode::Enter, now);
    assert!(!path.exists() && !listed(&s, &path));
    toast(&mut s);

    // The same map as an .osz, dropped on the window in the same session.
    let dir = std::env::temp_dir().join(format!(
        "taiko-rs-regression-reimport-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let archive = dir.join("Regression - again.osz");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
    zip.start_file(
        path.file_name().unwrap().to_string_lossy(),
        zip::write::SimpleFileOptions::default(),
    )
    .unwrap();
    zip.write_all(&bytes).unwrap();
    zip.finish().unwrap();
    s.import_osz(archive, "Regression - again", true);
    assert!(
        !toast(&mut s).contains("already in your songs"),
        "the deleted copy doesn't count as one in the list"
    );
    assert!(!s.downloads.is_empty(), "it's unpacked");
    assert!(
        wait_for(&mut s, &mut now, |s| s.downloads.is_empty()),
        "the archive is unpacked"
    );
    frame(&mut s, now);
    let back = s
        .select
        .rows()
        .iter()
        .map(|&i| &s.library[i])
        .find(|e| e.hash() == hash)
        .map(|e| e.path.clone());
    assert!(
        back.as_ref().is_some_and(|p| p.is_file()),
        "listed again without leaving song select"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
