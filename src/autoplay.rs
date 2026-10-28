//! Which drum key autoplay hits for every note, per playstyle: taiko_arranger's
//! `chart/autoplay.py`, the owner's own play taken down as 1234 strings (2026-10-10).
//!
//! Drum halves are numbered the way taiko players write them: 1 = left kat (rim), 2 = left don
//! (centre), 3 = right don, 4 = right kat; key index = half - 1.
//!
//! - **Full alt**: strict R/L alternation through every rest; only a break of a measure or more
//!   starts on the dominant hand again.
//! - **Semi alt**: full alt, but a group of five or more after a rest restarts on the dominant
//!   hand, and about one shorter group in five does too.
//! - **Single tap**: every group (notes closer than half a beat) starts on the dominant hand,
//!   then alternates.
//! - **Roll**: single tap, plus a roll: at a colour change inside a group, when the run just
//!   finished has two or more notes and the new one an odd length, the hand that just hit hits
//!   again (`kkkdddk` = 4143234).
//! - **DDKK**: one hand per colour; each colour run alternates its two halves from the start.
//!
//! A binding is the order the four physical keys (left kat, left don, right don, right kat)
//! light the drum in: 1324 puts each hand's don on the other side.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    FullAlt,
    SemiAlt,
    SingleTap,
    Roll,
    Ddkk,
}

impl Style {
    pub const ALL: [Style; 5] = [Style::FullAlt, Style::SemiAlt, Style::SingleTap, Style::Roll, Style::Ddkk];

    pub fn name(self) -> &'static str {
        match self {
            Style::FullAlt => "kddk full alternate",
            Style::SemiAlt => "kddk semi alternate",
            Style::SingleTap => "kddk single tap",
            Style::Roll => "kddk roll",
            Style::Ddkk => "ddkk",
        }
    }

    /// Its name in the config file (taiko_arranger's).
    pub fn key(self) -> &'static str {
        match self {
            Style::FullAlt => "full",
            Style::SemiAlt => "semi",
            Style::SingleTap => "single",
            Style::Roll => "roll",
            Style::Ddkk => "ddkk",
        }
    }

    pub fn parse(s: &str) -> Option<Style> {
        Style::ALL.into_iter().find(|st| st.key().eq_ignore_ascii_case(s))
    }

    pub fn step(self, dir: i32) -> Style {
        let i = Style::ALL.iter().position(|&s| s == self).unwrap_or(0) as i32;
        Style::ALL[(i + dir).rem_euclid(Style::ALL.len() as i32) as usize]
    }
}

pub const BINDINGS: [&str; 3] = ["1234", "1324", "4231"];

/// Autoplay's playstyle, binding (`BINDINGS[binding]`) and hand.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Settings {
    pub style: Style,
    pub binding: usize,
    pub left_handed: bool,
}

impl Default for Settings {
    /// kddk full alt, as taiko_arranger starts (owner, 2026-10-10).
    fn default() -> Self {
        Settings { style: Style::FullAlt, binding: 0, left_handed: false }
    }
}

/// Semi alt: groups of this many notes after a rest always start dominant, and this share of
/// the shorter ones do (owner: "80% full alt, 20% single tap").
const SEMI_RESET_LENGTH: usize = 5;
const SEMI_RESET_SHARE: f64 = 0.2;

const LEFT_KAT: usize = 0;
const LEFT_DON: usize = 1;
const RIGHT_DON: usize = 2;
const RIGHT_KAT: usize = 3;

/// A hash of the note's own time rather than a random draw, so every play is the same.
fn semi_resets(time: f64) -> bool {
    ((time as i64 as i128 * 2654435761).rem_euclid(1 << 32) as f64) < SEMI_RESET_SHARE * (1u64 << 32) as f64
}

/// Notes (time, kat, big) split where they're half a beat or more apart (a hair under counts:
/// snapped milliseconds are truncated).
fn groups(notes: &[(f64, bool, bool)], beat: f64) -> Vec<std::ops::Range<usize>> {
    let mut out: Vec<std::ops::Range<usize>> = Vec::new();
    for (i, n) in notes.iter().enumerate() {
        match out.last_mut() {
            Some(g) if n.0 - notes[g.end - 1].0 < beat / 2.0 - 1.0 => g.end = i + 1,
            _ => out.push(i..i + 1),
        }
    }
    out
}

/// Length of the same-colour run each position belongs to.
fn run_lengths(colours: &[bool]) -> Vec<usize> {
    let mut lengths = vec![0; colours.len()];
    let mut start = 0;
    for i in 1..=colours.len() {
        if i == colours.len() || colours[i] != colours[start] {
            lengths[start..i].fill(i - start);
            start = i;
        }
    }
    lengths
}

/// 0 = dominant hand, 1 = the other, per note.
fn kddk_hands(notes: &[(f64, bool, bool)], groups: &[std::ops::Range<usize>], style: Style, beat: f64) -> Vec<u8> {
    let mut hands: Vec<u8> = Vec::with_capacity(notes.len());
    let mut previous: Option<f64> = None;
    for g in groups {
        let group = &notes[g.clone()];
        let after_break = previous.is_none_or(|p| group[0].0 - p >= 4.0 * beat);
        let restart = match style {
            Style::FullAlt => after_break,
            Style::SemiAlt => after_break || group.len() >= SEMI_RESET_LENGTH || semi_resets(group[0].0),
            _ => true,
        };
        let mut hand = match hands.last() {
            Some(&h) if !restart => 1 - h,
            _ => 0,
        };
        let colours: Vec<bool> = group.iter().map(|n| n.1).collect();
        let runs = run_lengths(&colours);
        for i in 0..group.len() {
            if i > 0 {
                let rolls = style == Style::Roll && colours[i] != colours[i - 1] && runs[i - 1] >= 2 && runs[i] % 2 == 1;
                if !rolls {
                    hand = 1 - hand;
                }
            }
            hands.push(hand);
        }
        previous = Some(group[group.len() - 1].0);
    }
    hands
}

