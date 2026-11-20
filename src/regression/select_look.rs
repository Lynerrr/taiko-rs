//! Song select in the main menu's look draws with the design tokens (`style`: surfaces, ink,
//! colour roles): its surfaces have no edge round them, the accent is only on Play and the
//! selection, and a rating's colour only on the selected difficulty. The Legacy layout keeps its
//! own look (its goldens in `screens`).

use super::harness::{self, *};
use super::maps;
use crate::hit::Hit;
use crate::render::Shape;
use crate::*;

/// An accent nothing else on the screen is drawn in, so what's in it is the accent's.
const ACCENT: u32 = 0x13c4a7;

fn menu_look(tag: &str) -> (State, Songs, Instant) {
    menu_look_with(tag, &maps::all())
}

/// `menu_look` with these charts, each a set of its own, every one naming a picture.
fn menu_look_with(tag: &str, charts: &[maps::Chart]) -> (State, Songs, Instant) {
    menu_look_cfg(tag, charts, harness::config().animations)
}

/// `menu_look` with animations on.
fn menu_look_moving(tag: &str) -> (State, Songs, Instant) {
    menu_look_cfg(tag, &maps::all(), true)
}

fn menu_look_cfg(tag: &str, charts: &[maps::Chart], animations: bool) -> (State, Songs, Instant) {
    let songs = Songs::new(tag, charts);
    let cfg = Config {
        main_menu: true,
        accent: ACCENT,
        animations,
        ..harness::config()
    };
    let mut entries = songs.entries();
    for e in &mut entries {
        e.meta.background = Some("bg.png".into());
    }
    let mut s = state_with(cfg, entries);
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    assert!(!s.select.legacy && s.main_menu.is_none());
    (s, songs, now)
}

/// The screen once its fades have settled.
fn settled(s: &mut State, now: &mut Instant) -> Canvas {
    frame(s, *now);
    s.scene_since = *now - Duration::from_secs(5);
    s.toast = None;
    settle(s, now);
    *now += Duration::from_millis(16);
    frame(s, *now)
}

fn same_rgb(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).take(3).all(|(x, y)| (x - y).abs() < 1e-4)
}

/// A shape's `[x, y, w, h]`.
fn rect(s: &Shape) -> [f32; 4] {
    [
        s.center[0] - s.half[0],
        s.center[1] - s.half[1],
        s.half[0] * 2.0,
        s.half[1] * 2.0,
    ]
}

fn inside([x, y]: [f32; 2], [rx, ry, rw, rh]: [f32; 4], slack: f32) -> bool {
    x >= rx - slack && x <= rx + rw + slack && y >= ry - slack && y <= ry + rh + slack
}

/// A rectangle or rounded box (not a letter, a skin image or a picture).
fn plain(s: &Shape) -> bool {
    (0.0..10.0).contains(&s.params[3])
}

/// A letter of the font (the test canvas draws text as plain rectangles, a pixel each, so text
/// counts in what's plain too).
fn glyph(s: &Shape) -> bool {
    s.params[3] >= 100.0
}

/// Song select's own part of the screen: under the top bar, left of the friends tab, down to the
/// lane of actions.
fn on_select(s: &Shape, c: &Canvas, state: &State) -> bool {
    let [_, end] = state.select.frame_span();
    let room = [0.0, 16.0, c.width - social::TAB_ROOM, end - 16.0];
    inside(s.center, room, 0.0)
}

fn hit_rect(c: &Canvas, want: impl Fn(&Hit) -> bool) -> Option<[f32; 4]> {
    c.hits.iter().find(|h| want(&h.hit)).map(|h| h.rect)
}

/// Where the selected set's card is (its row's hit).
fn open_card(c: &Canvas, s: &State) -> [f32; 4] {
    let cursor = s.select.cursor();
    let start = s
        .select
        .song_groups()
        .iter()
        .find(|g| g.contains(&cursor))
        .map(|g| g.start);
    hit_rect(c, |h| matches!(h, Hit::SongHeader(i) if Some(*i) == start)).expect("the open set")
}

/// The sets' cards drawn: first row and where.
fn cards(c: &Canvas) -> Vec<(usize, [f32; 4])> {
    c.hits
        .iter()
        .filter_map(|h| match h.hit {
            Hit::SongHeader(i) => Some((i, h.rect)),
            _ => None,
        })
        .collect()
}

/// Twenty sets: more than the list shows.
fn many() -> Vec<maps::Chart> {
    (0..20)
        .map(|i| maps::Chart {
            name: Box::leak(format!("set-{i:02}").into_boxed_str()),
            ..maps::basics()
        })
        .collect()
}

#[test]
fn surfaces_have_no_edge_and_rows_no_lines_under_them() {
    let (mut s, _songs, mut now) = menu_look("select-look-edges");
    let c = settled(&mut s, &mut now);
    // The selection's outline round the open set's card is the one strong edge; the other set
    // cards have a faint white one each, so neighbouring pictures of like colours stay apart.
    let open = open_card(&c, &s);
    let set_cards = cards(&c);
    let faint_card_edge = |sh: &Shape| {
        same_rgb(sh.border, render::rgba(0xffffff, 1.0))
            && sh.border[3] <= 0.2
            && set_cards.iter().any(|(_, r)| inside(sh.center, *r, 0.5))
    };
    let edged: Vec<[f32; 4]> = c
        .shapes
        .iter()
        .filter(|sh| plain(sh) && on_select(sh, &c, &s) && !inside(sh.center, open, 1.0))
        .filter(|sh| sh.params[1] > 0.0 && sh.border[3] > 0.01)
        .filter(|sh| !same_rgb(sh.border, sh.fill) || sh.fill[3] < sh.border[3])
        .filter(|sh| !faint_card_edge(sh))
        // The selector's brackets round the selected difficulty: corners of one outline.
        .filter(|sh| !same_rgb(sh.border, render::rgba(style::selector_colour(), 1.0)))
        .map(rect)
        .collect();
    assert!(edged.is_empty(), "boxes with an edge round them: {edged:?}");
    let parted = c.shapes.iter().filter(|sh| faint_card_edge(sh)).count();
    assert!(
        parted + 1 >= set_cards.len(),
        "every set card but the open one parted: {parted}"
    );
    // No line under the search row or a row of the list: rows are parted by room.
    let lw = ui::Select::list_width(c.width, s.cfg.song_list_share);
    let lines: Vec<[f32; 4]> = c
        .shapes
        .iter()
        .filter(|sh| plain(sh) && sh.fill[3] > 0.05 && on_select(sh, &c, &s))
        .filter(|sh| sh.center[0] < 28.0 + lw)
        .map(rect)
        .filter(|r| r[3] <= 1.6 && r[2] >= 40.0)
        .collect();
    assert!(lines.is_empty(), "lines across the list: {lines:?}");
}

#[test]
fn the_accent_is_only_on_play_and_the_selection() {
    let (mut s, _songs, mut now) = menu_look("select-look-accent");
    let c = settled(&mut s, &mut now);
    let accent = render::rgba(ACCENT, 1.0);
    let play = hit_rect(&c, |h| matches!(h, Hit::Key(KeyCode::Enter))).expect("Play is drawn");
    let cursor = s.select.cursor();
    let row = hit_rect(&c, |h| matches!(h, Hit::SongRow(i) if *i == cursor)).expect("the row");
    let start = s
        .select
        .song_groups()
        .iter()
        .find(|g| g.contains(&cursor))
        .map(|g| g.start);
    let set = hit_rect(&c, |h| matches!(h, Hit::SongHeader(i) if Some(*i) == start))
        .expect("the selected set's row");
    let stray: Vec<[f32; 4]> = c
        .shapes
        .iter()
        .filter(|sh| (plain(sh) || glyph(sh)) && on_select(sh, &c, &s))
        .filter(|sh| {
            let lit = |col: [f32; 4], on: bool| on && col[3] > 0.01 && same_rgb(col, accent);
            lit(sh.fill, true) || lit(sh.border, sh.params[1] > 0.0)
        })
        .filter(|sh| {
            // Play's glow reaches a little past it, the selector's brackets past the rows.
            !inside(sh.center, play, 10.0)
                && !inside(sh.center, row, 6.0)
                && !inside(sh.center, set, 6.0)
        })
        .map(rect)
        .collect();
    assert!(
        stray.is_empty(),
        "the accent off Play and the selection: {stray:?}"
    );
}

#[test]
fn the_selected_difficulty_is_in_its_ratings_colour() {
    let (mut s, _songs, mut now) = menu_look("select-look-rating");
    let c = settled(&mut s, &mut now);
    let i = s.select.selected().expect("a map is selected");
    let rate = s.play_speed().0;
    let stars = s.stars.get(i, rate);
    assert!(stars.is_some(), "the selected map's stars are known");
    let level = s.library[i].meta.level;
    let colour = render::rgba(style::role::rating_ink(level, stars), 1.0);
    let cursor = s.select.cursor();
    let row = hit_rect(&c, |h| matches!(h, Hit::SongRow(r) if *r == cursor)).expect("the row");
    // Its pill in the list.
    assert!(
        c.shapes
            .iter()
            .any(|sh| plain(sh) && inside(sh.center, row, 0.0) && same_rgb(sh.fill, colour)),
        "the selected difficulty's pill is in its rating's colour"
    );
    // Its chip in the details (left of the details' middle; its pill is at their right end).
    let lw = ui::Select::list_width(c.width, s.cfg.song_list_share);
    let middle = (28.0 + lw + c.width) / 2.0;
    assert!(
        c.shapes.iter().any(|sh| plain(sh)
            && sh.center[0] > 28.0 + lw
            && sh.center[0] < middle
            && sh.half[1] * 2.0 >= 17.0
            && on_select(sh, &c, &s)
            && same_rgb(sh.fill, colour)),
        "the difficulty's chip in the details is in its rating's colour"
    );
}

#[test]
fn every_set_card_on_screen_asks_for_its_own_picture() {
    let (mut s, _songs, mut now) = menu_look("select-look-pictures");
    let c = settled(&mut s, &mut now);
    let shown = cards(&c);
    assert!(shown.len() >= 5, "the sets are cards: {shown:?}");
    for (row, _) in shown {
        let e = &s.library[s.select.row_map(row).expect("a set's map")];
        let bg = preview::background_path(e).expect("it names a picture");
        assert!(
            c.cards_wanted
                .iter()
                .any(|(k, p, drawn)| *p == bg && *k == render::path_picture_key(&bg) && *drawn),
            "{} asks for its picture",
            e.meta.title
        );
    }
}

