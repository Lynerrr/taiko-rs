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
use crate::config::{Config, JourneySaveTo};
use crate::journey::{MAX_PREFETCH_AHEAD, MapSource, PastOutcome, QueuedMap};
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
    assert!(s.journey.needs_level);
    assert_eq!(s.scene.screen().map(|sc| sc.name()), Some("Journey"));

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

    cfg.journey_save_to = JourneySaveTo::Osu;
    cfg.journey_save_on_like = false;
    let text = cfg.to_text();
    let parsed = Config::parse_text(&text);
    assert_eq!(parsed.journey_save_to, JourneySaveTo::Osu);
    assert!(!parsed.journey_save_on_like);
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
