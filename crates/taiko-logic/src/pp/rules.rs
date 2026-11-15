//! What taiko.rs's own mods do to a play's pp, on top of osu!'s formula: the rules worked out
//! in the pp lab (`taiko-pplab`), with its values written in here.
//!
//! - Each mod has a multiplier (`VALUES`, else its score multiplier as `custom_factor` took
//!   it); mods taken out of the game (`REMOVED`) and those that give no extra pp (`NO_PP`) count
//!   x1 and take part in nothing below.
//! - Memory: a play that hides its notes twice over (`memory`) counts as HDFL in osu!'s formula,
//!   and the game's other own mods give it nothing more (they count as nomod: `cancelled`).
//! - SV mods (`SV`) lose their multipliers and go into osu!'s formula as note speeds, like HR's
//!   and EZ's; so do the rails' lanes (`rail_factor`). With Maze, only `SV_KEEP` of what note
//!   speeds add counts.
//! - Mods in a group (`GROUPS`) multiply to at most its cap.
//! - Notes stacked on each other (`STACK_MS`) count as one hit.

use rosu_pp::Beatmap;
use rosu_pp::model::mode::GameMode;
use rosu_pp::taiko::TaikoPerformance;

use super::{Play, custom_factor, formula_mods};
use crate::mods::{Mod, Mods, REMOVED};
use crate::sim::{HIT_X, LANE_Y, RAIL_STEP, Rail, RailShape};

/// Mods whose multiplier differs from their score multiplier, by acronym.
const VALUES: [(&str, f64); 23] = [
    ("MZ", 1.001),
    ("LI", 1.0),
    ("CC", 1.0),
    ("HS", 1.0),
    ("AS", 1.0),
    ("CR", 1.01),
    ("SU", 1.0),
    ("LG", 1.01),
    ("CS", 1.0),
    ("BL", 1.01),
    ("AC", 1.07),
    ("BA", 1.0),
    ("SP", 1.0),
    ("CL", 1.0),
    ("BR", 1.07),
    ("DY", 1.01),
    ("MW", 1.04),
    ("MV", 1.04),
    ("MR", 1.0),
    ("TL", 1.01),
    ("SN", 1.0),
    ("PN", 1.0),
    ("RA", 1.05),
];

/// Mods that stay in the game but give no extra pp.
const NO_PP: [&str; 12] = [
    "MN", "NB", "GN", "PR", "RD", "SW", "AL", "WU", "WD", "RL", "PY", "FF",
];

/// The SV mods whose note speeds osu!'s formula reads (their multipliers go). HR's are in the
/// formula already; it only loses its (x1) multiplier here.
const SV: [&str; 10] = ["CS", "AS", "HS", "DR", "SG", "SR", "SL", "EV", "HR", "RL"];

/// With Maze, the share of what note speeds (the SV mods', the rails', HR's and EZ's) add that
/// counts. What they take away counts whole.
const SV_KEEP: f64 = 0.1;

/// Groups of mods whose multipliers together go no higher than a cap.
const GROUPS: [(&[&str], f64); 2] = [
    // Mods that leave the playfield as it is.
    (
        &["MW", "CR", "SK", "AC", "BR", "WV", "PU", "ST", "CC"],
        1.07,
    ),
    (&["MZ"], 1.0),
];

/// Mods that hide or disguise notes (`Mod::obscures`).
const HIDING: [&str; 11] = [
    "HD", "SU", "FL", "SP", "BL", "CL", "DY", "LI", "RA", "EC", "MV",
];

/// The mods osu! has too; the rest are the game's own.
const OSU: [&str; 18] = [
    "EZ", "NF", "HT", "DC", "HR", "SD", "PF", "DT", "NC", "HD", "FL", "RX", "AT", "SW", "RD", "CS",
    "WU", "WD",
];

/// The most a play's mods can multiply its pp by.
pub const CAP: f64 = super::MAX_CUSTOM_FACTOR;

/// Notes of a map closer than this to the note before them (in the map's own time) are stacked:
/// the fastest streams of real maps are 13ms apart, the notes of a stack 0 to 2.
const STACK_MS: f64 = 10.0;

/// The screen a rails lane is read on: 16:9 at osu!'s 480 tall.
const SCREEN: (f32, f32) = (480.0 * 16.0 / 9.0, 480.0);

fn of(acronyms: &[&str], m: Mod) -> bool {
    acronyms.contains(&m.acronym())
}

