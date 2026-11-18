use std::sync::Arc;

use crate::beatmap::{Beatmap, Kind};
use crate::branch::{EXPERT, MASTER, NORMAL};
use crate::mods::Mods;
use crate::sim::{Game, Judge, Silence};
use crate::tja;

/// A one-course Oni chart at 120 BPM (a bar is 2 s), `body` between #START and #END.
fn course(body: &str) -> Beatmap {
    let text = format!("TITLE:T\nBPM:120\nOFFSET:0\nCOURSE:Oni\nLEVEL:8\n\n#START\n{body}\n#END\n");
    let mut song = tja::parse(&text).expect("parses");
    song.courses.remove(0).map
}

/// Two bars of dons, then a branch: Normal plays dons, Expert kats, Master big dons; then a
/// shared bar.
const BRANCHING: &str =
    "1111,\n1111,\n#BRANCHSTART p,60,90\n#N\n1111,\n#E\n2222,\n#M\n3333,\n#BRANCHEND\n1111,";

/// Plays `g` to its end, every note on the branch played hit `err(i)` ms off (None: let go),
/// in time order as a player would; the presses made.
fn play(g: &mut Game, err: impl Fn(usize) -> Option<f64>) -> Vec<(f64, u8)> {
    let mut presses = Vec::new();
    let n = g.map().notes.len();
    for i in 0..n {
        let note = g.map().notes[i].clone();
        g.set_time(note.time - 150.0);
        if !g.on_path(i) {
            continue;
        }
        let key = if note.kind == Kind::Kat { 0 } else { 1 };
        match note.kind {
            Kind::Don | Kind::Kat => {
                if let Some(e) = err(i) {
                    g.press_at(key, note.time + e, None);
                    presses.push((note.time + e, key as u8));
                }
            }
            Kind::Roll | Kind::Swell => {
                if err(i).is_some() {
                    let mut t = note.time;
                    let mut k = 0;
                    while t <= note.end_time {
                        g.press_at(k % 2, t, None);
                        presses.push((t, (k % 2) as u8));
                        t += 30.0;
                        k += 1;
                    }
                }
            }
        }
    }
    let end = g.end_time() + 2000.0;
    g.set_time(end);
    presses
}

fn game(map: Beatmap) -> Game {
    Game::new(map, Arc::new(Silence), None)
}

/// The kinds of the notes judged, in order.
fn judged(g: &Game) -> Vec<(Kind, bool)> {
    (g.map().notes.iter().zip(g.state()))
        .filter(|(_, st)| st.judged().is_some())
        .map(|(n, _)| (n.kind, n.big))
        .collect()
}

#[test]
fn a_play_starts_on_normal() {
    let g = game(course(BRANCHING));
    assert_eq!(g.branch().unwrap().branch, NORMAL);
    assert_eq!(g.branch().unwrap().since, f64::NEG_INFINITY);
    let on: Vec<Kind> = (0..g.map().notes.len())
        .filter(|&i| g.on_path(i))
        .map(|i| g.map().notes[i].kind)
        .collect();
    assert_eq!(on, [Kind::Don; 16]);
    // A map that doesn't branch has none.
    assert_eq!(game(course("1111,")).branch(), None);
}

#[test]
fn accuracy_picks_the_branch() {
    // All GREAT: Master.
    let mut g = game(course(BRANCHING));
    play(&mut g, |_| Some(0.0));
    let now = g.branch().unwrap();
    assert_eq!((now.branch, now.was, now.since), (MASTER, NORMAL, 4000.0));
    assert_eq!((now.at(3999.0), now.at(4000.0)), (NORMAL, MASTER));
    let j = judged(&g);
    assert_eq!(j.len(), 16, "{j:?}");
    assert_eq!(j[8..12], [(Kind::Don, true); 4]);
    assert_eq!(g.stats().miss, 0);

    // Half GREAT, half OK in the bars counted (75%): Expert.
    let mut g = game(course(BRANCHING));
    play(&mut g, |i| Some(if i % 2 == 0 { 0.0 } else { 60.0 }));
    assert_eq!(g.branch().unwrap().branch, EXPERT);
    assert_eq!(judged(&g)[8..12], [(Kind::Kat, false); 4]);

    // Nothing hit: Normal, and the notes off it are never judged, not even as misses.
    let mut g = game(course(BRANCHING));
    play(&mut g, |_| None);
    assert_eq!(g.branch().unwrap().branch, NORMAL);
    assert_eq!(g.stats().miss, 16);
}

#[test]
fn the_bar_before_the_branch_does_not_count() {
    // Picked at the start of the second bar: only the first counts. Missing the whole second
    // bar still gives Master.
    let mut g = game(course(BRANCHING));
    play(&mut g, |i| (!(4..8).contains(&i)).then_some(0.0));
    assert_eq!(g.branch().unwrap().branch, MASTER);
}

