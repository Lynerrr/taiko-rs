//! The music player: its own speed, moved with `[` `]` as in song select and kept apart from the
//! game's, its keep-pitch switch and Daycore, and its music staying as loud with another window
//! in front.
use super::harness::{self, *};
use super::maps;
use crate::*;

/// A client on the main menu with the player open, on one song.
fn in_the_player(tag: &str, now: &mut Instant) -> State {
    let songs = Songs::new(tag, &[maps::basics()]);
    let mut s = state_with(harness::config(), songs.entries());
    ready(&mut s, now);
    s.cfg.main_menu = true;
    s.main_menu = Some(menu::Menu::new());
    s.music_control(player::Control::Open);
    assert!(s.player.is_some());
    s
}

fn rate(s: &State) -> (f64, bool) {
    s.player.as_ref().unwrap().rate()
}

fn bracket(s: &mut State, right: bool, shift: bool, at: Instant) {
    s.modifiers_shift = shift;
    let code = if right {
        KeyCode::BracketRight
    } else {
        KeyCode::BracketLeft
    };
    press(s, code, at);
    s.modifiers_shift = false;
}

/// Clicks the middle of the area that takes `hit`, as last drawn.
fn click(s: &mut State, now: &mut Instant, hit: hit::Hit) {
    *now += Duration::from_millis(16);
    frame(s, *now);
    let [x, y, w, h] = s
        .hits
        .iter()
        .find(|r| r.hit == hit)
        .unwrap_or_else(|| panic!("{hit:?} is drawn"))
        .rect;
    let k = s.window_h / render::VIRTUAL_HEIGHT;
    s.mouse_moved(f64::from((x + w / 2.0) * k), f64::from((y + h / 2.0) * k));
    s.mouse_button(true, None);
    *now += Duration::from_millis(16);
    frame(s, *now);
    s.mouse_button(false, None);
}

#[test]
fn brackets_move_the_players_own_speed_by_song_selects_steps() {
    let mut now = Instant::now();
    let mut s = in_the_player("player-brackets", &mut now);
    let game = (s.cfg.rate, s.cfg.rate_keep_pitch);
    assert_eq!(rate(&s), (1.0, true));
    bracket(&mut s, true, false, now);
    assert_eq!(rate(&s), (1.05, true), "] is 0.05x faster");
    bracket(&mut s, true, true, now);
    assert_eq!(rate(&s), (1.3, true), "Shift+] five steps at once");
    bracket(&mut s, false, false, now);
    assert_eq!(rate(&s), (1.25, true), "[ is slower");
    assert_eq!(s.preview_rate(), (1.25, true), "the music plays at it");
    for _ in 0..40 {
        bracket(&mut s, false, true, now);
    }
    assert_eq!(
        rate(&s).0,
        config::RATE_RANGE.0,
        "no slower than the game allows"
    );
    assert_eq!(
        (s.cfg.rate, s.cfg.rate_keep_pitch),
        game,
        "the game's speed in the settings stays as it was"
    );
}

#[test]
fn the_players_switch_keeps_the_pitch_and_daycore_stays() {
    let mut now = Instant::now();
    let mut s = in_the_player("player-keep-pitch", &mut now);
    let game = (s.cfg.rate, s.cfg.rate_keep_pitch);
    bracket(&mut s, true, false, now);
    let c = frame(&mut s, now);
    assert!(shows(&c, "1.05x [ ]"), "the speed and its keys");
    assert!(shows(&c, "keep pitch"));
    assert!(shows(&c, "DC"), "Daycore stays");
    for gone in ["DT", "NC", "HT"] {
        assert!(!c.texts.iter().any(|(_, t)| t.trim() == gone), "no {gone}");
    }

    click(&mut s, &mut now, hit::Hit::PlayerKeepPitch);
    assert_eq!(
        rate(&s),
        (1.05, false),
        "the switch lets the pitch go with the speed"
    );
    click(&mut s, &mut now, hit::Hit::PlayerKeepPitch);
    assert_eq!(rate(&s), (1.05, true));

    click(&mut s, &mut now, hit::Hit::PlayerDaycore);
    assert_eq!(rate(&s), (0.75, false), "Daycore over the speed picked");
    bracket(&mut s, true, false, now);
    assert_eq!(
        rate(&s),
        (0.8, true),
        "] goes on from Daycore's speed and turns it off"
    );
    assert!(!s.player.as_ref().unwrap().daycore);

    click(&mut s, &mut now, hit::Hit::PlayerRate);
    assert_eq!(
        rate(&s),
        (1.0, true),
        "a click on the speed puts it back to 1x"
    );
    assert_eq!((s.cfg.rate, s.cfg.rate_keep_pitch), game);
}

#[test]
fn the_player_stays_as_loud_with_another_window_in_front() {
    let mut now = Instant::now();
    let mut s = in_the_player("player-focus", &mut now);
    s.cfg.master_volume = 1.0;
    s.cfg.music_volume = 0.8;
    s.cfg.fx_volume = 0.4;
    s.cfg.unfocused_volume = 0.5;
    s.focused = false;
    assert_eq!(
        s.heard_volumes(),
        (0.8, 0.4),
        "music to listen to while doing something else"
    );
    press(&mut s, KeyCode::Escape, now);
    assert!(s.player.is_none());
    assert_eq!(
        s.heard_volumes(),
        (0.4, 0.2),
        "out of the player it's quieter again"
    );
}
