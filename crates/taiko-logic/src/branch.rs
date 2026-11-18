//! Branching charts (a TJA chart's `#BRANCHSTART`): some bars come in up to three versions,
//! Normal, Expert and Master, and which one is played is picked during the play from how the
//! player is doing, as in Taiko no Tatsujin. Every version's notes are in the map; `Branching`
//! says which are whose, and `sim::Game` leaves out the ones off the branch it plays.

use serde::{Deserialize, Serialize};

/// The branches, as `Branching` numbers them.
pub const NORMAL: u8 = 0;
pub const EXPERT: u8 = 1;
pub const MASTER: u8 = 2;

/// The branch names, by number.
pub const NAMES: [&str; 3] = ["Normal", "Expert", "Master"];

/// What a branch point looks at (OpenTaiko's condition letters).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Measure {
    /// No condition: the branch stays as it is.
    Keep,
    /// `p`: accuracy in %, (GREATs + OKs / 2) / notes judged.
    Accuracy,
    /// `pp`: GREATs in % of the notes judged.
    GreatShare,
    /// `jp` (and `d`, big notes only): how many GREATs.
    Greats,
    /// `jg`: how many OKs.
    Oks,
    /// `jb`: how many misses.
    Misses,
    /// `r`: drumroll and balloon hits.
    Rolls,
    /// `rb`: balloon hits.
    BalloonHits,
    /// `s`: the play's score (this game's, not the arcade's).
    Score,
}

/// A branch point's condition: Master at `master` or more, else Expert at `expert` or more,
/// else Normal (`below` turns it around: under the numbers).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Condition {
    pub measure: Measure,
    /// Written in capitals (`P`, `R`...): only big notes count.
    pub big_only: bool,
    /// `l` after the numbers: the branch is earned below them.
    pub below: bool,
    pub expert: f64,
    pub master: f64,
}

impl Condition {
    pub const KEEP: Condition = Condition {
        measure: Measure::Keep,
        big_only: false,
        below: false,
        expert: 0.0,
        master: 0.0,
    };

    /// `#BRANCHSTART`'s argument read as OpenTaiko does (`p,80,90`, `r 5,10`, `P,50,70,l`);
    /// None for what it can't read, local counters and triggers included.
    pub fn parse(arg: &str) -> Option<Condition> {
        let arg = arg.trim();
        if arg.is_empty() {
            return Some(Condition::KEEP);
        }
        // A missing comma between the letters and the first number is put in, as there.
        let split = arg
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(arg.len());
        let (letters, rest) = arg.split_at(split);
        let mut parts = rest
            .trim_start_matches([' ', '\t', ','])
            .split(',')
            .map(str::trim);
        let lower = letters.to_ascii_lowercase();
        let big = letters.starts_with(|c: char| c.is_ascii_uppercase());
        let (measure, big_only) = match lower.as_str() {
            "p" => (Measure::Accuracy, big),
            "pp" => (Measure::GreatShare, big),
            "jp" => (Measure::Greats, big),
            "d" => (Measure::Greats, true),
            "jg" => (Measure::Oks, big),
            "jb" => (Measure::Misses, big),
            "r" => (Measure::Rolls, big),
            "rb" => (Measure::BalloonHits, big),
            "s" => (Measure::Score, false),
            _ => return None,
        };
        let expert = parts
            .next()?
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())?;
        let master = parts
            .next()?
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())?;
        let below = match parts.next().unwrap_or("") {
            "l" => true,
            "m" | "" => false,
            _ => return None,
        };
        Some(Condition {
            measure,
            big_only,
            below,
            expert,
            master,
        })
    }

    /// The branch `value` earns, `current` the one played now.
    pub fn target(&self, value: f64, current: u8) -> u8 {
        if self.measure == Measure::Keep {
            return current;
        }
        let meets = |at: f64| if self.below { value < at } else { value >= at };
        if meets(self.master) {
            MASTER
        } else if meets(self.expert) {
            EXPERT
        } else {
            NORMAL
        }
    }
}

/// Where a chart branches.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    /// When the branch is picked (map ms): at the bar line a bar before `start`, so the notes
    /// coming are already the right ones; for roll conditions after the last roll before it.
    pub judge: f64,
    /// Where the branched bars start (map ms).
    pub start: f64,
    pub condition: Condition,
    /// `#LEVELHOLD` in a branch: once picked here, it is played to the end.
    pub hold: [bool; 3],
}

