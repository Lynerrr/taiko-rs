//! Regression tests for Journey v1: entry, cold-start level picker, queue advancement,
//! Pause -> Quit state cleanup, results Next/Save/Like-autosave, online protocol flow +
//! pool_exhausted anti-spam, `from_journey` tagging, and default save-folder config persistence.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use super::harness::{self, *};
use super::maps;
use crate::config::{Config, JourneyFlow, JourneyFocus, JourneySaveTo};
use crate::journey::{Gate, MAX_PREFETCH_AHEAD, MapSource, OUTAGE_GRACE, PastOutcome, QueuedMap};
use crate::*;

#[test]
fn journey_cold_start_pick_level_and_skip() {
    let songs = Songs::new("journey-flow", &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);

    // Open Journey via the Song Select button hit target.
    s.click(hit::Hit::JourneyButton, None);
    now += Duration::from_millis(50);
    frame(&mut s, now);
    assert!(s.journey.active);
    assert_eq!(s.scene.screen().map(|sc| sc.name()), Some("Journey"));

    // Signed out, Journey asks first; on offline, it asks for a level.
    assert_eq!(s.journey.gate, Some(Gate::SignedOut));
    s.click(hit::Hit::JourneyOffline, None);
    now += Duration::from_millis(50);
    frame(&mut s, now);
    assert_eq!(s.journey.gate, None);
    assert_eq!(s.journey.offline, Some(Gate::SignedOut));
    assert!(s.journey.needs_level);

    // Pick comfort level (4 - 5*).
    s.click(hit::Hit::JourneyLevel(2), None);
    now += Duration::from_millis(50);
    frame(&mut s, now);
    assert!(!s.journey.needs_level);
    assert!(!s.journey.queue.is_empty());
    assert!(s.journey.queue.len() <= MAX_PREFETCH_AHEAD);

    let first_md5 = s.journey.current().unwrap().item.md5.clone();

    // Skip advances to the next map and records PastOutcome::Skipped on the top lane.
    press(&mut s, KeyCode::Tab, now);
    now += Duration::from_millis(50);
    frame(&mut s, now);
    assert_eq!(s.journey.step, 1);
    assert_eq!(s.journey.history.len(), 1);
    assert_eq!(s.journey.history[0].outcome, PastOutcome::Skipped);
    if let Some(cur) = s.journey.current() {
        assert_ne!(cur.item.md5, first_md5);
    }

    // Escape exits Journey cleanly back to Song Select.
    press(&mut s, KeyCode::Escape, now);
    now += Duration::from_millis(50);
    frame(&mut s, now);
    assert!(!s.journey.active);
    assert!(!s.journey.in_play);
    assert_eq!(s.scene.screen().map(|sc| sc.name()), Some("Select"));
}

#[test]
fn pause_quit_during_journey_returns_to_journey_and_exit_clears_state() {
    let songs = Songs::new("journey-pause-quit", &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);

    s.open_journey();
    // Signed out: on offline, with the library.
    s.journey_go_offline();
    s.journey_pick_level(2);
    assert!(s.journey.active);
    s.journey.in_play = true;

    // Leaving a play via Pause -> Quit calls show_select(); during Journey play it returns
    // to the Journey screen instead of dropping the player to Song Select / Main Menu.
    s.show_select();
    assert!(s.journey.active, "Pause -> Quit returns to Journey");
    assert!(!s.journey.in_play, "show_select must clear in_play");
    assert_eq!(s.scene.screen().map(|sc| sc.name()), Some("Journey"));

    // Explicitly exiting Journey from the Journey screen clears all state.
    s.exit_journey();
    assert!(!s.journey.active, "exit_journey must stop Journey");
    assert!(!s.journey.in_play);
}

#[test]
fn results_screen_next_and_save_on_like_in_journey() {
    let songs = Songs::new("journey-results", &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);

    s.open_journey();
    // Signed out: on offline, with the library.
    s.journey_go_offline();
    s.journey_pick_level(2);
    let step_before = s.journey.step;

    // Simulate a cached map played from Journey, then hitting Like with save_on_like enabled.
    let base = std::env::temp_dir().join(format!("taiko-journey-res-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let cached_dir = base.join("999 Test Artist - Test Song");
    std::fs::create_dir_all(&cached_dir).unwrap();
    let osu_path = cached_dir.join("test.osu");
    std::fs::copy(&s.library[0].path, &osu_path).unwrap();

    let item = online::JourneyItem {
        md5: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        map_hash: None,
        set_id: Some(999),
        custom_id: None,
        title: "Test Song".into(),
        artist: "Test Artist".into(),
        creator: "Tester".into(),
        version: s.library[0].meta.version.clone(),
        stars: 4.5,
        bpm: 180.0,
        length_ms: 60_000,
        role: "main".into(),
        reason: "Fits your comfort level".into(),
        scroll_velocity: 0.45,
        profile: Default::default(),
    };
    s.journey.in_play = true;
    s.journey.playing_cached = Some((cached_dir.clone(), item));
    s.cfg.journey_save_to = JourneySaveTo::Osu;
    let osu_dest = base.join("osu-songs");
    std::fs::create_dir_all(&osu_dest).unwrap();
    library::set_songs_folder(Some(osu_dest.clone()));

    // Save from results moves the cached folder into osu-songs and clears playing_cached.
    s.journey_save_current();
    assert!(s.journey.playing_cached.is_none());
    assert!(osu_dest.join("999 Test Artist - Test Song").is_dir());
    assert!(!cached_dir.exists());

    // Next map from results advances step and returns to the Journey screen.
    s.journey_next_from_results(false);
    assert_eq!(s.journey.step, step_before + 1);
    assert_eq!(s.scene.screen().map(|sc| sc.name()), Some("Journey"));

    library::set_songs_folder(None);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn online_journey_fetches_once_and_does_not_spam_when_pool_exhausted() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_srv = calls.clone();

    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_ok() && !line.is_empty() {
                calls_srv.fetch_add(1, Ordering::SeqCst);
                let reply = online::Reply::Journey(online::JourneyBatch {
                    needs_level: false,
                    comfort_stars: 5.0,
                    items: Vec::new(),
                    ..Default::default()
                });
                let mut w = stream;
                let _ = writeln!(w, "{}", serde_json::to_string(&reply).unwrap());
                let _ = w.flush();
            }
        }
    });

    let songs = Songs::new("journey-online", &maps::all());
    let mut cfg = harness::config();
    cfg.online_server = format!("127.0.0.1:{}", addr.port());
    cfg.online_token = "tok".into();
    let mut s = state_with(cfg, songs.entries());
    s.journey_supported = true;
    let mut now = Instant::now();
    ready(&mut s, &mut now);

    s.open_journey();
    // Poll until the mock server answers.
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline && !s.journey.pool_exhausted {
        now += Duration::from_millis(16);
        frame(&mut s, now);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        s.journey.pool_exhausted,
        "empty server batch marks pool_exhausted"
    );
    let count_after = calls.load(Ordering::SeqCst);
    // Drawing 10 more frames must NOT fire any more JourneyNext requests.
    for _ in 0..10 {
        now += Duration::from_millis(16);
        frame(&mut s, now);
    }
    assert_eq!(calls.load(Ordering::SeqCst), count_after);
}

#[test]
fn failed_download_play_advances_without_sending_skip() {
    let songs = Songs::new("journey-failed-dl", &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);

    s.open_journey();
    s.journey.needs_level = false;
    s.journey.queue.clear();
    s.journey.queue.push_back(QueuedMap {
        item: online::JourneyItem {
            md5: "deadbeefdeadbeefdeadbeefdeadbeef".into(),
            map_hash: None,
            set_id: Some(12345),
            custom_id: None,
            title: "Broken".into(),
            artist: "Nobody".into(),
            creator: "Nobody".into(),
            version: "Oni".into(),
            stars: 4.0,
            bpm: 160.0,
            length_ms: 60_000,
            role: "main".into(),
            reason: "Fits".into(),
            scroll_velocity: 0.45,
            profile: Default::default(),
        },
        source: MapSource::Failed("network error".into()),
    });
    s.journey_play_current();
    assert_eq!(s.journey.step, 1);
}

#[test]
fn journey_config_defaults_to_own_folder_and_round_trips() {
    let mut cfg = Config::default();
    assert_eq!(cfg.journey_save_to, JourneySaveTo::Own);
    assert!(cfg.journey_save_on_like);
    assert!(!cfg.journey_marathon_save);

    cfg.journey_save_to = JourneySaveTo::Osu;
    cfg.journey_save_on_like = false;
    cfg.journey_marathon_save = true;
    let text = cfg.to_text();
    let parsed = Config::parse_text(&text);
    assert_eq!(parsed.journey_save_to, JourneySaveTo::Osu);
    assert!(!parsed.journey_save_on_like);
    assert!(parsed.journey_marathon_save);
}

#[test]
fn played_attempt_from_journey_wire_compat() {
    let normal = online::PlayedAttempt {
        ended: online::AttemptEnd::Quit,
        at_ms: 10_000,
        length_ms: 90_000,
        from_ms: None,
        great: 50,
        ok: 5,
        miss: 2,
        max_combo: 30,
        mods: vec![],
        rate: 1.0,
        scroll_speed: 1.0,
        from_journey: false,
    };
    let json_normal = serde_json::to_string(&normal).unwrap();
    assert!(!json_normal.contains("from_journey"));

    let mut from_j = normal;
    from_j.from_journey = true;
    let json_j = serde_json::to_string(&from_j).unwrap();
    assert!(json_j.contains("\"from_journey\":true"));
}

/// A Journey item for set `set` and difficulty `version`.
fn item(title: &str, version: &str, set: u32) -> online::JourneyItem {
    online::JourneyItem {
        md5: format!("{set:032}"),
        map_hash: None,
        set_id: Some(set),
        custom_id: None,
        title: title.into(),
        artist: "Artist".into(),
        creator: "Mapper".into(),
        version: version.into(),
        stars: 4.5,
        bpm: 180.0,
        length_ms: 60_000,
        role: "main".into(),
        reason: "Fits your comfort level".into(),
        scroll_velocity: 0.45,
        profile: Default::default(),
    }
}

