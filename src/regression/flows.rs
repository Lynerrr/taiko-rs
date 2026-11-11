//! Whole journeys through the client, as a player makes them with the keyboard: song select,
//! loading, playing with the drum keys, pausing, retrying, the results, the settings and back.

use super::harness::{self, *};
use super::maps::{self, Player};
use super::snapshot::assert_snapshot;
use crate::*;
use std::fmt::Write as _;

const FRAME: Duration = Duration::from_millis(4);

/// A client whose library is `charts`, on song select with the first one selected.
fn client(tag: &str, charts: Vec<maps::Chart>) -> (State, Songs, Instant) {
    let songs = Songs::new(tag, &charts);
    let mut s = state_with(harness::config(), songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    assert_eq!(s.library.len(), charts.len(), "the library is taken in");
    assert_eq!(scene_name(&s), "Select");
    (s, songs, now)
}

/// From song select: Enter, then waits for the map to load.
fn start_selected(s: &mut State, now: &mut Instant) {
    press(s, KeyCode::Enter, *now);
    assert!(
        matches!(scene_name(s), "Loading" | "Play"),
        "Enter on song select starts the map, not {}",
        scene_name(s)
    );
    assert!(
        wait_for(s, now, |s| scene_name(s) == "Play"),
        "the map loads"
    );
}

/// Plays the rest of the map in `Play` with `presses` (map time, drum key 0-3) on the keyboard,
/// frame by frame, until the scene changes.
fn play_out(s: &mut State, now: &mut Instant, presses: &[(f64, u8)]) {
    let keys = s.cfg.keys;
    let mut next = 0;
    for _ in 0..200_000 {
        if harness::game(s).is_none() {
            return;
        }
        let horizon = *now + FRAME;
        // Presses due before the next frame, each at its own instant.
        while let Some(&(t, k)) = presses.get(next) {
            let at = instant_of(s, *now, t);
            if at > horizon {
                break;
            }
            press(s, keys[k as usize], at.max(*now));
            next += 1;
        }
        *now = horizon;
        frame(s, *now);
    }
    panic!("the play never ended");
}

fn results_stats(s: &State) -> game::Stats {
    match s
        .scene
        .screen_as::<crate::screens::results::ResultsScreen>()
    {
        Some(r) => r.results.stats.clone(),
        _ => panic!("not on the results screen but {}", scene_name(s)),
    }
}

fn line(st: &game::Stats) -> String {
    format!(
        "great {} ok {} miss {} max combo {} score {} rolls {} swells {} strong {}",
        st.great, st.ok, st.miss, st.max_combo, st.score, st.roll_ticks, st.swells, st.strong
    )
}

#[test]
fn a_map_is_played_with_the_keyboard_from_song_select_to_the_results_and_back() {
    let (mut s, _songs, mut now) = client("journey", vec![maps::rolls_and_swells()]);
    start_selected(&mut s, &mut now);
    let map = harness::game(&s).unwrap().map().clone();
    play_out(&mut s, &mut now, &maps::presses(&map, Player::Perfect));
    assert!(
        wait_for(&mut s, &mut now, |s| scene_name(s) == "Results"),
        "the results come after the map"
    );
    let st = results_stats(&s);
    assert_eq!((st.ok, st.miss), (0, 0), "{}", line(&st));
    assert_eq!(
        st.great as usize,
        map.notes
            .iter()
            .filter(|n| n.kind == beatmap::Kind::Don || n.kind == beatmap::Kind::Kat)
            .count()
    );
    assert_snapshot("flows-keyboard-play-perfect", &line(&st));
    // Back to song select.
    press(&mut s, KeyCode::Escape, now);
    run_until(&mut s, &mut now, FRAME, 50, |s| scene_name(s) == "Select");
    assert_eq!(scene_name(&s), "Select");
}

#[test]
fn keyboard_play_judges_like_the_replay_of_it() {
    // The same presses, live on the keyboard and played back as a replay: the same results.
    for (chart, player) in [
        (maps::basics(), Player::Jitter(50.0)),
        (maps::big_notes(), Player::WrongColour(3)),
        (maps::sv_and_kiai(), Player::Skips(4)),
    ] {
        let name = chart.name;
        let beatmap = chart.beatmap();
        let presses = maps::presses(&beatmap, player);
        let replayed = super::gameplay::play(&beatmap, player, &[]);
        let (mut s, _songs, mut now) = client(&format!("vs-replay-{name}"), vec![chart]);
        start_selected(&mut s, &mut now);
        play_out(&mut s, &mut now, &presses);
        assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Results"));
        let live = results_stats(&s);
        assert_eq!(
            line(&live),
            line(replayed.stats()),
            "{name}: live vs replay"
        );
    }
}

#[test]
fn pausing_resuming_retrying_and_quitting() {
    let (mut s, _songs, mut now) = client("pause", vec![maps::basics()]);
    let mut log = String::new();
    start_selected(&mut s, &mut now);
    run_until(&mut s, &mut now, FRAME, 300, |_| false);
    // Escape pauses: the menu shows and the clock stops.
    press(&mut s, KeyCode::Escape, now);
    let c = frame(&mut s, now);
    let paused = harness::game(&s).unwrap().paused();
    let _ = writeln!(log, "after Escape: {} paused {:?}", scene_name(&s), paused);
    assert!(paused.is_some(), "Escape pauses");
    let t = harness::game(&s).unwrap().time();
    run_until(&mut s, &mut now, FRAME, 200, |_| false);
    assert_eq!(
        harness::game(&s).unwrap().time(),
        t,
        "paused, the map time stands still"
    );
    let _ = writeln!(log, "pause menu says: {:?}", pause_menu(&c));
    // Escape again resumes (after a countdown).
    press(&mut s, KeyCode::Escape, now);
    run_until(&mut s, &mut now, FRAME, 2000, |s| {
        harness::game(s).is_some_and(|g| g.paused().is_none() && g.resuming().is_none())
    });
    let g = harness::game(&s).unwrap();
    let _ = writeln!(
        log,
        "after the second Escape: paused {:?} resuming {}",
        g.paused(),
        g.resuming().is_some()
    );
    run_until(&mut s, &mut now, FRAME, 100, |_| false);
    assert!(harness::game(&s).unwrap().time() > t, "playing again");
    // Retry from the pause menu: the map starts over.
    press(&mut s, KeyCode::Escape, now);
    press(&mut s, KeyCode::ArrowDown, now);
    press(&mut s, KeyCode::Enter, now);
    let _ = writeln!(log, "retry: {}", scene_name(&s));
    assert!(wait_for(&mut s, &mut now, |s| scene_name(s) == "Play"));
    let restarted = harness::game(&s).unwrap();
    assert_eq!(
        restarted.stats().great + restarted.stats().miss,
        0,
        "a fresh play"
    );
    // Quit to song select from the pause menu (its last item).
    press(&mut s, KeyCode::Escape, now);
    for _ in 1..game::PAUSE_ITEMS {
        press(&mut s, KeyCode::ArrowDown, now);
    }
    press(&mut s, KeyCode::Enter, now);
    run_until(&mut s, &mut now, FRAME, 50, |s| scene_name(s) == "Select");
    let _ = writeln!(log, "quit: {}", scene_name(&s));
    assert_eq!(scene_name(&s), "Select");
    assert_snapshot("flows-pause-retry-quit", &log);
}

/// The pause menu's items as drawn.
fn pause_menu(c: &crate::hit::Canvas) -> Vec<&'static str> {
    let mut v = vec![
        "Continue",
        "Retry",
        "Playfield size",
        "Settings",
        "Quit to song select",
    ];
    assert_eq!(v.len(), game::PAUSE_ITEMS);
    v.retain(|l| shows(c, l));
    v
}

