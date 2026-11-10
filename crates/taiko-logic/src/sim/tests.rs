//! The play's own tests: judging, scoring, HP, autoplay, Defense and the mods' rules, with
//! nothing drawn. The ones that draw the play stay in the client (`src/game/tests.rs`).

use super::testing::*;
use super::*;

#[test]
fn the_outro_can_be_skipped_only_once_every_note_is_judged() {
    let mut g = modded("");
    assert!(!g.can_skip_outro());
    assert!(!g.skip_outro(), "a note is still to come");
    g.run_out_of_notes();
    assert!(g.can_skip_outro());
    assert!(g.skip_outro());
}

#[test]
fn random_colours_each_play_its_own_way_and_replays_the_same() {
    let colours = |seed: u64| {
        let g = Game::with_seed(
            beatmap::calibration_map(500.0, 0, 64),
            Arc::new(Silence),
            None,
            Mods::parse("RD"),
            seed,
        );
        g.map
            .notes
            .iter()
            .map(|n| n.kind == Kind::Don)
            .collect::<Vec<_>>()
    };
    // Seed 0 is how every play of a map looked before seeds (replays from then).
    let before: Vec<bool> = modded("RD")
        .map
        .notes
        .iter()
        .map(|n| hash01(n.time + 0.5) < 0.5)
        .collect();
    assert_eq!(colours(0), before);
    assert_eq!(colours(12345), colours(12345));
    let plays: Vec<_> = [1u64, 2, 777, 1 << 40].into_iter().map(colours).collect();
    for (i, a) in plays.iter().enumerate() {
        assert!(
            plays[i + 1..].iter().all(|b| b != a),
            "seeds give different colourings"
        );
        let dons = a.iter().filter(|&&d| d).count();
        assert!((16..=48).contains(&dons), "about half dons: {dons} of 64");
    }
}

#[test]
fn a_press_stamped_before_the_frame_that_closes_its_window_still_hits() {
    let ms = |x: f64| std::time::Duration::from_secs_f64(x / 1000.0);
    // The press comes in first, or waits for the state lock while the frame after it is
    // built: either way it's judged at its own time.
    for press_first in [true, false] {
        let mut g = game();
        let t0 = Instant::now();
        g.update(t0, None, 0.0);
        let start = g.time;
        let at = |t: f64| t0 + ms(t - start);
        let n = g.map.notes[1].time;
        let ok = g.windows.ok;
        let mut t = start;
        while t < n + ok - 3.0 {
            g.update(at(t), None, 0.0);
            t += 4.0;
        }
        let (press, frame) = (at(n + ok - 2.0), at(n + ok + 2.0));
        if press_first {
            g.press(1, press, None, 0.0);
            g.update(frame, None, 0.0);
        } else {
            g.update(frame, None, 0.0);
            g.press(1, press, None, 0.0);
        }
        assert_eq!(
            g.state[1].judged,
            Some(Judge::Ok),
            "press first: {press_first}"
        );
        assert_eq!(
            g.state[2].judged, None,
            "the press went on to the next note"
        );
    }
}

#[test]
fn a_note_nobody_hits_is_a_miss_a_frame_after_its_window() {
    let ms = |x: f64| std::time::Duration::from_secs_f64(x / 1000.0);
    let mut g = game();
    let t0 = Instant::now();
    g.update(t0, None, 0.0);
    let start = g.time;
    let at = |t: f64| t0 + ms(t - start);
    let close = g.map.notes[1].time + g.windows.ok;
    g.update(at(close + 2.0), None, 0.0);
    assert_eq!(g.state[1].judged, None, "gone before the frame after");
    g.update(at(close + 6.0), None, 0.0);
    assert_eq!(g.state[1].judged, Some(Judge::Miss));
}

#[test]
fn plays_judge_with_osus_windows_and_old_replays_with_their_own() {
    let mut g = game();
    let od = g.mods.od(g.map.meta.od);
    let exact = Windows::with_rule(od, WindowRule::Exact);
    let whole = |w: f64| (w + 1e-9).floor() - 0.5;
    assert_eq!(
        (g.windows.great, g.windows.ok, g.windows.miss),
        (whole(exact.great), whole(exact.ok), whole(exact.miss))
    );
    g.set_window_rule(WindowRule::Exact);
    assert_eq!(g.windows.great, exact.great);
}

#[test]
fn play_time_leaves_out_pauses_and_the_resume_countdown() {
    let mut g = game();
    let t0 = Instant::now();
    let at = |ms: u64| t0 + std::time::Duration::from_millis(ms);
    g.update(at(0), None, 0.0);
    g.update(at(1000), None, 0.0);
    g.update(at(1500), None, 0.0);
    assert!((g.played_ms - 1500.0).abs() < 1.0, "{}", g.played_ms);
    g.paused = Some(0);
    g.update(at(9000), None, 0.0);
    // Counting down to resume doesn't count either.
    g.paused = None;
    g.resuming = Some((at(9000), at(12_000)));
    g.update(at(11_000), None, 0.0);
    g.resuming = None;
    g.update(at(12_000), None, 0.0);
    g.update(at(12_500), None, 0.0);
    assert!((g.played_ms - 2000.0).abs() < 1.0, "{}", g.played_ms);
    // A stall counts as a second at most.
    g.update(at(60_000), None, 0.0);
    assert!((g.played_ms - 3000.0).abs() < 1.0, "{}", g.played_ms);
}