#[test]
fn the_open_set_slides_out_the_rest_sit_in_and_one_under_the_pointer_comes_out() {
    let (mut s, _songs, mut now) = menu_look("select-look-cards");
    let c = settled(&mut s, &mut now);
    let open = open_card(&c, &s);
    assert_eq!(open[0], 24.0, "the open set's card at the list's edge");
    let others: Vec<[f32; 4]> = cards(&c)
        .into_iter()
        .map(|(_, r)| r)
        .filter(|r| *r != open)
        .collect();
    assert!(!others.is_empty());
    // By 12 units, and the ones far from the target bend away farther (the lane's arc).
    assert!(
        others.iter().all(|r| r[0] >= 36.0) && others.iter().any(|r| r[0] == 36.0),
        "the rest sit in: {others:?}"
    );
    // The others are dimmed: none of their shapes is at full strength.
    let first = others[0];
    let strong = c
        .shapes
        .iter()
        .filter(|sh| plain(sh) && inside(sh.center, first, 0.0))
        .any(|sh| sh.fill[3] > 0.71);
    assert!(!strong, "a set not open is dimmed");
    // The pointer on one: it comes out 4 units (animations are off: at once).
    let k = (s.window_h / render::VIRTUAL_HEIGHT) as f64;
    let (px, py) = (first[0] + first[2] / 2.0, first[1] + 10.0);
    let (px, py) = (px as f64 * k, py as f64 * k);
    s.mouse_moved(px, py);
    s.mouse = (px, py);
    // Two frames: a frame's hits come back with the next one.
    for _ in 0..2 {
        now += Duration::from_millis(16);
        frame(&mut s, now);
    }
    now += Duration::from_millis(16);
    let c = frame(&mut s, now);
    let moved = cards(&c)
        .into_iter()
        .map(|(_, r)| r)
        .find(|r| (r[1] - first[1]).abs() < 0.5)
        .expect("the card under the pointer");
    assert_eq!(moved[0], first[0] - 4.0, "it comes out under the pointer");
}

#[test]
fn the_last_card_fades_out_at_the_lists_bottom() {
    let (mut s, _songs, mut now) = menu_look_with("select-look-bottom", &many());
    let c = settled(&mut s, &mut now);
    let [_, end] = s.select.frame_span();
    let bottom = end - 4.0;
    let lw = ui::Select::list_width(c.width, s.cfg.song_list_share);
    let cut = cards(&c)
        .into_iter()
        .map(|(_, r)| r)
        .find(|r| r[1] < bottom && r[1] + r[3] > bottom)
        .expect("a card runs past the list's bottom");
    // What of it is drawn just above the bottom is all but gone.
    let near: Vec<f32> = c
        .shapes
        .iter()
        .filter(|sh| plain(sh) && sh.center[0] > 20.0 && sh.center[0] < 28.0 + lw)
        .filter(|sh| {
            // Reaching into the last 3 units above the bottom.
            let [_, y, _, h] = rect(sh);
            y < bottom && y + h > bottom - 3.0
        })
        .map(|sh| sh.fill[3])
        .collect();
    assert!(
        !near.is_empty(),
        "the card at {cut:?} is drawn down to the bottom"
    );
    assert!(near.iter().all(|a| *a < 0.3), "faded out: {near:?}");
}

/// A picture drawn (`render::Canvas::picture`): its slot is in `params[3]` below -1000.
fn picture(s: &Shape) -> bool {
    s.params[3] <= -1000.0
}

#[test]
fn the_details_open_on_a_banner_of_the_maps_picture_fading_out() {
    let (mut s, _songs, mut now) = menu_look("select-look-banner");
    let i = s.select.selected().expect("a map is selected");
    let bg = preview::background_path(&s.library[i]).expect("it names a picture");
    s.pictures
        .pretend_loaded(&render::path_picture_key(&bg), 16.0 / 9.0);
    let c = settled(&mut s, &mut now);
    let lw = ui::Select::list_width(c.width, s.cfg.song_list_share);
    // Right of the list: the banner's layers of the picture, each cut to its clip.
    let layers: Vec<(f32, [f32; 4])> = c
        .shapes
        .iter()
        .enumerate()
        .filter(|(_, sh)| picture(sh) && sh.center[0] > 28.0 + lw)
        .map(|(i, sh)| {
            let clip = c
                .clips
                .iter()
                .find(|(r, _)| r.contains(&i))
                .map(|(_, rect)| *rect)
                .expect("a layer is cut to its strip");
            (sh.fill[3], clip)
        })
        .collect();
    assert!(layers.len() >= 6, "a banner in layers: {layers:?}");
    // How much of the picture shows at a height: the layers over it, together.
    let shown = |y: f32| {
        1.0 - layers
            .iter()
            .filter(|(_, r)| y >= r[1] && y < r[1] + r[3])
            .map(|(a, _)| 1.0 - a)
            .product::<f32>()
    };
    let (top, bottom) = (
        layers.iter().map(|(_, r)| r[1]).fold(f32::MAX, f32::min),
        layers.iter().map(|(_, r)| r[1] + r[3]).fold(0.0, f32::max),
    );
    let seen: Vec<f32> = (0..48)
        .map(|k| shown(top + (k as f32 + 0.5) / 48.0 * (bottom - top)))
        .collect();
    assert!((seen[0] - 1.0).abs() < 1e-3, "whole at the top: {seen:?}");
    assert!(
        seen.windows(2).all(|w| w[1] <= w[0] + 1e-4),
        "fading out downwards: {seen:?}"
    );
    assert!(seen[47] < 0.1, "gone at its bottom: {seen:?}");
    // No two layers end on the same edge: a shared edge is where rounding the clips to whole
    // pixels draws the picture twice, as a light line across it.
    let ends: Vec<f32> = layers.iter().map(|(_, r)| r[1] + r[3]).collect();
    for (k, e) in ends.iter().enumerate() {
        assert!(
            layers.iter().all(|(_, r)| (r[1] - e).abs() > 0.01),
            "a layer starts where another ends: {layers:?}"
        );
        assert!(
            ends.iter().skip(k + 1).all(|o| (o - e).abs() > 0.01),
            "two layers end alike: {ends:?}"
        );
    }
}

#[test]
fn don_and_kat_are_one_bar_split_in_proportion() {
    let (mut s, _songs, mut now) = menu_look("select-look-bar");
    let c = settled(&mut s, &mut now);
    let lw = ui::Select::list_width(c.width, s.cfg.song_list_share);
    let count = |suffix: &str| {
        c.texts
            .iter()
            .find_map(|(_, t)| t.strip_suffix(suffix)?.trim().parse::<f32>().ok())
            .unwrap_or_else(|| panic!("a count of{suffix}"))
    };
    let (dons, kats) = (count(" DON"), count(" KAT"));
    let notes = game::note_colours();
    let segment = |col: u32| {
        let fill = render::rgba(col, 1.0);
        c.shapes
            .iter()
            .filter(|sh| plain(sh) && sh.center[0] > 28.0 + lw && on_select(sh, &c, &s))
            .filter(|sh| same_rgb(sh.fill, fill))
            .map(rect)
            .find(|r| r[2] > 20.0 && r[3] < 8.0)
            .unwrap_or_else(|| panic!("a bar segment in {col:06x}"))
    };
    let (don, kat) = (segment(notes.don()), segment(notes.kat()));
    assert!(
        (don[0] + don[2] - kat[0]).abs() < 0.01,
        "kat runs on from don: {don:?} {kat:?}"
    );
    assert_eq!(don[1], kat[1]);
    let share = don[2] / (don[2] + kat[2]);
    assert!(
        (share - dons / (dons + kats)).abs() < 1e-3,
        "{share} of {dons}/{kats}"
    );
}

#[test]
fn a_cards_shade_is_one_sweep_as_round_as_the_card_over_its_rounded_picture() {
    let (mut s, _songs, mut now) = menu_look("select-look-shade");
    let i = s.select.selected().expect("a map is selected");
    let bg = preview::background_path(&s.library[i]).expect("it names a picture");
    s.card_pictures
        .pretend_loaded(&render::path_picture_key(&bg), [0.5, 0.5, 0.5]);
    let c = settled(&mut s, &mut now);
    let open = open_card(&c, &s);
    // The shade over the open card's picture: one shape (no seams between bands), from the
    // card's dark surface colour under the title to lighter on the right.
    let dark = render::rgba(style::SURFACE_1, 1.0);
    let shades: Vec<&Shape> = c
        .shapes
        .iter()
        .filter(|sh| sh.params[3] == render::SHADE_ACROSS && inside(sh.center, open, 0.0))
        .collect();
    assert_eq!(shades.len(), 1, "one sweep");
    let shade = shades[0];
    assert!(same_rgb(shade.fill, dark) && same_rgb(shade.border, dark));
    assert!(
        shade.fill[3] > 0.8 && (shade.border[3] - 0.4).abs() < 0.05,
        "shade {} to {}",
        shade.fill[3],
        shade.border[3]
    );
    // Over the picture, both as round as the card (1 in from its edge): no square corner pokes
    // out of it.
    let picture = c
        .shapes
        .iter()
        .find(|sh| card_picture(sh) && inside(sh.center, open, 0.0))
        .expect("the card's picture");
    assert_eq!(rect(picture), rect(shade), "the shade covers the picture");
    assert!(picture.params[0] > 0.0 && picture.params[0] == shade.params[0]);
    let [_, _, w, h] = rect(picture);
    assert!(
        w < open[2] && h < open[3],
        "inside the card, not a cover crop past it"
    );
}

