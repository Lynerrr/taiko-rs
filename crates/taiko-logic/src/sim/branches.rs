//! A branching chart in play (`crate::branch`): every branch's notes are in the map, and the
//! ones off the branch played are left out, as if they weren't there (never judged, never
//! drawn). The branch is picked at each branch point from what was judged since the last
//! `#SECTION`, by times alone, so a replay played back (here or on a server) picks the same.

use std::sync::Arc;

use super::{Game, Judge};
use crate::beatmap::Kind;
use crate::branch::{self, Branching, Measure};

/// How long after a branch point's judge time the branch is picked (map ms), so that a press
/// stamped just before it, which can reach the play a frame late, counts as it does when the
/// replay is played back. Never so late that the branch's first notes could already be hit.
const PICK_LAG_MS: f64 = 50.0;

/// What a branch condition counts.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Counted {
    /// A don or kat judged.
    Note(Judge),
    /// A drumroll tick hit.
    Roll,
    /// A hit on a balloon.
    Balloon,
}

#[derive(Clone, Copy, Debug)]
struct Event {
    /// Map ms: a press's own time, or for a note let go by, when its last chance went.
    t: f64,
    what: Counted,
    big: bool,
}

/// A branching chart's branch, as `Game::branch` tells it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BranchNow {
    pub branch: u8,
    pub was: u8,
    /// Map ms the branch takes over at.
    pub since: f64,
}

impl BranchNow {
    /// The branch played at map time `t`.
    pub fn at(&self, t: f64) -> u8 {
        if t >= self.since {
            self.branch
        } else {
            self.was
        }
    }
}

/// Where a branching chart's play is.
pub(crate) struct BranchRun {
    info: Arc<Branching>,
    /// The branch played from each branch point on: the ones picked, then the current one.
    path: Vec<u8>,
    /// The branch played now, the one before it, and when it took over (map ms; minus
    /// infinity: from the start).
    current: u8,
    was: u8,
    since: f64,
    /// The next branch point and `#LEVELHOLD` still to come.
    next: usize,
    next_hold: usize,
    /// A `#LEVELHOLD` was met: the branch stays to the end.
    held: bool,
    log: Vec<Event>,
}

impl BranchRun {
    fn new(info: Arc<Branching>) -> BranchRun {
        BranchRun {
            path: vec![branch::NORMAL; info.points.len()],
            info,
            current: branch::NORMAL,
            was: branch::NORMAL,
            since: f64::NEG_INFINITY,
            next: 0,
            next_hold: 0,
            held: false,
            log: Vec::new(),
        }
    }

    /// What `point`'s condition reads, from what was judged since the last `#SECTION` up to
    /// its judge time (`score`: the play's score now).
    fn value(&self, point: &branch::Point, score: u64) -> f64 {
        let c = &point.condition;
        let from = (self.info.resets.iter().copied())
            .filter(|&r| r <= point.judge)
            .fold(f64::NEG_INFINITY, f64::max);
        let counted = || {
            self.log
                .iter()
                .filter(|e| e.t >= from && e.t <= point.judge && (e.big || !c.big_only))
        };
        let notes = |j: Option<Judge>| {
            counted()
                .filter(|e| matches!(e.what, Counted::Note(k) if j.is_none_or(|j| j == k)))
                .count() as f64
        };
        let (all, greats, oks) = (
            notes(None),
            notes(Some(Judge::Great)),
            notes(Some(Judge::Ok)),
        );
        let share = |v: f64| if all == 0.0 { 0.0 } else { v / all * 100.0 };
        match c.measure {
            Measure::Keep => 0.0,
            Measure::Accuracy => share(greats + oks / 2.0),
            Measure::GreatShare => share(greats),
            Measure::Greats => greats,
            Measure::Oks => oks,
            Measure::Misses => notes(Some(Judge::Miss)),
            Measure::Rolls => counted()
                .filter(|e| matches!(e.what, Counted::Roll | Counted::Balloon))
                .count() as f64,
            Measure::BalloonHits => counted().filter(|e| e.what == Counted::Balloon).count() as f64,
            Measure::Score => score as f64,
        }
    }
}