#[test]
fn alternate_lets_either_hand_follow_finishers_drumrolls_and_swells() {
    // Dons 500 ms apart, with a finisher at 1000, a drumroll from 2500 to 3500 and a swell
    // from 5000 to 6000.
    let mut map = beatmap::calibration_map(500.0, 0, 15);
    let base = map.notes[0].clone();
    let at = |t: f64| beatmap::Note {
        time: t,
        end_time: t,
        ..base.clone()
    };
    map.notes = vec![
        at(0.0),
        at(500.0),
        beatmap::Note {
            big: true,
            ..at(1000.0)
        },
        at(1500.0),
        at(2000.0),
        beatmap::Note {
            kind: Kind::Roll,
            end_time: 3500.0,
            tick_spacing: 125.0,
            ..at(2500.0)
        },
        at(4000.0),
        at(4500.0),
        beatmap::Note {
            kind: Kind::Swell,
            end_time: 6000.0,
            required_hits: 6,
            ..at(5000.0)
        },
        at(6500.0),
    ];
    let mut g = Game::with_mods(map, Arc::new(Silence), None, Mods::parse("AL"));
    let (l, r) = (1, 2); // left and right centre
    g.press_time(l, 0.0, None);
    g.press_time(r, 500.0, None);
    // The finisher with the left hand only, then the left hand again.
    g.press_time(l, 1000.0, None);
    g.press_time(l, 1500.0, None);
    // Back to alternating: a second left in a row is ignored, the right then hits.
    g.press_time(l, 1990.0, None);
    g.press_time(r, 2000.0, None);
    // The drumroll mashed with the left hand, then the left hand again after it.
    for k in 0..=8 {
        g.press_time(l, 2500.0 + 125.0 * k as f64, None);
    }
    g.press_time(l, 4000.0, None);
    g.press_time(l, 4490.0, None);
    g.press_time(r, 4500.0, None);
    // The swell with the left hand alone (centre, rim, centre...), then the left again.
    for k in 0..6 {
        g.press_time(
            if k % 2 == 0 { l } else { 0 },
            5100.0 + 100.0 * k as f64,
            None,
        );
    }
    g.press_time(l, 6500.0, None);
    g.set_time(9000.0);
    let s = &g.stats;
    assert_eq!((s.great, s.ok, s.miss), (8, 0, 0), "every don hit");
    assert_eq!((s.roll_ticks, s.swells), (9, 1));
    // Without the finisher, drumroll and swell rule, the same hand twice is still out.
    let mut g = Game::with_mods(
        beatmap::calibration_map(500.0, 0, 4),
        Arc::new(Silence),
        None,
        Mods::parse("AL"),
    );
    g.press_time(l, 0.0, None);
    g.press_time(l, 500.0, None);
    g.set_time(9000.0);
    assert_eq!((g.stats.great, g.stats.miss), (1, 3));
}

#[test]
fn hp_passes_a_clean_play_and_fails_an_empty_one() {
    let mut g = modded("");
    play_all(&mut g, 0.0);
    assert!(g.hp >= 0.99 && !g.failed());
    // Nothing hit: HP never fills.
    let mut g = modded("");
    g.set_time(g.end_time + 2000.0);
    assert!(g.stats.miss == 64 && g.failed());
    // No Fail passes anyway.
    let mut g = modded("NF");
    g.set_time(g.end_time + 2000.0);
    assert!(!g.failed());
}

#[test]
fn sudden_death_and_perfect_fail_early() {
    let mut g = modded("SD");
    let first = g.map.notes[0].time;
    g.press_at(1, first + 80.0, None); // an OK: fine for SD
    assert!(!g.failed_early());
    g.set_time(g.map.notes[2].time + 500.0); // lets a note pass
    assert!(g.failed_early());
    // Nothing after the failing miss counts.
    assert_eq!(g.stats.miss, 1);
    let mut g = modded("PF");
    g.press_at(1, first + 80.0, None);
    assert!(g.failed_early());
}

#[test]
fn note_and_judging_mods() {
    // Great or Nothing: an OK is a miss.
    let mut g = modded("GN");
    g.press_at(1, g.map.notes[0].time + 80.0, None);
    assert_eq!((g.stats.ok, g.stats.miss), (0, 1));
    // Swap turns the dons into kats; Half leaves notes a beat apart alone.
    assert!(modded("SW").map.notes.iter().all(|n| n.kind == Kind::Kat));
    assert_eq!(modded("HA").map.notes.len(), 64);
    assert!(modded("AB").map.notes.iter().all(|n| n.big));
    // Random: a mix, the same every time.
    let kinds = |g: &Game| g.map.notes.iter().map(|n| n.kind).collect::<Vec<_>>();
    let r = kinds(&modded("RD"));
    assert!(r.contains(&Kind::Don) && r.contains(&Kind::Kat) && r == kinds(&modded("RD")));
    // Relax: rim keys on dons still hit.
    let mut g = modded("RX");
    let times: Vec<f64> = g.map.notes.iter().map(|n| n.time).collect();
    for t in times {
        g.press_at(0, t, None);
    }
    assert_eq!(g.stats.great, 64);
    // HR tightens the windows, EZ widens them, and both change the score multiplier.
    let great = |m: &str| Windows::new(Mods::parse(m).od(5.0)).great;
    assert!(great("HR") < great("") && great("EZ") > great("") && great("PR") < great(""));
    let (mut plain, mut hd) = (modded(""), modded("HD"));
    play_all(&mut plain, 0.0);
    play_all(&mut hd, 0.0);
    assert!(hd.stats.score > plain.stats.score);
}

#[test]
fn a_clean_play_is_a_million_and_mods_go_above_or_below_it() {
    let mut g = modded("");
    let mut last = 0;
    let times: Vec<(f64, bool)> = g
        .map
        .notes
        .iter()
        .filter(|n| matches!(n.kind, Kind::Don | Kind::Kat))
        .map(|n| (n.time, n.kind == Kind::Kat))
        .collect();
    for (t, kat) in times {
        g.press_at(if kat { 0 } else { 1 }, t, None);
        assert!(
            g.stats.score >= last,
            "the score only grows while nothing is missed"
        );
        last = g.stats.score;
    }
    assert_eq!(g.stats.score, 1_000_000);
    // Rolls and swells aren't part of it.
    g.set_time(g.end_time + 2000.0);
    assert_eq!(g.stats.score, 1_000_000);
    let mut hard = modded("HD,HR");
    play_all(&mut hard, 0.0);
    assert!(hard.stats.score > 1_000_000);
    let mut easy = modded("EZ");
    play_all(&mut easy, 0.0);
    assert!(easy.stats.score < 1_000_000);
    // An OK, and a miss, each cost something; a miss more.
    let mut ok = modded("");
    let first = ok.map.notes[0].time;
    ok.press_at(1, first + 80.0, None);
    let mut miss = modded("");
    miss.set_time(first + 500.0);
    assert!(ok.stats.ok == 1 && miss.stats.miss == 1);
    play_all(&mut ok, 0.0);
    play_all(&mut miss, 0.0);
    assert!(
        miss.stats.score < ok.stats.score && ok.stats.score < 1_000_000,
        "{} {}",
        miss.stats.score,
        ok.stats.score
    );
}

#[test]
fn wind_up_maps_song_time_to_faster_map_time() {
    let mut g = modded("WU");
    let last = g.end_time;
    // By the end the map runs 1.5x: a song second is 1.5 map seconds.
    let t = g.track_at(last);
    assert!(((g.speed.map(t + 1000.0) - last) - 1500.0).abs() < 1.0);
    play_all(&mut g, 0.0);
    assert_eq!(g.stats.great, 64);
}

/// Deterministic spread around `mean` with standard deviation close to `sd`.
fn spread(mean: f64, sd: f64, n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| mean + if i % 2 == 0 { sd } else { -sd })
        .collect()
}

