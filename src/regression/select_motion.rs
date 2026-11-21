//! Song select's motion in the main menu's look: how the details come in on a song change, what
//! stands in for what's still being read, and that a quick run of changes doesn't queue up.

use super::harness::{self, *};
use super::maps;
use crate::*;

/// Song select in the main menu's look with the interface's animations on, several sets.
fn animated(tag: &str) -> (State, Songs, Instant) {
    let charts: Vec<maps::Chart> = (0..6)
        .map(|i| maps::Chart {
            name: Box::leak(format!("motion-{i:02}").into_boxed_str()),
            ..maps::basics()
        })
        .collect();
    let songs = Songs::new(tag, &charts);
    let cfg = Config {
        main_menu: true,
        animations: true,
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

fn title(s: &State) -> String {
    let i = s.select.selected().expect("a map selected");
    s.library[i].meta.title.clone()
}

/// Where the text `t` is drawn on the details' side (left of its first letter) and how opaque.
fn text_on_panel(c: &Canvas, s: &State, t: &str) -> Option<(f32, f32)> {
    let lw = s.select.list_w(c.width);
    c.texts.iter().find_map(|(r, text)| {
        let sh = c.shapes.get(r.start)?;
        (text == t && sh.center[0] > 24.0 + lw).then(|| (sh.center[0] - sh.half[0], sh.fill[3]))
    })
}

fn step(s: &mut State, now: &mut Instant, ms: u64) -> Canvas {
    *now += Duration::from_millis(ms);
    frame(s, *now)
}

#[test]
fn a_song_picked_on_its_own_comes_in_a_part_at_a_time() {
    let (mut s, _songs, mut now) = animated("select-motion-enter");
    let c = settled(&mut s, &mut now);
    let (rest_x, _) = text_on_panel(&c, &s, &title(&s)).expect("the title");
    press(&mut s, KeyCode::ArrowRight, now);
    // The frame that takes the new song starts its entrance; one later it's on its way.
    step(&mut s, &mut now, 16);
    let c = step(&mut s, &mut now, 16);
    let name = title(&s);
    let (x, a) = text_on_panel(&c, &s, &name).expect("the new title");
    assert!(a > 0.0 && a < 0.6, "the title fades in: {a}");
    assert!(
        x > rest_x + 0.5,
        "from the right of its place: {x} vs {rest_x}"
    );
    // The tiles come after the title.
    let (_, od) = text_on_panel(&c, &s, "OD").expect("the OD tile");
    assert!(od < a, "the tiles later than the title: {od} vs {a}");
    let c = step(&mut s, &mut now, 600);
    let (x, a) = text_on_panel(&c, &s, &name).expect("the new title");
    assert_eq!((x, a), (rest_x, 1.0), "in place at the end");
    let (_, od) = text_on_panel(&c, &s, "OD").expect("the OD tile");
    assert_eq!(od, 1.0);
}

#[test]
fn songs_picked_in_a_quick_run_swap_in_at_once() {
    let (mut s, _songs, mut now) = animated("select-motion-run");
    let c = settled(&mut s, &mut now);
    let (rest_x, _) = text_on_panel(&c, &s, &title(&s)).expect("the title");
    press(&mut s, KeyCode::ArrowRight, now);
    step(&mut s, &mut now, 50);
    press(&mut s, KeyCode::ArrowRight, now);
    let c = step(&mut s, &mut now, 16);
    let (x, a) = text_on_panel(&c, &s, &title(&s)).expect("the title");
    assert_eq!((x, a), (rest_x, 1.0), "held keys swap the details at once");
}

#[test]
fn details_on_their_way_show_skeletons_not_dots() {
    let (mut s, _songs, mut now) = animated("select-motion-skeleton");
    settled(&mut s, &mut now);
    press(&mut s, KeyCode::ArrowRight, now);
    let c = step(&mut s, &mut now, 16);
    assert!(
        c.texts
            .iter()
            .all(|(_, t)| !t.contains("...") || t.contains("search")),
        "no dots for what's on its way: {:?}",
        c.texts.iter().map(|(_, t)| t).collect::<Vec<_>>()
    );
    let lw = s.select.list_w(c.width);
    let lifted = style::surface_fill(style::Surface::Lifted);
    let blocks = c
        .shapes
        .iter()
        .filter(|sh| sh.center[0] > 24.0 + lw && sh.params[3] >= 0.0 && sh.params[3] < 10.0)
        .filter(|sh| {
            sh.fill
                .iter()
                .zip(lifted)
                .take(3)
                .all(|(a, b)| (a - b).abs() < 1e-4)
        })
        .count();
    assert!(blocks >= 6, "skeletons where the values go: {blocks}");
    // Once the map is read they make way for the values.
    let c = settled(&mut s, &mut now);
    let (_, a) = text_on_panel(&c, &s, "BPM").expect("the BPM tile");
    assert_eq!(a, 1.0);
}
