use super::hud_draw::HP_BAR_Y;
use super::*;

#[test]
fn the_play_itself_reaches_for_nothing_that_draws() {
    // game.rs is the simulation (judging, scoring, HP, replays); drawing lives in game/field.rs and
    // game/hud_draw.rs. Keeping the drawing layers out of it is what lets it become taiko-logic.
    let sim = include_str!("../game.rs");
    for layer in ["render", "hit", "hud", "layout", "style", "skin", "medals"] {
        assert!(
            !sim.contains(&format!("crate::{layer}::"))
                && !sim.contains(&format!("crate::{layer};")),
            "game.rs names crate::{layer}"
        );
    }
    assert!(!sim.contains("Canvas"), "game.rs draws");
}
use crate::hit::Canvas;
use crate::hud::{self, Part};
use crate::render::{Shape, Sprite, VIRTUAL_HEIGHT, rgba};

#[test]
fn lifted_shapes_leave_a_hole_where_they_were_and_nothing_moves() {
    let mut c = Canvas::default();
    for x in [10.0, 20.0, 30.0, 40.0] {
        c.circle(x, 5.0, 3.0, [1.0; 4], [1.0; 4], 1.0);
    }
    // Two notes of a shape each.
    let lifted = lift_shapes(&mut c, &[1..2, 2..3]);
    assert_eq!(lifted.len(), 2);
    assert_eq!(lifted[0].center[0], 20.0, "copies, as they were drawn");
    assert_eq!(lifted[0].fill, [1.0; 4]);
    assert_eq!(c.shapes.len(), 4, "the list keeps its places");
    assert_eq!(c.shapes[1].fill, [0.0; 4], "drawn as nothing");
    assert_eq!(c.shapes[2].border, [0.0; 4]);
    assert_eq!(c.shapes[3].fill, [1.0; 4], "the rest untouched");
    assert!(
        lift_shapes(&mut c, &[9..12, 20..21]).is_empty(),
        "out of range: none"
    );
}

/// The left end of the HP bar, which ends 18 from the right.
fn hp_bar_x(w: f32) -> f32 {
    w - 18.0 - (w * 0.25).min(180.0)
}

/// A game with `m` on, drawn at map time `t`: where a part under the lane comes out, and the
/// lane's box.
fn part_under_the_lane(m: Option<Mod>, t: f64) -> (hud::Placed, [f32; 4]) {
    let mods = m.map_or(Mods::NONE, |m| Mods::parse(m.acronym()));
    let map = beatmap::calibration_map(500.0, 0, 64);
    let track = Arc::new(Track {
        samples: Vec::new(),
        rate: 1.0,
        stream: None,
    });
    let mut g = Game::with_seed(map, track, None, mods, 42);
    g.view.hidden = hud::Parts::NONE;
    g.time = t;
    let mut c = Canvas::default();
    c.clear(853.3);
    g.draw_playfield(&mut c, 1.0);
    g.draw_parts(&mut c);
    let lane = g.frames(&c).rect(hud::Target::Lane);
    let mut part = g.part_box(&c, Part::Counts).2;
    part.y = g.settled_top(Part::Counts).unwrap_or(part.y);
    (part, lane)
}

#[test]
fn without_mods_the_parts_on_the_lane_stay_where_the_lane_always_is() {
    let (_, lane) = part_under_the_lane(None, 3000.0);
    let plain = hud::Frames::new(853.3, crate::layout::Area::FULL);
    for (a, b) in lane.iter().zip(plain.rect(hud::Target::Lane)) {
        assert!((a - b).abs() < 0.01, "{lane:?}");
    }
}

#[test]
fn a_part_under_a_bent_lane_goes_by_the_lane_right_above_it() {
    // The left of these lanes runs straight: the counter on it stays just under it.
    let straight = part_under_the_lane(None, 3000.0).0;
    let drop = RailShape::Loop.drop();
    let (part, _) = part_under_the_lane(Some(Mod::Loop), 3000.0);
    assert!(
        (part.y - straight.y - drop).abs() < 1.0,
        "{part:?} from {straight:?}"
    );
}

#[test]
fn parts_pushed_against_the_edge_by_a_lane_give_way_to_each_other() {
    let map = beatmap::calibration_map(500.0, 0, 64);
    let track = Arc::new(Track {
        samples: Vec::new(),
        rate: 1.0,
        stream: None,
    });
    let mods = Mods::parse(Mod::Loop.acronym());
    let mut g = Game::with_seed(map, track, None, mods, 42);
    let lane = hud::builtin_presets()
        .into_iter()
        .find(|p| {
            p.1.place(Part::Score)
                .is_some_and(|s| s.y.target == hud::Target::Lane)
        })
        .expect("the lane preset");
    g.view.hud = lane.1;
    g.view.hidden = lane.1.off;
    g.live_pp_value = Some(100.0);
    g.time = 3000.0;
    let mut c = Canvas::default();
    c.clear(853.3);
    g.draw_playfield(&mut c, 1.0);
    g.draw_parts(&mut c);
    let at = |p: Part| {
        let mut b = g.part_box(&c, p).2;
        b.y = g.settled_top(p).unwrap_or(b.y);
        b
    };
    let parts = [Part::Score, Part::Accuracy, Part::Ur, Part::Mods];
    for (i, &p) in parts.iter().enumerate() {
        let (a, y0) = (at(p), at(p).y);
        for &q in &parts[i + 1..] {
            let b = at(q);
            let apart = a.x + a.w <= b.x + 0.5
                || b.x + b.w <= a.x + 0.5
                || y0 + a.h <= b.y + 0.5
                || b.y + b.h <= y0 + 0.5;
            assert!(apart, "{p:?} {a:?} on {q:?} {b:?}");
        }
    }
}

#[test]
fn parts_side_by_side_on_a_lane_that_only_moved_stay_side_by_side() {
    let map = beatmap::calibration_map(500.0, 0, 64);
    let track = Arc::new(Track {
        samples: Vec::new(),
        rate: 1.0,
        stream: None,
    });
    let mods = Mods::parse(Mod::Zigzag.acronym());
    let mut g = Game::with_seed(map, track, None, mods, 42);
    let (_, lane) = hud::builtin_presets().swap_remove(3);
    g.view.hud = lane;
    g.view.hidden = lane.off;
    g.live_pp_value = Some(100.0);
    g.time = 3000.0;
    let mut c = Canvas::default();
    c.clear(853.3);
    g.draw_playfield(&mut c, 1.0);
    g.draw_parts(&mut c);
    let top = |p| g.settled_top(p).unwrap();
    assert!(
        (top(Part::Pp) + 16.0 - top(Part::Accuracy) - 16.0).abs() < 40.0,
        "pp {} accuracy {}",
        top(Part::Pp),
        top(Part::Accuracy)
    );
}

#[test]
fn an_upright_lane_is_tall_and_narrow() {
    let (_, lane) = part_under_the_lane(Some(Mod::Mania), 3000.0);
    assert!(lane[3] > lane[2], "{lane:?}");
}

#[test]
fn what_shakes_the_playfield_leaves_the_parts_still() {
    for m in [Mod::Party, Mod::Tilt, Mod::Spin, Mod::BarrelRoll] {
        let a = part_under_the_lane(Some(m), 3000.0).0;
        let b = part_under_the_lane(Some(m), 3777.0).0;
        assert_eq!((a.x, a.y), (b.x, b.y), "{m:?}");
    }
}

#[test]
fn the_parts_on_the_lane_find_the_playfield_in_focus() {
    let (part, lane) = part_under_the_lane(Some(Mod::Multiverse), 3000.0);
    assert!(lane[2] > 0.0 && lane[3] > 0.0, "{lane:?}");
    assert!(part.y >= lane[1], "{part:?} in {lane:?}");
}

#[test]
fn a_morphing_lane_carries_the_parts_along_without_a_jump() {
    let map = beatmap::calibration_map(500.0, 0, 64);
    let track = Arc::new(Track {
        samples: Vec::new(),
        rate: 1.0,
        stream: None,
    });
    let mods = Mods::parse(Mod::Shapeshift.acronym());
    let mut g = Game::with_seed(map, track, None, mods, 42);
    let setup = "every=4 order=fixed shapes=spiral,loop,elbow,-zigzag,-uturn,-straight";
    g.use_mod_settings(None, Some(shapeshift::Config::parse(setup)));
    let (mut c, mut last, mut moved) = (Canvas::default(), None, 0.0f32);
    let (mut worst, mut at) = (0.0f32, 0.0);
    for i in 0..4000 {
        g.time = 2000.0 + i as f64 * 16.0;
        c.clear(853.3);
        g.draw_playfield(&mut c, 1.0);
        g.draw_parts(&mut c);
        let mut lane = g.frames(&c).rect(hud::Target::Lane);
        lane[1] = g.settled_top(Part::Counts).unwrap_or(lane[1]);
        if let Some(before) = last {
            let step = lane
                .iter()
                .zip::<[f32; 4]>(before)
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f32::max);
            moved += step;
            if step > worst {
                (worst, at) = (step, g.time);
            }
        }
        last = Some(lane);
    }
    assert!(moved > 50.0, "the lane's box went with the shapes: {moved}");
    assert!(worst < 40.0, "jumped {worst} at {at}");
}

#[test]
fn block_culling_preserves_the_original_draw_for_every_mod() {
    draw_with_defaults();
    let mut map = beatmap::calibration_map(500.0, 0, 64);
    let template = map.notes[0].clone();
    map.notes = (0..2048)
        .map(|i| {
            let time = 2000.0 + i as f64 * 100.0;
            let special = i % 511 == 0;
            beatmap::Note {
                time,
                end_time: time + if special { 9000.0 } else { 0.0 },
                kind: if special {
                    if i % 2 == 0 { Kind::Roll } else { Kind::Swell }
                } else {
                    Kind::Don
                },
                velocity: if i == 2047 { 1e-5 } else { 0.4 },
                fake_len: if i == 511 { 200.0 } else { 0.0 },
                ..template.clone()
            }
        })
        .collect();
    for mods in std::iter::once(Mods::NONE).chain(
        crate::mods::ALL
            .into_iter()
            .map(|m| Mods::parse(m.acronym())),
    ) {
        let track = Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        });
        let mut optimized = Game::with_seed(map.clone(), track.clone(), None, mods, 42);
        let mut reference = Game::with_seed(map.clone(), track, None, mods, 42);
        // Multiverse's universes are process-wide too: its own for each, not whatever
        // another test has just set.
        optimized.universes = Some(multiverse::classic());
        reference.universes = Some(multiverse::classic());
        // One special block disables the new bounds, reproducing the original scan.
        reference.view.note_blocks = vec![NoteBlock {
            range: 0..reference.map.notes.len(),
            min_velocity: 1e-6,
            last_end: f64::INFINITY,
            regular: false,
        }];
        for time in [0.0, 3000.0, 55_555.0, 190_000.0, 210_000.0] {
            optimized.time = time;
            reference.time = time;
            let mut a = Canvas::default();
            let mut b = Canvas::default();
            a.clear(853.3);
            b.clear(853.3);
            optimized.draw_playfield(&mut a, 1.0);
            reference.draw_playfield(&mut b, 1.0);
            assert!(
                bytemuck::cast_slice::<_, u8>(&a.shapes)
                    == bytemuck::cast_slice::<_, u8>(&b.shapes),
                "mods={mods:?}, time={time}, shapes={} vs {}",
                a.shapes.len(),
                b.shapes.len()
            );
        }
    }
}

fn game() -> Game {
    let map = beatmap::calibration_map(500.0, 0, 64);
    Game::new(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    )
}

