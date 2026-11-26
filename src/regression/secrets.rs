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
fn the_menu_s_stray_note_never_takes_a_button_s_click() {
    // The main menu as it draws over a playing song: tests have no music, so its notes are
    // given to it, and the animations it needs are on.
    render::edit_frame_theme(|t| t.animations = true);
    let notes: Vec<crate::menu_notes::Ghost> = (0..20_000)
        .map(|i| crate::menu_notes::Ghost {
            time: f64::from(i) * 120.0,
            kat: i % 3 == 1,
            big: i % 16 == 0,
        })
        .collect();
    let mut menu = menu::Menu::new();
    let mut eggs = crate::eggs::MenuEggs::new(3);
    eggs.stray.come_now();
    let (mut caught, mut seen) = (0, 0);
    for k in 0..200_000 {
        let time = f64::from(k) * 16.0;
        let mut c = crate::hit::Canvas::default();
        c.clear(WIDTH);
        c.dt_ms = 16.0;
        let frame = menu::Frame {
            beat: Some((time % 500.0, 500.0)),
            kiai: false,
            ghosts: Some(crate::menu_notes::View::of(&notes, time)),
            lane_dim: 0.0,
            song: Some("Artist - Title"),
            maps: 1,
            paused: false,
            session: (0.0, 0.0),
            glitch: false,
            shifting: false,
            away: None,
        };
        menu.draw(&mut c, &frame, &mut eggs);
        let Some(gold) = c.hits.iter().position(|h| h.hit == Hit::StrayNote) else {
            continue;
        };
        seen += 1;
        let [x, y, w, h] = c.hits[gold].rect;
        for other in c.hits.iter().filter(|o| o.hit != Hit::StrayNote) {
            let [ox, oy, ow, oh] = other.rect;
            assert!(
                x >= ox + ow || ox >= x + w || y >= oy + oh || oy >= y + h,
                "the gold note at {:?} lies over {:?}",
                c.hits[gold].rect,
                other.hit
            );
        }
        // Clicked a few frames after it can be: a player's reaction.
        if seen % 20 == 0 {
            caught += 1;
            assert_eq!(eggs.catch_stray(), caught == 3, "catch {caught}");
            if caught == 3 {
                return;
            }
        }
    }
    panic!("only {caught} gold notes caught");
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

/// A Journey marathon's combo tells each step it reaches, once.
#[test]
fn a_marathon_combo_tells_its_steps() {
    let mut cfg = harness::config();
    cfg.online_token = "t".into();
    ura::take_reported();
    let [one, two, three] = ura::MARATHON_COMBOS;
    ura::marathon_combo(&cfg, 0, one - 1);
    assert_eq!(ura::take_reported(), Vec::<Event>::new());
    ura::marathon_combo(&cfg, one - 1, one);
    assert_eq!(ura::take_reported(), vec![Event::Marathon(1)]);
    // Past two steps at once (a long map), both are told.
    ura::marathon_combo(&cfg, one, three);
    assert_eq!(
        ura::take_reported(),
        vec![Event::Marathon(2), Event::Marathon(3)]
    );
    assert!(one < two && two < three);
    // Signed out: nobody to tell.
    ura::marathon_combo(&harness::config(), 0, three);
    assert_eq!(ura::take_reported(), Vec::<Event>::new());
}