#[test]
fn flying_through_songs_asks_for_no_pictures_until_the_list_settles() {
    let songs = Songs::new("select-look-hold", &many());
    let cfg = Config {
        main_menu: true,
        accent: ACCENT,
        animations: true,
        ..harness::config()
    };
    let mut entries = songs.entries();
    for e in &mut entries {
        e.meta.background = Some("bg.png".into());
    }
    let mut s = state_with(cfg, entries);
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    let asked = |c: &Canvas| {
        c.pictures_wanted
            .iter()
            .filter(|k| k.starts_with(render::PATH_PICTURE_PREFIX))
            .count()
    };
    let mut c = frame(&mut s, now);
    for _ in 0..60 {
        now += Duration::from_millis(16);
        c = frame(&mut s, now);
    }
    assert!(asked(&c) > 0, "the cards ask for their pictures");
    // A song every other frame, as Random spammed or an arrow held: once they come faster than
    // the list gets to them, nothing new is asked for.
    for k in 0..16 {
        if k % 2 == 0 {
            press(&mut s, KeyCode::ArrowRight, now);
        }
        now += Duration::from_millis(16);
        c = frame(&mut s, now);
        if k >= 2 {
            assert_eq!(
                asked(&c),
                0,
                "frame {k}: pictures asked while the list flies"
            );
        }
    }
    // Settled: they're asked for again.
    for _ in 0..30 {
        now += Duration::from_millis(16);
        c = frame(&mut s, now);
    }
    assert!(asked(&c) > 0, "once it settles the cards ask again");
}

#[test]
fn a_jump_reads_the_pictures_where_the_list_is_going_on_its_way() {
    let songs = Songs::new("select-look-ahead", &many());
    let cfg = Config {
        main_menu: true,
        accent: ACCENT,
        animations: true,
        ..harness::config()
    };
    let mut entries = songs.entries();
    for e in &mut entries {
        e.meta.background = Some("bg.png".into());
    }
    let mut s = state_with(cfg, entries);
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    for _ in 0..60 {
        now += Duration::from_millis(16);
        frame(&mut s, now);
    }
    // Far down the list, as Random goes: the set it lands on isn't on screen yet.
    let to = s.select.song_groups().len() - 3;
    let row = s.select.song_groups()[to].start;
    s.select.set_cursor(row);
    let key = |i: usize| {
        let e = &s.library[s.select.visible_index(i)];
        render::path_picture_key(&preview::background_path(e).expect("a picture"))
    };
    let (landing, next) = (key(row), key(s.select.song_groups()[to + 1].start));
    now += Duration::from_millis(16);
    let c = frame(&mut s, now);
    let drawn = cards(&c).iter().any(|(i, _)| *i == row);
    assert!(!drawn, "not on screen yet");
    let card = |k: &String| c.cards_wanted.iter().any(|w| w.0 == *k);
    assert!(
        card(&landing) && card(&next),
        "the cards of the set it lands on and its neighbours are asked for at once"
    );
    assert!(
        c.pictures_wanted.contains(&landing),
        "and the banner's picture of the set it lands on"
    );
}

/// Sixty sets, each naming a picture: screens of them.
fn many_sets(tag: &str) -> (State, Songs, Instant) {
    let charts: Vec<maps::Chart> = (0..60)
        .map(|i| maps::Chart {
            name: Box::leak(format!("set-{i:02}").into_boxed_str()),
            ..maps::basics()
        })
        .collect();
    menu_look_with(tag, &charts)
}

/// The picture key of the set whose first row is `row`.
fn set_key(s: &State, row: usize) -> String {
    let e = &s.library[s.select.row_map(row).expect("a set's map")];
    render::path_picture_key(&preview::background_path(e).expect("it names a picture"))
}

/// A card's picture drawn (`render::Canvas::card_picture`): its slot in `params[3]` below
/// -100000.
fn card_picture(s: &Shape) -> bool {
    s.params[3] <= -100_000.0
}

/// The cards wholly in the list's view (their hits not cut by its ends): first row and where.
fn cards_in_view(c: &Canvas, s: &State) -> Vec<(usize, [f32; 4])> {
    let [_, end] = s.select.frame_span();
    cards(c)
        .into_iter()
        .filter(|(_, r)| r[1] >= s.select.list_top() && r[1] + r[3] <= end && r[3] > 25.0)
        .collect()
}

/// Drags the list up by `by` units over `frames` frames, then wheels it on down a notch a
/// frame for `frames` more, calling `each` with every frame (drawn twice: a frame's hits come
/// back with the next one, and no time passes between the two, so nothing moves).
fn drag_then_wheel(
    s: &mut State,
    now: &mut Instant,
    by: f32,
    frames: usize,
    mut each: impl FnMut(&State, &Canvas, &str),
) {
    for k in 1..=frames {
        s.select.drag_to(-by * k as f32 / frames as f32);
        *now += Duration::from_millis(16);
        frame(s, *now);
        let c = frame(s, *now);
        each(s, &c, "dragging");
    }
    s.select.drag_end(&s.library);
    for _ in 0..frames {
        s.wheel(-1);
        *now += Duration::from_millis(16);
        frame(s, *now);
        let c = frame(s, *now);
        each(s, &c, "wheeling");
    }
}

#[test]
fn dragging_or_wheeling_the_list_draws_every_card_on_screen_with_its_picture() {
    let (mut s, _songs, mut now) = many_sets("select-look-fly-drawn");
    // Every strip in, as the read-ahead has them by the time their cards come into view.
    let groups: Vec<usize> = s.select.song_groups().iter().map(|g| g.start).collect();
    for &row in &groups {
        let key = set_key(&s, row);
        s.card_pictures.pretend_loaded(&key, [0.3, 0.4, 0.5]);
    }
    let c = settled(&mut s, &mut now);
    let first = cards_in_view(&c, &s)[0].0;
    let mut seen = std::collections::HashSet::new();
    drag_then_wheel(&mut s, &mut now, 600.0, 12, |s, c, how| {
        let shown = cards_in_view(c, s);
        assert!(shown.len() >= 5, "{how}: cards on screen");
        for (row, r) in shown {
            seen.insert(row);
            let pictured = c
                .shapes
                .iter()
                .any(|sh| card_picture(sh) && inside(sh.center, r, 0.5) && sh.fill[3] > 0.05);
            assert!(
                pictured,
                "{how}: the card of row {row} at {r:?} shows its picture"
            );
        }
    });
    assert!(
        seen.iter().any(|&r| r > first + 20),
        "the list went a long way down"
    );
}

#[test]
fn dragging_or_wheeling_the_list_reads_the_cards_ahead_of_it() {
    let (mut s, _songs, mut now) = many_sets("select-look-fly-ahead");
    settled(&mut s, &mut now);
    let rows: Vec<usize> = s.select.song_groups().iter().map(|g| g.start).collect();
    drag_then_wheel(&mut s, &mut now, 600.0, 12, |s, c, how| {
        let shown = cards_in_view(c, s);
        let wanted = |row: usize| c.cards_wanted.iter().find(|w| w.0 == set_key(s, row));
        for (row, _) in &shown {
            assert!(
                wanted(*row).is_some_and(|w| w.2),
                "{how}: the card of row {row} asks for its picture as it's drawn"
            );
        }
        // Not drawn yet: the sets below the screen (the way it goes) and fewer above it.
        let (top, bottom) = (shown[0].0, shown[shown.len() - 1].0);
        let ahead = rows
            .iter()
            .filter(|&&r| r > bottom && wanted(r).is_some_and(|w| !w.2))
            .count();
        let behind = rows
            .iter()
            .filter(|&&r| r < top && wanted(r).is_some_and(|w| !w.2))
            .count();
        let below = rows.iter().filter(|&&r| r > bottom).count();
        assert!(
            ahead >= shown.len().min(below),
            "{how}: at least a screen read ahead ({ahead} of {below})"
        );
        assert!(
            ahead >= behind.min(below),
            "{how}: more ahead than behind ({ahead}, {behind})"
        );
    });
}

#[test]
fn a_card_shows_its_pictures_colour_until_the_picture_is_in_then_fades_it_in() {
    let (mut s, _songs, mut now) = menu_look_with("select-look-card-colour", &many());
    s.cfg.animations = true;
    let c = settled(&mut s, &mut now);
    // The open set's card: not dimmed, so its picture's at full once in.
    let card = open_card(&c, &s);
    let (row, _) = cards_in_view(&c, &s)
        .into_iter()
        .find(|(_, r)| *r == card)
        .expect("the open card in view");
    let key = set_key(&s, row);
    let colour = [0.2, 0.6, 0.9];
    s.card_pictures.pretend_colour(&key, colour);
    let in_card = |c: &Canvas, f: &dyn Fn(&Shape) -> bool| -> Vec<Shape> {
        c.shapes
            .iter()
            .filter(|sh| inside(sh.center, card, 0.5) && f(sh))
            .copied()
            .collect()
    };
    let coloured = |sh: &Shape| plain(sh) && same_rgb(sh.fill, [0.2, 0.6, 0.9, 1.0]);
    now += Duration::from_millis(16);
    let c = frame(&mut s, now);
    assert_eq!(
        in_card(&c, &coloured).len(),
        1,
        "the picture's colour stands in for it"
    );
    assert!(in_card(&c, &card_picture).is_empty(), "no picture yet");
    // It comes in: over the colour, from nothing.
    s.card_pictures.pretend_loaded(&key, colour);
    let mut last = 0.0f32;
    let mut first = true;
    for _ in 0..8 {
        now += Duration::from_millis(16);
        let c = frame(&mut s, now);
        let pic = in_card(&c, &card_picture);
        assert_eq!(pic.len(), 1, "the picture drawn");
        let a = pic[0].fill[3];
        if first {
            assert!(a < 0.2, "it starts all but clear, not at once: {a}");
            first = false;
        }
        assert!(a >= last, "it only fades in: {last} then {a}");
        last = a;
        assert_eq!(
            in_card(&c, &coloured).len(),
            1,
            "the colour under it meanwhile"
        );
    }
    for _ in 0..20 {
        now += Duration::from_millis(16);
        frame(&mut s, now);
    }
    let c = frame(&mut s, now);
    let pic = in_card(&c, &card_picture);
    assert!(pic[0].fill[3] > 0.99, "in: at full");
    assert!(
        in_card(&c, &coloured).is_empty(),
        "the colour gone under it"
    );
}

/// Where the selected difficulty's row is (its hit).
fn target(c: &Canvas, s: &State) -> [f32; 4] {
    let cursor = s.select.cursor();
    hit_rect(c, |h| matches!(h, Hit::SongRow(i) if *i == cursor)).expect("the selected row")
}

/// `n` songs on, each settled.
fn songs_on(s: &mut State, now: &mut Instant, n: usize) -> Canvas {
    for _ in 0..n {
        press(s, KeyCode::ArrowRight, *now);
        settled(s, now);
    }
    settled(s, now)
}

