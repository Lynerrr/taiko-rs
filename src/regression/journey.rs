//! Regression tests for Journey v1: entry, cold-start level picker, queue advancement,
//! `from_journey` attempt tagging, and default save-folder config persistence.

use std::time::{Duration, Instant};

use super::harness::{self, *};
use super::maps;
use crate::config::{Config, JourneySaveTo};
use crate::journey::{MAX_PREFETCH_AHEAD, PastOutcome};
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
    assert_eq!(s.scene.screen().map(|sc| sc.name()), Some("Select"));
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