/// `src`'s chart, retitled and renamed as a difficulty.
fn chart_as(src: &std::path::Path, title: &str, version: &str) -> String {
    std::fs::read_to_string(src)
        .unwrap()
        .lines()
        .map(|l| {
            if l.starts_with("Title:") {
                format!("Title:{title}")
            } else if l.starts_with("Version:") {
                format!("Version:{version}")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A set unpacked in Journey's cache: its folder and the entry for `version` of its maps
/// (`(file name, version)`).
fn cached_set(
    folder_name: &str,
    src: &std::path::Path,
    title: &str,
    maps: &[(&str, &str)],
    version: &str,
) -> (std::path::PathBuf, library::Entry) {
    let folder = config::journey_cache_dir().join(folder_name);
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    for (file, v) in maps {
        std::fs::write(folder.join(file), chart_as(src, title, v)).unwrap();
    }
    let entry = crate::journey::pick_difficulty(
        crate::journey::scan_single_folder(&folder),
        &item(title, version, 1),
    )
    .expect("the difficulty asked for");
    (folder, entry)
}

fn copy(e: &library::Entry) -> library::Entry {
    library::Entry::from_parts(e.path.clone(), e.meta.clone(), e.size, e.modified)
}

/// Journey up with `queue` as its whole queue, asking for nothing more.
fn journey_with(s: &mut State, queue: Vec<QueuedMap>) {
    s.open_journey();
    s.journey.gate = None;
    s.journey.needs_level = false;
    s.journey.local_mode = true;
    s.journey.pool_exhausted = true;
    s.journey.queue = queue.into();
}

/// The preview plays Journey's map, not song select's: each cached map (always entry 0 of a
/// list of its own) is told from the last by its path, a library map by its index, and a map
/// still on its way plays nothing.
#[test]
fn the_journey_preview_follows_journey_maps_not_song_select() {
    let songs = Songs::new("journey-preview", &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    let one = cached_set(
        "3001 Artist - Cached One",
        &songs.paths[0],
        "Cached One",
        &[("a.osu", "Oni")],
        "Oni",
    );
    let two = cached_set(
        "3002 Artist - Cached Two",
        &songs.paths[0],
        "Cached Two",
        &[("a.osu", "Oni")],
        "Oni",
    );
    let local = s.library.len() - 1;
    let local_path = s.library[local].path.clone();
    assert_ne!(
        s.preview_target(),
        Some(local),
        "song select is on another map"
    );
    let cached = |(folder, entry): &(std::path::PathBuf, library::Entry)| MapSource::Cached {
        folder: folder.clone(),
        entry: Box::new(copy(entry)),
    };
    journey_with(
        &mut s,
        vec![
            QueuedMap {
                item: item("Cached One", "Oni", 3001),
                source: cached(&one),
            },
            QueuedMap {
                item: item("Cached Two", "Oni", 3002),
                source: cached(&two),
            },
            QueuedMap {
                item: item("Local", "Oni", 3003),
                source: MapSource::Local(local_path),
            },
            QueuedMap {
                item: item("Broken", "Oni", 3004),
                source: MapSource::Failed("network error".into()),
            },
        ],
    );
    settle(&mut s, &mut now);
    assert_eq!(s.preview.shown_title().as_deref(), Some("Cached One"));

    s.journey.advance(PastOutcome::Skipped);
    settle(&mut s, &mut now);
    assert_eq!(s.preview.shown_title().as_deref(), Some("Cached Two"));

    s.journey.advance(PastOutcome::Skipped);
    settle(&mut s, &mut now);
    assert_eq!(s.preview.requested(), Some(local));
    assert_eq!(
        s.preview.shown_title(),
        Some(s.library[local].meta.title.clone())
    );

    s.journey.advance(PastOutcome::Skipped);
    settle(&mut s, &mut now);
    assert_eq!(s.preview.requested(), None, "nothing to play yet");
    let _ = std::fs::remove_dir_all(&one.0);
    let _ = std::fs::remove_dir_all(&two.0);
}

/// A cached map played from Journey and saved from its results stays the same entry at the
/// same index, now where it was saved: Ctrl+R plays it again, and so does playing it from the
/// Journey screen after quitting, with the difficulty Journey asked for (not the set's first).
#[test]
fn a_cached_map_saved_from_its_results_keeps_its_place_and_plays_again() {
    let songs = Songs::new("journey-save-retry", &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    let name = "3101 Artist - Two Diffs";
    let (folder, entry) = cached_set(
        name,
        &songs.paths[0],
        "Two Diffs",
        &[("a-easy.osu", "Easy"), ("b-oni.osu", "Oni Two")],
        "Oni Two",
    );
    assert_eq!(entry.meta.version, "Oni Two");
    journey_with(
        &mut s,
        vec![QueuedMap {
            item: item("Two Diffs", "Oni Two", 3101),
            source: MapSource::Cached {
                folder: folder.clone(),
                entry: Box::new(entry),
            },
        }],
    );
    let before: Vec<std::path::PathBuf> = s.library.iter().map(|e| e.path.clone()).collect();

    s.journey_play_current();
    assert_eq!(s.scene.screen().map(|sc| sc.name()), Some("Loading"));
    let idx = before.len();
    assert_eq!(s.library.len(), idx + 1, "joins the end of the list");
    assert_eq!(s.library[idx].meta.version, "Oni Two");
    assert!(s.playing_journey());

    // Saved from its results.
    s.cfg.journey_save_to = JourneySaveTo::Own;
    s.journey_save_current();
    let dest = config::own_songs_dir().join(name);
    assert!(dest.join("b-oni.osu").is_file());
    assert!(!folder.exists());
    let after: Vec<std::path::PathBuf> = s.library[..idx].iter().map(|e| e.path.clone()).collect();
    assert_eq!(after, before, "no other map moved");
    assert_eq!(s.library[idx].path, dest.join("b-oni.osu"));
    assert_eq!(s.library[idx].meta.version, "Oni Two");
    assert!(s.library.iter().any(|e| e.path == dest.join("a-easy.osu")));
    assert!(s.journey.playing_cached.is_none());
    assert!(matches!(
        &s.journey.current().unwrap().source,
        MapSource::Saved(p) if *p == dest.join("b-oni.osu")
    ));
    let saved_len = s.library.len();

    // Ctrl+R: the same map again.
    s.retry_from(idx, None);
    assert_eq!(s.scene.screen().map(|sc| sc.name()), Some("Loading"));

    // Pause -> Quit, then Play on the Journey screen: the saved map, from the library.
    s.show_select();
    assert_eq!(s.scene.screen().map(|sc| sc.name()), Some("Journey"));
    s.journey_play_current();
    assert_eq!(s.scene.screen().map(|sc| sc.name()), Some("Loading"));
    assert_eq!(s.library.len(), saved_len);
    let _ = std::fs::remove_dir_all(&dest);
}

/// Song select reached any way but Journey's own exit (here its tab) ends the session, so a
/// play started there isn't taken for Journey's.
#[test]
fn song_select_by_its_tab_ends_journey() {
    let songs = Songs::new("journey-songs-tab", &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    s.open_journey();
    // Signed out: on offline, with the library.
    s.journey_go_offline();
    s.journey_pick_level(2);
    s.journey.in_play = true;
    s.show_songs();
    now += Duration::from_millis(16);
    frame(&mut s, now);
    assert!(!s.journey.active);
    assert!(!s.playing_journey());
}

#[test]
fn journey_mods_button_opens_the_mod_select_over_journey() {
    let songs = Songs::new("journey-mods", &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    s.open_journey();
    // Signed out: on offline, with the library.
    s.journey_go_offline();
    s.journey_pick_level(2);
    now += Duration::from_millis(16);
    frame(&mut s, now);

    // Clicked where it is drawn, as a player does.
    let (x, y) = s
        .hits
        .clone()
        .into_iter()
        .map(|h| (h.rect[0] + h.rect[2] / 2.0, h.rect[1] + h.rect[3] / 2.0))
        .find(|&(x, y)| s.hit_at(x, y) == Some(hit::Hit::ModsButton))
        .expect("Journey shows a Mods button");
    let k = s.window_h / render::VIRTUAL_HEIGHT;
    s.mouse_moved(f64::from(x * k), f64::from(y * k));
    s.mouse_button(true, None);
    now += Duration::from_millis(16);
    frame(&mut s, now);
    s.mouse_button(false, None);
    now += Duration::from_millis(16);
    frame(&mut s, now);
    assert!(
        s.mod_select.is_some(),
        "the Mods button opens the mod select"
    );
    assert!(
        s.hits.iter().any(|h| s
            .hit_at(h.rect[0] + 1.0, h.rect[1] + 1.0)
            .is_some_and(|hit| matches!(hit, hit::Hit::ModToggle(_)))),
        "the mod select is drawn over Journey"
    );

    // A mod clicked there is on for the next Journey play.
    let (x, y) = s
        .hits
        .clone()
        .into_iter()
        .map(|h| (h.rect[0] + h.rect[2] / 2.0, h.rect[1] + h.rect[3] / 2.0))
        .find(|&(x, y)| matches!(s.hit_at(x, y), Some(hit::Hit::ModToggle(_))))
        .unwrap();
    s.mouse_moved(f64::from(x * k), f64::from(y * k));
    s.mouse_button(true, None);
    now += Duration::from_millis(16);
    frame(&mut s, now);
    s.mouse_button(false, None);
    now += Duration::from_millis(16);
    frame(&mut s, now);
    assert!(s.mod_select.is_some());
    assert!(!s.cfg.mods.is_empty(), "the mod clicked is on");

    // Escape puts the mod select away and stays in Journey.
    press(&mut s, KeyCode::Escape, now);
    now += Duration::from_millis(16);
    frame(&mut s, now);
    assert!(s.mod_select.is_none());
    assert!(s.journey.active);
    assert_eq!(scene_name(&s), "Journey");
}

#[test]
fn escape_from_journey_results_goes_on_to_the_next_map() {
    let songs = Songs::new("journey-results-esc", &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    s.open_journey();
    // Signed out: on offline, with the library.
    s.journey_go_offline();
    s.journey_pick_level(2);
    now += Duration::from_millis(16);
    frame(&mut s, now);
    let played = s.journey.current().unwrap().item.md5.clone();
    let step = s.journey.step;

    // A map played from Journey, on its results.
    s.journey.in_play = true;
    let mut r = ui::Results::new(
        s.library[0].label(),
        game::Stats::default(),
        game::Windows::new(5.0),
    );
    r.shown = now - Duration::from_secs(10);
    s.scene = crate::screens::results::ResultsScreen::scene(Box::new(r), 0, None);
    now += Duration::from_millis(16);
    frame(&mut s, now);

    press(&mut s, KeyCode::Escape, now);
    now += Duration::from_millis(16);
    frame(&mut s, now);
    assert_eq!(scene_name(&s), "Journey");
    assert!(!s.journey.in_play);
    // Counted as played, not skipped, and the next map is up.
    assert_eq!(s.journey.step, step + 1);
    assert_eq!(
        s.journey.history.last().unwrap().outcome,
        PastOutcome::Passed
    );
    if let Some(next) = s.journey.current() {
        assert_ne!(next.item.md5, played);
    }
}

/// Signed out, Journey asks to log in, make an account or go on offline, and says what offline
/// leaves out; it fetches nothing meanwhile.
#[test]
fn signed_out_journey_offers_log_in_register_or_offline() {
    let songs = Songs::new("journey-signed-out", &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);

    s.click(hit::Hit::JourneyButton, None);
    now += Duration::from_millis(16);
    frame(&mut s, now);
    assert_eq!(s.journey.gate, Some(Gate::SignedOut));
    assert!(s.journey.queue.is_empty() && s.journey.fetching.is_none());
    for offer in [
        hit::Hit::JourneyLogin(false),
        hit::Hit::JourneyLogin(true),
        hit::Hit::JourneyOffline,
    ] {
        assert!(s.hits.iter().any(|h| h.hit == offer), "{offer:?} is drawn");
    }
    assert!(!s.hits.iter().any(|h| h.hit == hit::Hit::ModsButton));

    // Log in: the account form, Journey left.
    s.click(hit::Hit::JourneyLogin(false), None);
    frame(&mut s, now);
    assert!(!s.journey.active);
    assert_eq!(scene_name(&s), "MpMenu");
    assert_eq!(s.mp_menu.form, multiplayer::Form::Main);

    // Make an account: the sign-up form.
    s.open_journey();
    press(&mut s, KeyCode::KeyR, now);
    assert!(!s.journey.active);
    assert_eq!(s.mp_menu.form, multiplayer::Form::Register);

    // Offline: Journey picks from the library.
    s.open_journey();
    press(&mut s, KeyCode::KeyO, now);
    assert_eq!(s.journey.offline, Some(Gate::SignedOut));
    s.click(hit::Hit::JourneyLevel(2), None);
    now += Duration::from_millis(16);
    frame(&mut s, now);
    assert!(s.journey.local_mode);
    assert!(!s.journey.queue.is_empty());
}

/// A server answering the first `fail` Journey requests with a dropped connection, then with
/// an empty Journey batch.
fn flaky_server(fail: usize) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            // Only Journey's requests: the account check and the social connection go here too.
            if reader.read_line(&mut line).is_err() || !line.contains("ourney") {
                continue;
            }
            if counted.fetch_add(1, Ordering::SeqCst) < fail {
                drop(stream);
                continue;
            }
            let reply = online::Reply::Journey(online::JourneyBatch {
                needs_level: false,
                comfort_stars: 5.0,
                items: Vec::new(),
                ..Default::default()
            });
            let mut w = stream;
            let _ = writeln!(w, "{}", serde_json::to_string(&reply).unwrap());
            let _ = w.flush();
        }
    });
    (format!("127.0.0.1:{}", addr.port()), calls)
}

/// Frames until `done`, for at most 3 s.
fn frames_until(s: &mut State, now: &mut Instant, done: impl Fn(&State) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline && !done(s) {
        *now += Duration::from_millis(16);
        frame(s, *now);
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The server dropping for a few seconds goes unnoticed; down for longer with nothing to play,
/// Journey says so and offers to go on offline, keeps asking, and goes on by itself once the
/// server answers.
#[test]
fn a_short_server_drop_is_retried_quietly_and_a_long_one_offers_offline() {
    let (server, calls) = flaky_server(1);
    let songs = Songs::new("journey-outage", &maps::all());
    let mut cfg = harness::config();
    cfg.online_server = server;
    cfg.online_token = "tok".into();
    let mut s = state_with(cfg, songs.entries());
    s.journey_supported = true;
    let mut now = Instant::now();
    ready(&mut s, &mut now);

    s.open_journey();
    assert_eq!(s.journey.gate, None, "signed in: no offer");
    frames_until(&mut s, &mut now, |s| s.journey.outage_since.is_some());
    assert!(s.journey.outage_since.is_some(), "the first request failed");
    // Within the grace: still online, nothing asked.
    assert_eq!(s.journey.gate, None);
    assert!(!s.journey.local_mode);

    // Down for longer than the grace, with nothing to play: Journey asks.
    s.journey.outage_since = Some(Instant::now() - OUTAGE_GRACE - Duration::from_secs(1));
    now += Duration::from_millis(16);
    frame(&mut s, now);
    assert_eq!(s.journey.gate, Some(Gate::ServerDown));
    assert!(s.hits.iter().any(|h| h.hit == hit::Hit::JourneyOffline));

    // Retry: the server answers now, and Journey goes on online.
    s.click(hit::Hit::JourneyRetry, None);
    frames_until(&mut s, &mut now, |s| s.journey.gate.is_none());
    assert_eq!(s.journey.gate, None);
    assert_eq!(s.journey.outage_since, None);
    assert!(!s.journey.local_mode);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

/// Offline for an outage, Journey plays the library and asks the server again now and then:
/// once it answers, Journey is back online.
#[test]
fn offline_for_an_outage_goes_back_online_when_the_server_answers() {
    let (server, _calls) = flaky_server(0);
    let songs = Songs::new("journey-outage-offline", &maps::all());
    let mut cfg = harness::config();
    cfg.online_server = server;
    cfg.online_token = "tok".into();
    let mut s = state_with(cfg, songs.entries());
    s.journey_supported = true;
    let mut now = Instant::now();
    ready(&mut s, &mut now);

    s.open_journey();
    s.journey.fetching = None;
    s.journey.gate = Some(Gate::ServerDown);
    press(&mut s, KeyCode::KeyO, now);
    assert_eq!(s.journey.offline, Some(Gate::ServerDown));
    assert!(s.journey.local_mode);
    s.journey_pick_level(2);
    assert!(!s.journey.queue.is_empty(), "the library's maps");

    // The next probe is due: the server answers.
    s.journey.next_probe = Some(Instant::now());
    frames_until(&mut s, &mut now, |s| s.journey.offline.is_none());
    assert_eq!(s.journey.offline, None);
    assert!(!s.journey.local_mode, "back online");
    assert!(
        !s.journey.queue.is_empty(),
        "the library's maps stay queued"
    );
}

/// Back goes to the map before, by key or its button; moving on past it again sends no second
/// skip and keeps how it ended.
#[test]
fn journey_goes_back_and_forward_through_its_maps() {
    let songs = Songs::new("journey-back", &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    let queue = (0..3)
        .map(|i| QueuedMap {
            item: item(&format!("Song {i}"), "Oni", 7000 + i),
            source: MapSource::Local(s.library[i as usize].path.clone()),
        })
        .collect();
    journey_with(&mut s, queue);
    now += Duration::from_millis(16);
    frame(&mut s, now);
    let title = |s: &State| s.journey.current().unwrap().item.title.clone();
    assert!(
        !s.hits
            .iter()
            .any(|h| h.hit == hit::Hit::Key(KeyCode::ArrowLeft)),
        "nothing to go back to yet"
    );

    press(&mut s, KeyCode::ArrowRight, now);
    press(&mut s, KeyCode::Tab, now);
    assert_eq!(title(&s), "Song 2");
    now += Duration::from_millis(16);
    frame(&mut s, now);

    // The Back button, then the key.
    s.click(hit::Hit::Key(KeyCode::ArrowLeft), None);
    assert_eq!(title(&s), "Song 1");
    press(&mut s, KeyCode::ArrowLeft, now);
    assert_eq!(title(&s), "Song 0");
    assert_eq!(s.journey.step, 0);
    assert!(s.journey.history.is_empty());

    // Forward again through the same maps.
    press(&mut s, KeyCode::ArrowRight, now);
    assert_eq!(title(&s), "Song 1");
    assert_eq!(s.journey.history.len(), 1);
    assert_eq!(s.journey.history[0].outcome, PastOutcome::Skipped);
}

#[test]
fn journey_updates_stars_and_potential_pp_for_current_map_and_mods() {
    let songs = Songs::new("journey-live-stars-pp", &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);

    // Seed stale pp as left behind by Song Select.
    s.preview.potential = Some([999.0, 950.0, 900.0]);
    let map_path = s.library[0].path.clone();
    let bytes = std::fs::read(&map_path).unwrap();
    let nomod_sr = taiko_logic::stars::star_rating_for(&bytes, 1.0, mods::Mods::NONE, 1.0).unwrap();
    let nomod_pp = pp::potential(&map_path, 1.0, mods::Mods::NONE, 1.0).unwrap();
    let dt = mods::Mods::parse("DT");
    let dt_sr = taiko_logic::stars::star_rating_for(&bytes, 1.5, dt, 1.0).unwrap();
    let dt_pp = pp::potential(&map_path, 1.5, dt, 1.0).unwrap();
    assert!(dt_sr > nomod_sr + 0.05, "fixture must scale up with DT");

    let mut q_item = item("Star Test", "Oni", 8100);
    // Deliberately different from the real chart so we prove live calculation takes over.
    q_item.stars = 1.23;
    journey_with(
        &mut s,
        vec![QueuedMap {
            item: q_item,
            source: MapSource::Local(map_path),
        }],
    );

    // Wait for Potentials worker to populate real pp and stars for the Journey map.
    frames_until(&mut s, &mut now, |s| {
        s.preview
            .potential
            .is_some_and(|p| (p[0] - nomod_pp[0]).abs() < 0.5)
    });
    assert!(
        s.preview
            .potential
            .is_some_and(|p| (p[0] - nomod_pp[0]).abs() < 0.5),
        "Journey must replace stale Song Select pp with the current map's pp: {:?}",
        s.preview.potential
    );

    let c = frame(&mut s, now);
    let nomod_label = format!("{nomod_sr:.2}");
    assert!(
        c.texts.iter().any(|(_, t)| t == &nomod_label),
        "star chip must show live nomod rating {nomod_label}, saw: {:?}",
        c.texts.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>()
    );

    // Enable DT: both star rating and potential pp must update.
    s.cfg.mods = dt;
    frames_until(&mut s, &mut now, |s| {
        s.preview
            .potential
            .is_some_and(|p| (p[0] - dt_pp[0]).abs() < 0.5)
    });
    assert!(
        s.preview
            .potential
            .is_some_and(|p| (p[0] - dt_pp[0]).abs() < 0.5),
        "DT must update Journey potential pp"
    );
    let c_dt = frame(&mut s, now);
    let dt_label = format!("{dt_sr:.2}");
    assert!(
        c_dt.texts.iter().any(|(_, t)| t == &dt_label),
        "star chip must show DT star rating {dt_label}"
    );

    // A map still downloading/pending must clear potential pp rather than keep the previous one.
    s.journey.queue[0].source = MapSource::Pending;
    now += Duration::from_millis(16);
    frame(&mut s, now);
    assert_eq!(s.preview.potential, None);
}

#[test]
fn journey_embedded_preview_stays_inside_card_and_scrubs_on_drag() {
    let songs = Songs::new("journey-preview-ui", &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);

    // Simulate entering Journey straight from Main Menu (preview.bare still false).
    s.preview.bare = false;
    let map_path = s.library[0].path.clone();
    let mut q_item = item("Preview Bounds", "Oni", 8200);
    q_item.role = "explore".into();
    journey_with(
        &mut s,
        vec![QueuedMap {
            item: q_item,
            source: MapSource::Local(map_path),
        }],
    );
    s.preview.prime_with_marker(&s.library, 0, 1000.0);

    now += Duration::from_millis(300);
    let c = frame(&mut s, now);

    // Must never render the classic hint outside the Journey card.
    assert!(
        !c.texts.iter().any(|(_, t)| t.contains("wheel to scroll")),
        "embedded Journey timeline must not draw overflow scroll hint"
    );
    // Card spans y = 76..424 at virtual height 480; bottom action bar starts at 432.
    // No text may sit in the gap between the card bottom (424) and the action bar (432).
    for (range, text) in &c.texts {
        if range.is_empty() {
            continue;
        }
        let min_y = c.shapes[range.clone()]
            .iter()
            .map(|sh| sh.center[1] - sh.half[1])
            .fold(f32::INFINITY, f32::min);
        let max_y = c.shapes[range.clone()]
            .iter()
            .map(|sh| sh.center[1] + sh.half[1])
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(
            !(min_y >= 422.0 && max_y <= 434.0),
            "text {text:?} leaked outside card bottom ({min_y:.1}..{max_y:.1})"
        );
    }

    // The top bar's EXPLORE label must not be crossed by the horizontal track line (y = 48..50).
    let explore_range = c
        .texts
        .iter()
        .find(|(_, t)| t == "EXPLORE")
        .map(|(r, _)| r.clone())
        .expect("top bar EXPLORE label");
    let explore_left = c.shapes[explore_range.clone()]
        .iter()
        .map(|sh| sh.center[0] - sh.half[0])
        .fold(f32::INFINITY, f32::min);
    let track_crosses_label = c.shapes.iter().any(|sh| {
        let top = sh.center[1] - sh.half[1];
        let bot = sh.center[1] + sh.half[1];
        let right = sh.center[0] + sh.half[0];
        let h = bot - top;
        (h - 2.0).abs() < 0.2 && (top - 48.0).abs() < 0.5 && right > explore_left - 2.0
    });
    assert!(
        !track_crosses_label,
        "top lane track line must stop before the EXPLORE label (left={explore_left:.1})"
    );

    // Hovering and dragging over the Journey density graph must show Pointer and scrub.
    let rect = s
        .preview
        .graph_rect()
        .expect("Journey density graph rect must be set");
    let scale = s.window_h / render::VIRTUAL_HEIGHT;
    let to_px = |vx: f32, vy: f32| (f64::from(vx * scale), f64::from(vy * scale));
    let (gx0, gy) = (rect[0] + rect[2] * 0.25, rect[1] + rect[3] * 0.5);
    let (gx1, _) = (rect[0] + rect[2] * 0.75, gy);

    let cursor = s.mouse_moved(to_px(gx0, gy).0, to_px(gx0, gy).1);
    assert_eq!(
        cursor,
        winit::window::CursorIcon::Pointer,
        "hovering Journey timeline must show Pointer cursor"
    );

    s.mouse_button(true, None);
    assert!(
        s.scrubbing.is_some(),
        "pressing on Journey timeline starts scrub"
    );
    let first_t = s.scrubbing.unwrap().1;
    // Force throttle window elapsed so drag updates immediately.
    s.scrubbing = Some((Instant::now() - Duration::from_millis(100), first_t));
    let drag_cursor = s.mouse_moved(to_px(gx1, gy).0, to_px(gx1, gy).1);
    assert_eq!(drag_cursor, winit::window::CursorIcon::Pointer);
    let dragged_t = s.scrubbing.unwrap().1;
    assert!(
        dragged_t > first_t + 100.0,
        "dragging across Journey timeline must seek forward ({first_t} -> {dragged_t})"
    );
    s.mouse_button(false, None);
}

#[test]
fn journey_match_profile_gauge_marker_matches_its_zone() {
    let songs = Songs::new("journey-gauge", &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);

    // Comfort 7.76*, map 8.05* (+0.29*): stretch (red) marker must sit inside the red stretch zone.
    let mut q_item = item("Stretch Pick", "Oni", 8300);
    q_item.stars = 8.05;
    journey_with(
        &mut s,
        vec![QueuedMap {
            item: q_item,
            source: MapSource::Pending,
        }],
    );
    s.journey.comfort_stars = 7.76;

    now += Duration::from_millis(300);
    let c = frame(&mut s, now);

    let stretch_fill = render::rgba(0xff4d5a, 0.28);
    let comfort_fill = render::rgba(0x4ee07a, 0.35);
    let marker_fill = render::rgba(0xff4d5a, 1.0);
    let eq = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.02);

    let stretch = c
        .shapes
        .iter()
        .find(|sh| eq(sh.fill, stretch_fill) && (sh.half[1] * 2.0 - 4.0).abs() < 0.2)
        .expect("stretch zone rect");
    let comfort = c
        .shapes
        .iter()
        .find(|sh| eq(sh.fill, comfort_fill) && (sh.half[1] * 2.0 - 4.0).abs() < 0.2)
        .expect("comfort zone rect");
    let marker = c
        .shapes
        .iter()
        .find(|sh| {
            eq(sh.fill, marker_fill)
                && (sh.half[0] - 4.5).abs() < 0.2
                && (sh.center[1] - stretch.center[1]).abs() < 1.0
        })
        .expect("stretch marker circle");

    let comfort_right = comfort.center[0] + comfort.half[0];
    let stretch_left = stretch.center[0] - stretch.half[0];
    assert!(
        marker.center[0] >= stretch_left - 0.1,
        "red stretch marker ({:.1}) must sit in stretch zone (>= {:.1}), not comfort zone (<= {:.1})",
        marker.center[0],
        stretch_left,
        comfort_right
    );
}

#[test]
fn journey_level_picker_offers_auto_and_esc_returns_to_active_card() {
    let songs = Songs::new("journey-auto-level", &maps::all());
    let local_hash = beatmap::map_hash(&std::fs::read(&songs.paths[0]).unwrap());
    let mut local_item = item("Auto Test", "Oni", 8400);
    local_item.map_hash = Some(local_hash);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen_targets = Arc::new(std::sync::Mutex::new(Vec::<Option<f64>>::new()));
    let seen_srv = seen_targets.clone();

    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            // Only Journey's requests: the account check and social connection arrive here too.
            if reader.read_line(&mut line).is_err() || !line.contains("JourneyNext") {
                continue;
            }
            let val: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
            let target_stars = val.get("target_stars").and_then(|v| v.as_f64());
            seen_srv.lock().unwrap().push(target_stars);
            let reply = online::Reply::Journey(online::JourneyBatch {
                needs_level: false,
                comfort_stars: target_stars.unwrap_or(7.76),
                items: vec![local_item.clone()],
                ..Default::default()
            });
            let mut w = stream;
            let _ = writeln!(w, "{}", serde_json::to_string(&reply).unwrap());
            let _ = w.flush();
        }
    });

    let mut cfg = harness::config();
    cfg.online_server = format!("127.0.0.1:{}", addr.port());
    cfg.online_token = "tok".into();
    let mut s = state_with(cfg, songs.entries());
    s.journey_supported = true;
    let mut now = Instant::now();
    ready(&mut s, &mut now);

    s.open_journey();
    frames_until(&mut s, &mut now, |s| !s.journey.queue.is_empty());
    assert_eq!(s.journey.auto_comfort_stars, Some(7.76));
    assert_eq!(s.journey.target_stars, None);

    // Step manual level up to 8.00*, then open the level picker.
    s.click(hit::Hit::JourneyLevelStep(1), None);
    assert_eq!(s.journey.target_stars, Some(8.0));
    frames_until(&mut s, &mut now, |s| !s.journey.queue.is_empty());

    s.click(hit::Hit::JourneyLevelOpen, None);
    assert!(s.journey.needs_level);
    let c = frame(&mut s, now);
    assert!(
        s.hits.iter().any(|h| h.hit == hit::Hit::JourneyLevelAuto),
        "online level picker must offer Auto"
    );
    assert!(
        c.texts.iter().any(|(_, t)| t.contains("Auto (~7.76*)")),
        "Auto row must show server-estimated comfort stars"
    );

    // Esc closes the picker back to the active Journey card instead of exiting Journey.
    press(&mut s, KeyCode::Escape, now);
    assert!(!s.journey.needs_level);
    assert!(s.journey.active);
    assert_eq!(s.scene.screen().map(|sc| sc.name()), Some("Journey"));

    // Reopen and choose Auto: clears target_stars and requests fresh auto picks from server.
    s.click(hit::Hit::JourneyLevelOpen, None);
    s.click(hit::Hit::JourneyLevelAuto, None);
    assert!(!s.journey.needs_level);
    assert_eq!(s.journey.target_stars, None);
    assert_eq!(s.journey.comfort_stars, 7.76);
    frames_until(&mut s, &mut now, |s| !s.journey.queue.is_empty());
    let targets = seen_targets.lock().unwrap().clone();
    assert!(
        targets.len() >= 3,
        "expected at least 3 JourneyNext calls, saw {targets:?}"
    );
    assert_eq!(
        targets.last().copied().flatten(),
        None,
        "picking Auto must query the server with target_stars=None (saw {targets:?})"
    );
}

#[test]
fn journey_card_displays_map_status_badge() {
    let songs = Songs::new("journey-status", &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);

    // Graveyard osu! map shows GRAVEYARD rather than just "osu!".
    let mut gy_item = item("Graveyard Pick", "Oni", 9101);
    gy_item.stars = 6.5;
    s.set_status.insert(9101, Some("graveyard".into()));
    journey_with(
        &mut s,
        vec![QueuedMap {
            item: gy_item.clone(),
            source: MapSource::Pending,
        }],
    );
    now += Duration::from_millis(300);
    let c = frame(&mut s, now);
    assert!(
        c.texts.iter().any(|(_, t)| t == "GRAVEYARD"),
        "Journey card must display GRAVEYARD badge, saw: {:?}",
        c.texts.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>()
    );

    // Ranked map shows RANKED.
    s.set_status.insert(9101, Some("ranked".into()));
    now += Duration::from_millis(16);
    let c = frame(&mut s, now);
    assert!(c.texts.iter().any(|(_, t)| t == "RANKED"));

    // Custom approved vs unapproved custom map.
    let mut custom_item = item("Custom Pick", "Oni", 9102);
    custom_item.set_id = None;
    custom_item.custom_id = Some(42);
    s.journey.queue[0].item = custom_item;
    now += Duration::from_millis(16);
    let c = frame(&mut s, now);
    assert!(c.texts.iter().any(|(_, t)| t == "CUSTOM"));

    s.select.custom_approved.insert(42);
    now += Duration::from_millis(16);
    let c = frame(&mut s, now);
    assert!(c.texts.iter().any(|(_, t)| t == "CUSTOM APPROVED"));

    // Unknown osu! set triggers on-demand status lookup wait on Journey.
    let unknown_item = item("Fresh Set", "Oni", 9199);
    s.journey.queue[0].item = unknown_item;
    s.status_wait = None;
    now += Duration::from_millis(16);
    frame(&mut s, now);
    assert_eq!(
        s.status_wait.map(|(set, _)| set),
        Some(9199),
        "Journey must queue on-demand status lookup for unknown sets"
    );
}

#[test]
fn journey_rate_keys_and_stepper_change_speed_without_resetting_queue() {
    let songs = Songs::new("journey-rate", &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);

    let map_path = s.library[0].path.clone();
    let bytes = std::fs::read(&map_path).unwrap();
    let nomod_sr = taiko_logic::stars::star_rating_for(&bytes, 1.0, mods::Mods::NONE, 1.0).unwrap();
    let fast_sr = taiko_logic::stars::star_rating_for(&bytes, 1.25, mods::Mods::NONE, 1.0).unwrap();
    let fast_pp = pp::potential(&map_path, 1.25, mods::Mods::NONE, 1.0).unwrap();
    assert!(fast_sr > nomod_sr + 0.05, "fixture must scale up at 1.25x");

    let queue = vec![
        QueuedMap {
            item: item("Rate Song A", "Oni", 9201),
            source: MapSource::Local(map_path.clone()),
        },
        QueuedMap {
            item: item("Rate Song B", "Oni", 9202),
            source: MapSource::Local(s.library[1].path.clone()),
        },
    ];
    journey_with(&mut s, queue);
    s.journey.comfort_stars = 5.0;
    s.journey.target_stars = None;

    now += Duration::from_millis(300);
    let c = frame(&mut s, now);

    // Bottom bar shows the rate stepper with working [-] and [+] hit targets.
    assert!(
        c.texts.iter().any(|(_, t)| t == "1.00x"),
        "Journey bottom bar must show 1.00x rate stepper"
    );
    assert!(
        s.hits
            .iter()
            .any(|h| h.hit == hit::Hit::Key(KeyCode::BracketLeft))
            && s.hits
                .iter()
                .any(|h| h.hit == hit::Hit::Key(KeyCode::BracketRight)),
        "Journey bottom bar must expose BracketLeft/BracketRight stepper buttons"
    );

    // Pressing ] steps map speed up by 0.05x without switching Journey to manual level or wiping queue.
    press(&mut s, KeyCode::BracketRight, now);
    assert!((s.cfg.rate - 1.05).abs() < 1e-9, "rate was {}", s.cfg.rate);
    assert_eq!(
        s.journey.target_stars, None,
        "] must not alter target_stars"
    );
    assert_eq!(s.journey.queue.len(), 2, "] must not clear Journey queue");
    assert_eq!(s.journey.current().unwrap().item.title, "Rate Song A");

    // Clicking the stepper's - button twice steps down to 0.95x and updates the readout.
    s.click(hit::Hit::Key(KeyCode::BracketLeft), None);
    s.click(hit::Hit::Key(KeyCode::BracketLeft), None);
    assert!((s.cfg.rate - 0.95).abs() < 1e-9, "rate was {}", s.cfg.rate);
    assert_eq!(s.journey.current().unwrap().item.title, "Rate Song A");
    now += Duration::from_millis(16);
    let c = frame(&mut s, now);
    assert!(
        c.texts.iter().any(|(_, t)| t == "0.95x"),
        "stepper readout must update to 0.95x"
    );

    // Shift + ] jumps by +0.25x (to 1.20x) and live star rating updates on the same map.
    s.modifiers_shift = true;
    press(&mut s, KeyCode::BracketRight, now);
    s.modifiers_shift = false;
    press(&mut s, KeyCode::BracketRight, now);
    assert!((s.cfg.rate - 1.25).abs() < 1e-9, "rate was {}", s.cfg.rate);
    assert_eq!(s.journey.current().unwrap().item.title, "Rate Song A");
    let fast_label = format!("{fast_sr:.2}");
    frames_until(&mut s, &mut now, |s| {
        s.preview
            .potential
            .is_some_and(|p| (p[0] - fast_pp[0]).abs() < 0.5)
    });
    let c_fast = frame(&mut s, now);
    assert!(
        c_fast.texts.iter().any(|(_, t)| t == &fast_label),
        "star chip must update to 1.25x rating {fast_label}"
    );

    // Enabling DT hides the custom rate stepper (mod overrides speed) and shows the DT badge.
    s.cfg.mods = mods::Mods::parse("DT");
    now += Duration::from_millis(16);
    let c_dt = frame(&mut s, now);
    assert!(
        c_dt.texts.iter().any(|(_, t)| t == "DT"),
        "Journey bottom bar must show active DT mod badge"
    );
    assert!(
        !s.hits.iter().any(|h| matches!(
            h.hit,
            hit::Hit::Key(KeyCode::BracketLeft | KeyCode::BracketRight)
        )),
        "custom rate stepper must hide while DT overrides speed"
    );
}

/// The Discord status shows a Journey session: the Journey screen, and loading and the results
/// of a map played from it, while a map played from song select stays as it was.
#[test]
fn discord_status_shows_journey_maps_as_journey() {
    let songs = Songs::new("journey-presence", &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    s.open_journey();
    s.journey_go_offline();
    s.journey_pick_level(2);
    now += Duration::from_millis(16);
    frame(&mut s, now);
    let (details, state) = s.presence_text().unwrap();
    assert_eq!(state, "On a Journey");
    let cur = s.journey.current().unwrap();
    assert!(details.contains(&cur.item.title), "{details}");

    // Loading a map from Journey.
    s.journey.in_play = true;
    s.start_loading_from(0, None);
    assert_eq!(scene_name(&s), "Loading");
    assert!(
        s.journey.active && s.journey.in_play,
        "active={} in_play={}",
        s.journey.active,
        s.journey.in_play
    );
    let (details, state) = s.presence_text().unwrap();
    assert_eq!(details, s.library[0].label());
    assert_eq!(state, "Journey: Loading...");

    // Its results.
    let results = |s: &State| {
        let mut r = ui::Results::new(
            s.library[0].label(),
            game::Stats::default(),
            game::Windows::new(5.0),
        );
        r.shown = now;
        crate::screens::results::ResultsScreen::scene(Box::new(r), 0, None)
    };
    s.scene = results(&s);
    assert_eq!(
        s.presence_text().unwrap().1,
        "Journey: Looking at the results"
    );

    // The same results outside Journey say nothing of it.
    s.journey.in_play = false;
    s.scene = results(&s);
    assert_eq!(s.presence_text().unwrap().1, "Looking at the results");
}

/// The fields of the `JourneyNext` request somewhere in a message as sent.
fn journey_next_of(val: &serde_json::Value) -> Option<&serde_json::Value> {
    match val {
        serde_json::Value::Object(m) => m
            .get("JourneyNext")
            .or_else(|| m.values().find_map(journey_next_of)),
        _ => None,
    }
}

/// Journey's gear opens its settings: a focus or a chill session is kept in the config and asks
/// the server again with it, the queue filled anew; Esc closes the panel, not Journey.
#[test]
fn journey_settings_ask_the_server_for_a_focus_and_a_chill_session() {
    let songs = Songs::new("journey-settings", &maps::all());
    let local_hash = beatmap::map_hash(&std::fs::read(&songs.paths[0]).unwrap());
    let mut local_item = item("Settings Test", "Oni", 8500);
    local_item.map_hash = Some(local_hash);

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    type SeenQuery = (Option<String>, bool, Option<u64>, Option<u64>);
    let seen = Arc::new(std::sync::Mutex::new(Vec::<SeenQuery>::new()));
    let seen_srv = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() || !line.contains("JourneyNext") {
                continue;
            }
            let val: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
            let val = journey_next_of(&val).cloned().unwrap_or_default();
            let focus = val
                .get("focus")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let chill = val.get("chill").and_then(|v| v.as_bool()).unwrap_or(false);
            let min_len = val.get("min_length_ms").and_then(|v| v.as_u64());
            let max_len = val.get("max_length_ms").and_then(|v| v.as_u64());
            seen_srv
                .lock()
                .unwrap()
                .push((focus, chill, min_len, max_len));
            let reply = online::Reply::Journey(online::JourneyBatch {
                needs_level: false,
                comfort_stars: 6.0,
                items: vec![local_item.clone()],
                ..Default::default()
            });
            let mut w = stream;
            let _ = writeln!(w, "{}", serde_json::to_string(&reply).unwrap());
            let _ = w.flush();
        }
    });

    let mut cfg = harness::config();
    cfg.online_server = format!("127.0.0.1:{}", addr.port());
    cfg.online_token = "tok".into();
    let mut s = state_with(cfg, songs.entries());
    s.journey_supported = true;
    let mut now = Instant::now();
    ready(&mut s, &mut now);

    s.open_journey();
    frames_until(&mut s, &mut now, |s| !s.journey.queue.is_empty());
    assert_eq!(
        seen.lock().unwrap().last().cloned(),
        Some((None, false, None, None)),
        "a plain Journey asks for no focus, no chill, and no length bounds"
    );

    // The gear sits on the action bar and opens the modal panel.
    let c = frame(&mut s, now);
    assert!(s.hits.iter().any(|h| h.hit == hit::Hit::JourneySettings));
    drop(c);
    s.click(hit::Hit::JourneySettings, None);
    assert_eq!(s.journey.settings, Some(0));
    let c = frame(&mut s, now);
    assert!(c.texts.iter().any(|(_, t)| t == "JOURNEY SETTINGS"));
    // Modal blocks underlying Journey controls and hides marathon-only options in Manual mode.
    assert!(!s.hits.iter().any(|h| h.hit == hit::Hit::ModsButton));
    assert!(!shows(&c, "Hardcore"));
    assert!(!shows(&c, "Between maps"));
    assert!(!shows(&c, "Skip intros"));
    assert!(!shows(&c, "Save maps"));
    assert!(
        s.hits
            .iter()
            .any(|h| h.hit == hit::Hit::JourneyFocus(JourneyFocus::Streams))
    );

    // Focus on streams: kept, and the queue is filled again with it.
    let asked = seen.lock().unwrap().len();
    s.click(hit::Hit::JourneyFocus(JourneyFocus::Streams), None);
    assert_eq!(s.cfg.journey_focus, JourneyFocus::Streams);
    assert_eq!(s.journey.focus, JourneyFocus::Streams);
    frames_until(&mut s, &mut now, |s| !s.journey.queue.is_empty());
    assert!(
        seen.lock().unwrap()[asked..].contains(&(Some("streams".into()), false, None, None)),
        "{:?}",
        seen.lock().unwrap()
    );

    // Arrow keys work the panel: down to Length -> Standard (1-4m), down to Chill -> On.
    let asked = seen.lock().unwrap().len();
    press(&mut s, KeyCode::ArrowDown, now);
    s.click(
        hit::Hit::JourneyLength(crate::config::JourneyLength::Standard),
        None,
    );
    assert_eq!(s.cfg.journey_length, crate::config::JourneyLength::Standard);
    frames_until(&mut s, &mut now, |s| !s.journey.queue.is_empty());
    assert!(
        seen.lock().unwrap()[asked..].contains(&(
            Some("streams".into()),
            false,
            Some(60_000),
            Some(240_000)
        )),
        "{:?}",
        seen.lock().unwrap()
    );

    let asked = seen.lock().unwrap().len();
    press(&mut s, KeyCode::ArrowDown, now);
    press(&mut s, KeyCode::ArrowRight, now);
    assert!(s.cfg.journey_chill);
    assert!(s.journey.chill);
    frames_until(&mut s, &mut now, |s| !s.journey.queue.is_empty());
    assert!(
        seen.lock().unwrap()[asked..].contains(&(
            Some("streams".into()),
            true,
            Some(60_000),
            Some(240_000)
        )),
        "{:?}",
        seen.lock().unwrap()
    );

    // Non-stop changes nothing asked of the server when length is explicit: the queue stays,
    // and only Skip intros appears (not Hardcore / Between maps).
    let queued = s.journey.queue.front().map(|q| q.item.md5.clone());
    s.click(hit::Hit::JourneyFlow(JourneyFlow::Nonstop), None);
    assert_eq!(s.cfg.journey_flow, JourneyFlow::Nonstop);
    assert_eq!(s.journey.queue.front().map(|q| q.item.md5.clone()), queued);
    let c = frame(&mut s, now);
    assert!(shows(&c, "Skip intros"));
    assert!(!shows(&c, "Hardcore"));
    assert!(!shows(&c, "Between maps"));
    assert!(!shows(&c, "Save maps"));

    // Marathon reveals Hardcore, Between maps, and Save maps, and caps Length::Any at 5 minutes.
    s.click(
        hit::Hit::JourneyLength(crate::config::JourneyLength::Any),
        None,
    );
    let asked = seen.lock().unwrap().len();
    s.click(hit::Hit::JourneyFlow(JourneyFlow::Marathon), None);
    let c = frame(&mut s, now);
    assert!(shows(&c, "Hardcore"));
    assert!(shows(&c, "Between maps"));
    assert!(shows(&c, "Save maps"));
    assert!(!s.cfg.journey_marathon_save);
    s.click(hit::Hit::JourneySetting(7, 1), None);
    assert!(s.cfg.journey_marathon_save);
    s.click(hit::Hit::JourneySetting(7, 1), None);
    assert!(!s.cfg.journey_marathon_save);
    frames_until(&mut s, &mut now, |s| !s.journey.queue.is_empty());
    assert!(
        seen.lock().unwrap()[asked..].contains(&(
            Some("streams".into()),
            true,
            None,
            Some(crate::config::MARATHON_DEFAULT_MAX_LENGTH_MS)
        )),
        "{:?}",
        seen.lock().unwrap()
    );

    // Esc closes the panel; Journey stays, and opened again it keeps the settings.
    press(&mut s, KeyCode::Escape, now);
    assert_eq!(s.journey.settings, None);
    assert!(s.journey.active);
    assert_eq!(scene_name(&s), "Journey");
    s.open_journey();
    assert_eq!(s.journey.focus, JourneyFocus::Streams);
    assert!(s.journey.chill);
    assert!(s.journey.marathon_cap);
}

/// Offline, a chill session goes without the stretch maps, and the level asked for stays.
#[test]
fn an_offline_chill_journey_has_no_stretch_maps() {
    let songs = Songs::new("journey-chill", &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    s.open_journey();
    s.journey_go_offline();
    s.journey_pick_level(2);
    let roles = |s: &State| -> Vec<String> {
        s.journey
            .queue
            .iter()
            .map(|q| q.item.role.clone())
            .collect()
    };
    let mut seen = Vec::new();
    for _ in 0..6 {
        seen.extend(roles(&s));
        s.journey_skip_current();
    }
    assert!(
        seen.iter().any(|r| r == "stretch"),
        "a plain session stretches: {seen:?}"
    );

    let level = s.journey.comfort_stars;
    s.journey_change_setting(2, 1);
    assert!(s.journey.chill);
    assert_eq!(s.journey.comfort_stars, level);
    let mut seen = Vec::new();
    for _ in 0..6 {
        seen.extend(roles(&s));
        s.journey_skip_current();
    }
    assert!(!seen.is_empty());
    assert!(!seen.iter().any(|r| r == "stretch"), "chill: {seen:?}");
}

/// Non-stop: a Journey play's results go on to the next map by themselves, which starts as
/// soon as it's ready; Esc on the results stops there instead.
#[test]
fn nonstop_journey_goes_from_results_straight_into_the_next_map() {
    let songs = Songs::new("journey-nonstop", &maps::all());
    let mut cfg = harness::config();
    cfg.journey_flow = JourneyFlow::Nonstop;
    let mut s = state_with(cfg, songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    s.open_journey();
    s.journey_go_offline();
    s.journey_pick_level(2);
    now += Duration::from_millis(16);
    frame(&mut s, now);
    let step = s.journey.step;

    let results_up = |s: &mut State, now: Instant, shown_ago: u64| {
        s.journey.in_play = true;
        let mut r = ui::Results::new(
            s.library[0].label(),
            game::Stats::default(),
            game::Windows::new(5.0),
        );
        r.shown = now - Duration::from_secs(shown_ago);
        s.scene = crate::screens::results::ResultsScreen::scene(Box::new(r), 0, None);
    };

    // Fresh results count down, and stay.
    results_up(&mut s, now, 1);
    now += Duration::from_millis(16);
    let c = frame(&mut s, now);
    assert_eq!(scene_name(&s), "Results");
    assert!(c.texts.iter().any(|(_, t)| t.starts_with("Next map in")));

    // Past the countdown, on to the next map, which starts by itself.
    now += Duration::from_secs(5);
    frame(&mut s, now);
    assert_eq!(s.journey.step, step + 1);
    assert_eq!(
        s.journey.history.last().unwrap().outcome,
        PastOutcome::Passed
    );
    now += Duration::from_millis(16);
    frame(&mut s, now);
    assert!(s.journey.in_play, "the next map starts without a key");
    assert_ne!(scene_name(&s), "Journey");

    // Esc on the results: the next map is up, waiting.
    s.scene = crate::screens::journey::JourneyScreen::scene();
    results_up(&mut s, now, 1);
    press(&mut s, KeyCode::Escape, now);
    for _ in 0..3 {
        now += Duration::from_millis(16);
        frame(&mut s, now);
    }
    assert_eq!(scene_name(&s), "Journey");
    assert!(!s.journey.in_play);

    // Reading the results (scrolling them) holds the countdown.
    results_up(&mut s, now, 1);
    press(&mut s, KeyCode::ArrowDown, now);
    now += Duration::from_secs(10);
    frame(&mut s, now);
    assert_eq!(scene_name(&s), "Results");
}

/// Non-stop shows what's up next near a map's end and in its breaks, and N in a play says no
/// to it: it leaves the queue and the server hears of it as a skip.
#[test]
fn nonstop_shows_up_next_in_breaks_and_n_says_no_to_it() {
    let songs = Songs::new("journey-up-next", &maps::all());
    let text = std::fs::read_to_string(&songs.paths[0]).unwrap();
    let map = beatmap::from_text(&text).unwrap();
    let note_at = |t: f64| {
        let mut n = map.notes[0].clone();
        n.time = t;
        n.end_time = t;
        n
    };
    // Notes every 200 ms to 10 s, a 10 s break, then on to 60 s.
    let mut notes: Vec<_> = (0..=50).map(|i| note_at(i as f64 * 200.0)).collect();
    notes.extend((0..=200).map(|i| note_at(20_000.0 + i as f64 * 200.0)));
    use crate::screens::journey::up_next_due;
    assert!(!up_next_due(&notes, 5_000.0), "mid-stream");
    assert!(!up_next_due(&notes, 10_100.0), "right as the break starts");
    assert!(up_next_due(&notes, 14_000.0), "in the break");
    assert!(
        !up_next_due(&notes, 19_000.0),
        "right before the break ends"
    );
    assert!(!up_next_due(&notes, 40_000.0), "mid-map");
    assert!(up_next_due(&notes, 50_000.0), "towards the end");
    assert!(up_next_due(&notes, 61_000.0), "after the last note");

    let mut cfg = harness::config();
    cfg.journey_flow = JourneyFlow::Nonstop;
    let mut s = state_with(cfg, songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    journey_with(
        &mut s,
        vec![
            QueuedMap {
                item: item("Playing", "Oni", 9201),
                source: MapSource::Pending,
            },
            QueuedMap {
                item: item("Not This", "Oni", 9202),
                source: MapSource::Pending,
            },
            QueuedMap {
                item: item("This One", "Oni", 9203),
                source: MapSource::Pending,
            },
        ],
    );
    s.journey.in_play = true;
    s.journey_skip_upcoming();
    let titles: Vec<_> = s
        .journey
        .queue
        .iter()
        .map(|q| q.item.title.as_str())
        .collect();
    assert_eq!(titles, ["Playing", "This One"]);
    // The map played isn't touched: with nothing after it, N does nothing.
    s.journey_skip_upcoming();
    s.journey_skip_upcoming();
    assert_eq!(s.journey.queue.len(), 1);
    assert_eq!(s.journey.queue[0].item.title, "Playing");
}

/// A marathon Journey up on the wheel, its first map loading: offline, with the library.
fn marathon(name: &str, cfg: Config) -> (State, Songs, Instant) {
    let songs = Songs::new(name, &maps::all());
    let mut cfg = cfg;
    cfg.journey_flow = JourneyFlow::Marathon;
    let mut s = state_with(cfg, songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    s.open_journey();
    s.journey_go_offline();
    s.journey_pick_level(2);
    now += Duration::from_millis(16);
    frame(&mut s, now);
    press(&mut s, KeyCode::Enter, now);
    assert_eq!(
        scene_name(&s),
        "Marathon",
        "Play starts a marathon on its wheel"
    );
    assert!(s.journey.marathon.is_some());
    (s, songs, now)
}

/// The play up now ends with every note gone by unhit (a fail), its outro skipped.
fn miss_the_rest(s: &mut State, now: &mut Instant) {
    let game = s.play.as_deref_mut().expect("a play");
    game.run_out_of_notes();
    press(s, KeyCode::Escape, *now);
    assert!(wait_for(s, now, |s| scene_name(s) != "Play"));
}

/// A marathon goes from map to map with no results: the wheel turns to the next map, which
/// starts by itself past its intro, and each map is counted as Journey counts a play.
#[test]
fn a_marathon_goes_from_map_to_map_with_no_results() {
    let (mut s, _songs, mut now) = marathon("journey-marathon", harness::config());
    let c = frame(&mut s, now);
    assert!(shows(&c, "Up next"));
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    let first = s.journey.current().unwrap().item.md5.clone();
    let game = harness::game(&s).unwrap();
    assert!(!game.can_skip(), "the intro is skipped");

    // Played through: no results, on to the wheel and the next map.
    let map = game.map().clone();
    super::flows::play_out(
        &mut s,
        &mut now,
        &maps::presses(&map, maps::Player::Perfect),
    );
    assert_eq!(scene_name(&s), "Marathon", "no results in a marathon");
    let m = s.journey.marathon.as_ref().unwrap();
    assert_eq!((m.passed, m.failed, m.skipped), (1, 0, 0));
    assert_eq!(m.last.as_ref().unwrap().item.md5, first);
    assert_eq!(s.journey.step, 1);
    assert_eq!(
        s.journey.history.last().unwrap().outcome,
        PastOutcome::Passed
    );
    let c = frame(&mut s, now);
    assert!(
        c.texts.iter().any(|(_, t)| t.starts_with("PASS")),
        "the last map's card"
    );
    assert!(c.texts.iter().any(|(_, t)| t.starts_with("MARATHON")));

    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    assert_ne!(s.journey.current().unwrap().item.md5, first);

    // A fail goes on too (not hardcore).
    miss_the_rest(&mut s, &mut now);
    assert_eq!(scene_name(&s), "Marathon");
    let m = s.journey.marathon.as_ref().unwrap();
    assert_eq!((m.passed, m.failed), (1, 1));
    assert!(m.ended.is_none());
    assert_eq!(
        s.journey.history.last().unwrap().outcome,
        PastOutcome::Failed
    );
}

/// Hardcore: the first failed map ends the marathon, on its summary; Enter goes back to Journey.
#[test]
fn a_hardcore_marathon_ends_on_its_first_fail() {
    let mut cfg = harness::config();
    cfg.journey_hardcore = true;
    let (mut s, _songs, mut now) = marathon("journey-hardcore", cfg);
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    miss_the_rest(&mut s, &mut now);
    assert_eq!(scene_name(&s), "Marathon");
    let m = s.journey.marathon.as_ref().unwrap();
    assert_eq!(
        m.ended.map(|(why, _)| why),
        Some(crate::journey::MarathonEnd::Hardcore)
    );
    let c = frame(&mut s, now);
    assert!(shows(&c, "HARDCORE: A FAIL ENDS IT"));
    // It stays on the summary.
    for _ in 0..10 {
        now += Duration::from_millis(500);
        frame(&mut s, now);
    }
    assert_eq!(scene_name(&s), "Marathon");
    press(&mut s, KeyCode::Enter, now);
    assert_eq!(scene_name(&s), "Journey");
    assert!(s.journey.marathon.is_none());
    assert!(s.journey.active);
}

/// A marathon's pause menu skips the map being played (no score, a skip for Journey) or ends
/// the marathon, which shows what it came to.
#[test]
fn a_marathons_pause_menu_skips_the_map_or_ends_the_marathon() {
    let (mut s, _songs, mut now) = marathon("journey-marathon-pause", harness::config());
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    let first = s.journey.current().unwrap().item.md5.clone();
    let scores = s.scores.len();

    press(&mut s, KeyCode::Escape, now);
    now += Duration::from_millis(400);
    let c = frame(&mut s, now);
    assert!(shows(&c, "Skip this map"));
    assert!(shows(&c, "End the marathon"));
    press(&mut s, KeyCode::ArrowDown, now);
    press(&mut s, KeyCode::Enter, now);
    assert_eq!(scene_name(&s), "Marathon");
    let m = s.journey.marathon.as_ref().unwrap();
    assert_eq!((m.passed, m.failed, m.skipped), (0, 0, 1));
    assert_eq!(
        s.journey.history.last().unwrap().outcome,
        PastOutcome::Skipped
    );
    assert_eq!(s.scores.len(), scores, "a skipped map keeps no score");
    let c = frame(&mut s, now);
    assert!(shows(&c, "SKIPPED"));

    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    assert_ne!(s.journey.current().unwrap().item.md5, first);
    // Up from Continue is the last item: End the marathon.
    press(&mut s, KeyCode::Escape, now);
    press(&mut s, KeyCode::ArrowUp, now);
    press(&mut s, KeyCode::Enter, now);
    assert_eq!(scene_name(&s), "Marathon");
    assert!(!s.journey.in_play);
    let c = frame(&mut s, now);
    assert!(shows(&c, "MARATHON OVER"));
    assert!(shows(&c, "Maps played"));
    press(&mut s, KeyCode::Escape, now);
    assert_eq!(scene_name(&s), "Journey");
    assert!(s.journey.marathon.is_none());
}

/// Between two maps, Space holds the marathon and Enter starts the next map at once; Esc
/// ends it there.
#[test]
fn a_marathons_wheel_holds_goes_on_and_ends_by_key() {
    let (mut s, _songs, mut now) = marathon("journey-marathon-wheel", harness::config());
    press(&mut s, KeyCode::Space, now);
    for _ in 0..20 {
        now += Duration::from_millis(250);
        frame(&mut s, now);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(scene_name(&s), "Marathon", "held");
    let c = frame(&mut s, now);
    assert!(shows(&c, "Held: Space to go on"));
    press(&mut s, KeyCode::Space, now);
    press(&mut s, KeyCode::Enter, now);
    // Enter doesn't wait out the wheel: it starts once loaded.
    let start = now;
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    assert!(now.duration_since(start) < crate::screens::marathon::MARATHON_GAP);

    miss_the_rest(&mut s, &mut now);
    press(&mut s, KeyCode::Escape, now);
    assert_eq!(scene_name(&s), "Marathon");
    assert!(s.journey.marathon.as_ref().unwrap().ended.is_some());
    assert!(!s.journey.in_play);
}

/// A marathon's combo carries on from map to map until a miss, a play that isn't the player's
/// own, or a map left partway; its best is kept.
#[test]
fn a_marathon_combo_carries_from_map_to_map_until_it_breaks() {
    use crate::journey::Marathon;
    let mut m = Marathon::new(Instant::now());
    m.map_started();
    assert_eq!(m.track_combo(120, 0, true), (0, 120));
    m.map_ended();
    assert_eq!(m.combo, 120);
    // The next map adds to it.
    m.map_started();
    assert_eq!(m.combo_now, 120);
    assert_eq!(m.track_combo(80, 0, true), (120, 200));
    // A miss breaks it: from there on only the map's own combo counts.
    assert_eq!(m.track_combo(0, 1, true).1, 0);
    assert_eq!(m.track_combo(30, 1, true).1, 30);
    m.map_ended();
    assert_eq!(m.combo, 30);
    assert_eq!(m.best_combo, 200);
    // A retry starts the map over from the combo carried into it.
    m.map_started();
    assert_eq!(m.track_combo(10, 0, true).1, 40);
    m.map_started();
    assert_eq!(m.track_combo(5, 0, true).1, 35);
    // Not the player's own play (Autoplay, unranked mods): nothing carries on.
    assert_eq!(m.track_combo(6, 0, false).1, 0);
    m.map_ended();
    assert_eq!(m.combo, 0);
    // Left partway: broken.
    m.map_started();
    m.track_combo(50, 0, true);
    m.combo_broken();
    assert_eq!((m.combo, m.combo_now), (0, 0));
    assert_eq!(m.best_combo, 200);
}

/// In a marathon the combo of a map played through carries onto the wheel and into the next
/// map's corner; skipping a map breaks it. The wheel's buttons do what their keys do.
#[test]
fn a_marathon_shows_its_combo_on_the_wheel_and_in_the_next_map() {
    let (mut s, _songs, mut now) = marathon("journey-marathon-combo", harness::config());
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    let map = harness::game(&s).unwrap().map().clone();
    super::flows::play_out(
        &mut s,
        &mut now,
        &maps::presses(&map, maps::Player::Perfect),
    );
    assert_eq!(scene_name(&s), "Marathon");
    let m = s.journey.marathon.as_ref().unwrap();
    let carried = m.combo;
    assert!(carried > 0, "a clean map's combo carries on");
    assert_eq!(m.last.as_ref().unwrap().max_combo, carried);
    let c = frame(&mut s, now);
    assert!(shows(&c, &format!("MARATHON COMBO x{carried}")));
    for label in ["Start now", "Hold", "Another map", "End"] {
        assert!(shows(&c, label), "{label}");
    }

    // Start now: the next map doesn't wait out the wheel, and starts from the carried combo,
    // shown in its corner.
    s.click(hit::Hit::Key(KeyCode::Enter), None);
    let start = now;
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    assert!(now.duration_since(start) < crate::screens::marathon::MARATHON_GAP);
    assert_eq!(s.journey.marathon.as_ref().unwrap().combo_now, carried);
    // The game's clock started on the real clock, far behind the test's by now.
    now = Instant::now();
    let c = frame(&mut s, now);
    assert_eq!(scene_name(&s), "Play");
    assert!(shows(&c, &format!("x{carried}")));

    // Skipping it from the pause menu breaks the combo.
    press(&mut s, KeyCode::Escape, now);
    press(&mut s, KeyCode::ArrowDown, now);
    press(&mut s, KeyCode::Enter, now);
    assert_eq!(scene_name(&s), "Marathon");
    let m = s.journey.marathon.as_ref().unwrap();
    assert_eq!((m.combo, m.best_combo), (0, carried));

    // Hold keeps the wheel up past its time; its button then says Go on.
    s.click(hit::Hit::Key(KeyCode::Space), None);
    for _ in 0..12 {
        now += Duration::from_millis(250);
        frame(&mut s, now);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(scene_name(&s), "Marathon", "held");
    let c = frame(&mut s, now);
    assert!(shows(&c, "Go on"));
    s.click(hit::Hit::Key(KeyCode::Escape), None);
    assert!(s.journey.marathon.as_ref().unwrap().ended.is_some());
}

/// Plays the map up through (every note hit) until the marathon has counted it, the next map
/// loading meanwhile (given the moment its thread needs). `before_end`: done once, 10 s before
/// the last note.
fn play_marathon_map(s: &mut State, now: &mut Instant, mut before_end: impl FnMut(&mut State)) {
    let map = harness::game(s).unwrap().map().clone();
    let last = map.notes.last().map_or(0.0, |n| n.end_time.max(n.time));
    let presses = maps::presses(&map, maps::Player::Perfect);
    let keys = s.cfg.keys;
    let played = s.journey.marathon.as_ref().unwrap().played();
    let (mut next, mut waited, mut done) = (0, false, false);
    for _ in 0..200_000 {
        if s.journey.marathon.as_ref().unwrap().played() > played {
            return;
        }
        let horizon = *now + Duration::from_millis(16);
        while let Some(&(t, k)) = presses.get(next) {
            let at = instant_of(s, *now, t);
            if at > horizon {
                break;
            }
            press(s, keys[k as usize], at.max(*now));
            next += 1;
        }
        if !waited && s.journey.marathon.as_ref().unwrap().preload.is_some() {
            waited = true;
            std::thread::sleep(Duration::from_millis(300));
        }
        if !done && harness::game(s).is_some_and(|g| g.time() >= last - 10_000.0) {
            done = true;
            before_end(s);
        }
        *now = horizon;
        frame(s, *now);
    }
    panic!("the map never ended");
}

/// Seamless: the next map loads during the last one and starts on its last note, with no wheel
/// in between, past its intro.
#[test]
fn a_seamless_marathon_goes_from_the_last_note_straight_into_the_next_map() {
    let mut cfg = harness::config();
    cfg.journey_gap = crate::config::MarathonGap::Seamless;
    let (mut s, _songs, mut now) = marathon("journey-marathon-seamless", cfg);
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    let first = s.journey.current().unwrap().item.md5.clone();
    let upcoming = s.journey.queue[1].item.md5.clone();
    play_marathon_map(&mut s, &mut now, |s| {
        assert!(
            s.journey.marathon.as_ref().unwrap().preload.is_some(),
            "the next map loads before this one ends"
        );
    });
    assert_eq!(scene_name(&s), "Play", "no wheel");
    assert_eq!(s.journey.current().unwrap().item.md5, upcoming);
    assert_ne!(upcoming, first);
    let m = s.journey.marathon.as_ref().unwrap();
    assert_eq!((m.passed, m.combo > 0), (1, true));
    assert!(m.preload.is_none());
    assert!(!harness::game(&s).unwrap().can_skip(), "past its intro");
}

/// A map loaded ahead of time that is no longer up next (N said no to it) is dropped so the
/// replacement map preloads and starts instead — never the vetoed map.
#[test]
fn a_seamless_marathon_never_starts_a_map_no_longer_up_next() {
    let mut cfg = harness::config();
    cfg.journey_gap = crate::config::MarathonGap::Seamless;
    let (mut s, _songs, mut now) = marathon("journey-marathon-seamless-n", cfg);
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    let mut said_no = String::new();
    let mut instead = String::new();
    play_marathon_map(&mut s, &mut now, |s| {
        said_no = s.journey.queue[1].item.md5.clone();
        press(s, KeyCode::KeyN, Instant::now());
        assert!(
            s.journey.marathon.as_ref().unwrap().preload.is_none(),
            "vetoing the upcoming map clears the stale preload"
        );
        instead = s.journey.queue[1].item.md5.clone();
        assert_ne!(instead, said_no);
    });
    assert_eq!(s.journey.current().unwrap().item.md5, instead);
    assert_ne!(s.journey.current().unwrap().item.md5, said_no);
    if scene_name(&s) != "Play" {
        assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    }
    assert_eq!(s.journey.current().unwrap().item.md5, instead);
}

/// Hardcore marathon closes both bypasses: Pause menu hides Skip & Retry, Ctrl+R / Backquote do
/// nothing, and calling `marathon_skip_map` ends Hardcore instead of continuing. In a normal
/// marathon, retrying breaks the carried combo so misses cannot be undone.
#[test]
fn hardcore_blocks_skip_and_retry_and_normal_retry_breaks_marathon_combo() {
    // 1. Hardcore marathon: pause menu has only Continue, Settings, End the marathon.
    let mut hc_cfg = harness::config();
    hc_cfg.journey_hardcore = true;
    let (mut s, _songs, mut now) = marathon("journey-hc-no-bypass", hc_cfg);
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));

    // Ctrl+R and Backquote are blocked in Hardcore play.
    s.modifiers_ctrl = true;
    press(&mut s, KeyCode::KeyR, now);
    s.modifiers_ctrl = false;
    assert_eq!(scene_name(&s), "Play");
    press(&mut s, KeyCode::Backquote, now);
    assert_eq!(scene_name(&s), "Play");

    // Pause menu hides Skip this map and Retry; only Continue, Settings, End the marathon show.
    press(&mut s, KeyCode::Escape, now);
    now += Duration::from_millis(400);
    let c = frame(&mut s, now);
    assert!(!shows(&c, "Skip this map"), "Hardcore must hide Skip");
    assert!(!shows(&c, "Retry"), "Hardcore must hide Retry");
    assert!(shows(&c, "Continue"));
    assert!(shows(&c, "Settings"));
    assert!(shows(&c, "End the marathon"));

    // Safety belt: even if marathon_skip_map is invoked directly in Hardcore, it ends Hardcore.
    s.marathon_skip_map();
    assert_eq!(
        s.journey
            .marathon
            .as_ref()
            .and_then(|m| m.ended.map(|(w, _)| w)),
        Some(crate::journey::MarathonEnd::Hardcore)
    );

    // 2. Normal marathon: retrying breaks the carried marathon combo.
    let (mut s, _songs, mut now) = marathon("journey-retry-breaks-combo", harness::config());
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    let map = harness::game(&s).unwrap().map().clone();
    super::flows::play_out(
        &mut s,
        &mut now,
        &maps::presses(&map, maps::Player::Perfect),
    );
    let carried = s.journey.marathon.as_ref().unwrap().combo;
    assert!(carried > 0);
    s.click(hit::Hit::Key(KeyCode::Enter), None);
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    assert_eq!(s.journey.marathon.as_ref().unwrap().combo, carried);

    // Pause -> Retry breaks the carried combo instead of letting misses be save-scummed.
    press(&mut s, KeyCode::Escape, now);
    press(&mut s, KeyCode::ArrowDown, now);
    press(&mut s, KeyCode::ArrowDown, now);
    press(&mut s, KeyCode::Enter, now);
    assert_eq!(s.journey.marathon.as_ref().unwrap().combo, 0);
    assert_eq!(s.journey.marathon.as_ref().unwrap().combo_now, 0);
}

/// Marathon combo chip stays anchored at `x0` after a miss (never flickers away), and pressing N
/// blindly mid-map (when the up-next card is hidden) does not veto the next map.
#[test]
fn marathon_combo_chip_stays_visible_at_zero_and_blind_n_does_not_skip() {
    let (mut s, _songs, mut now) = marathon("journey-combo-stable", harness::config());
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    now = Instant::now();
    // At combo 0 on map 1, the chip is already steadily visible with x0.
    let c = frame(&mut s, now);
    assert!(shows(&c, "MARATHON"));
    assert!(shows(&c, "x0"));

    // Mid-map with up_next hidden (time < last - 12s): build a map with >20s span check via queue.
    let upcoming = s.journey.queue[1].item.md5.clone();
    if let Some(g) = harness::game(&s)
        && !crate::screens::journey::up_next_due(&g.map().notes, g.time())
    {
        press(&mut s, KeyCode::KeyN, now);
        assert_eq!(
            s.journey.queue[1].item.md5, upcoming,
            "blind N mid-map must not veto an unseen upcoming map"
        );
    }
}

/// Clicking the mouse "Next map" button on a failed Journey results screen records Failed (not
/// Passed) and enables Non-stop autostart just like pressing Enter.
#[test]
fn results_next_button_preserves_failed_outcome_and_nonstop_autostart() {
    let songs = Songs::new("journey-results-next-hit", &maps::all());
    let mut cfg = harness::config();
    cfg.journey_flow = JourneyFlow::Nonstop;
    let mut s = state_with(cfg, songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    s.open_journey();
    s.journey_go_offline();
    s.journey_pick_level(2);
    s.journey.in_play = true;

    let mut r = ui::Results::new(
        s.library[0].label(),
        game::Stats::default(),
        game::Windows::new(5.0),
    );
    r.failed = true;
    s.scene = crate::screens::results::ResultsScreen::scene(Box::new(r), 0, None);
    frame(&mut s, now);
    s.click(hit::Hit::JourneyNext, None);
    assert_eq!(
        s.journey.history.last().unwrap().outcome,
        PastOutcome::Failed
    );
    assert!(s.journey.autostart);
}

/// Marathon summary renders Play again and clicking it starts a fresh marathon immediately.
#[test]
fn marathon_summary_play_again_starts_a_fresh_run() {
    let (mut s, _songs, now) = marathon("journey-summary-again", harness::config());
    press(&mut s, KeyCode::Escape, now);
    let c = frame(&mut s, now);
    assert!(shows(&c, "MARATHON OVER"));
    assert!(shows(&c, "Play again"));
    assert!(shows(&c, "Back to Journey"));
    assert!(s.hits.iter().any(|h| h.hit == hit::Hit::MarathonRestart));

    s.click(hit::Hit::MarathonRestart, None);
    assert!(s.journey.marathon.as_ref().unwrap().ended.is_none());
    assert_eq!(scene_name(&s), "Marathon");
}

/// The time between maps is a setting: Short turns the wheel for under a second.
#[test]
fn a_short_marathon_wheel_is_brief() {
    let mut cfg = harness::config();
    cfg.journey_gap = crate::config::MarathonGap::Short;
    let (mut s, _songs, mut now) = marathon("journey-marathon-short", cfg);
    let start = now;
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    assert!(now.duration_since(start) < Duration::from_millis(1500));
}

/// Marathon's "Save maps" option controls whether cached maps played to completion are moved into
/// the player's library automatically (Off keeps them in cache with `S: save`; On moves them
/// before score submission / replay recording and marks the wheel card `Saved`; Pause -> Skip
/// never auto-saves).
#[test]
fn marathon_save_maps_option_controls_auto_saving_played_maps() {
    let pid = std::process::id();
    let make_cached = |tag: &str,
                       set_id: u32,
                       src_osu: &std::path::Path|
     -> (String, std::path::PathBuf, QueuedMap) {
        let name = format!("{set_id} MarathonSave-{pid}-{tag}");
        let folder = config::journey_cache_dir().join(&name);
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        let chart_path = folder.join("chart.osu");
        std::fs::copy(src_osu, &chart_path).unwrap();
        let entries = crate::journey::scan_single_folder(&folder);
        let entry = entries.into_iter().next().expect("scanned cached chart");
        let version = entry.meta.version.clone();
        let q = QueuedMap {
            item: item(&format!("Map {tag}"), &version, set_id),
            source: MapSource::Cached {
                folder: folder.clone(),
                entry: Box::new(entry),
            },
        };
        (name, folder, q)
    };

    // 1. Default (journey_marathon_save = false): played cached map stays in cache.
    let songs = Songs::new("journey-marathon-save-off", &maps::all());
    let mut cfg = harness::config();
    cfg.journey_flow = JourneyFlow::Marathon;
    cfg.journey_save_to = JourneySaveTo::Own;
    cfg.journey_marathon_save = false;
    let mut s = state_with(cfg, songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    let src_osu = s.library[0].path.clone();
    let (name_off, folder_off, q_off) = make_cached("off", 9101, &src_osu);
    let dest_off = config::own_songs_dir().join(&name_off);
    let _ = std::fs::remove_dir_all(&dest_off);
    s.open_journey();
    s.journey_go_offline();
    s.journey_pick_level(2);
    s.journey.queue.push_front(q_off);
    s.marathon_start();
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    let map = harness::game(&s).unwrap().map().clone();
    super::flows::play_out(
        &mut s,
        &mut now,
        &maps::presses(&map, maps::Player::Perfect),
    );
    assert_eq!(scene_name(&s), "Marathon");
    assert!(
        folder_off.is_dir(),
        "cached folder stays in cache when Save maps is Off"
    );
    assert!(
        !dest_off.exists(),
        "must not move to library when Save maps is Off"
    );
    let last = s.journey.marathon.as_ref().unwrap().last.as_ref().unwrap();
    assert!(last.cached.is_some());
    assert!(!last.saved);
    let c = frame(&mut s, now);
    assert!(shows(&c, "S: save"));
    let _ = std::fs::remove_dir_all(&folder_off);

    // 2. Enabled (journey_marathon_save = true): played cached map is auto-saved to library.
    let songs = Songs::new("journey-marathon-save-on", &maps::all());
    let mut cfg = harness::config();
    cfg.journey_flow = JourneyFlow::Marathon;
    cfg.journey_save_to = JourneySaveTo::Own;
    cfg.journey_marathon_save = true;
    let mut s = state_with(cfg, songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    let src_osu = s.library[0].path.clone();
    let (name_on, folder_on, q_on) = make_cached("on", 9102, &src_osu);
    let (name_skip, folder_skip, q_skip) = make_cached("skip", 9103, &src_osu);
    let dest_on = config::own_songs_dir().join(&name_on);
    let dest_skip = config::own_songs_dir().join(&name_skip);
    let _ = std::fs::remove_dir_all(&dest_on);
    let _ = std::fs::remove_dir_all(&dest_skip);
    s.open_journey();
    s.journey_go_offline();
    s.journey_pick_level(2);
    s.journey.queue.push_front(q_skip);
    s.journey.queue.push_front(q_on);
    s.marathon_start();
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    let map = harness::game(&s).unwrap().map().clone();
    super::flows::play_out(
        &mut s,
        &mut now,
        &maps::presses(&map, maps::Player::Perfect),
    );
    assert_eq!(scene_name(&s), "Marathon");
    assert!(
        dest_on.join("chart.osu").is_file(),
        "played map is auto-saved into the library folder when Save maps is On"
    );
    assert!(!folder_on.exists(), "cached folder moved out of cache");
    assert!(
        s.library
            .iter()
            .any(|e| e.path == dest_on.join("chart.osu")),
        "library entry path updated in place to the permanent destination"
    );
    let last = s.journey.marathon.as_ref().unwrap().last.as_ref().unwrap();
    assert!(last.cached.is_none());
    assert!(last.saved);
    let c = frame(&mut s, now);
    assert!(shows(&c, "Saved"));
    assert!(!shows(&c, "S: save"));

    // 3. Pause -> Skip while Save maps is On must NOT auto-save the skipped map.
    s.click(hit::Hit::Key(KeyCode::Enter), None);
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    press(&mut s, KeyCode::Escape, now);
    press(&mut s, KeyCode::ArrowDown, now);
    press(&mut s, KeyCode::Enter, now);
    assert_eq!(scene_name(&s), "Marathon");
    assert!(folder_skip.is_dir(), "skipped map stays in cache");
    assert!(!dest_skip.exists(), "skipped map must not be auto-saved");

    let _ = std::fs::remove_dir_all(&dest_on);
    let _ = std::fs::remove_dir_all(&folder_skip);
    let _ = std::fs::remove_dir_all(&dest_skip);
}

#[test]
fn a_journey_load_failure_clears_skip_intro_and_returns_to_journey() {
    let songs = Songs::new("journey-load-fail", &maps::all());
    let mut cfg = harness::config();
    cfg.journey_flow = JourneyFlow::Nonstop;
    cfg.journey_skip_intro = true;
    let mut s = state_with(cfg, songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);

    s.open_journey();
    s.journey_go_offline();
    s.journey_pick_level(2);
    s.journey.in_play = true;
    s.journey.skip_intro_next = true;
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(Err("corrupt audio".into())).unwrap();
    s.scene = screens::loading::Loading::scene(
        crate::Job::from_receiver(rx),
        "broken map".into(),
        0,
        None,
    );

    now += Duration::from_millis(16);
    frame(&mut s, now);

    assert_eq!(scene_name(&s), "Journey");
    assert!(!s.journey.in_play);
    assert!(
        !s.journey.skip_intro_next,
        "load failure must not leak skip_intro_next"
    );

    // Exiting Journey also clears skip_intro_next if it was set.
    s.journey.skip_intro_next = true;
    s.journey.stop();
    assert!(!s.journey.skip_intro_next);
}

#[test]
fn marathon_outage_continue_offline_keeps_server_probe_and_fills_queue_on_map_one() {
    let songs = Songs::new("marathon-outage-offline", &maps::all());
    let mut cfg = harness::config();
    cfg.online_token = "tok".into();
    cfg.journey_flow = JourneyFlow::Marathon;
    let mut s = state_with(cfg, songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);

    // Simulate an online Marathon whose first fetch failed past OUTAGE_GRACE (step == 0, Auto).
    s.journey.reset();
    s.journey.fetching = None;
    s.journey.outage_since = Some(now - crate::journey::OUTAGE_GRACE - Duration::from_secs(1));
    s.journey.error = Some("server unreachable".into());
    s.journey.retry_after = Some(now + Duration::from_secs(30));
    s.marathon_start();

    let _ = frame(&mut s, now);
    assert!(
        s.hits.iter().any(|h| h.hit == hit::Hit::JourneyOffline),
        "outage card offers Continue offline"
    );

    s.click(hit::Hit::JourneyOffline, None);
    assert_eq!(
        s.journey.offline,
        Some(crate::journey::Gate::ServerDown),
        "must record ServerDown so recovery probing stays enabled"
    );
    assert!(s.journey.next_probe.is_some());
    assert!(
        !s.journey.needs_level,
        "Marathon must not stall on needs_level when going offline on map 1"
    );
    assert!(
        !s.journey.queue.is_empty(),
        "local library maps should fill the Marathon queue immediately"
    );
}

#[test]
fn background_journey_save_updates_library_and_marathon_last_when_polled() {
    let songs = Songs::new("journey-bg-save", &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);

    let cache_dir = config::journey_cache_dir().join(format!(
        "9901 Bg Artist - Bg Title ({})",
        std::process::id()
    ));
    let dest_dir = std::env::temp_dir().join(format!("taiko-bg-save-dest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cache_dir);
    let _ = std::fs::remove_dir_all(&dest_dir);
    std::fs::create_dir_all(&cache_dir).unwrap();
    std::fs::create_dir_all(&dest_dir).unwrap();
    let src_osu = s.library[0].path.clone();
    std::fs::copy(&src_osu, cache_dir.join("chart.osu")).unwrap();
    std::fs::copy(&src_osu, dest_dir.join("chart.osu")).unwrap();
    let cached_entry = crate::journey::scan_single_folder(&cache_dir)
        .into_iter()
        .next()
        .unwrap();
    let dest_entries = crate::journey::scan_single_folder(&dest_dir);

    s.append_maps(vec![cached_entry]);
    let mut m = crate::journey::Marathon::new(now);
    m.record(crate::journey::MarathonPlay {
        item: item("Bg Title", "Oni", 9901),
        outcome: crate::journey::MarathonOutcome::Passed,
        accuracy: 1.0,
        max_combo: 10,
        misses: 0,
        pp: None,
        vote_on: None,
        vote: None,
        cached: Some((cache_dir.clone(), item("Bg Title", "Oni", 9901))),
        saved: false,
    });
    s.journey.marathon = Some(m);

    let (tx, rx) = std::sync::mpsc::channel();
    s.journey.saving.push(crate::journey::SaveJob {
        folder: cache_dir.clone(),
        toast_on_success: false,
        rx,
    });
    assert!(
        s.journey.protected_cache_folders().contains(&cache_dir),
        "in-flight save folder must be protected from cache pruning"
    );

    tx.send(Ok((dest_dir.clone(), dest_entries))).unwrap();
    s.journey_poll_saves();

    assert!(s.journey.saving.is_empty());
    assert!(
        s.library
            .iter()
            .any(|e| e.path == dest_dir.join("chart.osu"))
    );
    let last = s.journey.marathon.as_ref().unwrap().last.as_ref().unwrap();
    assert!(last.cached.is_none());
    assert!(last.saved);

    let _ = std::fs::remove_dir_all(&cache_dir);
    let _ = std::fs::remove_dir_all(&dest_dir);
}