/// What a play's mods do to its pp.
#[derive(Clone, Debug, PartialEq)]
pub struct Bonus {
    /// The multiplier on its pp, up to `CAP`.
    pub factor: f64,
    /// It hides its notes twice over: osu!'s formula counts it as HDFL.
    pub memory: bool,
    /// The mods whose note speeds go into osu!'s formula (SV mods, rails).
    pub sv: Vec<Mod>,
    /// The mods that count as if the play didn't have them (beside memory).
    pub cancelled: Vec<Mod>,
    /// HR and EZ, whose note speeds osu!'s formula has (unless cancelled).
    pub speed_formula: Vec<Mod>,
    /// The share of what note speeds add that counts.
    pub keep: f64,
}

impl Bonus {
    /// Whether only a share of what note speeds add counts.
    fn cut(&self) -> bool {
        self.keep != 1.0 && !(self.sv.is_empty() && self.speed_formula.is_empty())
    }
}

/// A mod's multiplier by these rules: x1 for those out of the game or without extra pp.
fn value(m: Mod) -> f64 {
    if REMOVED.contains(&m) || of(&NO_PP, m) {
        return 1.0;
    }
    VALUES
        .iter()
        .find(|v| v.0 == m.acronym())
        .map_or_else(|| custom_factor(Mods::NONE.flip(m)), |v| v.1)
}

/// What the play's `mods` do to its pp.
pub fn bonus(mods: Mods) -> Bonus {
    let set: Vec<Mod> = mods.iter().collect();
    let flat = |m: Mod| REMOVED.contains(&m) || of(&NO_PP, m);
    let mut free: Vec<bool> = set.iter().map(|&m| !flat(m)).collect();
    let mut b = Bonus {
        factor: 1.0,
        memory: false,
        sv: Vec::new(),
        cancelled: Vec::new(),
        speed_formula: Vec::new(),
        keep: 1.0,
    };
    // Memory: two mods hiding the notes, one of them the game's own (osu!'s HDFL alone is osu!'s).
    let hiding: Vec<usize> = (0..set.len())
        .filter(|&i| free[i] && of(&HIDING, set[i]))
        .collect();
    if hiding.len() >= 2 && hiding.iter().any(|&i| !of(&OSU, set[i])) {
        b.memory = true;
        // Its hiding mods are what it is: their multipliers go. The game's other own mods add
        // nothing to a play learned by heart.
        // osu!'s other mods go on as usual.
        for i in 0..set.len() {
            if hiding.contains(&i) {
                free[i] = false;
            } else if free[i] && !of(&OSU, set[i]) {
                free[i] = false;
                b.cancelled.push(set[i]);
            }
        }
    }
    for i in 0..set.len() {
        if free[i] && of(&SV, set[i]) {
            free[i] = false;
            b.sv.push(set[i]);
        }
    }
    for (group, cap) in GROUPS {
        let mut own = 1.0;
        for i in 0..set.len() {
            if free[i] && of(group, set[i]) {
                free[i] = false;
                own *= value(set[i]);
            }
        }
        b.factor *= own.min(cap);
    }
    for i in 0..set.len() {
        if free[i] {
            b.factor *= value(set[i]);
        }
    }
    b.factor = b.factor.min(CAP);
    // Rails go into osu!'s formula as note speeds too, keeping their multipliers.
    for &m in &set {
        let rail = RailShape::of(Mods::NONE.flip(m)).is_some();
        if rail && !b.cancelled.contains(&m) && !b.sv.contains(&m) {
            b.sv.push(m);
        }
    }
    if mods.has(Mod::Maze) {
        b.keep = SV_KEEP;
    }
    b.speed_formula = (set.iter().copied())
        .filter(|m| matches!(m, Mod::HardRock | Mod::Easy) && !b.cancelled.contains(m))
        .collect();
    b
}

