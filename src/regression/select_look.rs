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
    let songs = Songs::new(tag, &maps::all());
    let cfg = Config {
        main_menu: true,
        accent: ACCENT,
        ..harness::config()
    };
    let mut s = state_with(cfg, songs.entries());
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

#[test]
fn surfaces_have_no_edge_and_rows_no_lines_under_them() {
    let (mut s, _songs, mut now) = menu_look("select-look-edges");
    let c = settled(&mut s, &mut now);
    let edged: Vec<[f32; 4]> = c
        .shapes
        .iter()
        .filter(|sh| plain(sh) && on_select(sh, &c, &s))
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
    // Its name over the details, right of the list: the letters' pixels, not the pill by it.
    let lw = ui::Select::list_width(c.width, s.cfg.song_list_share);
    assert!(
        c.shapes.iter().any(|sh| (plain(sh) || glyph(sh))
            && sh.half[0].min(sh.half[1]) < 1.0
            && sh.center[0] > 28.0 + lw
            && on_select(sh, &c, &s)
            && same_rgb(sh.fill, colour)),
        "the difficulty's name in the details is in its rating's colour"
    );
}
