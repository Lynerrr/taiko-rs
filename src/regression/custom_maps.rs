//! A taiko.rs custom set kept in an older version: one made from an osu! set keeps that set's
//! IDs, so a copy song select doesn't know as the custom set (no marker file in its folder) was
//! checked against osu! and called NOT SUBMITTED, wore no EPIC mark and gave no pp. Opening the
//! website's archive over it makes it the custom set, read anew; a copy song select knows is
//! offered the website's newer version, never osu!'s.

use super::harness::{self, *};
use super::maps;
use crate::*;
use std::io::Write as _;

/// The osu! set the custom set was made from, and the custom set's number on the website.
const OSU_SET: u32 = 622_136;
const CUSTOM_SET: u32 = 12;

/// Song select over the regression charts, the selected one standing in for an old copy of
/// custom set 12: it carries osu! set 622136's ID and has no marker file.
fn client(tag: &str) -> (State, Songs, Instant, usize) {
    let songs = Songs::new(tag, &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    s.songs_dir = songs.dir.clone();
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    let i = s.select.selected().expect("a map is selected");
    s.library[i].meta.set_id = Some(OSU_SET);
    (s, songs, now, i)
}

/// The website's archive of the set: `map` (the newer version of the difficulty, under the
/// same file name) and the marker naming custom set 12.
fn website_archive(dir: &std::path::Path, file: &str, map: &[u8]) -> std::path::PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(format!("c{CUSTOM_SET} Regression - Time Traveler.rsz"));
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file(file, options).unwrap();
    zip.write_all(map).unwrap();
    zip.start_file(custom::SIDECAR, options).unwrap();
    zip.write_all(format!("{{\"id\": {CUSTOM_SET}}}\n").as_bytes())
        .unwrap();
    zip.finish().unwrap();
    path
}

#[test]
fn the_websites_archive_opened_over_an_old_copy_makes_it_the_custom_set() {
    let (mut s, _songs, mut now, i) = client("custom-old-copy");
    let path = s.library[i].path.clone();
    let folder = path.parent().unwrap().to_path_buf();
    // As song select had it: checked against osu! set 622136, whose files it isn't.
    s.select.edited.insert(path.clone());
    s.select.server_epic.custom.insert(CUSTOM_SET);
    assert_eq!(s.map_status(i).map(|x| x.0), Some("NOT SUBMITTED"));
    assert!(!s.select.is_epic(&s.library[i]));
    let old_hash = s.library[i].hash();

    // The website's newer version: the same difficulty under the same file name.
    let mut newer = std::fs::read(&path).unwrap();
    newer.extend_from_slice(b"\n// the website's newer version\n");
    let file = path.file_name().unwrap().to_string_lossy().into_owned();
    let archive = website_archive(
        &std::env::temp_dir().join(format!(
            "taiko-rs-regression-custom-archive-{}",
            std::process::id()
        )),
        &file,
        &newer,
    );
    let name = archive.file_stem().unwrap().to_string_lossy().into_owned();
    s.import_osz(archive, &name, true);
    assert!(
        wait_for(&mut s, &mut now, |s| s.downloads.is_empty()),
        "the archive is unpacked"
    );

    assert_eq!(
        std::fs::read(&path).unwrap(),
        newer,
        "into the old copy's folder"
    );
    // Read anew: plays go to the new version's leaderboard (and its pp), not the old one's.
    assert_eq!(s.library[i].hash(), crate::net::map_hash(&newer));
    assert_ne!(s.library[i].hash(), old_hash);
    // The marker came with it: the custom set, not the osu! set it was made from.
    assert_eq!(s.select.custom_folder.get(&folder), Some(&CUSTOM_SET));
    assert!(!s.select.edited.contains(&path));
    assert_eq!(s.map_status(i).map(|x| x.0), Some("CUSTOM"));
    assert!(
        s.select.is_epic(&s.library[i]),
        "custom set 12 is marked EPIC"
    );
    let toast = s.toast.as_ref().map(|t| t.text.clone()).unwrap_or_default();
    assert!(
        toast.starts_with("updated"),
        "said what happened, not that it's already there: {toast}"
    );
}