/// The `.osu` map with each run of stacked notes merged into its first, and the share of its
/// notes that leaves. A stack falls to as many presses anywhere in the hit window, so it is one
/// hit to the hands, yet the star rating reads it as a stream of notes no time apart. None for
/// a map without stacks (or one that isn't an `.osu`).
pub(super) fn unstacked(map: &[u8]) -> Option<(Vec<u8>, f64)> {
    let text = String::from_utf8_lossy(map);
    let (head, body) = text.split_at(text.find("[HitObjects]")?);
    let lines: Vec<&str> = body.lines().collect();
    // The circles by their time: only they are hit, and only they count towards the stars.
    let mut hits: Vec<(f64, usize)> = (lines.iter().enumerate().skip(1))
        .take_while(|(_, l)| !l.starts_with('['))
        .filter_map(|(i, l)| {
            let mut fields = l.split(',').skip(2);
            let time: f64 = fields.next()?.trim().parse().ok()?;
            let kind: u32 = fields.next()?.trim().parse().ok()?;
            (kind & 1 != 0).then_some((time, i))
        })
        .collect();
    hits.sort_by(|a, b| a.0.total_cmp(&b.0));
    let merged: std::collections::HashSet<usize> = (hits.windows(2))
        .filter(|w| w[1].0 - w[0].0 < STACK_MS)
        .map(|w| w[1].1)
        .collect();
    if merged.is_empty() {
        return None;
    }
    let kept: Vec<&str> = (lines.iter().enumerate())
        .filter(|(i, _)| !merged.contains(i))
        .map(|(_, l)| *l)
        .collect();
    Some((
        format!("{head}{}\n", kept.join("\n")).into_bytes(),
        1.0 - merged.len() as f64 / hits.len() as f64,
    ))
}

/// The play's counts on its map with `kept` of the notes left: the same accuracy and share of
/// misses over fewer notes.
pub(super) fn scaled(mut play: Play, kept: f64) -> Play {
    let scale = |n: u32| (f64::from(n) * kept).round() as u32;
    let all = play.great + play.ok + play.miss;
    let judged = scale(all).max(u32::from(all > 0));
    play.miss = scale(play.miss).min(judged);
    play.ok = scale(play.ok).min(judged - play.miss);
    play.great = judged - play.miss - play.ok;
    play.max_combo = scale(play.max_combo);
    play
}

/// Rails: how many times as fast as on the plain lane its notes count as scrolling for osu!'s
/// formula. They go along the bent lane as fast as along the plain one, so they're there to read
/// for as long as the part of it on screen is long (the rest runs off the edge): the plain lane
/// over that. 1 without a rails mod.
pub fn rail_factor(mods: Mods) -> f64 {
    let Some(shape) = RailShape::of(mods) else {
        return 1.0;
    };
    let (w, h) = SCREEN;
    let (len, drop, zoom) = (shape.length(w), shape.drop(), shape.zoom(w));
    let rail = Rail::build(|s| shape.bend(s), len);
    // The playfield moves down `drop` and shrinks by `zoom` about the hit target.
    let on_screen = |s: f32| {
        let (x, y, _) = rail.at(s);
        let (sx, sy) = (
            HIT_X + (x - HIT_X) * zoom,
            LANE_Y + drop + (y - LANE_Y) * zoom,
        );
        (0.0..=w).contains(&sx) && (0.0..=h).contains(&sy)
    };
    let steps = (len / RAIL_STEP).ceil() as usize;
    let playable = (0..steps)
        .filter(|&k| on_screen(k as f32 * RAIL_STEP))
        .count() as f32
        * RAIL_STEP;
    f64::from((w - HIT_X) / playable.max(RAIL_STEP))
}

/// Gives each of the map's notes the scroll speed `factors` (by note time) say it has, on top of
/// the map's own: what osu!'s formula reads like HR's and EZ's. Everything else keeps the map's.
fn speed_up(map: &mut Beatmap, factors: &[(f64, f64)]) {
    use rosu_pp::model::control_point::EffectPoint;
    let ms = |t: f64| (t * 1000.0).round() as i64;
    let own = map.effect_points.clone();
    let at = |t: f64| {
        own[..own.partition_point(|e| e.time <= t)].last().map_or(
            (EffectPoint::DEFAULT_SCROLL_SPEED, EffectPoint::DEFAULT_KIAI),
            |e| (e.scroll_speed, e.kiai),
        )
    };
    let factor = |t: f64| {
        let i = factors.partition_point(|f| f.0 < t - 1.0);
        (factors.get(i))
            .filter(|f| (f.0 - t).abs() <= 1.0)
            .map_or(1.0, |f| f.1)
    };
    // One point per time (rosu-pp's search takes any of several), each object's last.
    let mut points: std::collections::BTreeMap<i64, EffectPoint> =
        own.iter().map(|e| (ms(e.time), *e)).collect();
    for o in &map.hit_objects {
        let (speed, kiai) = at(o.start_time);
        let f = if o.is_circle() {
            factor(o.start_time)
        } else {
            1.0
        };
        points.insert(
            ms(o.start_time),
            EffectPoint {
                time: o.start_time,
                kiai,
                scroll_speed: speed * f,
            },
        );
    }
    map.effect_points = points.into_values().collect();
}

