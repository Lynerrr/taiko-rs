//! Song select's list logic (`ui::Select`), without drawing it: how a library is sorted and
//! grouped into songs and headings, what each search finds, which maps converts and the
//! keyboard reach, and what "recently played" and "date added" put first. The screen goldens
//! only show the first rows of one library; this keeps the whole list for a varied one.

use super::snapshot::assert_snapshot;
use crate::beatmap::Meta;
use crate::library::Entry;
use crate::ui::{Hidden, Select, SortMode};
use std::fmt::Write as _;
use std::path::PathBuf;

/// The made-up library, a difficulty per row: its set's folder, artist, title, mapper, version,
/// mode (1 taiko, 0 a convert), OD, HP, file size (orders a set's difficulties) and date added.
/// Five sets: two artists starting alike, a convert-only set, a set mixing taiko and a convert,
/// odd capitals and a non-Latin title.
type Row = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    u8,
    f64,
    f64,
    u64,
    u64,
);
const MAPS: [Row; 9] = [
    (
        "1 a",
        "Alpha",
        "Zebra Run",
        "kat",
        "Easy",
        1,
        3.0,
        3.0,
        100,
        50,
    ),
    (
        "1 a",
        "Alpha",
        "Zebra Run",
        "kat",
        "Oni",
        1,
        7.5,
        6.0,
        300,
        50,
    ),
    (
        "1 a",
        "Alpha",
        "Zebra Run",
        "kat",
        "Hard",
        1,
        5.5,
        5.0,
        200,
        50,
    ),
    (
        "2 b",
        "alphabet",
        "Middle",
        "don",
        "Muzukashii",
        1,
        6.0,
        4.0,
        150,
        10,
    ),
    (
        "3 c",
        "Beta",
        "Converted",
        "std",
        "Insane",
        0,
        8.0,
        7.0,
        400,
        30,
    ),
    (
        "3 c",
        "Beta",
        "Converted",
        "std",
        "Normal",
        0,
        4.0,
        4.0,
        120,
        30,
    ),
    (
        "4 d",
        "Gamma",
        "Both Kinds",
        "kat",
        "Inner Oni",
        1,
        8.5,
        6.5,
        500,
        40,
    ),
    (
        "4 d",
        "Gamma",
        "Both Kinds",
        "kat",
        "Convert",
        0,
        5.0,
        5.0,
        220,
        40,
    ),
    (
        "5 e",
        "Delta",
        "夜明け",
        "don",
        "Futsuu",
        1,
        4.5,
        3.5,
        110,
        20,
    ),
];

/// `MAPS` as library entries, in the order the library scan hands them over (artist, title,
/// folder, then file size), which the list relies on.
fn library() -> Vec<Entry> {
    let mut maps: Vec<Entry> = MAPS
        .iter()
        .map(
            |&(set, artist, title, creator, version, mode, od, hp, size, added)| {
                let meta = Meta {
                    title: title.into(),
                    artist: artist.into(),
                    creator: creator.into(),
                    version: version.into(),
                    mode,
                    od,
                    hp,
                    ..Meta::default()
                };
                let path = PathBuf::from(format!("/songs/{set}/{version}.osu"));
                Entry::from_parts(path, meta, size, added)
            },
        )
        .collect();
    maps.sort_by(|a, b| {
        let key = |e: &Entry| {
            (
                e.meta.artist.to_lowercase(),
                e.meta.title.to_lowercase(),
                e.path.parent().map(|p| p.to_path_buf()),
            )
        };
        key(a).cmp(&key(b)).then(a.size.cmp(&b.size))
    });
    maps
}

/// The library's difficulty named `version`.
fn by_version(library: &[Entry], version: &str) -> usize {
    library
        .iter()
        .position(|e| e.meta.version == version)
        .unwrap()
}

/// A fresh list over `library` with nothing hidden, liked or played (whatever this machine's
/// data folder holds).
fn select(library: &[Entry]) -> Select {
    let mut s = Select::new(library);
    s.hidden = Hidden::default();
    s.favourites.clear();
    s.played.clear();
    s.refilter(library);
    s
}

fn name(library: &[Entry], i: usize) -> String {
    let m = &library[i].meta;
    format!("{} - {} [{}]", m.artist, m.title, m.version)
}

/// The list as headings, songs and their difficulties, in order.
fn list(out: &mut String, s: &Select, library: &[Entry]) {
    let songs = s.songs();
    let mut at = 0;
    for (heading, count) in s.sections() {
        let _ = writeln!(out, "  # {heading}");
        for song in &songs[at..at + count] {
            let names: Vec<String> = song.iter().map(|&i| name(library, i)).collect();
            let _ = writeln!(out, "    {}", names.join(" | "));
        }
        at += count;
    }
    for song in &songs[at..] {
        let names: Vec<String> = song.iter().map(|&i| name(library, i)).collect();
        let _ = writeln!(out, "    {}", names.join(" | "));
    }
}

