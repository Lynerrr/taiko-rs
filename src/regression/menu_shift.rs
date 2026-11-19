//! The way between the main menu and song select (`crate::menu_shift`): Play moves the camera on
//! along the menu's lane to song select, which comes in from the right as the menu goes off to
//! the left; back runs it the other way; without animations both are a cut, as before.

use super::harness::{self, *};
use super::maps;
use crate::hit::{Canvas, Hit};
use crate::*;

/// A client with the main menu up, animations on.
fn on_menu(tag: &str, animations: bool) -> (State, Songs, Instant) {
    let songs = Songs::new(tag, &maps::all());
    let cfg = Config {
        main_menu: true,
        animations,
        ..harness::config()
    };
    let mut s = state_with(cfg, songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    s.play_when_scanned = false;
    s.main_menu = Some(menu::Menu::new());
    for _ in 0..10 {
        now += Duration::from_millis(16);
        frame(&mut s, now);
    }
    (s, songs, now)
}

/// Frames 16 ms apart for `ms`; the last one drawn.
fn run(s: &mut State, now: &mut Instant, ms: u32) -> Canvas {
    let mut c = frame(s, *now);
    for _ in 0..ms / 16 {
        *now += Duration::from_millis(16);
        c = frame(s, *now);
    }
    c
}

/// Where `hit` is on the screen (its rect's left edge), if it's there.
fn left_of(c: &Canvas, hit: Hit) -> Option<f32> {
    c.hits.iter().find(|h| h.hit == hit).map(|h| h.rect[0])
}

/// Where the text `needle` is drawn (its first shape's middle), if it is.
fn text_x(c: &Canvas, needle: &str) -> Option<f32> {
    c.texts
        .iter()
        .find(|(_, t)| t.trim() == needle)
        .map(|(r, _)| c.shapes[r.start].center[0])
}

fn shift_t(s: &State) -> Option<f32> {
    s.menu_shift.as_ref().map(menu_shift::Shift::t)
}

#[test]
fn play_moves_the_camera_on_to_song_select_and_back() {
    let (mut s, _songs, mut now) = on_menu("menu-shift", true);
    let rest = run(&mut s, &mut now, 32);
    let play_note = left_of(&rest, Hit::MenuItem(0)).expect("the Play note on the menu");
    let exit = text_x(&rest, "EXIT").expect("Exit's name on the menu");

    s.menu_click(0);
    run(&mut s, &mut now, 16);
    assert!(s.menu_shift.as_ref().is_some_and(|m| m.into_songs));
    assert!(
        s.main_menu.is_none() && !s.on_menu(),
        "song select has the keys at once"
    );

    // Halfway: the menu has gone left, song select is coming in from the right.
    let half = run(&mut s, &mut now, 240);
    let t = shift_t(&s).expect("still on its way");
    assert!(t > 0.2 && t < 0.5, "{t}");
    assert!(
        left_of(&half, Hit::MenuItem(0)).is_none(),
        "the menu, going, takes no clicks"
    );
    let going = text_x(&half, "EXIT").expect("the menu, going");
    assert!(
        going < exit - 100.0,
        "the menu went left: {going} from {exit}"
    );
    let play = left_of(&half, Hit::Key(KeyCode::Enter)).expect("song select's Play");

    // There: no camera left on its way, song select as it is any time.
    let there = run(&mut s, &mut now, 760);
    assert_eq!(shift_t(&s), None);
    let play_there = left_of(&there, Hit::Key(KeyCode::Enter)).unwrap();
    assert!(
        play > play_there + 50.0,
        "Play came in from the right: {play} to {play_there}"
    );
    assert!(
        left_of(&there, Hit::MenuItem(0)).is_none(),
        "the menu's gone"
    );
    assert!(left_of(&there, Hit::ToMenu).is_some_and(|x| x < 40.0));

    // Back: the menu has the keys at once, and comes in from the left.
    press(&mut s, KeyCode::Escape, now);
    assert!(s.on_menu());
    assert!(s.menu_shift.as_ref().is_some_and(|m| !m.into_songs));
    let back = run(&mut s, &mut now, 160);
    let note = left_of(&back, Hit::MenuItem(0)).expect("the menu, coming");
    assert!(
        left_of(&back, Hit::Key(KeyCode::Enter)).is_none(),
        "song select, going, takes no clicks"
    );
    assert!(
        note < play_note - 50.0,
        "still on its way from the left: {note}"
    );
    let home = run(&mut s, &mut now, 700);
    assert_eq!(shift_t(&s), None, "back is quicker than going");
    let note = left_of(&home, Hit::MenuItem(0)).unwrap();
    assert!(
        (note - play_note).abs() < 0.5,
        "the menu where it was: {note}"
    );
    assert!(left_of(&home, Hit::Key(KeyCode::Enter)).is_none());
}

#[test]
fn back_halfway_turns_round_with_the_menu_that_was_left() {
    let (mut s, _songs, mut now) = on_menu("menu-shift-turn", true);
    s.menu_click(0);
    run(&mut s, &mut now, 300);
    let t = shift_t(&s).unwrap();
    press(&mut s, KeyCode::Escape, now);
    let back = s.menu_shift.as_ref().unwrap();
    assert!(
        !back.into_songs && (back.t() - t).abs() < 1e-4,
        "from where it was"
    );
    assert!(back.flying.is_none());
    assert!(s.main_menu.is_some());
    run(&mut s, &mut now, 400);
    assert_eq!(shift_t(&s), None);
    assert!(s.on_menu());
}

#[test]
fn play_again_as_the_menu_slides_back_in_throws_its_note_whole() {
    let (mut s, _songs, mut now) = on_menu("menu-shift-again", true);
    s.menu_click(0);
    run(&mut s, &mut now, 1000);
    assert_eq!(shift_t(&s), None, "on song select");
    press(&mut s, KeyCode::Escape, now);
    // The menu nearly back in its place.
    while shift_t(&s).is_some_and(|t| menu_shift::phases(t).cam > 0.15) {
        now += Duration::from_millis(16);
        frame(&mut s, now);
    }
    assert!(shift_t(&s).is_some(), "still on its way back");
    s.menu_click(0);
    run(&mut s, &mut now, 0);
    let way = s.menu_shift.as_ref().expect("on the way to song select");
    assert!(
        way.into_songs && way.flying == Some(menu::Item::Play),
        "the note flies: {} {:?} at {}",
        way.into_songs,
        way.flying,
        way.t()
    );
    assert_eq!(way.flight(), 0.0, "from its start");
    let mut flown = 0.0f32;
    while s.menu_shift.is_some() {
        now += Duration::from_millis(16);
        frame(&mut s, now);
        flown = flown.max(s.menu_shift.as_ref().map_or(0.0, menu_shift::Shift::flight));
    }
    assert!(flown > 0.8, "all the way: {flown}");
}

#[test]
fn back_from_song_select_moves_the_menu_at_once() {
    let (mut s, _songs, mut now) = on_menu("menu-shift-back-at-once", true);
    s.menu_click(0);
    run(&mut s, &mut now, 1200);
    assert_eq!(shift_t(&s), None, "song select all there");
    press(&mut s, KeyCode::Escape, now);
    let first = run(&mut s, &mut now, 0);
    let at = text_x(&first, "EXIT").expect("the menu, coming");
    // A hundred milliseconds on, the menu has visibly come in.
    let later = run(&mut s, &mut now, 96);
    let moved = text_x(&later, "EXIT").expect("the menu, coming") - at;
    assert!(moved > 10.0, "the menu moved {moved} in 100 ms");
}

#[test]
fn without_animations_song_select_is_simply_there() {
    let (mut s, _songs, mut now) = on_menu("menu-shift-off", false);
    s.menu_click(0);
    let c = run(&mut s, &mut now, 16);
    assert!(s.menu_shift.is_none() && !s.on_menu());
    assert!(left_of(&c, Hit::MenuItem(0)).is_none());
    press(&mut s, KeyCode::Escape, now);
    assert!(s.menu_shift.is_none() && s.on_menu());
}

#[test]
fn another_screen_leaves_no_camera_on_its_way() {
    let (mut s, _songs, mut now) = on_menu("menu-shift-away", true);
    s.menu_click(0);
    run(&mut s, &mut now, 200);
    assert!(s.menu_shift.is_some());
    s.open_settings();
    run(&mut s, &mut now, 32);
    assert!(s.menu_shift.is_none());
}
