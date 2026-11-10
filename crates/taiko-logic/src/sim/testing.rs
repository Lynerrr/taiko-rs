//! What the play's tests build their plays from, here and in the client's tests of its drawing
//! (the `testing` feature).

use std::sync::Arc;

use super::*;
use crate::beatmap;
use crate::mods::Mods;

pub fn game() -> Game {
    let map = beatmap::calibration_map(500.0, 0, 64);
    Game::new(map, Arc::new(Silence), None)
}

pub fn modded(mods: &str) -> Game {
    let map = beatmap::calibration_map(500.0, 0, 64);
    Game::with_mods(map, Arc::new(Silence), None, Mods::parse(mods))
}

/// Hits every note `err` ms off (centre key, so dons), then lets the map end.
pub fn play_all(g: &mut Game, err: f64) {
    let times: Vec<f64> = g.map.notes.iter().map(|n| n.time).collect();
    for t in times {
        g.press_at(1, t + err, None);
    }
    let end = g.end_time + 2000.0;
    g.set_time(end);
}

/// The calibration map with a kiai from 8 s, for the mods that wait for one.
pub fn kiai_game(mods: &str) -> Game {
    let mut map = beatmap::calibration_map(500.0, 0, 64);
    let cp = map.control_points[0];
    map.control_points.push(beatmap::ControlPoint {
        time: 8000.0,
        kiai: true,
        ..cp
    });
    Game::with_mods(map, Arc::new(Silence), None, Mods::parse(mods))
}

pub fn quiet_game(osu: &str) -> Game {
    Game::new(beatmap::parse_raw(osu), Arc::new(Silence), None)
}

/// A Defense game of the calibration map (a don every 500 ms, a bar every 2 s), 4-bar turns,
/// with us second of `team`.
pub fn defense_game(team: usize) -> Game {
    let map = beatmap::calibration_map(500.0, 0, 64);
    let mut g = Game::new(map, Arc::new(Silence), None);
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