#[test]
fn replay_reproduces_the_play() {
    // A sloppy play: early, late, wrong colour, doubled hits.
    let mut live = game();
    live.record = true;
    let notes: Vec<(f64, bool)> = live
        .map
        .notes
        .iter()
        .filter(|n| matches!(n.kind, Kind::Don | Kind::Kat))
        .map(|n| (n.time, n.kind == Kind::Kat))
        .collect();
    for (k, &(t, kat)) in notes.iter().enumerate() {
        let jitter = [-30.0, 12.0, 45.0, -70.0, 0.0, 100.0][k % 6];
        let key = if (k % 7 == 3) != kat { 0 } else { 1 };
        let at = t + jitter;
        live.inputs.push((at, key as u8));
        live.press_at(key, at, None);
        if k % 5 == 0 {
            live.inputs.push((at + 8.0, 2));
            live.press_at(2, at + 8.0, None);
        }
    }
    let end = notes.last().unwrap().0 + 1000.0;
    live.time = end;
    live.expire(end);

    let mut back = game();
    back.replay = Some((live.inputs.clone(), 0));
    back.time = end;
    back.feed_replay(None);
    back.expire(end);
    let (a, b) = (&live.stats, &back.stats);
    assert_eq!(
        (a.great, a.ok, a.miss, a.score, a.max_combo),
        (b.great, b.ok, b.miss, b.score, b.max_combo)
    );
    assert!(
        a.miss > 0 && a.great > 0,
        "the test play should mix judgements"
    );
}

#[test]
fn a_replay_knows_its_misses_ahead_and_stays_where_it_was() {
    let mut g = game();
    // Every fifth hit note left alone: those are the misses.
    let hits: Vec<&crate::beatmap::Note> = g
        .map
        .notes
        .iter()
        .filter(|n| matches!(n.kind, Kind::Don | Kind::Kat))
        .collect();
    let inputs: Vec<(f64, u8)> = (hits.iter().enumerate())
        .filter(|(k, _)| k % 5 != 0)
        .map(|(_, n)| (n.time, if n.kind == Kind::Kat { 0 } else { 1 }))
        .collect();
    let skipped: Vec<f64> = (hits.iter().enumerate())
        .filter(|(k, _)| k % 5 == 0)
        .map(|(_, n)| n.time)
        .collect();
    g.replay = Some((inputs, 0));
    let (start, end) = g.span();
    let mid = (start + end) / 2.0;
    g.seek(mid, None, 0.0);
    let before = (g.time, g.stats.great, g.stats.miss, g.stats.score);
    let misses = g.replay_misses();
    assert!(!skipped.is_empty());
    assert_eq!(misses, skipped);
    assert_eq!(
        (g.time, g.stats.great, g.stats.miss, g.stats.score),
        before,
        "still in the middle, judged as before"
    );
}

#[test]
fn seeking_a_replay_back_and_forth_lands_on_the_same_state() {
    let mut g = game();
    let inputs: Vec<(f64, u8)> = g
        .map
        .notes
        .iter()
        .filter(|n| matches!(n.kind, Kind::Don | Kind::Kat))
        .enumerate()
        .map(|(k, n)| {
            (
                n.time + [-20.0, 10.0, 60.0][k % 3],
                if n.kind == Kind::Kat { 0 } else { 1 },
            )
        })
        .collect();
    g.replay = Some((inputs, 0));
    let (start, end) = g.span();
    let mid = (start + end) / 2.0;
    g.seek(mid, None, 0.0);
    let at_mid = (
        g.stats.great,
        g.stats.ok,
        g.stats.miss,
        g.stats.score,
        g.stats.combo,
    );
    assert!(at_mid.0 > 0);
    g.seek(end, None, 0.0);
    assert!(g.stats.score > at_mid.3);
    // Back to the middle: rebuilt from the start, same as getting there directly.
    g.seek(mid, None, 0.0);
    assert_eq!(
        (
            g.stats.great,
            g.stats.ok,
            g.stats.miss,
            g.stats.score,
            g.stats.combo
        ),
        at_mid
    );
    assert_eq!(g.time, mid);
    g.seek(start - 10_000.0, None, 0.0);
    assert_eq!((g.stats.score, g.time), (0, start));
}

#[test]
fn a_long_combo_broken_plays_the_combo_break_once() {
    let mut g = game();
    g.sfx_log = Some(Vec::new());
    let notes: Vec<(f64, bool)> = g
        .map
        .notes
        .iter()
        .filter(|n| matches!(n.kind, Kind::Don | Kind::Kat))
        .map(|n| (n.time, n.kind == Kind::Kat))
        .collect();
    // 25 good hits, then let notes pass: one combo break, nothing for the misses after it.
    for &(t, kat) in &notes[..25] {
        g.press_at(if kat { 0 } else { 1 }, t, None);
    }
    g.set_time(notes[28].0 + 1000.0);
    let log = g.sfx_log.take().unwrap();
    let breaks: Vec<f64> = log
        .iter()
        .filter(|e| matches!(e.1, Sound::ComboBreak))
        .map(|e| e.0)
        .collect();
    assert_eq!(breaks.len(), 1, "{breaks:?}");
    assert!(
        log.iter()
            .filter(|e| !matches!(e.1, Sound::ComboBreak))
            .all(|e| e.2 == 1.0)
    );
}

#[test]
fn version_1_replays_keep_their_mash_setting() {
    let mut g = game();
    let first = g
        .map
        .notes
        .iter()
        .position(|n| matches!(n.kind, Kind::Don | Kind::Kat))
        .unwrap();
    let (t0, kat) = (
        g.map.notes[first].time,
        g.map.notes[first].kind == Kind::Kat,
    );
    let early = t0 - (g.windows.ok + g.windows.miss) / 2.0;
    // Off: too early or the wrong colour does nothing, the right hit still counts.
    g.mash = Mash::legacy(false);
    g.judge_press(1, kat, early);
    g.judge_press(1, !kat, t0);
    assert_eq!((g.stats.miss, g.state[first].judged), (0, None));
    g.judge_press(1, kat, t0);
    assert_eq!(g.state[first].judged, Some(Judge::Great));
    // On: the early hit is a miss, like osu!.
    let mut g = game();
    g.mash = Mash::legacy(true);
    g.judge_press(1, kat, early);
    assert_eq!(
        (g.stats.miss, g.state[first].judged),
        (1, Some(Judge::Miss))
    );
}