#[test]
fn the_selected_song_sits_in_the_target_at_one_height() {
    let (mut s, _songs, mut now) = menu_look_with("select-look-target", &many());
    let at = |c: &Canvas, s: &State| target(c, s)[1];
    // At the top of the list it sits at the top: the list doesn't scroll past its end.
    let c = settled(&mut s, &mut now);
    let first = at(&c, &s);
    let c = songs_on(&mut s, &mut now, 6);
    let (cursor, y) = (s.select.cursor(), at(&c, &s));
    assert!(
        y > first + 40.0,
        "down in the list, not at its top: {first} then {y}"
    );
    for n in [1, 3] {
        let c = songs_on(&mut s, &mut now, n);
        assert_ne!(s.select.cursor(), cursor, "another song");
        assert_eq!(at(&c, &s), y, "the selected song in the same place");
    }
}

#[test]
fn cards_bend_away_from_the_target_and_fade() {
    let (mut s, _songs, mut now) = menu_look_with("select-look-arc", &many());
    let c = songs_on(&mut s, &mut now, 6);
    let t = target(&c, &s);
    let ty = t[1] + t[3] / 2.0;
    let [_, end] = s.select.frame_span();
    let open = open_card(&c, &s);
    // The closed sets in full view (not under the top's edge nor the bottom's fade), nearest
    // to the target first.
    let mut closed: Vec<(f32, [f32; 4])> = cards(&c)
        .into_iter()
        .map(|(_, r)| r)
        .filter(|r| *r != open && r[1] > s.select.list_top() && r[1] + r[3] < end - 24.0)
        .map(|r| ((r[1] + r[3] / 2.0 - ty).abs(), r))
        .collect();
    closed.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert!(closed.len() >= 4, "cards round the target: {closed:?}");
    assert!(
        closed.windows(2).all(|w| w[1].1[0] >= w[0].1[0]),
        "farther, bent away farther: {closed:?}"
    );
    let (near, far) = (closed[0].1, closed[closed.len() - 1].1);
    assert!(
        far[0] - near[0] > 3.0,
        "an arc, not a line: {near:?} {far:?}"
    );
    let strongest = |r: [f32; 4]| {
        c.shapes
            .iter()
            .filter(|sh| plain(sh) && inside(sh.center, r, 0.0))
            .map(|sh| sh.fill[3])
            .fold(0.0, f32::max)
    };
    assert!(
        strongest(far) < strongest(near) - 0.02,
        "farther, fainter: {} near, {} far",
        strongest(near),
        strongest(far)
    );
}

/// The target's flash: a soft white fill across the selected row.
fn flashes(c: &Canvas, s: &State) -> bool {
    let lw = ui::Select::list_width(c.width, s.cfg.song_list_share);
    c.shapes.iter().any(|sh| {
        plain(sh)
            && same_rgb(sh.fill, render::rgba(0xffffff, 1.0))
            && sh.fill[3] > 0.02
            && sh.fill[3] < 0.3
            && sh.half[0] * 2.0 > lw / 2.0
    })
}

#[test]
fn a_song_landing_in_the_target_flashes_it() {
    for animations in [true, false] {
        let songs = Songs::new("select-look-flash", &many());
        let cfg = Config {
            main_menu: true,
            accent: ACCENT,
            animations,
            ..harness::config()
        };
        let mut s = state_with(cfg, songs.entries());
        let mut now = Instant::now();
        ready(&mut s, &mut now);
        // Settled first (what was selected on opening has landed and its flash is over).
        for _ in 0..120 {
            now += Duration::from_millis(16);
            frame(&mut s, now);
        }
        let mut seen = Vec::new();
        for _ in 0..20 {
            now += Duration::from_millis(16);
            seen.push(flashes(&frame(&mut s, now), &s));
        }
        assert!(!seen.contains(&true), "nothing landed yet");
        press(&mut s, KeyCode::ArrowRight, now);
        let mut seen = Vec::new();
        for _ in 0..80 {
            now += Duration::from_millis(16);
            seen.push(flashes(&frame(&mut s, now), &s));
        }
        if animations {
            let first = seen.iter().position(|f| *f).expect("a flash once it lands");
            assert!(first > 3, "when it lands, not as it leaves: frame {first}");
            assert!(!seen[60..].contains(&true), "over in a moment");
        } else {
            assert!(!seen.contains(&true), "without animations a cut, no flash");
        }
    }
}

/// Where a text starting with `s` is drawn: its first letter's top left.
fn text_at(c: &Canvas, s: &str) -> Option<[f32; 2]> {
    c.texts
        .iter()
        .find(|(_, t)| t.starts_with(s))
        .and_then(|(r, _)| c.shapes.get(r.start))
        .map(|sh| {
            let r = rect(sh);
            [r[0], r[1]]
        })
}

#[test]
fn the_preview_lane_has_no_lines_round_it_and_its_corners_round() {
    let (mut s, _songs, mut now) = menu_look("select-look-lane");
    let c = settled(&mut s, &mut now);
    let lw = ui::Select::list_width(c.width, s.cfg.song_list_share);
    let right_of_list = |sh: &&Shape| plain(sh) && sh.center[0] > 28.0 + lw;
    let lane_hit = hit_rect(&c, |h| *h == Hit::Preview).expect("the lane");
    // The strip: a tile as the stats' are (not a dark box), as round as them.
    let fill = style::surface_fill(style::Surface::Raised);
    let strips: Vec<&Shape> = c
        .shapes
        .iter()
        .filter(right_of_list)
        .filter(|sh| {
            same_rgb(sh.fill, fill) && sh.half[0] * 2.0 > 100.0 && inside(sh.center, lane_hit, 1.0)
        })
        .collect();
    let found: Vec<[f32; 4]> = strips.iter().map(|sh| rect(sh)).collect();
    assert_eq!(strips.len(), 1, "one strip: {found:?}");
    let strip = strips[0];
    assert_eq!(strip.params[0], style::radius::SMALL, "its corners round");
    assert_eq!(strip.params[1], 0.0, "no edge round it");
    let [left, y, w, h] = rect(strip);
    let right = left + w;
    // As far out past the lane's end as before its drum, so it's round at both ends (the lane
    // itself is cut square to its box).
    let (before, after) = (lane_hit[0] - left, right - (lane_hit[0] + lane_hit[2]));
    assert!(
        before > 4.0 && (before - after).abs() < 0.5,
        "{before} before the lane, {after} after it"
    );
    // No lines along it or across its ends.
    let edge = render::rgba(style::PANEL_EDGE, 1.0);
    let lines: Vec<[f32; 4]> = c
        .shapes
        .iter()
        .filter(right_of_list)
        .filter(|sh| same_rgb(sh.fill, edge))
        .map(rect)
        .filter(|r| r[1] < y + h + 2.0 && r[1] + r[3] > y - 2.0)
        .filter(|r| r[0] < right + 2.0 && r[0] + r[2] > left - 2.0)
        .collect();
    assert!(lines.is_empty(), "lines round the lane: {lines:?}");
}

#[test]
fn the_timeline_is_quiet_and_its_kiai_in_gold() {
    let (mut s, _songs, mut now) = menu_look("select-look-timeline");
    let path = s
        .library
        .iter()
        .find(|e| e.meta.title == "sv-and-kiai")
        .map(|e| e.path.clone())
        .expect("a map with kiai");
    s.select.select_path(&s.library, &path);
    let c = settled(&mut s, &mut now);
    let lw = ui::Select::list_width(c.width, s.cfg.song_list_share);
    let [_, ty] = text_at(&c, "0:00 / ").expect("the timeline's time");
    // The bars: thin shapes over the timeline, its 28 units up from the time under it.
    let (top, bottom) = (ty - 5.0 - 30.0, ty - 5.0);
    let green = render::rgba(0x86E05A, 1.0);
    let in_text = |i: usize| c.texts.iter().any(|(r, _)| r.contains(&i));
    let bars: Vec<&Shape> = c
        .shapes
        .iter()
        .enumerate()
        .filter(|(i, _)| !in_text(*i))
        .map(|(_, sh)| sh)
        .filter(|sh| plain(sh) && sh.center[0] > 28.0 + lw)
        .filter(|sh| sh.half[0] * 2.0 < 3.0 && sh.center[1] > top && sh.center[1] < bottom)
        .filter(|sh| !same_rgb(sh.fill, green))
        .collect();
    assert!(bars.len() > 20, "a timeline of bars: {}", bars.len());
    // The playhead a plain line among the bars: nothing glows round it.
    let glows: Vec<[f32; 4]> = c
        .shapes
        .iter()
        .filter(|sh| sh.center[0] > 28.0 + lw && sh.center[1] > top && sh.center[1] < bottom)
        .filter(|sh| sh.params[1] == 0.0 && sh.params[2] > 0.0 && sh.half[0] == sh.params[0])
        .map(rect)
        .collect();
    assert!(glows.is_empty(), "a glow on the timeline: {glows:?}");
    let (gold, ahead, played) = (
        render::rgba(0xFFD34D, 1.0),
        render::rgba(style::ink::SECONDARY, 1.0),
        render::rgba(style::ink::PRIMARY, 1.0),
    );
    for sh in &bars {
        assert!(
            [gold, ahead, played].iter().any(|c| same_rgb(sh.fill, *c)),
            "a bar in a loud colour: {:?}",
            sh.fill
        );
        if same_rgb(sh.fill, ahead) {
            assert!(
                sh.fill[3] <= 0.36,
                "ahead of the playhead, quiet: {}",
                sh.fill[3]
            );
        }
    }
    assert!(
        bars.iter().any(|sh| same_rgb(sh.fill, gold)),
        "its kiai in gold"
    );
}

/// The texts drawn inside `r`.
fn texts_in(c: &Canvas, [x, y, w, h]: [f32; 4]) -> Vec<String> {
    c.texts
        .iter()
        .filter(|(range, _)| {
            c.shapes.get(range.start).is_some_and(|sh| {
                let [sx, sy, ..] = rect(sh);
                sx >= x - 0.5 && sx <= x + w && sy >= y - 0.5 && sy <= y + h
            })
        })
        .map(|(_, t)| t.clone())
        .collect()
}

