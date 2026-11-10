//! What the playfield draws, shape by shape: the screen goldens (`screens`) keep only texts and
//! what can be clicked, so a note drawn in the wrong place or colour, a lost judgement or a lane
//! drawn twice shows up only here. Each frame is the list of shapes in draw order (what they
//! are, where, how big, their colours), rounded so tiny float noise doesn't count.

use super::gameplay::{replay, silent_track};
use super::maps::{self, Player};
use super::snapshot::{assert_snapshot, num};
use crate::game::{Look, Play, set_frame_look};
use crate::hit::Canvas;
use crate::multiverse;
use crate::render::{self, Shape};
use crate::scores;
use std::fmt::Write as _;
use std::time::Instant;

/// The default theme and look without animations, whatever other tests draw with meanwhile
/// (both are this thread's).
fn draw_with_defaults() {
    render::set_frame_theme(Some(render::Theme {
        animations: false,
        ..render::Theme::default()
    }));
    set_frame_look(Some(Look::default()));
    render::set_frame_time(Some(Instant::now()));
}

/// What a shape is, from its `params` (see `render::Shape`).
fn kind(s: &Shape) -> String {
    let p = s.params;
    if p[0] <= -1.0 {
        return "avatar".into();
    }
    if p[3] >= 100.0 {
        // A glyph or a skin image: which one is in w.
        return format!("char {}", num(p[3] as f64, 0));
    }
    let clip = match p[3] as i32 {
        1 => " clip-left",
        2 => " clip-right",
        _ => "",
    };
    format!(
        "rect r{} b{} g{}{clip}",
        num(p[0] as f64, 1),
        num(p[1] as f64, 1),
        num(p[2] as f64, 1)
    )
}

fn colour(c: [f32; 4]) -> String {
    if c[3] <= 0.0 {
        return "-".into();
    }
    c.iter()
        .map(|v| num(*v as f64, 2))
        .collect::<Vec<_>>()
        .join(",")
}

/// One frame's shapes, a line each.
fn dump(out: &mut String, shapes: &[Shape]) {
    let _ = writeln!(out, "  {} shapes", shapes.len());
    for s in shapes {
        let _ = write!(
            out,
            "  {} at {},{} half {},{} fill {} border {}",
            kind(s),
            num(s.center[0] as f64, 1),
            num(s.center[1] as f64, 1),
            num(s.half[0] as f64, 1),
            num(s.half[1] as f64, 1),
            colour(s.fill),
            colour(s.border),
        );
        if s.angle != 0.0 {
            let _ = write!(out, " angle {}", num(s.angle as f64, 3));
        }
        out.push('\n');
    }
}

/// The playfield of a dense map played by `player` with `mods`, drawn at `times` (map ms).
fn frames(mods: &[&str], player: Player, times: &[f64]) -> String {
    draw_with_defaults();
    let map = maps::stream().beatmap();
    let r = replay(maps::presses(&map, player), mods, scores::new_version());
    let mut game = scores::replay_game(&map, &silent_track(r.rate), &r);
    // Multiverse's universes from its defaults, not the process-wide setting other tests change.
    game.use_mod_settings(Some(multiverse::defaults()), None);
    let mut play = Play::new(game);
    let mut out = String::new();
    for &t in times {
        play.game.set_time(t);
        let mut c = Canvas::default();
        c.clear(853.3);
        play.draw_playfield(&mut c, 1.0);
        let _ = writeln!(out, "at {} ms, mods {:?}:", num(t, 0), mods);
        dump(&mut out, &c.shapes);
    }
    out
}

/// Before the first note, in the middle of the stream with judgements showing, and near its end.
const TIMES: [f64; 3] = [500.0, 4000.0, 9000.0];

#[test]
fn the_playfield_without_mods() {
    assert_snapshot(
        "drawing-playfield-no-mods",
        &frames(&[], Player::Jitter(25.0), &TIMES),
    );
}

#[test]
fn the_playfield_with_hidden() {
    assert_snapshot(
        "drawing-playfield-hidden",
        &frames(&["HD"], Player::Jitter(25.0), &TIMES),
    );
}

#[test]
fn the_playfield_with_hard_rock_and_double_time() {
    assert_snapshot(
        "drawing-playfield-hr-dt",
        &frames(&["HR", "DT"], Player::Jitter(25.0), &TIMES),
    );
}

#[test]
fn the_playfield_with_multiverse() {
    assert_snapshot(
        "drawing-playfield-multiverse",
        &frames(&["MV"], Player::Jitter(25.0), &TIMES),
    );
}

/// The same frames twice in a row draw the same shapes: nothing in them depends on the clock.
#[test]
fn a_frame_draws_the_same_shapes_every_time() {
    let a = frames(&["HD", "DT"], Player::Jitter(25.0), &TIMES);
    let b = frames(&["HD", "DT"], Player::Jitter(25.0), &TIMES);
    assert!(a == b, "the same frame drew different shapes");
}
