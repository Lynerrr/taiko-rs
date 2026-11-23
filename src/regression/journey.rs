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
fn pause_quit_to_song_select_clears_journey_state() {
    let songs = Songs::new("journey-pause-quit", &maps::all());
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);

    s.open_journey();
    s.journey_pick_level(2);
    assert!(s.journey.active);
    s.journey.in_play = true;

    // Leaving a play via Pause -> Quit calls show_select(); Journey state must not leak.
    s.show_select();
    assert!(!s.journey.active, "show_select must stop Journey");
    assert!(!s.journey.in_play, "show_select must clear in_play");
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
