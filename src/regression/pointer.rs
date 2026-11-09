//! The song preview and the pointer: its autoplay box, seek graph and play button take clicks,
//! the wheel and the hover cursor only where nothing is drawn over them.
//!
//! These clients have no sound, so song select's graph never gets music to seek in; the
//! fullscreen preview's graph seeks without it.

use super::harness::{self, *};
use super::maps;
use crate::hit::{Hit, HitBox};
use crate::*;
use winit::window::CursorIcon;

fn client(tag: &str) -> (State, Songs, Instant) {
    let songs = Songs::new(tag, &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    assert_eq!(scene_name(&s), "Select");
    // The seek graph is drawn once the preview's map is read, on a thread of its own: `ready`
    // waits for the screen's text, which the graph has none of.
    let until = Instant::now() + Duration::from_secs(20);
    while regions(&s.hits).len() < 2 {
        assert!(Instant::now() < until, "the preview's map never loaded");
        std::thread::sleep(Duration::from_millis(5));
        now += Duration::from_millis(16);
        frame(&mut s, now);
    }
    (s, songs, now)
}

/// Draws the next frame. Its clickable areas are kept in `State::hits` (the canvas is handed
/// back with the frame before's): what the pointer meets until the frame after.
fn next_frame<'a>(s: &'a mut State, now: &mut Instant) -> &'a [HitBox] {
    *now += Duration::from_millis(16);
    frame(s, *now);
    &s.hits
}

/// The preview's own areas on a frame, in drawing order.
fn regions(hits: &[HitBox]) -> Vec<[f32; 4]> {
    hits.iter()
        .filter(|h| matches!(h.hit, Hit::Preview))
        .map(|h| h.rect)
        .collect()
}

fn top(hits: &[HitBox], x: f32, y: f32) -> Option<Hit> {
    hits.iter().rev().find(|h| h.contains(x, y)).map(|h| h.hit)
}

fn middle([x, y, w, h]: [f32; 4]) -> (f32, f32) {
    (x + w / 2.0, y + h / 2.0)
}

/// A spot of `rect` where `want` is the topmost thing in `hits`.
fn spot_where(
    hits: &[HitBox],
    [x, y, w, h]: [f32; 4],
    want: fn(Hit) -> bool,
) -> Option<(f32, f32)> {
    (0..=40)
        .flat_map(|i| (0..=8).map(move |j| (i, j)))
        .map(|(i, j)| (x + w * i as f32 / 40.0, y + h * j as f32 / 8.0))
        .find(|&(px, py)| top(hits, px, py).is_some_and(want))
}

/// Moves the pointer to (`x`, `y`) in virtual units; the cursor shown there.
fn point(s: &mut State, (x, y): (f32, f32)) -> CursorIcon {
    let k = (s.window_h / render::VIRTUAL_HEIGHT) as f64;
    let (px, py) = (x as f64 * k, y as f64 * k);
    let icon = s.mouse_moved(px, py);
    s.mouse = (px, py);
    icon
}

fn click(s: &mut State) {
    s.mouse_button(true, None);
    s.mouse_button(false, None);
}

/// Song select's autoplay box and seek graph (the play button shows only once a start point is
/// picked, which takes music: `preview`'s own tests draw it).
fn box_and_graph(hits: &[HitBox]) -> ([f32; 4], [f32; 4]) {
    match regions(hits)[..] {
        [b, g] => (b, g),
        ref r => panic!("the box and the graph, not {r:?}"),
    }
}

/// From song select, a click on the autoplay box: the fullscreen preview, and its graph.
fn open_fullscreen(s: &mut State, now: &mut Instant) -> [f32; 4] {
    let hits = next_frame(s, now).to_vec();
    point(s, middle(box_and_graph(&hits).0));
    click(s);
    assert_eq!(scene_name(s), "Preview");
    // Past the fade between screens.
    s.scene_since = *now - Duration::from_secs(5);
    let hits = next_frame(s, now).to_vec();
    match regions(&hits)[..] {
        [graph] => graph,
        ref r => panic!("the fullscreen graph alone, not {r:?}"),
    }
}

#[test]
fn with_nothing_over_it_the_preview_takes_the_pointer() {
    let (mut s, _songs, mut now) = client("pointer-free");
    let hits = next_frame(&mut s, &mut now).to_vec();
    let (bx, graph) = box_and_graph(&hits);

    // The graph before its music: on top, but nothing to seek in, so no hand and no seek.
    let on_graph = middle(graph);
    assert!(s.preview_on_top(on_graph.0, on_graph.1));
    assert_eq!(point(&mut s, on_graph), CursorIcon::Default);
    s.mouse_button(true, None);
    assert!(s.scrubbing.is_none());
    s.mouse_button(false, None);

    let on_box = middle(bx);
    assert!(s.preview_on_top(on_box.0, on_box.1));
    assert_eq!(point(&mut s, on_box), CursorIcon::Pointer);
    click(&mut s);
    assert_eq!(
        scene_name(&s),
        "Preview",
        "a click on the box opens the fullscreen preview"
    );

    s.scene_since = now - Duration::from_secs(5);
    let hits = next_frame(&mut s, &mut now).to_vec();
    let graph = match regions(&hits)[..] {
        [graph] => graph,
        ref r => panic!("the fullscreen graph alone, not {r:?}"),
    };
    assert_eq!(point(&mut s, middle(graph)), CursorIcon::Pointer);
    s.mouse_button(true, None);
    assert!(
        s.scrubbing.is_some(),
        "a press on the fullscreen graph seeks"
    );
    s.mouse_button(false, None);
    assert!(s.scrubbing.is_none(), "and letting go ends it");
}

