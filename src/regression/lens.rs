//! Song select's lens (`ui::lens`): the source tabs over the list and each source's own sort,
//! order, headings and filters; the sort and filter panel's keys; the config keeping it all. On a
//! made-up library of osu!'s maps, taiko.rs's own and TJA charts.

use super::snapshot::assert_snapshot;
use crate::beatmap::Meta;
use crate::library::Entry;
use crate::ui::lens::{Lens, LensItem, Order, Source};
use crate::ui::{Select, SortMode};
use std::fmt::Write as _;
use std::path::PathBuf;
use winit::keyboard::KeyCode;

const OWN_ROOT: &str = "/regress-lens-own";

/// (path, artist, title, version, TJA level, date added). TJA charts are entries at their file's
/// path and course (`song.tja#oni`).
type Map = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    Option<f32>,
    u64,
);

const MAPS: [Map; 11] = [
    (
        "/songs/1 a/Easy.osu",
        "Alpha",
        "Zebra Run",
        "Easy",
        None,
        50,
    ),
    ("/songs/1 a/Oni.osu", "Alpha", "Zebra Run", "Oni", None, 50),
    ("/songs/2 b/Oni.osu", "Beta", "Apple Pie", "Oni", None, 300),
    (
        "/songs/3 c/Hard.osu",
        "Gamma",
        "Midnight",
        "Hard",
        None,
        200,
    ),
    (
        "/regress-lens-own/4 mine/Oni.osu",
        "Me",
        "Homemade",
        "Oni",
        None,
        400,
    ),
    (
        "/tja/Pack/Pop/Kaze/Kaze.tja#easy",
        "Kiraru",
        "Kaze no Uta",
        "Easy",
        Some(3.0),
        100,
    ),
    (
        "/tja/Pack/Pop/Kaze/Kaze.tja#oni",
        "Kiraru",
        "Kaze no Uta",
        "Oni",
        Some(8.0),
        100,
    ),
    (
        "/tja/Pack/Anime/Hoshi/Hoshi.tja#hard",
        "Senka",
        "Hoshi Drive",
        "Hard",
        Some(6.0),
        150,
    ),
    (
        "/tja/Pack/Anime/Hoshi/Hoshi.tja#oni",
        "Senka",
        "Hoshi Drive",
        "Oni",
        Some(9.0),
        150,
    ),
    (
        "/tja/Pack/Anime/Hoshi/Hoshi.tja#edit",
        "Senka",
        "Hoshi Drive",
        "Ura Oni",
        Some(10.0),
        150,
    ),
    (
        "/tja/Pack/Pop/Ame/Ame.tja#oni",
        "Ao Kumo",
        "Ame Agari",
        "Oni",
        Some(5.0),
        250,
    ),
];

