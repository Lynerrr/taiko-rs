//! Song select in the main menu's look draws with the design tokens (`style`: surfaces, ink,
//! colour roles): its surfaces have the main menu's crisp neutral edge, the accent is only on Play and the
//! selection, and a rating's colour only on the selected difficulty. The Legacy layout keeps its
//! own look (its goldens in `screens`).

use super::harness::{self, *};
use super::maps;
use crate::hit::Hit;
use crate::render::Shape;
use crate::*;

/// A frame, in song select's own units (`harness::in_select_units`): what's tested here is its
/// layout, whatever the size of the space it lays itself out in.
fn frame(s: &mut State, now: Instant) -> Canvas {
    let c = harness::frame(s, now);
    harness::in_select_units(c, s)
}

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

/// Where song select's point `x`, `y` (its own units) is on the window, in pixels.
fn to_pixels(s: &State, x: f32, y: f32) -> (f64, f64) {
    let k = s.window_h / render::VIRTUAL_HEIGHT * s.select_scale;
    (((x + s.select_left) * k) as f64, (y * k) as f64)
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

/// The right column's one panel: song select's cold one right of the list.
fn column_panel(c: &Canvas, s: &State) -> Option<[f32; 4]> {
    let lw = s.select.list_w(c.width);
    let fill = render::rgba(style::PANEL_COLD, style::PANEL_COLD_ALPHA);
    c.shapes
        .iter()
        .filter(|sh| plain(sh) && sh.fill == fill && sh.border[3] == fill[3])
        .filter(|sh| sh.center[0] > 28.0 + lw && sh.half[1] * 2.0 > 60.0)
        .map(rect)
        .next()
}

/// The lines across the right column's panel parting its sections, top down.
fn column_rules(c: &Canvas, s: &State) -> Vec<[f32; 4]> {
    let Some([px, py, pw, ph]) = column_panel(c, s) else {
        return Vec::new();
    };
    let edge = render::rgba(style::tone::EDGE, 1.0);
    let inset = crate::screens::select::song_select::COLUMN_RULE_INSET;
    let mut found: Vec<[f32; 4]> = c
        .shapes
        .iter()
        .filter(|sh| plain(sh) && sh.fill == edge)
        .map(rect)
        .filter(|r| {
            (r[3] - style::tone::LINE).abs() < 0.01
                && (r[0] - (px + inset)).abs() < 0.01
                && (r[2] - (pw - 2.0 * inset)).abs() < 0.01
                && r[1] > py
                && r[1] < py + ph
        })
        .collect();
    found.sort_by(|a, b| a[1].total_cmp(&b[1]));
    found.dedup();
    found
}

/// The right column's sections, top down: the map's details, the row of tabs and the body under
/// it (the preview or the leaderboard), on the column's one panel. A line parts the details
/// from the tabs, another closes the body over the lane of actions.
fn sections(c: &Canvas, s: &State) -> Vec<[f32; 4]> {
    let Some([px, py, pw, _]) = column_panel(c, s) else {
        return Vec::new();
    };
    let half_gap = style::space::SECTION / 2.0;
    let rules = column_rules(c, s);
    let [first, last] = match rules[..] {
        [first, last] => [first, last],
        _ => return Vec::new(),
    };
    let tabs_y = first[1] + half_gap;
    let body_y = tabs_y + COLUMN_TAB_H;
    vec![
        [px, py, pw, first[1] - half_gap - py],
        [px, tabs_y, pw, COLUMN_TAB_H],
        [px, body_y, pw, last[1] - half_gap - body_y],
    ]
}

/// Song select with the right column's body on the local (`online`: the online) leaderboard.
fn on_board(s: &mut State, now: &mut Instant, online: bool) -> Canvas {
    settled(s, now);
    s.click(Hit::BoardTab(online), None);
    settled(s, now)
}

/// A badge as the mods' are (`style::badge`): a dark tint of its colour under a one-unit edge
/// of it.
fn badge(sh: &Shape) -> bool {
    sh.params[0] == style::radius::SMALL
        && sh.params[1] == 1.0
        && (sh.border[3] - 0.9).abs() < 1e-3
        && (sh.fill[3] - 0.92).abs() < 1e-3
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
fn surfaces_are_parted_by_tone_with_lines_only_round_what_is_pressed_or_selected() {
    let (mut s, _songs, mut now) = menu_look("select-look-edges");
    let c = settled(&mut s, &mut now);
    // The selection's outline round the open set's card and the brackets round the selected
    // difficulty are the strong edges; a badge (a rating, the map's status) is edged in its
    // colour, and a button has the main menu's neutral edge. Nothing else has a line round it:
    // not the other cards, the difficulty rows, the right column's sections, the pp strip, the
    // leaderboard's tabs and rows or the preview's lane.
    let open = open_card(&c, &s);
    let set_cards = cards(&c);
    // What a click presses: the search row's and the actions' buttons, the search, the grip
    // (small; not a row, a card, a tab or the lane).
    let pressed = |p: [f32; 2]| {
        c.hits.iter().any(|h| {
            !matches!(
                h.hit,
                Hit::SongRow(_)
                    | Hit::SongHeader(_)
                    | Hit::ScoreRow(_)
                    | Hit::OnlineScoreRow(_)
                    | Hit::Preview
                    | Hit::BoardTab(false)
                    | Hit::PpSim
            ) && h.rect[2] < 400.0
                // (A button, the search field, or the list's grip down its side.)
                && (h.rect[3] < 60.0 || h.rect[2] <= 24.0)
                && inside(p, h.rect, 0.5)
        })
    };
    let primary = render::rgba(style::role::primary(), 1.0);
    let edged: Vec<[f32; 4]> = c
        .shapes
        .iter()
        .filter(|sh| plain(sh) && on_select(sh, &c, &s) && !inside(sh.center, open, 1.0))
        .filter(|sh| sh.params[1] > 0.0 && sh.border[3] > 0.01)
        .filter(|sh| !same_rgb(sh.border, sh.fill) || sh.fill[3] < sh.border[3])
        // The brackets round the selected difficulty.
        .filter(|sh| !same_rgb(sh.border, primary))
        .filter(|sh| !badge(sh))
        .filter(|sh| !pressed(sh.center))
        .map(rect)
        .collect();
    assert!(
        edged.is_empty(),
        "lines round what isn't pressed or selected: {edged:?}"
    );
    // The column is one panel (its sections parted by lines across it), as the list is.
    assert!(column_panel(&c, &s).is_some(), "the column's one panel");
    assert_eq!(
        sections(&c, &s).len(),
        3,
        "its details, its tabs and the body under them"
    );
    // No line under a row of the list: rows are parted by room (the one line is the list's
    // panel's, along its top above the rows).
    let lw = s.select.list_w(c.width);
    let first_top = set_cards.iter().map(|(_, r)| r[1]).fold(f32::MAX, f32::min);
    let lines: Vec<[f32; 4]> = c
        .shapes
        .iter()
        .filter(|sh| plain(sh) && sh.fill[3] > 0.05 && on_select(sh, &c, &s))
        .filter(|sh| sh.center[0] < 28.0 + lw)
        .map(rect)
        .filter(|r| r[3] <= 1.6 && r[2] >= 40.0 && r[1] >= first_top - 0.5)
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
    // The column's tab shown (the preview's, at first) is selected too: a bar of the accent
    // under its word.
    let tab = hit_rect(&c, |h| *h == Hit::PreviewTab).expect("the Preview tab");
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
                && !inside(sh.center, tab, 0.0)
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
    // Its name in the details, on a badge edged in its rating's colour (left of the details'
    // middle; the rating's own badge is at their right end).
    let head = sections(&c, &s)[0];
    let middle = head[0] + head[2] / 2.0;
    assert!(
        c.shapes.iter().any(|sh| badge(sh)
            && inside(sh.center, head, 0.0)
            && sh.center[0] < middle
            && sh.half[1] * 2.0 >= 13.0
            && on_select(sh, &c, &s)
            && same_rgb(sh.border, colour)),
        "the difficulty's badge in the details is in its rating's colour"
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
fn the_cards_sit_on_the_lists_wave_the_rest_dimmed_and_one_under_the_pointer_comes_out() {
    let (mut s, _songs, mut now) = menu_look("select-look-cards");
    let c = settled(&mut s, &mut now);
    let open = open_card(&c, &s);
    let (top, bottom) = (s.select.list_top(), s.select.list_bottom());
    // Every card where the list's wave has it for where it is, the open one too (no step out
    // of the line for it), its right edge on the list's.
    let lw = s.select.list_w(c.width);
    for r in cards(&c)
        .into_iter()
        .map(|(_, r)| r)
        .filter(|r| r[1] >= top && r[1] + r[3] <= bottom)
    {
        let wave = crate::ui::list_wave(r[1] + r[3] / 2.0, top, bottom);
        assert!(
            (r[0] - 24.0 - wave).abs() < 0.6,
            "on the wave: {r:?}, {wave} in"
        );
        assert!(
            (r[0] + r[2] - (24.0 + lw)).abs() < 0.01,
            "to the list's end: {r:?}"
        );
    }
    let others: Vec<[f32; 4]> = cards(&c)
        .into_iter()
        .map(|(_, r)| r)
        .filter(|r| *r != open)
        .collect();
    assert!(!others.is_empty());
    // The others are dimmed, darker rather than see-through: none of their text is at full
    // brightness.
    // (One far enough off the middle to come out of the wave by all of it.)
    let first = others
        .iter()
        .copied()
        .find(|r| r[0] >= 24.0 + 4.0)
        .expect("a card off the middle");
    let ink: Vec<&Shape> = c
        .texts
        .iter()
        .filter(|(r, _)| {
            c.shapes
                .get(r.start)
                .is_some_and(|sh| inside(sh.center, first, 0.0))
        })
        .flat_map(|(r, _)| &c.shapes[r.clone()])
        .collect();
    assert!(!ink.is_empty(), "the set's name on its card");
    assert!(
        ink.iter()
            .all(|sh| sh.fill[0].max(sh.fill[1]).max(sh.fill[2]) < 0.71),
        "a set not open is dimmed"
    );
    // The pointer on one: it comes out 4 units (animations are off: at once).
    let (px, py) = to_pixels(&s, first[0] + first[2] / 2.0, first[1] + 10.0);
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
    let lw = s.select.list_w(c.width);
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
        // The card's, not the panel the list runs down.
        .filter(|sh| sh.half[1] * 2.0 < 100.0)
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

#[test]
fn a_card_cut_by_the_lists_top_fades_out_rather_than_show_part_of_itself() {
    let (mut s, _songs, mut now) = many_sets("select-look-top");
    settled(&mut s, &mut now);
    let smooth = |x: f32| x * x * (3.0 - 2.0 * x);
    let (rh, _, _) = s.select.row_heights();
    let (mut gone, mut fading) = (0, 0);
    // Dragged by what isn't a whole number of rows, cards come to sit across the top.
    drag_then_wheel(&mut s, &mut now, 413.0, 12, |s, c, how| {
        let lw = s.select.list_w(c.width);
        // A card cut by the list's top takes the pointer from the top down only: its hit is
        // the part of it under the top.
        for (_, r) in cards(c) {
            // (Not one cut by the list's bottom.)
            if r[3] >= rh - 0.01 || r[1] > c.height / 2.0 {
                continue;
            }
            let (top, under) = (r[1], rh - r[3]);
            // What of it reaches below the top, where it's seen.
            let most = c
                .shapes
                .iter()
                .filter(|sh| plain(sh) && sh.center[0] > 20.0 && sh.center[0] < 28.0 + lw)
                .filter(|sh| {
                    let [_, y, _, h] = rect(sh);
                    y + h > top + 0.5 && sh.center[1] < r[1] + r[3] && sh.center[0] > r[0]
                })
                .map(|sh| sh.fill[3])
                .fold(0.0, f32::max);
            // Gone once half of it is cut; fading out on the way there.
            let left = (1.0 - under / (rh * 0.5)).clamp(0.0, 1.0);
            assert!(
                most <= smooth(left) + 0.01,
                "{how}: {under} of a card cut by the top at {top}: {most}"
            );
            if left == 0.0 {
                gone += 1;
            } else {
                fading += 1;
            }
        }
    });
    assert!(
        gone > 0 && fading > 0,
        "cards cut by the top: {gone} gone, {fading} fading"
    );
}

/// A picture drawn (`render::Canvas::picture`): its slot is in `params[3]` below -1000.
fn picture(s: &Shape) -> bool {
    s.params[3] <= -1000.0
}

#[test]
fn the_banners_text_sits_on_a_plate_dark_at_its_left_lighter_across_and_gone_down_with_the_picture()
{
    let (mut s, _songs, mut now) = menu_look("select-look-banner-plate");
    let i = s.select.selected().expect("a map is selected");
    let bg = preview::background_path(&s.library[i]).expect("it names a picture");
    s.pictures
        .pretend_loaded(&render::path_picture_key(&bg), 16.0 / 9.0);
    let c = settled(&mut s, &mut now);
    let lw = s.select.list_w(c.width);
    let dark = render::rgba(style::SURFACE_1, 1.0);
    // Right of the list: the plate's layers, eased sweeps of the panel's tone across the banner
    // from dark at the left to lighter at the right, each cut to its strip as the picture's
    // layers are.
    // Its strength at the left and at the right, its rect and its strip's.
    type Layer = (f32, f32, [f32; 4], [f32; 4]);
    let layers: Vec<Layer> = c
        .shapes
        .iter()
        .enumerate()
        .filter(|(_, sh)| {
            sh.params[3] == render::SHADE_EASED
                && sh.center[0] > 28.0 + lw
                && same_rgb(sh.fill, dark)
        })
        .map(|(i, sh)| {
            let clip = c
                .clips
                .iter()
                .find(|(r, _)| r.contains(&i))
                .map(|(_, rect)| *rect)
                .expect("a layer is cut to its strip");
            (sh.fill[3], sh.border[3], rect(sh), clip)
        })
        .collect();
    assert!(layers.len() >= 4, "a plate in layers: {layers:?}");
    let head = sections(&c, &s)[0];
    for (left, right, r, _) in &layers {
        assert!(r[2] > head[2] * 0.9, "across the banner: {r:?} in {head:?}");
        assert!(
            left > right && *right >= 0.0,
            "lighter to the right: {left} to {right}"
        );
    }
    let shown = |y: f32, at: fn(&Layer) -> f32| {
        1.0 - layers
            .iter()
            .filter(|(_, _, _, r)| y >= r[1] && y < r[1] + r[3])
            .map(|l| 1.0 - at(l))
            .product::<f32>()
    };
    let top = layers.iter().map(|l| l.3[1]).fold(f32::MAX, f32::min);
    let bottom = layers.iter().map(|l| l.3[1] + l.3[3]).fold(0.0, f32::max);
    // Dense under the title at the left, open at the right end where the picture shows; the
    // picture itself near its own brightness, not a black box.
    let left = shown(top + 10.0, |l| l.0);
    let right = shown(top + 10.0, |l| l.1);
    assert!(left > 0.5, "dark under the title: {left}");
    assert!(right < 0.05, "open at the right end: {right}");
    // Tall, for the picture to show (on 720), its text down at its foot: the picture open over
    // its top.
    let tall = bottom - top;
    assert!((165.0..=185.0).contains(&tall), "the banner {tall} tall");
    let bright = c
        .shapes
        .iter()
        .filter(|sh| picture(sh) && sh.center[0] > 28.0 + lw && sh.fill[3] > 0.0)
        .map(|sh| sh.fill[0])
        .fold(0.0, f32::max);
    assert!(bright >= 0.8, "the picture shows: drawn at {bright}");
    // The title has a soft shadow: a dark copy of it just under it, drawn first.
    let title = s.library[i].meta.title.clone();
    let (r, _) = c
        .texts
        .iter()
        .find(|(r, t)| *t == title && !r.is_empty() && c.shapes[r.start].center[0] > 28.0 + lw)
        .expect("the title");
    let glyph = c.shapes[r.start];
    assert!(
        glyph.center[1] > top + tall * 0.5,
        "the title in the banner's lower half: {} of {top}..{bottom}",
        glyph.center[1]
    );
    let shadow = c.shapes[..r.start].iter().rev().take(r.len()).any(|sh| {
        (sh.center[0] - glyph.center[0]).abs() < 0.01
            && sh.center[1] > glyph.center[1]
            && sh.fill[0] == 0.0
            && sh.fill[3] > 0.2
    });
    assert!(shadow, "a shadow under the title");
    let at_end = shown(bottom - 0.5, |l| l.0);
    assert!(at_end < 0.1, "gone where the picture has: {at_end}");
}

#[test]
fn the_details_sections_are_split_by_lines_of_the_panels_edge() {
    let (mut s, _songs, mut now) = menu_look("select-look-dividers");
    let c = settled(&mut s, &mut now);
    let lw = s.select.list_w(c.width);
    let edge = render::rgba(style::tone::EDGE, 1.0);
    let mut lines: Vec<[f32; 4]> = c
        .shapes
        .iter()
        .filter(|sh| plain(sh) && same_rgb(sh.fill, edge) && sh.center[0] > 28.0 + lw)
        .map(rect)
        .filter(|r| (r[3] - style::tone::LINE).abs() < 0.01 && r[2] > 200.0)
        .collect();
    lines.sort_by(|a, b| a[1].total_cmp(&b[1]));
    let [_, od] = text_at(&c, "OD").expect("the OD tile");
    let pp = hit_rect(&c, |h| *h == Hit::PpSim).expect("the pp button");
    let lane = hit_rect(&c, |h| *h == Hit::Preview).expect("the preview");
    let between = |a: f32, b: f32| lines.iter().any(|r| r[1] > a && r[1] + r[3] < b);
    assert!(
        between(od - 12.0, od),
        "over the stats: {lines:?}, OD at {od}"
    );
    assert!(
        between(pp[1] - 8.0, pp[1]),
        "over the pp: {lines:?}, {pp:?}"
    );
    // The preview is a section of its own, under the leaderboard's: no line needed over it.
    let column = sections(&c, &s);
    assert_eq!(column.len(), 3, "{column:?}");
    assert!(inside([pp[0] + 1.0, pp[1] + 1.0], column[0], 0.0), "{pp:?}");
    assert!(
        inside([lane[0] + 1.0, lane[1] + 1.0], column[2], 0.0),
        "{lane:?}"
    );
    // The timeline's bars stand on one, starting with it (its times are under its left end,
    // 5 under its bars' box).
    let all = texts_in(&c, [0.0, 0.0, c.width, c.height]);
    let times = all
        .iter()
        .find(|t| t.contains(" / "))
        .expect("the timeline's times");
    let [tx, ty] = text_at(&c, times).expect("where they are");
    assert!(
        lines
            .iter()
            .any(|r| (r[0] - tx).abs() < 0.5 && r[1] < ty - 5.0 && r[1] > ty - 12.0),
        "under the bars: {lines:?}, times at {tx},{ty}"
    );
}

#[test]
fn the_details_open_on_a_banner_of_the_maps_picture_fading_out() {
    let (mut s, _songs, mut now) = menu_look("select-look-banner");
    let i = s.select.selected().expect("a map is selected");
    let bg = preview::background_path(&s.library[i]).expect("it names a picture");
    s.pictures
        .pretend_loaded(&render::path_picture_key(&bg), 16.0 / 9.0);
    let c = settled(&mut s, &mut now);
    let lw = s.select.list_w(c.width);
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
    let lw = s.select.list_w(c.width);
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
fn a_cards_picture_comes_in_across_it_out_of_nothing_with_nothing_over_it() {
    let (mut s, _songs, mut now) = menu_look("select-look-shade");
    let i = s.select.selected().expect("a map is selected");
    assert!(
        preview::background_path(&s.library[i]).is_some(),
        "it names a picture"
    );
    // Every card's picture in, the open one's and the others'.
    for e in &s.library {
        let bg = preview::background_path(e).expect("each names a picture");
        s.card_pictures
            .pretend_loaded(&render::path_picture_key(&bg), [0.5, 0.5, 0.5]);
    }
    let c = settled(&mut s, &mut now);
    let open = open_card(&c, &s);
    let in_card = |r: [f32; 4]| {
        c.shapes
            .iter()
            .enumerate()
            .find(|(_, sh)| card_picture(sh) && inside(sh.center, r, 0.0))
            .expect("the card's picture")
    };
    let (at, picture) = in_card(open);
    let [px, _, pw, _] = rect(picture);
    assert!(picture.params[0] > 0.0, "the picture as round as the card");
    // Over the whole card (1 in), not cut anywhere: no edge of it to show.
    assert!(
        (px - (open[0] + 1.0)).abs() < 0.01 && (pw - (open[2] - 2.0)).abs() < 0.01,
        "the whole card: {:?} in {open:?}",
        rect(picture)
    );
    assert!(
        !c.clips
            .iter()
            .any(|(r, cut)| r.contains(&at)
                && (cut[0] > px + 1.0 || cut[0] + cut[2] < px + pw - 1.0)),
        "not cut"
    );
    // It comes in across the card in its shader: none of it over the left third, under the
    // title and artist, then eased in to the right end, a little more than half of it there on
    // the card open.
    let (from, rest, lit) = ui::CARD_ART;
    assert!((0.30..=0.35).contains(&from), "{from}");
    assert_eq!(
        [picture.border[2], picture.border[3]],
        [from, 1.0],
        "from {from} across to the right end"
    );
    assert!((0.55..=0.60).contains(&lit), "{lit}");
    assert!(
        (picture.fill[3] - lit).abs() < 0.01,
        "the open card's: {}",
        picture.fill[3]
    );
    // Nothing over it: no plate, no sweep, nothing of the card's dark tone on it (which showed a
    // seam where it ended and clashed with the card's light under the pointer).
    let dark = render::rgba(style::SURFACE_1, 1.0);
    let over: Vec<[f32; 4]> = c.shapes[at + 1..]
        .iter()
        .filter(|sh| inside(sh.center, open, 0.0))
        .filter(|sh| {
            matches!(sh.params[3], render::SHADE_EASED | render::SHADE_ACROSS)
                || (plain(sh) && same_rgb(sh.fill, dark) && sh.fill[3] > 0.0)
        })
        .map(rect)
        .collect();
    assert!(over.is_empty(), "nothing over the picture: {over:?}");
    // The cards not open show less of theirs, the same way.
    let other = cards(&c)
        .into_iter()
        .map(|(_, r)| r)
        .find(|r| *r != open)
        .expect("another card");
    let (_, theirs) = in_card(other);
    assert!((0.30..=0.35).contains(&rest), "{rest}");
    assert!(
        theirs.fill[3] <= rest + 1e-3 && theirs.fill[3] < picture.fill[3] - 0.15,
        "muted where not open: {} vs {}",
        theirs.fill[3],
        picture.fill[3]
    );
    assert_eq!([theirs.border[2], theirs.border[3]], [from, 1.0]);
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

#[test]
fn the_card_colour_fade_greys_each_cards_picture_towards_its_side() {
    let (mut s, _songs, mut now) = menu_look("select-look-card-fade");
    let i = s.select.selected().expect("a map is selected");
    let bg = preview::background_path(&s.library[i]).expect("it names a picture");
    s.card_pictures
        .pretend_loaded(&render::path_picture_key(&bg), [0.5, 0.5, 0.5]);
    let greys = |s: &mut State, now: &mut Instant| {
        let c = settled(s, now);
        let open = open_card(&c, s);
        let picture = c
            .shapes
            .iter()
            .find(|sh| card_picture(sh) && inside(sh.center, open, 0.0))
            .expect("the card's picture");
        [picture.border[0], picture.border[1]]
    };
    assert_eq!(greys(&mut s, &mut now), [0.0, 0.0], "off: all its colour");
    s.cfg.card_fade = config::CardFade::ToRight;
    let [left, right] = greys(&mut s, &mut now);
    assert!(
        left == 0.0 && right > 0.5,
        "grey to the right: {left} {right}"
    );
    s.cfg.card_fade = config::CardFade::ToLeft;
    let [left, right] = greys(&mut s, &mut now);
    assert!(
        right == 0.0 && left > 0.5,
        "grey to the left: {left} {right}"
    );
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
fn rebuilding_the_list_keeps_the_cards_pictures_shown_without_fading_them_in_again() {
    let (mut s, _songs, mut now) =
        menu_look_cfg("select-look-covers-kept", &many_sets_charts(), true);
    let groups: Vec<usize> = s.select.song_groups().iter().map(|g| g.start).collect();
    for &row in &groups {
        let key = set_key(&s, row);
        s.card_pictures.pretend_loaded(&key, [0.3, 0.4, 0.5]);
    }
    // How strong each card's picture is drawn.
    let pictures = |s: &State, c: &Canvas| -> Vec<(usize, f32)> {
        cards_in_view(c, s)
            .into_iter()
            .map(|(row, r)| {
                let a = c
                    .shapes
                    .iter()
                    .filter(|sh| card_picture(sh) && inside(sh.center, r, 0.5))
                    .map(|sh| sh.fill[3])
                    .fold(0.0, f32::max);
                (row, a)
            })
            .collect()
    };
    let c = settled(&mut s, &mut now);
    let before = pictures(&s, &c);
    assert!(before.len() >= 5, "cards on screen");
    assert!(
        before.iter().all(|&(_, a)| a > 0.5 * ui::CARD_ART.1),
        "pictures in: {before:?}"
    );
    // The list rebuilt (a library update, the search, a sort): the pictures on screen stay.
    s.select.refilter(&s.library);
    now += Duration::from_millis(16);
    let c = frame(&mut s, now);
    let after = pictures(&s, &c);
    // As strong as before (give or take the beat's pulse), not faded out to come in again.
    assert_eq!(after.len(), before.len());
    for ((row, a), (_, b)) in after.iter().zip(&before) {
        assert!(
            (a - b).abs() < 0.02,
            "the card of row {row} keeps its picture: {b} then {a}"
        );
    }
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
    // Its colour where the picture comes in, out of nothing as the picture does.
    let coloured = |sh: &Shape| {
        sh.params[3] == render::SHADE_EASED
            && same_rgb(sh.border, [0.2, 0.6, 0.9, 1.0])
            && sh.fill[3] == 0.0
            && sh.border[3] > 0.0
    };
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
    // Quickly: in well under a fifth of a second, scrolled to fast they ease in, not snap.
    for _ in 0..3 {
        now += Duration::from_millis(16);
        frame(&mut s, now);
    }
    let c = frame(&mut s, now);
    let pic = in_card(&c, &card_picture);
    assert!(
        (pic[0].fill[3] - ui::CARD_ART.2).abs() < 0.01,
        "in within 150 ms: {}",
        pic[0].fill[3]
    );
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
fn cards_lie_on_one_wave_about_the_views_middle_however_the_list_is_scrolled() {
    let (mut s, _songs, mut now) = menu_look_with("select-look-arc", &many());
    let lw = s.select.list_w(harness::WIDTH / s.select_scale.max(1e-3));
    let check = |c: &Canvas, s: &State, how: &str| {
        let (top, bottom) = (s.select.list_top(), s.select.list_bottom());
        let mid = (top + bottom) / 2.0;
        let lw = s.select.list_w(c.width);
        // The cards in full view, nearest the view's middle first.
        let mut shown: Vec<(f32, [f32; 4])> = cards(c)
            .into_iter()
            .map(|(_, r)| r)
            .filter(|r| r[1] >= top && r[1] + r[3] <= bottom)
            .map(|r| ((r[1] + r[3] / 2.0 - mid).abs(), r))
            .collect();
        shown.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert!(shown.len() >= 4, "{how}: cards in view: {shown:?}");
        for (_, r) in &shown {
            // Where its row's middle is.
            let cy = r[1] + r[3] / 2.0;
            let wave = crate::ui::list_wave(cy, top, bottom);
            assert!(
                (r[0] - 24.0 - wave).abs() < 0.01,
                "{how}: on the wave: {r:?} wants {wave} in"
            );
            assert!(
                (r[0] + r[2] - (24.0 + lw)).abs() < 0.01,
                "{how}: its right edge on the list's: {r:?}"
            );
        }
        // Farther from the middle, farther in; up to the wave's most at the view's edges.
        assert!(
            shown
                .windows(2)
                .all(|w| w[1].1[0] >= w[0].1[0] - 0.01 || w[1].0 < w[0].0 + 0.01),
            "{how}: in by how far from the middle: {shown:?}"
        );
        let far = shown[shown.len() - 1].1[0] - 24.0;
        assert!(
            far > 0.25 * crate::ui::WAVE_MAX && far <= crate::ui::WAVE_MAX + 0.01,
            "{how}: an arc, not a line: {far}"
        );
    };
    assert!(lw > 0.0);
    let c = songs_on(&mut s, &mut now, 6);
    check(&c, &s, "settled");
    // A card at the view's middle sits against the list's left edge.
    assert_eq!(crate::ui::list_wave(300.0, 100.0, 500.0), 0.0);
    assert!(
        (crate::ui::list_wave(100.0, 100.0, 500.0) - crate::ui::WAVE_MAX).abs() < 1e-4
            && (crate::ui::list_wave(500.0, 100.0, 500.0) - crate::ui::WAVE_MAX).abs() < 1e-4,
        "the most at both edges"
    );
    // Wheeled away from the selection, and held mid-drag: still on the one wave (no card left
    // in, no step).
    for _ in 0..3 {
        s.wheel(-1);
        now += Duration::from_millis(16);
        frame(&mut s, now);
    }
    let c = settled(&mut s, &mut now);
    check(&c, &s, "wheeled");
    s.select.drag_to(-90.0);
    now += Duration::from_millis(16);
    frame(&mut s, now);
    let c = frame(&mut s, now);
    check(&c, &s, "dragged");
    s.select.drag_end(&s.library);
    // Farther from the selection's target, fainter.
    let c = songs_on(&mut s, &mut now, 1);
    let t = target(&c, &s);
    let ty = t[1] + t[3] / 2.0;
    let [_, end] = s.select.frame_span();
    let open = open_card(&c, &s);
    let mut closed: Vec<(f32, [f32; 4])> = cards(&c)
        .into_iter()
        .map(|(_, r)| r)
        .filter(|r| *r != open && r[1] > s.select.list_top() && r[1] + r[3] < end - 24.0)
        .map(|r| ((r[1] + r[3] / 2.0 - ty).abs(), r))
        .collect();
    closed.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (near, far) = (closed[0].1, closed[closed.len() - 1].1);
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

/// The target's flash: soft white fills across the selected row, more of them than it has at
/// rest (the selected difficulty sits on a light of its own).
fn flashes(c: &Canvas, s: &State) -> usize {
    let lw = s.select.list_w(c.width);
    c.shapes
        .iter()
        .filter(|sh| {
            plain(sh)
                && same_rgb(sh.fill, render::rgba(0xffffff, 1.0))
                && sh.fill[3] > 0.02
                && sh.fill[3] < 0.3
                && sh.half[0] * 2.0 > lw / 2.0
        })
        .count()
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
        let rest = flashes(&frame(&mut s, now), &s);
        let mut seen = Vec::new();
        for _ in 0..20 {
            now += Duration::from_millis(16);
            seen.push(flashes(&frame(&mut s, now), &s) > rest);
        }
        assert!(!seen.contains(&true), "nothing landed yet");
        press(&mut s, KeyCode::ArrowRight, now);
        let mut seen = Vec::new();
        for _ in 0..80 {
            now += Duration::from_millis(16);
            seen.push(flashes(&frame(&mut s, now), &s) > rest);
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
    let lw = s.select.list_w(c.width);
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
        // The section it's in (the column's last) isn't the lane's.
        .filter(|sh| sh.params[0] != style::radius::CARD)
        .collect();
    let found: Vec<[f32; 4]> = strips.iter().map(|sh| rect(sh)).collect();
    assert_eq!(strips.len(), 1, "one strip: {found:?}");
    let strip = strips[0];
    assert_eq!(strip.params[0], style::radius::SMALL, "its corners round");
    assert!(
        strip.params[1] == 0.0 || strip.border[3] == 0.0,
        "no line round it: its tone apart from the panel's"
    );
    let [left, y, w, h] = rect(strip);
    let right = left + w;
    // From the lane's start (no drum left of it, out past the details' edge), on past the lane's
    // end, so it's round there too (the lane itself is cut square to its box).
    let (before, after) = (lane_hit[0] - left, right - (lane_hit[0] + lane_hit[2]));
    assert!(
        before.abs() < 0.01 && after > 4.0,
        "{before} before the lane, {after} after it"
    );
    // Its notes run on to the tile's end and come in out of it, not out of its middle.
    let notes_end = preview::bare_notes_end(lane_hit[0] + lane_hit[2]);
    assert!(
        (notes_end - right).abs() < 0.5,
        "notes to {notes_end}, the tile to {right}"
    );
    // No lines along it or across its ends (the lane's corridor in it has its own two).
    let edge = render::rgba(style::PANEL_EDGE, 1.0);
    let [_, cy, _, ch] = s.preview.corridor().expect("the lane's corridor");
    let lines: Vec<[f32; 4]> = c
        .shapes
        .iter()
        .filter(right_of_list)
        .filter(|sh| same_rgb(sh.fill, edge))
        .map(rect)
        .filter(|r| r[1] < y + h + 2.0 && r[1] + r[3] > y - 2.0)
        .filter(|r| r[0] < right + 2.0 && r[0] + r[2] > left - 2.0)
        .filter(|r| (r[1] - cy).abs() > 0.01 && (r[1] + r[3] - (cy + ch)).abs() > 0.01)
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
    let lw = s.select.list_w(c.width);
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
    // Where the menu's note lands is on the screen: in song select's units here.
    let (at, r) = s.play_icon_at.expect("where the menu's note lands");
    let k = s.select_scale;
    let (at, r) = ([at[0] / k - s.select_left, at[1] / k], r / k);
    assert!(inside(at, play, 0.0) && r > 10.0, "{at:?} {r} in {play:?}");
    // The note itself: its white rim round where the flying note lands.
    let rim = c
        .shapes
        .iter()
        .filter(|sh| same_rgb(sh.fill, render::rgba(0xffffff, 1.0)) && inside(sh.center, play, 0.0))
        .filter(|sh| (sh.center[0] - at[0]).hypot(sh.center[1] - at[1]) <= r * 1.2)
        .count();
    assert!(rim > 20, "the Play note's pixels: {rim}");
    // With animations on too, nothing plays first: the map is loading on the press.
    press(&mut s, KeyCode::Enter, now);
    assert_eq!(scene_name(&s), "Loading", "the map at once");
}

#[test]
fn without_animations_play_starts_the_map_at_once() {
    let (mut s, _songs, mut now) = menu_look("select-look-play-cut");
    settled(&mut s, &mut now);
    press(&mut s, KeyCode::Enter, now);
    assert_eq!(scene_name(&s), "Loading");
}

#[test]
fn mods_random_and_play_are_blocks_of_one_height_and_the_mods_on_show_beside_mods() {
    let near_rect = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.01);
    let (mut s, _songs, mut now) = menu_look("select-look-mod-badges");
    let c = settled(&mut s, &mut now);
    let mods = |c: &Canvas| hit_rect(c, |h| *h == Hit::ModsButton).expect("Mods");
    let random = |c: &Canvas| hit_rect(c, |h| matches!(h, Hit::Key(KeyCode::F2))).expect("Random");
    let play = hit_rect(&c, |h| matches!(h, Hit::Key(KeyCode::Enter))).expect("Play");
    // No strip round them: the row under the column, from its left edge.
    let lane = sections(&c, &s)[0];
    // Mods from 8 in from the column's start, Random after it, Play at the end: one height, one
    // middle line, each a block of its own (what's drawn is what takes the click).
    let (m, r) = (mods(&c), random(&c));
    assert!(
        (m[0] - lane[0] - 8.0).abs() < 0.5,
        "Mods 8 in: {m:?} in {lane:?}"
    );
    for b in [m, r] {
        assert!(
            (b[1] - play[1]).abs() < 0.5 && (b[3] - play[3]).abs() < 0.5,
            "as tall as Play, on its line: {b:?} {play:?}"
        );
        let drawn = c.shapes.iter().any(|sh| {
            sh.params[0] == style::radius::CONTROL
                && sh.params[1] == style::tone::LINE
                && near_rect(rect(sh), b)
        });
        assert!(drawn, "a block under {b:?}");
    }
    assert!(
        m[0] + m[2] < r[0] && r[0] + r[2] < play[0],
        "in order: {m:?} {r:?} {play:?}"
    );
    let at_rest = |c: &Canvas, b: [f32; 4]| {
        c.shapes.iter().any(|sh| {
            near_rect(rect(sh), b) && same_rgb(sh.fill, render::rgba(style::tone::CONTROL, 1.0))
        })
    };
    assert!(at_rest(&c, m), "Mods at rest with no mods on");
    // Mods on: Mods and Random stay where they were (Random keeps its word), Mods is lit, and
    // the mods' badges are a group in the room between Random and Play, on the same line.
    s.cfg.mods = crate::mods::Mods::parse("HD,HR");
    s.select.mods_label = s.cfg.mods.label();
    let c = settled(&mut s, &mut now);
    let (m2, r2) = (mods(&c), random(&c));
    assert!(
        (m2[0] - m[0]).abs() < 0.01 && (m2[1] - m[1]).abs() < 0.01,
        "{m:?} -> {m2:?}"
    );
    assert!(!at_rest(&c, m2), "Mods lit with mods on");
    assert_eq!(r2, r, "Random where it was, whole");
    assert!(
        texts_in(&c, r2).iter().any(|t| t == "Random"),
        "Random keeps its word: {:?}",
        texts_in(&c, r2)
    );
    let mid = m[1] + m[3] / 2.0;
    let mut xs = Vec::new();
    for name in ["HD", "HR"] {
        let [x, y] = text_at(&c, name).unwrap_or_else(|| panic!("{name}"));
        assert!(
            x > r2[0] + r2[2] && x < play[0],
            "{name} between Random and Play: {x}"
        );
        assert!((y - mid).abs() < 6.0, "{name} on the line: {y} by {mid}");
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
        xs.push(x);
    }
    assert!((xs[1] - xs[0]).abs() > 4.0, "side by side, apart: {xs:?}");
}

#[test]
fn many_mods_on_show_four_badges_and_a_count_of_the_rest_with_all_of_them_under_the_pointer() {
    let (mut s, _songs, mut now) = menu_look("select-look-many-mods");
    s.cfg.mods = crate::mods::Mods::parse("HD,HR,NF,FL,PR,SW,AL");
    s.select.mods_label = s.cfg.mods.label();
    let c = settled(&mut s, &mut now);
    let random = hit_rect(&c, |h| matches!(h, Hit::Key(KeyCode::F2))).expect("Random");
    let play = hit_rect(&c, |h| matches!(h, Hit::Key(KeyCode::Enter))).expect("Play");
    assert!(
        texts_in(&c, random).iter().any(|t| t == "Random"),
        "Random whole beside many mods"
    );
    // The badges between Random and Play: four of them and a +2, none touching the next.
    let row: Vec<(String, [f32; 4])> = text_boxes(&c)
        .into_iter()
        .filter(|(_, r)| r[0] > random[0] + random[2] && r[0] + r[2] < play[0])
        .filter(|(_, r)| r[1] >= play[1] && r[1] + r[3] <= play[1] + play[3])
        .collect();
    let names: Vec<&str> = row.iter().map(|(t, _)| t.as_str()).collect();
    let on = s.cfg.mods.iter().count();
    assert!(on > 5, "more mods on than show: {on}");
    assert_eq!(names.len(), 5, "four and a count: {names:?}");
    assert_eq!(names[4], format!("+{}", on - 4), "{names:?}");
    let badges: Vec<[f32; 4]> = c
        .shapes
        .iter()
        .filter(|sh| badge(sh))
        .map(rect)
        .filter(|r| r[0] > random[0] + random[2] && r[0] + r[2] < play[0])
        .collect();
    assert_eq!(badges.len(), 5, "{badges:?}");
    let mut by_x = badges.clone();
    by_x.sort_by(|a, b| a[0].total_cmp(&b[0]));
    for w in by_x.windows(2) {
        assert!(w[1][0] - (w[0][0] + w[0][2]) >= 4.0, "apart: {by_x:?}");
    }
    // Under the pointer: every mod that's on, by name, over the row.
    let first = by_x[0];
    let (px, py) = to_pixels(&s, first[0] + first[2] / 2.0, first[1] + first[3] / 2.0);
    s.mouse_moved(px, py);
    s.mouse = (px, py);
    for _ in 0..2 {
        now += Duration::from_millis(16);
        frame(&mut s, now);
    }
    now += Duration::from_millis(16);
    let c = frame(&mut s, now);
    let all: Vec<String> = c.texts.iter().map(|(_, t)| t.clone()).collect();
    for m in s.cfg.mods.iter() {
        assert!(
            all.iter()
                .any(|t| t.starts_with(m.acronym()) && t.contains(m.name())),
            "{} in the tip: {all:?}",
            m.name()
        );
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
fn the_right_column_s_sections_stay_put_from_map_to_map() {
    let (mut s, _songs, mut now) = menu_look("select-look-sections-stay");
    let c = settled(&mut s, &mut now);
    let first = sections(&c, &s);
    assert_eq!(first.len(), 3);
    // Every other map (titles, mappers and stats of their own), one of them with plays on its
    // leaderboard, and the pp simulator open: the sections don't move or change size.
    for k in 0..maps::all().len() {
        if k == 2 {
            for (n, name) in ["Ann", "Bo", "Cy"].iter().enumerate() {
                let play = local_play(&s, name, 900_000 - n as u64 * 1000, 0.99);
                s.scores.push(play);
            }
        }
        let c = settled(&mut s, &mut now);
        assert_eq!(sections(&c, &s), first, "map {k}");
        press(&mut s, KeyCode::ArrowRight, now);
    }
    s.click(Hit::PpSim, None);
    let c = settled(&mut s, &mut now);
    assert!(s.preview.sim.is_some());
    assert_eq!(sections(&c, &s), first, "with the simulator open");
    // On either board's tab too.
    for online in [false, true] {
        let c = on_board(&mut s, &mut now, online);
        assert_eq!(sections(&c, &s), first, "on the board (online: {online})");
    }
}

#[test]
fn the_search_row_has_a_glass_a_quiet_hint_its_key_at_its_end_and_the_counts_on_a_chip() {
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
    // Quiet: on no cap of its own.
    let cap = c.shapes.iter().any(|sh| {
        plain(sh)
            && inside(key.map(|v| v + 1.0), rect(sh), 0.0)
            && rect(sh)[2] < 40.0
            && rect(sh)[2] > 8.0
    });
    assert!(!cap, "the key on a cap");
    // The counts on a chip of their own beside the search row, not floating at the screen's
    // right edge.
    let counts = c
        .texts
        .iter()
        .find(|(_, t)| t.contains(" songs, "))
        .and_then(|(r, _)| c.shapes.get(r.start))
        .map(rect)
        .expect("the counts");
    let lw = s.select.list_w(c.width);
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
fn the_search_row_sits_under_the_performance_readout_its_buttons_blocks_of_one_height() {
    let (mut s, _songs, mut now) = menu_look("select-look-under-readout");
    let c = settled(&mut s, &mut now);
    let menu = hit_rect(&c, |h| *h == Hit::ToMenu).expect("Menu");
    let search = hit_rect(&c, |h| *h == Hit::TextField).expect("the search");
    let sort = hit_rect(&c, |h| *h == Hit::SortList).expect("Sort");
    // The readout (`show_fps`) takes 6 to 22 at the top left: the row starts below it.
    for r in [menu, search, sort] {
        assert!(r[1] >= 24.0, "below the readout: {r:?}");
        assert!(
            (r[1] + r[3] / 2.0 - (menu[1] + menu[3] / 2.0)).abs() < 0.5,
            "one row: {r:?} {menu:?}"
        );
    }
    for b in [menu, sort] {
        assert!(
            (b[3] - style::BLOCK_H).abs() < 0.01,
            "a block's height: {b:?}"
        );
        let drawn = c.shapes.iter().any(|sh| {
            sh.params[0] == style::radius::CONTROL
                && sh.params[1] == style::tone::LINE
                && rect(sh).iter().zip(b).all(|(a, b)| (a - b).abs() < 0.01)
        });
        assert!(drawn, "a block under {b:?}");
    }
    // The list starts under the row's strip, not into it.
    let top = cards(&c)
        .into_iter()
        .map(|(_, r)| r[1])
        .fold(f32::MAX, f32::min);
    assert!(
        top > menu[1] + menu[3] + 4.0,
        "the list under the row: {top}"
    );
}

#[test]
fn play_and_the_row_of_actions_keep_the_screen_s_margin_at_the_foot_of_the_column_s_panel() {
    let (mut s, _songs, mut now) = menu_look("select-look-play-margin");
    let c = settled(&mut s, &mut now);
    let play = hit_rect(&c, |h| *h == Hit::Key(KeyCode::Enter)).expect("Play");
    let column = sections(&c, &s);
    let last = column[column.len() - 1];
    // Under the column, level with its edges: Play ends 8 in from the right of it (the
    // screen's margin past that: 28 in the interface's space, clear of the friends tab),
    // under its last section.
    let right = c.width - 28.0;
    assert!(
        (last[0] + last[2] - right).abs() < 0.5,
        "the column to the margin: {last:?}"
    );
    assert!(
        (right - 8.0 - (play[0] + play[2])).abs() < 0.5,
        "Play 8 in from the margin: {play:?}"
    );
    assert!(play[1] > last[1] + last[3], "under the column: {play:?}");
    // No strip of their own round the buttons: they stand at the foot of the column's one
    // panel, under the line closing its last section, 8 up from its bottom, which is level
    // with the list's.
    let fill = style::surface_fill(style::Surface::Raised);
    let strips: Vec<[f32; 4]> = c
        .shapes
        .iter()
        .filter(|sh| plain(sh) && same_rgb(sh.fill, fill))
        .map(rect)
        .filter(|r| inside([play[0] + 1.0, play[1] + play[3] / 2.0], *r, 0.0))
        .filter(|r| r[2] > play[2] + 1.0)
        .collect();
    assert!(strips.is_empty(), "a strip round the actions: {strips:?}");
    let panel = column_panel(&c, &s).expect("the column's panel");
    let [_, end] = s.select.frame_span();
    assert!(
        (panel[1] + panel[3] - end).abs() < 0.5,
        "the panel down to the list's bottom: {panel:?} to {end}"
    );
    assert!(
        (panel[1] + panel[3] - 8.0 - (play[1] + play[3])).abs() < 0.5,
        "Play 8 up from its bottom: {play:?} in {panel:?}"
    );
    let rules = column_rules(&c, &s);
    assert_eq!(
        rules.len(),
        2,
        "a line under the details and one over the actions"
    );
    assert!(
        rules[1][1] < play[1],
        "the line over the actions: {rules:?} {play:?}"
    );
}

#[test]
fn the_right_column_s_panel_starts_level_with_the_search_row() {
    let (mut s, _songs, mut now) = menu_look("select-look-column-top");
    let c = settled(&mut s, &mut now);
    let search = hit_rect(&c, |h| *h == Hit::TextField).expect("the search");
    let panel = column_panel(&c, &s).expect("the column's panel");
    assert!(
        (panel[1] - search[1]).abs() < 0.5,
        "no hole over the column: {panel:?}, the search at {search:?}"
    );
    // The details start at the panel's top.
    assert!((sections(&c, &s)[0][1] - panel[1]).abs() < 0.01);
}

#[test]
fn the_right_column_is_the_details_a_row_of_tabs_and_the_tab_shown_under_them() {
    let (mut s, _songs, mut now) = menu_look("select-look-right-panel");
    let c = settled(&mut s, &mut now);
    let column = sections(&c, &s);
    assert_eq!(column.len(), 3, "{column:?}");
    let [head, tabs, body] = [column[0], column[1], column[2]];
    // One above the other, as wide as each other, to the screen's margin.
    for r in [tabs, body] {
        assert!(
            (r[0] - head[0]).abs() < 0.01 && (r[2] - head[2]).abs() < 0.01,
            "{column:?}"
        );
    }
    // (28 in the interface's space: clear of the friends tab at the screen's edge.)
    assert!(
        (head[0] + head[2] - (c.width - 28.0)).abs() < 0.5,
        "{head:?}"
    );
    // The details as tall as what's on them, no fixed share and no empty band; the body all
    // the rest, much the tallest.
    assert!(body[3] > head[3], "the body has the room: {column:?}");
    let pp = hit_rect(&c, |h| *h == Hit::PpSim).expect("the pp button");
    assert!(
        head[1] + head[3] - (pp[1] + pp[3]) < 8.0,
        "nothing under the pp: {pp:?} in {head:?}"
    );
    // The title on the details; Preview, Local, Online and the website on the row of tabs.
    let title = s.library[s.select.selected().unwrap()].meta.title.clone();
    assert!(
        c.texts.iter().any(|(r, t)| t.trim() == title
            && c.shapes
                .get(r.start)
                .is_some_and(|sh| inside(sh.center, head, 0.0))),
        "the title on the details"
    );
    let tab_hits = [
        Hit::PreviewTab,
        Hit::BoardTab(false),
        Hit::BoardTab(true),
        Hit::BoardWebsite,
    ];
    let mut last_x = f32::MIN;
    for hit in tab_hits {
        let r = hit_rect(&c, |h| *h == hit).expect("a tab");
        assert!(inside([r[0], r[1]], tabs, 0.0), "{r:?} in {tabs:?}");
        assert!(r[0] > last_x, "in order");
        last_x = r[0];
    }
    // At first the preview: the lane in the body, no leaderboard's rows.
    let lane = hit_rect(&c, |h| *h == Hit::Preview).expect("the preview lane");
    assert!(
        inside([lane[0], lane[1]], body, 0.0) && lane[0] + lane[2] <= body[0] + body[2],
        "{lane:?} in {body:?}"
    );
    assert!(s.board_rect.is_none(), "no board under the preview");
    // A board's tab: the body is the board's, no lane in it; the sections stay where they were.
    let c = on_board(&mut s, &mut now, false);
    assert_eq!(sections(&c, &s), column);
    assert!(hit_rect(&c, |h| *h == Hit::Preview).is_none(), "no lane");
    assert!(
        texts_in(&c, body)
            .iter()
            .any(|t| t == "No plays on this map yet"),
        "the local board in the body"
    );
    // No way to a leaderboard page: it's on the screen.
    assert!(hit_rect(&c, |h| *h == Hit::BoardPage).is_none());
    assert!(!c.texts.iter().any(|(_, t)| t.trim() == "Leaderboard"));
}

#[test]
fn the_tab_shown_stays_from_map_to_map_and_song_select_opens_on_the_preview() {
    let (mut s, _songs, mut now) = menu_look("select-look-tab-kept");
    let c = settled(&mut s, &mut now);
    assert!(
        hit_rect(&c, |h| *h == Hit::Preview).is_some(),
        "Preview first"
    );
    on_board(&mut s, &mut now, true);
    for _ in 0..2 {
        press(&mut s, KeyCode::ArrowRight, now);
        let c = settled(&mut s, &mut now);
        assert!(
            hit_rect(&c, |h| *h == Hit::Preview).is_none(),
            "still the board on another map"
        );
        assert!(s.board_online, "the online one");
    }
    // Back to the preview's tab, then away from song select and back: the preview again.
    s.click(Hit::PreviewTab, None);
    let c = settled(&mut s, &mut now);
    assert!(
        hit_rect(&c, |h| *h == Hit::Preview).is_some(),
        "the preview's tab"
    );
    on_board(&mut s, &mut now, false);
    s.show_select();
    let c = settled(&mut s, &mut now);
    assert!(
        hit_rect(&c, |h| *h == Hit::Preview).is_some(),
        "song select opens on the preview"
    );
}

#[test]
fn the_preview_s_tab_gives_the_lane_room_and_the_density_graph_height() {
    let (mut s, _songs, mut now) = menu_look("select-look-preview-tab");
    let c = settled(&mut s, &mut now);
    let body = sections(&c, &s)[2];
    let [_, ly, _, lane_h] = s.preview.box_rect().expect("the lane's box");
    // The density graph between the lane's box and its times under it: well taller than the
    // bare timeline's 28, the lane's box taller still.
    let [_, times] = text_at(&c, "0:00 / 0:10").expect("the timeline's times");
    let gh = times - (ly + lane_h) - 8.0;
    assert!(gh >= 60.0, "a graph to read: {gh}");
    assert!(lane_h > gh, "the lane's box over it: {lane_h} vs {gh}");
    // All within the body, near all of it used.
    assert!(times < body[1] + body[3], "{times} in {body:?}");
    assert!(
        lane_h + gh > 0.6 * body[3],
        "the body is the preview's: {lane_h}+{gh} of {body:?}"
    );
    // A click on it still goes fullscreen (the lane takes the click).
    let lane = hit_rect(&c, |h| *h == Hit::Preview).expect("the lane");
    assert!(inside([lane[0], lane[1]], body, 0.0));
}

#[test]
fn the_stats_are_one_dense_line_under_the_difficulty() {
    let (mut s, _songs, mut now) = menu_look("select-look-stat-line");
    let c = settled(&mut s, &mut now);
    let head = sections(&c, &s)[0];
    let boxes = text_boxes(&c);
    let at = |name: &str| {
        boxes
            .iter()
            .find(|(t, r)| t == name && inside([r[0], r[1]], head, 0.0))
            .map(|(_, r)| *r)
            .unwrap_or_else(|| panic!("{name} in the details"))
    };
    let caps: Vec<[f32; 4]> = ["OD", "BPM", "LENGTH", "NOTES"]
        .iter()
        .map(|n| at(n))
        .collect();
    // One line, left to right, small: no taller than the body text's line.
    for w in caps.windows(2) {
        assert!(
            ((w[0][1] + w[0][3] / 2.0) - (w[1][1] + w[1][3] / 2.0)).abs() < 1.0,
            "one line: {caps:?}"
        );
        assert!(w[1][0] > w[0][0] + w[0][2], "in order: {caps:?}");
    }
    let title = s.library[s.select.selected().unwrap()].meta.title.clone();
    let title_h = at(&title)[3];
    assert!(
        caps.iter().all(|r| r[3] < title_h * 0.8),
        "small next to the title ({title_h}): {caps:?}"
    );
    // No tiles under them.
    let fill = style::surface_fill(style::Surface::Raised);
    let tiles = c
        .shapes
        .iter()
        .filter(|sh| plain(sh) && same_rgb(sh.fill, fill) && inside(sh.center, caps[0], 2.0))
        .count();
    assert_eq!(tiles, 0, "no tile under OD");
}

#[test]
fn the_plays_are_on_the_leaderboard_section_under_the_details() {
    let (mut s, _songs, mut now) = menu_look("select-look-board-rows");
    for (k, name) in ["Ann", "Bo"].iter().enumerate() {
        let play = local_play(&s, name, 900_000 - k as u64 * 1000, 0.99);
        s.scores.push(play);
    }
    let c = on_board(&mut s, &mut now, false);
    let column = sections(&c, &s);
    let board = column[2];
    // Each play a row of its own on the board's section, best first.
    let rows: Vec<[f32; 4]> = (0..2)
        .map(|k| hit_rect(&c, |h| *h == Hit::ScoreRow(k)).expect("a row per play"))
        .collect();
    for r in &rows {
        assert!(
            inside([r[0], r[1]], board, 0.0) && r[1] + r[3] <= board[1] + board[3],
            "{r:?} in {board:?}"
        );
    }
    assert!(rows[0][1] < rows[1][1]);
    for (name, r) in [("Ann", rows[0]), ("Bo", rows[1])] {
        assert!(
            texts_in(&c, r).iter().any(|t| t == name),
            "{name} on its row: {:?}",
            texts_in(&c, r)
        );
    }
    // Not on the details: nothing of the board over them.
    let head = texts_in(&c, column[0]);
    assert!(!head.iter().any(|t| t == "Ann" || t == "Bo"), "{head:?}");
    // Both fit: nothing pinned under them.
    assert!(!texts_in(&c, board).iter().any(|t| t == "PB"));
}

#[test]
fn the_players_best_is_pinned_under_the_rows_when_not_all_of_them_fit() {
    let (mut s, _songs, mut now) = menu_look("select-look-board-pinned");
    for k in 0..30u64 {
        let play = local_play(&s, "Ann", 900_000 - k * 1000, 0.99 - k as f64 * 0.001);
        s.scores.push(play);
    }
    let c = on_board(&mut s, &mut now, false);
    let board = sections(&c, &s)[2];
    let rows: Vec<[f32; 4]> = c
        .hits
        .iter()
        .filter(|h| matches!(h.hit, Hit::ScoreRow(..)))
        .map(|h| h.rect)
        .collect();
    assert!(
        rows.len() >= 6 && rows.len() < 30,
        "five plays at least and the best: {}",
        rows.len()
    );
    // The last row is the best, pinned at the section's bottom under the others, marked PB.
    let pinned = hit_rect(&c, |h| *h == Hit::ScoreRow(rows.len() - 1)).expect("pinned");
    assert!(
        rows.iter().all(|r| r[1] < pinned[1] || *r == pinned),
        "under the rest"
    );
    assert!(
        board[1] + board[3] - (pinned[1] + pinned[3]) < 12.0,
        "at the bottom"
    );
    assert!(texts_in(&c, pinned).iter().any(|t| t == "PB"));
    assert_eq!(
        s.leaderboard.len(),
        rows.len(),
        "a click on each opens its play"
    );
    // Scrolled to the end, the best stays pinned.
    let [bx, by, bw, bh] = s.board_rect.expect("the rows' box");
    let k = s.window_h / render::VIRTUAL_HEIGHT;
    s.mouse = (((bx + bw / 2.0) * k) as f64, ((by + bh / 2.0) * k) as f64);
    s.wheel(-100);
    let c = settled(&mut s, &mut now);
    let again = hit_rect(&c, |h| *h == Hit::ScoreRow(rows.len() - 1)).expect("pinned");
    assert_eq!(again, pinned);
    assert!(texts_in(&c, again).iter().any(|t| t == "PB"));
}

#[test]
fn an_empty_local_board_says_so_calmly_and_offers_the_online_one() {
    let (mut s, _songs, mut now) = menu_look("select-look-board-empty");
    let c = on_board(&mut s, &mut now, false);
    let board = sections(&c, &s)[2];
    let shown = texts_in(&c, board);
    assert!(
        shown.iter().any(|t| t == "No plays on this map yet"),
        "{shown:?}"
    );
    // A button over to the online board, besides its tab.
    let tabs: Vec<[f32; 4]> = c
        .hits
        .iter()
        .filter(|h| h.hit == Hit::BoardTab(true))
        .map(|h| h.rect)
        .collect();
    assert_eq!(tabs.len(), 2, "the tab and the button: {tabs:?}");
    let button = tabs.iter().max_by(|a, b| a[1].total_cmp(&b[1])).unwrap();
    assert!(inside([button[0], button[1]], board, 0.0));
    assert!(
        texts_in(&c, *button)
            .iter()
            .any(|t| t == "Show online scores"),
        "{:?}",
        texts_in(&c, *button)
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
    let shown = texts_in(&c, [0.0, 0.0, c.width, c.height]);
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
        texts_in(&c, [0.0, 0.0, c.width, c.height])
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
fn the_full_combo_pp_is_one_button_across_its_row() {
    let (mut s, _songs, mut now) = menu_look("select-look-pp-row");
    let c = settled(&mut s, &mut now);
    let head = sections(&c, &s)[0];
    let pp = hit_rect(&c, |h| *h == Hit::PpSim).expect("the pp button");
    // The row is the button's alone (no way to a leaderboard page at its end): across the
    // details, as far in from the section's right edge as its left.
    assert!(hit_rect(&c, |h| *h == Hit::BoardPage).is_none());
    let (left_in, right_in) = (pp[0] - head[0], head[0] + head[2] - (pp[0] + pp[2]));
    assert!(
        (left_in - right_in).abs() < 0.5,
        "across the row: {pp:?} in {head:?}"
    );
    // A block of the buttons' tone, no line round it at rest (a strip of numbers in a box read
    // as a table's cell; the edge comes under the pointer).
    let block = c.shapes.iter().any(|sh| {
        same_rgb(sh.fill, render::rgba(style::tone::CONTROL, 1.0))
            && sh.params[0] == style::radius::CONTROL
            && sh.params[1] == 0.0
            && inside([pp[0] + pp[2] / 2.0, pp[1] + pp[3] / 2.0], rect(sh), 0.0)
    });
    assert!(block, "on a block");
    let shown = texts_in(&c, pp);
    for want in ["SS", "99%", "98%"] {
        assert!(shown.iter().any(|t| t == want), "{want} on it: {shown:?}");
    }
    assert!(
        !texts_in(&c, [0.0, 0.0, c.width, c.height])
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
    let lw = s.select.list_w(c.width);
    let panel_left = sections(&c, &s)[0][0];
    assert!(panel_left > 24.0 + lw);
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
    // (In the details' section the button goes up into the DON / KAT bar's place, which the
    // play's fields take.)
    let button = hit_rect(&c, |h| *h == Hit::PpSim).expect("the pp button");
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
    let lw = s.select.list_w(before.width);
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
    let (_, widest) = s.select.share_range_here(c.width).expect("a range");
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
            // The details' section, as far in from its right edge as the button's left.
            let head = sections(&c, &s)[0];
            let panel_right = head[0] + head[2] - (button[0] - head[0]) + 1.0;
            let boxes: Vec<(String, [f32; 4])> = text_boxes(&c)
                .into_iter()
                .filter(|(_, r)| {
                    let mid = [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0];
                    inside(mid, [area[0], area[1], panel_right - area[0], area[3]], 0.0)
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
    // In the interface's own space the panel is never narrower than `PANEL_MIN_UI`: room for
    // all three there.
    assert_eq!(fewest, 3, "the narrowest panel shows every value");
}

#[test]
fn the_leaderboard_stays_in_place_on_a_song_change() {
    let (mut s, _songs, mut now) = menu_look_moving("select-look-board-stays");
    let label = |c: &Canvas| {
        let tab = hit_rect(c, |h| *h == Hit::BoardTab(false)).expect("the Local tab");
        let (range, _) = c
            .texts
            .iter()
            .find(|(r, t)| t.trim() == "Local" && inside(c.shapes[r.start].center, tab, 2.0))
            .cloned()
            .expect("its label");
        c.shapes[range.start]
    };
    let rest = label(&settled(&mut s, &mut now));
    press(&mut s, KeyCode::ArrowRight, now);
    // While the details change, the board's tabs neither move nor fade: the section is the
    // same, only its rows are the new map's.
    for _ in 0..12 {
        now += Duration::from_millis(24);
        let l = label(&frame(&mut s, now));
        assert_eq!(l.center, rest.center);
        assert_eq!(l.fill[3], rest.fill[3]);
    }
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
fn the_volume_mixer_comes_up_opaque_above_the_lane_of_actions() {
    let (mut s, _songs, mut now) = menu_look("select-look-mixer");
    s.volume.touch(now);
    let c = settled(&mut s, &mut now);
    // The mixer's panel: the menu's tone, all but opaque (an overlay's).
    let mixer = c
        .shapes
        .iter()
        .filter(|sh| same_rgb(sh.fill, render::rgba(style::tone::PANEL, 1.0)))
        .filter(|sh| sh.fill[3] >= 0.97)
        .filter(|sh| sh.half[0] * 2.0 > 200.0 && sh.half[1] * 2.0 > 90.0)
        .filter(|sh| sh.center[0] > c.width / 2.0)
        .map(rect)
        .next_back()
        .expect("the mixer's panel, all but opaque");
    let overlaps = |a: [f32; 4], b: [f32; 4]| {
        a[0] < b[0] + b[2] && b[0] < a[0] + a[2] && a[1] < b[1] + b[3] && b[1] < a[1] + a[3]
    };
    for hit in [
        Hit::Key(KeyCode::Enter),
        Hit::ModsButton,
        Hit::Key(KeyCode::F2),
    ] {
        let at = hit_rect(&c, |h| *h == hit).unwrap_or_else(|| panic!("{hit:?} on the lane"));
        assert!(
            !overlaps(mixer, at),
            "the mixer {mixer:?} over {hit:?} at {at:?}"
        );
    }
}

#[test]
fn song_select_s_routes_are_as_sharp_as_the_menu_s_in_every_layout() {
    let (mut s, _songs, mut now) = menu_look_moving("select-look-routes-sharp");
    let c = settled(&mut s, &mut now);
    assert!(c.blur.is_none(), "the main menu's look: {:?}", c.blur);
    let songs = Songs::new("select-look-routes-sharp-legacy", &maps::all());
    let mut legacy = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut legacy, &mut now);
    let c = settled(&mut legacy, &mut now);
    assert!(c.blur.is_none(), "Legacy: {:?}", c.blur);
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
    let point = |s: &mut State, r: [f32; 4]| {
        // Its middle: the value, between its - and +.
        s.mouse = to_pixels(s, r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
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

#[test]
fn the_preview_lane_fits_its_round_tile_and_its_notes_slide_in_from_under_its_edge() {
    let plain = |center: [f32; 2], half: [f32; 2], params: [f32; 4]| Shape {
        center,
        half,
        fill: [1.0; 4],
        border: [1.0; 4],
        params,
        angle: 0.0,
    };
    let (x, end, h) = (100.0, 400.0, 40.0);
    let mut shapes = vec![
        // Something before the lane, left alone.
        plain([50.0, 20.0], [50.0, 20.0], [0.0; 4]),
        // The lane's band, from its drum to the end, and a line along its top.
        plain([250.0, 20.0], [150.0, 20.0], [0.0; 4]),
        plain([250.0, 0.5], [150.0, 0.5], [0.0; 4]),
        // The drum's dark panel, square, from the lane's start.
        plain([x + 25.0, 20.0], [25.0, 20.0], [0.0; 4]),
        // A note at the very end, one coming in, and one well inside.
        plain([end - 2.0, 20.0], [12.0, 12.0], [12.0, 0.0, 0.0, 0.0]),
        plain([end - 14.0, 20.0], [12.0, 12.0], [12.0, 0.0, 0.0, 0.0]),
        plain([end - 120.0, 20.0], [12.0, 12.0], [12.0, 0.0, 0.0, 0.0]),
    ];
    // The drum shown: the tile starts with it.
    let mut drum = shapes.clone();
    preview::bare_lane_end(&mut drum, 1, x, x, end, h);
    let round = style::radius::SMALL;
    assert_eq!(
        drum[3].params[0], round,
        "the drum's panel round at the tile's edge"
    );
    assert_eq!(drum[3].fill[3], 1.0, "and drawn");
    assert_eq!(drum[4].params[0], 0.0, "its square half");
    assert!(
        (drum[4].center[0] - drum[4].half[0] - (x + 25.0)).abs() < 0.01
            && (drum[4].center[0] + drum[4].half[0] - (x + 50.0)).abs() < 0.01,
        "over its right half"
    );
    // The drum off: the tile starts right of it.
    let tile_left = x + 50.0;
    preview::bare_lane_end(&mut shapes, 1, x, tile_left, end, h);
    let span = |sh: &Shape| [sh.center[0] - sh.half[0], sh.center[0] + sh.half[0]];
    assert_eq!(
        shapes[0].params[0], 0.0,
        "what's before the lane is left alone"
    );
    assert_eq!(shapes[1].params[0], round, "the band round");
    assert!(
        (span(&shapes[1])[0] - tile_left).abs() < 0.01 && (span(&shapes[1])[1] - end).abs() < 0.01,
        "the band from the tile's left edge (right of the drum) to its right one: {:?}",
        span(&shapes[1])
    );
    assert!(
        shapes[2].fill[3] == 0.0 && shapes[2].border[3] == 0.0,
        "no line along it: the tile is its frame"
    );
    // The drum's panel, left of the tile: not drawn.
    assert!(
        shapes[3].fill[3] == 0.0 && shapes[3].border[3] == 0.0,
        "no drum panel"
    );
    let a = |i: usize| shapes[i].fill[3];
    // The notes aren't faded near the end: they slide in from under the tile's edge, cut by it
    // (faded in short of it, they seemed to appear in plain view).
    for i in [4, 5, 6] {
        assert_eq!(a(i), 1.0, "note {i} drawn as it is");
    }
    assert_eq!(a(1), 1.0, "the band drawn as it is");
}

#[test]
fn the_preview_never_draws_the_black_playfield_and_plays_keep_it() {
    use crate::game::{self, Look};
    game::set_frame_look(Some(Look {
        black_lane: true,
        ..Look::default()
    }));
    let drawn = game::with_look(preview::preview_look(game::playfield_look()), || {
        game::playfield_look()
    });
    assert!(!drawn.black_lane, "the preview's lane isn't black");
    assert!(
        game::playfield_look().black_lane,
        "the play's look is as it was after the preview"
    );
    game::set_frame_look(None);
}

#[test]
fn a_difficulty_rows_rating_bar_is_drawn_over_its_light_and_the_light_is_as_round_as_it() {
    // The selected row's light (the pointer's is drawn the same way, as round).
    let (mut s, _songs, mut now) = menu_look("select-look-diff-light");
    let c = settled(&mut s, &mut now);
    let cursor = s.select.cursor();
    let row = hit_rect(&c, |h| *h == Hit::SongRow(cursor)).expect("the selected row");
    // The row's own shapes, the full-width ones (its card and its light) and its bar.
    let at_row = |sh: &&Shape| inside(sh.center, row, 0.5);
    let wide: Vec<(usize, &Shape)> = c
        .shapes
        .iter()
        .enumerate()
        .filter(|(_, sh)| at_row(sh) && sh.half[0] * 2.0 > row[2] * 0.8)
        .collect();
    assert!(wide.len() >= 2, "its card and its light: {}", wide.len());
    // Its card, then its light over exactly the same box.
    let card = wide[0];
    let light = *wide[1..]
        .iter()
        .find(|(_, sh)| sh.center == card.1.center && sh.half == card.1.half)
        .expect("its light over the card");
    assert_eq!(
        light.1.params[0], card.1.params[0],
        "the light as round as the card"
    );
    let bar = c
        .shapes
        .iter()
        .enumerate()
        .find(|(_, sh)| at_row(sh) && sh.half[0] * 2.0 == 3.0)
        .expect("its rating bar");
    assert!(
        bar.0 > light.0,
        "the bar over the light, not under its corners"
    );
}

#[test]
fn the_pp_simulator_opens_within_the_details_section() {
    let (mut s, _songs, mut now) = menu_look("select-look-sim-room");
    settled(&mut s, &mut now);
    s.click(Hit::PpSim, None);
    let c = settled(&mut s, &mut now);
    let head = sections(&c, &s)[0];
    // Its fields and steps, and the button, all inside the details' section.
    let sim: Vec<[f32; 4]> = c
        .hits
        .iter()
        .filter(|h| matches!(h.hit, Hit::PpSim | Hit::PpSimStep(..) | Hit::PpSimField(..)))
        .map(|h| h.rect)
        .collect();
    assert!(
        sim.len() >= 7,
        "the button, two fields and four steps: {sim:?}"
    );
    for r in sim {
        assert!(
            r[0] >= head[0]
                && r[1] >= head[1]
                && r[0] + r[2] <= head[0] + head[2]
                && r[1] + r[3] <= head[1] + head[3],
            "{r:?} in {head:?}"
        );
    }
}

#[test]
fn everything_in_the_right_column_is_within_the_preview_tiles_edges_at_any_list_width() {
    let (mut s, _songs, mut now) = menu_look("select-look-page-edges");
    // The screen's width in song select's own units.
    let own = harness::WIDTH / s.select_scale - 2.0 * s.select_left;
    let (min, max) = s.select.share_range_here(own).expect("room for both");
    for (share, drum) in [min, crate::config::LIST_SHARE.2, max]
        .into_iter()
        .flat_map(|share| [(share, true), (share, false)])
    {
        s.cfg.song_list_share = share;
        s.cfg.preview_drum = drum;
        let c = settled(&mut s, &mut now);
        let column = sections(&c, &s);
        let page = {
            let (first, last) = (column[0], column[column.len() - 1]);
            [first[0], first[1], first[2], last[1] + last[3] - first[1]]
        };
        let lane_hit = hit_rect(&c, |h| *h == Hit::Preview).expect("the lane");
        let fill = style::surface_fill(style::Surface::Raised);
        let tile = c
            .shapes
            .iter()
            .filter(|sh| {
                same_rgb(sh.fill, fill)
                    && sh.params[0] == style::radius::SMALL
                    && sh.half[0] * 2.0 > 100.0
                    && inside(sh.center, lane_hit, 1.0)
            })
            .map(rect)
            .next()
            .expect("the lane's tile");
        let (left, right) = (tile[0], tile[0] + tile[2]);
        // Everything drawn on the column's sections (not the sections themselves or the
        // picture across the top) keeps within the lane tile's edges: the details, the
        // leaderboard and the preview line up, nothing sticks out, not by part of a pixel.
        let out: Vec<[f32; 4]> = c
            .shapes
            .iter()
            .filter(|sh| inside(sh.center, page, 0.0) && sh.half[0] * 2.0 < page[2] - 1.0)
            // (Not the screen's backdrop, taller than the page: the vignette down its side.)
            .filter(|sh| sh.half[1] * 2.0 <= page[3] + 1.0)
            .filter(|sh| sh.fill[3] > 0.0 || sh.border[3] > 0.0)
            .map(rect)
            .filter(|r| r[0] < left - 0.01 || r[0] + r[2] > right + 0.01)
            .collect();
        assert!(
            out.is_empty(),
            "share {share}: out of {left}..{right}: {out:?}"
        );
        // No text runs into another.
        let texts: Vec<(String, [f32; 4])> = c
            .texts
            .iter()
            .filter(|(r, _)| r.start < r.end)
            .filter(|(r, _)| inside(c.shapes[r.start].center, page, 0.0))
            .map(|(r, t)| {
                let rs: Vec<[f32; 4]> = c.shapes[r.clone()].iter().map(rect).collect();
                let x0 = rs.iter().map(|r| r[0]).fold(f32::MAX, f32::min);
                let x1 = rs.iter().map(|r| r[0] + r[2]).fold(f32::MIN, f32::max);
                let y0 = rs.iter().map(|r| r[1]).fold(f32::MAX, f32::min);
                let y1 = rs.iter().map(|r| r[1] + r[3]).fold(f32::MIN, f32::max);
                (t.clone(), [x0, y0, x1, y1])
            })
            .collect();
        for (i, (a, ra)) in texts.iter().enumerate() {
            for (b, rb) in &texts[i + 1..] {
                let apart = ra[2] <= rb[0] || rb[2] <= ra[0] || ra[3] <= rb[1] || rb[3] <= ra[1];
                assert!(apart, "share {share}: {a:?} {ra:?} runs into {b:?} {rb:?}");
            }
        }
        // And the details start on the same line as the lane: the title, the stats' line (OD
        // first), the timeline's times.
        let od = c
            .texts
            .iter()
            .filter(|(_, s)| s == "OD")
            .filter_map(|(r, _)| c.shapes.get(r.clone()))
            .map(|g| {
                let x0 = g.iter().map(|sh| rect(sh)[0]).fold(f32::MAX, f32::min);
                let x1 = g
                    .iter()
                    .map(|sh| rect(sh)[0] + rect(sh)[2])
                    .fold(f32::MIN, f32::max);
                let y = rect(&g[0])[1];
                [x0, x1, y]
            })
            .find(|[x0, _, y]| inside([*x0, *y], page, 0.0))
            .expect("OD");
        assert!(
            od[0] >= left - 0.01 && od[0] < left + 2.0,
            "share {share}: the stats from {}, the lane from {left}",
            od[0]
        );
        for t in ["Regression", "0:00 / 0:10"] {
            let at = c
                .texts
                .iter()
                .filter(|(_, s)| s.starts_with(t))
                .filter_map(|(r, _)| c.shapes.get(r.start))
                .filter(|sh| inside(sh.center, page, 0.0))
                .map(rect)
                .next()
                .expect(t);
            assert!(
                at[0] >= left - 0.01 && at[0] < left + 7.0,
                "share {share}: {t} at {}, the lane from {left}",
                at[0]
            );
        }
    }
}

#[test]
fn the_selected_difficulty_has_no_line_round_it_and_its_badge_room_inside_the_brackets() {
    let (mut s, _songs, mut now) = menu_look("select-look-diff-brackets");
    let c = settled(&mut s, &mut now);
    let cursor = s.select.cursor();
    let row = hit_rect(&c, |h| matches!(h, Hit::SongRow(r) if *r == cursor)).expect("the row");
    // Only the brackets and a light lift mark it: nothing the row's size has a line round it.
    let lined: Vec<[f32; 4]> = c
        .shapes
        .iter()
        .filter(|sh| plain(sh) && sh.params[1] > 0.0 && !badge(sh))
        .map(rect)
        .filter(|r| {
            (r[0] - row[0]).abs() < 1.0
                && (r[2] - row[2]).abs() < 1.0
                && inside([r[0], r[1]], row, 1.0)
        })
        .collect();
    assert!(
        lined.is_empty(),
        "no line round the selected difficulty: {lined:?}"
    );
    // Its rating's badge well inside the brackets' right side (3 out from the row).
    let badge_right = c
        .shapes
        .iter()
        .filter(|sh| badge(sh) && inside(sh.center, row, 0.0))
        .map(|sh| sh.center[0] + sh.half[0])
        .fold(f32::MIN, f32::max);
    assert!(badge_right > row[0], "the badge is on the row");
    let brackets = row[0] + row[2] + 3.0;
    assert!(
        brackets - badge_right >= 16.0,
        "room between the badge and the brackets: {}",
        brackets - badge_right
    );
}

#[test]
fn a_board_row_s_value_and_its_age_under_it_have_room_between_them() {
    let (mut s, _songs, mut now) = menu_look("select-look-board-gap");
    let play = local_play(&s, "Ann", 900_000, 0.99);
    s.scores.push(play);
    let c = on_board(&mut s, &mut now, false);
    let row = hit_rect(&c, |h| *h == Hit::ScoreRow(0)).expect("the row");
    // The texts ending at the row's right end, by their ink: the value on top, its age under it.
    let ink = |r: &std::ops::Range<usize>| {
        c.shapes[r.clone()]
            .iter()
            .map(rect)
            .fold([f32::MAX, f32::MIN, f32::MIN], |[t, b, e], g| {
                [t.min(g[1]), b.max(g[1] + g[3]), e.max(g[0] + g[2])]
            })
    };
    let mut right: Vec<([f32; 3], String)> = c
        .texts
        .iter()
        .filter(|(r, _)| !r.is_empty())
        .map(|(r, t)| (ink(r), t.clone()))
        .filter(|([t, b, e], _)| {
            *t >= row[1] - 0.5 && *b <= row[1] + row[3] + 0.5 && *e > row[0] + row[2] - 12.0
        })
        .collect();
    right.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
    assert_eq!(right.len(), 2, "the value and the age: {right:?}");
    let gap = right[1].0[0] - right[0].0[1];
    assert!(
        gap >= 4.0,
        "room between {:?} and {:?}: {gap}",
        right[0].1,
        right[1].1
    );
}

#[test]
fn the_collection_button_is_on_the_search_row_once_there_are_collections() {
    let (mut s, _songs, mut now) = menu_look("select-look-collection");
    let c = settled(&mut s, &mut now);
    assert!(
        hit_rect(&c, |h| *h == Hit::CollectionPick).is_none(),
        "none without collections"
    );
    s.select.collections = vec![ui::Shown {
        name: "dans".into(),
        own: true,
        ..Default::default()
    }];
    let c = settled(&mut s, &mut now);
    let menu = hit_rect(&c, |h| *h == Hit::ToMenu).expect("Menu");
    let search = hit_rect(&c, |h| *h == Hit::TextField).expect("the search");
    let sort = hit_rect(&c, |h| *h == Hit::SortList).expect("Sort");
    let pick = hit_rect(&c, |h| *h == Hit::CollectionPick).expect("the collection button");
    assert!(
        (pick[1] + pick[3] / 2.0 - (menu[1] + menu[3] / 2.0)).abs() < 0.5,
        "on the search row: {pick:?} {menu:?}"
    );
    let apart = |a: [f32; 4], b: [f32; 4]| a[0] + a[2] <= b[0] + 0.5 || b[0] + b[2] <= a[0] + 0.5;
    assert!(
        apart(pick, search) && apart(pick, sort),
        "{pick:?} {search:?} {sort:?}"
    );
    assert!(
        pick[0] + pick[2] <= 24.0 + s.select.list_w(c.width) + 0.5,
        "over the list: {pick:?}"
    );
    let texts = texts_in(&c, pick);
    assert!(
        texts.iter().any(|t| t == "COLLECTION"),
        "its caption: {texts:?}"
    );
    // Its list opens over the screen: every row named whole, inside it, and framed in song
    // select's palette (a thin neutral edge, the accent left to the selection).
    s.click(Hit::CollectionPick, None);
    let c = settled(&mut s, &mut now);
    let rows: Vec<[f32; 4]> = c
        .hits
        .iter()
        .filter(|h| matches!(h.hit, Hit::PickRow(_)))
        .map(|h| h.rect)
        .collect();
    assert!(!rows.is_empty(), "the collections' list");
    let boxes = text_boxes(&c);
    assert!(
        boxes.iter().any(|(t, _)| t.starts_with("dans")),
        "named whole: {boxes:?}"
    );
    for (t, r) in boxes.iter().filter(|(_, r)| {
        rows.iter()
            .any(|row| inside([r[0], r[1] + r[3] / 2.0], *row, 0.0))
    }) {
        let row = rows
            .iter()
            .find(|row| inside([r[0], r[1] + r[3] / 2.0], **row, 0.0))
            .unwrap();
        assert!(
            r[0] + r[2] <= row[0] + row[2] + 0.01,
            "{t:?} {r:?} within {row:?}"
        );
    }
    let edge = render::rgba(style::tone::EDGE, 1.0);
    let left = rows.iter().map(|r| r[0]).fold(f32::MAX, f32::min);
    let frame = c
        .shapes
        .iter()
        .rev()
        .filter(|sh| plain(sh) && sh.params[1] > 0.0)
        .map(|sh| (rect(sh), *sh))
        .find(|(r, _)| r[0] < left && r[0] + r[2] > left + 100.0 && r[3] > 40.0)
        .map(|(_, sh)| sh)
        .expect("the list's frame");
    assert!(
        same_rgb(frame.border, edge),
        "a neutral edge round the list"
    );
    // (Drawn over the screen in its units: one there is 1.5 in song select's.)
    assert!(
        frame.params[1] <= 1.5 * style::tone::LINE + 0.01,
        "a thin one: {} {:?}",
        frame.params[1],
        rect(&frame)
    );
}

#[test]
fn the_song_list_runs_down_a_dark_panel_with_a_line_along_its_top() {
    let (mut s, _songs, mut now) = menu_look("select-look-list-panel");
    let c = settled(&mut s, &mut now);
    let lw = s.select.list_w(c.width);
    let shown = cards(&c);
    let first_top = shown.iter().map(|(_, r)| r[1]).fold(f32::MAX, f32::min);
    let last_bottom = shown
        .iter()
        .map(|(_, r)| r[1] + r[3])
        .fold(f32::MIN, f32::max);
    // A dark panel the whole list is on (nothing bright shows between its rows), no line round
    // it.
    let panel_rgb = render::rgba(style::PANEL_COLD, 1.0);
    let panel = c
        .shapes
        .iter()
        .filter(|sh| {
            plain(sh)
                && same_rgb(sh.fill, panel_rgb)
                && sh.fill[3] >= 0.9
                && (sh.params[1] == 0.0 || sh.border == sh.fill)
        })
        .map(rect)
        .find(|r| r[0] <= 24.0 && r[0] + r[2] >= 24.0 + lw && r[3] > 200.0)
        .expect("a dark panel under the list");
    assert!(
        panel[1] < first_top && panel[1] + panel[3] >= last_bottom - 0.5,
        "the list on it: {panel:?}, cards {first_top}..{last_bottom}"
    );
    // A thin line along its top, under the search row: the rows scrolling up go under it.
    let edge = render::rgba(style::tone::EDGE, 1.0);
    c.shapes
        .iter()
        .filter(|sh| plain(sh) && same_rgb(sh.fill, edge))
        .map(rect)
        .find(|r| {
            (r[1] - panel[1]).abs() < 0.5
                && (r[3] - style::tone::LINE).abs() < 0.01
                && r[2] > lw * 0.8
        })
        .expect("a line along its top");
    let search = hit_rect(&c, |h| *h == Hit::TextField).expect("the search");
    assert!(
        panel[1] >= search[1] + search[3],
        "under the search row: {panel:?} {search:?}"
    );
}

#[test]
fn a_maps_picture_behind_song_select_is_under_a_dense_cold_ground_darker_at_the_edges() {
    let (mut s, _songs, mut now) = menu_look("select-look-bg-dim");
    s.preview.has_background = true;
    settled(&mut s, &mut now);
    s.preview.has_background = true;
    now += Duration::from_millis(16);
    frame(&mut s, now);
    let c = frame(&mut s, now + Duration::from_millis(16));
    // Over the whole screen a deep navy, not a grey: a yellow or white picture doesn't colour
    // the screen and the room between the panels brown.
    let ground = render::rgba(style::GROUND, 1.0);
    let tint = c
        .shapes
        .iter()
        .find(|sh| {
            plain(sh)
                && same_rgb(sh.fill, ground)
                && sh.half[0] * 2.0 >= c.width - 1.0
                && sh.half[1] * 2.0 >= render::VIRTUAL_HEIGHT - 1.0
        })
        .expect("the ground over the whole screen");
    assert!(ground[2] > ground[0], "cold: {ground:?}");
    let shown = s.bg_brightness * (1.0 - tint.fill[3]);
    assert!(
        s.bg_brightness > 0.0 && shown <= 0.1,
        "a bright picture doesn't wash the screen out: {} under {}",
        s.bg_brightness,
        tint.fill[3]
    );
    // Darker still at the screen's sides: a shade from each edge in.
    let sides = c
        .shapes
        .iter()
        .filter(|sh| sh.params[3] == render::SHADE_EASED && same_rgb(sh.fill, ground))
        .filter(|sh| sh.half[1] * 2.0 >= render::VIRTUAL_HEIGHT - 1.0)
        .count();
    assert_eq!(sides, 2, "a vignette down both sides");
}

#[test]
fn the_sort_and_filter_panel_has_room_for_every_label_and_song_select_s_colours() {
    let (mut s, _songs, mut now) = menu_look("select-look-lens");
    settled(&mut s, &mut now);
    assert!(s.select.in_ui_space(), "the 720 layout");
    s.modifiers_ctrl = true;
    press(&mut s, KeyCode::KeyS, now);
    s.modifiers_ctrl = false;
    assert!(s.select.lens_ui.is_some());
    // A big library: every status there, counts of three digits.
    s.select.facets.has_status = 0x1f;
    s.select.facets.statuses = [837, 232, 151, 74, 28];
    s.select.facets.lengths = [209, 745, 318, 96];
    let c = settled(&mut s, &mut now);
    let panel = hit_rect(&c, |h| *h == Hit::Lens(ui::LensItem::Panel)).expect("the panel");
    // The panel's own texts: drawn over its frame (the list's are under it).
    let over = c
        .shapes
        .iter()
        .position(|sh| {
            let r = rect(sh);
            plain(sh)
                && sh.params[1] > 0.0
                && (r[0] - panel[0]).abs() < 0.5
                && (r[2] - panel[2]).abs() < 0.5
        })
        .expect("the panel's frame");
    let starts: Vec<usize> = c
        .texts
        .iter()
        .filter(|(r, _)| r.start < r.end)
        .map(|(r, _)| r.start)
        .collect();
    let texts: Vec<(String, [f32; 4])> = text_boxes(&c)
        .into_iter()
        .zip(starts)
        .filter(|((_, r), at)| *at > over && inside([r[0], r[1]], panel, 0.0))
        .map(|(t, _)| t)
        .collect();
    let has = |t: &str| texts.iter().any(|(s, _)| s == t);
    // Every label whole: the sorts, the filters' captions and each block's name and count.
    for t in [
        "recently played",
        "date added",
        "length (minutes)",
        "status",
        "not played yet",
        "reset",
    ] {
        assert!(has(t), "{t:?} whole: {texts:?}");
    }
    for (name, ..) in ui::lens::STATUSES {
        assert!(has(name), "the {name} status named: {texts:?}");
    }
    for (name, _) in ui::lens::LENGTHS {
        assert!(has(name), "the {name} length named: {texts:?}");
    }
    for n in ["837", "232", "151", "74", "28", "209", "745"] {
        assert!(has(n), "the count {n}: {texts:?}");
    }
    // No text runs into another, and each block's text keeps within it.
    for (i, (a, ra)) in texts.iter().enumerate() {
        for (b, rb) in &texts[i + 1..] {
            let apart = ra[0] + ra[2] <= rb[0]
                || rb[0] + rb[2] <= ra[0]
                || ra[1] + ra[3] <= rb[1]
                || rb[1] + rb[3] <= ra[1];
            assert!(apart, "{a:?} {ra:?} runs into {b:?} {rb:?}");
        }
    }
    for h in c.hits.iter().filter(|h| {
        matches!(
            h.hit,
            Hit::Lens(ui::LensItem::Status(_) | ui::LensItem::Length(_) | ui::LensItem::Sort(_))
        )
    }) {
        for (t, r) in &texts {
            if inside([r[0] + r[2] / 2.0, r[1] + r[3] / 2.0], h.rect, 0.0) {
                assert!(
                    r[0] >= h.rect[0] - 0.01 && r[0] + r[2] <= h.rect[0] + h.rect[2] + 0.01,
                    "{t:?} {r:?} within its block {:?}",
                    h.rect
                );
            }
        }
    }
    // Song select's palette: a thin neutral edge round the panel (no yellow or gold frame), the
    // sort picked in Play's colour.
    let edge = render::rgba(style::tone::EDGE, 1.0);
    let frame = c
        .shapes
        .iter()
        .filter(|sh| plain(sh) && sh.params[1] > 0.0)
        .map(|sh| (rect(sh), sh))
        .find(|(r, _)| (r[0] - panel[0]).abs() < 0.5 && (r[2] - panel[2]).abs() < 0.5)
        .map(|(_, sh)| *sh)
        .expect("the panel's frame");
    assert!(same_rgb(frame.border, edge), "a neutral edge");
    assert!(frame.params[1] <= style::tone::LINE + 0.01, "a thin one");
    let gold = render::rgba(0xFFD34D, 1.0);
    assert!(
        !c.shapes.iter().any(|sh| inside(sh.center, panel, 0.0)
            && sh.params[1] > 0.0
            && same_rgb(sh.border, gold)),
        "nothing gold"
    );
    let accent = render::rgba(style::role::primary(), 1.0);
    let sort = hit_rect(&c, |h| {
        *h == Hit::Lens(ui::LensItem::Sort(
            ui::SortMode::ALL
                .iter()
                .position(|&m| m == s.select.sort)
                .unwrap(),
        ))
    })
    .expect("the sort picked");
    assert!(
        c.shapes
            .iter()
            .any(|sh| inside(sh.center, sort, 0.0) && same_rgb(sh.fill, accent)),
        "the sort picked in Play's colour"
    );
}

#[test]
fn the_preview_s_notes_run_along_a_corridor_as_tall_as_the_lane_not_through_empty_room() {
    let (mut s, _songs, mut now) = menu_look("select-look-corridor");
    let c = settled(&mut s, &mut now);
    let [bx, by, bw, bh] = s.preview.box_rect().expect("the lane's box");
    let corridor = s.preview.corridor().expect("a plain map's corridor");
    let [cx, cy, cw, ch] = corridor;
    // Across the lane's tile, in the middle of its box, a share of it: the notes' and the drum's
    // height (the band of a play's lane at the preview's scale), not the whole box.
    let tile = hit_rect(&c, |h| *h == Hit::Preview).expect("the lane");
    assert!(
        (cx - tile[0]).abs() < 0.01 && cx + cw >= tile[0] + tile[2] - 0.01 && cw > 0.9 * bw,
        "across the tile: {corridor:?} over {tile:?}"
    );
    assert!(cx <= bx + 0.01, "from the drum's side");
    assert!(
        ((cy + ch / 2.0) - (by + bh / 2.0)).abs() < 1.0,
        "in the box's middle: {corridor:?} in {by}..{}",
        by + bh
    );
    assert!(
        ch < bh * 0.95 && ch > bh * 0.3,
        "the lane's height: {ch} of {bh}"
    );
    // A step lighter than the tile, a thin line of the panels' edge along its top and bottom.
    let lifted = style::surface_fill(style::Surface::Lifted);
    let at = |r: [f32; 4]| (r[0] - cx).abs() < 0.01 && (r[2] - cw).abs() < 0.01;
    assert!(
        c.shapes.iter().any(|sh| plain(sh)
            && same_rgb(sh.fill, lifted)
            && at(rect(sh))
            && (rect(sh)[3] - ch).abs() < 0.01),
        "its fill"
    );
    let edge = render::rgba(style::tone::EDGE, 1.0);
    let lines: Vec<f32> = c
        .shapes
        .iter()
        .filter(|sh| plain(sh) && same_rgb(sh.fill, edge) && at(rect(sh)))
        .map(|sh| rect(sh)[1])
        .filter(|&ly| (ly - cy).abs() < 0.01 || (ly + style::tone::LINE - (cy + ch)).abs() < 0.01)
        .collect();
    assert_eq!(lines.len(), 2, "a line along its top and bottom: {lines:?}");
}
