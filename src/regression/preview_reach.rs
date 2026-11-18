//! The song preview's box on a `.tko` map: as tall as the lane on a plain map or a gimmick that
//! stays on it, taller by as much as the map moves the playfield or puts its pictures off it.

use crate::preview::{lane_reach, tko_reach};
use crate::tko::gimmick::{FieldProp, Gimmick};
use crate::tko::track::{Ease, Key};
use std::sync::Arc;

fn with(build: impl FnOnce(&mut Gimmick)) -> crate::beatmap::Beatmap {
    let mut map = super::maps::basics().beatmap();
    let mut g = Gimmick::default();
    build(&mut g);
    g.sanitize();
    map.gimmick = Some(Arc::new(g));
    map
}

fn key(t: f64, v: f64) -> Key {
    Key {
        t,
        v,
        ease: Ease::Linear,
    }
}

#[test]
fn a_plain_map_shows_the_lane() {
    assert_eq!(tko_reach(&super::maps::basics().beatmap()), lane_reach());
}

#[test]
fn effects_that_stay_on_the_lane_keep_it_the_lanes_height() {
    let map = with(|g| {
        g.key_field(FieldProp::Flash, key(0.0, 1.0));
        g.key_field(FieldProp::Scroll, key(0.0, 2.0));
    });
    assert_eq!(tko_reach(&map), lane_reach());
}

#[test]
fn a_playfield_moved_down_reaches_as_far() {
    let map = with(|g| {
        g.key_field(FieldProp::Y, key(0.0, 0.0));
        g.key_field(FieldProp::Y, key(2000.0, 120.0));
    });
    let [top, bottom] = tko_reach(&map);
    let lane = lane_reach();
    assert_eq!(top, lane[0], "nothing goes above the lane");
    assert!(
        (bottom - (lane[1] + 120.0)).abs() < 1.0,
        "{bottom}: the lane's bottom 120 down"
    );
}

#[test]
fn a_turned_playfield_reaches_up_and_down() {
    let map = with(|g| g.key_field(FieldProp::Rotation, key(0.0, 30.0)));
    let [top, bottom] = tko_reach(&map);
    let lane = lane_reach();
    assert!(top < lane[0] && bottom > lane[1], "{top} {bottom}");
    assert!(top >= 0.0 && bottom <= crate::render::VIRTUAL_HEIGHT);
}