/// A local play on the selected map.
fn local_play(s: &State, player: &str, score: u64, accuracy: f64) -> crate::scores::ScoreRow {
    let i = s.select.selected().expect("a map selected");
    crate::scores::ScoreRow {
        map_hash: s.library[i].hash(),
        date: 1_700_000_000 + score,
        score,
        accuracy,
        max_combo: 100,
        miss: 0,
        rate: 1.0,
        player: player.into(),
        file: std::path::PathBuf::new(),
        mods: String::new(),
        pp: None,
    }
}

fn animated(tag: &str) -> (State, Songs, Instant) {
    let (mut s, songs, now) = menu_look(tag);
    s.cfg.animations = true;
    (s, songs, now)
}

#[test]
fn play_is_the_menus_play_note_and_its_map_starts_at_once() {
    let (mut s, _songs, mut now) = animated("select-look-play-note");
    let c = settled(&mut s, &mut now);
    let play = hit_rect(&c, |h| matches!(h, Hit::Key(KeyCode::Enter))).expect("Play");
    let (at, r) = s.play_icon_at.expect("where the menu's note lands");
    assert!(inside(at, play, 0.0) && r > 10.0, "{at:?} {r} in {play:?}");
    // The note itself: its white rim round where the flying note lands.
    let rim = c
        .shapes
        .iter()
        .filter(|sh| same_rgb(sh.fill, render::rgba(0xffffff, 1.0)) && inside(sh.center, play, 0.0))
        .filter(|sh| (sh.center[0] - at[0]).hypot(sh.center[1] - at[1]) <= r * 1.2)
        .count();
    assert!(rim > 20, "the Play note's pixels: {rim}");
    // With animations on nothing plays first: the map starts as the frame after the press is
    // drawn, and that frame is kept to fade out over it (instead of a cut to "Loading").
    press(&mut s, KeyCode::Enter, now);
    now += Duration::from_millis(16);
    frame(&mut s, now);
    assert_eq!(scene_name(&s), "Loading", "the map at the next frame");
    assert!(s.ghost_keep, "song select's last frame kept");
    assert_eq!(
        s.select_ghost_alpha(now),
        1.0,
        "over the map while it loads"
    );
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    let start = now;
    assert_eq!(s.select_ghost_alpha(start), 1.0, "the play starts under it");
    let half = s.select_ghost_alpha(start + Duration::from_millis(125));
    assert!(half > 0.2 && half < 0.8, "fading out: {half}");
    assert_eq!(
        s.select_ghost_alpha(start + Duration::from_millis(260)),
        0.0,
        "gone a quarter of a second in"
    );
    assert!(s.select_ghost.is_none());
}

#[test]
fn without_animations_play_starts_the_map_at_once() {
    let (mut s, _songs, mut now) = menu_look("select-look-play-cut");
    settled(&mut s, &mut now);
    press(&mut s, KeyCode::Enter, now);
    assert_eq!(scene_name(&s), "Loading");
}

#[test]
fn mods_and_random_keep_their_place_and_the_mods_on_show_as_badges_under_them() {
    let (mut s, _songs, mut now) = menu_look("select-look-mod-badges");
    let c = settled(&mut s, &mut now);
    let mods = |c: &Canvas| hit_rect(c, |h| *h == Hit::ModsButton).expect("Mods");
    let random = |c: &Canvas| hit_rect(c, |h| matches!(h, Hit::Key(KeyCode::F2))).expect("Random");
    let play = hit_rect(&c, |h| matches!(h, Hit::Key(KeyCode::Enter))).expect("Play");
    let fill = style::surface_fill(style::Surface::Raised);
    let lane = c
        .shapes
        .iter()
        .filter(|sh| same_rgb(sh.fill, fill) && sh.params[0] == style::radius::CARD)
        .map(rect)
        .find(|r| inside([play[0] + 1.0, play[1] + play[3] / 2.0], *r, 0.0))
        .expect("the lane of actions");
    // The two as one group in the middle of the lane left of Play: what's drawn of them (a
    // button's hit has 6 units of slack each side) as far from the lane's start as from Play.
    let (m, r) = (mods(&c), random(&c));
    let before = m[0] + 6.0 - lane[0];
    let after = play[0] - (r[0] + r[2] - 6.0);
    assert!(
        before > 16.0 && (before - after).abs() < 1.0,
        "{before} before Mods, {after} from Random to Play"
    );
    let [_, wy] = text_at(&c, "Mods").expect("the word");
    // Mods on: the two stay where they were across, a little higher, the badges under them.
    s.cfg.mods = crate::mods::Mods::parse("HD,HR");
    s.select.mods_label = s.cfg.mods.label();
    let c = settled(&mut s, &mut now);
    let (m2, r2) = (mods(&c), random(&c));
    assert!(
        (m2[0] - m[0]).abs() < 0.01 && (r2[0] - r[0]).abs() < 0.01,
        "moved across: {m:?} -> {m2:?}, {r:?} -> {r2:?}"
    );
    assert!(m2[1] < m[1] && (m2[1] - r2[1]).abs() < 0.01, "up, together");
    let [_, wy2] = text_at(&c, "Mods").expect("the word");
    assert!(wy2 < wy, "the word moves up with them");
    for name in ["HD", "HR"] {
        let [x, y] = text_at(&c, name).unwrap_or_else(|| panic!("{name}"));
        assert!(
            y > wy2 + 8.0 && x >= m2[0] && x < play[0],
            "{name} under the group: {x}, {y}"
        );
        assert!(y < lane[1] + lane[3] - 4.0, "{name} inside the lane: {y}");
        let badge = c.shapes.iter().any(|sh| {
            let [bx, by, bw, bh] = rect(sh);
            sh.params[1] > 0.0 && bx < x && bx + bw > x && by < y + 3.0 && by + bh > y + 3.0
        });
        assert!(badge, "{name} on a badge");
        let opens = c
            .hits
            .iter()
            .any(|b| b.hit == Hit::ModsButton && inside([x + 2.0, y + 3.0], b.rect, 0.0));
        assert!(opens, "a click on {name} opens the mod select");
    }
}

#[test]
fn randoms_die_rolls_when_pressed_and_rests_on_five() {
    let (mut s, _songs, mut now) = animated("select-look-dice");
    let die = |c: &Canvas| {
        let r = hit_rect(c, |h| matches!(h, Hit::Key(KeyCode::F2))).expect("Random");
        let mut shapes: Vec<[i32; 4]> = c
            .shapes
            .iter()
            .filter(|sh| inside(sh.center, [r[0], r[1], 24.0, r[3]], 0.0))
            .map(|sh| rect(sh).map(|v| (v * 4.0).round() as i32))
            .collect();
        shapes.sort_unstable();
        shapes
    };
    let rest = die(&settled(&mut s, &mut now));
    press(&mut s, KeyCode::F2, now);
    now += Duration::from_millis(16);
    let rolling = die(&frame(&mut s, now));
    assert_ne!(rolling, rest, "another face while it rolls");
    let rolled = die(&settled(&mut s, &mut now));
    assert_eq!(rolled, rest, "back on five");
}

#[test]
fn nothing_in_the_details_stretches_on_the_way_to_the_leaderboard() {
    let (mut s, _songs, mut now) = menu_look("select-look-board-slide");
    s.cfg.animations = true;
    let c = settled(&mut s, &mut now);
    let preview = |c: &Canvas| hit_rect(c, |h| *h == Hit::Preview).expect("the lane");
    let lane = preview(&c);
    s.mob.flip_board();
    let mut on_the_way = 0;
    for _ in 0..12 {
        now += Duration::from_millis(8);
        let c = frame(&mut s, now);
        // Gone once the board is all there; at its place before it sets off.
        let Some(l) = hit_rect(&c, |h| *h == Hit::Preview) else {
            continue;
        };
        let t = lane[0] - l[0];
        if t < 2.0 {
            continue;
        }
        on_the_way += 1;
        let shown = |r: [f32; 4]| r[0] + r[2];
        assert!(
            (l[2] - lane[2]).abs() < 0.5 || shown(l) >= c.width - 0.5,
            "the lane as long on the way ({t}): {} vs {}",
            l[2],
            lane[2]
        );
    }
    assert!(on_the_way > 0, "caught on the way");
}

#[test]
fn the_search_row_has_a_glass_a_quiet_hint_its_key_on_a_cap_and_the_counts_on_a_chip() {
    let (mut s, _songs, mut now) = menu_look("select-look-search-row");
    s.cfg.select_info = true;
    let c = settled(&mut s, &mut now);
    let field = hit_rect(&c, |h| matches!(h, Hit::TextField)).expect("the search box");
    let hint = text_at(&c, "click to search").expect("the hint");
    assert!(
        c.texts.iter().all(|(_, t)| !t.contains("(ctrl+f)")),
        "the key isn't in the hint"
    );
    // A magnifying glass before the hint: the icon's pixels left of it, inside the box.
    let glass = c.shapes.iter().any(|sh| {
        plain(sh)
            && same_rgb(sh.fill, render::rgba(style::ink::CAPTION, 1.0))
            && inside(
                sh.center,
                [field[0], field[1], hint[0] - field[0], field[3]],
                0.0,
            )
    });
    assert!(glass, "a glass at the box's start");
    let key = text_at(&c, "ctrl+f").expect("the key on its cap");
    assert!(
        key[0] > field[0] + field[2] * 0.6 && key[0] < field[0] + field[2],
        "at the box's right end: {key:?} in {field:?}"
    );
    let cap = c.shapes.iter().any(|sh| {
        same_rgb(sh.fill, style::surface_fill(style::Surface::Raised))
            && inside(key.map(|v| v + 1.0), rect(sh), 0.0)
            && rect(sh)[2] < 40.0
    });
    assert!(cap, "the key sits on a cap");
    // The counts on a chip of their own beside the search row, not floating at the screen's
    // right edge.
    let counts = c
        .texts
        .iter()
        .find(|(_, t)| t.contains(" songs, "))
        .and_then(|(r, _)| c.shapes.get(r.start))
        .map(rect)
        .expect("the counts");
    let lw = ui::Select::list_width(c.width, s.cfg.song_list_share);
    assert!(
        counts[0] > 24.0 + lw && counts[0] < 24.0 + lw + 30.0,
        "beside the list: {counts:?}"
    );
    let chip = c.shapes.iter().any(|sh| {
        same_rgb(sh.fill, style::surface_fill(style::Surface::Raised))
            && inside([counts[0] + 1.0, counts[1] + 1.0], rect(sh), 0.0)
            && rect(sh)[0] < counts[0]
    });
    assert!(chip, "on a chip");
}

