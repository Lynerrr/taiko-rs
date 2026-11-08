//! Gameplay: judging, combo, score, accuracy, HP, drumrolls, swells, big notes, mods and
//! replays, played frame by frame without audio and compared with golden results.

use super::maps::{self, Chart, Player};
use super::snapshot::{assert_snapshot, num};
use crate::audio::Track;
use crate::beatmap::{self, Beatmap};
use crate::game::{Game, Judge, WindowRule, Windows};
use crate::mods::{self, Mods};
use crate::scores::{self, Replay};
use std::fmt::Write as _;
use std::sync::Arc;

pub fn silent_track(rate: f64) -> Arc<Track> {
    Arc::new(Track {
        samples: Vec::new(),
        rate,
        stream: None,
    })
}

pub fn replay(inputs: Vec<(f64, u8)>, mods: &[&str], version: u32) -> Replay {
    let list: Vec<String> = mods.iter().map(|m| m.to_string()).collect();
    let (rate, keep_pitch) = Mods::from_list(&list).speed(1.0, true);
    Replay {
        version,
        map_hash: 1,
        map_path: String::new(),
        map_label: "regression".into(),
        player: "tester".into(),
        date: 0,
        rate,
        keep_pitch,
        anti_mash: true,
        mods: list,
        scroll_speed: 1.0,
        score: 0,
        accuracy: 1.0,
        max_combo: 0,
        great: 0,
        ok: 0,
        miss: 0,
        ur: 0.0,
        inputs,
        seed: 7,
    }
}

/// Plays `r` through `map` in `step` ms frames, as the results screen and video export do.
pub fn play_replay(map: &Beatmap, r: &Replay, step: f64) -> Game {
    let mut g = crate::scores::replay_game(map, &silent_track(r.rate), r);
    let end = g.end_time() + 2000.0;
    let mut t = g.time;
    while t < end {
        t = (t + step).min(end);
        g.set_time(t);
    }
    g
}

pub fn play(map: &Beatmap, player: Player, mods: &[&str]) -> Game {
    let r = replay(maps::presses(map, player), mods, scores::new_version());
    play_replay(map, &r, 8.0)
}

/// One line of results: everything the results screen and the score submission use.
pub fn summary(g: &Game) -> String {
    let s = &g.stats;
    format!(
        "great {} ok {} miss {} | combo {} max {} | score {} | rolls {} swells {} strong {} | \
         acc {} | err {} ur {} | hp {} | failed {}",
        s.great,
        s.ok,
        s.miss,
        s.combo,
        s.max_combo,
        s.score,
        s.roll_ticks,
        s.swells,
        s.strong,
        num(s.accuracy() * 100.0, 4),
        num(s.mean_error(), 2),
        num(s.unstable_rate(), 2),
        num(g.hp, 4),
        g.failed(),
    )
}

fn players() -> Vec<Player> {
    vec![
        Player::Perfect,
        Player::Offset(-30.0),
        Player::Offset(18.0),
        Player::Offset(45.0),
        Player::Offset(-90.0),
        Player::Offset(130.0),
        Player::Jitter(25.0),
        Player::Jitter(70.0),
        Player::WrongColour(4),
        Player::Skips(3),
        Player::OneKeyBigs,
        Player::Mash,
        Player::Idle,
        Player::GivesUp,
    ]
}

#[test]
fn every_chart_played_every_way() {
    let mut out = String::new();
    for chart in maps::all() {
        let map = chart.beatmap();
        let _ = writeln!(out, "== {}", chart.name);
        for p in players() {
            let g = play(&map, p, &[]);
            let _ = writeln!(out, "{:<24} {}", p.label(), summary(&g));
        }
    }
    assert_snapshot("gameplay-players", &out);
}

#[test]
fn every_mod_on_its_own() {
    let charts: Vec<(Chart, Player)> = vec![
        (maps::basics(), Player::Jitter(40.0)),
        (maps::big_notes(), Player::Perfect),
        (maps::rolls_and_swells(), Player::Perfect),
        (maps::stream(), Player::Skips(9)),
    ];
    let maps: Vec<(&str, Beatmap, Player)> = charts
        .iter()
        .map(|(c, p)| (c.name, c.beatmap(), *p))
        .collect();
    let mut out = String::new();
    for m in mods::ALL {
        let acr = m.acronym();
        let _ = writeln!(out, "== {acr} ({m:?}) x{}", num(m.multiplier(), 3));
        for (name, map, p) in &maps {
            let g = play(map, *p, &[acr]);
            let _ = writeln!(out, "{name:<18} {}", summary(&g));
        }
    }
    assert_snapshot("gameplay-mods", &out);
}