#[test]
fn one_slip_in_a_stream_costs_at_most_one_note() {
    for spacing in [60.0, 75.0, 90.0, 110.0] {
        let notes: Vec<(f64, bool)> = stream(spacing)
            .map
            .notes
            .iter()
            .map(|n| (n.time, n.kind == Kind::Kat))
            .collect();
        // An extra tap of the same colour right after the 5th note.
        let mut g = stream(spacing);
        let mut presses = notes.clone();
        presses.insert(5, (notes[4].0 + 12.0, notes[4].1));
        play(&mut g, &presses);
        assert_eq!(
            (g.stats.miss, g.stats.great),
            (0, 24),
            "extra tap @{spacing}"
        );
        // An extra tap of the other colour: still nothing, it's nearer the note just hit.
        let mut g = stream(spacing);
        let mut presses = notes.clone();
        presses.insert(5, (notes[4].0 + 12.0, !notes[4].1));
        play(&mut g, &presses);
        assert_eq!(g.stats.miss, 0, "extra wrong tap @{spacing}");
        // The 5th note skipped: that one misses, the rest are hit.
        let mut g = stream(spacing);
        let mut presses = notes.clone();
        presses.remove(4);
        play(&mut g, &presses);
        assert_eq!(
            (g.stats.miss, g.stats.great),
            (1, 23),
            "skipped note @{spacing}"
        );
        // The 5th note hit with the wrong colour, on time: that one misses.
        let mut g = stream(spacing);
        let mut presses = notes.clone();
        presses[4].1 = !presses[4].1;
        play(&mut g, &presses);
        assert_eq!(
            (g.stats.miss, g.stats.great),
            (1, 23),
            "wrong colour @{spacing}"
        );
    }
}

#[test]
fn mashing_is_still_punished() {
    // Both colours as fast as possible all through a stream.
    let mut g = stream(90.0);
    let end = g.map.notes.last().unwrap().time;
    let presses: Vec<(f64, bool)> = (0..)
        .map(|k| (1950.0 + k as f64 * 20.0, k % 2 == 1))
        .take_while(|p| p.0 < end + 50.0)
        .collect();
    play(&mut g, &presses);
    // No GREATs, some misses: worse than presses that would just be ignored (all OKs).
    let mut ignored = stream(90.0);
    ignored.mash = Mash::Ignore;
    play(&mut ignored, &presses);
    assert!(
        g.stats.great == 0 && g.stats.miss >= 4,
        "{:?}",
        (g.stats.great, g.stats.ok, g.stats.miss)
    );
    assert!(g.stats.accuracy() < ignored.stats.accuracy() - 0.05);
    // A right colour too early, with nothing else near, misses like osu!.
    let mut g = stream(200.0);
    let t0 = g.map.notes[0].time;
    g.judge_press(1, false, t0 - (g.windows.ok + g.windows.miss) / 2.0);
    assert_eq!(g.state[0].judged, Some(Judge::Miss));
}

#[test]
fn a_late_hitter_is_advised_a_lower_shown_offset() {
    let mut g = game();
    hits(&mut g, &spread(8.0, 2.0, 25), 0.0);
    g.shown_offset = 5.0;
    let h = g.offset_suggestion(0.0).unwrap();
    // Late by 8: the clock's offset +8 is the osu! one -8, whatever the map's own offset.
    assert_eq!(h.estimate.suggested_offset, 8.0);
    assert_eq!(g.suggested_shown_offset(h.estimate.suggested_offset), -3.0);
}

#[test]
fn needs_ten_hits() {
    let mut g = game();
    hits(&mut g, &[9.0; 9], 0.0);
    assert!(g.offset_suggestion(0.0).is_none());
}

#[test]
fn consistent_player_gets_full_correction_quickly() {
    let mut g = game();
    hits(&mut g, &spread(8.0, 2.0, 25), 0.0);
    let h = g.offset_suggestion(0.0).unwrap();
    assert!(h.significant);
    assert_eq!(h.needed, 20);
    assert_eq!(h.estimate.suggested_offset, 8.0);
}

#[test]
fn noisy_player_needs_more_hits_and_gets_no_false_alarm() {
    let mut g = game();
    // UR ~200 with a 4 ms bias: over 30 hits that is within the noise.
    hits(&mut g, &spread(4.0, 20.0, 30), 0.0);
    let h = g.offset_suggestion(0.0).unwrap();
    assert!(h.needed > 100, "needed {}", h.needed);
    assert!(!h.significant);
    // A real 12 ms bias at the same UR, once enough hits exist, is flagged but shrunk a little.
    let mut g = game();
    hits(&mut g, &spread(12.0, 20.0, 200), 0.0);
    let h = g.offset_suggestion(0.0).unwrap();
    assert!(h.significant);
    assert!(
        (11.0..=12.0).contains(&h.estimate.suggested_offset),
        "{}",
        h.estimate.suggested_offset
    );
}

#[test]
fn simple_mode_uses_plain_last_40_average() {
    let mut g = game();
    g.ur_scaled_hint = false;
    hits(&mut g, &spread(4.0, 20.0, 30), 0.0);
    let h = g.offset_suggestion(0.0).unwrap();
    // Same noisy hits the UR-scaled mode calls "looks good": simple mode flags and applies all of it.
    assert!(h.significant);
    assert_eq!(h.estimate.suggested_offset, 4.0);
    assert_eq!(h.estimate.hits, 30);
}

#[test]
fn applying_mid_song_does_not_double_count() {
    let mut g = game();
    hits(&mut g, &spread(10.0, 1.0, 30), 20.0);
    assert_eq!(
        g.offset_suggestion(20.0).unwrap().estimate.suggested_offset,
        30.0
    );
    // After applying, the same hits re-expressed under +30 suggest staying at +30.
    let h = g.offset_suggestion(30.0).unwrap();
    assert_eq!(h.estimate.suggested_offset, 30.0);
    assert!(!h.significant);
}

#[test]
fn a_note_before_a_nan_slider_cannot_be_hit() {
    let mut map = beatmap::calibration_map(500.0, 0, 64);
    map.notes[4].unhittable = true;
    let t = map.notes[4].time;
    let mut g = Game::new(map, Arc::new(Silence), None);
    g.press_at(1, t, None);
    g.press_at(1, t + 500.0, None);
    // The four before it went unpressed; it isn't a fifth miss.
    assert_eq!(
        (g.state[4].judged, g.stats.miss),
        (None, 4),
        "not even a miss"
    );
    assert_eq!(
        g.state[5].judged,
        Some(Judge::Great),
        "the press went on to the next note"
    );
    // Nor under autoplay.
    let mut map = beatmap::calibration_map(500.0, 0, 64);
    map.notes[4].unhittable = true;
    let mut g = Game::new(map, Arc::new(Silence), None);
    for k in 0..800 {
        g.time = k as f64 * 10.0;
        g.autoplay(None);
        g.expire(g.time);
    }
    assert_eq!((g.state[4].judged, g.stats.miss), (None, 0));
    assert_eq!(g.state[5].judged, Some(Judge::Great));
}