#[test]
fn an_older_custom_copy_is_offered_the_websites_version_not_osus() {
    let (mut s, _songs, mut now, i) = client("custom-update");
    let folder = s.library[i].path.parent().unwrap().to_path_buf();
    s.select.custom_folder.insert(folder, CUSTOM_SET);
    // osu! set 622136 is up to date; the website has a newer version of custom set 12.
    s.select.updates.insert(OSU_SET, ui::SetUpdate::UpToDate);
    s.select
        .custom_updates
        .insert(CUSTOM_SET, ui::SetUpdate::Available);
    assert_eq!(
        s.select.update_of(&s.library[i]),
        Some(&ui::SetUpdate::Available)
    );

    // Downloading it from the website again updates the copy instead of saying it's there.
    s.toast = None;
    s.start_download(download::SetInfo {
        id: CUSTOM_SET,
        artist: "Regression".into(),
        title: "Time Traveler".into(),
        custom: Some(download::CustomSet::default()),
        ..download::SetInfo::default()
    });
    let job = s.downloads.last().expect("an update started");
    assert!(job.update);
    assert!(job.set.custom.is_some(), "from the website");
    assert_eq!(job.set.id, CUSTOM_SET);
    assert_eq!(s.downloads.len(), 1, "osu! isn't asked for its set");
    assert_eq!(
        s.select.custom_updates.get(&CUSTOM_SET),
        Some(&ui::SetUpdate::Updating)
    );
    let toast = s.toast.as_ref().map(|t| t.text.clone()).unwrap_or_default();
    assert!(!toast.contains("already"), "{toast}");

    // Tests can't reach the website: the update fails, and the custom set's state says so,
    // not osu! set 12's.
    assert!(
        wait_for(&mut s, &mut now, |s| s.downloads.is_empty()),
        "the update ends"
    );
    assert_eq!(
        s.select.custom_updates.get(&CUSTOM_SET),
        Some(&ui::SetUpdate::Unknown)
    );
    assert!(!s.select.updates.contains_key(&CUSTOM_SET));
    assert_eq!(
        s.select.updates.get(&OSU_SET),
        Some(&ui::SetUpdate::UpToDate)
    );
}

#[test]
fn the_websites_archive_puts_aside_the_old_versions_renamed_difficulty() {
    let (mut s, _songs, mut now, i) = client("custom-renamed-diff");
    let old = s.library[i].path.clone();
    let folder = old.parent().unwrap().to_path_buf();
    s.select.server_epic.custom.insert(CUSTOM_SET);

    // The website's version renamed the difficulty, and its file is named otherwise.
    let text = std::fs::read_to_string(&old).unwrap();
    let version = s.library[i].meta.version.clone();
    let renamed = text.replace(&format!("Version:{version}"), "Version:Go backwards (TKO)");
    assert_ne!(renamed, text, "the difficulty is renamed");
    let file = "Regression - Time Traveler [Go backwards].osu";
    let archive = website_archive(
        &std::env::temp_dir().join(format!(
            "taiko-rs-regression-custom-renamed-{}",
            std::process::id()
        )),
        file,
        renamed.as_bytes(),
    );
    let name = archive.file_stem().unwrap().to_string_lossy().into_owned();
    s.import_osz(archive, &name, true);
    assert!(
        wait_for(&mut s, &mut now, |s| s.downloads.is_empty()),
        "the archive is unpacked"
    );

    // Only the website's difficulty is in the set's folder and in the list, with the EPIC mark.
    assert!(!old.exists(), "the old version's chart is put aside");
    let mut aside = old.clone().into_os_string();
    aside.push(format!(".{}", download::SET_ASIDE));
    assert_eq!(
        std::fs::read_to_string(std::path::PathBuf::from(aside)).unwrap(),
        text,
        "not deleted: renaming it back brings it back"
    );
    let listed: Vec<String> = s
        .library
        .iter()
        .filter(|e| e.path.starts_with(&folder) && !s.select.gone.contains(&e.path))
        .map(|e| e.meta.version.clone())
        .collect();
    assert_eq!(listed, ["Go backwards (TKO)"]);
    let new = s
        .library
        .iter()
        .position(|e| e.path == folder.join(file))
        .expect("the new difficulty is listed");
    assert_eq!(s.select.custom_folder.get(&folder), Some(&CUSTOM_SET));
    assert!(s.select.is_epic(&s.library[new]));
}

#[test]
fn opening_the_websites_archive_again_clears_an_old_version_left_behind() {
    let (mut s, _songs, mut now, i) = client("custom-left-behind");
    let current = s.library[i].path.clone();
    let folder = current.parent().unwrap().to_path_buf();
    s.select.custom_folder.insert(folder.clone(), CUSTOM_SET);
    s.select.server_epic.custom.insert(CUSTOM_SET);
    // An earlier import left the old version's chart next to the current one.
    let old = folder.join("Regression - Time Traveler [Inner Oni].osu");
    let text = std::fs::read_to_string(&current).unwrap();
    std::fs::write(&old, format!("{text}\n// the old version\n")).unwrap();
    let meta = crate::beatmap::read_meta_file(&old).unwrap();
    s.library.push(Entry::new(old.clone(), meta));
    s.select.refilter(&s.library);
    assert!(
        s.select.is_epic(s.library.last().unwrap()),
        "both wore EPIC"
    );

    let archive = website_archive(
        &std::env::temp_dir().join(format!(
            "taiko-rs-regression-custom-left-behind-{}",
            std::process::id()
        )),
        &current.file_name().unwrap().to_string_lossy(),
        text.as_bytes(),
    );
    let name = archive.file_stem().unwrap().to_string_lossy().into_owned();
    s.toast = None;
    s.import_osz(archive, &name, true);
    assert!(!s.downloads.is_empty(), "not \"already in your songs\"");
    assert!(
        wait_for(&mut s, &mut now, |s| s.downloads.is_empty()),
        "the archive is unpacked"
    );
    assert!(!old.exists(), "the old version's chart is put aside");
    assert!(s.select.gone.contains(&old), "and leaves song select");
    assert!(current.exists());
    assert!(!s.select.gone.contains(&current));
}