#[test]
fn mod_combinations() {
    let combos: [&[&str]; 14] = [
        &["HD", "HR"],
        &["HD", "DT"],
        &["HR", "DT"],
        &["EZ", "HT"],
        &["EZ", "NF"],
        &["HD", "HR", "DT", "FL"],
        &["NC", "HR"],
        &["SD", "HR"],
        &["PF", "DT"],
        &["NF", "HR"],
        &["RD", "HR"],
        &["SW", "HD"],
        &["HA", "DT"],
        &["RX", "HR"],
    ];
    let maps: Vec<Beatmap> = [maps::basics(), maps::high_od(), maps::sv_and_kiai()]
        .iter()
        .map(Chart::beatmap)
        .collect();
    let mut out = String::new();
    for combo in combos {
        let shown = Mods::from_list(&combo.iter().map(|m| m.to_string()).collect::<Vec<_>>());
        let _ = writeln!(
            out,
            "== {} (label {}, x{}, rate {})",
            combo.join("+"),
            shown.label(),
            num(shown.multiplier(), 4),
            num(shown.speed(1.0, true).0, 3)
        );
        for map in &maps {
            for p in [Player::Perfect, Player::Jitter(55.0), Player::Skips(5)] {
                let g = play(map, p, combo);
                let _ = writeln!(
                    out,
                    "{:<14} {:<16} {}",
                    map.meta.version,
                    p.label(),
                    summary(&g)
                );
            }
        }
    }
    assert_snapshot("gameplay-mod-combinations", &out);
}

#[test]
fn sudden_death_and_perfect_fail_on_the_first_mistake_and_no_fail_never_fails() {
    let map = maps::basics().beatmap();
    let one_miss = Player::Skips(20);
    for (mods, fails) in [(&["SD"][..], true), (&["PF"], true), (&["NF"], false)] {
        let g = play(&map, one_miss, mods);
        assert_eq!(g.failed(), fails, "{mods:?}");
        if fails {
            assert!(g.failed_early(), "{mods:?} stops the play early");
            assert_eq!(g.stats.miss, 1, "{mods:?} stops at the first miss");
        }
    }
    // Perfect also fails on an OK, Sudden Death doesn't.
    let one_ok = Player::Offset(40.0);
    assert!(play(&map, one_ok, &["PF"]).failed_early());
    assert!(!play(&map, one_ok, &["SD"]).failed_early());
    // Without a mod, missing everything fails the play at the end (HP under half).
    assert!(play(&map, Player::Idle, &[]).failed());
    assert!(!play(&map, Player::Idle, &["NF"]).failed());
}

#[test]
fn judging_does_not_depend_on_the_frame_rate() {
    for chart in maps::all() {
        let map = chart.beatmap();
        for p in [Player::Jitter(60.0), Player::Mash, Player::Perfect] {
            let r = replay(maps::presses(&map, p), &[], scores::new_version());
            let base = summary(&play_replay(&map, &r, 8.0));
            for step in [1.0, 4.17, 16.67, 33.3, 100.0] {
                assert_eq!(
                    summary(&play_replay(&map, &r, step)),
                    base,
                    "{} {} at {step} ms frames",
                    chart.name,
                    p.label()
                );
            }
        }
    }
}

#[test]
fn a_replay_plays_back_the_same_every_time() {
    let map = maps::stream().beatmap();
    let r = replay(
        maps::presses(&map, Player::Jitter(80.0)),
        &["HR"],
        scores::new_version(),
    );
    let first = summary(&play_replay(&map, &r, 8.0));
    for _ in 0..3 {
        assert_eq!(summary(&play_replay(&map, &r, 8.0)), first);
    }
    // And through the saved file (JSON) too.
    let back: Replay = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
    assert_eq!(summary(&play_replay(&map, &back, 8.0)), first);
}

#[test]
fn every_replay_version_is_judged_by_its_own_rules() {
    let maps: Vec<Beatmap> = [maps::basics(), maps::stream(), maps::low_od()]
        .iter()
        .map(Chart::beatmap)
        .collect();
    let mut out = String::new();
    for version in 1..=scores::new_version() {
        for anti_mash in [false, true] {
            if version > 1 && !anti_mash {
                continue;
            }
            for map in &maps {
                for p in [Player::Mash, Player::Jitter(90.0), Player::Offset(-110.0)] {
                    let mut r = replay(maps::presses(map, p), &["HR"], version);
                    r.anti_mash = anti_mash;
                    let g = play_replay(map, &r, 8.0);
                    let _ = writeln!(
                        out,
                        "v{version} anti_mash {anti_mash:<5} {:<8} {:<16} {}",
                        map.meta.version,
                        p.label(),
                        summary(&g)
                    );
                }
            }
        }
    }
    assert_snapshot("gameplay-replay-versions", &out);
}