#[test]
fn autoplay_leaves_a_sub_ms_fake_slider_alone() {
    // Notes 4 and 5 become tiny rolls (written 0.1 to -0.1 long), one with a tick to play.
    let mut map = beatmap::calibration_map(500.0, 0, 64);
    for (i, len) in [(4, 0.001), (5, 0.0)] {
        let n = &mut map.notes[i];
        (n.kind, n.end_time, n.tick_spacing, n.tiny) = (Kind::Roll, n.time + len, 125.0, true);
    }
    let mut g = Game::new(map, Arc::new(Silence), None);
    for k in 0..800 {
        g.time = k as f64 * 10.0;
        g.autoplay(None);
        g.expire(g.time);
    }
    for i in [4, 5] {
        assert!(
            g.state[i].ticks.iter().all(|hit| !hit),
            "roll {i} was played"
        );
    }
    assert_eq!((g.state[6].judged, g.stats.miss), (Some(Judge::Great), 0));
}

#[test]
fn autoplay_hands_each_sound_over_once_at_its_notes_place_in_the_song() {
    // A don, a kat, a big don and a roll.
    let osu = "osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nSliderMultiplier:1.4\nOverallDifficulty:5\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,1500,1,2,0:0:0:0:\n256,192,2000,1,4,0:0:0:0:\n256,192,3000,2,0,L|356:192,1,70\n";
    let mut g = quiet_game(osu);
    g.offset = 30.0;
    g.time = 700.0;
    // Nothing within the lead yet.
    assert!(g.plan_auto_sounds(200.0).is_empty());
    g.time = 850.0;
    let got = g.plan_auto_sounds(200.0);
    assert_eq!(got.len(), 1);
    assert_eq!((got[0].0, got[0].1), (1030.0, Sound::Don));
    // Not twice, however often it is asked.
    assert!(g.plan_auto_sounds(200.0).is_empty());
    g.time = 1400.0;
    let got = g.plan_auto_sounds(200.0);
    assert_eq!(
        got.iter().map(|g| (g.0, g.1)).collect::<Vec<_>>(),
        [(1530.0, Sound::Kat)]
    );
    // The hit finds its sound already sent and leaves it be.
    g.time = 1000.0;
    g.autoplay(None);
    assert!(!g.auto_sched.contains(&(0, 0)));
    assert!(g.auto_sched.contains(&(1, 0)));
    // A big note: the second hand's press is the finisher.
    g.time = 1900.0;
    let got = g.plan_auto_sounds(200.0);
    assert!(!got.is_empty() && got.iter().all(|s| s.0 == 2030.0));
    assert_eq!(got[0].1, Sound::Don);
    assert!(got[1..].iter().all(|s| s.1 == Sound::BigDon));
    // Roll ticks, each at its own place.
    g.time = 2900.0;
    let ticks = g.plan_auto_sounds(600.0);
    assert!(ticks.len() >= 2);
    assert_eq!(ticks[0].0, 3030.0);
    assert_eq!(ticks[1].0, 3030.0 + g.map.notes[3].tick_spacing);
    assert!(g.plan_auto_sounds(600.0).is_empty());
}

#[test]
fn autoplay_sounds_follow_the_speed_and_start_again_after_a_seek() {
    let osu = "osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nSliderMultiplier:1.4\nOverallDifficulty:5\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,3000,1,0,0:0:0:0:\n";
    let mut g = quiet_game(osu);
    g.speed = Speed::constant(1.5);
    g.offset = -20.0;
    // Map time 2700 is song time 1800; the lead is song time.
    g.time = 2700.0;
    assert!(g.plan_auto_sounds(100.0).is_empty());
    g.time = 2900.0;
    let got = g.plan_auto_sounds(100.0);
    assert_eq!(got.len(), 1);
    assert!((got[0].0 - (3000.0 / 1.5 - 20.0)).abs() < 1e-9);
    assert!(g.plan_auto_sounds(100.0).is_empty());
    // The engine drops what waits when the song moves; the game plans it again.
    g.drop_scheduled();
    assert_eq!(g.plan_auto_sounds(100.0).len(), 1);
}

#[test]
fn autoplay_never_misses_over_a_negative_fake_slider() {
    // A -50 fake slider (a zero-length roll, as parsed) 50 ms before a kat.
    let osu = "osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nSliderMultiplier:1.4\nOverallDifficulty:5\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,1450,2,0,L|356:192,1,-50\n256,192,1500,1,2,0:0:0:0:\n256,192,2000,1,0,0:0:0:0:\n";
    let map = beatmap::parse_raw(osu);
    let mut g = Game::new(map, Arc::new(Silence), None);
    for k in 0..300 {
        g.time = k as f64 * 10.0;
        g.autoplay(None);
        g.expire(g.time);
    }
    assert_eq!(g.stats.miss, 0);
}

#[test]
fn notes_under_a_swell_still_take_their_hits() {
    let mut map = beatmap::calibration_map(500.0, 0, 64);
    let swell = beatmap::Note {
        time: 1900.0,
        end_time: 4100.0,
        kind: Kind::Swell,
        required_hits: 50,
        ..map.notes[0].clone()
    };
    map.notes.insert(4, swell);
    let mut g = Game::new(map, Arc::new(Silence), None);
    for t in [2000.0, 2500.0, 3000.0, 3500.0, 4000.0] {
        g.press_at(1, t, None);
    }
    let under: Vec<_> = (0..g.map.notes.len())
        .filter(|&i| {
            g.map.notes[i].kind == Kind::Don && (2000.0..=4000.0).contains(&g.map.notes[i].time)
        })
        .collect();
    assert_eq!(under.len(), 5);
    assert!(
        under
            .iter()
            .all(|&i| g.state[i].judged == Some(Judge::Great)),
        "notes under the swell were swallowed"
    );
    // Each counts for the swell too when it swaps colour: five dons are one swap.
    assert_eq!(g.state[4].hits, 1);
}