/// A map of `rolls` drumrolls with ticks a hair apart, on screen together: the kind of gimmick
/// that once asked the graphics card for more shapes in a frame than it can hold.
fn map_of_dense_rolls(rolls: usize) -> Game {
    let mut map = beatmap::calibration_map(500.0, 0, 64);
    let template = map.notes[0].clone();
    map.notes = (0..rolls)
        .map(|i| beatmap::Note {
            kind: Kind::Roll,
            time: 2000.0 + i as f64,
            end_time: 200_000.0,
            tick_spacing: 0.5,
            velocity: 0.02,
            ..template.clone()
        })
        .collect();
    let track = Arc::new(Track {
        samples: Vec::new(),
        rate: 1.0,
        stream: None,
    });
    Game::new(map, track, None)
}

#[test]
fn roll_bodies_stacked_on_a_bent_lane_dont_flood_the_frame() {
    let mut map = beatmap::calibration_map(500.0, 0, 64);
    let template = map.notes[0].clone();
    // As Kikuo - Gakkou o Yasunda Hi no Koto [Aspire]: long rolls, all at once, on the Maze.
    map.notes = (0..400)
        .map(|i| beatmap::Note {
            kind: Kind::Roll,
            time: 2000.0 + i as f64,
            end_time: 400_000.0,
            velocity: 0.4,
            ..template.clone()
        })
        .collect();
    let track = Arc::new(Track {
        samples: Vec::new(),
        rate: 1.0,
        stream: None,
    });
    let mut g = Game::with_seed(map, track, None, Mods::parse("MZ"), 42);
    g.time = 1990.0;
    let mut c = Canvas::default();
    c.clear(853.3);
    g.draw_playfield(&mut c, 1.0);
    assert!(c.shapes.len() < 100_000, "{} shapes", c.shapes.len());
}

#[test]
fn ticks_too_close_to_tell_apart_dont_flood_the_frame() {
    let mut g = map_of_dense_rolls(8);
    g.time = 1990.0;
    let mut c = Canvas::default();
    c.clear(853.3);
    g.draw_playfield(&mut c, 1.0);
    assert!(c.shapes.len() < 50_000, "{} shapes", c.shapes.len());
}

#[test]
fn notes_stay_on_screen_as_long_as_in_osu() {
    // A 180 BPM map at SV 1 and SliderMultiplier 1.4 (taiko's 1.4x on top).
    let v = 1.4 * 1.4 * 100.0 / (60_000.0 / 180.0);
    for (aspect, osu_aspect) in [
        (16.0 / 9.0, 16.0 / 9.0),
        (4.0 / 3.0, 4.0 / 3.0),
        (21.0 / 9.0, 16.0 / 9.0),
    ] {
        let w = aspect as f32 * VIRTUAL_HEIGHT;
        let ours = (w - HIT_X) as f64 / (osu_speed_scale(w) * v);
        let osu = (osu_aspect * 480.0 - 160.0) / v;
        assert!((ours - osu).abs() < 0.01, "{aspect}: {ours} vs {osu}");
    }
}

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
fn drawing_multiverse_leaves_the_play_s_mods_alone() {
    // Each universe is drawn with its own mods on top of the play's, passed to the drawing, not
    // set on the play (which judging reads).
    draw_with_defaults();
    let mut g = modded("MV");
    g.use_mod_settings(Some(multiverse::defaults()), None);
    g.set_time(3000.0);
    let before = g.mods;
    let mut c = Canvas {
        width: 853.3,
        ..Default::default()
    };
    g.draw_playfield(&mut c, 1.0);
    assert_eq!(g.mods, before);
    assert!(!c.shapes.is_empty());
}

fn modded(mods: &str) -> Game {
    let map = beatmap::calibration_map(500.0, 0, 64);
    Game::with_mods(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
        Mods::parse(mods),
    )
}