fn library() -> Vec<Entry> {
    crate::library::add_own_root(std::path::Path::new(OWN_ROOT));
    let mut maps: Vec<Entry> = MAPS
        .iter()
        .enumerate()
        .map(|(k, &(path, artist, title, version, level, added))| {
            let meta = Meta {
                artist: artist.into(),
                title: title.into(),
                creator: "kat".into(),
                version: version.into(),
                mode: 1,
                od: 5.0,
                hp: 5.0,
                level,
                ..Meta::default()
            };
            Entry::from_parts(PathBuf::from(path), meta, 100 + k as u64, added)
        })
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

fn select(library: &[Entry]) -> Select {
    let mut s = Select::new(library);
    s.favourites.clear();
    s.played.clear();
    s.refilter(library);
    s
}

/// The list as headings and songs, each song as its difficulties.
fn list(out: &mut String, s: &Select, library: &[Entry]) {
    let _ = writeln!(out, "  [{}: {}]", s.shown_source().label(), s.lens().text());
    let songs = s.songs();
    let mut at = 0;
    let line = |out: &mut String, song: &[usize]| {
        let names: Vec<String> = song
            .iter()
            .map(|&i| format!("{} [{}]", library[i].meta.title, library[i].meta.version))
            .collect();
        let _ = writeln!(out, "    {}", names.join(" | "));
    };
    if s.headings {
        for (heading, count) in s.sections() {
            let _ = writeln!(out, "  # {heading}");
            for song in &songs[at..at + count] {
                line(out, song);
            }
            at += count;
        }
    }
    for song in &songs[at..] {
        line(out, song);
    }
}

fn sort_item(sort: SortMode) -> LensItem {
    LensItem::Sort(SortMode::ALL.iter().position(|&m| m == sort).unwrap())
}

fn order_item(order: Order) -> LensItem {
    LensItem::Order(Order::ALL.iter().position(|&o| o == order).unwrap())
}

#[test]
fn each_source_has_its_tab_and_its_own_lens() {
    let lib = library();
    let mut s = select(&lib);
    let mut out = String::new();
    let tabs: Vec<&str> = s.source_tabs().iter().map(|t| t.label()).collect();
    let _ = writeln!(out, "tabs: {tabs:?}, maps {:?}", s.source_maps);
    let _ = writeln!(out, "all sources, as the game starts:");
    list(&mut out, &s, &lib);
    s.set_source(Source::Tja, &lib);
    let _ = writeln!(out, "TJA: under the pack's folders the first time:");
    list(&mut out, &s, &lib);
    s.lens_pick(sort_item(SortMode::Difficulty), &lib);
    let _ = writeln!(out, "TJA by difficulty:");
    list(&mut out, &s, &lib);
    s.set_source(Source::Osu, &lib);
    let _ = writeln!(out, "osu!: as all sources were:");
    list(&mut out, &s, &lib);
    s.set_source(Source::Own, &lib);
    let _ = writeln!(out, "taiko.rs's own, by status:");
    list(&mut out, &s, &lib);
    s.set_source(Source::Tja, &lib);
    let _ = writeln!(out, "back to TJA, still by difficulty:");
    list(&mut out, &s, &lib);
    assert_eq!(s.sort, SortMode::Difficulty);
    assert_snapshot("lens-sources", &out);

    // Ctrl and a digit: the tabs in order, all first; none past the last.
    assert!(s.source_key(0, &lib));
    assert_eq!(s.shown_source(), Source::All);
    assert!(s.source_key(3, &lib));
    assert_eq!(s.shown_source(), Source::Own);
    assert!(!s.source_key(4, &lib), "four tabs only");

    // One source only: no tabs, and all of it listed whatever was picked.
    let osu: Vec<Entry> = library()
        .into_iter()
        .filter(|e| Source::of(e) == Source::Osu)
        .collect();
    let mut one = select(&osu);
    one.tabs.source = Source::Tja;
    one.refilter(&osu);
    assert!(one.source_tabs().is_empty());
    assert_eq!(one.shown_source(), Source::All);
    assert_eq!(one.song_groups().len(), 3);
}

#[test]
fn the_config_keeps_the_source_and_each_lens() {
    let lib = library();
    let mut s = select(&lib);
    s.set_source(Source::Tja, &lib);
    s.lens_pick(order_item(Order::Stars), &lib);
    s.lens_pick(LensItem::Desc, &lib);
    s.set_source(Source::All, &lib);
    s.lens_pick(sort_item(SortMode::Title), &lib);
    s.lens_pick(order_item(Order::Added), &lib);
    s.set_source(Source::Own, &lib);
    let mut cfg = crate::config::Config::default();
    s.write_lenses(&mut cfg);
    assert_eq!(cfg.sort, "title");
    assert_eq!(cfg.song_order, "added");
    assert_eq!(cfg.song_source, "own");
    assert_eq!(cfg.lens_tja, "folder stars desc headings");
    // osu!'s tab keeps the lens all sources had when the game started: each tab is its own.
    assert_eq!(cfg.lens_osu, "artist sort asc flat");
    assert_eq!(cfg.lens_own, "", "as it starts");
    let cfg = crate::config::Config::parse_text(&cfg.to_text());
    let mut back = select(&lib);
    back.read_lenses(&cfg);
    back.refilter(&lib);
    assert_eq!(back.shown_source(), Source::Own);
    for source in Source::ALL {
        assert_eq!(back.lens_of(source), s.lens_of(source), "{source:?}");
    }
    // A lens the config can't read: the one the source starts with.
    assert_eq!(Lens::parse("folder sideways asc headings"), None);
    assert_eq!(
        Lens::parse("recently_played bpm desc flat").map(|l| l.sort),
        Some(SortMode::Played)
    );
}

#[test]
fn the_lens_filters_and_orders_the_list() {
    let lib = library();
    let mut s = select(&lib);
    let mut out = String::new();
    s.set_source(Source::Tja, &lib);
    s.toggle_lens(&lib);
    let _ = writeln!(out, "TJA star bars: {:?}", s.star_bins);
    // A bar, then a higher one: from the first to the second.
    s.lens_pick(LensItem::Star(7), &lib);
    s.lens_pick(LensItem::Star(9), &lib);
    let _ = writeln!(out, "stars 8 to 10+ ({:?}):", s.tabs.filters.tokens());
    list(&mut out, &s, &lib);
    s.lens_pick(LensItem::Course(3), &lib);
    let _ = writeln!(out, "and Oni only:");
    list(&mut out, &s, &lib);
    // The only bar on, clicked again: any stars.
    s.lens_pick(LensItem::Star(7), &lib);
    s.lens_pick(LensItem::Star(7), &lib);
    assert_eq!(s.tabs.filters.stars, None);
    s.lens_pick(LensItem::Course(3), &lib);
    assert!(s.tabs.filters.is_empty());
    s.lens_pick(order_item(Order::Title), &lib);
    s.lens_pick(LensItem::Desc, &lib);
    let _ = writeln!(out, "by folder, then by title backwards:");
    list(&mut out, &s, &lib);
    s.lens_pick(LensItem::Headings, &lib);
    s.lens_pick(order_item(Order::Added), &lib);
    let _ = writeln!(out, "one list, newest first:");
    list(&mut out, &s, &lib);
    s.lens_pick(LensItem::Reset, &lib);
    let _ = writeln!(out, "reset:");
    list(&mut out, &s, &lib);

    // osu!'s status and plays.
    s.set_source(Source::All, &lib);
    let path = |title: &str| {
        lib.iter()
            .find(|e| e.meta.title == title)
            .unwrap()
            .path
            .clone()
    };
    s.status.insert(path("Apple Pie"), crate::search::RANKED);
    s.status.insert(path("Midnight"), crate::search::LOVED);
    s.played.insert(path("Midnight"), 1_000);
    s.lens_pick(LensItem::Status(0), &lib);
    let _ = writeln!(out, "all sources, ranked:");
    list(&mut out, &s, &lib);
    s.lens_pick(LensItem::Status(1), &lib);
    let _ = writeln!(out, "ranked or loved:");
    list(&mut out, &s, &lib);
    s.lens_pick(LensItem::Unplayed, &lib);
    let _ = writeln!(out, "and not played yet:");
    list(&mut out, &s, &lib);
    let tokens: Vec<String> = s.tabs.filters.tokens().into_iter().map(|t| t.1).collect();
    let _ = writeln!(out, "tokens: {tokens:?}");
    // Taking a token off takes its filter off.
    s.tabs.filters.clear(crate::ui::Token::Status);
    s.refilter(&lib);
    let _ = writeln!(out, "status token off:");
    list(&mut out, &s, &lib);
    // Lengths aren't known without the background analysis: nothing passes a length filter.
    s.tabs.filters = Default::default();
    s.lens_pick(LensItem::Length(1), &lib);
    assert!(s.song_groups().is_empty(), "lengths not worked out yet");
    let _ = writeln!(out, "a length filter before the lengths are known: nothing");
    assert_snapshot("lens-filters", &out);
}

#[test]
fn the_panel_keys_move_pick_and_close() {
    let lib = library();
    let mut s = select(&lib);
    s.toggle_lens(&lib);
    let ui = s.lens_ui.expect("open");
    assert_eq!(ui.at.0, 0, "on the sort column");
    assert_eq!(SortMode::ALL[ui.at.1], s.sort, "on the sort the list has");
    // Down to the next sort, Enter picks it.
    s.lens_key(KeyCode::ArrowDown, &lib);
    assert!(s.lens_key(KeyCode::Enter, &lib), "a pick changes the lens");
    assert_eq!(s.sort, SortMode::Title);
    // Right: the order column, at about the same height.
    s.lens_key(KeyCode::ArrowRight, &lib);
    s.lens_key(KeyCode::Space, &lib);
    assert_eq!(s.tabs.order, Order::Title);
    // Right again: the star bars, then along them.
    s.lens_key(KeyCode::ArrowRight, &lib);
    s.lens_key(KeyCode::ArrowUp, &lib);
    s.lens_key(KeyCode::ArrowUp, &lib);
    s.lens_key(KeyCode::ArrowRight, &lib);
    s.lens_key(KeyCode::Enter, &lib);
    assert_eq!(s.tabs.filters.stars, Some((1, 1)), "the second bar");
    // Esc closes, the lens staying as picked.
    assert!(!s.lens_key(KeyCode::Escape, &lib));
    assert!(s.lens_ui.is_none());
    assert_eq!(s.tabs.filters.stars, Some((1, 1)));
    // Opening again, then the toggle: closed.
    s.toggle_lens(&lib);
    s.toggle_lens(&lib);
    assert!(s.lens_ui.is_none());
}
