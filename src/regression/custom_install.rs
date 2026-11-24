//! Website custom sets in taiko.rs's own custom maps folder, installed and updated whole: the
//! new version takes the old one's place (no chart of an older version left listed beside it),
//! a mapper's own difficulty goes along, the folder keeps its name (favourites stay), a failed
//! download changes nothing, and the version replaced can be put back from the song menu.

use super::harness::*;
use super::maps;
use crate::*;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// A scratch `Custom` folder for a test.
struct Custom {
    root: PathBuf,
    dir: PathBuf,
}

impl Custom {
    fn new(tag: &str) -> Custom {
        let root = std::env::temp_dir().join(format!(
            "taiko-rs-regression-custom-install-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("Custom");
        std::fs::create_dir_all(&dir).unwrap();
        Custom { root, dir }
    }
}

impl Drop for Custom {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A regression chart as difficulty `version`.
fn chart(version: &str) -> String {
    let c = &maps::all()[1];
    c.text().replace(
        &format!("Version:{}", c.name),
        &format!("Version:{version}"),
    )
}

/// Website set `id` at upload `version`, as the download tab lists it.
fn website_set(id: u32, version: u32) -> download::SetInfo {
    download::SetInfo {
        id,
        artist: "Regression".into(),
        title: "Swap".into(),
        custom: Some(download::CustomSet {
            approved: true,
            version,
            ..download::CustomSet::default()
        }),
        ..download::SetInfo::default()
    }
}

/// Writes the website's archive of set `id` with these difficulties (and its marker) to `path`.
fn archive_at(path: &Path, id: u32, diffs: &[&str]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    for d in diffs {
        zip.start_file(format!("Regression - Swap [{d}].osu"), options)
            .unwrap();
        zip.write_all(chart(d).as_bytes()).unwrap();
    }
    zip.start_file("audio.mp3", options).unwrap();
    zip.write_all(b"not really audio").unwrap();
    zip.start_file(custom::SIDECAR, options).unwrap();
    zip.write_all(format!("{{\"id\": {id}}}\n").as_bytes())
        .unwrap();
    zip.finish().unwrap();
}

/// What the download tab's preview of `set` fetched already: the download takes it from there.
fn previewed(set: &download::SetInfo, diffs: &[&str]) {
    archive_at(&custom::preview_archive(set), set.id, diffs);
}

fn client(c: &Custom) -> (State, Instant) {
    let mut s = state_with(config(), Vec::new());
    s.set_custom_dir(c.dir.clone());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    (s, now)
}

fn finish(s: &mut State, now: &mut Instant) {
    assert!(
        wait_for(s, now, |s| s.downloads.is_empty()),
        "the download ends"
    );
}

/// The difficulties listed from `folder`, by name.
fn listed(s: &State, folder: &Path) -> Vec<String> {
    let mut v: Vec<String> = s
        .library
        .iter()
        .filter(|e| e.path.parent() == Some(folder) && !s.select.gone.contains(&e.path))
        .map(|e| e.meta.version.clone())
        .collect();
    v.sort();
    v
}

fn entry(s: &State, folder: &Path, version: &str) -> usize {
    s.library
        .iter()
        .position(|e| {
            e.path.parent() == Some(folder)
                && e.meta.version == version
                && !s.select.gone.contains(&e.path)
        })
        .unwrap_or_else(|| panic!("[{version}] is listed"))
}

fn toast(s: &State) -> String {
    s.toast.as_ref().map(|t| t.text.clone()).unwrap_or_default()
}

#[test]
fn a_website_set_is_downloaded_into_the_custom_folder_and_updated_whole() {
    let c = Custom::new("swap");
    let (mut s, mut now) = client(&c);
    let v1 = website_set(9101, 1);
    previewed(&v1, &["Oni"]);
    s.start_download(v1);
    finish(&mut s, &mut now);
    let folder = c.dir.join("c9101 Regression - Swap");
    assert_eq!(listed(&s, &folder), ["Oni"], "{}", toast(&s));
    assert!(!s.songs_dir.join("c9101 Regression - Swap").exists());
    let i = entry(&s, &folder, "Oni");
    assert_eq!(s.select.custom_folder.get(&folder), Some(&9101));
    assert_eq!(s.map_status(i).map(|x| x.0), Some("CUSTOM APPROVED"));
    s.select.server_epic.custom.insert(9101);
    assert!(s.select.is_epic(&s.library[i]));
    assert_eq!(custom::Manifest::read(&folder).unwrap().version, Some(1));
    // Liked, and a difficulty of the mapper's own put into the folder.
    s.select.toggle_like(&s.library[i]);
    std::fs::write(folder.join("Regression - Swap [Mine].osu"), chart("Mine")).unwrap();
    s.refresh_library(false);
    assert!(wait_for(&mut s, &mut now, |s| s.scan.is_none()));
    assert_eq!(listed(&s, &folder), ["Mine", "Oni"]);

    // The website's next version renamed [Oni] to [Inner Oni].
    let v2 = website_set(9101, 2);
    previewed(&v2, &["Inner Oni"]);
    s.select
        .custom_updates
        .insert(9101, ui::SetUpdate::Available);
    s.start_download(v2);
    assert!(s.downloads.last().is_some_and(|j| j.update));
    finish(&mut s, &mut now);
    assert_eq!(toast(&s), "updated Regression - Swap");
    // The old version's chart is gone, not listed beside the new one; the mapper's stays.
    assert_eq!(listed(&s, &folder), ["Inner Oni", "Mine"]);
    assert!(!folder.join("Regression - Swap [Oni].osu").exists());
    assert_eq!(
        s.select.custom_updates.get(&9101),
        Some(&ui::SetUpdate::UpToDate)
    );
    let i = entry(&s, &folder, "Inner Oni");
    assert_eq!(s.map_status(i).map(|x| x.0), Some("CUSTOM APPROVED"));
    assert!(s.select.is_epic(&s.library[i]));
    // Same folder, same name: still liked.
    assert!(s.select.is_liked(&s.library[i]));
    assert_eq!(custom::Manifest::read(&folder).unwrap().version, Some(2));

    // The song menu can put the version before back, and that can be undone the same way.
    let row = s
        .select
        .rows()
        .iter()
        .position(|&v| v == i)
        .expect("listed");
    let previous = s.menu_previous(row);
    assert_eq!(previous, Some(1));
    s.select.menu = Some(ui::SongMenu {
        row,
        header: false,
        skin: false,
        previous,
        x: 0.0,
        y: 0.0,
    });
    let items = s.select.menu_items(&s.library);
    assert!(
        items
            .iter()
            .any(|it| it.action == ui::MenuAction::PreviousVersion
                && it.label == "Previous version (v1)"),
        "{items:?}"
    );
    s.select.menu = None;
    s.previous_version(i);
    assert_eq!(toast(&s), "big-notes: back to the previous version");
    assert_eq!(listed(&s, &folder), ["Mine", "Oni"]);
    assert_eq!(custom::Manifest::read(&folder).unwrap().version, Some(1));
    let i = entry(&s, &folder, "Oni");
    assert_eq!(s.select.custom_folder.get(&folder), Some(&9101));
    assert_eq!(
        s.menu_previous(s.select.rows().iter().position(|&v| v == i).unwrap()),
        Some(2)
    );
}

#[test]
fn a_failed_update_leaves_the_installed_version_as_it_was() {
    let c = Custom::new("failed");
    let (mut s, mut now) = client(&c);
    let v1 = website_set(9102, 1);
    previewed(&v1, &["Oni"]);
    s.start_download(v1);
    finish(&mut s, &mut now);
    let folder = c.dir.join("c9102 Regression - Swap");
    let before: Vec<_> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    // Version 3 isn't previewed, and tests can't reach the website.
    s.select
        .custom_updates
        .insert(9102, ui::SetUpdate::Available);
    s.start_download(website_set(9102, 3));
    finish(&mut s, &mut now);
    assert!(toast(&s).starts_with("download failed"), "{}", toast(&s));
    let after: Vec<_> = std::fs::read_dir(&folder)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(before, after);
    assert_eq!(listed(&s, &folder), ["Oni"]);
    assert_eq!(custom::Manifest::read(&folder).unwrap().version, Some(1));
    assert_eq!(
        s.select.custom_updates.get(&9102),
        Some(&ui::SetUpdate::Unknown)
    );
    assert!(custom::store::previous_version(&c.dir, &folder).is_none());
}

#[test]
fn the_websites_archive_opened_in_the_game_goes_into_the_custom_folder() {
    let c = Custom::new("opened");
    let (mut s, mut now) = client(&c);
    let archive = c.root.join("downloads/c9103 Regression - Swap.rsz");
    archive_at(&archive, 9103, &["Oni"]);
    s.import_osz(archive.clone(), "c9103 Regression - Swap", false);
    finish(&mut s, &mut now);
    let folder = c.dir.join("c9103 Regression - Swap");
    assert_eq!(listed(&s, &folder), ["Oni"], "{}", toast(&s));
    assert_eq!(s.select.custom_folder.get(&folder), Some(&9103));
    assert!(s.library[entry(&s, &folder, "Oni")].is_own());
    assert!(!archive.exists(), "taken in, like an imported .osz");
    let m = custom::Manifest::read(&folder).unwrap();
    assert_eq!(m.id, Some(9103));
    assert_eq!(m.origin, Some(custom::Origin::Site));
}

#[test]
fn lobby_lookup_never_overwrites_a_custom_set_in_osus_songs_folder() {
    let c = Custom::new("watch-songs");
    let osu_songs = c.root.join("osu-Songs");
    let folder = osu_songs.join("c9104 Regression - Swap");
    std::fs::create_dir_all(&folder).unwrap();
    let local_chart = folder.join("Regression - Swap [Oni].osu");
    let edited = format!("{}\n// unsent mapper work\n", chart("Oni"));
    std::fs::write(&local_chart, &edited).unwrap();
    std::fs::write(folder.join(custom::SIDECAR), "{\"id\": 9104}\n").unwrap();

    let (mut s, mut now) = client(&c);
    s.songs_dir = osu_songs;
    let meta = crate::beatmap::read_meta_file(&local_chart).unwrap();
    s.library.push(Entry::new(local_chart.clone(), meta));
    s.select.custom_folder.insert(folder.clone(), 9104);

    // Spectating a friend playing the published v1 resolves to custom set 9104.
    let v1 = website_set(9104, 1);
    previewed(&v1, &["Oni"]);
    s.lobby_lookup = Job::spawn(move || Ok(v1));
    assert!(wait_for(&mut s, &mut now, |s| !s.lobby_lookup.is_busy()));

    assert!(
        s.downloads.is_empty(),
        "must not start an update over osu!'s Songs folder"
    );
    assert_eq!(
        std::fs::read_to_string(&local_chart).unwrap(),
        edited,
        "local edits in osu!'s Songs folder stay untouched"
    );
    assert!(
        toast(&s).contains("keeping your local copy"),
        "{}",
        toast(&s)
    );
}

#[test]
fn lobby_lookup_protects_locally_edited_custom_sets_but_updates_clean_site_installs() {
    let c = Custom::new("watch-custom");
    let (mut s, mut now) = client(&c);

    // 1. A website install in Custom whose chart the mapper edited locally must not be overwritten.
    let v1 = website_set(9105, 1);
    previewed(&v1, &["Oni"]);
    s.start_download(v1);
    finish(&mut s, &mut now);
    let folder = c.dir.join("c9105 Regression - Swap");
    let chart_path = folder.join("Regression - Swap [Oni].osu");
    let edited = format!(
        "{}\n// local edits\n",
        std::fs::read_to_string(&chart_path).unwrap()
    );
    std::fs::write(&chart_path, &edited).unwrap();

    let v2 = website_set(9105, 2);
    previewed(&v2, &["Inner Oni"]);
    s.lobby_lookup = Job::spawn(move || Ok(v2));
    assert!(wait_for(&mut s, &mut now, |s| !s.lobby_lookup.is_busy()));
    assert!(s.downloads.is_empty(), "edited copy is not overwritten");
    assert_eq!(std::fs::read_to_string(&chart_path).unwrap(), edited);
    assert!(
        toast(&s).contains("keeping your local copy"),
        "{}",
        toast(&s)
    );

    // 2. An unmodified website install in Custom still updates cleanly when a friend plays v2.
    let clean_v1 = website_set(9106, 1);
    previewed(&clean_v1, &["Oni"]);
    s.start_download(clean_v1);
    finish(&mut s, &mut now);
    let clean_folder = c.dir.join("c9106 Regression - Swap");
    assert_eq!(listed(&s, &clean_folder), ["Oni"]);

    let clean_v2 = website_set(9106, 2);
    previewed(&clean_v2, &["Inner Oni"]);
    s.lobby_lookup = Job::spawn(move || Ok(clean_v2));
    assert!(wait_for(&mut s, &mut now, |s| !s.lobby_lookup.is_busy()));
    finish(&mut s, &mut now);
    assert_eq!(listed(&s, &clean_folder), ["Inner Oni"]);
    assert_eq!(
        custom::store::previous_version(&c.dir, &clean_folder).map(|(_, m)| m.version),
        Some(Some(1))
    );
}