#[test]
fn a_failed_play_ends_on_the_results_as_failed() {
    let mut cfg = harness::config();
    cfg.mods = Mods::parse("SD");
    let songs = Songs::new("failed", &[maps::basics()]);
    let mut s = state_with(cfg, songs.entries());
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    start_selected(&mut s, &mut now);
    let map = harness::game(&s).unwrap().map().clone();
    play_out(&mut s, &mut now, &maps::presses(&map, Player::Skips(10)));
    let failed_early = scene_name(&s);
    let _ = wait_for(&mut s, &mut now, |s| scene_name(s) != "Play");
    let mut log = format!("after the miss: {failed_early} -> {}\n", scene_name(&s));
    if let Some(r) = s
        .scene
        .screen_as::<crate::screens::results::ResultsScreen>()
    {
        let _ = writeln!(
            log,
            "failed {} {}",
            r.results.failed,
            line(&r.results.stats)
        );
    }
    assert_snapshot("flows-sudden-death-fail", &log);
}

#[test]
fn settings_open_change_and_close_with_the_keyboard() {
    let (mut s, _songs, mut now) = client("settings", vec![maps::basics()]);
    let mut log = String::new();
    s.open_settings();
    frame(&mut s, now);
    let _ = writeln!(
        log,
        "open: {} cursor {}",
        scene_name(&s),
        s.settings_ui.cursor
    );
    // Walking down the list moves through the sections.
    let mut sections = Vec::new();
    for _ in 0..settings::ITEMS.len() + 2 {
        let sec = s.settings_ui.section();
        if sections.last() != Some(&sec) {
            sections.push(sec);
        }
        press(&mut s, KeyCode::ArrowDown, now);
        now += Duration::from_millis(16);
        frame(&mut s, now);
    }
    let names: Vec<&str> = sections
        .iter()
        .map(|&i| settings::section_name(i))
        .collect();
    let _ = writeln!(log, "sections walked: {}", names.join(", "));
    assert_eq!(
        sections.len(),
        settings::SECTIONS.len(),
        "every section is reachable"
    );
    // Escape closes the settings and goes back where they were opened from.
    press(&mut s, KeyCode::Escape, now);
    frame(&mut s, now);
    let _ = writeln!(log, "after Escape: {}", scene_name(&s));
    assert_eq!(scene_name(&s), "Select");
    assert_snapshot("flows-settings-keyboard", &log);
}

