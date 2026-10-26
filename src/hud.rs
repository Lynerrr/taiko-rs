//! The interface during play and what its toggle (Shift+Tab by default) hides: the player picks
//! which parts go: everything but GREAT / OK / MISS (like osu!, which keeps judgements on with
//! the HUD hidden) unless they choose otherwise.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Score,
    Accuracy,
    Pp,
    Combo,
    Progress,
    Health,
    Mods,
    HitError,
    OffsetHint,
    Judgements,
    HitAnimation,
    /// A multiplayer match's scoreboard.
    Scoreboard,
}

impl Part {
    pub const ALL: [Part; 12] = [
        Part::Score,
        Part::Accuracy,
        Part::Pp,
        Part::Combo,
        Part::Progress,
        Part::Health,
        Part::Mods,
        Part::HitError,
        Part::OffsetHint,
        Part::Judgements,
        Part::HitAnimation,
        Part::Scoreboard,
    ];

    /// For the settings list.
    pub fn label(self) -> &'static str {
        match self {
            Part::Score => "Score",
            Part::Accuracy => "Accuracy",
            Part::Pp => "pp counter",
            Part::Combo => "Combo (on the drum)",
            Part::Progress => "Progress bar",
            Part::Health => "Health bar",
            Part::Mods => "Mods",
            Part::HitError => "Hit error bar",
            Part::OffsetHint => "Offset suggestion",
            Part::Judgements => "GREAT / OK / MISS",
            Part::HitAnimation => "Note hit animation",
            Part::Scoreboard => "Match scoreboard",
        }
    }

    /// For the config.
    fn key(self) -> &'static str {
        match self {
            Part::Score => "score",
            Part::Accuracy => "accuracy",
            Part::Pp => "pp",
            Part::Combo => "combo",
            Part::Progress => "progress",
            Part::Health => "health",
            Part::Mods => "mods",
            Part::HitError => "hit_error",
            Part::OffsetHint => "offset_hint",
            Part::Judgements => "judgements",
            Part::HitAnimation => "hit_animation",
            Part::Scoreboard => "scoreboard",
        }
    }

    fn bit(self) -> u16 {
        1 << Part::ALL.iter().position(|&p| p == self).unwrap_or(0)
    }
}

/// A set of interface parts.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Parts(u16);

impl Parts {
    pub const NONE: Parts = Parts(0);
    pub const ALL: Parts = Parts((1 << Part::ALL.len()) - 1);
    /// What the toggle hides to begin with: all but the judgements (`Part::Judgements`, 10th).
    pub const DEFAULT_HIDES: Parts = Parts(Parts::ALL.0 & !(1 << 9));

    pub fn has(self, p: Part) -> bool {
        self.0 & p.bit() != 0
    }

    pub fn toggle(&mut self, p: Part) {
        self.0 ^= p.bit();
    }

    pub fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    /// "score,accuracy,..." (empty: none).
    pub fn format(self) -> String {
        Part::ALL.iter().filter(|&&p| self.has(p)).map(|p| p.key()).collect::<Vec<_>>().join(",")
    }

    /// The config's line: saved before parts that came later (the pp counter, the scoreboard),
    /// hiding all it knew of still hides those too.
    pub fn parse_saved(s: &str) -> Parts {
        let mut parts = Parts::parse(s);
        let newer = [Part::Pp, Part::Scoreboard];
        let saved = |p: Part| s.split(',').any(|k| k.trim() == p.key());
        if Part::ALL.iter().all(|&p| parts.has(p) || (newer.contains(&p) && !saved(p))) {
            parts = Parts::ALL;
        }
        parts
    }

    pub fn parse(s: &str) -> Parts {
        let mut out = Parts::NONE;
        for word in s.split(',').map(str::trim) {
            if let Some(&p) = Part::ALL.iter().find(|p| p.key() == word) {
                out.0 |= p.bit();
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hiding_everything_saved_before_newer_parts_still_hides_them() {
        let old = "score,accuracy,combo,progress,health,mods,hit_error,offset_hint,judgements,hit_animation";
        assert_eq!(Parts::parse_saved(old), Parts::ALL);
        assert_eq!(Parts::parse_saved(&format!("{old},pp")), Parts::ALL);
        let some = Parts::parse_saved("score,pp");
        assert!(some.has(Part::Score) && !some.has(Part::Scoreboard) && some.len() == 2);
        assert_eq!(Parts::parse_saved(&Parts::DEFAULT_HIDES.format()), Parts::DEFAULT_HIDES);
    }

    #[test]
    fn hiding_the_interface_keeps_the_judgements_to_begin_with() {
        let d = Parts::DEFAULT_HIDES;
        assert!(!d.has(Part::Judgements) && d.len() == Part::ALL.len() - 1);
    }

    #[test]
    fn parts_round_trip_through_the_config() {
        let mut some = Parts::NONE;
        some.toggle(Part::Score);
        some.toggle(Part::HitAnimation);
        for p in [Parts::NONE, Parts::ALL, some] {
            assert_eq!(Parts::parse(&p.format()), p);
        }
        assert_eq!(Parts::ALL.len(), Part::ALL.len());
        assert!(Parts::ALL.has(Part::HitAnimation));
    }
}
