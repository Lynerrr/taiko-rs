//! Star rating, pp, letters, levels and the weighted pp total: the numbers players compare.

use super::gameplay::{play, summary};
use super::maps::{self, Player};
use super::snapshot::{assert_snapshot, num};
use crate::mods::{HalfRule, Mods};
use crate::{online, pp, stars};
use std::fmt::Write as _;
use std::path::PathBuf;

/// Writes `chart` into a folder of its own (stars and pp read map files).
pub fn chart_file(chart: &maps::Chart) -> PathBuf {
    // A folder per call: tests running side by side write the same chart, and one rewriting
    // (truncating) the file while another reads it gave that one an empty map.
    static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "taiko-rs-regression-{}-{}-{}",
        chart.name,
        std::process::id(),
        CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{}.osu", chart.name));
    std::fs::write(&path, chart.text()).unwrap();
    path
}

#[test]
fn a_chart_written_again_leaves_the_earlier_file_alone() {
    // Two tests reading the same chart at once each keep their own whole file.
    let chart = maps::rolls_and_swells();
    let first = chart_file(&chart);
    let second = chart_file(&chart);
    assert_ne!(first, second);
    std::fs::write(&second, "").unwrap();
    assert_eq!(std::fs::read_to_string(&first).unwrap(), chart.text());
    assert!(stars::star_rating(&first, 1.0).is_some_and(|s| s > 0.0));
}

fn opt(v: Option<f64>, places: usize) -> String {
    v.map_or_else(|| "-".into(), |v| num(v, places))
}

#[test]
fn star_ratings_by_rate_and_mods() {
    let mut out = String::new();
    for chart in maps::all() {
        let path = chart_file(&chart);
        let _ = write!(
            out,
            "{:<18} text {}",
            chart.name,
            opt(stars::star_rating_text(&chart.text()), 4)
        );
        for rate in [0.75, 1.0, 1.5] {
            let _ = write!(
                out,
                " | x{rate} {}",
                opt(stars::star_rating(&path, rate), 4)
            );
        }
        for m in ["HR", "EZ", "HD", "RX", "HA", "SW", "RD", "FL"] {
            let mods = Mods::parse(m);
            let _ = write!(
                out,
                " | {m} {}",
                opt(stars::star_rating_for(&path, 1.0, mods, 1.0), 4)
            );
        }
        let _ = write!(
            out,
            " | slow-scroll {}",
            opt(stars::star_rating_for(&path, 1.0, Mods::default(), 0.8), 4)
        );
        out.push('\n');
    }
    assert_snapshot("scoring-stars", &out);
}

#[test]
fn pp_for_plays() {
    let mut out = String::new();
    for chart in maps::all() {
        let path = chart_file(&chart);
        let map = chart.beatmap();
        for (player, mods) in [
            (Player::Perfect, ""),
            (Player::Perfect, "HD,HR"),
            (Player::Perfect, "DT"),
            (Player::Perfect, "EZ"),
            (Player::Jitter(45.0), ""),
            (Player::Skips(6), "HD"),
            (Player::Perfect, "WV,PY"),
            (Player::Perfect, "AT"),
        ] {
            let mods = Mods::parse(mods);
            let list = mods.list();
            let refs: Vec<&str> = list.iter().map(String::as_str).collect();
            let g = play(&map, player, &refs);
            let s = g.stats();
            let p = pp::Play {
                great: s.great,
                ok: s.ok,
                miss: s.miss,
                max_combo: s.max_combo,
                rate: mods.speed(1.0, true).0,
                mods,
                scroll_speed: 1.0,
                seed: 7,
                half: HalfRule::OnTheBeat,
            };
            let b = pp::breakdown(&path, &p);
            let _ = writeln!(
                out,
                "{:<18} {:<12} {:<10} pp {} | custom x{} | breakdown {}",
                chart.name,
                player.label(),
                if mods.is_empty() {
                    "NM".into()
                } else {
                    mods.label()
                },
                opt(pp::play_pp(&path, &p), 3),
                num(pp::custom_factor(mods), 4),
                b.map_or_else(
                    || "-".into(),
                    |b| format!(
                        "diff {} acc {} fc {} ss {} stars {} combo {}",
                        num(b.difficulty, 3),
                        num(b.accuracy, 3),
                        num(b.if_fc, 3),
                        num(b.if_ss, 3),
                        num(b.stars, 4),
                        b.max_combo
                    )
                )
            );
        }
        let pot = pp::potential(&path, 1.0, Mods::default(), 1.0);
        let _ = writeln!(
            out,
            "{:<18} potential {}",
            chart.name,
            pot.map_or("-".into(), |p| p.map(|v| num(v, 3)).join(" / "))
        );
    }
    assert_snapshot("scoring-pp", &out);
}

#[test]
fn letters_levels_and_totals() {
    let mut out = String::new();
    for (great, ok, miss) in [
        (100, 0, 0),
        (99, 1, 0),
        (95, 5, 0),
        (90, 10, 0),
        (90, 9, 1),
        (80, 15, 5),
        (70, 20, 10),
        (60, 20, 20),
        (40, 30, 30),
        (0, 0, 100),
    ] {
        let n = (great + ok + miss) as f64;
        let acc = (great as f64 + 0.5 * ok as f64) / n;
        let _ = writeln!(
            out,
            "{great}/{ok}/{miss} acc {} -> {}",
            num(acc * 100.0, 2),
            online::grade(great, ok, miss, acc)
        );
    }
    for score in [
        0u64,
        1,
        100_000,
        1_000_000,
        30_000_000,
        1_000_000_000,
        50_000_000_000,
    ] {
        let _ = writeln!(out, "level({score}) = {}", num(online::level(score), 4));
    }
    let best = vec![500.0, 420.0, 400.0, 380.5, 120.0, 99.9, 50.0, 10.0];
    let _ = writeln!(out, "weighted {}", num(online::weighted_pp(best), 4));
    let _ = writeln!(
        out,
        "weighted empty {}",
        num(online::weighted_pp(Vec::new()), 4)
    );
    for mods in [
        vec!["HD", "HR"],
        vec!["WV", "PY", "SU", "MR"],
        vec!["EZ", "HD", "HR", "FL", "NF", "SD"],
    ] {
        let list: Vec<String> = mods.iter().map(|m| m.to_string()).collect();
        let _ = writeln!(
            out,
            "past the cap {mods:?}: {}",
            online::past_the_cap(&list)
        );
    }
    assert_snapshot("scoring-letters-levels", &out);
}

#[test]
fn the_results_of_a_saved_replay_are_worked_out_again_the_same() {
    // What the results screen does with a replay file: the same numbers as the play itself.
    let chart = maps::rolls_and_swells();
    let path = chart_file(&chart);
    let map = chart.beatmap();
    let inputs = maps::presses(&map, Player::Jitter(35.0));
    let mut r = super::gameplay::replay(inputs, &["HR"], crate::scores::new_version());
    r.map_path = path.to_string_lossy().into_owned();
    let live = super::gameplay::play_replay(&map, &r, 8.0);
    let (stats, windows) = crate::scores::simulate(&path, &r).expect("the map reads");
    assert_eq!(stats.great, live.stats().great);
    assert_eq!(stats.ok, live.stats().ok);
    assert_eq!(stats.miss, live.stats().miss);
    assert_eq!(stats.score, live.stats().score);
    assert_eq!(stats.roll_ticks, live.stats().roll_ticks);
    assert!((windows.great - live.windows().great).abs() < 1e-9);
    assert_snapshot("scoring-simulated-replay", &summary(&live));
}