#[test]
fn song_select_searches_sorts_and_moves_through_the_list() {
    let charts = maps::all();
    let n = charts.len();
    let (mut s, _songs, mut now) = client("select", charts);
    let mut log = String::new();
    let selected = |s: &State| {
        s.library[s.select.selected().unwrap_or(0)]
            .meta
            .version
            .clone()
    };
    let _ = writeln!(log, "first: {}", selected(&s));
    for _ in 0..n + 1 {
        press(&mut s, KeyCode::ArrowDown, now);
        now += Duration::from_millis(16);
        frame(&mut s, now);
        let _ = writeln!(log, "down: {}", selected(&s));
    }
    press(&mut s, KeyCode::ArrowUp, now);
    frame(&mut s, now);
    let _ = writeln!(log, "up: {}", selected(&s));
    // Typing searches.
    s.select.searching = true;
    type_text(&mut s, "stream", now);
    now += Duration::from_millis(16);
    frame(&mut s, now);
    let _ = writeln!(log, "search {:?}: {}", s.select.filter, selected(&s));
    assert!(selected(&s).contains("stream"));
    assert_snapshot("flows-song-select", &log);
}

#[test]
fn the_top_tabs_lead_to_every_part_of_the_game() {
    let (mut s, _songs, mut now) = client("tabs", vec![maps::basics()]);
    let mut log = String::new();
    for tab in 0..3 {
        s.click(crate::hit::Hit::Tab(tab), None);
        now += Duration::from_millis(16);
        frame(&mut s, now);
        let _ = writeln!(log, "tab {tab}: {}", scene_name(&s));
        press(&mut s, KeyCode::Escape, now);
        now += Duration::from_millis(16);
        frame(&mut s, now);
        let _ = writeln!(log, "  Escape: {}", scene_name(&s));
        s.scene = SongSelect::scene();
    }
    assert_snapshot("flows-top-tabs", &log);
}