#[test]
fn the_guide_over_the_preview_takes_the_pointer() {
    let (mut s, _songs, mut now) = client("pointer-guide");
    s.guide = Some(0);
    let hits = next_frame(&mut s, &mut now).to_vec();
    let (bx, graph) = box_and_graph(&hits);
    for (name, rect) in [("box", bx), ("graph", graph)] {
        let spot = spot_where(&hits, rect, |h| matches!(h, Hit::Overlay))
            .unwrap_or_else(|| panic!("the guide covers the preview's {name}"));
        assert!(!s.preview_on_top(spot.0, spot.1), "{name}");
        point(&mut s, spot);
        s.mouse_button(true, None);
        assert!(s.scrubbing.is_none(), "{name}: no seek through the guide");
        s.mouse_button(false, None);
        assert_eq!(scene_name(&s), "Select", "{name}: no fullscreen preview");
        assert_eq!(s.guide, Some(0), "{name}: the guide stays");
    }
}

#[test]
fn the_friends_panel_over_the_preview_takes_the_pointer() {
    let (mut s, _songs, mut now) = client("pointer-friends");
    s.social.open = true;
    settle(&mut s, &mut now);
    let hits = next_frame(&mut s, &mut now).to_vec();
    let bx = box_and_graph(&hits).0;
    let spot = spot_where(&hits, bx, |h| matches!(h, Hit::SocialPanel))
        .expect("the friends panel covers part of the autoplay box");
    assert!(!s.preview_on_top(spot.0, spot.1));
    point(&mut s, spot);
    click(&mut s);
    assert_eq!(
        scene_name(&s),
        "Select",
        "no fullscreen preview through the panel"
    );
    assert!(s.social.open, "a click on the panel keeps it open");
}

#[test]
fn the_mod_select_and_open_drop_downs_keep_the_pointer_off_the_preview() {
    let (mut s, _songs, mut now) = client("pointer-menus");
    let hits = next_frame(&mut s, &mut now).to_vec();
    let (x, y) = middle(box_and_graph(&hits).0);
    assert!(s.preview_on_top(x, y));

    s.mod_select = Some(0);
    assert!(!s.preview_on_top(x, y), "the mod select");
    s.mod_select = None;

    s.settings_ui.dropdown = Some(settings::Dropdown::new(0));
    assert!(!s.preview_on_top(x, y), "a settings drop-down");
    s.settings_ui.dropdown = None;

    // The download tab's drop-down is drawn before the preview, its click-away area under it.
    s.scene = Scene::Download;
    assert!(s.preview_on_top(x, y));
    s.browse.menu = Some(browse::Menu::Sort);
    assert!(!s.preview_on_top(x, y), "the download tab's drop-down");
    s.browse.menu = None;
    s.scene = Scene::Select;
    assert!(s.preview_on_top(x, y));
}

#[test]
fn anything_drawn_over_the_preview_takes_the_pointer_and_the_wheel() {
    let (mut s, _songs, mut now) = client("pointer-overlay");
    let graph = open_fullscreen(&mut s, &mut now);
    // The wheel resizes the playfield here (down: it starts full screen) wherever the graph
    // doesn't take it.
    s.cfg.wheel_resizes_playfield = true;
    let area = s.cfg.playfield_area.describe();
    let on_graph = middle(graph);
    assert_eq!(point(&mut s, on_graph), CursorIcon::Pointer);
    s.wheel(-1);
    assert_eq!(
        s.cfg.playfield_area.describe(),
        area,
        "the graph takes the wheel"
    );

    // An overlay over the whole screen, drawn after the preview.
    next_frame(&mut s, &mut now);
    s.hits.push(HitBox {
        rect: [0.0, 0.0, f32::MAX, render::VIRTUAL_HEIGHT],
        hit: Hit::Overlay,
        shape: 0,
    });
    assert!(!s.preview_on_top(on_graph.0, on_graph.1));
    assert_ne!(point(&mut s, on_graph), CursorIcon::Default);
    s.mouse_button(true, None);
    assert!(s.scrubbing.is_none(), "no seek under the overlay");
    s.mouse_button(false, None);
    s.wheel(-1);
    assert_ne!(
        s.cfg.playfield_area.describe(),
        area,
        "the wheel goes past the covered graph"
    );
}