#[test]
fn section_starts_the_count_over() {
    let body =
        "1111,\n#SECTION\n1111,\n1111,\n#BRANCHSTART p,60,90\n#N\n1111,\n#E\n2222,\n#M\n3333,";
    // The first bar missed, the second hit: counted from #SECTION, it's 100%.
    let mut g = game(course(body));
    play(&mut g, |i| (i >= 4).then_some(0.0));
    assert_eq!(g.branch().unwrap().branch, MASTER);
    // Without it, 4 of 8: Normal.
    let mut g = game(course(&body.replace("#SECTION\n", "")));
    play(&mut g, |i| (i >= 4).then_some(0.0));
    assert_eq!(g.branch().unwrap().branch, NORMAL);
}

#[test]
fn rolls_pick_by_their_hits() {
    let body = "1111,\n5008,\n#BRANCHSTART r,10,30\n#N\n1111,\n#E\n2222,\n#M\n3333,";
    // Every press counts, not just the roll's ticks: 51 on its 1.5 s at one every 30 ms.
    let mut g = game(course(body));
    play(&mut g, |_| Some(0.0));
    assert_eq!(g.branch().unwrap().branch, MASTER);
    // Not touched: Normal.
    let mut g = game(course(body));
    play(&mut g, |i| (i != ROLL).then_some(0.0));
    assert_eq!(g.branch().unwrap().branch, NORMAL);
}

/// In `rolls_pick_by_their_hits`' chart, the roll's index.
const ROLL: usize = 4;

#[test]
fn levelhold_keeps_the_branch() {
    let body = "1111,\n1111,\n#BRANCHSTART p,60,90\n#N\n1111,\n#E\n2222,\n#M\n#LEVELHOLD\n3333,\n#BRANCHEND\n1111,\n#BRANCHSTART p,60,90\n#N\n1111,\n#E\n2222,\n#M\n3333,";
    // Master earned, then everything missed: held on Master to the end.
    let mut g = game(course(body));
    play(&mut g, |i| (i < 8).then_some(0.0));
    assert_eq!(g.branch().unwrap().branch, MASTER);
    let on: Vec<(Kind, bool)> = (0..g.map().notes.len())
        .filter(|&i| g.on_path(i))
        .map(|i| (g.map().notes[i].kind, g.map().notes[i].big))
        .collect();
    assert_eq!(on.len(), 20);
    assert_eq!(on[16..], [(Kind::Don, true); 4]);
}

#[test]
fn a_replay_picks_the_same_branches_at_any_pace() {
    let mut live = game(course(BRANCHING));
    let presses = play(&mut live, |i| Some(if i % 2 == 0 { 0.0 } else { 60.0 }));
    // Played back in one jump to the end, as a seek or a server does.
    let mut back = game(course(BRANCHING));
    back.set_replay(Some((presses.clone(), 0)));
    let end = back.end_time() + 2000.0;
    back.set_time(end);
    assert_eq!(back.branch(), live.branch());
    assert_eq!(judged(&back), judged(&live));
    assert_eq!(back.stats().score, live.stats().score);
    // Rewound and played again (a replay's seek bar).
    back.seek(0.0, None, 0.0);
    back.set_time(end);
    assert_eq!(back.branch(), live.branch());
    assert_eq!(judged(&back), judged(&live));
    // And on the board chart a server is sent.
    let board = tja::BoardChart::of(tja::CourseKind::Oni, &course(BRANCHING));
    let mut server = game(board.beatmap().unwrap());
    server.set_replay(Some((presses, 0)));
    server.set_time(end);
    assert_eq!(server.branch(), live.branch());
    assert_eq!(server.stats().score, live.stats().score);
}

#[test]
fn half_keeps_the_branches_in_step() {
    let g = Game::with_mods(
        course(BRANCHING),
        Arc::new(Silence),
        None,
        Mods::parse("HA"),
    );
    let b = g.map().branching.as_ref().unwrap();
    assert_eq!(b.notes.len(), g.map().notes.len());
    // Whatever Half took, the branches' notes are still told apart.
    let on = (0..g.map().notes.len()).filter(|&i| g.on_path(i)).count();
    assert!(on < g.map().notes.len());
}

#[test]
fn notes_off_the_branch_are_not_judged() {
    let mut g = game(course(BRANCHING));
    play(&mut g, |_| Some(0.0));
    for (i, st) in g.state().iter().enumerate() {
        assert_eq!(st.judged().is_some(), g.on_path(i), "note {i}");
    }
    assert!(g.state().iter().all(|st| st.judged() != Some(Judge::Miss)));
}