/// The map osu!'s formula reads for a play with `mods`: notes taken off or recoloured
/// (`stars::load_played`), and scrolling at the `sv` mods' speeds.
pub(super) fn formula_map(bytes: &[u8], p: &Play, mods: Mods, sv: &[Mod]) -> Option<Beatmap> {
    let mut map = crate::stars::load_played(bytes, mods, p.seed, p.half)?;
    if !sv.is_empty() {
        map.convert_mut(GameMode::Taiko, &Default::default()).ok()?;
        // As written: the times osu!'s formula reads, not moved for the game's music offset.
        let notes = crate::beatmap::parse_raw(&String::from_utf8_lossy(bytes));
        let sv = sv.iter().fold(Mods::NONE, |all, m| all.flip(*m));
        let rail = rail_factor(sv);
        let mut factors = crate::sim::note_speed_factors(&notes, sv);
        factors.iter_mut().for_each(|f| f.1 *= rail);
        speed_up(&mut map, &factors);
    }
    Some(map)
}

/// The mods osu!'s formula takes for a play: without `out`, and as HDFL for `memory`.
pub(super) fn formula_of(mods: Mods, out: &[Mod], memory: bool) -> Mods {
    let mut mods = out
        .iter()
        .fold(mods, |all, &m| if all.has(m) { all.flip(m) } else { all });
    for m in [Mod::Hidden, Mod::Flashlight] {
        if memory && !mods.has(m) {
            mods = mods.flip(m);
        }
    }
    mods
}

/// What osu!'s formula is asked about a play.
#[derive(Clone, Copy)]
pub(super) enum Hits {
    /// Its counts.
    Counts {
        great: u32,
        ok: u32,
        miss: u32,
        combo: u32,
    },
    /// A full combo at this accuracy (percent).
    Accuracy(f64),
}

/// The difficulty and accuracy parts of `hits`, and of an SS, with the star rating and the
/// map's combo, on `bytes` worked out without the mods `out` and with the `sv` mods' speeds.
pub(super) type Parts = ([f64; 4], f64, u32);

fn parts_on(
    bytes: &[u8],
    p: &Play,
    b: &Bonus,
    sv: &[Mod],
    out: &[Mod],
    hits: Hits,
) -> Option<Parts> {
    let mods = formula_of(p.mods, out, b.memory);
    let map = formula_map(bytes, p, mods, sv)?;
    let base = TaikoPerformance::new(&map)
        .mods(formula_mods(mods, p.scroll_speed))
        .clock_rate(p.rate);
    let attrs = match hits {
        Hits::Counts {
            great,
            ok,
            miss,
            combo,
        } => base
            .clone()
            .n300(great)
            .n100(ok)
            .misses(miss)
            .combo(combo)
            .calculate(),
        Hits::Accuracy(acc) => base.clone().accuracy(acc).misses(0).calculate(),
    }
    .ok()?;
    let max_combo = attrs.difficulty.max_combo;
    let ss = base
        .n300(max_combo)
        .n100(0)
        .misses(0)
        .combo(max_combo)
        .calculate()
        .ok()?;
    Some((
        [
            attrs.pp_difficulty,
            attrs.pp_acc,
            ss.pp_difficulty,
            ss.pp_acc,
        ],
        attrs.difficulty.stars,
        max_combo,
    ))
}

