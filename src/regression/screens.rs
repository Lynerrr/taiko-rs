//! Every screen as it's drawn: the texts on it, where they are and what can be clicked, kept
//! golden. A moved button, a lost label or a new one shows up as a diff of the screen's file.
//!
//! Linux only: other systems name keys, folders and renderers differently.
#![cfg(target_os = "linux")]

use super::harness::{self, *};
use super::maps::{self, Player};
use super::snapshot::assert_snapshot;
use crate::*;

/// Texts that change by themselves and so aren't kept: the frame rate counter (the frames are
/// drawn at made-up instants, with animations off, so the rest stays put) and the build the
/// update setting and the changelog name (a local build has none, CI's is its commit).
pub(super) const VOLATILE: &[&str] = &[
    " fps  ",
    "local build (no updates)",
    ", Enter to check",
    "ローカルビルド",
    "Enterで確認",
    "локальная сборка",
    "Enter — проверить",
    "this build: ",
    "a local build: the latest changes",
    "このビルド: ",
    "эта сборка: ",
];

/// The screen `s` is on, drawn until fades settle, as text.
fn shot(s: &mut State, now: &mut Instant) -> String {
    frame(s, *now);
    s.scene_since = *now - Duration::from_secs(5);
    s.toast = None;
    settle(s, now);
    *now += Duration::from_millis(16);
    screen(&frame(s, *now), VOLATILE)
}