impl Game {
    /// Starts a branching chart's play on the Normal branch (none for other maps).
    pub(crate) fn start_branches(&mut self) {
        self.branches = self.map.branching.clone().map(BranchRun::new);
        self.apply_path(0);
    }

    /// Whether note `i` is on the branch played (every note of a map that doesn't branch).
    #[inline]
    pub fn on_path(&self, i: usize) -> bool {
        self.branches
            .as_ref()
            .is_none_or(|b| b.info.plays(i, |p| b.path[p]))
    }

    /// A branching chart's branch: the one picked last (`crate::branch` numbers), the one
    /// before it, and the map time it takes over at (it's picked a bar ahead).
    pub fn branch(&self) -> Option<BranchNow> {
        self.branches.as_ref().map(|b| BranchNow {
            branch: b.current,
            was: b.was,
            since: b.since,
        })
    }

    /// Counts `what` at map time `t` for the branch conditions.
    pub(crate) fn count_for_branches(&mut self, t: f64, note: usize, judge: Option<Judge>) {
        let n = &self.map.notes[note];
        let Some(b) = &mut self.branches else { return };
        let what = match (judge, n.kind) {
            (Some(j), _) => Counted::Note(j),
            (None, Kind::Swell) => Counted::Balloon,
            (None, _) => Counted::Roll,
        };
        b.log.push(Event {
            t,
            what,
            big: n.big,
        });
    }

    /// Puts the notes after branch point `from` on or off the path: off, a don or kat is
    /// unhittable and a roll or balloon is over before it starts. A note already played is
    /// left as it is.
    fn apply_path(&mut self, from: usize) {
        let Some(b) = &self.branches else { return };
        let mut first_on = None;
        for (i, tag) in b.info.notes.iter().enumerate() {
            let Some((point, _)) = *tag else { continue };
            if usize::from(point) < from {
                continue;
            }
            let on = b.info.plays(i, |p| b.path[p]);
            let st = &mut self.state[i];
            if st.judged.is_some() || st.hits > 0 || st.ticks.iter().any(|&t| t) {
                continue;
            }
            let n = &mut self.map.notes[i];
            match n.kind {
                Kind::Don | Kind::Kat => n.unhittable = !on,
                Kind::Roll | Kind::Swell => st.finished = !on,
            }
            if on && first_on.is_none() {
                first_on = Some(i);
            }
        }
        if let Some(i) = first_on {
            self.cursor = self.cursor.min(i);
        }
        // Notes came into play or went out of it: `expire` looks at them again.
        self.rescan_expiry();
        // What's left to hit changed: scores and autoplay work it out again.
        self.score_basis = None;
        self.auto_plan = None;
    }

    /// Picks the branch at each branch point (and holds it at each `#LEVELHOLD`) that `upto`
    /// (map ms) has gone past, in time order.
    pub(crate) fn pick_branches(&mut self, upto: f64) {
        loop {
            let Some(b) = &self.branches else { return };
            let pick_at = b.info.points.get(b.next).map(|p| {
                let latest = p.start - self.windows.miss - 1.0;
                (p.judge + PICK_LAG_MS).min(latest).max(p.judge)
            });
            let hold_at = b.info.holds.get(b.next_hold).copied();
            let point_first = match (pick_at, hold_at) {
                (Some(p), Some(h)) => p <= h,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => return,
            };
            let due = if point_first { pick_at } else { hold_at };
            if due.is_none_or(|t| t > upto) {
                return;
            }
            let score = self.stats.score;
            let b = self.branches.as_mut().expect("branches");
            if !point_first {
                b.held = true;
                b.next_hold += 1;
                continue;
            }
            let i = b.next;
            b.next += 1;
            let point = b.info.points[i];
            if b.held {
                continue;
            }
            let target = point.condition.target(b.value(&point, score), b.current);
            if point.hold[usize::from(target)] {
                b.held = true;
            }
            if target != b.current {
                b.was = b.current;
                b.current = target;
                b.since = point.start;
                b.path[i..].fill(target);
                self.apply_path(i);
            }
        }
    }
}

#[cfg(test)]
mod tests;