#[test]
fn the_map_s_picture_behind_song_select_is_blurred_to_a_field_of_colour() {
    let (mut s, _songs, mut now) = menu_look("select-look-blur");
    settled(&mut s, &mut now);
    assert_eq!(s.background_blur(), 1.0, "the main menu's look");
    // Legacy keeps the picture sharp.
    let songs = Songs::new("select-look-blur-legacy", &maps::all());
    let mut legacy = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut legacy, &mut now);
    assert!(legacy.select.legacy);
    assert_eq!(legacy.background_blur(), 0.0);
}

#[test]
fn play_and_the_lane_of_actions_keep_the_screen_s_margin() {
    let (mut s, _songs, mut now) = menu_look("select-look-play-margin");
    let c = settled(&mut s, &mut now);
    let play = hit_rect(&c, |h| *h == Hit::Key(KeyCode::Enter)).expect("Play");
    let margin = c.width - (play[0] + play[2]);
    assert!(
        (margin - 24.0).abs() < 0.5,
        "Play ends 24 in from the right, as the panel above it: {margin}"
    );
    // And the lane under Mods and Random ends with it, both ends round.
    let lane = c
        .shapes
        .iter()
        .filter(|sh| plain(sh) && same_rgb(sh.fill, style::surface_fill(style::Surface::Raised)))
        .map(|sh| (rect(sh), sh.params[0]))
        .find(|(r, _)| (r[1] - play[1]).abs() < 0.5 && (r[3] - play[3]).abs() < 0.5)
        .expect("the lane of actions");
    assert!(
        (lane.0[0] + lane.0[2] - (play[0] + play[2])).abs() < 0.5,
        "{lane:?}"
    );
    assert_eq!(lane.1, style::radius::CARD);
}

#[test]
fn the_details_sit_on_a_panel_of_their_own_and_nothing_of_them_runs_off_it() {
    let (mut s, _songs, mut now) = menu_look("select-look-right-panel");
    let c = settled(&mut s, &mut now);
    let lw = ui::Select::list_width(c.width, s.cfg.song_list_share);
    let raised = style::surface_fill(style::Surface::Raised);
    // A raised surface, as round as the cards, from beside the list to the screen's margin.
    let panel = c
        .shapes
        .iter()
        .filter(|sh| plain(sh) && same_rgb(sh.fill, raised) && sh.params[0] == style::radius::CARD)
        .map(rect)
        .find(|r| r[0] > 24.0 + lw && r[3] > 200.0)
        .expect("the details' panel");
    assert!(
        (panel[0] + panel[2] - (c.width - 24.0)).abs() < 0.5,
        "{panel:?}"
    );
    // The preview's lane ends inside it.
    let lane = hit_rect(&c, |h| *h == Hit::Preview).expect("the preview lane");
    assert!(
        lane[0] + lane[2] <= panel[0] + panel[2],
        "{lane:?} in {panel:?}"
    );
}

#[test]
fn the_details_leave_the_plays_to_the_leaderboard_s_page() {
    let (mut s, _songs, mut now) = menu_look("select-look-no-strip");
    for (k, name) in ["Ann", "Bo"].iter().enumerate() {
        let play = local_play(&s, name, 900_000 - k as u64 * 1000, 0.99);
        s.scores.push(play);
    }
    let c = settled(&mut s, &mut now);
    let shown = texts_in(&c, [0.0, 0.0, c.width, crate::render::VIRTUAL_HEIGHT]);
    assert!(
        !shown.iter().any(|t| t == "Ann" || t == "Bo"),
        "no strip of the board under the timeline: {shown:?}"
    );
    assert!(
        hit_rect(&c, |h| *h == Hit::BoardPage).is_some(),
        "the way to the board"
    );
}

#[test]
fn a_map_whose_bpm_changes_shows_its_whole_range() {
    let chart = maps::Chart {
        name: "bpm-range",
        timing: vec![
            "1000,600,4,1,0,100,1,0".into(),
            "5000,240,4,1,0,100,1,0".into(),
        ],
        ..maps::basics()
    };
    let (mut s, _songs, mut now) = menu_look_with("select-look-bpm-range", &[chart]);
    let c = settled(&mut s, &mut now);
    let shown = texts_in(&c, [0.0, 0.0, c.width, crate::render::VIRTUAL_HEIGHT]);
    assert!(
        shown.iter().any(|t| t == "100-250"),
        "the range whole: {shown:?}"
    );
    assert!(
        !shown
            .iter()
            .any(|t| t.starts_with('1') && t.ends_with("...")),
        "nothing cut: {shown:?}"
    );
}

#[test]
fn the_library_counts_and_the_device_line_wait_for_their_setting() {
    let (mut s, _songs, mut now) = menu_look("select-look-info");
    let shown = |s: &mut State, now: &mut Instant| {
        let c = settled(s, now);
        texts_in(&c, [0.0, 0.0, c.width, crate::render::VIRTUAL_HEIGHT])
    };
    s.status = "gpu  |  audio  |  60 Hz".into();
    let off = shown(&mut s, &mut now);
    assert!(
        !off.iter()
            .any(|t| t.contains(" songs, ") || t.contains("60 Hz")),
        "off by default: {off:?}"
    );
    s.cfg.select_info = true;
    let on = shown(&mut s, &mut now);
    assert!(
        on.iter().any(|t| t.contains(" songs, ")),
        "the counts: {on:?}"
    );
    assert!(
        on.iter().any(|t| t.contains("60 Hz")),
        "the device line: {on:?}"
    );
}

#[test]
fn the_full_combo_pp_is_one_button_and_the_way_to_the_board_ends_its_row() {
    let (mut s, _songs, mut now) = menu_look("select-look-pp-row");
    let c = settled(&mut s, &mut now);
    let pp = hit_rect(&c, |h| *h == Hit::PpSim).expect("the pp button");
    let way = hit_rect(&c, |h| *h == Hit::BoardPage).expect("the way to the board");
    assert!(
        (way[1] + way[3] / 2.0 - (pp[1] + pp[3] / 2.0)).abs() < 2.5,
        "one row: {pp:?} {way:?}"
    );
    assert!(
        way[0] > pp[0] + pp[2],
        "the board's way after it: {pp:?} {way:?}"
    );
    let lifted = |r: [f32; 4]| {
        c.shapes.iter().any(|sh| {
            same_rgb(sh.fill, style::surface_fill(style::Surface::Lifted))
                && sh.params[0] == style::radius::SMALL
                && inside([r[0] + r[2] / 2.0, r[1] + r[3] / 2.0], rect(sh), 0.0)
        })
    };
    assert!(lifted(pp) && lifted(way), "both on chips alike");
    let shown = texts_in(&c, pp);
    for want in ["SS", "99%", "98%"] {
        assert!(shown.iter().any(|t| t == want), "{want} on it: {shown:?}");
    }
    assert!(
        !texts_in(&c, [0.0, 0.0, c.width, crate::render::VIRTUAL_HEIGHT])
            .iter()
            .any(|t| t.starts_with("click / drag")),
        "no hint under the timeline"
    );
}

#[test]
fn the_pp_button_opens_a_simulator_of_accuracy_and_misses() {
    let (mut s, _songs, mut now) = menu_look("select-look-pp-sim");
    settled(&mut s, &mut now);
    s.click(Hit::PpSim, None);
    let c = settled(&mut s, &mut now);
    let button = hit_rect(&c, |h| *h == Hit::PpSim).expect("the simulator");
    // The button and the row of the play under it.
    let steps = hit_rect(&c, |h| *h == Hit::PpSimStep(1, 1)).expect("a step");
    let pp = [
        button[0],
        button[1],
        steps[0] + steps[2] - button[0],
        steps[1] + steps[3] - button[1],
    ];
    assert!(
        steps[1] > button[1] + button[3],
        "the play on a row under the button"
    );
    let shown = texts_in(&c, pp);
    assert!(shown.iter().any(|t| t == "98.00%"), "at 98%: {shown:?}");
    let at_98 = s.preview.sim_pp.expect("its pp");
    assert!(
        shown.iter().any(|t| *t == format!("{at_98:.0}pp")),
        "{shown:?}"
    );
    let full = s.preview.potential.expect("the full-combo pp");
    assert!(
        (at_98 - full[2]).abs() < 1.0,
        "as the 98% chip: {at_98} vs {}",
        full[2]
    );
    // A miss and less accuracy give less.
    for hit in [
        Hit::PpSimStep(1, 1),
        Hit::PpSimStep(0, -1),
        Hit::PpSimStep(0, -1),
    ] {
        assert!(hit_rect(&c, |h| *h == hit).is_some(), "{hit:?} drawn");
        s.click(hit, None);
    }
    let c = settled(&mut s, &mut now);
    let sim = s.preview.sim.expect("still open");
    assert_eq!(sim.misses, 1);
    assert!((sim.accuracy - 0.97).abs() < 1e-9, "{}", sim.accuracy);
    let lower = s.preview.sim_pp.expect("its pp");
    assert!(lower < at_98, "{lower} under {at_98}");
    assert!(texts_in(&c, pp).iter().any(|t| t == "97.00%"));
    s.click(Hit::PpSim, None);
    let c = settled(&mut s, &mut now);
    assert!(s.preview.sim.is_none(), "closed again");
    assert!(
        texts_in(&c, pp).iter().any(|t| t == "SS"),
        "the full-combo chips back"
    );
}

#[test]
fn the_list_s_grip_sits_in_the_gap_between_the_cards_and_the_details_panel() {
    let (mut s, _songs, mut now) = menu_look("select-look-grip");
    let c = settled(&mut s, &mut now);
    let grip = hit_rect(&c, |h| *h == Hit::SplitList).expect("the grip");
    let cards_right = cards(&c)
        .iter()
        .map(|(_, r)| r[0] + r[2])
        .fold(0.0, f32::max);
    let lw = ui::Select::list_width(c.width, s.cfg.song_list_share);
    let raised = style::surface_fill(style::Surface::Raised);
    let panel_left = c
        .shapes
        .iter()
        .filter(|sh| plain(sh) && same_rgb(sh.fill, raised) && sh.params[0] == style::radius::CARD)
        .map(rect)
        .filter(|r| r[0] > 24.0 + lw && r[3] > 200.0)
        .map(|r| r[0])
        .fold(f32::MAX, f32::min);
    let mid = grip[0] + grip[2] / 2.0;
    assert!(
        mid > cards_right && mid < panel_left,
        "between {cards_right} and {panel_left}: {grip:?}"
    );
    assert!(
        (mid - (cards_right + panel_left) / 2.0).abs() < 2.0,
        "in the gap's middle: {mid} of {cards_right}..{panel_left}"
    );
}