#[test]
fn random_colours_each_play_its_own_way_and_replays_the_same() {
    let colours = |seed: u64| {
        let g = Game::with_seed(
            beatmap::calibration_map(500.0, 0, 64),
            Arc::new(Track {
                samples: Vec::new(),
                rate: 1.0,
                stream: None,
            }),
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

/// Hits every note `err` ms off (centre key, so dons), then lets the map end.
fn play_all(g: &mut Game, err: f64) {
    let times: Vec<f64> = g.map.notes.iter().map(|n| n.time).collect();
    for t in times {
        g.press_at(1, t + err, None);
    }
    let end = g.end_time + 2000.0;
    g.set_time(end);
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
    let mut g = Game::with_mods(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
        Mods::parse("AL"),
    );
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
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
        Mods::parse("AL"),
    );
    g.press_time(l, 0.0, None);
    g.press_time(l, 500.0, None);
    g.set_time(9000.0);
    assert_eq!((g.stats.great, g.stats.miss), (1, 3));
}

#[test]
fn tact_notes_are_in_the_hit_circle_on_time_and_never_go_back() {
    let mods = Mods::parse("TC");
    // Bars of 2000 ms from 0 on.
    let start = |x: f64| (x / 2000.0).floor() * 2000.0;
    for note in [2500.0, 3999.0, 4000.0, 7321.0] {
        let mut last = f32::INFINITY;
        let mut t = note - 3000.0;
        while t <= note {
            let warp = crate::tact::shown_time(t, start) - t;
            let s = Scroll {
                t,
                speed: osu_speed_scale(640.0 + HIT_X),
                lane: 640.0,
                mods,
                warp,
                live: 1.0,
                shared: None,
            };
            let x = s.x(note, 0.5);
            assert!(x <= last + 1e-3, "note {note} at {t}: {last} -> {x}");
            last = x;
            t += 1.0;
        }
        assert!(
            (last - HIT_X).abs() < 1e-3,
            "note {note} is at {last}, not the drum, on time"
        );
    }
}

#[test]
fn a_shared_scroll_freezes_and_reverses_notes_and_they_still_land_on_time() {
    use crate::tko::scroll::ScrollTable;
    use crate::tko::track::{Ease, Key, Track};
    let mut speed = Track::default();
    for (t, v) in [(0.0, 1.0), (1000.0, 0.0), (2000.0, -1.0), (2500.0, 2.0)] {
        speed.set(Key {
            t,
            v,
            ease: Ease::Step,
        });
    }
    let table = std::sync::Arc::new(ScrollTable::new(&speed).unwrap());
    let at = |t: f64| {
        let s = Scroll {
            t,
            speed: osu_speed_scale(640.0 + HIT_X),
            lane: 640.0,
            mods: Mods::default(),
            warp: 0.0,
            live: 1.0,
            shared: Some((table.clone(), 0.0, table.x(t))),
        };
        s.x(4000.0, 0.5)
    };
    // Frozen: the note holds still; backwards: it moves away from the drum.
    assert_eq!(at(1200.0), at(1800.0));
    assert!(at(2400.0) > at(2100.0));
    // At its hit it is on the drum, whatever the scroll did on the way.
    assert!((at(4000.0) - HIT_X).abs() < 1e-3);
    // The map's own slider velocity still counts: a note twice as fast is twice as far.
    let s = Scroll {
        t: 3000.0,
        speed: osu_speed_scale(640.0 + HIT_X),
        lane: 640.0,
        mods: Mods::default(),
        warp: 0.0,
        live: 1.0,
        shared: Some((table.clone(), 0.0, table.x(3000.0))),
    };
    let (slow, fast) = (s.x(3100.0, 0.25) - HIT_X, s.x(3100.0, 0.5) - HIT_X);
    assert!((fast - 2.0 * slow).abs() < 1e-3);
}

#[test]
fn pulse_and_stutter_never_pull_notes_back_on_sv_heavy_maps() {
    // A red line, then green lines off the beat every 130 ms (as on SV-heavy maps).
    let red = beatmap::ControlPoint {
        time: 0.0,
        beat_origin: 0.0,
        beat_length: 500.0,
        sv: 1.0,
        kiai: false,
        volume: 1.0,
    };
    let cps: Vec<_> = std::iter::once(red)
        .chain((0..150).map(|k| beatmap::ControlPoint {
            time: 65.0 + 130.0 * k as f64,
            ..red
        }))
        .collect();
    for mods in ["PU", "ST"] {
        let mods = Mods::parse(mods);
        let mut last = f32::INFINITY;
        let mut t = 6000.0;
        while t < 10_000.0 {
            let warp = beat_warp(mods, beatmap::control_point_at(&cps, t), t);
            let s = Scroll {
                t,
                speed: osu_speed_scale(640.0 + HIT_X),
                lane: 640.0,
                mods,
                warp,
                live: 1.0,
                shared: None,
            };
            let x = s.x(10_000.0, 0.5);
            assert!(x <= last + 1e-3, "{mods:?} at {t}: {last} -> {x}");
            last = x;
            t += 2.0;
        }
    }
}

#[test]
fn multiverse_breathes_without_a_jump_on_the_beat() {
    // From 300 BPM to 60 BPM, a 60 fps frame never moves the breath more than a third of
    // the way (it used to jump all the way on every beat).
    for beat in [200.0, 300.0, 500.0, 1000.0] {
        let mut last = breath(0.0, beat);
        let mut t = 16.7;
        while t < 4.0 * beat {
            let b = breath((t / beat).rem_euclid(1.0), beat);
            assert!((0.0..=1.0).contains(&b));
            assert!(
                (b - last).abs() < 0.34,
                "beat {beat} ms at {t}: {last} -> {b}"
            );
            last = b;
            t += 16.7;
        }
    }
}

#[test]
fn visual_offset_moves_the_lane_not_the_judging() {
    for rate in [1.0, 1.5] {
        let map = beatmap::calibration_map(500.0, 0, 64);
        let mut g = Game::new(
            map,
            Arc::new(Track {
                samples: Vec::new(),
                rate,
                stream: None,
            }),
            None,
        );
        g.view.visual_offset = 30.0;
        g.set_time(2000.0);
        // Real milliseconds: at 1.5x, 30 ms of song is 45 ms of map.
        assert!(
            (g.visual_time() - (2000.0 + 30.0 * rate)).abs() < 1e-6,
            "{rate}x: {}",
            g.visual_time()
        );
    }
    let mut g = modded("");
    g.view.visual_offset = 30.0;
    play_all(&mut g, 0.0);
    assert!(g.stats.great == 64 && g.stats.mean_error().abs() < 1e-9);
}

#[test]
fn a_paused_lane_holds_still_however_the_vsync_lead_moves() {
    let mut g = Game::new(
        beatmap::calibration_map(500.0, 0, 64),
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    );
    g.set_visual_offset(30.0);
    g.set_time(2000.0);
    g.pause(None);
    let frozen = g.visual_time();
    for lead in [30.4, 29.7, 31.2] {
        g.set_visual_offset(lead);
        assert_eq!(g.visual_time(), frozen);
    }
    g.resume(None);
    g.set_visual_offset(31.2);
    assert!((g.visual_time() - (2000.0 + 31.2)).abs() < 1e-6);
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

fn hits(g: &mut Game, errors: &[f64], offset: f64) {
    g.stats.errors.extend_from_slice(errors);
    g.stats
        .error_offsets
        .extend(std::iter::repeat_n(offset, errors.len()));
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
        .filter(|e| matches!(e.1, Sfx::ComboBreak))
        .map(|e| e.0)
        .collect();
    assert_eq!(breaks.len(), 1, "{breaks:?}");
    assert!(
        log.iter()
            .filter(|e| !matches!(e.1, Sfx::ComboBreak))
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
    Game::with_mods(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
        Mods::NONE,
    )
}

fn play(g: &mut Game, presses: &[(f64, bool)]) {
    for &(t, kat) in presses {
        g.judge_press(if kat { 0 } else { 1 }, kat, t);
    }
    g.set_time(g.map.notes.last().unwrap().time + 1000.0);
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
/// The calibration map with a kiai from 8 s, for the mods that wait for one.
fn kiai_game(mods: &str) -> Game {
    let mut map = beatmap::calibration_map(500.0, 0, 64);
    let cp = map.control_points[0];
    map.control_points.push(beatmap::ControlPoint {
        time: 8000.0,
        kiai: true,
        ..cp
    });
    Game::with_mods(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
        Mods::parse(mods),
    )
}

#[test]
fn in_kiai_each_fake_slider_under_a_note_adds_its_own_flash() {
    // Lights drawn with `stack` fake sliders under the note at 10 s, at map time `at`.
    let lights = |stack: usize, at: f64| {
        let mut map = beatmap::calibration_map(500.0, 0, 64);
        let cp = map.control_points[0];
        map.control_points.push(beatmap::ControlPoint {
            time: 8000.0,
            kiai: true,
            ..cp
        });
        let head = map
            .notes
            .iter()
            .find(|n| n.time == 10000.0)
            .unwrap()
            .clone();
        for _ in 0..stack {
            map.notes.push(beatmap::Note {
                kind: Kind::Roll,
                fake_len: 0.0001,
                fake_fades: true,
                tick_spacing: 125.0,
                ..head.clone()
            });
        }
        map.notes.sort_by(|a, b| a.time.total_cmp(&b.time));
        let mut g = Game::new(
            map,
            Arc::new(Track {
                samples: Vec::new(),
                rate: 1.0,
                stream: None,
            }),
            None,
        );
        g.time = at;
        let mut c = Canvas::default();
        c.clear(853.3);
        g.draw_playfield(&mut c, 1.0);
        c.shapes.iter().filter(|s| s.params[2] < -0.5).count()
    };
    // On a kiai beat, the deeper the pile the more light; outside kiai, none at all.
    assert_eq!(lights(3, 9510.0) - lights(0, 9510.0), 3);
    assert_eq!(lights(3, 5010.0), 0);
}

#[test]
fn the_kiai_flash_is_stables_three_quarter_beat_on_the_first_red_lines_beats() {
    // 500 ms beats from 0; green lines toggle kiai at `kiai`, a 6 ms red line from 20.1 s.
    let game = |kiai: &[(f64, bool)]| {
        let mut map = beatmap::calibration_map(500.0, 0, 64);
        let cp = map.control_points[0];
        for &(time, on) in kiai {
            map.control_points.push(beatmap::ControlPoint {
                time,
                kiai: on,
                ..cp
            });
        }
        map.control_points.push(beatmap::ControlPoint {
            time: 20100.0,
            beat_origin: 20100.0,
            beat_length: 6.0,
            kiai: true,
            ..cp
        });
        Game::new(
            map,
            Arc::new(Track {
                samples: Vec::new(),
                rate: 1.0,
                stream: None,
            }),
            None,
        )
    };
    let inf = f64::INFINITY;
    // An object at `t` itself, so always within reach.
    let at = |g: &Game, t: f64, end: f64| g.kiai_flash(t, t, end);
    // Kiai 8100..8700, off the beat: nothing until the red line's beat at 8500.
    let g = game(&[(8100.0, true), (8700.0, false)]);
    assert_eq!(at(&g, 8100.0, inf), 0.0);
    assert_eq!(at(&g, 8500.0, inf), 1.0);
    // Linear over int(375) ms, then dark for the last quarter.
    assert!((at(&g, 8687.5, inf) - 0.5).abs() < 1e-6);
    // The kiai ends at 8700, mid-flash: the flash finishes anyway.
    assert!((at(&g, 8800.0, inf) - 0.2).abs() < 1e-6);
    assert_eq!(at(&g, 8875.0, inf), 0.0);
    assert_eq!(at(&g, 9000.0, inf), 0.0);
    // A note's flash stops at the note: fading over the 50 ms left to it.
    assert!((at(&g, 8525.0, 8550.0) - 0.5).abs() < 1e-6);
    assert_eq!(at(&g, 8560.0, 8550.0), 0.0);
    // Only objects up to 2.625 first-line beats past the beat take it.
    assert_eq!(g.kiai_flash(8500.0, 9812.0, inf), 1.0);
    assert_eq!(g.kiai_flash(8500.0, 9813.0, inf), 0.0);
    // A kiai with no beat in it never flashes.
    let g = game(&[(8100.0, true), (8300.0, false), (19000.0, true)]);
    assert!((8000..9000).all(|t| at(&g, t as f64, inf) == 0.0));
    // A gimmick red line keeps the first line's beats: 20000's flash, 100 ms in.
    assert!((at(&g, 20100.0, inf) - (1.0 - 100.0 / 375.0)).abs() < 1e-6);
    // Colours: don and kat brightened by 25 each.
    assert_eq!(
        (kiai_brighten(DON), kiai_brighten(KAT)),
        (0xFF5E45, 0x5CA7C6)
    );
}

#[test]
fn a_pop_up_fake_slider_stands_by_its_length_and_fades_in() {
    let head = beatmap::calibration_map(387.0, 0, 4).notes[0].clone();
    // On a 1 ms red line (velocity ~302), lengths -300, -450 and -510, at 1000.
    let fake = |len: f64, velocity: f64| beatmap::Note {
        time: 1000.0,
        kind: Kind::Roll,
        fake_len: 1.0,
        fake_fades: false,
        fake_length: len,
        velocity,
        ..head.clone()
    };
    let lane = 640.0;
    let at = |len: f64, vt: f64| popup(&fake(len, 302.0), vt, lane, 1.0, 387.0, 1.8, 1.0);
    let s = |len: f64| at(len, 950.0).unwrap().0 / lane;
    // Stable: -300 at ~17% of the lane, the farthest well inside its first 75%.
    assert!(
        (s(300.0) - 0.17).abs() < 0.01 && s(510.0) < 0.75,
        "{} {}",
        s(300.0),
        s(510.0)
    );
    assert!((s(450.0) - s(300.0) - 150.0 / 30.0 * 95.0 / 1560.0).abs() < 1e-4);
    // Shown 300 / 180 beats before (645 ms), -510 2.83 beats (1097 ms), fading in over
    // 666 ms; gone at its time, no fade out (stable).
    assert_eq!(at(300.0, 1000.0 - 650.0).unwrap().2, 0.0);
    assert!(at(300.0, 1000.0 - 600.0).unwrap().2 > 0.0);
    assert!(at(510.0, 1000.0 - 1050.0).unwrap().2 > 0.0);
    assert_eq!(at(510.0, 1000.0 - 400.0).unwrap().2, 1.0);
    assert!(at(300.0, 1000.0).is_none());
    // At an ordinary speed it's the usual fake slider.
    assert!(popup(&fake(300.0, 0.78), 950.0, lane, 1.0, 387.0, 1.8, 1.0).is_none());
}

#[test]
fn a_note_before_a_nan_slider_cannot_be_hit() {
    let mut map = beatmap::calibration_map(500.0, 0, 64);
    map.notes[4].unhittable = true;
    let t = map.notes[4].time;
    let mut g = Game::new(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    );
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
    let mut g = Game::new(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    );
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
    let mut g = Game::new(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    );
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

fn quiet_game(osu: &str) -> Game {
    Game::new(
        beatmap::tests::parse(osu),
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    )
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
    assert_eq!((got[0].0, got[0].1), (1030.0, Sfx::Don));
    // Not twice, however often it is asked.
    assert!(g.plan_auto_sounds(200.0).is_empty());
    g.time = 1400.0;
    let got = g.plan_auto_sounds(200.0);
    assert_eq!(
        got.iter().map(|g| (g.0, g.1)).collect::<Vec<_>>(),
        [(1530.0, Sfx::Kat)]
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
    assert_eq!(got[0].1, Sfx::Don);
    assert!(got[1..].iter().all(|s| s.1 == Sfx::BigDon));
    // Roll ticks, each at its own place.
    g.time = 2900.0;
    let ticks = g.plan_auto_sounds(600.0);
    assert!(ticks.len() >= 2);
    assert_eq!(ticks[0].0, 3030.0);
    assert_eq!(ticks[1].0, 3030.0 + g.map.notes[3].tick_spacing);
    assert!(g.plan_auto_sounds(600.0).is_empty());
}

#[test]
fn autoplay_sounds_are_heard_with_the_music_at_the_osu_note_time() {
    // The test maps are not shifted on load: a .osu note at 1000 is at map time 1023; with an osu! offset of 0 and a latency of 30
    // the clock's offset is -23 - 30, and the sound goes to song position 1000 all the same.
    let osu = "osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nSliderMultiplier:1.4\nOverallDifficulty:5\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n256,192,1023,1,0,0:0:0:0:\n";
    let mut g = quiet_game(osu);
    g.latency = 30.0;
    g.offset = crate::config::play_offset_of(0.0, 30.0);
    g.time = 950.0;
    let got = g.plan_auto_sounds(30.0 + AUTO_SOUND_LEAD_MS);
    assert_eq!(got.len(), 1);
    assert!((got[0].0 - 1000.0).abs() < 1e-9, "{}", got[0].0);
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
    let map = beatmap::tests::parse(osu);
    let mut g = Game::new(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    );
    for k in 0..300 {
        g.time = k as f64 * 10.0;
        g.autoplay(None);
        g.expire(g.time);
    }
    assert_eq!(g.stats.miss, 0);
}

/// The playfield of `map` drawn at map time `t`, with a skin's `sliderscorepoint` (so ticks
/// are sprites, tinted as the game tints them).
fn drawn(map: beatmap::Beatmap, t: f64) -> Canvas {
    let mut g = Game::new(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    );
    g.set_time(t);
    let mut c = Canvas {
        width: 853.3,
        ..Default::default()
    };
    c.sprites[Sprite::RollTick as usize] = Some([0.4, 0.4]);
    g.draw_playfield(&mut c, 1.0);
    c
}

fn is_sprite(sh: &Shape, sprite: Sprite) -> bool {
    sh.params[3] == 10.0 + sprite as usize as f32
}

// Base mechanics the gimmick fixes broke once (owner, 2026-10-30): these keep them as stable
// has them.

#[test]
fn roll_ticks_stay_on_the_body_in_sliderscorepoints_own_colour() {
    // 100 BPM (1/8 ticks, 75 ms apart) and 180 BPM (1/4): each roll 7.6 ticks long, so its
    // last tick is rounded half a tick past its end.
    for beat in [600.0, 333.3] {
        let mut map = beatmap::calibration_map(beat, 0, 16);
        let spacing = if beat > 480.0 { beat / 8.0 } else { beat / 4.0 };
        let head = map.notes[4].clone();
        let roll = beatmap::Note {
            kind: Kind::Roll,
            end_time: head.time + 7.6 * spacing,
            tick_spacing: spacing,
            ..head.clone()
        };
        map.notes
            .retain(|n| n.time < head.time || n.time > roll.end_time + 1000.0);
        map.notes.insert(4, roll);
        let c = drawn(map, head.time - 50.0);
        let body = c
            .shapes
            .iter()
            .find(|sh| {
                sh.params[3] == 0.0
                    && sh.half[0] > sh.half[1] * 2.0
                    && sh.fill[..3] == rgba(ROLL, 1.0)[..3]
            })
            .expect("roll body");
        let body_end = body.center[0] + body.half[0] - body.half[1];
        let ticks: Vec<&Shape> = c
            .shapes
            .iter()
            .filter(|sh| is_sprite(sh, Sprite::RollTick))
            .collect();
        // k = 0 is under the head, k = 8 past the end: 1..=7 drawn.
        assert_eq!(ticks.len(), 7, "beat {beat}");
        for t in ticks {
            assert!(
                t.center[0] <= body_end + 0.01,
                "beat {beat}: a tick at {} past the body's end at {body_end}",
                t.center[0]
            );
            assert_eq!(
                t.fill[..3],
                [1.0, 1.0, 1.0],
                "beat {beat}: ticks aren't tinted"
            );
        }
    }
}

/// A map: `first` SV from the start, a 333 ms beat, SV `later` from 2000 on, a swell at 4000.
fn swell_map(first: f64, later: f64) -> beatmap::Beatmap {
    let sv = |x: f64| -100.0 / x;
    beatmap::tests::parse(&format!(
        "osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nSliderMultiplier:1.4\nOverallDifficulty:5\n\n[TimingPoints]\n0,333.333333333333,4,1,0,100,1,0\n0,{},4,1,0,100,0,0\n2000,{},4,1,0,100,0,0\n\n[HitObjects]\n256,192,1000,1,0,0:0:0:0:\n256,192,3000,1,0,0:0:0:0:\n256,192,4000,12,0,6000,0:0:0:0:\n",
        sv(first),
        sv(later)
    ))
}

#[test]
fn stables_preempt_comes_from_the_first_timing_point() {
    // 1.4 x 1.4 x 100 / 333.33 = 0.588 osu!px per ms at 1x: 1000 px in 1700 ms.
    for (first, later, preempt) in [
        (1.0, 1.0, 1700.0),
        (1.0, 0.5, 1700.0),
        (10.0, 1.0, 170.0),
        (0.5, 2.0, 3401.0),
        (0.1, 1.0, 5000.0),
    ] {
        let map = swell_map(first, later);
        assert_eq!(
            stable_preempt(map.first_velocity),
            preempt,
            "SV {first} then {later}"
        );
        // The swell itself scrolls at its own SV.
        let (head, swell) = (&map.notes[0], &map.notes[2]);
        assert_eq!(swell.kind, Kind::Swell);
        assert!((swell.velocity / head.velocity - later / first).abs() < 1e-9);
    }
}

#[test]
fn a_swell_fades_in_from_preempt_to_six_tenths_of_it_before() {
    assert_eq!(swell_fade(1700.0, 1800.0), 0.0);
    assert_eq!(swell_fade(1700.0, 1700.0), 0.0);
    assert_eq!(swell_fade(1700.0, 1360.0), 0.5);
    assert_eq!(swell_fade(1700.0, 1020.0), 1.0);
    assert_eq!(swell_fade(1700.0, 0.0), 1.0);
}

#[test]
fn when_a_swell_shows_depends_on_the_first_sv_not_its_own() {
    let swell_alpha = |c: &Canvas| {
        c.shapes
            .iter()
            .filter(|sh| sh.params[3] == 0.0 && sh.fill[..3] == rgba(SWELL, 1.0)[..3])
            .map(|sh| sh.fill[3])
            .fold(0.0, f32::max)
    };
    // Starting at 10x: PreEmpt 170 ms, so a 0.5x swell 400 ms out (well on the lane) is still
    // hidden, and only fully there 102 ms before.
    assert_eq!(swell_alpha(&drawn(swell_map(10.0, 0.5), 3600.0)), 0.0);
    assert!(swell_alpha(&drawn(swell_map(10.0, 0.5), 3920.0)) > 0.99);
    // Starting at 1x, the same swell is fully there 1 s out.
    assert!(swell_alpha(&drawn(swell_map(1.0, 0.5), 3000.0)) > 0.99);
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
    let mut g = Game::new(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    );
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
    let mut g = Game::new(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    );
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
    let mut g = Game::new(
        map.clone(),
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    );
    let mut t = 1800.0;
    while t < 4300.0 {
        g.time = t;
        g.autoplay(None);
        t += 4.0;
    }
    assert_eq!(under(&g), vec![Some(Judge::Great); 5]);
    assert_eq!(g.stats.roll_ticks, 0);
    // A player: a press on the kat inside the roll is the kat's, not a tick.
    let mut g = Game::new(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    );
    g.press_at(0, 3000.0, None);
    assert_eq!(g.state[kat + 1].judged, Some(Judge::Great));
}

#[test]
fn a_barline_slower_than_every_note_sizes_the_lookahead() {
    // An anti-barline wall / scenery bar scrolling a tenth of the notes' speed.
    let mut map = beatmap::calibration_map(500.0, 0, 64);
    let slow = map.notes[0].velocity / 10.0;
    map.barlines.push(beatmap::Barline {
        time: 40_000.0,
        velocity: slow,
    });
    let g = Game::new(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    );
    assert_eq!(g.view.min_velocity, slow);

    // A fake slider at SV 0.02 (120 BPM): ~0.008, under the old 0.01 floor, so it was looked
    // for only once it was already well onto the lane.
    let mut map = beatmap::calibration_map(500.0, 0, 64);
    let crawl = map.notes[0].velocity * 0.02;
    map.notes[10].velocity = crawl;
    let g = Game::new(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    );
    assert!(crawl < 0.01);
    assert_eq!(g.view.min_velocity, crawl);
}

#[test]
fn radar_lights_notes_as_the_beam_passes_them() {
    // The beam leaves the drum every other beat and crosses the lane in two, in kiai in one.
    assert_eq!(radar_sweep(0.5, false), 0.25);
    assert_eq!(radar_sweep(3.0, false), 0.5);
    assert_eq!(radar_sweep(4.5, true), 0.5);
    let (len, u) = (600.0, 100.0);
    // A note 300 along lights up as the beam gets there and is gone by the next one.
    assert!(radar_light(300.0, u, len, 0.5) > 0.99);
    assert!(radar_light(300.0, u, len, 0.75) > radar_light(300.0, u, len, 0.95));
    assert!(radar_light(300.0, u, len, 0.499) < 1e-3);
    // By the drum it's always lit.
    assert_eq!(radar_light(50.0, u, len, 0.9), 1.0);
    // One that came on screen after the beam went by stays dark till the next one.
    assert_eq!(radar_age(590.0, u, len, 0.3), None);
    assert_eq!(radar_light(590.0, u, len, 0.3), 0.0);
    // Hidden decides when far notes show too: picking it switches Radar off.
    assert!(!Mods::parse("RA,HD").has(Mod::Radar));
}

#[test]
fn echo_shows_the_next_bar_in_colour_and_the_lane_in_grey() {
    let mut map = beatmap::calibration_map(500.0, 0, 64);
    for (k, n) in map.notes.iter_mut().enumerate() {
        if k % 3 == 1 {
            n.kind = Kind::Kat;
        }
    }
    let mut g = Game::with_mods(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
        Mods::parse("EC"),
    );
    // Bars are 2 s: in the one from 2 s, the one from 4 s shows; before the first barline,
    // the first bar.
    assert_eq!(
        g.echo_bars(2500.0),
        Some([(2000.0, 2000.0), (4000.0, 2000.0)])
    );
    assert_eq!(
        g.echo_bars(-500.0),
        Some([(-2000.0, 2000.0), (0.0, 2000.0)])
    );
    g.time = 2500.0;
    let mut c = Canvas::default();
    c.clear(853.3);
    g.draw_playfield(&mut c, 1.0);
    let fill = |s: &Shape| {
        (s.fill[0] * 255.0).round() as u32 * 65536
            + (s.fill[1] * 255.0).round() as u32 * 256
            + (s.fill[2] * 255.0).round() as u32
    };
    let notes: Vec<u32> = c
        .shapes
        .iter()
        .filter(|s| s.center[1] == LANE_Y && s.half[0] == NOTE_R && s.center[0] > HIT_X + 40.0)
        .map(fill)
        .collect();
    assert!(
        !notes.is_empty() && notes.iter().all(|&f| f == GREY),
        "{notes:x?}"
    );
    let dots: std::collections::HashSet<u32> = c
        .shapes
        .iter()
        .filter(|s| s.center[1] == ECHO_Y && s.half[0] < 10.0 && s.half[0] > 3.0)
        .map(fill)
        .collect();
    assert!(dots.len() >= 2 && !dots.contains(&GREY), "{dots:x?}");
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
fn every_mod_draws_a_sane_playfield_and_judges_the_same() {
    let all: Vec<&str> = crate::mods::ALL.iter().map(|m| m.acronym()).collect();
    let mut sets: Vec<String> = all.iter().map(|a| a.to_string()).collect();
    // And every one at once, as far as conflicts let them.
    sets.push(all.join(","));
    for mods in &sets {
        let mut g = kiai_game(mods);
        let mut c = Canvas::default();
        for k in 0..400 {
            g.time = -2000.0 + k as f64 * 87.3;
            g.press_at(k % 4, g.time, None);
            c.clear(853.3);
            g.draw_playfield(&mut c, 1.0);
            for sh in &c.shapes {
                let v = [sh.center[0], sh.center[1], sh.half[0], sh.half[1], sh.angle];
                assert!(
                    v.iter().all(|x| x.is_finite()),
                    "{mods} at {}: {v:?}",
                    g.time
                );
            }
        }
        // Judging doesn't care how notes are drawn.
        if ["AT", "HA", "SW", "RD"].contains(&mods.as_str()) || mods.contains(',') {
            continue;
        }
        let mut g = kiai_game(mods);
        play_all(&mut g, 0.0);
        assert_eq!(g.stats.great, 64, "{mods}");
    }
}

#[test]
fn baked_speeds_bend_the_maps_own() {
    let v = |mods: &str| {
        kiai_game(mods)
            .map
            .notes
            .iter()
            .map(|n| n.velocity)
            .collect::<Vec<_>>()
    };
    let plain = v("");
    // One speed throughout: Anti-SV and Hyper SV leave it, Crescendo ramps it up.
    assert_eq!(v("AS"), plain);
    assert_eq!(v("HS"), plain);
    let c = v("CC");
    assert!(c[0] < plain[0] && c[63] > plain[63] && c.windows(2).all(|p| p[1] >= p[0]));
    // Leapfrog alternates; Drop speeds up the kiai.
    let l = v("LF");
    assert!(l[0] > plain[0] && l[1] < plain[1]);
    let d = v("DR");
    assert!(d[0] == plain[0] && d[63] > plain[63]);
    // Roulette changes every 4 bars, and never to the same speed.
    let g = kiai_game("RL");
    assert!(
        g.view.sections.len() > 2
            && g.view.sections[0].1 == 1.0
            && g.view.sections.windows(2).all(|p| p[0].1 != p[1].1)
    );
    // Even: evenly spaced notes keep their speed.
    assert_eq!(v("EV"), plain);
}

#[test]
fn drop_freezes_far_notes_and_releases_them_by_the_kiai() {
    let g = kiai_game("DR");
    let cp = g.map.control_points[0];
    // Held from two beats before (8000 - 1000) for a beat and a half, back to 0 at the kiai.
    assert_eq!(g.extra_warp(6900.0, &cp, g.mods), 0.0);
    assert!((g.extra_warp(7500.0, &cp, g.mods) - 500.0).abs() < 1e-9);
    assert!(g.extra_warp(7999.0, &cp, g.mods) < 1.0);
    assert_eq!(g.extra_warp(8100.0, &cp, g.mods), 0.0);
}

#[test]
fn flip_flop_turns_over_every_8_bars() {
    let g = kiai_game("FF");
    let bars = &g.map.barlines;
    assert_eq!(flip_flop(bars, bars[3].time), 1.0);
    assert_eq!(flip_flop(bars, bars[9].time), -1.0);
    assert!(flip_flop(bars, bars[8].time + 175.0).abs() < 1e-3);
}

#[test]
fn multiverse_universes_draw_with_their_own_mods_only_while_drawn() {
    multiverse::set(multiverse::defaults());
    for t in [2000.0, 3600.0, 9000.0] {
        let mut g = kiai_game("MV,HD");
        g.set_time(t);
        let mut c = Canvas {
            width: 853.3,
            ..Default::default()
        };
        g.draw_playfield(&mut c, 1.0);
        assert_eq!(g.mods, Mods::parse("MV,HD"));
        assert!(!c.shapes.is_empty());
    }
    // Sudden in a universe would hide every note with the play's Hidden: it's left out.
    assert_eq!(
        Mods::parse("HD").plus(Mods::parse("SU,SK")),
        Mods::parse("HD,SK")
    );
}

#[test]
fn a_crawling_note_at_the_end_of_the_map_is_looked_for_from_the_start() {
    // Ten minutes out, slow enough to be on the lane already: no lookahead cap hides it.
    let mut map = beatmap::calibration_map(500.0, 0, 64);
    let last = map.notes.len() - 1;
    map.notes[last].time = 600_000.0;
    map.notes[last].end_time = 600_000.0;
    map.notes[last].velocity = 1e-4;
    let mut g = Game::new(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    );
    g.set_time(1000.0);
    g.draw_playfield(
        &mut Canvas {
            width: 853.3,
            ..Default::default()
        },
        1.0,
    );
    assert!(g.view.visible.contains(&last));
}

#[test]
fn a_faster_universe_shows_its_notes_further_out() {
    let mut g = kiai_game("");
    g.set_time(1000.0);
    let mut far = Vec::new();
    for speed in [2, 4] {
        let uni = Universe {
            speed,
            ..Universe::CLASSIC
        };
        let mut c = Canvas {
            width: 853.3,
            ..Default::default()
        };
        g.draw_field(&mut c, 1.0, Focus::FULL, &uni, g.mods);
        far.push(g.view.visible.len());
    }
    // Faster notes cross the lane sooner: fewer of the map's are on it at once.
    assert!(far[1] < far[0], "notes on the lane at 1x and 1.5x: {far:?}");
}

#[test]
fn multiverse_calls_the_next_playfield_and_moves_the_focus_on_bars() {
    let g = kiai_game("MV");
    // 2 s bars, kiai from 8 s: every 2 bars before it, every bar in it.
    let times: Vec<f64> = g.view.focus_switches.iter().map(|s| s.at).collect();
    let want: Vec<f64> = std::iter::once(4000.0)
        .chain((4..16).map(|i| i as f64 * 2000.0))
        .collect();
    assert_eq!(times, want);
    let mut focus = FIRST_FOCUS;
    for s in &g.view.focus_switches {
        // Well above or below, so neither hides the other.
        assert!(s.field < FIELD_COUNT && (parked(s.field).1 - parked(focus).1).abs() >= 80.0);
        focus = s.field;
    }
    let next = g.view.focus_switches[0].field;
    let other = (0..FIELD_COUNT)
        .find(|&f| f != next && f != FIRST_FOCUS)
        .unwrap();
    let at = |t: f64| g.multiverse_at(t);
    let calm = at(2000.0);
    assert!(calm[FIRST_FOCUS].focus == 1.0 && calm[FIRST_FOCUS].settle == 1.0);
    assert!(calm[next].settle == 0.0 && calm[next].aim == 0.0);
    // Called over the two beats before: it parks, lights up and pings on each beat.
    let called = at(3510.0);
    assert!(called[next].aim == 1.0 && called[next].call > 0.9 && called[next].settle > 0.5);
    assert!(called[other].aim == 0.0 && called[other].settle == 0.0);
    assert!(at(3625.0)[next].settle == 1.0);
    // Its notes are in colour a beat before the focus lands; the one in focus keeps its own.
    assert!(
        at(2990.0)[next].colour == 0.0
            && at(3500.0)[next].colour == 1.0
            && at(3500.0)[next].focus == 0.0
    );
    assert!(at(3600.0)[FIRST_FOCUS].colour == 1.0);
    // The focus slides across over the last 375 ms and lands on the bar.
    let mid = at(3812.5);
    assert!((mid[next].focus - 0.5).abs() < 1e-6 && (mid[FIRST_FOCUS].focus - 0.5).abs() < 1e-6);
    let done = at(4000.0);
    assert!(done[next].focus == 1.0 && done[next].aim == 0.0 && done[next].burst == Some(0.0));
    assert!(done[FIRST_FOCUS].focus == 0.0 && done[FIRST_FOCUS].settle == 0.0);
    // It overshoots its size on the way in and settles.
    assert!(at(3950.0)[next].grow > 1.0 && (done[next].grow - 1.0).abs() < 1e-6);
}

/// `TAIKO_MOD_SHOTS=<dir>`: a few frames of every mod as PNGs (drawn on the CPU), to look at.
#[test]
fn mod_shots() {
    let Some(dir) = std::env::var_os("TAIKO_MOD_SHOTS") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    for m in crate::mods::ALL {
        let mut g = kiai_game(m.acronym());
        let mut c = Canvas::default();
        for t in [5000.0, 5200.0, 7400.0, 8300.0, 18_100.0] {
            g.set_time(t - 400.0);
            g.press_at(1, t - 400.0, None);
            g.time = t;
            c.clear(853.3);
            g.draw_playfield(&mut c, 1.0);
            let (w, h, rgb) = crate::debug::rasterize(&c, 1.0);
            crate::debug::write_png(&dir.join(format!("{}-{t}.png", m.acronym())), w, h, &rgb)
                .unwrap();
        }
        // Streams and triplets, for the mods that go by rhythm.
        if ["SK", "PY", "LI", "DY", "SH", "SR", "CR"].contains(&m.acronym()) {
            let mut g = rhythm_game(m.acronym());
            for t in [2000.0, 5600.0, 7400.0] {
                // Every note so far hit on time (Crumble breaks the lane by them).
                let times: Vec<f64> = g
                    .map
                    .notes
                    .iter()
                    .map(|n| n.time)
                    .take_while(|&n| n < t)
                    .collect();
                for n in times {
                    g.press_at(1, n, None);
                }
                g.time = t;
                c.clear(853.3);
                g.draw_playfield(&mut c, 1.0);
                let (w, h, rgb) = crate::debug::rasterize(&c, 1.0);
                crate::debug::write_png(
                    &dir.join(format!("{}-rhythm-{t}.png", m.acronym())),
                    w,
                    h,
                    &rgb,
                )
                .unwrap();
            }
        }
        if let Some(&at) = g.scares.first() {
            g.time = at + 300.0;
            c.clear(853.3);
            g.draw_playfield(&mut c, 1.0);
            g.draw_hud(&mut c);
            let (w, h, rgb) = crate::debug::rasterize(&c, 1.0);
            crate::debug::write_png(&dir.join("scare.png"), w, h, &rgb).unwrap();
        }
    }
    // Defense: a team of three on the Maze, 2-bar turns, ours second (4-8 s), on dons and
    // kats in turn. auneu's don tower shoots through our turn, quass spots for us and builds
    // a kat tower after it; we slow the lane.
    let mut map = beatmap::calibration_map(500.0, 0, 64);
    for (k, n) in map.notes.iter_mut().enumerate() {
        if k % 2 == 1 {
            n.kind = Kind::Kat;
        }
    }
    let mut g = Game::with_mods(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
        Mods::parse("MZ"),
    );
    g.set_defense(
        vec![(1, "auneu".into()), (2, "you".into()), (3, "quass".into())],
        1,
        Defense {
            lives: 10,
            bars: 2,
            ..Defense::DEFAULT
        },
    );
    g.defense_act(
        1,
        DefenseAct::Buy {
            item: DefenseItem::DonTower,
            at: 2000.0,
            until: 0.0,
        },
    );
    g.defense_act(
        2,
        DefenseAct::Buy {
            item: DefenseItem::Slow,
            at: 5000.0,
            until: 0.0,
        },
    );
    g.defense_act(
        3,
        DefenseAct::Buy {
            item: DefenseItem::KatTower,
            at: 9000.0,
            until: 0.0,
        },
    );
    g.set_team(TeamState {
        lives: 9,
        max: 10,
        gold: 75,
        shield: 3,
        towers: 2,
        heals: 0,
        shields: 0,
        tower_buys: 2,
    });
    g.view.banner = Some("WATCHING  -  quassbottle".into());
    let mut c = Canvas::default();
    for lang in [crate::lang::Lang::En, crate::lang::Lang::Ru] {
        crate::lang::set(lang);
        for t in [3200.0, 4700.0, 6500.0, 11_200.0] {
            g.set_time(t - 400.0);
            g.press_at(1, t - 400.0, None);
            g.time = t;
            c.clear(853.3);
            g.draw_playfield(&mut c, 1.0);
            g.draw_hud(&mut c);
            let (w, h, rgb) = crate::debug::rasterize(&c, 1.0);
            let name = format!("defense-{}-{t}.png", lang.code());
            crate::debug::write_png(&dir.join(name), w, h, &rgb).unwrap();
            // The text of the HUD alone, audited: nothing off screen, cut off or over other text.
            for width in [640.0, 853.3, 1100.0] {
                let mut a = Canvas {
                    audit: true,
                    ..Canvas::default()
                };
                a.clear(width);
                g.draw_hud(&mut a);
                for p in crate::debug::layout_problems(&a) {
                    eprintln!("[defense {} {t} @{width}] {p}", lang.code());
                }
            }
        }
    }
    crate::lang::set(crate::lang::Lang::En);
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
fn the_maze_fits_between_the_hud_and_the_meter_and_never_runs_alongside_itself() {
    let shape = RailShape::Maze;
    // The last stretch into the drum reads like an ordinary lane: straight, level, and
    // nothing else of the maze beside it there (the towers stand on either side).
    let rail = Rail::build(|s| shape.bend(s), shape.length(853.3));
    for s in 0..500 {
        let (_, y, a) = rail.at(s as f32);
        assert!(y == LANE_Y && a == 0.0, "at {s}: {y}, {a}");
    }
    let past_first_corner = maze_straight(0) + std::f32::consts::PI / 2.0 * MAZE_R;
    for k in (0..rail.pts.len()).filter(|&k| k as f32 * RAIL_STEP > past_first_corner) {
        let (x, y, _) = rail.pts[k];
        let beside = x < HIT_X + 500.0 && (y - LANE_Y).abs() < MAZE_CELL - 1.0;
        assert!(
            !beside,
            "the maze comes back beside the straight at {x}, {y}"
        );
    }
    for w in [853.3, 640.0] {
        let (len, zoom, drop) = (shape.length(w), shape.zoom(w), shape.drop());
        let rail = Rail::build(|s| shape.bend(s), len);
        let at = |s: f32, side: f32| {
            let (x, y, a) = rail.at(s);
            let (x, y) = (x - a.sin() * side, y - a.cos() * side);
            (
                HIT_X + zoom * (x - HIT_X),
                LANE_Y + drop + zoom * (y - LANE_Y),
            )
        };
        // Up to the corner where it heads off the right of the screen.
        let body = len - MAZE_EXIT - std::f32::consts::PI / 2.0 * MAZE_R;
        let mut s = 0.0;
        while s < body {
            for side in [-LANE_H / 2.0, 0.0, LANE_H / 2.0] {
                let (x, y) = at(s, side);
                assert!(x < w - 4.0, "{w}: off the right at {s} ({x}, {y})");
                // Under the Defense panel (to 42), over the hit error meter (from 415).
                assert!((42.0..415.0).contains(&y), "{w}: at {s} ({x}, {y})");
                if w > 800.0 {
                    // Clear of the score, accuracy and HP in the top right.
                    let hud = x > hp_bar_x(w) - 4.0 && y < HP_BAR_Y + 8.0;
                    assert!(!hud, "under the score at {s} ({x}, {y})");
                }
            }
            s += 2.0;
        }
        // Bits of the lane far apart along it are at least a lane and a gap apart on screen.
        let pts: Vec<(f32, f32, f32)> = (0..(body / 8.0) as usize)
            .map(|k| k as f32 * 8.0)
            .map(|s| (s, rail.at(s).0, rail.at(s).1))
            .collect();
        for (k, a) in pts.iter().enumerate() {
            for b in &pts[k + 1..] {
                if b.0 - a.0 > 2.0 * MAZE_CELL {
                    let d = (a.1 - b.1).hypot(a.2 - b.2);
                    assert!(d >= LANE_H + 20.0, "{} and {} are {d} apart", a.0, b.0);
                }
            }
        }
    }
}

#[test]
fn defense_takes_turns_by_bars_and_fails_the_team_when_the_lives_run_out() {
    // A don every 500 ms, a bar every 2 s: with 4-bar turns, notes 0-15 are the first
    // player's and 16-31 ours.
    let map = beatmap::calibration_map(500.0, 0, 64);
    let mut g = Game::new(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    );
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

/// A Defense game of the calibration map (a don every 500 ms, a bar every 2 s), 4-bar turns,
/// with us second of `team`.
fn defense_game(team: usize) -> Game {
    let map = beatmap::calibration_map(500.0, 0, 64);
    let mut g = Game::new(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    );
    let team = (0..team as u32).map(|k| (k + 1, format!("p{k}"))).collect();
    g.set_defense(
        team,
        1,
        Defense {
            lives: 25,
            bars: 4,
            ..Defense::DEFAULT
        },
    );
    g
}

/// A Defense team of two playing one mod: everyone gets the host's mods, so both games have
/// to agree on the notes and whose they are, and between them take every one.
fn defense_with(mods: Mods, me: usize) -> Game {
    let map = beatmap::calibration_map(500.0, 0, 64);
    let track = Arc::new(Track {
        samples: Vec::new(),
        rate: 1.0,
        stream: None,
    });
    let mut g = Game::with_seed(map, track, None, mods, 7);
    g.set_defense(
        vec![(1, "a".into()), (2, "b".into())],
        me,
        Defense {
            lives: 25,
            bars: 4,
            ..Defense::DEFAULT
        },
    );
    for act in [
        DefenseAct::Buy {
            item: DefenseItem::DonTower,
            at: 1000.0,
            until: 0.0,
        },
        DefenseAct::Buy {
            item: DefenseItem::Slow,
            at: 9000.0,
            until: 0.0,
        },
    ] {
        g.defense_act(2, act);
    }
    g.defense_act(1, DefenseAct::TagIn { at: 8500.0 });
    g
}

#[test]
fn every_mod_a_defense_team_can_have_plays_the_same_notes_for_both_players() {
    draw_with_defaults();
    let mut bad: Vec<String> = Vec::new();
    for m in crate::mods::ALL.into_iter().filter(|m| m.defense_ok()) {
        let mods = Mods::parse(m.acronym());
        let (mut a, mut b) = (defense_with(mods, 0), defense_with(mods, 1));
        let same = a.map.notes.len() == b.map.notes.len()
            && a.map
                .notes
                .iter()
                .zip(&b.map.notes)
                .all(|(x, y)| x.time == y.time && x.kind == y.kind);
        if !same {
            bad.push(format!("{}: the notes differ", m.acronym()));
            continue;
        }
        let (da, db) = (a.defense.as_ref().unwrap(), b.defense.as_ref().unwrap());
        if da.owner != db.owner {
            bad.push(format!("{}: the turns differ", m.acronym()));
        }
        // Between them they take every note that can be hit, once.
        for i in 0..a.map.notes.len() {
            if !matches!(a.map.notes[i].kind, Kind::Don | Kind::Kat) || da.map_unhittable[i] {
                continue;
            }
            let (ha, hb) = (!a.map.notes[i].unhittable, !b.map.notes[i].unhittable);
            let shot = da.shot[i].is_some();
            if shot && (ha || hb) || !shot && ha == hb {
                bad.push(format!("{}: note {i} is taken by {ha} {hb}", m.acronym()));
                break;
            }
        }
        // Autoplay them both through the map: nothing panics, nothing of theirs gets through.
        for g in [&mut a, &mut b] {
            let mut t = -2000.0;
            while t < 34_000.0 {
                g.time = t;
                g.autoplay(None);
                g.expire(t);
                if (t as i64) % 4000 == 0 {
                    let mut c = Canvas::default();
                    c.clear(853.3);
                    g.draw_playfield(&mut c, 1.0);
                    g.draw_hud(&mut c);
                }
                t += 20.0;
            }
            if g.stats.miss > 0 {
                bad.push(format!(
                    "{}: {} misses under autoplay",
                    m.acronym(),
                    g.stats.miss
                ));
                break;
            }
            if g.stats.great + g.stats.ok < 8 {
                bad.push(format!(
                    "{}: only {} hits under autoplay",
                    m.acronym(),
                    g.stats.great + g.stats.ok
                ));
                break;
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
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
fn the_camera_zooms_in_on_our_turn_and_back_out() {
    // Us second of two, 4-bar turns: bars 4-7 (8-16 s) are ours.
    let g = defense_game(2);
    let f = |t: f64| g.defense_focus(t);
    assert_eq!((f(1000.0), f(6500.0)), (0.0, 0.0));
    assert_eq!(
        f(7500.0),
        0.5,
        "half way in over the second half of the bar before"
    );
    assert_eq!((f(8000.0), f(12_000.0), f(15_900.0)), (1.0, 1.0, 1.0));
    assert_eq!(
        f(16_500.0),
        0.5,
        "and out over the first half of the one after"
    );
    assert_eq!(f(17_000.0), 0.0);
}

#[test]
fn tag_ins_run_out_for_the_map() {
    // Three players, 4-bar turns, one tag-in each: p2 takes over p0's first turn, but not
    // its next one (bars 12-15).
    let map = beatmap::calibration_map(500.0, 0, 64);
    let mut g = Game::new(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    );
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
    let mut g = Game::new(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
    );
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
fn slows_halve_the_lane_for_a_while() {
    let mut g = defense_game(2);
    assert_eq!(g.slow_factor(3000.0), 1.0);
    g.defense_act(
        2,
        DefenseAct::Buy {
            item: DefenseItem::Slow,
            at: 2000.0,
            until: 0.0,
        },
    );
    assert_eq!(g.slow_factor(1900.0), 1.0);
    assert_eq!(g.slow_factor(2200.0), 0.75);
    assert_eq!(g.slow_factor(5000.0), 0.5);
    assert_eq!(g.slow_factor(2000.0 + SLOW_MS + 1.0), 1.0);
}

#[test]
fn the_loop_is_full_size_on_screen_and_crosses_the_lane_at_an_angle() {
    let shape = RailShape::Loop;
    // As big as the other rails: on 16:9 nothing shrinks.
    assert_eq!(shape.zoom(853.3), 1.0);
    for w in [853.3, 600.0] {
        let (len, zoom, drop) = (shape.length(w), shape.zoom(w), shape.drop());
        let rail = Rail::build(|s| shape.bend(s), len);
        // Where `s` along, `side` off the middle, ends up on screen.
        let at = |s: f32, side: f32| {
            let (x, y, a) = rail.at(s);
            let (x, y) = (x - a.sin() * side, y - a.cos() * side);
            (
                HIT_X + zoom * (x - HIT_X),
                LANE_Y + drop + zoom * (y - LANE_Y),
            )
        };
        let mut s = 0.0;
        while s < len {
            for side in [-62.0, 0.0, 62.0] {
                let (x, y) = at(s, side);
                // The lane in leaves by the bottom right.
                if x > w || y > VIRTUAL_HEIGHT {
                    continue;
                }
                assert!(y > 4.0, "{w}: off the top at {s} ({x}, {y})");
                if w > 800.0 {
                    // Its error ticks reach up to 419.
                    let meter = (x - w / 2.0).abs() < 158.0 && y > 415.0;
                    assert!(!meter, "over the hit error meter at {s} ({x}, {y})");
                }
            }
            s += 2.0;
        }
        // The lane into the loop comes up across the straight, not along it.
        let after = rail.at(LOOP_AT + LOOP_TURN * LOOP_R + 10.0).2;
        assert!((after - LOOP_TURN).abs() < 0.05, "{w}: heading {after}");
    }
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
fn rails_bend_the_lane_without_speeding_notes_up() {
    // Along the lane, notes go as on a plain one (as many to a stretch of it), whatever the
    // shape, its length, and how much the playfield shrinks to fit it.
    for w in [853.3, 660.0, 600.0] {
        for mods in ["ZZ", "UT", "SI", "LP", "EB", "SS"] {
            let g = kiai_game(mods);
            for t in [0.0, 20_000.0, 50_000.0] {
                let (rail, len, _, zoom) = g.rail_at(t, w, None, g.mods);
                assert!(zoom <= 1.0);
                assert_eq!(lane_stretch(rail.is_some(), len, w), 1.0, "{mods} at {w}");
            }
        }
        // Plain lanes are as they were.
        assert_eq!(lane_stretch(false, w - HIT_X, w), 1.0);
    }
}

#[test]
fn a_tko_lane_clip_bends_the_lane_over_the_players_own_and_keeps_note_speed() {
    let w = 853.3;
    let plain = game();
    assert!(plain.rail_at(0.0, w, None, plain.mods).0.is_none());
    for k in 0..LANE_SHAPES_OF_CLIPS.len() {
        for u in [0.25, 0.5, 1.0] {
            let (rail, len, _, zoom) = plain.rail_at(0.0, w, Some((k, u)), plain.mods);
            assert!(rail.is_some() && zoom <= 1.0 + 1e-6);
            assert_eq!(lane_stretch(true, len, w), 1.0, "shape {k} at {u}");
        }
    }
    // The map's spiral goes before the player's zigzag.
    let zz = kiai_game("ZZ");
    let theirs = zz.rail_at(0.0, w, None, zz.mods).1;
    let map = zz.rail_at(0.0, w, Some((2, 1.0)), zz.mods).1;
    assert_ne!(theirs, map);
    assert_eq!(map, plain.rail_at(0.0, w, Some((2, 1.0)), plain.mods).1);
}

#[test]
fn portal_keeps_the_drum_and_jumps_at_the_entry() {
    let p = (400.0, 150.0);
    assert_eq!(through_portal(0.0, p).0, 0.0);
    assert_eq!(through_portal(-30.0, p).0, -30.0);
    assert_eq!(through_portal(500.0, p), (500.0, 1.0));
    // Just past the entry it is at the exit, small; halfway in, halfway to the drum.
    let (s, size) = through_portal(399.9, p);
    assert!((s - 150.0).abs() < 0.1 && size < 0.2);
    assert!((through_portal(200.0, p).0 - 75.0).abs() < 1e-3);
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
fn a_skins_overtime_frames_play_through_the_scare_instead_of_the_bear() {
    use crate::render::SPRITE_BASE;
    let drawn = |c: &Canvas| -> Vec<usize> {
        c.shapes
            .iter()
            .filter(|s| s.params[3] >= SPRITE_BASE)
            .map(|s| (s.params[3] - SPRITE_BASE) as usize)
            .collect()
    };
    let mut c = Canvas::default();
    draw_scare(&mut c, 0.5, 0.0);
    assert!(
        drawn(&c).is_empty() && c.shapes.len() > 10,
        "the built-in bear"
    );
    let mut c = Canvas::default();
    c.skin_layout.overtime_frames = 3;
    c.skin_layout.overtime_frame_ms = 100.0;
    for i in 0..3 {
        c.sprites[Sprite::Overtime0 as usize + i] = Some([4.0, 3.0]);
    }
    // 0.5 of 900 ms in: frame 4 of a 100 ms loop of three, the second.
    draw_scare(&mut c, 0.5, 0.0);
    assert_eq!(drawn(&c), vec![Sprite::Overtime1 as usize]);
    assert_eq!(c.shapes.len(), 2, "the backdrop and the picture");
    let pic = &c.shapes[1];
    assert!((pic.half[0] / pic.half[1] - 4.0 / 3.0).abs() < 1e-3);
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
            .filter(|e| matches!(e.1, Sfx::Scream))
            .count()
    };
    assert_eq!(screams(&g), 1);
    // Jumping past one leaves it quiet.
    let mut g = kiai_game("OT");
    g.sfx_log = Some(Vec::new());
    g.set_time(at + 5000.0);
    assert_eq!(screams(&g), 0);
}
/// 1/4s from 0 ms, triplets from 2880 ms and 1/6s from 6720 ms, at 125 BPM.
fn rhythm_game(mods: &str) -> Game {
    let mut map = beatmap::calibration_map(480.0, 0, 64);
    for (k, n) in map.notes.iter_mut().enumerate() {
        n.time = match k {
            0..24 => k as f64 * 120.0,
            24..48 => 2880.0 + (k - 24) as f64 * 160.0,
            _ => 6720.0 + (k - 48) as f64 * 80.0,
        };
        n.end_time = n.time;
    }
    Game::with_mods(
        map,
        Arc::new(Track {
            samples: Vec::new(),
            rate: 1.0,
            stream: None,
        }),
        None,
        Mods::parse(mods),
    )
}

#[test]
fn snaps_and_liar() {
    let g = rhythm_game("LI");
    let snaps: Vec<u8> = g
        .map
        .notes
        .iter()
        .map(|n| snap_of(&g.map.control_points, n.time))
        .collect();
    assert_eq!(&snaps[..4], &[1, 4, 2, 4]);
    assert_eq!(&snaps[24..27], &[1, 3, 3]);
    assert_eq!(&snaps[48..51], &[1, 6, 3]);
    // A 1/3 pretends to be the 1/4 next to it; 1/4s and notes on the beat tell the truth.
    assert_eq!(g.view.lies[25], 3000.0);
    assert_eq!(g.view.lies[1], g.map.notes[1].time);
    assert_eq!(g.view.lies[24], g.map.notes[24].time);
    // Snap Speed: triplets slower, 1/6s faster.
    let v = rhythm_game("SR")
        .map
        .notes
        .iter()
        .map(|n| n.velocity)
        .collect::<Vec<_>>();
    assert!(v[25] < v[1] && v[49] > v[1] && v[0] == v[1] && v[26] == v[25]);
    assert_eq!(
        (
            g.flows[0],
            g.flows[2],
            g.flows[25],
            g.flows[26],
            g.flows[50]
        ),
        (4, 4, 3, 3, 6)
    );
}

#[test]
fn decoys_get_into_streams_but_never_twice_running() {
    let g = rhythm_game("DY");
    let in_stream: Vec<f64> = g
        .view
        .decoys
        .iter()
        .map(|n| n.time)
        .filter(|&t| t > 6720.0)
        .collect();
    assert!(
        !in_stream.is_empty() && in_stream.len() <= 5,
        "{in_stream:?}"
    );
    assert!(
        in_stream.windows(2).all(|p| p[1] - p[0] > 100.0),
        "{in_stream:?}"
    );
    assert!(
        g.view
            .decoys
            .iter()
            .filter(|n| n.time > 6720.0)
            .all(|n| !n.big)
    );
}

#[test]
fn a_tiny_positive_roll_is_gone_at_the_drum() {
    // Lunatic Rough Party!!'s first object: drawn just before 1000, not after.
    let osu = "osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nSliderMultiplier:1.4\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n\n[HitObjects]\n136,208,1000,6,2,L|224:200,1,0.0001\n";
    let map = beatmap::tests::parse(osu);
    assert!(map.notes[0].tiny && map.notes[0].fake_len == 0.0);
    let shapes = |map: beatmap::Beatmap, t: f64| {
        let mut g = Game::new(
            map,
            Arc::new(Track {
                samples: Vec::new(),
                rate: 1.0,
                stream: None,
            }),
            None,
        );
        g.time = t;
        let mut c = Canvas::default();
        c.clear(853.3);
        g.draw_playfield(&mut c, 1.0);
        c.shapes.len()
    };
    let none = |t| {
        shapes(
            beatmap::tests::parse(&osu.replace("136,208,1000,6,2,L|224:200,1,0.0001\n", "")),
            t,
        )
    };
    assert!(shapes(map.clone(), 990.0) > none(990.0));
    assert_eq!(shapes(map, 1010.0), none(1010.0));
}

#[test]
fn a_fake_slider_stops_200_of_length_after_its_cap_lands() {
    // 120 BPM, SM 1.4, 3x: -200 is 238 ms of cap ahead of its head at 2000, -100 half that.
    let osu = "osu file format v14\n\n[General]\nMode: 1\n\n[Difficulty]\nSliderMultiplier:1.4\n\n[TimingPoints]\n0,500,4,1,0,100,1,0\n0,-33.3333333333333,4,1,0,100,0,0\n\n[HitObjects]\n256,192,2000,2,0,L|356:192,1,-200\n256,192,5000,2,0,L|356:192,1,-100\n";
    let map = beatmap::tests::parse(osu);
    let (long, short) = (&map.notes[0], &map.notes[1]);
    assert!((long.fake_len - 238.095).abs() < 0.01);
    // -200's cap lands at ~1762 and it stops 238 ms on: at its head's time, its body's
    // left end just at the drum (owner's screenshot).
    assert_eq!(fake_freeze(long, 2000.0), 0.0);
    assert!((fake_freeze(long, 2100.0) - 100.0).abs() < 0.01);
    // -100 lands at ~4881 and stops at the same ~238 ms on: 119 ms past its head's time, its
    // body's right end at the drum.
    assert!((fake_freeze(short, 5119.05 + 50.0) - 50.0).abs() < 0.05);
}

#[test]
fn a_new_speed_carries_on_from_the_same_place_in_the_map() {
    let map = crate::beatmap::calibration_map(500.0, 4, 64);
    let track = |rate| {
        Arc::new(Track {
            samples: Vec::new(),
            rate,
            stream: None,
        })
    };
    let mut g = Game::new(map, track(1.0), None);
    g.seek(4000.0, None, 0.0);
    g.held = true;
    g.change_speed(track(2.0), 2.0, None, 0.0);
    assert_eq!(
        g.raw_time, 2000.0,
        "the track's own time, at twice the speed"
    );
    g.update(Instant::now(), None, 0.0);
    assert_eq!(g.time, 4000.0);
    assert_eq!(g.rate, 1.0, "what the play was, for its results");
}

/// A calibration map with a `.tko` gimmick: rotation and scroll on the field, and the notes at
/// `grouped` hidden and shrunk.
fn tko_game(gimmick: Option<crate::tko::gimmick::Gimmick>, mods: Mods) -> Game {
    let mut map = beatmap::calibration_map(500.0, 0, 64);
    map.gimmick = gimmick.map(Arc::new);
    let track = Arc::new(Track {
        samples: Vec::new(),
        rate: 1.0,
        stream: None,
    });
    Game::with_seed(map, track, None, mods, 42)
}

fn some_gimmick() -> crate::tko::gimmick::Gimmick {
    use crate::tko::gimmick::{FieldProp, Group, NoteProp};
    use crate::tko::track::{Ease, Key};
    let key = |t, v| Key {
        t,
        v,
        ease: Ease::CubicInOut,
    };
    let mut g = crate::tko::gimmick::Gimmick::default();
    g.key_field(FieldProp::Rotation, key(0.0, 0.0));
    g.key_field(FieldProp::Rotation, key(20_000.0, 360.0));
    g.key_field(FieldProp::Zoom, key(0.0, 1.0));
    g.key_field(FieldProp::Zoom, key(10_000.0, 0.6));
    g.key_field(FieldProp::X, key(0.0, 0.0));
    g.key_field(FieldProp::X, key(10_000.0, 40.0));
    g.key_field(FieldProp::Scroll, key(0.0, 1.0));
    g.key_field(FieldProp::Scroll, key(10_000.0, 2.5));
    g.groups.push(Group {
        name: "every other".into(),
        notes: (0..64)
            .step_by(2)
            .map(|b| b as f64 * 500.0 - beatmap::OSU_TIMING_MS)
            .collect(),
        ..Group::default()
    });
    g.key_note(0, NoteProp::Scale, key(0.0, 0.5));
    g.key_note(0, NoteProp::Dy, key(0.0, 0.0));
    g.key_note(0, NoteProp::Dy, key(10_000.0, 60.0));
    g.key_note(0, NoteProp::Opacity, key(0.0, 0.5));
    g
}

/// These draws are compared: the default theme and look, without animations, whatever
/// other tests' frames are drawn with.
fn draw_with_defaults() {
    crate::render::set_frame_theme(Some(crate::render::Theme {
        animations: false,
        ..crate::render::Theme::default()
    }));
    set_frame_look(Some(Look::default()));
    crate::render::set_frame_time(Some(Instant::now()));
}

fn frame_bytes(g: &mut Game, time: f64) -> Vec<u8> {
    g.time = time;
    let mut c = Canvas::default();
    c.clear(853.3);
    g.draw_playfield(&mut c, 1.0);
    bytemuck::cast_slice::<_, u8>(&c.shapes).to_vec()
}

#[test]
fn a_tko_map_without_a_gimmick_draws_exactly_like_its_osu() {
    draw_with_defaults();
    for mods in [Mods::NONE, Mods::parse("TC"), Mods::parse("WV")] {
        let mut plain = tko_game(None, mods);
        let mut empty = tko_game(Some(Default::default()), mods);
        for time in [0.0, 1500.0, 9000.0, 15_000.0] {
            assert!(
                frame_bytes(&mut plain, time) == frame_bytes(&mut empty, time),
                "mods={mods:?} time={time}"
            );
        }
    }
}

#[test]
fn a_gimmick_changes_what_is_drawn() {
    draw_with_defaults();
    let mut plain = tko_game(None, Mods::NONE);
    let mut moved = tko_game(Some(some_gimmick()), Mods::NONE);
    assert!(frame_bytes(&mut plain, 9000.0) != frame_bytes(&mut moved, 9000.0));
    // Before its keys have done anything the first frame is the same.
    let a = frame_bytes(&mut plain, 0.0).len();
    let b = frame_bytes(&mut moved, 0.0).len();
    assert!(a > 0 && b > 0);
}

#[test]
fn a_maps_text_and_pictures_are_drawn_while_their_clips_last() {
    use crate::tko::clips::{Clip, Show, Target};
    draw_with_defaults();
    let mut g = crate::tko::gimmick::Gimmick::default();
    let mut text = Clip::new(Target::Layer, 2000.0, 6000.0);
    text.layer.as_deref_mut().unwrap().show = Show::Text {
        text: "HELLO".into(),
    };
    let mut pic = Clip::new(Target::Layer, 2000.0, 6000.0);
    {
        let l = pic.layer.as_deref_mut().unwrap();
        l.show = Show::Picture {
            file: "Logo.png".into(),
        };
        l.behind = true;
    }
    g.clips = vec![text, pic];
    let mut with = tko_game(Some(g), Mods::NONE);
    let mut plain = tko_game(None, Mods::NONE);
    let draw = |game: &mut Game, t: f64, loaded: bool| {
        game.time = t;
        let mut c = Canvas::default();
        c.clear(853.3);
        if loaded {
            c.picture_slots = std::sync::Arc::new([("logo.png".to_string(), (3, 2.0))].into());
        }
        game.draw_playfield(&mut c, 1.0);
        c
    };
    let before = draw(&mut with, 1000.0, false);
    assert!(
        before.pictures_wanted.is_empty(),
        "nothing asked before its start"
    );
    let c = draw(&mut with, 4000.0, false);
    assert_eq!(
        c.pictures_wanted,
        vec!["logo.png".to_string()],
        "the picture is asked for"
    );
    let base = draw(&mut plain, 4000.0, false).shapes.len();
    assert_eq!(c.shapes.len(), base + 5, "a quad per letter of HELLO");
    let c = draw(&mut with, 4000.0, true);
    assert_eq!(c.shapes.len(), base + 6, "and once loaded, the picture");
    // Behind the notes: drawn first.
    assert_eq!(c.shapes[0].params[3], -1003.0);
    assert_eq!(c.shapes[0].half, [36.0, 18.0], "as wide as its shape says");
}

/// The whole point of version 1: the gimmick is only ever drawn. The same hits give the same
/// judgements, score and combo, with or without it.
#[test]
fn a_gimmick_never_changes_judging() {
    draw_with_defaults();
    let run = |gimmick: Option<crate::tko::gimmick::Gimmick>| {
        let mut g = tko_game(gimmick, Mods::NONE);
        let mut t = -2000.0;
        while t < 34_000.0 {
            g.time = t;
            g.autoplay(None);
            g.expire(t);
            if (t as i64) % 2000 == 0 {
                let mut c = Canvas::default();
                c.clear(853.3);
                g.draw_playfield(&mut c, 1.0);
            }
            t += 5.0;
        }
        (
            g.stats.great,
            g.stats.ok,
            g.stats.miss,
            g.stats.combo,
            g.stats.max_combo,
            g.stats.score,
            g.stats.errors.clone(),
        )
    };
    let plain = run(None);
    assert!(plain.0 > 30, "autoplay hits the notes: {plain:?}");
    assert_eq!(run(Some(some_gimmick())), plain);
}

#[test]
fn notes_on_an_approach_path_look_different_and_are_judged_the_same() {
    use crate::tko::gimmick::Group;
    use crate::tko::path::NotePath;
    draw_with_defaults();
    let mut gm = crate::tko::gimmick::Gimmick::default();
    gm.groups.push(Group {
        notes: (0..64)
            .map(|b| b as f64 * 500.0 - beatmap::OSU_TIMING_MS)
            .collect(),
        path: Some(NotePath {
            points: vec![[300.0, -300.0], [60.0, -200.0]],
            show: true,
            ..NotePath::default()
        }),
        ..Group::default()
    });
    let mut plain = tko_game(None, Mods::NONE);
    let mut flown = tko_game(Some(gm.clone()), Mods::NONE);
    assert!(frame_bytes(&mut plain, 9000.0) != frame_bytes(&mut flown, 9000.0));
    let run = |gimmick: Option<crate::tko::gimmick::Gimmick>| {
        let mut g = tko_game(gimmick, Mods::NONE);
        let mut t = -2000.0;
        while t < 20_000.0 {
            g.time = t;
            g.autoplay(None);
            g.expire(t);
            if (t as i64) % 1000 == 0 {
                let mut c = Canvas::default();
                c.clear(853.3);
                g.draw_playfield(&mut c, 1.0);
            }
            t += 5.0;
        }
        (g.stats.great, g.stats.ok, g.stats.miss, g.stats.score)
    };
    let base = run(None);
    assert!(base.0 > 30);
    assert_eq!(run(Some(gm)), base);
}

#[test]
fn a_notes_tko_motion_moves_everything_drawn_for_it_alike() {
    let drum = (HIT_X, LANE_Y);
    let lane = (HIT_X + 300.0, LANE_Y);
    let near = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01;
    let plain = NoteFx::default();
    assert!(near(tko_moved(&plain, lane.0, lane.1, drum), lane));
    // On a path: from the drum, or half way there at half weight.
    let on = NoteFx {
        path: Some([0.0, -100.0, 1.0]),
        ..NoteFx::default()
    };
    assert!(near(
        tko_moved(&on, lane.0, lane.1, drum),
        (HIT_X, LANE_Y - 100.0)
    ));
    let half = NoteFx {
        path: Some([0.0, -100.0, 0.5]),
        ..NoteFx::default()
    };
    assert!(near(
        tko_moved(&half, lane.0, lane.1, drum),
        (HIT_X + 150.0, LANE_Y - 50.0)
    ));
    // Shifted, then turned a quarter about the drum.
    let turned = NoteFx {
        dx: -200.0,
        swirl: 90.0,
        ..NoteFx::default()
    };
    assert!(near(
        tko_moved(&turned, lane.0, lane.1, drum),
        (HIT_X, LANE_Y + 100.0)
    ));
}

/// Every screen effect at once, steady.
fn effects_gimmick() -> crate::tko::gimmick::Gimmick {
    use crate::tko::gimmick::{FieldProp, Group, NoteProp};
    use crate::tko::track::{Ease, Key};
    let key = |v| Key {
        t: 0.0,
        v,
        ease: Ease::Linear,
    };
    let mut g = crate::tko::gimmick::Gimmick::default();
    for (p, v) in [
        (FieldProp::Flash, 0.3),
        (FieldProp::Vignette, 0.5),
        (FieldProp::Wash, 0.4),
        (FieldProp::WashHue, 200.0),
        (FieldProp::Glitch, 0.6),
        (FieldProp::Chroma, 8.0),
        (FieldProp::Echo, 3.0),
        (FieldProp::Kaleido, 3.0),
        (FieldProp::StretchX, 1.3),
        (FieldProp::StretchY, 0.8),
    ] {
        g.key_field(p, key(v));
    }
    g.groups.push(Group {
        notes: (0..64)
            .step_by(2)
            .map(|b| b as f64 * 500.0 - beatmap::OSU_TIMING_MS)
            .collect(),
        ..Group::default()
    });
    g.key_note(0, NoteProp::Swirl, key(40.0));
    g
}

#[test]
fn screen_effects_add_to_the_picture_and_never_change_judging() {
    draw_with_defaults();
    let count = |g: &mut Game, t: f64| {
        g.time = t;
        let mut c = Canvas::default();
        c.clear(853.3);
        g.draw_playfield(&mut c, 1.0);
        c.shapes.len()
    };
    let mut plain = tko_game(None, Mods::NONE);
    let mut wild = tko_game(Some(effects_gimmick()), Mods::NONE);
    let (a, b) = (count(&mut plain, 9000.0), count(&mut wild, 9000.0));
    assert!(
        b > a + 20,
        "copies, slices and overlays were drawn: {a} vs {b}"
    );
    assert!(frame_bytes(&mut plain, 9000.0) != frame_bytes(&mut wild, 9000.0));
    // The same frame twice is the same (a replay looks like the play).
    assert!(frame_bytes(&mut wild, 9000.0) == frame_bytes(&mut wild, 9000.0));
    let run = |gimmick: Option<crate::tko::gimmick::Gimmick>| {
        let mut g = tko_game(gimmick, Mods::NONE);
        let mut t = -2000.0;
        while t < 20_000.0 {
            g.time = t;
            g.autoplay(None);
            g.expire(t);
            if (t as i64) % 2000 == 0 {
                count(&mut g, t);
            }
            t += 5.0;
        }
        (
            g.stats.great,
            g.stats.ok,
            g.stats.miss,
            g.stats.combo,
            g.stats.score,
        )
    };
    let base = run(None);
    assert!(base.0 > 20, "autoplay hits the notes: {base:?}");
    assert_eq!(run(Some(effects_gimmick())), base);
}

/// Beats, hits and combos wired to motion.
fn graph_gimmick() -> crate::tko::gimmick::Gimmick {
    use crate::tko::gimmick::{FieldProp, Group, NoteProp};
    use crate::tko::graph::{Kind, Target};
    let mut g = crate::tko::gimmick::Gimmick::default();
    g.groups.push(Group {
        notes: (0..64)
            .step_by(2)
            .map(|b| b as f64 * 500.0 - beatmap::OSU_TIMING_MS)
            .collect(),
        ..Group::default()
    });
    let gr = &mut g.graph;
    let mut add = |kind: Kind, p: &[f64], target: Option<Target>, from: &[u32]| {
        let id = gr.add(kind, 0.0, 0.0).unwrap();
        let n = gr.node_mut(id).unwrap();
        n.p = p.to_vec();
        n.target = target;
        for (port, f) in from.iter().enumerate() {
            gr.connect(*f, id, port).unwrap();
        }
        id
    };
    let beat = add(Kind::OnBeat, &[1.0, 0.0], None, &[]);
    add(
        Kind::Pulse,
        &[0.2, 250.0],
        Some(Target::Field(FieldProp::Zoom)),
        &[beat],
    );
    let hit = add(Kind::OnHit, &[1.0, 0.0], None, &[]);
    add(Kind::Shake, &[10.0, 300.0], None, &[hit]);
    add(
        Kind::Pulse,
        &[30.0, 300.0],
        Some(Target::Group(0, NoteProp::Dy)),
        &[hit],
    );
    let miss = add(Kind::OnMiss, &[0.0], None, &[]);
    add(
        Kind::Pulse,
        &[90.0, 500.0],
        Some(Target::Field(FieldProp::Rotation)),
        &[miss],
    );
    let combo = add(Kind::Combo, &[], None, &[]);
    add(
        Kind::Drive,
        &[0.01],
        Some(Target::Field(FieldProp::X)),
        &[combo],
    );
    g.sanitize();
    g
}

#[test]
fn a_node_graph_moves_the_field_on_hits_and_never_changes_judging() {
    draw_with_defaults();
    let run = |gimmick: Option<crate::tko::gimmick::Gimmick>| {
        let mut g = tko_game(gimmick, Mods::NONE);
        let mut frames = Vec::new();
        let mut t = -2000.0;
        while t < 34_000.0 {
            g.time = t;
            g.autoplay(None);
            g.expire(t);
            if (t as i64) % 1000 == 0 {
                frames.push(frame_bytes(&mut g, t));
            }
            t += 5.0;
        }
        let judged = (
            g.stats.great,
            g.stats.ok,
            g.stats.miss,
            g.stats.combo,
            g.stats.max_combo,
            g.stats.score,
            g.stats.errors.clone(),
        );
        (judged, frames)
    };
    let (plain, plain_frames) = run(None);
    let (graphed, graph_frames) = run(Some(graph_gimmick()));
    let (_, again_frames) = run(Some(graph_gimmick()));
    assert!(plain.0 > 30);
    assert_eq!(graphed, plain, "the graph only draws");
    assert!(plain_frames != graph_frames, "it did draw something");
    assert!(graph_frames == again_frames, "and the same every time");
}

#[test]
fn a_hidden_group_is_not_drawn_but_the_notes_stay_hittable() {
    use crate::tko::gimmick::{Gimmick, Group, NoteProp};
    use crate::tko::track::{Ease, Key};
    draw_with_defaults();
    let mut g = Gimmick::default();
    g.groups.push(Group {
        // The chart's own times: the game's clock is `OSU_TIMING_MS` ahead of them.
        notes: (0..64)
            .map(|b| b as f64 * 500.0 - beatmap::OSU_TIMING_MS)
            .collect(),
        ..Group::default()
    });
    g.key_note(
        0,
        NoteProp::Hidden,
        Key {
            t: 0.0,
            v: 1.0,
            ease: Ease::Step,
        },
    );
    let mut plain = tko_game(None, Mods::NONE);
    let mut hidden = tko_game(Some(g), Mods::NONE);
    let a = frame_bytes(&mut plain, 3000.0).len();
    let b = frame_bytes(&mut hidden, 3000.0).len();
    assert!(b < a, "the notes are not drawn: {b} vs {a}");
    hidden.time = 20_000.0;
    for t in (0..40_000).step_by(5) {
        hidden.time = t as f64 - 2000.0;
        hidden.autoplay(None);
        hidden.expire(hidden.time);
    }
    assert!(hidden.stats.great > 30 && hidden.stats.miss == 0);
}