#[test]
fn autoplay_hits_the_notes_under_a_swell_and_spins_it_between() {
    // A swell from 1900 to 4100 over the calibration dons, one of them a kat.
    let mut map = beatmap::calibration_map(500.0, 0, 64);
    let kat = map.notes.iter().position(|n| n.time == 3000.0).unwrap();
    map.notes[kat].kind = Kind::Kat;
    let swell = beatmap::Note {
        time: 1900.0,
        end_time: 4100.0,
        kind: Kind::Swell,
        required_hits: 1000,
        ..map.notes[0].clone()
    };
    map.notes.insert(4, swell);
    let mut g = Game::new(map, Arc::new(Silence), None);
    let mut t = 1800.0;
    while t < 4300.0 {
        g.time = t;
        g.autoplay(None);
        t += 4.0;
    }
    let under: Vec<_> = (0..g.map.notes.len())
        .filter(|&i| {
            matches!(g.map.notes[i].kind, Kind::Don | Kind::Kat)
                && (2000.0..=4000.0).contains(&g.map.notes[i].time)
        })
        .map(|i| g.state[i].judged)
        .collect();
    assert_eq!(under, vec![Some(Judge::Great); 5]);
    // 2.2 s at one press per 40+ ms: dozens of swaps, but nowhere near 1000.
    let hits = g.state[4].hits;
    assert!(
        (30..60).contains(&hits) && g.state[4].judged.is_none(),
        "{hits}"
    );
}

#[test]
fn a_roll_over_notes_is_played_as_the_notes() {
    // A roll from 1900 to 4100 over the calibration dons, one of them a kat.
    let mut map = beatmap::calibration_map(500.0, 0, 64);
    let kat = map.notes.iter().position(|n| n.time == 3000.0).unwrap();
    map.notes[kat].kind = Kind::Kat;
    let roll = beatmap::Note {
        time: 1900.0,
        end_time: 4100.0,
        kind: Kind::Roll,
        tick_spacing: 125.0,
        ..map.notes[0].clone()
    };
    map.notes.insert(4, roll);
    let under = |g: &Game| -> Vec<Option<Judge>> {
        (0..g.map.notes.len())
            .filter(|&i| {
                matches!(g.map.notes[i].kind, Kind::Don | Kind::Kat)
                    && (2000.0..=4000.0).contains(&g.map.notes[i].time)
            })
            .map(|i| g.state[i].judged)
            .collect()
    };
    // Autoplay: every note Great, no tick taken.
    let mut g = Game::new(map.clone(), Arc::new(Silence), None);
    let mut t = 1800.0;
    while t < 4300.0 {
        g.time = t;
        g.autoplay(None);
        t += 4.0;
    }
    assert_eq!(under(&g), vec![Some(Judge::Great); 5]);
    assert_eq!(g.stats.roll_ticks, 0);
    // A player: a press on the kat inside the roll is the kat's, not a tick.
    let mut g = Game::new(map, Arc::new(Silence), None);
    g.press_at(0, 3000.0, None);
    assert_eq!(g.state[kat + 1].judged, Some(Judge::Great));
}

#[test]
fn a_lobby_hosts_mod_settings_are_what_the_match_plays_with() {
    let mut g = kiai_game("MV");
    g.use_mod_settings(Some(multiverse::classic()), None);
    assert_eq!(g.universes, Some(multiverse::classic()));
    let mut g = kiai_game("SS");
    let host = shapeshift::Config::parse(
        "every=4 order=fixed shapes=spiral,loop,-zigzag,-uturn,-elbow,-straight",
    );
    g.use_mod_settings(None, Some(host));
    assert_eq!(g.shapeshift.bars(), 4);
    assert_eq!(&g.shape_order[..4], &[2, 3, 2, 3]);
    // Settings the host didn't send stay the player's own.
    let mut g = kiai_game("SS");
    let own = g.shape_order.clone();
    g.use_mod_settings(None, None);
    assert_eq!((g.universes, g.shape_order.clone()), (None, own));
}

#[test]
fn rails_leave_the_hit_target_heading_right_and_never_jump() {
    for shape in [
        RailShape::Zigzag,
        RailShape::UTurn,
        RailShape::Spiral,
        RailShape::Loop,
        RailShape::Elbow,
        RailShape::Maze,
        RailShape::Meadow,
    ] {
        let len = shape.length(853.3);
        let rail = Rail::build(|s| shape.bend(s), len);
        assert_eq!(rail.at(0.0), (HIT_X, LANE_Y, 0.0));
        assert_eq!(rail.at(-50.0), (HIT_X - 50.0, LANE_Y, 0.0));
        // One unit along the lane is one unit on screen (give or take the corners).
        let mut prev = rail.at(0.0);
        for k in 1..(len as usize + 100) {
            let p = rail.at(k as f32);
            let step = (p.0 - prev.0).hypot(p.1 - prev.1);
            assert!((0.9..=1.01).contains(&step), "{shape:?} at {k}: {step}");
            prev = p;
        }
    }
}

#[test]
fn defense_takes_turns_by_bars_and_fails_the_team_when_the_lives_run_out() {
    // A don every 500 ms, a bar every 2 s: with 4-bar turns, notes 0-15 are the first
    // player's and 16-31 ours.
    let map = beatmap::calibration_map(500.0, 0, 64);
    let mut g = Game::new(map, Arc::new(Silence), None);
    g.set_defense(
        vec![(3, "a".into()), (7, "b".into())],
        1,
        Defense {
            lives: 5,
            bars: 4,
            ..Defense::DEFAULT
        },
    );
    assert_eq!(g.defense_turn_at(0.0), Some((0, 4)));
    assert_eq!(g.defense_turn_at(2500.0), Some((0, 3)));
    assert_eq!(g.defense_turn_at(8000.0), Some((1, 0)));
    assert_eq!(g.defense_turn_at(16000.0), Some((0, 4)));
    // Their notes take none of our presses and aren't our misses.
    g.press_at(1, 0.0, None);
    g.expire(7900.0);
    assert_eq!((g.state[0].judged, g.stats.miss), (None, 0));
    g.press_at(1, 8000.0, None);
    assert_eq!(g.state[16].judged, Some(Judge::Great));
    // Two of ours get through: misses, which the room turns into lives.
    g.expire(9000.0 + g.windows.ok + 1.0);
    assert_eq!(g.stats.miss, 2);
    // HP isn't what fails it: the team's lives are, as the room counts them.
    assert!(!g.failed());
    g.set_team(TeamState {
        lives: 1,
        max: 5,
        ..TeamState::default()
    });
    assert_eq!(g.lives_left(), Some(1));
    assert!(!g.failed_early());
    g.set_team(TeamState {
        lives: 0,
        max: 5,
        ..TeamState::default()
    });
    assert!(g.failed_early());
}