/// The drum keys (0..4) hit for each of `notes`, (time, kat, big) sorted by time. A big note
/// hits both of its colour.
pub fn note_keys(notes: &[(f64, bool, bool)], beat: f64, s: Settings) -> Vec<Vec<usize>> {
    let groups = groups(notes, beat);
    if s.style == Style::Ddkk {
        // d on 2 then 3, k on 1 then 4; mirrored for the left hand.
        let cycle = |kat: bool| match (kat, s.left_handed) {
            (false, false) => [2, 3],
            (true, false) => [1, 4],
            (false, true) => [3, 2],
            (true, true) => [4, 1],
        };
        let mut out = Vec::with_capacity(notes.len());
        for g in &groups {
            let mut step = 0;
            for i in g.clone() {
                let (_, kat, big) = notes[i];
                step = if i > g.start && kat == notes[i - 1].1 { step + 1 } else { 0 };
                let pair = cycle(kat);
                let mut halves = if big { pair.to_vec() } else { vec![pair[step % 2]] };
                halves.sort();
                out.push(halves.into_iter().map(|h| h - 1).collect());
            }
        }
        return out;
    }
    let keys: Vec<usize> = BINDINGS[s.binding.min(BINDINGS.len() - 1)].bytes().map(|b| (b - b'0') as usize).collect();
    let hands = kddk_hands(notes, &groups, s.style, beat);
    notes
        .iter()
        .zip(hands)
        .map(|(&(_, kat, big), hand)| {
            let right = (hand == 0) != s.left_handed;
            let positions: Vec<usize> = match (big, kat, right) {
                (true, true, _) => vec![LEFT_KAT, RIGHT_KAT],
                (true, false, _) => vec![LEFT_DON, RIGHT_DON],
                (false, true, r) => vec![if r { RIGHT_KAT } else { LEFT_KAT }],
                (false, false, r) => vec![if r { RIGHT_DON } else { LEFT_DON }],
            };
            let mut halves: Vec<usize> = positions.into_iter().map(|p| keys[p]).collect();
            halves.sort();
            halves.into_iter().map(|h| h - 1).collect()
        })
        .collect()
}

/// A drumroll's `k`th tick: dons alternating from the dominant hand, as osu!'s autoplay plays one.
pub fn roll_key(k: usize, left_handed: bool) -> usize {
    // Halves 3 then 2 (right-handed), 2 then 3 (left).
    if (k % 2 == 0) != left_handed { RIGHT_DON } else { LEFT_DON }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `pattern` (d/k/D/K, one per 1/4 at 120 BPM, ' ' a 1/4 rest) as drum halves 1-4.
    fn play(pattern: &str, style: Style, binding: usize, left: bool) -> String {
        let notes: Vec<(f64, bool, bool)> = pattern
            .chars()
            .enumerate()
            .filter(|(_, c)| *c != ' ')
            .map(|(i, c)| (i as f64 * 125.0, c.eq_ignore_ascii_case(&'k'), c.is_uppercase()))
            .collect();
        note_keys(&notes, 500.0, Settings { style, binding, left_handed: left })
            .iter()
            .map(|k| k.iter().map(|h| (b'1' + *h as u8) as char).collect::<String>())
            .collect::<Vec<_>>()
            .join("")
    }

    #[test]
    fn the_styles_play_like_taiko_arranger() {
        // Full alt keeps alternating over a short rest; single tap restarts on the right.
        assert_eq!(play("ddd   ddd", Style::FullAlt, 0, false), "323232");
        assert_eq!(play("ddd   ddd", Style::SingleTap, 0, false), "323323");
        // Roll: kkkdddk = 4143234.
        assert_eq!(play("kkkdddk", Style::Roll, 0, false), "4143234");
        // DDKK: one hand per colour.
        assert_eq!(play("ddkk", Style::Ddkk, 0, false), "2314");
        // Big notes hit both halves of their colour.
        assert_eq!(play("D", Style::FullAlt, 0, false), "23");
        // Left-handed starts on the left; 1324 swaps each hand's don.
        assert_eq!(play("dd", Style::FullAlt, 0, true), "23");
        assert_eq!(play("dd", Style::FullAlt, 1, false), "23");
    }

    #[test]
    fn rolls_alternate_from_the_dominant_hand() {
        assert_eq!((roll_key(0, false), roll_key(1, false)), (RIGHT_DON, LEFT_DON));
        assert_eq!((roll_key(0, true), roll_key(1, true)), (LEFT_DON, RIGHT_DON));
    }

    #[test]
    fn styles_read_back() {
        for s in Style::ALL {
            assert_eq!(Style::parse(s.key()), Some(s));
        }
        assert_eq!(Style::FullAlt.step(-1), Style::Ddkk);
    }
}