/// `hits` on the map at `bytes` as these rules have them (before the mods' multiplier): its
/// parts worked out with the note speeds, and with only `Bonus::keep` of what they add.
pub(super) fn parts(bytes: &[u8], p: &Play, b: &Bonus, hits: Hits) -> Option<Parts> {
    let with = parts_on(bytes, p, b, &b.sv, &b.cancelled, hits)?;
    if !b.cut() {
        return Some(with);
    }
    let out: Vec<Mod> = [&b.cancelled[..], &b.speed_formula[..]].concat();
    let Some(bare) = parts_on(bytes, p, b, &[], &out, hits) else {
        return Some(with);
    };
    let keep = |with: f64, without: f64| {
        let gain = with - without;
        without + if gain > 0.0 { b.keep * gain } else { gain }
    };
    Some((
        std::array::from_fn(|i| keep(with.0[i], bare.0[i])),
        keep(with.1, bare.1),
        with.2,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_acronym_in_the_rules_is_a_mod() {
        let lists: [&[&str]; 5] = [&NO_PP, &SV, &HIDING, &OSU, GROUPS[0].0];
        for a in lists
            .iter()
            .flat_map(|l| l.iter())
            .chain(VALUES.iter().map(|v| &v.0))
        {
            assert!(Mod::from_acronym(a).is_some(), "{a}");
        }
    }

    #[test]
    fn removed_and_no_pp_mods_count_x1_and_groups_stop_at_their_cap() {
        for m in REMOVED {
            assert_eq!(value(m), 1.0, "{}", m.acronym());
        }
        assert_eq!(bonus(Mods::parse("PR")).factor, 1.0);
        assert_eq!(bonus(Mods::parse("AC")).factor, 1.07);
        // AC and BR are in a group capped at 1.07: together no more than one.
        assert_eq!(bonus(Mods::parse("AC,BR")).factor, 1.07);
        // Maze's own group is capped at 1.
        assert_eq!(bonus(Mods::parse("MZ")).factor, 1.0);
        assert_eq!(bonus(Mods::parse("RA")).factor, 1.05);
    }

    #[test]
    fn sv_mods_and_rails_go_into_the_formula_and_maze_cuts_their_gain() {
        let b = bonus(Mods::parse("SL,HR,ZZ"));
        assert_eq!(b.sv, [Mod::HardRock, Mod::Split, Mod::Zigzag]);
        assert_eq!(
            (b.keep, b.speed_formula.clone()),
            (1.0, vec![Mod::HardRock])
        );
        let maze = bonus(Mods::parse("SL,MZ"));
        assert!(maze.cut() && maze.keep == SV_KEEP && maze.sv.contains(&Mod::Maze));
    }

    #[test]
    fn hiding_the_notes_twice_is_memory_and_the_games_own_mods_add_nothing() {
        // Sudden and Flashlight: notes only near the drum, in the dark.
        let b = bonus(Mods::parse("SU,FL,AC,HR"));
        assert!(b.memory);
        assert_eq!(b.cancelled, [Mod::Accel]);
        assert_eq!(b.factor, 1.0);
        // osu!'s HR goes on as usual: into the formula as a note speed.
        assert_eq!(b.sv, [Mod::HardRock]);
        // osu!'s own HDFL is osu!'s, and one hiding mod is just that.
        assert!(!bonus(Mods::parse("HD,FL,AC")).memory);
        assert!(!bonus(Mods::parse("SU,AC")).memory);
    }

    #[test]
    fn rails_count_as_slower_notes_by_how_much_lane_is_on_screen() {
        assert_eq!(rail_factor(Mods::NONE), 1.0);
        let factor = |m| rail_factor(Mods::NONE.flip(m));
        for m in [
            Mod::Zigzag,
            Mod::UTurn,
            Mod::Spiral,
            Mod::Loop,
            Mod::Elbow,
            Mod::Maze,
        ] {
            // Longer on screen than the plain lane: slower; shorter (Elbow, off the top): faster.
            assert!(factor(m) > 0.1 && factor(m) < 2.0, "{m:?}");
        }
        assert!(factor(Mod::Maze) < 0.5 && factor(Mod::Elbow) > 1.0);
    }

    #[test]
    fn stacked_notes_are_merged_and_the_play_scaled_to_what_is_left() {
        let head = "osu file format v14\n\n[TimingPoints]\n0,300,4,1,0,100,1,0\n\n";
        let notes = "[HitObjects]\n1,1,1000,5,0\n1,1,1000,1,0\n1,1,1004,2,0,L|1:1,1,100\n\
                     1,1,1001,1,8\n1,1,2009,1,0\n1,1,2000,1,0\n1,1,2030,1,0\n";
        let (map, kept) = unstacked(format!("{head}{notes}").as_bytes()).unwrap();
        assert_eq!(
            String::from_utf8(map).unwrap(),
            format!(
                "{head}[HitObjects]\n1,1,1000,5,0\n1,1,1004,2,0,L|1:1,1,100\n1,1,2000,1,0\n1,1,2030,1,0\n"
            )
        );
        assert_eq!(kept, 0.5);
        let play = Play {
            great: 9,
            ok: 0,
            miss: 1,
            max_combo: 9,
            rate: 1.0,
            mods: Mods::NONE,
            scroll_speed: 1.0,
            seed: 0,
            half: Default::default(),
        };
        let half = scaled(play, 0.5);
        assert_eq!(
            (half.great, half.ok, half.miss, half.max_combo),
            (4, 0, 1, 5)
        );
    }
}