#[test]
fn a_don_tower_shoots_every_don_for_four_bars_and_ours_count_as_greats() {
    let mut g = defense_game(2);
    // Bought at 6.6 s: from the first barline 1.5 s on (10 s) for 4 bars, every don: notes
    // 20-35, ours up to bar 8 (16 s), p0's after.
    g.defense_act(
        1,
        DefenseAct::Buy {
            item: DefenseItem::DonTower,
            at: 6600.0,
            until: 0.0,
        },
    );
    // One tower at a time: it stands from the purchase until its fourth bar is over.
    let until = g.tower_until(6600.0);
    assert_eq!(until, g.defense.as_ref().unwrap().towers[0].to);
    assert!(g.tower_active(6600.0) && g.tower_active(until - 1.0));
    assert!(!g.tower_active(until));
    let d = g.defense.as_ref().unwrap();
    let taken: Vec<usize> = (0..64).filter(|&i| d.shot[i].is_some()).collect();
    assert_eq!(taken, (20..36).collect::<Vec<_>>());
    // They don't take our presses; the tower hits ours on time.
    g.press_at(1, 10_000.0, None);
    g.expire(18_000.0);
    assert!((20..32).all(|i| g.state[i].judged == Some(Judge::Great)));
    assert_eq!(
        (g.stats.great, g.stats.miss),
        (12, 4),
        "16-19 went by unpressed"
    );
    // The map has no kats: a kat tower shoots nothing (but stands).
    g.defense_act(
        2,
        DefenseAct::Buy {
            item: DefenseItem::KatTower,
            at: 20_000.0,
            until: 0.0,
        },
    );
    assert!(g.defense.as_ref().unwrap().towers[1].notes.is_empty());
    // From someone not in the team: nothing.
    g.defense_act(
        99,
        DefenseAct::Buy {
            item: DefenseItem::DonTower,
            at: 0.0,
            until: 0.0,
        },
    );
    assert_eq!(g.defense.as_ref().unwrap().towers.len(), 2);
}

#[test]
fn tag_ins_run_out_for_the_map() {
    // Three players, 4-bar turns, one tag-in each: p2 takes over p0's first turn, but not
    // its next one (bars 12-15).
    let map = beatmap::calibration_map(500.0, 0, 64);
    let mut g = Game::new(map, Arc::new(Silence), None);
    let team = (0..3u32).map(|k| (k + 1, format!("p{k}"))).collect();
    g.set_defense(
        team,
        1,
        Defense {
            bars: 4,
            tag_ins: 1,
            ..Defense::DEFAULT
        },
    );
    g.defense_act(3, DefenseAct::TagIn { at: 500.0 });
    g.defense_act(3, DefenseAct::TagIn { at: 24_500.0 });
    assert_eq!(
        g.defense.as_ref().unwrap().tags.len(),
        1,
        "p2 has none left"
    );
    // Ours: one, then none.
    assert_eq!(g.tag_ins_left(), 1);
    assert!(g.can_tag_in(24_500.0));
    g.defense_act(2, DefenseAct::TagIn { at: 24_500.0 });
    assert_eq!(g.tag_ins_left(), 0);
    assert!(!g.can_tag_in(48_500.0));
    // With none allowed, nobody tags in at all.
    let map = beatmap::calibration_map(500.0, 0, 64);
    let mut g = Game::new(map, Arc::new(Silence), None);
    let team = (0..2u32).map(|k| (k + 1, format!("p{k}"))).collect();
    g.set_defense(
        team,
        1,
        Defense {
            bars: 4,
            tag_ins: 0,
            ..Defense::DEFAULT
        },
    );
    assert!(!g.can_tag_in(500.0));
    g.defense_act(1, DefenseAct::TagIn { at: 8500.0 });
    assert!(g.defense.as_ref().unwrap().tags.is_empty());
}

#[test]
fn a_tag_in_takes_the_rest_of_the_turn_from_the_next_barline_once() {
    // Three players: the turns go p0 (bars 0-3), us (4-7), p2 (8-11).
    let mut g = defense_game(3);
    assert_eq!(g.defense_turn_at(500.0), Some((0, 4)));
    assert!(g.can_tag_in(500.0));
    assert!(!g.can_tag_in(8500.0), "our own turn");
    assert!(!g.can_tag_in(7500.0), "no barline left in the turn");
    // At 0.5 s: from bar 1 (2 s, the first barline over a second on) to the end of the turn.
    g.defense_act(2, DefenseAct::TagIn { at: 500.0 });
    // Ours now: the rest of p0's turn and our own after it.
    assert!(
        g.map
            .notes
            .iter()
            .all(|n| n.unhittable == !(n.time >= 2000.0 && n.time < 16_000.0))
    );
    assert_eq!(
        g.defense_turn_at(500.0),
        Some((0, 1)),
        "ours from the next bar"
    );
    assert_eq!(g.defense_turn_at(2500.0), Some((1, 0)));
    // p2 can't take it from us now: one tag-in a turn.
    g.defense_act(3, DefenseAct::TagIn { at: 600.0 });
    assert_eq!(g.defense.as_ref().unwrap().tags.len(), 1);
    assert!(!g.can_tag_in(700.0));
}

#[test]
fn the_meadow_turns_square_corners_and_fits_on_screen() {
    use std::f32::consts::FRAC_PI_2;
    let shape = RailShape::Meadow;
    let (len, zoom, drop) = (shape.length(853.3), shape.zoom(853.3), shape.drop());
    let rail = Rail::build(|s| shape.bend(s), len);
    // Every run is square to the screen: the heading a whole number of quarter turns.
    let mut at = 0.0;
    for (run, _) in MEADOW {
        let a = rail.at(at + run / 2.0).2 / FRAC_PI_2;
        assert!(
            (a - a.round()).abs() < 1e-3,
            "run at {at}: heading {a} quarters"
        );
        at += run + MEADOW_TURN;
    }
    // Out of the bottom of the screen.
    assert!((rail.at(len).2 + FRAC_PI_2).abs() < 1e-3);
    // Both loops' holes are alike, and so are the gaps between the three stacked lanes.
    let run = |i: usize| MEADOW[i].0;
    assert_eq!((run(2), run(1)), (run(4), run(5)));
    let corner = 2.0 * MEADOW_TURN / FRAC_PI_2;
    let (middle, bottom) = (run(3) - run(1) - run(5) - corner, run(9) - run(7));
    assert!(
        (middle - bottom).abs() < 2.0,
        "gaps {middle} and {bottom} apart"
    );
    // All but the lane out is on screen and clear of the hit error meter (ticks up to 419).
    let out = len - MEADOW[MEADOW.len() - 1].0;
    let mut s = 0.0;
    while s < out {
        let (x, y, a) = rail.at(s);
        for side in [-62.0, 62.0] {
            let (x, y) = (x - a.sin() * side, y - a.cos() * side);
            let (x, y) = (
                HIT_X + zoom * (x - HIT_X),
                LANE_Y + drop + zoom * (y - LANE_Y),
            );
            assert!(
                x > 0.0 && x < 853.3 && y > 4.0 && y < 419.0,
                "at {s}: ({x}, {y})"
            );
        }
        s += 2.0;
    }
}

