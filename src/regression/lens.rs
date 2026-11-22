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
    let _ = writeln!(out, "TJA star bars: {:?}", s.facets.stars);
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

#[test]
fn the_filters_stay_across_tabs_and_count_what_they_leave() {
    let lib = library();
    let mut s = select(&lib);
    let path = |title: &str| {
        lib.iter()
            .find(|e| e.meta.title == title)
            .unwrap()
            .path
            .clone()
    };
    s.status.insert(path("Apple Pie"), crate::search::RANKED);
    s.status.insert(path("Midnight"), crate::search::LOVED);
    s.played.insert(path("Kaze no Uta"), 1_000);
    s.refilter(&lib);
    let mut out = String::new();
    let tokens = |s: &Select| -> Vec<String> {
        s.tabs
            .active(s.shown_source())
            .tokens()
            .into_iter()
            .map(|t| t.1)
            .collect()
    };
    let rows = |s: &Select| -> Vec<String> {
        s.lens_grid()[2]
            .iter()
            .map(|row| format!("{:?}", row[0]))
            .collect()
    };
    let state = |out: &mut String, s: &Select, what: &str| {
        let _ = writeln!(
            out,
            "{what}: tabs list {:?} (maps {:?}), tokens {:?}",
            s.source_listed,
            s.source_maps,
            tokens(s)
        );
        let _ = writeln!(out, "  filter rows: {:?}", rows(s));
        let f = &s.facets;
        let _ = writeln!(
            out,
            "  counts: stars {:?}, status {:?}, course {:?}",
            f.stars, f.statuses, f.courses
        );
    };
    s.toggle_lens(&lib);
    state(&mut out, &s, "all sources, nothing on");
    list(&mut out, &s, &lib);
    // Not played yet: on in every tab, the tabs' counts with it.
    s.lens_pick(LensItem::Unplayed, &lib);
    state(&mut out, &s, "not played yet");
    s.set_source(Source::Tja, &lib);
    state(&mut out, &s, "TJA, still not played yet");
    list(&mut out, &s, &lib);
    // A course: only among TJA charts.
    s.lens_pick(LensItem::Course(3), &lib);
    state(&mut out, &s, "TJA, and Oni");
    list(&mut out, &s, &lib);
    s.set_source(Source::Osu, &lib);
    state(&mut out, &s, "osu!: the course filter left for TJA");
    list(&mut out, &s, &lib);
    // A status: never among TJA charts.
    s.lens_pick(LensItem::Status(0), &lib);
    state(&mut out, &s, "osu!, and ranked");
    list(&mut out, &s, &lib);
    s.set_source(Source::Tja, &lib);
    state(&mut out, &s, "TJA: Oni again, ranked left for the others");
    list(&mut out, &s, &lib);
    s.set_source(Source::All, &lib);
    state(&mut out, &s, "all sources: ranked, not played");
    list(&mut out, &s, &lib);
    assert_eq!(
        s.tabs.filters.courses,
        1 << 3,
        "kept, for the TJA tab, though it doesn't filter here"
    );
    assert_snapshot("lens-filters-across-tabs", &out);

    // The status choices are the statuses the tab's maps have, and any picked.
    s.tabs.filters = Default::default();
    s.set_source(Source::Own, &lib);
    let statuses: Vec<String> = s.lens_grid()[2]
        .iter()
        .flatten()
        .filter(|i| matches!(i, LensItem::Status(_)))
        .map(|i| format!("{i:?}"))
        .collect();
    assert_eq!(
        statuses,
        ["Status(3)"],
        "taiko.rs's map, never submitted: Local only"
    );
}

#[test]
fn the_shown_source_tab_has_a_bar_of_the_accent_and_no_brackets_or_colour_squares() {
    let lib = library();
    let s = select(&lib);
    let tabs = s.source_tabs();
    let mut c = crate::render::Canvas {
        width: 853.0,
        ..Default::default()
    };
    s.draw_source_tabs(&mut c, 24.0, 60.0, 400.0, &tabs);
    let accent = crate::render::rgba(crate::render::accent(), 1.0);
    let bars: Vec<_> = c
        .shapes
        .iter()
        .filter(|sh| sh.fill == accent && sh.half[1] * 2.0 <= 2.01 && sh.half[0] * 2.0 > 40.0)
        .collect();
    assert_eq!(bars.len(), 1, "one bar, under the shown tab");
    let shown = c
        .hits
        .iter()
        .find(|h| h.hit == crate::hit::Hit::SongSource(tabs[0].index()))
        .map(|h| h.rect)
        .expect("the shown tab");
    let [bx, by] = bars[0].center;
    assert!(
        bx > shown[0] && bx < shown[0] + shown[2],
        "under the shown tab"
    );
    assert!(by > shown[1] + shown[3] / 2.0, "along its bottom");
    // No outline in the selector's colour (the list's brackets), no small square per source.
    let selector = crate::render::rgba(crate::style::selector_colour(), 1.0);
    assert!(
        c.shapes
            .iter()
            .all(|sh| sh.border[..3] != selector[..3] || sh.params[1] == 0.0)
    );
    for tab in &tabs {
        let col = crate::render::rgba(tab.colour(), 1.0);
        assert!(
            !c.shapes
                .iter()
                .any(|sh| sh.fill[..3] == col[..3] && sh.half[0] * 2.0 <= 6.0),
            "no square of {}'s colour",
            tab.label()
        );
    }
}

#[test]
fn each_source_tab_sits_on_a_chip_of_its_own_the_shown_one_lifted() {
    use crate::style::{Surface, radius, surface_fill};
    let lib = library();
    let s = select(&lib);
    let tabs = s.source_tabs();
    let mut c = crate::render::Canvas {
        width: 853.0,
        ..Default::default()
    };
    s.draw_source_tabs(&mut c, 24.0, 60.0, 400.0, &tabs);
    let chip = |level: Surface| {
        c.shapes
            .iter()
            .filter(move |sh| sh.fill == surface_fill(level) && sh.params[0] == radius::SMALL)
            .map(|sh| sh.center)
            .collect::<Vec<_>>()
    };
    let (raised, lifted) = (chip(Surface::Raised), chip(Surface::Lifted));
    assert_eq!(lifted.len(), 1, "the shown tab lifted: {lifted:?}");
    assert_eq!(
        raised.len(),
        tabs.len() - 1,
        "the others raised: {raised:?}"
    );
    for (k, tab) in tabs.iter().enumerate() {
        let hit = c
            .hits
            .iter()
            .find(|h| h.hit == crate::hit::Hit::SongSource(tab.index()))
            .map(|h| h.rect)
            .unwrap_or_else(|| panic!("{}'s tab", tab.label()));
        let on = |[x, y]: [f32; 2]| {
            x > hit[0] && x < hit[0] + hit[2] && y > hit[1] && y < hit[1] + hit[3]
        };
        let under: Vec<_> = raised.iter().chain(&lifted).filter(|p| on(**p)).collect();
        assert_eq!(under.len(), 1, "one chip under {}", tab.label());
        assert_eq!(on(lifted[0]), k == 0, "{} lifted", tab.label());
    }
}
