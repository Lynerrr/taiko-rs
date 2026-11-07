//! Snap presets: the mapper's own short lists of beat divisions. The wheel and the snap keys
//! step only through the active preset's divisions; "All" is the editor's full list.

use super::timing::SNAP_DIVISORS;

/// The built-in preset: every division the editor offers. It cannot be edited or deleted.
pub const ALL: &str = "All";
/// How many presets are kept; more in the config are ignored.
pub const MAX_PRESETS: usize = 24;

#[derive(Clone, PartialEq, Debug)]
pub struct SnapPreset {
    pub name: String,
    /// Ascending, without repeats, each one of `SNAP_DIVISORS`.
    pub divisors: Vec<u32>,
}

/// `[1, 4, 8]` as "1/1 1/4 1/8".
pub fn auto_name(divisors: &[u32]) -> String {
    let parts: Vec<String> = divisors.iter().map(|d| format!("1/{d}")).collect();
    parts.join(" ")
}

impl SnapPreset {
    /// A preset of `divisors` (the ones on offer, sorted); `None` when none are left. An empty
    /// `name` becomes the divisions' own.
    pub fn new(name: &str, divisors: &[u32]) -> Option<Self> {
        let mut d: Vec<u32> = divisors
            .iter()
            .copied()
            .filter(|d| SNAP_DIVISORS.contains(d))
            .collect();
        d.sort_unstable();
        d.dedup();
        if d.is_empty() {
            return None;
        }
        let name: String = name
            .chars()
            .filter(|c| !c.is_control() && *c != '|')
            .collect();
        let name = name.trim();
        let name = if name.is_empty() {
            auto_name(&d)
        } else {
            name.chars().take(40).collect()
        };
        Some(Self { name, divisors: d })
    }

    /// One config value: "3,6,12|Triplets".
    pub fn to_line(&self) -> String {
        let d: Vec<String> = self.divisors.iter().map(u32::to_string).collect();
        format!("{}|{}", d.join(","), self.name)
    }

    /// Reads a config value; anything malformed gives `None`.
    pub fn parse(line: &str) -> Option<Self> {
        let (divs, name) = line.split_once('|').unwrap_or((line, ""));
        let mut d = Vec::new();
        for part in divs.split(',') {
            let n = part.trim().parse::<u32>().ok()?;
            if !SNAP_DIVISORS.contains(&n) {
                return None;
            }
            d.push(n);
        }
        Self::new(name, &d)
    }
}

/// The presets in config lines: bad ones, repeated names and the reserved "All" are skipped.
pub fn parse_all(lines: &[String]) -> Vec<SnapPreset> {
    let mut out: Vec<SnapPreset> = Vec::new();
    for l in lines {
        let Some(p) = SnapPreset::parse(l) else {
            continue;
        };
        if p.name.eq_ignore_ascii_case(ALL) || out.iter().any(|o| o.name == p.name) {
            continue;
        }
        if out.len() < MAX_PRESETS {
            out.push(p);
        }
    }
    out
}

/// The division of `list` nearest to `cur` (the finer one on a tie); `cur` itself if `list` is
/// empty.
pub fn nearest(list: &[u32], cur: u32) -> u32 {
    list.iter()
        .copied()
        .min_by_key(|d| (d.abs_diff(cur), std::cmp::Reverse(*d)))
        .unwrap_or(cur)
}

/// One step along `list` (ascending) from `cur`, `dir` > 0 finer. From a division not in it, the
/// next one on that side; at an end it stays.
pub fn step(list: &[u32], cur: u32, dir: i32) -> u32 {
    if list.is_empty() || dir == 0 {
        return cur;
    }
    if let Some(at) = list.iter().position(|d| *d == cur) {
        let to = (at as i32 + dir.signum()).clamp(0, list.len() as i32 - 1);
        return list[to as usize];
    }
    if dir > 0 {
        list.iter()
            .copied()
            .find(|d| *d > cur)
            .unwrap_or(list[list.len() - 1])
    } else {
        list.iter()
            .rev()
            .copied()
            .find(|d| *d < cur)
            .unwrap_or(list[0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wheel_steps_only_along_the_preset() {
        let l = [1, 2, 4, 8];
        assert_eq!(step(&l, 4, 1), 8);
        assert_eq!(step(&l, 4, -1), 2);
        assert_eq!(step(&l, 8, 1), 8, "stays at the end");
        assert_eq!(step(&l, 1, -1), 1);
        // From a division outside it, the next one on that side.
        assert_eq!(step(&l, 6, 1), 8);
        assert_eq!(step(&l, 6, -1), 4);
        assert_eq!(step(&l, 16, 1), 8);
        assert_eq!(step(&l, 3, -1), 2);
    }

    #[test]
    fn switching_jumps_to_the_nearest_division() {
        assert_eq!(nearest(&[3, 6, 12], 4), 3);
        assert_eq!(nearest(&[3, 6, 12], 16), 12);
        assert_eq!(nearest(&[3, 6, 12], 6), 6);
        assert_eq!(nearest(&[2, 4], 3), 4, "a tie goes to the finer");
        assert_eq!(nearest(&[], 5), 5);
    }

    #[test]
    fn a_preset_is_sorted_named_and_round_trips() {
        let p = SnapPreset::new("", &[8, 1, 4, 4, 2]).unwrap();
        assert_eq!(p.divisors, [1, 2, 4, 8]);
        assert_eq!(p.name, "1/1 1/2 1/4 1/8");
        let t = SnapPreset::new("Trip|lets", &[12, 3, 6]).unwrap();
        assert_eq!(t.name, "Triplets");
        assert_eq!(SnapPreset::parse(&t.to_line()), Some(t));
        assert!(SnapPreset::new("x", &[]).is_none());
        assert!(SnapPreset::new("x", &[99]).is_none());
    }

    #[test]
    fn bad_config_lines_are_skipped() {
        let lines: Vec<String> = [
            "1,2,4|Basic",
            "",
            "|nothing",
            "a,b|Junk",
            "1,99|Unknown division",
            "3,6|All",
            "1,2,4|Basic",
            "3,6,12",
            "5,,6|Gap",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let p = parse_all(&lines);
        let names: Vec<&str> = p.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Basic", "1/3 1/6 1/12"]);
        let many: Vec<String> = (1..=40).map(|i| format!("1,2|n{i}")).collect();
        assert_eq!(parse_all(&many).len(), MAX_PRESETS);
    }
}