#[test]
fn a_miss_cracks_the_lane_and_50_hits_mend_it() {
    let mut g = kiai_game("CK");
    let times: Vec<f64> = g.map.notes.iter().map(|n| n.time).collect();
    g.set_time(times[0] + 200.0);
    assert_eq!(g.stats.miss, 1);
    assert_eq!(g.cracks_at(times[0] + 400.0, 700.0).len(), 1);
    for &t in &times[1..52] {
        g.press_at(1, t, None);
    }
    assert!(g.cracks_at(times[51], 700.0).is_empty());
}

#[test]
fn every_hit_crumbles_the_lane_by_its_rhythm_and_it_settles_back() {
    let mut g = kiai_game("CR");
    let times: Vec<f64> = g.map.notes.iter().map(|n| n.time).collect();
    // A 1/1 note knocks out a bigger piece, further, for longer, than a 1/4 one.
    g.flows[0] = 1;
    g.flows[1] = 4;
    g.press_at(1, times[0], None);
    g.press_at(1, times[1], None);
    assert_eq!(g.crumbs.len(), 2);
    let (slow, fast) = (g.crumbs[0], g.crumbs[1]);
    assert!(slow.len > fast.len && slow.shift.abs() > fast.shift.abs() && slow.life > fast.life);
    // Never near the drum, and never further out than the cap however many pile up.
    for &t in &times[2..40] {
        g.press_at(1, t, None);
        let geo = LaneGeo {
            rail: None,
            len: 700.0,
            shatter: [0.0; 4],
            cracks: g.cracks_at(t + 80.0, 700.0),
        };
        for s in (0..700).step_by(5) {
            assert!(geo.shift(s as f32).abs() <= CRUMBLE_MAX_SHIFT);
        }
        assert!(
            geo.cracks
                .iter()
                .all(|&(a, ..)| a >= 700.0 * CRUMBLE_SPAN.0 - 0.01)
        );
    }
    assert!(g.cracks_at(times[39] + 3000.0, 700.0).is_empty());
}

#[test]
fn crumble_grows_with_the_combo_and_a_miss_puts_it_back() {
    assert!(crumble_power(0) < 0.3 && crumble_power(200) == 1.0 && crumble_power(1000) > 1.3);
    let mut g = kiai_game("CR");
    let times: Vec<f64> = g.map.notes.iter().map(|n| n.time).collect();
    g.flows.iter_mut().for_each(|f| *f = 1);
    // A miss knocks nothing out.
    g.set_time(times[0] + 200.0);
    assert!(g.stats.miss == 1 && g.crumbs.is_empty());
    // The same 1/1 note knocks out more at a higher combo.
    g.press_at(1, times[1], None);
    let low = g.crumbs[0];
    g.stats.combo = 300;
    g.press_at(1, times[2], None);
    let high = *g.crumbs.last().unwrap();
    assert!(high.len > low.len && high.shift.abs() > low.shift.abs() && high.life > low.life);
    // A miss snaps the pieces still out back into line.
    let out = |g: &Game, t: f64| g.crumbs_at(t, 700.0).map(|c| c.2.abs()).fold(0.0, f32::max);
    g.set_time(times[3] + 200.0);
    assert_eq!(g.stats.miss, 2);
    let miss = *g.misses.back().unwrap();
    assert!(out(&g, miss - 1.0) > 10.0);
    assert_eq!(out(&g, miss + CRUMBLE_MEND_MS), 0.0);
}

#[test]
fn overtime_screams_once_per_scare() {
    let mut g = kiai_game("OT");
    g.sfx_log = Some(Vec::new());
    assert_eq!(g.scares.len(), 1);
    let at = g.scares[0];
    assert!(at > g.first_note + 8000.0 && at < g.end_time);
    for k in 0..200 {
        g.set_time(at - 1000.0 + k as f64 * 10.0);
    }
    let screams = |g: &Game| {
        g.sfx_log
            .as_ref()
            .unwrap()
            .iter()
            .filter(|e| matches!(e.1, Sound::Scream))
            .count()
    };
    assert_eq!(screams(&g), 1);
    // Jumping past one leaves it quiet.
    let mut g = kiai_game("OT");
    g.sfx_log = Some(Vec::new());
    g.set_time(at + 5000.0);
    assert_eq!(screams(&g), 0);
}

thread_local! {
    static LOGGED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn record(line: &str) {
    LOGGED.with(|l| l.borrow_mut().push(line.to_owned()));
}

#[test]
fn a_play_s_end_goes_to_the_log_it_was_handed() {
    // The client hands over its audio log (`audiolog::start`); a play's line goes wherever it's
    // told. Here: 40 hits 12 ms late under a -2 ms offset wanted an offset of 10 ms.
    log_to(record);
    let mut p = game();
    p.stats.errors = vec![12.0; 40];
    p.stats.error_offsets = vec![-2.0; 40];
    drop(p);
    let logged = LOGGED.with(|l| l.take());
    assert_eq!(logged.len(), 1, "{logged:?}");
    assert!(
        logged[0].starts_with("end ")
            && logged[0].contains(" hits=40 ")
            && logged[0].contains(" wanted=10.0 "),
        "{}",
        logged[0]
    );
}

fn hits(g: &mut Game, errors: &[f64], offset: f64) {
    g.stats.errors.extend_from_slice(errors);
    g.stats
        .error_offsets
        .extend(std::iter::repeat_n(offset, errors.len()));
}

/// A stream of alternating dons and kats, `spacing` ms apart, at OD 5.
fn stream(spacing: f64) -> Game {
    let notes = (0..24)
        .map(|k| beatmap::Note {
            time: 2000.0 + k as f64 * spacing,
            end_time: 2000.0 + k as f64 * spacing,
            kind: if k % 2 == 0 { Kind::Don } else { Kind::Kat },
            big: false,
            velocity: 0.5,
            tick_spacing: 0.0,
            required_hits: 0,
            fake_len: 0.0,
            fake_fades: false,
            fake_length: 0.0,
            tiny: false,
            unhittable: false,
        })
        .collect();
    let mut map = beatmap::calibration_map(500.0, 4, 1);
    map.notes = notes;
    map.meta.od = 5.0;
    Game::with_mods(map, Arc::new(Silence), None, Mods::NONE)
}

fn play(g: &mut Game, presses: &[(f64, bool)]) {
    for &(t, kat) in presses {
        g.judge_press(if kat { 0 } else { 1 }, kat, t);
    }
    g.set_time(g.map.notes.last().unwrap().time + 1000.0);
}
