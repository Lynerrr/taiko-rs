//! What taiko.rs makes or is given goes into its own custom maps folder: a map made in the
//! editor, a mapping session's set and a lobby host's custom set. osu!'s maps (a plain `.osz`, a
//! host's osu! set) still go into osu!'s Songs folder.

use super::harness::*;
use super::maps;
use crate::*;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// osu!'s Songs folder and taiko.rs's own, side by side.
struct Folders {
    root: PathBuf,
    songs: PathBuf,
    custom: PathBuf,
}

impl Folders {
    fn new(tag: &str) -> Folders {
        let root = std::env::temp_dir().join(format!(
            "taiko-rs-regression-custom-routing-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let (songs, custom) = (root.join("osu!/Songs"), root.join("taiko-rs/Custom"));
        std::fs::create_dir_all(&songs).unwrap();
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

fn client(f: &Folders) -> (State, Instant) {
    let mut s = state_with(config(), Vec::new());
    s.songs_dir = f.songs.clone();
    s.song_dirs = vec![f.songs.clone()];
    s.set_custom_dir(f.custom.clone());
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

fn toast(s: &State) -> String {
    s.toast.as_ref().map(|t| t.text.clone()).unwrap_or_default()
}

/// The set folders in `dir`, by name.
fn folders(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// An archive at `path` of regression chart `n` (as `chart`), its song and, when `marker`, the
/// website's marker. Returns the chart's hash.
fn archive_at(path: &Path, chart: &str, n: usize, marker: bool) -> u64 {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file(chart, options).unwrap();
    zip.write_all(maps::all()[n].text().as_bytes()).unwrap();
    zip.start_file("audio.mp3", options).unwrap();
    zip.write_all(b"not really audio").unwrap();
    if marker {
        zip.start_file(custom::SIDECAR, options).unwrap();
        zip.write_all(b"{\"id\": 9201}\n").unwrap();
    }
    zip.finish().unwrap();
    download::archive_maps(path).unwrap()[0].1
}

#[test]
fn a_map_made_in_the_editor_goes_into_the_custom_folder_and_opens() {
    let f = Folders::new("editor");
    let (mut s, _now) = client(&f);
    std::fs::write(f.root.join("song.mp3"), b"not really audio").unwrap();
    let plan = editor::new_map::NewMap {
        audio: f.root.join("song.mp3"),
        background: None,
        artist: "Regression".into(),
        title: "Made Here".into(),
        artist_romanised: String::new(),
        title_romanised: String::new(),
        creator: "me".into(),
        version: "Oni".into(),
        source: String::new(),
        tags: String::new(),
        bpm: 120.0,
        offset_ms: 0.0,
        per_bar: 4,
        length_ms: 60_000.0,
        tko: false,
    };
    s.home_out(editor::home::Out::Create(Box::new(plan)));
    let folder = f.custom.join("Regression - Made Here");
    let view = &s
        .scene
        .screen_as::<MapEditor>()
        .unwrap_or_else(|| panic!("the editor opens: {}", toast(&s)))
        .view;
    assert_eq!(view.ed.path.parent(), Some(folder.as_path()));
    assert!(
        folders(&f.songs).is_empty(),
        "nothing in osu!'s Songs folder"
    );
    let m = custom::Manifest::read(&folder).unwrap();
    assert_eq!((m.id, m.origin), (None, Some(custom::Origin::Local)));
    let i = s
        .library
        .iter()
        .position(|e| e.path == view.ed.path)
        .unwrap();
    assert!(s.library[i].is_own());
    assert_eq!(s.map_status(i).map(|x| x.0), Some("NOT SUBMITTED"));
}

#[test]
fn a_plain_osz_still_goes_into_osus_songs_folder_and_a_tko_set_into_the_custom_folder() {
    let f = Folders::new("import");
    let (mut s, mut now) = client(&f);
    let osz = f.root.join("downloads/Regression - Plain.osz");
    archive_at(&osz, "Regression - Plain [Oni].osu", 1, false);
    s.import_osz(osz, "Regression - Plain", false);
    finish(&mut s, &mut now);
    assert_eq!(folders(&f.songs), ["Regression - Plain"], "{}", toast(&s));
    assert!(folders(&f.custom).is_empty());
    assert!(
        !f.songs
            .join("Regression - Plain")
            .join(custom::SIDECAR)
            .exists()
    );

    let tko = f.root.join("downloads/Regression - Effects.osz");
    archive_at(&tko, "Regression - Effects [Oni].tko", 2, false);
    s.import_osz(tko, "Regression - Effects", false);
    finish(&mut s, &mut now);
    assert_eq!(
        folders(&f.custom),
        ["Regression - Effects"],
        "{}",
        toast(&s)
    );
    assert_eq!(folders(&f.songs), ["Regression - Plain"]);
    let m = custom::Manifest::read(&f.custom.join("Regression - Effects")).unwrap();
    assert_eq!((m.id, m.origin), (None, Some(custom::Origin::Import)));
}

/// A lobby player asking the host for map `hash`, then taking `archive` as the host sends it.
fn take_from_host(s: &mut State, now: &mut Instant, archive: &Path, hash: u64, name: &str) {
    let mut room = crate::layout_tests::session(false);
    if let Some(m) = &mut room.map {
        m.hash = hash;
    }
    s.mp = Some(room);
    s.host_map_asked = true;
    let bytes = std::fs::read(archive).unwrap();
    let total = bytes.len() as u64;
    for (k, chunk) in bytes.chunks(download::PART_BYTES).enumerate() {
        s.receive_map_part(net::MapPart {
            name: name.into(),
            total,
            offset: (k * download::PART_BYTES) as u64,
            data: download::base64_encode(chunk),
            error: None,
        });
    }
    finish(s, now);
    s.mp = None;
}

#[test]
fn a_hosts_custom_set_goes_into_the_custom_folder_and_an_osu_set_into_songs() {
    let f = Folders::new("host");
    let (mut s, mut now) = client(&f);
    let sent = f.root.join("sent.osz");
    let hash = archive_at(&sent, "Regression - Theirs [Oni].osu", 1, true);
    take_from_host(&mut s, &mut now, &sent, hash, "c9201 Regression - Theirs");
    assert_eq!(
        folders(&f.custom),
        ["c9201 Regression - Theirs"],
        "{}",
        toast(&s)
    );
    assert!(folders(&f.songs).is_empty());
    let m = custom::Manifest::read(&f.custom.join("c9201 Regression - Theirs")).unwrap();
    assert_eq!(m.id, Some(9201), "the website's number it came with");
    assert_eq!(m.origin, Some(custom::Origin::Lobby));

    let osu = f.root.join("osu.osz");
    let hash = archive_at(&osu, "Regression - Ranked [Oni].osu", 2, false);
    take_from_host(&mut s, &mut now, &osu, hash, "622136 Regression - Ranked");
    assert_eq!(
        folders(&f.songs),
        ["622136 Regression - Ranked"],
        "{}",
        toast(&s)
    );
    assert_eq!(folders(&f.custom), ["c9201 Regression - Theirs"]);
}