#[test]
fn song_select_sorts_groups_and_searches() {
    let lib = library();
    let mut s = select(&lib);
    let mut out = String::new();

    let _ = writeln!(
        out,
        "taiko only (the default with taiko maps in the library):"
    );
    list(&mut out, &s, &lib);
    s.keep_converts(&lib, true);
    let _ = writeln!(out, "with converts:");
    list(&mut out, &s, &lib);

    // Sorts that need no background analysis.
    for sort in [
        SortMode::Artist,
        SortMode::Title,
        SortMode::Hp,
        SortMode::Od,
        SortMode::Added,
    ] {
        s.set_sort(sort, &lib);
        let _ = writeln!(out, "sort {}:", sort.name());
        list(&mut out, &s, &lib);
    }
    s.set_headings(false, &lib);
    let _ = writeln!(out, "sort date added, no headings:");
    list(&mut out, &s, &lib);
    s.set_headings(true, &lib);

    // Recently played first.
    s.played
        .insert(lib[by_version(&lib, "Muzukashii")].path.clone(), 2_000);
    s.played
        .insert(lib[by_version(&lib, "Futsuu")].path.clone(), 1_000);
    s.set_sort(SortMode::Played, &lib);
    let _ = writeln!(out, "sort recently played (Middle, then 夜明け):");
    list(&mut out, &s, &lib);

    s.set_sort(SortMode::Artist, &lib);
    for query in [
        "zebra",
        "ALPHA",
        "kat",
        "od>7",
        "hp<4",
        "od>=5 hp<=5",
        "creator=don",
        "夜明け",
        "nothing like this",
    ] {
        s.filter = query.into();
        s.refilter(&lib);
        let _ = writeln!(out, "search {query:?}:");
        list(&mut out, &s, &lib);
    }
    s.filter.clear();
    s.refilter(&lib);
    assert_snapshot("select-list-logic", &out);
}

#[test]
fn song_select_moves_through_songs_and_difficulties() {
    let lib = library();
    let mut s = select(&lib);
    s.keep_converts(&lib, true);
    let mut out = String::new();
    s.set_cursor(0);
    let shown = |s: &Select| s.selected().map_or("nothing".into(), |i| name(&lib, i));
    let _ = writeln!(out, "first row: {}", shown(&s));
    for (what, n) in [("down", 1), ("down", 1), ("down", 1), ("up", -1)] {
        s.step(n, &lib);
        let _ = writeln!(out, "{what}: {}", shown(&s));
    }
    for (what, n) in [("next song", 1), ("next song", 1), ("previous song", -1)] {
        s.jump_songs(n, &lib);
        let _ = writeln!(out, "{what}: {}", shown(&s));
    }
    for (what, n) in [("ten songs on", 10), ("ten songs back", -10)] {
        s.jump_songs(n, &lib);
        let _ = writeln!(out, "{what}: {}", shown(&s));
    }
    s.select_path(&lib, &lib[by_version(&lib, "Convert")].path);
    let _ = writeln!(out, "picked by path: {}", shown(&s));
    s.filter = "gamma".into();
    s.refilter(&lib);
    let _ = writeln!(out, "after searching gamma: {}", shown(&s));
    s.filter.clear();
    s.refilter(&lib);
    let _ = writeln!(out, "search cleared: {}", shown(&s));
    assert_snapshot("select-list-moves", &out);
}

#[test]
fn song_select_search_finds_tags_source_and_ids() {
    let mut lib = library();
    for e in &mut lib {
        let mut meta = e.meta.clone();
        match meta.version.as_str() {
            "Oni" => {
                meta.tags = "Vocaloid Speedcore".into();
                meta.source = "Touhou Project".into();
                meta.beatmap_id = Some(9001);
                meta.set_id = Some(4242);
            }
            "Easy" => {
                meta.tags = "vocaloid".into();
                meta.set_id = Some(4242);
                meta.beatmap_id = Some(9000);
            }
            _ => {}
        }
        *e = Entry::from_parts(e.path.clone(), meta, e.size, e.modified);
    }
    let mut s = select(&lib);
    let mut out = String::new();
    for query in [
        "vocaloid",
        "speedcore zebra",
        "touhou",
        "tag=vocal",
        "source=touhou",
        "9001",
        "4242",
        "id=9000",
        "set=4242",
        "set=4243",
        "id>9000",
        "no tag here",
    ] {
        s.filter = query.into();
        s.refilter(&lib);
        let _ = writeln!(out, "search {query:?}:");
        list(&mut out, &s, &lib);
    }
    assert_snapshot("select-search-tags-ids", &out);
}