/// Which of a map's notes are in which branch, and where and how a play picks the branch.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Branching {
    /// For each note of the map: None for one every branch plays, else the branch point it
    /// comes after (an index into `points`) and its branch.
    pub notes: Vec<Option<(u16, u8)>>,
    pub points: Vec<Point>,
    /// `#SECTION`s (map ms): what the conditions count starts over from each.
    pub resets: Vec<f64>,
    /// `#LEVELHOLD`s outside any branch (map ms): from each on, the branch stays.
    pub holds: Vec<f64>,
}

impl Branching {
    /// Whether note `i` is played when the branches picked are `path` (by point).
    pub fn plays(&self, i: usize, path: impl Fn(usize) -> u8) -> bool {
        match self.notes.get(i).copied().flatten() {
            None => true,
            Some((point, branch)) => path(usize::from(point)) == branch,
        }
    }

    /// Every time moved by `ms` (`Beatmap::shift`).
    pub fn shift(&mut self, ms: f64) {
        for p in &mut self.points {
            p.judge += ms;
            p.start += ms;
        }
        for t in self.resets.iter_mut().chain(&mut self.holds) {
            *t += ms;
        }
    }

    /// What's wrong with it for a map of `notes` notes, if anything (a chart sent to a server).
    pub fn check(&self, notes: usize) -> Result<(), String> {
        if self.notes.len() != notes {
            return Err("branches don't match the notes".into());
        }
        if self.points.len() > usize::from(u16::MAX) {
            return Err("too many branch points".into());
        }
        let bad_note = self
            .notes
            .iter()
            .flatten()
            .any(|&(point, branch)| usize::from(point) >= self.points.len() || branch > MASTER);
        if bad_note {
            return Err("a note in a branch that isn't there".into());
        }
        let time = |t: f64| t.is_finite() && t.abs() <= 1e8;
        let points_ok = self.points.iter().all(|p| {
            time(p.judge)
                && time(p.start)
                && p.judge <= p.start
                && p.condition.expert.is_finite()
                && p.condition.master.is_finite()
        });
        let ordered = self
            .points
            .windows(2)
            .all(|w| w[0].start <= w[1].start && w[0].judge <= w[1].judge);
        if !points_ok || !ordered || !self.resets.iter().chain(&self.holds).all(|&t| time(t)) {
            return Err("a branch point out of range".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditions_read_as_opentaiko_reads_them() {
        let p = Condition::parse("p,80,90").unwrap();
        assert_eq!(
            (p.measure, p.big_only, p.below),
            (Measure::Accuracy, false, false)
        );
        assert_eq!((p.expert, p.master), (80.0, 90.0));
        // No comma after the letter, capitals for big notes only, `l` for below.
        let r = Condition::parse("R 5, 10,l").unwrap();
        assert_eq!(
            (r.measure, r.big_only, r.below),
            (Measure::Rolls, true, true)
        );
        assert_eq!((r.expert, r.master), (5.0, 10.0));
        assert!(Condition::parse("d,3,6").unwrap().big_only);
        assert_eq!(Condition::parse("").unwrap().measure, Measure::Keep);
        assert_eq!(Condition::parse("lc:x,1,2"), None);
        assert_eq!(Condition::parse("p,80"), None);
    }

    #[test]
    fn the_highest_branch_met_wins() {
        let p = Condition::parse("p,80,90").unwrap();
        assert_eq!(p.target(95.0, NORMAL), MASTER);
        assert_eq!(p.target(90.0, NORMAL), MASTER);
        assert_eq!(p.target(85.0, MASTER), EXPERT);
        assert_eq!(p.target(10.0, MASTER), NORMAL);
        let below = Condition::parse("jb,5,2,l").unwrap();
        assert_eq!(below.target(1.0, NORMAL), MASTER);
        assert_eq!(below.target(4.0, NORMAL), EXPERT);
        assert_eq!(below.target(9.0, MASTER), NORMAL);
        assert_eq!(Condition::KEEP.target(100.0, EXPERT), EXPERT);
    }
}