/// Every text drawn and its `[x, y, w, h]` (from its letters).
fn text_boxes(c: &Canvas) -> Vec<(String, [f32; 4])> {
    c.texts
        .iter()
        .filter(|(r, _)| r.start < r.end)
        .map(|(r, t)| {
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for sh in &c.shapes[r.clone()] {
                let [x, y, w, h] = rect(sh);
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + w), y1.max(y + h));
            }
            (t.trim().to_string(), [x0, y0, x1 - x0, y1 - y0])
        })
        .collect()
}

#[test]
fn the_pp_button_shows_every_number_whole_and_its_simulator_overlaps_nothing() {
    let (mut s, _songs, mut now) = menu_look("select-look-pp-whole");
    let c = settled(&mut s, &mut now);
    let button = hit_rect(&c, |h| *h == Hit::PpSim).expect("the pp button");
    let full = s.preview.potential.expect("the full-combo pp");
    let shown: Vec<(String, [f32; 4])> = text_boxes(&c)
        .into_iter()
        .filter(|(_, r)| inside([r[0] + r[2] / 2.0, r[1] + r[3] / 2.0], button, 0.0))
        .collect();
    for pp in full {
        let whole = format!("{pp:.0}");
        assert!(
            shown
                .iter()
                .any(|(t, _)| *t == whole || *t == format!("{whole}pp")),
            "{whole} shown whole: {shown:?}"
        );
    }
    for (t, r) in &shown {
        assert!(!t.contains("..."), "nothing cut: {t}");
        assert!(
            r[0] + r[2] <= button[0] + button[2] + 0.5,
            "{t} inside the button"
        );
    }

    // Open: the button reads the play and its pp; the play's fields are on a row of their own
    // under it, and nothing on the two rows overlaps.
    s.click(Hit::PpSim, None);
    let c = settled(&mut s, &mut now);
    let steps: Vec<[f32; 4]> = c
        .hits
        .iter()
        .filter(|h| matches!(h.hit, Hit::PpSimStep(..)))
        .map(|h| h.rect)
        .collect();
    assert_eq!(steps.len(), 4, "- and + for accuracy and misses");
    let row = steps[0][1];
    assert!(row >= button[1] + button[3], "under the button: {row}");
    let area = [button[0], button[1], c.width, row + steps[0][3] - button[1]];
    let mut boxes: Vec<(String, [f32; 4])> = text_boxes(&c)
        .into_iter()
        .filter(|(_, r)| inside([r[0] + r[2] / 2.0, r[1] + r[3] / 2.0], area, 0.0))
        .collect();
    let pp = format!("{:.0}pp", s.preview.sim_pp.expect("its pp"));
    assert!(boxes.iter().any(|(t, _)| *t == pp), "{pp}: {boxes:?}");
    assert!(
        boxes.iter().any(|(t, _)| t.starts_with("98.00%")),
        "{boxes:?}"
    );
    boxes.extend(steps.iter().map(|r| ("step".to_string(), *r)));
    for (i, (a, ra)) in boxes.iter().enumerate() {
        for (b, rb) in &boxes[i + 1..] {
            let apart = ra[0] + ra[2] <= rb[0] + 0.5
                || rb[0] + rb[2] <= ra[0] + 0.5
                || ra[1] + ra[3] <= rb[1] + 0.5
                || rb[1] + rb[3] <= ra[1] + 0.5;
            // A step's own mark is inside it.
            let own =
                (a == "step") != (b == "step") && (a == "-" || a == "+" || b == "-" || b == "+");
            assert!(apart || own, "{a} {ra:?} overlaps {b} {rb:?}");
        }
    }
    assert!(!boxes.iter().any(|(t, _)| t.contains("...")), "{boxes:?}");
}

#[test]
fn song_select_s_routes_are_plain_shapes_with_no_frosted_glass_over_them() {
    // #643 tagged the routes as frosted (`params[3]` 5) and softened them under panes of
    // glass; that look was dropped. The routes are drawn as on the menu, sharp everywhere.
    let (mut s, _songs, mut now) = menu_look("select-look-no-glass");
    let c = settled(&mut s, &mut now);
    let frosted = c
        .shapes
        .iter()
        .filter(|sh| sh.params[3] > 4.5 && sh.params[3] < 5.5)
        .count();
    assert_eq!(frosted, 0, "no shape carries the old frost tag");
    assert!(
        c.shapes.iter().any(|sh| sh.params[3] == 0.0),
        "plain shapes are drawn"
    );
}

#[test]
fn a_song_picked_on_its_own_sends_the_old_details_out_to_the_left() {
    let (mut s, _songs, mut now) = menu_look_moving("select-look-leave");
    let before = settled(&mut s, &mut now);
    let title = s.library[s.select.selected().unwrap()].meta.title.clone();
    let lw = ui::Select::list_width(before.width, s.cfg.song_list_share);
    let (range, _) = before
        .texts
        .iter()
        .find(|(r, t)| t.trim() == title && before.shapes[r.start].center[0] > 24.0 + lw)
        .cloned()
        .expect("the title in the details");
    let letter = before.shapes[range.start];
    press(&mut s, KeyCode::ArrowRight, now);
    assert_ne!(
        s.library[s.select.selected().unwrap()].meta.title,
        title,
        "another song"
    );
    // The old title's first letter, a little left and fainter, for a moment.
    let gone = |c: &Canvas| {
        c.shapes.iter().any(|sh| {
            sh.half == letter.half
                && sh.params == letter.params
                && (sh.center[1] - letter.center[1]).abs() < 0.01
                && sh.center[0] < letter.center[0] - 0.01
                && sh.center[0] > letter.center[0] - 6.5
                && sh.fill[3] < letter.fill[3]
        })
    };
    // The frame that sees the change starts the way out; a little later it's on it.
    now += Duration::from_millis(16);
    frame(&mut s, now);
    now += Duration::from_millis(32);
    let going = frame(&mut s, now);
    assert!(gone(&going), "the old details going out");
    now += Duration::from_millis(150);
    let later = frame(&mut s, now);
    assert!(!gone(&later), "gone in a moment");
}

#[test]
fn the_details_and_the_picture_behind_lean_with_the_pointer_the_other_way() {
    let (mut s, _songs, mut now) = menu_look_moving("select-look-lean");
    let rest = settled(&mut s, &mut now);
    let play_at = |c: &Canvas| hit_rect(c, |h| *h == Hit::PpSim).expect("the pp button");
    let mid = play_at(&rest);
    s.mouse = (0.0, 0.0);
    let mut c = rest;
    for _ in 0..120 {
        now += Duration::from_millis(16);
        c = frame(&mut s, now);
    }
    let leant = play_at(&c);
    let [bx, by, room] = screens::select::song_select::background_lean(&s);
    assert!(
        bx > 1.0 && by > 1.0 && room >= bx.max(by),
        "the picture leans away: {bx} {by} {room}"
    );
    assert!(
        leant[0] < mid[0] - 1.0 && leant[1] < mid[1] - 1.0,
        "the details lean towards the pointer: {mid:?} to {leant:?}"
    );
    assert!(leant[0] > mid[0] - 2.5, "a little: {mid:?} to {leant:?}");
}

#[test]
fn set_cards_carry_no_difficulty_count_over_their_pictures() {
    let (mut s, _songs, mut now) = menu_look_with("select-look-no-count", &many());
    let c = settled(&mut s, &mut now);
    let counts = |c: &Canvas| {
        c.texts
            .iter()
            .filter(|(_, t)| t.trim().ends_with(" diffs") || t.trim().ends_with(" diff"))
            .count()
    };
    assert_eq!(counts(&c), 0, "the main menu's look");
    // Legacy keeps it.
    let songs = Songs::new("select-look-no-count-legacy", &many());
    let mut legacy = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut legacy, &mut now);
    let c = settled(&mut legacy, &mut now);
    assert!(counts(&c) > 0, "Legacy's cards");
}

#[test]
fn the_pp_button_leaves_values_out_rather_than_spill_out_of_a_narrow_panel() {
    let (mut s, _songs, mut now) = menu_look("select-look-pp-narrow");
    let c = settled(&mut s, &mut now);
    let (_, widest) = ui::Select::share_range(c.width).expect("a range");
    let mut fewest = 3;
    for k in 0..=8 {
        // From the list's default share to its widest (the panel's narrowest).
        let share = s.cfg.song_list_share + (widest - s.cfg.song_list_share) * k as f32 / 8.0;
        s.cfg.song_list_share = share;
        for open in [false, true] {
            if s.preview.sim.is_some() != open {
                s.click(Hit::PpSim, None);
            }
            let c = settled(&mut s, &mut now);
            let button = hit_rect(&c, |h| *h == Hit::PpSim).expect("the pp button");
            let mut area = button;
            for h in c
                .hits
                .iter()
                .filter(|h| matches!(h.hit, Hit::PpSimStep(..)))
            {
                area[3] = area[3].max(h.rect[1] + h.rect[3] - area[1]);
            }
            let board = hit_rect(&c, |h| *h == Hit::BoardPage).expect("Leaderboard");
            let panel_right = board[0] + board[2] + 1.0;
            let boxes: Vec<(String, [f32; 4])> = text_boxes(&c)
                .into_iter()
                .filter(|(_, r)| {
                    let mid = [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0];
                    inside(mid, [area[0], area[1], panel_right - area[0], area[3]], 0.0)
                        && !inside(mid, board, 0.0)
                })
                .collect();
            for (t, r) in &boxes {
                assert!(!t.contains("..."), "{share}: nothing cut: {t}");
                let right = if r[1] < button[1] + button[3] {
                    button[0] + button[2]
                } else {
                    panel_right
                };
                assert!(
                    r[0] + r[2] <= right + 0.5,
                    "{share} open {open}: {t} {r:?} spills past {right}"
                );
            }
            for (i, (a, ra)) in boxes.iter().enumerate() {
                for (b, rb) in &boxes[i + 1..] {
                    let apart = ra[0] + ra[2] <= rb[0] + 0.5
                        || rb[0] + rb[2] <= ra[0] + 0.5
                        || ra[1] + ra[3] <= rb[1] + 0.5
                        || rb[1] + rb[3] <= ra[1] + 0.5;
                    assert!(apart, "{share} open {open}: {a} {ra:?} overlaps {b} {rb:?}");
                }
            }
            if !open {
                let shown = ["SS", "99%", "98%"]
                    .iter()
                    .filter(|l| boxes.iter().any(|(t, _)| t == *l))
                    .count();
                assert!(
                    shown >= 1 && boxes.iter().any(|(t, _)| t == "SS"),
                    "{share}: SS stays"
                );
                fewest = fewest.min(shown);
            }
        }
    }
    assert!(fewest < 3, "the narrowest panel leaves 98% out");
}

