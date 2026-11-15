//! The easter eggs behind the secret medals (`crate::eggs`, `crate::ura`), played through the
//! client as a player would: the drum keys turning the main menu over, clicks breaking its logo
//! and hatching an unknown medal. What each one tells the server is checked, not sent.

use super::harness::{self, *};
use super::maps;
use crate::hit::Hit;
use crate::ura::{self, Event};
use crate::*;

/// A logged-in client on the main menu.
fn on_menu(tag: &str, now: &mut Instant) -> (State, Songs) {
    let songs = Songs::new(&format!("secrets-{tag}"), &[maps::basics()]);
    let mut cfg = harness::config();
    cfg.online_token = "t".into();
    let mut s = state_with(cfg, songs.entries());
    ready(&mut s, now);
    s.cfg.main_menu = true;
    s.main_menu = Some(menu::Menu::new());
    *now += Duration::from_millis(16);
    frame(&mut s, *now);
    ura::take_reported();
    (s, songs)
}

/// The middle of the first area drawn last frame for `hit`.
fn spot(s: &State, hit: Hit) -> Option<(f32, f32)> {
    s.hits.iter().find(|h| h.hit == hit).map(|h| {
        let [x, y, w, ht] = h.rect;
        (x + w / 2.0, y + ht / 2.0)
    })
}

fn click_at(s: &mut State, now: &mut Instant, (x, y): (f32, f32)) {
    let k = s.window_h / render::VIRTUAL_HEIGHT;
    s.mouse_moved(f64::from(x * k), f64::from(y * k));
    s.mouse_button(true, None);
    *now += Duration::from_millis(16);
    frame(s, *now);
    s.mouse_button(false, None);
    *now += Duration::from_millis(16);
    frame(s, *now);
}

/// Both rims `apart` ms from each other, `times` times, a third of a second between.
fn both_rims(s: &mut State, now: &mut Instant, apart: u64, times: usize) {
    let (left, right) = (s.cfg.keys[0], s.cfg.keys[3]);
    for _ in 0..times {
        press(s, left, *now);
        press(s, right, *now + Duration::from_millis(apart));
        *now += Duration::from_millis(330);
        frame(s, *now);
    }
}

#[test]
fn both_rims_at_once_ten_times_turn_the_main_menu_over() {
    let mut now = Instant::now();
    let (mut s, _songs) = on_menu("flip", &mut now);
    let before = spot(&s, Hit::MenuItem(0)).expect("the menu's first note");
    // Further apart than a big note's two hits may be: never both at once.
    both_rims(&mut s, &mut now, 45, 12);
    assert_eq!(ura::take_reported(), Vec::<Event>::new());
    assert_eq!(spot(&s, Hit::MenuItem(0)), Some(before));
    // Within the window: the tenth turns it over, once.
    both_rims(&mut s, &mut now, 25, 9);
    assert_eq!(ura::take_reported(), Vec::<Event>::new());
    both_rims(&mut s, &mut now, 0, 3);
    assert_eq!(ura::take_reported(), vec![Event::Flip]);
    // Upside down, its notes (and where they're clicked) are on the other side of the middle.
    let after = spot(&s, Hit::MenuItem(0)).expect("the menu's first note");
    assert!(
        (after.0 - (WIDTH - before.0)).abs() < 0.5,
        "{before:?} -> {after:?}"
    );
    assert!((after.1 - (render::VIRTUAL_HEIGHT - before.1)).abs() < 0.5);
    // And stays so, back in the menu after leaving it.
    s.main_menu = Some(menu::Menu::new());
    now += Duration::from_millis(16);
    frame(&mut s, now);
    assert!((spot(&s, Hit::MenuItem(0)).unwrap().0 - after.0).abs() < 0.5);
}

#[test]
fn ten_quick_clicks_on_the_logo_break_it() {
    let mut now = Instant::now();
    let (mut s, _songs) = on_menu("logo", &mut now);
    let logo = spot(&s, Hit::MenuLogo).expect("the logo can be clicked");
    for _ in 0..9 {
        click_at(&mut s, &mut now, logo);
    }
    assert_eq!(ura::take_reported(), Vec::<Event>::new());
    click_at(&mut s, &mut now, logo);
    assert_eq!(ura::take_reported(), vec![Event::Logo]);
    // Logged out, nothing goes to the server.
    s.cfg.online_token.clear();
    now += Duration::from_secs(5);
    frame(&mut s, now);
    for _ in 0..10 {
        click_at(&mut s, &mut now, logo);
    }
    assert_eq!(ura::take_reported(), Vec::<Event>::new());
}

fn row(id: &str, section: u8, art: Vec<String>) -> online::MedalRow {
    online::MedalRow {
        id: id.into(),
        section,
        tier: 2,
        secret: true,
        name: None,
        how: None,
        hint: None,
        art,
        at: None,
        holders: 0,
        have: 0,
        need: 0,
    }
}

#[test]
fn seven_clicks_on_an_unknown_medal_hatch_it() {
    let mut now = Instant::now();
    let (mut s, _songs) = on_menu("curious", &mut now);
    let view = online::MedalsView {
        players: 3,
        earned: 0,
        medals: vec![
            row("s_known", 7, vec![".".repeat(16); 16]),
            row("s_unknown", 7, Vec::new()),
        ],
        collection: Vec::new(),
        collection_size: 0,
    };
    s.medals = Some(medals::View {
        name: None,
        state: medals::State::Loaded(Box::new(view)),
        tab: 8,
        scroll: 0,
        egg: Default::default(),
    });
    now += Duration::from_millis(16);
    frame(&mut s, now);
    // Only the unknown one has a question mark to poke.
    assert_eq!(
        s.hits
            .iter()
            .filter(|h| matches!(h.hit, Hit::MedalMystery(_)))
            .count(),
        1
    );
    let mark = s
        .hits
        .iter()
        .find_map(|h| match h.hit {
            Hit::MedalMystery(_) => {
                let [x, y, w, ht] = h.rect;
                Some((x + w / 2.0, y + ht / 2.0))
            }
            _ => None,
        })
        .unwrap();
    for _ in 0..6 {
        click_at(&mut s, &mut now, mark);
    }
    assert_eq!(ura::take_reported(), Vec::<Event>::new());
    click_at(&mut s, &mut now, mark);
    assert_eq!(ura::take_reported(), vec![Event::Curious]);
    click_at(&mut s, &mut now, mark);
    assert_eq!(ura::take_reported(), Vec::<Event>::new());
}