#[test]
fn hit_windows_by_od_rule_and_rate() {
    let mut out = String::new();
    for k in 0..=20 {
        let od = k as f64 * 0.5;
        for rule in [WindowRule::Osu, WindowRule::Exact] {
            let w = Windows::with_rule(od, rule);
            let _ = write!(
                out,
                "od {od:>4} {rule:?}: great {} ok {} miss {}",
                num(w.great, 3),
                num(w.ok, 3),
                num(w.miss, 3)
            );
            for rate in [0.75, 1.5] {
                let r = w.at_rate(rate);
                let _ = write!(
                    out,
                    " | x{rate} {} {} {}",
                    num(r.great, 2),
                    num(r.ok, 2),
                    num(r.miss, 2)
                );
            }
            out.push('\n');
        }
    }
    for od in [0.0, 5.0, 10.0] {
        let _ = writeln!(
            out,
            "od {od}: HR {} EZ {} DT-rate {} great_ms {}",
            num(Mods::parse("HR").od(od), 3),
            num(Mods::parse("EZ").od(od), 3),
            num(beatmap::od_at_rate(od, 1.5), 3),
            num(beatmap::great_ms(od), 3)
        );
    }
    assert_snapshot("gameplay-hit-windows", &out);
}

/// A single don played `offset` ms off: how it was judged.
fn judge_one(od: f64, offset: f64) -> Option<Judge> {
    let chart = Chart {
        name: "one",
        od,
        hp: 5.0,
        slider_multiplier: 1.4,
        tick_rate: 1.0,
        timing: vec!["1000,500,4,1,0,100,1,0".into()],
        objects: vec![maps::Obj::Don(3000)],
    };
    let map = chart.beatmap();
    let t = map.notes[0].time + offset;
    let g = play_replay(&map, &replay(vec![(t, 1)], &[], scores::new_version()), 8.0);
    let s = &g.stats;
    match (s.great, s.ok, s.miss) {
        (1, 0, 0) => Some(Judge::Great),
        (0, 1, 0) => Some(Judge::Ok),
        (0, 0, 1) => Some(Judge::Miss),
        _ => None,
    }
}

#[test]
fn judgement_boundaries_match_the_windows() {
    let mut out = String::new();
    for od in [0.0, 3.0, 5.0, 8.0, 10.0] {
        // Runs of the same judgement over -160..160 ms in 0.5 ms steps.
        let mut runs: Vec<(f64, f64, Option<Judge>)> = Vec::new();
        for k in -320..=320 {
            let off = k as f64 * 0.5;
            let j = judge_one(od, off);
            match runs.last_mut() {
                Some(r) if r.2 == j => r.1 = off,
                _ => runs.push((off, off, j)),
            }
        }
        let _ = write!(out, "od {od}:");
        for (a, b, j) in runs {
            let _ = write!(out, " [{a}..{b}] {j:?};");
        }
        out.push('\n');
        // The windows say the same.
        let w = Windows::new(od);
        assert_eq!(judge_one(od, w.great - 0.5), Some(Judge::Great), "od {od}");
        assert_eq!(judge_one(od, -w.great + 0.5), Some(Judge::Great), "od {od}");
        assert_eq!(judge_one(od, w.ok - 0.5), Some(Judge::Ok), "od {od}");
    }
    assert_snapshot("gameplay-judgement-boundaries", &out);
}

#[test]
fn autoplay_hits_everything() {
    let mut out = String::new();
    for chart in maps::all() {
        let map = chart.beatmap();
        let mut g = Game::with_mods(map, silent_track(1.0), None, Mods::parse("AT"));
        g.mute_hits = true;
        let end = g.end_time() + 2000.0;
        let mut t = g.time;
        while t < end {
            t = (t + 8.0).min(end);
            g.set_time(t);
            g.autoplay(None);
        }
        assert_eq!(g.stats.miss, 0, "{}", chart.name);
        assert_eq!(g.stats.ok, 0, "{}", chart.name);
        assert!(!g.failed(), "{}", chart.name);
        let _ = writeln!(out, "{:<18} {}", chart.name, summary(&g));
    }
    assert_snapshot("gameplay-autoplay", &out);
}

#[test]
fn practice_from_the_middle_counts_only_what_is_left() {
    let map = maps::basics().beatmap();
    let from = map.notes[32].time - 10.0;
    let r = replay(
        maps::presses(&map, Player::Perfect),
        &[],
        scores::new_version(),
    );
    let mut g = crate::scores::replay_game(&map, &silent_track(1.0), &r);
    g.skip_to(from);
    let end = g.end_time() + 2000.0;
    let mut t = g.time;
    while t < end {
        t = (t + 8.0).min(end);
        g.set_time(t);
    }
    let judged = g.stats.great + g.stats.ok + g.stats.miss;
    assert_eq!(judged, 32, "the notes before the start point aren't played");
    assert_eq!(g.stats.miss, 0);
    assert_snapshot("gameplay-practice-from-middle", &summary(&g));
}

#[test]
fn stats_formulas() {
    let s = crate::game::Stats {
        great: 90,
        ok: 8,
        miss: 2,
        errors: vec![-12.0, 4.0, 9.5, -3.25, 0.0, 22.0],
        ..Default::default()
    };
    assert!((s.accuracy() - 0.94).abs() < 1e-12);
    assert!((crate::game::Stats::default().accuracy() - 1.0).abs() < 1e-12);
    let line = format!(
        "acc {} mean {} ur {}",
        num(s.accuracy(), 6),
        num(s.mean_error(), 6),
        num(s.unstable_rate(), 6)
    );
    assert_snapshot("gameplay-stats-formulas", &line);
}
