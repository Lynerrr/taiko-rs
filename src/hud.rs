//! The interface during play and what its toggle (Shift+Tab by default) hides: the player picks
//! which parts go, everything (the hit animation too) unless they choose otherwise.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Score,
    Accuracy,
    Combo,
    Progress,
    Health,
    Mods,
    HitError,
    OffsetHint,
    Judgements,
    HitAnimation,
}

impl Part {
    pub const ALL: [Part; 10] = [
        Part::Score,
        Part::Accuracy,
        Part::Combo,
        Part::Progress,
        Part::Health,
        Part::Mods,
        Part::HitError,
        Part::OffsetHint,
        Part::Judgements,
        Part::HitAnimation,
    ];

    /// For the settings list.
    pub fn label(self) -> &'static str {
        match self {
            Part::Score => "Score",
            Part::Accuracy => "Accuracy",
            Part::Combo => "Combo (on the drum)",
            Part::Progress => "Progress bar",
            Part::Health => "Health bar",
            Part::Mods => "Mods",
            Part::HitError => "Hit error bar",
            Part::OffsetHint => "Offset suggestion",
            Part::Judgements => "GREAT / OK / MISS",
            Part::HitAnimation => "Note hit animation",
        }
    }

    /// For the config.
    fn key(self) -> &'static str {
        match self {
            Part::Score => "score",
            Part::Accuracy => "accuracy",
            Part::Combo => "combo",
            Part::Progress => "progress",
            Part::Health => "health",
            Part::Mods => "mods",
            Part::HitError => "hit_error",
            Part::OffsetHint => "offset_hint",
            Part::Judgements => "judgements",
            Part::HitAnimation => "hit_animation",
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