fn client(tag: &str) -> (State, Songs, Instant) {
    let songs = Songs::new(tag, &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    (s, songs, now)
}

#[test]
fn song_select() {
    let (mut s, _songs, mut now) = client("screen-select");
    assert_snapshot("screen-song-select", &shot(&mut s, &mut now));
    // The mod select over it.
    s.cfg.mods = Mods::parse("HD,HR");
    s.select.mods_label = s.cfg.mods.label();
    assert_snapshot("screen-song-select-with-mods", &shot(&mut s, &mut now));
}

#[test]
fn main_menu() {
    let (mut s, _songs, mut now) = client("screen-menu");
    s.cfg.main_menu = true;
    s.main_menu = Some(menu::Menu::new());
    assert_snapshot("screen-main-menu", &shot(&mut s, &mut now));
    for (i, item) in menu::ITEMS.iter().enumerate() {
        let mut m = menu::Menu::new();
        m.pick(i);
        s.main_menu = Some(m);
        s.scene = SongSelect::scene();
        let name = format!("{item:?}").to_lowercase();
        assert_snapshot(&format!("screen-main-menu-{name}"), &shot(&mut s, &mut now));
    }
}

#[test]
fn settings_sections() {
    let (mut s, _songs, mut now) = client("screen-settings");
    s.open_settings();
    for sec in 0..settings::SECTIONS.len() {
        s.settings_ui.cursor = settings::section_start(sec);
        let name = settings::section_name(sec).to_lowercase();
        assert_snapshot(&format!("screen-settings-{name}"), &shot(&mut s, &mut now));
    }
}

#[test]
fn multiplayer_menu_download_and_editor_home() {
    let (mut s, _songs, mut now) = client("screen-tabs");
    s.open_multiplayer();
    assert_snapshot("screen-multiplayer-menu", &shot(&mut s, &mut now));
    // The download tab without asking the mirrors (nothing searched yet).
    s.scene = crate::screens::download::Download::scene();
    assert_snapshot("screen-download", &shot(&mut s, &mut now));
    s.scene =
        crate::screens::editor_home::EditorHome::scene(editor::home::Home::new(&[], "tester"));
    assert_snapshot("screen-editor-home", &shot(&mut s, &mut now));
    s.scene = SongSelect::scene();
    s.open_hud_editor();
    assert_snapshot("screen-interface-editor", &shot(&mut s, &mut now));
    s.scene = SongSelect::scene();
    s.open_layout_editor();
    assert_snapshot("screen-playfield-editor", &shot(&mut s, &mut now));
}

/// The map editor's screen on a small map, alone and in a mapping session (the participants'
/// panel, another player's playhead and selection, collab parts on the ruler).
#[test]
fn map_editor_alone_and_in_a_mapping_session() {
    use crate::editor::collab::session::tests::session;
    use taiko_proto::mapping::{MapDown, Part};
    let dir =
        std::env::temp_dir().join(format!("taiko-rs-screen-map-editor-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("map.osu");
    std::fs::write(&path, crate::editor::chart::tests::MAP).unwrap();
    let draw = |v: &mut editor::view::View| {
        let mut c = Canvas {
            audit: true,
            ..Default::default()
        };
        c.clear(WIDTH);
        v.draw(&mut c, 1.0, crate::autoplay::Settings::default());
        screen(&c, VOLATILE)
    };
    let mut v = editor::view::View::open(path.clone(), 0).unwrap();
    assert_snapshot("screen-map-editor", &draw(&mut v));
    // On a server that runs sessions: the menu bar's button, and the window it opens.
    let mut offered = editor::view::View::open(path.clone(), 0).unwrap();
    offered.sessions = true;
    draw(&mut offered);
    offered.open_session_window();
    assert_snapshot("screen-map-editor-session-window", &draw(&mut offered));
    let (mut session, mut ed) = session();
    ed.path = path;
    session.state.documents[0].parts = vec![
        Part {
            from: 0.0,
            to: 2000.0,
            assignee: Some(2),
        },
        Part {
            from: 2000.0,
            to: 12000.0,
            assignee: None,
        },
    ];
    let id = ed.chart.objects[1].id;
    let now = session.now();
    session.handle(
        &mut ed,
        MapDown::Presence {
            who: 2,
            diff: 0,
            time: 1500.0,
            cursor: Some(2000.0),
            selection: vec![id],
        },
        now,
    );
    session.handle(
        &mut ed,
        MapDown::Presence {
            who: 3,
            diff: 1,
            time: 500.0,
            cursor: None,
            selection: Vec::new(),
        },
        now,
    );
    session.by_author = true;
    v.ed = ed;
    v.collab = Some(Box::new(session));
    // The session's panel is shut: only its menu-bar button, nothing over the preview.
    assert_snapshot("screen-map-editor-session", &draw(&mut v));
    // Opened from that button: people, the host's buttons and the toggle that lets everyone
    // map anywhere, hanging under the menu bar.
    v.open_session_window();
    assert_snapshot("screen-map-editor-session-panel", &draw(&mut v));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn guide_pages() {
    let (mut s, _songs, mut now) = client("screen-guide");
    for page in 0..guide::PAGES {
        s.guide.page = Some(page);
        assert_snapshot(&format!("screen-guide-{page}"), &shot(&mut s, &mut now));
    }
}

#[test]
fn playing_paused_and_the_results() {
    let songs = Songs::new("screen-play", &[maps::basics()]);
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    press(&mut s, KeyCode::Enter, now);
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    // Halfway through, every note so far hit.
    let map = harness::game(&s).unwrap().map().clone();
    let presses = maps::presses(&map, Player::Jitter(30.0));
    let half = map.notes[map.notes.len() / 2].time;
    let keys = s.cfg.keys;
    let mut next = 0;
    while harness::game(&s).is_some_and(|g| g.time() < half) {
        let horizon = now + Duration::from_millis(4);
        while let Some(&(t, k)) = presses.get(next) {
            let at = instant_of(&s, now, t);
            if at > horizon {
                break;
            }
            press(&mut s, keys[k as usize], at.max(now));
            next += 1;
        }
        now = horizon;
        frame(&mut s, now);
    }
    // The pp counter's answer for the notes so far, worked out on its own thread.
    wait_idle(&mut s, now);
    let c = frame(&mut s, now);
    assert_snapshot("screen-play", &screen(&c, VOLATILE));
    press(&mut s, KeyCode::Escape, now);
    let c = frame(&mut s, now);
    assert_snapshot("screen-pause", &screen(&c, VOLATILE));
    // Results of a finished play.
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
    s.scene = crate::screens::results::ResultsScreen::scene(Box::new(r), 0, None);
    assert_snapshot("screen-results", &shot(&mut s, &mut now));
    let mut r = ui::Results::new(
        s.library[0].label(),
        game::Stats::default(),
        game::Windows::new(5.0),
    );
    r.shown = now - Duration::from_secs(10);
    r.failed = true;
    r.no_pp = Some("failed plays get no pp");
    s.scene = crate::screens::results::ResultsScreen::scene(Box::new(r), 0, None);
    assert_snapshot("screen-results-failed", &shot(&mut s, &mut now));
}

#[test]
fn every_screen_names_its_parts_in_every_language() {
    // The same screens in Japanese and Russian: every label is there (no English left over
    // where a translation is expected, nothing missing).
    for lang in [lang::Lang::Ja, lang::Lang::Ru] {
        let mut cfg = harness::config();
        cfg.language = lang;
        let songs = Songs::new(&format!("screen-lang-{lang:?}"), &[maps::basics()]);
        let mut s = state_with(cfg, songs.entries());
        let mut now = Instant::now();
        ready(&mut s, &mut now);
        let name = format!("{lang:?}").to_lowercase();
        assert_snapshot(
            &format!("screen-{name}-song-select"),
            &shot(&mut s, &mut now),
        );
        s.open_settings();
        assert_snapshot(&format!("screen-{name}-settings"), &shot(&mut s, &mut now));
        s.close_settings();
        s.open_multiplayer();
        assert_snapshot(
            &format!("screen-{name}-multiplayer-menu"),
            &shot(&mut s, &mut now),
        );
    }
    lang::set(lang::Lang::En);
}
