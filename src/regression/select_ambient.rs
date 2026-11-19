//! Song select's small continuous motion in the main menu's look: a set opening drops its
//! difficulties in one after another.

use super::harness::{self, *};
use super::maps;
use crate::*;

/// Song select in the main menu's look, animations on: set "ambient-a" with three difficulties
/// and set "ambient-b" with one.
fn sets(tag: &str) -> (State, Songs, Instant) {
    let charts = [
        maps::Chart {
            name: "ambient-a",
            ..maps::basics()
        },
        maps::Chart {
            name: "ambient-b",
            ..maps::basics()
        },
    ];
    let songs = Songs::new(tag, &charts);
    let first = songs.paths[0].clone();
    let text = std::fs::read_to_string(&first).unwrap();
    // Its difficulties next to each other in the library, as a scan finds a folder's.
    let mut paths = vec![first.clone()];
    for v in ["ambient-a-2", "ambient-a-3"] {
        let p = first.with_file_name(format!("{v}.osu"));
        std::fs::write(
            &p,
            text.replace("Version:ambient-a", &format!("Version:{v}")),
        )
        .unwrap();
        paths.push(p);
    }
    paths.push(songs.paths[1].clone());
    let entries = paths
        .iter()
        .map(|p| Entry::new(p.clone(), beatmap::read_meta_file(p).expect("a chart")))
        .collect();
    let cfg = Config {
        main_menu: true,
        animations: true,
        ..harness::config()
    };
    let mut s = state_with(cfg, entries);
    let mut now = Instant::now();
    ready(&mut s, &mut now);
    assert!(!s.select.legacy);
    (s, songs, now)
}

fn settled(s: &mut State, now: &mut Instant) -> Canvas {
    frame(s, *now);
    s.scene_since = *now - Duration::from_secs(5);
    s.toast = None;
    settle(s, now);
    *now += Duration::from_millis(16);
    frame(s, *now)
}

/// Where the difficulty `v`'s name is drawn (its top) and how opaque.
fn diff(c: &Canvas, v: &str) -> Option<(f32, f32)> {
    c.texts.iter().find_map(|(r, t)| {
        let sh = c.shapes.get(r.start)?;
        (t == v).then(|| (sh.center[1] - sh.half[1], sh.fill[3]))
    })
}

fn selected_title(s: &State) -> String {
    let i = s.select.selected().expect("a map selected");
    s.library[i].meta.title.clone()
}

#[test]
fn opening_a_set_drops_its_difficulties_in_one_after_another() {
    let (mut s, _songs, mut now) = sets("select-ambient-cascade");
    settled(&mut s, &mut now);
    // To the other set and back: the three difficulties' set opens again.
    for key in [KeyCode::ArrowRight, KeyCode::ArrowLeft] {
        let before = selected_title(&s);
        press(&mut s, key, now);
        settled(&mut s, &mut now);
        if key == KeyCode::ArrowRight {
            assert_ne!(selected_title(&s), before, "on the other set");
        }
    }
    assert_eq!(selected_title(&s), "ambient-a");
    let c = settled(&mut s, &mut now);
    let rest: Vec<(f32, f32)> = ["ambient-a", "ambient-a-2", "ambient-a-3"]
        .iter()
        .map(|v| {
            diff(&c, v).unwrap_or_else(|| {
                panic!(
                    "{v} shown: {:?}",
                    c.texts.iter().map(|(_, t)| t).collect::<Vec<_>>()
                )
            })
        })
        .collect();
    press(&mut s, KeyCode::ArrowRight, now);
    settled(&mut s, &mut now);
    press(&mut s, KeyCode::ArrowLeft, now);
    now += Duration::from_millis(16);
    frame(&mut s, now);
    now += Duration::from_millis(60);
    let c = frame(&mut s, now);
    let now_at: Vec<(f32, f32)> = ["ambient-a", "ambient-a-2", "ambient-a-3"]
        .iter()
        .map(|v| diff(&c, v).unwrap_or((f32::NAN, 0.0)))
        .collect();
    assert!(
        now_at[0].1 > now_at[2].1 + 0.05,
        "the first in before the last: {now_at:?}"
    );
    assert!(
        now_at[2].1 < rest[2].1,
        "the last still on its way: {now_at:?} vs {rest:?}"
    );
    for _ in 0..40 {
        now += Duration::from_millis(16);
        frame(&mut s, now);
    }
    let c = frame(&mut s, now);
    for (v, r) in ["ambient-a", "ambient-a-2", "ambient-a-3"]
        .iter()
        .zip(&rest)
    {
        assert_eq!(diff(&c, v), Some(*r), "{v} in its place at the end");
    }
}
