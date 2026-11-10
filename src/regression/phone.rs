//! The phone's interface (`mobile.rs`): its main screens as drawn on a phone-shaped window, the
//! texts on them, where they are and what can be clicked, kept golden like `screens`. A phone
//! build draws these on Android only; the tests switch them on for their own thread.
//!
//! Linux only, as `screens`: other systems name keys, folders and renderers differently.
#![cfg(target_os = "linux")]

use super::harness::{self, *};
use super::maps;
use super::screens::VOLATILE;
use super::snapshot::assert_snapshot;
use crate::*;

/// A 20:9 phone held sideways: the game's 480 units tall, this wide.
const PHONE_W: f32 = 480.0 * 20.0 / 9.0;

/// One frame at `now` on the phone-shaped window.
fn phone_frame(s: &mut State, now: Instant) -> Canvas {
    let mut c = Canvas {
        audit: true,
        ..Default::default()
    };
    s.build_frame(&mut c, now, PHONE_W);
    c
}

/// Draws until the screen stops changing, as `harness::settle` does on the monitor's width.
fn phone_settle(s: &mut State, now: &mut Instant) {
    let until = Instant::now() + Duration::from_secs(20);
    let mut last = String::new();
    let mut same = 0;
    while Instant::now() < until {
        *now += Duration::from_millis(16);
        let shown = screen(&phone_frame(s, *now), &[]);
        if shown == last {
            same += 1;
            if same >= 8 {
                return;
            }
        } else {
            same = 0;
            last = shown;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
}

/// The screen `s` is on, drawn until fades settle, as text.
fn shot(s: &mut State, now: &mut Instant) -> String {
    phone_frame(s, *now);
    s.scene_since = *now - Duration::from_secs(5);
    s.toast = None;
    phone_settle(s, now);
    *now += Duration::from_millis(16);
    screen(&phone_frame(s, *now), VOLATILE)
}

/// A phone client with the test charts, its song select ready.
fn client(tag: &str) -> (State, Songs, Instant) {
    mobile::phone_for_tests::set(true);
    let songs = Songs::new(tag, &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    s.window_w = 2400.0;
    s.window_h = 1080.0;
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    (s, songs, now)
}

#[test]
fn phone_song_select_and_mods() {
    let (mut s, _songs, mut now) = client("phone-select");
    assert_snapshot("phone-song-select", &shot(&mut s, &mut now));
    s.cfg.mods = Mods::parse("HD,HR");
    s.select.mods_label = s.cfg.mods.label();
    assert_snapshot("phone-song-select-with-mods", &shot(&mut s, &mut now));
}

#[test]
fn phone_settings_sections() {
    let (mut s, _songs, mut now) = client("phone-settings");
    s.open_settings();
    for sec in 0..settings::SECTIONS.len() {
        s.settings_ui.cursor = settings::section_start(sec);
        let name = settings::section_name(sec).to_lowercase();
        assert_snapshot(&format!("phone-settings-{name}"), &shot(&mut s, &mut now));
    }
}

#[test]
fn phone_multiplayer_menu_and_download() {
    let (mut s, _songs, mut now) = client("phone-tabs");
    s.open_multiplayer();
    assert_snapshot("phone-multiplayer-menu", &shot(&mut s, &mut now));
    s.scene = Scene::Download;
    assert_snapshot("phone-download", &shot(&mut s, &mut now));
}

#[test]
fn phone_results() {
    let (mut s, _songs, mut now) = client("phone-results");
    let stats = game::Stats {
        great: 60,
        ok: 3,
        miss: 1,
        max_combo: 50,
        score: 912_345,
        roll_ticks: 4,
        ..Default::default()
    };
    let mut r = ui::Results::new(s.library[0].label(), stats, game::Windows::new(5.0));
    r.shown = now - Duration::from_secs(10);
    r.pp = Some(123.4);
    s.scene = Scene::Results {
        results: Box::new(r),
        entry: 0,
        from: None,
    };
    assert_snapshot("phone-results", &shot(&mut s, &mut now));
}