#[test]
fn the_leaderboard_chip_comes_in_with_the_pp_row_on_a_song_change() {
    let (mut s, _songs, mut now) = menu_look_moving("select-look-board-enter");
    let label = |c: &Canvas| {
        let board = hit_rect(c, |h| *h == Hit::BoardPage).expect("Leaderboard");
        let (range, _) = c
            .texts
            .iter()
            .find(|(r, t)| {
                t.trim() == "Leaderboard" && inside(c.shapes[r.start].center, board, 2.0)
            })
            .cloned()
            .expect("its label");
        c.shapes[range.start]
    };
    let rest = label(&settled(&mut s, &mut now));
    press(&mut s, KeyCode::ArrowRight, now);
    // Just after the pp row starts in (260 ms on): right of its place and faint.
    now += Duration::from_millis(16);
    frame(&mut s, now);
    now += Duration::from_millis(270);
    let coming = label(&frame(&mut s, now));
    assert!(
        coming.center[0] > rest.center[0] + 1.0 && coming.center[0] <= rest.center[0] + 8.5,
        "shifted right: {} vs {}",
        coming.center[0],
        rest.center[0]
    );
    assert!(coming.fill[3] < rest.fill[3] - 0.1, "fading in");
    assert_eq!(coming.center[1], rest.center[1]);
    let landed = label(&settled(&mut s, &mut now));
    assert_eq!(landed.center, rest.center, "in place after");
    assert_eq!(landed.fill[3], rest.fill[3]);
}

#[test]
fn the_leaderboard_chip_goes_out_with_the_old_details() {
    let (mut s, _songs, mut now) = menu_look_moving("select-look-board-leave");
    let c = settled(&mut s, &mut now);
    let chip = hit_rect(&c, |h| *h == Hit::BoardPage).expect("Leaderboard");
    let (range, _) = c
        .texts
        .iter()
        .find(|(r, t)| t.trim() == "Leaderboard" && inside(c.shapes[r.start].center, chip, 2.0))
        .cloned()
        .expect("its label");
    let letter = c.shapes[range.start];
    press(&mut s, KeyCode::ArrowRight, now);
    now += Duration::from_millis(16);
    frame(&mut s, now);
    now += Duration::from_millis(32);
    let going = frame(&mut s, now);
    // The old one's first letter, a little left and fainter.
    assert!(
        going.shapes.iter().any(|sh| {
            sh.half == letter.half
                && sh.params == letter.params
                && (sh.center[1] - letter.center[1]).abs() < 0.01
                && sh.center[0] < letter.center[0] - 0.01
                && sh.center[0] > letter.center[0] - 6.5
                && sh.fill[3] < letter.fill[3]
        }),
        "the old chip going out with the details"
    );
}

/// The y of the card of set `row` (its first row), if drawn.
fn card_y(c: &Canvas, row: usize) -> Option<f32> {
    cards(c)
        .into_iter()
        .find(|(r, _)| *r == row)
        .map(|(_, r)| r[1])
}

/// A frame 16 ms on, drawn twice (a frame's hits come back with the next one).
fn step(s: &mut State, now: &mut Instant) -> Canvas {
    *now += Duration::from_millis(16);
    frame(s, *now);
    frame(s, *now)
}

#[test]
fn a_notch_of_the_wheel_glides_the_cards_rather_than_jumping_them() {
    let (mut s, _songs, mut now) =
        menu_look_cfg("select-look-wheel-glide", &many_sets_charts(), true);
    let c = settled(&mut s, &mut now);
    let row = cards(&c)[2].0;
    let y0 = card_y(&c, row).expect("a card");
    s.wheel(-1);
    let mut ys = Vec::new();
    for _ in 0..40 {
        ys.push(card_y(&step(&mut s, &mut now), row).unwrap_or(f32::NAN));
    }
    let end = card_y(&settled(&mut s, &mut now), row).expect("still on screen");
    let total = y0 - end;
    assert!(total > 60.0, "a notch moves the list up: {y0} to {end}");
    // Moving on the very first frame, then still on its way for a while: seen gliding.
    let moved = |k: usize| (y0 - ys[k]) / total;
    assert!(moved(0) > 0.02, "it moves at once: {ys:?}");
    assert!(moved(0) < 0.3, "not a jump on the first frame: {ys:?}");
    assert!(moved(5) < 0.7, "still gliding 100 ms on: {ys:?}");
    assert!(moved(39) > 0.97, "there in the end: {ys:?}");
    assert!(
        ys.windows(2).all(|w| w[1] <= w[0] + 0.01),
        "one way, no wobble: {ys:?}"
    );
}

#[test]
fn a_notch_past_the_list_s_end_bumps_it_and_it_springs_back() {
    let (mut s, _songs, mut now) = menu_look_cfg("select-look-wheel-end", &maps::all(), true);
    settled(&mut s, &mut now);
    for _ in 0..30 {
        s.wheel(-1);
    }
    let rest = settled(&mut s, &mut now);
    let row = cards(&rest).last().expect("a card").0;
    let y0 = card_y(&rest, row).unwrap();
    s.wheel(-1);
    let ys: Vec<f32> = (0..6)
        .map(|_| card_y(&step(&mut s, &mut now), row).unwrap())
        .collect();
    assert!(
        ys.iter().any(|y| y < &(y0 - 2.0)),
        "the list gives a little at its end: {y0} {ys:?}"
    );
    let back = card_y(&settled(&mut s, &mut now), row).unwrap();
    assert!((back - y0).abs() < 0.1, "and springs back: {y0} {back}");
}

#[test]
fn a_set_picked_after_wheeling_glides_to_the_target_without_a_jump() {
    let (mut s, _songs, mut now) =
        menu_look_cfg("select-look-wheel-pick", &many_sets_charts(), true);
    settled(&mut s, &mut now);
    for _ in 0..4 {
        s.wheel(-1);
    }
    let c = settled(&mut s, &mut now);
    // A set low on the screen, far from the target.
    let [_, end] = s.select.frame_span();
    let (row, r) = cards(&c)
        .into_iter()
        .rev()
        .find(|(_, r)| r[1] + r[3] < end - 30.0)
        .expect("a card low down");
    s.click(Hit::SongHeader(row), None);
    let mut ys = vec![r[1]];
    for _ in 0..40 {
        ys.push(card_y(&step(&mut s, &mut now), row).expect("the picked card"));
    }
    let to = *ys.last().unwrap();
    let far = (r[1] - to).abs();
    assert!(far > 60.0, "it goes a way up to the target: {ys:?}");
    let biggest = ys
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(
        biggest < far * 0.16,
        "no frame moves it more than a small part of the way: {biggest} of {far}: {ys:?}"
    );
    assert!(
        (ys[18] - to).abs() > 0.5,
        "still gliding ~290 ms on: {ys:?}"
    );
}

fn many_sets_charts() -> Vec<maps::Chart> {
    (0..60)
        .map(|i| maps::Chart {
            name: Box::leak(format!("set-{i:02}").into_boxed_str()),
            ..maps::basics()
        })
        .collect()
}

#[test]
fn song_select_s_routes_go_through_the_blur_and_legacy_s_do_not() {
    let (mut s, _songs, mut now) = menu_look_moving("select-look-routes-blur");
    let c = settled(&mut s, &mut now);
    let (range, k) = c.blur.clone().expect("the routes are blurred");
    assert_eq!(k, 1.0, "wholly, as the picture");
    // The routes (none drawn here, without music): plain shapes, before song select's own.
    assert!(
        c.shapes[range.clone()].iter().all(plain),
        "plain shapes only: the routes' notes and lines"
    );
    assert!(
        c.texts
            .iter()
            .all(|(r, _)| r.end <= range.start || r.start >= range.end),
        "no text of song select's in it"
    );
    // Legacy keeps them as they are.
    let songs = Songs::new("select-look-routes-blur-legacy", &maps::all());
    let mut legacy = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut legacy, &mut now);
    let c = settled(&mut legacy, &mut now);
    assert!(c.blur.is_none(), "{:?}", c.blur);
}

#[test]
fn the_wheel_over_a_pp_simulator_field_steps_it() {
    let (mut s, _songs, mut now) = menu_look("select-look-pp-wheel");
    settled(&mut s, &mut now);
    s.click(Hit::PpSim, None);
    let c = settled(&mut s, &mut now);
    let field = |c: &Canvas, f: u8| {
        hit_rect(c, |h| *h == Hit::PpSimField(f)).expect("the simulator's field")
    };
    let k = (s.window_h / render::VIRTUAL_HEIGHT) as f64;
    let point = |s: &mut State, r: [f32; 4]| {
        // Its middle: the value, between its - and +.
        let (px, py) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        s.mouse = (px as f64 * k, py as f64 * k);
    };
    let before = s.preview.sim.expect("open");
    point(&mut s, field(&c, 0));
    s.wheel(-2);
    let sim = s.preview.sim.expect("still open");
    assert!(sim.accuracy < before.accuracy, "down: less accuracy");
    assert_eq!(sim.misses, before.misses, "misses untouched");
    s.wheel(2);
    assert_eq!(
        s.preview.sim.unwrap().accuracy,
        before.accuracy,
        "and back up"
    );
    let c = settled(&mut s, &mut now);
    point(&mut s, field(&c, 1));
    s.wheel(3);
    assert_eq!(
        s.preview.sim.unwrap().misses,
        before.misses + 3,
        "up: more misses"
    );
    // A click on the field itself does nothing.
    let at = s.preview.sim.unwrap();
    s.click(Hit::PpSimField(1), None);
    assert_eq!(s.preview.sim.unwrap().misses, at.misses);
}
