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
    let songs = Songs::new(tag, charts);
    let cfg = Config {
        main_menu: true,
        accent: ACCENT,
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
    // The selection's outline round the open set's card is the one edge.
    let open = open_card(&c, &s);
    let edged: Vec<[f32; 4]> = c
        .shapes
        .iter()
        .filter(|sh| plain(sh) && on_select(sh, &c, &s) && !inside(sh.center, open, 1.0))
        .filter(|sh| sh.params[1] > 0.0 && sh.border[3] > 0.01)
        .filter(|sh| !same_rgb(sh.border, sh.fill) || sh.fill[3] < sh.border[3])
        .map(rect)
        .collect();
    assert!(edged.is_empty(), "boxes with an edge round them: {edged:?}");
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
            c.picture_paths
                .iter()
                .any(|(k, p)| *p == bg && *k == render::path_picture_key(&bg)),
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
    assert!(
        others.iter().all(|r| r[0] == 36.0),
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
    assert_eq!(moved[0], 32.0, "it comes out under the pointer");
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
    // Right of the list: the banner's strips, top to bottom (the card's own copy is in the list).
    let strips: Vec<f32> = c
        .shapes
        .iter()
        .filter(|sh| picture(sh) && sh.center[0] > 28.0 + lw)
        .map(|sh| sh.fill[3])
        .collect();
    assert!(strips.len() >= 6, "a banner in strips: {strips:?}");
    assert!(
        (strips[0] - 1.0).abs() < 1e-3,
        "whole at the top: {strips:?}"
    );
    assert!(
        strips.windows(2).all(|w| w[1] <= w[0] + 1e-4),
        "fading out downwards: {strips:?}"
    );
    assert!(
        strips[strips.len() - 1] < 0.1,
        "gone at its bottom: {strips:?}"
    );
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
